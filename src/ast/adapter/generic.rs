//! Generic AST Reduction Adapter for C-family grammars without a dedicated adapter.
//!
//! Registered for JavaScript (whose grammar is linked). It maps common declaration,
//! statement and expression kinds by name; any other kind is eliminated so that its
//! children survive, and comments are dropped. Operators are encoded from the operator
//! token's kind.

use super::{child_of_kind, ASTReductionAdapter, ReductionContext, ReductionDecision};
use crate::core::types::ast::{ASTNodeType, NodeAttr, OperatorArity, OperatorId};
use tree_sitter::{Language, Node};

pub struct GenericASTReductionAdapter {
    ts_lang: Language,
}

impl GenericASTReductionAdapter {
    pub fn new(ts_lang: Language) -> Self {
        Self { ts_lang }
    }
}

fn operator_of(node: &Node, arity: OperatorArity) -> OperatorId {
    node.child_by_field_name("operator")
        .and_then(|op| OperatorId::from_symbol(op.kind(), arity))
        .unwrap_or(OperatorId::None)
}

impl ASTReductionAdapter for GenericASTReductionAdapter {
    fn ts_language(&self) -> Language {
        self.ts_lang
    }

    fn classify(&self, kind: &str, _node: &Node, _ctx: &ReductionContext) -> ReductionDecision {
        use ASTNodeType::*;
        use ReductionDecision::*;

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
            | "generator_function_declaration"
            | "function_definition"
            | "function"
            | "function_expression"
            | "generator_function"
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
            "import_statement" | "import_declaration" => Keep(NN_IMPORT_DECL),

            // Statements
            "statement_block" | "block" | "compound_statement" => Keep(NN_BLOCK),
            "if_statement" => Keep(NN_IF_STMT),
            "for_statement" | "for_in_statement" | "for_of_statement" => Keep(NN_FOR_STMT),
            "while_statement" => Keep(NN_WHILE_STMT),
            "do_statement" => Keep(NN_DO_WHILE_STMT),
            "switch_statement" => Keep(NN_SWITCH_STMT),
            "switch_case" | "switch_default" | "case_clause" => Keep(NN_SWITCH_CASE),
            "try_statement" => Keep(NN_TRY_STMT),
            "catch_clause" => Keep(NN_CATCH_CLAUSE),
            "finally_clause" => Keep(NN_FINALLY_CLAUSE),
            "return_statement" => Keep(NN_RETURN_STMT),
            "throw_statement" => Keep(NN_THROW_STMT),
            "break_statement" => Keep(NN_BREAK_STMT),
            "continue_statement" => Keep(NN_CONTINUE_STMT),
            "expression_statement" => Keep(NN_EXPR_STMT),
            "labeled_statement" => Keep(NN_JAVA_LABELED_STMT),

            // Expressions
            "assignment_expression" | "augmented_assignment_expression" => Keep(NN_ASSIGN_EXPR),
            "binary_expression" => Keep(NN_BINARY_EXPR),
            "unary_expression" | "update_expression" => Keep(NN_UNARY_EXPR),
            "ternary_expression" | "conditional_expression" => Keep(NN_TERNARY_EXPR),
            "call_expression" | "method_invocation" => Keep(NN_CALL_EXPR),
            "new_expression" | "object_creation_expression" => Keep(NN_NEW_EXPR),
            "member_expression" | "field_access" => Keep(NN_FIELD_ACCESS),
            "subscript_expression" | "array_access" => Keep(NN_ARRAY_ACCESS),
            "array" => Keep(NN_ARRAY_INIT),
            "decorator" => Keep(NN_ANNOTATION_USE),

            // Terminals & Literals
            "identifier"
            | "property_identifier"
            | "shorthand_property_identifier"
            | "private_property_identifier" => Keep(NN_IDENTIFIER_EXPR),
            "number"
            | "integer"
            | "decimal_integer_literal"
            | "string"
            | "string_literal"
            | "template_string"
            | "regex"
            | "true"
            | "false"
            | "null"
            | "undefined" => Keep(NN_LITERAL),
            "this" => Keep(NN_THIS_EXPR),
            "super" => Keep(NN_SUPER_EXPR),

            // Explicit trivia
            "comment" | "line_comment" | "block_comment" | "hash_bang_line" | "html_comment"
            | "string_fragment" | "escape_sequence" | "regex_pattern" | "regex_flags" => Drop,

            // Wrappers and unknown kinds keep their contents.
            _ => Eliminate,
        }
    }

    fn encode_attrs(&self, kind: &str, node: &Node, _source: &[u8], _ctx: &ReductionContext) -> u32 {
        let mut aux = 0u8;
        let op_id = match kind {
            "binary_expression" => operator_of(node, OperatorArity::Binary),
            "augmented_assignment_expression" => operator_of(node, OperatorArity::Assignment),
            "assignment_expression" => OperatorId::Assign,
            "unary_expression" => operator_of(node, OperatorArity::Unary),
            "update_expression" => {
                // `x++` puts the argument first.
                if node
                    .child_by_field_name("argument")
                    .zip(node.child_by_field_name("operator"))
                    .is_some_and(|(arg, op)| arg.start_byte() < op.start_byte())
                {
                    aux |= NodeAttr::AUX_UNARY_POSTFIX;
                }
                operator_of(node, OperatorArity::Unary)
            }
            _ => OperatorId::None,
        };
        NodeAttr::pack(NodeAttr::VISIBILITY_NONE, 0, op_id, aux, 0)
    }

    fn name_node<'tree>(&self, kind: &str, node: &Node<'tree>) -> Option<Node<'tree>> {
        match kind {
            "class_declaration"
            | "function_declaration"
            | "generator_function_declaration"
            | "method_definition"
            | "variable_declarator"
            | "field_definition" => node.child_by_field_name("name"),
            "call_expression" => {
                let callee = node.child_by_field_name("function")?;
                match callee.kind() {
                    "member_expression" => callee.child_by_field_name("property"),
                    "identifier" => Some(callee),
                    _ => None,
                }
            }
            "member_expression" => node.child_by_field_name("property"),
            "identifier" | "property_identifier" => Some(*node),
            "labeled_statement" => child_of_kind(node, "statement_identifier"),
            _ => None,
        }
    }
}
