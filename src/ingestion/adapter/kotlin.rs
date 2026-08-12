use tree_sitter::Language;

use crate::core::types::token::{LangId, TokenType};
use crate::ingestion::adapter::LanguageAdapter;

pub struct KotlinLanguageAdapter;

impl KotlinLanguageAdapter {
    pub fn new() -> Self {
        Self
    }
}

impl Default for KotlinLanguageAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl LanguageAdapter for KotlinLanguageAdapter {
    fn language_id(&self) -> LangId {
        LangId::Kotlin
    }

    fn file_extensions(&self) -> &[&str] {
        &["kt", "kts"]
    }

    fn ts_language(&self) -> Language {
        tree_sitter_java::language()
    }

    fn map_node_type(&self, kind: &str) -> TokenType {
        match kind {
            "identifier" | "type_identifier" | "simple_identifier" => TokenType::Identifier,
            "decimal_integer_literal"
            | "hex_integer_literal"
            | "binary_integer_literal"
            | "integer_literal" => TokenType::IntegerLiteral,
            "decimal_floating_point_literal" | "hex_floating_point_literal" | "real_literal" => {
                TokenType::FloatLiteral
            }
            "string_literal"
            | "text_block"
            | "line_string_literal"
            | "multi_line_string_literal" => TokenType::StringLiteral,
            "character_literal" => TokenType::CharLiteral,
            "true" | "false" => TokenType::BooleanLiteral,
            "null_literal" | "null" => TokenType::NullLiteral,
            "line_comment" => TokenType::CommentLine,
            "block_comment" => TokenType::CommentBlock,
            "comment" => TokenType::CommentBlock,
            // Operators
            "+" | "-" | "*" | "/" | "%" | "==" | "!=" | "<" | ">" | "<=" | ">=" | "&&" | "||"
            | "!" | "&" | "|" | "^" | "~" | "<<" | ">>" | ">>>" | "=" | "+=" | "-=" | "*="
            | "/=" | "%=" | "&=" | "|=" | "^=" | "<<=" | ">>=" | ">>>=" | "++" | "--" | "->"
            | "?:" | "!!" | ".." | "::" => TokenType::Operator,
            // Keywords
            "class" | "interface" | "enum" | "record" | "extends" | "implements" | "public"
            | "private" | "protected" | "static" | "final" | "abstract" | "synchronized"
            | "native" | "transient" | "volatile" | "strictfp" | "default" | "package"
            | "import" | "if" | "else" | "switch" | "case" | "while" | "do" | "for" | "break"
            | "continue" | "return" | "throw" | "throws" | "try" | "catch" | "finally" | "new"
            | "this" | "super" | "instanceof" | "void" | "fun" | "val" | "var" | "object"
            | "companion" | "data" | "sealed" | "open" | "override" | "internal" | "when"
            | "in" | "is" | "by" => TokenType::Keyword,
            // Punctuation
            "{" | "}" | "(" | ")" | "[" | "]" | ";" | "," | "." | ":" | "?" | "@" => {
                TokenType::Punctuation
            }
            _ => TokenType::Unknown,
        }
    }

    fn include_anonymous(&self, kind: &str) -> bool {
        self.map_node_type(kind) != TokenType::Unknown
    }
}
