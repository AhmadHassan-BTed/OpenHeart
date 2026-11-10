//! NavigationEngine — bidirectional source <-> entity navigation via TRA (§10.5).
//!
//! `NavigationIndex` is the indexed form used by the engine:
//! - entity → source: a dense `sym_id → slot` array, O(1);
//! - source → entities: spans sorted by start inside each file plus a max-end segment tree,
//!   giving every span that contains a position in O((k + 1) · log n) for k results, with the
//!   innermost entity first. This is exact for arbitrary (also non-nested) spans.

use serde::Serialize;

use crate::scpg::query::source::{position_key, FileTable, TokenIndex};
use crate::scpg::serializer::codec::{SectionDecoder, SectionEncoder};
use crate::scpg::types::ScpgFormatError;
use crate::tra::types::{SourceRange, TraceabilityArtifact, UMLLinkRecord};

const NO_SLOT: u32 = u32::MAX;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntitySpan {
    pub sym_id: u32,
    pub file_id: u16,
    pub line_start: u32,
    pub col_start: u16,
    pub line_end: u32,
    /// Exclusive end column.
    pub col_end: u16,
    pub first_token: u32,
    pub last_token: u32,
}

impl EntitySpan {
    pub fn start_key(&self) -> u64 {
        position_key(self.line_start, self.col_start)
    }

    pub fn end_key(&self) -> u64 {
        position_key(self.line_end, self.col_end)
    }

    fn extent(&self) -> u64 {
        self.end_key().saturating_sub(self.start_key())
    }

    pub fn contains(&self, line: u32, col: u16) -> bool {
        let p = position_key(line, col);
        self.start_key() <= p && p < self.end_key()
    }

    pub fn range(&self) -> SourceRange {
        SourceRange {
            file_id: self.file_id,
            line_start: self.line_start,
            col_start: self.col_start,
            line_end: self.line_end,
            col_end: self.col_end,
        }
    }
}

/// A resolved, user-facing source location stamped with the snapshot's short hash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SourceLocation {
    pub sym_id: u32,
    pub file_id: u16,
    pub path: Option<String>,
    pub line_start: u32,
    pub col_start: u16,
    pub line_end: u32,
    pub col_end: u16,
    pub first_token: u32,
    pub last_token: u32,
    pub scpg_hash: u32,
}

#[derive(Debug, Clone, Default)]
pub struct NavigationIndex {
    spans: Vec<EntitySpan>,
    slot_of: Vec<u32>,
    tree_cap: usize,
    max_end: Vec<u64>,
}

impl NavigationIndex {
    /// Builds spans from the TRA symbol backward index (declaration span, falling back to the
    /// definition span) resolved through real token positions. Symbols whose tokens cannot be
    /// resolved get no span instead of a fabricated `0:0` range.
    pub fn build(tra: &TraceabilityArtifact, tokens: &TokenIndex, symbol_count: usize) -> Self {
        let mut spans = Vec::new();
        for sym in 0..symbol_count.min(tra.bi_sym.len()) {
            let e = &tra.bi_sym[sym];
            let candidates = [
                (e.decl_first_tok, e.decl_last_tok),
                (e.def_first_tok, e.def_last_tok),
            ];
            for (first, last) in candidates {
                if let Some(span) = span_from_tokens(sym as u32, first, last, tokens) {
                    spans.push(span);
                    break;
                }
            }
        }
        Self::from_spans(spans, symbol_count)
    }

    fn from_spans(mut spans: Vec<EntitySpan>, symbol_count: usize) -> Self {
        spans.sort_by(|a, b| {
            a.file_id
                .cmp(&b.file_id)
                .then(a.start_key().cmp(&b.start_key()))
                .then(b.end_key().cmp(&a.end_key()))
                .then(a.sym_id.cmp(&b.sym_id))
        });
        let mut slot_of = vec![NO_SLOT; symbol_count];
        for (i, s) in spans.iter().enumerate() {
            if let Some(slot) = slot_of.get_mut(s.sym_id as usize) {
                *slot = i as u32;
            }
        }
        let tree_cap = spans.len().next_power_of_two().max(1);
        let mut max_end = vec![0u64; 2 * tree_cap];
        for (i, s) in spans.iter().enumerate() {
            max_end[tree_cap + i] = s.end_key();
        }
        for i in (1..tree_cap).rev() {
            max_end[i] = max_end[2 * i].max(max_end[2 * i + 1]);
        }
        Self {
            spans,
            slot_of,
            tree_cap,
            max_end,
        }
    }

    pub fn len(&self) -> usize {
        self.spans.len()
    }

    pub fn is_empty(&self) -> bool {
        self.spans.is_empty()
    }

    pub fn spans(&self) -> &[EntitySpan] {
        &self.spans
    }

    /// O(1) entity → source span.
    pub fn to_source(&self, sym_id: u32) -> Option<&EntitySpan> {
        let slot = *self.slot_of.get(sym_id as usize)?;
        if slot == NO_SLOT {
            None
        } else {
            self.spans.get(slot as usize)
        }
    }

    pub fn location(
        &self,
        sym_id: u32,
        files: &FileTable,
        scpg_hash: u32,
    ) -> Option<SourceLocation> {
        let s = self.to_source(sym_id)?;
        Some(SourceLocation {
            sym_id,
            file_id: s.file_id,
            path: files.path(s.file_id).map(|p| p.to_string()),
            line_start: s.line_start,
            col_start: s.col_start,
            line_end: s.line_end,
            col_end: s.col_end,
            first_token: s.first_token,
            last_token: s.last_token,
            scpg_hash,
        })
    }

    /// A UMLLink for `sym_id` stamped with the engine's own short hash, so links handed out by
    /// the engine can be checked against `OpenHeartEngine::scpg_hash()`.
    pub fn link_record(&self, sym_id: u32, sym_kind: u8, scpg_hash: u32) -> Option<UMLLinkRecord> {
        let s = self.to_source(sym_id)?;
        Some(UMLLinkRecord {
            sym_id,
            file_id: s.file_id,
            line_start: s.line_start,
            col_start: s.col_start,
            line_end: s.line_end,
            col_end: s.col_end,
            scpg_hash,
            sym_kind,
            _reserved: [0; 3],
        })
    }

    fn file_range(&self, file_id: u16) -> (usize, usize) {
        let lo = self.spans.partition_point(|s| s.file_id < file_id);
        let hi = self.spans.partition_point(|s| s.file_id <= file_id);
        (lo, hi.max(lo))
    }

    /// Collects the indices in `[lo, hi)` whose end key is strictly greater than `threshold`.
    fn collect_ending_after(&self, lo: usize, hi: usize, threshold: u64, out: &mut Vec<usize>) {
        if lo >= hi {
            return;
        }
        let mut stack = vec![(1usize, 0usize, self.tree_cap)];
        while let Some((node, nl, nr)) = stack.pop() {
            if nr <= lo || nl >= hi || self.max_end[node] <= threshold {
                continue;
            }
            if nr - nl == 1 {
                out.push(nl);
            } else {
                let mid = nl + (nr - nl) / 2;
                stack.push((2 * node + 1, mid, nr));
                stack.push((2 * node, nl, mid));
            }
        }
    }

    fn innermost_first(&self, mut indices: Vec<usize>) -> Vec<EntitySpan> {
        indices.sort_by(|&a, &b| {
            let (sa, sb) = (&self.spans[a], &self.spans[b]);
            sa.extent()
                .cmp(&sb.extent())
                .then(sb.start_key().cmp(&sa.start_key()))
                .then(sa.sym_id.cmp(&sb.sym_id))
        });
        indices.into_iter().map(|i| self.spans[i]).collect()
    }

    /// Every entity whose span contains `(line, col)`, innermost first.
    pub fn entities_at(&self, file_id: u16, line: u32, col: u16) -> Vec<EntitySpan> {
        let p = position_key(line, col);
        let (lo, hi) = self.file_range(file_id);
        let prefix_end = lo + self.spans[lo..hi].partition_point(|s| s.start_key() <= p);
        let mut out = Vec::new();
        self.collect_ending_after(lo, prefix_end, p, &mut out);
        self.innermost_first(out)
    }

    pub fn innermost_at(&self, file_id: u16, line: u32, col: u16) -> Option<EntitySpan> {
        self.entities_at(file_id, line, col).into_iter().next()
    }

    /// Every entity whose span intersects `line`, innermost first.
    pub fn entities_on_line(&self, file_id: u16, line: u32) -> Vec<EntitySpan> {
        let line_last = position_key(line, u16::MAX);
        let line_first = position_key(line, 0);
        let (lo, hi) = self.file_range(file_id);
        let prefix_end = lo + self.spans[lo..hi].partition_point(|s| s.start_key() <= line_last);
        let mut out = Vec::new();
        self.collect_ending_after(lo, prefix_end, line_first, &mut out);
        self.innermost_first(out)
    }

    /// Every entity whose span intersects the position range `[from, to)` of a file.
    pub fn entities_intersecting(
        &self,
        file_id: u16,
        from: (u32, u16),
        to: (u32, u16),
    ) -> Vec<EntitySpan> {
        let lo_key = position_key(from.0, from.1);
        let hi_key = position_key(to.0, to.1).max(lo_key.saturating_add(1));
        let (lo, hi) = self.file_range(file_id);
        let prefix_end = lo + self.spans[lo..hi].partition_point(|s| s.start_key() < hi_key);
        let mut out = Vec::new();
        self.collect_ending_after(lo, prefix_end, lo_key, &mut out);
        self.innermost_first(out)
    }

    pub fn encode(&self, enc: &mut SectionEncoder) {
        enc.put_u64(self.slot_of.len() as u64);
        enc.put_u32s(&self.spans.iter().map(|s| s.sym_id).collect::<Vec<_>>());
        enc.put_u16s(&self.spans.iter().map(|s| s.file_id).collect::<Vec<_>>());
        enc.put_u32s(&self.spans.iter().map(|s| s.line_start).collect::<Vec<_>>());
        enc.put_u16s(&self.spans.iter().map(|s| s.col_start).collect::<Vec<_>>());
        enc.put_u32s(&self.spans.iter().map(|s| s.line_end).collect::<Vec<_>>());
        enc.put_u16s(&self.spans.iter().map(|s| s.col_end).collect::<Vec<_>>());
        enc.put_u32s(&self.spans.iter().map(|s| s.first_token).collect::<Vec<_>>());
        enc.put_u32s(&self.spans.iter().map(|s| s.last_token).collect::<Vec<_>>());
    }

    pub fn decode(
        dec: &mut SectionDecoder<'_>,
        symbol_count: usize,
    ) -> Result<Self, ScpgFormatError> {
        let stored_symbols = dec.u64()? as usize;
        if stored_symbols != symbol_count {
            return Err(dec.malformed(format!(
                "navigation covers {} symbols, symbol table has {}",
                stored_symbols, symbol_count
            )));
        }
        let sym_ids = dec.u32s()?;
        let file_ids = dec.u16s()?;
        let line_starts = dec.u32s()?;
        let col_starts = dec.u16s()?;
        let line_ends = dec.u32s()?;
        let col_ends = dec.u16s()?;
        let first_tokens = dec.u32s()?;
        let last_tokens = dec.u32s()?;
        let n = sym_ids.len();
        dec.expect_len("file_ids", file_ids.len(), n)?;
        dec.expect_len("line_starts", line_starts.len(), n)?;
        dec.expect_len("col_starts", col_starts.len(), n)?;
        dec.expect_len("line_ends", line_ends.len(), n)?;
        dec.expect_len("col_ends", col_ends.len(), n)?;
        dec.expect_len("first_tokens", first_tokens.len(), n)?;
        dec.expect_len("last_tokens", last_tokens.len(), n)?;
        if sym_ids.iter().any(|&s| s as usize >= symbol_count) {
            return Err(dec.malformed("navigation span references an unknown symbol"));
        }
        let spans = (0..n)
            .map(|i| EntitySpan {
                sym_id: sym_ids[i],
                file_id: file_ids[i],
                line_start: line_starts[i],
                col_start: col_starts[i],
                line_end: line_ends[i],
                col_end: col_ends[i],
                first_token: first_tokens[i],
                last_token: last_tokens[i],
            })
            .collect();
        Ok(Self::from_spans(spans, symbol_count))
    }
}

fn span_from_tokens(sym_id: u32, first: u32, last: u32, tokens: &TokenIndex) -> Option<EntitySpan> {
    if first == u32::MAX || last == u32::MAX || last < first {
        return None;
    }
    let start = tokens.start(first)?;
    let end = tokens.end(last)?;
    if start.file_id != end.file_id || start.line == 0 {
        return None;
    }
    if position_key(end.line, end.col) <= position_key(start.line, start.col) {
        return None;
    }
    Some(EntitySpan {
        sym_id,
        file_id: start.file_id,
        line_start: start.line,
        col_start: start.col,
        line_end: end.line,
        col_end: end.col,
        first_token: first,
        last_token: last,
    })
}

/// Artifact-level navigation over the TRA UMLLink table, for callers that hold a
/// `TraceabilityArtifact` but no engine. Indexed lookups live in `NavigationIndex`.
pub struct NavigationEngine;

impl NavigationEngine {
    /// Entity -> source range. `UMLLinkRegistry` emits links sorted by `sym_id`, so this is a
    /// binary search, O(log n). Returns `None` for symbols without a UMLLink and for links whose
    /// span Phase 7 could not resolve (recorded as line 0).
    pub fn to_source(sym_id: u32, tra: &TraceabilityArtifact) -> Option<SourceRange> {
        let idx = tra
            .uml_links
            .binary_search_by_key(&sym_id, |l| l.sym_id)
            .ok()?;
        let link = &tra.uml_links[idx];
        if link.line_start == 0 && link.line_end == 0 {
            return None;
        }
        Some(SourceRange {
            file_id: link.file_id,
            line_start: link.line_start,
            col_start: link.col_start,
            line_end: link.line_end,
            col_end: link.col_end,
        })
    }

    /// Source line -> UMLLinks covering it, innermost first. This artifact-level form scans
    /// the link table, O(n); `NavigationIndex::entities_on_line` is the O(log n + k) form.
    pub fn from_source(file_id: u16, line: u32, tra: &TraceabilityArtifact) -> Vec<UMLLinkRecord> {
        let mut links: Vec<UMLLinkRecord> = tra
            .uml_links
            .iter()
            .filter(|l| {
                l.line_start != 0
                    && l.file_id == file_id
                    && l.line_start <= line
                    && l.line_end >= line
            })
            .cloned()
            .collect();
        links.sort_by(|a, b| {
            let ea = position_key(a.line_end, a.col_end)
                .saturating_sub(position_key(a.line_start, a.col_start));
            let eb = position_key(b.line_end, b.col_end)
                .saturating_sub(position_key(b.line_start, b.col_start));
            ea.cmp(&eb).then(a.sym_id.cmp(&b.sym_id))
        });
        links
    }
}
