//! Java AST Reduction Adapter implementation (tree-sitter-java).
//!
//! Classification covers every construct of the grammar that carries program meaning:
//! compilation-unit structure (package, imports, modules), all type and member
//! declarations (including enum constants, records, compact constructors, annotation
//! elements, interface constants, instance initializers and anonymous class bodies),
//! constructor bodies and explicit constructor invocations, every statement (switch rules
//! and groups, try-with-resources, yield), every expression, patterns, and the full type
//! syntax (generic, array, qualified, wildcard types and type parameters).
//!
//! Unlisted kinds are *eliminated* (their children are kept), never dropped, so a node
//! kind this table does not know cannot silently delete a subtree; ERROR regions are
//! eliminated for the same reason. Only explicit trivia is dropped.
//!
//! Modifiers are read from the `modifiers` child by token kind (annotation arguments are
//! never inspected as text), and the JLS implicit modifiers and default access are
//! applied: interface members (9.3, 9.4, 9.5), annotation elements (9.6.1), enum constants
//! and constructors (8.9), records and record components (8.10), nested member types.

use super::{child_of_kind, children_of, ASTReductionAdapter, ReductionContext, ReductionDecision};
use crate::core::types::ast::{ASTNodeType, NodeAttr, OperatorArity, OperatorId};
use tree_sitter::Node;

pub struct JavaASTReductionAdapter;

impl Default for JavaASTReductionAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl JavaASTReductionAdapter {
    pub fn new() -> Self {
        Self
    }
}

/// CST kinds whose children are statements.
fn is_statement_container(kind: &str) -> bool {
    matches!(
        kind,
        "block" | "constructor_body" | "switch_block_statement_group" | "program"
    )
}

/// True when a `switch_expression` is used as a statement rather than as a value.
fn is_statement_position(ctx: &ReductionContext) -> bool {
    match ctx.parent_kind() {
        Some(p) if is_statement_container(p) || p == "labeled_statement" => true,
        Some(
            "if_statement"
            | "while_statement"
            | "for_statement"
            | "enhanced_for_statement"
            | "do_statement",
        ) => !matches!(
            ctx.field_name,
            Some("condition" | "init" | "update" | "value")
        ),
        _ => false,
    }
}

/// Where a declaration sits, for the JLS default-access and implicit-modifier rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Container {
    TopLevel,
    ClassBody,
    InterfaceBody,
    AnnotationBody,
    /// `enum_body_declarations`: members of an enum after the constants.
    EnumMembers,
    /// `enum_body`: the constant list itself.
    EnumConstants,
    /// Inside a method, constructor, initializer or lambda body.
    Local,
    /// Parameter lists and other non-member positions.
    Other,
}

fn container_of(ctx: &ReductionContext) -> Container {
    match ctx.parent_kind() {
        Some("program") => Container::TopLevel,
        Some("class_body") => Container::ClassBody,
        Some("interface_body") => Container::InterfaceBody,
        Some("annotation_type_body") => Container::AnnotationBody,
        Some("enum_body_declarations") => Container::EnumMembers,
        Some("enum_body") => Container::EnumConstants,
        Some(p) if is_statement_container(p) => Container::Local,
        Some("labeled_statement" | "switch_rule") => Container::Local,
        _ => Container::Other,
    }
}

fn is_type_declaration(kind: &str) -> bool {
    matches!(
        kind,
        "class_declaration"
            | "interface_declaration"
            | "enum_declaration"
            | "record_declaration"
            | "annotation_type_declaration"
    )
}

/// Modifiers written in source.
#[derive(Debug, Clone, Copy, Default)]
struct ModifierScan {
    visibility: Option<u8>,
    mods: u8,
    lang: u8,
}

fn annotation_is_deprecated(annotation: &Node, source: &[u8]) -> bool {
    annotation
        .child_by_field_name("name")
        .and_then(|n| n.utf8_text(source).ok())
        .is_some_and(|name| name == "Deprecated" || name == "java.lang.Deprecated")
}

/// Reads the `modifiers` child by token kind. Annotation children only set the
/// annotation flags; their arguments are never matched against modifier keywords.
fn scan_modifiers(node: &Node, source: &[u8]) -> ModifierScan {
    let mut scan = ModifierScan::default();
    let Some(modifiers) = child_of_kind(node, "modifiers") else {
        return scan;
    };
    for child in children_of(&modifiers) {
        match child.kind() {
            "public" => scan.visibility = Some(NodeAttr::VISIBILITY_PUBLIC),
            "private" => scan.visibility = Some(NodeAttr::VISIBILITY_PRIVATE),
            "protected" => scan.visibility = Some(NodeAttr::VISIBILITY_PROTECTED),
            "static" => scan.mods |= NodeAttr::MOD_STATIC,
            "final" => scan.mods |= NodeAttr::MOD_FINAL,
            "abstract" => scan.mods |= NodeAttr::MOD_ABSTRACT,
            "synchronized" => scan.mods |= NodeAttr::MOD_SYNCHRONIZED,
            "native" => scan.mods |= NodeAttr::MOD_NATIVE,
            "volatile" => scan.mods |= NodeAttr::MOD_VOLATILE,
            "transient" => scan.mods |= NodeAttr::MOD_TRANSIENT,
            "sealed" => scan.mods |= NodeAttr::MOD_SEALED,
            "strictfp" => scan.lang |= NodeAttr::LANG_JAVA_STRICTFP,
            "default" => scan.lang |= NodeAttr::LANG_JAVA_DEFAULT,
            "non-sealed" => scan.lang |= NodeAttr::LANG_JAVA_NON_SEALED,
            "marker_annotation" | "annotation" => {
                scan.lang |= NodeAttr::LANG_HAS_ANNOTATIONS;
                if annotation_is_deprecated(&child, source) {
                    scan.lang |= NodeAttr::LANG_DEPRECATED;
                }
            }
            _ => {}
        }
    }
    scan
}

/// JLS 8.9: an enum is implicitly final unless a constant has a class body.
fn enum_has_constant_bodies(enum_decl: &Node) -> bool {
    let Some(body) = enum_decl.child_by_field_name("body") else {
        return false;
    };
    children_of(&body)
        .iter()
        .any(|c| c.kind() == "enum_constant" && c.child_by_field_name("body").is_some())
}

fn has_spread_parameter(decl: &Node) -> bool {
    decl.child_by_field_name("parameters")
        .is_some_and(|params| {
            children_of(&params)
                .iter()
                .any(|c| c.kind() == "spread_parameter")
        })
}

/// An identifier that is an implicitly typed lambda parameter (`x -> ..`, `(a, b) -> ..`).
fn is_lambda_parameter(ctx: &ReductionContext) -> bool {
    let parent = ctx.parent_kind();
    parent == Some("inferred_parameters")
        || (parent == Some("lambda_expression") && ctx.field_name == Some("parameters"))
}

fn is_record_component(ctx: &ReductionContext) -> bool {
    ctx.parent_kind() == Some("formal_parameters")
        && ctx.grandparent_kind() == Some("record_declaration")
}

/// Visibility, modifiers and lang flags of a declaration after the JLS rules.
fn declaration_attrs(
    kind: &str,
    node: &Node,
    source: &[u8],
    ctx: &ReductionContext,
) -> (u8, u8, u8) {
    let mut scan = scan_modifiers(node, source);
    let container = container_of(ctx);
    let in_interface = matches!(
        container,
        Container::InterfaceBody | Container::AnnotationBody
    );

    let mut implicit_mods = 0u8;
    let mut implicit_visibility: Option<u8> = None;
    // Does an access level apply to this declaration at all?
    let mut has_access = false;

    match kind {
        k if is_type_declaration(k) => {
            has_access = !matches!(container, Container::Local | Container::Other);
            if in_interface {
                implicit_visibility = Some(NodeAttr::VISIBILITY_PUBLIC);
                implicit_mods |= NodeAttr::MOD_STATIC;
            } else if matches!(container, Container::ClassBody | Container::EnumMembers)
                && k != "class_declaration"
            {
                // Member enums, records, interfaces and annotation types are static.
                implicit_mods |= NodeAttr::MOD_STATIC;
            }
            match k {
                "interface_declaration" | "annotation_type_declaration" => {
                    implicit_mods |= NodeAttr::MOD_ABSTRACT
                }
                "record_declaration" => implicit_mods |= NodeAttr::MOD_FINAL,
                "enum_declaration" if !enum_has_constant_bodies(node) => {
                    implicit_mods |= NodeAttr::MOD_FINAL
                }
                _ => {}
            }
        }
        "method_declaration" => {
            has_access = true;
            if has_spread_parameter(node) {
                scan.lang |= NodeAttr::LANG_JAVA_VARARGS;
            }
            if in_interface {
                if scan.visibility.is_none() {
                    implicit_visibility = Some(NodeAttr::VISIBILITY_PUBLIC);
                }
                let has_body = node.child_by_field_name("body").is_some();
                let is_static = scan.mods & NodeAttr::MOD_STATIC != 0;
                let is_default = scan.lang & NodeAttr::LANG_JAVA_DEFAULT != 0;
                let is_private = scan.visibility == Some(NodeAttr::VISIBILITY_PRIVATE);
                if !has_body && !is_static && !is_default && !is_private {
                    implicit_mods |= NodeAttr::MOD_ABSTRACT;
                }
            }
        }
        "annotation_type_element_declaration" => {
            has_access = true;
            implicit_visibility = Some(NodeAttr::VISIBILITY_PUBLIC);
            implicit_mods |= NodeAttr::MOD_ABSTRACT;
        }
        "constructor_declaration" | "compact_constructor_declaration" => {
            has_access = true;
            if has_spread_parameter(node) {
                scan.lang |= NodeAttr::LANG_JAVA_VARARGS;
            }
            if container == Container::EnumMembers {
                implicit_visibility = Some(NodeAttr::VISIBILITY_PRIVATE);
            }
        }
        "field_declaration" => {
            has_access = true;
        }
        "constant_declaration" => {
            has_access = true;
            implicit_visibility = Some(NodeAttr::VISIBILITY_PUBLIC);
            implicit_mods |= NodeAttr::MOD_STATIC | NodeAttr::MOD_FINAL;
        }
        "enum_constant" => {
            has_access = true;
            implicit_visibility = Some(NodeAttr::VISIBILITY_PUBLIC);
            implicit_mods |= NodeAttr::MOD_STATIC | NodeAttr::MOD_FINAL;
        }
        "formal_parameter" if is_record_component(ctx) => {
            // The component's implicit field is private final (JLS 8.10.3).
            has_access = true;
            implicit_visibility = Some(NodeAttr::VISIBILITY_PRIVATE);
            implicit_mods |= NodeAttr::MOD_FINAL;
        }
        "spread_parameter" => {
            scan.lang |= NodeAttr::LANG_JAVA_VARARGS;
        }
        _ => {}
    }

    if implicit_mods & !scan.mods != 0 {
        scan.lang |= NodeAttr::LANG_IMPLICIT_MODIFIERS;
    }
    let mods = scan.mods | implicit_mods;

    let visibility = match (scan.visibility, implicit_visibility) {
        (Some(v), _) => v,
        (None, Some(v)) => {
            scan.lang |= NodeAttr::LANG_IMPLICIT_VISIBILITY;
            v
        }
        (None, None) if has_access => NodeAttr::VISIBILITY_PACKAGE_PRIVATE,
        (None, None) => NodeAttr::VISIBILITY_NONE,
    };

    (visibility, mods, scan.lang)
}

fn operator_of(node: &Node, arity: OperatorArity) -> OperatorId {
    node.child_by_field_name("operator")
        .and_then(|op| OperatorId::from_symbol(op.kind(), arity))
        .unwrap_or(OperatorId::None)
}

/// `x++` / `--x`: (operator, is_postfix).
fn update_operator(node: &Node) -> (OperatorId, bool) {
    let kids = children_of(node);
    for (i, child) in kids.iter().enumerate() {
        let op = match child.kind() {
            "++" => OperatorId::Inc,
            "--" => OperatorId::Dec,
            _ => continue,
        };
        return (op, i > 0);
    }
    (OperatorId::None, false)
}

fn literal_category(kind: &str) -> u8 {
    match kind {
        "decimal_integer_literal"
        | "hex_integer_literal"
        | "octal_integer_literal"
        | "binary_integer_literal" => NodeAttr::LITERAL_INTEGER,
        "decimal_floating_point_literal" | "hex_floating_point_literal" => NodeAttr::LITERAL_FLOAT,
        "string_literal" => NodeAttr::LITERAL_STRING,
        "text_block" => NodeAttr::LITERAL_TEXT_BLOCK,
        "character_literal" => NodeAttr::LITERAL_CHAR,
        "true" | "false" => NodeAttr::LITERAL_BOOLEAN,
        "null_literal" => NodeAttr::LITERAL_NULL,
        "class_literal" => NodeAttr::LITERAL_CLASS,
        _ => 0,
    }
}

/// Flags of a switch group or rule.
fn switch_case_flags(kind: &str, node: &Node) -> u8 {
    let mut aux = 0u8;
    if kind == "switch_rule" {
        aux |= NodeAttr::AUX_CASE_ARROW;
    }
    for child in children_of(node) {
        match child.kind() {
            "->" => aux |= NodeAttr::AUX_CASE_ARROW,
            "switch_label" => {
                for part in children_of(&child) {
                    match part.kind() {
                        "default" => aux |= NodeAttr::AUX_CASE_DEFAULT,
                        "pattern" | "type_pattern" | "record_pattern" | "guard" => {
                            aux |= NodeAttr::AUX_CASE_PATTERN
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    aux
}

fn array_dimensions(node: &Node) -> u8 {
    let Some(dims) = node.child_by_field_name("dimensions") else {
        return 0;
    };
    let count = children_of(&dims)
        .iter()
        .filter(|c| c.kind() == "[")
        .count();
    u8::try_from(count).unwrap_or(u8::MAX)
}

fn wildcard_bound(node: &Node) -> u8 {
    for child in children_of(node) {
        match child.kind() {
            "extends" => return NodeAttr::WILDCARD_EXTENDS,
            "super" => return NodeAttr::WILDCARD_SUPER,
            _ => {}
        }
    }
    NodeAttr::WILDCARD_UNBOUNDED
}

impl ASTReductionAdapter for JavaASTReductionAdapter {
    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_java::LANGUAGE.into()
    }

    fn classify(&self, kind: &str, node: &Node, ctx: &ReductionContext) -> ReductionDecision {
        use ASTNodeType::*;
        use ReductionDecision::*;

        let parent = ctx.parent_kind();

        match kind {
            // ── Compilation unit & module system ──
            "program" => Keep(NN_MODULE),
            "package_declaration" => Keep(NN_PACKAGE_DECL),
            "import_declaration" => Keep(NN_IMPORT_DECL),
            "module_declaration" => Keep(NN_JAVA_MODULE_DECL),
            "module_directive"
            | "requires_module_directive"
            | "exports_module_directive"
            | "opens_module_directive"
            | "uses_module_directive"
            | "provides_module_directive" => Keep(NN_JAVA_MODULE_DIRECTIVE),
            "requires_modifier" => Drop,

            // ── Type declarations ──
            "class_declaration" => Keep(NN_CLASS_DECL),
            "interface_declaration" => Keep(NN_INTERFACE_DECL),
            "enum_declaration" => Keep(NN_ENUM_DECL),
            "record_declaration" => Keep(NN_RECORD_DECL),
            "annotation_type_declaration" => Keep(NN_ANNOTATION_DECL),
            "class_body" => match parent {
                Some("object_creation_expression" | "enum_constant") => Keep(NN_ANON_CLASS_BODY),
                _ => Eliminate,
            },
            "enum_constant" => Keep(NN_ENUM_CONSTANT),
            "permits" => Keep(NN_PERMITS_CLAUSE),

            // ── Members ──
            "method_declaration" | "annotation_type_element_declaration" => Keep(NN_METHOD_DECL),
            "constructor_declaration" | "compact_constructor_declaration" => {
                Keep(NN_CONSTRUCTOR_DECL)
            }
            "constructor_body" => Keep(NN_BLOCK),
            "explicit_constructor_invocation" => Keep(NN_CALL_EXPR),
            "field_declaration" | "constant_declaration" => Keep(NN_FIELD_DECL),
            "static_initializer" => Keep(NN_JAVA_STATIC_INIT),
            "block" if matches!(parent, Some("class_body" | "enum_body_declarations")) => {
                Wrap(NN_JAVA_INSTANCE_INIT, NN_BLOCK)
            }
            "throws" => Keep(NN_THROWS_CLAUSE),
            "formal_parameter" | "spread_parameter" | "catch_formal_parameter" => {
                Keep(NN_PARAM_DECL)
            }
            // A receiver parameter (`Foo this`) is not a formal parameter (JLS 8.4).
            "receiver_parameter" => Drop,
            "local_variable_declaration" | "variable_declarator" => Keep(NN_LOCAL_VAR_DECL),
            // A resource that declares a variable; a bare variable or field reference
            // resource is eliminated to that reference.
            "resource" => {
                if node.child_by_field_name("name").is_some() {
                    Keep(NN_LOCAL_VAR_DECL)
                } else {
                    Eliminate
                }
            }

            // ── Statements ──
            "block" => Keep(NN_BLOCK),
            "if_statement" => Keep(NN_IF_STMT),
            "for_statement" => Keep(NN_FOR_STMT),
            "enhanced_for_statement" => Keep(NN_ENHANCED_FOR),
            "while_statement" => Keep(NN_WHILE_STMT),
            "do_statement" => Keep(NN_DO_WHILE_STMT),
            "switch_statement" => Keep(NN_SWITCH_STMT),
            "switch_expression" => {
                if is_statement_position(ctx) {
                    Keep(NN_SWITCH_STMT)
                } else {
                    Keep(NN_SWITCH_EXPR)
                }
            }
            "switch_block_statement_group" | "switch_rule" => Keep(NN_SWITCH_CASE),
            "try_statement" | "try_with_resources_statement" => Keep(NN_TRY_STMT),
            "catch_clause" => Keep(NN_CATCH_CLAUSE),
            "finally_clause" => Keep(NN_FINALLY_CLAUSE),
            "return_statement" => Keep(NN_RETURN_STMT),
            "throw_statement" => Keep(NN_THROW_STMT),
            "break_statement" => Keep(NN_BREAK_STMT),
            "continue_statement" => Keep(NN_CONTINUE_STMT),
            "yield_statement" => Keep(NN_YIELD_STMT),
            "expression_statement" => Keep(NN_EXPR_STMT),
            "assert_statement" => Keep(NN_JAVA_ASSERT_STMT),
            "labeled_statement" => Keep(NN_JAVA_LABELED_STMT),
            "synchronized_statement" => Keep(NN_JAVA_SYNCHRONIZED),

            // ── Expressions ──
            "assignment_expression" => Keep(NN_ASSIGN_EXPR),
            "binary_expression" => Keep(NN_BINARY_EXPR),
            "unary_expression" | "update_expression" => Keep(NN_UNARY_EXPR),
            "ternary_expression" => Keep(NN_TERNARY_EXPR),
            "method_invocation" | "explicit_generic_invocation" => Keep(NN_CALL_EXPR),
            "object_creation_expression" => Keep(NN_NEW_EXPR),
            "field_access" => Keep(NN_FIELD_ACCESS),
            "array_access" => Keep(NN_ARRAY_ACCESS),
            "cast_expression" => Keep(NN_CAST_EXPR),
            "instanceof_expression" => Keep(NN_INSTANCEOF_EXPR),
            "lambda_expression" => Keep(NN_LAMBDA_EXPR),
            "method_reference" => Keep(NN_METHOD_REF),
            "array_creation_expression" => Keep(NN_ARRAY_CREATE),
            "array_initializer" | "element_value_array_initializer" => Keep(NN_ARRAY_INIT),
            "class_literal" => Keep(NN_LITERAL),
            "type_pattern" | "record_pattern" => Keep(NN_PATTERN_MATCH),

            // ── Annotations ──
            "marker_annotation" | "annotation" => Keep(NN_ANNOTATION_USE),

            // ── Types ──
            "type_parameter" => Keep(NN_TYPE_PARAM),
            "generic_type" | "array_type" | "wildcard" => Keep(NN_TYPE_REF),
            "scoped_type_identifier" => match parent {
                Some("scoped_type_identifier") => Eliminate,
                _ => Keep(NN_TYPE_REF),
            },
            // Segments of a qualified type and the name of a type parameter are covered
            // by the enclosing node's token range / name token.
            "type_identifier" => match parent {
                Some("scoped_type_identifier" | "type_parameter") => Eliminate,
                _ => Keep(NN_TYPE_REF),
            },
            "void_type" | "integral_type" | "floating_point_type" | "boolean_type" => {
                Keep(NN_TYPE_REF)
            }

            // ── Names ──
            "scoped_identifier" => match parent {
                Some("scoped_identifier") => Eliminate,
                _ => Keep(NN_IDENTIFIER_EXPR),
            },
            "identifier" => {
                if is_lambda_parameter(ctx) {
                    // Implicitly typed lambda parameter: a parameter declaration holding
                    // its name, like an explicit `formal_parameter`.
                    Wrap(NN_PARAM_DECL, NN_IDENTIFIER_EXPR)
                } else if parent == Some("scoped_identifier") {
                    Eliminate
                } else {
                    Keep(NN_IDENTIFIER_EXPR)
                }
            }

            // ── Literals ──
            "decimal_integer_literal"
            | "hex_integer_literal"
            | "octal_integer_literal"
            | "binary_integer_literal"
            | "decimal_floating_point_literal"
            | "hex_floating_point_literal"
            | "string_literal"
            | "text_block"
            | "character_literal"
            | "true"
            | "false"
            | "null_literal" => Keep(NN_LITERAL),
            "super" => Keep(NN_SUPER_EXPR),
            "this" => Keep(NN_THIS_EXPR),

            // ── Explicit trivia ──
            // Literal internals, the `*` of an on-demand import (recorded in the import's
            // aux flags), array dimension brackets (recorded in the type's lang byte),
            // and comments.
            "string_fragment"
            | "escape_sequence"
            | "multiline_string_fragment"
            | "asterisk"
            | "dimensions"
            | "line_comment"
            | "block_comment"
            | "comment" => Drop,

            // Grouping and disambiguation wrappers (parenthesized_expression, modifiers,
            // formal_parameters, argument_list, class/interface/enum/annotation bodies,
            // superclass, super_interfaces, extends_interfaces, type_list,
            // type_arguments, type_parameters, type_bound, annotated_type, switch_block,
            // switch_label, guard, catch_type, resource_specification,
            // inferred_parameters, annotation_argument_list, element_value_pair,
            // dimensions_expr, record_pattern_body, module_body, ...) and ERROR regions
            // are eliminated so that their contents survive.
            _ => Eliminate,
        }
    }

    fn encode_attrs(&self, kind: &str, node: &Node, source: &[u8], ctx: &ReductionContext) -> u32 {
        let mut vis = NodeAttr::VISIBILITY_NONE;
        let mut mods = 0u8;
        let mut op_id = OperatorId::None;
        let mut aux = 0u8;
        let mut lang = 0u8;

        match kind {
            k if is_type_declaration(k)
                || matches!(
                    k,
                    "method_declaration"
                        | "annotation_type_element_declaration"
                        | "constructor_declaration"
                        | "compact_constructor_declaration"
                        | "field_declaration"
                        | "constant_declaration"
                        | "enum_constant"
                        | "local_variable_declaration"
                        | "formal_parameter"
                        | "spread_parameter"
                        | "catch_formal_parameter"
                ) =>
            {
                (vis, mods, lang) = declaration_attrs(k, node, source, ctx);
                match k {
                    "method_declaration"
                    | "constructor_declaration"
                    | "compact_constructor_declaration" => {
                        if node.child_by_field_name("body").is_some() {
                            aux |= NodeAttr::AUX_METHOD_HAS_BODY;
                        }
                        if k == "compact_constructor_declaration" {
                            aux |= NodeAttr::AUX_CTOR_COMPACT;
                        }
                    }
                    "annotation_type_element_declaration" => {
                        aux |= NodeAttr::AUX_METHOD_ANNOTATION_ELEMENT;
                    }
                    "constant_declaration" => aux |= NodeAttr::AUX_FIELD_CONSTANT,
                    "formal_parameter" if is_record_component(ctx) => {
                        aux |= NodeAttr::AUX_PARAM_RECORD_COMPONENT;
                    }
                    "catch_formal_parameter" => aux |= NodeAttr::AUX_PARAM_CATCH,
                    _ => {}
                }
            }
            "identifier" if is_lambda_parameter(ctx) => {
                // The outer `NN_PARAM_DECL` of an implicitly typed lambda parameter.
                aux |= NodeAttr::AUX_PARAM_LAMBDA_INFERRED;
            }
            "binary_expression" => op_id = operator_of(node, OperatorArity::Binary),
            "assignment_expression" => op_id = operator_of(node, OperatorArity::Assignment),
            "unary_expression" => op_id = operator_of(node, OperatorArity::Unary),
            "update_expression" => {
                let (op, postfix) = update_operator(node);
                op_id = op;
                if postfix {
                    aux |= NodeAttr::AUX_UNARY_POSTFIX;
                }
            }
            "import_declaration" => {
                for child in children_of(node) {
                    match child.kind() {
                        "static" => aux |= NodeAttr::AUX_IMPORT_STATIC,
                        "asterisk" => aux |= NodeAttr::AUX_IMPORT_ON_DEMAND,
                        _ => {}
                    }
                }
            }
            "scoped_identifier" => aux |= NodeAttr::AUX_IDENT_QUALIFIED,
            "generic_type" => aux |= NodeAttr::AUX_TYPE_GENERIC,
            "scoped_type_identifier" => aux |= NodeAttr::AUX_TYPE_QUALIFIED,
            "array_type" => {
                aux |= NodeAttr::AUX_TYPE_ARRAY;
                lang = array_dimensions(node);
            }
            "wildcard" => {
                aux |= NodeAttr::AUX_TYPE_WILDCARD;
                lang = wildcard_bound(node);
            }
            "switch_block_statement_group" | "switch_rule" => aux = switch_case_flags(kind, node),
            "switch_expression" | "switch_statement" => {
                let arrow = node
                    .child_by_field_name("body")
                    .is_some_and(|b| children_of(&b).iter().any(|c| c.kind() == "switch_rule"));
                if arrow {
                    aux |= NodeAttr::AUX_SWITCH_ARROW_FORM;
                }
            }
            "try_with_resources_statement" => aux |= NodeAttr::AUX_TRY_WITH_RESOURCES,
            "explicit_constructor_invocation" => {
                aux |= NodeAttr::AUX_CALL_EXPLICIT_CTOR;
                if node
                    .child_by_field_name("constructor")
                    .is_some_and(|c| c.kind() == "super")
                {
                    aux |= NodeAttr::AUX_CALL_SUPER_TARGET;
                }
                if node.child_by_field_name("object").is_some() {
                    aux |= NodeAttr::AUX_CALL_QUALIFIED;
                }
            }
            "method_invocation" => {
                if node.child_by_field_name("object").is_some() {
                    aux |= NodeAttr::AUX_CALL_QUALIFIED;
                }
            }
            "object_creation_expression" => {
                let kids = children_of(node);
                if kids.iter().any(|c| c.kind() == "class_body") {
                    aux |= NodeAttr::AUX_NEW_ANON_BODY;
                }
                if kids.first().is_some_and(|c| c.kind() != "new") {
                    aux |= NodeAttr::AUX_NEW_QUALIFIED;
                }
            }
            "method_reference" => {
                if children_of(node).last().is_some_and(|c| c.kind() == "new") {
                    aux |= NodeAttr::AUX_METHOD_REF_CONSTRUCTOR;
                }
            }
            "lambda_expression" => {
                if node
                    .child_by_field_name("body")
                    .is_some_and(|b| b.kind() != "block")
                {
                    aux |= NodeAttr::AUX_LAMBDA_EXPRESSION_BODY;
                }
            }
            "instanceof_expression" => {
                let binds = node.child_by_field_name("name").is_some()
                    || children_of(node)
                        .iter()
                        .any(|c| matches!(c.kind(), "type_pattern" | "record_pattern"));
                if binds {
                    aux |= NodeAttr::AUX_INSTANCEOF_PATTERN;
                }
            }
            _ => {
                lang = literal_category(kind);
            }
        }

        NodeAttr::pack(vis, mods, op_id, aux, lang)
    }

    fn name_node<'tree>(&self, kind: &str, node: &Node<'tree>) -> Option<Node<'tree>> {
        match kind {
            "class_declaration"
            | "interface_declaration"
            | "enum_declaration"
            | "record_declaration"
            | "annotation_type_declaration"
            | "method_declaration"
            | "constructor_declaration"
            | "compact_constructor_declaration"
            | "annotation_type_element_declaration"
            | "enum_constant"
            | "variable_declarator"
            | "formal_parameter"
            | "catch_formal_parameter"
            | "enhanced_for_statement"
            | "resource"
            | "method_invocation" => node.child_by_field_name("name"),
            "field_access" => node.child_by_field_name("field"),
            "explicit_constructor_invocation" => node.child_by_field_name("constructor"),
            "spread_parameter" => child_of_kind(node, "variable_declarator")
                .and_then(|d| d.child_by_field_name("name")),
            "type_parameter" => children_of(node)
                .into_iter()
                .find(|c| matches!(c.kind(), "type_identifier" | "identifier")),
            "marker_annotation" | "annotation" => {
                let name = node.child_by_field_name("name")?;
                if name.kind() == "scoped_identifier" {
                    name.child_by_field_name("name")
                } else {
                    Some(name)
                }
            }
            "labeled_statement" | "break_statement" | "continue_statement" => {
                child_of_kind(node, "identifier")
            }
            "identifier" => Some(*node),
            _ => None,
        }
    }
}
