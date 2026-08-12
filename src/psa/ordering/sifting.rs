//! SiftingOptimizer — Rudell's per-variable local sift (§8.2.4).
//!
//! After FORCE provides a near-optimal ordering, sifting performs per-variable
//! local refinement: each variable is sifted up then down while measuring |ROBDD|
//! after each adjacent swap, and placed at its minimum-cost position.
//!
//! Time: O(m × |ROBDD|) amortized (§8.2.4).

use crate::psa::bdd::BDDLibrary;
use crate::psa::ordering::VariableOrdering;

/// Rudell sifting optimizer.
pub struct SiftingOptimizer;

impl SiftingOptimizer {
    /// Apply Rudell sifting to reduce |ROBDD| after FORCE ordering.
    ///
    /// Processes variables in descending order of their node-count contribution
    /// (highest-impact variables first — they benefit most from position changes).
    pub fn optimize(bdd: &mut BDDLibrary, root: &mut u32, ordering: &mut VariableOrdering) {
        let n_vars = ordering.n_vars() as usize;
        if n_vars <= 1 {
            return;
        }

        // Count reachable nodes per variable (for priority ordering).
        let var_counts = Self::count_var_nodes(bdd, *root);

        // Sort by descending node count.
        let mut priority: Vec<usize> = (0..n_vars).collect();
        priority.sort_unstable_by(|&a, &b| {
            var_counts.get(b).unwrap_or(&0)
                .cmp(var_counts.get(a).unwrap_or(&0))
        });

        for var_pos in priority {
            Self::sift_variable(bdd, root, ordering, var_pos);
        }
    }

    /// Sift a single variable at `var_pos` up then down; leave it at its minimum-size position.
    fn sift_variable(
        bdd: &mut BDDLibrary,
        _root: &mut u32,
        ordering: &mut VariableOrdering,
        var_pos: usize,
    ) {
        let n_vars = ordering.n_vars() as usize;
        let mut best_size = bdd.node_count();
        let mut best_pos = var_pos;
        let mut current = var_pos;

        // Sift upward.
        while current > 0 {
            ordering.swap_adjacent(current - 1);
            current -= 1;
            let sz = bdd.node_count();
            if sz < best_size {
                best_size = sz;
                best_pos = current;
            }
        }

        // Sift downward.
        while current < n_vars - 1 {
            ordering.swap_adjacent(current);
            current += 1;
            let sz = bdd.node_count();
            if sz < best_size {
                best_size = sz;
                best_pos = current;
            }
        }

        // Return to best position.
        while current > best_pos {
            ordering.swap_adjacent(current - 1);
            current -= 1;
        }
    }

    /// Count reachable nodes per variable index via DFS from `root`.
    fn count_var_nodes(bdd: &BDDLibrary, root: u32) -> Vec<usize> {
        let total = bdd.node_count();
        let mut counts: Vec<usize> = Vec::new();
        let mut visited = vec![false; total];
        let mut stack = vec![root];

        while let Some(nid) = stack.pop() {
            let idx = nid as usize;
            if idx >= total || visited[idx] {
                continue;
            }
            visited[idx] = true;
            let node = &bdd.nodes[idx];
            if node.is_terminal() {
                continue;
            }
            let v = node.var as usize;
            if v >= counts.len() {
                counts.resize(v + 1, 0);
            }
            counts[v] += 1;
            stack.push(node.lo);
            stack.push(node.hi);
        }
        counts
    }
}
