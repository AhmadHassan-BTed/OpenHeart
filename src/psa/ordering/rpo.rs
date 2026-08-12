//! Initial variable ordering from reverse post-order traversal of CFG edges (§8.2.4, §8.5.2).
//!
//! RPO provides the initial position vector for the FORCE algorithm. Edges discovered
//! earlier in DFS (from the entry block) receive lower position values.

use crate::cfg::builder::FunctionCFGData;

/// Compute the initial RPO ordering of CFG edges.
///
/// Returns `rpo_order[position] = edge_index_into_cfg_edges[]`.
/// The DFS is rooted at block 0 (the ENTRY block in all OpenHeart CFGs).
pub fn rpo_edge_ordering(cfg: &FunctionCFGData) -> Vec<usize> {
    let n_blocks = cfg.blocks.len();
    let n_edges = cfg.edges.len();

    if n_edges == 0 {
        return Vec::new();
    }

    let mut visited = vec![false; n_blocks];
    let mut edge_order: Vec<usize> = Vec::with_capacity(n_edges);
    let mut in_order = vec![false; n_edges];

    // DFS stack: (block_id, successor_index_within_succ_adj)
    let entry = 0u32;
    let mut stack: Vec<(u32, usize)> = vec![(entry, 0)];
    if n_blocks > 0 {
        visited[0] = true;
    }

    while let Some((block, succ_i)) = stack.last_mut() {
        let block_id = *block;
        let succs = succ_list(cfg, block_id);

        if *succ_i < succs.len() {
            let succ = succs[*succ_i];
            *succ_i += 1;

            // Find the edge index for (block_id → succ).
            if let Some(eidx) = edge_index_of(cfg, block_id, succ) {
                if !in_order[eidx] {
                    edge_order.push(eidx);
                    in_order[eidx] = true;
                }
            }

            if (succ as usize) < n_blocks && !visited[succ as usize] {
                visited[succ as usize] = true;
                stack.push((succ, 0));
            }
        } else {
            stack.pop();
        }
    }

    // Include any edges not reached by DFS (dead code / back edges not traversed).
    for i in 0..n_edges {
        if !in_order[i] {
            edge_order.push(i);
        }
    }

    edge_order
}

/// Compute the position vector for the RPO ordering.
///
/// Returns `pos[edge_index] = position_in_rpo_ordering`.
/// This is the initial position vector fed into FORCE.
pub fn rpo_positions(rpo_order: &[usize]) -> Vec<f64> {
    let n = rpo_order.len();
    let mut pos = vec![0.0f64; n];
    for (position, &edge_idx) in rpo_order.iter().enumerate() {
        if edge_idx < n {
            pos[edge_idx] = position as f64;
        }
    }
    pos
}

/// Helper: get the successor block IDs for `block_id` from the CSR succ_adj.
pub(crate) fn succ_list(cfg: &FunctionCFGData, block_id: u32) -> &[u32] {
    let idx = block_id as usize;
    if idx + 1 < cfg.succ_offsets.len() {
        let s = cfg.succ_offsets[idx] as usize;
        let e = cfg.succ_offsets[idx + 1] as usize;
        &cfg.succ_adj[s..e]
    } else {
        &[]
    }
}

/// Helper: get the predecessor block IDs for `block_id` from the CSR pred_adj.
pub(crate) fn pred_list(cfg: &FunctionCFGData, block_id: u32) -> &[u32] {
    let idx = block_id as usize;
    if idx + 1 < cfg.pred_offsets.len() {
        let s = cfg.pred_offsets[idx] as usize;
        let e = cfg.pred_offsets[idx + 1] as usize;
        &cfg.pred_adj[s..e]
    } else {
        &[]
    }
}

/// Helper: find the index of edge (from → to) in cfg.edges[].
pub(crate) fn edge_index_of(cfg: &FunctionCFGData, from: u32, to: u32) -> Option<usize> {
    cfg.edges.iter().position(|&(f, t, _)| f == from && t == to)
}
