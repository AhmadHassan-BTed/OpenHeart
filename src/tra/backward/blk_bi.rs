//! Basic Block Backward Index (§7.3, §7.4, Invariant 1).
//!
//! Blocks are laid out function by function in CFA order; each function's offset is the
//! prefix sum of the preceding functions' block counts and is recorded explicitly, so a
//! `(sym_id, local block id)` pair resolves in O(log F) without guessing.
//!
//! Synthetic and empty blocks (ENTRY, EXIT, join blocks) have no tokens of their own. They
//! are mapped to their owning function's source span (definition, else declaration) rather
//! than to token 0 of file 0, which is a real but unrelated location; a function without
//! any span yields [`NO_SOURCE_TOKEN`].

use crate::cfg::serializer::CFGArtifact;
use crate::tra::types::{BIBlkEntry, BISymEntry, NO_SOURCE_TOKEN};

/// `(sym_id, first_block, block_count)` of one CFA function.
pub type FunctionBlockRange = (u32, u32, u32);

pub struct BlockBackwardIndex;

impl BlockBackwardIndex {
    pub fn build(
        cfa: &CFGArtifact,
        bi_sym: &[BISymEntry],
    ) -> (Vec<BIBlkEntry>, Vec<FunctionBlockRange>) {
        let mut bi_blk = Vec::with_capacity(cfa.total_blocks as usize);
        let mut ranges = Vec::with_capacity(cfa.functions.len());

        for func in &cfa.functions {
            let first_block = bi_blk.len() as u32;
            let owner = owning_span(bi_sym, func.sym_id);
            for blk in &func.blocks {
                let has_tokens = blk.first_token != u32::MAX && blk.last_token >= blk.first_token;
                let (ft, lt) = if has_tokens {
                    (blk.first_token, blk.last_token)
                } else {
                    owner
                };
                bi_blk.push(BIBlkEntry {
                    first_token_id: ft,
                    last_token_id: lt,
                });
            }
            ranges.push((func.sym_id, first_block, func.blocks.len() as u32));
        }

        (bi_blk, ranges)
    }
}

/// The token range of function `sym_id`: its definition span, else its declaration span,
/// else `(NO_SOURCE_TOKEN, NO_SOURCE_TOKEN)`.
pub fn owning_span(bi_sym: &[BISymEntry], sym_id: u32) -> (u32, u32) {
    match bi_sym.get(sym_id as usize) {
        Some(e) if e.def_first_tok != u32::MAX && e.def_last_tok >= e.def_first_tok => {
            (e.def_first_tok, e.def_last_tok)
        }
        Some(e) if e.decl_first_tok != u32::MAX && e.decl_last_tok >= e.decl_first_tok => {
            (e.decl_first_tok, e.decl_last_tok)
        }
        _ => (NO_SOURCE_TOKEN, NO_SOURCE_TOKEN),
    }
}
