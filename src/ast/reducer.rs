//! Core CST reduction walk and token-span lookup.
//!
//! The walk is iterative over a single `TreeCursor`, so nesting depth costs heap frames
//! instead of native stack (deeply nested generated code cannot overflow the stack), and
//! no cursor is allocated per internal node.
//!
//! Token spans are computed from each kept node's own source range: the node covers
//! exactly the tokens whose start position lies in `[start, end)`. No neighbouring token is
//! borrowed when a node starts on a token-less position, and no ancestor propagation is
//! needed.

use super::adapter::{ASTReductionAdapter, ReductionContext, ReductionDecision};
use super::builder::BPASTBuilder;
use crate::core::types::ast::ASTNodeType;
use crate::core::types::token::{saturating_sort_key, TokenRecord, SORT_KEY_POSITION_MASK};
use tree_sitter::{Node, Point};

/// Counts of what the reduction did with one file's CST.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReductionStats {
    /// AST nodes emitted (a `Wrap` decision emits two).
    pub kept: u32,
    /// Named CST nodes eliminated (children kept).
    pub eliminated: u32,
    /// Named CST subtrees dropped by the adapter's trivia allowlist.
    pub dropped: u32,
    /// ERROR regions encountered (their recoverable contents are kept).
    pub error_regions: u32,
    /// Zero-width MISSING nodes skipped.
    pub missing: u32,
}

/// Result of reducing one file.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FileReduction {
    /// Token span of the whole file, or `None` when it has no tokens.
    pub span: Option<(u32, u32)>,
    pub stats: ReductionStats,
}

struct Frame {
    /// AST nodes opened for this CST node, outermost first, with their spans.
    opened: [Option<(u32, Option<(u32, u32)>)>; 2],
}

impl Frame {
    const EMPTY: Frame = Frame {
        opened: [None, None],
    };
}

#[inline]
fn point_key(file_id: u16, p: Point) -> u64 {
    saturating_sort_key(
        file_id,
        u32::try_from(p.row.saturating_add(1)).unwrap_or(u32::MAX),
        u32::try_from(p.column).unwrap_or(u32::MAX),
    )
}

/// Tokens whose start lies in `[start, end)`, as an inclusive id range.
pub fn token_span(tok_table: &[TokenRecord], file_id: u16, start: Point, end: Point) -> Option<(u32, u32)> {
    if tok_table.is_empty() {
        return None;
    }
    let start_key = point_key(file_id, start);
    let end_key = point_key(file_id, end);
    if end_key <= start_key {
        return None;
    }
    let lo = tok_table.partition_point(|r| r.sort_key < start_key);
    let hi = tok_table.partition_point(|r| r.sort_key < end_key);
    if lo < hi {
        Some((lo as u32, (hi - 1) as u32))
    } else {
        None
    }
}

/// The token starting exactly at `p`, if any.
pub fn token_at(tok_table: &[TokenRecord], file_id: u16, p: Point) -> Option<u32> {
    let key = point_key(file_id, p);
    let idx = tok_table.partition_point(|r| r.sort_key < key);
    tok_table
        .get(idx)
        .filter(|r| (r.sort_key & SORT_KEY_POSITION_MASK) == key)
        .map(|_| idx as u32)
}

fn close_frame(builder: &mut BPASTBuilder, frame: Frame) {
    for (idx, span) in frame.opened.iter().rev().flatten() {
        let (first, last) = span.unwrap_or((u32::MAX, 0));
        builder.close_node(*idx, first, last);
    }
}

/// Reduces one file's CST into the builder and returns the file's token span.
pub fn reduce_and_encode(
    node: Node,
    source: &[u8],
    file_id: u16,
    adapter: &dyn ASTReductionAdapter,
    tok_table: &[TokenRecord],
    builder: &mut BPASTBuilder,
) -> Option<(u32, u32)> {
    reduce_file(node, source, file_id, adapter, tok_table, builder).span
}

/// Reduces one file's CST into the builder, reporting reduction statistics.
pub fn reduce_file(
    root: Node,
    source: &[u8],
    file_id: u16,
    adapter: &dyn ASTReductionAdapter,
    tok_table: &[TokenRecord],
    builder: &mut BPASTBuilder,
) -> FileReduction {
    let mut stats = ReductionStats::default();
    let span = token_span(tok_table, file_id, root.start_position(), root.end_position());

    let mut cursor = root.walk();
    let mut ancestors: Vec<&'static str> = Vec::with_capacity(64);
    let mut frames: Vec<Frame> = Vec::with_capacity(64);

    'walk: loop {
        let node = cursor.node();
        let kind = node.kind();

        let decision = if node.is_missing() {
            stats.missing += 1;
            ReductionDecision::Drop
        } else if !node.is_named() {
            ReductionDecision::Drop
        } else {
            if kind == "ERROR" {
                stats.error_regions += 1;
            }
            let ctx = ReductionContext {
                ancestors: &ancestors,
                field_name: cursor.field_name(),
                depth: builder.current_depth(),
            };
            adapter.classify(kind, &node, &ctx)
        };

        let mut frame = Frame::EMPTY;
        match decision {
            ReductionDecision::Drop => {
                if node.is_named() && !node.is_missing() {
                    stats.dropped += 1;
                }
            }
            ReductionDecision::Eliminate => stats.eliminated += 1,
            ReductionDecision::Keep(node_type) => {
                frame.opened[0] = Some(open_kept(
                    node_type, kind, &node, source, file_id, adapter, tok_table, builder,
                    &ancestors, &cursor, true,
                ));
                stats.kept += 1;
            }
            ReductionDecision::Wrap(outer, inner) => {
                frame.opened[0] = Some(open_kept(
                    outer, kind, &node, source, file_id, adapter, tok_table, builder,
                    &ancestors, &cursor, true,
                ));
                frame.opened[1] = Some(open_kept(
                    inner, kind, &node, source, file_id, adapter, tok_table, builder,
                    &ancestors, &cursor, false,
                ));
                stats.kept += 2;
            }
        }

        let descend = !matches!(decision, ReductionDecision::Drop) && node.child_count() > 0;
        if descend && cursor.goto_first_child() {
            ancestors.push(kind);
            frames.push(frame);
            continue 'walk;
        }
        close_frame(builder, frame);

        loop {
            if cursor.goto_next_sibling() {
                continue 'walk;
            }
            if !cursor.goto_parent() {
                break 'walk;
            }
            ancestors.pop();
            if let Some(parent_frame) = frames.pop() {
                close_frame(builder, parent_frame);
            }
        }
    }

    FileReduction { span, stats }
}

/// Opens one AST node for `node`. `with_attrs` selects whether the adapter's attribute
/// word and name token apply (the inner node of a `Wrap` carries neither).
#[allow(clippy::too_many_arguments)]
fn open_kept(
    node_type: ASTNodeType,
    kind: &str,
    node: &Node,
    source: &[u8],
    file_id: u16,
    adapter: &dyn ASTReductionAdapter,
    tok_table: &[TokenRecord],
    builder: &mut BPASTBuilder,
    ancestors: &[&'static str],
    cursor: &tree_sitter::TreeCursor,
    with_attrs: bool,
) -> (u32, Option<(u32, u32)>) {
    let attrs = if with_attrs {
        let ctx = ReductionContext {
            ancestors,
            field_name: cursor.field_name(),
            depth: builder.current_depth(),
        };
        adapter.encode_attrs(kind, node, source, &ctx)
    } else {
        0
    };
    let idx = builder.open_node(node_type, attrs);
    if with_attrs {
        if let Some(name_tok) = adapter
            .name_node(kind, node)
            .and_then(|n| token_at(tok_table, file_id, n.start_position()))
        {
            builder.set_name_token(idx, name_tok);
        }
    }
    let span = token_span(tok_table, file_id, node.start_position(), node.end_position());
    (idx, span)
}
