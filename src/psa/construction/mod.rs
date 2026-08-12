//! FunctionROBDDBuilder — the full 8-step ROBDD construction pipeline (§8.5.2, §8.3).
//!
//! Implements the exact 8-step pipeline per §8.5.2:
//! ```text
//! Step 1: Compute initial variable ordering (RPO of CFG edges)
//! Step 2: Refine with FORCE algorithm (hypergraph gravity-center iteration)
//! Step 3: Build the VariableOrdering (edge_idx ↔ var_index bijection)
//! Step 4: Construct structural constraints Φ_b and conjunct into f_paths
//! Step 5: Apply entry constraint (entry edge always taken: restrict f_paths, entry_var, 1)
//! Step 6: Feasibility filtering from SSA branch conditions (sound over-approximation)
//! Step 7: Sifting optimization (Rudell's per-variable local sift)
//! Step 8: Compute path metrics (sat_count, cyclomatic, max_path_len)
//! ```

pub mod constraints;
pub mod feasibility;
pub mod recursive;

use crate::cfg::builder::FunctionCFGData;
use crate::psa::bdd::BDDLibrary;
use crate::psa::ordering::{
    force::{build_constraint_hyperedges, force_ordering},
    rpo::{edge_index_of, rpo_edge_ordering, rpo_positions, succ_list},
    sifting::SiftingOptimizer,
    VariableOrdering,
};
use crate::psa::types::{BoolOp, FunctionROBDD};
use crate::ssa::serializer::FunctionSSAData;

use self::constraints::build_phi_b;
use self::feasibility::FeasibilityFilter;
use self::recursive::RecursiveHandler;

/// Builds the ROBDD for a single function.
///
/// Called from `Phase8Stage::run()` in SCC topological order (callees first).
pub struct FunctionROBDDBuilder;

impl FunctionROBDDBuilder {
    /// Build the ROBDD and path metrics for one function.
    ///
    /// # Parameters
    /// - `sym_id`: function symbol ID from STA.
    /// - `cfg`: CFG data for this function from the CFA artifact.
    /// - `ssa_data`: optional SSA data for feasibility filtering.
    /// - `is_recursive`: true iff this function's SCC has `scc_class ≥ 1`.
    pub fn build(
        sym_id: u32,
        cfg: &FunctionCFGData,
        ssa_data: Option<&FunctionSSAData>,
        is_recursive: bool,
    ) -> FunctionROBDD {
        let mut bdd = BDDLibrary::new();

        // ── Step 1: Compute initial variable ordering (RPO of CFG edges) ─────
        let rpo_order = rpo_edge_ordering(cfg);
        let n_vars = rpo_order.len();

        if n_vars == 0 {
            // Empty function (no edges) — trivially TRUE, 1 path, V(G)=1.
            return FunctionROBDD {
                sym_id,
                ordering: VariableOrdering::from_order(&[], cfg),
                nodes: bdd.into_nodes(),
                root: 1, // TRUE
                sat_count: 1,
                cyclomatic: 1,
                max_path_len: 1,
                unwind_depth: 0,
            };
        }

        // ── Step 2: Refine with FORCE algorithm ───────────────────────────────
        let hyperedges = build_constraint_hyperedges(cfg);
        let init_pos = rpo_positions(&rpo_order);
        let force_order = force_ordering(n_vars, &hyperedges, init_pos);

        // ── Step 3: Build the VariableOrdering ───────────────────────────────
        let mut ordering = VariableOrdering::from_order(&force_order, cfg);

        // ── Step 4: Construct structural constraints Φ_b for all blocks ──────
        // f_paths starts as TRUE (conjunction identity element).
        let mut f_paths = bdd.true_id();
        for block_id in 0..cfg.blocks.len() as u32 {
            let phi_b = build_phi_b(block_id, cfg, &ordering, &mut bdd);
            f_paths = bdd.apply(BoolOp::And, f_paths, phi_b);
        }

        // ── Step 5: Apply entry constraint (entry edge always taken) ──────────
        // Block 0 = ENTRY in all OpenHeart CFGs. Its single outgoing edge is always taken.
        let entry_succs = succ_list(cfg, 0);
        if !entry_succs.is_empty() {
            if let Some(entry_ei) = edge_index_of(cfg, 0, entry_succs[0]) {
                let entry_var = ordering.var_for_edge_idx(entry_ei);
                f_paths = bdd.restrict(f_paths, entry_var, 1);
            }
        }

        // ── Step 6: Feasibility filtering from SSA branch conditions ──────────
        f_paths = FeasibilityFilter::apply(f_paths, cfg, ssa_data, &ordering, &mut bdd);

        // ── Step 7: Sifting optimization ──────────────────────────────────────
        SiftingOptimizer::optimize(&mut bdd, &mut f_paths, &mut ordering);

        // ── Step 8: Compute metrics ────────────────────────────────────────────
        let n_vars_u16 = ordering.n_vars();

        // #SAT = number of feasible execution paths (§8.2.5).
        let sat = bdd.sat_count(f_paths, n_vars_u16);

        // V(G) = |E| - |B| + 2. Stored as u16; already computed by Phase 4 in cfg.cyclomatic.
        // We use the Phase 4 value directly for consistency (§8.2.5: "computed from CFA metadata").
        let cyclomatic = cfg.cyclomatic;

        // Longest path via bounded DFS.
        let max_path_len = compute_max_path_length(cfg);

        // Handle recursive SCC (§8.2.6): record bounded unwinding depth.
        let (f_paths, unwind_depth) = if is_recursive {
            RecursiveHandler::apply(f_paths)
        } else {
            (f_paths, 0u16)
        };

        // ── Invariant 3 check (§8.8): ROBDD canonicity ────────────────────────
        #[cfg(debug_assertions)]
        {
            if let Err(msg) = bdd.verify_canonicity() {
                panic!(
                    "Phase 8 Invariant 3 (Canonicity) violated for sym_id={}: {}",
                    sym_id, msg
                );
            }
        }

        FunctionROBDD {
            sym_id,
            ordering,
            nodes: bdd.into_nodes(),
            root: f_paths,
            sat_count: sat,
            cyclomatic,
            max_path_len,
            unwind_depth,
        }
    }
}

/// Compute the length of the longest path through the CFG (§8.5.2 Step 8).
///
/// Returns the maximum number of basic blocks visited on any path from entry to exit.
/// Bounded at `block_count` iterations to handle loops without infinite traversal.
pub fn compute_max_path_length(cfg: &FunctionCFGData) -> u16 {
    let n = cfg.blocks.len();
    if n == 0 {
        return 0;
    }
    let bound = n;
    let mut max_len = 0u16;
    // DFS stack: (block_id, depth).
    let mut stack: Vec<(u32, u16)> = vec![(0, 1)];
    while let Some((block, depth)) = stack.pop() {
        let succs = succ_list(cfg, block);
        if succs.is_empty() || depth as usize >= bound {
            max_len = max_len.max(depth);
        } else {
            for &s in succs {
                stack.push((s, depth + 1));
            }
        }
    }
    max_len
}
