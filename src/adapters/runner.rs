//! Shared analysis runner: the single orchestration of source discovery, the 10-phase SCPG
//! pipeline and diagram export used by both `openheart analyze` and the HTTP server.
//!
//! Every run writes into a private, freshly created work directory, so two runs never
//! truncate artifacts the other has memory-mapped. Callers either keep that directory
//! (server sessions) or publish its files into a destination with per-file atomic renames
//! (CLI output directories). Phases run behind panic isolation and report typed errors;
//! [`run_isolated`] additionally moves a whole run onto a thread with an explicit stack
//! budget, because several CFG/SSA walks recurse proportionally to method size.

use std::any::Any;
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ignore::{DirEntry, WalkBuilder};
use serde::Serialize;
use thiserror::Error;

use crate::adapters::env;
use crate::ast::{ASTStage, ASTStageInput, BPASTArtifact};
use crate::cfg::serializer::CFGArtifact;
use crate::cfg::Phase4Stage;
use crate::cg::Phase6Stage;
use crate::core::io::mmap::MemoryMappedFile;
use crate::core::logger::{log_info, log_warn};
use crate::core::types::cg::CallGraphArtifact;
use crate::core::types::source::SkipReason;
use crate::ingestion::manifest::SourceManifest;
use crate::ingestion::{IngestionReport, IngestionStage, TokenCorpusArtifact};
use crate::psa::{PathSummaryArtifact, Phase8Stage};
use crate::scpg::diagram::{DiagramFormat, UniversalDiagramEngine};
use crate::scpg::{OpenHeartEngine, Phase10Stage};
use crate::ssa::{Phase5Stage, SSAArtifact};
use crate::symbol::{Phase3Stage, SymbolTableArtifact};
use crate::tra::{Phase7Stage, TraceabilityArtifact};
use crate::uma::{Phase9Stage, UMLMetadataArtifact};

// ── Languages, diagram catalog and limits ────────────────────────────────────

/// A source language accepted by the pipeline and the file extensions routed to it.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct SourceLanguage {
    /// Stable language identifier.
    pub id: &'static str,
    /// Lower-case file extensions (without the dot) routed to this language's frontend.
    pub extensions: &'static [&'static str],
    /// Maturity of the frontend: `"supported"` or `"experimental"`.
    pub status: &'static str,
}

/// Languages the pipeline ingests. Files with any other extension are skipped and counted
/// in the [`SkipReport`] instead of being re-parsed with an unrelated grammar.
pub static SOURCE_LANGUAGES: [SourceLanguage; 2] = [
    SourceLanguage {
        id: "java",
        extensions: &["java"],
        status: "supported",
    },
    SourceLanguage {
        id: "kotlin",
        extensions: &["kt", "kts"],
        status: "experimental",
    },
];

/// The 14 UML 2.5 projections, by canonical diagram identifier.
pub const UML_DIAGRAM_TYPES: [&str; 14] = [
    "class",
    "object",
    "component",
    "deployment",
    "package",
    "composite",
    "profile",
    "usecase",
    "activity",
    "statemachine",
    "sequence",
    "communication",
    "interaction",
    "timing",
];

/// The 5 program-analysis graph projections, by canonical diagram identifier.
pub const ANALYSIS_GRAPH_TYPES: [&str; 5] = ["cfg", "robdd", "dfg", "cdg", "callgraph"];

/// Formats exported when the caller does not choose any.
pub const DEFAULT_DIAGRAM_FORMATS: [DiagramFormat; 3] = [
    DiagramFormat::PlantUML,
    DiagramFormat::Mermaid,
    DiagramFormat::JSON,
];

/// Hard ceiling imposed by the token corpus format (16-bit file ids), as Phase 1 enforces it.
pub const MAX_CORPUS_FILES: usize = crate::ingestion::MAX_CORPUS_FILES;

/// Default stack reserved for an isolated pipeline thread (see [`run_isolated`]).
pub const DEFAULT_PIPELINE_STACK_BYTES: usize = 256 * 1024 * 1024;

/// Directory names never descended into during discovery (compared case-insensitively).
pub const EXCLUDED_DIRECTORY_NAMES: [&str; 8] = [
    "target",
    "build",
    "out",
    "dist",
    "node_modules",
    "vendor",
    "venv",
    "generated",
];

/// Directory names treated as test trees unless tests are explicitly included.
const TEST_DIRECTORY_NAMES: [&str; 10] = [
    "test",
    "tests",
    "__tests__",
    "spec",
    "specs",
    "androidtest",
    "testdebug",
    "testrelease",
    "integrationtest",
    "testfixtures",
];

/// Every canonical diagram identifier: the UML projections followed by the analysis graphs.
pub fn all_diagram_types() -> Vec<&'static str> {
    UML_DIAGRAM_TYPES
        .iter()
        .chain(ANALYSIS_GRAPH_TYPES.iter())
        .copied()
        .collect()
}

/// Maps a diagram name or one of its historical aliases (`use_case`, `state`, ...) to its
/// canonical identifier, or `None` when the name is unknown.
pub fn canonical_diagram_type(name: &str) -> Option<&'static str> {
    let lower = name.trim().to_ascii_lowercase();
    let wanted = match lower.as_str() {
        "use_case" | "use-case" => "usecase",
        "state" | "state_machine" | "state-machine" => "statemachine",
        "composite_structure" | "composite-structure" => "composite",
        "interaction_overview" | "interaction-overview" => "interaction",
        "cg" | "call_graph" | "call-graph" => "callgraph",
        other => other,
    };
    UML_DIAGRAM_TYPES
        .iter()
        .chain(ANALYSIS_GRAPH_TYPES.iter())
        .copied()
        .find(|candidate| *candidate == wanted)
}

/// Parses a diagram format name (`puml`/`plantuml`, `mmd`/`mermaid`, `json`, `xmi`).
pub fn parse_diagram_format(name: &str) -> Option<DiagramFormat> {
    match name.trim().to_ascii_lowercase().as_str() {
        "puml" | "plantuml" => Some(DiagramFormat::PlantUML),
        "mmd" | "mermaid" => Some(DiagramFormat::Mermaid),
        "json" => Some(DiagramFormat::JSON),
        "xmi" => Some(DiagramFormat::XMI),
        _ => None,
    }
}

/// Returns the language whose extension list contains `path`'s extension (case-insensitive).
pub fn language_for_path(path: &Path) -> Option<&'static SourceLanguage> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    SOURCE_LANGUAGES
        .iter()
        .find(|language| language.extensions.contains(&extension.as_str()))
}

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

// ── Source discovery ─────────────────────────────────────────────────────────

/// How [`discover_sources`] selects files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiscoveryOptions {
    /// Size and count limits.
    pub limits: SourceLimits,
    /// Admit test sources (test directories and `*Test`/`*Tests`/`*IT` files).
    pub include_tests: bool,
    /// Honor `.gitignore`, `.git/info/exclude` and `.ignore` files.
    pub respect_ignore_files: bool,
}

impl Default for DiscoveryOptions {
    fn default() -> Self {
        Self {
            limits: SourceLimits::default(),
            include_tests: false,
            respect_ignore_files: true,
        }
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
    /// Language identifier from [`SOURCE_LANGUAGES`].
    pub language: &'static str,
}

/// Why candidate entries were left out of an inventory.
#[derive(Debug, Clone, Default, Serialize)]
pub struct SkipReport {
    /// Files with an extension no frontend accepts, keyed by lower-case extension.
    pub unsupported_extensions: BTreeMap<String, usize>,
    /// Files without any extension.
    pub without_extension: usize,
    /// Test sources left out because tests were not included.
    pub test_files: usize,
    /// Excluded directories (build output, dependencies, test trees) not descended into.
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
    fn record_unsupported(&mut self, path: &Path) {
        match path.extension().and_then(|ext| ext.to_str()) {
            Some(ext) => *self
                .unsupported_extensions
                .entry(ext.to_ascii_lowercase())
                .or_insert(0) += 1,
            None => self.without_extension += 1,
        }
    }

    /// Total number of unsupported files of any extension.
    pub fn unsupported_total(&self) -> usize {
        self.unsupported_extensions.values().sum::<usize>() + self.without_extension
    }
}

/// The input-selection rules an inventory was built with, for run reports and responses.
#[derive(Debug, Clone, Serialize)]
pub struct InputSelection {
    /// Accepted languages and extensions.
    pub languages: &'static [SourceLanguage],
    /// Whether test sources were admitted.
    pub include_tests: bool,
    /// Whether ignore files were honored.
    pub respect_ignore_files: bool,
    /// Directory names never descended into.
    pub excluded_directory_names: &'static [&'static str],
    /// Hidden files and directories are always skipped.
    pub hidden_entries_skipped: bool,
    /// Symbolic links are never followed.
    pub symlinks_followed: bool,
    /// Limits in force.
    pub limits: SourceLimits,
}

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
    fn empty(root: PathBuf, options: DiscoveryOptions) -> Self {
        Self {
            root,
            files: Vec::new(),
            skipped: SkipReport::default(),
            total_bytes: 0,
            options,
        }
    }

    /// Absolute paths of the admitted files, in inventory order.
    pub fn paths(&self) -> Vec<PathBuf> {
        self.files.iter().map(|file| file.path.clone()).collect()
    }

    /// The selection rules this inventory was built with.
    pub fn selection(&self) -> InputSelection {
        InputSelection {
            languages: &SOURCE_LANGUAGES,
            include_tests: self.options.include_tests,
            respect_ignore_files: self.options.respect_ignore_files,
            excluded_directory_names: &EXCLUDED_DIRECTORY_NAMES,
            hidden_entries_skipped: true,
            symlinks_followed: false,
            limits: self.options.limits,
        }
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
        if self.files.len() >= limits.max_files.min(MAX_CORPUS_FILES) {
            return Err(PipelineError::TooManyFiles {
                limit: limits.max_files.min(MAX_CORPUS_FILES),
            });
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
            language: language.id,
        });
        Ok(())
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

fn accepted_extensions() -> String {
    SOURCE_LANGUAGES
        .iter()
        .flat_map(|language| language.extensions.iter())
        .map(|ext| format!(".{}", ext))
        .collect::<Vec<_>>()
        .join(", ")
}

fn is_test_directory_name(name: &str) -> bool {
    TEST_DIRECTORY_NAMES.contains(&name.to_ascii_lowercase().as_str())
}

/// Recognizes test sources by Java/Kotlin naming conventions (`FooTest`, `FooTests`,
/// `FooTestCase`, `FooIT`) and snake-case conventions (`foo_test`, `test_foo`, `foo_spec`).
fn is_test_file(path: &Path) -> bool {
    let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
        return false;
    };
    let camel_case = stem.len() > 4
        && (stem.ends_with("Test") || stem.ends_with("Tests") || stem.ends_with("TestCase"));
    let integration = stem.len() > 2
        && stem.ends_with("IT")
        && stem[..stem.len() - 2]
            .chars()
            .last()
            .is_some_and(|c| c.is_ascii_lowercase());
    let lower = stem.to_ascii_lowercase();
    let snake_case = lower.ends_with("_test")
        || lower.ends_with("_tests")
        || lower.ends_with("_spec")
        || lower.starts_with("test_")
        || lower == "test";
    camel_case || integration || snake_case
}

fn is_excluded_directory(entry: &DirEntry, include_tests: bool) -> bool {
    if !entry.file_type().is_some_and(|kind| kind.is_dir()) {
        return false;
    }
    let Some(name) = entry.file_name().to_str() else {
        return false;
    };
    let lower = name.to_ascii_lowercase();
    EXCLUDED_DIRECTORY_NAMES.contains(&lower.as_str())
        || (!include_tests && is_test_directory_name(&lower))
}

/// Selects the analyzable source files under `input` (a directory or a single file).
///
/// Symbolic links are never followed, hidden entries and build/dependency directories are
/// skipped, ignore files are honored when requested, unreadable entries are logged and
/// skipped rather than aborting, paths are canonicalized and de-duplicated, and every
/// limit in [`SourceLimits`] is enforced.
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

    let mut inventory = SourceInventory::empty(root.clone(), *options);
    let mut seen = HashSet::new();
    let include_tests = options.include_tests;
    let excluded = Arc::new(AtomicUsize::new(0));
    let excluded_in_filter = Arc::clone(&excluded);

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
            if entry.depth() > 0 && is_excluded_directory(entry, include_tests) {
                excluded_in_filter.fetch_add(1, Ordering::Relaxed);
                return false;
            }
            true
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
        let Some(language) = language_for_path(path) else {
            inventory.skipped.record_unsupported(path);
            continue;
        };
        if !include_tests && is_test_file(path) {
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
    if inventory.files.is_empty() {
        return Err(PipelineError::NoSourceFiles {
            languages: accepted_extensions(),
        });
    }
    inventory
        .files
        .sort_by(|left, right| left.relative.cmp(&right.relative));
    Ok(inventory)
}

fn single_file_inventory(
    file: &Path,
    bytes: u64,
    options: &DiscoveryOptions,
) -> Result<SourceInventory, PipelineError> {
    let language = language_for_path(file).ok_or_else(|| {
        PipelineError::InvalidInput(format!(
            "'{}' is not a supported source file (accepted: {})",
            file.display(),
            accepted_extensions()
        ))
    })?;
    let root = file.parent().map(Path::to_path_buf).unwrap_or_default();
    let mut inventory = SourceInventory::empty(root, *options);
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

/// Which diagrams a run exports and in which formats.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelineOptions {
    /// Canonical diagram identifiers to export.
    pub diagram_types: Vec<&'static str>,
    /// Formats to export each diagram in. XMI is a model-level format and is exported once,
    /// as `class.xmi`.
    pub formats: Vec<DiagramFormat>,
}

impl Default for PipelineOptions {
    fn default() -> Self {
        Self {
            diagram_types: all_diagram_types(),
            formats: DEFAULT_DIAGRAM_FORMATS.to_vec(),
        }
    }
}

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
    /// Standard artifact file names inside `dir`.
    pub fn in_dir(dir: &Path) -> Self {
        Self {
            tca: dir.join("corpus.tca"),
            bpa: dir.join("ast.bpa"),
            sta: dir.join("symbols.sta"),
            cfa: dir.join("cfg.cfa"),
            ssa: dir.join("ssa.ssa"),
            cga: dir.join("callgraph.cga"),
            tra: dir.join("traceability.tra"),
            psa: dir.join("paths.psa"),
            uma: dir.join("metadata.uma"),
            scpg: dir.join("unified.scpg"),
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

/// A requested diagram/format pair that could not be produced, with the reason.
#[derive(Debug, Clone, Serialize)]
pub struct DiagramGap {
    /// Canonical diagram identifier.
    pub diagram_type: &'static str,
    /// Format name.
    pub format: &'static str,
    /// Why nothing was written.
    pub reason: String,
}

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
    /// Manifest entries dropped as duplicates of an already-listed file.
    pub duplicate_paths_removed: usize,
}

impl IngestionSummary {
    fn from_report(report: &IngestionReport) -> Self {
        let mut summary = Self {
            duplicate_paths_removed: report.duplicate_paths_removed,
            ..Self::default()
        };
        for file in &report.diagnostics {
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
            }
            summary.syntax_error_nodes += u64::from(file.error_count);
            summary.missing_nodes += u64::from(file.missing_count);
        }
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

    /// Machine-readable description of the run: inputs, selection rules, phase timings,
    /// counters, artifacts and diagrams.
    pub fn report(&self, inventory: &SourceInventory) -> RunReport {
        RunReport {
            openheart_version: env!("CARGO_PKG_VERSION"),
            generated_unix_secs: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|elapsed| elapsed.as_secs())
                .unwrap_or(0),
            input: InputReport {
                files: inventory.files.len(),
                total_bytes: inventory.total_bytes,
                selection: inventory.selection(),
                skipped: inventory.skipped.clone(),
            },
            phases: self.phases.clone(),
            phases_executed: self.phases.len(),
            ingestion: self.ingestion.clone(),
            counts: self.counts(),
            scpg_hash: format!("0x{:08X}", self.scpg_hash()),
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
                    file: format!("diagrams/{}", output.file_name),
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

/// File name of the machine-readable run report.
pub const RUN_REPORT_FILE: &str = "run.json";

/// Serialized form of a run, written as `run.json`.
#[derive(Debug, Serialize)]
pub struct RunReport {
    /// Engine version that produced the run.
    pub openheart_version: &'static str,
    /// Report creation time.
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

/// Times phases and converts their errors and panics into [`PipelineError`]s.
#[derive(Default)]
struct PhaseClock {
    records: Vec<PhaseRecord>,
}

impl PhaseClock {
    fn run<T, F>(&mut self, phase: u8, name: &'static str, body: F) -> Result<T, PipelineError>
    where
        F: FnOnce() -> Result<T, String>,
    {
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

/// Runs Phases 1-10 over `inventory`, writing artifacts and diagrams into `work_dir`.
///
/// `work_dir` must be private to this run (see [`WorkDir`]); artifacts are handed between
/// phases in memory and written once each.
pub fn run_pipeline(
    inventory: &SourceInventory,
    work_dir: &Path,
    options: &PipelineOptions,
) -> Result<PipelineRun, PipelineError> {
    let started = Instant::now();
    let paths = ArtifactPaths::in_dir(work_dir);
    let mut clock = PhaseClock::default();

    let mut manifest = SourceManifest::new(inventory.paths());
    manifest.max_file_size_bytes = inventory.options.limits.max_file_bytes;
    let (tca, report) = clock.run(1, "Lexical Ingestion", || {
        IngestionStage::run_with_report(manifest, &paths.tca)
    })?;
    let ingestion = IngestionSummary::from_report(&report);
    let tca_bytes = report.tca_bytes;
    if ingestion.files_ingested == 0 {
        return Err(PipelineError::NothingIngested {
            selected: inventory.files.len(),
            unsupported: ingestion.skipped_unsupported_language,
        });
    }

    let (bpa, bpa_bytes) = clock.run(2, "CST Reduction & BP AST Encoding", || {
        let input = ASTStageInput {
            tca: MemoryMappedFile::open(&paths.tca)
                .map_err(|err| format!("cannot map the token corpus: {}", err))?,
        };
        let bpa = ASTStage::run(&input, &paths.bpa).map_err(|err| err.to_string())?;
        drop(input);
        Ok((bpa, read_artifact(&paths.bpa)?))
    })?;

    let (sta, sta_bytes) = clock.run(3, "Symbol Table & Type Hierarchy", || {
        let sta = Phase3Stage::run(&tca, &bpa, &tca_bytes, &bpa_bytes)
            .map_err(|err| err.to_string())?;
        let bytes = sta.serialize();
        write_artifact(&paths.sta, &bytes)?;
        Ok((sta, bytes))
    })?;

    let (cfa, cfa_bytes) = clock.run(4, "Control Flow & Dominators", || {
        let cfa = Phase4Stage::run(&bpa, &sta, &sta_bytes, &bpa_bytes, &paths.cfa)
            .map_err(|err| err.to_string())?;
        Ok((cfa, read_artifact(&paths.cfa)?))
    })?;

    let (ssa, ssa_bytes) = clock.run(5, "SSA Form & Data Flow Graph", || {
        let ssa = Phase5Stage::run(&bpa, &sta, &cfa, &cfa_bytes, &paths.ssa)
            .map_err(|err| err.to_string())?;
        Ok((ssa, read_artifact(&paths.ssa)?))
    })?;

    let cga = clock.run(6, "Call Graph & Points-To", || {
        Phase6Stage::run(&bpa, &sta, &cfa, &ssa, &ssa_bytes, &sta_bytes, &paths.cga)
            .map_err(|err| err.to_string())
    })?;

    let (tra, tra_bytes) = clock.run(7, "Traceability Index", || {
        let tra = Phase7Stage::run(&tca, &bpa, &sta, &cfa, &ssa, &cga, &paths.tra);
        Ok((tra, read_artifact(&paths.tra)?))
    })?;

    let psa = clock.run(8, "ROBDD Path Summaries", || {
        Ok(Phase8Stage::run(&cfa, &ssa, &cga, &cfa_bytes, &paths.psa))
    })?;

    let uma = clock.run(9, "UML Semantic Extraction", || {
        Ok(Phase9Stage::run(
            &tca, &bpa, &sta, &cfa, &ssa, &cga, &tra, &psa, &tra_bytes, &paths.uma,
        ))
    })?;

    let engine = clock.run(10, "SCPG Unified Binary", || {
        Ok(Phase10Stage::run(
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
        ))
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
        elapsed: started.elapsed(),
    })
}

fn export_diagrams(
    artifacts: &AnalysisArtifacts,
    work_dir: &Path,
    options: &PipelineOptions,
) -> Result<(Vec<DiagramOutput>, Vec<DiagramGap>), PipelineError> {
    let diagram_dir = work_dir.join("diagrams");
    fs::create_dir_all(&diagram_dir)
        .map_err(io_error("create the diagram directory", &diagram_dir))?;

    let engine = UniversalDiagramEngine::new();
    let mut outputs = Vec::new();
    let mut gaps = Vec::new();
    for &diagram_type in &options.diagram_types {
        for &format in &options.formats {
            if format == DiagramFormat::XMI && diagram_type != "class" {
                continue;
            }
            let rendered = panic::catch_unwind(AssertUnwindSafe(|| {
                engine.export_diagram(
                    format,
                    diagram_type,
                    &artifacts.uma,
                    &artifacts.sta,
                    &artifacts.tca,
                )
            }));
            let gap = |reason: String| DiagramGap {
                diagram_type,
                format: format.as_str(),
                reason,
            };
            match rendered {
                Ok(Some(content)) if content.trim().is_empty() => {
                    gaps.push(gap("the exporter produced an empty document".to_string()));
                }
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
                Ok(None) => gaps.push(gap(
                    "no exporter is registered for this diagram type and format".to_string(),
                )),
                Err(payload) => {
                    let message = panic_message(payload.as_ref());
                    log_warn(&format!(
                        "Diagram exporter for {} ({}) panicked: {}",
                        diagram_type,
                        format.as_str(),
                        message
                    ));
                    gaps.push(gap(format!("the exporter failed: {}", message)));
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
