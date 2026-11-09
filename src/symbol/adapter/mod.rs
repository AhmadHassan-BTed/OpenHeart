pub mod java;

pub use java::*;

use crate::core::types::ast::ASTNodeType;
use crate::core::types::symbol::{ScopeKind, SymbolKind, SymbolVisibility};
use crate::core::types::token::LangId;
use crate::symbol::model::{Dialect, FrontEnd};

/// Trait defining language-specific semantic resolution rules.
///
/// The first six methods are required. The remaining ones have conservative defaults so an
/// adapter only overrides the rules its language has.
pub trait SemanticAdapter: Send + Sync {
    fn is_declaration(&self, node_type: ASTNodeType) -> bool;
    fn symbol_kind(&self, node_type: ASTNodeType) -> SymbolKind;
    fn scope_kind(&self, node_type: ASTNodeType) -> ScopeKind;
    fn primitive_type_id(&self, text: &str) -> Option<u32>;

    /// True when the type with this (simple or qualified) name is a collection whose
    /// elements are its first type argument. Consulted for library types that the built-in
    /// catalog does not describe.
    fn is_collection_type(&self, type_name: &str) -> bool;

    /// Access of a declaration of `kind` written without an access modifier, independent
    /// of where it is declared.
    fn default_visibility(&self, kind: SymbolKind) -> SymbolVisibility;

    /// Access of a declaration of `kind` written without an access modifier inside a
    /// declaration of kind `enclosing` (`None` at top level).
    fn default_visibility_in(
        &self,
        kind: SymbolKind,
        _enclosing: Option<SymbolKind>,
    ) -> SymbolVisibility {
        self.default_visibility(kind)
    }

    /// Modifier bits implied by the language for a declaration of `kind` inside `enclosing`,
    /// given its explicit modifiers and visibility and whether it has a body.
    fn implicit_modifiers(
        &self,
        _kind: SymbolKind,
        _enclosing: Option<SymbolKind>,
        _explicit_modifiers: u16,
        _explicit_visibility: Option<SymbolVisibility>,
        _has_body: bool,
    ) -> u16 {
        0
    }

    /// True when a non-declaration node of this type opens a block scope.
    fn opens_block_scope(&self, node_type: ASTNodeType) -> bool {
        node_type == ASTNodeType::NN_BLOCK
    }

    /// Front end that discovers declarations in files of `lang` (`None`: language unknown,
    /// e.g. a synthetic AST without a file record). An adapter that does not extract names
    /// from grammar fields only supports the heuristic front end.
    fn front_end(&self, _lang: Option<LangId>) -> FrontEnd {
        FrontEnd::AstHeuristic
    }

    /// Surface syntax of files of `lang`, used by the token-level recognizers.
    fn dialect(&self, _lang: Option<LangId>) -> Dialect {
        Dialect::Other
    }
}
