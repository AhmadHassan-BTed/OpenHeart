//! Reverse Post-Order (RPO) Traversal algorithm for CFGs.

pub fn reverse_postorder(n: usize, succs: &[Vec<u32>]) -> Vec<u32> {
    let mut visited = vec![false; n];
    let mut postorder = Vec::with_capacity(n);

    fn dfs(b: u32, succs: &[Vec<u32>], visited: &mut Vec<bool>, postorder: &mut Vec<u32>) {
        visited[b as usize] = true;
        if let Some(children) = succs.get(b as usize) {
            for &s in children {
                if (s as usize) < visited.len() && !visited[s as usize] {
                    dfs(s, succs, visited, postorder);
                }
            }
        }
        postorder.push(b);
    }

    if n > 0 {
        dfs(0, succs, &mut visited, &mut postorder);
    }

    postorder.reverse();
    postorder
}
