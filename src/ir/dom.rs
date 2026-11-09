//! Dominance over IR control flow: dominator and post-dominator trees (Cooper, Harvey,
//! Kennedy: "A Simple, Fast Dominance Algorithm", 2001), dominance frontiers (Cytron et al.
//! 1991, computed with the CHK runner scheme), natural loops and the loop nesting forest.
//!
//! The graphs include exceptional edges (`Call::unwind`, `BlockIr::unwind`), so a landing
//! block is dominated by the protected code that reaches it.
//!
//! Post-dominance is computed on the reverse graph rooted at a virtual exit node
//! `BlockId(n)` (one past the last block) that every exit block reaches: returns and throws
//! that leave the function. Regions that reach no exit (infinite loops, blocks ending in
//! `Unreachable`) are connected to the virtual exit through one representative block of
//! every terminal strongly connected component, so every block has a post-dominator; the
//! added connections are reported in `PostDomTree::virtual_edges`.

use crate::ir::func::FuncIr;
use crate::ir::types::BlockId;

const UNDEF: u32 = u32::MAX;

/// Dominator tree of a rooted graph with O(1) ancestry queries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DomTree {
    root: u32,
    idom: Vec<u32>,
    children: Vec<Vec<BlockId>>,
    /// Reverse post-order of the nodes reachable from the root.
    rpo: Vec<BlockId>,
    /// DFS entry / exit numbers in the dominator tree, for ancestry tests.
    pre: Vec<u32>,
    post: Vec<u32>,
    depth: Vec<u32>,
}

impl DomTree {
    /// Builds the tree of a graph given as successor lists.
    fn build(n: usize, root: u32, succs: &[Vec<u32>]) -> Self {
        let mut preds = vec![Vec::new(); n];
        for (u, list) in succs.iter().enumerate() {
            for &v in list {
                if (v as usize) < n && !preds[v as usize].contains(&(u as u32)) {
                    preds[v as usize].push(u as u32);
                }
            }
        }
        let rpo = reverse_post_order(n, root, succs);
        let idom = cooper_idom(n, &preds, &rpo);

        let mut children = vec![Vec::new(); n];
        for &b in &rpo {
            let d = idom[b as usize];
            if d != UNDEF && d != b {
                children[d as usize].push(BlockId(b));
            }
        }

        let mut pre = vec![UNDEF; n];
        let mut post = vec![UNDEF; n];
        let mut depth = vec![0u32; n];
        if (root as usize) < n {
            let mut clock = 0u32;
            let mut stack: Vec<(u32, usize)> = vec![(root, 0)];
            pre[root as usize] = clock;
            clock += 1;
            while let Some(top) = stack.last_mut() {
                let (node, next) = (top.0, top.1);
                if next < children[node as usize].len() {
                    top.1 += 1;
                    let c = children[node as usize][next].0;
                    pre[c as usize] = clock;
                    clock += 1;
                    depth[c as usize] = depth[node as usize] + 1;
                    stack.push((c, 0));
                } else {
                    post[node as usize] = clock;
                    clock += 1;
                    stack.pop();
                }
            }
        }

        Self {
            root,
            idom,
            children,
            rpo: rpo.into_iter().map(BlockId).collect(),
            pre,
            post,
            depth,
        }
    }

    pub fn root(&self) -> BlockId {
        BlockId(self.root)
    }

    /// Number of nodes the tree is defined over.
    pub fn node_count(&self) -> usize {
        self.idom.len()
    }

    /// Immediate dominator; `None` for the root and for unreachable nodes.
    pub fn idom(&self, b: BlockId) -> Option<BlockId> {
        match self.idom.get(b.index()) {
            Some(&d) if d != UNDEF && d != b.0 => Some(BlockId(d)),
            _ => None,
        }
    }

    pub fn is_reachable(&self, b: BlockId) -> bool {
        self.idom.get(b.index()).is_some_and(|&d| d != UNDEF)
    }

    /// `a` dominates `b` (reflexive).
    pub fn dominates(&self, a: BlockId, b: BlockId) -> bool {
        if !self.is_reachable(a) || !self.is_reachable(b) {
            return false;
        }
        self.pre[a.index()] <= self.pre[b.index()] && self.post[b.index()] <= self.post[a.index()]
    }

    pub fn strictly_dominates(&self, a: BlockId, b: BlockId) -> bool {
        a != b && self.dominates(a, b)
    }

    pub fn children(&self, b: BlockId) -> &[BlockId] {
        self.children.get(b.index()).map(|c| c.as_slice()).unwrap_or(&[])
    }

    /// Depth in the tree (root = 0).
    pub fn depth(&self, b: BlockId) -> u32 {
        self.depth.get(b.index()).copied().unwrap_or(0)
    }

    /// Reverse post-order of the reachable nodes (root first).
    pub fn rpo(&self) -> &[BlockId] {
        &self.rpo
    }

    /// Nearest common dominator of two reachable nodes.
    pub fn common_dominator(&self, a: BlockId, b: BlockId) -> Option<BlockId> {
        if !self.is_reachable(a) || !self.is_reachable(b) {
            return None;
        }
        let (mut x, mut y) = (a, b);
        while self.depth(x) > self.depth(y) {
            x = self.idom(x)?;
        }
        while self.depth(y) > self.depth(x) {
            y = self.idom(y)?;
        }
        while x != y {
            x = self.idom(x)?;
            y = self.idom(y)?;
        }
        Some(x)
    }

    /// Nodes of the tree in preorder (root first).
    pub fn preorder(&self) -> Vec<BlockId> {
        let mut out = Vec::with_capacity(self.rpo.len());
        if (self.root as usize) >= self.idom.len() {
            return out;
        }
        let mut stack = vec![BlockId(self.root)];
        while let Some(b) = stack.pop() {
            out.push(b);
            for &c in self.children(b).iter().rev() {
                stack.push(c);
            }
        }
        out
    }
}

/// Post-dominator tree with a virtual exit node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostDomTree {
    tree: DomTree,
    virtual_exit: BlockId,
    /// Blocks that reach no exit and were connected to the virtual exit.
    virtual_edges: Vec<BlockId>,
    /// Reverse-graph successor lists (graph successors of the virtual exit included).
    reverse_succs: Vec<Vec<u32>>,
}

impl PostDomTree {
    /// The virtual exit node (`BlockId(block_count)`).
    pub fn virtual_exit(&self) -> BlockId {
        self.virtual_exit
    }

    pub fn virtual_edges(&self) -> &[BlockId] {
        &self.virtual_edges
    }

    /// Immediate post-dominator; the virtual exit is returned as `virtual_exit()`.
    pub fn ipdom(&self, b: BlockId) -> Option<BlockId> {
        self.tree.idom(b)
    }

    /// `a` post-dominates `b` (reflexive).
    pub fn post_dominates(&self, a: BlockId, b: BlockId) -> bool {
        self.tree.dominates(a, b)
    }

    pub fn strictly_post_dominates(&self, a: BlockId, b: BlockId) -> bool {
        self.tree.strictly_dominates(a, b)
    }

    pub fn children(&self, b: BlockId) -> &[BlockId] {
        self.tree.children(b)
    }

    pub fn tree(&self) -> &DomTree {
        &self.tree
    }
}

/// Dominator tree of a function (rooted at its entry).
pub fn dominator_tree(func: &FuncIr) -> DomTree {
    let succs = successor_graph(func);
    DomTree::build(func.blocks.len(), func.entry.0, &succs)
}

/// Post-dominator tree of a function.
pub fn post_dominator_tree(func: &FuncIr) -> PostDomTree {
    let n = func.blocks.len();
    let exit = n as u32;
    let succs = successor_graph(func);

    let mut exits: Vec<u32> = func.exit_blocks().into_iter().map(|b| b.0).collect();
    let mut virtual_edges = Vec::new();

    // Connect the terminal strongly connected components that reach no exit.
    let reaches_exit = reverse_reachable(n, &succs, &exits);
    let forward_reachable = forward_reachable(n, func.entry.0, &succs);
    if (0..n).any(|i| forward_reachable[i] && !reaches_exit[i]) {
        let rpo_pos = {
            let order = reverse_post_order(n, func.entry.0, &succs);
            let mut pos = vec![UNDEF; n];
            for (i, &b) in order.iter().enumerate() {
                pos[b as usize] = i as u32;
            }
            pos
        };
        let stuck: Vec<bool> = (0..n)
            .map(|i| forward_reachable[i] && !reaches_exit[i])
            .collect();
        for scc in terminal_sccs(n, &succs, &stuck) {
            // The member latest in reverse post-order: for a loop, the block that closes it.
            if let Some(&rep) = scc.iter().max_by_key(|&&b| rpo_pos[b as usize]) {
                exits.push(rep);
                virtual_edges.push(BlockId(rep));
            }
        }
    }

    // Reverse graph over n + 1 nodes: the virtual exit leads to every exit block, and every
    // block leads to its graph predecessors.
    let mut reverse_succs: Vec<Vec<u32>> = vec![Vec::new(); n + 1];
    for (u, list) in succs.iter().enumerate() {
        for &v in list {
            if !reverse_succs[v as usize].contains(&(u as u32)) {
                reverse_succs[v as usize].push(u as u32);
            }
        }
    }
    for &e in &exits {
        if !reverse_succs[n].contains(&e) {
            reverse_succs[n].push(e);
        }
    }

    let tree = DomTree::build(n + 1, exit, &reverse_succs);
    PostDomTree {
        tree,
        virtual_exit: BlockId(exit),
        virtual_edges,
        reverse_succs,
    }
}

/// Dominance frontier of every block: `DF(b)` holds the joins where `b`'s dominance ends.
pub fn dominance_frontiers(func: &FuncIr, dom: &DomTree) -> Vec<Vec<BlockId>> {
    let n = func.blocks.len();
    let preds = func.predecessor_lists();
    let mut df: Vec<Vec<BlockId>> = vec![Vec::new(); n];
    for b in 0..n {
        let bid = BlockId::new(b);
        if preds[b].len() < 2 || !dom.is_reachable(bid) {
            continue;
        }
        let stop = dom.idom(bid);
        for &p in &preds[b] {
            let mut runner = Some(p);
            while let Some(r) = runner {
                if Some(r) == stop || !dom.is_reachable(r) {
                    break;
                }
                if !df[r.index()].contains(&bid) {
                    df[r.index()].push(bid);
                }
                runner = dom.idom(r);
            }
        }
    }
    df
}

/// Post-dominance frontier of every block (and of the virtual exit at index `n`):
/// `PDF(b)` holds the branch blocks `b` is control dependent on.
pub fn post_dominance_frontiers(func: &FuncIr, pdom: &PostDomTree) -> Vec<Vec<BlockId>> {
    let n = func.blocks.len();
    // Predecessors in the reverse graph are the graph successors, plus the virtual exit for
    // the exit blocks.
    let mut rpreds: Vec<Vec<u32>> = vec![Vec::new(); n + 1];
    for (u, list) in pdom.reverse_succs.iter().enumerate() {
        for &v in list {
            if !rpreds[v as usize].contains(&(u as u32)) {
                rpreds[v as usize].push(u as u32);
            }
        }
    }
    let tree = &pdom.tree;
    let mut pdf: Vec<Vec<BlockId>> = vec![Vec::new(); n + 1];
    for b in 0..=n {
        let bid = BlockId::new(b);
        if rpreds[b].len() < 2 || !tree.is_reachable(bid) {
            continue;
        }
        let stop = tree.idom(bid);
        for &p in &rpreds[b] {
            let mut runner = Some(BlockId(p));
            while let Some(r) = runner {
                if Some(r) == stop || !tree.is_reachable(r) {
                    break;
                }
                if !pdf[r.index()].contains(&bid) {
                    pdf[r.index()].push(bid);
                }
                runner = tree.idom(r);
            }
        }
    }
    pdf
}

/// Iterated dominance frontier of a set of blocks (the phi placement set of Cytron et al.).
pub fn iterated_frontier(df: &[Vec<BlockId>], defs: &[BlockId]) -> Vec<BlockId> {
    let mut in_set = vec![false; df.len()];
    let mut queued = vec![false; df.len()];
    let mut work: Vec<BlockId> = Vec::new();
    for &d in defs {
        if d.index() < df.len() && !queued[d.index()] {
            queued[d.index()] = true;
            work.push(d);
        }
    }
    let mut out = Vec::new();
    while let Some(b) = work.pop() {
        for &f in &df[b.index()] {
            if !in_set[f.index()] {
                in_set[f.index()] = true;
                out.push(f);
                if !queued[f.index()] {
                    queued[f.index()] = true;
                    work.push(f);
                }
            }
        }
    }
    out.sort();
    out
}

/// Natural loop: the blocks of every back edge into one header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NaturalLoop {
    pub header: BlockId,
    /// Sources of the back edges.
    pub latches: Vec<BlockId>,
    /// Member blocks, sorted, header included.
    pub blocks: Vec<BlockId>,
    /// Edges leaving the loop (inside block, outside successor).
    pub exits: Vec<(BlockId, BlockId)>,
    /// Index of the innermost enclosing loop.
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    /// Nesting depth (outermost = 1).
    pub depth: u32,
}

impl NaturalLoop {
    pub fn contains(&self, b: BlockId) -> bool {
        self.blocks.binary_search(&b).is_ok()
    }
}

/// Loop nesting forest of a function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoopForest {
    pub loops: Vec<NaturalLoop>,
    /// Innermost loop of each block.
    pub loop_of: Vec<Option<usize>>,
    /// Retreating edges whose target does not dominate their source: the cycles they close
    /// are irreducible and have no natural loop.
    pub irreducible_edges: Vec<(BlockId, BlockId)>,
}

impl LoopForest {
    /// Nesting depth of a block (0 outside every loop).
    pub fn depth_of(&self, b: BlockId) -> u32 {
        match self.loop_of.get(b.index()).copied().flatten() {
            Some(l) => self.loops[l].depth,
            None => 0,
        }
    }

    pub fn is_header(&self, b: BlockId) -> bool {
        self.loops.iter().any(|l| l.header == b)
    }

    /// Outermost loops.
    pub fn roots(&self) -> impl Iterator<Item = usize> + '_ {
        (0..self.loops.len()).filter(move |&i| self.loops[i].parent.is_none())
    }
}

/// Natural loops and their nesting.
pub fn loop_forest(func: &FuncIr, dom: &DomTree) -> LoopForest {
    let n = func.blocks.len();
    let succs = successor_graph(func);
    let preds = func.predecessor_lists();

    // Retreating edges from a DFS: a back edge when the target dominates the source.
    let (dfs_pre, dfs_post) = dfs_numbers(n, func.entry.0, &succs);
    let mut back_edges: Vec<(u32, u32)> = Vec::new();
    let mut irreducible_edges = Vec::new();
    for u in 0..n {
        if dfs_pre[u] == UNDEF {
            continue;
        }
        for &v in &succs[u] {
            let vi = v as usize;
            let retreating = dfs_pre[vi] != UNDEF
                && dfs_pre[vi] <= dfs_pre[u]
                && dfs_post[u] <= dfs_post[vi];
            if !retreating {
                continue;
            }
            if dom.dominates(BlockId(v), BlockId(u as u32)) {
                back_edges.push((u as u32, v));
            } else {
                irreducible_edges.push((BlockId(u as u32), BlockId(v)));
            }
        }
    }

    // One loop per header: the union of the natural loops of its back edges.
    let mut headers: Vec<u32> = back_edges.iter().map(|&(_, h)| h).collect();
    headers.sort_unstable();
    headers.dedup();
    let mut loops: Vec<NaturalLoop> = Vec::with_capacity(headers.len());
    for &h in &headers {
        let latches: Vec<u32> = back_edges
            .iter()
            .filter(|&&(_, hh)| hh == h)
            .map(|&(t, _)| t)
            .collect();
        let mut member = vec![false; n];
        member[h as usize] = true;
        let mut work: Vec<u32> = Vec::new();
        for &t in &latches {
            if !member[t as usize] {
                member[t as usize] = true;
                work.push(t);
            }
        }
        while let Some(x) = work.pop() {
            for &p in &preds[x as usize] {
                let pi = p.index();
                if !member[pi] && dom.is_reachable(p) {
                    member[pi] = true;
                    work.push(p.0);
                }
            }
        }
        let blocks: Vec<BlockId> = (0..n).filter(|&i| member[i]).map(BlockId::new).collect();
        let mut exits = Vec::new();
        for &b in &blocks {
            for &s in &succs[b.index()] {
                if !member[s as usize] {
                    exits.push((b, BlockId(s)));
                }
            }
        }
        loops.push(NaturalLoop {
            header: BlockId(h),
            latches: latches.into_iter().map(BlockId).collect(),
            blocks,
            exits,
            parent: None,
            children: Vec::new(),
            depth: 0,
        });
    }

    // Natural loops with distinct headers are nested or disjoint: the parent of a loop is
    // the smallest other loop that contains its header.
    let mut order: Vec<usize> = (0..loops.len()).collect();
    order.sort_by_key(|&i| loops[i].blocks.len());
    for (pos, &i) in order.iter().enumerate() {
        let header = loops[i].header;
        let parent = order[pos + 1..]
            .iter()
            .copied()
            .find(|&j| loops[j].contains(header) && loops[j].blocks.len() > loops[i].blocks.len());
        loops[i].parent = parent;
        if let Some(p) = parent {
            loops[p].children.push(i);
        }
    }
    for &i in order.iter().rev() {
        loops[i].depth = match loops[i].parent {
            Some(p) => loops[p].depth + 1,
            None => 1,
        };
    }

    let mut loop_of: Vec<Option<usize>> = vec![None; n];
    for &i in &order {
        for &b in &loops[i].blocks {
            if loop_of[b.index()].is_none() {
                loop_of[b.index()] = Some(i);
            }
        }
    }

    LoopForest {
        loops,
        loop_of,
        irreducible_edges,
    }
}

/// Successor lists (normal and exceptional) as raw indices.
fn successor_graph(func: &FuncIr) -> Vec<Vec<u32>> {
    func.blocks
        .iter()
        .map(|b| {
            b.successors()
                .into_iter()
                .map(|s| s.0)
                .filter(|&s| (s as usize) < func.blocks.len())
                .collect()
        })
        .collect()
}

fn reverse_post_order(n: usize, root: u32, succs: &[Vec<u32>]) -> Vec<u32> {
    let mut post = Vec::with_capacity(n);
    if (root as usize) >= n {
        return post;
    }
    let mut seen = vec![false; n];
    let mut stack: Vec<(u32, usize)> = vec![(root, 0)];
    seen[root as usize] = true;
    while let Some(top) = stack.last_mut() {
        let (node, next) = (top.0, top.1);
        let list = &succs[node as usize];
        if next < list.len() {
            top.1 += 1;
            let s = list[next];
            if (s as usize) < n && !seen[s as usize] {
                seen[s as usize] = true;
                stack.push((s, 0));
            }
        } else {
            post.push(node);
            stack.pop();
        }
    }
    post.reverse();
    post
}

fn dfs_numbers(n: usize, root: u32, succs: &[Vec<u32>]) -> (Vec<u32>, Vec<u32>) {
    let mut pre = vec![UNDEF; n];
    let mut post = vec![UNDEF; n];
    if (root as usize) >= n {
        return (pre, post);
    }
    let mut clock = 0u32;
    let mut stack: Vec<(u32, usize)> = vec![(root, 0)];
    pre[root as usize] = clock;
    clock += 1;
    while let Some(top) = stack.last_mut() {
        let (node, next) = (top.0, top.1);
        let list = &succs[node as usize];
        if next < list.len() {
            top.1 += 1;
            let s = list[next];
            if (s as usize) < n && pre[s as usize] == UNDEF {
                pre[s as usize] = clock;
                clock += 1;
                stack.push((s, 0));
            }
        } else {
            post[node as usize] = clock;
            clock += 1;
            stack.pop();
        }
    }
    (pre, post)
}

/// Cooper-Harvey-Kennedy iterative dominators; `rpo[0]` is the root.
fn cooper_idom(n: usize, preds: &[Vec<u32>], rpo: &[u32]) -> Vec<u32> {
    let mut idom = vec![UNDEF; n];
    if rpo.is_empty() {
        return idom;
    }
    let mut rpo_num = vec![UNDEF; n];
    for (i, &b) in rpo.iter().enumerate() {
        rpo_num[b as usize] = i as u32;
    }
    let root = rpo[0];
    idom[root as usize] = root;
    let mut changed = true;
    while changed {
        changed = false;
        for &b in &rpo[1..] {
            let mut new_idom = UNDEF;
            for &p in &preds[b as usize] {
                if idom[p as usize] == UNDEF {
                    continue;
                }
                new_idom = if new_idom == UNDEF {
                    p
                } else {
                    intersect(p, new_idom, &idom, &rpo_num)
                };
            }
            if new_idom != UNDEF && idom[b as usize] != new_idom {
                idom[b as usize] = new_idom;
                changed = true;
            }
        }
    }
    idom
}

fn intersect(mut a: u32, mut b: u32, idom: &[u32], rpo_num: &[u32]) -> u32 {
    while a != b {
        while rpo_num[a as usize] > rpo_num[b as usize] {
            a = idom[a as usize];
        }
        while rpo_num[b as usize] > rpo_num[a as usize] {
            b = idom[b as usize];
        }
    }
    a
}

fn forward_reachable(n: usize, root: u32, succs: &[Vec<u32>]) -> Vec<bool> {
    let mut seen = vec![false; n];
    if (root as usize) >= n {
        return seen;
    }
    let mut stack = vec![root];
    seen[root as usize] = true;
    while let Some(u) = stack.pop() {
        for &v in &succs[u as usize] {
            if (v as usize) < n && !seen[v as usize] {
                seen[v as usize] = true;
                stack.push(v);
            }
        }
    }
    seen
}

/// Nodes from which one of `targets` is reachable.
fn reverse_reachable(n: usize, succs: &[Vec<u32>], targets: &[u32]) -> Vec<bool> {
    let mut preds = vec![Vec::new(); n];
    for (u, list) in succs.iter().enumerate() {
        for &v in list {
            if (v as usize) < n {
                preds[v as usize].push(u as u32);
            }
        }
    }
    let mut seen = vec![false; n];
    let mut stack: Vec<u32> = Vec::new();
    for &t in targets {
        if (t as usize) < n && !seen[t as usize] {
            seen[t as usize] = true;
            stack.push(t);
        }
    }
    while let Some(v) = stack.pop() {
        for &u in &preds[v as usize] {
            if !seen[u as usize] {
                seen[u as usize] = true;
                stack.push(u);
            }
        }
    }
    seen
}

/// Strongly connected components of the subgraph induced by `member` that have no edge to
/// another component of that subgraph.
fn terminal_sccs(n: usize, succs: &[Vec<u32>], member: &[bool]) -> Vec<Vec<u32>> {
    let mut index = vec![UNDEF; n];
    let mut low = vec![UNDEF; n];
    let mut on_stack = vec![false; n];
    let mut comp = vec![UNDEF; n];
    let mut stack: Vec<u32> = Vec::new();
    let mut next = 0u32;
    let mut sccs: Vec<Vec<u32>> = Vec::new();

    for root in 0..n as u32 {
        if !member[root as usize] || index[root as usize] != UNDEF {
            continue;
        }
        let mut call: Vec<(u32, usize)> = vec![(root, 0)];
        index[root as usize] = next;
        low[root as usize] = next;
        next += 1;
        stack.push(root);
        on_stack[root as usize] = true;
        while let Some(top) = call.last_mut() {
            let (v, pos) = (top.0, top.1);
            let list = &succs[v as usize];
            if pos < list.len() {
                top.1 += 1;
                let w = list[pos];
                if !member[w as usize] {
                    continue;
                }
                if index[w as usize] == UNDEF {
                    index[w as usize] = next;
                    low[w as usize] = next;
                    next += 1;
                    stack.push(w);
                    on_stack[w as usize] = true;
                    call.push((w, 0));
                } else if on_stack[w as usize] {
                    low[v as usize] = low[v as usize].min(index[w as usize]);
                }
            } else {
                call.pop();
                if let Some(parent) = call.last() {
                    let p = parent.0 as usize;
                    low[p] = low[p].min(low[v as usize]);
                }
                if low[v as usize] == index[v as usize] {
                    let id = sccs.len() as u32;
                    let mut scc = Vec::new();
                    while let Some(w) = stack.pop() {
                        on_stack[w as usize] = false;
                        comp[w as usize] = id;
                        scc.push(w);
                        if w == v {
                            break;
                        }
                    }
                    sccs.push(scc);
                }
            }
        }
    }

    sccs.into_iter()
        .enumerate()
        .filter(|(id, scc)| {
            scc.iter().all(|&u| {
                succs[u as usize].iter().all(|&w| {
                    !member[w as usize] || comp[w as usize] == *id as u32
                })
            })
        })
        .map(|(_, scc)| scc)
        .collect()
}
