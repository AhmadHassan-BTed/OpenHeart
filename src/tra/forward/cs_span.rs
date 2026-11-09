//! Call Site Span Index for Forward Queries (§7.2.2, §7.3).

use crate::core::types::cg::CallGraphArtifact;
use crate::core::types::token::unpack_sort_key;
use crate::ingestion::TokenCorpusArtifact;
use crate::tra::types::{BICsEntry, CallSiteSpanRecord};

pub struct CallSiteSpanIndex;

impl CallSiteSpanIndex {
    pub fn build(
        bi_cs: &[BICsEntry],
        tca: &TokenCorpusArtifact,
        cga: &CallGraphArtifact,
    ) -> Vec<CallSiteSpanRecord> {
        let mut records = Vec::with_capacity(cga.call_site_count as usize);

        for (cs_idx, entry) in bi_cs.iter().enumerate() {
            if entry.call_token == u32::MAX
                || (entry.call_token as usize) >= tca.token_records.len()
            {
                continue;
            }

            let start_tok = &tca.token_records[entry.call_token as usize];
            let (file_id, line_start, _) = unpack_sort_key(start_tok.sort_key);

            records.push(CallSiteSpanRecord {
                first_token_id: entry.call_token,
                call_site_id: cs_idx as u32,
                file_id,
                line_start,
            });
        }

        records.sort_unstable_by_key(|r| (r.file_id, r.first_token_id, r.call_site_id));
        records
    }

    /// O(log N + k) forward query: returns all call_site_ids at token_id in file_id.
    ///
    /// Records are sorted by `(file_id, first_token_id)`, so the matches form one contiguous
    /// run found by two binary searches.
    pub fn forward_cs_query(
        token_id: u32,
        file_id: u16,
        records: &[CallSiteSpanRecord],
    ) -> Vec<u32> {
        let lower =
            records.partition_point(|r| (r.file_id, r.first_token_id) < (file_id, token_id));
        let upper =
            records.partition_point(|r| (r.file_id, r.first_token_id) <= (file_id, token_id));
        records[lower..upper]
            .iter()
            .map(|r| r.call_site_id)
            .collect()
    }
}
