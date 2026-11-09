//! Java Semantic Adapter implementing Java-specific declaration rules, implicit modifiers
//! (JLS 8.9, 8.10, 9.3, 9.4, 9.5), default access (JLS 6.6.1) and primitive types.

use crate::core::types::ast::ASTNodeType;
use crate::core::types::symbol::{ScopeKind, SymbolKind, SymbolModifiers, SymbolVisibility};
use crate::core::types::token::LangId;
use crate::symbol::adapter::SemanticAdapter;
use crate::symbol::model::{Dialect, FrontEnd};

pub struct JavaSemanticAdapter;

impl JavaSemanticAdapter {
    pub fn new() -> Self {
        Self
    }
}

impl Default for JavaSemanticAdapter {
    fn default() -> Self {
        Self::new()
    }
}

pub const PRIMITIVE_VOID_ID: u32 = 0xB000_0000;
pub const PRIMITIVE_BOOLEAN_ID: u32 = 0xB000_0001;
pub const PRIMITIVE_BYTE_ID: u32 = 0xB000_0002;
pub const PRIMITIVE_SHORT_ID: u32 = 0xB000_0003;
pub const PRIMITIVE_CHAR_ID: u32 = 0xB000_0004;
pub const PRIMITIVE_INT_ID: u32 = 0xB000_0005;
pub const PRIMITIVE_LONG_ID: u32 = 0xB000_0006;
pub const PRIMITIVE_FLOAT_ID: u32 = 0xB000_0007;
pub const PRIMITIVE_DOUBLE_ID: u32 = 0xB000_0008;

/// True for the primitive pseudo-ids stored in `type_id` (they are not symbol ids).
pub fn is_primitive_type_id(id: u32) -> bool {
    (PRIMITIVE_VOID_ID..=PRIMITIVE_DOUBLE_ID).contains(&id)
}

fn is_type_kind(kind: SymbolKind) -> bool {
    matches!(
        kind,
        SymbolKind::SK_CLASS
            | SymbolKind::SK_INTERFACE
            | SymbolKind::SK_ENUM
            | SymbolKind::SK_RECORD
            | SymbolKind::SK_ANNOTATION_TYPE
            | SymbolKind::SK_ANON_CLASS
    )
}

fn is_interface_like(kind: Option<SymbolKind>) -> bool {
    matches!(
        kind,
        Some(SymbolKind::SK_INTERFACE | SymbolKind::SK_ANNOTATION_TYPE)
    )
}

impl SemanticAdapter for JavaSemanticAdapter {
    fn is_declaration(&self, node_type: ASTNodeType) -> bool {
        matches!(
            node_type,
            ASTNodeType::NN_MODULE
                | ASTNodeType::NN_CLASS_DECL
                | ASTNodeType::NN_INTERFACE_DECL
                | ASTNodeType::NN_ENUM_DECL
                | ASTNodeType::NN_RECORD_DECL
                | ASTNodeType::NN_ANNOTATION_DECL
                | ASTNodeType::NN_METHOD_DECL
                | ASTNodeType::NN_CONSTRUCTOR_DECL
                | ASTNodeType::NN_FIELD_DECL
                | ASTNodeType::NN_PARAM_DECL
                | ASTNodeType::NN_LOCAL_VAR_DECL
                | ASTNodeType::NN_TYPE_PARAM
                | ASTNodeType::NN_LAMBDA_EXPR
                | ASTNodeType::NN_JAVA_STATIC_INIT
                | ASTNodeType::NN_JAVA_INSTANCE_INIT
        )
    }

    fn symbol_kind(&self, node_type: ASTNodeType) -> SymbolKind {
        match node_type {
            ASTNodeType::NN_MODULE => SymbolKind::SK_MODULE,
            ASTNodeType::NN_CLASS_DECL => SymbolKind::SK_CLASS,
            ASTNodeType::NN_INTERFACE_DECL => SymbolKind::SK_INTERFACE,
            ASTNodeType::NN_ENUM_DECL => SymbolKind::SK_ENUM,
            ASTNodeType::NN_RECORD_DECL => SymbolKind::SK_RECORD,
            ASTNodeType::NN_ANNOTATION_DECL => SymbolKind::SK_ANNOTATION_TYPE,
            ASTNodeType::NN_METHOD_DECL => SymbolKind::SK_METHOD,
            ASTNodeType::NN_CONSTRUCTOR_DECL => SymbolKind::SK_CONSTRUCTOR,
            ASTNodeType::NN_FIELD_DECL => SymbolKind::SK_FIELD,
            ASTNodeType::NN_PARAM_DECL => SymbolKind::SK_PARAM,
            ASTNodeType::NN_LOCAL_VAR_DECL => SymbolKind::SK_LOCAL_VAR,
            ASTNodeType::NN_TYPE_PARAM => SymbolKind::SK_TYPE_PARAM,
            ASTNodeType::NN_LAMBDA_EXPR => SymbolKind::SK_LAMBDA,
            ASTNodeType::NN_JAVA_STATIC_INIT => SymbolKind::SK_STATIC_INIT,
            ASTNodeType::NN_JAVA_INSTANCE_INIT => SymbolKind::SK_INSTANCE_INIT,
            _ => SymbolKind::SK_EXTERNAL,
        }
    }

    fn scope_kind(&self, node_type: ASTNodeType) -> ScopeKind {
        match node_type {
            ASTNodeType::NN_MODULE => ScopeKind::File,
            ASTNodeType::NN_CLASS_DECL
            | ASTNodeType::NN_INTERFACE_DECL
            | ASTNodeType::NN_ENUM_DECL
            | ASTNodeType::NN_RECORD_DECL
            | ASTNodeType::NN_ANNOTATION_DECL => ScopeKind::Class,
            ASTNodeType::NN_METHOD_DECL | ASTNodeType::NN_CONSTRUCTOR_DECL => ScopeKind::Method,
            ASTNodeType::NN_LAMBDA_EXPR => ScopeKind::Lambda,
            _ => ScopeKind::Block,
        }
    }

    fn primitive_type_id(&self, text: &str) -> Option<u32> {
        match text {
            "void" => Some(PRIMITIVE_VOID_ID),
            "boolean" => Some(PRIMITIVE_BOOLEAN_ID),
            "byte" => Some(PRIMITIVE_BYTE_ID),
            "short" => Some(PRIMITIVE_SHORT_ID),
            "char" => Some(PRIMITIVE_CHAR_ID),
            "int" => Some(PRIMITIVE_INT_ID),
            "long" => Some(PRIMITIVE_LONG_ID),
            "float" => Some(PRIMITIVE_FLOAT_ID),
            "double" => Some(PRIMITIVE_DOUBLE_ID),
            _ => None,
        }
    }

    fn default_visibility(&self, kind: SymbolKind, enclosing: Option<SymbolKind>) -> SymbolVisibility {
        // JLS 9.3, 9.4, 9.5: interface and annotation members are implicitly public.
        if is_interface_like(enclosing) && kind != SymbolKind::SK_PARAM {
            return SymbolVisibility::Public;
        }
        // JLS 8.9.2: an enum constructor without an access modifier is private.
        if kind == SymbolKind::SK_CONSTRUCTOR && enclosing == Some(SymbolKind::SK_ENUM) {
            return SymbolVisibility::Private;
        }
        // JLS 8.9.1: enum constants are implicitly public.
        if kind == SymbolKind::SK_ENUM_CONSTANT {
            return SymbolVisibility::Public;
        }
        // JLS 6.6.1: otherwise, no modifier means package access.
        SymbolVisibility::Package
    }

    fn implicit_modifiers(
        &self,
        kind: SymbolKind,
        enclosing: Option<SymbolKind>,
        explicit_modifiers: u16,
        explicit_visibility: Option<SymbolVisibility>,
        has_body: bool,
    ) -> u16 {
        let mut mods = 0u16;
        let nested_in_type = enclosing.map(is_type_kind).unwrap_or(false);
        let in_interface = is_interface_like(enclosing);

        match kind {
            SymbolKind::SK_INTERFACE | SymbolKind::SK_ANNOTATION_TYPE => {
                // JLS 9.1.1.1, 8.5.1: interfaces are abstract; member interfaces are static.
                mods |= SymbolModifiers::ABSTRACT;
                if nested_in_type {
                    mods |= SymbolModifiers::STATIC;
                }
            }
            SymbolKind::SK_ENUM => {
                // JLS 8.9: a nested enum is implicitly static.
                if nested_in_type {
                    mods |= SymbolModifiers::STATIC;
                }
            }
            SymbolKind::SK_RECORD => {
                // JLS 8.10: records are implicitly final; nested records are static.
                mods |= SymbolModifiers::FINAL;
                if nested_in_type {
                    mods |= SymbolModifiers::STATIC;
                }
            }
            SymbolKind::SK_CLASS => {
                // JLS 9.5: member classes of interfaces are implicitly static.
                if in_interface {
                    mods |= SymbolModifiers::STATIC;
                }
            }
            SymbolKind::SK_FIELD => {
                // JLS 9.3: interface fields are implicitly public, static and final.
                if in_interface {
                    mods |= SymbolModifiers::STATIC | SymbolModifiers::FINAL;
                }
            }
            SymbolKind::SK_ENUM_CONSTANT => {
                // JLS 8.9.3: enum constants are implicitly public static final.
                mods |= SymbolModifiers::STATIC | SymbolModifiers::FINAL;
            }
            SymbolKind::SK_METHOD => {
                // JLS 9.4: an interface method without a body that is neither static, default
                // nor private is implicitly abstract.
                if in_interface
                    && !has_body
                    && explicit_modifiers & (SymbolModifiers::STATIC | SymbolModifiers::DEFAULT)
                        == 0
                    && explicit_visibility != Some(SymbolVisibility::Private)
                {
                    mods |= SymbolModifiers::ABSTRACT;
                }
            }
            _ => {}
        }
        mods
    }

    fn opens_block_scope(&self, node_type: ASTNodeType) -> bool {
        matches!(
            node_type,
            ASTNodeType::NN_BLOCK
                | ASTNodeType::NN_FOR_STMT
                | ASTNodeType::NN_ENHANCED_FOR
                | ASTNodeType::NN_CATCH_CLAUSE
                | ASTNodeType::NN_SWITCH_STMT
                | ASTNodeType::NN_SWITCH_EXPR
        )
    }

    fn front_end(&self, lang: Option<LangId>) -> FrontEnd {
        match lang {
            None | Some(LangId::Java) => FrontEnd::Structural,
            // Phase 2 parses Kotlin with the JavaScript grammar; that tree is not usable.
            Some(LangId::Kotlin) => FrontEnd::Lexical,
            Some(_) => FrontEnd::AstHeuristic,
        }
    }

    fn dialect(&self, lang: Option<LangId>) -> Dialect {
        match lang {
            None | Some(LangId::Java) => Dialect::Java,
            Some(LangId::Kotlin) => Dialect::Kotlin,
            Some(_) => Dialect::Other,
        }
    }
}
