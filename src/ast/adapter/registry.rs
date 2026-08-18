//! Registry for AST Reduction Adapters mapped by LangId.

use super::generic::GenericASTReductionAdapter;
use super::java::JavaASTReductionAdapter;
use super::ASTReductionAdapter;
use crate::core::types::token::LangId;
use std::collections::HashMap;
use std::sync::Arc;

pub struct ASTAdapterRegistry {
    adapters: HashMap<LangId, Arc<dyn ASTReductionAdapter>>,
}

impl ASTAdapterRegistry {
    pub fn new() -> Self {
        let mut registry = Self {
            adapters: HashMap::new(),
        };
        registry.register(LangId::Java, Arc::new(JavaASTReductionAdapter::new()));
        
        let generic_js = Arc::new(GenericASTReductionAdapter::new(tree_sitter_javascript::language()));
        registry.register(LangId::JavaScript, generic_js.clone());
        registry.register(LangId::TypeScript, generic_js.clone());
        registry.register(LangId::Rust, generic_js.clone());
        registry.register(LangId::Generic, generic_js.clone());
        registry.register(LangId::Unknown, generic_js);
        
        registry
    }

    pub fn register(&mut self, lang: LangId, adapter: Arc<dyn ASTReductionAdapter>) {
        self.adapters.insert(lang, adapter);
    }

    pub fn get(&self, lang: LangId) -> Option<Arc<dyn ASTReductionAdapter>> {
        self.adapters.get(&lang).cloned().or_else(|| self.adapters.get(&LangId::Unknown).cloned())
    }
}

impl Default for ASTAdapterRegistry {
    fn default() -> Self {
        Self::new()
    }
}
