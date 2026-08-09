pub mod java;

pub use java::*;

use crate::core::types::ast::ASTNodeType;
use crate::core::types::symbol::{ScopeKind, SymbolKind, SymbolVisibility};

/// Trait defining language-specific semantic resolution rules
pub trait SemanticAdapter: Send + Sync {
    fn is_declaration(&self, node_type: ASTNodeType) -> bool;
    fn symbol_kind(&self, node_type: ASTNodeType) -> SymbolKind;
    fn scope_kind(&self, node_type: ASTNodeType) -> ScopeKind;
    fn primitive_type_id(&self, text: &str) -> Option<u32>;
    fn is_collection_type(&self, type_name: &str) -> bool;
    fn default_visibility(&self, kind: SymbolKind) -> SymbolVisibility;
}
