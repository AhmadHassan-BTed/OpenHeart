//! Stale Detector for Incremental Invalidation (§7.2.3, §7.5.4).

use crate::tra::types::UMLLinkRecord;

pub struct StaleDetector;

impl StaleDetector {
    /// Returns true if a UMLLinkRecord is stale based on composite scpg_hash comparison.
    #[inline(always)]
    pub fn is_stale(record: &UMLLinkRecord, current_scpg_hash: u32) -> bool {
        record.scpg_hash != current_scpg_hash
    }

    /// Filters all stale records out of a list of UMLLinkRecords.
    pub fn filter_stale(records: &[UMLLinkRecord], current_scpg_hash: u32) -> Vec<u32> {
        records
            .iter()
            .filter(|r| Self::is_stale(r, current_scpg_hash))
            .map(|r| r.sym_id)
            .collect()
    }
}
