//! Symbol Span Index for Forward Queries (§7.2.2, §7.5.2).

use crate::core::types::token::unpack_sort_key;
use crate::ingestion::TokenCorpusArtifact;
use crate::symbol::SymbolTableArtifact;
use crate::tra::types::{BISymEntry, SymbolSpanRecord};

pub struct SymbolSpanIndex;

impl SymbolSpanIndex {
    pub fn build(
        bi_sym: &[BISymEntry],
        tca: &TokenCorpusArtifact,
        sta: &SymbolTableArtifact,
    ) -> Vec<SymbolSpanRecord> {
        let mut records = Vec::with_capacity(sta.symbol_records.len());

        for sym_id in 0..sta.symbol_records.len() {
            let entry = &bi_sym[sym_id];
            if entry.decl_first_tok == u32::MAX
                || (entry.decl_first_tok as usize) >= tca.token_records.len()
            {
                continue;
            }

            let start_tok = &tca.token_records[entry.decl_first_tok as usize];
            let end_tok = &tca.token_records
                [entry.decl_last_tok.min(tca.token_records.len() as u32 - 1) as usize];

            let (file_id, line_start, col_start) = unpack_sort_key(start_tok.sort_key);
            let (_, line_end, _) = unpack_sort_key(end_tok.sort_key);

            records.push(SymbolSpanRecord {
                first_token_id: entry.decl_first_tok,
                last_token_id: entry.decl_last_tok,
                sym_id: sym_id as u32,
                file_id,
                line_start: line_start as u16,
                col_start,
                line_end: line_end as u16,
            });
        }

        // Sort by (file_id, first_token_id) for binary search
        records.sort_unstable_by_key(|r| (r.file_id, r.first_token_id));
        records
    }

    /// O(log N + k) forward query: returns all sym_ids spanning given token_id in file_id.
    pub fn forward_sym_query(
        token_id: u32,
        file_id: u16,
        records: &[SymbolSpanRecord],
    ) -> Vec<u32> {
        let upper =
            records.partition_point(|r| (r.file_id, r.first_token_id) <= (file_id, token_id));

        let mut results = Vec::new();
        let mut i = upper;
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
