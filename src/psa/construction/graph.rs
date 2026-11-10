//! PathGraph — an edge-indexed view of one function's CFG for path analysis (§8.2.3, §8.2.5).
//!
//! Phase 4 emits edges as `(from, to, type)` triples plus block-level CSR adjacency. Path
//! analysis needs *edge ids*: parallel edges are distinct paths, and every per-edge lookup
//! must be O(1). `PathGraph` builds edge-id CSR tables once per function in O(V + E)
//! (successor and predecessor edge ids, each list in `cfg.edges` order), replacing the
//! former `edge_index_of` linear scans that made every pass O(E²) and merged parallel edges.
//!
//! The input is a language-neutral [`PathCfg`] (blocks, entry, `(from, to, kind)` edges),
//! produced from a Phase 4 CFG by [`PathCfg::from_cfa`] or by any other frontend's CFG.
//!
//! On top of that it classifies edges with one iterative DFS from the entry block:
//!
//! - **back edges** (target on the DFS stack) — removing them leaves a DAG;
//! - **inactive edges** — an endpoint outside the block range, a source unreachable from
//!   the entry, or an exceptional edge excluded by [`ExceptionalEdges::Exclude`]; they can
//!   lie on no path;
//! - **loop headers** — distinct targets of back edges.
//!
//! **Path model (Ball & Larus, MICRO 1996).** Each back edge `t → h` is replaced by the dummy
//! edges `ENTRY → h` and `t → EXIT`. The resulting DAG's ENTRY→EXIT paths are the *acyclic
//! path segments* of the function: a segment starts at ENTRY or at a loop header and ends at
//! a block without successors or by taking a back edge. For an acyclic CFG these are exactly
//! the ENTRY→EXIT paths; for a cyclic CFG every execution is a concatenation of segments, and
//! the function is flagged as containing loops. Both metrics below are O(V + E) dynamic
//! programs over the DAG in topological order.

use crate::cfg::builder::FunctionCFGData;
use crate::core::types::cfg::CFGEdgeType;
use crate::psa::count::PathCount;

/// The ENTRY block of every Phase 4 CFG: Phase 4 numbers blocks in reverse post-order from
/// ENTRY, so ENTRY is block 0. Neutral CFGs name their entry explicitly ([`PathCfg::entry`]).
pub const ENTRY_BLOCK: u32 = 0;

/// Language-neutral classification of a CFG edge — exactly what path analysis needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PathEdgeKind {
    /// Unconditional transfer: fall-through, jump, return, loop back edge.
    Normal,
    /// Taken when a two-way branch condition holds.
    True,
    /// Taken when a two-way branch condition does not hold.
    False,
    /// One arm of a multi-way branch (switch case or default).
    Case,
    /// Exceptional transfer (throw to a handler, unwind out of a call).
    Exceptional,
}

impl PathEdgeKind {
    /// The neutral kind of a Phase 4 edge.
    pub fn from_cfa(t: CFGEdgeType) -> Self {
        match t {
            CFGEdgeType::True => PathEdgeKind::True,
            CFGEdgeType::False => PathEdgeKind::False,
            CFGEdgeType::Except => PathEdgeKind::Exceptional,
            CFGEdgeType::Switch => PathEdgeKind::Case,
            CFGEdgeType::Uncond | CFGEdgeType::Return | CFGEdgeType::LoopBack => {
                PathEdgeKind::Normal
            }
        }
    }
}

/// Whether exceptional edges belong to the path model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExceptionalEdges {
    /// Exceptional edges are ordinary path alternatives.
    Include,
    /// Exceptional edges can lie on no path: the counted paths are the normal paths, and a
    /// block whose only successors are exceptional ends its path (it throws).
    Exclude,
}

/// One function's CFG in the language-neutral form Phase 8 consumes.
///
/// Any frontend can produce it: [`PathCfg::from_cfa`] maps a Phase 4 CFG, and an IR CFG
/// maps its terminator and unwind edges one to one. Edge ids are positions in `edges`; the
/// ordering tables of the resulting summary refer to them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathCfg {
    /// Number of blocks; blocks are `0..n_blocks`.
    pub n_blocks: usize,
    /// The entry block.
    pub entry: u32,
    /// `(from, to, kind)` per edge. Parallel edges are distinct; edges with an endpoint
    /// outside `0..n_blocks` are kept (so ids stay positions) and can lie on no path.
    pub edges: Vec<(u32, u32, PathEdgeKind)>,
}

impl PathCfg {
    /// The neutral form of a Phase 4 CFG (edge ids = positions in `cfg.edges`).
    pub fn from_cfa(cfg: &FunctionCFGData) -> Self {
        Self {
            n_blocks: cfg.blocks.len(),
            entry: ENTRY_BLOCK,
            edges: cfg
                .edges
                .iter()
                .map(|&(u, v, t)| (u, v, PathEdgeKind::from_cfa(t)))
                .collect(),
        }
    }

    /// Structural lower bound of McCabe's V(G) = decision points + 1: a block with k
    /// non-exceptional out-edges (valid endpoints) contributes k − 1 decisions. Producers
    /// that also count handler clauses (Phase 4 adds one per catch clause) report a value at
    /// least this large.
    pub fn structural_cyclomatic(&self) -> u64 {
        let mut out_degree = vec![0u64; self.n_blocks];
        for &(u, v, kind) in &self.edges {
            if kind == PathEdgeKind::Exceptional {
                continue;
            }
            if (u as usize) < self.n_blocks && (v as usize) < self.n_blocks {
                out_degree[u as usize] += 1;
            }
        }
        1 + out_degree.iter().map(|&k| k.saturating_sub(1)).sum::<u64>()
    }
}

/// Edge-indexed, DFS-classified view of a function CFG.
pub struct PathGraph {
    n_blocks: usize,
    entry: u32,
    edges: Vec<(u32, u32, PathEdgeKind)>,
    succ_offsets: Vec<u32>,
    succ_edges: Vec<u32>,
    pred_offsets: Vec<u32>,
    pred_edges: Vec<u32>,
    reachable: Vec<bool>,
    back_edge: Vec<bool>,
    inactive: Vec<bool>,
    excluded: usize,
    is_header: Vec<bool>,
    headers: Vec<u32>,
    topo: Vec<u32>,
}

impl PathGraph {
    /// Build the edge-id CSR tables and the DFS classification for `cfg`.
    ///
    /// With [`ExceptionalEdges::Exclude`], exceptional edges are treated like edges with an
    /// invalid endpoint: inactive, absent from the adjacency, constrained to 0.
    pub fn new(cfg: &PathCfg, exceptional: ExceptionalEdges) -> Self {
        let n_blocks = cfg.n_blocks;
        let entry = cfg.entry;
        let edges = cfg.edges.clone();
        let n_edges = edges.len();
        let entry_valid = (entry as usize) < n_blocks;

        // ── Edge-id CSR (counting sort keeps each list in edge-id order) ─────
        let mut inactive = vec![false; n_edges];
        let mut excluded = 0usize;
        let mut succ_offsets = vec![0u32; n_blocks + 1];
        let mut pred_offsets = vec![0u32; n_blocks + 1];
        for (e, &(u, v, kind)) in edges.iter().enumerate() {
            let dropped =
                exceptional == ExceptionalEdges::Exclude && kind == PathEdgeKind::Exceptional;
            if dropped {
                excluded += 1;
            }
            if !dropped && (u as usize) < n_blocks && (v as usize) < n_blocks {
                succ_offsets[u as usize + 1] += 1;
                pred_offsets[v as usize + 1] += 1;
            } else {
                inactive[e] = true;
            }
        }
        for i in 0..n_blocks {
            succ_offsets[i + 1] += succ_offsets[i];
            pred_offsets[i + 1] += pred_offsets[i];
        }
        let mut succ_edges = vec![0u32; succ_offsets[n_blocks] as usize];
        let mut pred_edges = vec![0u32; pred_offsets[n_blocks] as usize];
        let mut succ_fill: Vec<u32> = succ_offsets[..n_blocks].to_vec();
        let mut pred_fill: Vec<u32> = pred_offsets[..n_blocks].to_vec();
        for (e, &(u, v, _)) in edges.iter().enumerate() {
            if inactive[e] {
                continue;
            }
            succ_edges[succ_fill[u as usize] as usize] = e as u32;
            succ_fill[u as usize] += 1;
            pred_edges[pred_fill[v as usize] as usize] = e as u32;
            pred_fill[v as usize] += 1;
        }

        // ── Iterative DFS from the entry: back edges and post-order ──────────
        const WHITE: u8 = 0;
        const GRAY: u8 = 1;
        const BLACK: u8 = 2;
        let mut color = vec![WHITE; n_blocks];
        let mut back_edge = vec![false; n_edges];
        let mut postorder: Vec<u32> = Vec::with_capacity(n_blocks);
        if entry_valid {
            color[entry as usize] = GRAY;
            let mut stack: Vec<(u32, u32)> = vec![(entry, succ_offsets[entry as usize])];
            while let Some(top) = stack.last_mut() {
                let (b, pos) = *top;
                if pos < succ_offsets[b as usize + 1] {
                    top.1 += 1;
                    let e = succ_edges[pos as usize];
                    let v = edges[e as usize].1;
                    match color[v as usize] {
                        WHITE => {
                            color[v as usize] = GRAY;
                            stack.push((v, succ_offsets[v as usize]));
                        }
                        GRAY => back_edge[e as usize] = true,
                        _ => {}
                    }
                } else {
                    color[b as usize] = BLACK;
                    postorder.push(b);
                    stack.pop();
                }
            }
        }
        let reachable: Vec<bool> = color.iter().map(|&c| c != WHITE).collect();

        // Edges leaving unreachable blocks can lie on no path.
        for (e, &(u, _, _)) in edges.iter().enumerate() {
            if !inactive[e] && !reachable[u as usize] {
                inactive[e] = true;
            }
        }

        let mut is_header = vec![false; n_blocks];
        for (e, &(_, v, _)) in edges.iter().enumerate() {
            if back_edge[e] {
                is_header[v as usize] = true;
            }
        }
        let headers: Vec<u32> = (0..n_blocks as u32)
            .filter(|&b| is_header[b as usize])
            .collect();

        // Reverse post-order of a DFS is a topological order of the graph minus back edges.
        postorder.reverse();

        Self {
            n_blocks,
            entry,
            edges,
            succ_offsets,
            succ_edges,
            pred_offsets,
            pred_edges,
            reachable,
            back_edge,
            inactive,
            excluded,
            is_header,
            headers,
            topo: postorder,
        }
    }

    /// The entry block.
    #[inline]
    pub fn entry(&self) -> u32 {
        self.entry
    }

    /// True iff the entry block is one of the CFG's blocks.
    #[inline]
    pub fn entry_is_valid(&self) -> bool {
        (self.entry as usize) < self.n_blocks
    }

    /// Number of exceptional edges removed by [`ExceptionalEdges::Exclude`].
    #[inline]
    pub fn excluded_exceptional_edges(&self) -> usize {
        self.excluded
    }

    /// Number of blocks in the CFG.
    #[inline]
    pub fn n_blocks(&self) -> usize {
        self.n_blocks
    }

    /// Number of edges in the CFG (`cfg.edges.len()`).
    #[inline]
    pub fn n_edges(&self) -> usize {
        self.edges.len()
    }

    /// `(from, to, kind)` of edge `e`.
    #[inline]
    pub fn edge(&self, e: u32) -> (u32, u32, PathEdgeKind) {
        self.edges[e as usize]
    }

    /// Ids of the active edges leaving `block` (valid endpoints, not excluded), in edge-id
    /// order.
    #[inline]
    pub fn out_edges(&self, block: u32) -> &[u32] {
        let b = block as usize;
        if b >= self.n_blocks {
            return &[];
        }
        &self.succ_edges[self.succ_offsets[b] as usize..self.succ_offsets[b + 1] as usize]
    }

    /// Ids of the active edges entering `block` (valid endpoints, not excluded), in edge-id
    /// order.
    #[inline]
    pub fn in_edges(&self, block: u32) -> &[u32] {
        let b = block as usize;
        if b >= self.n_blocks {
            return &[];
        }
        &self.pred_edges[self.pred_offsets[b] as usize..self.pred_offsets[b + 1] as usize]
    }

    /// True iff `block` is reachable from ENTRY.
    #[inline]
    pub fn is_reachable(&self, block: u32) -> bool {
        self.reachable.get(block as usize).copied().unwrap_or(false)
    }

    /// True iff edge `e` is a DFS back edge (closes a loop).
    #[inline]
    pub fn is_back_edge(&self, e: u32) -> bool {
        self.back_edge[e as usize]
    }

    /// True iff edge `e` can lie on no path from ENTRY.
    #[inline]
    pub fn is_inactive(&self, e: u32) -> bool {
        self.inactive[e as usize]
    }

    /// True iff edge `e` belongs to the acyclic DAG (active and not a back edge).
    #[inline]
    pub fn is_forward(&self, e: u32) -> bool {
        !self.inactive[e as usize] && !self.back_edge[e as usize]
    }

    /// True iff `block` is the target of a back edge.
    #[inline]
    pub fn is_header(&self, block: u32) -> bool {
        self.is_header.get(block as usize).copied().unwrap_or(false)
    }

    /// Loop headers in increasing block order.
    #[inline]
    pub fn headers(&self) -> &[u32] {
        &self.headers
    }

    /// Reachable blocks in a topological order of the DAG (the entry first).
    #[inline]
    pub fn topo_order(&self) -> &[u32] {
        &self.topo
    }

    /// True iff the CFG contains a cycle reachable from ENTRY.
    #[inline]
    pub fn has_loops(&self) -> bool {
        !self.headers.is_empty()
    }

    /// Number of back edges.
    pub fn back_edge_count(&self) -> usize {
        self.back_edge.iter().filter(|&&b| b).count()
    }

    /// Exact number of Ball-Larus acyclic path segments (= ENTRY→EXIT paths when acyclic).
    ///
    /// `NP(b)` = number of segment suffixes starting at `b`: 1 for a block without
    /// successors, otherwise the sum over its out-edges of 1 (a back edge ends the segment)
    /// or `NP(target)`. The total is `NP(entry) + Σ_headers NP(h)`. Exact, O(V + E).
    pub fn path_count(&self) -> PathCount {
        if !self.entry_is_valid() {
            return PathCount::zero();
        }
        let mut np: Vec<PathCount> = vec![PathCount::zero(); self.n_blocks];
        for &b in self.topo.iter().rev() {
            let outs = self.out_edges(b);
            let mut c = PathCount::zero();
            if outs.is_empty() {
                c = PathCount::one();
            } else {
                for &e in outs {
                    if self.back_edge[e as usize] {
                        c.add_assign(&PathCount::one());
                    } else {
                        let target = self.edges[e as usize].1 as usize;
                        c.add_assign(&np[target]);
                    }
                }
            }
            np[b as usize] = c;
        }
        let mut total = np[self.entry as usize].clone();
        for &h in &self.headers {
            total.add_assign(&np[h as usize]);
        }
        total
    }

    /// Number of blocks on the longest path from the entry in the DAG left after removing
    /// back edges (entry and final block included). O(V + E); 0 for an empty CFG.
    pub fn longest_path_blocks(&self) -> u64 {
        if !self.entry_is_valid() {
            return 0;
        }
        let mut lp = vec![0u64; self.n_blocks];
        for &b in self.topo.iter().rev() {
            let mut best = 0u64;
            for &e in self.out_edges(b) {
                if !self.back_edge[e as usize] {
                    best = best.max(lp[self.edges[e as usize].1 as usize]);
                }
            }
            lp[b as usize] = 1 + best;
        }
        lp[self.entry as usize]
    }
}
