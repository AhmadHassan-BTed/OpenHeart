use tree_sitter::Language;

use crate::core::types::token::{LangId, TokenType};
use crate::ingestion::adapter::LanguageAdapter;

/// Grammar-agnostic Phase 1 adapter for registering an additional tree-sitter grammar
/// without a dedicated classification table.
///
/// Leaves are typed from their kind and whether the grammar names them:
/// - named leaves: `*identifier*` kinds are identifiers, `*comment*` kinds comments,
///   `*string*` / `*char*` kinds literals, `*number*` / `*integer*` / `*float*` kinds numbers;
///   other named leaves are `Unknown` (kept, so no source text is lost);
/// - anonymous tokens: alphabetic spellings are keywords, bracket and separator characters
///   punctuation, and every other symbol an operator.
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

    fn classify_named(kind: &str) -> TokenType {
        match kind {
            "true" | "false" => return TokenType::BooleanLiteral,
            "null" | "nil" | "none" | "undefined" => return TokenType::NullLiteral,
            _ => {}
        }
        if kind.contains("identifier") {
            TokenType::Identifier
        } else if kind.contains("comment") {
            TokenType::CommentBlock
        } else if kind.contains("char") {
            TokenType::CharLiteral
        } else if kind.contains("string") {
            TokenType::StringLiteral
        } else if kind.contains("float") || kind.contains("real") {
            TokenType::FloatLiteral
        } else if kind.contains("number") || kind.contains("integer") {
            TokenType::IntegerLiteral
        } else {
            TokenType::Unknown
        }
    }

    fn classify_anonymous(kind: &str) -> TokenType {
        if kind.is_empty() {
            return TokenType::Unknown;
        }
        if kind
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
            && kind.chars().next().is_some_and(|c| c.is_alphabetic())
        {
            return TokenType::Keyword;
        }
        if kind.chars().all(|c| "()[]{};,.:".contains(c)) {
            return TokenType::Punctuation;
        }
        if kind.chars().all(|c| c.is_ascii_punctuation()) {
            return TokenType::Operator;
        }
        TokenType::Unknown
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
        self.ts_lang.clone()
    }

    /// Kind-only classification (named-leaf rules); the walker uses `classify_leaf`, which
    /// also knows whether the leaf is an anonymous token.
    fn map_node_type(&self, kind: &str) -> TokenType {
        Self::classify_named(kind)
    }

    fn classify_leaf(&self, kind: &str, is_named: bool) -> TokenType {
        if is_named {
            Self::classify_named(kind)
        } else {
            Self::classify_anonymous(kind)
        }
    }

    fn include_anonymous(&self, kind: &str) -> bool {
        Self::classify_anonymous(kind) != TokenType::Unknown
    }
}
