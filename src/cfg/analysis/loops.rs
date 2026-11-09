//! Loop nesting forest: natural loops merged per header (as LLVM LoopInfo does), with
//! detection of irreducible control flow (Havlak 1997, Ramalingam 2002).
//!
//! An edge `t -> h` is a back edge only when `h` dominates `t`. All back edges into the
//! same header form one loop, so the several `continue` edges of one `while` loop do not
//! inflate the nesting depth. Retreating DFS edges whose target does not dominate their
//! source mark irreducible regions; they are reported and never treated as loops.

use super::dominators::DominatorTree;

#[derive(Debug, Clone, Default)]
pub struct LoopInfo {
    /// (tail_block, header_block) pairs where the header dominates the tail.
    pub back_edges: Vec<(u32, u32)>,
    /// Number of natural loops containing each block.
    pub loop_depth: Vec<u8>,
    pub max_loop_depth: u8,
    /// One header per natural loop, ordered from outermost to innermost.
    pub headers: Vec<u32>,
    /// For each entry of `headers`, the index of the enclosing loop or `u32::MAX`.
    pub loop_parent: Vec<u32>,
    /// Retreating edges whose target does not dominate their source (irreducible flow).
    pub irreducible_edges: Vec<(u32, u32)>,
}

impl LoopInfo {
    pub fn is_irreducible(&self) -> bool {
        !self.irreducible_edges.is_empty()
    }
}

pub fn analyze_loops(
    n: usize,
    succs: &[Vec<u32>],
    preds: &[Vec<u32>],
    dom: &DominatorTree,
) -> LoopInfo {
    if n == 0 {
        return LoopInfo::default();
    }

    // ── 1. Back edges (dominance) and irreducible retreating edges (DFS) ──
    let mut back_edges = Vec::new();
    for (u, list) in succs.iter().enumerate().take(n) {
        for &s in list {
            if (s as usize) < n && dom.dominates(s, u as u32) {
                back_edges.push((u as u32, s));
            }
        }
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Color {
        White,
        Gray,
        Black,
    }
    let mut irreducible_edges = Vec::new();
    let mut color = vec![Color::White; n];
    let mut stack: Vec<(u32, usize)> = vec![(dom.root, 0)];
    if (dom.root as usize) < n {
        color[dom.root as usize] = Color::Gray;
    } else {
        stack.clear();
    }
    while let Some(top) = stack.last_mut() {
        let b = top.0;
        let list: &[u32] = succs.get(b as usize).map(|v| v.as_slice()).unwrap_or(&[]);
        if top.1 < list.len() {
            let s = list[top.1];
            top.1 += 1;
            let si = s as usize;
            if si >= n {
                continue;
            }
            match color[si] {
                Color::White => {
                    color[si] = Color::Gray;
                    stack.push((s, 0));
                }
                Color::Gray => {
                    if !dom.dominates(s, b) {
                        irreducible_edges.push((b, s));
                    }
                }
                Color::Black => {}
            }
        } else {
            color[b as usize] = Color::Black;
            stack.pop();
        }
    }

    // ── 2. Natural loop bodies, one per header ──
    let mut by_header: Vec<(u32, Vec<u32>)> = Vec::new();
    {
        let mut sorted = back_edges.clone();
        sorted.sort_unstable_by_key(|&(t, h)| (h, t));
        for (t, h) in sorted {
            let same_header = by_header.last().is_some_and(|(hh, _)| *hh == h);
            if same_header {
                if let Some((_, tails)) = by_header.last_mut() {
                    tails.push(t);
                }
            } else {
                by_header.push((h, vec![t]));
            }
        }
    }

    let mut stamp = vec![u32::MAX; n];
    let mut bodies: Vec<(u32, Vec<u32>)> = Vec::with_capacity(by_header.len());
    for (loop_idx, (header, tails)) in by_header.iter().enumerate() {
        let tag = loop_idx as u32;
        let mut body = vec![*header];
        stamp[*header as usize] = tag;
        let mut worklist: Vec<u32> = Vec::new();
        for &t in tails {
            if stamp[t as usize] != tag {
                stamp[t as usize] = tag;
                body.push(t);
                worklist.push(t);
            }
        }
        while let Some(b) = worklist.pop() {
            for &p in preds.get(b as usize).map(|v| v.as_slice()).unwrap_or(&[]) {
                let pi = p as usize;
                // Only blocks dominated by the header belong to its natural loop.
                if pi < n && stamp[pi] != tag && dom.dominates(*header, p) {
                    stamp[pi] = tag;
                    body.push(p);
                    worklist.push(p);
                }
            }
        }
        bodies.push((*header, body));
    }

    // ── 3. Nesting: outermost (largest) loops first ──
    bodies.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then(a.0.cmp(&b.0)));
    let mut innermost = vec![u32::MAX; n];
    let mut loop_depth = vec![0u8; n];
    let mut headers = Vec::with_capacity(bodies.len());
    let mut loop_parent = Vec::with_capacity(bodies.len());
    for (idx, (header, body)) in bodies.iter().enumerate() {
        headers.push(*header);
        loop_parent.push(innermost[*header as usize]);
        for &b in body {
            innermost[b as usize] = idx as u32;
            loop_depth[b as usize] = loop_depth[b as usize].saturating_add(1);
        }
    }

    let max_loop_depth = loop_depth.iter().copied().max().unwrap_or(0);

    LoopInfo {
        back_edges,
        loop_depth,
        max_loop_depth,
        headers,
        loop_parent,
        irreducible_edges,
    }
}
