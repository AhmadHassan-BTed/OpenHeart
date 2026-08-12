//! Basic Block Backward Index (§7.3, §7.4, Invariant 1).

use crate::cfg::serializer::CFGArtifact;
use crate::tra::types::BIBlkEntry;

pub struct BlockBackwardIndex;

impl BlockBackwardIndex {
    pub fn build(cfa: &CFGArtifact) -> Vec<BIBlkEntry> {
        let mut bi_blk = Vec::with_capacity(cfa.total_blocks as usize);

        for func in &cfa.functions {
            for blk in &func.blocks {
                let mut ft = blk.first_token;
                let mut lt = blk.last_token;

                // Invariant 1 guarantee: fallback for synthetic/empty basic blocks
                if ft == u32::MAX {
                    ft = 0;
                    lt = 0;
                }

                bi_blk.push(BIBlkEntry {
                    first_token_id: ft,
                    last_token_id: lt,
                });
            }
        }

        bi_blk
    }
}
