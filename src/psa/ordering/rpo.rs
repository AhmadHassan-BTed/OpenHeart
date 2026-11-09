//! Locality-preserving initial variable ordering (§8.2.4, §8.5.2).
//!
//! The size of the path ROBDD is governed by its *frontier*: at every level, how many
//! distinct "where is the path now" states are still open. Ordering variables by the
//! position of their block in a topological order of the CFG DAG (reverse post-order of a
//! DFS from ENTRY), with each block's loop-entry variable immediately followed by its
//! out-edges, keeps a block's incoming and outgoing edges adjacent and makes the frontier
//! equal to the set of blocks a path can be "between" — linear BDD size for
//! series-parallel (structured) code.
//!
//! The previous seed was a DFS *edge pre-order*, which interleaves unrelated branches and
//! produced orders up to two orders of magnitude worse than a plain topological order.

use crate::psa::construction::graph::PathGraph;
use crate::psa::ordering::VarKey;

/// Compute the seed ordering, top level first: for each reachable block in topological
/// order, its loop-entry variable (if it is a loop header) followed by its out-edges; then
/// every remaining edge (inactive edges — they are constrained to 0) in id order.
pub fn locality_seed(graph: &PathGraph) -> Vec<VarKey> {
    let n_edges = graph.n_edges();
    let mut placed = vec![false; n_edges];
    let mut order: Vec<VarKey> = Vec::with_capacity(n_edges + graph.headers().len());

    for &b in graph.topo_order() {
        if graph.is_header(b) {
            order.push(VarKey::LoopEntry(b));
        }
        for &e in graph.out_edges(b) {
            if !placed[e as usize] {
                placed[e as usize] = true;
                order.push(VarKey::Edge(e));
            }
        }
    }
    for (e, done) in placed.iter().enumerate() {
        if !done {
            order.push(VarKey::Edge(e as u32));
        }
    }
    order
}
