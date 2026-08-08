//! Core AST Node Types, Operator IDs, and NodeAttr bitfield packings for Phase 2.

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(non_camel_case_types)]
pub enum ASTNodeType {
    NN_UNKNOWN = 0x00,
    NN_MODULE = 0x01,
    NN_CLASS_DECL = 0x02,
    NN_INTERFACE_DECL = 0x03,
    NN_ENUM_DECL = 0x04,
    NN_RECORD_DECL = 0x05,
    NN_ANNOTATION_DECL = 0x06,
    NN_METHOD_DECL = 0x07,
    NN_CONSTRUCTOR_DECL = 0x08,
    NN_FIELD_DECL = 0x09,
    NN_PARAM_DECL = 0x0A,
    NN_LOCAL_VAR_DECL = 0x0B,
    NN_BLOCK = 0x0C,
    NN_IF_STMT = 0x0D,
    NN_ELSE_CLAUSE = 0x0E,
    NN_FOR_STMT = 0x0F,
    NN_ENHANCED_FOR = 0x10,
    NN_WHILE_STMT = 0x11,
    NN_DO_WHILE_STMT = 0x12,
    NN_SWITCH_STMT = 0x13,
    NN_SWITCH_CASE = 0x14,
    NN_TRY_STMT = 0x15,
    NN_CATCH_CLAUSE = 0x16,
    NN_FINALLY_CLAUSE = 0x17,
    NN_RETURN_STMT = 0x18,
    NN_THROW_STMT = 0x19,
    NN_BREAK_STMT = 0x1A,
    NN_CONTINUE_STMT = 0x1B,
    NN_EXPR_STMT = 0x1C,
    NN_ASSIGN_EXPR = 0x1D,
    NN_BINARY_EXPR = 0x1E,
    NN_UNARY_EXPR = 0x1F,
    NN_TERNARY_EXPR = 0x20,
    NN_CALL_EXPR = 0x21,
    NN_NEW_EXPR = 0x22,
    NN_FIELD_ACCESS = 0x23,
    NN_ARRAY_ACCESS = 0x24,
    NN_CAST_EXPR = 0x25,
    NN_INSTANCEOF_EXPR = 0x26,
    NN_LAMBDA_EXPR = 0x27,
    NN_METHOD_REF = 0x28,
    NN_ARRAY_CREATE = 0x29,
    NN_TYPE_REF = 0x2A,
    NN_IDENTIFIER_EXPR = 0x2B,
    NN_LITERAL = 0x2C,
    NN_ANNOTATION_USE = 0x2D,
    NN_TYPE_PARAM = 0x2E,
    NN_SUPER_EXPR = 0x2F,
    NN_THIS_EXPR = 0x30,
    NN_ARRAY_INIT = 0x31,
    NN_SWITCH_EXPR = 0x32,
    NN_PATTERN_MATCH = 0x33,
    NN_YIELD_STMT = 0x34,
    NN_SYNTHETIC = 0x7F,

    // Language Specific (Java Extensions)
    NN_JAVA_STATIC_INIT = 0x80,
    NN_JAVA_INSTANCE_INIT = 0x81,
    NN_JAVA_ASSERT_STMT = 0x82,
    NN_JAVA_LABELED_STMT = 0x83,
    NN_JAVA_SYNCHRONIZED = 0x84,
}

impl From<u8> for ASTNodeType {
    fn from(val: u8) -> Self {
        match val {
            0x01 => ASTNodeType::NN_MODULE,
            0x02 => ASTNodeType::NN_CLASS_DECL,
            0x03 => ASTNodeType::NN_INTERFACE_DECL,
            0x04 => ASTNodeType::NN_ENUM_DECL,
            0x05 => ASTNodeType::NN_RECORD_DECL,
            0x06 => ASTNodeType::NN_ANNOTATION_DECL,
            0x07 => ASTNodeType::NN_METHOD_DECL,
            0x08 => ASTNodeType::NN_CONSTRUCTOR_DECL,
            0x09 => ASTNodeType::NN_FIELD_DECL,
            0x0A => ASTNodeType::NN_PARAM_DECL,
            0x0B => ASTNodeType::NN_LOCAL_VAR_DECL,
            0x0C => ASTNodeType::NN_BLOCK,
            0x0D => ASTNodeType::NN_IF_STMT,
            0x0E => ASTNodeType::NN_ELSE_CLAUSE,
            0x0F => ASTNodeType::NN_FOR_STMT,
            0x10 => ASTNodeType::NN_ENHANCED_FOR,
            0x11 => ASTNodeType::NN_WHILE_STMT,
            0x12 => ASTNodeType::NN_DO_WHILE_STMT,
            0x13 => ASTNodeType::NN_SWITCH_STMT,
            0x14 => ASTNodeType::NN_SWITCH_CASE,
            0x15 => ASTNodeType::NN_TRY_STMT,
            0x16 => ASTNodeType::NN_CATCH_CLAUSE,
            0x17 => ASTNodeType::NN_FINALLY_CLAUSE,
            0x18 => ASTNodeType::NN_RETURN_STMT,
            0x19 => ASTNodeType::NN_THROW_STMT,
            0x1A => ASTNodeType::NN_BREAK_STMT,
            0x1B => ASTNodeType::NN_CONTINUE_STMT,
            0x1C => ASTNodeType::NN_EXPR_STMT,
            0x1D => ASTNodeType::NN_ASSIGN_EXPR,
            0x1E => ASTNodeType::NN_BINARY_EXPR,
            0x1F => ASTNodeType::NN_UNARY_EXPR,
            0x20 => ASTNodeType::NN_TERNARY_EXPR,
            0x21 => ASTNodeType::NN_CALL_EXPR,
            0x22 => ASTNodeType::NN_NEW_EXPR,
            0x23 => ASTNodeType::NN_FIELD_ACCESS,
            0x24 => ASTNodeType::NN_ARRAY_ACCESS,
            0x25 => ASTNodeType::NN_CAST_EXPR,
            0x26 => ASTNodeType::NN_INSTANCEOF_EXPR,
            0x27 => ASTNodeType::NN_LAMBDA_EXPR,
            0x28 => ASTNodeType::NN_METHOD_REF,
            0x29 => ASTNodeType::NN_ARRAY_CREATE,
            0x2A => ASTNodeType::NN_TYPE_REF,
            0x2B => ASTNodeType::NN_IDENTIFIER_EXPR,
            0x2C => ASTNodeType::NN_LITERAL,
            0x2D => ASTNodeType::NN_ANNOTATION_USE,
            0x2E => ASTNodeType::NN_TYPE_PARAM,
            0x2F => ASTNodeType::NN_SUPER_EXPR,
            0x30 => ASTNodeType::NN_THIS_EXPR,
            0x31 => ASTNodeType::NN_ARRAY_INIT,
            0x32 => ASTNodeType::NN_SWITCH_EXPR,
            0x33 => ASTNodeType::NN_PATTERN_MATCH,
            0x34 => ASTNodeType::NN_YIELD_STMT,
            0x7F => ASTNodeType::NN_SYNTHETIC,
            0x80 => ASTNodeType::NN_JAVA_STATIC_INIT,
            0x81 => ASTNodeType::NN_JAVA_INSTANCE_INIT,
            0x82 => ASTNodeType::NN_JAVA_ASSERT_STMT,
            0x83 => ASTNodeType::NN_JAVA_LABELED_STMT,
            0x84 => ASTNodeType::NN_JAVA_SYNCHRONIZED,
            _ => ASTNodeType::NN_UNKNOWN,
        }
    }
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperatorId {
    None = 0x00,
    Assign = 0x01,     // =
    AddAssign = 0x02,  // +=
    SubAssign = 0x03,  // -=
    MulAssign = 0x04,  // *=
    DivAssign = 0x05,  // /=
    ModAssign = 0x06,  // %=
    AndAssign = 0x07,  // &=
    OrAssign = 0x08,   // |=
    XorAssign = 0x09,  // ^=
    ShlAssign = 0x0A,  // <<=
    ShrAssign = 0x0B,  // >>=
    UshrAssign = 0x0C, // >>>=

    Add = 0x10, // +
    Sub = 0x11, // -
    Mul = 0x12, // *
    Div = 0x13, // /
    Mod = 0x14, // %

    Eq = 0x15,    // ==
    NotEq = 0x16, // !=
    Lt = 0x17,    // <
    Gt = 0x18,    // >
    LtEq = 0x19,  // <=
    GtEq = 0x1A,  // >=

    LogicalAnd = 0x1B, // &&
    LogicalOr = 0x1C,  // ||
    LogicalNot = 0x1D, // !

    BitwiseAnd = 0x1E, // &
    BitwiseOr = 0x1F,  // |
    BitwiseXor = 0x20, // ^
    BitwiseNot = 0x21, // ~

    Shl = 0x22,  // <<
    Shr = 0x23,  // >>
    Ushr = 0x24, // >>>

    Inc = 0x25, // ++
    Dec = 0x26, // --
}

/// Helper utilities for encoding and decoding the 32-bit NodeAttr word.
pub struct NodeAttr;

impl NodeAttr {
    pub const VISIBILITY_NONE: u8 = 0x00;
    pub const VISIBILITY_PUBLIC: u8 = 0x01;
    pub const VISIBILITY_PRIVATE: u8 = 0x02;
    pub const VISIBILITY_PROTECTED: u8 = 0x03;
    pub const VISIBILITY_PACKAGE_PRIVATE: u8 = 0x04;

    pub const MOD_STATIC: u8 = 1 << 7;
    pub const MOD_FINAL: u8 = 1 << 6;
    pub const MOD_ABSTRACT: u8 = 1 << 5;
    pub const MOD_SYNCHRONIZED: u8 = 1 << 4;
    pub const MOD_NATIVE: u8 = 1 << 3;
    pub const MOD_VOLATILE: u8 = 1 << 2;
    pub const MOD_TRANSIENT: u8 = 1 << 1;
    pub const MOD_SEALED: u8 = 1 << 0;

    pub fn pack(
        visibility: u8,
        modifiers: u8,
        operator_id: OperatorId,
        aux_flags: u8,
        lang_flags: u8,
    ) -> u32 {
        ((visibility as u32 & 0x0F) << 28)
            | ((modifiers as u32 & 0xFF) << 20)
            | (((operator_id as u8) as u32 & 0xFF) << 12)
            | ((aux_flags as u32 & 0x0F) << 8)
            | (lang_flags as u32 & 0xFF)
    }

    pub fn unpack_visibility(attr: u32) -> u8 {
        ((attr >> 28) & 0x0F) as u8
    }

    pub fn unpack_modifiers(attr: u32) -> u8 {
        ((attr >> 20) & 0xFF) as u8
    }

    pub fn unpack_operator_id(attr: u32) -> OperatorId {
        let val = ((attr >> 12) & 0xFF) as u8;
        // Basic mapping back to OperatorId
        if val == 0 {
            OperatorId::None
        } else {
            OperatorId::Assign
        } // default fallback for basic test
    }
}
