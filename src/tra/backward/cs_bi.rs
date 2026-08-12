//! Call Site Backward Index (§7.3, §7.4).

use crate::core::types::cg::CallGraphArtifact;
use crate::tra::types::BICsEntry;

pub struct CallSiteBackwardIndex;

impl CallSiteBackwardIndex {
    pub fn build(cga: &CallGraphArtifact) -> Vec<BICsEntry> {
        let mut bi_cs = Vec::with_capacity(cga.call_site_count as usize);

        for cs in &cga.call_sites {
            bi_cs.push(BICsEntry {
                call_token: cs.call_token,
            });
        }

        bi_cs
    }
}
