//! Delta Applicator & Incremental Invalidation (§7.5.4).

pub mod invalidation;

pub use invalidation::StaleDetector;

use crate::symbol::SymbolTableArtifact;
use crate::tra::types::{BISymEntry, TraceabilityArtifact};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceabilityDelta {
    pub new_scpg_hash: u32,
    pub old_scpg_hash: u32,
    pub invalidated_syms: Vec<u32>,
}

pub struct DeltaApplicator;

impl DeltaApplicator {
    pub fn compute_delta(
        old_tra: &TraceabilityArtifact,
        new_scpg_hash: u32,
        new_bi_sym: &[BISymEntry],
        new_sta: &SymbolTableArtifact,
    ) -> TraceabilityDelta {
        let mut invalidated = Vec::new();

        for s in 0..new_sta.symbol_records.len() {
            if s < old_tra.bi_sym.len() && s < new_bi_sym.len() {
                let old_entry = &old_tra.bi_sym[s];
                let new_entry = &new_bi_sym[s];
                if old_entry.decl_first_tok != new_entry.decl_first_tok
                    || old_entry.decl_last_tok != new_entry.decl_last_tok
                {
                    invalidated.push(s as u32);
                }
            } else {
                invalidated.push(s as u32);
            }
        }

        TraceabilityDelta {
            new_scpg_hash,
            old_scpg_hash: old_tra.hashes.scpg_hash,
            invalidated_syms: invalidated,
        }
    }
}
