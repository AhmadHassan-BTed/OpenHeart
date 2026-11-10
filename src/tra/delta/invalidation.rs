//! Stale Detector for Incremental Invalidation (§7.2.3, §7.5.4).
//!
//! Staleness is decided per element: a record derived from symbol `s` of a baseline build is
//! stale in the current build iff the entity with `s`'s stable key is gone, its content
//! digest differs, or its declaration moved. The build tag `scpg_hash` carried by every
//! UMLLink only says *which build* produced a record; comparing it is all-or-nothing and is
//! exposed separately as [`StaleDetector::is_from_build`].

use std::collections::HashSet;

use crate::tra::delta::{DeltaApplicator, TraceabilityDelta};
use crate::tra::types::{TraceabilityArtifact, UMLLinkRecord};

pub struct StaleDetector;

impl StaleDetector {
    /// True iff `record` was produced by the build that produced `tra`.
    #[inline]
    pub fn is_from_build(record: &UMLLinkRecord, tra: &TraceabilityArtifact) -> bool {
        record.scpg_hash == tra.hashes.scpg_hash
    }

    /// True iff `record`, derived from `baseline`, is stale with respect to `current`.
    pub fn is_stale(
        record: &UMLLinkRecord,
        baseline: &TraceabilityArtifact,
        current: &TraceabilityArtifact,
    ) -> bool {
        let before = match baseline.entity(record.sym_id) {
            Some(e) => e,
            None => return true,
        };
        let after = current
            .entities
            .iter()
            .find(|e| e.stable_key == before.stable_key);
        match after {
            None => true,
            Some(e) => {
                e.content_digest != before.content_digest
                    || e.file_id != before.file_id
                    || e.line_start != before.line_start
                    || e.col_start != before.col_start
                    || e.line_end != before.line_end
            }
        }
    }

    /// The baseline sym_ids of the records in `records` that are stale in `current` (the
    /// records to regenerate), in record order, in O(|records| + |entities|).
    pub fn stale_ids(
        records: &[UMLLinkRecord],
        baseline: &TraceabilityArtifact,
        current: &TraceabilityArtifact,
    ) -> Vec<u32> {
        let delta = DeltaApplicator::compute_delta(baseline, current);
        let mut stale_old: HashSet<u32> = delta.removed.iter().copied().collect();
        for &(old, _) in delta.modified.iter().chain(delta.relocated.iter()) {
            stale_old.insert(old);
        }
        records
            .iter()
            .filter(|r| stale_old.contains(&r.sym_id) || baseline.entity(r.sym_id).is_none())
            .map(|r| r.sym_id)
            .collect()
    }

    /// The full entity-level delta between two builds.
    pub fn delta(
        baseline: &TraceabilityArtifact,
        current: &TraceabilityArtifact,
    ) -> TraceabilityDelta {
        DeltaApplicator::compute_delta(baseline, current)
    }
}
