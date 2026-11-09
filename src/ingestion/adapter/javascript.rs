use tree_sitter::Language;

use crate::core::types::token::{LangId, TokenType};
use crate::ingestion::adapter::LanguageAdapter;

/// Phase 1 adapter for tree-sitter-javascript (ECMAScript + JSX).
pub struct JavaScriptLanguageAdapter;

impl JavaScriptLanguageAdapter {
    pub fn new() -> Self {
        Self
    }
}

impl Default for JavaScriptLanguageAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl LanguageAdapter for JavaScriptLanguageAdapter {
    fn language_id(&self) -> LangId {
        LangId::JavaScript
    }

    fn file_extensions(&self) -> &[&str] {
        &["js", "jsx", "mjs", "cjs"]
    }

    fn ts_language(&self) -> Language {
        tree_sitter_javascript::LANGUAGE.into()
    }

    fn map_node_type(&self, kind: &str) -> TokenType {
        match kind {
            "identifier"
            | "property_identifier"
            | "shorthand_property_identifier"
            | "shorthand_property_identifier_pattern"
            | "private_property_identifier"
            | "statement_identifier"
            | "jsx_identifier" => TokenType::Identifier,
            "number" => TokenType::IntegerLiteral,
            "string" | "template_string" | "regex" | "jsx_text" => TokenType::StringLiteral,
            "true" | "false" => TokenType::BooleanLiteral,
            "null" | "undefined" => TokenType::NullLiteral,
            "comment" | "html_comment" => TokenType::CommentBlock,
            "hash_bang_line" => TokenType::CommentLine,
            "this" | "super" => TokenType::Keyword,
            // Operators
            "=" | "+=" | "-=" | "*=" | "/=" | "%=" | "**=" | "<<=" | ">>=" | ">>>=" | "&="
            | "^=" | "|=" | "&&=" | "||=" | "??=" | "==" | "===" | "!=" | "!==" | "<" | "<="
            | ">" | ">=" | "+" | "-" | "*" | "/" | "%" | "**" | "++" | "--" | "<<" | ">>"
            | ">>>" | "&" | "|" | "^" | "~" | "!" | "&&" | "||" | "??" | "?" | "?." | "=>"
            | "..." => TokenType::Operator,
            // Punctuation
            "(" | ")" | "[" | "]" | "{" | "}" | ";" | "," | "." | ":" | "${" | "`" | "</"
            | "/>" | "@" | "#" => TokenType::Punctuation,
            // Keywords (reserved and contextual)
            "as" | "async" | "await" | "break" | "case" | "catch" | "class" | "const"
            | "continue" | "debugger" | "default" | "delete" | "do" | "else" | "export"
            | "extends" | "finally" | "for" | "from" | "function" | "get" | "if" | "import"
            | "in" | "instanceof" | "let" | "new" | "of" | "return" | "set" | "static"
            | "switch" | "target" | "meta" | "throw" | "try" | "typeof" | "var" | "void"
            | "while" | "with" | "yield" => TokenType::Keyword,
            _ => TokenType::Unknown,
        }
    }

    fn include_anonymous(&self, kind: &str) -> bool {
        self.map_node_type(kind) != TokenType::Unknown
    }

    fn is_atomic(&self, kind: &str) -> bool {
        matches!(kind, "string" | "template_string" | "regex")
    }

    fn is_annotation_kind(&self, kind: &str) -> bool {
        kind == "decorator"
    }

    /// tree-sitter-javascript uses one `comment` kind for `//`, `/* */` and HTML-style
    /// comments; the comment form is recovered from the text.
    fn refine_token_type(&self, token_type: TokenType, text: &[u8]) -> TokenType {
        if token_type == TokenType::CommentBlock {
            if text.starts_with(b"//") || text.starts_with(b"<!--") || text.starts_with(b"-->") {
                return TokenType::CommentLine;
            }
            if text.len() >= 5 && text.starts_with(b"/**") {
                return TokenType::CommentDoc;
            }
        }
        token_type
    }
}
