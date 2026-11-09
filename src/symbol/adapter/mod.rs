pub mod java;

pub use java::*;

use crate::core::types::ast::ASTNodeType;
use crate::core::types::symbol::{ScopeKind, SymbolKind, SymbolVisibility};
use crate::core::types::token::LangId;
use crate::symbol::model::{Dialect, FrontEnd};

/// Trait defining language-specific semantic resolution rules
pub trait SemanticAdapter: Send + Sync {
    fn is_declaration(&self, node_type: ASTNodeType) -> bool;
    fn symbol_kind(&self, node_type: ASTNodeType) -> SymbolKind;
    fn scope_kind(&self, node_type: ASTNodeType) -> ScopeKind;
    fn primitive_type_id(&self, text: &str) -> Option<u32>;

    /// Access of a declaration of `kind` that has no access modifier, given the kind of the
    /// declaration that encloses it.
    fn default_visibility(&self, kind: SymbolKind, enclosing: Option<SymbolKind>) -> SymbolVisibility;

    /// Modifier bits implied by the language for a declaration of `kind` inside `enclosing`,
    /// given its explicit modifiers and visibility and whether it has a body.
    fn implicit_modifiers(
        &self,
        kind: SymbolKind,
        enclosing: Option<SymbolKind>,
        explicit_modifiers: u16,
        explicit_visibility: Option<SymbolVisibility>,
        has_body: bool,
    ) -> u16;

    /// True when a non-declaration node of this type opens a block scope (JLS 6.3).
    fn opens_block_scope(&self, node_type: ASTNodeType) -> bool;

    /// Front end that discovers declarations in files of `lang` (`None`: language unknown,
    /// e.g. a synthetic AST without a file record).
    fn front_end(&self, lang: Option<LangId>) -> FrontEnd;

    /// Surface syntax of files of `lang`.
    fn dialect(&self, lang: Option<LangId>) -> Dialect;
}
