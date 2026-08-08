use tree_sitter::Language;

use crate::core::types::token::{LangId, TokenType};
use crate::phase1::adapter::LanguageAdapter;

pub struct JavaLanguageAdapter;

impl JavaLanguageAdapter {
    pub fn new() -> Self {
        Self
    }
}

impl Default for JavaLanguageAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl LanguageAdapter for JavaLanguageAdapter {
    fn language_id(&self) -> LangId {
        LangId::Java
    }

    fn file_extensions(&self) -> &[&str] {
        &["java"]
    }

    fn ts_language(&self) -> Language {
        tree_sitter_java::language()
    }

    fn map_node_type(&self, kind: &str) -> TokenType {
        match kind {
            "identifier" | "type_identifier" => TokenType::Identifier,
            "decimal_integer_literal" | "hex_integer_literal" | "binary_integer_literal" => {
                TokenType::IntegerLiteral
            }
            "decimal_floating_point_literal" | "hex_floating_point_literal" => {
                TokenType::FloatLiteral
            }
            "string_literal" | "text_block" => TokenType::StringLiteral,
            "character_literal" => TokenType::CharLiteral,
            "true" | "false" => TokenType::BooleanLiteral,
            "null_literal" => TokenType::NullLiteral,
            "line_comment" => TokenType::CommentLine,
            "block_comment" => TokenType::CommentBlock,
            "comment" => TokenType::CommentBlock,
            // Operators
            "+" | "-" | "*" | "/" | "%" | "==" | "!=" | "<" | ">" | "<=" | ">=" | "&&" | "||"
            | "!" | "&" | "|" | "^" | "++" | "--" | "~" | "<<" | ">>" | ">>>" | "=" | "+="
            | "-=" | "*=" | "/=" | "%=" | "&=" | "|=" | "^=" | "<<=" | ">>=" | ">>>=" => {
                TokenType::Operator
            }
            // Punctuation
            ";" | "," | "." | "(" | ")" | "[" | "]" | "{" | "}" | "..." | "::" | "->" => {
                TokenType::Punctuation
            }
            // Keywords
            "if" | "else" | "for" | "while" | "do" | "switch" | "case" | "break" | "continue"
            | "return" | "throw" | "try" | "catch" | "finally" | "new" | "instanceof" | "class"
            | "interface" | "enum" | "record" | "extends" | "implements" | "throws" | "import"
            | "public" | "private" | "protected" | "static" | "final" | "abstract" | "native"
            | "synchronized" | "volatile" | "transient" | "strictfp" | "default" | "void"
            | "boolean" | "byte" | "char" | "short" | "int" | "long" | "float" | "double"
            | "this" | "super" | "package" | "assert" | "yield" => TokenType::Keyword,

            "var" => TokenType::JavaVarKeyword,
            "marker_annotation" | "annotation" => TokenType::Annotation,
            "@" => TokenType::JavaAnnotationMarker,
            _ => TokenType::Unknown,
        }
    }

    fn include_anonymous(&self, kind: &str) -> bool {
        matches!(
            kind,
            "+" | "-"
                | "*"
                | "/"
                | "%"
                | "=="
                | "!="
                | "<"
                | ">"
                | "<="
                | ">="
                | "&&"
                | "||"
                | "!"
                | "&"
                | "|"
                | "^"
                | "++"
                | "--"
                | "~"
                | "<<"
                | ">>"
                | ">>>"
                | "="
                | "+="
                | "-="
                | "*="
                | "/="
                | "%="
                | "&="
                | "|="
                | "^="
                | "<<="
                | ">>="
                | ">>>="
                | ";"
                | ","
                | "."
                | "("
                | ")"
                | "["
                | "]"
                | "{"
                | "}"
                | "..."
                | "::"
                | "->"
                | "@"
        )
    }
}
