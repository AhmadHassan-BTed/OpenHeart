pub mod generic;
pub mod java;
pub mod javascript;
pub mod kotlin;
pub mod registry;

use crate::core::types::source::{FileDiagnostics, TokenFilter};
use crate::core::types::token::{LangId, TokenType};
use crate::ingestion::walker::RawToken;

/// Phase 1 adapter for a language backed by a linked tree-sitter grammar.
///
/// The walker visits every CST leaf; `classify_leaf` decides its token type, and the
/// adapter must classify every named leaf kind and every anonymous token of its grammar
/// (anything mapped to `TokenType::Unknown` is dropped when anonymous and emitted as
/// `Unknown` when named, so that no source text silently disappears).
pub trait LanguageAdapter: Send + Sync + 'static {
    fn language_id(&self) -> LangId;
    fn file_extensions(&self) -> &[&str];
    fn ts_language(&self) -> tree_sitter::Language;
    fn map_node_type(&self, ts_node_kind: &str) -> TokenType;
    fn include_anonymous(&self, _ts_node_kind: &str) -> bool {
        false
    }

    /// Classifies a CST leaf. `is_named` distinguishes a grammar rule (`identifier`) from
    /// an anonymous token whose kind is its own spelling (`if`, `+`).
    fn classify_leaf(&self, ts_node_kind: &str, _is_named: bool) -> TokenType {
        self.map_node_type(ts_node_kind)
    }

    /// Node kinds emitted as a single token even though the grammar gives them children
    /// (string literals with fragments and escapes, template strings, regexes).
    fn is_atomic(&self, _ts_node_kind: &str) -> bool {
        false
    }

    /// Node kinds whose `name` field holds an annotation / decorator name. Identifier
    /// leaves inside that field are typed `TokenType::Annotation`.
    fn is_annotation_kind(&self, _ts_node_kind: &str) -> bool {
        false
    }

    /// Refines a token type using the token text. The default distinguishes documentation
    /// comments (`/** ... */`, but not the empty `/**/`) from ordinary block comments.
    fn refine_token_type(&self, token_type: TokenType, text: &[u8]) -> TokenType {
        refine_doc_comment(token_type, text)
    }

    fn should_skip(&self, token_type: TokenType, filter: &TokenFilter) -> bool {
        match token_type {
            TokenType::Whitespace | TokenType::Newline => !filter.include_whitespace,
            TokenType::CommentLine => !filter.include_line_comments,
            TokenType::CommentBlock => !filter.include_block_comments,
            TokenType::CommentDoc => !filter.include_doc_comments,
            _ => false,
        }
    }
}

/// Shared doc-comment refinement for C-family block comments.
pub fn refine_doc_comment(token_type: TokenType, text: &[u8]) -> TokenType {
    if token_type == TokenType::CommentBlock && text.len() >= 5 && text.starts_with(b"/**") {
        TokenType::CommentDoc
    } else {
        token_type
    }
}

/// Should a token of this type be kept under the filter? Shared by grammar walkers and
/// lexer frontends.
pub fn filter_allows(token_type: TokenType, filter: &TokenFilter) -> bool {
    match token_type {
        TokenType::Whitespace | TokenType::Newline => filter.include_whitespace,
        TokenType::CommentLine => filter.include_line_comments,
        TokenType::CommentBlock => filter.include_block_comments,
        TokenType::CommentDoc => filter.include_doc_comments,
        _ => true,
    }
}

/// Phase 1 frontend for a language that has a hand-written lexer but no linked grammar.
/// Files lexed this way carry `FILE_FLAG_LEXICAL_ONLY`: they contribute tokens to the
/// corpus, and Phase 2 reports that no syntax tree is available for them.
pub trait SourceLexer: Send + Sync + 'static {
    fn language_id(&self) -> LangId;
    fn file_extensions(&self) -> &[&str];

    /// Lexes `source` into raw tokens in source order. Unlexable input is emitted as
    /// `TokenType::Unknown` tokens and counted in `diagnostics.error_count`.
    fn lex(
        &self,
        source: &[u8],
        file_id: u16,
        filter: &TokenFilter,
        diagnostics: &mut FileDiagnostics,
    ) -> Vec<RawToken>;
}
