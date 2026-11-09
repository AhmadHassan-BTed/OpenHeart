use std::collections::HashMap;
use std::ffi::OsString;
use std::path::Path;
use std::sync::Arc;

use crate::core::types::token::LangId;
use crate::ingestion::adapter::java::JavaLanguageAdapter;
use crate::ingestion::adapter::javascript::JavaScriptLanguageAdapter;
use crate::ingestion::adapter::kotlin::KotlinLexer;
use crate::ingestion::adapter::{LanguageAdapter, SourceLexer};

/// How Phase 1 turns a file of a given language into tokens.
#[derive(Clone)]
pub enum Frontend {
    /// A linked tree-sitter grammar; Phase 2 can rebuild the syntax tree.
    Grammar(Arc<dyn LanguageAdapter>),
    /// A hand-written lexer; the file is lexical-only (no syntax tree).
    Lexer(Arc<dyn SourceLexer>),
}

/// Registry of Phase 1 frontends.
///
/// Lookups never fall back to another language: a language without a registered
/// frontend is reported as unsupported, and its files are skipped with a diagnostic
/// instead of being re-parsed with an unrelated grammar.
#[derive(Clone)]
pub struct AdapterRegistry {
    adapters: HashMap<LangId, Arc<dyn LanguageAdapter>>,
    lexers: HashMap<LangId, Arc<dyn SourceLexer>>,
    ext_map: HashMap<String, LangId>,
}

impl AdapterRegistry {
    /// Built-in frontends: Java and JavaScript (tree-sitter grammars) and Kotlin (lexer).
    pub fn new() -> Self {
        let mut reg = Self {
            adapters: HashMap::new(),
            lexers: HashMap::new(),
            ext_map: HashMap::new(),
        };
        reg.register(Arc::new(JavaLanguageAdapter::new()));
        reg.register(Arc::new(JavaScriptLanguageAdapter::new()));
        reg.register_lexer(Arc::new(KotlinLexer::new()));
        reg
    }

    /// Registers a grammar-backed adapter, replacing any frontend for its language.
    pub fn register(&mut self, adapter: Arc<dyn LanguageAdapter>) {
        let lang_id = adapter.language_id();
        for ext in adapter.file_extensions() {
            self.ext_map.insert(ext.to_lowercase(), lang_id);
        }
        self.lexers.remove(&lang_id);
        self.adapters.insert(lang_id, adapter);
    }

    /// Registers a lexer-only frontend. A grammar-backed adapter for the same language,
    /// if present, takes precedence.
    pub fn register_lexer(&mut self, lexer: Arc<dyn SourceLexer>) {
        let lang_id = lexer.language_id();
        for ext in lexer.file_extensions() {
            self.ext_map.insert(ext.to_lowercase(), lang_id);
        }
        if !self.adapters.contains_key(&lang_id) {
            self.lexers.insert(lang_id, lexer);
        }
    }

    /// Grammar-backed adapter for exactly `lang_id`, if one is registered.
    pub fn get(&self, lang_id: LangId) -> Option<Arc<dyn LanguageAdapter>> {
        self.adapters.get(&lang_id).cloned()
    }

    /// The frontend for exactly `lang_id`, or `None` when the language is unsupported.
    pub fn frontend(&self, lang_id: LangId) -> Option<Frontend> {
        if let Some(adapter) = self.adapters.get(&lang_id) {
            return Some(Frontend::Grammar(adapter.clone()));
        }
        self.lexers
            .get(&lang_id)
            .map(|lexer| Frontend::Lexer(lexer.clone()))
    }

    /// Language whose registered frontend claims `ext` (case-insensitive, without the dot).
    pub fn language_for_extension(&self, ext: &str) -> Option<LangId> {
        self.ext_map.get(&ext.to_lowercase()).copied()
    }

    /// Languages with a registered frontend, in id order.
    pub fn supported_languages(&self) -> Vec<LangId> {
        let mut langs: Vec<LangId> = self
            .adapters
            .keys()
            .chain(self.lexers.keys())
            .copied()
            .collect();
        langs.sort();
        langs.dedup();
        langs
    }

    /// Language of `path`, from an explicit override or the (case-insensitive) extension.
    /// Detection is independent of support: `frontend` decides whether a detected
    /// language can be ingested.
    pub fn detect(overrides: &HashMap<OsString, LangId>, path: &Path) -> LangId {
        if let Some(ext) = path.extension() {
            if let Some(&lang) = overrides.get(ext) {
                return lang;
            }
            if let Some(ext_str) = ext.to_str() {
                let lower = ext_str.to_lowercase();
                if let Some(&lang) = overrides.get(&OsString::from(lower.as_str())) {
                    return lang;
                }
                match lower.as_str() {
                    "java" => return LangId::Java,
                    "kt" | "kts" => return LangId::Kotlin,
                    "swift" => return LangId::Swift,
                    "py" | "pyw" | "pyx" => return LangId::Python,
                    "js" | "jsx" | "mjs" | "cjs" => return LangId::JavaScript,
                    "ts" | "tsx" | "mts" | "cts" => return LangId::TypeScript,
                    "rs" => return LangId::Rust,
                    "cpp" | "c" | "h" | "hpp" | "cc" | "cxx" | "hh" | "hxx" | "c++" | "h++" => {
                        return LangId::Cpp
                    }
                    "go" => return LangId::Go,
                    "cs" => return LangId::Generic,
                    _ => {}
                }
            }
        }
        LangId::Unknown
    }
}

impl Default for AdapterRegistry {
    fn default() -> Self {
        Self::new()
    }
}
