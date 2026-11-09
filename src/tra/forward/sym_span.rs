//! Symbol Span Index for Forward Queries (§7.2.2, §7.5.2).
//!
//! Records are sorted by `(file_id, first_token_id, last_token_id descending, sym_id)` and
//! each stores the index of its innermost enclosing record (`parent`), computed with one
//! stack pass. Declaration spans of a well-formed file nest (package ⊃ class ⊃ method ⊃
//! local), so every span containing a token is an ancestor of the last span *starting* at
//! or before it:
//!
//! ```text
//! forward query(t): i = last record with first_token ≤ t   (binary search, O(log N))
//!                   walk i, parent(i), parent(parent(i)), … reporting spans with last ≥ t
//! ```
//!
//! which costs O(log N + d) for nesting depth d (results are a suffix of the walk), instead
//! of scanning every earlier span of the file. Files whose spans overlap without nesting are
//! marked [`PARENT_NOT_NESTED`] and answered by a scan of that file, which stays correct.

use crate::core::types::token::unpack_sort_key;
use crate::ingestion::TokenCorpusArtifact;
use crate::symbol::SymbolTableArtifact;
use crate::tra::types::{BISymEntry, SymbolSpanRecord, NO_PARENT, PARENT_NOT_NESTED};

pub struct SymbolSpanIndex;

impl SymbolSpanIndex {
    pub fn build(
        bi_sym: &[BISymEntry],
        tca: &TokenCorpusArtifact,
        sta: &SymbolTableArtifact,
    ) -> Vec<SymbolSpanRecord> {
        let mut records = Vec::with_capacity(sta.symbol_records.len());
        let n_tokens = tca.token_records.len();

        for sym_id in 0..sta.symbol_records.len().min(bi_sym.len()) {
            let entry = &bi_sym[sym_id];
            if entry.decl_first_tok == u32::MAX || (entry.decl_first_tok as usize) >= n_tokens {
                continue;
            }
            let last = entry.decl_last_tok.min(n_tokens as u32 - 1);
            if last < entry.decl_first_tok {
                continue; // empty span: nothing to locate
            }

            let start_tok = &tca.token_records[entry.decl_first_tok as usize];
            let end_tok = &tca.token_records[last as usize];
            let (file_id, line_start, col_start) = unpack_sort_key(start_tok.sort_key);
            let (_, line_end, _) = unpack_sort_key(end_tok.sort_key);

            records.push(SymbolSpanRecord {
                first_token_id: entry.decl_first_tok,
                last_token_id: last,
                sym_id: sym_id as u32,
                file_id,
                col_start,
                line_start,
                line_end,
                parent: NO_PARENT,
            });
        }

        records.sort_unstable_by(|a, b| {
            (a.file_id, a.first_token_id)
                .cmp(&(b.file_id, b.first_token_id))
                .then(b.last_token_id.cmp(&a.last_token_id))
                .then(a.sym_id.cmp(&b.sym_id))
        });
        Self::link_parents(&mut records);
        records
    }

    /// Fill `parent` with one stack pass per file; mark non-nested files.
    fn link_parents(records: &mut [SymbolSpanRecord]) {
        let mut file_start = 0usize;
        while file_start < records.len() {
            let file_id = records[file_start].file_id;
            let mut file_end = file_start;
            while file_end < records.len() && records[file_end].file_id == file_id {
                file_end += 1;
            }

            let mut stack: Vec<usize> = Vec::new();
            let mut nested = true;
            for i in file_start..file_end {
                let (first, last) = (records[i].first_token_id, records[i].last_token_id);
                while let Some(&top) = stack.last() {
                    if records[top].last_token_id < first {
                        stack.pop();
                    } else {
                        break;
                    }
                }
                match stack.last() {
                    Some(&top) if records[top].last_token_id >= last => {
                        records[i].parent = top as u32;
                    }
                    Some(_) => {
                        nested = false;
                        break;
                    }
                    None => records[i].parent = NO_PARENT,
                }
                stack.push(i);
            }
            if !nested {
                for r in &mut records[file_start..file_end] {
                    r.parent = PARENT_NOT_NESTED;
                }
            }
            file_start = file_end;
        }
    }

    /// O(log N + d) forward query: all sym_ids whose spans contain `token_id` in `file_id`,
    /// innermost first.
    pub fn forward_sym_query(
        token_id: u32,
        file_id: u16,
        records: &[SymbolSpanRecord],
    ) -> Vec<u32> {
        let candidate = match Self::last_starting_at_or_before(token_id, file_id, records) {
            Some(i) => i,
            None => return Vec::new(),
        };
        if records[candidate].parent == PARENT_NOT_NESTED {
            return Self::scan_file(token_id, file_id, candidate, records);
        }

        let mut results = Vec::new();
        let mut i = candidate;
        let mut steps = 0usize;
        loop {
            let r = &records[i];
            if r.last_token_id >= token_id {
                results.push(r.sym_id);
            }
            steps += 1;
            // Parents always precede their children; a larger index or a cycle means the
            // links are corrupt — fall back to the scan, which needs no links.
            if r.parent == NO_PARENT {
                break;
            }
            if r.parent as usize >= i || steps > records.len() {
                return Self::scan_file(token_id, file_id, candidate, records);
            }
            i = r.parent as usize;
        }
        results
    }

    /// The innermost symbol whose span contains `token_id` in `file_id`.
    pub fn innermost_sym(token_id: u32, file_id: u16, records: &[SymbolSpanRecord]) -> Option<u32> {
        Self::forward_sym_query(token_id, file_id, records)
            .first()
            .copied()
    }

    /// Index of the last record of `file_id` with `first_token_id <= token_id`.
    fn last_starting_at_or_before(
        token_id: u32,
        file_id: u16,
        records: &[SymbolSpanRecord],
    ) -> Option<usize> {
        let upper =
            records.partition_point(|r| (r.file_id, r.first_token_id) <= (file_id, token_id));
        if upper == 0 || records[upper - 1].file_id != file_id {
            return None;
        }
        Some(upper - 1)
    }

    /// Scan the file's records at or before `candidate` (correct for any span layout).
    fn scan_file(
        token_id: u32,
        file_id: u16,
        candidate: usize,
        records: &[SymbolSpanRecord],
    ) -> Vec<u32> {
        let mut results = Vec::new();
        let mut i = candidate + 1;
        while i > 0 {
            i -= 1;
            let r = &records[i];
            if r.file_id != file_id {
                break;
            }
            if r.last_token_id >= token_id {
                results.push(r.sym_id);
            }
        }
        results
    }
}
