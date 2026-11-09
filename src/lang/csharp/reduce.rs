//! C# AST Reduction Adapter implementation.
//!
//! Maps tree-sitter-c-sharp (0.23; C# 1 through C# 12) concrete syntax trees onto the
//! language-neutral BP AST node set. Every named kind of the grammar's `node-types.json` is
//! classified explicitly; a construct whose role depends on its position (an `identifier`
//! naming a type, declaring a pattern variable, naming a `params` array or a query range
//! variable; a `variable_declaration` that is or is not already wrapped; a `member_access`
//! that is or is not the callee of an invocation) is decided from its parent and field.
//!
//! Shapes produced (children in source order):
//! - type: `[NN_ANNOTATION_USE*, NN_IDENTIFIER_EXPR name, NN_TYPE_PARAM*, NN_PARAM_DECL*
//!   (primary constructor / record positional parameters), NN_TYPE_REF base* | NN_CALL_EXPR
//!   base-constructor call, NN_TYPE_PARAM constraint-clause*, members*]`. A base list does not
//!   say which entry is the base class; Phase 3 decides. `record B(int X) : A(X)` gives
//!   `NN_CALL_EXPR [NN_TYPE_REF A, X]` and `class B(int x) : A(x)` gives `NN_TYPE_REF A`
//!   followed by `NN_CALL_EXPR [x]`, both flagged `AUX_CALL_EXPLICIT_CTOR |
//!   AUX_CALL_SUPER_TARGET`.
//! - namespace: `namespace N { ... }` is `NN_NAMESPACE_DECL [name, members*]`. A file-scoped
//!   `namespace N;` is a childless `NN_NAMESPACE_DECL [name]` flagged
//!   `AUX_NAMESPACE_FILE_SCOPED`; the declarations that follow it in the compilation unit are
//!   its members.
//! - `using` directive: `NN_USING_DIRECTIVE [alias?, name]` (`AUX_USING_*`); a qualified name
//!   is one `NN_IDENTIFIER_EXPR` (`AUX_IDENT_QUALIFIED`) whose children are its segments.
//! - method / local function / operator / accessor: `[NN_ANNOTATION_USE*, NN_TYPE_REF
//!   return?, NN_TYPE_REF interface?, NN_IDENTIFIER_EXPR name?, NN_TYPE_PARAM*,
//!   NN_PARAM_DECL*, NN_TYPE_PARAM constraint-clause*, body?]`. With `AUX_EXPLICIT_IMPL` the
//!   second `NN_TYPE_REF` names the implemented interface. An accessor is an `NN_METHOD_DECL`
//!   whose parent is an `NN_PROPERTY_DECL`, `NN_INDEXER_DECL` or `NN_EVENT_DECL`; its kind
//!   (`get` / `set` / `init` / `add` / `remove`) is in the lang byte. Local functions are
//!   `NN_FUNCTION_DECL`.
//! - an expression body `=> e` is `NN_RETURN_STMT [e]` flagged `AUX_STMT_EXPRESSION_BODY`;
//!   when the member returns no value (void, constructor, destructor, `set` / `init` / `add` /
//!   `remove`, `async Task`) it also carries `AUX_STMT_DISCARDS_VALUE`: `e` is evaluated for
//!   its effects, then the member returns.
//! - parameter: `NN_PARAM_DECL [NN_ANNOTATION_USE*, NN_TYPE_REF?, NN_IDENTIFIER_EXPR name,
//!   default?]`; the passing mode is in the lang byte. The grammar keeps the `params T[] xs`
//!   rule hidden, so a parameter array is an `NN_TYPE_REF` followed by an `NN_PARAM_DECL`
//!   leaf (the name) flagged `LANG_JAVA_VARARGS`. `x => ...` gives an `NN_PARAM_DECL` leaf
//!   (`AUX_PARAM_LAMBDA_INFERRED`).
//! - variables: a local declaration statement, a field and an event field hold an
//!   `NN_TYPE_REF` and one `NN_LOCAL_VAR_DECL` per declarator (`[NN_IDENTIFIER_EXPR name,
//!   initializer?]`, or nested declarators for `var (a, b) = ...`). Pattern designations
//!   (`is int x`, `out var y`, `foreach (var (k, v) in d)`, `case (var a, _)`) are
//!   `NN_LOCAL_VAR_DECL` leaves whose token is the declared name.
//! - `a.b(c)` is `NN_CALL_EXPR [a, NN_IDENTIFIER_EXPR b, c]` (`AUX_CALL_QUALIFIED`) and
//!   `a?.b(c)` adds `AUX_NULL_AWARE`; `b<T>(c)` is `NN_CALL_EXPR [NN_IDENTIFIER_EXPR b [T],
//!   c]` (`AUX_IDENT_GENERIC`). `a.b` is `NN_FIELD_ACCESS [a, b]`, `a[i]` `NN_ARRAY_ACCESS`.
//!   A `ref` / `out` / `in` argument is an `NN_UNARY_EXPR` around the argument (payload
//!   `UNARY_REF` / `UNARY_OUT` / `UNARY_IN`); argument names (`f(x: 1)`) are dropped.
//! - `switch` statement sections and switch-expression arms are `NN_SWITCH_CASE`; a section
//!   holds its label values or patterns, then its statements; an arm holds its pattern and
//!   its value. A `when` guard is an `NN_PATTERN_MATCH` (`PATTERN_GUARD`) whose child is the
//!   condition; it guards the label or arm pattern immediately before it.
//! - `checked` / `unchecked` / `unsafe` / `fixed` statements are `NN_BLOCK`s flagged
//!   `AUX_BLOCK_*`; a `fixed` block holds the pinned `NN_LOCAL_VAR_DECL` then the body.
//! - `foreach` is `NN_FOREACH_STMT`, `lock` `NN_LOCK_STMT`, `using (...)` `NN_USING_STMT`
//!   (a `using var` declaration is an `NN_LOCAL_VAR_DECL` flagged `AUX_LOCAL_USING`),
//!   `goto` `NN_GOTO_STMT`, `l:` `NN_LABELED_STMT`. `yield return` / `yield break` are
//!   `NN_YIELD_STMT`, the latter flagged `AUX_YIELD_BREAK`.
//! - LINQ: `query_expression` and each of its clauses are `NN_CALL_EXPR` nodes (the method
//!   calls a query lowers to) identified by the lang payload `CALL_QUERY_*`; range variables
//!   are `NN_PARAM_DECL` leaves (`AUX_PARAM_LAMBDA_INFERRED`).
//! - anonymous-type members (`new { Name = v }`) are `NN_PROPERTY_DECL` leaves followed by
//!   their value; a `with` initializer holds `member, value` pairs.
//!
//! Conditional compilation: tree-sitter-c-sharp parses every `#if` / `#elif` / `#else`
//! branch, so the code of all branches is reduced; the directive conditions are dropped, as
//! are all other directives (they are not program constructs).
//!
//! Syntax errors: ERROR nodes are eliminated (their recoverable children attach to the
//! enclosing node) and MISSING nodes are dropped (they have no source text); both are counted,
//! and so are named kinds absent from the grammar's `node-types.json`.

use std::sync::atomic::{AtomicU64, Ordering};

use tree_sitter::Node;

use crate::ast::adapter::{ASTReductionAdapter, ReductionDecision};
use crate::core::types::ast::{ASTNodeType, NodeAttr, OperatorArity, OperatorId};

/// C#-specific payloads of the `NodeAttr` word.
///
/// Visibility uses the shared codes plus the C#-only access levels (the same codes as the
/// neutral `Visibility` vocabulary: 0x05, 0x06, and 0x07 for `file` types). Declaration lang bits
/// 3..7 keep their shared meaning (`NodeAttr::LANG_JAVA_VARARGS` marks a `params` parameter,
/// then `LANG_IMPLICIT_VISIBILITY`, `LANG_IMPLICIT_MODIFIERS`, `LANG_HAS_ANNOTATIONS`
/// (attributes) and `LANG_DEPRECATED` (`[Obsolete]`)); bits 0..2 and the aux nibble are
/// interpreted per node type as listed here. Aux bits that also exist for Java keep the Java
/// value (`AUX_METHOD_HAS_BODY`, `AUX_FIELD_CONSTANT`, `AUX_PARAM_*`, `AUX_CALL_*`,
/// `AUX_CASE_*`, `AUX_LAMBDA_EXPRESSION_BODY`, `AUX_TYPE_*`, `AUX_IDENT_QUALIFIED`,
/// `AUX_UNARY_POSTFIX`, `AUX_INSTANCEOF_PATTERN`, `AUX_SWITCH_ARROW_FORM`).
///
/// Modifiers map onto the shared `MOD_*` bits: `static` → `MOD_STATIC`; `sealed`, `readonly`
/// (fields and struct members) and `const` → `MOD_FINAL`; `abstract` → `MOD_ABSTRACT`;
/// `extern` → `MOD_NATIVE`; `volatile` → `MOD_VOLATILE`; `[NonSerialized]` →
/// `MOD_TRANSIENT`; `[MethodImpl(MethodImplOptions.Synchronized)]` → `MOD_SYNCHRONIZED`.
/// A `sealed override` member carries `MOD_FINAL` with `LANG_OVERRIDE`. Rules of the language
/// add implied bits with `LANG_IMPLICIT_MODIFIERS`: structs, enums and delegates are sealed,
/// interfaces abstract, `const` static, enum members `public static const`, and an interface
/// member without `static`, `sealed`, `private` or `extern` is abstract when it has no body
/// and virtual when it has one. Implied visibility (`internal` for top-level types, `private`
/// for class and struct members, `public` for interface and enum members, `protected` for
/// finalizers, the property's for its accessors) carries `LANG_IMPLICIT_VISIBILITY`.
pub struct CSharpAttr;

impl CSharpAttr {
    // ── Visibility nibble ──
    /// `internal` (same assembly); the shared package-level code.
    pub const VISIBILITY_INTERNAL: u8 = NodeAttr::VISIBILITY_PACKAGE_PRIVATE;
    /// `protected internal` (derived types, or the same assembly).
    pub const VISIBILITY_PROTECTED_INTERNAL: u8 = 0x05;
    /// `private protected` (derived types within the same assembly).
    pub const VISIBILITY_PRIVATE_PROTECTED: u8 = 0x06;
    /// `file` type (visible only in its source file).
    pub const VISIBILITY_FILE: u8 = 0x07;

    // ── Lang bits 0..2 of member and type declarations ──
    /// `partial` type, method or property.
    pub const LANG_PARTIAL: u8 = 1 << 0;
    /// `virtual` member (written, or implied for an interface member with a body).
    pub const LANG_VIRTUAL: u8 = 1 << 1;
    /// `override` member.
    pub const LANG_OVERRIDE: u8 = 1 << 2;

    // ── Lang bits 0..2 of an accessor (NN_METHOD_DECL under a property, indexer or event);
    //    accessors cannot be partial, virtual or override themselves ──
    pub const LANG_ACCESSOR_MASK: u8 = 0x07;
    /// An accessor named by an identifier (only produced by error recovery).
    pub const ACCESSOR_OTHER: u8 = 0;
    pub const ACCESSOR_GET: u8 = 1;
    pub const ACCESSOR_SET: u8 = 2;
    pub const ACCESSOR_INIT: u8 = 3;
    pub const ACCESSOR_ADD: u8 = 4;
    pub const ACCESSOR_REMOVE: u8 = 5;

    // ── Lang bits 0..2 of NN_PARAM_DECL: passing mode ──
    pub const LANG_PASS_MASK: u8 = 0x07;
    pub const PASS_VALUE: u8 = 0;
    pub const PASS_REF: u8 = 1;
    pub const PASS_OUT: u8 = 2;
    pub const PASS_IN: u8 = 3;
    pub const PASS_REF_READONLY: u8 = 4;

    // ── Lang bits 0..2 of NN_LOCAL_VAR_DECL ──
    /// `const` local (also `MOD_FINAL`).
    pub const LANG_LOCAL_CONST: u8 = 1 << 0;
    /// `ref` local (`ref int r = ref a[0];`).
    pub const LANG_LOCAL_REF: u8 = 1 << 1;
    /// `scoped` local.
    pub const LANG_LOCAL_SCOPED: u8 = 1 << 2;

    // ── NN_NAMESPACE_DECL ──
    /// `namespace N;`: the declarations after it in the compilation unit are its members.
    pub const AUX_NAMESPACE_FILE_SCOPED: u8 = 1 << 0;

    // ── NN_USING_DIRECTIVE ──
    /// `global using`.
    pub const AUX_USING_GLOBAL: u8 = 1 << 0;
    /// `using static T;`: imports the static members of a type.
    pub const AUX_USING_STATIC: u8 = 1 << 1;
    /// `using A = T;`: the first child is the alias, the second the aliased name or type.
    pub const AUX_USING_ALIAS: u8 = 1 << 2;
    /// `extern alias A;`.
    pub const AUX_USING_EXTERN_ALIAS: u8 = 1 << 3;

    // ── Type declarations: NN_CLASS_DECL / NN_STRUCT_DECL / NN_INTERFACE_DECL /
    //    NN_RECORD_DECL / NN_ENUM_DECL / NN_DELEGATE_DECL ──
    /// `record struct` (a value type; `record` and `record class` are reference types).
    pub const AUX_DECL_RECORD_STRUCT: u8 = 1 << 0;
    /// `readonly struct` / `readonly record struct`.
    pub const AUX_DECL_READONLY: u8 = 1 << 1;
    /// `ref struct` (stack-only type).
    pub const AUX_DECL_REF_STRUCT: u8 = 1 << 2;
    /// The type declares a primary constructor (or record positional parameters); its
    /// parameters are the `NN_PARAM_DECL` children of the type.
    pub const AUX_DECL_PRIMARY_CTOR: u8 = 1 << 3;

    // ── NN_METHOD_DECL / NN_FUNCTION_DECL (bit 1 is NodeAttr::AUX_METHOD_HAS_BODY) ──
    /// `async` method or local function.
    pub const AUX_METHOD_ASYNC: u8 = 1 << 2;
    /// Explicit interface implementation (`void IFoo.M()`), for methods, operators,
    /// properties, indexers and events: the member's second `NN_TYPE_REF` child names the
    /// interface.
    pub const AUX_EXPLICIT_IMPL: u8 = 1 << 3;

    // ── NN_OPERATOR_DECL (bit 1 is AUX_METHOD_HAS_BODY, bit 3 AUX_EXPLICIT_IMPL); the
    //    operator id is set for `operator +` and friends, `OperatorId::None` for
    //    `operator true` / `operator false` (whose keyword is an NN_LITERAL child) and for
    //    conversions ──
    /// `implicit operator T` / `explicit operator T` (user-defined conversion).
    pub const AUX_OPERATOR_CONVERSION: u8 = 1 << 0;
    /// For a conversion: `implicit` (clear: `explicit`). For any other operator: `operator
    /// checked`. A checked conversion is always `explicit`; its `checked` keyword is not
    /// encoded (the four aux bits are taken) and is available from the CST.
    pub const AUX_OPERATOR_IMPLICIT_OR_CHECKED: u8 = 1 << 2;

    // ── NN_PROPERTY_DECL / NN_INDEXER_DECL (bit 3 is AUX_EXPLICIT_IMPL) ──
    /// Auto-implemented property (`{ get; set; }` without bodies, in a class or struct).
    pub const AUX_PROPERTY_AUTO: u8 = 1 << 0;
    /// Expression-bodied (`int X => e;`): the body is an NN_RETURN_STMT child.
    pub const AUX_PROPERTY_EXPRESSION_BODIED: u8 = 1 << 1;
    /// `required` member.
    pub const AUX_PROPERTY_REQUIRED: u8 = 1 << 2;

    // ── NN_EVENT_DECL (bit 3 is AUX_EXPLICIT_IMPL) ──
    /// Field-like event (`event EventHandler E;`): its declarators are NN_LOCAL_VAR_DECL.
    pub const AUX_EVENT_FIELD_LIKE: u8 = 1 << 0;

    // ── NN_FIELD_DECL (bit 0 is NodeAttr::AUX_FIELD_CONSTANT, set for `const`) ──
    /// `fixed` size buffer (`fixed byte b[16];`).
    pub const AUX_FIELD_FIXED_BUFFER: u8 = 1 << 1;
    /// `required` member.
    pub const AUX_FIELD_REQUIRED: u8 = 1 << 2;
    /// `ref` field of a ref struct.
    pub const AUX_FIELD_REF: u8 = 1 << 3;

    // ── NN_PARAM_DECL (bits 0..2 are AUX_PARAM_RECORD_COMPONENT, AUX_PARAM_CATCH,
    //    AUX_PARAM_LAMBDA_INFERRED) ──
    /// `this` receiver parameter of an extension method.
    pub const AUX_PARAM_THIS: u8 = 1 << 3;

    // ── NN_LOCAL_VAR_DECL ──
    /// `using var x = ...;` declaration (disposed at the end of the enclosing block).
    pub const AUX_LOCAL_USING: u8 = 1 << 0;
    /// `await using var x = ...;`.
    pub const AUX_LOCAL_AWAIT: u8 = 1 << 1;
    /// Deconstructing declarator (`var (a, b) = t;`): its children are the nested
    /// declarators, then the initializer.
    pub const AUX_LOCAL_DECONSTRUCTION: u8 = 1 << 2;
    /// A discard designation (`_`): declares nothing.
    pub const AUX_LOCAL_DISCARD: u8 = 1 << 3;

    // ── NN_TYPE_PARAM ──
    /// `in T` (contravariant).
    pub const AUX_TYPE_PARAM_IN: u8 = 1 << 0;
    /// `out T` (covariant).
    pub const AUX_TYPE_PARAM_OUT: u8 = 1 << 1;
    /// A `where T : ...` constraint clause: the first child names the type parameter, the
    /// `NN_TYPE_REF` children are type constraints, and the lang byte holds `CONSTRAINT_*`.
    pub const AUX_TYPE_PARAM_CONSTRAINTS: u8 = 1 << 2;
    pub const CONSTRAINT_CLASS: u8 = 1 << 0;
    /// `class?`.
    pub const CONSTRAINT_NULLABLE_CLASS: u8 = 1 << 1;
    pub const CONSTRAINT_STRUCT: u8 = 1 << 2;
    pub const CONSTRAINT_NOTNULL: u8 = 1 << 3;
    pub const CONSTRAINT_UNMANAGED: u8 = 1 << 4;
    /// `new()`.
    pub const CONSTRAINT_NEW: u8 = 1 << 5;
    /// `default` (override / explicit-implementation constraint).
    pub const CONSTRAINT_DEFAULT: u8 = 1 << 6;

    // ── NN_BLOCK ──
    pub const AUX_BLOCK_UNSAFE: u8 = 1 << 0;
    pub const AUX_BLOCK_CHECKED: u8 = 1 << 1;
    pub const AUX_BLOCK_UNCHECKED: u8 = 1 << 2;
    /// `fixed (T* p = ...) body`: the pinned NN_LOCAL_VAR_DECL, then the body.
    pub const AUX_BLOCK_FIXED: u8 = 1 << 3;

    // ── NN_RETURN_STMT ──
    /// Synthesized from an expression body (`=> e`).
    pub const AUX_STMT_EXPRESSION_BODY: u8 = 1 << 0;
    /// The member returns no value: the expression is evaluated for its effects only.
    pub const AUX_STMT_DISCARDS_VALUE: u8 = 1 << 1;

    // ── NN_THROW_STMT ──
    /// `throw` expression (`x ?? throw new E()`).
    pub const AUX_THROW_EXPRESSION: u8 = 1 << 0;

    // ── NN_SWITCH_CASE (bits 0..2 are AUX_CASE_ARROW, AUX_CASE_DEFAULT, AUX_CASE_PATTERN) ──
    /// A `when` guard follows a pattern of the case.
    pub const AUX_CASE_GUARD: u8 = 1 << 3;

    // ── NN_CATCH_CLAUSE ──
    /// `catch (...) when (cond)`: the filter condition precedes the block.
    pub const AUX_CATCH_FILTER: u8 = 1 << 0;
    /// `catch { }` without a declaration (catches everything).
    pub const AUX_CATCH_GENERAL: u8 = 1 << 1;

    // ── NN_FOREACH_STMT ──
    /// `await foreach`.
    pub const AUX_FOREACH_AWAIT: u8 = 1 << 0;
    /// `foreach (var (a, b) in ...)`.
    pub const AUX_FOREACH_DECONSTRUCTION: u8 = 1 << 1;

    // ── NN_USING_STMT ──
    /// `await using (...)`.
    pub const AUX_USING_STMT_AWAIT: u8 = 1 << 0;
    /// The resource is declared (`using (var r = ...)`): the first child is an
    /// NN_LOCAL_VAR_DECL; otherwise it is an expression.
    pub const AUX_USING_STMT_DECLARATION: u8 = 1 << 1;

    // ── NN_GOTO_STMT ──
    /// `goto case X;`.
    pub const AUX_GOTO_CASE: u8 = 1 << 0;
    /// `goto default;`.
    pub const AUX_GOTO_DEFAULT: u8 = 1 << 1;

    // ── NN_YIELD_STMT (an iterator suspension point; `yield return x` has no flag) ──
    /// `yield break`: the iterator ends.
    pub const AUX_YIELD_BREAK: u8 = 1 << 0;

    // ── NN_CALL_EXPR / NN_FIELD_ACCESS / NN_ARRAY_ACCESS / NN_BINARY_EXPR / NN_ASSIGN_EXPR ──
    /// Null-aware form: `a?.b`, `a?[i]`, `a?.m()`, `a ?? b`, `a ??= b`.
    pub const AUX_NULL_AWARE: u8 = 1 << 3;

    // ── NN_FIELD_ACCESS / NN_ARRAY_ACCESS ──
    /// The receiver is implicit (`.b` / `[i]` of an object or collection initializer).
    pub const AUX_ACCESS_IMPLICIT_RECEIVER: u8 = 1 << 0;
    /// Pointer member access `p->b`.
    pub const AUX_ACCESS_POINTER: u8 = 1 << 1;

    // ── NN_CAST_EXPR ──
    /// `e as T` (yields null instead of throwing).
    pub const AUX_CAST_AS: u8 = 1 << 0;

    // ── NN_LAMBDA_EXPR (bit 0 is AUX_LAMBDA_EXPRESSION_BODY) ──
    pub const AUX_LAMBDA_ASYNC: u8 = 1 << 1;
    pub const AUX_LAMBDA_STATIC: u8 = 1 << 2;
    /// `delegate (...) { ... }` anonymous method.
    pub const AUX_LAMBDA_ANONYMOUS_METHOD: u8 = 1 << 3;

    // ── NN_ARRAY_CREATE ──
    /// `stackalloc` allocation.
    pub const AUX_ARRAY_STACKALLOC: u8 = 1 << 0;
    /// Element type inferred (`new[] { ... }`, `stackalloc[] { ... }`).
    pub const AUX_ARRAY_IMPLICIT: u8 = 1 << 1;

    // ── NN_LITERAL (the lang byte is NodeAttr::LITERAL_*; raw strings are
    //    LITERAL_TEXT_BLOCK) ──
    /// Interpolated string; its interpolations are the children.
    pub const AUX_STRING_INTERPOLATED: u8 = 1 << 0;
    /// Verbatim string (`@"..."`, `$@"..."`).
    pub const AUX_STRING_VERBATIM: u8 = 1 << 1;
    /// Raw string (`"""..."""`, `$"""..."""`).
    pub const AUX_STRING_RAW: u8 = 1 << 2;
    /// UTF-8 string literal (`"..."u8`, a `ReadOnlySpan<byte>`).
    pub const AUX_STRING_UTF8: u8 = 1 << 3;
    /// `default` / `default(T)`.
    pub const LITERAL_DEFAULT: u8 = 9;

    // ── NN_IDENTIFIER_EXPR (bit 0 is NodeAttr::AUX_IDENT_QUALIFIED) ──
    /// Generic name in expression position (`M<int>`): the children are the type arguments.
    pub const AUX_IDENT_GENERIC: u8 = 1 << 1;
    /// The name starts with an alias qualifier (`global::System`, `A::B`).
    pub const AUX_IDENT_ALIAS_QUALIFIED: u8 = 1 << 2;

    // ── NN_TYPE_REF lang payload (when neither AUX_TYPE_ARRAY nor AUX_TYPE_WILDCARD is set;
    //    a wrapper type's child is the wrapped type) ──
    pub const TYPE_NAMED: u8 = 0;
    /// `var`.
    pub const TYPE_IMPLICIT: u8 = 1;
    /// A predefined type keyword (`int`, `string`, `object`, ...).
    pub const TYPE_PREDEFINED: u8 = 2;
    pub const TYPE_VOID: u8 = 3;
    /// `T?`.
    pub const TYPE_NULLABLE: u8 = 4;
    /// `T*`.
    pub const TYPE_POINTER: u8 = 5;
    /// `ref T` / `ref readonly T`.
    pub const TYPE_REF: u8 = 6;
    /// `scoped ref T` / `scoped T`.
    pub const TYPE_SCOPED: u8 = 7;
    /// `(int a, string b)`; the children are the element types.
    pub const TYPE_TUPLE: u8 = 8;
    /// `delegate*<int, void>`; the children are the parameter types, then the return type.
    pub const TYPE_FUNCTION_POINTER: u8 = 9;

    // ── NN_UNARY_EXPR lang payload (operator id `None`) ──
    pub const UNARY_ADDRESS_OF: u8 = 1;
    pub const UNARY_DEREFERENCE: u8 = 2;
    /// `^i` (index from end).
    pub const UNARY_INDEX_FROM_END: u8 = 3;
    pub const UNARY_AWAIT: u8 = 4;
    pub const UNARY_SIZEOF: u8 = 5;
    pub const UNARY_CHECKED: u8 = 6;
    pub const UNARY_UNCHECKED: u8 = 7;
    /// `ref e` expression, or a `ref` argument.
    pub const UNARY_REF: u8 = 8;
    /// `out` argument (the operand is assigned by the callee).
    pub const UNARY_OUT: u8 = 9;
    /// `in` argument.
    pub const UNARY_IN: u8 = 10;
    pub const UNARY_MAKEREF: u8 = 11;
    pub const UNARY_REFTYPE: u8 = 12;
    pub const UNARY_REFVALUE: u8 = 13;

    // ── NN_BINARY_EXPR lang payload ──
    /// `a..b` range (either operand may be absent).
    pub const BINARY_RANGE: u8 = 1;

    // ── NN_NEW_EXPR lang payload ──
    pub const NEW_OBJECT: u8 = 0;
    /// `new(args)` (type from the target).
    pub const NEW_TARGET_TYPED: u8 = 1;
    /// `new { A = 1 }` (anonymous type).
    pub const NEW_ANONYMOUS_TYPE: u8 = 2;
    /// `e with { ... }` (non-destructive mutation: copy, then assign the listed members).
    pub const NEW_WITH: u8 = 3;

    // ── NN_ARRAY_INIT lang payload ──
    pub const INIT_ARRAY: u8 = 0;
    /// Object initializer (`{ A = 1, B = 2 }`); also the member list of `with`.
    pub const INIT_OBJECT: u8 = 1;
    /// Collection initializer (`{ 1, 2 }`, `{ { "k", v } }`): one `Add` call per element.
    pub const INIT_COLLECTION: u8 = 2;
    /// Tuple literal `(a, b)` (also the target of a deconstructing assignment).
    pub const INIT_TUPLE: u8 = 3;

    // ── NN_PATTERN_MATCH lang payload ──
    pub const PATTERN_CONSTANT: u8 = 1;
    /// `T x` (declares x).
    pub const PATTERN_DECLARATION: u8 = 2;
    /// `var x`.
    pub const PATTERN_VAR: u8 = 3;
    /// `_`.
    pub const PATTERN_DISCARD: u8 = 4;
    /// `T` (type test).
    pub const PATTERN_TYPE: u8 = 5;
    /// `T(..) { .. } x` positional / property pattern.
    pub const PATTERN_RECURSIVE: u8 = 6;
    /// `< e`, `>= e`, ...; the operator id holds the comparison.
    pub const PATTERN_RELATIONAL: u8 = 7;
    /// `not p` (operator id `LogicalNot`).
    pub const PATTERN_NOT: u8 = 8;
    /// `p and q` (operator id `LogicalAnd`).
    pub const PATTERN_AND: u8 = 9;
    /// `p or q` (operator id `LogicalOr`).
    pub const PATTERN_OR: u8 = 10;
    /// `[p, q, ..]`.
    pub const PATTERN_LIST: u8 = 11;
    /// `..` inside a list pattern.
    pub const PATTERN_SLICE: u8 = 12;
    /// `Member: p` inside a property or positional pattern; the member then the pattern.
    pub const PATTERN_SUBPATTERN: u8 = 13;
    /// `when cond` guard of the preceding case label or arm pattern.
    pub const PATTERN_GUARD: u8 = 14;

    // ── NN_CALL_EXPR lang payload ──
    pub const CALL_INVOCATION: u8 = 0;
    pub const CALL_QUERY: u8 = 1;
    pub const CALL_QUERY_FROM: u8 = 2;
    pub const CALL_QUERY_JOIN: u8 = 3;
    pub const CALL_QUERY_JOIN_INTO: u8 = 4;
    pub const CALL_QUERY_LET: u8 = 5;
    pub const CALL_QUERY_WHERE: u8 = 6;
    pub const CALL_QUERY_ORDER_BY: u8 = 7;
    pub const CALL_QUERY_SELECT: u8 = 8;
    pub const CALL_QUERY_GROUP: u8 = 9;

    // ── NN_ANNOTATION_USE lang payload: attribute target ──
    pub const TARGET_DEFAULT: u8 = 0;
    pub const TARGET_ASSEMBLY: u8 = 1;
    pub const TARGET_MODULE: u8 = 2;
    pub const TARGET_FIELD: u8 = 3;
    pub const TARGET_EVENT: u8 = 4;
    pub const TARGET_METHOD: u8 = 5;
    pub const TARGET_PARAM: u8 = 6;
    pub const TARGET_PROPERTY: u8 = 7;
    pub const TARGET_RETURN: u8 = 8;
    pub const TARGET_TYPE: u8 = 9;
}

/// Modifier keywords written on a C# declaration, parameter or lambda.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct CSharpModifiers(u32);

impl CSharpModifiers {
    pub const PUBLIC: u32 = 1 << 0;
    pub const PRIVATE: u32 = 1 << 1;
    pub const PROTECTED: u32 = 1 << 2;
    pub const INTERNAL: u32 = 1 << 3;
    pub const STATIC: u32 = 1 << 4;
    pub const ABSTRACT: u32 = 1 << 5;
    pub const VIRTUAL: u32 = 1 << 6;
    pub const OVERRIDE: u32 = 1 << 7;
    pub const SEALED: u32 = 1 << 8;
    pub const READONLY: u32 = 1 << 9;
    pub const ASYNC: u32 = 1 << 10;
    pub const EXTERN: u32 = 1 << 11;
    pub const PARTIAL: u32 = 1 << 12;
    pub const CONST: u32 = 1 << 13;
    pub const NEW: u32 = 1 << 14;
    pub const UNSAFE: u32 = 1 << 15;
    pub const VOLATILE: u32 = 1 << 16;
    pub const REQUIRED: u32 = 1 << 17;
    pub const FIXED: u32 = 1 << 18;
    pub const FILE: u32 = 1 << 19;
    pub const REF: u32 = 1 << 20;
    pub const SCOPED: u32 = 1 << 21;
    /// Parameter modifiers.
    pub const THIS: u32 = 1 << 22;
    pub const OUT: u32 = 1 << 23;
    pub const IN: u32 = 1 << 24;
    pub const PARAMS: u32 = 1 << 25;

    /// The modifier bit a keyword stands for (0 for any other word).
    pub fn keyword_bit(word: &str) -> u32 {
        match word {
            "public" => Self::PUBLIC,
            "private" => Self::PRIVATE,
            "protected" => Self::PROTECTED,
            "internal" => Self::INTERNAL,
            "static" => Self::STATIC,
            "abstract" => Self::ABSTRACT,
            "virtual" => Self::VIRTUAL,
            "override" => Self::OVERRIDE,
            "sealed" => Self::SEALED,
            "readonly" => Self::READONLY,
            "async" => Self::ASYNC,
            "extern" => Self::EXTERN,
            "partial" => Self::PARTIAL,
            "const" => Self::CONST,
            "new" => Self::NEW,
            "unsafe" => Self::UNSAFE,
            "volatile" => Self::VOLATILE,
            "required" => Self::REQUIRED,
            "fixed" => Self::FIXED,
            "file" => Self::FILE,
            "ref" => Self::REF,
            "scoped" => Self::SCOPED,
            "this" => Self::THIS,
            "out" => Self::OUT,
            "in" => Self::IN,
            "params" => Self::PARAMS,
            _ => 0,
        }
    }

    /// The modifiers written on `node`: its `modifier` children, plus the modifier keywords
    /// the grammar places directly on a declaration (`ref struct`, `extern` on a finalizer).
    pub fn of(node: &Node<'_>, source: &[u8]) -> Self {
        let mut bits = 0u32;
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "modifier" {
                for word in node_text(&child, source).split_whitespace() {
                    bits |= Self::keyword_bit(word);
                }
            } else if !child.is_named() && matches!(child.kind(), "ref" | "extern" | "async") {
                bits |= Self::keyword_bit(child.kind());
            }
        }
        Self(bits)
    }

    pub fn bits(self) -> u32 {
        self.0
    }

    /// True when any of `bits` is present.
    pub fn contains_any(self, bits: u32) -> bool {
        self.0 & bits != 0
    }

    /// The written access level, as a `NodeAttr` visibility code.
    pub fn visibility(self) -> Option<u8> {
        let public = self.contains_any(Self::PUBLIC);
        let private = self.contains_any(Self::PRIVATE);
        let protected = self.contains_any(Self::PROTECTED);
        let internal = self.contains_any(Self::INTERNAL);
        if public {
            Some(NodeAttr::VISIBILITY_PUBLIC)
        } else if protected && internal {
            Some(CSharpAttr::VISIBILITY_PROTECTED_INTERNAL)
        } else if private && protected {
            Some(CSharpAttr::VISIBILITY_PRIVATE_PROTECTED)
        } else if private {
            Some(NodeAttr::VISIBILITY_PRIVATE)
        } else if protected {
            Some(NodeAttr::VISIBILITY_PROTECTED)
        } else if internal {
            Some(CSharpAttr::VISIBILITY_INTERNAL)
        } else if self.contains_any(Self::FILE) {
            Some(CSharpAttr::VISIBILITY_FILE)
        } else {
            None
        }
    }
}

/// Counters of the nodes the reducer could not map structurally, aggregated over every file
/// reduced through one adapter instance.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CSharpReductionStats {
    /// ERROR nodes eliminated (their children were reduced in place).
    pub error_nodes: u64,
    /// MISSING nodes dropped.
    pub missing_nodes: u64,
    /// Named nodes of a kind the grammar's `node-types.json` does not list (eliminated).
    pub unclassified_nodes: u64,
}

pub struct CSharpASTReductionAdapter {
    error_nodes: AtomicU64,
    missing_nodes: AtomicU64,
    unclassified_nodes: AtomicU64,
}

impl Default for CSharpASTReductionAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl CSharpASTReductionAdapter {
    pub fn new() -> Self {
        Self {
            error_nodes: AtomicU64::new(0),
            missing_nodes: AtomicU64::new(0),
            unclassified_nodes: AtomicU64::new(0),
        }
    }

    /// ERROR / MISSING / unclassified node counts accumulated so far.
    pub fn stats(&self) -> CSharpReductionStats {
        CSharpReductionStats {
            error_nodes: self.error_nodes.load(Ordering::Relaxed),
            missing_nodes: self.missing_nodes.load(Ordering::Relaxed),
            unclassified_nodes: self.unclassified_nodes.load(Ordering::Relaxed),
        }
    }

    /// The CST node holding the name a kept declaration introduces: the identifier of a type,
    /// member, parameter, declarator, designation or label, the alias of a using alias, the
    /// operator token of an operator, the target type of a conversion, the `this` keyword of
    /// an indexer and the keyword of an accessor. `None` for declarations whose names are
    /// carried by their declarators (field, event-field and local declaration statements),
    /// for using directives without an alias, and for nodes that declare nothing.
    pub fn name_node<'tree>(&self, kind: &str, node: &Node<'tree>) -> Option<Node<'tree>> {
        match kind {
            "namespace_declaration"
            | "file_scoped_namespace_declaration"
            | "class_declaration"
            | "struct_declaration"
            | "interface_declaration"
            | "enum_declaration"
            | "record_declaration"
            | "delegate_declaration"
            | "enum_member_declaration"
            | "type_parameter"
            | "method_declaration"
            | "local_function_statement"
            | "constructor_declaration"
            | "destructor_declaration"
            | "property_declaration"
            | "event_declaration"
            | "accessor_declaration"
            | "parameter"
            | "catch_declaration"
            | "variable_declarator"
            | "declaration_expression"
            | "extern_alias_directive"
            | "using_directive"
            | "attribute" => node.child_by_field_name("name"),
            "operator_declaration" => node.child_by_field_name("operator"),
            "conversion_operator_declaration" => node.child_by_field_name("type"),
            "indexer_declaration" => first_child_of_kind(node, "this", false),
            "type_parameter_constraints_clause" | "labeled_statement" => {
                first_child_of_kind(node, "identifier", true)
            }
            "identifier" | "implicit_parameter" | "discard" => Some(*node),
            _ => None,
        }
    }
}

impl ASTReductionAdapter for CSharpASTReductionAdapter {
    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_c_sharp::LANGUAGE.into()
    }

    fn classify(&self, kind: &str, node: &Node, _depth: usize) -> ReductionDecision {
        let (decision, tally) = decide(kind, node);
        let counter = match tally {
            Tally::None => return decision,
            Tally::Error => &self.error_nodes,
            Tally::Missing => &self.missing_nodes,
            Tally::Unclassified => &self.unclassified_nodes,
        };
        counter.fetch_add(1, Ordering::Relaxed);
        decision
    }

    fn encode_attrs(&self, kind: &str, node: &Node, source: &[u8]) -> u32 {
        match decide(kind, node).0 {
            ReductionDecision::Keep(node_type) => encode(node_type, kind, node, source),
            ReductionDecision::Eliminate | ReductionDecision::Drop => 0,
        }
    }
}

// ── Classification ───────────────────────────────────────────────────────────

/// Which counter a classification feeds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tally {
    None,
    Error,
    Missing,
    Unclassified,
}

fn decide(kind: &str, node: &Node<'_>) -> (ReductionDecision, Tally) {
    use ASTNodeType::*;
    use ReductionDecision::*;

    if node.is_missing() {
        return (Drop, Tally::Missing);
    }
    if kind == "ERROR" {
        return (Eliminate, Tally::Error);
    }

    let decision = match kind {
        // Compilation unit, namespaces, directives
        "compilation_unit" => Keep(NN_MODULE),
        "namespace_declaration" | "file_scoped_namespace_declaration" => Keep(NN_NAMESPACE_DECL),
        "using_directive" | "extern_alias_directive" => Keep(NN_USING_DIRECTIVE),
        "global_statement" | "global_attribute" => Eliminate,
        // Every branch of a conditional-compilation block is code; the conditions are not.
        "preproc_if" | "preproc_elif" | "preproc_else" => Eliminate,
        "preproc_region" | "preproc_endregion" | "preproc_define" | "preproc_undef"
        | "preproc_error" | "preproc_warning" | "preproc_line" | "preproc_pragma"
        | "preproc_nullable" | "preproc_arg" | "shebang_directive" | "comment" => Drop,

        // Type declarations
        "class_declaration" => Keep(NN_CLASS_DECL),
        "struct_declaration" => Keep(NN_STRUCT_DECL),
        "interface_declaration" => Keep(NN_INTERFACE_DECL),
        "enum_declaration" => Keep(NN_ENUM_DECL),
        "record_declaration" => Keep(NN_RECORD_DECL),
        "delegate_declaration" => Keep(NN_DELEGATE_DECL),
        "enum_member_declaration" => Keep(NN_ENUM_CONSTANT),
        "type_parameter" | "type_parameter_constraints_clause" => Keep(NN_TYPE_PARAM),
        "primary_constructor_base_type" | "constructor_initializer" => Keep(NN_CALL_EXPR),
        // `class B(int x) : A(x)`: the base-constructor arguments follow the base type.
        "argument_list" => match parent_kind(node) {
            "base_list" => Keep(NN_CALL_EXPR),
            _ => Eliminate,
        },

        // Members
        "method_declaration" | "accessor_declaration" => Keep(NN_METHOD_DECL),
        "local_function_statement" => Keep(NN_FUNCTION_DECL),
        "constructor_declaration" => Keep(NN_CONSTRUCTOR_DECL),
        "destructor_declaration" => Keep(NN_DESTRUCTOR_DECL),
        "operator_declaration" | "conversion_operator_declaration" => Keep(NN_OPERATOR_DECL),
        "property_declaration" => Keep(NN_PROPERTY_DECL),
        "indexer_declaration" => Keep(NN_INDEXER_DECL),
        "event_declaration" | "event_field_declaration" => Keep(NN_EVENT_DECL),
        "field_declaration" => Keep(NN_FIELD_DECL),
        "parameter" | "implicit_parameter" | "catch_declaration" => Keep(NN_PARAM_DECL),
        "local_declaration_statement" | "declaration_expression" | "variable_declarator" => {
            Keep(NN_LOCAL_VAR_DECL)
        }
        // Wrapped by a local, field or event declaration everywhere except in these three.
        "variable_declaration" => match parent_kind(node) {
            "for_statement" | "using_statement" | "fixed_statement" => Keep(NN_LOCAL_VAR_DECL),
            _ => Eliminate,
        },
        "attribute" => Keep(NN_ANNOTATION_USE),
        // `[A(Name = v)]` assigns a member of the attribute; `[A(name: v)]` names a parameter.
        "attribute_argument" => {
            if has_anonymous_child(node, "=") {
                Keep(NN_ASSIGN_EXPR)
            } else {
                Eliminate
            }
        }
        "arrow_expression_clause" => Keep(NN_RETURN_STMT),

        // Statements
        "block" => match parent_kind(node) {
            "unsafe_statement" | "checked_statement" => Eliminate,
            _ => Keep(NN_BLOCK),
        },
        "unsafe_statement" | "checked_statement" | "fixed_statement" => Keep(NN_BLOCK),
        "expression_statement" => Keep(NN_EXPR_STMT),
        "if_statement" => Keep(NN_IF_STMT),
        "for_statement" => Keep(NN_FOR_STMT),
        "foreach_statement" => Keep(NN_FOREACH_STMT),
        "while_statement" => Keep(NN_WHILE_STMT),
        "do_statement" => Keep(NN_DO_WHILE_STMT),
        "switch_statement" => Keep(NN_SWITCH_STMT),
        "switch_section" | "switch_expression_arm" => Keep(NN_SWITCH_CASE),
        "try_statement" => Keep(NN_TRY_STMT),
        "catch_clause" => Keep(NN_CATCH_CLAUSE),
        "finally_clause" => Keep(NN_FINALLY_CLAUSE),
        "return_statement" => Keep(NN_RETURN_STMT),
        "throw_statement" | "throw_expression" => Keep(NN_THROW_STMT),
        "break_statement" => Keep(NN_BREAK_STMT),
        "continue_statement" => Keep(NN_CONTINUE_STMT),
        "goto_statement" => Keep(NN_GOTO_STMT),
        "labeled_statement" => Keep(NN_LABELED_STMT),
        "lock_statement" => Keep(NN_LOCK_STMT),
        "using_statement" => Keep(NN_USING_STMT),
        "yield_statement" => Keep(NN_YIELD_STMT),

        // Expressions
        "assignment_expression" => Keep(NN_ASSIGN_EXPR),
        "binary_expression" => {
            if is_preproc_condition(node) {
                Drop
            } else {
                Keep(NN_BINARY_EXPR)
            }
        }
        "range_expression" => Keep(NN_BINARY_EXPR),
        "prefix_unary_expression" | "await_expression" | "checked_expression"
        | "sizeof_expression" | "ref_expression" | "makeref_expression"
        | "reftype_expression" | "refvalue_expression" => Keep(NN_UNARY_EXPR),
        // `!` of a `#if` / `#elif` condition.
        "unary_expression" => {
            if is_preproc_condition(node) {
                Drop
            } else {
                Keep(NN_UNARY_EXPR)
            }
        }
        // `e!` (null-forgiving) has no run-time effect.
        "postfix_unary_expression" => {
            if last_child_kind(node) == "!" {
                Eliminate
            } else {
                Keep(NN_UNARY_EXPR)
            }
        }
        "argument" => {
            if argument_pass_mode(node).is_some() {
                Keep(NN_UNARY_EXPR)
            } else {
                Eliminate
            }
        }
        "conditional_expression" => Keep(NN_TERNARY_EXPR),
        "invocation_expression" | "query_expression" | "from_clause" | "join_clause"
        | "join_into_clause" | "let_clause" | "where_clause" | "order_by_clause"
        | "select_clause" | "group_clause" => Keep(NN_CALL_EXPR),
        "object_creation_expression"
        | "implicit_object_creation_expression"
        | "anonymous_object_creation_expression"
        | "with_expression" => Keep(NN_NEW_EXPR),
        // The callee of `a.b(c)` is flattened into the call: `[a, b, c]`.
        "member_access_expression" => {
            if is_invocation_callee(node) {
                Eliminate
            } else {
                Keep(NN_FIELD_ACCESS)
            }
        }
        "conditional_access_expression" => {
            if has_named_child(node, "element_binding_expression") {
                Keep(NN_ARRAY_ACCESS)
            } else if is_invocation_callee(node) {
                Eliminate
            } else {
                Keep(NN_FIELD_ACCESS)
            }
        }
        "member_binding_expression" => match parent_kind(node) {
            "conditional_access_expression" => Eliminate,
            _ => Keep(NN_FIELD_ACCESS),
        },
        "element_binding_expression" => match parent_kind(node) {
            "conditional_access_expression" => Eliminate,
            _ => Keep(NN_ARRAY_ACCESS),
        },
        "element_access_expression" => Keep(NN_ARRAY_ACCESS),
        "cast_expression" | "as_expression" => Keep(NN_CAST_EXPR),
        "is_expression" | "is_pattern_expression" => Keep(NN_INSTANCEOF_EXPR),
        "lambda_expression" | "anonymous_method_expression" => Keep(NN_LAMBDA_EXPR),
        "array_creation_expression"
        | "implicit_array_creation_expression"
        | "stackalloc_expression"
        | "implicit_stackalloc_expression" => Keep(NN_ARRAY_CREATE),
        "initializer_expression" | "with_initializer" | "tuple_expression" => Keep(NN_ARRAY_INIT),
        "switch_expression" => Keep(NN_SWITCH_EXPR),
        // `this` / `base` have no wrapper node: the keyword itself is the expression.
        "this" => match parent_kind(node) {
            "indexer_declaration" | "constructor_initializer" => Drop,
            _ => Keep(NN_THIS_EXPR),
        },
        "base" => match parent_kind(node) {
            "constructor_initializer" => Drop,
            _ => Keep(NN_SUPER_EXPR),
        },

        // Patterns
        "constant_pattern" | "declaration_pattern" | "var_pattern" | "type_pattern"
        | "recursive_pattern" | "relational_pattern" | "negated_pattern" | "and_pattern"
        | "or_pattern" | "list_pattern" | "subpattern" | "when_clause" => Keep(NN_PATTERN_MATCH),
        ".." => match parent_kind(node) {
            "list_pattern" => Keep(NN_PATTERN_MATCH),
            _ => Drop,
        },
        "discard" => decide_discard(node),

        // Types and names
        "predefined_type" | "implicit_type" | "nullable_type" | "pointer_type" | "ref_type"
        | "scoped_type" | "function_pointer_type" | "tuple_type" => Keep(NN_TYPE_REF),
        // The outermost array type carries the rank count; in a creation expression the
        // element type and the sizes are children of the creation.
        "array_type" => match parent_kind(node) {
            "array_type" | "array_creation_expression" | "stackalloc_expression" => Eliminate,
            _ => Keep(NN_TYPE_REF),
        },
        "generic_name" => {
            if in_type_position(node) {
                Keep(NN_TYPE_REF)
            } else {
                Keep(NN_IDENTIFIER_EXPR)
            }
        }
        "qualified_name" | "alias_qualified_name" => match parent_kind(node) {
            "qualified_name" | "alias_qualified_name" => Eliminate,
            _ => {
                if in_type_position(node) {
                    Keep(NN_TYPE_REF)
                } else {
                    Keep(NN_IDENTIFIER_EXPR)
                }
            }
        },
        "identifier" => decide_identifier(node),

        // Literals
        "integer_literal" | "boolean_literal" | "character_literal" => {
            if is_preproc_condition(node) {
                Drop
            } else {
                Keep(NN_LITERAL)
            }
        }
        "real_literal" | "string_literal" | "verbatim_string_literal" | "raw_string_literal"
        | "null_literal" | "interpolated_string_expression" | "typeof_expression"
        | "default_expression" => Keep(NN_LITERAL),
        // `operator true` / `operator false`; elsewhere the keyword is inside a literal.
        "true" | "false" => match parent_kind(node) {
            "operator_declaration" => Keep(NN_LITERAL),
            _ => Drop,
        },
        "parenthesized_expression" => {
            if is_preproc_condition(node) {
                Drop
            } else {
                Eliminate
            }
        }

        // Wrappers: their children attach to the enclosing kept node
        "declaration_list"
        | "enum_member_declaration_list"
        | "accessor_list"
        | "base_list"
        | "type_parameter_list"
        | "type_parameter_constraint"
        | "type_argument_list"
        | "parameter_list"
        | "bracketed_parameter_list"
        | "bracketed_argument_list"
        | "attribute_list"
        | "attribute_argument_list"
        | "explicit_interface_specifier"
        | "parenthesized_pattern"
        | "parenthesized_variable_designation"
        | "positional_pattern_clause"
        | "property_pattern_clause"
        | "switch_body"
        | "catch_filter_clause"
        | "interpolation"
        | "tuple_pattern"
        | "tuple_element"
        | "array_rank_specifier"
        | "function_pointer_parameter" => Eliminate,

        // Leaves and clauses whose content is carried by the enclosing node's attributes
        "modifier"
        | "attribute_target_specifier"
        | "constructor_constraint"
        | "calling_convention"
        | "interpolation_alignment_clause"
        | "interpolation_format_clause"
        | "interpolation_brace"
        | "interpolation_start"
        | "interpolation_quote"
        | "string_content"
        | "string_literal_content"
        | "string_literal_encoding"
        | "character_literal_content"
        | "escape_sequence"
        | "raw_string_start"
        | "raw_string_content"
        | "raw_string_end"
        | "empty_statement" => Drop,

        // Supertypes are hidden in trees; listed so the grammar's kind set is closed.
        "declaration" | "expression" | "literal" | "lvalue_expression"
        | "non_lvalue_expression" | "pattern" | "statement" | "type" | "type_declaration" => {
            Eliminate
        }

        _ => {
            return if node.is_named() {
                (Eliminate, Tally::Unclassified)
            } else {
                (Drop, Tally::None)
            };
        }
    };
    (decision, Tally::None)
}

/// An `identifier` names a type, declares a variable, parameter or anonymous-type member,
/// or is a name in an expression, depending on where it stands.
fn decide_identifier(node: &Node<'_>) -> ReductionDecision {
    use ASTNodeType::*;
    use ReductionDecision::*;

    let parent = match node.parent() {
        Some(parent) => parent,
        None => return Keep(NN_IDENTIFIER_EXPR),
    };
    match parent.kind() {
        // Head of a generic name: the first child of a generic NN_TYPE_REF; in expression
        // position the generic NN_IDENTIFIER_EXPR itself carries the name token.
        "generic_name" => {
            if in_type_position(&parent) {
                Keep(NN_TYPE_REF)
            } else {
                Drop
            }
        }
        // Designations declare locals.
        "tuple_pattern" | "parenthesized_variable_designation" => Keep(NN_LOCAL_VAR_DECL),
        "declaration_pattern" | "var_pattern" | "recursive_pattern" | "declaration_expression"
            if is_field(&parent, "name", node) =>
        {
            Keep(NN_LOCAL_VAR_DECL)
        }
        "foreach_statement"
            if is_field(&parent, "left", node) && parent.child_by_field_name("type").is_some() =>
        {
            Keep(NN_LOCAL_VAR_DECL)
        }
        // Name of a `params` array: the grammar keeps the parameter-array rule hidden.
        "parameter_list" | "bracketed_parameter_list" => Keep(NN_PARAM_DECL),
        // Query range variables are the parameters of the lambdas a query lowers to.
        "from_clause" if is_field(&parent, "name", node) => Keep(NN_PARAM_DECL),
        "join_clause" if next_sibling_kind(node) == "in" => Keep(NN_PARAM_DECL),
        "let_clause" if next_sibling_kind(node) == "=" => Keep(NN_PARAM_DECL),
        "join_into_clause" => Keep(NN_PARAM_DECL),
        "query_expression" if prev_sibling_kind(node) == "into" => Keep(NN_PARAM_DECL),
        // `new { Name = v }` declares a property of the anonymous type.
        "anonymous_object_creation_expression" if next_sibling_kind(node) == "=" => {
            Keep(NN_PROPERTY_DECL)
        }
        // Names without run-time meaning: tuple element names, argument names
        // (`f(x: 1)`, `[A(x: 1)]`), unmanaged calling conventions.
        "tuple_element" if is_field(&parent, "name", node) => Drop,
        "argument" if is_field(&parent, "name", node) => Drop,
        "attribute_argument" if next_sibling_kind(node) == ":" => Drop,
        "calling_convention" => Drop,
        "preproc_if" | "preproc_elif" if is_field(&parent, "condition", node) => Drop,
        _ => {
            if in_type_position_under(&parent, node) {
                Keep(NN_TYPE_REF)
            } else {
                Keep(NN_IDENTIFIER_EXPR)
            }
        }
    }
}

/// `_` discards a value in a designation, is a pattern in a pattern position, and is a
/// plain name elsewhere (`_ = f();`).
fn decide_discard(node: &Node<'_>) -> ReductionDecision {
    use ASTNodeType::*;
    use ReductionDecision::*;

    match parent_kind(node) {
        "tuple_pattern" | "parenthesized_variable_designation" | "declaration_pattern"
        | "var_pattern" | "recursive_pattern" => Keep(NN_LOCAL_VAR_DECL),
        "switch_expression_arm" | "switch_section" | "subpattern" | "list_pattern"
        | "is_pattern_expression" | "negated_pattern" | "and_pattern" | "or_pattern"
        | "parenthesized_pattern" => Keep(NN_PATTERN_MATCH),
        _ => Keep(NN_IDENTIFIER_EXPR),
    }
}

/// True when `node` stands where a type is expected.
fn in_type_position(node: &Node<'_>) -> bool {
    match node.parent() {
        Some(parent) => in_type_position_under(&parent, node),
        None => false,
    }
}

fn in_type_position_under<'tree>(parent: &Node<'tree>, node: &Node<'tree>) -> bool {
    match parent.kind() {
        "type_argument_list" | "base_list" | "explicit_interface_specifier" => true,
        "is_expression" | "as_expression" => is_field(parent, "right", node),
        _ => is_field(parent, "type", node) || is_field(parent, "returns", node),
    }
}

/// The condition of a `#if` / `#elif` directive.
fn is_preproc_condition(node: &Node<'_>) -> bool {
    match node.parent() {
        Some(parent) if matches!(parent.kind(), "preproc_if" | "preproc_elif") => {
            is_field(&parent, "condition", node)
        }
        _ => false,
    }
}

fn is_invocation_callee(node: &Node<'_>) -> bool {
    match node.parent() {
        Some(parent) if parent.kind() == "invocation_expression" => {
            is_field(&parent, "function", node)
        }
        _ => false,
    }
}

/// The `UNARY_*` payload of a `ref` / `out` / `in` argument.
fn argument_pass_mode(node: &Node<'_>) -> Option<u8> {
    let mut cursor = node.walk();
    let mode = node
        .children(&mut cursor)
        .filter(|c| !c.is_named())
        .find_map(|c| match c.kind() {
            "ref" => Some(CSharpAttr::UNARY_REF),
            "out" => Some(CSharpAttr::UNARY_OUT),
            "in" => Some(CSharpAttr::UNARY_IN),
            _ => None,
        });
    mode
}

// ── Attribute encoding ───────────────────────────────────────────────────────

fn encode(node_type: ASTNodeType, kind: &str, node: &Node<'_>, source: &[u8]) -> u32 {
    use ASTNodeType::*;

    match node_type {
        NN_CLASS_DECL | NN_STRUCT_DECL | NN_INTERFACE_DECL | NN_RECORD_DECL | NN_ENUM_DECL
        | NN_DELEGATE_DECL => encode_type_declaration(kind, node, source),
        NN_NAMESPACE_DECL => aux_only(if kind == "file_scoped_namespace_declaration" {
            CSharpAttr::AUX_NAMESPACE_FILE_SCOPED
        } else {
            0
        }),
        NN_USING_DIRECTIVE => encode_using_directive(kind, node),
        NN_METHOD_DECL => {
            if kind == "accessor_declaration" {
                encode_accessor(node, source)
            } else {
                encode_method(node, source)
            }
        }
        NN_FUNCTION_DECL => encode_local_function(node, source),
        NN_CONSTRUCTOR_DECL => encode_constructor(node, source),
        NN_DESTRUCTOR_DECL => encode_destructor(node, source),
        NN_OPERATOR_DECL => encode_operator(kind, node, source),
        NN_PROPERTY_DECL if kind == "identifier" => NodeAttr::pack(
            NodeAttr::VISIBILITY_PUBLIC,
            0,
            OperatorId::None,
            0,
            NodeAttr::LANG_IMPLICIT_VISIBILITY,
        ),
        NN_PROPERTY_DECL | NN_INDEXER_DECL => encode_property(kind, node, source),
        NN_EVENT_DECL => encode_event(kind, node, source),
        NN_FIELD_DECL => encode_field(node, source),
        NN_ENUM_CONSTANT => encode_enum_member(node, source),
        NN_PARAM_DECL => encode_parameter(kind, node, source),
        NN_LOCAL_VAR_DECL => encode_local(kind, node, source),
        NN_TYPE_PARAM => encode_type_parameter(kind, node, source),
        NN_BLOCK => encode_block(kind, node),
        NN_RETURN_STMT if kind == "arrow_expression_clause" => {
            let mut aux = CSharpAttr::AUX_STMT_EXPRESSION_BODY;
            if expression_body_discards_value(node, source) {
                aux |= CSharpAttr::AUX_STMT_DISCARDS_VALUE;
            }
            aux_only(aux)
        }
        NN_THROW_STMT if kind == "throw_expression" => aux_only(CSharpAttr::AUX_THROW_EXPRESSION),
        NN_SWITCH_EXPR => aux_only(NodeAttr::AUX_SWITCH_ARROW_FORM),
        NN_SWITCH_CASE => encode_switch_case(kind, node),
        NN_PATTERN_MATCH => encode_pattern(kind, node),
        NN_CATCH_CLAUSE => encode_catch(node),
        NN_FOREACH_STMT => encode_foreach(node),
        NN_USING_STMT => encode_using_statement(node),
        NN_GOTO_STMT => encode_goto(node),
        NN_YIELD_STMT => aux_only(if has_anonymous_child(node, "break") {
            CSharpAttr::AUX_YIELD_BREAK
        } else {
            0
        }),
        NN_CALL_EXPR => encode_call(kind, node),
        NN_FIELD_ACCESS | NN_ARRAY_ACCESS => encode_access(kind, node),
        NN_BINARY_EXPR => encode_binary(kind, node),
        NN_ASSIGN_EXPR => encode_assignment(kind, node),
        NN_UNARY_EXPR => encode_unary(kind, node),
        NN_CAST_EXPR if kind == "as_expression" => aux_only(CSharpAttr::AUX_CAST_AS),
        NN_INSTANCEOF_EXPR if kind == "is_pattern_expression" => {
            aux_only(NodeAttr::AUX_INSTANCEOF_PATTERN)
        }
        NN_LAMBDA_EXPR => encode_lambda(kind, node, source),
        NN_NEW_EXPR => payload_only(match kind {
            "implicit_object_creation_expression" => CSharpAttr::NEW_TARGET_TYPED,
            "anonymous_object_creation_expression" => CSharpAttr::NEW_ANONYMOUS_TYPE,
            "with_expression" => CSharpAttr::NEW_WITH,
            _ => CSharpAttr::NEW_OBJECT,
        }),
        NN_ARRAY_CREATE => aux_only(match kind {
            "stackalloc_expression" => CSharpAttr::AUX_ARRAY_STACKALLOC,
            "implicit_array_creation_expression" => CSharpAttr::AUX_ARRAY_IMPLICIT,
            "implicit_stackalloc_expression" => {
                CSharpAttr::AUX_ARRAY_STACKALLOC | CSharpAttr::AUX_ARRAY_IMPLICIT
            }
            _ => 0,
        }),
        NN_ARRAY_INIT => payload_only(initializer_kind(kind, node)),
        NN_LITERAL => encode_literal(kind, node, source),
        NN_TYPE_REF => encode_type_ref(kind, node, source),
        NN_IDENTIFIER_EXPR => encode_identifier(kind, node, source),
        NN_ANNOTATION_USE => payload_only(attribute_target(node, source)),
        _ => 0,
    }
}

#[inline]
fn aux_only(aux: u8) -> u32 {
    NodeAttr::pack(NodeAttr::VISIBILITY_NONE, 0, OperatorId::None, aux, 0)
}

#[inline]
fn payload_only(lang: u8) -> u32 {
    NodeAttr::pack(NodeAttr::VISIBILITY_NONE, 0, OperatorId::None, 0, lang)
}

/// Attribute facts of a declaration's `[...]` lists.
#[derive(Debug, Clone, Copy, Default)]
struct AttributeFacts {
    any: bool,
    obsolete: bool,
    non_serialized: bool,
    synchronized: bool,
}

impl AttributeFacts {
    fn of(node: &Node<'_>, source: &[u8]) -> Self {
        let mut facts = Self::default();
        let mut cursor = node.walk();
        for list in node.named_children(&mut cursor) {
            if list.kind() != "attribute_list" {
                continue;
            }
            facts.any = true;
            // `[return: ...]` and `[param: ...]` lists describe the return value or a
            // parameter, not the declaration itself.
            let target = attribute_list_target(&list, source);
            if matches!(target, CSharpAttr::TARGET_RETURN | CSharpAttr::TARGET_PARAM) {
                continue;
            }
            let mut inner = list.walk();
            for attribute in list.named_children(&mut inner) {
                if attribute.kind() != "attribute" {
                    continue;
                }
                let name = attribute
                    .child_by_field_name("name")
                    .map_or("", |n| node_text(&n, source));
                match attribute_simple_name(name) {
                    "Obsolete" => facts.obsolete = true,
                    "NonSerialized" => facts.non_serialized = true,
                    "MethodImpl" => {
                        let mut args = attribute.walk();
                        let synchronized = attribute.named_children(&mut args).any(|a| {
                            a.kind() == "attribute_argument_list"
                                && node_text(&a, source).contains("Synchronized")
                        });
                        facts.synchronized |= synchronized;
                    }
                    _ => {}
                }
            }
        }
        facts
    }
}

/// `global::System.ObsoleteAttribute` → `Obsolete`; `Foo<int>` → `Foo`.
fn attribute_simple_name(name: &str) -> &str {
    let head = name.split('<').next().unwrap_or(name);
    let last = head
        .rsplit(|c: char| c == '.' || c == ':')
        .next()
        .unwrap_or(head)
        .trim();
    match last.strip_suffix("Attribute") {
        Some(stem) if !stem.is_empty() => stem,
        _ => last,
    }
}

/// The NodeAttr fields of one declaration under construction.
struct Decl {
    vis: u8,
    mods: u8,
    aux: u8,
    lang: u8,
    written: CSharpModifiers,
}

impl Decl {
    /// Written modifiers and attributes of `node`, with `default_visibility` (when not
    /// `VISIBILITY_NONE`) applied as implied visibility if no access modifier is written.
    fn new(node: &Node<'_>, source: &[u8], default_visibility: u8) -> Self {
        Self::with_modifiers(
            node,
            source,
            CSharpModifiers::of(node, source),
            default_visibility,
        )
    }

    fn with_modifiers(
        node: &Node<'_>,
        source: &[u8],
        written: CSharpModifiers,
        default_visibility: u8,
    ) -> Self {
        use CSharpModifiers as M;

        let mut decl = Decl {
            vis: NodeAttr::VISIBILITY_NONE,
            mods: 0,
            aux: 0,
            lang: 0,
            written,
        };
        match written.visibility() {
            Some(vis) => decl.vis = vis,
            None if default_visibility != NodeAttr::VISIBILITY_NONE => {
                decl.vis = default_visibility;
                decl.lang |= NodeAttr::LANG_IMPLICIT_VISIBILITY;
            }
            None => {}
        }
        if written.contains_any(M::STATIC) {
            decl.mods |= NodeAttr::MOD_STATIC;
        }
        if written.contains_any(M::ABSTRACT) {
            decl.mods |= NodeAttr::MOD_ABSTRACT;
        }
        if written.contains_any(M::SEALED) {
            decl.mods |= NodeAttr::MOD_FINAL;
        }
        if written.contains_any(M::EXTERN) {
            decl.mods |= NodeAttr::MOD_NATIVE;
        }
        if written.contains_any(M::VOLATILE) {
            decl.mods |= NodeAttr::MOD_VOLATILE;
        }
        if written.contains_any(M::PARTIAL) {
            decl.lang |= CSharpAttr::LANG_PARTIAL;
        }
        if written.contains_any(M::VIRTUAL) {
            decl.lang |= CSharpAttr::LANG_VIRTUAL;
        }
        if written.contains_any(M::OVERRIDE) {
            decl.lang |= CSharpAttr::LANG_OVERRIDE;
        }

        let attributes = AttributeFacts::of(node, source);
        if attributes.any {
            decl.lang |= NodeAttr::LANG_HAS_ANNOTATIONS;
        }
        if attributes.obsolete {
            decl.lang |= NodeAttr::LANG_DEPRECATED;
        }
        if attributes.non_serialized {
            decl.mods |= NodeAttr::MOD_TRANSIENT;
        }
        if attributes.synchronized {
            decl.mods |= NodeAttr::MOD_SYNCHRONIZED;
        }
        decl
    }

    /// `readonly` on a field, or on a struct method, property, indexer, event or accessor.
    fn readonly_is_final(&mut self) {
        if self.written.contains_any(CSharpModifiers::READONLY) {
            self.mods |= NodeAttr::MOD_FINAL;
        }
    }

    /// Adds modifier bits the language implies.
    fn imply(&mut self, mods: u8) {
        if self.mods & mods != mods {
            self.mods |= mods;
            self.lang |= NodeAttr::LANG_IMPLICIT_MODIFIERS;
        }
    }

    /// An interface member without `static`, `sealed`, `private` or `extern` is abstract
    /// without a body and virtual with one (default interface member).
    fn imply_interface_dispatch(&mut self, has_body: bool) {
        use CSharpModifiers as M;
        if self.written.contains_any(
            M::STATIC | M::SEALED | M::PRIVATE | M::EXTERN | M::ABSTRACT | M::VIRTUAL | M::OVERRIDE,
        ) {
            return;
        }
        if has_body {
            self.lang |= CSharpAttr::LANG_VIRTUAL | NodeAttr::LANG_IMPLICIT_MODIFIERS;
        } else {
            self.imply(NodeAttr::MOD_ABSTRACT);
        }
    }

    fn pack(&self, op: OperatorId) -> u32 {
        NodeAttr::pack(self.vis, self.mods, op, self.aux, self.lang)
    }
}

/// Kind of the declaration that syntactically contains a member or nested type, skipping
/// the list and conditional-compilation nodes between them.
fn container_kind(node: &Node<'_>) -> &'static str {
    let mut current = node.parent();
    while let Some(parent) = current {
        match parent.kind() {
            "declaration_list" | "enum_member_declaration_list" | "accessor_list"
            | "preproc_if" | "preproc_elif" | "preproc_else" | "ERROR" => {
                current = parent.parent();
            }
            kind => return kind,
        }
    }
    ""
}

/// Implied accessibility of a type declared in `container`.
fn type_default_visibility(container: &str) -> u8 {
    match container {
        "compilation_unit" | "namespace_declaration" | "file_scoped_namespace_declaration" => {
            CSharpAttr::VISIBILITY_INTERNAL
        }
        "interface_declaration" => NodeAttr::VISIBILITY_PUBLIC,
        "class_declaration" | "struct_declaration" | "record_declaration" => {
            NodeAttr::VISIBILITY_PRIVATE
        }
        _ => NodeAttr::VISIBILITY_NONE,
    }
}

/// Implied accessibility of a member declared in `container`.
fn member_default_visibility(container: &str) -> u8 {
    match container {
        "interface_declaration" | "enum_declaration" => NodeAttr::VISIBILITY_PUBLIC,
        "class_declaration" | "struct_declaration" | "record_declaration" => {
            NodeAttr::VISIBILITY_PRIVATE
        }
        _ => NodeAttr::VISIBILITY_NONE,
    }
}

fn encode_type_declaration(kind: &str, node: &Node<'_>, source: &[u8]) -> u32 {
    let mut decl = Decl::new(node, source, type_default_visibility(container_kind(node)));
    let record_struct = kind == "record_declaration" && has_anonymous_child(node, "struct");
    match kind {
        "struct_declaration" | "enum_declaration" | "delegate_declaration" => {
            decl.imply(NodeAttr::MOD_FINAL)
        }
        "interface_declaration" => decl.imply(NodeAttr::MOD_ABSTRACT),
        _ if record_struct => decl.imply(NodeAttr::MOD_FINAL),
        _ => {}
    }
    if record_struct {
        decl.aux |= CSharpAttr::AUX_DECL_RECORD_STRUCT;
    }
    if decl.written.contains_any(CSharpModifiers::READONLY) {
        decl.aux |= CSharpAttr::AUX_DECL_READONLY;
    }
    if decl.written.contains_any(CSharpModifiers::REF) {
        decl.aux |= CSharpAttr::AUX_DECL_REF_STRUCT;
    }
    if kind == "delegate_declaration" {
        if has_params_array(node.child_by_field_name("parameters")) {
            decl.lang |= NodeAttr::LANG_JAVA_VARARGS;
        }
    } else if has_named_child(node, "parameter_list") {
        decl.aux |= CSharpAttr::AUX_DECL_PRIMARY_CTOR;
    }
    decl.pack(OperatorId::None)
}

fn encode_using_directive(kind: &str, node: &Node<'_>) -> u32 {
    if kind == "extern_alias_directive" {
        return aux_only(CSharpAttr::AUX_USING_EXTERN_ALIAS);
    }
    let mut aux = 0u8;
    if has_anonymous_child(node, "global") {
        aux |= CSharpAttr::AUX_USING_GLOBAL;
    }
    if has_anonymous_child(node, "static") {
        aux |= CSharpAttr::AUX_USING_STATIC;
    }
    if node.child_by_field_name("name").is_some() {
        aux |= CSharpAttr::AUX_USING_ALIAS;
    }
    aux_only(aux)
}

fn encode_method(node: &Node<'_>, source: &[u8]) -> u32 {
    let container = container_kind(node);
    let explicit = has_named_child(node, "explicit_interface_specifier");
    // An explicit interface implementation is reachable only through the interface.
    let default_visibility = if explicit {
        NodeAttr::VISIBILITY_PRIVATE
    } else {
        member_default_visibility(container)
    };
    let mut decl = Decl::new(node, source, default_visibility);
    decl.readonly_is_final();
    let has_body = node.child_by_field_name("body").is_some();
    if has_body {
        decl.aux |= NodeAttr::AUX_METHOD_HAS_BODY;
    }
    if decl.written.contains_any(CSharpModifiers::ASYNC) {
        decl.aux |= CSharpAttr::AUX_METHOD_ASYNC;
    }
    if explicit {
        decl.aux |= CSharpAttr::AUX_EXPLICIT_IMPL;
    } else if container == "interface_declaration" {
        decl.imply_interface_dispatch(has_body);
    }
    if has_params_array(node.child_by_field_name("parameters")) {
        decl.lang |= NodeAttr::LANG_JAVA_VARARGS;
    }
    decl.pack(OperatorId::None)
}

fn encode_local_function(node: &Node<'_>, source: &[u8]) -> u32 {
    let mut decl = Decl::new(node, source, NodeAttr::VISIBILITY_NONE);
    if node.child_by_field_name("body").is_some() {
        decl.aux |= NodeAttr::AUX_METHOD_HAS_BODY;
    }
    if decl.written.contains_any(CSharpModifiers::ASYNC) {
        decl.aux |= CSharpAttr::AUX_METHOD_ASYNC;
    }
    if has_params_array(node.child_by_field_name("parameters")) {
        decl.lang |= NodeAttr::LANG_JAVA_VARARGS;
    }
    decl.pack(OperatorId::None)
}

fn encode_constructor(node: &Node<'_>, source: &[u8]) -> u32 {
    let written = CSharpModifiers::of(node, source);
    // A static constructor takes no access modifier and is invoked by the runtime only.
    let default_visibility = if written.contains_any(CSharpModifiers::STATIC) {
        NodeAttr::VISIBILITY_PRIVATE
    } else {
        member_default_visibility(container_kind(node))
    };
    let mut decl = Decl::with_modifiers(node, source, written, default_visibility);
    if node.child_by_field_name("body").is_some() {
        decl.aux |= NodeAttr::AUX_METHOD_HAS_BODY;
    }
    if has_params_array(node.child_by_field_name("parameters")) {
        decl.lang |= NodeAttr::LANG_JAVA_VARARGS;
    }
    decl.pack(OperatorId::None)
}

fn encode_destructor(node: &Node<'_>, source: &[u8]) -> u32 {
    // A finalizer overrides `Object.Finalize`, which is protected.
    let mut decl = Decl::new(node, source, NodeAttr::VISIBILITY_PROTECTED);
    if node.child_by_field_name("body").is_some() {
        decl.aux |= NodeAttr::AUX_METHOD_HAS_BODY;
    }
    decl.pack(OperatorId::None)
}

fn encode_operator(kind: &str, node: &Node<'_>, source: &[u8]) -> u32 {
    let mut decl = Decl::new(node, source, member_default_visibility(container_kind(node)));
    if node.child_by_field_name("body").is_some() {
        decl.aux |= NodeAttr::AUX_METHOD_HAS_BODY;
    }
    if has_named_child(node, "explicit_interface_specifier") {
        decl.aux |= CSharpAttr::AUX_EXPLICIT_IMPL;
    }
    let op = if kind == "conversion_operator_declaration" {
        decl.aux |= CSharpAttr::AUX_OPERATOR_CONVERSION;
        if has_anonymous_child(node, "implicit") {
            decl.aux |= CSharpAttr::AUX_OPERATOR_IMPLICIT_OR_CHECKED;
        }
        OperatorId::None
    } else {
        if has_anonymous_child(node, "checked") {
            decl.aux |= CSharpAttr::AUX_OPERATOR_IMPLICIT_OR_CHECKED;
        }
        overloaded_operator(node)
    };
    decl.pack(op)
}

/// The operator an `operator` declaration overloads. `+` and `-` are unary with one
/// parameter; `true` / `false` have no operator id.
fn overloaded_operator(node: &Node<'_>) -> OperatorId {
    let symbol = match node.child_by_field_name("operator") {
        Some(op) => op.kind(),
        None => return OperatorId::None,
    };
    let arity = match symbol {
        "!" | "~" | "++" | "--" => OperatorArity::Unary,
        "+" | "-" if parameter_count(node) == 1 => OperatorArity::Unary,
        _ => OperatorArity::Binary,
    };
    OperatorId::from_symbol(symbol, arity).unwrap_or(OperatorId::None)
}

fn parameter_count(node: &Node<'_>) -> usize {
    match node.child_by_field_name("parameters") {
        Some(list) => {
            let mut cursor = list.walk();
            let count = list
                .named_children(&mut cursor)
                .filter(|c| c.kind() == "parameter")
                .count();
            count
        }
        None => 0,
    }
}

fn encode_property(kind: &str, node: &Node<'_>, source: &[u8]) -> u32 {
    use CSharpModifiers as M;

    let container = container_kind(node);
    let explicit = has_named_child(node, "explicit_interface_specifier");
    let default_visibility = if explicit {
        NodeAttr::VISIBILITY_PRIVATE
    } else {
        member_default_visibility(container)
    };
    let mut decl = Decl::new(node, source, default_visibility);
    decl.readonly_is_final();

    let expression_bodied = node
        .child_by_field_name("value")
        .is_some_and(|v| v.kind() == "arrow_expression_clause");
    let accessors = node.child_by_field_name("accessors");
    let accessor_body = accessors.is_some_and(|list| any_accessor_has_body(&list));
    let has_body = expression_bodied || accessor_body;

    if kind == "property_declaration"
        && accessors.is_some()
        && !has_body
        && container != "interface_declaration"
        && !decl.written.contains_any(M::ABSTRACT | M::EXTERN | M::PARTIAL)
    {
        decl.aux |= CSharpAttr::AUX_PROPERTY_AUTO;
    }
    if expression_bodied {
        decl.aux |= CSharpAttr::AUX_PROPERTY_EXPRESSION_BODIED;
    }
    if decl.written.contains_any(M::REQUIRED) {
        decl.aux |= CSharpAttr::AUX_PROPERTY_REQUIRED;
    }
    if explicit {
        decl.aux |= CSharpAttr::AUX_EXPLICIT_IMPL;
    } else if container == "interface_declaration" {
        decl.imply_interface_dispatch(has_body);
    }
    if kind == "indexer_declaration" && has_params_array(node.child_by_field_name("parameters")) {
        decl.lang |= NodeAttr::LANG_JAVA_VARARGS;
    }
    decl.pack(OperatorId::None)
}

fn encode_event(kind: &str, node: &Node<'_>, source: &[u8]) -> u32 {
    let container = container_kind(node);
    let explicit = has_named_child(node, "explicit_interface_specifier");
    let default_visibility = if explicit {
        NodeAttr::VISIBILITY_PRIVATE
    } else {
        member_default_visibility(container)
    };
    let mut decl = Decl::new(node, source, default_visibility);
    decl.readonly_is_final();
    let has_body = node
        .child_by_field_name("accessors")
        .is_some_and(|list| any_accessor_has_body(&list));
    if kind == "event_field_declaration" {
        decl.aux |= CSharpAttr::AUX_EVENT_FIELD_LIKE;
    }
    if explicit {
        decl.aux |= CSharpAttr::AUX_EXPLICIT_IMPL;
    } else if container == "interface_declaration" {
        decl.imply_interface_dispatch(has_body);
    }
    decl.pack(OperatorId::None)
}

fn any_accessor_has_body(list: &Node<'_>) -> bool {
    let mut cursor = list.walk();
    let found = list
        .named_children(&mut cursor)
        .any(|a| a.kind() == "accessor_declaration" && a.child_by_field_name("body").is_some());
    found
}

fn encode_accessor(node: &Node<'_>, source: &[u8]) -> u32 {
    let owner = node.parent().and_then(|list| list.parent());
    let owner_attr = match owner {
        Some(o) if matches!(o.kind(), "property_declaration" | "indexer_declaration") => {
            encode_property(o.kind(), &o, source)
        }
        Some(o) if o.kind() == "event_declaration" => encode_event(o.kind(), &o, source),
        _ => 0,
    };
    let mut decl = Decl::new(node, source, NodeAttr::unpack_visibility(owner_attr));
    decl.readonly_is_final();
    // An accessor is static, abstract or extern with its property.
    decl.imply(
        NodeAttr::unpack_modifiers(owner_attr)
            & (NodeAttr::MOD_STATIC | NodeAttr::MOD_ABSTRACT | NodeAttr::MOD_NATIVE),
    );
    if node.child_by_field_name("body").is_some() {
        decl.aux |= NodeAttr::AUX_METHOD_HAS_BODY;
    }
    let accessor = match node.child_by_field_name("name").map(|n| n.kind()) {
        Some("get") => CSharpAttr::ACCESSOR_GET,
        Some("set") => CSharpAttr::ACCESSOR_SET,
        Some("init") => CSharpAttr::ACCESSOR_INIT,
        Some("add") => CSharpAttr::ACCESSOR_ADD,
        Some("remove") => CSharpAttr::ACCESSOR_REMOVE,
        _ => CSharpAttr::ACCESSOR_OTHER,
    };
    decl.lang = (decl.lang & !CSharpAttr::LANG_ACCESSOR_MASK) | accessor;
    decl.pack(OperatorId::None)
}

fn encode_field(node: &Node<'_>, source: &[u8]) -> u32 {
    use CSharpModifiers as M;

    let mut decl = Decl::new(node, source, member_default_visibility(container_kind(node)));
    decl.readonly_is_final();
    if decl.written.contains_any(M::CONST) {
        decl.mods |= NodeAttr::MOD_FINAL;
        decl.imply(NodeAttr::MOD_STATIC);
        decl.aux |= NodeAttr::AUX_FIELD_CONSTANT;
    }
    if decl.written.contains_any(M::FIXED) {
        decl.aux |= CSharpAttr::AUX_FIELD_FIXED_BUFFER;
    }
    if decl.written.contains_any(M::REQUIRED) {
        decl.aux |= CSharpAttr::AUX_FIELD_REQUIRED;
    }
    let declared_type = first_child_of_kind(node, "variable_declaration", true)
        .and_then(|d| d.child_by_field_name("type"));
    if declared_type.is_some_and(|t| t.kind() == "ref_type") {
        decl.aux |= CSharpAttr::AUX_FIELD_REF;
    }
    decl.pack(OperatorId::None)
}

fn encode_enum_member(node: &Node<'_>, source: &[u8]) -> u32 {
    let mut decl = Decl::new(node, source, NodeAttr::VISIBILITY_PUBLIC);
    decl.imply(NodeAttr::MOD_STATIC | NodeAttr::MOD_FINAL);
    decl.pack(OperatorId::None)
}

fn encode_parameter(kind: &str, node: &Node<'_>, source: &[u8]) -> u32 {
    use CSharpModifiers as M;

    match kind {
        "parameter" => {
            let mut decl = Decl::new(node, source, NodeAttr::VISIBILITY_NONE);
            let w = decl.written;
            let pass = if w.contains_any(M::OUT) {
                CSharpAttr::PASS_OUT
            } else if w.contains_any(M::REF) && w.contains_any(M::READONLY) {
                CSharpAttr::PASS_REF_READONLY
            } else if w.contains_any(M::REF) {
                CSharpAttr::PASS_REF
            } else if w.contains_any(M::IN) {
                CSharpAttr::PASS_IN
            } else {
                CSharpAttr::PASS_VALUE
            };
            decl.lang = (decl.lang & !CSharpAttr::LANG_PASS_MASK) | pass;
            if w.contains_any(M::PARAMS) {
                decl.lang |= NodeAttr::LANG_JAVA_VARARGS;
            }
            if w.contains_any(M::THIS) {
                decl.aux |= CSharpAttr::AUX_PARAM_THIS;
            }
            match node.parent().and_then(|list| list.parent()).map(|o| o.kind()) {
                Some("record_declaration") => decl.aux |= NodeAttr::AUX_PARAM_RECORD_COMPONENT,
                Some("lambda_expression") if node.child_by_field_name("type").is_none() => {
                    decl.aux |= NodeAttr::AUX_PARAM_LAMBDA_INFERRED
                }
                _ => {}
            }
            decl.pack(OperatorId::None)
        }
        "catch_declaration" => aux_only(NodeAttr::AUX_PARAM_CATCH),
        // The name of a `params` array.
        "identifier"
            if matches!(
                parent_kind(node),
                "parameter_list" | "bracketed_parameter_list"
            ) =>
        {
            payload_only(NodeAttr::LANG_JAVA_VARARGS)
        }
        // `x => ...` and query range variables.
        _ => aux_only(NodeAttr::AUX_PARAM_LAMBDA_INFERRED),
    }
}

fn encode_local(kind: &str, node: &Node<'_>, source: &[u8]) -> u32 {
    match kind {
        "local_declaration_statement" => {
            let written = CSharpModifiers::of(node, source);
            let mut mods = 0u8;
            let mut lang = 0u8;
            let mut aux = 0u8;
            if written.contains_any(CSharpModifiers::CONST) {
                mods |= NodeAttr::MOD_FINAL;
                lang |= CSharpAttr::LANG_LOCAL_CONST;
            }
            let declared_type = first_child_of_kind(node, "variable_declaration", true)
                .and_then(|d| d.child_by_field_name("type"));
            if let Some(t) = declared_type {
                match t.kind() {
                    "ref_type" => lang |= CSharpAttr::LANG_LOCAL_REF,
                    "scoped_type" => {
                        lang |= CSharpAttr::LANG_LOCAL_SCOPED;
                        if t
                            .child_by_field_name("type")
                            .is_some_and(|inner| inner.kind() == "ref_type")
                        {
                            lang |= CSharpAttr::LANG_LOCAL_REF;
                        }
                    }
                    _ => {}
                }
            }
            if has_anonymous_child(node, "using") {
                aux |= CSharpAttr::AUX_LOCAL_USING;
            }
            if has_anonymous_child(node, "await") {
                aux |= CSharpAttr::AUX_LOCAL_AWAIT;
            }
            NodeAttr::pack(NodeAttr::VISIBILITY_NONE, mods, OperatorId::None, aux, lang)
        }
        "variable_declarator" if has_named_child(node, "tuple_pattern") => {
            aux_only(CSharpAttr::AUX_LOCAL_DECONSTRUCTION)
        }
        "discard" => aux_only(CSharpAttr::AUX_LOCAL_DISCARD),
        _ => 0,
    }
}

fn encode_type_parameter(kind: &str, node: &Node<'_>, source: &[u8]) -> u32 {
    if kind == "type_parameter_constraints_clause" {
        return NodeAttr::pack(
            NodeAttr::VISIBILITY_NONE,
            0,
            OperatorId::None,
            CSharpAttr::AUX_TYPE_PARAM_CONSTRAINTS,
            constraint_flags(node, source),
        );
    }
    let mut aux = 0u8;
    if has_anonymous_child(node, "in") {
        aux |= CSharpAttr::AUX_TYPE_PARAM_IN;
    }
    if has_anonymous_child(node, "out") {
        aux |= CSharpAttr::AUX_TYPE_PARAM_OUT;
    }
    let lang = if has_named_child(node, "attribute_list") {
        NodeAttr::LANG_HAS_ANNOTATIONS
    } else {
        0
    };
    NodeAttr::pack(NodeAttr::VISIBILITY_NONE, 0, OperatorId::None, aux, lang)
}

/// `CONSTRAINT_*` bits of a `where T : ...` clause.
fn constraint_flags(clause: &Node<'_>, source: &[u8]) -> u8 {
    let mut flags = 0u8;
    let mut cursor = clause.walk();
    for constraint in clause.named_children(&mut cursor) {
        if constraint.kind() != "type_parameter_constraint" {
            continue;
        }
        let mut inner = constraint.walk();
        for part in constraint.children(&mut inner) {
            match (part.is_named(), part.kind()) {
                (false, "class") => flags |= CSharpAttr::CONSTRAINT_CLASS,
                (false, "?") => flags |= CSharpAttr::CONSTRAINT_NULLABLE_CLASS,
                (false, "struct") => flags |= CSharpAttr::CONSTRAINT_STRUCT,
                (false, "notnull") => flags |= CSharpAttr::CONSTRAINT_NOTNULL,
                (false, "unmanaged") => flags |= CSharpAttr::CONSTRAINT_UNMANAGED,
                (false, "default") => flags |= CSharpAttr::CONSTRAINT_DEFAULT,
                (true, "constructor_constraint") => flags |= CSharpAttr::CONSTRAINT_NEW,
                // `notnull` / `unmanaged` are also valid type names, so the grammar may
                // read them as a type constraint.
                (true, _) if constraint.child_by_field_name("type") == Some(part) => {
                    match node_text(&part, source) {
                        "notnull" => flags |= CSharpAttr::CONSTRAINT_NOTNULL,
                        "unmanaged" => flags |= CSharpAttr::CONSTRAINT_UNMANAGED,
                        _ => {}
                    }
                }
                _ => {}
            }
        }
    }
    flags
}

fn encode_block(kind: &str, node: &Node<'_>) -> u32 {
    aux_only(match kind {
        "unsafe_statement" => CSharpAttr::AUX_BLOCK_UNSAFE,
        "checked_statement" => {
            if first_child_kind(node) == "unchecked" {
                CSharpAttr::AUX_BLOCK_UNCHECKED
            } else {
                CSharpAttr::AUX_BLOCK_CHECKED
            }
        }
        "fixed_statement" => CSharpAttr::AUX_BLOCK_FIXED,
        _ => 0,
    })
}

/// True when the member an expression body `=> e` belongs to returns no value.
fn expression_body_discards_value(node: &Node<'_>, source: &[u8]) -> bool {
    let owner = match node.parent() {
        Some(owner) => owner,
        None => return false,
    };
    match owner.kind() {
        "constructor_declaration" | "destructor_declaration" => true,
        "accessor_declaration" => {
            owner.child_by_field_name("name").map(|n| n.kind()) != Some("get")
        }
        "method_declaration" | "local_function_statement" => {
            let field = if owner.kind() == "method_declaration" {
                "returns"
            } else {
                "type"
            };
            match owner.child_by_field_name(field) {
                Some(t) if t.kind() == "predefined_type" && node_text(&t, source) == "void" => {
                    true
                }
                // `async Task` / `async ValueTask` complete without a result; `async Task<T>`
                // returns the value of the expression.
                Some(t)
                    if CSharpModifiers::of(&owner, source).contains_any(CSharpModifiers::ASYNC) =>
                {
                    !is_generic_type(&t)
                }
                _ => false,
            }
        }
        _ => false,
    }
}

fn is_generic_type(node: &Node<'_>) -> bool {
    match node.kind() {
        "generic_name" => true,
        "qualified_name" | "alias_qualified_name" => node
            .child_by_field_name("name")
            .is_some_and(|n| n.kind() == "generic_name"),
        _ => false,
    }
}

fn is_pattern_kind(kind: &str) -> bool {
    matches!(
        kind,
        "and_pattern"
            | "constant_pattern"
            | "declaration_pattern"
            | "discard"
            | "list_pattern"
            | "negated_pattern"
            | "or_pattern"
            | "parenthesized_pattern"
            | "recursive_pattern"
            | "relational_pattern"
            | "type_pattern"
            | "var_pattern"
    )
}

fn encode_switch_case(kind: &str, node: &Node<'_>) -> u32 {
    let mut aux = 0u8;
    if kind == "switch_expression_arm" {
        aux |= NodeAttr::AUX_CASE_ARROW;
        let guarded = has_named_child(node, "when_clause");
        if guarded {
            aux |= CSharpAttr::AUX_CASE_GUARD | NodeAttr::AUX_CASE_PATTERN;
        }
        match first_named_child_kind(node) {
            "discard" if !guarded => aux |= NodeAttr::AUX_CASE_DEFAULT,
            k if is_pattern_kind(k) && k != "constant_pattern" => {
                aux |= NodeAttr::AUX_CASE_PATTERN
            }
            _ => {}
        }
        return aux_only(aux);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match (child.is_named(), child.kind()) {
            (false, "default") => aux |= NodeAttr::AUX_CASE_DEFAULT,
            (true, "when_clause") => {
                aux |= CSharpAttr::AUX_CASE_GUARD | NodeAttr::AUX_CASE_PATTERN
            }
            (true, k) if is_pattern_kind(k) && k != "constant_pattern" => {
                aux |= NodeAttr::AUX_CASE_PATTERN
            }
            _ => {}
        }
    }
    aux_only(aux)
}

fn encode_pattern(kind: &str, node: &Node<'_>) -> u32 {
    let (payload, op) = match kind {
        "constant_pattern" => (CSharpAttr::PATTERN_CONSTANT, OperatorId::None),
        "declaration_pattern" => (CSharpAttr::PATTERN_DECLARATION, OperatorId::None),
        "var_pattern" => (CSharpAttr::PATTERN_VAR, OperatorId::None),
        "discard" => (CSharpAttr::PATTERN_DISCARD, OperatorId::None),
        "type_pattern" => (CSharpAttr::PATTERN_TYPE, OperatorId::None),
        "recursive_pattern" => (CSharpAttr::PATTERN_RECURSIVE, OperatorId::None),
        "relational_pattern" => (
            CSharpAttr::PATTERN_RELATIONAL,
            OperatorId::from_symbol(first_child_kind(node), OperatorArity::Binary)
                .unwrap_or(OperatorId::None),
        ),
        "negated_pattern" => (CSharpAttr::PATTERN_NOT, OperatorId::LogicalNot),
        "and_pattern" => (CSharpAttr::PATTERN_AND, OperatorId::LogicalAnd),
        "or_pattern" => (CSharpAttr::PATTERN_OR, OperatorId::LogicalOr),
        "list_pattern" => (CSharpAttr::PATTERN_LIST, OperatorId::None),
        ".." => (CSharpAttr::PATTERN_SLICE, OperatorId::None),
        "subpattern" => (CSharpAttr::PATTERN_SUBPATTERN, OperatorId::None),
        "when_clause" => (CSharpAttr::PATTERN_GUARD, OperatorId::None),
        _ => (0, OperatorId::None),
    };
    NodeAttr::pack(NodeAttr::VISIBILITY_NONE, 0, op, 0, payload)
}

fn encode_catch(node: &Node<'_>) -> u32 {
    let mut aux = 0u8;
    if has_named_child(node, "catch_filter_clause") {
        aux |= CSharpAttr::AUX_CATCH_FILTER;
    }
    if !has_named_child(node, "catch_declaration") {
        aux |= CSharpAttr::AUX_CATCH_GENERAL;
    }
    aux_only(aux)
}

fn encode_foreach(node: &Node<'_>) -> u32 {
    let mut aux = 0u8;
    if has_anonymous_child(node, "await") {
        aux |= CSharpAttr::AUX_FOREACH_AWAIT;
    }
    if node
        .child_by_field_name("left")
        .is_some_and(|l| l.kind() == "tuple_pattern")
    {
        aux |= CSharpAttr::AUX_FOREACH_DECONSTRUCTION;
    }
    aux_only(aux)
}

fn encode_using_statement(node: &Node<'_>) -> u32 {
    let mut aux = 0u8;
    if has_anonymous_child(node, "await") {
        aux |= CSharpAttr::AUX_USING_STMT_AWAIT;
    }
    if has_named_child(node, "variable_declaration") {
        aux |= CSharpAttr::AUX_USING_STMT_DECLARATION;
    }
    aux_only(aux)
}

fn encode_goto(node: &Node<'_>) -> u32 {
    let mut aux = 0u8;
    if has_anonymous_child(node, "case") {
        aux |= CSharpAttr::AUX_GOTO_CASE;
    }
    if has_anonymous_child(node, "default") {
        aux |= CSharpAttr::AUX_GOTO_DEFAULT;
    }
    aux_only(aux)
}

fn encode_call(kind: &str, node: &Node<'_>) -> u32 {
    let (aux, payload) = match kind {
        "invocation_expression" => {
            let aux = match node.child_by_field_name("function") {
                Some(f) if f.kind() == "member_access_expression" => NodeAttr::AUX_CALL_QUALIFIED,
                Some(f)
                    if f.kind() == "conditional_access_expression"
                        && has_named_child(&f, "member_binding_expression") =>
                {
                    NodeAttr::AUX_CALL_QUALIFIED | CSharpAttr::AUX_NULL_AWARE
                }
                _ => 0,
            };
            (aux, CSharpAttr::CALL_INVOCATION)
        }
        "constructor_initializer" => {
            let mut aux = NodeAttr::AUX_CALL_EXPLICIT_CTOR;
            if has_anonymous_child(node, "base") {
                aux |= NodeAttr::AUX_CALL_SUPER_TARGET;
            }
            (aux, CSharpAttr::CALL_INVOCATION)
        }
        "primary_constructor_base_type" | "argument_list" => (
            NodeAttr::AUX_CALL_EXPLICIT_CTOR | NodeAttr::AUX_CALL_SUPER_TARGET,
            CSharpAttr::CALL_INVOCATION,
        ),
        "query_expression" => (0, CSharpAttr::CALL_QUERY),
        "from_clause" => (0, CSharpAttr::CALL_QUERY_FROM),
        "join_clause" => (0, CSharpAttr::CALL_QUERY_JOIN),
        "join_into_clause" => (0, CSharpAttr::CALL_QUERY_JOIN_INTO),
        "let_clause" => (0, CSharpAttr::CALL_QUERY_LET),
        "where_clause" => (0, CSharpAttr::CALL_QUERY_WHERE),
        "order_by_clause" => (0, CSharpAttr::CALL_QUERY_ORDER_BY),
        "select_clause" => (0, CSharpAttr::CALL_QUERY_SELECT),
        "group_clause" => (0, CSharpAttr::CALL_QUERY_GROUP),
        _ => (0, CSharpAttr::CALL_INVOCATION),
    };
    NodeAttr::pack(NodeAttr::VISIBILITY_NONE, 0, OperatorId::None, aux, payload)
}

fn encode_access(kind: &str, node: &Node<'_>) -> u32 {
    aux_only(match kind {
        "member_access_expression" if has_anonymous_child(node, "->") => {
            CSharpAttr::AUX_ACCESS_POINTER
        }
        "conditional_access_expression" => CSharpAttr::AUX_NULL_AWARE,
        "member_binding_expression" | "element_binding_expression" => {
            CSharpAttr::AUX_ACCESS_IMPLICIT_RECEIVER
        }
        _ => 0,
    })
}

fn encode_binary(kind: &str, node: &Node<'_>) -> u32 {
    if kind == "range_expression" {
        return payload_only(CSharpAttr::BINARY_RANGE);
    }
    let symbol = node
        .child_by_field_name("operator")
        .map_or("", |op| op.kind());
    if symbol == "??" {
        return aux_only(CSharpAttr::AUX_NULL_AWARE);
    }
    let op = OperatorId::from_symbol(symbol, OperatorArity::Binary).unwrap_or(OperatorId::None);
    NodeAttr::pack(NodeAttr::VISIBILITY_NONE, 0, op, 0, 0)
}

fn encode_assignment(kind: &str, node: &Node<'_>) -> u32 {
    if kind == "attribute_argument" {
        return NodeAttr::pack(NodeAttr::VISIBILITY_NONE, 0, OperatorId::Assign, 0, 0);
    }
    let symbol = node
        .child_by_field_name("operator")
        .map_or("", |op| op.kind());
    if symbol == "??=" {
        return aux_only(CSharpAttr::AUX_NULL_AWARE);
    }
    let op =
        OperatorId::from_symbol(symbol, OperatorArity::Assignment).unwrap_or(OperatorId::None);
    NodeAttr::pack(NodeAttr::VISIBILITY_NONE, 0, op, 0, 0)
}

fn encode_unary(kind: &str, node: &Node<'_>) -> u32 {
    match kind {
        "prefix_unary_expression" => match first_child_kind(node) {
            "&" => payload_only(CSharpAttr::UNARY_ADDRESS_OF),
            "*" => payload_only(CSharpAttr::UNARY_DEREFERENCE),
            "^" => payload_only(CSharpAttr::UNARY_INDEX_FROM_END),
            symbol => NodeAttr::pack(
                NodeAttr::VISIBILITY_NONE,
                0,
                OperatorId::from_symbol(symbol, OperatorArity::Unary).unwrap_or(OperatorId::None),
                0,
                0,
            ),
        },
        "postfix_unary_expression" => NodeAttr::pack(
            NodeAttr::VISIBILITY_NONE,
            0,
            OperatorId::from_symbol(last_child_kind(node), OperatorArity::Unary)
                .unwrap_or(OperatorId::None),
            NodeAttr::AUX_UNARY_POSTFIX,
            0,
        ),
        "unary_expression" => {
            NodeAttr::pack(NodeAttr::VISIBILITY_NONE, 0, OperatorId::LogicalNot, 0, 0)
        }
        "await_expression" => payload_only(CSharpAttr::UNARY_AWAIT),
        "checked_expression" => payload_only(if first_child_kind(node) == "unchecked" {
            CSharpAttr::UNARY_UNCHECKED
        } else {
            CSharpAttr::UNARY_CHECKED
        }),
        "sizeof_expression" => payload_only(CSharpAttr::UNARY_SIZEOF),
        "ref_expression" => payload_only(CSharpAttr::UNARY_REF),
        "makeref_expression" => payload_only(CSharpAttr::UNARY_MAKEREF),
        "reftype_expression" => payload_only(CSharpAttr::UNARY_REFTYPE),
        "refvalue_expression" => payload_only(CSharpAttr::UNARY_REFVALUE),
        "argument" => payload_only(argument_pass_mode(node).unwrap_or(0)),
        _ => 0,
    }
}

fn encode_lambda(kind: &str, node: &Node<'_>, source: &[u8]) -> u32 {
    let written = CSharpModifiers::of(node, source);
    let mut aux = 0u8;
    if kind == "anonymous_method_expression" {
        aux |= CSharpAttr::AUX_LAMBDA_ANONYMOUS_METHOD;
    } else if node
        .child_by_field_name("body")
        .is_some_and(|b| b.kind() != "block")
    {
        aux |= NodeAttr::AUX_LAMBDA_EXPRESSION_BODY;
    }
    if written.contains_any(CSharpModifiers::ASYNC) {
        aux |= CSharpAttr::AUX_LAMBDA_ASYNC;
    }
    if written.contains_any(CSharpModifiers::STATIC) {
        aux |= CSharpAttr::AUX_LAMBDA_STATIC;
    }
    let lang = if has_named_child(node, "attribute_list") {
        NodeAttr::LANG_HAS_ANNOTATIONS
    } else {
        0
    };
    NodeAttr::pack(NodeAttr::VISIBILITY_NONE, 0, OperatorId::None, aux, lang)
}

/// `INIT_*` payload of an initializer or tuple.
fn initializer_kind(kind: &str, node: &Node<'_>) -> u8 {
    match kind {
        "tuple_expression" => return CSharpAttr::INIT_TUPLE,
        "with_initializer" => return CSharpAttr::INIT_OBJECT,
        _ => {}
    }
    match node.parent() {
        Some(parent)
            if matches!(
                parent.kind(),
                "array_creation_expression"
                    | "implicit_array_creation_expression"
                    | "stackalloc_expression"
                    | "implicit_stackalloc_expression"
                    | "variable_declarator"
            ) =>
        {
            CSharpAttr::INIT_ARRAY
        }
        Some(parent)
            if parent.kind() == "initializer_expression"
                && initializer_kind("initializer_expression", &parent) == CSharpAttr::INIT_ARRAY =>
        {
            CSharpAttr::INIT_ARRAY
        }
        _ => {
            let mut cursor = node.walk();
            let mut elements = node.named_children(&mut cursor).peekable();
            if elements.peek().is_none() {
                return CSharpAttr::INIT_OBJECT;
            }
            if elements.any(|e| e.kind() == "assignment_expression") {
                CSharpAttr::INIT_OBJECT
            } else {
                CSharpAttr::INIT_COLLECTION
            }
        }
    }
}

fn encode_literal(kind: &str, node: &Node<'_>, source: &[u8]) -> u32 {
    let (category, aux) = match kind {
        "integer_literal" => (NodeAttr::LITERAL_INTEGER, 0),
        "real_literal" => (NodeAttr::LITERAL_FLOAT, 0),
        "character_literal" => (NodeAttr::LITERAL_CHAR, 0),
        "boolean_literal" | "true" | "false" => (NodeAttr::LITERAL_BOOLEAN, 0),
        "null_literal" => (NodeAttr::LITERAL_NULL, 0),
        "string_literal" => {
            let utf8 = if has_named_child(node, "string_literal_encoding") {
                CSharpAttr::AUX_STRING_UTF8
            } else {
                0
            };
            (NodeAttr::LITERAL_STRING, utf8)
        }
        "verbatim_string_literal" => (
            NodeAttr::LITERAL_STRING,
            CSharpAttr::AUX_STRING_VERBATIM | utf8_suffix(node, source),
        ),
        "raw_string_literal" => (
            NodeAttr::LITERAL_TEXT_BLOCK,
            CSharpAttr::AUX_STRING_RAW | utf8_suffix(node, source),
        ),
        "interpolated_string_expression" => {
            let start = first_child_of_kind(node, "interpolation_start", true)
                .map_or("", |s| node_text(&s, source));
            let raw = start.contains("\"\"\"") || has_named_child(node, "interpolation_quote");
            let mut aux = CSharpAttr::AUX_STRING_INTERPOLATED;
            if start.contains('@') {
                aux |= CSharpAttr::AUX_STRING_VERBATIM;
            }
            if raw {
                aux |= CSharpAttr::AUX_STRING_RAW;
                (NodeAttr::LITERAL_TEXT_BLOCK, aux)
            } else {
                (NodeAttr::LITERAL_STRING, aux)
            }
        }
        "typeof_expression" => (NodeAttr::LITERAL_CLASS, 0),
        "default_expression" => (CSharpAttr::LITERAL_DEFAULT, 0),
        _ => (0, 0),
    };
    NodeAttr::pack(NodeAttr::VISIBILITY_NONE, 0, OperatorId::None, aux, category)
}

fn utf8_suffix(node: &Node<'_>, source: &[u8]) -> u8 {
    let text = node_text(node, source);
    if text.ends_with("u8") || text.ends_with("U8") {
        CSharpAttr::AUX_STRING_UTF8
    } else {
        0
    }
}

fn encode_type_ref(kind: &str, node: &Node<'_>, source: &[u8]) -> u32 {
    let (aux, lang) = match kind {
        "array_type" => (NodeAttr::AUX_TYPE_ARRAY, array_rank_count(node)),
        "generic_name" => (NodeAttr::AUX_TYPE_GENERIC, CSharpAttr::TYPE_NAMED),
        "qualified_name" | "alias_qualified_name" => {
            (NodeAttr::AUX_TYPE_QUALIFIED, CSharpAttr::TYPE_NAMED)
        }
        "predefined_type" if node_text(node, source) == "void" => (0, CSharpAttr::TYPE_VOID),
        "predefined_type" => (0, CSharpAttr::TYPE_PREDEFINED),
        "implicit_type" => (0, CSharpAttr::TYPE_IMPLICIT),
        "nullable_type" => (0, CSharpAttr::TYPE_NULLABLE),
        "pointer_type" => (0, CSharpAttr::TYPE_POINTER),
        "ref_type" => (0, CSharpAttr::TYPE_REF),
        "scoped_type" => (0, CSharpAttr::TYPE_SCOPED),
        "tuple_type" => (0, CSharpAttr::TYPE_TUPLE),
        "function_pointer_type" => (0, CSharpAttr::TYPE_FUNCTION_POINTER),
        _ => (0, CSharpAttr::TYPE_NAMED),
    };
    NodeAttr::pack(NodeAttr::VISIBILITY_NONE, 0, OperatorId::None, aux, lang)
}

/// Number of rank specifiers of a (jagged) array type: `int[][,]` has two.
fn array_rank_count(node: &Node<'_>) -> u8 {
    let mut count = 1u32;
    let mut element = node.child_by_field_name("type");
    while let Some(t) = element {
        if t.kind() != "array_type" {
            break;
        }
        count += 1;
        element = t.child_by_field_name("type");
    }
    u8::try_from(count).unwrap_or(u8::MAX)
}

fn encode_identifier(kind: &str, node: &Node<'_>, source: &[u8]) -> u32 {
    aux_only(match kind {
        "qualified_name" | "alias_qualified_name" => {
            if kind == "alias_qualified_name" || node_text(node, source).contains("::") {
                NodeAttr::AUX_IDENT_QUALIFIED | CSharpAttr::AUX_IDENT_ALIAS_QUALIFIED
            } else {
                NodeAttr::AUX_IDENT_QUALIFIED
            }
        }
        "generic_name" => CSharpAttr::AUX_IDENT_GENERIC,
        _ => 0,
    })
}

/// `TARGET_*` of an attribute, from its list's `target:` specifier.
fn attribute_target(node: &Node<'_>, source: &[u8]) -> u8 {
    match node.parent() {
        Some(parent) if parent.kind() == "global_attribute" => {
            if has_anonymous_child(&parent, "module") {
                CSharpAttr::TARGET_MODULE
            } else {
                CSharpAttr::TARGET_ASSEMBLY
            }
        }
        Some(parent) if parent.kind() == "attribute_list" => attribute_list_target(&parent, source),
        _ => CSharpAttr::TARGET_DEFAULT,
    }
}

fn attribute_list_target(list: &Node<'_>, source: &[u8]) -> u8 {
    let specifier = match first_child_of_kind(list, "attribute_target_specifier", true) {
        Some(s) => s,
        None => return CSharpAttr::TARGET_DEFAULT,
    };
    match node_text(&specifier, source).trim_end_matches(':').trim() {
        "assembly" => CSharpAttr::TARGET_ASSEMBLY,
        "module" => CSharpAttr::TARGET_MODULE,
        "field" => CSharpAttr::TARGET_FIELD,
        "event" => CSharpAttr::TARGET_EVENT,
        "method" => CSharpAttr::TARGET_METHOD,
        "param" => CSharpAttr::TARGET_PARAM,
        "property" => CSharpAttr::TARGET_PROPERTY,
        "return" => CSharpAttr::TARGET_RETURN,
        "type" => CSharpAttr::TARGET_TYPE,
        _ => CSharpAttr::TARGET_DEFAULT,
    }
}

/// True when a `params` array (an anonymous `params` keyword) is among the parameters.
fn has_params_array(list: Option<Node<'_>>) -> bool {
    match list {
        Some(list) => has_anonymous_child(&list, "params"),
        None => false,
    }
}

// ── Tree helpers ─────────────────────────────────────────────────────────────

#[inline]
fn parent_kind(node: &Node<'_>) -> &'static str {
    node.parent().map_or("", |p| p.kind())
}

/// True when `node` is the child of `parent` in `field`.
#[inline]
fn is_field<'tree>(parent: &Node<'tree>, field: &str, node: &Node<'tree>) -> bool {
    parent.child_by_field_name(field) == Some(*node)
}

#[inline]
fn next_sibling_kind(node: &Node<'_>) -> &'static str {
    node.next_sibling().map_or("", |n| n.kind())
}

#[inline]
fn prev_sibling_kind(node: &Node<'_>) -> &'static str {
    node.prev_sibling().map_or("", |n| n.kind())
}

fn first_child_of_kind<'tree>(node: &Node<'tree>, kind: &str, named: bool) -> Option<Node<'tree>> {
    let mut cursor = node.walk();
    let found = node
        .children(&mut cursor)
        .find(|c| c.is_named() == named && c.kind() == kind);
    found
}

fn has_named_child(node: &Node<'_>, kind: &str) -> bool {
    first_child_of_kind(node, kind, true).is_some()
}

fn has_anonymous_child(node: &Node<'_>, kind: &str) -> bool {
    first_child_of_kind(node, kind, false).is_some()
}

fn first_child_kind(node: &Node<'_>) -> &'static str {
    let mut cursor = node.walk();
    let kind = node.children(&mut cursor).next().map_or("", |c| c.kind());
    kind
}

fn last_child_kind(node: &Node<'_>) -> &'static str {
    let mut cursor = node.walk();
    let kind = node.children(&mut cursor).last().map_or("", |c| c.kind());
    kind
}

fn first_named_child_kind(node: &Node<'_>) -> &'static str {
    let mut cursor = node.walk();
    let kind = node
        .named_children(&mut cursor)
        .next()
        .map_or("", |c| c.kind());
    kind
}

#[inline]
fn node_text<'s>(node: &Node<'_>, source: &'s [u8]) -> &'s str {
    node.utf8_text(source).unwrap_or("")
}
