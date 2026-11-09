use tree_sitter::Language;

use crate::core::types::token::{LangId, TokenType};
use crate::ingestion::adapter::LanguageAdapter;

/// Phase 1 adapter for tree-sitter-java.
///
/// The tables cover every named leaf kind and every anonymous token of the grammar
/// (keywords including the contextual `record`, `sealed`, `non-sealed`, `permits`, `yield`,
/// `when` and the module-declaration keywords; all operators; all separators, including
/// `?` and `:` of ternaries, labels, enhanced-for and switch labels).
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
        tree_sitter_java::LANGUAGE.into()
    }

    fn map_node_type(&self, kind: &str) -> TokenType {
        match kind {
            "identifier" | "type_identifier" => TokenType::Identifier,
            "decimal_integer_literal"
            | "hex_integer_literal"
            | "octal_integer_literal"
            | "binary_integer_literal" => TokenType::IntegerLiteral,
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
            | "-=" | "*=" | "/=" | "%=" | "&=" | "|=" | "^=" | "<<=" | ">>=" | ">>>=" | "?"
            | "asterisk" => TokenType::Operator,
            // Punctuation
            ";" | "," | "." | "(" | ")" | "[" | "]" | "{" | "}" | "..." | "::" | "->" | ":" => {
                TokenType::Punctuation
            }
            // Keywords (reserved and contextual)
            "if" | "else" | "for" | "while" | "do" | "switch" | "case" | "break" | "continue"
            | "return" | "throw" | "try" | "catch" | "finally" | "new" | "instanceof" | "class"
            | "interface" | "enum" | "record" | "extends" | "implements" | "throws" | "import"
            | "public" | "private" | "protected" | "static" | "final" | "abstract" | "native"
            | "synchronized" | "volatile" | "transient" | "strictfp" | "default" | "void"
            | "boolean" | "byte" | "char" | "short" | "int" | "long" | "float" | "double"
            | "this" | "super" | "package" | "assert" | "yield" | "sealed" | "non-sealed"
            | "permits" | "when" | "goto" | "const" | "module" | "open" | "requires"
            | "transitive" | "exports" | "opens" | "to" | "uses" | "provides" | "with"
            | "void_type" | "boolean_type" => TokenType::Keyword,
            "@interface" => TokenType::Keyword,

            "var" => TokenType::JavaVarKeyword,
            "marker_annotation" | "annotation" => TokenType::Annotation,
            "@" => TokenType::JavaAnnotationMarker,
            _ => TokenType::Unknown,
        }
    }

    fn include_anonymous(&self, kind: &str) -> bool {
        self.map_node_type(kind) != TokenType::Unknown
    }

    fn is_atomic(&self, kind: &str) -> bool {
        // Newer grammar revisions give string literals `string_fragment` /
        // `escape_sequence` children; the literal stays one token either way.
        matches!(kind, "string_literal" | "text_block" | "character_literal")
    }

    fn is_annotation_kind(&self, kind: &str) -> bool {
        matches!(kind, "marker_annotation" | "annotation")
    }
}
