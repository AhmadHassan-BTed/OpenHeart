//! CallGraphIndex — CSR call graph, SCC condensation and a reachability index (§10.3, §10.5).
//!
//! Method reachability is answered on the condensation DAG of the call graph:
//! - Tarjan component ids are assigned in completion order, so a component can only reach
//!   components with a *smaller* id. That order alone rejects roughly half of all queries.
//! - Graphs with at most `CLOSURE_COMPONENT_LIMIT` components keep a full transitive-closure
//!   bitset (O(1) queries).
//! - Larger graphs use GRAIL interval labels (Yildirim, Chaoji, Zaki; VLDB 2010) with
//!   `GRAIL_LABELINGS` independent DFS orders as a negative filter, followed by a pruned DFS.
//!
//! Every traversal uses CSR adjacency, so no query rescans the whole edge list.

use std::collections::VecDeque;

use crate::core::types::cg::*;
use crate::scpg::query::csr::{csr_from_pairs, csr_from_triples, row};
use crate::scpg::serializer::codec::{SectionDecoder, SectionEncoder};
use crate::scpg::types::ScpgFormatError;

pub const CLOSURE_COMPONENT_LIMIT: usize = 4096;
pub const GRAIL_LABELINGS: usize = 3;

const REACH_KIND_CLOSURE: u8 = 0;
const REACH_KIND_GRAIL: u8 = 1;

pub fn call_type_name(call_type: u8) -> &'static str {
    match call_type {
        CG_EDGE_DIRECT => "direct",
        CG_EDGE_SPECIAL => "special",
        CG_EDGE_VIRTUAL => "virtual",
        CG_EDGE_INTERFACE => "interface",
        CG_EDGE_CONSTRUCTOR => "constructor",
        CG_EDGE_DYNAMIC => "dynamic",
        CG_EDGE_REFLECTION => "reflection",
        _ => "unknown",
    }
}

/// One resolved or unresolved call site, with its resolved targets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallSiteRow {
    pub site_id: u32,
    pub caller: u32,
    pub call_node: u32,
    pub call_block: u32,
    pub call_token: u32,
    pub call_type: u8,
    pub flags: u8,
    pub arg_count: u16,
}

#[derive(Debug, Clone)]
enum ReachIndex {
    /// Row-major closure bitsets, `words` u64 words per component.
    Closure { words: usize, bits: Vec<u64> },
    /// `low`/`rank` per labeling, each `component_count` long, concatenated.
    Grail { low: Vec<u32>, rank: Vec<u32> },
}

#[derive(Debug, Clone)]
pub struct CallGraphIndex {
    node_count: usize,
    callee_offsets: Vec<u32>,
    callee_adj: Vec<u32>,
    callee_kinds: Vec<u8>,
    caller_offsets: Vec<u32>,
    caller_adj: Vec<u32>,

    sites: Vec<CallSiteRow>,
    site_target_offsets: Vec<u32>,
    site_target_adj: Vec<u32>,

    component_of: Vec<u32>,
    component_count: usize,
    component_recursive: Vec<u8>,
    dag_offsets: Vec<u32>,
    dag_adj: Vec<u32>,
    reach: ReachIndex,

    /// Points-to facts carried over from Phase 6 (`ssa_id`, allocated type symbol).
    points_to: Vec<(u32, u32)>,
}

impl CallGraphIndex {
    pub fn build(cga: &CallGraphArtifact) -> Self {
        let mut sites: Vec<CallSiteRow> = cga
            .call_sites
            .iter()
            .map(|s| CallSiteRow {
                site_id: s.call_site_id,
                caller: s.caller_sym,
                call_node: s.call_node,
                call_block: s.call_block,
                call_token: s.call_token,
                call_type: s.call_type,
                flags: s.flags,
                arg_count: s.arg_count,
            })
            .collect();
        sites.sort_by_key(|s| s.site_id);
        sites.dedup_by_key(|s| s.site_id);

        let mut node_count = cga.method_count as usize;
        for &(caller, callee, _) in &cga.site_to_edge_map {
            if caller != u32::MAX && callee != u32::MAX {
                node_count = node_count.max(caller as usize + 1).max(callee as usize + 1);
            }
        }

        let mut edges = Vec::with_capacity(cga.site_to_edge_map.len());
        let mut site_targets = Vec::with_capacity(cga.site_to_edge_map.len());
        for &(caller, callee, site_id) in &cga.site_to_edge_map {
            if caller == u32::MAX || callee == u32::MAX {
                continue;
            }
            let kind = sites
                .binary_search_by_key(&site_id, |s| s.site_id)
                .map(|i| sites[i].call_type)
                .unwrap_or(CG_EDGE_DIRECT);
            edges.push((caller, callee, kind));
            if let Ok(i) = sites.binary_search_by_key(&site_id, |s| s.site_id) {
                site_targets.push((i as u32, callee));
            }
        }

        let reverse: Vec<(u32, u32)> = edges.iter().map(|&(c, e, _)| (e, c)).collect();
        let (callee_offsets, callee_adj, callee_kinds) = csr_from_triples(node_count, edges, true);
        let (caller_offsets, caller_adj) = csr_from_pairs(node_count, reverse);
        let (site_target_offsets, site_target_adj) = csr_from_pairs(sites.len(), site_targets);

        let mut points_to: Vec<(u32, u32)> = cga
            .points_to_table
            .iter()
            .map(|p| (p.ssa_id, p.alloc_type_sym_id))
            .collect();
        points_to.sort_unstable();
        points_to.dedup();

        let (component_of, component_count) = tarjan(node_count, &callee_offsets, &callee_adj);
        Self::assemble(
            node_count,
            callee_offsets,
            callee_adj,
            callee_kinds,
            caller_offsets,
            caller_adj,
            sites,
            site_target_offsets,
            site_target_adj,
            component_of,
            component_count,
            points_to,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn assemble(
        node_count: usize,
        callee_offsets: Vec<u32>,
        callee_adj: Vec<u32>,
        callee_kinds: Vec<u8>,
        caller_offsets: Vec<u32>,
        caller_adj: Vec<u32>,
        sites: Vec<CallSiteRow>,
        site_target_offsets: Vec<u32>,
        site_target_adj: Vec<u32>,
        component_of: Vec<u32>,
        component_count: usize,
        points_to: Vec<(u32, u32)>,
        stored_reach: Option<ReachIndex>,
    ) -> Self {
        let mut component_recursive = vec![0u8; component_count];
        let mut component_size = vec![0u32; component_count];
        for &c in &component_of {
            component_size[c as usize] += 1;
        }
        let mut dag_pairs = Vec::new();
        for u in 0..node_count as u32 {
            let cu = component_of[u as usize];
            if component_size[cu as usize] > 1 {
                component_recursive[cu as usize] = 1;
            }
            for &v in row(&callee_offsets, &callee_adj, u) {
                let cv = component_of[v as usize];
                if cu == cv {
                    if u == v {
                        component_recursive[cu as usize] = 1;
                    }
                } else {
                    dag_pairs.push((cu, cv));
                }
            }
        }
        let (dag_offsets, dag_adj) = csr_from_pairs(component_count, dag_pairs);
        let reach = stored_reach
            .unwrap_or_else(|| build_reach_index(component_count, &dag_offsets, &dag_adj));
        Self {
            node_count,
            callee_offsets,
            callee_adj,
            callee_kinds,
            caller_offsets,
            caller_adj,
            sites,
            site_target_offsets,
            site_target_adj,
            component_of,
            component_count,
            component_recursive,
            dag_offsets,
            dag_adj,
            reach,
            points_to,
        }
    }

    pub fn node_count(&self) -> usize {
        self.node_count
    }

    pub fn edge_count(&self) -> usize {
        self.callee_adj.len()
    }

    pub fn contains(&self, method: u32) -> bool {
        (method as usize) < self.node_count
    }

    pub fn callees(&self, method: u32) -> &[u32] {
        row(&self.callee_offsets, &self.callee_adj, method)
    }

    /// Call type of each callee edge, parallel to `callees`.
    pub fn callee_kinds(&self, method: u32) -> &[u8] {
        row(&self.callee_offsets, &self.callee_kinds, method)
    }

    pub fn callers(&self, method: u32) -> &[u32] {
        row(&self.caller_offsets, &self.caller_adj, method)
    }

    pub fn edges(&self) -> impl Iterator<Item = (u32, u32, u8)> + '_ {
        (0..self.node_count as u32).flat_map(move |u| {
            self.callees(u)
                .iter()
                .zip(self.callee_kinds(u).iter())
                .map(move |(&v, &k)| (u, v, k))
        })
    }

    pub fn call_sites(&self) -> &[CallSiteRow] {
        &self.sites
    }

    pub fn call_site(&self, site_id: u32) -> Option<&CallSiteRow> {
        self.sites
            .binary_search_by_key(&site_id, |s| s.site_id)
            .ok()
            .map(|i| &self.sites[i])
    }

    /// Resolved targets of the call site stored at row `site_row` of `call_sites()`.
    pub fn site_targets(&self, site_row: usize) -> &[u32] {
        row(
            &self.site_target_offsets,
            &self.site_target_adj,
            site_row as u32,
        )
    }

    pub fn points_to(&self) -> &[(u32, u32)] {
        &self.points_to
    }

    pub fn component_of(&self, method: u32) -> Option<u32> {
        self.component_of.get(method as usize).copied()
    }

    pub fn is_recursive(&self, method: u32) -> bool {
        self.component_of(method)
            .map(|c| self.component_recursive[c as usize] != 0)
            .unwrap_or(false)
    }

    fn grail_contains(&self, u: u32, v: u32) -> bool {
        match &self.reach {
            ReachIndex::Grail { low, rank } => {
                let c = self.component_count;
                (0..GRAIL_LABELINGS).all(|l| {
                    let (lu, ru) = (low[l * c + u as usize], rank[l * c + u as usize]);
                    let (lv, rv) = (low[l * c + v as usize], rank[l * c + v as usize]);
                    lu <= lv && rv <= ru
                })
            }
            ReachIndex::Closure { .. } => true,
        }
    }

    fn component_reaches(&self, cu: u32, cv: u32) -> bool {
        if cu == cv {
            return true;
        }
        if cv > cu {
            return false;
        }
        match &self.reach {
            ReachIndex::Closure { words, bits } => {
                let w = bits[cu as usize * words + (cv as usize / 64)];
                (w >> (cv % 64)) & 1 == 1
            }
            ReachIndex::Grail { .. } => {
                if !self.grail_contains(cu, cv) {
                    return false;
                }
                let mut visited = vec![false; self.component_count];
                let mut stack = vec![cu];
                visited[cu as usize] = true;
                while let Some(x) = stack.pop() {
                    for &y in row(&self.dag_offsets, &self.dag_adj, x) {
                        if y == cv {
                            return true;
                        }
                        if y < cv || visited[y as usize] || !self.grail_contains(y, cv) {
                            continue;
                        }
                        visited[y as usize] = true;
                        stack.push(y);
                    }
                }
                false
            }
        }
    }

    /// Whether `target` is reachable from `source` through zero or more calls.
    /// `None` when either id is outside the call graph.
    pub fn is_reachable(&self, source: u32, target: u32) -> Option<bool> {
        let cu = self.component_of(source)?;
        let cv = self.component_of(target)?;
        Some(self.component_reaches(cu, cv))
    }

    /// A shortest call chain `source → … → target`, if one exists.
    pub fn call_path(&self, source: u32, target: u32) -> Option<Vec<u32>> {
        if !self.is_reachable(source, target)? {
            return None;
        }
        if source == target {
            return Some(vec![source]);
        }
        let cv = self.component_of[target as usize];
        let mut parent = vec![u32::MAX; self.node_count];
        let mut queue = VecDeque::new();
        parent[source as usize] = source;
        queue.push_back(source);
        while let Some(u) = queue.pop_front() {
            for &v in self.callees(u) {
                if parent[v as usize] != u32::MAX {
                    continue;
                }
                if !self.component_reaches(self.component_of[v as usize], cv) {
                    continue;
                }
                parent[v as usize] = u;
                if v == target {
                    let mut path = vec![target];
                    let mut cur = target;
                    while cur != source {
                        cur = parent[cur as usize];
                        path.push(cur);
                    }
                    path.reverse();
                    return Some(path);
                }
                queue.push_back(v);
            }
        }
        None
    }

    /// All methods reachable from `source` (excluding it unless it is recursive), with BFS
    /// depth, bounded by `depth_limit`.
    pub fn reachable_from(&self, source: u32, depth_limit: Option<u32>) -> Vec<(u32, u32)> {
        self.bfs(source, depth_limit, |m| self.callees(m))
    }

    /// All transitive callers of `target`, with BFS depth, bounded by `depth_limit`.
    pub fn transitive_callers(&self, target: u32, depth_limit: Option<u32>) -> Vec<(u32, u32)> {
        self.bfs(target, depth_limit, |m| self.callers(m))
    }

    fn bfs<'a, F>(&'a self, start: u32, depth_limit: Option<u32>, next: F) -> Vec<(u32, u32)>
    where
        F: Fn(u32) -> &'a [u32],
    {
        if !self.contains(start) {
            return Vec::new();
        }
        // `dist[v]` is the length of the shortest path of at least one edge from `start`;
        // `start` itself only appears when it lies on a cycle.
        let mut dist = vec![u32::MAX; self.node_count];
        let mut queue = VecDeque::new();
        queue.push_back((start, 0u32));
        while let Some((u, d)) = queue.pop_front() {
            if depth_limit.map(|l| d >= l).unwrap_or(false) {
                continue;
            }
            for &v in next(u) {
                if dist[v as usize] == u32::MAX {
                    dist[v as usize] = d + 1;
                    queue.push_back((v, d + 1));
                }
            }
        }
        dist.iter()
            .enumerate()
            .filter(|&(_, &d)| d != u32::MAX)
            .map(|(v, &d)| (v as u32, d))
            .collect()
    }

    pub fn encode(&self, enc: &mut SectionEncoder) {
        enc.put_u64(self.node_count as u64);
        enc.put_u32s(&self.callee_offsets);
        enc.put_u32s(&self.callee_adj);
        enc.put_u8s(&self.callee_kinds);
        enc.put_u32s(&self.caller_offsets);
        enc.put_u32s(&self.caller_adj);

        enc.put_u32s(&self.sites.iter().map(|s| s.site_id).collect::<Vec<_>>());
        enc.put_u32s(&self.sites.iter().map(|s| s.caller).collect::<Vec<_>>());
        enc.put_u32s(&self.sites.iter().map(|s| s.call_node).collect::<Vec<_>>());
        enc.put_u32s(&self.sites.iter().map(|s| s.call_block).collect::<Vec<_>>());
        enc.put_u32s(&self.sites.iter().map(|s| s.call_token).collect::<Vec<_>>());
        enc.put_u8s(&self.sites.iter().map(|s| s.call_type).collect::<Vec<_>>());
        enc.put_u8s(&self.sites.iter().map(|s| s.flags).collect::<Vec<_>>());
        enc.put_u16s(&self.sites.iter().map(|s| s.arg_count).collect::<Vec<_>>());
        enc.put_u32s(&self.site_target_offsets);
        enc.put_u32s(&self.site_target_adj);

        enc.put_u64(self.component_count as u64);
        enc.put_u32s(&self.component_of);

        enc.put_u32s(&self.points_to.iter().map(|p| p.0).collect::<Vec<_>>());
        enc.put_u32s(&self.points_to.iter().map(|p| p.1).collect::<Vec<_>>());

        match &self.reach {
            ReachIndex::Closure { words, bits } => {
                enc.put_u8(REACH_KIND_CLOSURE);
                enc.put_u64(*words as u64);
                enc.put_u64s(bits);
            }
            ReachIndex::Grail { low, rank } => {
                enc.put_u8(REACH_KIND_GRAIL);
                enc.put_u32s(low);
                enc.put_u32s(rank);
            }
        }
    }

    pub fn decode(dec: &mut SectionDecoder<'_>) -> Result<Self, ScpgFormatError> {
        let node_count = dec.u64()? as usize;
        let callee_offsets = dec.u32s()?;
        let callee_adj = dec.u32s()?;
        let callee_kinds = dec.u8s()?.to_vec();
        let caller_offsets = dec.u32s()?;
        let caller_adj = dec.u32s()?;
        dec.check_csr(
            "callees",
            &callee_offsets,
            &callee_adj,
            node_count,
            Some(node_count),
        )?;
        dec.check_csr(
            "callers",
            &caller_offsets,
            &caller_adj,
            node_count,
            Some(node_count),
        )?;
        dec.expect_len("callee_kinds", callee_kinds.len(), callee_adj.len())?;

        let site_ids = dec.u32s()?;
        let callers = dec.u32s()?;
        let call_nodes = dec.u32s()?;
        let call_blocks = dec.u32s()?;
        let call_tokens = dec.u32s()?;
        let call_types = dec.u8s()?;
        let flags = dec.u8s()?;
        let arg_counts = dec.u16s()?;
        let n_sites = site_ids.len();
        dec.expect_len("site_callers", callers.len(), n_sites)?;
        dec.expect_len("site_nodes", call_nodes.len(), n_sites)?;
        dec.expect_len("site_blocks", call_blocks.len(), n_sites)?;
        dec.expect_len("site_tokens", call_tokens.len(), n_sites)?;
        dec.expect_len("site_types", call_types.len(), n_sites)?;
        dec.expect_len("site_flags", flags.len(), n_sites)?;
        dec.expect_len("site_arg_counts", arg_counts.len(), n_sites)?;
        if site_ids.windows(2).any(|w| w[0] >= w[1]) {
            return Err(dec.malformed("call sites are not sorted by site id"));
        }
        let sites = (0..n_sites)
            .map(|i| CallSiteRow {
                site_id: site_ids[i],
                caller: callers[i],
                call_node: call_nodes[i],
                call_block: call_blocks[i],
                call_token: call_tokens[i],
                call_type: call_types[i],
                flags: flags[i],
                arg_count: arg_counts[i],
            })
            .collect();
        let site_target_offsets = dec.u32s()?;
        let site_target_adj = dec.u32s()?;
        dec.check_csr(
            "site_targets",
            &site_target_offsets,
            &site_target_adj,
            n_sites,
            Some(node_count),
        )?;

        let component_count = dec.u64()? as usize;
        let component_of = dec.u32s()?;
        dec.expect_len("component_of", component_of.len(), node_count)?;
        if component_of.iter().any(|&c| c as usize >= component_count) {
            return Err(dec.malformed("component id out of range"));
        }
        if component_count > node_count {
            return Err(dec.malformed("more components than methods"));
        }
        // Components must respect Tarjan completion order on every cross-component edge.
        for u in 0..node_count as u32 {
            let cu = component_of[u as usize];
            for &v in row(&callee_offsets, &callee_adj, u) {
                if component_of[v as usize] > cu {
                    return Err(dec.malformed("component order violates the call graph"));
                }
            }
        }

        let pt_ssa = dec.u32s()?;
        let pt_alloc = dec.u32s()?;
        dec.expect_len("points_to_alloc", pt_alloc.len(), pt_ssa.len())?;
        let points_to = pt_ssa.into_iter().zip(pt_alloc).collect();

        let reach_kind = dec.u8()?;
        let stored_reach = match reach_kind {
            REACH_KIND_CLOSURE => {
                let words = dec.u64()? as usize;
                let bits = dec.u64s()?;
                if words != component_count.div_ceil(64).max(1)
                    || bits.len() != words * component_count
                {
                    return Err(dec.malformed("closure bitset has the wrong shape"));
                }
                ReachIndex::Closure { words, bits }
            }
            REACH_KIND_GRAIL => {
                let low = dec.u32s()?;
                let rank = dec.u32s()?;
                dec.expect_len("grail_low", low.len(), GRAIL_LABELINGS * component_count)?;
                dec.expect_len("grail_rank", rank.len(), GRAIL_LABELINGS * component_count)?;
                ReachIndex::Grail { low, rank }
            }
            other => {
                return Err(dec.malformed(format!("unknown reachability index kind {}", other)))
            }
        };

        // The stored index was produced from the same DAG, so it is reused instead of rebuilt.
        Ok(Self::assemble(
            node_count,
            callee_offsets,
            callee_adj,
            callee_kinds,
            caller_offsets,
            caller_adj,
            sites,
            site_target_offsets,
            site_target_adj,
            component_of,
            component_count,
            points_to,
            Some(stored_reach),
        ))
    }
}

/// Iterative Tarjan SCC. Component ids follow completion order, i.e. reverse topological
/// order of the condensation: an edge between components always goes to a smaller id.
fn tarjan(n: usize, offsets: &[u32], adj: &[u32]) -> (Vec<u32>, usize) {
    const UNVISITED: u32 = u32::MAX;
    let mut index = vec![UNVISITED; n];
    let mut low = vec![0u32; n];
    let mut on_stack = vec![false; n];
    let mut stack: Vec<u32> = Vec::new();
    let mut component_of = vec![0u32; n];
    let mut next_index = 0u32;
    let mut component_count = 0u32;
    let mut frames: Vec<(u32, u32)> = Vec::new();

    for root in 0..n as u32 {
        if index[root as usize] != UNVISITED {
            continue;
        }
        index[root as usize] = next_index;
        low[root as usize] = next_index;
        next_index += 1;
        stack.push(root);
        on_stack[root as usize] = true;
        frames.push((root, offsets[root as usize]));

        while let Some(top) = frames.len().checked_sub(1) {
            let (v, pos) = frames[top];
            let end = offsets[v as usize + 1];
            if pos < end {
                frames[top].1 = pos + 1;
                let w = adj[pos as usize];
                if index[w as usize] == UNVISITED {
                    index[w as usize] = next_index;
                    low[w as usize] = next_index;
                    next_index += 1;
                    stack.push(w);
                    on_stack[w as usize] = true;
                    frames.push((w, offsets[w as usize]));
                } else if on_stack[w as usize] {
                    low[v as usize] = low[v as usize].min(index[w as usize]);
                }
            } else {
                frames.pop();
                if let Some(&(parent, _)) = frames.last() {
                    low[parent as usize] = low[parent as usize].min(low[v as usize]);
                }
                if low[v as usize] == index[v as usize] {
                    while let Some(w) = stack.pop() {
                        on_stack[w as usize] = false;
                        component_of[w as usize] = component_count;
                        if w == v {
                            break;
                        }
                    }
                    component_count += 1;
                }
            }
        }
    }
    (component_of, component_count as usize)
}

fn build_reach_index(components: usize, dag_offsets: &[u32], dag_adj: &[u32]) -> ReachIndex {
    if components <= CLOSURE_COMPONENT_LIMIT {
        let words = components.div_ceil(64).max(1);
        let mut bits = vec![0u64; words * components];
        // Successors always have smaller ids, so rows are final when visited in id order.
        for c in 0..components {
            let (done, rest) = bits.split_at_mut(c * words);
            let row_c = &mut rest[..words];
            row_c[c / 64] |= 1u64 << (c % 64);
            for &s in row(dag_offsets, dag_adj, c as u32) {
                let row_s = &done[s as usize * words..(s as usize + 1) * words];
                for (dst, src) in row_c.iter_mut().zip(row_s) {
                    *dst |= *src;
                }
            }
        }
        return ReachIndex::Closure { words, bits };
    }

    let mut low = vec![0u32; GRAIL_LABELINGS * components];
    let mut rank = vec![0u32; GRAIL_LABELINGS * components];
    for labeling in 0..GRAIL_LABELINGS {
        let base = labeling * components;
        let mut visited = vec![false; components];
        let mut next_rank = 1u32;
        let mut frames: Vec<(u32, usize)> = Vec::new();
        let roots: Vec<u32> = if labeling % 2 == 0 {
            (0..components as u32).rev().collect()
        } else {
            (0..components as u32).collect()
        };
        for root in roots {
            if visited[root as usize] {
                continue;
            }
            visited[root as usize] = true;
            frames.push((root, 0));
            while let Some(top) = frames.len().checked_sub(1) {
                let (v, pos) = frames[top];
                let children = row(dag_offsets, dag_adj, v);
                if pos < children.len() {
                    frames[top].1 = pos + 1;
                    let child = children[permuted_index(labeling, v, pos, children.len())];
                    if !visited[child as usize] {
                        visited[child as usize] = true;
                        frames.push((child, 0));
                    }
                } else {
                    frames.pop();
                    let r = next_rank;
                    next_rank += 1;
                    let mut l = r;
                    for &c in children {
                        l = l.min(low[base + c as usize]);
                    }
                    rank[base + v as usize] = r;
                    low[base + v as usize] = l;
                }
            }
        }
    }
    ReachIndex::Grail { low, rank }
}

/// Deterministic child permutation per labeling: forward, reverse, and a rotated order.
fn permuted_index(labeling: usize, node: u32, pos: usize, len: usize) -> usize {
    match labeling % 3 {
        0 => pos,
        1 => len - 1 - pos,
        _ => {
            let shift = (node as usize).wrapping_mul(2_654_435_761) % len;
            (pos + shift) % len
        }
    }
}
