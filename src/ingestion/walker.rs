use tree_sitter::Node;

use crate::core::types::source::TokenFilter;
use crate::core::types::token::TokenType;
use crate::ingestion::adapter::LanguageAdapter;

/// Transient raw token emitted during CST walking.
#[derive(Debug, Clone)]
pub struct RawToken {
    pub file_id: u16,
    pub line: u32,
    pub col: u16,
    pub len: u16,
    pub token_type: TokenType,
    pub text_start: usize,
    pub text_len: usize,
}

/// Walk the tree-sitter CST for one file, extracting all leaf tokens in source order (DFS).
pub fn walk_cst(
    node: Node,
    _source: &[u8],
    file_id: u16,
    adapter: &dyn LanguageAdapter,
    filter: &TokenFilter,
) -> Vec<RawToken> {
    let mut tokens = Vec::with_capacity(node.child_count() * 4);
    walk_recursive(node, file_id, adapter, filter, &mut tokens);
    tokens
}

fn walk_recursive(
    node: Node,
    file_id: u16,
    adapter: &dyn LanguageAdapter,
    filter: &TokenFilter,
    out: &mut Vec<RawToken>,
) {
    if node.child_count() == 0 {
        let ts_kind = node.kind();

        if !node.is_named() && !adapter.include_anonymous(ts_kind) {
            return;
        }

        let token_type = adapter.map_node_type(ts_kind);

        if adapter.should_skip(token_type, filter) {
            return;
        }

        let start = node.start_position();
        let byte_range = node.byte_range();
        let len = (byte_range.end - byte_range.start).min(u16::MAX as usize) as u16;

        out.push(RawToken {
            file_id,
            line: (start.row + 1) as u32,
            col: start.column as u16,
            len,
            token_type,
            text_start: byte_range.start,
            text_len: len as usize,
        });
        return;
    }

    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            walk_recursive(cursor.node(), file_id, adapter, filter, out);
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
}
