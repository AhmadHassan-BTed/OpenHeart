//! Post-Dominator Computation (Cooper's algorithm on the reverse CFG).
//!
//! The reverse graph is rooted at a virtual exit node `V = n` that every exit feeds:
//! the real EXIT block, every block without successors and, for each region that cannot
//! reach any exit (an infinite loop), one representative block of that region (the
//! unvisited block that comes last in forward reverse post-order, mirroring the root
//! choice of LLVM's PostDominatorTree). Optionally the Ferrante–Ottenstein–Warren
//! augmentation edge `ENTRY -> V` is added, which makes ENTRY a predicate on which every
//! unconditionally executed block is control dependent.

use super::dominators::{compute_idom_cooper, UNDEFINED_IDOM};
use super::rpo::{postorder_from, reverse_postorder};

/// Post-dominator tree over `n` real blocks plus one virtual exit.
#[derive(Debug, Clone)]
pub struct PostDominators {
    /// `ipdom[b]` for `b in 0..=n`; `ipdom[virtual_exit] == virtual_exit`.
    pub ipdom: Vec<u32>,
    /// Index of the virtual exit node (`== n`).
    pub virtual_exit: u32,
    /// Blocks linked to the virtual exit because their region cannot reach any exit.
    pub infinite_loop_roots: Vec<u32>,
}

/// Post-dominators without the ENTRY augmentation edge.
pub fn compute_post_idom(n: usize, succs: &[Vec<u32>], exit_block: u32) -> PostDominators {
    compute_post_idom_augmented(n, succs, exit_block, None)
}

/// Post-dominators; when `entry` is given, the FOW edge `entry -> virtual exit` is added.
pub fn compute_post_idom_augmented(
    n: usize,
    succs: &[Vec<u32>],
    exit_block: u32,
    entry: Option<u32>,
) -> PostDominators {
    let v = n as u32;
    let total = n + 1;

    // Forward successor lists of the augmented graph.
    let mut aug_succs: Vec<Vec<u32>> = Vec::with_capacity(total);
    for b in 0..n {
        let mut list: Vec<u32> = succs
            .get(b)
            .map(|l| l.iter().copied().filter(|&s| (s as usize) < n).collect())
            .unwrap_or_default();
        if list.is_empty() || b as u32 == exit_block {
            list.push(v);
        }
        if entry == Some(b as u32) && !list.contains(&v) {
            list.push(v);
        }
        aug_succs.push(list);
    }
    aug_succs.push(Vec::new());

    // Reverse adjacency (forward predecessors), used for the reverse-graph traversal.
    let mut aug_preds: Vec<Vec<u32>> = vec![Vec::new(); total];
    for (b, list) in aug_succs.iter().enumerate() {
        for &s in list {
            aug_preds[s as usize].push(b as u32);
        }
    }

    // Reverse-graph post-order from V; regions that cannot reach V get a root edge.
    let mut visited = vec![false; total];
    let mut post = postorder_from(total, &aug_preds, v, &mut visited);

    // Adding `cand -> V` makes `cand` the last reverse-graph child of V, so the DFS can be
    // resumed from `cand` and V moved back to the end of the post-order.
    let add_root = |cand: u32,
                        aug_succs: &mut Vec<Vec<u32>>,
                        aug_preds: &mut Vec<Vec<u32>>,
                        visited: &mut Vec<bool>,
                        post: &mut Vec<u32>| {
        aug_succs[cand as usize].push(v);
        aug_preds[v as usize].push(cand);
        if post.last() == Some(&v) {
            post.pop();
        }
        post.extend(postorder_from(total, aug_preds, cand, visited));
        post.push(v);
    };

    let forward_rpo = reverse_postorder(n, succs);
    let mut infinite_loop_roots = Vec::new();
    for &cand in forward_rpo.iter().rev() {
        if !visited[cand as usize] {
            add_root(cand, &mut aug_succs, &mut aug_preds, &mut visited, &mut post);
            infinite_loop_roots.push(cand);
        }
    }
    // Blocks unreachable from ENTRY that still cannot reach V (no forward RPO entry):
    // give each its own root edge so every node is covered.
    for b in 0..n as u32 {
        if !visited[b as usize] {
            add_root(b, &mut aug_succs, &mut aug_preds, &mut visited, &mut post);
            infinite_loop_roots.push(b);
        }
    }

    post.reverse();
    let rev_rpo = post;

    // Dominators of the reverse graph: predecessors in the reverse graph are the forward
    // successors.
    let ipdom = if rev_rpo.is_empty() {
        vec![UNDEFINED_IDOM; total]
    } else {
        compute_idom_cooper(total, &aug_succs, &rev_rpo)
    };

    PostDominators {
        ipdom,
        virtual_exit: v,
        infinite_loop_roots,
    }
}
