//! Reverse Post-Order (RPO) Traversal algorithm for CFGs.
//!
//! The traversal uses an explicit stack: its depth follows the number of blocks on a DFS
//! path (about two per sequential `if`), so recursion would overflow on large generated
//! methods.

/// Reverse post-order of the blocks reachable from block 0.
pub fn reverse_postorder(n: usize, succs: &[Vec<u32>]) -> Vec<u32> {
    reverse_postorder_from(n, succs, 0)
}

/// Reverse post-order of the blocks reachable from `root`.
pub fn reverse_postorder_from(n: usize, succs: &[Vec<u32>], root: u32) -> Vec<u32> {
    let mut postorder = postorder_from(n, succs, root, &mut vec![false; n]);
    postorder.reverse();
    postorder
}

/// Post-order of the blocks reachable from `root` that are not yet marked in `visited`;
/// marks every block it visits.
pub fn postorder_from(n: usize, succs: &[Vec<u32>], root: u32, visited: &mut [bool]) -> Vec<u32> {
    let mut postorder = Vec::new();
    if n == 0 || (root as usize) >= n || (root as usize) >= visited.len() || visited[root as usize]
    {
        return postorder;
    }

    let mut stack: Vec<(u32, usize)> = vec![(root, 0)];
    visited[root as usize] = true;

    while let Some(top) = stack.last_mut() {
        let b = top.0;
        let children: &[u32] = succs.get(b as usize).map(|v| v.as_slice()).unwrap_or(&[]);
        if top.1 < children.len() {
            let s = children[top.1];
            top.1 += 1;
            let si = s as usize;
            if si < n && si < visited.len() && !visited[si] {
                visited[si] = true;
                stack.push((s, 0));
            }
        } else {
            postorder.push(b);
            stack.pop();
        }
    }

    postorder
}
