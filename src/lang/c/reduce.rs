//! Phase 2 adapter for tree-sitter-c, and the C-family reduction core shared with the
//! C++ frontend (`lang::cpp::reduce` handles the C++-only kinds and delegates every kind
//! the two grammars share to `classify_c_family` / `encode_c_family`).
//!
//! AST shape conventions (C and C++):
//! * Declarations: `function_definition` and function-declaring `declaration` /
//!   `field_declaration` nodes become `NN_FUNCTION_DECL` (free function),
//!   `NN_METHOD_DECL`, `NN_CONSTRUCTOR_DECL`, `NN_DESTRUCTOR_DECL` or `NN_OPERATOR_DECL`
//!   by the shape of the declarator and the enclosing scope. Variable declarations are
//!   `NN_FIELD_DECL` at file, namespace and class scope and `NN_LOCAL_VAR_DECL` in
//!   blocks. The declared name is an `NN_IDENTIFIER_EXPR` child (qualified for
//!   out-of-class definitions such as `Foo::bar`).
//! * Each declarator of a variable or field declaration is one `NN_VAR_DECLARATOR`; its
//!   name is its first `NN_IDENTIFIER_EXPR` descendant, or the declarator itself when it
//!   has no children (`int a, b;`). Pointer, reference, array and parenthesised layers
//!   are folded into the declarator's attribute word. Out-of-class static member
//!   definitions without initializer (`int A::n;`) keep the qualified identifier as the
//!   declared name.
//! * Parameters are `NN_PARAM_DECL` (`[type, name?, default?]`); the `void` of `f(void)`
//!   is a parameter node flagged `PARAM_VOID_LIST`. The parameter list of a function
//!   *type* (function pointers, casts, template arguments) is an `NN_TYPE_REF` flagged
//!   `TYPE_FUNCTION_SIGNATURE` whose `NN_PARAM_DECL` children are signature components,
//!   not variables.
//! * `struct` / `union` / `enum` (and C++ `class`) specifiers with a body, or written as
//!   a forward declaration, are declarations (`NN_STRUCT_DECL`, `NN_UNION_DECL`,
//!   `NN_ENUM_DECL`, `NN_CLASS_DECL`); elaborated type references (`struct S *p`) reduce
//!   to an `NN_TYPE_REF` of the tag name.
//! * Statements keep the Java shapes the CFG builders expect: `else_clause` is
//!   eliminated (`NN_IF_STMT` = `[cond, then, else?]`), empty `;` statements are dropped,
//!   `case_statement` is an `NN_SWITCH_CASE` holding its label then its statements (C
//!   cases fall through: no `AUX_CASE_ARROW`), `goto` is `NN_GOTO_STMT`, labels are
//!   `NN_LABELED_STMT`.
//! * Preprocessor: `#include` is `NN_INCLUDE_DIRECTIVE`; object-like macros are
//!   `NN_FIELD_DECL` flagged `AUX_VAR_MACRO`, function-like macros `NN_FUNCTION_DECL`
//!   flagged `AUX_FN_MACRO` with one `NN_PARAM_DECL` per macro parameter; replacement
//!   text is opaque. Conditional-compilation directives are eliminated so that every
//!   branch contributes its declarations; their conditions are dropped.
//! * ERROR nodes are eliminated (recovered children are kept) and MISSING nodes are
//!   dropped; both are counted in the adapter's `SyntaxErrorTally`.
//!
//! Attribute words follow `NodeAttr::pack`; the C-family meaning of each bit is
//! documented on `CFamilyAttr`.

use std::sync::atomic::{AtomicU64, Ordering};

use tree_sitter::Node;

use crate::ast::adapter::{ASTReductionAdapter, ReductionDecision};
use crate::core::types::ast::{ASTNodeType, NodeAttr, OperatorArity, OperatorId};
use crate::lang::c::ingest::{is_error_node, is_floating_spelling};
use crate::lang::c::Dialect;

/// C-family meaning of the `NodeAttr` bit fields. Bits that share a position with a
/// `NodeAttr` constant of the same meaning are defined from it; the others reuse
/// positions that have no C-family meaning for the node types they apply to.
pub struct CFamilyAttr;

impl CFamilyAttr {
    // ── Modifiers (bits 27..20) ──
    /// Static storage duration or static member.
    pub const MOD_STATIC: u8 = NodeAttr::MOD_STATIC;
    /// Classes and virtual functions: `final`.
    pub const MOD_FINAL: u8 = NodeAttr::MOD_FINAL;
    /// Variables, fields, parameters, declarators: the declared object itself is
    /// `const` / `constexpr` (`const int x`, `int *const p`; not `const int *p`).
    pub const MOD_CONST: u8 = NodeAttr::MOD_FINAL;
    /// Functions: pure virtual (`= 0`). Classes: declare at least one pure virtual member.
    pub const MOD_ABSTRACT: u8 = NodeAttr::MOD_ABSTRACT;
    /// Functions: `explicit` / `explicit(expr)`.
    pub const MOD_EXPLICIT: u8 = NodeAttr::MOD_SYNCHRONIZED;
    /// Variables: thread storage duration (`thread_local`, `_Thread_local`, `__thread`).
    pub const MOD_THREAD_LOCAL: u8 = NodeAttr::MOD_SYNCHRONIZED;
    /// `extern` storage class, or the direct operand of `extern "C"` without braces.
    pub const MOD_EXTERN: u8 = NodeAttr::MOD_NATIVE;
    /// Functions and lambdas: non-throwing (`noexcept`, `noexcept(true)`, `throw()`).
    pub const MOD_NOEXCEPT: u8 = NodeAttr::MOD_VOLATILE;
    /// Variables, fields, parameters: `volatile`.
    pub const MOD_VOLATILE: u8 = NodeAttr::MOD_VOLATILE;
    /// Functions: defined as deleted (`= delete`).
    pub const MOD_DELETED: u8 = NodeAttr::MOD_TRANSIENT;
    /// Fields and lambdas: `mutable`.
    pub const MOD_MUTABLE: u8 = NodeAttr::MOD_TRANSIENT;
    /// Functions: explicitly defaulted (`= default`).
    pub const MOD_DEFAULTED: u8 = NodeAttr::MOD_SEALED;
    /// Variables and parameters: `register`.
    pub const MOD_REGISTER: u8 = NodeAttr::MOD_SEALED;

    // ── Lang flags (bits 7..0) of declarations ──
    /// `virtual` (written, or implied by `override` / `final`).
    pub const LANG_VIRTUAL: u8 = 1 << 0;
    /// `override` virt-specifier.
    pub const LANG_OVERRIDE: u8 = 1 << 1;
    /// `constexpr`, `consteval` or `constinit`.
    pub const LANG_CONSTEXPR: u8 = 1 << 2;
    /// `inline` (written, or implied for in-class and constexpr function definitions).
    pub const LANG_INLINE: u8 = 1 << 3;
    /// The visibility follows from the context (class key default, namespace scope).
    pub const LANG_IMPLICIT_VISIBILITY: u8 = NodeAttr::LANG_IMPLICIT_VISIBILITY;
    /// At least one modifier bit was implied by the language rules.
    pub const LANG_IMPLICIT_MODIFIERS: u8 = NodeAttr::LANG_IMPLICIT_MODIFIERS;
    /// `[[...]]`, `__attribute__((...))`, `__declspec(...)` or `alignas(...)` present.
    pub const LANG_HAS_ATTRIBUTES: u8 = NodeAttr::LANG_HAS_ANNOTATIONS;
    /// `[[deprecated]]`, `__attribute__((deprecated))` or `__declspec(deprecated)`.
    pub const LANG_DEPRECATED: u8 = NodeAttr::LANG_DEPRECATED;

    // ── Aux flags of function-like declarations ──
    /// The declarator name is qualified (`void Foo::bar() {}`, `ns::f`): an out-of-class
    /// definition. Phase 3 resolves whether the qualifier names a class or a namespace.
    pub const AUX_FN_OUT_OF_CLASS: u8 = 1 << 0;
    /// Has a body (a definition, not a prototype).
    pub const AUX_FN_HAS_BODY: u8 = NodeAttr::AUX_METHOD_HAS_BODY;
    /// `const`-qualified member function.
    pub const AUX_FN_CONST: u8 = 1 << 2;
    /// Function-like macro (`#define NAME(params) body`).
    pub const AUX_FN_MACRO: u8 = 1 << 3;

    // ── Aux flags of NN_FIELD_DECL / NN_LOCAL_VAR_DECL ──
    /// Compile-time constant: `constexpr`, a `const` non-pointer object at namespace or
    /// static class scope, or an object-like macro.
    pub const AUX_VAR_CONSTANT: u8 = NodeAttr::AUX_FIELD_CONSTANT;
    /// Bit-field member (`unsigned flag : 1;`); the width expression follows the name.
    pub const AUX_VAR_BITFIELD: u8 = 1 << 1;
    /// Object-like macro (`#define NAME value`).
    pub const AUX_VAR_MACRO: u8 = 1 << 2;
    /// Placeholder-typed (`auto`, `decltype(auto)`, `Concept auto`).
    pub const AUX_VAR_AUTO: u8 = 1 << 3;

    // ── NN_VAR_DECLARATOR ──
    /// Pointer declarator (`*p`).
    pub const AUX_DECL_POINTER: u8 = 1 << 0;
    /// Lvalue or rvalue reference (`&` / `&&`).
    pub const AUX_DECL_REFERENCE: u8 = 1 << 1;
    /// Array declarator (`a[n]`).
    pub const AUX_DECL_ARRAY: u8 = 1 << 2;
    /// Function or pointer-to-function declarator.
    pub const AUX_DECL_FUNCTION: u8 = 1 << 3;
    /// Lang byte: structured binding (`auto [a, b] = ...`); every identifier before the
    /// initializer is a declared name.
    pub const DECL_STRUCTURED_BINDING: u8 = 1 << 0;
    /// Lang byte: has an initializer (`= e`, `(args)` or `{...}`).
    pub const DECL_HAS_INITIALIZER: u8 = 1 << 1;
    /// Lang byte: rvalue reference (`&&`).
    pub const DECL_RVALUE_REFERENCE: u8 = 1 << 2;
    /// Lang byte: pack declarator (`Ts... args`).
    pub const DECL_PACK: u8 = 1 << 3;

    // ── NN_PARAM_DECL ──
    /// Exception parameter of a `catch` handler.
    pub const AUX_PARAM_CATCH: u8 = NodeAttr::AUX_PARAM_CATCH;
    /// Has a default argument.
    pub const AUX_PARAM_HAS_DEFAULT: u8 = 1 << 3;
    /// Lang byte: the `void` of an empty prototype parameter list `f(void)`.
    pub const PARAM_VOID_LIST: u8 = 1 << 0;
    /// Lang byte: pointer parameter (array parameters adjust to pointers).
    pub const PARAM_POINTER: u8 = 1 << 1;
    /// Lang byte: reference parameter (`&` / `&&`).
    pub const PARAM_REFERENCE: u8 = 1 << 2;
    /// Lang byte: C `...`, a pack `Ts... args`, or a macro `...` (same bit as Java varargs).
    pub const PARAM_VARIADIC: u8 = NodeAttr::LANG_JAVA_VARARGS;

    // ── Class-like declarations (struct / union / class / enum) ──
    /// Forward declaration: no body.
    pub const AUX_RECORD_FORWARD: u8 = 1 << 0;
    /// No tag name (`struct { ... } x;`, anonymous unions).
    pub const AUX_RECORD_ANONYMOUS: u8 = 1 << 1;
    /// Class / struct / union: the name has template arguments (a specialization).
    pub const AUX_CLASS_SPECIALIZATION: u8 = 1 << 2;
    /// Enum: scoped (`enum class` / `enum struct`).
    pub const AUX_ENUM_SCOPED: u8 = 1 << 2;
    /// Class / struct / union: declares at least one virtual member function.
    pub const AUX_CLASS_POLYMORPHIC: u8 = 1 << 3;
    /// Enum: fixed underlying type (`enum E : uint8_t`).
    pub const AUX_ENUM_FIXED_BASE: u8 = 1 << 3;

    // ── NN_NAMESPACE_DECL ──
    /// `inline namespace`.
    pub const AUX_NS_INLINE: u8 = 1 << 0;
    /// Unnamed namespace (internal linkage).
    pub const AUX_NS_ANONYMOUS: u8 = 1 << 1;
    /// `namespace a::b::c { }`: the name components are the leading identifiers.
    pub const AUX_NS_NESTED: u8 = 1 << 2;
    /// `namespace fs = std::filesystem;`: children are the alias then the target.
    pub const AUX_NS_ALIAS: u8 = 1 << 3;

    // ── NN_TYPEDEF_DECL ──
    /// Alias declaration (`using T = U;`).
    pub const AUX_TYPEDEF_ALIAS_SYNTAX: u8 = 1 << 0;
    /// Concept definition (`concept C = expr;`).
    pub const AUX_TYPEDEF_CONCEPT: u8 = 1 << 1;

    // ── NN_USING_DIRECTIVE ──
    /// `using namespace N;`
    pub const AUX_USING_NAMESPACE: u8 = 1 << 0;
    /// `using enum E;`
    pub const AUX_USING_ENUM: u8 = 1 << 1;
    /// Member using-declaration inside a class (`using Base::f;`).
    pub const AUX_USING_MEMBER: u8 = 1 << 2;

    // ── NN_INCLUDE_DIRECTIVE ──
    /// `#include <...>`.
    pub const AUX_INCLUDE_SYSTEM: u8 = 1 << 0;
    /// `#include MACRO`: the path is computed by the preprocessor.
    pub const AUX_INCLUDE_COMPUTED: u8 = 1 << 1;

    // ── NN_TEMPLATE_DECL ──
    /// `template<>`: explicit (full) specialization.
    pub const AUX_TEMPLATE_FULL_SPECIALIZATION: u8 = 1 << 0;
    /// Partial specialization of a class template.
    pub const AUX_TEMPLATE_PARTIAL_SPECIALIZATION: u8 = 1 << 1;
    /// Constrained by a `requires` clause.
    pub const AUX_TEMPLATE_CONSTRAINED: u8 = 1 << 2;
    /// Explicit instantiation (`template class Foo<int>;`).
    pub const AUX_TEMPLATE_INSTANTIATION: u8 = 1 << 3;

    // ── NN_TYPE_PARAM (one template parameter) ──
    /// Non-type template parameter (`int N`).
    pub const AUX_TPARAM_NON_TYPE: u8 = 1 << 0;
    /// Parameter pack (`typename... Ts`).
    pub const AUX_TPARAM_PACK: u8 = 1 << 1;
    /// Has a default argument.
    pub const AUX_TPARAM_HAS_DEFAULT: u8 = 1 << 2;
    /// Template template parameter.
    pub const AUX_TPARAM_TEMPLATE: u8 = 1 << 3;

    // ── NN_IF_STMT / NN_SWITCH_STMT / NN_WHILE_STMT / NN_ENHANCED_FOR ──
    /// The first child is an init-statement (`if (init; cond)`, `for (init; auto x : r)`).
    pub const AUX_STMT_HAS_INIT: u8 = 1 << 1;
    /// `if constexpr`.
    pub const AUX_IF_CONSTEXPR: u8 = 1 << 2;
    /// The condition is a declaration (`while (Node* n = next())`).
    pub const AUX_COND_DECLARATION: u8 = 1 << 3;

    // ── NN_TRY_STMT / NN_CATCH_CLAUSE ──
    /// MSVC structured exception handling (`__try` / `__except` / `__finally`).
    pub const AUX_TRY_SEH: u8 = 1 << 1;
    /// `catch (...)`.
    pub const AUX_CATCH_ALL: u8 = 1 << 0;
    /// `__except (filter)`: the first child is the filter expression.
    pub const AUX_CATCH_SEH: u8 = 1 << 1;

    // ── NN_IDENTIFIER_EXPR ──
    /// Qualified name (`a::b`); children are the scope components then the name.
    pub const AUX_IDENT_QUALIFIED: u8 = NodeAttr::AUX_IDENT_QUALIFIED;
    /// Template name with arguments (`max<int>`); children are the name then the arguments.
    pub const AUX_IDENT_TEMPLATE_ARGS: u8 = 1 << 1;
    /// `operator@` name; the operator field holds the overloaded operator when it has an id.
    pub const AUX_IDENT_OPERATOR_NAME: u8 = 1 << 2;
    /// Global-scope qualification (`::name`).
    pub const AUX_IDENT_GLOBAL_SCOPE: u8 = 1 << 3;

    // ── NN_TYPE_REF lang byte (C-family payload) ──
    /// Base specifier: `virtual` inheritance. A base specifier is an `NN_TYPE_REF` child of
    /// a class whose visibility is the inheritance access.
    pub const TYPE_BASE_VIRTUAL: u8 = 1 << 0;
    /// Pointer type (`T*`).
    pub const TYPE_POINTER: u8 = 1 << 1;
    /// Reference type (`T&` / `T&&`).
    pub const TYPE_REFERENCE: u8 = 1 << 2;
    /// `const`-qualified type.
    pub const TYPE_CONST: u8 = 1 << 3;
    /// Base specifier: the inheritance access follows from the class key.
    pub const TYPE_IMPLICIT_ACCESS: u8 = NodeAttr::LANG_IMPLICIT_VISIBILITY;
    /// `friend class X;` inside a class body.
    pub const TYPE_FRIEND: u8 = 1 << 5;
    /// `decltype(expr)`.
    pub const TYPE_DECLTYPE: u8 = 1 << 6;
    /// Function type: the children are the parameter types (`void (*)(int, char)`).
    pub const TYPE_FUNCTION_SIGNATURE: u8 = 1 << 7;

    // ── NN_LITERAL aux (the lang byte holds NodeAttr::LITERAL_*) ──
    /// Raw string literal (`R"(...)"`).
    pub const AUX_LITERAL_RAW: u8 = 1 << 0;
    /// User-defined literal (`12_km`); the category is that of the literal part.
    pub const AUX_LITERAL_USER_DEFINED: u8 = 1 << 1;
    /// Adjacent string literals; the pieces are `NN_LITERAL` / macro identifier children.
    pub const AUX_LITERAL_CONCATENATED: u8 = 1 << 2;
    /// Encoding prefix (`L`, `u`, `U`, `u8`).
    pub const AUX_LITERAL_ENCODING_PREFIX: u8 = 1 << 3;

    // ── NN_NEW_EXPR lang byte ──
    /// Temporary with automatic storage: C compound literal `(T){...}`, C++ `T{...}`.
    pub const NEW_TEMPORARY: u8 = 1 << 0;
    /// `new T[n]`.
    pub const NEW_ARRAY: u8 = 1 << 1;
    /// Placement new; the placement arguments are the leading children.
    pub const NEW_PLACEMENT: u8 = 1 << 2;
    /// `::new`.
    pub const NEW_GLOBAL_SCOPE: u8 = 1 << 3;

    // ── NN_FIELD_ACCESS lang byte ──
    /// Member access with `.`.
    pub const FIELD_ACCESS_DOT: u8 = 0;
    /// Member access through a pointer with `->`.
    pub const FIELD_ACCESS_ARROW: u8 = 1;

    // ── NN_UNARY_EXPR lang byte: which unary construct (operator id is None unless PLAIN) ──
    /// Operator expression; the operator field holds the operator.
    pub const UNARY_PLAIN: u8 = 0;
    /// Indirection `*p`.
    pub const UNARY_DEREF: u8 = 1;
    /// Address-of `&x`.
    pub const UNARY_ADDRESS_OF: u8 = 2;
    /// `sizeof`.
    pub const UNARY_SIZEOF: u8 = 3;
    /// `alignof` / `_Alignof`.
    pub const UNARY_ALIGNOF: u8 = 4;
    /// `offsetof(T, member)`.
    pub const UNARY_OFFSETOF: u8 = 5;
    /// `sizeof...(Ts)`.
    pub const UNARY_SIZEOF_PACK: u8 = 6;
    /// `co_await`.
    pub const UNARY_CO_AWAIT: u8 = 7;
    /// `delete p`.
    pub const UNARY_DELETE: u8 = 8;
    /// `delete[] p`.
    pub const UNARY_DELETE_ARRAY: u8 = 9;
    /// `pattern...`.
    pub const UNARY_PACK_EXPANSION: u8 = 10;
    /// C11 `_Generic(controlling, T: e, ...)`.
    pub const UNARY_GENERIC_SELECTION: u8 = 11;

    // ── NN_BINARY_EXPR lang byte ──
    /// Operator expression; the operator field holds the operator.
    pub const BINARY_PLAIN: u8 = 0;
    /// Comma operator.
    pub const BINARY_COMMA: u8 = 1;
    /// `<=>`.
    pub const BINARY_THREE_WAY: u8 = 2;
    /// `.*` / `->*`.
    pub const BINARY_POINTER_TO_MEMBER: u8 = 3;
    /// Fold expression (`(args + ...)`); the operator field holds the folded operator.
    pub const BINARY_FOLD: u8 = 4;

    // ── NN_RETURN_STMT / NN_YIELD_STMT lang byte ──
    /// `co_return` / `co_yield`.
    pub const STMT_COROUTINE: u8 = 1;

    // ── NN_GOTO_STMT lang byte ──
    /// `goto label;`
    pub const GOTO_LABEL: u8 = 0;
    /// GNU computed goto (`goto *p;`).
    pub const GOTO_COMPUTED: u8 = 1;
    /// MSVC `__leave`: jumps to the end of the enclosing `__try` block.
    pub const GOTO_SEH_LEAVE: u8 = 2;

    // ── NN_CALL_EXPR ──
    /// Mem-initializer of a constructor (`: member(args)` / `: Base(args)`).
    pub const AUX_CALL_EXPLICIT_CTOR: u8 = NodeAttr::AUX_CALL_EXPLICIT_CTOR;
    /// The mem-initializer names a base class.
    pub const AUX_CALL_SUPER_TARGET: u8 = NodeAttr::AUX_CALL_SUPER_TARGET;
    /// Explicit receiver or qualified callee (`obj.f()`, `p->f()`, `A::f()`).
    pub const AUX_CALL_QUALIFIED: u8 = NodeAttr::AUX_CALL_QUALIFIED;
    /// GNU inline assembly (`asm volatile ("..." : outputs : inputs : clobbers)`).
    pub const AUX_CALL_INLINE_ASM: u8 = 1 << 3;

    // ── NN_LAMBDA_EXPR lang byte ──
    /// Default capture by copy (`[=]`).
    pub const LAMBDA_CAPTURE_COPY: u8 = 1 << 0;
    /// Default capture by reference (`[&]`).
    pub const LAMBDA_CAPTURE_REFERENCE: u8 = 1 << 1;
}

/// Counts of the ERROR and MISSING nodes met during reduction. The adapter is shared by
/// every file of a run; `take` returns and resets the counts so a caller can attribute
/// them per file.
#[derive(Debug, Default)]
pub struct SyntaxErrorTally {
    error_nodes: AtomicU64,
    missing_nodes: AtomicU64,
}

impl SyntaxErrorTally {
    /// An empty tally.
    pub fn new() -> Self {
        Self::default()
    }

    /// Classifies and counts a syntax-error node: MISSING nodes are dropped (they have no
    /// text), ERROR nodes are eliminated (their recovered children are kept). Returns
    /// `None` for ordinary nodes.
    pub fn record(&self, node: &Node) -> Option<ReductionDecision> {
        if node.is_missing() {
            self.missing_nodes.fetch_add(1, Ordering::Relaxed);
            Some(ReductionDecision::Drop)
        } else if is_error_node(node) {
            self.error_nodes.fetch_add(1, Ordering::Relaxed);
            Some(ReductionDecision::Eliminate)
        } else {
            None
        }
    }

    /// ERROR nodes counted since the last `take`.
    pub fn error_nodes(&self) -> u64 {
        self.error_nodes.load(Ordering::Relaxed)
    }

    /// MISSING nodes counted since the last `take`.
    pub fn missing_nodes(&self) -> u64 {
        self.missing_nodes.load(Ordering::Relaxed)
    }

    /// Returns `(error_nodes, missing_nodes)` and resets both counts.
    pub fn take(&self) -> (u64, u64) {
        (
            self.error_nodes.swap(0, Ordering::Relaxed),
            self.missing_nodes.swap(0, Ordering::Relaxed),
        )
    }
}

/// Phase 2 reduction adapter for C translation units (tree-sitter-c).
pub struct CASTReductionAdapter {
    syntax_errors: SyntaxErrorTally,
}

impl Default for CASTReductionAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl CASTReductionAdapter {
    /// Creates the adapter.
    pub fn new() -> Self {
        Self {
            syntax_errors: SyntaxErrorTally::new(),
        }
    }

    /// ERROR / MISSING nodes met by this adapter.
    pub fn syntax_errors(&self) -> &SyntaxErrorTally {
        &self.syntax_errors
    }
}

impl ASTReductionAdapter for CASTReductionAdapter {
    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_c::language()
    }

    fn classify(&self, kind: &str, node: &Node, _depth: usize) -> ReductionDecision {
        if let Some(decision) = self.syntax_errors.record(node) {
            return decision;
        }
        classify_c_family(kind, node, Dialect::C)
    }

    fn encode_attrs(&self, kind: &str, node: &Node, source: &[u8]) -> u32 {
        encode_c_family(kind, node, source, Dialect::C)
    }
}

// ════════════════════════════════════════════════════════════════════════════════
// Classification
// ════════════════════════════════════════════════════════════════════════════════

/// Classifies a node kind that tree-sitter-c and tree-sitter-cpp share (every C kind).
/// Unrecognised named kinds are eliminated so that no kept descendant is lost;
/// unrecognised anonymous tokens are dropped.
pub fn classify_c_family(kind: &str, node: &Node, dialect: Dialect) -> ReductionDecision {
    use ASTNodeType::*;
    use ReductionDecision::*;

    match kind {
        "translation_unit" => Keep(NN_MODULE),

        // ── Preprocessor ──
        "preproc_include" => Keep(NN_INCLUDE_DIRECTIVE),
        "preproc_def" => Keep(NN_FIELD_DECL),
        "preproc_function_def" => Keep(NN_FUNCTION_DECL),
        "preproc_params" => Eliminate,
        "preproc_if" | "preproc_ifdef" | "preproc_elif" | "preproc_elifdef" | "preproc_else" => {
            Eliminate
        }
        // `#pragma`, `#error`, `defined(X)`, macro bodies: no structure to keep.
        "preproc_call" | "preproc_arg" | "preproc_defined" | "preproc_directive" => Drop,

        // ── Declarations ──
        "function_definition" => Keep(function_decl_type(node, dialect)),
        "declaration" => classify_declaration(node, dialect),
        "field_declaration" => classify_field_declaration(node, dialect),
        "type_definition" => Keep(NN_TYPEDEF_DECL),
        "struct_specifier" => classify_record(node, NN_STRUCT_DECL),
        "union_specifier" => classify_record(node, NN_UNION_DECL),
        "enum_specifier" => classify_record(node, NN_ENUM_DECL),
        "enumerator" => Keep(NN_ENUM_CONSTANT),
        "linkage_specification" | "declaration_list" | "field_declaration_list"
        | "enumerator_list" | "bitfield_clause" => Eliminate,
        "parameter_list" => match parameter_list_role(node) {
            ParameterListRole::Function => Eliminate,
            ParameterListRole::Signature => Keep(NN_TYPE_REF),
        },
        "parameter_declaration" => classify_parameter(node),
        "variadic_parameter" => Keep(NN_PARAM_DECL),
        "init_declarator" => Keep(NN_VAR_DECLARATOR),
        "pointer_declarator" | "array_declarator" | "function_declarator"
        | "parenthesized_declarator" | "attributed_declarator" => classify_declarator_layer(node),
        "abstract_pointer_declarator"
        | "abstract_array_declarator"
        | "abstract_function_declarator"
        | "abstract_parenthesized_declarator" => Eliminate,

        // Specifiers and qualifiers are encoded in the declaration's attribute word.
        "storage_class_specifier"
        | "type_qualifier"
        | "ms_based_modifier"
        | "ms_call_modifier"
        | "ms_pointer_modifier"
        | "ms_restrict_modifier"
        | "ms_signed_ptr_modifier"
        | "ms_unsigned_ptr_modifier"
        | "ms_unaligned_ptr_modifier" => Drop,
        "attribute_specifier" | "attribute" | "ms_declspec_modifier" => Keep(NN_ANNOTATION_USE),
        "attribute_declaration" | "attributed_statement" => Eliminate,

        // ── Types ──
        "primitive_type" => classify_primitive_type(node),
        "type_identifier" => classify_type_identifier(node),
        "sized_type_specifier" | "macro_type_specifier" => Keep(NN_TYPE_REF),
        "type_descriptor" => {
            if node.named_child_count() <= 1 {
                Eliminate
            } else {
                Keep(NN_TYPE_REF)
            }
        }

        // ── Statements ──
        "compound_statement" => Keep(NN_BLOCK),
        "expression_statement" => {
            if node.named_child_count() == 0 {
                Drop
            } else {
                Keep(NN_EXPR_STMT)
            }
        }
        "if_statement" => Keep(NN_IF_STMT),
        "else_clause" => Eliminate,
        "switch_statement" => Keep(NN_SWITCH_STMT),
        "case_statement" => Keep(NN_SWITCH_CASE),
        "while_statement" => Keep(NN_WHILE_STMT),
        "do_statement" => Keep(NN_DO_WHILE_STMT),
        "for_statement" => Keep(NN_FOR_STMT),
        "return_statement" => Keep(NN_RETURN_STMT),
        "break_statement" => Keep(NN_BREAK_STMT),
        "continue_statement" => Keep(NN_CONTINUE_STMT),
        "goto_statement" | "seh_leave_statement" => Keep(NN_GOTO_STMT),
        "labeled_statement" => Keep(NN_LABELED_STMT),
        "seh_try_statement" => Keep(NN_TRY_STMT),
        "seh_except_clause" => Keep(NN_CATCH_CLAUSE),
        "seh_finally_clause" => Keep(NN_FINALLY_CLAUSE),

        // ── Expressions ──
        "assignment_expression" => Keep(NN_ASSIGN_EXPR),
        "binary_expression" | "comma_expression" => {
            unless_preproc_condition(node, Keep(NN_BINARY_EXPR))
        }
        "unary_expression" => unless_preproc_condition(node, Keep(NN_UNARY_EXPR)),
        "update_expression" | "pointer_expression" | "sizeof_expression"
        | "alignof_expression" | "offsetof_expression" | "generic_expression" => {
            Keep(NN_UNARY_EXPR)
        }
        "call_expression" => unless_preproc_condition(node, Keep(NN_CALL_EXPR)),
        "parenthesized_expression" => unless_preproc_condition(node, Eliminate),
        "field_expression" => Keep(NN_FIELD_ACCESS),
        "subscript_expression" => Keep(NN_ARRAY_ACCESS),
        "cast_expression" => Keep(NN_CAST_EXPR),
        "conditional_expression" => Keep(NN_TERNARY_EXPR),
        "compound_literal_expression" => Keep(NN_NEW_EXPR),
        "initializer_list" => Keep(NN_ARRAY_INIT),
        // Designated initializer `.x = 1` / `[2] = v`: an assignment to the designator.
        "initializer_pair" => Keep(NN_ASSIGN_EXPR),
        "field_designator" | "subscript_designator" | "subscript_range_designator"
        | "argument_list" => Eliminate,
        "gnu_asm_expression" => Keep(NN_CALL_EXPR),
        "gnu_asm_output_operand_list"
        | "gnu_asm_input_operand_list"
        | "gnu_asm_clobber_list"
        | "gnu_asm_goto_list"
        | "gnu_asm_output_operand"
        | "gnu_asm_input_operand" => Eliminate,
        "gnu_asm_qualifier" => Drop,

        // ── Leaves ──
        "identifier" | "field_identifier" => classify_identifier(node),
        "statement_identifier" => Keep(NN_IDENTIFIER_EXPR),
        "number_literal" | "char_literal" | "string_literal" | "concatenated_string"
        | "system_lib_string" | "true" | "false" | "null" => classify_literal(node),
        // Literal internals and comments.
        "string_content" | "escape_sequence" | "character" | "comment" => Drop,

        _ => {
            if node.is_named() {
                Eliminate
            } else {
                Drop
            }
        }
    }
}

/// True for the declarations whose `declarator` children are variables or fields.
fn declares_variables(decl: &Node) -> bool {
    match decl.kind() {
        // K&R parameter declarations sit directly under the function definition.
        "declaration" => {
            !decl
                .parent()
                .is_some_and(|p| p.kind() == "function_definition")
                && !declares_function(decl)
        }
        "field_declaration" => !declares_function(decl),
        "for_range_loop" => true,
        _ => false,
    }
}

/// True when `node` is the outermost declarator of a declared variable or field.
pub(crate) fn is_variable_declarator(node: &Node) -> bool {
    match node.parent() {
        Some(parent) => {
            matches!(
                parent.kind(),
                "declaration" | "field_declaration" | "for_range_loop"
            ) && is_field(&parent, node, "declarator")
                && declares_variables(&parent)
        }
        None => false,
    }
}

fn classify_identifier(node: &Node) -> ReductionDecision {
    use ReductionDecision::*;
    let parent = match node.parent() {
        Some(p) => p,
        None => return Keep(ASTNodeType::NN_IDENTIFIER_EXPR),
    };
    match parent.kind() {
        // Macro names tested by `#ifdef` / `#elifdef`.
        "preproc_ifdef" | "preproc_elifdef" => Drop,
        "preproc_if" | "preproc_elif" if is_field(&parent, node, "condition") => Drop,
        "preproc_params" => Keep(ASTNodeType::NN_PARAM_DECL),
        "declaration" | "field_declaration" | "for_range_loop"
            if is_field(&parent, node, "declarator") && declares_variables(&parent) =>
        {
            Keep(ASTNodeType::NN_VAR_DECLARATOR)
        }
        _ => Keep(ASTNodeType::NN_IDENTIFIER_EXPR),
    }
}

fn classify_declarator_layer(node: &Node) -> ReductionDecision {
    if is_variable_declarator(node) {
        ReductionDecision::Keep(ASTNodeType::NN_VAR_DECLARATOR)
    } else {
        ReductionDecision::Eliminate
    }
}

fn classify_literal(node: &Node) -> ReductionDecision {
    use ReductionDecision::*;
    if let Some(parent) = node.parent() {
        match parent.kind() {
            // The user-defined literal node is the literal.
            "user_defined_literal" => return Drop,
            // `extern "C"`.
            "linkage_specification" => return Drop,
            "preproc_if" | "preproc_elif" if is_field(&parent, node, "condition") => {
                return Drop
            }
            // `= 0` of a pure virtual function, encoded as MOD_ABSTRACT.
            "field_declaration"
                if node.kind() == "number_literal" && declares_function(&parent) =>
            {
                return Drop
            }
            _ => {}
        }
    }
    Keep(ASTNodeType::NN_LITERAL)
}

/// Drops the controlling expression of `#if` / `#elif`; other nodes get `decision`.
fn unless_preproc_condition(node: &Node, decision: ReductionDecision) -> ReductionDecision {
    match node.parent() {
        Some(p)
            if matches!(p.kind(), "preproc_if" | "preproc_elif")
                && is_field(&p, node, "condition") =>
        {
            ReductionDecision::Drop
        }
        _ => decision,
    }
}

fn classify_primitive_type(node: &Node) -> ReductionDecision {
    use ReductionDecision::*;
    match node.parent() {
        // `unsigned int`: the specifier node is the type.
        Some(p) if p.kind() == "sized_type_specifier" => Drop,
        // `typedef int bool;`: a typedef redeclaring a primitive spelling.
        Some(p) if p.kind() == "type_definition" && is_field(&p, node, "declarator") => {
            Keep(ASTNodeType::NN_IDENTIFIER_EXPR)
        }
        _ => Keep(ASTNodeType::NN_TYPE_REF),
    }
}

/// `type_identifier` is the declared name of a tag, typedef, alias or template parameter,
/// and a type reference everywhere else.
pub(crate) fn classify_type_identifier(node: &Node) -> ReductionDecision {
    let declared = match node.parent() {
        Some(p) => match p.kind() {
            "struct_specifier" | "union_specifier" | "enum_specifier" | "class_specifier" => {
                is_field(&p, node, "name") && specifier_role(&p) != SpecifierRole::Reference
            }
            // Class template specialization `struct Foo<T*> { ... }`.
            "template_type" => p.parent().is_some_and(|spec| {
                matches!(
                    spec.kind(),
                    "struct_specifier" | "union_specifier" | "class_specifier"
                ) && is_field(&spec, &p, "name")
                    && specifier_role(&spec) != SpecifierRole::Reference
                    && is_field(&p, node, "name")
            }),
            "type_definition" => is_field(&p, node, "declarator"),
            // Name inside a typedef declarator (`typedef int (*fn_t)(int);`).
            "pointer_declarator" | "pointer_type_declarator" | "function_declarator"
            | "array_declarator" | "parenthesized_declarator" | "attributed_declarator" => true,
            "alias_declaration" | "concept_definition" => is_field(&p, node, "name"),
            "type_parameter_declaration" | "variadic_type_parameter_declaration" => true,
            "optional_type_parameter_declaration" => is_field(&p, node, "name"),
            _ => false,
        },
        None => false,
    };
    if declared {
        ReductionDecision::Keep(ASTNodeType::NN_IDENTIFIER_EXPR)
    } else {
        ReductionDecision::Keep(ASTNodeType::NN_TYPE_REF)
    }
}

/// Role of a `struct` / `union` / `enum` / `class` specifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SpecifierRole {
    /// Has a body.
    Definition,
    /// `struct S;` on its own.
    ForwardDeclaration,
    /// Elaborated type reference (`struct S *p`, `enum E e`).
    Reference,
}

pub(crate) fn specifier_role(spec: &Node) -> SpecifierRole {
    if spec.child_by_field_name("body").is_some() {
        return SpecifierRole::Definition;
    }
    match spec.parent().map(|p| p.kind()) {
        Some(
            "translation_unit" | "declaration_list" | "field_declaration_list"
            | "compound_statement" | "template_declaration" | "linkage_specification"
            | "preproc_if" | "preproc_ifdef" | "preproc_elif" | "preproc_elifdef"
            | "preproc_else" | "case_statement" | "labeled_statement",
        ) => SpecifierRole::ForwardDeclaration,
        _ => SpecifierRole::Reference,
    }
}

fn classify_record(node: &Node, node_type: ASTNodeType) -> ReductionDecision {
    match specifier_role(node) {
        SpecifierRole::Reference => ReductionDecision::Eliminate,
        _ => ReductionDecision::Keep(node_type),
    }
}

fn classify_declaration(node: &Node, dialect: Dialect) -> ReductionDecision {
    use ASTNodeType::*;
    use ReductionDecision::*;
    let ctx = declaration_context(node);
    if ctx == DeclContext::Parameter {
        return Keep(NN_PARAM_DECL);
    }
    if declares_function(node) {
        return Keep(function_decl_type(node, dialect));
    }
    if node.child_by_field_name("declarator").is_none() {
        return Eliminate;
    }
    match ctx {
        DeclContext::Block => Keep(NN_LOCAL_VAR_DECL),
        _ => Keep(NN_FIELD_DECL),
    }
}

fn classify_field_declaration(node: &Node, dialect: Dialect) -> ReductionDecision {
    use ReductionDecision::*;
    if declares_function(node) {
        let node_type = match dialect {
            // A C struct cannot declare member functions; a function declarator directly
            // naming a member only occurs in C++ code parsed as C.
            Dialect::C => ASTNodeType::NN_METHOD_DECL,
            Dialect::Cpp => function_decl_type(node, dialect),
        };
        return Keep(node_type);
    }
    if node.child_by_field_name("declarator").is_none() {
        // Anonymous struct / union member: its members belong to the enclosing record.
        return Eliminate;
    }
    Keep(ASTNodeType::NN_FIELD_DECL)
}

/// `parameter_declaration` and its C++ variants: a template parameter inside a
/// template parameter list, a function parameter elsewhere.
pub(crate) fn classify_parameter(node: &Node) -> ReductionDecision {
    match node.parent() {
        Some(p) if p.kind() == "template_parameter_list" => {
            ReductionDecision::Keep(ASTNodeType::NN_TYPE_PARAM)
        }
        _ => ReductionDecision::Keep(ASTNodeType::NN_PARAM_DECL),
    }
}

/// Node type of a function-like declaration (`function_definition`, or a `declaration` /
/// `field_declaration` whose declarator declares a function).
pub(crate) fn function_decl_type(node: &Node, dialect: Dialect) -> ASTNodeType {
    use ASTNodeType::*;
    let name = match node
        .child_by_field_name("declarator")
        .and_then(|d| declarator_shape(d).name)
    {
        Some(n) => n,
        None => return NN_FUNCTION_DECL,
    };
    let (terminal, qualified) = terminal_name(name);
    match terminal.kind() {
        "destructor_name" => return NN_DESTRUCTOR_DECL,
        "operator_name" | "operator_cast" => return NN_OPERATOR_DECL,
        _ => {}
    }
    if dialect == Dialect::C {
        return NN_FUNCTION_DECL;
    }
    let has_type = node.child_by_field_name("type").is_some();
    match declaration_context(node) {
        DeclContext::Friend => NN_FUNCTION_DECL,
        // In-class constructors use an `identifier` declarator and have no return type;
        // other members use `field_identifier`.
        DeclContext::Class(_) => {
            if !has_type && terminal.kind() == "identifier" {
                NN_CONSTRUCTOR_DECL
            } else {
                NN_METHOD_DECL
            }
        }
        _ => {
            if qualified {
                if has_type {
                    NN_METHOD_DECL
                } else {
                    NN_CONSTRUCTOR_DECL
                }
            } else {
                NN_FUNCTION_DECL
            }
        }
    }
}

/// Whether a parameter list declares the parameters of a function (or lambda, catch
/// handler, requires-expression), or only spells the parameter types of a function type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ParameterListRole {
    Function,
    Signature,
}

pub(crate) fn parameter_list_role(list: &Node) -> ParameterListRole {
    let parent = match list.parent() {
        Some(p) => p,
        None => return ParameterListRole::Function,
    };
    match parent.kind() {
        "function_declarator" => {
            if function_declarator_binds_entity(&parent) {
                ParameterListRole::Function
            } else {
                ParameterListRole::Signature
            }
        }
        "abstract_function_declarator" => match parent.parent().map(|g| g.kind()) {
            Some("lambda_expression") | Some("operator_cast") => ParameterListRole::Function,
            _ => ParameterListRole::Signature,
        },
        _ => ParameterListRole::Function,
    }
}

/// True when a `function_declarator` directly binds a declared function name (as opposed
/// to a pointer-to-function layer, a function-typed parameter, or a function typedef).
fn function_declarator_binds_entity(fd: &Node) -> bool {
    let mut inner = fd.child_by_field_name("declarator");
    while let Some(n) = inner {
        if matches!(n.kind(), "parenthesized_declarator" | "attributed_declarator") {
            inner = inner_declarator(&n);
        } else {
            break;
        }
    }
    match inner {
        Some(n) if is_name_kind(n.kind()) => {}
        _ => return false,
    }
    let mut up = fd.parent();
    while let Some(p) = up {
        match p.kind() {
            "pointer_declarator" | "reference_declarator" | "parenthesized_declarator"
            | "attributed_declarator" | "array_declarator" | "function_declarator" => {
                up = p.parent();
            }
            "parameter_declaration"
            | "optional_parameter_declaration"
            | "variadic_parameter_declaration"
            | "type_definition"
            | "type_descriptor"
            | "alias_declaration" => return false,
            _ => return true,
        }
    }
    true
}

/// Scope in which a declaration appears.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeclContext<'t> {
    /// Translation unit, namespace body or linkage-specification body.
    File,
    /// Member of the given class / struct / union specifier.
    Class(Node<'t>),
    /// Function body, statement, condition or lambda.
    Block,
    /// K&R parameter declaration, function or template parameter list.
    Parameter,
    /// Operand of a `friend` declaration.
    Friend,
}

pub(crate) fn declaration_context<'t>(node: &Node<'t>) -> DeclContext<'t> {
    let mut cur = node.parent();
    let mut direct = true;
    while let Some(p) = cur {
        match p.kind() {
            "translation_unit" | "declaration_list" => return DeclContext::File,
            "field_declaration_list" => {
                return match p.parent() {
                    Some(class) => DeclContext::Class(class),
                    None => DeclContext::File,
                }
            }
            "friend_declaration" => return DeclContext::Friend,
            "function_definition" if direct => return DeclContext::Parameter,
            "parameter_list" | "template_parameter_list" => return DeclContext::Parameter,
            "compound_statement" | "function_definition" | "for_statement"
            | "for_range_loop" | "condition_clause" | "init_statement" | "case_statement"
            | "labeled_statement" | "if_statement" | "while_statement" | "do_statement"
            | "switch_statement" | "lambda_expression" | "catch_clause" | "try_statement"
            | "seh_try_statement" | "seh_except_clause" => return DeclContext::Block,
            _ => {}
        }
        direct = false;
        cur = p.parent();
    }
    DeclContext::File
}

// ════════════════════════════════════════════════════════════════════════════════
// Declarators, names and specifiers
// ════════════════════════════════════════════════════════════════════════════════

/// Kinds that terminate a declarator chain: the declared name.
pub(crate) fn is_name_kind(kind: &str) -> bool {
    matches!(
        kind,
        "identifier"
            | "field_identifier"
            | "type_identifier"
            | "primitive_type"
            | "qualified_identifier"
            | "destructor_name"
            | "operator_name"
            | "operator_cast"
            | "template_function"
            | "template_method"
            | "structured_binding_declarator"
    )
}

/// The declarator nested inside a declarator layer.
pub(crate) fn inner_declarator<'t>(node: &Node<'t>) -> Option<Node<'t>> {
    if let Some(d) = node.child_by_field_name("declarator") {
        return Some(d);
    }
    match node.kind() {
        "parenthesized_declarator"
        | "attributed_declarator"
        | "reference_declarator"
        | "variadic_declarator"
        | "abstract_parenthesized_declarator"
        | "abstract_reference_declarator" => {
            let mut cursor = node.walk();
            let found = node.named_children(&mut cursor).find(|c| {
                !matches!(
                    c.kind(),
                    "attribute_declaration"
                        | "attribute_specifier"
                        | "ms_call_modifier"
                        | "ms_based_modifier"
                        | "ms_pointer_modifier"
                        | "ms_restrict_modifier"
                        | "ms_unaligned_ptr_modifier"
                        | "ms_signed_ptr_modifier"
                        | "ms_unsigned_ptr_modifier"
                        | "type_qualifier"
                        | "comment"
                )
            });
            found
        }
        _ => None,
    }
}

/// Shape of a declarator chain, from the outermost layer to the declared name.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct DeclaratorShape<'t> {
    /// The declared name (or structured binding), when the declarator is not abstract.
    pub name: Option<Node<'t>>,
    /// Kind of the layer that binds the name most tightly (parentheses and attributes
    /// ignored). `function_declarator` (or `operator_cast`) means a function is declared.
    pub binder: Option<&'static str>,
    pub pointer: bool,
    pub reference: bool,
    pub rvalue_reference: bool,
    pub array: bool,
    pub function: bool,
    pub pack: bool,
    pub structured: bool,
    pub has_initializer: bool,
}

impl DeclaratorShape<'_> {
    /// True when the declarator declares a function (not a pointer to one).
    pub fn declares_function(&self) -> bool {
        matches!(self.binder, Some("function_declarator") | Some("operator_cast"))
    }
}

pub(crate) fn declarator_shape<'t>(decl: Node<'t>) -> DeclaratorShape<'t> {
    let mut shape = DeclaratorShape::default();
    let mut cur = Some(decl);
    // Declarator chains are short; the bound protects against malformed recovery trees.
    let mut remaining = 256u32;
    while let Some(n) = cur {
        if remaining == 0 {
            break;
        }
        remaining -= 1;
        let kind = n.kind();
        if is_name_kind(kind) {
            match kind {
                "structured_binding_declarator" => shape.structured = true,
                "operator_cast" => {
                    shape.function = true;
                    shape.binder = Some("operator_cast");
                }
                _ => {}
            }
            shape.name = Some(n);
            break;
        }
        match kind {
            "pointer_declarator" | "pointer_type_declarator" | "abstract_pointer_declarator" => {
                shape.pointer = true;
                shape.binder = Some("pointer_declarator");
            }
            "reference_declarator" | "abstract_reference_declarator" => {
                shape.reference = true;
                if has_child_kind(&n, "&&") {
                    shape.rvalue_reference = true;
                }
                shape.binder = Some("reference_declarator");
            }
            "array_declarator" | "abstract_array_declarator" => {
                shape.array = true;
                shape.binder = Some("array_declarator");
            }
            "function_declarator" | "abstract_function_declarator" => {
                shape.function = true;
                shape.binder = Some("function_declarator");
            }
            "variadic_declarator" => shape.pack = true,
            "init_declarator" => shape.has_initializer = true,
            _ => {}
        }
        cur = inner_declarator(&n);
    }
    shape
}

/// The layer that carries a function's parameters and trailing qualifiers: the
/// `function_declarator` binding the name, or the abstract declarator of a conversion
/// operator.
pub(crate) fn function_layer<'t>(decl: Node<'t>) -> Option<Node<'t>> {
    let mut cur = Some(decl);
    let mut binder: Option<Node<'t>> = None;
    let mut remaining = 256u32;
    while let Some(n) = cur {
        if remaining == 0 {
            break;
        }
        remaining -= 1;
        match n.kind() {
            "operator_cast" => {
                return n.child_by_field_name("declarator").and_then(|d| {
                    let mut c = Some(d);
                    while let Some(x) = c {
                        if x.kind() == "abstract_function_declarator" {
                            return Some(x);
                        }
                        c = inner_declarator(&x);
                    }
                    None
                });
            }
            k if is_name_kind(k) => break,
            "function_declarator" | "pointer_declarator" | "reference_declarator"
            | "array_declarator" => binder = Some(n),
            _ => {}
        }
        cur = inner_declarator(&n);
    }
    binder.filter(|b| b.kind() == "function_declarator")
}

/// Unwraps qualification and template arguments: the simple name node, and whether the
/// name was qualified (`A::f`, `::g`).
pub(crate) fn terminal_name(name: Node) -> (Node, bool) {
    let mut n = name;
    let mut qualified = false;
    let mut remaining = 64u32;
    while remaining > 0 {
        remaining -= 1;
        let next = match n.kind() {
            "qualified_identifier" => {
                qualified = true;
                n.child_by_field_name("name")
            }
            "template_function" | "template_method" => n.child_by_field_name("name"),
            "dependent_name" => n.named_child(0),
            _ => None,
        };
        match next {
            Some(x) => n = x,
            None => break,
        }
    }
    (n, qualified)
}

/// True when a `declaration` / `field_declaration` / `function_definition` declares a
/// function (its first declarator binds a function).
pub(crate) fn declares_function(decl: &Node) -> bool {
    if decl.kind() == "function_definition" {
        return true;
    }
    decl.child_by_field_name("declarator")
        .map(|d| declarator_shape(d).declares_function())
        .unwrap_or(false)
}

/// True when `node` is the child of `parent` stored under `field`.
pub(crate) fn is_field(parent: &Node, node: &Node, field: &str) -> bool {
    let mut cursor = parent.walk();
    if !cursor.goto_first_child() {
        return false;
    }
    loop {
        if cursor.node().id() == node.id() {
            return cursor.field_name() == Some(field);
        }
        if !cursor.goto_next_sibling() {
            return false;
        }
    }
}

/// True when `node` has a direct child (named or anonymous) of the given kind.
pub(crate) fn has_child_kind(node: &Node, kind: &str) -> bool {
    let mut cursor = node.walk();
    let found = node.children(&mut cursor).any(|c| c.kind() == kind);
    found
}

/// Source text of a node, trimmed; empty when the bytes are not UTF-8.
pub(crate) fn node_text<'s>(node: &Node, source: &'s [u8]) -> &'s str {
    node.utf8_text(source).map(str::trim).unwrap_or("")
}

/// Specifiers written directly on a declaration (storage class, qualifiers, function
/// specifiers, attributes).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Specifiers {
    pub is_static: bool,
    pub is_extern: bool,
    pub is_register: bool,
    pub is_inline: bool,
    pub is_thread_local: bool,
    pub is_const: bool,
    pub is_volatile: bool,
    pub is_mutable: bool,
    pub is_constexpr: bool,
    pub is_virtual: bool,
    pub is_explicit: bool,
    pub has_attributes: bool,
    pub is_deprecated: bool,
    pub has_placeholder_type: bool,
}

impl Specifiers {
    fn apply_keyword(&mut self, word: &str) {
        match word {
            "static" => self.is_static = true,
            "extern" => self.is_extern = true,
            "register" => self.is_register = true,
            "inline" | "__inline" | "__inline__" | "__forceinline" => self.is_inline = true,
            "thread_local" | "_Thread_local" | "__thread" => self.is_thread_local = true,
            "const" => self.is_const = true,
            "volatile" | "__volatile__" => self.is_volatile = true,
            "mutable" => self.is_mutable = true,
            "constexpr" | "consteval" | "constinit" => self.is_constexpr = true,
            "virtual" => self.is_virtual = true,
            w if w == "explicit" || w.starts_with("explicit(") || w.starts_with("explicit (") => {
                self.is_explicit = true
            }
            _ => {}
        }
    }

    fn apply_attribute(&mut self, text: &str) {
        self.has_attributes = true;
        if text.contains("deprecated") {
            self.is_deprecated = true;
        }
    }
}

/// Collects the specifiers that are direct children of `node`.
pub(crate) fn collect_specifiers(node: &Node, source: &[u8]) -> Specifiers {
    let mut specs = Specifiers::default();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "storage_class_specifier" | "type_qualifier" | "virtual"
            | "explicit_function_specifier" => specs.apply_keyword(node_text(&child, source)),
            "inline" | "static" | "extern" | "const" | "constexpr" | "consteval"
            | "constinit" | "volatile" | "mutable" | "register" | "thread_local" => {
                specs.apply_keyword(child.kind())
            }
            "attribute_specifier" | "attribute_declaration" | "ms_declspec_modifier"
            | "alignas_specifier" | "alignas_qualifier" => {
                specs.apply_attribute(node_text(&child, source))
            }
            "placeholder_type_specifier" => specs.has_placeholder_type = true,
            _ => {}
        }
    }
    if let Some(t) = node.child_by_field_name("type") {
        if t.kind() == "placeholder_type_specifier" {
            specs.has_placeholder_type = true;
        }
    }
    specs
}

/// True when the outermost pointer layer of a declarator is itself `const`
/// (`int *const p`).
pub(crate) fn outer_pointer_is_const(decl: Node, source: &[u8]) -> bool {
    let mut cur = Some(decl);
    let mut remaining = 256u32;
    while let Some(n) = cur {
        if remaining == 0 {
            break;
        }
        remaining -= 1;
        match n.kind() {
            "pointer_declarator" | "pointer_type_declarator" | "abstract_pointer_declarator" => {
                let mut cursor = n.walk();
                let is_const = n.children(&mut cursor).any(|c| {
                    c.kind() == "type_qualifier" && node_text(&c, source) == "const"
                });
                return is_const;
            }
            "reference_declarator" | "array_declarator" | "function_declarator" => return false,
            k if is_name_kind(k) => return false,
            _ => {}
        }
        cur = inner_declarator(&n);
    }
    false
}

/// Access of a class member: the last access specifier before it in the member list, or
/// the class-key default (`class`: private; `struct` / `union`: public). Returns `None`
/// when `node` is not a member. The boolean is true when the access is implicit.
pub(crate) fn member_access(node: &Node, source: &[u8]) -> Option<(u8, bool)> {
    let mut item = *node;
    let mut list = node.parent()?;
    while list.kind() != "field_declaration_list" {
        match list.kind() {
            "template_declaration" | "declaration" | "field_declaration" | "type_definition"
            | "preproc_if" | "preproc_ifdef" | "preproc_elif" | "preproc_elifdef"
            | "preproc_else" | "attributed_statement" => {
                item = list;
                list = list.parent()?;
            }
            _ => return None,
        }
    }
    let class = list.parent()?;
    let default = if class.kind() == "class_specifier" {
        NodeAttr::VISIBILITY_PRIVATE
    } else {
        NodeAttr::VISIBILITY_PUBLIC
    };
    let mut access = None;
    let mut cursor = list.walk();
    for child in list.children(&mut cursor) {
        if child.id() == item.id() {
            break;
        }
        if child.kind() == "access_specifier" {
            access = access_from_text(node_text(&child, source));
        }
    }
    Some(match access {
        Some(v) => (v, false),
        None => (default, true),
    })
}

/// Visibility of an access keyword (`public`, `private:`, `protected`).
pub(crate) fn access_from_text(text: &str) -> Option<u8> {
    if text.starts_with("public") {
        Some(NodeAttr::VISIBILITY_PUBLIC)
    } else if text.starts_with("private") {
        Some(NodeAttr::VISIBILITY_PRIVATE)
    } else if text.starts_with("protected") {
        Some(NodeAttr::VISIBILITY_PROTECTED)
    } else {
        None
    }
}

/// True when `node` lies inside an unnamed namespace (internal linkage).
pub(crate) fn in_anonymous_namespace(node: &Node) -> bool {
    let mut cur = node.parent();
    while let Some(p) = cur {
        if p.kind() == "namespace_definition" && p.child_by_field_name("name").is_none() {
            return true;
        }
        cur = p.parent();
    }
    false
}

/// Visibility of a declaration in its context: member access in classes, external /
/// internal linkage at namespace scope, none in blocks, parameters and friend
/// declarations. Out-of-class definitions take their access from the in-class
/// declaration, which Phase 3 links, so they carry none.
fn declaration_visibility(
    node: &Node,
    ctx: DeclContext,
    is_static: bool,
    qualified: bool,
    source: &[u8],
) -> (u8, bool) {
    match ctx {
        DeclContext::Class(_) => member_access(node, source)
            .unwrap_or((NodeAttr::VISIBILITY_PUBLIC, true)),
        DeclContext::File if qualified => (NodeAttr::VISIBILITY_NONE, false),
        DeclContext::File => {
            if is_static || in_anonymous_namespace(node) {
                (NodeAttr::VISIBILITY_PRIVATE, true)
            } else {
                (NodeAttr::VISIBILITY_PUBLIC, true)
            }
        }
        DeclContext::Block | DeclContext::Parameter | DeclContext::Friend => {
            (NodeAttr::VISIBILITY_NONE, false)
        }
    }
}

/// True when the declaration is the direct operand of `extern "C"` (no braces).
fn is_linkage_operand(node: &Node) -> bool {
    node.parent()
        .is_some_and(|p| p.kind() == "linkage_specification")
}

/// Maps an alternative operator spelling (`and`, `not_eq`, `bitor`, ...) to its symbol.
pub(crate) fn normalize_operator(symbol: &str) -> &str {
    match symbol {
        "and" => "&&",
        "or" => "||",
        "not" => "!",
        "not_eq" => "!=",
        "bitand" => "&",
        "bitor" => "|",
        "xor" => "^",
        "compl" => "~",
        "and_eq" => "&=",
        "or_eq" => "|=",
        "xor_eq" => "^=",
        other => other,
    }
}

/// Text of the `operator` field of an expression node.
fn operator_text<'s>(node: &Node, source: &'s [u8]) -> &'s str {
    node.child_by_field_name("operator")
        .map(|op| node_text(&op, source))
        .unwrap_or("")
}

/// Operator overloaded by an `operator@` name (`operator+=`, `operator ==`), if it has an
/// `OperatorId`. Binary meaning is preferred for symbols that are both unary and binary;
/// the parameter count decides in Phase 3.
pub(crate) fn overloaded_operator(name: &Node, source: &[u8]) -> OperatorId {
    let text = node_text(name, source);
    let symbol = text.strip_prefix("operator").map(str::trim).unwrap_or("");
    let symbol = normalize_operator(symbol);
    OperatorId::from_symbol(symbol, OperatorArity::Assignment)
        .or_else(|| OperatorId::from_symbol(symbol, OperatorArity::Binary))
        .or_else(|| OperatorId::from_symbol(symbol, OperatorArity::Unary))
        .unwrap_or(OperatorId::None)
}

/// Virtual and pure-virtual members of a class body: `(polymorphic, abstract)`.
fn class_virtuality(class: &Node, source: &[u8]) -> (bool, bool) {
    let body = match class.child_by_field_name("body") {
        Some(b) => b,
        None => return (false, false),
    };
    let mut polymorphic = false;
    let mut abstract_ = false;
    let mut cursor = body.walk();
    for member in body.children(&mut cursor) {
        let member = if member.kind() == "template_declaration" {
            let mut c = member.walk();
            let inner = member
                .named_children(&mut c)
                .find(|n| matches!(n.kind(), "declaration" | "field_declaration" | "function_definition"));
            match inner {
                Some(i) => i,
                None => continue,
            }
        } else {
            member
        };
        if !matches!(
            member.kind(),
            "declaration" | "field_declaration" | "function_definition"
        ) {
            continue;
        }
        if !declares_function(&member) {
            continue;
        }
        let specs = collect_specifiers(&member, source);
        let overrides = member
            .child_by_field_name("declarator")
            .and_then(function_layer)
            .is_some_and(|f| has_child_kind(&f, "virtual_specifier"));
        if specs.is_virtual || overrides {
            polymorphic = true;
        }
        if is_pure_virtual(&member, source) {
            polymorphic = true;
            abstract_ = true;
        }
    }
    (polymorphic, abstract_)
}

/// `virtual void f() = 0;`
fn is_pure_virtual(member: &Node, source: &[u8]) -> bool {
    member.kind() == "field_declaration"
        && member
            .child_by_field_name("default_value")
            .is_some_and(|v| v.kind() == "number_literal" && node_text(&v, source) == "0")
}

// ════════════════════════════════════════════════════════════════════════════════
// Attribute encoding
// ════════════════════════════════════════════════════════════════════════════════

/// Encodes the attribute word of a kept node whose kind tree-sitter-c and
/// tree-sitter-cpp share. Kinds without attributes encode as 0.
pub fn encode_c_family(kind: &str, node: &Node, source: &[u8], dialect: Dialect) -> u32 {
    use ASTNodeType::*;
    match kind {
        "function_definition" => {
            encode_function_decl(node, function_decl_type(node, dialect), source)
        }
        "declaration" => {
            if node
                .parent()
                .is_some_and(|p| p.kind() == "function_definition")
            {
                encode_parameter(node, source)
            } else if declares_function(node) {
                encode_function_decl(node, function_decl_type(node, dialect), source)
            } else {
                encode_variable_decl(node, source)
            }
        }
        "field_declaration" => {
            if declares_function(node) {
                let node_type = match dialect {
                    Dialect::C => NN_METHOD_DECL,
                    Dialect::Cpp => function_decl_type(node, dialect),
                };
                encode_function_decl(node, node_type, source)
            } else {
                encode_variable_decl(node, source)
            }
        }
        "preproc_def" => NodeAttr::pack(
            NodeAttr::VISIBILITY_PUBLIC,
            0,
            OperatorId::None,
            CFamilyAttr::AUX_VAR_MACRO | CFamilyAttr::AUX_VAR_CONSTANT,
            CFamilyAttr::LANG_IMPLICIT_VISIBILITY,
        ),
        "preproc_function_def" => NodeAttr::pack(
            NodeAttr::VISIBILITY_PUBLIC,
            0,
            OperatorId::None,
            CFamilyAttr::AUX_FN_MACRO,
            CFamilyAttr::LANG_IMPLICIT_VISIBILITY,
        ),
        "preproc_include" => {
            let aux = match node.child_by_field_name("path").map(|p| p.kind()) {
                Some("system_lib_string") => CFamilyAttr::AUX_INCLUDE_SYSTEM,
                Some("string_literal") | None => 0,
                Some(_) => CFamilyAttr::AUX_INCLUDE_COMPUTED,
            };
            NodeAttr::pack(NodeAttr::VISIBILITY_NONE, 0, OperatorId::None, aux, 0)
        }
        "struct_specifier" | "union_specifier" | "enum_specifier" | "class_specifier" => {
            encode_record(node, source)
        }
        "enumerator" => NodeAttr::pack(
            NodeAttr::VISIBILITY_PUBLIC,
            CFamilyAttr::MOD_STATIC | CFamilyAttr::MOD_CONST,
            OperatorId::None,
            0,
            CFamilyAttr::LANG_IMPLICIT_VISIBILITY | CFamilyAttr::LANG_IMPLICIT_MODIFIERS,
        ),
        "type_definition" => encode_typedef(node, source, 0),
        "parameter_declaration" => match node.parent() {
            Some(p) if p.kind() == "template_parameter_list" => {
                encode_template_parameter(node, source)
            }
            _ => encode_parameter(node, source),
        },
        "variadic_parameter" => NodeAttr::pack(
            NodeAttr::VISIBILITY_NONE,
            0,
            OperatorId::None,
            0,
            CFamilyAttr::PARAM_VARIADIC,
        ),
        "init_declarator" | "pointer_declarator" | "array_declarator" | "function_declarator"
        | "parenthesized_declarator" | "attributed_declarator" | "reference_declarator"
        | "structured_binding_declarator" => encode_var_declarator(node, source),
        "identifier" | "field_identifier" => {
            if is_variable_declarator(node) {
                encode_var_declarator(node, source)
            } else {
                0
            }
        }
        "parameter_list" => NodeAttr::pack(
            NodeAttr::VISIBILITY_NONE,
            0,
            OperatorId::None,
            0,
            CFamilyAttr::TYPE_FUNCTION_SIGNATURE,
        ),
        "type_descriptor" => encode_type_descriptor(node, source),
        "attribute_specifier" | "attribute" | "ms_declspec_modifier" => {
            let lang = if node_text(node, source).contains("deprecated") {
                CFamilyAttr::LANG_DEPRECATED
            } else {
                0
            };
            NodeAttr::pack(NodeAttr::VISIBILITY_NONE, 0, OperatorId::None, 0, lang)
        }

        // ── Statements ──
        "if_statement" => {
            let mut aux = condition_flags(node);
            if has_child_kind(node, "constexpr") {
                aux |= CFamilyAttr::AUX_IF_CONSTEXPR;
            }
            NodeAttr::pack(NodeAttr::VISIBILITY_NONE, 0, OperatorId::None, aux, 0)
        }
        "switch_statement" | "while_statement" => NodeAttr::pack(
            NodeAttr::VISIBILITY_NONE,
            0,
            OperatorId::None,
            condition_flags(node),
            0,
        ),
        "case_statement" => {
            let is_default = node
                .child(0)
                .is_some_and(|first| first.kind() == "default");
            let aux = if is_default {
                NodeAttr::AUX_CASE_DEFAULT
            } else {
                0
            };
            NodeAttr::pack(NodeAttr::VISIBILITY_NONE, 0, OperatorId::None, aux, 0)
        }
        "seh_try_statement" => NodeAttr::pack(
            NodeAttr::VISIBILITY_NONE,
            0,
            OperatorId::None,
            CFamilyAttr::AUX_TRY_SEH,
            0,
        ),
        "seh_except_clause" => NodeAttr::pack(
            NodeAttr::VISIBILITY_NONE,
            0,
            OperatorId::None,
            CFamilyAttr::AUX_CATCH_SEH,
            0,
        ),
        "goto_statement" => {
            let computed = node
                .child_by_field_name("label")
                .is_some_and(|l| l.kind() != "statement_identifier");
            let lang = if computed {
                CFamilyAttr::GOTO_COMPUTED
            } else {
                CFamilyAttr::GOTO_LABEL
            };
            NodeAttr::pack(NodeAttr::VISIBILITY_NONE, 0, OperatorId::None, 0, lang)
        }
        "seh_leave_statement" => NodeAttr::pack(
            NodeAttr::VISIBILITY_NONE,
            0,
            OperatorId::None,
            0,
            CFamilyAttr::GOTO_SEH_LEAVE,
        ),

        // ── Expressions ──
        "binary_expression" => encode_binary(node, source),
        "comma_expression" => NodeAttr::pack(
            NodeAttr::VISIBILITY_NONE,
            0,
            OperatorId::None,
            0,
            CFamilyAttr::BINARY_COMMA,
        ),
        "assignment_expression" => {
            let symbol = normalize_operator(operator_text(node, source));
            let op = OperatorId::from_symbol(symbol, OperatorArity::Assignment)
                .unwrap_or(OperatorId::None);
            NodeAttr::pack(NodeAttr::VISIBILITY_NONE, 0, op, 0, 0)
        }
        "initializer_pair" => {
            NodeAttr::pack(NodeAttr::VISIBILITY_NONE, 0, OperatorId::Assign, 0, 0)
        }
        "unary_expression" => {
            let symbol = normalize_operator(operator_text(node, source));
            let op = OperatorId::from_symbol(symbol, OperatorArity::Unary)
                .unwrap_or(OperatorId::None);
            NodeAttr::pack(NodeAttr::VISIBILITY_NONE, 0, op, 0, CFamilyAttr::UNARY_PLAIN)
        }
        "update_expression" => encode_update(node, source),
        "pointer_expression" => {
            let lang = match operator_text(node, source) {
                "*" => CFamilyAttr::UNARY_DEREF,
                "&" => CFamilyAttr::UNARY_ADDRESS_OF,
                _ => CFamilyAttr::UNARY_PLAIN,
            };
            NodeAttr::pack(NodeAttr::VISIBILITY_NONE, 0, OperatorId::None, 0, lang)
        }
        "sizeof_expression" => {
            let lang = if has_child_kind(node, "...") {
                CFamilyAttr::UNARY_SIZEOF_PACK
            } else {
                CFamilyAttr::UNARY_SIZEOF
            };
            NodeAttr::pack(NodeAttr::VISIBILITY_NONE, 0, OperatorId::None, 0, lang)
        }
        "alignof_expression" => unary_payload(CFamilyAttr::UNARY_ALIGNOF),
        "offsetof_expression" => unary_payload(CFamilyAttr::UNARY_OFFSETOF),
        "generic_expression" => unary_payload(CFamilyAttr::UNARY_GENERIC_SELECTION),
        "call_expression" => {
            let qualified = node
                .child_by_field_name("function")
                .is_some_and(|f| matches!(f.kind(), "field_expression" | "qualified_identifier"));
            let aux = if qualified {
                CFamilyAttr::AUX_CALL_QUALIFIED
            } else {
                0
            };
            NodeAttr::pack(NodeAttr::VISIBILITY_NONE, 0, OperatorId::None, aux, 0)
        }
        "gnu_asm_expression" => NodeAttr::pack(
            NodeAttr::VISIBILITY_NONE,
            0,
            OperatorId::None,
            CFamilyAttr::AUX_CALL_INLINE_ASM,
            0,
        ),
        "field_expression" => {
            let lang = if operator_text(node, source) == "->" {
                CFamilyAttr::FIELD_ACCESS_ARROW
            } else {
                CFamilyAttr::FIELD_ACCESS_DOT
            };
            NodeAttr::pack(NodeAttr::VISIBILITY_NONE, 0, OperatorId::None, 0, lang)
        }
        "compound_literal_expression" => NodeAttr::pack(
            NodeAttr::VISIBILITY_NONE,
            0,
            OperatorId::None,
            0,
            CFamilyAttr::NEW_TEMPORARY,
        ),

        // ── Literals ──
        "number_literal" | "char_literal" | "string_literal" | "concatenated_string"
        | "system_lib_string" | "true" | "false" | "null" | "raw_string_literal"
        | "user_defined_literal" => encode_literal(node, source),

        _ => 0,
    }
}

fn unary_payload(lang: u8) -> u32 {
    NodeAttr::pack(NodeAttr::VISIBILITY_NONE, 0, OperatorId::None, 0, lang)
}

/// Attribute word of a function-like declaration.
pub(crate) fn encode_function_decl(node: &Node, node_type: ASTNodeType, source: &[u8]) -> u32 {
    let specs = collect_specifiers(node, source);
    let ctx = declaration_context(node);
    let declarator = node.child_by_field_name("declarator");
    let name = declarator.and_then(|d| declarator_shape(d).name);
    let qualified = name.map(|n| terminal_name(n).1).unwrap_or(false);

    let mut mods = 0u8;
    let mut aux = 0u8;
    let mut lang = 0u8;

    if specs.is_static {
        mods |= CFamilyAttr::MOD_STATIC;
    }
    if specs.is_extern || is_linkage_operand(node) {
        mods |= CFamilyAttr::MOD_EXTERN;
    }
    if specs.is_explicit {
        mods |= CFamilyAttr::MOD_EXPLICIT;
    }
    if specs.is_virtual {
        lang |= CFamilyAttr::LANG_VIRTUAL;
    }
    if specs.is_constexpr {
        lang |= CFamilyAttr::LANG_CONSTEXPR;
    }
    if specs.is_inline {
        lang |= CFamilyAttr::LANG_INLINE;
    }
    if specs.has_attributes {
        lang |= CFamilyAttr::LANG_HAS_ATTRIBUTES;
    }
    if specs.is_deprecated {
        lang |= CFamilyAttr::LANG_DEPRECATED;
    }

    if let Some(layer) = declarator.and_then(function_layer) {
        let mut cursor = layer.walk();
        for child in layer.children(&mut cursor) {
            match child.kind() {
                "virtual_specifier" => match node_text(&child, source) {
                    "override" => lang |= CFamilyAttr::LANG_OVERRIDE,
                    "final" => mods |= CFamilyAttr::MOD_FINAL,
                    _ => {}
                },
                "type_qualifier" => {
                    if node_text(&child, source) == "const" {
                        aux |= CFamilyAttr::AUX_FN_CONST;
                    }
                }
                "noexcept" => {
                    let text: String = node_text(&child, source)
                        .chars()
                        .filter(|c| !c.is_whitespace())
                        .collect();
                    if text == "noexcept" || text == "noexcept(true)" {
                        mods |= CFamilyAttr::MOD_NOEXCEPT;
                    }
                }
                "throw_specifier" => {
                    if child.named_child_count() == 0 {
                        mods |= CFamilyAttr::MOD_NOEXCEPT;
                    }
                }
                "attribute_specifier" | "attribute_declaration" => {
                    lang |= CFamilyAttr::LANG_HAS_ATTRIBUTES;
                    if node_text(&child, source).contains("deprecated") {
                        lang |= CFamilyAttr::LANG_DEPRECATED;
                    }
                }
                _ => {}
            }
        }
    }

    if node.child_by_field_name("body").is_some() {
        aux |= CFamilyAttr::AUX_FN_HAS_BODY;
    }
    if has_child_kind(node, "default_method_clause") {
        mods |= CFamilyAttr::MOD_DEFAULTED;
    }
    if has_child_kind(node, "delete_method_clause") {
        mods |= CFamilyAttr::MOD_DELETED;
    }
    if is_pure_virtual(node, source) {
        mods |= CFamilyAttr::MOD_ABSTRACT;
        if lang & CFamilyAttr::LANG_VIRTUAL == 0 {
            lang |= CFamilyAttr::LANG_VIRTUAL | CFamilyAttr::LANG_IMPLICIT_MODIFIERS;
        }
    }

    let in_class = matches!(ctx, DeclContext::Class(_));
    // `override` and `final` imply `virtual`.
    let overrides = lang & CFamilyAttr::LANG_OVERRIDE != 0 || mods & CFamilyAttr::MOD_FINAL != 0;
    if overrides && lang & CFamilyAttr::LANG_VIRTUAL == 0 {
        lang |= CFamilyAttr::LANG_VIRTUAL | CFamilyAttr::LANG_IMPLICIT_MODIFIERS;
    }
    // In-class and friend definitions, and constexpr functions, are implicitly inline.
    let implicit_inline = (aux & CFamilyAttr::AUX_FN_HAS_BODY != 0
        && (in_class || ctx == DeclContext::Friend))
        || lang & CFamilyAttr::LANG_CONSTEXPR != 0;
    if implicit_inline && lang & CFamilyAttr::LANG_INLINE == 0 {
        lang |= CFamilyAttr::LANG_INLINE | CFamilyAttr::LANG_IMPLICIT_MODIFIERS;
    }
    if qualified && !in_class {
        aux |= CFamilyAttr::AUX_FN_OUT_OF_CLASS;
    }

    let (vis, implicit) = declaration_visibility(node, ctx, specs.is_static, qualified, source);
    if implicit {
        lang |= CFamilyAttr::LANG_IMPLICIT_VISIBILITY;
    }

    let op = if node_type == ASTNodeType::NN_OPERATOR_DECL {
        name.map(|n| terminal_name(n).0)
            .filter(|t| t.kind() == "operator_name")
            .map(|t| overloaded_operator(&t, source))
            .unwrap_or(OperatorId::None)
    } else {
        OperatorId::None
    };

    NodeAttr::pack(vis, mods, op, aux, lang)
}

/// Attribute word of a variable or field declaration.
pub(crate) fn encode_variable_decl(node: &Node, source: &[u8]) -> u32 {
    let specs = collect_specifiers(node, source);
    let ctx = declaration_context(node);
    let first = node.child_by_field_name("declarator");
    let shape = first.map(declarator_shape).unwrap_or_default();
    let qualified = shape.name.map(|n| terminal_name(n).1).unwrap_or(false);

    let mut mods = 0u8;
    let mut aux = 0u8;
    let mut lang = 0u8;

    let indirect = shape.pointer || shape.reference;
    let const_object = (specs.is_const && !indirect)
        || first.is_some_and(|d| outer_pointer_is_const(d, source))
        || specs.is_constexpr;

    if specs.is_static {
        mods |= CFamilyAttr::MOD_STATIC;
    }
    if specs.is_extern || is_linkage_operand(node) {
        mods |= CFamilyAttr::MOD_EXTERN;
    }
    if specs.is_thread_local {
        mods |= CFamilyAttr::MOD_THREAD_LOCAL;
    }
    if specs.is_volatile {
        mods |= CFamilyAttr::MOD_VOLATILE;
    }
    if specs.is_mutable {
        mods |= CFamilyAttr::MOD_MUTABLE;
    }
    if specs.is_register {
        mods |= CFamilyAttr::MOD_REGISTER;
    }
    if const_object {
        mods |= CFamilyAttr::MOD_CONST;
    }

    let namespace_scope = ctx == DeclContext::File;
    if specs.is_constexpr || (const_object && (specs.is_static || namespace_scope)) {
        aux |= CFamilyAttr::AUX_VAR_CONSTANT;
    }
    if has_child_kind(node, "bitfield_clause") {
        aux |= CFamilyAttr::AUX_VAR_BITFIELD;
    }
    if specs.has_placeholder_type {
        aux |= CFamilyAttr::AUX_VAR_AUTO;
    }

    if specs.is_constexpr {
        lang |= CFamilyAttr::LANG_CONSTEXPR;
    }
    if specs.is_inline {
        lang |= CFamilyAttr::LANG_INLINE;
    }
    if specs.has_attributes {
        lang |= CFamilyAttr::LANG_HAS_ATTRIBUTES;
    }
    if specs.is_deprecated {
        lang |= CFamilyAttr::LANG_DEPRECATED;
    }

    let (vis, implicit) = declaration_visibility(node, ctx, specs.is_static, qualified, source);
    if implicit {
        lang |= CFamilyAttr::LANG_IMPLICIT_VISIBILITY;
    }
    NodeAttr::pack(vis, mods, OperatorId::None, aux, lang)
}

/// Attribute word of one `NN_VAR_DECLARATOR`.
pub(crate) fn encode_var_declarator(node: &Node, source: &[u8]) -> u32 {
    let shape = declarator_shape(*node);
    let mut mods = 0u8;
    let mut aux = 0u8;
    let mut lang = 0u8;
    if shape.pointer {
        aux |= CFamilyAttr::AUX_DECL_POINTER;
    }
    if shape.reference {
        aux |= CFamilyAttr::AUX_DECL_REFERENCE;
    }
    if shape.rvalue_reference {
        lang |= CFamilyAttr::DECL_RVALUE_REFERENCE;
    }
    if shape.array {
        aux |= CFamilyAttr::AUX_DECL_ARRAY;
    }
    if shape.function {
        aux |= CFamilyAttr::AUX_DECL_FUNCTION;
    }
    if shape.structured {
        lang |= CFamilyAttr::DECL_STRUCTURED_BINDING;
    }
    if shape.pack {
        lang |= CFamilyAttr::DECL_PACK;
    }
    if outer_pointer_is_const(*node, source) {
        mods |= CFamilyAttr::MOD_CONST;
    }
    // `init_declarator`, or a field / condition declarator followed by `= e` / `{...}`.
    let initialized = shape.has_initializer
        || node
            .next_sibling()
            .is_some_and(|s| matches!(s.kind(), "=" | "initializer_list"));
    if initialized {
        lang |= CFamilyAttr::DECL_HAS_INITIALIZER;
    }
    NodeAttr::pack(NodeAttr::VISIBILITY_NONE, mods, OperatorId::None, aux, lang)
}

/// Attribute word of a function parameter (also K&R parameter declarations).
pub(crate) fn encode_parameter(node: &Node, source: &[u8]) -> u32 {
    let specs = collect_specifiers(node, source);
    let declarator = node.child_by_field_name("declarator");
    let shape = declarator.map(declarator_shape).unwrap_or_default();

    let mut mods = 0u8;
    let mut aux = 0u8;
    let mut lang = 0u8;

    if (specs.is_const && !shape.pointer && !shape.reference && !shape.array)
        || declarator.is_some_and(|d| outer_pointer_is_const(d, source))
    {
        mods |= CFamilyAttr::MOD_CONST;
    }
    if specs.is_volatile {
        mods |= CFamilyAttr::MOD_VOLATILE;
    }
    if specs.is_register {
        mods |= CFamilyAttr::MOD_REGISTER;
    }
    // Array parameters adjust to pointers.
    if shape.pointer || shape.array || shape.function {
        lang |= CFamilyAttr::PARAM_POINTER;
    }
    if shape.reference {
        lang |= CFamilyAttr::PARAM_REFERENCE;
    }
    if shape.pack || matches!(node.kind(), "variadic_parameter_declaration" | "variadic_parameter")
    {
        lang |= CFamilyAttr::PARAM_VARIADIC;
    }
    if node.kind() == "optional_parameter_declaration"
        || node.child_by_field_name("default_value").is_some()
    {
        aux |= CFamilyAttr::AUX_PARAM_HAS_DEFAULT;
    }
    if node
        .parent()
        .and_then(|list| list.parent())
        .is_some_and(|owner| owner.kind() == "catch_clause")
    {
        aux |= CFamilyAttr::AUX_PARAM_CATCH;
    }
    if is_void_parameter_list(node, declarator.is_some(), source) {
        lang |= CFamilyAttr::PARAM_VOID_LIST;
    }
    if specs.has_attributes {
        lang |= CFamilyAttr::LANG_HAS_ATTRIBUTES;
    }
    if specs.is_deprecated {
        lang |= CFamilyAttr::LANG_DEPRECATED;
    }
    NodeAttr::pack(NodeAttr::VISIBILITY_NONE, mods, OperatorId::None, aux, lang)
}

/// `f(void)`: the only parameter is an unnamed, unqualified `void`.
fn is_void_parameter_list(param: &Node, has_declarator: bool, source: &[u8]) -> bool {
    if has_declarator || param.kind() != "parameter_declaration" {
        return false;
    }
    let is_void = param
        .child_by_field_name("type")
        .is_some_and(|t| t.kind() == "primitive_type" && node_text(&t, source) == "void");
    is_void
        && param
            .parent()
            .is_some_and(|list| list.kind() == "parameter_list" && list.named_child_count() == 1)
}

/// Attribute word of a template parameter (`NN_TYPE_PARAM`).
pub(crate) fn encode_template_parameter(node: &Node, source: &[u8]) -> u32 {
    let mut aux = 0u8;
    match node.kind() {
        "parameter_declaration" => aux |= CFamilyAttr::AUX_TPARAM_NON_TYPE,
        "optional_parameter_declaration" => {
            aux |= CFamilyAttr::AUX_TPARAM_NON_TYPE | CFamilyAttr::AUX_TPARAM_HAS_DEFAULT
        }
        "variadic_parameter_declaration" => {
            aux |= CFamilyAttr::AUX_TPARAM_NON_TYPE | CFamilyAttr::AUX_TPARAM_PACK
        }
        "variadic_type_parameter_declaration" => aux |= CFamilyAttr::AUX_TPARAM_PACK,
        "optional_type_parameter_declaration" => aux |= CFamilyAttr::AUX_TPARAM_HAS_DEFAULT,
        "template_template_parameter_declaration" => {
            aux |= CFamilyAttr::AUX_TPARAM_TEMPLATE;
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                match child.kind() {
                    "variadic_type_parameter_declaration" => aux |= CFamilyAttr::AUX_TPARAM_PACK,
                    "optional_type_parameter_declaration" => {
                        aux |= CFamilyAttr::AUX_TPARAM_HAS_DEFAULT
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
    let lang = if collect_specifiers(node, source).has_attributes {
        CFamilyAttr::LANG_HAS_ATTRIBUTES
    } else {
        0
    };
    NodeAttr::pack(NodeAttr::VISIBILITY_NONE, 0, OperatorId::None, aux, lang)
}

/// Attribute word of a struct / union / class / enum declaration.
pub(crate) fn encode_record(node: &Node, source: &[u8]) -> u32 {
    let mut mods = 0u8;
    let mut aux = 0u8;
    let mut lang = 0u8;
    let is_enum = node.kind() == "enum_specifier";
    let name = node.child_by_field_name("name");

    if node.child_by_field_name("body").is_none() {
        aux |= CFamilyAttr::AUX_RECORD_FORWARD;
    }
    if name.is_none() {
        aux |= CFamilyAttr::AUX_RECORD_ANONYMOUS;
    }
    if is_enum {
        if has_child_kind(node, "class") || has_child_kind(node, "struct") {
            aux |= CFamilyAttr::AUX_ENUM_SCOPED;
        }
        if node.child_by_field_name("base").is_some() {
            aux |= CFamilyAttr::AUX_ENUM_FIXED_BASE;
        }
    } else {
        if name.is_some_and(|n| n.kind() == "template_type") {
            aux |= CFamilyAttr::AUX_CLASS_SPECIALIZATION;
        }
        let (polymorphic, abstract_) = class_virtuality(node, source);
        if polymorphic {
            aux |= CFamilyAttr::AUX_CLASS_POLYMORPHIC;
        }
        if abstract_ {
            mods |= CFamilyAttr::MOD_ABSTRACT;
            lang |= CFamilyAttr::LANG_IMPLICIT_MODIFIERS;
        }
        let mut cursor = node.walk();
        let is_final = node.children(&mut cursor).any(|c| {
            c.kind() == "virtual_specifier" && node_text(&c, source) == "final"
        });
        if is_final {
            mods |= CFamilyAttr::MOD_FINAL;
        }
    }

    let specs = collect_specifiers(node, source);
    if specs.has_attributes {
        lang |= CFamilyAttr::LANG_HAS_ATTRIBUTES;
    }
    if specs.is_deprecated {
        lang |= CFamilyAttr::LANG_DEPRECATED;
    }

    let (vis, implicit) = match declaration_context(node) {
        DeclContext::Class(_) => {
            member_access(node, source).unwrap_or((NodeAttr::VISIBILITY_PUBLIC, true))
        }
        DeclContext::File => {
            if in_anonymous_namespace(node) {
                (NodeAttr::VISIBILITY_PRIVATE, true)
            } else {
                (NodeAttr::VISIBILITY_PUBLIC, true)
            }
        }
        _ => (NodeAttr::VISIBILITY_NONE, false),
    };
    if implicit {
        lang |= CFamilyAttr::LANG_IMPLICIT_VISIBILITY;
    }
    NodeAttr::pack(vis, mods, OperatorId::None, aux, lang)
}

/// Attribute word of a typedef, alias declaration or concept.
pub(crate) fn encode_typedef(node: &Node, source: &[u8], aux: u8) -> u32 {
    let specs = collect_specifiers(node, source);
    let mut lang = 0u8;
    if specs.has_attributes {
        lang |= CFamilyAttr::LANG_HAS_ATTRIBUTES;
    }
    if specs.is_deprecated {
        lang |= CFamilyAttr::LANG_DEPRECATED;
    }
    let (vis, implicit) = match declaration_context(node) {
        DeclContext::Class(_) => {
            member_access(node, source).unwrap_or((NodeAttr::VISIBILITY_PUBLIC, true))
        }
        DeclContext::File => {
            if in_anonymous_namespace(node) {
                (NodeAttr::VISIBILITY_PRIVATE, true)
            } else {
                (NodeAttr::VISIBILITY_PUBLIC, true)
            }
        }
        _ => (NodeAttr::VISIBILITY_NONE, false),
    };
    if implicit {
        lang |= CFamilyAttr::LANG_IMPLICIT_VISIBILITY;
    }
    NodeAttr::pack(vis, 0, OperatorId::None, aux, lang)
}

/// Attribute word of a kept `type_descriptor` (qualified or derived type).
fn encode_type_descriptor(node: &Node, source: &[u8]) -> u32 {
    let mut aux = 0u8;
    let mut lang = 0u8;
    let shape = node
        .child_by_field_name("declarator")
        .map(declarator_shape)
        .unwrap_or_default();
    if shape.pointer {
        lang |= CFamilyAttr::TYPE_POINTER;
    }
    if shape.reference {
        lang |= CFamilyAttr::TYPE_REFERENCE;
    }
    if shape.array {
        aux |= NodeAttr::AUX_TYPE_ARRAY;
    }
    if shape.function {
        lang |= CFamilyAttr::TYPE_FUNCTION_SIGNATURE;
    }
    if collect_specifiers(node, source).is_const && !shape.pointer && !shape.reference {
        lang |= CFamilyAttr::TYPE_CONST;
    }
    NodeAttr::pack(NodeAttr::VISIBILITY_NONE, 0, OperatorId::None, aux, lang)
}

/// `AUX_STMT_HAS_INIT` / `AUX_COND_DECLARATION` of a statement with a C++
/// `condition_clause`.
fn condition_flags(node: &Node) -> u8 {
    let mut aux = 0u8;
    if let Some(cond) = node.child_by_field_name("condition") {
        if cond.kind() == "condition_clause" {
            if cond.child_by_field_name("initializer").is_some() {
                aux |= CFamilyAttr::AUX_STMT_HAS_INIT;
            }
            if cond
                .child_by_field_name("value")
                .is_some_and(|v| v.kind() == "declaration")
            {
                aux |= CFamilyAttr::AUX_COND_DECLARATION;
            }
        }
    }
    aux
}

fn encode_binary(node: &Node, source: &[u8]) -> u32 {
    let symbol = normalize_operator(operator_text(node, source));
    let (op, lang) = match symbol {
        "<=>" => (OperatorId::None, CFamilyAttr::BINARY_THREE_WAY),
        ".*" | "->*" => (OperatorId::None, CFamilyAttr::BINARY_POINTER_TO_MEMBER),
        s => (
            OperatorId::from_symbol(s, OperatorArity::Binary).unwrap_or(OperatorId::None),
            CFamilyAttr::BINARY_PLAIN,
        ),
    };
    NodeAttr::pack(NodeAttr::VISIBILITY_NONE, 0, op, 0, lang)
}

fn encode_update(node: &Node, source: &[u8]) -> u32 {
    let op_node = node.child_by_field_name("operator");
    let op = op_node
        .and_then(|o| OperatorId::from_symbol(node_text(&o, source), OperatorArity::Unary))
        .unwrap_or(OperatorId::None);
    let postfix = match (op_node, node.child_by_field_name("argument")) {
        (Some(o), Some(a)) => o.start_byte() > a.start_byte(),
        _ => false,
    };
    let aux = if postfix {
        NodeAttr::AUX_UNARY_POSTFIX
    } else {
        0
    };
    NodeAttr::pack(NodeAttr::VISIBILITY_NONE, 0, op, aux, CFamilyAttr::UNARY_PLAIN)
}

/// Attribute word of a literal: the category in the lang byte, the form in the aux bits.
pub(crate) fn encode_literal(node: &Node, source: &[u8]) -> u32 {
    let text = node_text(node, source);
    let prefixed = |t: &str| {
        t.starts_with('L') || t.starts_with('u') || t.starts_with('U')
    };
    let (category, aux) = match node.kind() {
        "number_literal" => {
            let category = if is_floating_spelling(text.as_bytes()) {
                NodeAttr::LITERAL_FLOAT
            } else {
                NodeAttr::LITERAL_INTEGER
            };
            (category, 0)
        }
        "string_literal" | "system_lib_string" => (
            NodeAttr::LITERAL_STRING,
            if prefixed(text) {
                CFamilyAttr::AUX_LITERAL_ENCODING_PREFIX
            } else {
                0
            },
        ),
        "char_literal" => (
            NodeAttr::LITERAL_CHAR,
            if prefixed(text) {
                CFamilyAttr::AUX_LITERAL_ENCODING_PREFIX
            } else {
                0
            },
        ),
        "concatenated_string" => (NodeAttr::LITERAL_STRING, CFamilyAttr::AUX_LITERAL_CONCATENATED),
        "raw_string_literal" => {
            let mut aux = CFamilyAttr::AUX_LITERAL_RAW;
            if !text.starts_with('R') {
                aux |= CFamilyAttr::AUX_LITERAL_ENCODING_PREFIX;
            }
            (NodeAttr::LITERAL_STRING, aux)
        }
        "user_defined_literal" => {
            let category = match node.named_child(0).map(|c| c.kind()) {
                Some("number_literal") => {
                    let literal = node.named_child(0).map(|c| node_text(&c, source)).unwrap_or("");
                    if is_floating_spelling(literal.as_bytes()) {
                        NodeAttr::LITERAL_FLOAT
                    } else {
                        NodeAttr::LITERAL_INTEGER
                    }
                }
                Some("char_literal") => NodeAttr::LITERAL_CHAR,
                _ => NodeAttr::LITERAL_STRING,
            };
            (category, CFamilyAttr::AUX_LITERAL_USER_DEFINED)
        }
        "true" | "false" => (NodeAttr::LITERAL_BOOLEAN, 0),
        "null" => (NodeAttr::LITERAL_NULL, 0),
        _ => (0, 0),
    };
    NodeAttr::pack(NodeAttr::VISIBILITY_NONE, 0, OperatorId::None, aux, category)
}
