//! Phase 1 adapter for tree-sitter-cpp.
//!
//! tree-sitter-cpp extends tree-sitter-c: the C tables of `lang::c::ingest` classify
//! every inherited kind, and this module adds the C++ kinds and tokens of
//! tree-sitter-cpp 0.20.5 (`src/node-types.json`): C++ keywords, the alternative
//! operator spellings (`and`, `not_eq`, ...), `<=>`, `.*` / `->*`, raw string literals
//! and user-defined literals. Interior C++ rules are listed explicitly as `Unknown`.

use tree_sitter::Language;

use crate::core::types::token::{LangId, TokenType};
use crate::ingestion::adapter::LanguageAdapter;
use crate::lang::c::ingest::{classify_c_anonymous, classify_c_named, refine_c_family_token};
use crate::lang::c::Dialect;

/// Phase 1 adapter for C++ sources (tree-sitter-cpp).
pub struct CppLanguageAdapter;

impl CppLanguageAdapter {
    /// Creates the adapter.
    pub fn new() -> Self {
        Self
    }
}

impl Default for CppLanguageAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl LanguageAdapter for CppLanguageAdapter {
    fn language_id(&self) -> LangId {
        LangId::Cpp
    }

    fn file_extensions(&self) -> &[&str] {
        &[
            "cpp", "cc", "cxx", "c++", "hpp", "hh", "hxx", "h++", "ipp", "tpp", "inl", "ixx",
            "cppm",
        ]
    }

    fn ts_language(&self) -> Language {
        tree_sitter_cpp::language()
    }

    fn map_node_type(&self, kind: &str) -> TokenType {
        match classify_cpp_named(kind) {
            TokenType::Unknown => classify_cpp_anonymous(kind),
            t => t,
        }
    }

    fn include_anonymous(&self, kind: &str) -> bool {
        classify_cpp_anonymous(kind) != TokenType::Unknown
    }

    fn classify_leaf(&self, kind: &str, is_named: bool) -> TokenType {
        if is_named {
            classify_cpp_named(kind)
        } else {
            classify_cpp_anonymous(kind)
        }
    }

    fn is_atomic(&self, kind: &str) -> bool {
        matches!(
            kind,
            "string_literal" | "char_literal" | "raw_string_literal" | "user_defined_literal"
        )
    }

    fn is_annotation_kind(&self, kind: &str) -> bool {
        // `[[nodiscard]]`, `[[gnu::always_inline]]`.
        kind == "attribute"
    }

    fn refine_token_type(&self, token_type: TokenType, text: &[u8]) -> TokenType {
        refine_c_family_token(token_type, text, Dialect::Cpp)
    }
}

/// Token type of a named tree-sitter-cpp kind. Interior rules map to `Unknown`.
pub fn classify_cpp_named(kind: &str) -> TokenType {
    match kind {
        "namespace_identifier" => TokenType::Identifier,
        "raw_string_literal" | "raw_string_content" | "raw_string_delimiter" => {
            TokenType::StringLiteral
        }
        // The literal part decides the category; refined by text.
        "user_defined_literal" => TokenType::IntegerLiteral,
        // Suffix of a user-defined literal emitted on its own (not atomic).
        "literal_suffix" => TokenType::Identifier,
        "this" | "auto" | "virtual" | "access_specifier" | "virtual_specifier" => {
            TokenType::Keyword
        }
        // Interior C++ rules.
        "abstract_reference_declarator"
        | "alias_declaration"
        | "alignas_specifier"
        | "base_class_clause"
        | "catch_clause"
        | "class_specifier"
        | "co_await_expression"
        | "co_return_statement"
        | "co_yield_statement"
        | "compound_requirement"
        | "concept_definition"
        | "condition_clause"
        | "constraint_conjunction"
        | "constraint_disjunction"
        | "decltype"
        | "default_method_clause"
        | "delete_expression"
        | "delete_method_clause"
        | "dependent_name"
        | "dependent_type"
        | "destructor_name"
        | "explicit_function_specifier"
        | "field_initializer"
        | "field_initializer_list"
        | "fold_expression"
        | "for_range_loop"
        | "friend_declaration"
        | "init_statement"
        | "lambda_capture_specifier"
        | "lambda_default_capture"
        | "lambda_expression"
        | "namespace_alias_definition"
        | "namespace_definition"
        | "nested_namespace_specifier"
        | "new_declarator"
        | "new_expression"
        | "noexcept"
        | "operator_cast"
        | "operator_name"
        | "optional_parameter_declaration"
        | "optional_type_parameter_declaration"
        | "parameter_pack_expansion"
        | "placeholder_type_specifier"
        | "pointer_type_declarator"
        | "qualified_identifier"
        | "ref_qualifier"
        | "reference_declarator"
        | "requirement_seq"
        | "requires_clause"
        | "requires_expression"
        | "simple_requirement"
        | "static_assert_declaration"
        | "structured_binding_declarator"
        | "subscript_argument_list"
        | "template_argument_list"
        | "template_declaration"
        | "template_function"
        | "template_instantiation"
        | "template_method"
        | "template_parameter_list"
        | "template_template_parameter_declaration"
        | "template_type"
        | "throw_specifier"
        | "throw_statement"
        | "trailing_return_type"
        | "try_statement"
        | "type_parameter_declaration"
        | "type_requirement"
        | "using_declaration"
        | "variadic_declarator"
        | "variadic_parameter_declaration"
        | "variadic_type_parameter_declaration" => TokenType::Unknown,
        _ => classify_c_named(kind),
    }
}

/// Token type of an anonymous tree-sitter-cpp token.
pub fn classify_cpp_anonymous(kind: &str) -> TokenType {
    match kind {
        // Alternative operator spellings and C++-only operators.
        "and" | "and_eq" | "bitand" | "bitor" | "compl" | "not" | "not_eq" | "or" | "or_eq"
        | "xor" | "xor_eq" | "<=>" | ".*" | "->*" => TokenType::Operator,
        // `operator()`, `operator[]`, `delete[]`, `operator""_x`.
        "()" | "[]" | "\"\"" => TokenType::Punctuation,
        // Raw string openers (inside atomic raw string literals).
        "R\"" | "LR\"" | "uR\"" | "UR\"" | "u8R\"" => TokenType::Punctuation,
        "alignas" | "catch" | "class" | "co_await" | "co_return" | "co_yield" | "concept"
        | "consteval" | "constinit" | "decltype" | "delete" | "explicit" | "final"
        | "friend" | "mutable" | "namespace" | "new" | "noexcept" | "operator" | "override"
        | "private" | "protected" | "public" | "requires" | "static_assert" | "template"
        | "throw" | "try" | "typename" | "using" | "virtual" | "this" | "typeid"
        | "dynamic_cast" | "static_cast" | "reinterpret_cast" | "const_cast" | "export"
        | "import" | "module" | "bool" | "wchar_t" | "char8_t" | "char16_t" | "char32_t" => {
            TokenType::Keyword
        }
        "true" | "false" => TokenType::BooleanLiteral,
        _ => classify_c_anonymous(kind),
    }
}
