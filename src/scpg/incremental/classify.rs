//! DeltaClassifier — maps text edits onto tokens and entity spans of a snapshot (§10.6.2).
//!
//! An edit is classified against the *old* snapshot:
//! - **Trivia**: it only rewrites whitespace between tokens (without merging two word
//!   tokens), or stays strictly inside one comment and the rewritten comment is still exactly
//!   one comment. Positions shift, analysis results do not.
//! - **Body**: it lies strictly inside the definition body of a callable.
//! - **Declaration**: it touches an entity outside any callable body.
//! - **Outside declarations**: imports, package clause, or unknown files.

use crate::core::types::token::TokenType;
use crate::scpg::incremental::delta::*;
use crate::scpg::query::snapshot::ScpgSnapshot;
use crate::scpg::query::source::SourcePosition;
use crate::scpg::query::symbols::is_callable_kind;

/// Byte offsets of line starts (line 1 starts at offset 0).
fn line_starts(text: &[u8]) -> Vec<usize> {
    let mut starts = vec![0];
    for (i, &b) in text.iter().enumerate() {
        if b == b'\n' {
            starts.push(i + 1);
        }
    }
    starts
}

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$' || b >= 0x80
}

fn is_whitespace_only(bytes: &[u8]) -> bool {
    bytes.iter().all(|b| b.is_ascii_whitespace())
}

pub struct DeltaClassifier<'a> {
    snapshot: &'a ScpgSnapshot,
}

struct OldText<'t> {
    text: &'t [u8],
    starts: Vec<usize>,
}

impl<'t> OldText<'t> {
    fn offset_of(&self, line: u32, col: u16) -> Option<usize> {
        let start = *self.starts.get((line as usize).checked_sub(1)?)?;
        let off = start + col as usize;
        if off <= self.text.len() {
            Some(off)
        } else {
            None
        }
    }

    /// 1-based line / byte column of `offset` (the snapshot's position convention).
    fn position_of(&self, offset: usize) -> (u32, u16) {
        let line_idx = self
            .starts
            .partition_point(|&s| s <= offset)
            .saturating_sub(1);
        let col = offset - self.starts[line_idx];
        ((line_idx + 1) as u32, col.min(u16::MAX as usize) as u16)
    }
}

impl<'a> DeltaClassifier<'a> {
    pub fn new(snapshot: &'a ScpgSnapshot) -> Self {
        Self { snapshot }
    }

    /// Byte range `[start, end)` of a token in the old text.
    fn token_bytes(&self, old: &OldText<'_>, token: u32) -> Option<(usize, usize)> {
        let SourcePosition { line, col, .. } = self.snapshot.tokens.start(token)?;
        let start = old.offset_of(line, col)?;
        let len = self.snapshot.tokens.byte_len(token)? as usize;
        Some((start, (start + len).min(old.text.len())))
    }

    /// Tokens of `file_id` overlapping `[start, end)` (for an insertion, `start == end`: the
    /// token strictly containing the insertion point).
    fn overlapping_tokens(
        &self,
        old: &OldText<'_>,
        file_id: u16,
        start: usize,
        end: usize,
    ) -> Vec<(u32, usize, usize)> {
        let mut out = Vec::new();
        let (sl, sc) = old.position_of(start);
        if let Some(t) = self.snapshot.tokens.token_at_or_before(file_id, sl, sc) {
            if let Some((ts, te)) = self.token_bytes(old, t) {
                let overlaps = if start == end {
                    ts < start && start < te
                } else {
                    ts < end && start < te
                };
                if overlaps {
                    out.push((t, ts, te));
                }
            }
        }
        if end > start {
            let (el, ec) = old.position_of(end);
            for &t in self
                .snapshot
                .tokens
                .tokens_starting_in(file_id, (sl, sc), (el, ec))
            {
                if out.iter().any(|&(x, _, _)| x == t) {
                    continue;
                }
                if let Some((ts, te)) = self.token_bytes(old, t) {
                    if ts < end && start < te {
                        out.push((t, ts, te));
                    }
                }
            }
        }
        out
    }

    fn is_trivia_edit(&self, delta: &SourceDelta, old: &OldText<'_>, edit: &TextEdit) -> bool {
        let deleted = &delta.old_text[edit.start_byte..edit.old_end_byte];
        let inserted = &delta.new_text[edit.start_byte..edit.new_end_byte];
        let tokens =
            self.overlapping_tokens(old, delta.file_id, edit.start_byte, edit.old_end_byte);

        // Whitespace rewritten between tokens.
        if tokens.is_empty() && is_whitespace_only(deleted) && is_whitespace_only(inserted) {
            let before = edit
                .start_byte
                .checked_sub(1)
                .and_then(|i| delta.old_text.get(i))
                .copied();
            let after = delta.old_text.get(edit.old_end_byte).copied();
            let would_merge = inserted.is_empty()
                && !deleted.is_empty()
                && before.map(is_word_byte).unwrap_or(false)
                && after.map(is_word_byte).unwrap_or(false);
            return !would_merge;
        }

        // Rewrite strictly inside a single comment that remains a single comment.
        if tokens.len() != 1 {
            return false;
        }
        let (t, ts, te) = tokens[0];
        let ty = self.snapshot.tokens.token_type(t);
        let is_block = matches!(
            ty,
            Some(TokenType::CommentBlock) | Some(TokenType::CommentDoc)
        );
        let is_line = matches!(ty, Some(TokenType::CommentLine));
        if !is_block && !is_line {
            return false;
        }
        let interior_start = ts + 2;
        let interior_end = if is_block { te.saturating_sub(2) } else { te };
        if edit.start_byte < interior_start || edit.old_end_byte > interior_end {
            return false;
        }
        let mut rewritten = Vec::with_capacity(te - ts + inserted.len());
        rewritten.extend_from_slice(&delta.old_text[ts..edit.start_byte]);
        rewritten.extend_from_slice(inserted);
        rewritten.extend_from_slice(&delta.old_text[edit.old_end_byte..te]);
        if is_block {
            // The lexer searches for the closing `*/` from offset 2, so the comment stays one
            // comment exactly when its interior contains no `*/`.
            rewritten.len() >= 4
                && rewritten.starts_with(b"/*")
                && rewritten.ends_with(b"*/")
                && !rewritten[2..rewritten.len() - 2]
                    .windows(2)
                    .any(|w| w == b"*/")
        } else {
            rewritten.starts_with(b"//") && !rewritten.iter().any(|&b| b == b'\n' || b == b'\r')
        }
    }

    fn body_range(&self, old: &OldText<'_>, sym: u32) -> Option<(usize, usize)> {
        let def = self.snapshot.symbols.def_node(sym)?;
        let (first, last) = self.snapshot.ast.token_range(def)?;
        let (start, _) = self.token_bytes(old, first)?;
        let (_, end) = self.token_bytes(old, last)?;
        Some((start, end))
    }

    fn classify_edit(
        &self,
        delta: &SourceDelta,
        old: &OldText<'_>,
        edit: &TextEdit,
    ) -> (EditClass, RebuildScope) {
        if self.is_trivia_edit(delta, old, edit) {
            return (EditClass::Trivia, RebuildScope::Positional);
        }
        let from = old.position_of(edit.start_byte);
        let to = old.position_of(edit.old_end_byte);
        let entities = self
            .snapshot
            .navigation
            .entities_intersecting(delta.file_id, from, to);
        let Some(innermost) = entities.first() else {
            return (EditClass::OutsideDeclarations, RebuildScope::Full);
        };

        // The innermost callable whose body strictly contains the edit.
        let symbols = &self.snapshot.symbols;
        for span in &entities {
            let sym = span.sym_id;
            if !symbols.kind(sym).map(is_callable_kind).unwrap_or(false) {
                continue;
            }
            let Some((body_start, body_end)) = self.body_range(old, sym) else {
                continue;
            };
            // Strictly inside the braces: the first and last body tokens stay intact.
            if edit.start_byte > body_start && edit.old_end_byte < body_end {
                let mut methods = vec![sym];
                let mut cur = symbols.parent(sym);
                while let Some(p) = cur {
                    if symbols.kind(p).map(is_callable_kind).unwrap_or(false) {
                        methods.push(p);
                    }
                    cur = symbols.parent(p);
                    if methods.len() > symbols.len() {
                        break;
                    }
                }
                return (EditClass::Body { methods }, RebuildScope::Behavioral);
            }
        }
        (
            EditClass::Declaration {
                entity: innermost.sym_id,
            },
            RebuildScope::Structural,
        )
    }

    /// Classifies every edit of `delta`.
    pub fn classify(&self, delta: &SourceDelta) -> Vec<EditClassification> {
        if delta.is_identity() {
            return Vec::new();
        }
        let known_file = self.snapshot.files.content_hash(delta.file_id).is_some();
        let old = OldText {
            text: &delta.old_text,
            starts: line_starts(&delta.old_text),
        };
        delta
            .edits
            .iter()
            .map(|edit| {
                let (class, scope) =
                    if !known_file || !edit.is_valid_for(&delta.old_text, &delta.new_text) {
                        (EditClass::OutsideDeclarations, RebuildScope::Full)
                    } else {
                        self.classify_edit(delta, &old, edit)
                    };
                EditClassification {
                    edit: edit.clone(),
                    class,
                    scope,
                }
            })
            .collect()
    }
}
