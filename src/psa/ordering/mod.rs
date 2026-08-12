//! VariableOrdering — manages the xᵢ ↔ edge_id bijection (§8.2.4, §8.3).
//!
//! The VariableOrdering is the bridge between abstract BDD variable indices and
//! concrete CFG edge IDs from the CFA artifact.
//!
//! Invariants:
//! - `var_to_edge[i]` = the CFA edge index assigned to BDD variable index `i`.
//! - `edge_to_var[edge_idx]` = the BDD variable index for that CFG edge position.
//! - These two arrays are always inverse permutations of each other.

pub mod force;
pub mod rpo;
pub mod sifting;

pub use force::{build_constraint_hyperedges, force_ordering};
pub use rpo::{rpo_edge_ordering, rpo_positions};
pub use sifting::SiftingOptimizer;

use crate::cfg::builder::FunctionCFGData;

/// The bijection between BDD variable indices (0..n_vars-1) and CFG edge positions.
///
/// Created from the FORCE-converged ordering and used throughout construction steps 3–8.
pub struct VariableOrdering {
    /// `var_to_edge[var_idx]` = CFG edge index (position in FunctionCFGData.edges[]).
    var_to_edge: Vec<u16>,

    /// `edge_to_var[edge_idx]` = BDD variable index for that CFG edge.
    edge_to_var: Vec<u16>,

    /// Number of Boolean variables (= number of CFG edges in this function).
    n_vars: u16,
}

impl VariableOrdering {
    /// Build a VariableOrdering from a FORCE-converged ordering.
    ///
    /// `force_order[position] = edge_index_in_cfg_edges_array` (from `force_ordering()`).
    pub fn from_order(force_order: &[usize], cfg: &FunctionCFGData) -> Self {
        let n_vars = force_order.len().min(65535);
        let n_edges = cfg.edges.len();

        let mut var_to_edge: Vec<u16> = vec![0u16; n_vars];
        let mut edge_to_var: Vec<u16> = vec![0u16; n_edges];

        for (var_idx, &edge_idx) in force_order.iter().enumerate().take(n_vars) {
            let ei = edge_idx.min(n_edges.saturating_sub(1));
            var_to_edge[var_idx] = ei as u16;
            edge_to_var[ei] = var_idx as u16;
        }

        Self {
            var_to_edge,
            edge_to_var,
            n_vars: n_vars as u16,
        }
    }

    /// Build a VariableOrdering from raw edge index sequences (for deserialization).
    pub fn from_edge_sequence(edge_indices: Vec<u16>) -> Self {
        let n_vars = edge_indices.len();
        let max_idx = edge_indices.iter().copied().max().unwrap_or(0) as usize;
        let mut edge_to_var = vec![0u16; max_idx + 1];
        for (var_idx, &ei) in edge_indices.iter().enumerate() {
            if (ei as usize) < edge_to_var.len() {
                edge_to_var[ei as usize] = var_idx as u16;
            }
        }
        Self {
            var_to_edge: edge_indices,
            edge_to_var,
            n_vars: n_vars as u16,
        }
    }

    /// Returns the BDD variable index for the edge at position `edge_idx` in cfg.edges[].
    #[inline]
    pub fn var_for_edge_idx(&self, edge_idx: usize) -> u16 {
        if edge_idx < self.edge_to_var.len() {
            self.edge_to_var[edge_idx]
        } else {
            0
        }
    }

    /// Returns the CFG edge index (position in cfg.edges[]) for BDD variable `var_idx`.
    #[inline]
    pub fn edge_for_var(&self, var_idx: u16) -> u16 {
        if (var_idx as usize) < self.var_to_edge.len() {
            self.var_to_edge[var_idx as usize]
        } else {
            0
        }
    }

    /// Returns the total number of Boolean variables (= CFG edge count).
    #[inline]
    pub fn n_vars(&self) -> u16 {
        self.n_vars
    }

    /// Returns the var_to_edge slice as u32 values for PSA serialization
    /// (§8.6 Variable Ordering Tables: per-function edge_id[] of length n_vars).
    pub fn as_u32_edge_slice(&self) -> Vec<u32> {
        self.var_to_edge.iter().map(|&e| e as u32).collect()
    }

    /// Returns the var_to_edge raw slice for serialization.
    pub fn as_edge_slice(&self) -> &[u16] {
        &self.var_to_edge
    }

    /// Swap adjacent variables at positions `pos` and `pos+1` in the ordering.
    ///
    /// Used by the SiftingOptimizer. Updates both var_to_edge and edge_to_var atomically.
    pub fn swap_adjacent(&mut self, pos: usize) {
        if pos + 1 >= self.var_to_edge.len() {
            return;
        }
        let ei_a = self.var_to_edge[pos];
        let ei_b = self.var_to_edge[pos + 1];
        self.var_to_edge.swap(pos, pos + 1);
        if (ei_a as usize) < self.edge_to_var.len() {
            self.edge_to_var[ei_a as usize] = (pos + 1) as u16;
        }
        if (ei_b as usize) < self.edge_to_var.len() {
            self.edge_to_var[ei_b as usize] = pos as u16;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swap_adjacent_updates_both_maps() {
        let mut o = VariableOrdering {
            var_to_edge: vec![3, 7],
            edge_to_var: {
                let mut v = vec![0u16; 10];
                v[3] = 0;
                v[7] = 1;
                v
            },
            n_vars: 2,
        };
        o.swap_adjacent(0);
        assert_eq!(o.var_to_edge[0], 7);
        assert_eq!(o.var_to_edge[1], 3);
        assert_eq!(o.edge_to_var[7], 0);
        assert_eq!(o.edge_to_var[3], 1);
    }
}
