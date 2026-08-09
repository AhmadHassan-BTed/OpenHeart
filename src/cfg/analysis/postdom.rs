//! Post-Dominator Computation (running Cooper's algorithm on reverse CFG).

use super::dominators::{compute_idom_cooper, UNDEFINED_IDOM};

pub fn compute_post_idom(n: usize, succs: &[Vec<u32>], exit_block: u32) -> Vec<u32> {
    if n == 0 {
        return Vec::new();
    }

    // Reverse graph: succs become preds
    let mut rev_succs = vec![Vec::new(); n];
    for (src, list) in succs.iter().enumerate() {
        for &dst in list {
            if (dst as usize) < n {
                rev_succs[dst as usize].push(src as u32);
            }
        }
    }

    // RPO from exit block
    let mut visited = vec![false; n];
    let mut postorder = Vec::with_capacity(n);

    fn rev_dfs(b: u32, succs: &[Vec<u32>], visited: &mut Vec<bool>, postorder: &mut Vec<u32>) {
        let b_idx = b as usize;
        if b_idx >= visited.len() || visited[b_idx] {
            return;
        }
        visited[b_idx] = true;
        for &s in &succs[b_idx] {
            rev_dfs(s, succs, visited, postorder);
        }
        postorder.push(b);
    }

    rev_dfs(exit_block, &rev_succs, &mut visited, &mut postorder);
    postorder.reverse();

    if postorder.is_empty() {
        return vec![UNDEFINED_IDOM; n];
    }

    compute_idom_cooper(n, &succs, &postorder)
}
