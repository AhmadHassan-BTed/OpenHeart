//! Kotlin AST Reduction Adapter implementation.
//!
//! Maps tree-sitter-kotlin (fwcd grammar, 0.3.x) concrete syntax trees onto the
//! language-neutral BP AST node set. The grammar declares no fields, so every decision is made
//! from node kinds, anonymous keyword / operator children and, where a construct's role depends
//! on its position (a `type_identifier` naming a class versus referencing a type, a `statements`
//! list forming a block or merely filling one), from the parent node.
//!
//! Shapes produced (children in source order):
//! - classifier: `[NN_ANNOTATION_USE*, NN_IDENTIFIER_EXPR name, NN_TYPE_PARAM*,
//!   NN_CONSTRUCTOR_DECL primary?, NN_TYPE_REF supertype* | NN_CALL_EXPR super-call*, members*]`.
//!   Supertype references carry `KotlinAttr::TYPE_SUPERTYPE` (plus `TYPE_SUPERCLASS_CALL` for
//!   `B()` and `TYPE_DELEGATED` for `I by d`); the `B(args)` invocation itself is an
//!   `NN_CALL_EXPR` with `AUX_CALL_EXPLICIT_CTOR | AUX_CALL_SUPER_TARGET`.
//! - function: `[NN_ANNOTATION_USE*, NN_TYPE_PARAM*, NN_TYPE_REF receiver?, NN_IDENTIFIER_EXPR
//!   name, (NN_PARAM_DECL default-expr?)*, NN_TYPE_REF return?, where-clause*, NN_BLOCK?]`.
//!   A default value is the expression that directly follows its parameter
//!   (`KotlinAttr::AUX_PARAM_HAS_DEFAULT`). A where-clause entry is an `NN_IDENTIFIER_EXPR`
//!   naming the type parameter followed by its bound `NN_TYPE_REF` (`TYPE_WHERE_BOUND`).
//!   Member functions are `NN_METHOD_DECL`; top-level and local functions are free functions,
//!   `NN_FUNCTION_DECL`. An expression body `= e` is an `NN_BLOCK` flagged
//!   `BLOCK_EXPRESSION_BODY` whose single child is `e`, the value the function returns.
//! - property: `NN_PROPERTY_DECL` (member and top-level) or `NN_LOCAL_VAR_DECL` (local), with
//!   `[NN_ANNOTATION_USE*, NN_TYPE_PARAM*, NN_TYPE_REF receiver?, NN_LOCAL_VAR_DECL
//!   declarator+, initializer-or-delegate?, NN_METHOD_DECL getter?, NN_METHOD_DECL setter?]`;
//!   each declarator is `[NN_IDENTIFIER_EXPR, NN_TYPE_REF?]` (more than one for destructuring).
//!   An `NN_METHOD_DECL` whose parent is a property is an accessor; its first keyword token
//!   (`get` / `set`) tells which.
//! - `when` is `NN_SWITCH_STMT` in statement position and `NN_SWITCH_EXPR` elsewhere, always in
//!   arrow form; each entry is `NN_SWITCH_CASE [conditions*, NN_BLOCK body]`.
//! - `if` / `try` / `for` / `while` / `do` keep their statement kinds; every control-structure
//!   body is an `NN_BLOCK` (`BLOCK_IMPLICIT` when written without braces), and the hidden
//!   braces of `try` / `catch` / `finally` / `init` / secondary-constructor / lambda bodies are
//!   recovered from their `statements` list. `for` is always `NN_ENHANCED_FOR`.
//! - a statement label `outer@` is an `NN_IDENTIFIER_EXPR` flagged `IDENT_LABEL` that labels its
//!   next sibling; `l@ expr` in expression position is `NN_JAVA_LABELED_STMT [label, expr]`.
//! - `a.b(c)` is `NN_CALL_EXPR [a, NN_IDENTIFIER_EXPR b, c]` (`AUX_CALL_QUALIFIED`); `a.b` is
//!   `NN_FIELD_ACCESS`; `x::f` is `NN_METHOD_REF`; `T::class` is `NN_LITERAL` (`LITERAL_CLASS`);
//!   `a to b` (infix call) is `NN_CALL_EXPR [a, to, b]` flagged `CALL_INFIX`. Constructor
//!   invocations are syntactically calls (`Foo(1)`); Phase 3 binds them to the class.
//! - Kotlin-only operators keep a C-family `OperatorId` where one applies (`===` is `Eq` plus
//!   `BINARY_IDENTITY_EQ`) and are otherwise identified by the lang payload (`in`, `..`, `?:`).
//!
//! String templates: Phase 1 emits a string literal as one token (the grammar hides the
//! fragments), so the interpolations inside it have no tokens of their own and are not
//! reduced; the literal carries `AUX_LITERAL_TEMPLATE`.
//!
//! Syntax errors: ERROR nodes are eliminated (their recoverable children attach to the
//! enclosing node) and MISSING nodes are dropped (they have no source text); both are counted,
//! and so are named kinds absent from the grammar's `node-types.json`.

use std::sync::atomic::{AtomicU64, Ordering};

use tree_sitter::Node;

use crate::ast::adapter::{ASTReductionAdapter, ReductionDecision};
use crate::core::types::ast::{ASTNodeType, NodeAttr, OperatorArity, OperatorId};
use crate::lang::kotlin::semantics::KotlinModifierSet;

/// Kotlin-specific payloads of the `NodeAttr` word.
///
/// Declaration lang bits 4..7 keep their shared meaning (`NodeAttr::LANG_IMPLICIT_VISIBILITY`,
/// `LANG_IMPLICIT_MODIFIERS`, `LANG_HAS_ANNOTATIONS`, `LANG_DEPRECATED`); bits 0..3 and the
/// aux nibble are interpreted per node type as listed here. Bits that also exist for Java keep
/// the Java value (`*_VARARGS` = `LANG_JAVA_VARARGS`, `AUX_PARAM_PROPERTY` =
/// `AUX_PARAM_RECORD_COMPONENT`, `AUX_METHOD_HAS_BODY`, `AUX_PARAM_LAMBDA_INFERRED`), so arity
/// and body checks written for Java apply unchanged.
pub struct KotlinAttr;

impl KotlinAttr {
    /// `internal` (module-wide) visibility, in the 4-bit visibility field.
    pub const VISIBILITY_INTERNAL: u8 = 0x05;

    // ── Classifiers: NN_CLASS_DECL / NN_INTERFACE_DECL / NN_ENUM_DECL / NN_ANNOTATION_DECL ──
    /// `data class` / `data object`.
    pub const LANG_CLASS_DATA: u8 = 1 << 0;
    /// `value class` (inline class).
    pub const LANG_CLASS_VALUE: u8 = 1 << 1;
    /// `inner class` (holds a reference to the outer instance; not static on the JVM).
    pub const LANG_CLASS_INNER: u8 = 1 << 2;
    /// `fun interface` (single abstract method interface).
    pub const LANG_INTERFACE_FUN: u8 = 1 << 3;
    /// `object` declaration (singleton), including companion objects.
    pub const AUX_CLASS_OBJECT: u8 = 1 << 0;
    /// `companion object`.
    pub const AUX_CLASS_COMPANION: u8 = 1 << 1;
    /// `expect` (multiplatform declaration without platform implementation).
    pub const AUX_DECL_EXPECT: u8 = 1 << 2;
    /// `actual` (platform implementation of an `expect` declaration).
    pub const AUX_DECL_ACTUAL: u8 = 1 << 3;

    // ── Functions: NN_FUNCTION_DECL / NN_METHOD_DECL (accessors included) ──
    /// `suspend fun`.
    pub const LANG_FUN_SUSPEND: u8 = 1 << 0;
    /// `inline fun` / `inline get()`.
    pub const LANG_FUN_INLINE: u8 = 1 << 1;
    /// `operator fun`.
    pub const LANG_FUN_OPERATOR: u8 = 1 << 2;
    /// A parameter is `vararg`.
    pub const LANG_FUN_VARARGS: u8 = NodeAttr::LANG_JAVA_VARARGS;
    /// Extension function (`fun T.f()`); the receiver is the `NN_TYPE_REF` before the name.
    pub const AUX_FUN_EXTENSION: u8 = 1 << 2;
    /// `override fun`.
    pub const AUX_FUN_OVERRIDE: u8 = 1 << 3;

    // ── Constructors: NN_CONSTRUCTOR_DECL ──
    /// Primary constructor (class header); its parameters may declare properties.
    pub const LANG_CTOR_PRIMARY: u8 = 1 << 0;
    /// Secondary constructor delegating with `: this(...)`.
    pub const LANG_CTOR_DELEGATES_THIS: u8 = 1 << 1;
    /// Secondary constructor delegating with `: super(...)`.
    pub const LANG_CTOR_DELEGATES_SUPER: u8 = 1 << 2;
    /// A parameter is `vararg`.
    pub const LANG_CTOR_VARARGS: u8 = NodeAttr::LANG_JAVA_VARARGS;

    // ── Properties and locals: NN_PROPERTY_DECL / NN_LOCAL_VAR_DECL ──
    /// `var` (mutable). Read-only `val` declarations carry `MOD_FINAL` instead.
    pub const LANG_VAR_MUTABLE: u8 = 1 << 0;
    /// The property declares a getter (`get() = ...`, or an annotated / restricted `get`).
    pub const LANG_PROP_CUSTOM_GETTER: u8 = 1 << 1;
    /// The property declares a setter.
    pub const LANG_PROP_CUSTOM_SETTER: u8 = 1 << 2;
    /// `override val` / `override var`.
    pub const LANG_PROP_OVERRIDE: u8 = 1 << 3;
    /// `const val` (compile-time constant).
    pub const AUX_PROP_CONST: u8 = 1 << 0;
    /// `lateinit var`.
    pub const AUX_PROP_LATEINIT: u8 = 1 << 1;
    /// Delegated property (`by lazy { }`, `by Delegates.observable(...)`, `by map`); the
    /// delegate expression is the child that follows the declarator.
    pub const AUX_PROP_DELEGATED: u8 = 1 << 2;
    /// Extension property (`val T.p`); the receiver is the `NN_TYPE_REF` before the declarator.
    pub const AUX_PROP_EXTENSION: u8 = 1 << 3;

    // ── Parameters: NN_PARAM_DECL ──
    /// `var` class parameter (mutable property).
    pub const LANG_PARAM_MUTABLE: u8 = 1 << 0;
    /// `noinline` lambda parameter of an inline function.
    pub const LANG_PARAM_NOINLINE: u8 = 1 << 1;
    /// `crossinline` lambda parameter of an inline function.
    pub const LANG_PARAM_CROSSINLINE: u8 = 1 << 2;
    /// `vararg` parameter.
    pub const LANG_PARAM_VARARG: u8 = NodeAttr::LANG_JAVA_VARARGS;
    /// Class parameter declared with `val` / `var`: it also declares a property (the JVM shape
    /// of a record component: private field plus accessor).
    pub const AUX_PARAM_PROPERTY: u8 = NodeAttr::AUX_PARAM_RECORD_COMPONENT;
    /// The parameter has a default value.
    pub const AUX_PARAM_HAS_DEFAULT: u8 = 1 << 3;

    // ── Type parameters: NN_TYPE_PARAM ──
    /// `reified T` (inline functions).
    pub const LANG_TYPE_PARAM_REIFIED: u8 = 1 << 0;
    /// Declaration-site contravariance `in T`.
    pub const LANG_TYPE_PARAM_IN: u8 = 1 << 1;
    /// Declaration-site covariance `out T`.
    pub const LANG_TYPE_PARAM_OUT: u8 = 1 << 2;

    // ── Type references: NN_TYPE_REF lang byte when neither AUX_TYPE_ARRAY nor
    //    AUX_TYPE_WILDCARD is set (use-site variance `out T` / `in T` / `*` is encoded as a
    //    wildcard with WILDCARD_EXTENDS / WILDCARD_SUPER / WILDCARD_UNBOUNDED) ──
    /// `T?`.
    pub const TYPE_NULLABLE: u8 = 1 << 0;
    /// Function type `(A) -> B` / `R.(A) -> B`.
    pub const TYPE_FUNCTION: u8 = 1 << 1;
    /// `suspend (A) -> B`.
    pub const TYPE_SUSPEND_FUNCTION: u8 = 1 << 2;
    /// Definitely non-nullable type `T & Any`.
    pub const TYPE_DEFINITELY_NON_NULL: u8 = 1 << 3;
    /// Entry of a classifier's supertype list.
    pub const TYPE_SUPERTYPE: u8 = 1 << 4;
    /// Supertype invoked with a constructor call (`: Base(x)`): the superclass.
    pub const TYPE_SUPERCLASS_CALL: u8 = 1 << 5;
    /// Supertype implemented by delegation (`: I by impl`).
    pub const TYPE_DELEGATED: u8 = 1 << 6;
    /// Upper bound in a `where` clause.
    pub const TYPE_WHERE_BOUND: u8 = 1 << 7;

    // ── Blocks: NN_BLOCK ──
    /// Expression body `= e` of a function or accessor; the child is the returned value.
    pub const BLOCK_EXPRESSION_BODY: u8 = 1 << 0;
    /// Control-structure body written without braces.
    pub const BLOCK_IMPLICIT: u8 = 1 << 1;

    // ── NN_IF_STMT / NN_TRY_STMT ──
    /// Used as an expression (its value is consumed).
    pub const EXPR_VALUE_USED: u8 = 1 << 0;
    // ── NN_SWITCH_STMT / NN_SWITCH_EXPR (`when`) ──
    /// `when (subject)`; the subject is the first child.
    pub const WHEN_HAS_SUBJECT: u8 = 1 << 0;
    /// `when (val x = e)`; the first child is the `NN_LOCAL_VAR_DECL` of `x`.
    pub const WHEN_SUBJECT_BINDING: u8 = 1 << 1;

    // ── NN_RETURN_STMT / NN_BREAK_STMT / NN_CONTINUE_STMT ──
    /// `return@l` / `break@l` / `continue@l`; the label is the first child.
    pub const JUMP_LABELED: u8 = 1 << 0;
    // ── NN_THIS_EXPR / NN_SUPER_EXPR ──
    /// `this@Outer` / `super@Outer`.
    pub const RECEIVER_LABELED: u8 = 1 << 0;
    /// `super<Interface>`.
    pub const SUPER_TYPE_QUALIFIED: u8 = 1 << 1;
    // ── NN_IDENTIFIER_EXPR ──
    /// A label: `name@` in front of a statement or lambda, or the target of a labeled jump.
    pub const IDENT_LABEL: u8 = 1 << 0;

    // ── NN_BINARY_EXPR lang payload (Kotlin-only operators) ──
    /// `a in b` (`b.contains(a)`); a `when` range condition has the subject implied.
    pub const BINARY_IN: u8 = 1;
    /// `a !in b`.
    pub const BINARY_NOT_IN: u8 = 2;
    /// `a..b` (`rangeTo`).
    pub const BINARY_RANGE_TO: u8 = 3;
    /// `a..<b` (`rangeUntil`).
    pub const BINARY_RANGE_UNTIL: u8 = 4;
    /// `a ?: b`.
    pub const BINARY_ELVIS: u8 = 5;
    /// `a === b` (operator id `Eq`).
    pub const BINARY_IDENTITY_EQ: u8 = 6;
    /// `a !== b` (operator id `NotEq`).
    pub const BINARY_IDENTITY_NE: u8 = 7;
    // ── NN_UNARY_EXPR lang payload ──
    /// `a!!`.
    pub const UNARY_NOT_NULL_ASSERT: u8 = 1;
    /// Spread `*array` in an argument list.
    pub const UNARY_SPREAD: u8 = 2;
    // ── NN_INSTANCEOF_EXPR / NN_CAST_EXPR lang payload ──
    /// `!is`.
    pub const INSTANCEOF_NEGATED: u8 = 1;
    /// `as?`.
    pub const CAST_SAFE: u8 = 1;

    // ── NN_CALL_EXPR / NN_FIELD_ACCESS / NN_METHOD_REF ──
    /// Infix call `a f b`.
    pub const CALL_INFIX: u8 = 1 << 0;
    /// Safe call / access `a?.b`.
    pub const SAFE_ACCESS: u8 = 1 << 1;
    /// The last argument is a trailing lambda `f(x) { ... }`.
    pub const CALL_TRAILING_LAMBDA: u8 = 1 << 2;
    /// Superclass constructor invocation in a classifier header (`: Base(x)`).
    pub const CALL_SUPERTYPE_INIT: u8 = 1 << 3;
    /// Bound reference `expr::member`.
    pub const METHOD_REF_BOUND: u8 = 1 << 0;

    // ── NN_LAMBDA_EXPR ──
    /// Anonymous function `fun(x: Int) = ...`.
    pub const LAMBDA_ANONYMOUS_FUNCTION: u8 = 1 << 0;
    /// Labeled lambda `l@{ ... }`.
    pub const LAMBDA_LABELED: u8 = 1 << 1;
    /// Declares its parameters (`{ a, b -> }`); otherwise a single parameter is the implicit `it`.
    pub const LAMBDA_EXPLICIT_PARAMS: u8 = 1 << 2;

    // ── NN_IMPORT_DECL ──
    /// `import a.B as C`; the last child is the alias.
    pub const IMPORT_ALIASED: u8 = 1 << 0;

    // ── NN_LITERAL aux (the lang byte keeps the NodeAttr::LITERAL_* category) ──
    /// String template with `$name` / `${expr}` interpolations.
    pub const AUX_LITERAL_TEMPLATE: u8 = 1 << 0;
    /// `L` suffix.
    pub const AUX_LITERAL_LONG: u8 = 1 << 1;
    /// `u` / `U` suffix.
    pub const AUX_LITERAL_UNSIGNED: u8 = 1 << 2;

    // ── NN_ANNOTATION_USE lang payload: use-site target ──
    pub const TARGET_NONE: u8 = 0;
    pub const TARGET_FILE: u8 = 1;
    pub const TARGET_PROPERTY: u8 = 2;
    pub const TARGET_FIELD: u8 = 3;
    pub const TARGET_GET: u8 = 4;
    pub const TARGET_SET: u8 = 5;
    pub const TARGET_RECEIVER: u8 = 6;
    pub const TARGET_PARAM: u8 = 7;
    pub const TARGET_SETPARAM: u8 = 8;
    pub const TARGET_DELEGATE: u8 = 9;
}

/// Counters of the nodes the reducer could not map structurally, aggregated over every file
/// reduced through one adapter instance.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KotlinReductionStats {
    /// ERROR nodes eliminated (their children were reduced in place).
    pub error_nodes: u64,
    /// MISSING nodes dropped.
    pub missing_nodes: u64,
    /// Named nodes of a kind the grammar's `node-types.json` does not list (eliminated).
    pub unclassified_nodes: u64,
}

pub struct KotlinASTReductionAdapter {
    error_nodes: AtomicU64,
    missing_nodes: AtomicU64,
    unclassified_nodes: AtomicU64,
}

impl Default for KotlinASTReductionAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl KotlinASTReductionAdapter {
    pub fn new() -> Self {
        Self {
            error_nodes: AtomicU64::new(0),
            missing_nodes: AtomicU64::new(0),
            unclassified_nodes: AtomicU64::new(0),
        }
    }

    /// ERROR / MISSING / unclassified node counts accumulated so far.
    pub fn stats(&self) -> KotlinReductionStats {
        KotlinReductionStats {
            error_nodes: self.error_nodes.load(Ordering::Relaxed),
            missing_nodes: self.missing_nodes.load(Ordering::Relaxed),
            unclassified_nodes: self.unclassified_nodes.load(Ordering::Relaxed),
        }
    }
}

impl ASTReductionAdapter for KotlinASTReductionAdapter {
    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_kotlin::language()
    }

    fn classify(&self, kind: &str, node: &Node, _depth: usize) -> ReductionDecision {
        use ASTNodeType::*;
        use ReductionDecision::*;

        let node = *node;
        if node.is_missing() {
            self.missing_nodes.fetch_add(1, Ordering::Relaxed);
            return Drop;
        }
        if !node.is_named() {
            // Anonymous tokens are structure, except the two that are values on their own.
            return match kind {
                "null" => Keep(NN_LITERAL),
                "dynamic" => Keep(NN_TYPE_REF),
                _ => Drop,
            };
        }

        match kind {
            "ERROR" => {
                self.error_nodes.fetch_add(1, Ordering::Relaxed);
                Eliminate
            }

            // Compilation unit, directives, annotations
            "source_file" => Keep(NN_MODULE),
            "package_header" => Keep(NN_PACKAGE_DECL),
            "import_list" | "import_alias" => Eliminate,
            "import_header" => Keep(NN_IMPORT_DECL),
            "identifier" => {
                if count_named_children_of_kind(node, "simple_identifier") > 1 {
                    Keep(NN_IDENTIFIER_EXPR)
                } else {
                    Eliminate
                }
            }
            "annotation" | "file_annotation" => Keep(NN_ANNOTATION_USE),
            "use_site_target" | "shebang_line" | "line_comment" | "multiline_comment" => Drop,

            // Classifiers
            "class_declaration" => Keep(classifier_flavor(node).node_type()),
            "object_declaration" | "companion_object" => Keep(NN_CLASS_DECL),
            "type_alias" => Keep(NN_TYPEDEF_DECL),
            "class_body" => match parent_kind(node) {
                Some("enum_entry") | Some("object_literal") => Keep(NN_ANON_CLASS_BODY),
                _ => Eliminate,
            },
            "enum_class_body" => Eliminate,
            "enum_entry" => Keep(NN_ENUM_CONSTANT),
            "delegation_specifier" | "explicit_delegation" => Eliminate,
            "constructor_invocation" => {
                if parent_kind(node) == Some("delegation_specifier") {
                    Keep(NN_CALL_EXPR)
                } else {
                    Eliminate
                }
            }
            "primary_constructor" | "secondary_constructor" => Keep(NN_CONSTRUCTOR_DECL),
            "class_parameter" => Keep(NN_PARAM_DECL),
            "constructor_delegation_call" => Keep(NN_CALL_EXPR),
            "anonymous_initializer" => Keep(NN_JAVA_INSTANCE_INIT),

            // Functions and properties
            "function_declaration" => match owner_of(node) {
                Owner::TopLevel | Owner::Local => Keep(NN_FUNCTION_DECL),
                _ => Keep(NN_METHOD_DECL),
            },
            "getter" | "setter" => Keep(NN_METHOD_DECL),
            "function_value_parameters" | "parameter_modifiers" | "lambda_parameters" => {
                Eliminate
            }
            "parameter" => {
                if parent_kind(node) == Some("function_type_parameters") {
                    // `(name: T) -> R`: the name documents the function type, it declares nothing.
                    Eliminate
                } else {
                    Keep(NN_PARAM_DECL)
                }
            }
            "parameter_with_optional_type" => Keep(NN_PARAM_DECL),
            "function_body" => Keep(NN_BLOCK),
            "property_declaration" => match owner_of(node) {
                Owner::Local => Keep(NN_LOCAL_VAR_DECL),
                _ => Keep(NN_PROPERTY_DECL),
            },
            "property_delegate" => Eliminate,
            "variable_declaration" => match parent_kind(node) {
                Some("lambda_parameters") => Keep(NN_PARAM_DECL),
                _ => Keep(NN_LOCAL_VAR_DECL),
            },
            "multi_variable_declaration" => match parent_kind(node) {
                Some("property_declaration") => Eliminate,
                Some("lambda_parameters") => Keep(NN_PARAM_DECL),
                _ => Keep(NN_LOCAL_VAR_DECL),
            },

            // Generics and modifiers
            "type_parameters" | "type_parameter_modifiers" | "type_constraints"
            | "type_constraint" | "modifiers" | "type_modifiers" => Eliminate,
            "type_parameter" => Keep(NN_TYPE_PARAM),
            "class_modifier" | "member_modifier" | "visibility_modifier"
            | "function_modifier" | "property_modifier" | "inheritance_modifier"
            | "parameter_modifier" | "platform_modifier" | "variance_modifier"
            | "reification_modifier" | "type_projection_modifiers" => Drop,

            // Types
            "user_type" => {
                if is_simple_user_type(node) {
                    Eliminate
                } else {
                    Keep(NN_TYPE_REF)
                }
            }
            "type_identifier" => match type_identifier_role(node) {
                TypeIdentRole::Name => Keep(NN_IDENTIFIER_EXPR),
                TypeIdentRole::Reference(_) => Keep(NN_TYPE_REF),
            },
            "nullable_type" | "parenthesized_type" | "parenthesized_user_type" => Eliminate,
            "not_nullable_type" | "function_type" => Keep(NN_TYPE_REF),
            "function_type_parameters" | "type_arguments" => Eliminate,
            "type_projection" => {
                if is_wildcard_projection(node) {
                    Keep(NN_TYPE_REF)
                } else {
                    Eliminate
                }
            }

            // Blocks and statements
            "statements" => match parent_kind(node) {
                Some("function_body") | Some("control_structure_body") | Some("source_file") => {
                    Eliminate
                }
                _ => Keep(NN_BLOCK),
            },
            "control_structure_body" => Keep(NN_BLOCK),
            "label" => Keep(NN_IDENTIFIER_EXPR),
            "for_statement" => Keep(NN_ENHANCED_FOR),
            "while_statement" => Keep(NN_WHILE_STMT),
            "do_while_statement" => Keep(NN_DO_WHILE_STMT),
            "if_expression" => Keep(NN_IF_STMT),
            "when_expression" => {
                if value_is_used(node) {
                    Keep(NN_SWITCH_EXPR)
                } else {
                    Keep(NN_SWITCH_STMT)
                }
            }
            "when_subject" | "when_condition" => Eliminate,
            "when_entry" => Keep(NN_SWITCH_CASE),
            "range_test" => Keep(NN_BINARY_EXPR),
            "type_test" => Keep(NN_INSTANCEOF_EXPR),
            "try_expression" => Keep(NN_TRY_STMT),
            "catch_block" => Keep(NN_CATCH_CLAUSE),
            "finally_block" => Keep(NN_FINALLY_CLAUSE),
            "jump_expression" => match first_child_kind(node) {
                Some("throw") => Keep(NN_THROW_STMT),
                Some("return") | Some("return@") => Keep(NN_RETURN_STMT),
                Some("continue") | Some("continue@") => Keep(NN_CONTINUE_STMT),
                Some("break") | Some("break@") => Keep(NN_BREAK_STMT),
                _ => Eliminate,
            },
            "assignment" => Keep(NN_ASSIGN_EXPR),
            "directly_assignable_expression" => {
                if first_named_child_of_kind(node, "navigation_suffix").is_some() {
                    Keep(NN_FIELD_ACCESS)
                } else if first_named_child_of_kind(node, "indexing_suffix").is_some() {
                    Keep(NN_ARRAY_ACCESS)
                } else {
                    Eliminate
                }
            }

            // Expressions
            "additive_expression"
            | "multiplicative_expression"
            | "comparison_expression"
            | "equality_expression"
            | "conjunction_expression"
            | "disjunction_expression"
            | "elvis_expression"
            | "range_expression" => Keep(NN_BINARY_EXPR),
            "check_expression" => match operator_kind(node) {
                Some("is") | Some("!is") => Keep(NN_INSTANCEOF_EXPR),
                _ => Keep(NN_BINARY_EXPR),
            },
            "infix_expression" => Keep(NN_CALL_EXPR),
            "prefix_expression" => match node.child(0).map(|c| (c.kind(), c.is_named())) {
                Some(("label", true)) => Keep(NN_JAVA_LABELED_STMT),
                Some(("annotation", true)) => Eliminate,
                _ => Keep(NN_UNARY_EXPR),
            },
            "postfix_expression" | "spread_expression" => Keep(NN_UNARY_EXPR),
            "as_expression" => Keep(NN_CAST_EXPR),
            "call_expression" => Keep(NN_CALL_EXPR),
            "call_suffix" | "value_arguments" | "value_argument" | "annotated_lambda"
            | "navigation_suffix" | "indexing_suffix" | "parenthesized_expression" => Eliminate,
            "navigation_expression" => match navigation_role(node) {
                NavigationRole::Callee => Eliminate,
                NavigationRole::FieldAccess { .. } => Keep(NN_FIELD_ACCESS),
                NavigationRole::BoundReference => Keep(NN_METHOD_REF),
                NavigationRole::ClassLiteral => Keep(NN_LITERAL),
            },
            "indexing_expression" => Keep(NN_ARRAY_ACCESS),
            "callable_reference" => {
                if has_anonymous_child(node, "class") {
                    Keep(NN_LITERAL)
                } else {
                    Keep(NN_METHOD_REF)
                }
            }
            "lambda_literal" | "anonymous_function" => Keep(NN_LAMBDA_EXPR),
            "object_literal" => Keep(NN_NEW_EXPR),
            "collection_literal" => Keep(NN_ARRAY_INIT),
            "this_expression" => Keep(NN_THIS_EXPR),
            "super_expression" => Keep(NN_SUPER_EXPR),
            "simple_identifier" => {
                if is_named_argument_name(node) {
                    // `f(name = v)`: the name refers to a parameter of the callee, not to a value.
                    Drop
                } else {
                    Keep(NN_IDENTIFIER_EXPR)
                }
            }

            // Literals
            "integer_literal" | "hex_literal" | "bin_literal" | "real_literal"
            | "character_literal" | "string_literal" | "boolean_literal" => Keep(NN_LITERAL),
            // `10L` / `7u`: the inner digits are the literal (same first token).
            "long_literal" | "unsigned_literal" => Eliminate,
            // Pieces of atomic literals: no tokens of their own in the corpus.
            "interpolated_identifier" | "interpolated_expression" | "character_escape_seq" => {
                Drop
            }

            _ => {
                self.unclassified_nodes.fetch_add(1, Ordering::Relaxed);
                Eliminate
            }
        }
    }

    fn encode_attrs(&self, kind: &str, node: &Node, source: &[u8]) -> u32 {
        let node = *node;
        if node.is_missing() {
            return 0;
        }
        if !node.is_named() {
            return match kind {
                "null" => literal_attr(NodeAttr::LITERAL_NULL, 0),
                _ => 0,
            };
        }

        match kind {
            "class_declaration" | "object_declaration" | "companion_object" => {
                encode_classifier(node, kind, source)
            }
            "type_alias" => encode_type_alias(node, source),
            "function_declaration" => encode_function(node, source),
            "getter" | "setter" => encode_accessor(node, source),
            "primary_constructor" | "secondary_constructor" => {
                encode_constructor(node, kind, source)
            }
            "property_declaration" => encode_property(node, source),
            "variable_declaration" | "multi_variable_declaration" => encode_variable(node, source),
            "class_parameter" => encode_class_parameter(node, source),
            "parameter" | "parameter_with_optional_type" => encode_parameter(node, kind, source),
            "type_parameter" => encode_type_parameter(node, source),
            "enum_entry" => encode_enum_entry(node, source),
            "annotation" | "file_annotation" => encode_annotation(node, kind),
            "import_header" => {
                let aux = if has_anonymous_child(node, ".*") {
                    NodeAttr::AUX_IMPORT_ON_DEMAND
                } else {
                    0
                };
                let lang = if first_named_child_of_kind(node, "import_alias").is_some() {
                    KotlinAttr::IMPORT_ALIASED
                } else {
                    0
                };
                pack(NodeAttr::VISIBILITY_NONE, 0, OperatorId::None, aux, lang)
            }
            "identifier" => pack(
                NodeAttr::VISIBILITY_NONE,
                0,
                OperatorId::None,
                NodeAttr::AUX_IDENT_QUALIFIED,
                0,
            ),
            "label" => pack(
                NodeAttr::VISIBILITY_NONE,
                0,
                OperatorId::None,
                0,
                KotlinAttr::IDENT_LABEL,
            ),
            "type_identifier" => match type_identifier_role(node) {
                TypeIdentRole::Name => 0,
                TypeIdentRole::Reference(flags) => {
                    pack(NodeAttr::VISIBILITY_NONE, 0, OperatorId::None, 0, flags)
                }
            },
            "user_type" | "function_type" | "not_nullable_type" | "type_projection" => {
                encode_type_ref(node, kind)
            }
            "function_body" => {
                let lang = if first_child_kind(node) == Some("=") {
                    KotlinAttr::BLOCK_EXPRESSION_BODY
                } else {
                    0
                };
                pack(NodeAttr::VISIBILITY_NONE, 0, OperatorId::None, 0, lang)
            }
            "control_structure_body" => {
                let lang = if first_child_kind(node) == Some("{") {
                    0
                } else {
                    KotlinAttr::BLOCK_IMPLICIT
                };
                pack(NodeAttr::VISIBILITY_NONE, 0, OperatorId::None, 0, lang)
            }
            "if_expression" | "try_expression" => {
                let lang = if value_is_used(node) {
                    KotlinAttr::EXPR_VALUE_USED
                } else {
                    0
                };
                pack(NodeAttr::VISIBILITY_NONE, 0, OperatorId::None, 0, lang)
            }
            "when_expression" => encode_when(node),
            "when_entry" => encode_when_entry(node),
            "jump_expression" => {
                let labeled = first_child_kind(node).is_some_and(|k| k.ends_with('@'));
                let lang = if labeled && first_child_kind(node) != Some("throw") {
                    KotlinAttr::JUMP_LABELED
                } else {
                    0
                };
                pack(NodeAttr::VISIBILITY_NONE, 0, OperatorId::None, 0, lang)
            }
            "assignment" => {
                let op = operator_kind(node)
                    .and_then(|s| OperatorId::from_symbol(s, OperatorArity::Assignment))
                    .unwrap_or(OperatorId::None);
                pack(NodeAttr::VISIBILITY_NONE, 0, op, 0, 0)
            }
            "additive_expression"
            | "multiplicative_expression"
            | "comparison_expression"
            | "conjunction_expression"
            | "disjunction_expression" => {
                let op = operator_kind(node)
                    .and_then(|s| OperatorId::from_symbol(s, OperatorArity::Binary))
                    .unwrap_or(OperatorId::None);
                pack(NodeAttr::VISIBILITY_NONE, 0, op, 0, 0)
            }
            "equality_expression" => {
                let (op, lang) = match operator_kind(node) {
                    Some("==") => (OperatorId::Eq, 0),
                    Some("!=") => (OperatorId::NotEq, 0),
                    Some("===") => (OperatorId::Eq, KotlinAttr::BINARY_IDENTITY_EQ),
                    Some("!==") => (OperatorId::NotEq, KotlinAttr::BINARY_IDENTITY_NE),
                    _ => (OperatorId::None, 0),
                };
                pack(NodeAttr::VISIBILITY_NONE, 0, op, 0, lang)
            }
            "elvis_expression" => pack(
                NodeAttr::VISIBILITY_NONE,
                0,
                OperatorId::None,
                0,
                KotlinAttr::BINARY_ELVIS,
            ),
            "range_expression" => {
                let lang = match operator_kind(node) {
                    Some("..<") => KotlinAttr::BINARY_RANGE_UNTIL,
                    Some("..") => KotlinAttr::BINARY_RANGE_TO,
                    _ => 0,
                };
                pack(NodeAttr::VISIBILITY_NONE, 0, OperatorId::None, 0, lang)
            }
            "check_expression" | "range_test" | "type_test" => {
                let lang = match operator_kind(node) {
                    Some("in") => KotlinAttr::BINARY_IN,
                    Some("!in") => KotlinAttr::BINARY_NOT_IN,
                    Some("!is") => KotlinAttr::INSTANCEOF_NEGATED,
                    _ => 0,
                };
                pack(NodeAttr::VISIBILITY_NONE, 0, OperatorId::None, 0, lang)
            }
            "infix_expression" => pack(
                NodeAttr::VISIBILITY_NONE,
                0,
                OperatorId::None,
                NodeAttr::AUX_CALL_QUALIFIED,
                KotlinAttr::CALL_INFIX,
            ),
            "prefix_expression" => {
                let op = node
                    .child(0)
                    .filter(|c| !c.is_named())
                    .and_then(|c| OperatorId::from_symbol(c.kind(), OperatorArity::Unary))
                    .unwrap_or(OperatorId::None);
                pack(NodeAttr::VISIBILITY_NONE, 0, op, 0, 0)
            }
            "postfix_expression" => {
                let (op, lang) = match operator_kind(node) {
                    Some("++") => (OperatorId::Inc, 0),
                    Some("--") => (OperatorId::Dec, 0),
                    Some("!!") => (OperatorId::None, KotlinAttr::UNARY_NOT_NULL_ASSERT),
                    _ => (OperatorId::None, 0),
                };
                pack(
                    NodeAttr::VISIBILITY_NONE,
                    0,
                    op,
                    NodeAttr::AUX_UNARY_POSTFIX,
                    lang,
                )
            }
            "spread_expression" => pack(
                NodeAttr::VISIBILITY_NONE,
                0,
                OperatorId::None,
                0,
                KotlinAttr::UNARY_SPREAD,
            ),
            "as_expression" => {
                let lang = if operator_kind(node) == Some("as?") {
                    KotlinAttr::CAST_SAFE
                } else {
                    0
                };
                pack(NodeAttr::VISIBILITY_NONE, 0, OperatorId::None, 0, lang)
            }
            "call_expression" => encode_call(node),
            "constructor_invocation" => pack(
                NodeAttr::VISIBILITY_NONE,
                0,
                OperatorId::None,
                NodeAttr::AUX_CALL_EXPLICIT_CTOR | NodeAttr::AUX_CALL_SUPER_TARGET,
                KotlinAttr::CALL_SUPERTYPE_INIT,
            ),
            "constructor_delegation_call" => {
                let mut aux = NodeAttr::AUX_CALL_EXPLICIT_CTOR;
                if has_anonymous_child(node, "super") {
                    aux |= NodeAttr::AUX_CALL_SUPER_TARGET;
                }
                pack(NodeAttr::VISIBILITY_NONE, 0, OperatorId::None, aux, 0)
            }
            "navigation_expression" => match navigation_role(node) {
                NavigationRole::FieldAccess { safe } => {
                    let lang = if safe { KotlinAttr::SAFE_ACCESS } else { 0 };
                    pack(NodeAttr::VISIBILITY_NONE, 0, OperatorId::None, 0, lang)
                }
                NavigationRole::BoundReference => pack(
                    NodeAttr::VISIBILITY_NONE,
                    0,
                    OperatorId::None,
                    0,
                    KotlinAttr::METHOD_REF_BOUND,
                ),
                NavigationRole::ClassLiteral => literal_attr(NodeAttr::LITERAL_CLASS, 0),
                NavigationRole::Callee => 0,
            },
            "directly_assignable_expression" => {
                let safe = first_named_child_of_kind(node, "navigation_suffix")
                    .is_some_and(|s| member_access_operator(s) == Some("?."));
                let lang = if safe { KotlinAttr::SAFE_ACCESS } else { 0 };
                pack(NodeAttr::VISIBILITY_NONE, 0, OperatorId::None, 0, lang)
            }
            "callable_reference" => {
                if has_anonymous_child(node, "class") {
                    literal_attr(NodeAttr::LITERAL_CLASS, 0)
                } else {
                    0
                }
            }
            "lambda_literal" => {
                let mut lang = 0u8;
                if first_named_child_of_kind(node, "lambda_parameters").is_some() {
                    lang |= KotlinAttr::LAMBDA_EXPLICIT_PARAMS;
                }
                let labeled = node
                    .parent()
                    .filter(|p| p.kind() == "annotated_lambda")
                    .is_some_and(|p| first_named_child_of_kind(p, "label").is_some());
                if labeled {
                    lang |= KotlinAttr::LAMBDA_LABELED;
                }
                pack(NodeAttr::VISIBILITY_NONE, 0, OperatorId::None, 0, lang)
            }
            "anonymous_function" => pack(
                NodeAttr::VISIBILITY_NONE,
                0,
                OperatorId::None,
                0,
                KotlinAttr::LAMBDA_ANONYMOUS_FUNCTION | KotlinAttr::LAMBDA_EXPLICIT_PARAMS,
            ),
            "object_literal" => pack(
                NodeAttr::VISIBILITY_NONE,
                0,
                OperatorId::None,
                NodeAttr::AUX_NEW_ANON_BODY,
                0,
            ),
            "this_expression" => {
                let lang = if first_named_child_of_kind(node, "type_identifier").is_some() {
                    KotlinAttr::RECEIVER_LABELED
                } else {
                    0
                };
                pack(NodeAttr::VISIBILITY_NONE, 0, OperatorId::None, 0, lang)
            }
            "super_expression" => {
                let mut lang = 0u8;
                if has_anonymous_child(node, "super@") {
                    lang |= KotlinAttr::RECEIVER_LABELED;
                }
                let mut cursor = node.walk();
                let qualified = node
                    .named_children(&mut cursor)
                    .any(|c| is_type_kind(c.kind()));
                if qualified {
                    lang |= KotlinAttr::SUPER_TYPE_QUALIFIED;
                }
                pack(NodeAttr::VISIBILITY_NONE, 0, OperatorId::None, 0, lang)
            }
            "integer_literal" | "hex_literal" | "bin_literal" => {
                let mut aux = 0u8;
                match parent_kind(node) {
                    Some("long_literal") => aux |= KotlinAttr::AUX_LITERAL_LONG,
                    Some("unsigned_literal") => {
                        aux |= KotlinAttr::AUX_LITERAL_UNSIGNED;
                        let long_suffix = node
                            .parent()
                            .and_then(|p| node_text(p, source))
                            .is_some_and(|t| t.ends_with('L'));
                        if long_suffix {
                            aux |= KotlinAttr::AUX_LITERAL_LONG;
                        }
                    }
                    _ => {}
                }
                literal_attr(NodeAttr::LITERAL_INTEGER, aux)
            }
            "real_literal" => literal_attr(NodeAttr::LITERAL_FLOAT, 0),
            "character_literal" => literal_attr(NodeAttr::LITERAL_CHAR, 0),
            "boolean_literal" => literal_attr(NodeAttr::LITERAL_BOOLEAN, 0),
            "string_literal" => {
                let category = if node_text(node, source).is_some_and(|t| t.starts_with("\"\"\""))
                {
                    NodeAttr::LITERAL_TEXT_BLOCK
                } else {
                    NodeAttr::LITERAL_STRING
                };
                let mut cursor = node.walk();
                let template = node.named_children(&mut cursor).any(|c| {
                    matches!(c.kind(), "interpolated_identifier" | "interpolated_expression")
                });
                let aux = if template {
                    KotlinAttr::AUX_LITERAL_TEMPLATE
                } else {
                    0
                };
                literal_attr(category, aux)
            }
            _ => 0,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────────────────
// Node helpers
// ─────────────────────────────────────────────────────────────────────────────────────────

#[inline]
fn pack(vis: u8, mods: u8, op: OperatorId, aux: u8, lang: u8) -> u32 {
    NodeAttr::pack(vis, mods, op, aux, lang)
}

#[inline]
fn literal_attr(category: u8, aux: u8) -> u32 {
    pack(NodeAttr::VISIBILITY_NONE, 0, OperatorId::None, aux, category)
}

fn node_text<'s>(node: Node, source: &'s [u8]) -> Option<&'s str> {
    source
        .get(node.byte_range())
        .and_then(|bytes| std::str::from_utf8(bytes).ok())
}

fn parent_kind(node: Node) -> Option<&'static str> {
    node.parent().map(|p| p.kind())
}

fn first_child_kind(node: Node) -> Option<&'static str> {
    node.child(0).map(|c| c.kind())
}

fn first_named_child_of_kind<'t>(node: Node<'t>, kind: &str) -> Option<Node<'t>> {
    let mut cursor = node.walk();
    let found = node.named_children(&mut cursor).find(|c| c.kind() == kind);
    found
}

fn count_named_children_of_kind(node: Node, kind: &str) -> usize {
    let mut cursor = node.walk();
    let count = node
        .named_children(&mut cursor)
        .filter(|c| c.kind() == kind)
        .count();
    count
}

fn has_anonymous_child(node: Node, kind: &str) -> bool {
    let mut cursor = node.walk();
    let found = node
        .children(&mut cursor)
        .any(|c| !c.is_named() && c.kind() == kind);
    found
}

/// Spelling of the first anonymous child that is an operator of a binary / unary / assignment
/// production (the grammar gives operators no field; each such production has exactly one).
fn operator_kind(node: Node) -> Option<&'static str> {
    let mut cursor = node.walk();
    let found = node
        .children(&mut cursor)
        .filter(|c| !c.is_named())
        .map(|c| c.kind())
        .find(|k| !matches!(*k, "(" | ")" | "[" | "]" | "{" | "}" | "," | ";"));
    found
}

/// Kinds that a `_type` position can hold.
fn is_type_kind(kind: &str) -> bool {
    matches!(
        kind,
        "user_type"
            | "nullable_type"
            | "function_type"
            | "parenthesized_type"
            | "not_nullable_type"
            | "type_modifiers"
    )
}

/// `.` / `?.` / `::` of a `navigation_suffix`.
fn member_access_operator(suffix: Node) -> Option<&'static str> {
    let mut cursor = suffix.walk();
    let found = suffix
        .children(&mut cursor)
        .filter(|c| !c.is_named())
        .map(|c| c.kind())
        .find(|k| matches!(*k, "." | "?." | "::"));
    found
}

/// The (last) `navigation_suffix` of a `navigation_expression`.
fn navigation_suffix(node: Node) -> Option<Node> {
    let mut cursor = node.walk();
    let found = node
        .named_children(&mut cursor)
        .filter(|c| c.kind() == "navigation_suffix")
        .last();
    found
}

/// Last named child that is not a comment (comments are named `extras` and may trail).
fn last_significant_named_child(node: Node) -> Option<Node> {
    let mut cursor = node.walk();
    let found = node
        .named_children(&mut cursor)
        .filter(|c| !matches!(c.kind(), "line_comment" | "multiline_comment"))
        .last();
    found
}

/// A declaration's syntactic owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Owner {
    TopLevel,
    Local,
    Class,
    Enum,
    Annotation,
    Interface,
    Object,
    Companion,
    AnonymousObject,
}

impl Owner {
    fn is_classifier(self) -> bool {
        !matches!(self, Owner::TopLevel | Owner::Local)
    }

    fn is_object(self) -> bool {
        matches!(
            self,
            Owner::Object | Owner::Companion | Owner::AnonymousObject
        )
    }
}

fn owner_of(decl: Node) -> Owner {
    let parent = match decl.parent() {
        Some(p) => p,
        None => return Owner::TopLevel,
    };
    match parent.kind() {
        "source_file" => Owner::TopLevel,
        // `.kts` scripts and error recovery may wrap top-level statements.
        "statements" => {
            if parent_kind(parent) == Some("source_file") {
                Owner::TopLevel
            } else {
                Owner::Local
            }
        }
        "class_body" | "enum_class_body" => match parent.parent() {
            Some(classifier) => match classifier.kind() {
                "class_declaration" => match classifier_flavor(classifier) {
                    ClassifierFlavor::Class => Owner::Class,
                    ClassifierFlavor::Interface { .. } => Owner::Interface,
                    ClassifierFlavor::Enum => Owner::Enum,
                    ClassifierFlavor::Annotation => Owner::Annotation,
                },
                "object_declaration" => Owner::Object,
                "companion_object" => Owner::Companion,
                "object_literal" | "enum_entry" => Owner::AnonymousObject,
                _ => Owner::Local,
            },
            None => Owner::Local,
        },
        _ => Owner::Local,
    }
}

/// Which declaration a `class_declaration` node is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClassifierFlavor {
    Class,
    Interface { fun: bool },
    Enum,
    Annotation,
}

impl ClassifierFlavor {
    fn node_type(self) -> ASTNodeType {
        match self {
            ClassifierFlavor::Class => ASTNodeType::NN_CLASS_DECL,
            ClassifierFlavor::Interface { .. } => ASTNodeType::NN_INTERFACE_DECL,
            ClassifierFlavor::Enum => ASTNodeType::NN_ENUM_DECL,
            ClassifierFlavor::Annotation => ASTNodeType::NN_ANNOTATION_DECL,
        }
    }
}

fn classifier_flavor(node: Node) -> ClassifierFlavor {
    if has_anonymous_child(node, "interface") {
        return ClassifierFlavor::Interface {
            fun: has_anonymous_child(node, "fun"),
        };
    }
    if has_anonymous_child(node, "enum")
        || first_named_child_of_kind(node, "enum_class_body").is_some()
    {
        return ClassifierFlavor::Enum;
    }
    let keywords = modifier_keywords(first_named_child_of_kind(node, "modifiers"));
    if keywords.contains(KotlinModifierSet::ENUM) {
        ClassifierFlavor::Enum
    } else if keywords.contains(KotlinModifierSet::ANNOTATION) {
        ClassifierFlavor::Annotation
    } else {
        ClassifierFlavor::Class
    }
}

/// Modifier keywords of a `modifiers` / `parameter_modifiers` / `type_parameter_modifiers`
/// node, read from the anonymous keyword tokens (whose kind is their spelling).
fn modifier_keywords(modifiers: Option<Node>) -> KotlinModifierSet {
    let mut set = KotlinModifierSet::empty();
    let modifiers = match modifiers {
        Some(m) => m,
        None => return set,
    };
    let mut cursor = modifiers.walk();
    for child in modifiers.named_children(&mut cursor) {
        match child.kind() {
            "annotation" => {}
            "property_modifier" => set.insert(KotlinModifierSet::CONST),
            "reification_modifier" => set.insert(KotlinModifierSet::REIFIED),
            _ => {
                let mut inner = child.walk();
                for token in child.children(&mut inner) {
                    if !token.is_named() {
                        if let Some(bit) = KotlinModifierSet::from_keyword(token.kind()) {
                            set.insert(bit);
                        }
                    }
                }
            }
        }
    }
    set
}

/// Modifiers of a declaration: keywords plus the annotations that change JVM semantics.
#[derive(Debug, Clone, Copy, Default)]
struct DeclModifiers {
    keywords: KotlinModifierSet,
    annotated: bool,
    deprecated: bool,
    jvm_static: bool,
    volatile: bool,
    transient: bool,
    synchronized: bool,
}

impl DeclModifiers {
    fn has(&self, bit: u32) -> bool {
        self.keywords.contains(bit)
    }

    /// Shared declaration lang bits 4..7.
    fn shared_lang_bits(&self, implicit_visibility: bool, implicit_modifiers: bool) -> u8 {
        let mut lang = 0u8;
        if implicit_visibility {
            lang |= NodeAttr::LANG_IMPLICIT_VISIBILITY;
        }
        if implicit_modifiers {
            lang |= NodeAttr::LANG_IMPLICIT_MODIFIERS;
        }
        if self.annotated {
            lang |= NodeAttr::LANG_HAS_ANNOTATIONS;
        }
        if self.deprecated {
            lang |= NodeAttr::LANG_DEPRECATED;
        }
        lang
    }

    /// JVM modifier bits carried by annotations.
    fn annotation_mod_bits(&self) -> u8 {
        let mut mods = 0u8;
        if self.jvm_static {
            mods |= NodeAttr::MOD_STATIC;
        }
        if self.volatile {
            mods |= NodeAttr::MOD_VOLATILE;
        }
        if self.transient {
            mods |= NodeAttr::MOD_TRANSIENT;
        }
        if self.synchronized {
            mods |= NodeAttr::MOD_SYNCHRONIZED;
        }
        mods
    }
}

fn decl_modifiers(modifiers: Option<Node>, source: &[u8]) -> DeclModifiers {
    let mut out = DeclModifiers {
        keywords: modifier_keywords(modifiers),
        ..DeclModifiers::default()
    };
    if let Some(m) = modifiers {
        let mut cursor = m.walk();
        for child in m.named_children(&mut cursor) {
            if child.kind() == "annotation" {
                note_annotation(&mut out, child, source);
            }
        }
    }
    out
}

/// Records an annotation and the JVM semantics its well-known names carry.
fn note_annotation(out: &mut DeclModifiers, annotation: Node, source: &[u8]) {
    out.annotated = true;
    for name in annotation_names(annotation, source) {
        if annotation_is(&name, "Deprecated", &["kotlin.Deprecated", "java.lang.Deprecated"]) {
            out.deprecated = true;
        } else if annotation_is(&name, "JvmStatic", &["kotlin.jvm.JvmStatic"]) {
            out.jvm_static = true;
        } else if annotation_is(
            &name,
            "Volatile",
            &["kotlin.jvm.Volatile", "kotlin.concurrent.Volatile"],
        ) {
            out.volatile = true;
        } else if annotation_is(&name, "Transient", &["kotlin.jvm.Transient"]) {
            out.transient = true;
        } else if annotation_is(&name, "Synchronized", &["kotlin.jvm.Synchronized"]) {
            out.synchronized = true;
        }
    }
}

fn annotation_is(name: &str, simple: &str, qualified: &[&str]) -> bool {
    name == simple || qualified.contains(&name)
}

/// Annotation class names of an `annotation` node (`@A`, `@A(...)`, `@field:A`, `@[A B(1)]`),
/// without type arguments and whitespace.
fn annotation_names(annotation: Node, source: &[u8]) -> Vec<String> {
    let mut names = Vec::new();
    let mut cursor = annotation.walk();
    for child in annotation.named_children(&mut cursor) {
        let user_type = match child.kind() {
            "user_type" => Some(child),
            "constructor_invocation" => first_named_child_of_kind(child, "user_type"),
            _ => None,
        };
        if let Some(t) = user_type.and_then(|t| node_text(t, source)) {
            let head = t.split('<').next().unwrap_or(t);
            let name: String = head.chars().filter(|c| !c.is_whitespace()).collect();
            if !name.is_empty() {
                names.push(name);
            }
        }
    }
    names
}

/// Explicit visibility, or the language default for the declaration's position.
fn visibility(mods: &DeclModifiers, default: u8) -> (u8, bool) {
    let kw = mods.keywords;
    if kw.contains(KotlinModifierSet::PRIVATE) {
        (NodeAttr::VISIBILITY_PRIVATE, false)
    } else if kw.contains(KotlinModifierSet::PROTECTED) {
        (NodeAttr::VISIBILITY_PROTECTED, false)
    } else if kw.contains(KotlinModifierSet::INTERNAL) {
        (KotlinAttr::VISIBILITY_INTERNAL, false)
    } else if kw.contains(KotlinModifierSet::PUBLIC) {
        (NodeAttr::VISIBILITY_PUBLIC, false)
    } else {
        (default, default != NodeAttr::VISIBILITY_NONE)
    }
}

/// Default visibility: `public` everywhere a visibility applies (Kotlin spec, "Declaration
/// visibility"); local declarations have none.
fn default_visibility(owner: Owner) -> u8 {
    match owner {
        Owner::Local => NodeAttr::VISIBILITY_NONE,
        _ => NodeAttr::VISIBILITY_PUBLIC,
    }
}

fn expect_actual_aux(mods: &DeclModifiers) -> u8 {
    let mut aux = 0u8;
    if mods.has(KotlinModifierSet::EXPECT) {
        aux |= KotlinAttr::AUX_DECL_EXPECT;
    }
    if mods.has(KotlinModifierSet::ACTUAL) {
        aux |= KotlinAttr::AUX_DECL_ACTUAL;
    }
    aux
}

/// Extension declarations write `Receiver.name`: the only direct `.` child of a function or
/// property declaration separates the receiver type from the name.
fn is_extension(decl: Node) -> bool {
    has_anonymous_child(decl, ".")
}

/// True when the value of `node` is consumed. Kotlin `if`, `when` and `try` are expressions:
/// in statement position their value is discarded, except where the statement is itself the
/// value of an enclosing construct (last statement of a lambda, of a used `try` / `catch`
/// block, or the body of a used `if` branch / `when` entry, as in `else if` chains).
fn value_is_used(node: Node) -> bool {
    let parent = match node.parent() {
        Some(p) => p,
        None => return false,
    };
    match parent.kind() {
        "source_file" => false,
        "statements" => {
            let is_last = last_significant_named_child(parent).is_some_and(|l| l.id() == node.id());
            if !is_last {
                return false;
            }
            match parent.parent() {
                Some(owner) => match owner.kind() {
                    "lambda_literal" => true,
                    "try_expression" => value_is_used(owner),
                    "catch_block" => owner.parent().is_some_and(value_is_used),
                    "control_structure_body" => body_value_is_used(owner),
                    _ => false,
                },
                None => false,
            }
        }
        "control_structure_body" => body_value_is_used(parent),
        _ => true,
    }
}

/// Whether the value of a `control_structure_body` is the value of a used `if` / `when`.
fn body_value_is_used(body: Node) -> bool {
    match body.parent() {
        Some(owner) if owner.kind() == "if_expression" => value_is_used(owner),
        Some(owner) if owner.kind() == "when_entry" => owner.parent().is_some_and(value_is_used),
        _ => false,
    }
}

/// `f(name = value)`: the argument name precedes `=` inside a `value_argument`.
fn is_named_argument_name(node: Node) -> bool {
    if parent_kind(node) != Some("value_argument") {
        return false;
    }
    node.next_sibling()
        .is_some_and(|s| !s.is_named() && s.kind() == "=")
}

fn is_simple_user_type(user_type: Node) -> bool {
    user_type.named_child_count() == 1
        && user_type
            .named_child(0)
            .is_some_and(|c| c.kind() == "type_identifier")
}

fn is_wildcard_projection(projection: Node) -> bool {
    has_anonymous_child(projection, "*")
        || first_named_child_of_kind(projection, "type_projection_modifiers").is_some()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TypeIdentRole {
    /// Declares or labels something (class, object, alias, type parameter, import alias,
    /// `this@L`, where-clause subject): an `NN_IDENTIFIER_EXPR`.
    Name,
    /// Refers to a type: an `NN_TYPE_REF` with these lang flags.
    Reference(u8),
}

fn type_identifier_role(node: Node) -> TypeIdentRole {
    let parent = match node.parent() {
        Some(p) => p,
        None => return TypeIdentRole::Reference(0),
    };
    match parent.kind() {
        "class_declaration" | "object_declaration" | "companion_object" | "type_alias"
        | "type_parameter" | "import_alias" | "this_expression" | "super_expression"
        | "type_constraint" => TypeIdentRole::Name,
        "user_type" => {
            if count_named_children_of_kind(parent, "type_identifier") > 1 {
                // A segment of a qualified name: the enclosing NN_TYPE_REF spans the name.
                TypeIdentRole::Name
            } else if is_simple_user_type(parent) {
                TypeIdentRole::Reference(type_context_flags(parent))
            } else {
                // Head of a parameterized type; the enclosing NN_TYPE_REF carries the flags.
                TypeIdentRole::Reference(0)
            }
        }
        _ => TypeIdentRole::Reference(type_context_flags(node)),
    }
}

/// Flags a type reference inherits from the type positions that enclose it.
fn type_context_flags(start: Node) -> u8 {
    let mut flags = 0u8;
    let mut current = start;
    while let Some(parent) = current.parent() {
        match parent.kind() {
            "nullable_type" => flags |= KotlinAttr::TYPE_NULLABLE,
            "parenthesized_type" | "parenthesized_user_type" => {}
            "delegation_specifier" => {
                flags |= KotlinAttr::TYPE_SUPERTYPE;
                break;
            }
            "constructor_invocation" => {
                if parent_kind(parent) == Some("delegation_specifier") {
                    flags |= KotlinAttr::TYPE_SUPERTYPE | KotlinAttr::TYPE_SUPERCLASS_CALL;
                }
                break;
            }
            "explicit_delegation" => {
                flags |= KotlinAttr::TYPE_SUPERTYPE | KotlinAttr::TYPE_DELEGATED;
                break;
            }
            "type_constraint" => {
                flags |= KotlinAttr::TYPE_WHERE_BOUND;
                break;
            }
            _ => break,
        }
        current = parent;
    }
    flags
}

fn encode_type_ref(node: Node, kind: &str) -> u32 {
    match kind {
        "type_projection" => {
            let bound = if has_anonymous_child(node, "*") {
                NodeAttr::WILDCARD_UNBOUNDED
            } else {
                let modifiers = first_named_child_of_kind(node, "type_projection_modifiers");
                let keywords = variance_keywords(modifiers);
                if keywords.contains(KotlinModifierSet::OUT) {
                    NodeAttr::WILDCARD_EXTENDS
                } else if keywords.contains(KotlinModifierSet::IN) {
                    NodeAttr::WILDCARD_SUPER
                } else {
                    NodeAttr::WILDCARD_UNBOUNDED
                }
            };
            pack(
                NodeAttr::VISIBILITY_NONE,
                0,
                OperatorId::None,
                NodeAttr::AUX_TYPE_WILDCARD,
                bound,
            )
        }
        "user_type" => {
            let mut aux = 0u8;
            if first_named_child_of_kind(node, "type_arguments").is_some() {
                aux |= NodeAttr::AUX_TYPE_GENERIC;
            }
            if count_named_children_of_kind(node, "type_identifier") > 1 {
                aux |= NodeAttr::AUX_TYPE_QUALIFIED;
            }
            pack(
                NodeAttr::VISIBILITY_NONE,
                0,
                OperatorId::None,
                aux,
                type_context_flags(node),
            )
        }
        "function_type" => {
            let mut lang = KotlinAttr::TYPE_FUNCTION | type_context_flags(node);
            let suspend = node
                .prev_named_sibling()
                .filter(|s| s.kind() == "type_modifiers")
                .is_some_and(|s| has_anonymous_child(s, "suspend"));
            if suspend {
                lang |= KotlinAttr::TYPE_SUSPEND_FUNCTION;
            }
            pack(NodeAttr::VISIBILITY_NONE, 0, OperatorId::None, 0, lang)
        }
        "not_nullable_type" => pack(
            NodeAttr::VISIBILITY_NONE,
            0,
            OperatorId::None,
            0,
            KotlinAttr::TYPE_DEFINITELY_NON_NULL | type_context_flags(node),
        ),
        _ => 0,
    }
}

/// `in` / `out` of a `type_projection_modifiers` / `type_parameter_modifiers` node, whose
/// `variance_modifier` children wrap the keyword token.
fn variance_keywords(modifiers: Option<Node>) -> KotlinModifierSet {
    let mut set = KotlinModifierSet::empty();
    if let Some(m) = modifiers {
        let mut cursor = m.walk();
        for child in m.named_children(&mut cursor) {
            if child.kind() == "variance_modifier" {
                match first_child_kind(child) {
                    Some("in") => set.insert(KotlinModifierSet::IN),
                    Some("out") => set.insert(KotlinModifierSet::OUT),
                    _ => {}
                }
            }
        }
    }
    set
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NavigationRole {
    /// `a.b` directly called as `a.b(...)`: the call node carries receiver and name.
    Callee,
    FieldAccess { safe: bool },
    BoundReference,
    ClassLiteral,
}

fn navigation_role(node: Node) -> NavigationRole {
    let suffix = navigation_suffix(node);
    let operator = suffix.and_then(member_access_operator);
    if operator == Some("::") {
        return if suffix.is_some_and(|s| has_anonymous_child(s, "class")) {
            NavigationRole::ClassLiteral
        } else {
            NavigationRole::BoundReference
        };
    }
    let is_callee = node
        .parent()
        .filter(|p| p.kind() == "call_expression")
        .and_then(|p| p.child(0))
        .is_some_and(|first| first.id() == node.id());
    if is_callee {
        NavigationRole::Callee
    } else {
        NavigationRole::FieldAccess {
            safe: operator == Some("?."),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────────────────
// Declaration encoders
// ─────────────────────────────────────────────────────────────────────────────────────────

fn encode_classifier(node: Node, kind: &str, source: &[u8]) -> u32 {
    let mods = decl_modifiers(first_named_child_of_kind(node, "modifiers"), source);
    let owner = owner_of(node);
    let (vis, implicit_vis) = visibility(&mods, default_visibility(owner));

    let mut m = 0u8;
    let mut implicit_mods = false;
    let mut lang = 0u8;
    let mut aux = expect_actual_aux(&mods);

    let flavor = match kind {
        "object_declaration" | "companion_object" => None,
        _ => Some(classifier_flavor(node)),
    };

    if mods.has(KotlinModifierSet::ABSTRACT) {
        m |= NodeAttr::MOD_ABSTRACT;
    }
    if mods.has(KotlinModifierSet::FINAL) {
        m |= NodeAttr::MOD_FINAL;
    }
    if mods.has(KotlinModifierSet::EXTERNAL) {
        m |= NodeAttr::MOD_NATIVE;
    }
    if mods.has(KotlinModifierSet::SEALED) {
        // A sealed class is abstract (Kotlin spec, "Sealed classes and interfaces").
        m |= NodeAttr::MOD_SEALED | NodeAttr::MOD_ABSTRACT;
        if !mods.has(KotlinModifierSet::ABSTRACT) {
            implicit_mods = true;
        }
    }

    match flavor {
        Some(ClassifierFlavor::Interface { fun }) => {
            if !mods.has(KotlinModifierSet::ABSTRACT) {
                m |= NodeAttr::MOD_ABSTRACT;
                implicit_mods = true;
            }
            if fun {
                lang |= KotlinAttr::LANG_INTERFACE_FUN;
            }
        }
        _ => {
            // Classes, enums, annotation classes and objects are final unless opened.
            let opened = mods.has(KotlinModifierSet::OPEN)
                || mods.has(KotlinModifierSet::ABSTRACT)
                || mods.has(KotlinModifierSet::SEALED)
                || mods.has(KotlinModifierSet::FINAL);
            if !opened {
                m |= NodeAttr::MOD_FINAL;
                implicit_mods = true;
            }
        }
    }

    match kind {
        "object_declaration" => aux |= KotlinAttr::AUX_CLASS_OBJECT,
        "companion_object" => aux |= KotlinAttr::AUX_CLASS_OBJECT | KotlinAttr::AUX_CLASS_COMPANION,
        _ => {}
    }
    if mods.has(KotlinModifierSet::DATA) {
        lang |= KotlinAttr::LANG_CLASS_DATA;
    }
    if mods.has(KotlinModifierSet::VALUE) {
        lang |= KotlinAttr::LANG_CLASS_VALUE;
    }
    let inner = mods.has(KotlinModifierSet::INNER);
    if inner {
        lang |= KotlinAttr::LANG_CLASS_INNER;
    }

    // A classifier nested in another classifier's body is static on the JVM unless `inner`.
    if owner.is_classifier() && !inner {
        m |= NodeAttr::MOD_STATIC;
        implicit_mods = true;
    }

    lang |= mods.shared_lang_bits(implicit_vis, implicit_mods);
    pack(vis, m, OperatorId::None, aux, lang)
}

fn encode_type_alias(node: Node, source: &[u8]) -> u32 {
    let mods = decl_modifiers(first_named_child_of_kind(node, "modifiers"), source);
    let (vis, implicit_vis) = visibility(&mods, default_visibility(owner_of(node)));
    let aux = expect_actual_aux(&mods);
    let lang = mods.shared_lang_bits(implicit_vis, false);
    pack(vis, 0, OperatorId::None, aux, lang)
}

/// True when any parameter of `params` (a `function_value_parameters` node) is `vararg`.
fn has_vararg_parameter(params: Option<Node>) -> bool {
    let params = match params {
        Some(p) => p,
        None => return false,
    };
    let mut cursor = params.walk();
    let found = params
        .named_children(&mut cursor)
        .filter(|c| c.kind() == "parameter_modifiers")
        .any(|c| modifier_keywords(Some(c)).contains(KotlinModifierSet::VARARG));
    found
}

/// Overridability of a member function or property: members are final unless `open`,
/// `abstract` or `override` (an override stays open unless marked `final`); interface members
/// are never final. Returns (MOD_* bits, implied by the language).
fn member_finality(mods: &DeclModifiers, owner: Owner) -> (u8, bool) {
    if !owner.is_classifier() || owner == Owner::Interface {
        return (0, false);
    }
    if mods.has(KotlinModifierSet::FINAL) {
        return (NodeAttr::MOD_FINAL, false);
    }
    if mods.has(KotlinModifierSet::OPEN)
        || mods.has(KotlinModifierSet::ABSTRACT)
        || mods.has(KotlinModifierSet::OVERRIDE)
    {
        return (0, false);
    }
    (NodeAttr::MOD_FINAL, true)
}

fn encode_function(node: Node, source: &[u8]) -> u32 {
    let mods = decl_modifiers(first_named_child_of_kind(node, "modifiers"), source);
    let owner = owner_of(node);
    let (vis, implicit_vis) = visibility(&mods, default_visibility(owner));
    let has_body = first_named_child_of_kind(node, "function_body").is_some();

    let mut m = mods.annotation_mod_bits();
    let mut implicit_mods = false;
    if mods.has(KotlinModifierSet::ABSTRACT) {
        m |= NodeAttr::MOD_ABSTRACT;
    } else if owner == Owner::Interface && !has_body {
        m |= NodeAttr::MOD_ABSTRACT;
        implicit_mods = true;
    }
    if mods.has(KotlinModifierSet::EXTERNAL) {
        m |= NodeAttr::MOD_NATIVE;
    }
    let (finality, implied) = member_finality(&mods, owner);
    m |= finality;
    implicit_mods |= implied;

    let mut lang = 0u8;
    if mods.has(KotlinModifierSet::SUSPEND) {
        lang |= KotlinAttr::LANG_FUN_SUSPEND;
    }
    if mods.has(KotlinModifierSet::INLINE) {
        lang |= KotlinAttr::LANG_FUN_INLINE;
    }
    if mods.has(KotlinModifierSet::OPERATOR) {
        lang |= KotlinAttr::LANG_FUN_OPERATOR;
    }
    if has_vararg_parameter(first_named_child_of_kind(node, "function_value_parameters")) {
        lang |= KotlinAttr::LANG_FUN_VARARGS;
    }
    lang |= mods.shared_lang_bits(implicit_vis, implicit_mods);

    let mut aux = 0u8;
    if has_body {
        aux |= NodeAttr::AUX_METHOD_HAS_BODY;
    }
    if is_extension(node) {
        aux |= KotlinAttr::AUX_FUN_EXTENSION;
    }
    if mods.has(KotlinModifierSet::OVERRIDE) {
        aux |= KotlinAttr::AUX_FUN_OVERRIDE;
    }
    pack(vis, m, OperatorId::None, aux, lang)
}

/// Visibility of a property declaration (accessors default to it).
fn property_visibility(property: Node, source: &[u8]) -> u8 {
    let mods = decl_modifiers(first_named_child_of_kind(property, "modifiers"), source);
    visibility(&mods, default_visibility(owner_of(property))).0
}

fn encode_accessor(node: Node, source: &[u8]) -> u32 {
    let mods = decl_modifiers(first_named_child_of_kind(node, "modifiers"), source);
    let default = match node.parent() {
        Some(p) if p.kind() == "property_declaration" => property_visibility(p, source),
        _ => NodeAttr::VISIBILITY_PUBLIC,
    };
    let (vis, implicit_vis) = visibility(&mods, default);

    let mut m = mods.annotation_mod_bits();
    if mods.has(KotlinModifierSet::EXTERNAL) {
        m |= NodeAttr::MOD_NATIVE;
    }
    let mut lang = 0u8;
    if mods.has(KotlinModifierSet::INLINE) {
        lang |= KotlinAttr::LANG_FUN_INLINE;
    }
    lang |= mods.shared_lang_bits(implicit_vis, false);
    let aux = if first_named_child_of_kind(node, "function_body").is_some() {
        NodeAttr::AUX_METHOD_HAS_BODY
    } else {
        0
    };
    pack(vis, m, OperatorId::None, aux, lang)
}

fn encode_constructor(node: Node, kind: &str, source: &[u8]) -> u32 {
    let mods = decl_modifiers(first_named_child_of_kind(node, "modifiers"), source);
    let class_node = if kind == "primary_constructor" {
        node.parent()
    } else {
        node.parent().and_then(|body| body.parent())
    };

    // Enum constructors are private and sealed-class constructors protected by default.
    let default = match class_node {
        Some(c) if c.kind() == "class_declaration" => {
            let class_mods = modifier_keywords(first_named_child_of_kind(c, "modifiers"));
            if classifier_flavor(c) == ClassifierFlavor::Enum {
                NodeAttr::VISIBILITY_PRIVATE
            } else if class_mods.contains(KotlinModifierSet::SEALED) {
                NodeAttr::VISIBILITY_PROTECTED
            } else {
                NodeAttr::VISIBILITY_PUBLIC
            }
        }
        _ => NodeAttr::VISIBILITY_PUBLIC,
    };
    let (vis, implicit_vis) = visibility(&mods, default);

    let mut lang = 0u8;
    let mut aux = 0u8;
    if kind == "primary_constructor" {
        lang |= KotlinAttr::LANG_CTOR_PRIMARY;
        let mut cursor = node.walk();
        let vararg = node.named_children(&mut cursor).any(|p| {
            p.kind() == "class_parameter"
                && modifier_keywords(first_named_child_of_kind(p, "modifiers"))
                    .contains(KotlinModifierSet::VARARG)
        });
        if vararg {
            lang |= KotlinAttr::LANG_CTOR_VARARGS;
        }
    } else {
        if let Some(call) = first_named_child_of_kind(node, "constructor_delegation_call") {
            if has_anonymous_child(call, "super") {
                lang |= KotlinAttr::LANG_CTOR_DELEGATES_SUPER;
            } else if has_anonymous_child(call, "this") {
                lang |= KotlinAttr::LANG_CTOR_DELEGATES_THIS;
            }
        }
        if has_vararg_parameter(first_named_child_of_kind(node, "function_value_parameters")) {
            lang |= KotlinAttr::LANG_CTOR_VARARGS;
        }
        // The body braces belong to the hidden `_block` rule and surface as direct children.
        if has_anonymous_child(node, "{") {
            aux |= NodeAttr::AUX_METHOD_HAS_BODY;
        }
    }
    lang |= mods.shared_lang_bits(implicit_vis, false);
    pack(vis, mods.annotation_mod_bits(), OperatorId::None, aux, lang)
}

fn encode_property(node: Node, source: &[u8]) -> u32 {
    let mods = decl_modifiers(first_named_child_of_kind(node, "modifiers"), source);
    let owner = owner_of(node);
    let (vis, implicit_vis) = visibility(&mods, default_visibility(owner));
    let mutable = has_anonymous_child(node, "var");
    let delegated = first_named_child_of_kind(node, "property_delegate").is_some();
    let getter = first_named_child_of_kind(node, "getter");
    let setter = first_named_child_of_kind(node, "setter");

    let mut m = mods.annotation_mod_bits();
    let mut implicit_mods = false;
    if !mutable {
        m |= NodeAttr::MOD_FINAL;
    }
    if mods.has(KotlinModifierSet::ABSTRACT) {
        m |= NodeAttr::MOD_ABSTRACT;
    } else if owner == Owner::Interface {
        // An interface property is abstract unless an accessor provides its value.
        let implemented = getter.is_some_and(|g| first_named_child_of_kind(g, "function_body").is_some())
            || delegated;
        if !implemented {
            m |= NodeAttr::MOD_ABSTRACT;
            implicit_mods = true;
        }
    }
    if mods.has(KotlinModifierSet::EXTERNAL) {
        m |= NodeAttr::MOD_NATIVE;
    }
    // `const val` in an object compiles to a static final field.
    if mods.has(KotlinModifierSet::CONST) && owner.is_object() {
        m |= NodeAttr::MOD_STATIC;
        implicit_mods = true;
    }

    let mut aux = 0u8;
    if mods.has(KotlinModifierSet::CONST) {
        aux |= KotlinAttr::AUX_PROP_CONST;
    }
    if mods.has(KotlinModifierSet::LATEINIT) {
        aux |= KotlinAttr::AUX_PROP_LATEINIT;
    }
    if delegated {
        aux |= KotlinAttr::AUX_PROP_DELEGATED;
    }
    if is_extension(node) {
        aux |= KotlinAttr::AUX_PROP_EXTENSION;
    }

    let mut lang = 0u8;
    if mutable {
        lang |= KotlinAttr::LANG_VAR_MUTABLE;
    }
    if getter.is_some() {
        lang |= KotlinAttr::LANG_PROP_CUSTOM_GETTER;
    }
    if setter.is_some() {
        lang |= KotlinAttr::LANG_PROP_CUSTOM_SETTER;
    }
    if mods.has(KotlinModifierSet::OVERRIDE) {
        lang |= KotlinAttr::LANG_PROP_OVERRIDE;
    }
    lang |= mods.shared_lang_bits(implicit_vis, implicit_mods);
    pack(vis, m, OperatorId::None, aux, lang)
}

/// Annotations written directly on a node (variable declarations and catch / loop variables
/// carry them as children, not inside `modifiers`).
fn direct_annotations(node: Node, source: &[u8]) -> DeclModifiers {
    let mut out = DeclModifiers::default();
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "annotation" {
            note_annotation(&mut out, child, source);
        }
    }
    out
}

fn has_type_child(node: Node) -> bool {
    let mut cursor = node.walk();
    let found = node.children(&mut cursor).any(|c| {
        (c.is_named() && is_type_kind(c.kind())) || (!c.is_named() && c.kind() == "dynamic")
    });
    found
}

fn encode_variable(node: Node, source: &[u8]) -> u32 {
    let annotations = direct_annotations(node, source);
    let parent = node.parent();
    let parent_kind = parent.map(|p| p.kind());

    if parent_kind == Some("lambda_parameters") {
        // Lambda parameters are read-only.
        let aux = if has_type_child(node) {
            0
        } else {
            NodeAttr::AUX_PARAM_LAMBDA_INFERRED
        };
        let lang = annotations.shared_lang_bits(false, true);
        return pack(
            NodeAttr::VISIBILITY_NONE,
            NodeAttr::MOD_FINAL,
            OperatorId::None,
            aux,
            lang,
        );
    }

    // A declarator takes `val` / `var` from its property declaration (directly, or through a
    // destructuring `multi_variable_declaration`); loop, `when`-subject and other bindings
    // are read-only.
    let property = match parent {
        Some(p) if p.kind() == "property_declaration" => Some(p),
        Some(p) if p.kind() == "multi_variable_declaration" => {
            p.parent().filter(|g| g.kind() == "property_declaration")
        }
        _ => None,
    };
    let mutable = property.is_some_and(|p| has_anonymous_child(p, "var"));
    let (m, lang_mut, implicit) = if mutable {
        (0, KotlinAttr::LANG_VAR_MUTABLE, false)
    } else {
        (NodeAttr::MOD_FINAL, 0, property.is_none())
    };
    let lang = lang_mut | annotations.shared_lang_bits(false, implicit);
    pack(NodeAttr::VISIBILITY_NONE, m, OperatorId::None, 0, lang)
}

fn param_modifier_lang(keywords: KotlinModifierSet) -> u8 {
    let mut lang = 0u8;
    if keywords.contains(KotlinModifierSet::NOINLINE) {
        lang |= KotlinAttr::LANG_PARAM_NOINLINE;
    }
    if keywords.contains(KotlinModifierSet::CROSSINLINE) {
        lang |= KotlinAttr::LANG_PARAM_CROSSINLINE;
    }
    if keywords.contains(KotlinModifierSet::VARARG) {
        lang |= KotlinAttr::LANG_PARAM_VARARG;
    }
    lang
}

fn encode_class_parameter(node: Node, source: &[u8]) -> u32 {
    let mods = decl_modifiers(first_named_child_of_kind(node, "modifiers"), source);
    let declares_property = has_anonymous_child(node, "val") || has_anonymous_child(node, "var");
    let mutable = has_anonymous_child(node, "var");

    let (vis, implicit_vis) = if declares_property {
        visibility(&mods, NodeAttr::VISIBILITY_PUBLIC)
    } else {
        (NodeAttr::VISIBILITY_NONE, false)
    };

    let mut m = mods.annotation_mod_bits();
    let mut implicit_mods = false;
    if !mutable {
        m |= NodeAttr::MOD_FINAL;
        // A plain constructor parameter is read-only by language rule.
        implicit_mods = !declares_property;
    }

    let mut aux = 0u8;
    if declares_property {
        aux |= KotlinAttr::AUX_PARAM_PROPERTY;
    }
    if has_anonymous_child(node, "=") {
        aux |= KotlinAttr::AUX_PARAM_HAS_DEFAULT;
    }

    let mut lang = param_modifier_lang(mods.keywords);
    if mutable {
        lang |= KotlinAttr::LANG_PARAM_MUTABLE;
    }
    lang |= mods.shared_lang_bits(implicit_vis, implicit_mods);
    pack(vis, m, OperatorId::None, aux, lang)
}

fn encode_parameter(node: Node, kind: &str, source: &[u8]) -> u32 {
    // `parameter` keeps its modifiers in the preceding `parameter_modifiers` sibling;
    // `parameter_with_optional_type` holds them as a child.
    let modifiers = if kind == "parameter" {
        node.prev_named_sibling()
            .filter(|s| s.kind() == "parameter_modifiers")
    } else {
        first_named_child_of_kind(node, "parameter_modifiers")
    };
    let mods = decl_modifiers(modifiers, source);

    let mut aux = 0u8;
    if kind == "parameter" {
        let has_default = node
            .next_sibling()
            .is_some_and(|s| !s.is_named() && s.kind() == "=");
        if has_default {
            aux |= KotlinAttr::AUX_PARAM_HAS_DEFAULT;
        }
    }
    let lang = param_modifier_lang(mods.keywords) | mods.shared_lang_bits(false, true);
    pack(
        NodeAttr::VISIBILITY_NONE,
        NodeAttr::MOD_FINAL,
        OperatorId::None,
        aux,
        lang,
    )
}

fn encode_type_parameter(node: Node, source: &[u8]) -> u32 {
    let modifiers = first_named_child_of_kind(node, "type_parameter_modifiers");
    let keywords = modifier_keywords(modifiers);
    let variance = variance_keywords(modifiers);
    let mut annotations = DeclModifiers::default();
    if let Some(m) = modifiers {
        let mut cursor = m.walk();
        for child in m.named_children(&mut cursor) {
            if child.kind() == "annotation" {
                note_annotation(&mut annotations, child, source);
            }
        }
    }

    let mut lang = 0u8;
    if keywords.contains(KotlinModifierSet::REIFIED) {
        lang |= KotlinAttr::LANG_TYPE_PARAM_REIFIED;
    }
    if variance.contains(KotlinModifierSet::IN) {
        lang |= KotlinAttr::LANG_TYPE_PARAM_IN;
    }
    if variance.contains(KotlinModifierSet::OUT) {
        lang |= KotlinAttr::LANG_TYPE_PARAM_OUT;
    }
    lang |= annotations.shared_lang_bits(false, false);
    pack(NodeAttr::VISIBILITY_NONE, 0, OperatorId::None, 0, lang)
}

fn encode_enum_entry(node: Node, source: &[u8]) -> u32 {
    let mods = decl_modifiers(first_named_child_of_kind(node, "modifiers"), source);
    // Enum entries are public static final fields of the enum class on the JVM.
    let lang = mods.shared_lang_bits(true, true);
    pack(
        NodeAttr::VISIBILITY_PUBLIC,
        NodeAttr::MOD_STATIC | NodeAttr::MOD_FINAL,
        OperatorId::None,
        0,
        lang,
    )
}

fn encode_annotation(node: Node, kind: &str) -> u32 {
    let target = if kind == "file_annotation" {
        KotlinAttr::TARGET_FILE
    } else {
        match first_named_child_of_kind(node, "use_site_target").and_then(first_child_kind) {
            Some("file") => KotlinAttr::TARGET_FILE,
            Some("property") => KotlinAttr::TARGET_PROPERTY,
            Some("field") => KotlinAttr::TARGET_FIELD,
            Some("get") => KotlinAttr::TARGET_GET,
            Some("set") => KotlinAttr::TARGET_SET,
            Some("receiver") => KotlinAttr::TARGET_RECEIVER,
            Some("param") => KotlinAttr::TARGET_PARAM,
            Some("setparam") => KotlinAttr::TARGET_SETPARAM,
            Some("delegate") => KotlinAttr::TARGET_DELEGATE,
            _ => KotlinAttr::TARGET_NONE,
        }
    };
    pack(NodeAttr::VISIBILITY_NONE, 0, OperatorId::None, 0, target)
}

// ─────────────────────────────────────────────────────────────────────────────────────────
// Statement / expression encoders
// ─────────────────────────────────────────────────────────────────────────────────────────

fn encode_when(node: Node) -> u32 {
    let mut lang = 0u8;
    if let Some(subject) = first_named_child_of_kind(node, "when_subject") {
        lang |= KotlinAttr::WHEN_HAS_SUBJECT;
        if has_anonymous_child(subject, "val") {
            lang |= KotlinAttr::WHEN_SUBJECT_BINDING;
        }
    }
    pack(
        NodeAttr::VISIBILITY_NONE,
        0,
        OperatorId::None,
        NodeAttr::AUX_SWITCH_ARROW_FORM,
        lang,
    )
}

fn encode_when_entry(node: Node) -> u32 {
    let mut aux = NodeAttr::AUX_CASE_ARROW;
    if has_anonymous_child(node, "else") {
        aux |= NodeAttr::AUX_CASE_DEFAULT;
    }
    let mut cursor = node.walk();
    let pattern = node
        .named_children(&mut cursor)
        .filter(|c| c.kind() == "when_condition")
        .any(|c| first_named_child_of_kind(c, "type_test").is_some());
    if pattern {
        aux |= NodeAttr::AUX_CASE_PATTERN;
    }
    pack(NodeAttr::VISIBILITY_NONE, 0, OperatorId::None, aux, 0)
}

fn encode_call(node: Node) -> u32 {
    let mut aux = 0u8;
    let mut lang = 0u8;
    if let Some(callee) = node.child(0).filter(|c| c.kind() == "navigation_expression") {
        if navigation_role(callee) == NavigationRole::Callee {
            aux |= NodeAttr::AUX_CALL_QUALIFIED;
            let safe = navigation_suffix(callee).and_then(member_access_operator) == Some("?.");
            if safe {
                lang |= KotlinAttr::SAFE_ACCESS;
            }
        }
    }
    let trailing_lambda = first_named_child_of_kind(node, "call_suffix")
        .is_some_and(|s| first_named_child_of_kind(s, "annotated_lambda").is_some());
    if trailing_lambda {
        lang |= KotlinAttr::CALL_TRAILING_LAMBDA;
    }
    pack(NodeAttr::VISIBILITY_NONE, 0, OperatorId::None, aux, lang)
}
