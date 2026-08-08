//! Registry for AST Reduction Adapters mapped by LangId.

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
        registry
    }

    pub fn register(&mut self, lang: LangId, adapter: Arc<dyn ASTReductionAdapter>) {
        self.adapters.insert(lang, adapter);
    }

    pub fn get(&self, lang: LangId) -> Option<Arc<dyn ASTReductionAdapter>> {
        self.adapters.get(&lang).cloned()
    }
}

impl Default for ASTAdapterRegistry {
    fn default() -> Self {
        Self::new()
    }
}
