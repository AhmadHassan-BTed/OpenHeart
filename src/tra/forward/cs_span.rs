//! Call Site Span Index for Forward Queries (§7.2.2, §7.3).
//!
//! One record per call site with a valid call token, carrying the site's own
//! `call_site_id` (never its position in the CGA vector), sorted by
//! `(file_id, first_token_id, call_site_id)`.

use crate::core::types::cg::CallGraphArtifact;
use crate::ingestion::TokenCorpusArtifact;
use crate::tra::position::token_start;
use crate::tra::types::CallSiteSpanRecord;

pub struct CallSiteSpanIndex;

impl CallSiteSpanIndex {
    pub fn build(cga: &CallGraphArtifact, tca: &TokenCorpusArtifact) -> Vec<CallSiteSpanRecord> {
        let n_tokens = tca.token_records.len();
        let mut records = Vec::with_capacity(cga.call_sites.len());

        for cs in &cga.call_sites {
            if cs.call_token == u32::MAX || (cs.call_token as usize) >= n_tokens {
                continue;
            }
            let (file_id, line_start, _) = token_start(tca, cs.call_token);
            records.push(CallSiteSpanRecord {
                first_token_id: cs.call_token,
                call_site_id: cs.call_site_id,
                file_id,
                line_start,
            });
        }

        records.sort_unstable_by_key(|r| (r.file_id, r.first_token_id, r.call_site_id));
        records.dedup();
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
