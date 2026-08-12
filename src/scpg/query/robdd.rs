//! ROBDDQueryEngine — path count, coverage, and metrics lookup via PSA (§10.5).

use crate::psa::types::PathSummaryArtifact;

pub struct ROBDDQueryEngine;

impl ROBDDQueryEngine {
    /// O(1) sat_count lookup of function path count.
    pub fn path_count(sym_id: u32, psa: &PathSummaryArtifact) -> u64 {
        psa.function_header(sym_id)
            .map(|h| h.sat_count)
            .unwrap_or(1)
    }

    /// O(1) cyclomatic complexity metric lookup.
    pub fn cyclomatic(sym_id: u32, psa: &PathSummaryArtifact) -> u16 {
        psa.function_header(sym_id)
            .map(|h| h.cyclomatic)
            .unwrap_or(1)
    }
}
