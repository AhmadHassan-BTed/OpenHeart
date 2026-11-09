//! Registry for AST Reduction Adapters mapped by LangId.
//!
//! Lookups are exact: a language without a registered adapter has no syntax tree in
//! Phase 2 (its files are reported and contribute tokens only). Nothing falls back to
//! another language's grammar.

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
    /// Built-in adapters: Java (dedicated) and JavaScript (generic C-family adapter over
    /// the linked JavaScript grammar).
    pub fn new() -> Self {
        let mut registry = Self {
            adapters: HashMap::new(),
        };
        registry.register(LangId::Java, Arc::new(JavaASTReductionAdapter::new()));
        registry.register(
            LangId::JavaScript,
            Arc::new(GenericASTReductionAdapter::new(
                tree_sitter_javascript::language(),
            )),
        );
        registry
    }

    pub fn register(&mut self, lang: LangId, adapter: Arc<dyn ASTReductionAdapter>) {
        self.adapters.insert(lang, adapter);
    }

    /// The adapter for exactly `lang`, or `None` when Phase 2 has no tree for it.
    pub fn get(&self, lang: LangId) -> Option<Arc<dyn ASTReductionAdapter>> {
        self.adapters.get(&lang).cloned()
    }

    pub fn supports(&self, lang: LangId) -> bool {
        self.adapters.contains_key(&lang)
    }
}

impl Default for ASTAdapterRegistry {
    fn default() -> Self {
        Self::new()
    }
}
