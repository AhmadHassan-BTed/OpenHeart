//! Core CST Reduction DFS Walk and Token ID lookup.

use super::adapter::{ASTReductionAdapter, ReductionDecision};
use super::builder::BPASTBuilder;
use crate::core::types::token::{build_sort_key, TokenRecord};
use tree_sitter::Node;

pub fn reduce_and_encode(
    node: Node,
    source: &[u8],
    file_id: u16,
    adapter: &dyn ASTReductionAdapter,
    tok_table: &[TokenRecord],
    builder: &mut BPASTBuilder,
) -> Option<(u32, u32)> {
    match adapter.classify(node.kind(), &node, builder.current_depth()) {
        ReductionDecision::Drop => None,

        ReductionDecision::Eliminate => {
            let mut first_tok = u32::MAX;
            let mut last_tok = 0u32;
            let mut cursor = node.walk();
            if cursor.goto_first_child() {
                loop {
                    if let Some((ft, lt)) = reduce_and_encode(
                        cursor.node(),
                        source,
                        file_id,
                        adapter,
                        tok_table,
                        builder,
                    ) {
                        first_tok = first_tok.min(ft);
                        last_tok = last_tok.max(lt);
                    }
                    if !cursor.goto_next_sibling() {
                        break;
                    }
                }
            }
            if first_tok == u32::MAX {
                None
            } else {
                Some((first_tok, last_tok))
            }
        }

        ReductionDecision::Keep(node_type) => {
            let attrs = adapter.encode_attrs(node.kind(), &node, source);
            let preorder = builder.open_node(node_type, attrs);

            let mut first_tok = u32::MAX;
            let mut last_tok = 0u32;

            let start = node.start_position();
            let col = (start.column.min(u16::MAX as usize)) as u16;
            let sort_key = build_sort_key(file_id, (start.row + 1) as u32, col);
            let token_id = tok_table_lookup(tok_table, sort_key);
            if token_id != u32::MAX {
                first_tok = token_id;
                last_tok = token_id;
            }

            if node.child_count() > 0 {
                let mut cursor = node.walk();
                cursor.goto_first_child();
                loop {
                    if let Some((ft, lt)) = reduce_and_encode(
                        cursor.node(),
                        source,
                        file_id,
                        adapter,
                        tok_table,
                        builder,
                    ) {
                        first_tok = first_tok.min(ft);
                        last_tok = last_tok.max(lt);
                    }
                    if !cursor.goto_next_sibling() {
                        break;
                    }
                }
            }

            builder.close_node(preorder, first_tok, last_tok);
            if first_tok == u32::MAX {
                None
            } else {
                Some((first_tok, last_tok))
            }
        }
    }
}

fn tok_table_lookup(table: &[TokenRecord], sort_key: u64) -> u32 {
    match table.binary_search_by_key(&sort_key, |r| r.sort_key) {
        Ok(idx) => idx as u32,
        Err(idx) => {
            let (file_id, line, _col) = crate::core::types::token::unpack_sort_key(sort_key);
            if idx < table.len() {
                let (rec_file, rec_line, _rec_col) =
                    crate::core::types::token::unpack_sort_key(table[idx].sort_key);
                if rec_file == file_id && rec_line == line {
                    return idx as u32;
                }
            }
            if idx > 0 {
                let (rec_file, rec_line, _rec_col) =
                    crate::core::types::token::unpack_sort_key(table[idx - 1].sort_key);
                if rec_file == file_id && rec_line == line {
                    return (idx - 1) as u32;
                }
            }
            u32::MAX
        }
    }
}
