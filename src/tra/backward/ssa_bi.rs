//! SSA Variable Backward Index (§7.3, §7.4).
//!
//! SSA ids are function-local (they start at 0 in every function), so the flat index is laid
//! out function by function in SSA order and each function's offset is recorded explicitly.
//!
//! A definition resolves to the token range of its defining AST statement. Definitions
//! without one (φ-nodes, implicit parameter definitions) resolve through their *block*,
//! using the CFA block offsets of the same function (not a guess derived from the largest
//! `def_block` seen):
//!
//! - φ-nodes use the exact `u32` block id of their `PhiRecord`;
//! - other definitions use `SSARecord::def_block`, which Phase 5 stores in 8 bits; it is
//!   only used when the function has at most 256 blocks, where it is exact. Otherwise the
//!   block is unknown and the owning function's span is used instead of an aliased block.

use std::collections::HashMap;

use crate::ast::BPASTArtifact;
use crate::ssa::SSAArtifact;
use crate::tra::backward::blk_bi::{owning_span, FunctionBlockRange};
use crate::tra::types::{BIBlkEntry, BISsaEntry, BISymEntry};

/// `(sym_id, first_ssa, ssa_count)` of one SSA function.
pub type FunctionSsaRange = (u32, u32, u32);

/// Number of distinct block ids representable by `SSARecord::def_block`.
const DEF_BLOCK_RANGE: u32 = 1 << 8;

pub struct SSABackwardIndex;

impl SSABackwardIndex {
    pub fn build(
        ssa: &SSAArtifact,
        bpa: &BPASTArtifact,
        bi_blk: &[BIBlkEntry],
        block_ranges: &[FunctionBlockRange],
        bi_sym: &[BISymEntry],
    ) -> (Vec<BISsaEntry>, Vec<FunctionSsaRange>) {
        let mut bi_ssa = Vec::with_capacity(ssa.total_ssa_vars as usize);
        let mut ranges = Vec::with_capacity(ssa.functions.len());

        let blocks_of: HashMap<u32, (u32, u32)> = block_ranges
            .iter()
            .map(|&(sym, first, count)| (sym, (first, count)))
            .collect();

        for func in &ssa.functions {
            let first_ssa = bi_ssa.len() as u32;
            let owner = owning_span(bi_sym, func.sym_id);
            let func_blocks = blocks_of.get(&func.sym_id).copied();
            let phi_block: HashMap<u32, u32> = func
                .phi_records
                .iter()
                .map(|p| (p.ssa_id, p.block_id))
                .collect();

            for var in &func.ssa_records {
                let from_stmt = if var.def_stmt != u32::MAX && var.def_stmt < bpa.node_count {
                    let (ft, lt) = bpa.token_range(var.def_stmt);
                    if ft != u32::MAX && lt >= ft {
                        Some((ft, lt))
                    } else {
                        None
                    }
                } else {
                    None
                };

                let (ft, lt) = match from_stmt {
                    Some(range) => range,
                    None => {
                        let local_block = if var.is_phi() {
                            phi_block.get(&var.ssa_id).copied()
                        } else {
                            match func_blocks {
                                Some((_, count)) if count <= DEF_BLOCK_RANGE => {
                                    Some(var.def_block as u32)
                                }
                                _ => None,
                            }
                        };
                        match (func_blocks, local_block) {
                            (Some((first, count)), Some(local)) if local < count => {
                                let entry = bi_blk[(first + local) as usize];
                                (entry.first_token_id, entry.last_token_id)
                            }
                            _ => owner,
                        }
                    }
                };

                bi_ssa.push(BISsaEntry {
                    def_stmt: var.def_stmt,
                    first_token_id: ft,
                    last_token_id: lt,
                });
            }
            ranges.push((func.sym_id, first_ssa, func.ssa_records.len() as u32));
        }

        (bi_ssa, ranges)
    }
}
