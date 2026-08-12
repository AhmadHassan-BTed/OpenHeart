//! Structural constraint Φ_b for each CFG block (§8.2.3, §8.5.3).
//!
//! For function f with CFG (B, E), assign Boolean variable xₑ to each CFG edge e ∈ E.
//! An assignment A: E → {0,1} represents a structurally valid path iff it satisfies
//! the flow conservation constraint Φ_b at every block:
//!
//! **EXIT block (0 successors):** Φ_b = TRUE.
//!
//! **Unconditional (1 successor):**
//! ```text
//! Φ_b = (∨ x_{pᵢ→b}) → x_{b→s}
//! ```
//! "If any predecessor edge is taken, the single outgoing edge is taken."
//!
//! **Binary branch (2 successors — most common for conditionals):**
//! ```text
//! Φ_b = (∨ x_{pᵢ→b}) → (x_{e_true} ⊕ x_{e_false})
//! ```
//! "If entered, exactly one of the two branch edges is taken (XOR)."
//!
//! **Switch (n > 2 successors):**
//! Exactly one outgoing edge taken — pairwise AND-NOT exclusion.

use crate::cfg::builder::FunctionCFGData;
use crate::psa::bdd::BDDLibrary;
use crate::psa::ordering::{rpo::edge_index_of, rpo::pred_list, rpo::succ_list, VariableOrdering};
use crate::psa::types::BoolOp;

/// Build the structural constraint Φ_b for block `block_id` in `cfg`.
///
/// Returns the root node_id of the ROBDD encoding Φ_b within `bdd`.
pub fn build_phi_b(
    block_id: u32,
    cfg: &FunctionCFGData,
    ordering: &VariableOrdering,
    bdd: &mut BDDLibrary,
) -> u32 {
    let succs = succ_list(cfg, block_id);
    let preds = pred_list(cfg, block_id);

    // ── is_reachable: OR of all incoming edge variables ───────────────────────
    let is_reachable = preds.iter().fold(bdd.false_id(), |acc, &pred| {
        if let Some(ei) = edge_index_of(cfg, pred, block_id) {
            let var = ordering.var_for_edge_idx(ei);
            let x = bdd.var(var);
            bdd.apply(BoolOp::Or, acc, x)
        } else {
            acc
        }
    });

    match succs.len() {
        // EXIT block: no outgoing constraint — return TRUE.
        0 => bdd.true_id(),

        // Unconditional: if reachable, the single outgoing edge is taken.
        1 => {
            let x_out = if let Some(ei) = edge_index_of(cfg, block_id, succs[0]) {
                let var = ordering.var_for_edge_idx(ei);
                bdd.var(var)
            } else {
                bdd.true_id()
            };
            bdd.implies(is_reachable, x_out)
        }

        // Binary branch: if entered, exactly one branch is taken (XOR).
        2 => {
            let x_true = if let Some(ei) = edge_index_of(cfg, block_id, succs[0]) {
                let v = ordering.var_for_edge_idx(ei);
                bdd.var(v)
            } else {
                bdd.false_id()
            };
            let x_false = if let Some(ei) = edge_index_of(cfg, block_id, succs[1]) {
                let v = ordering.var_for_edge_idx(ei);
                bdd.var(v)
            } else {
                bdd.false_id()
            };
            let xor = bdd.apply(BoolOp::Xor, x_true, x_false);
            bdd.implies(is_reachable, xor)
        }

        // Switch: exactly one successor taken.
        _ => build_switch_constraint(block_id, succs, is_reachable, cfg, ordering, bdd),
    }
}

/// Switch constraint: exactly one of n outgoing edges taken.
///
/// `at_least_one ∧ at_most_one` where at_most_one uses pairwise ¬(xᵢ ∧ xⱼ) clauses.
fn build_switch_constraint(
    block_id: u32,
    succs: &[u32],
    is_reachable: u32,
    cfg: &FunctionCFGData,
    ordering: &VariableOrdering,
    bdd: &mut BDDLibrary,
) -> u32 {
    let xs: Vec<u32> = succs
        .iter()
        .filter_map(|&s| {
            edge_index_of(cfg, block_id, s).map(|ei| {
                let v = ordering.var_for_edge_idx(ei);
                bdd.var(v)
            })
        })
        .collect();

    if xs.is_empty() {
        return bdd.true_id();
    }

    // at_least_one: ∨ xᵢ
    let at_least_one = xs.iter().fold(bdd.false_id(), |acc, &x| bdd.apply(BoolOp::Or, acc, x));

    // at_most_one: ∧ ¬(xᵢ ∧ xⱼ) for all i < j
    let mut at_most_one = bdd.true_id();
    for i in 0..xs.len() {
        for j in (i + 1)..xs.len() {
            let both = bdd.apply(BoolOp::And, xs[i], xs[j]);
            let not_both = bdd.apply_not(both);
            at_most_one = bdd.apply(BoolOp::And, at_most_one, not_both);
        }
    }

    let exactly_one = bdd.apply(BoolOp::And, at_least_one, at_most_one);
    bdd.implies(is_reachable, exactly_one)
}
