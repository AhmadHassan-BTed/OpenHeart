//! `restrict(f, var, val)` — cofactor computation (§8.2.2, §8.5.2).
//!
//! Given an ROBDD for function f and a variable xᵢ fixed to value `val` ∈ {0,1},
//! computes the ROBDD for `f|_{xᵢ=val}` (the cofactor of f with xᵢ fixed).
//!
//! Time: O(|f|) — one DFS pass with memoization.
//!
//! Used in Phase 8 step 5 to apply the entry constraint:
//! ```text
//! f_paths = bdd.restrict(f_paths, entry_edge_var, 1);  // fix entry edge = 1
//! ```
//! This halves the search space by eliminating the entry edge variable from the ROBDD.

use std::collections::HashMap;

use super::node::{ROBDDNode, FALSE_ID, TRUE_ID};
use super::unique_table::UniqueTable;

/// Compute `f|_{var=val}` — the cofactor of f with variable `var` fixed to `val` (0 or 1).
///
/// - `f` is the root node_id of the ROBDD.
/// - `var` is the variable index to restrict.
/// - `val` is 0 (lo branch) or 1 (hi branch).
/// - `nodes` is the shared node array.
/// - `unique_table` is used to construct any new nodes needed after restriction.
/// - `all_nodes` is the mutable node vector for potential new node allocation.
/// - `memo` caches `node_id → restricted_node_id` for O(|f|) total traversal.
pub fn restrict(
    f: u32,
    var: u16,
    val: u8,
    nodes: &[ROBDDNode],
    unique_table: &mut UniqueTable,
    all_nodes: &mut Vec<ROBDDNode>,
    memo: &mut HashMap<u32, u32>,
) -> u32 {
    // Terminal base cases: restriction does not change terminals.
    if f == FALSE_ID {
        return FALSE_ID;
    }
    if f == TRUE_ID {
        return TRUE_ID;
    }

    // Memoization.
    if let Some(&cached) = memo.get(&f) {
        return cached;
    }

    let fn_ = &nodes[f as usize];

    let result = if fn_.var == var {
        // This node's variable is the one being restricted.
        // Follow the appropriate branch based on val, then recurse.
        let child = if val == 0 { fn_.lo } else { fn_.hi };
        // Recurse into the child — it may itself depend on var (shouldn't happen in a valid
        // ROBDD since var ordering is strict, but we recurse defensively).
        restrict(child, var, val, nodes, unique_table, all_nodes, memo)
    } else if fn_.var > var {
        // The variable ordering means fn_.var > var, so var does not appear in this sub-DAG.
        // The restriction is a no-op — return f unchanged.
        f
    } else {
        // fn_.var < var: var may appear in the children. Recurse.
        let lo = restrict(fn_.lo, var, val, nodes, unique_table, all_nodes, memo);
        let hi = restrict(fn_.hi, var, val, nodes, unique_table, all_nodes, memo);
        // Reconstruct with make_node (applies reduction rules on the restricted subtree).
        let fn_var = fn_.var; // reborrow-safe copy
        unique_table.make_node(fn_var, lo, hi, all_nodes)
    };

    memo.insert(f, result);
    result
}

#[cfg(test)]
mod tests {
    use crate::psa::bdd::BDDLibrary;
    use crate::psa::bdd::node::{FALSE_ID, TRUE_ID};

    #[test]
    fn restrict_terminal_false() {
        let mut lib = BDDLibrary::new();
        let r = lib.restrict(FALSE_ID, 2, 0);
        assert_eq!(r, FALSE_ID);
    }

    #[test]
    fn restrict_terminal_true() {
        let mut lib = BDDLibrary::new();
        let r = lib.restrict(TRUE_ID, 2, 1);
        assert_eq!(r, TRUE_ID);
    }

    #[test]
    fn restrict_var_to_one_gives_true() {
        let mut lib = BDDLibrary::new();
        let x2 = lib.var(2);
        // x2 restricted to 1 → follow hi branch → TRUE
        let r = lib.restrict(x2, 2, 1);
        assert_eq!(r, TRUE_ID);
    }

    #[test]
    fn restrict_var_to_zero_gives_false() {
        let mut lib = BDDLibrary::new();
        let x2 = lib.var(2);
        // x2 restricted to 0 → follow lo branch → FALSE
        let r = lib.restrict(x2, 2, 0);
        assert_eq!(r, FALSE_ID);
    }

    #[test]
    fn restrict_unrelated_var_is_noop() {
        let mut lib = BDDLibrary::new();
        let x2 = lib.var(2);
        // Restrict x5 (doesn't appear) — result should be x2 unchanged
        let r = lib.restrict(x2, 5, 1);
        assert_eq!(r, x2);
    }
}

