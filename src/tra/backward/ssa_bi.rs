//! SSA Variable Backward Index (§7.3, §7.4).

use crate::ast::BPASTArtifact;
use crate::ssa::SSAArtifact;
use crate::tra::types::{BIBlkEntry, BISsaEntry};

pub struct SSABackwardIndex;

impl SSABackwardIndex {
    pub fn build(
        ssa: &SSAArtifact,
        bpa: &BPASTArtifact,
        bi_blk: &[BIBlkEntry],
    ) -> Vec<BISsaEntry> {
        let mut bi_ssa = Vec::with_capacity(ssa.total_ssa_vars as usize);

        let mut block_offset_map = Vec::new();
        let mut current_offset = 0u32;
        for func in &ssa.functions {
            block_offset_map.push(current_offset);
            let max_blk = func.ssa_records.iter().map(|v| v.def_block as u32).max().unwrap_or(0);
            current_offset += max_blk + 1;
        }

        for (f_idx, func) in ssa.functions.iter().enumerate() {
            let base_blk_id = block_offset_map.get(f_idx).copied().unwrap_or(0);

            for var in &func.ssa_records {
                let (ft, lt) = if var.def_stmt != u32::MAX && (var.def_stmt as usize) < bpa.node_count as usize {
                    bpa.token_range(var.def_stmt)
                } else {
                    let global_bid = (base_blk_id + var.def_block as u32) as usize;
                    if global_bid < bi_blk.len() {
                        (bi_blk[global_bid].first_token_id, bi_blk[global_bid].last_token_id)
                    } else {
                        (0, 0)
                    }
                };

                bi_ssa.push(BISsaEntry {
                    def_stmt: var.def_stmt,
                    first_token_id: ft,
                    last_token_id: lt,
                });
            }
        }

        bi_ssa
    }
}
