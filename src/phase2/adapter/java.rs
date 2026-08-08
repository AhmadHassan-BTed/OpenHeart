//! Java AST Reduction Adapter implementation.

use super::{ASTReductionAdapter, ReductionDecision};
use crate::core::types::ast::{ASTNodeType, NodeAttr, OperatorId};
use tree_sitter::Node;

pub struct JavaASTReductionAdapter;

impl JavaASTReductionAdapter {
    pub fn new() -> Self {
        Self
    }
}

impl ASTReductionAdapter for JavaASTReductionAdapter {
    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_java::language().into()
    }

    fn classify(&self, kind: &str, _node: &Node, _depth: usize) -> ReductionDecision {
        use ASTNodeType::*;
        use ReductionDecision::*;

        match kind {
            // Declarations
            "program" => Keep(NN_MODULE),
            "class_declaration" => Keep(NN_CLASS_DECL),
            "interface_declaration" => Keep(NN_INTERFACE_DECL),
            "enum_declaration" => Keep(NN_ENUM_DECL),
            "record_declaration" => Keep(NN_RECORD_DECL),
            "annotation_type_declaration" => Keep(NN_ANNOTATION_DECL),
            "method_declaration" => Keep(NN_METHOD_DECL),
            "constructor_declaration" => Keep(NN_CONSTRUCTOR_DECL),
            "field_declaration" => Keep(NN_FIELD_DECL),
            "formal_parameter" | "spread_parameter" => Keep(NN_PARAM_DECL),
            "local_variable_declaration" | "variable_declarator" => Keep(NN_LOCAL_VAR_DECL),

            // Statements
            "block" => Keep(NN_BLOCK),
            "if_statement" => Keep(NN_IF_STMT),
            "for_statement" => Keep(NN_FOR_STMT),
            "enhanced_for_statement" => Keep(NN_ENHANCED_FOR),
            "while_statement" => Keep(NN_WHILE_STMT),
            "do_statement" => Keep(NN_DO_WHILE_STMT),
            "switch_statement" => Keep(NN_SWITCH_STMT),
            "switch_expression" => Keep(NN_SWITCH_EXPR),
            "switch_block_statement_group" => Keep(NN_SWITCH_CASE),
            "try_statement" => Keep(NN_TRY_STMT),
            "catch_clause" => Keep(NN_CATCH_CLAUSE),
            "finally_clause" => Keep(NN_FINALLY_CLAUSE),
            "return_statement" => Keep(NN_RETURN_STMT),
            "throw_statement" => Keep(NN_THROW_STMT),
            "break_statement" => Keep(NN_BREAK_STMT),
            "continue_statement" => Keep(NN_CONTINUE_STMT),
            "expression_statement" => Keep(NN_EXPR_STMT),
            "assert_statement" => Keep(NN_JAVA_ASSERT_STMT),
            "labeled_statement" => Keep(NN_JAVA_LABELED_STMT),
            "synchronized_statement" => Keep(NN_JAVA_SYNCHRONIZED),
            "static_initializer" => Keep(NN_JAVA_STATIC_INIT),

            // Expressions
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
            "array_initializer" => Keep(NN_ARRAY_INIT),
            "marker_annotation" | "annotation" => Keep(NN_ANNOTATION_USE),
            "type_parameters" => Keep(NN_TYPE_PARAM),

            // Leaf Tokens
            "identifier" => Keep(NN_IDENTIFIER_EXPR),
            "type_identifier"
            | "void_type"
            | "integral_type"
            | "floating_point_type"
            | "boolean_type" => Keep(NN_TYPE_REF),
            "decimal_integer_literal"
            | "hex_integer_literal"
            | "binary_integer_literal"
            | "decimal_floating_point_literal"
            | "string_literal"
            | "text_block"
            | "character_literal"
            | "true"
            | "false"
            | "null_literal" => Keep(NN_LITERAL),
            "super" => Keep(NN_SUPER_EXPR),
            "this" => Keep(NN_THIS_EXPR),

            // Eliminate Grouping & Disambiguation Wrappers
            "parenthesized_expression"
            | "modifiers"
            | "formal_parameters"
            | "argument_list"
            | "class_body"
            | "interface_body"
            | "enum_body"
            | "enum_body_declarations"
            | "block_statements"
            | "superclass"
            | "super_interfaces"
            | "type_bound" => Eliminate,

            // Pure Punctuation & Structural Delimiters
            "{" | "}" | "(" | ")" | "[" | "]" | ";" | "," | "." | ":" | "::" | "->" | "@" | "<"
            | ">" | "?" | "..." | "switch" | "case" | "default" | "catch" | "finally"
            | "throws" | "extends" | "implements" | "permits" | "line_comment"
            | "block_comment" => Drop,

            _ => Drop,
        }
    }

    fn encode_attrs(&self, kind: &str, node: &Node, source: &[u8]) -> u32 {
        let mut vis = NodeAttr::VISIBILITY_NONE;
        let mut mods = 0u8;
        let mut op_id = OperatorId::None;

        // Extract modifiers if child exists
        if let Some(modifiers_node) = node.child_by_field_name("modifiers") {
            let text = modifiers_node.utf8_text(source).unwrap_or("");
            if text.contains("public") {
                vis = NodeAttr::VISIBILITY_PUBLIC;
            } else if text.contains("private") {
                vis = NodeAttr::VISIBILITY_PRIVATE;
            } else if text.contains("protected") {
                vis = NodeAttr::VISIBILITY_PROTECTED;
            }

            if text.contains("static") {
                mods |= NodeAttr::MOD_STATIC;
            }
            if text.contains("final") {
                mods |= NodeAttr::MOD_FINAL;
            }
            if text.contains("abstract") {
                mods |= NodeAttr::MOD_ABSTRACT;
            }
        }

        if kind == "binary_expression" || kind == "assignment_expression" {
            if let Some(op_node) = node.child_by_field_name("operator") {
                let op_text = op_node.utf8_text(source).unwrap_or("");
                op_id = match op_text {
                    "=" => OperatorId::Assign,
                    "+" => OperatorId::Add,
                    "-" => OperatorId::Sub,
                    "*" => OperatorId::Mul,
                    "/" => OperatorId::Div,
                    "==" => OperatorId::Eq,
                    "!=" => OperatorId::NotEq,
                    "&&" => OperatorId::LogicalAnd,
                    "||" => OperatorId::LogicalOr,
                    _ => OperatorId::None,
                };
            }
        }

        NodeAttr::pack(vis, mods, op_id, 0, 0)
    }
}
