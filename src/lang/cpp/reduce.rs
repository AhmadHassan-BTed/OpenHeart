//! Phase 2 adapter for tree-sitter-cpp.
//!
//! The C++-only kinds are classified and encoded here; every kind tree-sitter-cpp
//! inherits from tree-sitter-c goes through the C-family core in `lang::c::reduce`
//! (with `Dialect::Cpp`), whose conventions this module follows. C++ additions:
//! * `namespace a { }`, `namespace a::b { }`, `inline namespace`, unnamed namespaces and
//!   namespace aliases are `NN_NAMESPACE_DECL`; `using namespace` / using-declarations
//!   are `NN_USING_DIRECTIVE`; `using T = U;` and concepts are `NN_TYPEDEF_DECL`.
//! * `class` specifiers are `NN_CLASS_DECL`; member access comes from the preceding
//!   access specifier or the class-key default. Base specifiers are `NN_TYPE_REF`
//!   children of the class whose visibility is the inheritance access
//!   (`TYPE_BASE_VIRTUAL` for virtual inheritance); `friend class X;` is an `NN_TYPE_REF`
//!   flagged `TYPE_FRIEND`, and friend functions are `NN_FUNCTION_DECL` /
//!   `NN_OPERATOR_DECL` with `VISIBILITY_NONE` inside the class body.
//! * Constructors carry their mem-initializers as `NN_CALL_EXPR` children flagged
//!   `AUX_CALL_EXPLICIT_CTOR` (plus `AUX_CALL_SUPER_TARGET` when the initializer names a
//!   base of the class of an in-class definition), between the parameters and the body.
//! * `template<...>` is an `NN_TEMPLATE_DECL` holding one `NN_TYPE_PARAM` per template
//!   parameter followed by the templated declaration; `template<>` and partial
//!   specializations, constraints and explicit instantiations are aux flags.
//! * Qualified names (`std::vector<int>`, `A::f`) are one `NN_TYPE_REF` /
//!   `NN_IDENTIFIER_EXPR` flagged qualified whose children are the scope components then
//!   the final name; nested qualifiers are flattened.
//! * Lambdas are `NN_LAMBDA_EXPR` `[captures..., params..., body]`; range-for is
//!   `NN_ENHANCED_FOR` `[init?, type, declarator, range, body]`; `try` / `catch` /
//!   `throw`, `co_return` / `co_yield` / `co_await`, `new` / `delete` map onto the
//!   existing statement and expression kinds with C-family payloads.
//! * Compile-time-only constructs (`static_assert`, requires-clauses and
//!   requires-expressions, `noexcept` / `override` / `= default` specifiers) are dropped
//!   from the tree and recorded in the attribute words of their owners.

use tree_sitter::Node;

use crate::ast::adapter::{ASTReductionAdapter, ReductionDecision};
use crate::core::types::ast::{ASTNodeType, NodeAttr, OperatorArity, OperatorId};
use crate::lang::c::reduce::{
    access_from_text, classify_c_family, classify_parameter, collect_specifiers,
    declaration_context, encode_c_family, encode_parameter, encode_record,
    encode_template_parameter, encode_typedef, has_child_kind, is_variable_declarator,
    member_access, node_text, normalize_operator, overloaded_operator, specifier_role,
    terminal_name, CFamilyAttr, DeclContext, SpecifierRole, SyntaxErrorTally,
};
use crate::lang::c::Dialect;

/// Phase 2 reduction adapter for C++ translation units (tree-sitter-cpp).
pub struct CppASTReductionAdapter {
    syntax_errors: SyntaxErrorTally,
}

impl Default for CppASTReductionAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl CppASTReductionAdapter {
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

impl ASTReductionAdapter for CppASTReductionAdapter {
    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_cpp::language()
    }

    fn classify(&self, kind: &str, node: &Node, _depth: usize) -> ReductionDecision {
        if let Some(decision) = self.syntax_errors.record(node) {
            return decision;
        }
        classify_cpp(kind, node).unwrap_or_else(|| classify_c_family(kind, node, Dialect::Cpp))
    }

    fn encode_attrs(&self, kind: &str, node: &Node, source: &[u8]) -> u32 {
        encode_cpp(kind, node, source)
            .unwrap_or_else(|| encode_c_family(kind, node, source, Dialect::Cpp))
    }
}

/// Classifies the C++-only kinds; `None` hands the kind to the C-family core.
pub fn classify_cpp(kind: &str, node: &Node) -> Option<ReductionDecision> {
    use ASTNodeType::*;
    use ReductionDecision::*;

    let decision = match kind {
        // ── Namespaces and using ──
        "namespace_definition" | "namespace_alias_definition" => Keep(NN_NAMESPACE_DECL),
        "nested_namespace_specifier" => Eliminate,
        "namespace_identifier" => Keep(NN_IDENTIFIER_EXPR),
        "using_declaration" => Keep(NN_USING_DIRECTIVE),
        "alias_declaration" | "concept_definition" => Keep(NN_TYPEDEF_DECL),

        // ── Classes ──
        "class_specifier" => match specifier_role(node) {
            SpecifierRole::Reference => Eliminate,
            _ => Keep(NN_CLASS_DECL),
        },
        "base_class_clause" | "friend_declaration" => Eliminate,
        // Encoded as the visibility of the following members.
        "access_specifier" => Drop,
        "field_initializer_list" => Eliminate,
        "field_initializer" => Keep(NN_CALL_EXPR),

        // ── Templates ──
        "template_declaration" | "template_instantiation" => Keep(NN_TEMPLATE_DECL),
        "template_parameter_list" | "template_argument_list" => Eliminate,
        "type_parameter_declaration"
        | "variadic_type_parameter_declaration"
        | "optional_type_parameter_declaration" => {
            let inside_template_template = node
                .parent()
                .is_some_and(|p| p.kind() == "template_template_parameter_declaration");
            if inside_template_template {
                Eliminate
            } else {
                Keep(NN_TYPE_PARAM)
            }
        }
        "template_template_parameter_declaration" => Keep(NN_TYPE_PARAM),
        "optional_parameter_declaration" | "variadic_parameter_declaration" => {
            classify_parameter(node)
        }

        // ── Names and types ──
        "qualified_identifier" => classify_qualified(node),
        "template_type" | "dependent_type" | "decltype" | "placeholder_type_specifier" => {
            Keep(NN_TYPE_REF)
        }
        "template_function" | "template_method" | "operator_name" => Keep(NN_IDENTIFIER_EXPR),
        "auto" => match node.parent() {
            Some(p) if p.kind() == "placeholder_type_specifier" => Drop,
            _ => Keep(NN_TYPE_REF),
        },
        // `~Foo`: the destructor kind is the declaration's node type.
        "destructor_name" | "dependent_name" | "operator_cast" => Eliminate,

        // ── Declarators ──
        "reference_declarator" | "structured_binding_declarator" => {
            if is_variable_declarator(node) {
                Keep(NN_VAR_DECLARATOR)
            } else {
                Eliminate
            }
        }
        "pointer_type_declarator"
        | "variadic_declarator"
        | "abstract_reference_declarator"
        | "new_declarator"
        | "trailing_return_type" => Eliminate,

        // ── Specifiers encoded in the owner's attribute word ──
        "virtual_specifier" | "virtual" | "explicit_function_specifier" | "ref_qualifier"
        | "noexcept" | "default_method_clause" | "delete_method_clause" => Drop,
        "throw_specifier" => Keep(NN_THROWS_CLAUSE),
        "alignas_specifier" => Keep(NN_ANNOTATION_USE),

        // ── Compile-time constructs without run-time semantics ──
        "static_assert_declaration"
        | "requires_clause"
        | "requires_expression"
        | "requirement_seq"
        | "simple_requirement"
        | "compound_requirement"
        | "type_requirement"
        | "constraint_conjunction"
        | "constraint_disjunction" => Drop,

        // ── Statements ──
        "for_range_loop" => Keep(NN_ENHANCED_FOR),
        "condition_clause" | "init_statement" => Eliminate,
        "try_statement" => Keep(NN_TRY_STMT),
        "catch_clause" => Keep(NN_CATCH_CLAUSE),
        "throw_statement" => Keep(NN_THROW_STMT),
        "co_return_statement" => Keep(NN_RETURN_STMT),
        "co_yield_statement" => Keep(NN_YIELD_STMT),

        // ── Expressions ──
        "lambda_expression" => Keep(NN_LAMBDA_EXPR),
        "lambda_capture_specifier" => Eliminate,
        "lambda_default_capture" => Drop,
        "new_expression" => Keep(NN_NEW_EXPR),
        "delete_expression" | "co_await_expression" | "parameter_pack_expansion" => {
            Keep(NN_UNARY_EXPR)
        }
        "fold_expression" => Keep(NN_BINARY_EXPR),
        "subscript_argument_list" => Eliminate,
        "this" => Keep(NN_THIS_EXPR),

        // ── Literals ──
        "raw_string_literal" | "user_defined_literal" => Keep(NN_LITERAL),
        "raw_string_content" | "raw_string_delimiter" | "literal_suffix" => Drop,

        _ => return None,
    };
    Some(decision)
}

/// A qualified name is a type reference when its final component names a type, an
/// identifier otherwise. Nested qualifiers (`a::b::c` is right-recursive) are flattened
/// into the outermost one.
fn classify_qualified(node: &Node) -> ReductionDecision {
    if node
        .parent()
        .is_some_and(|p| p.kind() == "qualified_identifier")
    {
        return ReductionDecision::Eliminate;
    }
    if names_type(node) {
        ReductionDecision::Keep(ASTNodeType::NN_TYPE_REF)
    } else {
        ReductionDecision::Keep(ASTNodeType::NN_IDENTIFIER_EXPR)
    }
}

fn names_type(qualified: &Node) -> bool {
    let (terminal, _) = terminal_name(*qualified);
    matches!(
        terminal.kind(),
        "type_identifier" | "template_type" | "dependent_type" | "primitive_type"
    )
}

/// Encodes the C++-only kinds and the C++ cases of shared kinds (base specifiers,
/// friend types); `None` hands the kind to the C-family core.
pub fn encode_cpp(kind: &str, node: &Node, source: &[u8]) -> Option<u32> {
    let none = NodeAttr::VISIBILITY_NONE;
    let attr = match kind {
        "namespace_definition" => {
            let mut aux = 0u8;
            if has_child_kind(node, "inline") {
                aux |= CFamilyAttr::AUX_NS_INLINE;
            }
            match node.child_by_field_name("name") {
                None => aux |= CFamilyAttr::AUX_NS_ANONYMOUS,
                Some(n) if n.kind() == "nested_namespace_specifier" => {
                    aux |= CFamilyAttr::AUX_NS_NESTED
                }
                Some(_) => {}
            }
            NodeAttr::pack(none, 0, OperatorId::None, aux, 0)
        }
        "namespace_alias_definition" => {
            NodeAttr::pack(none, 0, OperatorId::None, CFamilyAttr::AUX_NS_ALIAS, 0)
        }
        "using_declaration" => encode_using(node, source),
        "alias_declaration" => encode_typedef(node, source, CFamilyAttr::AUX_TYPEDEF_ALIAS_SYNTAX),
        "concept_definition" => encode_typedef(node, source, CFamilyAttr::AUX_TYPEDEF_CONCEPT),
        "class_specifier" => encode_record(node, source),
        "template_declaration" => encode_template(node),
        "template_instantiation" => NodeAttr::pack(
            none,
            0,
            OperatorId::None,
            CFamilyAttr::AUX_TEMPLATE_INSTANTIATION,
            0,
        ),
        "type_parameter_declaration"
        | "variadic_type_parameter_declaration"
        | "optional_type_parameter_declaration"
        | "template_template_parameter_declaration" => encode_template_parameter(node, source),
        "optional_parameter_declaration" | "variadic_parameter_declaration" => {
            match node.parent() {
                Some(p) if p.kind() == "template_parameter_list" => {
                    encode_template_parameter(node, source)
                }
                _ => encode_parameter(node, source),
            }
        }

        // ── Type and name references ──
        "type_identifier" => return encode_type_name(node, source, 0),
        "template_type" => {
            let generic = NodeAttr::AUX_TYPE_GENERIC;
            encode_type_name(node, source, generic)
                .unwrap_or_else(|| NodeAttr::pack(none, 0, OperatorId::None, generic, 0))
        }
        "qualified_identifier" => encode_qualified(node, source),
        "template_function" | "template_method" => NodeAttr::pack(
            none,
            0,
            OperatorId::None,
            CFamilyAttr::AUX_IDENT_TEMPLATE_ARGS,
            0,
        ),
        "operator_name" => NodeAttr::pack(
            none,
            0,
            overloaded_operator(node, source),
            CFamilyAttr::AUX_IDENT_OPERATOR_NAME,
            0,
        ),
        "decltype" => NodeAttr::pack(none, 0, OperatorId::None, 0, CFamilyAttr::TYPE_DECLTYPE),
        "placeholder_type_specifier" => {
            let lang = if has_child_kind(node, "decltype") {
                CFamilyAttr::TYPE_DECLTYPE
            } else {
                0
            };
            NodeAttr::pack(none, 0, OperatorId::None, 0, lang)
        }

        // ── Statements ──
        "for_range_loop" => {
            let aux = if node.child_by_field_name("initializer").is_some() {
                CFamilyAttr::AUX_STMT_HAS_INIT
            } else {
                0
            };
            NodeAttr::pack(none, 0, OperatorId::None, aux, 0)
        }
        "catch_clause" => {
            let catch_all = node
                .child_by_field_name("parameters")
                .is_some_and(|p| has_child_kind(&p, "..."));
            let aux = if catch_all {
                CFamilyAttr::AUX_CATCH_ALL
            } else {
                0
            };
            NodeAttr::pack(none, 0, OperatorId::None, aux, 0)
        }
        "co_return_statement" | "co_yield_statement" => {
            NodeAttr::pack(none, 0, OperatorId::None, 0, CFamilyAttr::STMT_COROUTINE)
        }

        // ── Expressions ──
        "field_initializer" => {
            let mut aux = CFamilyAttr::AUX_CALL_EXPLICIT_CTOR;
            if initializer_targets_base(node, source) {
                aux |= CFamilyAttr::AUX_CALL_SUPER_TARGET;
            }
            NodeAttr::pack(none, 0, OperatorId::None, aux, 0)
        }
        "lambda_expression" => encode_lambda(node, source),
        "new_expression" => {
            let mut lang = 0u8;
            if node.child_by_field_name("placement").is_some() {
                lang |= CFamilyAttr::NEW_PLACEMENT;
            }
            if node
                .child_by_field_name("declarator")
                .is_some_and(|d| d.kind() == "new_declarator")
            {
                lang |= CFamilyAttr::NEW_ARRAY;
            }
            if node.child(0).is_some_and(|c| c.kind() == "::") {
                lang |= CFamilyAttr::NEW_GLOBAL_SCOPE;
            }
            NodeAttr::pack(none, 0, OperatorId::None, 0, lang)
        }
        "delete_expression" => {
            let array = has_child_kind(node, "[") || has_child_kind(node, "[]");
            let lang = if array {
                CFamilyAttr::UNARY_DELETE_ARRAY
            } else {
                CFamilyAttr::UNARY_DELETE
            };
            NodeAttr::pack(none, 0, OperatorId::None, 0, lang)
        }
        "co_await_expression" => {
            NodeAttr::pack(none, 0, OperatorId::None, 0, CFamilyAttr::UNARY_CO_AWAIT)
        }
        "parameter_pack_expansion" => NodeAttr::pack(
            none,
            0,
            OperatorId::None,
            0,
            CFamilyAttr::UNARY_PACK_EXPANSION,
        ),
        "fold_expression" => {
            let symbol = node
                .child_by_field_name("operator")
                .map(|o| node_text(&o, source))
                .unwrap_or("");
            let symbol = normalize_operator(symbol);
            let op = OperatorId::from_symbol(symbol, OperatorArity::Binary)
                .or_else(|| OperatorId::from_symbol(symbol, OperatorArity::Assignment))
                .unwrap_or(OperatorId::None);
            NodeAttr::pack(none, 0, op, 0, CFamilyAttr::BINARY_FOLD)
        }
        _ => return None,
    };
    Some(attr)
}

/// A type name used as a base specifier or a friend type; `None` for any other use.
fn encode_type_name(node: &Node, source: &[u8], aux: u8) -> Option<u32> {
    let parent = node.parent()?;
    match parent.kind() {
        "base_class_clause" => Some(encode_base_specifier(node, &parent, source, aux)),
        "friend_declaration" => Some(NodeAttr::pack(
            NodeAttr::VISIBILITY_NONE,
            0,
            OperatorId::None,
            aux,
            CFamilyAttr::TYPE_FRIEND,
        )),
        _ => None,
    }
}

/// Inheritance access and virtuality of one base specifier: the access specifier and
/// `virtual` written since the previous `,` of the base clause, or the class-key default.
fn encode_base_specifier(node: &Node, clause: &Node, source: &[u8], aux: u8) -> u32 {
    let mut access = None;
    let mut is_virtual = false;
    let mut cursor = clause.walk();
    for child in clause.children(&mut cursor) {
        if child.id() == node.id() {
            break;
        }
        match child.kind() {
            "," => {
                access = None;
                is_virtual = false;
            }
            "access_specifier" => access = access_from_text(node_text(&child, source)),
            "virtual" => is_virtual = true,
            _ => {}
        }
    }
    let class_key_default = match clause.parent().map(|c| c.kind()) {
        Some("class_specifier") => NodeAttr::VISIBILITY_PRIVATE,
        _ => NodeAttr::VISIBILITY_PUBLIC,
    };
    let mut lang = 0u8;
    if is_virtual {
        lang |= CFamilyAttr::TYPE_BASE_VIRTUAL;
    }
    let vis = match access {
        Some(v) => v,
        None => {
            lang |= CFamilyAttr::TYPE_IMPLICIT_ACCESS;
            class_key_default
        }
    };
    NodeAttr::pack(vis, 0, OperatorId::None, aux, lang)
}

fn encode_qualified(node: &Node, source: &[u8]) -> u32 {
    let (terminal, _) = terminal_name(*node);
    let global = node.child_by_field_name("scope").is_none();
    if names_type(node) {
        let mut aux = NodeAttr::AUX_TYPE_QUALIFIED;
        if terminal.kind() == "template_type" {
            aux |= NodeAttr::AUX_TYPE_GENERIC;
        }
        return encode_type_name(node, source, aux)
            .unwrap_or_else(|| NodeAttr::pack(NodeAttr::VISIBILITY_NONE, 0, OperatorId::None, aux, 0));
    }
    let mut aux = CFamilyAttr::AUX_IDENT_QUALIFIED;
    if global {
        aux |= CFamilyAttr::AUX_IDENT_GLOBAL_SCOPE;
    }
    let mut op = OperatorId::None;
    let mut name = node.child_by_field_name("name");
    while let Some(n) = name {
        match n.kind() {
            "qualified_identifier" => name = n.child_by_field_name("name"),
            "template_function" | "template_method" => {
                aux |= CFamilyAttr::AUX_IDENT_TEMPLATE_ARGS;
                break;
            }
            "operator_name" => {
                aux |= CFamilyAttr::AUX_IDENT_OPERATOR_NAME;
                op = overloaded_operator(&n, source);
                break;
            }
            _ => break,
        }
    }
    NodeAttr::pack(NodeAttr::VISIBILITY_NONE, 0, op, aux, 0)
}

fn encode_using(node: &Node, source: &[u8]) -> u32 {
    let mut aux = 0u8;
    if has_child_kind(node, "namespace") {
        aux |= CFamilyAttr::AUX_USING_NAMESPACE;
    }
    if has_child_kind(node, "enum") {
        aux |= CFamilyAttr::AUX_USING_ENUM;
    }
    let (vis, lang) = match declaration_context(node) {
        DeclContext::Class(_) => {
            aux |= CFamilyAttr::AUX_USING_MEMBER;
            match member_access(node, source) {
                Some((v, true)) => (v, CFamilyAttr::LANG_IMPLICIT_VISIBILITY),
                Some((v, false)) => (v, 0),
                None => (NodeAttr::VISIBILITY_PUBLIC, CFamilyAttr::LANG_IMPLICIT_VISIBILITY),
            }
        }
        _ => (NodeAttr::VISIBILITY_NONE, 0),
    };
    NodeAttr::pack(vis, 0, OperatorId::None, aux, lang)
}

fn encode_template(node: &Node) -> u32 {
    let mut aux = 0u8;
    let full = node
        .child_by_field_name("parameters")
        .is_some_and(|p| p.named_child_count() == 0);
    if full {
        aux |= CFamilyAttr::AUX_TEMPLATE_FULL_SPECIALIZATION;
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "requires_clause" => aux |= CFamilyAttr::AUX_TEMPLATE_CONSTRAINED,
            "class_specifier" | "struct_specifier" | "union_specifier" if !full => {
                if child
                    .child_by_field_name("name")
                    .is_some_and(|n| n.kind() == "template_type")
                {
                    aux |= CFamilyAttr::AUX_TEMPLATE_PARTIAL_SPECIALIZATION;
                }
            }
            _ => {}
        }
    }
    NodeAttr::pack(NodeAttr::VISIBILITY_NONE, 0, OperatorId::None, aux, 0)
}

fn encode_lambda(node: &Node, source: &[u8]) -> u32 {
    let mut mods = 0u8;
    let mut lang = 0u8;
    if let Some(captures) = node.child_by_field_name("captures") {
        let mut cursor = captures.walk();
        for child in captures.children(&mut cursor) {
            if child.kind() == "lambda_default_capture" {
                match node_text(&child, source) {
                    "=" => lang |= CFamilyAttr::LAMBDA_CAPTURE_COPY,
                    "&" => lang |= CFamilyAttr::LAMBDA_CAPTURE_REFERENCE,
                    _ => {}
                }
            }
        }
    }
    if let Some(declarator) = node.child_by_field_name("declarator") {
        let specs = collect_specifiers(&declarator, source);
        if specs.is_mutable {
            mods |= CFamilyAttr::MOD_MUTABLE;
        }
        if specs.is_constexpr {
            lang |= CFamilyAttr::LANG_CONSTEXPR;
        }
        let mut cursor = declarator.walk();
        for child in declarator.children(&mut cursor) {
            if child.kind() == "noexcept" {
                let text: String = node_text(&child, source)
                    .chars()
                    .filter(|c| !c.is_whitespace())
                    .collect();
                if text == "noexcept" || text == "noexcept(true)" {
                    mods |= CFamilyAttr::MOD_NOEXCEPT;
                }
            }
        }
    }
    NodeAttr::pack(NodeAttr::VISIBILITY_NONE, mods, OperatorId::None, 0, lang)
}

/// True when a mem-initializer of an in-class constructor definition names a base class
/// listed in the class's base clause. Out-of-class definitions cannot see the base list
/// and are resolved by Phase 3.
fn initializer_targets_base(init: &Node, source: &[u8]) -> bool {
    let target = match init.named_child(0) {
        Some(n) => simple_type_name(n, source),
        None => return false,
    };
    if target.is_empty() {
        return false;
    }
    let function = match init.parent().and_then(|list| list.parent()) {
        Some(f) => f,
        None => return false,
    };
    let class = match declaration_context(&function) {
        DeclContext::Class(c) => c,
        _ => return false,
    };
    let mut cursor = class.walk();
    let clause = class
        .children(&mut cursor)
        .find(|c| c.kind() == "base_class_clause");
    let clause = match clause {
        Some(c) => c,
        None => return false,
    };
    let mut cursor = clause.walk();
    let found = clause
        .named_children(&mut cursor)
        .filter(|c| {
            matches!(
                c.kind(),
                "type_identifier" | "qualified_identifier" | "template_type"
            )
        })
        .any(|base| simple_type_name(base, source) == target);
    found
}

/// Unqualified name of a (possibly qualified or templated) type or member name.
fn simple_type_name<'s>(node: Node, source: &'s [u8]) -> &'s str {
    let (terminal, _) = terminal_name(node);
    let terminal = if terminal.kind() == "template_type" {
        terminal.child_by_field_name("name").unwrap_or(terminal)
    } else {
        terminal
    };
    node_text(&terminal, source)
}
