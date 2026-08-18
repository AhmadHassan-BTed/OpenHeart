//! Generic AST Reduction Adapter implementation for universal language support.

use super::{ASTReductionAdapter, ReductionDecision};
use crate::core::types::ast::ASTNodeType;
use tree_sitter::{Language, Node};

pub struct GenericASTReductionAdapter {
    ts_lang: Language,
}

impl GenericASTReductionAdapter {
    pub fn new(ts_lang: Language) -> Self {
        Self { ts_lang }
    }
}

impl ASTReductionAdapter for GenericASTReductionAdapter {
    fn ts_language(&self) -> Language {
        self.ts_lang
    }

    fn classify(&self, kind: &str, _node: &Node, depth: usize) -> ReductionDecision {
        use ASTNodeType::*;
        use ReductionDecision::*;

        if kind.contains("class") || kind.contains("export") {
            println!("[TS-KIND] kind={}", kind);
        }

        match kind {
            // Declarations
            "program" | "module" | "translation_unit" | "source_file" => Keep(NN_MODULE),
            "class_declaration"
            | "class_definition"
            | "class"
            | "class_specifier"
            | "object_declaration"
            | "struct_item"
            | "struct_specifier"
            | "union_item"
            | "type_alias_declaration" => Keep(NN_CLASS_DECL),
            "interface_declaration" | "interface" | "trait_item" | "protocol_declaration" => {
                Keep(NN_INTERFACE_DECL)
            }
            "enum_declaration" | "enum_specifier" | "enum_item" => Keep(NN_ENUM_DECL),
            "method_declaration"
            | "method_definition"
            | "function_declaration"
            | "function_definition"
            | "function"
            | "arrow_function"
            | "function_item"
            | "function_component" => Keep(NN_METHOD_DECL),
            "constructor_declaration" | "constructor" => Keep(NN_CONSTRUCTOR_DECL),
            "field_declaration"
            | "field_definition"
            | "property_definition"
            | "public_field_definition" => Keep(NN_FIELD_DECL),
            "formal_parameter" | "parameter" | "required_parameter" => Keep(NN_PARAM_DECL),
            "local_variable_declaration"
            | "lexical_declaration"
            | "variable_declaration"
            | "variable_declarator"
            | "let_declaration" => Keep(NN_LOCAL_VAR_DECL),

            // Statements
            "statement_block" | "block" | "compound_statement" => Keep(NN_BLOCK),
            "if_statement" => Keep(NN_IF_STMT),
            "for_statement" | "for_in_statement" | "for_of_statement" => Keep(NN_FOR_STMT),
            "while_statement" => Keep(NN_WHILE_STMT),
            "do_statement" => Keep(NN_DO_WHILE_STMT),
            "switch_statement" => Keep(NN_SWITCH_STMT),
            "switch_case" | "case_clause" => Keep(NN_SWITCH_CASE),
            "try_statement" => Keep(NN_TRY_STMT),
            "catch_clause" => Keep(NN_CATCH_CLAUSE),
            "finally_clause" => Keep(NN_FINALLY_CLAUSE),
            "return_statement" => Keep(NN_RETURN_STMT),
            "throw_statement" => Keep(NN_THROW_STMT),
            "break_statement" => Keep(NN_BREAK_STMT),
            "continue_statement" => Keep(NN_CONTINUE_STMT),
            "expression_statement" => Keep(NN_EXPR_STMT),

            // Expressions
            "assignment_expression" => Keep(NN_ASSIGN_EXPR),
            "binary_expression" => Keep(NN_BINARY_EXPR),
            "unary_expression" | "update_expression" => Keep(NN_UNARY_EXPR),
            "ternary_expression" | "conditional_expression" => Keep(NN_TERNARY_EXPR),
            "call_expression" | "method_invocation" => Keep(NN_CALL_EXPR),
            "new_expression" | "object_creation_expression" => Keep(NN_NEW_EXPR),
            "member_expression" | "field_access" => Keep(NN_FIELD_ACCESS),
            "subscript_expression" | "array_access" => Keep(NN_ARRAY_ACCESS),

            // Terminals & Literals
            "number"
            | "integer"
            | "decimal_integer_literal"
            | "string"
            | "string_literal"
            | "template_string"
            | "true"
            | "false"
            | "null"
            | "undefined" => Keep(NN_LITERAL),
            "this" | "super" => Keep(NN_THIS_EXPR),

            _ => {
                if kind.contains("declaration") || kind.contains("definition") {
                    Keep(NN_LOCAL_VAR_DECL)
                } else if kind.contains("statement") {
                    Keep(NN_EXPR_STMT)
                } else if kind.contains("expression") {
                    Keep(NN_EXPR_STMT)
                } else {
                    Eliminate
                }
            }
        }
    }

    fn encode_attrs(&self, _kind: &str, _node: &Node, _source: &[u8]) -> u32 {
        0
    }
}
