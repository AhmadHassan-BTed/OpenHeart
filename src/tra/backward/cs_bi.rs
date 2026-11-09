//! Call Site Backward Index (§7.3, §7.4).
//!
//! `BI_cs` is indexed by `call_site_id`, not by the position of a site in
//! `CallGraphArtifact::call_sites`: Phase 6 may order its site vector by caller, and a
//! lookup `BI_cs[call_site_id]` must still land on the site that CGA edges name. Ids are
//! dense (`0..call_sites.len()`); a site whose id is out of range or repeated cannot be
//! placed and is reported, and a slot no site claims holds [`NO_SOURCE_TOKEN`].

use crate::core::types::cg::CallGraphArtifact;
use crate::tra::types::{BICsEntry, NO_SOURCE_TOKEN};

/// Sites that could not be placed in `BI_cs`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CallSiteIndexReport {
    /// Sites whose `call_site_id` is not below the number of sites.
    pub out_of_range: usize,
    /// Sites whose `call_site_id` was already claimed by an earlier site.
    pub duplicates: usize,
    /// Slots no site claimed.
    pub unclaimed: usize,
}

impl CallSiteIndexReport {
    /// True iff every site was placed and every slot is claimed.
    #[inline]
    pub fn is_clean(&self) -> bool {
        self.out_of_range == 0 && self.duplicates == 0 && self.unclaimed == 0
    }
}

pub struct CallSiteBackwardIndex;

impl CallSiteBackwardIndex {
    pub fn build(cga: &CallGraphArtifact) -> (Vec<BICsEntry>, CallSiteIndexReport) {
        let n = cga.call_sites.len();
        let mut bi_cs = vec![
            BICsEntry {
                call_token: NO_SOURCE_TOKEN,
            };
            n
        ];
        let mut claimed = vec![false; n];
        let mut report = CallSiteIndexReport::default();

        for cs in &cga.call_sites {
            let id = cs.call_site_id as usize;
            if id >= n {
                report.out_of_range += 1;
                continue;
            }
            if claimed[id] {
                report.duplicates += 1;
                continue;
            }
            claimed[id] = true;
            bi_cs[id] = BICsEntry {
                call_token: cs.call_token,
            };
        }
        report.unclaimed = claimed.iter().filter(|&&c| !c).count();

        (bi_cs, report)
    }
}
