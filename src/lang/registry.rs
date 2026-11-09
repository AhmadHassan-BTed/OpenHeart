//! Frontend registry: the single list of supported languages, explicit registration of
//! the language frontends, and the detection of a file's language.
//!
//! `LANGUAGES` is the one list the CLI, the server and the web studio use for language
//! ids, names, extensions and status; it is static so source discovery needs no frontend
//! instance. `FrontendRegistry` holds the frontends (Java, Kotlin, C, C++, C#) and answers
//! lookups by `LangId`, extension, shebang interpreter, editor modeline and content.
//!
//! Detection order for one file: a manifest override for its extension; a file-name
//! pattern; its extension — where the primary claimant's own rule
//! (`LanguageFrontend::resolve_detection`) decides first, and an extension claimed by
//! several languages (`.h`) is settled by an editor modeline, then syntax only one
//! candidate has, then the languages already present in the project (a superset language
//! wins, so a C++ project's headers are C++), then likely-content evidence, and finally the
//! primary claimant. Files without a claimed extension are detected by shebang, modeline or
//! definite content. `src/lang/profile.rs` owns the C-family policy and plugs in through the
//! C / C++ frontends' `resolve_detection` and `sniff`.

use std::collections::HashMap;
use std::ffi::OsString;
use std::fmt;
use std::path::Path;
use std::sync::Arc;

use thiserror::Error;

use crate::ast::adapter::registry::ASTAdapterRegistry;
use crate::ast::adapter::ASTReductionAdapter;
use crate::core::types::token::{LangId, TokenType};
use crate::ingestion::adapter::registry::AdapterRegistry;
use crate::ingestion::adapter::{LanguageAdapter, SourceLexer};
use crate::lang::frontend::{
    Capabilities, DetectionContext, ExtensionClaim, Fidelity, HeadScan, LanguageDescriptor,
    LanguageFamily, LanguageFrontend, Phase, SniffVerdict, SupportStatus,
};
use crate::lang::{c, cpp, csharp, java, kotlin, profile};
use crate::symbol::adapter::SemanticAdapter;

// ── The supported languages ──────────────────────────────────────────────────

pub static JAVA_LANGUAGE: LanguageDescriptor = LanguageDescriptor {
    lang: LangId::Java,
    id: "java",
    display_name: "Java",
    badge: "J",
    family: LanguageFamily::Jvm,
    status: SupportStatus::Supported,
    extensions: &["java"],
    secondary_extensions: &[],
    header_extensions: &[],
    filename_patterns: &[],
    shebang_interpreters: &["java"],
    editor_modes: &["java"],
    supersedes: &[],
};

pub static KOTLIN_LANGUAGE: LanguageDescriptor = LanguageDescriptor {
    lang: LangId::Kotlin,
    id: "kotlin",
    display_name: "Kotlin",
    badge: "K",
    family: LanguageFamily::Jvm,
    status: SupportStatus::Experimental,
    extensions: &["kt", "kts"],
    secondary_extensions: &[],
    header_extensions: &[],
    filename_patterns: &[],
    shebang_interpreters: &["kotlin", "kotlinc", "kscript"],
    editor_modes: &["kotlin"],
    supersedes: &[],
};

pub static C_LANGUAGE: LanguageDescriptor = LanguageDescriptor {
    lang: LangId::C,
    id: "c",
    display_name: "C",
    badge: "C",
    family: LanguageFamily::CFamily,
    status: SupportStatus::Supported,
    extensions: profile::C_EXTENSIONS,
    secondary_extensions: &[],
    header_extensions: profile::C_HEADER_EXTENSIONS,
    filename_patterns: &[],
    shebang_interpreters: &["tcc"],
    editor_modes: &["c"],
    supersedes: &[],
};

/// C++ headers, including the `.h` files C++ claims on evidence.
const CPP_HEADER_EXTENSIONS: &[&str] = &["hpp", "hh", "hxx", "h++", "ipp", "tpp", "inl", "h"];

pub static CPP_LANGUAGE: LanguageDescriptor = LanguageDescriptor {
    lang: LangId::Cpp,
    id: "cpp",
    display_name: "C++",
    badge: "C++",
    family: LanguageFamily::CFamily,
    status: SupportStatus::Supported,
    extensions: profile::CPP_EXTENSIONS,
    secondary_extensions: profile::C_HEADER_EXTENSIONS,
    header_extensions: CPP_HEADER_EXTENSIONS,
    filename_patterns: &[],
    shebang_interpreters: &["cling"],
    editor_modes: &["c++", "cpp"],
    supersedes: &[LangId::C],
};

pub static CSHARP_LANGUAGE: LanguageDescriptor = LanguageDescriptor {
    lang: LangId::CSharp,
    id: "csharp",
    display_name: "C#",
    badge: "C#",
    family: LanguageFamily::CFamily,
    status: SupportStatus::Supported,
    extensions: profile::CSHARP_EXTENSIONS,
    secondary_extensions: &[],
    header_extensions: &[],
    filename_patterns: &[],
    shebang_interpreters: &["dotnet-script", "csi", "scriptcs"],
    editor_modes: &["csharp", "cs", "c#"],
    supersedes: &[],
};

/// Every supported language, in registration order. The single list consumers use.
pub static LANGUAGES: [&LanguageDescriptor; 5] = [
    &JAVA_LANGUAGE,
    &KOTLIN_LANGUAGE,
    &C_LANGUAGE,
    &CPP_LANGUAGE,
    &CSHARP_LANGUAGE,
];

pub fn languages() -> &'static [&'static LanguageDescriptor] {
    &LANGUAGES
}

/// Language by stable id, display name or editor-mode alias (`cpp`, `C++`, `c#`), ignoring
/// case.
pub fn language_by_id(id: &str) -> Option<&'static LanguageDescriptor> {
    let id = id.trim();
    LANGUAGES.iter().copied().find(|d| {
        d.id.eq_ignore_ascii_case(id)
            || d.display_name.eq_ignore_ascii_case(id)
            || d.matches_editor_mode(id)
    })
}

pub fn language_by_lang(lang: LangId) -> Option<&'static LanguageDescriptor> {
    LANGUAGES.iter().copied().find(|d| d.lang == lang)
}

/// Languages claiming an extension: primary claimants first, then secondary ones.
pub fn languages_for_extension(ext: &str) -> Vec<&'static LanguageDescriptor> {
    let mut primary = Vec::new();
    let mut secondary = Vec::new();
    for d in LANGUAGES.iter().copied() {
        match d.claims_extension(ext) {
            Some(ExtensionClaim::Primary) => primary.push(d),
            Some(ExtensionClaim::Secondary) => secondary.push(d),
            None => {}
        }
    }
    primary.extend(secondary);
    primary
}

/// The language an extension routes to before content is considered (the primary
/// claimant, else the first secondary claimant).
pub fn language_for_extension(ext: &str) -> Option<&'static LanguageDescriptor> {
    languages_for_extension(ext).into_iter().next()
}

/// The language of a path from its file name and extension alone. Ambiguous extensions
/// route to their primary claimant; [`FrontendRegistry::detect`] refines them from content
/// and project context.
pub fn language_for_path(path: &Path) -> Option<&'static LanguageDescriptor> {
    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        if let Some(d) = LANGUAGES.iter().copied().find(|d| d.matches_file_name(name)) {
            return Some(d);
        }
    }
    let ext = path.extension()?.to_str()?;
    language_for_extension(ext)
}

/// Every accepted extension (primary and secondary), lower case, sorted, without repeats.
pub fn supported_extensions() -> Vec<&'static str> {
    let mut all: Vec<&'static str> = LANGUAGES
        .iter()
        .flat_map(|d| d.all_extensions())
        .collect();
    all.sort_unstable();
    all.dedup();
    all
}

/// Accepted extensions for messages: `.c, .cc, .cpp, ...`.
pub fn accepted_extensions_display() -> String {
    supported_extensions()
        .iter()
        .map(|ext| format!(".{}", ext))
        .collect::<Vec<_>>()
        .join(", ")
}

// ── Registry errors ──────────────────────────────────────────────────────────

/// An inconsistency between frontends, descriptors and adapters.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RegistryError {
    #[error("language id {0:?} is reserved and cannot be registered")]
    ReservedLanguage(LangId),
    #[error("a frontend for {0:?} is already registered")]
    DuplicateLanguage(LangId),
    #[error("frontend reports language {found:?} but its descriptor is for {expected:?}")]
    DescriptorMismatch { expected: LangId, found: LangId },
    #[error("extension .{extension} is claimed as primary by both {first:?} and {second:?}")]
    ExtensionConflict {
        extension: String,
        first: LangId,
        second: LangId,
    },
    #[error("the {component} of the {lang:?} frontend reports language {found:?}")]
    ComponentMismatch {
        lang: LangId,
        component: &'static str,
        found: LangId,
    },
    #[error("the {lang:?} {component} handles .{extension}, which the descriptor does not declare")]
    UndeclaredExtension {
        lang: LangId,
        component: &'static str,
        extension: String,
    },
    #[error("the {lang:?} descriptor declares .{extension}, which its Phase 1 adapter does not handle")]
    UnhandledExtension { lang: LangId, extension: String },
    #[error("the {lang:?} grammar has ABI version {version}; the linked tree-sitter accepts {min}..={max}")]
    IncompatibleGrammar {
        lang: LangId,
        version: usize,
        min: usize,
        max: usize,
    },
    #[error("the {lang:?} Phase 1 and Phase 2 adapters use different grammars")]
    GrammarMismatch { lang: LangId },
    #[error("the {lang:?} frontend claims {declared:?} fidelity for {phase:?}; its prerequisites allow {ceiling:?}")]
    OverstatedCapability {
        lang: LangId,
        phase: Phase,
        declared: Fidelity,
        ceiling: Fidelity,
    },
    #[error("the {lang:?} descriptor status is {descriptor}, the language profile reports {profile}")]
    StatusMismatch {
        lang: LangId,
        descriptor: &'static str,
        profile: &'static str,
    },
    #[error("{0:?} is registered but missing from the supported-language list")]
    UnlistedLanguage(LangId),
    #[error("{0:?} is in the supported-language list but no frontend is registered for it")]
    MissingFrontend(LangId),
}

// ── Detection results ────────────────────────────────────────────────────────

/// Which rule decided a file's language.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DetectionMethod {
    /// `SourceManifest::language_overrides` names the extension.
    Override,
    /// A descriptor's file-name pattern.
    FileName,
    /// The only language claiming the extension.
    Extension,
    /// The primary claimant's own rule (`LanguageFrontend::resolve_detection`).
    FrontendRule,
    /// An Emacs / Vim modeline.
    EditorModeline,
    /// Syntax in the file's head.
    Content,
    /// The languages already present in the project.
    ProjectContext,
    /// No evidence: the extension's primary claimant.
    PrimaryClaimant,
    /// The `#!` interpreter.
    Shebang,
}

impl DetectionMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            DetectionMethod::Override => "override",
            DetectionMethod::FileName => "file-name",
            DetectionMethod::Extension => "extension",
            DetectionMethod::FrontendRule => "frontend-rule",
            DetectionMethod::EditorModeline => "modeline",
            DetectionMethod::Content => "content",
            DetectionMethod::ProjectContext => "project-context",
            DetectionMethod::PrimaryClaimant => "primary-claimant",
            DetectionMethod::Shebang => "shebang",
        }
    }
}

/// The language of one file and how it was decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Detection {
    pub lang: LangId,
    pub method: DetectionMethod,
    /// More than one language claimed the file's extension.
    pub ambiguous: bool,
}

/// One file of a project for [`FrontendRegistry::detect_project`].
#[derive(Debug, Clone, Copy)]
pub struct ProjectFile<'a> {
    pub path: &'a Path,
    /// The file's head; `None` when it was not read (content rules are then skipped).
    pub head: Option<&'a HeadScan>,
}

/// Anonymous grammar tokens a Phase 1 adapter leaves unclassified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrammarAudit {
    pub lang: LangId,
    pub abi_version: usize,
    pub node_kind_count: usize,
    /// Anonymous tokens mapped to `TokenType::Unknown` (dropped from the corpus unless the
    /// adapter treats an enclosing kind as atomic), sorted.
    pub unclassified_tokens: Vec<&'static str>,
}

// ── The registry ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
struct Claimants {
    primary: Vec<usize>,
    secondary: Vec<usize>,
}

impl Claimants {
    fn all(&self) -> impl Iterator<Item = usize> + '_ {
        self.primary.iter().chain(self.secondary.iter()).copied()
    }
}

/// The registered language frontends.
#[derive(Clone)]
pub struct FrontendRegistry {
    frontends: Vec<Arc<dyn LanguageFrontend>>,
    by_lang: HashMap<LangId, usize>,
    by_extension: HashMap<String, Claimants>,
    /// Registration failures of the built-in frontends.
    issues: Vec<RegistryError>,
}

impl fmt::Debug for FrontendRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FrontendRegistry")
            .field(
                "frontends",
                &self.frontends.iter().map(|fe| fe.lang_id()).collect::<Vec<_>>(),
            )
            .field("issues", &self.issues)
            .finish()
    }
}

impl Default for FrontendRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl FrontendRegistry {
    /// A registry with every built-in frontend registered: Java, Kotlin, C, C++ and C#.
    /// Registration failures are kept and reported by [`FrontendRegistry::validate`].
    pub fn new() -> Self {
        let mut registry = Self::empty();
        let builtin: [Arc<dyn LanguageFrontend>; 5] = [
            Arc::new(java::JavaFrontend::new()),
            Arc::new(kotlin::KotlinFrontend::new()),
            Arc::new(c::CFrontend::new()),
            Arc::new(cpp::CppFrontend::new()),
            Arc::new(csharp::CSharpFrontend::new()),
        ];
        for frontend in builtin {
            if let Err(issue) = registry.register(frontend) {
                registry.issues.push(issue);
            }
        }
        registry
    }

    /// A registry without frontends.
    pub fn empty() -> Self {
        Self {
            frontends: Vec::new(),
            by_lang: HashMap::new(),
            by_extension: HashMap::new(),
            issues: Vec::new(),
        }
    }

    /// Registers a frontend. Fails on a reserved or duplicate language id, a frontend whose
    /// language differs from its descriptor's, or a primary extension another frontend
    /// already owns.
    pub fn register(&mut self, frontend: Arc<dyn LanguageFrontend>) -> Result<(), RegistryError> {
        let descriptor = frontend.descriptor();
        let lang = frontend.lang_id();
        if lang == LangId::Unknown {
            return Err(RegistryError::ReservedLanguage(lang));
        }
        if lang != descriptor.lang {
            return Err(RegistryError::DescriptorMismatch {
                expected: descriptor.lang,
                found: lang,
            });
        }
        if self.by_lang.contains_key(&lang) {
            return Err(RegistryError::DuplicateLanguage(lang));
        }
        for ext in descriptor.extensions {
            let key = ext.to_ascii_lowercase();
            let owner = self
                .by_extension
                .get(&key)
                .and_then(|claim| claim.primary.first());
            if let Some(&owner) = owner {
                return Err(RegistryError::ExtensionConflict {
                    extension: key,
                    first: self.frontends[owner].lang_id(),
                    second: lang,
                });
            }
        }
        let idx = self.frontends.len();
        for ext in descriptor.extensions {
            self.by_extension
                .entry(ext.to_ascii_lowercase())
                .or_default()
                .primary
                .push(idx);
        }
        for ext in descriptor.secondary_extensions {
            self.by_extension
                .entry(ext.to_ascii_lowercase())
                .or_default()
                .secondary
                .push(idx);
        }
        self.by_lang.insert(lang, idx);
        self.frontends.push(frontend);
        Ok(())
    }

    pub fn get(&self, lang: LangId) -> Option<&Arc<dyn LanguageFrontend>> {
        self.by_lang.get(&lang).map(|&i| &self.frontends[i])
    }

    pub fn contains(&self, lang: LangId) -> bool {
        self.by_lang.contains_key(&lang)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Arc<dyn LanguageFrontend>> {
        self.frontends.iter()
    }

    pub fn len(&self) -> usize {
        self.frontends.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frontends.is_empty()
    }

    /// Descriptors of the registered languages, in registration order.
    pub fn languages(&self) -> Vec<&'static LanguageDescriptor> {
        self.frontends.iter().map(|f| f.descriptor()).collect()
    }

    /// Capabilities of every registered language, for reports and the server.
    pub fn capabilities_report(&self) -> Vec<(&'static LanguageDescriptor, Capabilities)> {
        self.frontends
            .iter()
            .map(|f| (f.descriptor(), f.capabilities()))
            .collect()
    }

    /// Frontends claiming an extension: primary claimants first.
    pub fn by_extension(&self, ext: &str) -> Vec<&Arc<dyn LanguageFrontend>> {
        let key = ext.trim_start_matches('.').to_ascii_lowercase();
        match self.by_extension.get(&key) {
            Some(claimants) => claimants.all().map(|i| &self.frontends[i]).collect(),
            None => Vec::new(),
        }
    }

    pub fn by_shebang(&self, interpreter: &str) -> Option<&Arc<dyn LanguageFrontend>> {
        self.frontends
            .iter()
            .find(|f| f.descriptor().matches_interpreter(interpreter))
    }

    pub fn by_editor_mode(&self, mode: &str) -> Option<&Arc<dyn LanguageFrontend>> {
        self.frontends
            .iter()
            .find(|f| f.descriptor().matches_editor_mode(mode))
    }

    pub fn by_file_name(&self, file_name: &str) -> Option<&Arc<dyn LanguageFrontend>> {
        self.frontends
            .iter()
            .find(|f| f.descriptor().matches_file_name(file_name))
    }

    /// Frontends whose `sniff` gives the strongest verdict for a file head, with the verdict.
    pub fn sniff(&self, scan: &HeadScan) -> (SniffVerdict, Vec<&Arc<dyn LanguageFrontend>>) {
        let verdicts: Vec<SniffVerdict> = self.frontends.iter().map(|f| f.sniff(scan)).collect();
        let best = verdicts
            .iter()
            .copied()
            .max()
            .unwrap_or(SniffVerdict::Neutral);
        let top = self
            .frontends
            .iter()
            .zip(verdicts.iter())
            .filter(|(_, v)| **v == best)
            .map(|(f, _)| f)
            .collect();
        (best, top)
    }

    /// The language of one file; `None` when no registered language claims it.
    pub fn detect(
        &self,
        path: &Path,
        overrides: &HashMap<OsString, LangId>,
        head: Option<&HeadScan>,
        ctx: &DetectionContext,
    ) -> Option<Detection> {
        let extension = path.extension();
        if let Some(ext) = extension {
            let lower = OsString::from(ext.to_string_lossy().to_ascii_lowercase());
            if let Some(&lang) = overrides.get(ext).or_else(|| overrides.get(&lower)) {
                return Some(Detection {
                    lang,
                    method: DetectionMethod::Override,
                    ambiguous: false,
                });
            }
        }

        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if let Some(frontend) = self.by_file_name(name) {
                return Some(Detection {
                    lang: frontend.lang_id(),
                    method: DetectionMethod::FileName,
                    ambiguous: false,
                });
            }
        }

        let ext = extension
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase());
        if let Some(claimants) = ext.as_deref().and_then(|e| self.by_extension.get(e)) {
            let candidates: Vec<usize> = claimants.all().collect();
            let ambiguous = candidates.len() > 1;
            if let Some(&primary) = claimants.primary.first() {
                if let Some(lang) = self.frontends[primary].resolve_detection(path, head, ctx) {
                    if self.contains(lang) {
                        return Some(Detection {
                            lang,
                            method: DetectionMethod::FrontendRule,
                            ambiguous,
                        });
                    }
                }
            }
            match candidates.as_slice() {
                [] => {}
                [only] => {
                    return Some(Detection {
                        lang: self.frontends[*only].lang_id(),
                        method: DetectionMethod::Extension,
                        ambiguous: false,
                    })
                }
                _ => {
                    let (idx, method) = self.disambiguate(&candidates, head, ctx);
                    return Some(Detection {
                        lang: self.frontends[idx].lang_id(),
                        method,
                        ambiguous: true,
                    });
                }
            }
        }

        let scan = head?;
        if let Some(interpreter) = scan.shebang_interpreter() {
            if let Some(frontend) = self.by_shebang(&interpreter) {
                return Some(Detection {
                    lang: frontend.lang_id(),
                    method: DetectionMethod::Shebang,
                    ambiguous: false,
                });
            }
        }
        if let Some(mode) = scan.modeline_mode() {
            if let Some(frontend) = self.by_editor_mode(&mode) {
                return Some(Detection {
                    lang: frontend.lang_id(),
                    method: DetectionMethod::EditorModeline,
                    ambiguous: false,
                });
            }
        }
        let definite: Vec<usize> = (0..self.frontends.len())
            .filter(|&i| self.frontends[i].sniff(scan) == SniffVerdict::Definite)
            .collect();
        if definite.is_empty() {
            return None;
        }
        Some(Detection {
            lang: self.frontends[self.most_specific(&definite)].lang_id(),
            method: DetectionMethod::Content,
            ambiguous: definite.len() > 1,
        })
    }

    /// Detects every file of a project in two passes: files whose extension names one
    /// language first, then ambiguous files with the languages found in the first pass as
    /// project context. Results are in input order.
    pub fn detect_project(
        &self,
        files: &[ProjectFile<'_>],
        overrides: &HashMap<OsString, LangId>,
    ) -> Vec<Option<Detection>> {
        let no_context = DetectionContext::new();
        let mut ctx = DetectionContext::new();
        let mut results = Vec::with_capacity(files.len());
        let mut deferred = Vec::new();
        for (i, file) in files.iter().enumerate() {
            let detection = self.detect(file.path, overrides, file.head, &no_context);
            match detection {
                Some(d) if !d.ambiguous => {
                    ctx.insert(d.lang);
                    results.push(Some(d));
                }
                other => {
                    results.push(other);
                    deferred.push(i);
                }
            }
        }
        for i in deferred {
            results[i] = self.detect(files[i].path, overrides, files[i].head, &ctx);
        }
        results
    }

    /// Chooses among several frontends claiming one extension (`candidates` is non-empty,
    /// primary claimants first).
    fn disambiguate(
        &self,
        candidates: &[usize],
        head: Option<&HeadScan>,
        ctx: &DetectionContext,
    ) -> (usize, DetectionMethod) {
        let mut remaining: Vec<usize> = candidates.to_vec();
        let mut verdicts: Vec<(usize, SniffVerdict)> = Vec::new();
        if let Some(scan) = head {
            if let Some(mode) = scan.modeline_mode() {
                if let Some(&i) = remaining
                    .iter()
                    .find(|&&i| self.frontends[i].descriptor().matches_editor_mode(&mode))
                {
                    return (i, DetectionMethod::EditorModeline);
                }
            }
            verdicts = remaining
                .iter()
                .map(|&i| (i, self.frontends[i].sniff(scan)))
                .collect();
            let definite: Vec<usize> = verdicts
                .iter()
                .filter(|(_, v)| *v == SniffVerdict::Definite)
                .map(|(i, _)| *i)
                .collect();
            if !definite.is_empty() {
                return (self.most_specific(&definite), DetectionMethod::Content);
            }
            let possible: Vec<usize> = verdicts
                .iter()
                .filter(|(_, v)| *v != SniffVerdict::Impossible)
                .map(|(i, _)| *i)
                .collect();
            if !possible.is_empty() {
                remaining = possible;
            }
        }
        let present: Vec<usize> = remaining
            .iter()
            .copied()
            .filter(|&i| ctx.contains(self.frontends[i].lang_id()))
            .collect();
        if !present.is_empty() {
            return (self.most_specific(&present), DetectionMethod::ProjectContext);
        }
        let likely: Vec<usize> = verdicts
            .iter()
            .filter(|(i, v)| *v == SniffVerdict::Likely && remaining.contains(i))
            .map(|(i, _)| *i)
            .collect();
        if !likely.is_empty() {
            return (self.most_specific(&likely), DetectionMethod::Content);
        }
        (remaining[0], DetectionMethod::PrimaryClaimant)
    }

    /// The candidate that supersedes the most other candidates (C++ over C); ties keep the
    /// given order. `indices` is non-empty.
    fn most_specific(&self, indices: &[usize]) -> usize {
        let mut best = indices[0];
        let mut best_score = 0usize;
        for &i in indices {
            let descriptor = self.frontends[i].descriptor();
            let score = indices
                .iter()
                .filter(|&&j| j != i && descriptor.supersedes_lang(self.frontends[j].lang_id()))
                .count();
            if score > best_score {
                best = i;
                best_score = score;
            }
        }
        best
    }

    // ── Per-phase adapters ──

    pub fn ingestion_adapters(&self) -> Vec<Arc<dyn LanguageAdapter>> {
        self.frontends
            .iter()
            .filter_map(|f| f.ingestion_adapter())
            .collect()
    }

    pub fn lexers(&self) -> Vec<Arc<dyn SourceLexer>> {
        self.frontends.iter().filter_map(|f| f.lexer()).collect()
    }

    pub fn reduction_adapters(&self) -> Vec<(LangId, Arc<dyn ASTReductionAdapter>)> {
        self.frontends
            .iter()
            .filter_map(|f| f.reduction_adapter().map(|a| (f.lang_id(), a)))
            .collect()
    }

    pub fn semantic_adapters(&self) -> Vec<(LangId, Arc<dyn SemanticAdapter>)> {
        self.frontends
            .iter()
            .filter_map(|f| f.semantic_adapter().map(|a| (f.lang_id(), a)))
            .collect()
    }

    pub fn semantic_adapter(&self, lang: LangId) -> Option<Arc<dyn SemanticAdapter>> {
        self.get(lang).and_then(|f| f.semantic_adapter())
    }

    /// Registers every frontend's Phase 1 adapter and lexer with the ingestion registry.
    pub fn install_ingestion(&self, registry: &mut AdapterRegistry) {
        for adapter in self.ingestion_adapters() {
            registry.register(adapter);
        }
        for lexer in self.lexers() {
            registry.register_lexer(lexer);
        }
    }

    /// Registers every frontend's Phase 2 adapter with the reduction registry.
    pub fn install_reduction(&self, registry: &mut ASTAdapterRegistry) {
        for (lang, adapter) in self.reduction_adapters() {
            registry.register(lang, adapter);
        }
    }

    // ── Consistency checks ──

    /// Every inconsistency between the registered frontends, their descriptors, their
    /// adapters, the supported-language list and the C-family language profiles.
    pub fn validate(&self) -> Vec<RegistryError> {
        let mut issues = self.issues.clone();
        for frontend in &self.frontends {
            let lang = frontend.lang_id();
            let descriptor = frontend.descriptor();
            if !LANGUAGES.iter().any(|d| d.lang == lang) {
                issues.push(RegistryError::UnlistedLanguage(lang));
            }
            if let Some(adapter) = frontend.ingestion_adapter() {
                Self::check_component(
                    &mut issues,
                    descriptor,
                    "Phase 1 adapter",
                    adapter.language_id(),
                    adapter.file_extensions(),
                );
                for ext in descriptor.extensions {
                    let handled = adapter
                        .file_extensions()
                        .iter()
                        .any(|e| e.eq_ignore_ascii_case(ext));
                    if !handled {
                        issues.push(RegistryError::UnhandledExtension {
                            lang,
                            extension: ext.to_string(),
                        });
                    }
                }
            }
            if let Some(lexer) = frontend.lexer() {
                Self::check_component(
                    &mut issues,
                    descriptor,
                    "lexer",
                    lexer.language_id(),
                    lexer.file_extensions(),
                );
            }
            if let Some(language) = frontend.ts_language() {
                let version = language.abi_version();
                let min = tree_sitter::MIN_COMPATIBLE_LANGUAGE_VERSION;
                let max = tree_sitter::LANGUAGE_VERSION;
                if version < min || version > max {
                    issues.push(RegistryError::IncompatibleGrammar {
                        lang,
                        version,
                        min,
                        max,
                    });
                }
                if let Some(reduction) = frontend.reduction_adapter() {
                    let other = reduction.ts_language();
                    let same_name = match (language.name(), other.name()) {
                        (Some(a), Some(b)) => a == b,
                        _ => true,
                    };
                    if !same_name
                        || other.abi_version() != version
                        || other.node_kind_count() != language.node_kind_count()
                        || other.field_count() != language.field_count()
                    {
                        issues.push(RegistryError::GrammarMismatch { lang });
                    }
                }
            }
            let declared = frontend.capabilities();
            for (phase, fidelity) in declared.iter() {
                let ceiling = declared.supported_ceiling(phase);
                if fidelity > ceiling {
                    issues.push(RegistryError::OverstatedCapability {
                        lang,
                        phase,
                        declared: fidelity,
                        ceiling,
                    });
                }
            }
            if let Some(language_profile) = profile::profile(lang) {
                let profile_status = language_profile.status();
                if profile_status != descriptor.status.as_str() {
                    issues.push(RegistryError::StatusMismatch {
                        lang,
                        descriptor: descriptor.status.as_str(),
                        profile: profile_status,
                    });
                }
            }
        }
        for descriptor in LANGUAGES.iter() {
            if !self.contains(descriptor.lang) {
                issues.push(RegistryError::MissingFrontend(descriptor.lang));
            }
        }
        issues
    }

    fn check_component(
        issues: &mut Vec<RegistryError>,
        descriptor: &LanguageDescriptor,
        component: &'static str,
        found: LangId,
        extensions: &[&str],
    ) {
        if found != descriptor.lang {
            issues.push(RegistryError::ComponentMismatch {
                lang: descriptor.lang,
                component,
                found,
            });
        }
        for ext in extensions {
            if descriptor.claims_extension(ext).is_none() {
                issues.push(RegistryError::UndeclaredExtension {
                    lang: descriptor.lang,
                    component,
                    extension: ext.to_string(),
                });
            }
        }
    }

    /// Lists the anonymous tokens of a language's grammar that its Phase 1 adapter maps to
    /// `TokenType::Unknown`, so a grammar upgrade cannot drop source text silently.
    pub fn audit_grammar(&self, lang: LangId) -> Option<GrammarAudit> {
        let adapter = self.get(lang)?.ingestion_adapter()?;
        let language = adapter.ts_language();
        let count = language.node_kind_count();
        let mut unclassified = Vec::new();
        for id in 0..count {
            let Ok(id) = u16::try_from(id) else { break };
            if language.node_kind_is_named(id) || !language.node_kind_is_visible(id) {
                continue;
            }
            let Some(kind) = language.node_kind_for_id(id) else {
                continue;
            };
            if kind.is_empty() || kind.starts_with('_') || kind == "end" {
                continue;
            }
            if adapter.classify_leaf(kind, false) == TokenType::Unknown {
                unclassified.push(kind);
            }
        }
        unclassified.sort_unstable();
        unclassified.dedup();
        Some(GrammarAudit {
            lang,
            abi_version: language.abi_version(),
            node_kind_count: count,
            unclassified_tokens: unclassified,
        })
    }
}
