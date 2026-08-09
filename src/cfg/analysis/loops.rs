//! Loop Analysis & Back Edge Detection via DFS spanning tree traversal.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Color {
    White,
    Gray,
    Black,
}

#[derive(Debug, Clone)]
pub struct LoopInfo {
    pub back_edges: Vec<(u32, u32)>, // (tail_block, header_block)
    pub loop_depth: Vec<u8>,
    pub max_loop_depth: u8,
}

pub fn analyze_loops(n: usize, succs: &[Vec<u32>]) -> LoopInfo {
    if n == 0 {
        return LoopInfo {
            back_edges: Vec::new(),
            loop_depth: Vec::new(),
            max_loop_depth: 0,
        };
    }

    let mut color = vec![Color::White; n];
    let mut back_edges = Vec::new();

    fn dfs(b: u32, succs: &[Vec<u32>], color: &mut Vec<Color>, back_edges: &mut Vec<(u32, u32)>) {
        let b_idx = b as usize;
        color[b_idx] = Color::Gray;

        if let Some(children) = succs.get(b_idx) {
            for &s in children {
                let s_idx = s as usize;
                if s_idx < color.len() {
                    match color[s_idx] {
                        Color::White => dfs(s, succs, color, back_edges),
                        Color::Gray => back_edges.push((b, s)),
                        Color::Black => {}
                    }
                }
            }
        }
        color[b_idx] = Color::Black;
    }

    dfs(0, succs, &mut color, &mut back_edges);

    let mut loop_depth = vec![0u8; n];

    // Compute predecessors map
    let mut preds = vec![Vec::new(); n];
    for (src, list) in succs.iter().enumerate() {
        for &dst in list {
            if (dst as usize) < n {
                preds[dst as usize].push(src as u32);
            }
        }
    }

    for &(tail, header) in &back_edges {
        let mut in_loop = vec![false; n];
        if (header as usize) < n {
            in_loop[header as usize] = true;
        }

        let mut worklist = vec![tail];
        while let Some(b) = worklist.pop() {
            let b_idx = b as usize;
            if b_idx < n && !in_loop[b_idx] {
                in_loop[b_idx] = true;
                for &p in &preds[b_idx] {
                    if (p as usize) < n && !in_loop[p as usize] {
                        worklist.push(p);
                    }
                }
            }
        }

        for i in 0..n {
            if in_loop[i] {
                loop_depth[i] = loop_depth[i].saturating_add(1);
            }
        }
    }

    let max_loop_depth = loop_depth.iter().copied().max().unwrap_or(0);

    LoopInfo {
        back_edges,
        loop_depth,
        max_loop_depth,
    }
}
