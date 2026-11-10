//! FORCE algorithm — hypergraph center-of-gravity variable ordering (§8.2.4).
//!
//! FORCE (Aloul, Markov & Sakallah, GLSVLSI 2003) pulls variables that share a constraint
//! toward adjacent positions. Each iteration moves every variable to the mean of the
//! centers of gravity of its hyperedges and re-ranks; the quality measure is the **total
//! span** `Σ_h (max_pos(h) − min_pos(h))`. Iteration stops as soon as the span stops
//! decreasing, and the best order seen — the seed included — is returned, so FORCE can never
//! make the seed's span worse.
//!
//! Hyperedges are exactly the variable sets of the flow constraints conjoined into
//! `f_paths`: for each reachable block, its incoming DAG edges, its loop-entry variable (if
//! it is a header) and all of its outgoing edges.
//!
//! Time: O(Σ|h| × iterations + n log n × iterations), iterations ≤ [`FORCE_MAX_ITERATIONS`].

use crate::psa::construction::graph::PathGraph;
use crate::psa::ordering::VarKey;

/// Upper bound on FORCE iterations (the span stop rule usually ends far earlier).
pub const FORCE_MAX_ITERATIONS: usize = 64;

/// Refine an ordering of `n_vars` items.
///
/// Items are identified by their seed position `0..n_vars`; `hyperedges` list item indices.
/// Returns `order[position] = item`.
pub fn force_ordering(
    n_vars: usize,
    hyperedges: &[Vec<usize>],
    max_iterations: usize,
) -> Vec<usize> {
    if n_vars == 0 {
        return Vec::new();
    }

    let mut rank: Vec<usize> = (0..n_vars).collect();
    let mut best_order: Vec<usize> = (0..n_vars).collect();
    let mut best_span = total_span(hyperedges, &rank);
    let mut pos: Vec<f64> = (0..n_vars).map(|i| i as f64).collect();

    for _ in 0..max_iterations {
        let mut gravity = vec![0.0f64; n_vars];
        let mut weight = vec![0usize; n_vars];

        for hedge in hyperedges {
            if hedge.len() < 2 {
                continue;
            }
            let center: f64 = hedge.iter().map(|&v| pos[v]).sum::<f64>() / hedge.len() as f64;
            for &v in hedge {
                gravity[v] += center;
                weight[v] += 1;
            }
        }
        for v in 0..n_vars {
            if weight[v] > 0 {
                pos[v] = gravity[v] / weight[v] as f64;
            }
        }

        // Re-rank; ties keep the previous relative order, which makes the result stable.
        let mut order: Vec<usize> = (0..n_vars).collect();
        order.sort_by(|&a, &b| {
            pos[a]
                .partial_cmp(&pos[b])
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(rank[a].cmp(&rank[b]))
        });
        for (new_rank, &v) in order.iter().enumerate() {
            rank[v] = new_rank;
            pos[v] = new_rank as f64;
        }

        let span = total_span(hyperedges, &rank);
        if span < best_span {
            best_span = span;
            best_order = order;
        } else {
            break;
        }
    }

    best_order
}

/// `Σ_h (max rank − min rank)` over all hyperedges.
fn total_span(hyperedges: &[Vec<usize>], rank: &[usize]) -> u64 {
    hyperedges
        .iter()
        .filter(|h| h.len() >= 2)
        .map(|h| {
            let mut lo = usize::MAX;
            let mut hi = 0usize;
            for &v in h {
                lo = lo.min(rank[v]);
                hi = hi.max(rank[v]);
            }
            (hi - lo) as u64
        })
        .sum()
}

/// Build the constraint hyperedges over the items of `keys` (item = index into `keys`).
pub fn build_constraint_hyperedges(graph: &PathGraph, keys: &[VarKey]) -> Vec<Vec<usize>> {
    let mut edge_item = vec![usize::MAX; graph.n_edges()];
    let mut loop_entry_item = vec![usize::MAX; graph.n_blocks()];
    for (item, key) in keys.iter().enumerate() {
        match *key {
            VarKey::Edge(e) => {
                if let Some(slot) = edge_item.get_mut(e as usize) {
                    *slot = item;
                }
            }
            VarKey::LoopEntry(b) => {
                if let Some(slot) = loop_entry_item.get_mut(b as usize) {
                    *slot = item;
                }
            }
        }
    }

    let mut hyperedges: Vec<Vec<usize>> = Vec::new();
    for &b in graph.topo_order() {
        let mut hedge: Vec<usize> = Vec::new();
        for &e in graph.in_edges(b) {
            if graph.is_forward(e) && edge_item[e as usize] != usize::MAX {
                hedge.push(edge_item[e as usize]);
            }
        }
        if graph.is_header(b) && loop_entry_item[b as usize] != usize::MAX {
            hedge.push(loop_entry_item[b as usize]);
        }
        for &e in graph.out_edges(b) {
            if edge_item[e as usize] != usize::MAX {
                hedge.push(edge_item[e as usize]);
            }
        }
        if hedge.len() >= 2 {
            hyperedges.push(hedge);
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
        let order = force_ordering(3, &hyperedges, FORCE_MAX_ITERATIONS);
        assert_eq!(order.len(), 3);
        let mut s = order.clone();
        s.sort();
        assert_eq!(s, vec![0, 1, 2]);
    }

    #[test]
    fn force_empty_returns_empty() {
        assert!(force_ordering(0, &[], FORCE_MAX_ITERATIONS).is_empty());
    }
}
