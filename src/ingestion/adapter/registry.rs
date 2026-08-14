use std::collections::HashMap;
use std::ffi::OsString;
use std::path::Path;
use std::sync::Arc;

use crate::core::types::token::LangId;
use crate::ingestion::adapter::generic::GenericLanguageAdapter;
use crate::ingestion::adapter::java::JavaLanguageAdapter;
use crate::ingestion::adapter::kotlin::KotlinLanguageAdapter;
use crate::ingestion::adapter::LanguageAdapter;

#[derive(Clone)]
pub struct AdapterRegistry {
    adapters: HashMap<LangId, Arc<dyn LanguageAdapter>>,
    ext_map: HashMap<String, LangId>,
}

impl AdapterRegistry {
    pub fn new() -> Self {
        let mut reg = Self {
            adapters: HashMap::new(),
            ext_map: HashMap::new(),
        };
        reg.register(Arc::new(JavaLanguageAdapter::new()));
        reg.register(Arc::new(KotlinLanguageAdapter::new()));
        
        let js_adapter = Arc::new(GenericLanguageAdapter::new(
            LangId::JavaScript,
            vec!["js", "jsx", "mjs", "cjs"],
            tree_sitter_javascript::language(),
        ));
        reg.register(js_adapter.clone());

        let unknown_adapter = Arc::new(GenericLanguageAdapter::new(
            LangId::Unknown,
            vec!["*"],
            tree_sitter_javascript::language(),
        ));
        reg.register(unknown_adapter);

        reg
    }

    pub fn register(&mut self, adapter: Arc<dyn LanguageAdapter>) {
        let lang_id = adapter.language_id();
        for ext in adapter.file_extensions() {
            self.ext_map.insert(ext.to_lowercase(), lang_id);
        }
        self.adapters.insert(lang_id, adapter);
    }

    pub fn get(&self, lang_id: LangId) -> Option<Arc<dyn LanguageAdapter>> {
        self.adapters.get(&lang_id).cloned().or_else(|| self.adapters.get(&LangId::Unknown).cloned())
    }

    pub fn detect(overrides: &HashMap<OsString, LangId>, path: &Path) -> LangId {
        if let Some(ext) = path.extension() {
            if let Some(&lang) = overrides.get(ext) {
                return lang;
            }
            if let Some(ext_str) = ext.to_str() {
                match ext_str.to_lowercase().as_str() {
                    "java" => return LangId::Java,
                    "kt" | "kts" => return LangId::Kotlin,
                    "swift" => return LangId::Swift,
                    "py" => return LangId::Python,
                    "js" | "jsx" | "mjs" | "cjs" => return LangId::JavaScript,
                    "ts" | "tsx" => return LangId::TypeScript,
                    "rs" => return LangId::Rust,
                    "cpp" | "c" | "h" | "hpp" => return LangId::Cpp,
                    "go" => return LangId::Go,
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
