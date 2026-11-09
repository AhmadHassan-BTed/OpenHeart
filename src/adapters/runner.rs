//! Shared analysis runner: the single orchestration of source discovery, the 10-phase SCPG
//! pipeline and diagram export used by both `openheart analyze` and the HTTP server.
//!
//! Nothing here keeps its own list of languages or diagrams. The language catalog is read
//! from the frontend registries of Phases 1-3 (which grammar or lexer Phase 1 links, whether
//! Phase 2 builds a syntax tree, how Phase 3 discovers declarations), and the diagram catalog
//! from the diagram engine's view kinds and registered exporters, so a frontend or exporter
//! added to those registries is picked up by the CLI, the server and every API response.
//!
//! Every run writes into a private, freshly created work directory, so two runs never
//! truncate artifacts the other has memory-mapped. Callers either keep that directory
//! (server sessions) or publish its files into a destination with per-file atomic renames
//! (CLI output directories). Phases run behind panic isolation, report typed errors, and
//! observe a [`RunControl`] (cancellation, wall-clock budget, progress) between phases;
//! [`run_isolated`] additionally moves a whole run onto a thread with an explicit stack
//! budget, because several CFG/SSA walks recurse proportionally to method size.

#![forbid(unsafe_code)]

use std::any::Any;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, OnceLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ignore::WalkBuilder;
use serde::Serialize;
use thiserror::Error;

use crate::adapters::env;
use crate::ast::adapter::registry::ASTAdapterRegistry;
use crate::ast::{ASTStage, ASTStageOptions, BPASTArtifact};
use crate::cfg::serializer::CFGArtifact;
use crate::cfg::Phase4Stage;
use crate::cg::Phase6Stage;
use crate::core::logger::{log_debug, log_info, log_warn};
use crate::core::types::cg::CallGraphArtifact;
use crate::core::types::source::SkipReason;
use crate::core::types::token::LangId;
use crate::ingestion::adapter::registry::{AdapterRegistry, Frontend};
use crate::ingestion::manifest::SourceManifest;
use crate::ingestion::{IngestionReport, IngestionStage, TokenCorpusArtifact};
use crate::psa::{PathSummaryArtifact, Phase8Config, Phase8Stage};
use crate::scpg::diagram::{
    DiagramContext, DiagramFormat, UniversalDiagramEngine, ViewKind, ViewOptions,
};
use crate::scpg::{OpenHeartEngine, Phase10Stage};
use crate::ssa::{Phase5Stage, SSAArtifact};
use crate::symbol::{JavaSemanticAdapter, Phase3Stage, SemanticAdapter, SymbolTableArtifact};
use crate::tra::{Phase7Stage, TraceabilityArtifact};
use crate::uma::{Phase9Options, Phase9Stage, UMLMetadataArtifact};

// ── Language catalog ─────────────────────────────────────────────────────────

/// One language the pipeline ingests, with the depth of analysis its registered frontends
/// provide. Every field is read from the Phase 1-3 registries, never configured here.
#[derive(Debug, Clone, Serialize)]
pub struct SourceLanguage {
    /// Stable, lower-case language identifier.
    pub id: String,
    /// The engine's language id.
    #[serde(skip)]
    pub lang: LangId,
    /// Lower-case file extensions (without the dot) the Phase 1 frontend declares.
    pub extensions: Vec<String>,
    /// Phase 1 frontend: `"grammar"` (a linked tree-sitter grammar) or `"lexer"`.
    pub lexical_frontend: &'static str,
    /// Whether Phase 2 builds a syntax tree for files of this language.
    pub syntax_tree: bool,
    /// How Phase 3 discovers declarations: `"structural"` (grammar fields of the
    /// language's own AST), `"ast-heuristic"` (generic AST nodes, names from tokens) or
    /// `"lexical"` (a token-level parser, no AST).
    pub declaration_frontend: String,
    /// Summary of the above: `"full"` (syntax tree and structural declarations),
    /// `"partial"` (syntax tree, heuristic declarations) or `"lexical"` (no syntax tree).
    pub tier: &'static str,
}

/// The result of routing one path through the Phase 1 language detection.
#[derive(Debug, Clone, Copy)]
pub enum LanguageMatch {
    /// The language has a registered frontend.
    Supported(&'static SourceLanguage),
    /// The language was recognised but no frontend is linked for it.
    NoFrontend(LangId),
    /// The extension maps to no language at all.
    Unknown,
}

/// Languages with a registered Phase 1 frontend, built once from the engine registries.
pub struct LanguageCatalog {
    registry: AdapterRegistry,
    languages: Vec<SourceLanguage>,
}

impl LanguageCatalog {
    fn build() -> Self {
        let registry = AdapterRegistry::new();
        let trees = ASTAdapterRegistry::new();
        let semantics = JavaSemanticAdapter::new();
        let mut languages = Vec::new();
        for lang in registry.supported_languages() {
            let Some(frontend) = registry.frontend(lang) else {
                continue;
            };
            let (lexical_frontend, declared) = match &frontend {
                Frontend::Grammar(adapter) => ("grammar", adapter.file_extensions().to_vec()),
                Frontend::Lexer(lexer) => ("lexer", lexer.file_extensions().to_vec()),
            };
            let mut extensions: Vec<String> = declared
                .iter()
                .map(|ext| ext.trim_start_matches('.').to_ascii_lowercase())
                .filter(|ext| !ext.is_empty())
                .collect();
            extensions.sort();
            extensions.dedup();
            let syntax_tree = trees.supports(lang);
            let declaration_frontend = kebab_case(&format!("{:?}", semantics.front_end(Some(lang))));
            let tier = match (syntax_tree, declaration_frontend.as_str()) {
                (true, "structural") => "full",
                (true, _) => "partial",
                (false, _) => "lexical",
            };
            languages.push(SourceLanguage {
                id: language_name(lang),
                lang,
                extensions,
                lexical_frontend,
                syntax_tree,
                declaration_frontend,
                tier,
            });
        }
        Self {
            registry,
            languages,
        }
    }

    /// Every language with a registered frontend, in engine id order.
    pub fn languages(&self) -> &[SourceLanguage] {
        &self.languages
    }

    /// The language with identifier `id` (case-insensitive).
    pub fn get(&self, id: &str) -> Option<&SourceLanguage> {
        let wanted = id.trim().to_ascii_lowercase();
        self.languages.iter().find(|language| language.id == wanted)
    }

    fn by_lang(&self, lang: LangId) -> Option<&SourceLanguage> {
        self.languages.iter().find(|language| language.lang == lang)
    }

    /// Routes `path` exactly as Phase 1 does: the built-in extension detection first, then
    /// the extensions claimed by registered frontends.
    pub fn detect(&'static self, path: &Path) -> LanguageMatch {
        let mut lang = AdapterRegistry::detect(&HashMap::new(), path);
        if lang == LangId::Unknown {
            if let Some(registered) = path
                .extension()
                .and_then(|ext| ext.to_str())
                .and_then(|ext| self.registry.language_for_extension(ext))
            {
                lang = registered;
            }
        }
        if lang == LangId::Unknown {
            return LanguageMatch::Unknown;
        }
        match self.by_lang(lang) {
            Some(language) => LanguageMatch::Supported(language),
            None => LanguageMatch::NoFrontend(lang),
        }
    }

    /// The accepted extensions as `.ext` list, for messages.
    pub fn accepted_extensions(&self) -> String {
        let listed: Vec<String> = self
            .languages
            .iter()
            .flat_map(|language| language.extensions.iter())
            .map(|ext| format!(".{}", ext))
            .collect();
        if listed.is_empty() {
            "no frontend is registered".to_string()
        } else {
            listed.join(", ")
        }
    }

    /// Comma-separated language identifiers, for messages.
    pub fn identifiers(&self) -> String {
        self.languages
            .iter()
            .map(|language| language.id.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// The process-wide language catalog.
pub fn language_catalog() -> &'static LanguageCatalog {
    static CATALOG: OnceLock<LanguageCatalog> = OnceLock::new();
    CATALOG.get_or_init(LanguageCatalog::build)
}

/// Every language with a registered frontend, read from the [`language_catalog`].
pub static SOURCE_LANGUAGES: LazyLock<&'static [SourceLanguage]> =
    LazyLock::new(|| language_catalog().languages());

/// Lower-case name of an engine language id.
pub fn language_name(lang: LangId) -> String {
    format!("{:?}", lang).to_ascii_lowercase()
}

/// `AstHeuristic` → `ast-heuristic`.
fn kebab_case(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    for (index, c) in name.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if index > 0 {
                out.push('-');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// Resolves language identifiers against the catalog; `"all"` (alone) selects every
/// language and yields `None`.
pub fn resolve_languages<I, S>(names: I) -> Result<Option<Vec<&'static str>>, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let catalog = language_catalog();
    let mut selected: Vec<&'static str> = Vec::new();
    let mut unknown = Vec::new();
    let mut saw_all = false;
    for name in names {
        let name = name.as_ref().trim();
        if name.is_empty() {
            continue;
        }
        if name.eq_ignore_ascii_case("all") {
            saw_all = true;
            continue;
        }
        match catalog.get(name) {
            Some(language) if !selected.contains(&language.id.as_str()) => {
                selected.push(language.id.as_str())
            }
            Some(_) => {}
            None => unknown.push(name.to_string()),
        }
    }
    if !unknown.is_empty() {
        return Err(format!(
            "unknown or unsupported languages: {} (available: {})",
            unknown.join(", "),
            catalog.identifiers()
        ));
    }
    if saw_all {
        if !selected.is_empty() {
            return Err("'all' cannot be combined with individual languages".to_string());
        }
        return Ok(None);
    }
    if selected.is_empty() {
        return Err("the language list is empty".to_string());
    }
    Ok(Some(selected))
}

/// Parses a comma-separated language list (see [`resolve_languages`]).
pub fn parse_language_list(raw: &str) -> Result<Option<Vec<&'static str>>, String> {
    resolve_languages(raw.split(','))
}

/// `OPENHEART_LANGUAGES`, a comma-separated language filter (unset: every language).
pub fn languages_from_env() -> Result<Option<Vec<&'static str>>, String> {
    match env::string("OPENHEART_LANGUAGES") {
        None => Ok(None),
        Some(raw) => parse_language_list(&raw)
            .map_err(|err| format!("environment variable OPENHEART_LANGUAGES: {}", err)),
    }
}

// ── Diagram catalog ──────────────────────────────────────────────────────────

/// Views derived from compiler IR rather than from UML metadata.
fn is_analysis_graph(kind: ViewKind) -> bool {
    matches!(
        kind,
        ViewKind::Cfg | ViewKind::Dfg | ViewKind::Cdg | ViewKind::CallGraph | ViewKind::Robdd
    )
}

/// One entry of the diagram catalog.
#[derive(Debug, Clone, Serialize)]
pub struct DiagramKindInfo {
    /// Canonical identifier.
    pub id: &'static str,
    /// Human-readable title.
    pub title: &'static str,
    /// `"uml"` or `"analysis"`.
    pub family: &'static str,
}

/// Every diagram the engine can project, in engine order.
pub fn diagram_catalog() -> Vec<DiagramKindInfo> {
    ViewKind::ALL
        .iter()
        .map(|kind| DiagramKindInfo {
            id: kind.key(),
            title: kind.title(),
            family: if is_analysis_graph(*kind) {
                "analysis"
            } else {
                "uml"
            },
        })
        .collect()
}

/// Canonical identifiers of the UML projections.
pub fn uml_diagram_types() -> Vec<&'static str> {
    ViewKind::ALL
        .iter()
        .filter(|kind| !is_analysis_graph(**kind))
        .map(|kind| kind.key())
        .collect()
}

/// Canonical identifiers of the program-analysis graph projections.
pub fn analysis_graph_types() -> Vec<&'static str> {
    ViewKind::ALL
        .iter()
        .filter(|kind| is_analysis_graph(**kind))
        .map(|kind| kind.key())
        .collect()
}

/// Canonical identifiers of the UML projections (see [`uml_diagram_types`]).
pub static UML_DIAGRAM_TYPES: LazyLock<Vec<&'static str>> = LazyLock::new(uml_diagram_types);

/// Canonical identifiers of the program-analysis graphs (see [`analysis_graph_types`]).
pub static ANALYSIS_GRAPH_TYPES: LazyLock<Vec<&'static str>> =
    LazyLock::new(analysis_graph_types);

/// Every canonical diagram identifier: the UML projections followed by the analysis graphs.
pub fn all_diagram_types() -> Vec<&'static str> {
    let mut all = uml_diagram_types();
    all.extend(analysis_graph_types());
    all
}

/// Maps a diagram name or one of its aliases (`use_case`, `use-case`, `state`, `cg`, ...)
/// to its canonical identifier, or `None` when the engine has no such view.
pub fn canonical_diagram_type(name: &str) -> Option<&'static str> {
    ViewKind::parse(&name.trim().replace('-', "_")).map(ViewKind::key)
}

/// Formats with a registered exporter, in engine order.
pub fn diagram_formats() -> Vec<DiagramFormat> {
    UniversalDiagramEngine::new().formats()
}

/// Parses a format name or file extension, accepting only formats with a registered
/// exporter.
pub fn parse_diagram_format(name: &str) -> Option<DiagramFormat> {
    DiagramFormat::parse(name).filter(|format| diagram_formats().contains(format))
}

/// Formats exported when the caller does not choose any.
pub const DEFAULT_DIAGRAM_FORMATS: [DiagramFormat; 3] = [
    DiagramFormat::PlantUML,
    DiagramFormat::Mermaid,
    DiagramFormat::JSON,
];

/// Parses a comma-separated diagram list; `all` selects every view, `uml` the UML
/// projections and `analysis` the program-analysis graphs.
pub fn parse_diagram_list(raw: &str) -> Result<Vec<&'static str>, String> {
    resolve_diagram_types(raw.split(','))
}

/// Resolves diagram names (see [`parse_diagram_list`]) without duplicates.
pub fn resolve_diagram_types<I, S>(names: I) -> Result<Vec<&'static str>, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    fn push_unique(selection: &mut Vec<&'static str>, id: &'static str) {
        if !selection.contains(&id) {
            selection.push(id);
        }
    }
    let mut selection: Vec<&'static str> = Vec::new();
    let mut unknown = Vec::new();
    for name in names {
        let name = name.as_ref().trim();
        if name.is_empty() {
            continue;
        }
        let group = match name.to_ascii_lowercase().as_str() {
            "all" => Some(all_diagram_types()),
            "uml" => Some(uml_diagram_types()),
            "analysis" => Some(analysis_graph_types()),
            _ => None,
        };
        match group {
            Some(ids) => ids
                .into_iter()
                .for_each(|id| push_unique(&mut selection, id)),
            None => match canonical_diagram_type(name) {
                Some(id) => push_unique(&mut selection, id),
                None => unknown.push(name.to_string()),
            },
        }
    }
    if !unknown.is_empty() {
        return Err(format!(
            "unknown diagram types: {} (available: {})",
            unknown.join(", "),
            all_diagram_types().join(", ")
        ));
    }
    if selection.is_empty() {
        return Err("the diagram list is empty".to_string());
    }
    Ok(selection)
}

/// Parses a comma-separated format list; `all` selects every registered format.
pub fn parse_format_list(raw: &str) -> Result<Vec<DiagramFormat>, String> {
    resolve_formats(raw.split(','))
}

/// Resolves format names (see [`parse_format_list`]) without duplicates.
pub fn resolve_formats<I, S>(names: I) -> Result<Vec<DiagramFormat>, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let registered = diagram_formats();
    let mut formats = Vec::new();
    let mut unknown = Vec::new();
    for name in names {
        let name = name.as_ref().trim();
        if name.is_empty() {
            continue;
        }
        if name.eq_ignore_ascii_case("all") {
            for format in &registered {
                if !formats.contains(format) {
                    formats.push(*format);
                }
            }
            continue;
        }
        match parse_diagram_format(name) {
            Some(format) if !formats.contains(&format) => formats.push(format),
            Some(_) => {}
            None => unknown.push(name.to_string()),
        }
    }
    if !unknown.is_empty() {
        let available: Vec<&str> = registered.iter().map(|format| format.as_str()).collect();
        return Err(format!(
            "unknown diagram formats: {} (available: {})",
            unknown.join(", "),
            available.join(", ")
        ));
    }
    if formats.is_empty() {
        return Err("the format list is empty".to_string());
    }
    Ok(formats)
}

// ── Limits and budgets ───────────────────────────────────────────────────────

/// Hard ceiling imposed by the token corpus format (16-bit file ids), as Phase 1 enforces it.
pub const MAX_CORPUS_FILES: usize = crate::ingestion::MAX_CORPUS_FILES;

/// Default stack reserved for an isolated pipeline thread (see [`run_isolated`]).
pub const DEFAULT_PIPELINE_STACK_BYTES: usize = 256 * 1024 * 1024;

/// Resource limits applied while selecting source files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SourceLimits {
    /// Maximum number of files admitted into one analysis.
    pub max_files: usize,
    /// Files larger than this are skipped and reported as oversized.
    pub max_file_bytes: u64,
    /// Maximum combined size of all admitted files.
    pub max_total_bytes: u64,
}

impl Default for SourceLimits {
    fn default() -> Self {
        Self {
            max_files: 20_000,
            max_file_bytes: 4 * 1024 * 1024,
            max_total_bytes: 256 * 1024 * 1024,
        }
    }
}

impl SourceLimits {
    /// Defaults overridden by `OPENHEART_MAX_FILES`, `OPENHEART_MAX_FILE_KB` and
    /// `OPENHEART_MAX_SOURCE_MB`.
    pub fn from_env() -> Result<Self, String> {
        let mut limits = Self::default();
        if let Some(files) = env::number::<usize>("OPENHEART_MAX_FILES")? {
            limits.max_files = files.clamp(1, MAX_CORPUS_FILES);
        }
        if let Some(kib) = env::number::<u64>("OPENHEART_MAX_FILE_KB")? {
            limits.max_file_bytes = kib.saturating_mul(1024).max(1);
        }
        if let Some(mib) = env::number::<u64>("OPENHEART_MAX_SOURCE_MB")? {
            limits.max_total_bytes = mib.saturating_mul(1024 * 1024).max(1);
        }
        Ok(limits)
    }
}

/// Stack size for isolated pipeline threads: `OPENHEART_STACK_MB` or the default.
pub fn pipeline_stack_bytes_from_env() -> Result<usize, String> {
    Ok(match env::number::<usize>("OPENHEART_STACK_MB")? {
        Some(mib) => mib.clamp(8, 4096) * 1024 * 1024,
        None => DEFAULT_PIPELINE_STACK_BYTES,
    })
}

/// `base` overridden by `OPENHEART_BDD_MAX_NODES`, `OPENHEART_BDD_TIME_MS`,
/// `OPENHEART_BDD_SIFT`, `OPENHEART_BDD_SIFT_TIME_MS` and `OPENHEART_PHASE8_THREADS`.
pub fn phase8_config_from_env(base: Phase8Config) -> Result<Phase8Config, String> {
    let mut config = base;
    if let Some(nodes) = env::number::<usize>("OPENHEART_BDD_MAX_NODES")? {
        config.max_bdd_nodes = nodes.max(1024);
    }
    if let Some(millis) = env::number::<u64>("OPENHEART_BDD_TIME_MS")? {
        config.bdd_time_budget = Duration::from_millis(millis.max(1));
    }
    if let Some(sift) = env::flag("OPENHEART_BDD_SIFT")? {
        config.sift = sift;
    }
    if let Some(millis) = env::number::<u64>("OPENHEART_BDD_SIFT_TIME_MS")? {
        config.sift_time_budget = Duration::from_millis(millis.max(1));
    }
    if let Some(threads) = env::number::<usize>("OPENHEART_PHASE8_THREADS")? {
        config.worker_threads = threads.min(256);
    }
    Ok(config)
}

// ── Errors ───────────────────────────────────────────────────────────────────

/// Failure of source discovery, of one pipeline phase, or of artifact I/O.
#[derive(Debug, Error)]
pub enum PipelineError {
    /// The input path is unusable (missing, not a file or directory, unsupported file).
    #[error("{0}")]
    InvalidInput(String),
    /// Discovery finished without admitting any analyzable file.
    #[error("no analyzable source files ({languages}) were found under the analysis root")]
    NoSourceFiles {
        /// Human-readable list of accepted extensions.
        languages: String,
    },
    /// More files than the configured limit would have been admitted.
    #[error("the source tree has more than {limit} analyzable files")]
    TooManyFiles {
        /// The configured file-count limit.
        limit: usize,
    },
    /// The admitted files exceed the configured total size.
    #[error("the selected sources exceed the size limit of {limit_bytes} bytes")]
    SourceTooLarge {
        /// The configured total-size limit in bytes.
        limit_bytes: u64,
    },
    /// Phase 1 skipped every selected file, so there is nothing to analyze.
    #[error(
        "none of the {selected} selected files could be ingested \
         ({unsupported} have no grammar linked for their language)"
    )]
    NothingIngested {
        /// Files handed to Phase 1.
        selected: usize,
        /// Of those, files skipped for lack of a grammar.
        unsupported: usize,
    },
    /// A filesystem operation outside the phases failed.
    #[error("failed to {action} '{path}': {source}")]
    Io {
        /// What was being attempted.
        action: &'static str,
        /// The path involved (server responses never expose it, see [`Self::client_message`]).
        path: String,
        /// The underlying I/O error.
        source: io::Error,
    },
    /// A phase returned an error.
    #[error("Phase {phase} ({name}) failed: {message}")]
    Phase {
        /// Phase number, 1 through 10.
        phase: u8,
        /// Phase name.
        name: &'static str,
        /// The phase's error message.
        message: String,
    },
    /// A phase panicked; the panic was contained and the run aborted.
    #[error("Phase {phase} ({name}) aborted with an internal error: {message}")]
    PhasePanicked {
        /// Phase number, 1 through 10.
        phase: u8,
        /// Phase name.
        name: &'static str,
        /// The panic payload rendered as text.
        message: String,
    },
    /// The run was cancelled through its [`RunControl`].
    #[error("the analysis was cancelled")]
    Cancelled,
    /// The run exceeded the wall-clock budget of its [`RunControl`].
    #[error("the analysis exceeded its time budget of {budget_secs} s")]
    TimedOut {
        /// The budget in seconds.
        budget_secs: u64,
    },
}

impl PipelineError {
    /// A description safe to return to remote clients: it never contains filesystem paths
    /// or raw phase diagnostics, which stay in the server log.
    pub fn client_message(&self) -> String {
        match self {
            Self::InvalidInput(_) => "the analysis input is not usable".to_string(),
            Self::Io { action, .. } => format!("server-side I/O failure while trying to {}", action),
            Self::Phase { phase, name, .. } => format!("Phase {} ({}) failed", phase, name),
            Self::PhasePanicked { phase, name, .. } => {
                format!("Phase {} ({}) aborted with an internal error", phase, name)
            }
            other => other.to_string(),
        }
    }

    /// Whether the failure is caused by the submitted input rather than by the server.
    pub fn is_input_error(&self) -> bool {
        matches!(
            self,
            Self::InvalidInput(_)
                | Self::NoSourceFiles { .. }
                | Self::TooManyFiles { .. }
                | Self::SourceTooLarge { .. }
                | Self::NothingIngested { .. }
        )
    }
}

fn io_error(action: &'static str, path: &Path) -> impl FnOnce(io::Error) -> PipelineError {
    let path = path.display().to_string();
    move |source| PipelineError::Io {
        action,
        path,
        source,
    }
}

/// Renders a panic payload (`&str` or `String`) as text.
pub fn panic_message(payload: &(dyn Any + Send)) -> String {
    if let Some(text) = payload.downcast_ref::<&str>() {
        (*text).to_string()
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else {
        "non-textual panic payload".to_string()
    }
}

// ── Run control ──────────────────────────────────────────────────────────────

/// Names of the ten phases, indexed by phase number minus one.
pub const PHASE_NAMES: [&str; 10] = [
    "Lexical Ingestion",
    "CST Reduction & BP AST Encoding",
    "Symbol Table & Type Hierarchy",
    "Control Flow & Dominators",
    "SSA Form & Data Flow Graph",
    "Call Graph & Points-To",
    "Traceability Index",
    "ROBDD Path Summaries",
    "UML Semantic Extraction",
    "SCPG Unified Binary",
];

fn phase_name(phase: u8) -> &'static str {
    PHASE_NAMES
        .get(usize::from(phase).wrapping_sub(1))
        .copied()
        .unwrap_or("unknown phase")
}

const STAGE_PENDING: u8 = 0;
const STAGE_EXPORT: u8 = 11;
const STAGE_FINISHED: u8 = 12;

/// What a run is doing, as observed through [`RunControl::stage`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "stage", rename_all = "snake_case")]
pub enum RunStage {
    /// Not started yet (source fetching and discovery happen in this stage).
    Pending,
    /// Running one phase.
    Phase {
        /// Phase number, 1 through 10.
        phase: u8,
        /// Phase name.
        name: &'static str,
    },
    /// All phases finished; diagrams are being exported.
    ExportingDiagrams,
    /// The run ended (successfully or not).
    Finished,
}

#[derive(Debug, Default)]
struct ControlState {
    cancelled: AtomicBool,
    stage: AtomicU8,
    deadline: Option<Instant>,
    budget: Option<Duration>,
}

/// Cancellation, wall-clock budget and progress of one run. Clones share state, so a
/// server can cancel or observe a run that executes on another thread. The pipeline checks
/// the control between phases and between diagram exports; a phase that is already running
/// completes (its own budgets, such as Phase 8's per-function limits, bound it).
#[derive(Debug, Clone, Default)]
pub struct RunControl {
    state: Arc<ControlState>,
}

impl RunControl {
    /// A control that is never cancelled and has no time budget.
    pub fn new() -> Self {
        Self::default()
    }

    /// A control whose run must finish within `budget` from now.
    pub fn with_time_budget(budget: Duration) -> Self {
        Self {
            state: Arc::new(ControlState {
                deadline: Instant::now().checked_add(budget),
                budget: Some(budget),
                ..ControlState::default()
            }),
        }
    }

    /// Requests cancellation; the run stops at its next checkpoint.
    pub fn cancel(&self) {
        self.state.cancelled.store(true, Ordering::SeqCst);
    }

    /// Whether cancellation was requested.
    pub fn is_cancelled(&self) -> bool {
        self.state.cancelled.load(Ordering::SeqCst)
    }

    /// The stage the run is in.
    pub fn stage(&self) -> RunStage {
        match self.state.stage.load(Ordering::SeqCst) {
            STAGE_PENDING => RunStage::Pending,
            phase @ 1..=10 => RunStage::Phase {
                phase,
                name: phase_name(phase),
            },
            STAGE_EXPORT => RunStage::ExportingDiagrams,
            _ => RunStage::Finished,
        }
    }

    /// Marks the run as ended.
    pub fn finish(&self) {
        self.enter(STAGE_FINISHED);
    }

    fn enter(&self, stage: u8) {
        self.state.stage.store(stage, Ordering::SeqCst);
    }

    /// Fails with [`PipelineError::Cancelled`] or [`PipelineError::TimedOut`] when the run
    /// must stop.
    pub fn checkpoint(&self) -> Result<(), PipelineError> {
        if self.is_cancelled() {
            return Err(PipelineError::Cancelled);
        }
        if let (Some(deadline), Some(budget)) = (self.state.deadline, self.state.budget) {
            if Instant::now() >= deadline {
                return Err(PipelineError::TimedOut {
                    budget_secs: budget.as_secs(),
                });
            }
        }
        Ok(())
    }
}

// ── Source discovery ─────────────────────────────────────────────────────────

/// Directory names that hold third-party dependencies or tool caches wherever they appear.
pub const DEPENDENCY_DIRECTORY_NAMES: [&str; 3] = ["node_modules", "bower_components", "__pycache__"];

/// Build-output and vendored-dependency directory names. They are excluded only at a
/// module root (a directory holding one of [`BUILD_DESCRIPTORS`]), so a source package
/// that happens to be called `build` or `out` is still analyzed.
pub const BUILD_OUTPUT_DIRECTORY_NAMES: [&str; 7] =
    ["target", "build", "out", "dist", "bin", "obj", "vendor"];

/// Files that mark a module root.
pub const BUILD_DESCRIPTORS: [&str; 15] = [
    "pom.xml",
    "build.gradle",
    "build.gradle.kts",
    "settings.gradle",
    "settings.gradle.kts",
    "build.xml",
    "build.sbt",
    "package.json",
    "Cargo.toml",
    "CMakeLists.txt",
    "Makefile",
    "go.mod",
    "composer.json",
    "meson.build",
    "project.clj",
];

/// Extensions of project files that mark a module root (MSBuild projects and solutions).
pub const BUILD_DESCRIPTOR_EXTENSIONS: [&str; 4] = ["csproj", "vbproj", "fsproj", "sln"];

/// A directory containing this file is a cache directory (Cache Directory Tagging
/// Specification) and is never analyzed.
pub const CACHE_DIRECTORY_TAG: &str = "CACHEDIR.TAG";

/// How [`discover_sources`] selects files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveryOptions {
    /// Size and count limits.
    pub limits: SourceLimits,
    /// Admit test sources (test source sets and the test-file conventions of each
    /// language's standard test tooling).
    pub include_tests: bool,
    /// Honor `.gitignore`, `.git/info/exclude` and `.ignore` files.
    pub respect_ignore_files: bool,
    /// Restricts analysis to these language identifiers; `None` admits every language with
    /// a registered frontend.
    pub languages: Option<Vec<&'static str>>,
}

impl Default for DiscoveryOptions {
    fn default() -> Self {
        Self {
            limits: SourceLimits::default(),
            include_tests: false,
            respect_ignore_files: true,
            languages: None,
        }
    }
}

impl DiscoveryOptions {
    fn admits(&self, language: &SourceLanguage) -> bool {
        self.languages
            .as_ref()
            .map_or(true, |ids| ids.contains(&language.id.as_str()))
    }
}

/// One admitted source file.
#[derive(Debug, Clone)]
pub struct SourceFile {
    /// Canonical absolute path handed to the pipeline.
    pub path: PathBuf,
    /// Path relative to the inventory root, with `/` separators.
    pub relative: String,
    /// File size in bytes at discovery time.
    pub bytes: u64,
    /// Language identifier from the [`language_catalog`].
    pub language: &'static str,
}

/// Why candidate entries were left out of an inventory.
#[derive(Debug, Clone, Default, Serialize)]
pub struct SkipReport {
    /// Files with an extension no frontend accepts, keyed by lower-case extension.
    pub unsupported_extensions: BTreeMap<String, usize>,
    /// Of those, files of a recognised language that has no frontend, keyed by language.
    pub languages_without_frontend: BTreeMap<String, usize>,
    /// Files of a supported language excluded by the language filter, keyed by language.
    pub filtered_languages: BTreeMap<String, usize>,
    /// Files without any extension.
    pub without_extension: usize,
    /// Test sources left out by file-name convention because tests were not included.
    pub test_files: usize,
    /// Test source directories (test source sets, `__tests__`) not descended into.
    pub test_directories: usize,
    /// Build-output, dependency and cache directories not descended into.
    pub excluded_directories: usize,
    /// Files larger than the per-file limit.
    pub oversized_files: usize,
    /// Up to 20 relative paths of oversized files.
    pub oversized_examples: Vec<String>,
    /// Entries that could not be read or resolved.
    pub unreadable_entries: usize,
    /// Symbolic links and reparse points, which are never followed.
    pub symlinks: usize,
    /// Files reached through more than one path.
    pub duplicates: usize,
}

impl SkipReport {
    fn record_unsupported(&mut self, path: &Path, recognised: Option<LangId>) {
        match path.extension().and_then(|ext| ext.to_str()) {
            Some(ext) => *self
                .unsupported_extensions
                .entry(ext.to_ascii_lowercase())
                .or_insert(0) += 1,
            None => self.without_extension += 1,
        }
        if let Some(lang) = recognised {
            *self
                .languages_without_frontend
                .entry(language_name(lang))
                .or_insert(0) += 1;
        }
    }

    /// Total number of unsupported files of any extension.
    pub fn unsupported_total(&self) -> usize {
        self.unsupported_extensions.values().sum::<usize>() + self.without_extension
    }

    /// Total number of files excluded by the language filter.
    pub fn filtered_total(&self) -> usize {
        self.filtered_languages.values().sum()
    }
}

/// The input-selection rules an inventory was built with, for run reports and responses.
#[derive(Debug, Clone, Serialize)]
pub struct InputSelection {
    /// Languages with a registered frontend and the depth of analysis each receives.
    pub languages: &'static [SourceLanguage],
    /// The language filter in force (`null`: every language above).
    pub language_filter: Option<Vec<&'static str>>,
    /// Whether test sources were admitted.
    pub include_tests: bool,
    /// Whether ignore files were honored.
    pub respect_ignore_files: bool,
    /// Directory names never descended into.
    pub dependency_directory_names: &'static [&'static str],
    /// Directory names not descended into at module roots.
    pub build_output_directory_names: &'static [&'static str],
    /// Files and extensions that mark a module root.
    pub build_descriptors: Vec<&'static str>,
    /// How test sources are recognised.
    pub test_source_rules: &'static str,
    /// Hidden files and directories are always skipped.
    pub hidden_entries_skipped: bool,
    /// Symbolic links are never followed.
    pub symlinks_followed: bool,
    /// Limits in force.
    pub limits: SourceLimits,
}

/// The test-source rules applied by discovery, for reports.
pub const TEST_SOURCE_RULES: &str = "directories directly under a 'src' directory named \
     test, tests, test<Variant> or <variant>Test (Maven/Gradle/Kotlin source sets); 'test' and \
     'tests' directories at a module root; '__tests__' directories; and the file-name \
     conventions of the language's standard test runner (Go *_test, pytest test_*/*_test, \
     Jest/Mocha *.test.*/*.spec.*)";

/// The files selected for one analysis, rooted at a canonical directory.
#[derive(Debug, Clone)]
pub struct SourceInventory {
    /// Canonical analysis root; every file lies beneath it.
    pub root: PathBuf,
    /// Admitted files sorted by relative path.
    pub files: Vec<SourceFile>,
    /// What was left out and why.
    pub skipped: SkipReport,
    /// Combined size of the admitted files.
    pub total_bytes: u64,
    /// Options the inventory was built with.
    pub options: DiscoveryOptions,
}

impl SourceInventory {
    fn empty(root: PathBuf, options: &DiscoveryOptions) -> Self {
        Self {
            root,
            files: Vec::new(),
            skipped: SkipReport::default(),
            total_bytes: 0,
            options: options.clone(),
        }
    }

    /// An inventory of an explicit file list (no directory walk): every path must be a
    /// regular file of a supported language; the root is their deepest common directory.
    /// Test-file conventions do not apply, since the caller chose the files.
    pub fn from_paths(
        paths: &[PathBuf],
        options: &DiscoveryOptions,
    ) -> Result<SourceInventory, PipelineError> {
        if paths.is_empty() {
            return Err(PipelineError::InvalidInput(
                "the explicit source file list is empty".to_string(),
            ));
        }
        let mut canonical = Vec::with_capacity(paths.len());
        for path in paths {
            let resolved =
                canonicalize_clean(path).map_err(io_error("resolve the source file", path))?;
            let meta = fs::metadata(&resolved).map_err(io_error("read metadata of", path))?;
            if !meta.is_file() {
                return Err(PipelineError::InvalidInput(format!(
                    "'{}' is not a regular file",
                    path.display()
                )));
            }
            canonical.push((resolved, meta.len()));
        }
        let root = common_directory(canonical.iter().map(|(path, _)| path.as_path()))
            .ok_or_else(|| {
                PipelineError::InvalidInput(
                    "the source files share no common directory".to_string(),
                )
            })?;

        let catalog = language_catalog();
        let mut inventory = SourceInventory::empty(root, options);
        let mut seen = HashSet::new();
        for (path, bytes) in &canonical {
            match catalog.detect(path) {
                LanguageMatch::Supported(language) if options.admits(language) => {
                    inventory.admit(path, *bytes, language, &mut seen)?
                }
                LanguageMatch::Supported(language) => {
                    *inventory
                        .skipped
                        .filtered_languages
                        .entry(language.id.clone())
                        .or_insert(0) += 1
                }
                LanguageMatch::NoFrontend(lang) => {
                    inventory.skipped.record_unsupported(path, Some(lang))
                }
                LanguageMatch::Unknown => inventory.skipped.record_unsupported(path, None),
            }
        }
        inventory.finish()
    }

    /// Absolute paths of the admitted files, in inventory order.
    pub fn paths(&self) -> Vec<PathBuf> {
        self.files.iter().map(|file| file.path.clone()).collect()
    }

    /// The selection rules this inventory was built with.
    pub fn selection(&self) -> InputSelection {
        InputSelection {
            languages: language_catalog().languages(),
            language_filter: self.options.languages.clone(),
            include_tests: self.options.include_tests,
            respect_ignore_files: self.options.respect_ignore_files,
            dependency_directory_names: &DEPENDENCY_DIRECTORY_NAMES,
            build_output_directory_names: &BUILD_OUTPUT_DIRECTORY_NAMES,
            build_descriptors: BUILD_DESCRIPTORS
                .iter()
                .copied()
                .chain(BUILD_DESCRIPTOR_EXTENSIONS.iter().copied())
                .collect(),
            test_source_rules: TEST_SOURCE_RULES,
            hidden_entries_skipped: true,
            symlinks_followed: false,
            limits: self.options.limits,
        }
    }

    /// Admitted files per language identifier.
    pub fn files_per_language(&self) -> BTreeMap<&'static str, usize> {
        let mut counts = BTreeMap::new();
        for file in &self.files {
            *counts.entry(file.language).or_insert(0) += 1;
        }
        counts
    }

    /// Maps each path as recorded in a token corpus (the manifest path, rendered lossily)
    /// to its root-relative form.
    pub fn corpus_path_index(&self) -> HashMap<String, &str> {
        self.files
            .iter()
            .map(|file| {
                (
                    file.path.to_string_lossy().into_owned(),
                    file.relative.as_str(),
                )
            })
            .collect()
    }

    /// Maps a path as recorded in the token corpus back to its root-relative form.
    pub fn relative_for_corpus_path(&self, corpus_path: &str) -> Option<&str> {
        self.files
            .iter()
            .find(|file| file.path.to_string_lossy() == corpus_path)
            .map(|file| file.relative.as_str())
    }

    fn admit(
        &mut self,
        path: &Path,
        bytes: u64,
        language: &'static SourceLanguage,
        seen: &mut HashSet<PathBuf>,
    ) -> Result<(), PipelineError> {
        let canonical = match canonicalize_clean(path) {
            Ok(canonical) => canonical,
            Err(err) => {
                self.skipped.unreadable_entries += 1;
                log_warn(&format!(
                    "Skipping '{}': cannot resolve path ({})",
                    path.display(),
                    err
                ));
                return Ok(());
            }
        };
        if !canonical.starts_with(&self.root) {
            // Reached through a reparse point that resolves outside the root.
            self.skipped.symlinks += 1;
            return Ok(());
        }
        if !seen.insert(canonical.clone()) {
            self.skipped.duplicates += 1;
            return Ok(());
        }
        let relative = relative_path_string(&self.root, &canonical);
        let limits = self.options.limits;
        if bytes > limits.max_file_bytes {
            self.skipped.oversized_files += 1;
            if self.skipped.oversized_examples.len() < 20 {
                self.skipped.oversized_examples.push(relative);
            }
            return Ok(());
        }
        let max_files = limits.max_files.min(MAX_CORPUS_FILES);
        if self.files.len() >= max_files {
            return Err(PipelineError::TooManyFiles { limit: max_files });
        }
        if self.total_bytes.saturating_add(bytes) > limits.max_total_bytes {
            return Err(PipelineError::SourceTooLarge {
                limit_bytes: limits.max_total_bytes,
            });
        }
        self.total_bytes += bytes;
        self.files.push(SourceFile {
            path: canonical,
            relative,
            bytes,
            language: language.id.as_str(),
        });
        Ok(())
    }

    fn finish(mut self) -> Result<SourceInventory, PipelineError> {
        if self.files.is_empty() {
            return Err(PipelineError::NoSourceFiles {
                languages: language_catalog().accepted_extensions(),
            });
        }
        self.files
            .sort_by(|left, right| left.relative.cmp(&right.relative));
        Ok(self)
    }
}

/// Canonicalizes `path` and, on Windows, drops the `\\?\` verbatim prefix from ordinary
/// drive paths so canonical paths compare and display like user-supplied ones.
pub fn canonicalize_clean(path: &Path) -> io::Result<PathBuf> {
    let canonical = fs::canonicalize(path)?;
    let simplified = {
        let text = canonical.to_string_lossy();
        text.strip_prefix(r"\\?\")
            .filter(|rest| {
                let bytes = rest.as_bytes();
                bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && &bytes[1..3] == br":\"
            })
            .map(PathBuf::from)
    };
    Ok(simplified.unwrap_or(canonical))
}

/// Renders `path` relative to `root` with `/` separators.
pub fn relative_path_string(root: &Path, path: &Path) -> String {
    let relative = path.strip_prefix(root).unwrap_or(path);
    relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// The deepest directory containing every path.
fn common_directory<'a, I>(paths: I) -> Option<PathBuf>
where
    I: IntoIterator<Item = &'a Path>,
{
    let mut iter = paths.into_iter();
    let mut root = iter.next()?.parent()?.to_path_buf();
    for path in iter {
        while !path.starts_with(&root) {
            if !root.pop() {
                return None;
            }
        }
    }
    Some(root)
}

/// What discovery does with one directory.
enum DirectoryClass {
    Source,
    Excluded,
    Tests,
}

fn is_module_root(dir: &Path) -> bool {
    if BUILD_DESCRIPTORS
        .iter()
        .any(|descriptor| dir.join(descriptor).is_file())
    {
        return true;
    }
    fs::read_dir(dir)
        .map(|entries| {
            entries.flatten().any(|entry| {
                entry.file_type().is_ok_and(|kind| kind.is_file())
                    && entry
                        .path()
                        .extension()
                        .and_then(|ext| ext.to_str())
                        .is_some_and(|ext| {
                            BUILD_DESCRIPTOR_EXTENSIONS
                                .iter()
                                .any(|known| ext.eq_ignore_ascii_case(known))
                        })
            })
        })
        .unwrap_or(false)
}

/// `test`, `tests`, `test<Variant>` (testFixtures, testDebug) or `<variant>Test`
/// (androidTest, integrationTest, commonTest, jvmTest): the test source-set names of
/// Maven, Gradle, Android and Kotlin Multiplatform.
fn is_test_source_set_name(name: &str) -> bool {
    if name == "test" || name == "tests" {
        return true;
    }
    let variant_after = name
        .strip_prefix("test")
        .and_then(|rest| rest.chars().next())
        .is_some_and(|c| c.is_ascii_uppercase());
    let variant_before = name
        .strip_suffix("Test")
        .and_then(|rest| rest.chars().last())
        .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
    variant_after || variant_before
}

fn classify_directory(path: &Path, include_tests: bool) -> DirectoryClass {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return DirectoryClass::Source;
    };
    let lower = name.to_ascii_lowercase();
    if DEPENDENCY_DIRECTORY_NAMES.contains(&lower.as_str())
        || path.join(CACHE_DIRECTORY_TAG).is_file()
    {
        return DirectoryClass::Excluded;
    }
    let parent = path.parent();
    let parent_is_module_root = || parent.is_some_and(is_module_root);
    if BUILD_OUTPUT_DIRECTORY_NAMES.contains(&lower.as_str()) && parent_is_module_root() {
        return DirectoryClass::Excluded;
    }
    if include_tests {
        return DirectoryClass::Source;
    }
    if name == "__tests__" {
        return DirectoryClass::Tests;
    }
    let parent_is_src = parent
        .and_then(|dir| dir.file_name())
        .and_then(|dir| dir.to_str())
        .is_some_and(|dir| dir.eq_ignore_ascii_case("src"));
    if parent_is_src && is_test_source_set_name(name) {
        return DirectoryClass::Tests;
    }
    if (name == "test" || name == "tests") && parent_is_module_root() {
        return DirectoryClass::Tests;
    }
    DirectoryClass::Source
}

/// The file-name conventions of each language's standard test runner. Languages whose
/// tests are separated by source set (Java, Kotlin) have none.
fn is_conventional_test_file(path: &Path, language: &str) -> bool {
    let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
        return false;
    };
    match language {
        "go" => stem.ends_with("_test"),
        "python" => stem.starts_with("test_") || stem.ends_with("_test"),
        "javascript" | "typescript" => {
            stem.ends_with(".test")
                || stem.ends_with(".spec")
                || stem.contains(".test.")
                || stem.contains(".spec.")
        }
        _ => false,
    }
}

/// Selects the analyzable source files under `input` (a directory or a single file).
///
/// Files are routed to languages exactly as Phase 1 routes them, and only languages with a
/// registered frontend (and in the language filter) are admitted. Symbolic links are never
/// followed, hidden entries, dependency and cache directories and module-root build
/// outputs are skipped, ignore files are honored when requested, unreadable entries are
/// logged and skipped rather than aborting, paths are canonicalized and de-duplicated, and
/// every limit in [`SourceLimits`] is enforced.
pub fn discover_sources(
    input: &Path,
    options: &DiscoveryOptions,
) -> Result<SourceInventory, PipelineError> {
    let root = canonicalize_clean(input).map_err(io_error("resolve the source path", input))?;
    let metadata = fs::metadata(&root).map_err(io_error("read metadata of", input))?;
    if metadata.is_file() {
        return single_file_inventory(&root, metadata.len(), options);
    }
    if !metadata.is_dir() {
        return Err(PipelineError::InvalidInput(format!(
            "'{}' is neither a file nor a directory",
            input.display()
        )));
    }

    let catalog = language_catalog();
    let mut inventory = SourceInventory::empty(root.clone(), options);
    let mut seen = HashSet::new();
    let include_tests = options.include_tests;
    let excluded = Arc::new(AtomicUsize::new(0));
    let test_dirs = Arc::new(AtomicUsize::new(0));
    let (excluded_in_filter, test_dirs_in_filter) = (Arc::clone(&excluded), Arc::clone(&test_dirs));

    let mut walker = WalkBuilder::new(&root);
    walker
        .hidden(true)
        .parents(false)
        .follow_links(false)
        .git_global(false)
        .ignore(options.respect_ignore_files)
        .git_ignore(options.respect_ignore_files)
        .git_exclude(options.respect_ignore_files)
        .require_git(false)
        .filter_entry(move |entry| {
            if entry.depth() == 0 || !entry.file_type().is_some_and(|kind| kind.is_dir()) {
                return true;
            }
            match classify_directory(entry.path(), include_tests) {
                DirectoryClass::Source => true,
                DirectoryClass::Excluded => {
                    excluded_in_filter.fetch_add(1, Ordering::Relaxed);
                    false
                }
                DirectoryClass::Tests => {
                    test_dirs_in_filter.fetch_add(1, Ordering::Relaxed);
                    false
                }
            }
        });

    for result in walker.build() {
        let entry = match result {
            Ok(entry) => entry,
            Err(err) => {
                inventory.skipped.unreadable_entries += 1;
                log_warn(&format!("Skipping unreadable entry during discovery: {}", err));
                continue;
            }
        };
        if entry.depth() == 0 {
            continue;
        }
        let Some(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_symlink() {
            inventory.skipped.symlinks += 1;
            continue;
        }
        if !kind.is_file() {
            continue;
        }
        let path = entry.path();
        let language = match catalog.detect(path) {
            LanguageMatch::Supported(language) => language,
            LanguageMatch::NoFrontend(lang) => {
                inventory.skipped.record_unsupported(path, Some(lang));
                continue;
            }
            LanguageMatch::Unknown => {
                inventory.skipped.record_unsupported(path, None);
                continue;
            }
        };
        if !options.admits(language) {
            *inventory
                .skipped
                .filtered_languages
                .entry(language.id.clone())
                .or_insert(0) += 1;
            continue;
        }
        if !include_tests && is_conventional_test_file(path, &language.id) {
            inventory.skipped.test_files += 1;
            continue;
        }
        let bytes = match entry.metadata() {
            Ok(meta) => meta.len(),
            Err(err) => {
                inventory.skipped.unreadable_entries += 1;
                log_warn(&format!("Skipping '{}': {}", path.display(), err));
                continue;
            }
        };
        inventory.admit(path, bytes, language, &mut seen)?;
    }

    inventory.skipped.excluded_directories = excluded.load(Ordering::Relaxed);
    inventory.skipped.test_directories = test_dirs.load(Ordering::Relaxed);
    inventory.finish()
}

fn single_file_inventory(
    file: &Path,
    bytes: u64,
    options: &DiscoveryOptions,
) -> Result<SourceInventory, PipelineError> {
    let catalog = language_catalog();
    let language = match catalog.detect(file) {
        LanguageMatch::Supported(language) => language,
        LanguageMatch::NoFrontend(lang) => {
            return Err(PipelineError::InvalidInput(format!(
                "'{}' is {} source, for which no frontend is registered (supported: {})",
                file.display(),
                language_name(lang),
                catalog.identifiers()
            )))
        }
        LanguageMatch::Unknown => {
            return Err(PipelineError::InvalidInput(format!(
                "'{}' is not a supported source file (accepted: {})",
                file.display(),
                catalog.accepted_extensions()
            )))
        }
    };
    if !options.admits(language) {
        return Err(PipelineError::InvalidInput(format!(
            "'{}' is {} source, which the language filter excludes",
            file.display(),
            language.id
        )));
    }
    let root = file.parent().map(Path::to_path_buf).unwrap_or_default();
    let mut inventory = SourceInventory::empty(root, options);
    let mut seen = HashSet::new();
    inventory.admit(file, bytes, language, &mut seen)?;
    if inventory.files.is_empty() {
        return Err(PipelineError::InvalidInput(format!(
            "'{}' exceeds the per-file size limit of {} bytes",
            file.display(),
            options.limits.max_file_bytes
        )));
    }
    Ok(inventory)
}

// ── Pipeline execution ───────────────────────────────────────────────────────

/// What a run exports and the budgets it runs under.
#[derive(Debug, Clone)]
pub struct PipelineOptions {
    /// Canonical diagram identifiers to export.
    pub diagram_types: Vec<&'static str>,
    /// Formats to export each diagram in. A pair the exporter does not produce is
    /// reported as a gap, never fabricated.
    pub formats: Vec<DiagramFormat>,
    /// View scoping and size budgets of the exported diagrams.
    pub views: ViewOptions,
    /// Phase 8 node, time and thread budgets.
    pub phase8: Phase8Config,
    /// Worker threads for the parallel phases (1, 2 and 8); `None` keeps each phase's
    /// default (`OPENHEART_NUM_THREADS` for Phases 1-2).
    pub worker_threads: Option<usize>,
    /// Cancellation, time budget and progress.
    pub control: RunControl,
}

impl Default for PipelineOptions {
    fn default() -> Self {
        Self {
            diagram_types: all_diagram_types(),
            formats: DEFAULT_DIAGRAM_FORMATS.to_vec(),
            views: ViewOptions::default(),
            phase8: Phase8Config::default(),
            worker_threads: None,
            control: RunControl::new(),
        }
    }
}

/// File names of the ten binary artifacts inside a run directory, in phase order.
pub const ARTIFACT_FILE_NAMES: [&str; 10] = [
    "corpus.tca",
    "ast.bpa",
    "symbols.sta",
    "cfg.cfa",
    "ssa.ssa",
    "callgraph.cga",
    "traceability.tra",
    "paths.psa",
    "metadata.uma",
    "unified.scpg",
];

/// Directory of the exported diagrams inside a run directory.
pub const DIAGRAM_DIR: &str = "diagrams";

/// Paths of the ten binary artifacts inside one run directory.
#[derive(Debug, Clone)]
pub struct ArtifactPaths {
    /// Phase 1 token corpus.
    pub tca: PathBuf,
    /// Phase 2 balanced-parentheses AST.
    pub bpa: PathBuf,
    /// Phase 3 symbol table.
    pub sta: PathBuf,
    /// Phase 4 control-flow graphs.
    pub cfa: PathBuf,
    /// Phase 5 SSA form.
    pub ssa: PathBuf,
    /// Phase 6 call graph.
    pub cga: PathBuf,
    /// Phase 7 traceability index.
    pub tra: PathBuf,
    /// Phase 8 ROBDD path summaries.
    pub psa: PathBuf,
    /// Phase 9 UML metadata.
    pub uma: PathBuf,
    /// Phase 10 unified SCPG binary.
    pub scpg: PathBuf,
}

impl ArtifactPaths {
    /// Standard artifact file names ([`ARTIFACT_FILE_NAMES`]) inside `dir`.
    pub fn in_dir(dir: &Path) -> Self {
        let [tca, bpa, sta, cfa, ssa, cga, tra, psa, uma, scpg] =
            ARTIFACT_FILE_NAMES.map(|name| dir.join(name));
        Self {
            tca,
            bpa,
            sta,
            cfa,
            ssa,
            cga,
            tra,
            psa,
            uma,
            scpg,
        }
    }

    /// All ten artifact paths in phase order.
    pub fn all(&self) -> [&Path; 10] {
        [
            &self.tca, &self.bpa, &self.sta, &self.cfa, &self.ssa, &self.cga, &self.tra,
            &self.psa, &self.uma, &self.scpg,
        ]
    }
}

/// In-memory results of all ten phases.
pub struct AnalysisArtifacts {
    /// Phase 1 token corpus.
    pub tca: TokenCorpusArtifact,
    /// Phase 2 balanced-parentheses AST.
    pub bpa: BPASTArtifact,
    /// Phase 3 symbol table.
    pub sta: SymbolTableArtifact,
    /// Phase 4 control-flow graphs.
    pub cfa: CFGArtifact,
    /// Phase 5 SSA form.
    pub ssa: SSAArtifact,
    /// Phase 6 call graph.
    pub cga: CallGraphArtifact,
    /// Phase 7 traceability index.
    pub tra: TraceabilityArtifact,
    /// Phase 8 ROBDD path summaries.
    pub psa: PathSummaryArtifact,
    /// Phase 9 UML metadata.
    pub uma: UMLMetadataArtifact,
    /// Phase 10 query engine over the unified SCPG binary.
    pub engine: OpenHeartEngine,
}

/// Wall-clock duration of one executed phase.
#[derive(Debug, Clone, Serialize)]
pub struct PhaseRecord {
    /// Phase number, 1 through 10.
    pub phase: u8,
    /// Phase name.
    pub name: &'static str,
    /// Duration in milliseconds.
    pub duration_ms: f64,
}

/// One exported diagram document.
#[derive(Debug, Clone)]
pub struct DiagramOutput {
    /// Canonical diagram identifier.
    pub diagram_type: &'static str,
    /// Export format.
    pub format: DiagramFormat,
    /// File name inside the run's `diagrams/` directory.
    pub file_name: String,
    /// The exported document.
    pub content: String,
}

/// Why a requested diagram/format pair produced no file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GapKind {
    /// The exporter does not produce this diagram in this format.
    NotExported,
    /// The exporter returned an empty document.
    Empty,
    /// The exporter failed (its panic was contained).
    Failed,
}

/// A requested diagram/format pair that could not be produced, with the reason.
#[derive(Debug, Clone, Serialize)]
pub struct DiagramGap {
    /// Canonical diagram identifier.
    pub diagram_type: &'static str,
    /// Format name.
    pub format: &'static str,
    /// Classification of the gap.
    pub kind: GapKind,
    /// Why nothing was written.
    pub reason: String,
}

/// Syntax-error counts of one ingested file.
#[derive(Debug, Clone, Serialize)]
pub struct FileSyntaxErrors {
    /// Root-relative path (`null` when the corpus path cannot be mapped back).
    pub file: Option<String>,
    /// ERROR nodes in the concrete syntax tree.
    pub error_nodes: u32,
    /// MISSING nodes the parser inserted.
    pub missing_nodes: u32,
}

/// Files listed individually in [`IngestionSummary::files_with_errors`].
const MAX_LISTED_ERROR_FILES: usize = 50;

/// What Phase 1 did with each admitted file.
#[derive(Debug, Clone, Default, Serialize)]
pub struct IngestionSummary {
    /// Files that contributed tokens.
    pub files_ingested: usize,
    /// Files skipped because no grammar is linked for their language.
    pub skipped_unsupported_language: usize,
    /// Files skipped as larger than the per-file limit.
    pub skipped_too_large: usize,
    /// Files skipped because a token position cannot be encoded.
    pub skipped_unrepresentable_position: usize,
    /// Files whose parse was aborted (timeout or cancellation).
    pub skipped_parse_aborted: usize,
    /// Files skipped for any other reason.
    pub skipped_other: usize,
    /// Ingested files whose syntax tree contains ERROR or MISSING nodes.
    pub files_with_syntax_errors: usize,
    /// Total ERROR nodes across all files.
    pub syntax_error_nodes: u64,
    /// Total MISSING nodes inserted by the parser across all files.
    pub missing_nodes: u64,
    /// The files with the most ERROR and MISSING nodes (at most 50), worst first.
    pub files_with_errors: Vec<FileSyntaxErrors>,
    /// Manifest entries dropped as duplicates of an already-listed file.
    pub duplicate_paths_removed: usize,
}

impl IngestionSummary {
    fn from_report(report: &IngestionReport, inventory: &SourceInventory) -> Self {
        let mut summary = Self {
            duplicate_paths_removed: report.duplicate_paths_removed,
            ..Self::default()
        };
        let index = inventory.corpus_path_index();
        let mut with_errors = Vec::new();
        for (position, file) in report.diagnostics.iter().enumerate() {
            let reason = file.skip_reason;
            if reason == SkipReason::None {
                summary.files_ingested += 1;
            } else if reason == SkipReason::UnsupportedLanguage {
                summary.skipped_unsupported_language += 1;
            } else if reason == SkipReason::TooLarge {
                summary.skipped_too_large += 1;
            } else if reason == SkipReason::UnrepresentablePosition {
                summary.skipped_unrepresentable_position += 1;
            } else if reason == SkipReason::ParseAborted {
                summary.skipped_parse_aborted += 1;
            } else {
                summary.skipped_other += 1;
            }
            if file.error_count > 0 || file.missing_count > 0 {
                summary.files_with_syntax_errors += 1;
                let relative = report
                    .file_paths
                    .get(position)
                    .and_then(|path| index.get(&*path.to_string_lossy()))
                    .map(|relative| relative.to_string());
                with_errors.push(FileSyntaxErrors {
                    file: relative,
                    error_nodes: file.error_count,
                    missing_nodes: file.missing_count,
                });
            }
            summary.syntax_error_nodes += u64::from(file.error_count);
            summary.missing_nodes += u64::from(file.missing_count);
        }
        with_errors.sort_by(|left, right| {
            (right.error_nodes + right.missing_nodes)
                .cmp(&(left.error_nodes + left.missing_nodes))
                .then_with(|| left.file.cmp(&right.file))
        });
        with_errors.truncate(MAX_LISTED_ERROR_FILES);
        summary.files_with_errors = with_errors;
        summary
    }

    /// Number of admitted files that contributed no tokens.
    pub fn skipped_total(&self) -> usize {
        self.skipped_unsupported_language
            + self.skipped_too_large
            + self.skipped_unrepresentable_position
            + self.skipped_parse_aborted
            + self.skipped_other
    }
}

/// Counters summarizing a completed run.
#[derive(Debug, Clone, Serialize)]
pub struct AnalysisCounts {
    /// Tokens in the corpus.
    pub tokens: u64,
    /// AST nodes.
    pub ast_nodes: u64,
    /// Symbols.
    pub symbols: u64,
    /// Functions with a CFG.
    pub functions: u64,
    /// Basic blocks.
    pub basic_blocks: u64,
    /// CFG edges.
    pub cfg_edges: u64,
    /// SSA variables.
    pub ssa_variables: u64,
    /// Phi functions.
    pub phi_functions: u64,
    /// Call sites.
    pub call_sites: u64,
    /// Call-graph edges.
    pub call_edges: u64,
    /// Strongly connected components of the call graph.
    pub call_graph_sccs: u64,
    /// Traceability links.
    pub traceability_links: u64,
    /// Functions with an ROBDD path summary.
    pub robdd_functions: u64,
    /// UML classes.
    pub uml_classes: u64,
    /// UML activities.
    pub uml_activities: u64,
    /// Detected design-pattern instances.
    pub design_patterns: u64,
}

/// One traceability link with its symbol and file resolved.
#[derive(Debug, Clone, Serialize)]
pub struct TraceLink {
    /// Symbol id.
    pub sym_id: u32,
    /// Qualified name of the symbol (`null` when the symbol table has no name for it).
    pub symbol: Option<String>,
    /// Symbol kind (`null` when the symbol is unknown to the symbol table).
    pub kind: Option<String>,
    /// Root-relative source file (`null` when the link has no source location).
    pub file: Option<String>,
    /// First line (1-based).
    pub line_start: u32,
    /// First column (0-based).
    pub col_start: u16,
    /// Last line (1-based).
    pub line_end: u32,
    /// Last column (0-based).
    pub col_end: u16,
    /// SCPG hash stamped on the link.
    pub scpg_hash: String,
}

/// A completed pipeline run.
pub struct PipelineRun {
    /// In-memory phase results.
    pub artifacts: AnalysisArtifacts,
    /// Where the binary artifacts were written.
    pub paths: ArtifactPaths,
    /// Executed phases with timings, in order.
    pub phases: Vec<PhaseRecord>,
    /// Phase 1 per-file outcome counts.
    pub ingestion: IngestionSummary,
    /// Exported diagram documents.
    pub diagrams: Vec<DiagramOutput>,
    /// Requested diagrams that could not be produced.
    pub diagram_gaps: Vec<DiagramGap>,
    /// Root-relative path of each corpus file, indexed by file id (`None` when unmapped).
    pub relative_paths: Vec<Option<String>>,
    /// Total wall-clock time of phases and export.
    pub elapsed: Duration,
}

impl PipelineRun {
    /// Finds an exported diagram by identifier and format.
    pub fn diagram(&self, diagram_type: &str, format: DiagramFormat) -> Option<&DiagramOutput> {
        self.diagrams
            .iter()
            .find(|output| output.diagram_type == diagram_type && output.format == format)
    }

    /// The unified SCPG hash reported by the Phase 10 engine.
    pub fn scpg_hash(&self) -> u32 {
        self.artifacts.engine.scpg_hash()
    }

    /// The SHA-256 content digest of the unified SCPG snapshot, as hex.
    pub fn scpg_digest(&self) -> String {
        self.artifacts.engine.digest_hex()
    }

    /// Gaps caused by exporter failures or empty documents (not by unsupported pairs).
    pub fn export_failures(&self) -> usize {
        self.diagram_gaps
            .iter()
            .filter(|gap| gap.kind != GapKind::NotExported)
            .count()
    }

    /// Summary counters of the run.
    pub fn counts(&self) -> AnalysisCounts {
        let a = &self.artifacts;
        AnalysisCounts {
            tokens: a.tca.token_records.len() as u64,
            ast_nodes: a.bpa.node_count as u64,
            symbols: a.sta.symbol_count as u64,
            functions: a.cfa.function_count as u64,
            basic_blocks: a.cfa.total_blocks as u64,
            cfg_edges: a.cfa.total_edges as u64,
            ssa_variables: a.ssa.total_ssa_vars as u64,
            phi_functions: a.ssa.total_phi_funcs as u64,
            call_sites: a.cga.call_site_count as u64,
            call_edges: a.cga.call_edge_count as u64,
            call_graph_sccs: a.cga.sccs.len() as u64,
            traceability_links: a.tra.uml_links.len() as u64,
            robdd_functions: a.psa.function_count() as u64,
            uml_classes: a.uma.classes.len() as u64,
            uml_activities: a.uma.activities.len() as u64,
            design_patterns: a.uma.design_patterns.len() as u64,
        }
    }

    /// Every traceability link with its symbol name, kind and root-relative file.
    pub fn traceability(&self) -> Vec<TraceLink> {
        let a = &self.artifacts;
        let ctx = DiagramContext::new(&a.uma, &a.sta, &a.tca);
        a.tra
            .uml_links
            .iter()
            .map(|link| TraceLink {
                sym_id: link.sym_id,
                symbol: ctx.qualified_name(link.sym_id),
                kind: ctx.kind_of(link.sym_id).map(|kind| {
                    let name = format!("{:?}", kind);
                    name.strip_prefix("SK_").unwrap_or(&name).to_ascii_lowercase()
                }),
                file: if link.has_source() {
                    self.relative_paths
                        .get(usize::from(link.file_id))
                        .cloned()
                        .flatten()
                } else {
                    None
                },
                line_start: link.line_start,
                col_start: link.col_start,
                line_end: link.line_end,
                col_end: link.col_end,
                scpg_hash: format!("0x{:08X}", link.scpg_hash),
            })
            .collect()
    }

    /// Machine-readable description of the run: inputs, selection rules, phase timings,
    /// counters, artifacts and diagrams.
    pub fn report(&self, inventory: &SourceInventory) -> RunReport {
        RunReport {
            openheart_version: env!("CARGO_PKG_VERSION"),
            generated_unix_secs: report_timestamp(),
            input: InputReport {
                files: inventory.files.len(),
                total_bytes: inventory.total_bytes,
                files_per_language: inventory.files_per_language(),
                selection: inventory.selection(),
                skipped: inventory.skipped.clone(),
            },
            phases: self.phases.clone(),
            phases_executed: self.phases.len(),
            ingestion: self.ingestion.clone(),
            counts: self.counts(),
            scpg_hash: format!("0x{:08X}", self.scpg_hash()),
            scpg_digest: self.scpg_digest(),
            artifacts: self
                .paths
                .all()
                .iter()
                .map(|path| ArtifactEntry {
                    file: path
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    bytes: fs::metadata(path).ok().map(|meta| meta.len()),
                })
                .collect(),
            diagrams: self
                .diagrams
                .iter()
                .map(|output| DiagramEntry {
                    diagram_type: output.diagram_type,
                    format: output.format.as_str(),
                    file: format!("{}/{}", DIAGRAM_DIR, output.file_name),
                })
                .collect(),
            unavailable_diagrams: self.diagram_gaps.clone(),
            elapsed_ms: duration_ms(self.elapsed),
        }
    }

    /// Writes [`Self::report`] as `run.json` into `dir` and returns its path.
    pub fn write_report(
        &self,
        inventory: &SourceInventory,
        dir: &Path,
    ) -> Result<PathBuf, PipelineError> {
        let path = dir.join(RUN_REPORT_FILE);
        let json = serde_json::to_vec_pretty(&self.report(inventory)).map_err(|err| {
            PipelineError::Io {
                action: "serialize the run report",
                path: path.display().to_string(),
                source: io::Error::new(io::ErrorKind::InvalidData, err),
            }
        })?;
        fs::write(&path, json).map_err(io_error("write the run report", &path))?;
        Ok(path)
    }
}

/// Report timestamp: `SOURCE_DATE_EPOCH` when set (reproducible reports), else now.
fn report_timestamp() -> u64 {
    if let Ok(Some(epoch)) = env::number::<u64>("SOURCE_DATE_EPOCH") {
        return epoch;
    }
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

/// File name of the machine-readable run report.
pub const RUN_REPORT_FILE: &str = "run.json";

/// Serialized form of a run, written as `run.json`.
#[derive(Debug, Serialize)]
pub struct RunReport {
    /// Engine version that produced the run.
    pub openheart_version: &'static str,
    /// Report creation time (`SOURCE_DATE_EPOCH` when set).
    pub generated_unix_secs: u64,
    /// Input selection.
    pub input: InputReport,
    /// Executed phases with timings.
    pub phases: Vec<PhaseRecord>,
    /// Number of phases that ran (10 for a complete run).
    pub phases_executed: usize,
    /// Phase 1 per-file outcome counts.
    pub ingestion: IngestionSummary,
    /// Summary counters.
    pub counts: AnalysisCounts,
    /// Unified SCPG hash.
    pub scpg_hash: String,
    /// SHA-256 content digest of the unified SCPG snapshot.
    pub scpg_digest: String,
    /// Binary artifacts and their sizes (`null` when a size could not be read).
    pub artifacts: Vec<ArtifactEntry>,
    /// Exported diagram files.
    pub diagrams: Vec<DiagramEntry>,
    /// Requested diagrams that could not be produced.
    pub unavailable_diagrams: Vec<DiagramGap>,
    /// Total wall-clock time in milliseconds.
    pub elapsed_ms: f64,
}

/// Input section of a [`RunReport`].
#[derive(Debug, Serialize)]
pub struct InputReport {
    /// Number of analyzed files.
    pub files: usize,
    /// Combined size of the analyzed files.
    pub total_bytes: u64,
    /// Analyzed files per language.
    pub files_per_language: BTreeMap<&'static str, usize>,
    /// Selection rules.
    pub selection: InputSelection,
    /// Skipped entries.
    pub skipped: SkipReport,
}

/// One binary artifact in a [`RunReport`].
#[derive(Debug, Serialize)]
pub struct ArtifactEntry {
    /// Artifact file name.
    pub file: String,
    /// Size in bytes, or `null` if it could not be determined.
    pub bytes: Option<u64>,
}

/// One diagram file in a [`RunReport`].
#[derive(Debug, Serialize)]
pub struct DiagramEntry {
    /// Canonical diagram identifier.
    pub diagram_type: &'static str,
    /// Format name.
    pub format: &'static str,
    /// Path relative to the run directory.
    pub file: String,
}

fn duration_ms(duration: Duration) -> f64 {
    (duration.as_secs_f64() * 1_000_000.0).round() / 1000.0
}

/// Times phases, converts their errors and panics into [`PipelineError`]s, and checks the
/// run control before each phase.
struct PhaseClock<'a> {
    control: &'a RunControl,
    records: Vec<PhaseRecord>,
}

impl<'a> PhaseClock<'a> {
    fn new(control: &'a RunControl) -> Self {
        Self {
            control,
            records: Vec::with_capacity(PHASE_NAMES.len()),
        }
    }

    fn run<T, F>(&mut self, phase: u8, body: F) -> Result<T, PipelineError>
    where
        F: FnOnce() -> Result<T, String>,
    {
        self.control.checkpoint()?;
        self.control.enter(phase);
        let name = phase_name(phase);
        let started = Instant::now();
        let outcome = panic::catch_unwind(AssertUnwindSafe(body));
        let elapsed = started.elapsed();
        match outcome {
            Ok(Ok(value)) => {
                log_info(&format!(
                    "Phase {} ({}) completed in {:.2} ms",
                    phase,
                    name,
                    elapsed.as_secs_f64() * 1000.0
                ));
                self.records.push(PhaseRecord {
                    phase,
                    name,
                    duration_ms: duration_ms(elapsed),
                });
                Ok(value)
            }
            Ok(Err(message)) => Err(PipelineError::Phase {
                phase,
                name,
                message,
            }),
            Err(payload) => Err(PipelineError::PhasePanicked {
                phase,
                name,
                message: panic_message(payload.as_ref()),
            }),
        }
    }
}

fn read_artifact(path: &Path) -> Result<Vec<u8>, String> {
    fs::read(path).map_err(|err| format!("cannot read back '{}': {}", path.display(), err))
}

fn write_artifact(path: &Path, bytes: &[u8]) -> Result<(), String> {
    fs::write(path, bytes).map_err(|err| format!("cannot write '{}': {}", path.display(), err))
}

/// Root-relative path of every corpus file, indexed by file id.
fn corpus_relative_paths(inventory: &SourceInventory, tca: &TokenCorpusArtifact) -> Vec<Option<String>> {
    let index = inventory.corpus_path_index();
    (0..tca.file_records.len())
        .map(|position| {
            u16::try_from(position)
                .ok()
                .and_then(|file_id| tca.file_path(file_id))
                .and_then(|path| index.get(path))
                .map(|relative| relative.to_string())
        })
        .collect()
}

/// Runs Phases 1-10 over `inventory`, writing artifacts and diagrams into `work_dir`.
///
/// `work_dir` must be private to this run (see [`WorkDir`]); artifacts are handed between
/// phases in memory and written once each. The run stops with
/// [`PipelineError::Cancelled`] or [`PipelineError::TimedOut`] at the first checkpoint
/// after `options.control` asks it to.
pub fn run_pipeline(
    inventory: &SourceInventory,
    work_dir: &Path,
    options: &PipelineOptions,
) -> Result<PipelineRun, PipelineError> {
    let result = execute_phases(inventory, work_dir, options);
    options.control.finish();
    result
}

fn execute_phases(
    inventory: &SourceInventory,
    work_dir: &Path,
    options: &PipelineOptions,
) -> Result<PipelineRun, PipelineError> {
    let started = Instant::now();
    let paths = ArtifactPaths::in_dir(work_dir);
    let mut clock = PhaseClock::new(&options.control);

    let mut manifest = SourceManifest::new(inventory.paths());
    manifest.max_file_size_bytes = inventory.options.limits.max_file_bytes;
    if let Some(threads) = options.worker_threads {
        manifest.worker_threads = threads;
    }
    let (tca, report) = clock.run(1, || IngestionStage::run_with_report(manifest, &paths.tca))?;
    let ingestion = IngestionSummary::from_report(&report, inventory);
    if ingestion.files_ingested == 0 {
        return Err(PipelineError::NothingIngested {
            selected: inventory.files.len(),
            unsupported: ingestion.skipped_unsupported_language,
        });
    }
    let relative_paths = corpus_relative_paths(inventory, &tca);
    let tca_hash = report.tca_hash();
    let tca_bytes = report.tca_bytes;

    // Phase 2 consumes the in-memory corpus and the hash of the bytes Phase 1 wrote, so the
    // corpus is neither re-read nor mapped (a mapping would also pin the file on Windows).
    let mut ast_options = ASTStageOptions::default();
    if let Some(threads) = options.worker_threads {
        ast_options.worker_threads = threads;
    }
    let (bpa, bpa_bytes) = clock.run(2, || {
        let bpa = ASTStage::run_with_options(&tca, tca_hash, &paths.bpa, ast_options)
            .map_err(|err| err.to_string())?;
        Ok((bpa, read_artifact(&paths.bpa)?))
    })?;

    let (sta, sta_bytes) = clock.run(3, || {
        let sta = Phase3Stage::run(&tca, &bpa, &tca_bytes, &bpa_bytes)
            .map_err(|err| err.to_string())?;
        let bytes = sta.serialize();
        write_artifact(&paths.sta, &bytes)?;
        Ok((sta, bytes))
    })?;

    let (cfa, cfa_bytes) = clock.run(4, || {
        let cfa =
            Phase4Stage::run_with_source(&bpa, &sta, Some(&tca), &sta_bytes, &bpa_bytes, &paths.cfa)?;
        Ok((cfa, read_artifact(&paths.cfa)?))
    })?;

    let (ssa, ssa_bytes) = clock.run(5, || {
        let ssa = Phase5Stage::run_with_source(&bpa, &sta, Some(&tca), &cfa, &cfa_bytes, &paths.ssa)?;
        Ok((ssa, read_artifact(&paths.ssa)?))
    })?;

    let cga = clock.run(6, || {
        Phase6Stage::run(&bpa, &sta, &cfa, &ssa, &ssa_bytes, &sta_bytes, &paths.cga)
    })?;

    let (tra, tra_bytes) = clock.run(7, || {
        let tra = Phase7Stage::run(&tca, &bpa, &sta, &cfa, &ssa, &cga, &paths.tra);
        Ok((tra, read_artifact(&paths.tra)?))
    })?;

    let mut phase8 = options.phase8.clone();
    if let Some(threads) = options.worker_threads {
        phase8.worker_threads = threads;
    }
    // No branch-predicate table is produced by an earlier phase yet, so path counts are
    // structural (the artifact says so) rather than filtered by branch correlation.
    let psa = clock.run(8, || {
        Phase8Stage::try_run_with(&cfa, &ssa, &cga, &cfa_bytes, None, &phase8, &paths.psa)
            .map_err(|err| err.to_string())
    })?;

    let phase9 = Phase9Options {
        include_tests: inventory.options.include_tests,
        ..Phase9Options::default()
    };
    let uma = clock.run(9, || {
        Ok(Phase9Stage::run_with_options(
            &tca, &bpa, &sta, &cfa, &ssa, &cga, &tra, &psa, &tra_bytes, &paths.uma, &phase9,
        ))
    })?;

    let engine = clock.run(10, || {
        Phase10Stage::try_run(
            &tca,
            &bpa,
            &sta,
            &cfa,
            &ssa,
            &cga,
            &tra,
            &uma,
            &psa,
            &paths.scpg,
        )
        .map_err(|err| err.to_string())
    })?;

    let artifacts = AnalysisArtifacts {
        tca,
        bpa,
        sta,
        cfa,
        ssa,
        cga,
        tra,
        psa,
        uma,
        engine,
    };
    let (diagrams, diagram_gaps) = export_diagrams(&artifacts, work_dir, options)?;

    Ok(PipelineRun {
        artifacts,
        paths,
        phases: clock.records,
        ingestion,
        diagrams,
        diagram_gaps,
        relative_paths,
        elapsed: started.elapsed(),
    })
}

fn export_diagrams(
    artifacts: &AnalysisArtifacts,
    work_dir: &Path,
    options: &PipelineOptions,
) -> Result<(Vec<DiagramOutput>, Vec<DiagramGap>), PipelineError> {
    let control = &options.control;
    control.checkpoint()?;
    control.enter(STAGE_EXPORT);
    let diagram_dir = work_dir.join(DIAGRAM_DIR);
    fs::create_dir_all(&diagram_dir)
        .map_err(io_error("create the diagram directory", &diagram_dir))?;

    let mut outputs = Vec::new();
    let mut gaps = Vec::new();
    let gap = |diagram_type: &'static str, format: DiagramFormat, kind: GapKind, reason: String| {
        DiagramGap {
            diagram_type,
            format: format.as_str(),
            kind,
            reason,
        }
    };

    // One context (and its symbol, class, file and per-function indexes) serves every
    // diagram and format of the run.
    let a = artifacts;
    let context = panic::catch_unwind(AssertUnwindSafe(|| {
        DiagramContext::new(&a.uma, &a.sta, &a.tca)
            .with_bpa(&a.bpa)
            .with_cfa(&a.cfa)
            .with_ssa(&a.ssa)
            .with_cga(&a.cga)
            .with_psa(&a.psa)
            .with_tra(&a.tra)
            .with_options(options.views.clone())
    }));
    let context = match context {
        Ok(context) => context,
        Err(payload) => {
            let message = panic_message(payload.as_ref());
            log_warn(&format!("Diagram context construction failed: {}", message));
            for &diagram_type in &options.diagram_types {
                for &format in &options.formats {
                    gaps.push(gap(
                        diagram_type,
                        format,
                        GapKind::Failed,
                        format!("the diagram context could not be built: {}", message),
                    ));
                }
            }
            return Ok((outputs, gaps));
        }
    };

    let engine = UniversalDiagramEngine::new();
    for &diagram_type in &options.diagram_types {
        for &format in &options.formats {
            control.checkpoint()?;
            let rendered = panic::catch_unwind(AssertUnwindSafe(|| {
                engine.export_diagram_with_context(format, diagram_type, &context)
            }));
            match rendered {
                Ok(Some(content)) if content.trim().is_empty() => gaps.push(gap(
                    diagram_type,
                    format,
                    GapKind::Empty,
                    "the exporter produced an empty document".to_string(),
                )),
                Ok(Some(content)) => {
                    let file_name = format!("{}.{}", diagram_type, format.file_extension());
                    let path = diagram_dir.join(&file_name);
                    fs::write(&path, content.as_bytes())
                        .map_err(io_error("write the diagram file", &path))?;
                    outputs.push(DiagramOutput {
                        diagram_type,
                        format,
                        file_name,
                        content,
                    });
                }
                Ok(None) => {
                    log_debug(&format!(
                        "No {} exporter output for the {} diagram",
                        format.as_str(),
                        diagram_type
                    ));
                    gaps.push(gap(
                        diagram_type,
                        format,
                        GapKind::NotExported,
                        "the exporter does not produce this diagram in this format".to_string(),
                    ))
                }
                Err(payload) => {
                    let message = panic_message(payload.as_ref());
                    log_warn(&format!(
                        "Diagram exporter for {} ({}) panicked: {}",
                        diagram_type,
                        format.as_str(),
                        message
                    ));
                    gaps.push(gap(
                        diagram_type,
                        format,
                        GapKind::Failed,
                        format!("the exporter failed: {}", message),
                    ));
                }
            }
        }
    }
    Ok((outputs, gaps))
}

/// Runs `job` on a dedicated thread with `stack_bytes` of stack, returning its result or a
/// description of the panic that ended it.
///
/// Recursive CFG/SSA/call-graph walks scale with method size, and a stack overflow aborts
/// the whole process rather than unwinding, so pipelines never run on a default-sized
/// stack.
pub fn run_isolated<T, F>(name: &str, stack_bytes: usize, job: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let handle = thread::Builder::new()
        .name(name.to_string())
        .stack_size(stack_bytes)
        .spawn(job)
        .map_err(|err| format!("cannot start the analysis thread: {}", err))?;
    handle
        .join()
        .map_err(|payload| format!("the analysis thread panicked: {}", panic_message(&*payload)))
}

// ── Work directories ─────────────────────────────────────────────────────────

/// A private, uniquely named directory that one run writes into. It is removed on drop
/// unless [`WorkDir::keep`] or [`WorkDir::publish_into`] consumed it.
pub struct WorkDir {
    path: PathBuf,
    armed: bool,
}

/// What [`WorkDir::publish_run_into`] changed in the destination.
#[derive(Debug, Default)]
pub struct Publication {
    /// Files moved into the destination.
    pub published: Vec<PathBuf>,
    /// Diagram files of an earlier run that this run did not produce, now removed.
    pub removed_stale: Vec<PathBuf>,
}

impl WorkDir {
    /// Creates `<parent>/<prefix><unique>`; creation fails rather than reusing an existing
    /// entry, so a planted file or symlink is never written through.
    pub fn create_in(parent: &Path, prefix: &str) -> Result<Self, PipelineError> {
        fs::create_dir_all(parent).map_err(io_error("create the output directory", parent))?;
        let mut last_error = None;
        for _ in 0..16 {
            let path = parent.join(format!("{}{}", prefix, unique_suffix()));
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self { path, armed: true }),
                Err(err) if err.kind() == io::ErrorKind::AlreadyExists => last_error = Some(err),
                Err(err) => return Err(io_error("create a work directory in", parent)(err)),
            }
        }
        Err(io_error("create a unique work directory in", parent)(
            last_error.unwrap_or_else(|| io::Error::other("no unique name available")),
        ))
    }

    /// The directory path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Disarms cleanup and returns the path; the caller now owns the directory.
    pub fn keep(mut self) -> PathBuf {
        self.armed = false;
        std::mem::take(&mut self.path)
    }

    /// Moves every file of the work directory into `dest` (same relative layout) with
    /// per-file renames, then removes the work directory. Returns the published files.
    pub fn publish_into(mut self, dest: &Path) -> Result<Vec<PathBuf>, PipelineError> {
        let mut published = Vec::new();
        move_tree(&self.path, dest, &mut published)?;
        self.armed = false;
        fs::remove_dir_all(&self.path)
            .map_err(io_error("remove the work directory", &self.path))?;
        Ok(published)
    }

    /// [`Self::publish_into`] for a complete run directory: afterwards, diagram files in
    /// `dest/diagrams` that carry a canonical diagram file name but were not produced by
    /// this run are removed, so the destination never mixes outputs of two runs.
    pub fn publish_run_into(self, dest: &Path) -> Result<Publication, PipelineError> {
        let published = self.publish_into(dest)?;
        let produced: HashSet<&Path> = published.iter().map(PathBuf::as_path).collect();
        let mut removed_stale = Vec::new();
        let diagram_dir = dest.join(DIAGRAM_DIR);
        let entries = match fs::read_dir(&diagram_dir) {
            Ok(entries) => entries,
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                return Ok(Publication {
                    published,
                    removed_stale,
                })
            }
            Err(err) => return Err(io_error("list", &diagram_dir)(err)),
        };
        for entry in entries {
            let entry = entry.map_err(io_error("list", &diagram_dir))?;
            let path = entry.path();
            let is_file = entry.file_type().is_ok_and(|kind| kind.is_file());
            if !is_file || produced.contains(path.as_path()) || !is_diagram_file_name(&path) {
                continue;
            }
            fs::remove_file(&path).map_err(io_error("remove the stale diagram file", &path))?;
            removed_stale.push(path);
        }
        Ok(Publication {
            published,
            removed_stale,
        })
    }
}

/// Whether `path` is named `<canonical diagram id>.<registered format extension>`.
fn is_diagram_file_name(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let Some((stem, extension)) = name.rsplit_once('.') else {
        return false;
    };
    canonical_diagram_type(stem) == Some(stem)
        && diagram_formats()
            .iter()
            .any(|format| format.file_extension() == extension)
}

impl Drop for WorkDir {
    fn drop(&mut self) {
        if self.armed {
            if let Err(err) = fs::remove_dir_all(&self.path) {
                log_warn(&format!(
                    "Could not remove work directory '{}': {}",
                    self.path.display(),
                    err
                ));
            }
        }
    }
}

fn move_tree(src: &Path, dest: &Path, published: &mut Vec<PathBuf>) -> Result<(), PipelineError> {
    fs::create_dir_all(dest).map_err(io_error("create", dest))?;
    for entry in fs::read_dir(src).map_err(io_error("list", src))? {
        let entry = entry.map_err(io_error("list", src))?;
        let from = entry.path();
        let to = dest.join(entry.file_name());
        let kind = entry.file_type().map_err(io_error("inspect", &from))?;
        if kind.is_dir() {
            move_tree(&from, &to, published)?;
        } else {
            fs::rename(&from, &to).map_err(io_error("publish", &to))?;
            published.push(to);
        }
    }
    Ok(())
}

/// A process-unique, time-ordered suffix for directory names.
pub fn unique_suffix() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    format!(
        "{}-{:x}-{}",
        std::process::id(),
        nanos,
        COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}
