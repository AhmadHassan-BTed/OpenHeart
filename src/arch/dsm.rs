//! Dependency structure matrix (DSM): partitioning into strongly connected components
//! (Tarjan 1972), levelized ordering of the condensation and feedback-minimizing ordering
//! inside each component.
//!
//! **Convention.** Row `r`, column `c` holds the weight of the dependencies of the row node on
//! the column node (`r` uses `c`). Providers come first, so in an acyclic graph every mark
//! lies below the diagonal; marks above the diagonal (feedback) exist only inside cyclic
//! blocks.
//!
//! **Order.** Components are sorted by their longest-path level over the condensation (level
//! 0 depends on nothing outside itself), ties broken by their smallest member id, which is a
//! topological order of the condensation (dependencies before dependents). Inside a cyclic
//! component, members are ordered by the weighted Eades–Lin–Smyth greedy heuristic for the
//! minimum feedback arc set (Eades, Lin & Smyth 1993), then improved by single-vertex moves
//! until no move lowers the feedback weight. Minimum feedback arc set is NP-hard, so the
//! remaining feedback is a property of the ordering that is reported, not a lower bound.
//!
//! **Propagation cost** (MacCormack, Rusnak & Baldwin 2006) is the density of the visibility
//! matrix: the fraction of ordered pairs `(i, j)`, `i = j` included, such that `i` depends on
//! `j` directly or transitively.

use crate::arch::graph::{
    span_key, Certainty, DependencyGraph, KindSet, Level, NodeId, WeightedDigraph,
};
use crate::ir::Span;

// ── Strongly connected components ────────────────────────────────────────────

/// Strongly connected components of a [`WeightedDigraph`].
#[derive(Debug, Clone, Default)]
pub struct Components {
    /// Component of each node.
    pub component: Vec<u32>,
    /// Sorted members of each component. Components are numbered in Tarjan's emission order,
    /// a reverse topological order: every component a component depends on has a smaller
    /// number.
    pub members: Vec<Vec<u32>>,
}

impl Components {
    pub fn count(&self) -> usize {
        self.members.len()
    }

    /// A component is cyclic when it has more than one member (the digraph has no self loops).
    pub fn is_cyclic(&self, c: u32) -> bool {
        self.members[c as usize].len() > 1
    }
}

/// Tarjan's algorithm, iterative (no recursion, so arbitrarily long dependency chains are
/// safe). Roots are visited in node order and successors in sorted order, so the result is
/// deterministic.
pub fn strongly_connected_components(g: &WeightedDigraph) -> Components {
    let n = g.node_count();
    let mut index = vec![u32::MAX; n];
    let mut low = vec![0u32; n];
    let mut on_stack = vec![false; n];
    let mut stack: Vec<u32> = Vec::new();
    let mut frames: Vec<(u32, usize)> = Vec::new();
    let mut next_index = 0u32;
    let mut component = vec![u32::MAX; n];
    let mut members: Vec<Vec<u32>> = Vec::new();

    for root in 0..n as u32 {
        if index[root as usize] != u32::MAX {
            continue;
        }
        index[root as usize] = next_index;
        low[root as usize] = next_index;
        next_index += 1;
        stack.push(root);
        on_stack[root as usize] = true;
        frames.push((root, 0));

        while let Some(top) = frames.len().checked_sub(1) {
            let (v, pos) = frames[top];
            let targets = g.targets(v);
            if pos < targets.len() {
                frames[top].1 = pos + 1;
                let w = targets[pos];
                if index[w as usize] == u32::MAX {
                    index[w as usize] = next_index;
                    low[w as usize] = next_index;
                    next_index += 1;
                    stack.push(w);
                    on_stack[w as usize] = true;
                    frames.push((w, 0));
                } else if on_stack[w as usize] {
                    low[v as usize] = low[v as usize].min(index[w as usize]);
                }
                continue;
            }

            frames.pop();
            if let Some(&(parent, _)) = frames.last() {
                low[parent as usize] = low[parent as usize].min(low[v as usize]);
            }
            if low[v as usize] == index[v as usize] {
                let c = members.len() as u32;
                let mut comp = Vec::new();
                while let Some(w) = stack.pop() {
                    on_stack[w as usize] = false;
                    component[w as usize] = c;
                    comp.push(w);
                    if w == v {
                        break;
                    }
                }
                comp.sort_unstable();
                members.push(comp);
            }
        }
    }

    Components { component, members }
}

/// The condensation: one node per component, edge weights summed over member edges.
pub fn condensation(g: &WeightedDigraph, comps: &Components) -> WeightedDigraph {
    let mut edges = Vec::with_capacity(g.edge_count());
    for u in 0..g.node_count() as u32 {
        let cu = comps.component[u as usize];
        for (v, w) in g.successors(u) {
            let cv = comps.component[v as usize];
            if cu != cv {
                edges.push((cu, cv, w));
            }
        }
    }
    WeightedDigraph::from_edges(comps.count(), edges)
}

/// Longest-path level of each component over the condensation: 0 for a component with no
/// dependency outside itself, otherwise one more than the highest level it depends on.
pub fn component_levels(g: &WeightedDigraph, comps: &Components) -> Vec<u32> {
    let mut level = vec![0u32; comps.count()];
    // Tarjan numbers a component after every component it depends on.
    for c in 0..comps.count() {
        let mut lvl = 0u32;
        for &v in &comps.members[c] {
            for &w in g.targets(v) {
                let d = comps.component[w as usize] as usize;
                if d != c {
                    lvl = lvl.max(level[d] + 1);
                }
            }
        }
        level[c] = lvl;
    }
    level
}

// ── Ordering inside a component ──────────────────────────────────────────────

/// Weight of dependencies `u → v` with `v` placed after `u` (above the diagonal).
pub fn feedback_weight(g: &WeightedDigraph, order: &[u32], position: &[u32]) -> u64 {
    let mut total = 0u64;
    for &u in order {
        for (v, w) in g.successors(u) {
            let (pu, pv) = (position[u as usize], position[v as usize]);
            if pu != u32::MAX && pv != u32::MAX && pv > pu {
                total += w;
            }
        }
    }
    total
}

/// Orders the members of one strongly connected component so that few dependencies point
/// to later members: weighted Eades–Lin–Smyth, then single-vertex moves for up to `rounds`
/// rounds when the component has at most `max_improvement_size` members.
pub fn order_component(
    g: &WeightedDigraph,
    members: &[u32],
    rounds: u32,
    max_improvement_size: usize,
) -> Vec<u32> {
    let mut members = members.to_vec();
    members.sort_unstable();
    members.dedup();
    if members.len() <= 1 {
        return members;
    }
    let k = members.len();
    let local = |v: u32| members.binary_search(&v).ok();

    // In the precedence graph P an edge j → i means "j should precede i", i.e. i depends on
    // j. Eades–Lin–Smyth places most P-edges left to right.
    let mut dependents: Vec<Vec<(usize, u64)>> = vec![Vec::new(); k];
    let mut dependencies: Vec<Vec<(usize, u64)>> = vec![Vec::new(); k];
    for (i, &v) in members.iter().enumerate() {
        for (w, weight) in g.successors(v) {
            if let Some(j) = local(w) {
                dependencies[i].push((j, weight));
                dependents[j].push((i, weight));
            }
        }
    }
    let mut out_w: Vec<i128> = dependents
        .iter()
        .map(|l| l.iter().map(|&(_, w)| i128::from(w)).sum())
        .collect();
    let mut in_w: Vec<i128> = dependencies
        .iter()
        .map(|l| l.iter().map(|&(_, w)| i128::from(w)).sum())
        .collect();

    let mut removed = vec![false; k];
    let mut front: Vec<usize> = Vec::with_capacity(k);
    let mut back: Vec<usize> = Vec::with_capacity(k);
    let mut remaining = k;

    let remove = |x: usize, removed: &mut [bool], out_w: &mut [i128], in_w: &mut [i128]| {
        removed[x] = true;
        // x's dependents lose an incoming precedence edge, its dependencies an outgoing one.
        for &(i, w) in &dependents[x] {
            in_w[i] -= i128::from(w);
        }
        for &(j, w) in &dependencies[x] {
            out_w[j] -= i128::from(w);
        }
    };

    while remaining > 0 {
        let mut progressed = true;
        while progressed {
            progressed = false;
            // Sinks of P (nothing remaining depends on them) go to the back.
            for x in 0..k {
                if !removed[x] && out_w[x] == 0 {
                    remove(x, &mut removed, &mut out_w, &mut in_w);
                    back.push(x);
                    remaining -= 1;
                    progressed = true;
                }
            }
            // Sources of P (they depend on nothing remaining) go to the front.
            for x in 0..k {
                if !removed[x] && in_w[x] == 0 {
                    remove(x, &mut removed, &mut out_w, &mut in_w);
                    front.push(x);
                    remaining -= 1;
                    progressed = true;
                }
            }
        }
        if remaining == 0 {
            break;
        }
        let mut best: Option<(usize, i128)> = None;
        for x in 0..k {
            if removed[x] {
                continue;
            }
            let delta = out_w[x] - in_w[x];
            if best.is_none_or(|(_, d)| delta > d) {
                best = Some((x, delta));
            }
        }
        if let Some((x, _)) = best {
            remove(x, &mut removed, &mut out_w, &mut in_w);
            front.push(x);
            remaining -= 1;
        }
    }
    back.reverse();
    front.extend(back);
    let mut order: Vec<u32> = front.into_iter().map(|i| members[i]).collect();

    if k <= max_improvement_size {
        improve_by_moves(g, &mut order, rounds);
    }
    order
}

/// Moves single vertices to the position that lowers the feedback weight most, until a
/// round makes no move or `rounds` rounds have run.
fn improve_by_moves(g: &WeightedDigraph, order: &mut Vec<u32>, rounds: u32) {
    let k = order.len();
    for _ in 0..rounds {
        let mut improved = false;
        for p in 0..k {
            let v = order[p];
            // Moving v left past u: dependency v → u becomes feedback, u → v stops being one.
            let mut best_delta = 0i128;
            let mut best_pos = p;
            let mut delta = 0i128;
            for q in (0..p).rev() {
                let u = order[q];
                delta += i128::from(g.weight(v, u)) - i128::from(g.weight(u, v));
                if delta < best_delta {
                    best_delta = delta;
                    best_pos = q;
                }
            }
            // Moving v right past u: u → v becomes feedback, v → u stops being one.
            delta = 0;
            for (q, &u) in order.iter().enumerate().skip(p + 1) {
                delta += i128::from(g.weight(u, v)) - i128::from(g.weight(v, u));
                if delta < best_delta {
                    best_delta = delta;
                    best_pos = q;
                }
            }
            if best_pos != p {
                let x = order.remove(p);
                order.insert(best_pos, x);
                improved = true;
            }
        }
        if !improved {
            break;
        }
    }
}

// ── Matrix ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct DsmConfig {
    pub kinds: KindSet,
    pub include_external: bool,
    /// Rounds of single-vertex moves inside each cyclic component.
    pub improvement_rounds: u32,
    /// Components larger than this keep the Eades–Lin–Smyth order without moves.
    pub max_improvement_size: usize,
    /// The visibility matrix is computed only up to this many components (its bit matrix
    /// needs `components² / 8` bytes).
    pub max_closure_components: usize,
}

impl Default for DsmConfig {
    fn default() -> Self {
        Self {
            kinds: KindSet::STATIC,
            include_external: false,
            improvement_rounds: 16,
            max_improvement_size: 1024,
            max_closure_components: 16_384,
        }
    }
}

/// One diagonal block: a strongly connected component.
#[derive(Debug, Clone)]
pub struct DsmBlock {
    /// First matrix position of the block.
    pub start: u32,
    pub len: u32,
    pub component: u32,
    /// Longest-path level of the component.
    pub level: u32,
    pub cyclic: bool,
    /// Weight above the diagonal inside the block.
    pub feedback_weight: u64,
}

/// One non-empty matrix cell.
#[derive(Debug, Clone)]
pub struct DsmCell {
    pub row: u32,
    pub col: u32,
    /// Distinct dependency sites of the row node on the column node over the selected kinds.
    pub weight: u64,
    pub kinds: KindSet,
    pub certainty: Certainty,
    pub spans: Vec<Span>,
}

#[derive(Debug, Clone)]
pub struct Dsm {
    pub level: Level,
    pub kinds: KindSet,
    /// `order[i]` is the node of row and column `i`.
    pub order: Vec<NodeId>,
    /// Matrix position of each graph node, `u32::MAX` for excluded nodes.
    pub position: Vec<u32>,
    pub blocks: Vec<DsmBlock>,
    /// Block of each matrix position.
    pub block_of: Vec<u32>,
    /// Non-empty cells, row-major.
    pub cells: Vec<DsmCell>,
    row_offsets: Vec<u32>,
    pub components: Components,
    /// Total weight above the diagonal.
    pub feedback_weight: u64,
    pub feedback_cells: u32,
    pub propagation_cost: Option<f64>,
    /// Why the propagation cost is absent.
    pub propagation_note: Option<String>,
}

impl Dsm {
    pub fn size(&self) -> usize {
        self.order.len()
    }

    /// Cells of matrix row `row`, sorted by column.
    pub fn row(&self, row: u32) -> &[DsmCell] {
        let r = row as usize;
        if r + 1 >= self.row_offsets.len() {
            return &[];
        }
        &self.cells[self.row_offsets[r] as usize..self.row_offsets[r + 1] as usize]
    }

    pub fn cell(&self, row: u32, col: u32) -> Option<&DsmCell> {
        let cells = self.row(row);
        cells
            .binary_search_by_key(&col, |c| c.col)
            .ok()
            .map(|i| &cells[i])
    }

    /// True for a mark above the diagonal.
    pub fn is_feedback(cell: &DsmCell) -> bool {
        cell.col > cell.row
    }
}

/// Builds the partitioned, ordered matrix of `graph`.
pub fn build(graph: &DependencyGraph, config: &DsmConfig) -> Dsm {
    let n = graph.node_count();
    let adjacency = graph.adjacency(config.kinds, config.include_external);
    let components = strongly_connected_components(&adjacency);
    let levels = component_levels(&adjacency, &components);
    let included = |v: u32| graph.included(NodeId(v), config.include_external);

    // Components holding included nodes, in levelized topological order.
    let mut comp_order: Vec<u32> = (0..components.count() as u32)
        .filter(|&c| components.members[c as usize].iter().any(|&v| included(v)))
        .collect();
    comp_order.sort_by_key(|&c| (levels[c as usize], components.members[c as usize][0]));

    let mut order: Vec<NodeId> = Vec::new();
    let mut blocks: Vec<DsmBlock> = Vec::new();
    let mut block_of: Vec<u32> = Vec::new();
    for &c in &comp_order {
        let members: Vec<u32> = components.members[c as usize]
            .iter()
            .copied()
            .filter(|&v| included(v))
            .collect();
        let ordered = order_component(
            &adjacency,
            &members,
            config.improvement_rounds,
            config.max_improvement_size,
        );
        let block = blocks.len() as u32;
        blocks.push(DsmBlock {
            start: order.len() as u32,
            len: ordered.len() as u32,
            component: c,
            level: levels[c as usize],
            cyclic: ordered.len() > 1,
            feedback_weight: 0,
        });
        for v in ordered {
            order.push(NodeId(v));
            block_of.push(block);
        }
    }

    let mut position = vec![u32::MAX; n];
    for (i, node) in order.iter().enumerate() {
        position[node.index()] = i as u32;
    }

    // Cells from the typed edges, aggregated per (row, column).
    let mut cells: Vec<DsmCell> = Vec::new();
    let mut row_offsets = vec![0u32; order.len() + 1];
    let mut feedback = 0u64;
    let mut feedback_cells = 0u32;
    for (row, node) in order.iter().enumerate() {
        let mut row_cells: Vec<DsmCell> = Vec::new();
        for edge in graph.outgoing(*node) {
            if !config.kinds.contains(edge.kind) {
                continue;
            }
            let col = position[edge.to.index()];
            if col == u32::MAX {
                continue;
            }
            match row_cells.last_mut() {
                Some(cell) if cell.col == col => {
                    cell.weight += u64::from(edge.weight());
                    cell.kinds = cell.kinds.with(edge.kind);
                    cell.certainty = cell.certainty.clone().strongest(edge.certainty.clone());
                    cell.spans.extend(edge.spans());
                }
                _ => row_cells.push(DsmCell {
                    row: row as u32,
                    col,
                    weight: u64::from(edge.weight()),
                    kinds: KindSet::NONE.with(edge.kind),
                    certainty: edge.certainty.clone(),
                    spans: edge.spans(),
                }),
            }
        }
        // Outgoing edges are sorted by target id, not by matrix column.
        row_cells.sort_by_key(|c| c.col);
        let mut merged: Vec<DsmCell> = Vec::with_capacity(row_cells.len());
        for cell in row_cells {
            match merged.last_mut() {
                Some(last) if last.col == cell.col => {
                    last.weight += cell.weight;
                    last.kinds = last.kinds.union(cell.kinds);
                    last.certainty = last.certainty.clone().strongest(cell.certainty);
                    last.spans.extend(cell.spans);
                }
                _ => merged.push(cell),
            }
        }
        for cell in &mut merged {
            cell.spans.sort_by_key(span_key);
            cell.spans.dedup_by_key(|s| span_key(s));
            if cell.col > cell.row {
                feedback += cell.weight;
                feedback_cells += 1;
                let b = block_of[row] as usize;
                blocks[b].feedback_weight += cell.weight;
            }
        }
        cells.extend(merged);
        row_offsets[row + 1] = cells.len() as u32;
    }

    let (propagation_cost, propagation_note) =
        propagation_cost(&adjacency, &components, &comp_order, &order, config);

    Dsm {
        level: graph.level,
        kinds: config.kinds,
        order,
        position,
        blocks,
        block_of,
        cells,
        row_offsets,
        components,
        feedback_weight: feedback,
        feedback_cells,
        propagation_cost,
        propagation_note,
    }
}

/// Visibility-matrix density over the included nodes, computed on the condensation with one
/// reachability bit set per component.
fn propagation_cost(
    g: &WeightedDigraph,
    comps: &Components,
    comp_order: &[u32],
    order: &[NodeId],
    config: &DsmConfig,
) -> (Option<f64>, Option<String>) {
    let n = order.len();
    if n == 0 {
        return (
            None,
            Some("the matrix is empty, so the propagation cost is undefined".to_string()),
        );
    }
    let c = comps.count();
    if c > config.max_closure_components {
        return (
            None,
            Some(format!(
                "{} components exceed the visibility-matrix limit of {}",
                c, config.max_closure_components
            )),
        );
    }
    let mut size = vec![0u64; c];
    for node in order {
        size[comps.component[node.index()] as usize] += 1;
    }
    let words = c.div_ceil(64);
    let mut reach: Vec<Vec<u64>> = vec![Vec::new(); c];
    let mut visible: u128 = 0;
    // Tarjan numbering puts every dependency of a component before it.
    for comp in 0..c {
        let mut bits = vec![0u64; words];
        bits[comp / 64] |= 1u64 << (comp % 64);
        for &v in &comps.members[comp] {
            for &w in g.targets(v) {
                let d = comps.component[w as usize] as usize;
                if d != comp {
                    for (b, &r) in bits.iter_mut().zip(reach[d].iter()) {
                        *b |= r;
                    }
                }
            }
        }
        reach[comp] = bits;
    }
    for &comp in comp_order {
        let comp = comp as usize;
        let mut seen: u128 = 0;
        for (word_index, &word) in reach[comp].iter().enumerate() {
            let mut rest = word;
            while rest != 0 {
                let bit = rest.trailing_zeros() as usize;
                rest &= rest - 1;
                seen += u128::from(size[word_index * 64 + bit]);
            }
        }
        visible += u128::from(size[comp]) * seen;
    }
    let total = (n as u128) * (n as u128);
    (Some(visible as f64 / total as f64), None)
}
