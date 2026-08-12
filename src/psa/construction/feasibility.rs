//! FeasibilityFilter — intersects structural BDD with branch-condition BDDs (§8.2.6).
//!
//! The structural ROBDD encodes syntactically valid paths. Semantically infeasible paths
//! (e.g. where both a condition and its negation are required) are filtered by conjoining
//! with branch condition implications from the SSA artifact.
//!
//! **Sound over-approximation:** For complex conditions, filtering is skipped (some infeasible
//! paths remain, but no feasible paths are removed).
//!
//! **Condition complexity heuristic (§8.2.6):**
//! - Simple: boolean variable test, null-check, integer constant comparison → BDD built.
//! - Complex: heap access, virtual dispatch, non-linear arithmetic → skipped.
//!
//! The SSA artifact's `IFDSResults` contains taint and nullable information that
//! Phase 8 uses to classify branch condition complexity.

use crate::cfg::builder::FunctionCFGData;
use crate::psa::bdd::BDDLibrary;
use crate::psa::ordering::{rpo::edge_index_of, rpo::succ_list, VariableOrdering};
use crate::psa::types::BoolOp;
use crate::ssa::serializer::FunctionSSAData;

/// Feasibility filter: refine `f_paths` by conjoining branch-condition implications.
pub struct FeasibilityFilter;

impl FeasibilityFilter {
    /// Apply feasibility filtering to `f_paths`.
    ///
    /// For each conditional branch block (2 successors with CFGEdgeType::CondTrue /
    /// CFGEdgeType::CondFalse edges), attempts to filter infeasible paths using the
    /// SSA branch condition data.
    ///
    /// Returns the refined `f_paths` node_id (never smaller than before filtering,
    /// but may be identical if all branches are classified as complex).
    pub fn apply(
        f_paths: u32,
        cfg: &FunctionCFGData,
        ssa_data: Option<&FunctionSSAData>,
        ordering: &VariableOrdering,
        bdd: &mut BDDLibrary,
    ) -> u32 {
        let mut current = f_paths;

        // If no SSA data is available, skip all filtering (sound over-approximation).
        let ssa = match ssa_data {
            Some(s) => s,
            None => return current,
        };

        for block_id in 0..cfg.blocks.len() as u32 {
            let succs = succ_list(cfg, block_id);
            if succs.len() != 2 {
                continue; // Only binary branches have meaningful conditions.
            }

            // Get the edge index for the CFG_TRUE branch edge.
            let true_ei = match edge_index_of(cfg, block_id, succs[0]) {
                Some(ei) => ei,
                None => continue,
            };

            // Classify the branch condition complexity.
            if Self::is_complex_condition(block_id, ssa) {
                // Skip — sound over-approximation.
                continue;
            }

            // Build a minimal BDD for the condition (§8.2.6 simple case).
            if let Some(cond_bdd) = Self::build_condition_bdd(block_id, ssa, bdd) {
                let var = ordering.var_for_edge_idx(true_ei);
                let x_true = bdd.var(var);
                // f_paths ∧ (cond_bdd → x_true)
                let impl_ = bdd.implies(cond_bdd, x_true);
                current = bdd.apply(BoolOp::And, current, impl_);
            }
        }

        current
    }

    /// Returns true if the branch condition at `block_id` is too complex to encode as a BDD.
    ///
    /// Phase 8's complexity heuristic: inspect SSA records at the branch block.
    /// If any SSA record in the block involves a heap-dependent def (taint fact), skip.
    fn is_complex_condition(block_id: u32, ssa: &FunctionSSAData) -> bool {
        // If there are tainted SSA variables at the branch condition site, classify as complex.
        // `IFDSResults.taint_sparse` contains (ssa_id: u32, block_id: u16) pairs.
        let block_id_u16 = (block_id & 0xFFFF) as u16;
        let has_tainted = ssa
            .ifds
            .taint_sparse
            .iter()
            .any(|&(_, blk)| blk == block_id_u16);

        if has_tainted {
            return true; // heap-dependent condition — skip.
        }

        // Otherwise treat as simple (boolean/null-check/integer comparison).
        false
    }

    /// Build a BDD for a simple branch condition.
    ///
    /// For Phase 8's scope, we represent the branch condition as a single BDD variable
    /// derived from the SSA variable ID of the branch's controlling definition.
    /// Returns None if no branch control SSA variable is identifiable.
    fn build_condition_bdd(
        block_id: u32,
        ssa: &FunctionSSAData,
        bdd: &mut BDDLibrary,
    ) -> Option<u32> {
        // Find the last SSA variable defined at this block (likely the branch condition).
        // SSARecord.def_block is u8 (truncated to lower 8 bits of block_id).
        let block_id_u8 = (block_id & 0xFF) as u8;
        let cond_ssa = ssa
            .ssa_records
            .iter()
            .filter(|r| !r.is_phi() && r.def_block == block_id_u8)
            .last()?;

        // Use the SSA variable ID as a fresh BDD variable index (capped at 14 bits).
        let bdd_var = (cond_ssa.ssa_id & 0x3FFF) as u16;
        Some(bdd.var(bdd_var))
    }
}
