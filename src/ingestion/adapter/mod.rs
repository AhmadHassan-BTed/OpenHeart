pub mod java;
pub mod kotlin;
pub mod registry;

use crate::core::types::source::TokenFilter;
use crate::core::types::token::{LangId, TokenType};

pub trait LanguageAdapter: Send + Sync + 'static {
    fn language_id(&self) -> LangId;
    fn file_extensions(&self) -> &[&str];
    fn ts_language(&self) -> tree_sitter::Language;
    fn map_node_type(&self, ts_node_kind: &str) -> TokenType;
    fn include_anonymous(&self, ts_node_kind: &str) -> bool;

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
