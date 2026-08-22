//! `sat_count(f, depth, n_vars)` — #SAT computation via memoized DFS (§8.2.5).
//!
//! The number of satisfying assignments of `f_paths` = the number of feasible execution paths
//! through the CFG.
//!
//! **Variable gaps:** Variables that don't appear in a sub-ROBDD are implicitly free — they can
//! take any value. The gap between a node's variable index and its child's variable index
//! determines how many free variables exist in that sub-problem.
//!
//! ```text
//! sat_count(TRUE, depth) = 1 << depth   // all remaining vars are free
//! sat_count(FALSE, depth) = 0
//!
//! sat_count(N, depth) =
//!   (1 << lo_gap) * sat_count(N.lo, depth - 1 - lo_gap)
//! + (1 << hi_gap) * sat_count(N.hi, depth - 1 - hi_gap)
//! ```
//! where `lo_gap = top_var(N.lo) - N.var - 1` (variables between N.var and N.lo's top).
//!
//! Time: O(|ROBDD|) — each node is visited at most once due to memoization.
//! For a function with 500 nodes, #SAT is computed in ~500 hash lookups — sub-microsecond.
//!
//! **Cyclomatic complexity** is NOT derived from the ROBDD. It is computed once from CFA edge
//! and block counts: `V(G) = |E| - |B| + 2` and stored permanently as a `u16` in the
//! function's path metrics record (§8.2.5).

use std::collections::HashMap;

use super::node::{ROBDDNode, FALSE_ID, TRUE_ID};

/// Compute #SAT(f) — the number of satisfying assignments of the Boolean function
/// encoded by the ROBDD rooted at `node`.
///
/// # Parameters
/// - `node`: root node_id of the sub-ROBDD.
/// - `depth`: number of variables remaining from this node downward (i.e. `n_vars - current_level`).
/// - `nodes`: the full node array.
/// - `n_vars`: total number of Boolean variables in the ROBDD.
/// - `memo`: memoization cache mapping `node_id → sat_count`.
///
/// # Variable Gap Computation
/// For a node N at variable index `N.var` with child C:
/// - If C is a terminal: `top_var(C) = n_vars` (sentinel beyond all variables).
/// - Otherwise: `top_var(C) = nodes[C].var`.
/// - `gap = top_var(C) - N.var - 1`.
/// - Free assignments in the gap: `2^gap`.
pub fn sat_count(
    node: u32,
    depth: u16,
    nodes: &[ROBDDNode],
    n_vars: u16,
    memo: &mut HashMap<u32, u64>,
) -> u64 {
    // Terminal base cases.
    if node == FALSE_ID {
        return 0;
    }
    if node == TRUE_ID {
        // All remaining `depth` variables are free — 2^depth satisfying assignments.
        return if depth >= 64 { u64::MAX } else { 1u64 << depth };
    }

    // Memoization.
    if let Some(&cached) = memo.get(&node) {
        return cached;
    }

    let n = &nodes[node as usize];

    // ── lo branch ────────────────────────────────────────────────────────────
    let lo_top = if n.lo == FALSE_ID || n.lo == TRUE_ID {
        n_vars
    } else {
        nodes[n.lo as usize].var
    };
    let lo_gap = lo_top.saturating_sub(n.var).saturating_sub(1) as u32;
    let lo_depth = depth.saturating_sub(1 + lo_gap as u16);
    let lo_count =
        (1u64 << lo_gap.min(63)).saturating_mul(sat_count(n.lo, lo_depth, nodes, n_vars, memo));

    // ── hi branch ────────────────────────────────────────────────────────────
    let hi_top = if n.hi == FALSE_ID || n.hi == TRUE_ID {
        n_vars
    } else {
        nodes[n.hi as usize].var
    };
    let hi_gap = hi_top.saturating_sub(n.var).saturating_sub(1) as u32;
    let hi_depth = depth.saturating_sub(1 + hi_gap as u16);
    let hi_count =
        (1u64 << hi_gap.min(63)).saturating_mul(sat_count(n.hi, hi_depth, nodes, n_vars, memo));

    let total = lo_count.saturating_add(hi_count);
    memo.insert(node, total);
    total
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::psa::bdd::unique_table::UniqueTable;

    /// Build: x0 (var=0) with lo=FALSE, hi=TRUE → single-variable BDD.
    fn single_var_bdd() -> (Vec<ROBDDNode>, u32) {
        let mut nodes = vec![ROBDDNode::false_terminal(), ROBDDNode::true_terminal()];
        let mut ut = UniqueTable::new();
        let id = ut.make_node(0, FALSE_ID, TRUE_ID, &mut nodes);
        (nodes, id)
    }

    #[test]
    fn sat_false_is_zero() {
        let nodes = vec![ROBDDNode::false_terminal(), ROBDDNode::true_terminal()];
        let result = sat_count(FALSE_ID, 4, &nodes, 4, &mut HashMap::new());
        assert_eq!(result, 0);
    }

    #[test]
    fn sat_true_with_depth_is_2_pow_depth() {
        let nodes = vec![ROBDDNode::false_terminal(), ROBDDNode::true_terminal()];
        // depth=3 → 2^3 = 8 free assignments
        let result = sat_count(TRUE_ID, 3, &nodes, 3, &mut HashMap::new());
        assert_eq!(result, 8);
    }

    #[test]
    fn sat_single_variable_bdd_is_one() {
        // x0: lo=FALSE, hi=TRUE → one satisfying assignment (x0=1).
        let (nodes, root) = single_var_bdd();
        // n_vars=1, depth=1 (one variable remaining at the root level)
        let result = sat_count(root, 1, &nodes, 1, &mut HashMap::new());
        assert_eq!(result, 1, "x0=1 is the only sat assignment");
    }

    #[test]
    fn memoization_does_not_double_count() {
        let (nodes, root) = single_var_bdd();
        let mut memo = HashMap::new();
        let first = sat_count(root, 1, &nodes, 1, &mut memo);
        // Second call should hit memo and return the same result.
        let second = sat_count(root, 1, &nodes, 1, &mut memo);
        assert_eq!(first, second);
    }
}
