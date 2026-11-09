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
    /// A local variable declaration statement, and also each variable declarator of a
    /// field / constant / local declaration (`a` and `b` in `int a, b = 1;`). Declarators
    /// carry their name in the name-token side array.
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
    /// `package a.b;` — one per compilation unit; its single child is the (qualified) name.
    NN_PACKAGE_DECL = 0x35,
    /// `import [static] a.b.C[.*];` — aux flags carry static / on-demand; child is the name.
    NN_IMPORT_DECL = 0x36,
    /// `throws A, B` clause of a method or constructor; children are `NN_TYPE_REF`.
    NN_THROWS_CLAUSE = 0x39,
    /// `permits A, B` clause of a sealed type; children are `NN_TYPE_REF`.
    NN_PERMITS_CLAUSE = 0x3A,
    /// Enum constant declaration (JLS 8.9.1).
    NN_ENUM_CONSTANT = 0x3B,
    /// Body of an anonymous class: `new T() { ... }` or a constant-specific enum body.
    NN_ANON_CLASS_BODY = 0x3C,
    NN_SYNTHETIC = 0x7F,

    // Language Specific (Java Extensions)
    NN_JAVA_STATIC_INIT = 0x80,
    NN_JAVA_INSTANCE_INIT = 0x81,
    NN_JAVA_ASSERT_STMT = 0x82,
    NN_JAVA_LABELED_STMT = 0x83,
    NN_JAVA_SYNCHRONIZED = 0x84,
    /// `module m { ... }` declaration of a `module-info.java` compilation unit.
    NN_JAVA_MODULE_DECL = 0x85,
    /// One `requires` / `exports` / `opens` / `uses` / `provides` directive of a module.
    NN_JAVA_MODULE_DIRECTIVE = 0x86,
}

impl ASTNodeType {
    #[inline]
    pub fn as_u8(self) -> u8 {
        self as u8
    }
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
            0x35 => ASTNodeType::NN_PACKAGE_DECL,
            0x36 => ASTNodeType::NN_IMPORT_DECL,
            0x39 => ASTNodeType::NN_THROWS_CLAUSE,
            0x3A => ASTNodeType::NN_PERMITS_CLAUSE,
            0x3B => ASTNodeType::NN_ENUM_CONSTANT,
            0x3C => ASTNodeType::NN_ANON_CLASS_BODY,
            0x7F => ASTNodeType::NN_SYNTHETIC,
            0x80 => ASTNodeType::NN_JAVA_STATIC_INIT,
            0x81 => ASTNodeType::NN_JAVA_INSTANCE_INIT,
            0x82 => ASTNodeType::NN_JAVA_ASSERT_STMT,
            0x83 => ASTNodeType::NN_JAVA_LABELED_STMT,
            0x84 => ASTNodeType::NN_JAVA_SYNCHRONIZED,
            0x85 => ASTNodeType::NN_JAVA_MODULE_DECL,
            0x86 => ASTNodeType::NN_JAVA_MODULE_DIRECTIVE,
            _ => ASTNodeType::NN_UNKNOWN,
        }
    }
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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

    Inc = 0x25, // ++ (prefix or postfix; see NodeAttr::AUX_UNARY_POSTFIX)
    Dec = 0x26, // -- (prefix or postfix; see NodeAttr::AUX_UNARY_POSTFIX)

    Neg = 0x27, // unary -
    Pos = 0x28, // unary +
}

/// Syntactic position an operator symbol appears in. The same symbol maps to
/// different operators by position (`-a` is `Neg`, `a - b` is `Sub`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperatorArity {
    Unary,
    Binary,
    Assignment,
}

impl OperatorId {
    /// Maps an operator token (C-family spelling) in the given position to its id.
    /// Returns `None` when the symbol is not an operator of that position.
    pub fn from_symbol(symbol: &str, arity: OperatorArity) -> Option<OperatorId> {
        use OperatorId::*;
        let op = match arity {
            OperatorArity::Assignment => match symbol {
                "=" => Assign,
                "+=" => AddAssign,
                "-=" => SubAssign,
                "*=" => MulAssign,
                "/=" => DivAssign,
                "%=" => ModAssign,
                "&=" => AndAssign,
                "|=" => OrAssign,
                "^=" => XorAssign,
                "<<=" => ShlAssign,
                ">>=" => ShrAssign,
                ">>>=" => UshrAssign,
                _ => return Option::None,
            },
            OperatorArity::Binary => match symbol {
                "+" => Add,
                "-" => Sub,
                "*" => Mul,
                "/" => Div,
                "%" => Mod,
                "==" => Eq,
                "!=" => NotEq,
                "<" => Lt,
                ">" => Gt,
                "<=" => LtEq,
                ">=" => GtEq,
                "&&" => LogicalAnd,
                "||" => LogicalOr,
                "&" => BitwiseAnd,
                "|" => BitwiseOr,
                "^" => BitwiseXor,
                "<<" => Shl,
                ">>" => Shr,
                ">>>" => Ushr,
                _ => return Option::None,
            },
            OperatorArity::Unary => match symbol {
                "!" => LogicalNot,
                "~" => BitwiseNot,
                "-" => Neg,
                "+" => Pos,
                "++" => Inc,
                "--" => Dec,
                _ => return Option::None,
            },
        };
        Some(op)
    }

    /// True for the relational and equality operators that path conditions are built from.
    pub fn is_comparison(self) -> bool {
        matches!(
            self,
            OperatorId::Eq
                | OperatorId::NotEq
                | OperatorId::Lt
                | OperatorId::Gt
                | OperatorId::LtEq
                | OperatorId::GtEq
        )
    }
}

impl TryFrom<u8> for OperatorId {
    /// The unrecognised code is returned so callers can report it.
    type Error = u8;

    fn try_from(val: u8) -> Result<Self, Self::Error> {
        use OperatorId::*;
        Ok(match val {
            0x00 => None,
            0x01 => Assign,
            0x02 => AddAssign,
            0x03 => SubAssign,
            0x04 => MulAssign,
            0x05 => DivAssign,
            0x06 => ModAssign,
            0x07 => AndAssign,
            0x08 => OrAssign,
            0x09 => XorAssign,
            0x0A => ShlAssign,
            0x0B => ShrAssign,
            0x0C => UshrAssign,
            0x10 => Add,
            0x11 => Sub,
            0x12 => Mul,
            0x13 => Div,
            0x14 => Mod,
            0x15 => Eq,
            0x16 => NotEq,
            0x17 => Lt,
            0x18 => Gt,
            0x19 => LtEq,
            0x1A => GtEq,
            0x1B => LogicalAnd,
            0x1C => LogicalOr,
            0x1D => LogicalNot,
            0x1E => BitwiseAnd,
            0x1F => BitwiseOr,
            0x20 => BitwiseXor,
            0x21 => BitwiseNot,
            0x22 => Shl,
            0x23 => Shr,
            0x24 => Ushr,
            0x25 => Inc,
            0x26 => Dec,
            0x27 => Neg,
            0x28 => Pos,
            other => return Err(other),
        })
    }
}

/// Helper utilities for encoding and decoding the 32-bit NodeAttr word.
///
/// Layout (MSB to LSB):
/// ```text
/// bits 31..28  visibility   (VISIBILITY_*)
/// bits 27..20  modifiers    (MOD_*)
/// bits 19..12  operator id  (OperatorId)
/// bits 11..8   aux flags    (AUX_*, meaning depends on the node type)
/// bits  7..0   lang flags   (LANG_* for declarations; per-node-type payload otherwise)
/// ```
pub struct NodeAttr;

impl NodeAttr {
    /// No visibility applies (locals, parameters, expressions, local classes).
    pub const VISIBILITY_NONE: u8 = 0x00;
    pub const VISIBILITY_PUBLIC: u8 = 0x01;
    pub const VISIBILITY_PRIVATE: u8 = 0x02;
    pub const VISIBILITY_PROTECTED: u8 = 0x03;
    /// Declared without an access modifier where package access applies (JLS 6.6.1).
    pub const VISIBILITY_PACKAGE_PRIVATE: u8 = 0x04;

    pub const MOD_STATIC: u8 = 1 << 7;
    pub const MOD_FINAL: u8 = 1 << 6;
    pub const MOD_ABSTRACT: u8 = 1 << 5;
    pub const MOD_SYNCHRONIZED: u8 = 1 << 4;
    pub const MOD_NATIVE: u8 = 1 << 3;
    pub const MOD_VOLATILE: u8 = 1 << 2;
    pub const MOD_TRANSIENT: u8 = 1 << 1;
    pub const MOD_SEALED: u8 = 1 << 0;

    // ── Lang flags for declarations (Java) ──
    pub const LANG_JAVA_STRICTFP: u8 = 1 << 0;
    /// `default` interface method modifier.
    pub const LANG_JAVA_DEFAULT: u8 = 1 << 1;
    pub const LANG_JAVA_NON_SEALED: u8 = 1 << 2;
    /// Method with a trailing `T...` parameter, or the variable-arity parameter itself.
    pub const LANG_JAVA_VARARGS: u8 = 1 << 3;
    /// The visibility was implied by the JLS (interface members, enum constructors, ...),
    /// not written in source.
    pub const LANG_IMPLICIT_VISIBILITY: u8 = 1 << 4;
    /// At least one modifier bit was implied by the JLS rather than written in source.
    pub const LANG_IMPLICIT_MODIFIERS: u8 = 1 << 5;
    /// The declaration carries one or more annotations.
    pub const LANG_HAS_ANNOTATIONS: u8 = 1 << 6;
    /// The declaration is annotated `@Deprecated` / `@java.lang.Deprecated`.
    pub const LANG_DEPRECATED: u8 = 1 << 7;

    // ── Aux flags (4 bits); interpretation depends on the node type ──
    /// NN_IMPORT_DECL: `import static`.
    pub const AUX_IMPORT_STATIC: u8 = 1 << 0;
    /// NN_IMPORT_DECL: on-demand import (`.*`).
    pub const AUX_IMPORT_ON_DEMAND: u8 = 1 << 1;

    /// NN_TYPE_REF: array type; lang flags hold the dimension count.
    pub const AUX_TYPE_ARRAY: u8 = 1 << 0;
    /// NN_TYPE_REF: parameterized type; children are the head type then the type arguments.
    pub const AUX_TYPE_GENERIC: u8 = 1 << 1;
    /// NN_TYPE_REF: qualified name (`java.util.Date`, `Map.Entry`); its token range covers the name.
    pub const AUX_TYPE_QUALIFIED: u8 = 1 << 2;
    /// NN_TYPE_REF: wildcard type argument; lang flags hold the bound kind (WILDCARD_*).
    pub const AUX_TYPE_WILDCARD: u8 = 1 << 3;

    /// NN_SWITCH_CASE: arrow form (`case X ->`), no fall-through.
    pub const AUX_CASE_ARROW: u8 = 1 << 0;
    /// NN_SWITCH_CASE: has a `default` label.
    pub const AUX_CASE_DEFAULT: u8 = 1 << 1;
    /// NN_SWITCH_CASE: has a pattern label or a guard.
    pub const AUX_CASE_PATTERN: u8 = 1 << 2;
    /// NN_SWITCH_STMT / NN_SWITCH_EXPR: the body consists of arrow rules.
    pub const AUX_SWITCH_ARROW_FORM: u8 = 1 << 0;

    /// NN_TRY_STMT: try-with-resources; the resources (`NN_LOCAL_VAR_DECL` for declared
    /// resources, the variable / field reference otherwise) precede the protected block.
    pub const AUX_TRY_WITH_RESOURCES: u8 = 1 << 0;

    /// NN_CONSTRUCTOR_DECL: compact canonical record constructor.
    pub const AUX_CTOR_COMPACT: u8 = 1 << 0;

    /// NN_CALL_EXPR: explicit constructor invocation statement (`this(..)` / `super(..)`).
    pub const AUX_CALL_EXPLICIT_CTOR: u8 = 1 << 0;
    /// NN_CALL_EXPR: the explicit constructor invocation targets the superclass (`super(..)`).
    pub const AUX_CALL_SUPER_TARGET: u8 = 1 << 1;
    /// NN_CALL_EXPR: the call has an explicit receiver (`obj.m()`, `outer.super(..)`).
    pub const AUX_CALL_QUALIFIED: u8 = 1 << 2;

    /// NN_PARAM_DECL: record component (JLS 8.10.1).
    pub const AUX_PARAM_RECORD_COMPONENT: u8 = 1 << 0;
    /// NN_PARAM_DECL: exception parameter of a catch clause.
    pub const AUX_PARAM_CATCH: u8 = 1 << 1;
    /// NN_PARAM_DECL: implicitly typed lambda parameter.
    pub const AUX_PARAM_LAMBDA_INFERRED: u8 = 1 << 2;

    /// NN_UNARY_EXPR: postfix `x++` / `x--`.
    pub const AUX_UNARY_POSTFIX: u8 = 1 << 0;

    /// NN_METHOD_DECL: annotation type element (`String value() default "";`).
    pub const AUX_METHOD_ANNOTATION_ELEMENT: u8 = 1 << 0;
    /// NN_METHOD_DECL / NN_CONSTRUCTOR_DECL: has a body.
    pub const AUX_METHOD_HAS_BODY: u8 = 1 << 1;

    /// NN_FIELD_DECL: interface or annotation-type constant declaration.
    pub const AUX_FIELD_CONSTANT: u8 = 1 << 0;

    /// NN_IDENTIFIER_EXPR: qualified name (`a.b.c` in package, import or annotation names).
    pub const AUX_IDENT_QUALIFIED: u8 = 1 << 0;

    /// NN_INSTANCEOF_EXPR: declares a pattern binding variable.
    pub const AUX_INSTANCEOF_PATTERN: u8 = 1 << 0;

    /// NN_LAMBDA_EXPR: expression body (not a block).
    pub const AUX_LAMBDA_EXPRESSION_BODY: u8 = 1 << 0;

    /// NN_NEW_EXPR: has an anonymous class body.
    pub const AUX_NEW_ANON_BODY: u8 = 1 << 0;
    /// NN_NEW_EXPR: qualified instance creation (`outer.new Inner()`).
    pub const AUX_NEW_QUALIFIED: u8 = 1 << 1;

    /// NN_METHOD_REF: constructor reference (`T::new`).
    pub const AUX_METHOD_REF_CONSTRUCTOR: u8 = 1 << 0;

    // ── Lang-byte payloads for non-declaration node types ──
    /// NN_TYPE_REF with AUX_TYPE_WILDCARD: `?`.
    pub const WILDCARD_UNBOUNDED: u8 = 0;
    /// NN_TYPE_REF with AUX_TYPE_WILDCARD: `? extends T`.
    pub const WILDCARD_EXTENDS: u8 = 1;
    /// NN_TYPE_REF with AUX_TYPE_WILDCARD: `? super T`.
    pub const WILDCARD_SUPER: u8 = 2;

    /// NN_LITERAL lang byte: literal category.
    pub const LITERAL_INTEGER: u8 = 1;
    pub const LITERAL_FLOAT: u8 = 2;
    pub const LITERAL_STRING: u8 = 3;
    pub const LITERAL_CHAR: u8 = 4;
    pub const LITERAL_BOOLEAN: u8 = 5;
    pub const LITERAL_NULL: u8 = 6;
    pub const LITERAL_TEXT_BLOCK: u8 = 7;
    /// `Foo.class`; the child NN_TYPE_REF names the type.
    pub const LITERAL_CLASS: u8 = 8;

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

    /// Raw operator code stored in the attribute word.
    pub fn unpack_operator_code(attr: u32) -> u8 {
        ((attr >> 12) & 0xFF) as u8
    }

    /// Decodes the operator id. A code that names no operator (a corrupt or foreign
    /// attribute word) is reported as `Err(code)`.
    pub fn try_unpack_operator_id(attr: u32) -> Result<OperatorId, u8> {
        OperatorId::try_from(Self::unpack_operator_code(attr))
    }

    /// Decodes the operator id; codes that name no operator decode to `OperatorId::None`.
    /// Use `try_unpack_operator_id` to distinguish "no operator" from "invalid code".
    pub fn unpack_operator_id(attr: u32) -> OperatorId {
        Self::try_unpack_operator_id(attr).unwrap_or(OperatorId::None)
    }

    pub fn unpack_aux_flags(attr: u32) -> u8 {
        ((attr >> 8) & 0x0F) as u8
    }

    pub fn unpack_lang_flags(attr: u32) -> u8 {
        (attr & 0xFF) as u8
    }
}
