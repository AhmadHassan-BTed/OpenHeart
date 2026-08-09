//! Java Semantic Adapter implementing Java-specific declaration & primitive types.

use crate::core::types::ast::ASTNodeType;
use crate::core::types::symbol::{ScopeKind, SymbolKind, SymbolVisibility};
use crate::symbol::adapter::SemanticAdapter;

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

    fn is_collection_type(&self, type_name: &str) -> bool {
        matches!(
            type_name,
            "List"
                | "java.util.List"
                | "Set"
                | "java.util.Set"
                | "Map"
                | "java.util.Map"
                | "Collection"
                | "java.util.Collection"
                | "Iterable"
                | "java.lang.Iterable"
                | "ArrayList"
                | "java.util.ArrayList"
                | "HashMap"
                | "java.util.HashMap"
                | "HashSet"
                | "java.util.HashSet"
        )
    }

    fn default_visibility(&self, kind: SymbolKind) -> SymbolVisibility {
        match kind {
            SymbolKind::SK_CLASS
            | SymbolKind::SK_INTERFACE
            | SymbolKind::SK_ENUM
            | SymbolKind::SK_RECORD => SymbolVisibility::Package,
            _ => SymbolVisibility::Public,
        }
    }
}
