use tree_sitter::Node;

use crate::core::types::source::{FileDiagnostics, TokenFilter};
use crate::core::types::token::TokenType;
use crate::ingestion::adapter::LanguageAdapter;

/// Transient raw token emitted during CST walking or lexing.
///
/// Positions and lengths are kept at full width here; range checks against the packed
/// sort-key layout happen when the token is committed to the corpus.
#[derive(Debug, Clone)]
pub struct RawToken {
    pub file_id: u16,
    /// 1-based line.
    pub line: u32,
    /// 0-based byte column within the line.
    pub col: u32,
    pub token_type: TokenType,
    pub text_start: usize,
    pub text_len: usize,
}

/// Walk the tree-sitter CST for one file, extracting all leaf tokens in source order (DFS).
pub fn walk_cst(
    node: Node,
    source: &[u8],
    file_id: u16,
    adapter: &dyn LanguageAdapter,
    filter: &TokenFilter,
) -> Vec<RawToken> {
    let mut diagnostics = FileDiagnostics::new(file_id);
    walk_cst_with_diagnostics(node, source, file_id, adapter, filter, &mut diagnostics)
}

/// Iterative CST walk with a single `TreeCursor` (constant native stack depth, no cursor
/// allocation per node). ERROR nodes are descended into so recoverable tokens are kept,
/// and counted; MISSING nodes are zero-width parser insertions with no source text, so
/// they are counted and never emitted as tokens.
pub fn walk_cst_with_diagnostics(
    root: Node,
    source: &[u8],
    file_id: u16,
    adapter: &dyn LanguageAdapter,
    filter: &TokenFilter,
    diagnostics: &mut FileDiagnostics,
) -> Vec<RawToken> {
    let mut out = Vec::with_capacity(source.len() / 4);
    let mut cursor = root.walk();
    // One frame per ancestor of the cursor's node: (kind, inside an annotation name).
    let mut frames: Vec<(&'static str, bool)> = Vec::with_capacity(64);

    loop {
        let node = cursor.node();
        let kind = node.kind();
        let in_annotation_name = match frames.last() {
            Some(&(parent_kind, parent_flag)) => {
                parent_flag
                    || (adapter.is_annotation_kind(parent_kind)
                        && cursor.field_name() == Some("name"))
            }
            None => false,
        };

        if kind == "ERROR" {
            diagnostics.error_count = diagnostics.error_count.saturating_add(1);
            diagnostics.record_error_range(node.start_byte(), node.end_byte());
        }

        let mut descend = false;
        if node.is_missing() {
            diagnostics.missing_count = diagnostics.missing_count.saturating_add(1);
            diagnostics.record_error_range(node.start_byte(), node.end_byte());
        } else if node.child_count() == 0 || adapter.is_atomic(kind) {
            emit_leaf(
                &node,
                kind,
                source,
                file_id,
                adapter,
                filter,
                in_annotation_name,
                &mut out,
            );
        } else {
            descend = true;
        }

        if descend && cursor.goto_first_child() {
            frames.push((kind, in_annotation_name));
            continue;
        }

        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                return out;
            }
            frames.pop();
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_leaf(
    node: &Node,
    kind: &str,
    source: &[u8],
    file_id: u16,
    adapter: &dyn LanguageAdapter,
    filter: &TokenFilter,
    in_annotation_name: bool,
    out: &mut Vec<RawToken>,
) {
    let is_named = node.is_named();
    if !is_named && !adapter.include_anonymous(kind) {
        return;
    }

    let range = node.byte_range();
    // Zero-width leaves carry no source text (e.g. automatic semicolons).
    if range.end <= range.start || range.end > source.len() {
        return;
    }
    let text = &source[range.start..range.end];

    let mut token_type = adapter.classify_leaf(kind, is_named);
    if in_annotation_name && token_type == TokenType::Identifier {
        token_type = TokenType::Annotation;
    }
    token_type = adapter.refine_token_type(token_type, text);

    if adapter.should_skip(token_type, filter) {
        return;
    }

    let start = node.start_position();
    out.push(RawToken {
        file_id,
        line: u32::try_from(start.row)
            .map(|r| r.saturating_add(1))
            .unwrap_or(u32::MAX),
        col: u32::try_from(start.column).unwrap_or(u32::MAX),
        token_type,
        text_start: range.start,
        text_len: range.end - range.start,
    });
}
