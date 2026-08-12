//! FORCE algorithm — hypergraph gravity-center variable ordering (§8.2.4).
//!
//! FORCE (Aloul et al. 2003) pulls correlated variables (those sharing a constraint)
//! toward adjacent positions in the ordering, minimizing ROBDD bandwidth.
//!
//! Time: O(m × |constraints| × n_iters) where n_iters = min(10×n_vars, 200).

use crate::cfg::builder::FunctionCFGData;
use crate::psa::ordering::rpo::{edge_index_of, pred_list, succ_list};

/// Run the FORCE algorithm to produce a near-optimal variable ordering.
///
/// Returns `force_order[position] = edge_index` — the final variable ordering after convergence.
///
/// # Parameters
/// - `n_vars`: number of Boolean variables (= cfg.edges.len()).
/// - `hyperedges`: constraint hypergraph — each entry is a set of edge indices in one constraint.
/// - `pos`: initial continuous position for each variable (from RPO).
pub fn force_ordering(n_vars: usize, hyperedges: &[Vec<usize>], mut pos: Vec<f64>) -> Vec<usize> {
    if n_vars == 0 {
        return Vec::new();
    }

    let n_iters = (10 * n_vars).min(200);

    for _ in 0..n_iters {
        let mut gravity = vec![0.0f64; n_vars];
        let mut weight = vec![0usize; n_vars];

        // Accumulate gravity from each hyperedge.
        for hedge in hyperedges {
            if hedge.is_empty() {
                continue;
            }
            let center: f64 = hedge
                .iter()
                .map(|&v| if v < n_vars { pos[v] } else { 0.0 })
                .sum::<f64>()
                / hedge.len() as f64;
            for &v in hedge {
                if v < n_vars {
                    gravity[v] += center;
                    weight[v] += 1;
                }
            }
        }

        // Update positions to gravity centers.
        for v in 0..n_vars {
            if weight[v] > 0 {
                pos[v] = gravity[v] / weight[v] as f64;
            }
        }

        // Normalize: re-rank all positions as integers 0..n_vars-1.
        let mut order: Vec<usize> = (0..n_vars).collect();
        order.sort_unstable_by(|&a, &b| {
            pos[a]
                .partial_cmp(&pos[b])
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.cmp(&b))
        });
        for (new_p, &v) in order.iter().enumerate() {
            pos[v] = new_p as f64;
        }
    }

    // Final ordering: sort by converged position.
    let mut final_order: Vec<usize> = (0..n_vars).collect();
    final_order.sort_unstable_by(|&a, &b| {
        pos[a]
            .partial_cmp(&pos[b])
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    final_order
}

/// Build the constraint hyperedges for a function's CFG (§8.2.4).
///
/// Each hyperedge is the set of edge indices involved in one structural constraint Φ_b.
pub fn build_constraint_hyperedges(cfg: &FunctionCFGData) -> Vec<Vec<usize>> {
    let n_blocks = cfg.blocks.len();
    let mut hyperedges: Vec<Vec<usize>> = Vec::new();

    for block_id in 0..n_blocks as u32 {
        let succs = succ_list(cfg, block_id);
        let preds = pred_list(cfg, block_id);

        match succs.len() {
            0 => { /* EXIT block — no outgoing constraint */ }

            1 => {
                // Unconditional: incoming edges correlated with outgoing edge.
                let mut hedge: Vec<usize> = preds
                    .iter()
                    .filter_map(|&p| edge_index_of(cfg, p, block_id))
                    .collect();
                if let Some(ei) = edge_index_of(cfg, block_id, succs[0]) {
                    hedge.push(ei);
                }
                if hedge.len() > 1 {
                    hyperedges.push(hedge);
                }
            }

            2 => {
                // Binary branch: XOR constraint between true and false edges.
                let mut hedge: Vec<usize> = Vec::new();
                if let Some(ei) = edge_index_of(cfg, block_id, succs[0]) {
                    hedge.push(ei);
                }
                if let Some(ei) = edge_index_of(cfg, block_id, succs[1]) {
                    hedge.push(ei);
                }
                if hedge.len() == 2 {
                    hyperedges.push(hedge);
                }
            }

            _ => {
                // Switch: all outgoing edges correlated.
                let hedge: Vec<usize> = succs
                    .iter()
                    .filter_map(|&s| edge_index_of(cfg, block_id, s))
                    .collect();
                if hedge.len() > 1 {
                    hyperedges.push(hedge);
                }
            }
        }
    }

    hyperedges
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn force_preserves_all_variables() {
        let hyperedges = vec![vec![0usize, 1]];
        let pos = vec![0.0, 1.0, 2.0];
        let order = force_ordering(3, &hyperedges, pos);
        assert_eq!(order.len(), 3);
        let mut s = order.clone();
        s.sort();
        assert_eq!(s, vec![0, 1, 2]);
    }

    #[test]
    fn force_empty_returns_empty() {
        assert!(force_ordering(0, &[], vec![]).is_empty());
    }
}
