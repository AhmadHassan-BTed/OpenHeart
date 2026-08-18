use tree_sitter::Language;

use crate::core::types::token::{LangId, TokenType};
use crate::ingestion::adapter::LanguageAdapter;

pub struct GenericLanguageAdapter {
    lang_id: LangId,
    exts: Vec<&'static str>,
    ts_lang: Language,
}

impl GenericLanguageAdapter {
    pub fn new(lang_id: LangId, exts: Vec<&'static str>, ts_lang: Language) -> Self {
        Self {
            lang_id,
            exts,
            ts_lang,
        }
    }
}

impl LanguageAdapter for GenericLanguageAdapter {
    fn language_id(&self) -> LangId {
        self.lang_id
    }

    fn file_extensions(&self) -> &[&str] {
        &self.exts
    }

    fn ts_language(&self) -> Language {
        self.ts_lang
    }

    fn map_node_type(&self, kind: &str) -> TokenType {
        match kind {
            "identifier"
            | "property_identifier"
            | "shorthand_property_identifier"
            | "type_identifier" => TokenType::Identifier,
            "number" | "integer" | "decimal_integer_literal" => TokenType::IntegerLiteral,
            "float" | "decimal_floating_point_literal" => TokenType::FloatLiteral,
            "string" | "string_literal" | "template_string" => TokenType::StringLiteral,
            "true" | "false" => TokenType::BooleanLiteral,
            "null" | "undefined" => TokenType::NullLiteral,
            "comment" | "line_comment" => TokenType::CommentLine,
            "block_comment" => TokenType::CommentBlock,
            "+" | "-" | "*" | "/" | "%" | "==" | "===" | "!=" | "!==" | "<" | ">" | "<=" | ">="
            | "&&" | "||" | "!" | "&" | "|" | "^" | "++" | "--" | "~" | "<<" | ">>" | "="
            | "+=" | "-=" | "*=" | "/=" | "=>" => TokenType::Operator,
            ";" | "," | "." | "(" | ")" | "[" | "]" | "{" | "}" | ":" => TokenType::Punctuation,
            _ => {
                if kind.chars().all(|c| c.is_alphabetic() || c == '_') && kind.len() > 1 {
                    TokenType::Keyword
                } else {
                    TokenType::Unknown
                }
            }
        }
    }
}
