//! Subsystem recovery by modularity clustering: Louvain (Blondel, Guillaume, Lambiotte &
//! Lefebvre 2008) over the weighted, symmetrized dependency graph, with a connectivity
//! refinement.
//!
//! **Graph.** Every edge of the selected kinds between included nodes contributes its weight
//! (distinct dependency sites) times the configured weight of its kind to the undirected
//! weight `A_ij = A_ji`; both directions add up. Modularity with resolution `γ` is
//! `Q = Σ_c [ Σ_in(c) / 2m − γ (Σ_tot(c) / 2m)² ]`, where `Σ_in(c)` sums `A_ij` over ordered
//! member pairs and `Σ_tot(c)` the weighted degrees of the members.
//!
//! **Louvain.** Local moving: each node in turn moves to the neighbouring community with the
//! largest gain `k_i,in(C) − γ Σ_tot(C) k_i / 2m` when it beats staying by more than
//! `min_gain`; passes repeat until no node moves. Aggregation: communities become nodes
//! (their internal weight a self loop) and both steps repeat until a level moves nothing.
//!
//! **Refinement.** Louvain can leave a community internally disconnected (Traag, Waltman &
//! van Eck 2019). Each community is split into its connected components. Splitting parts
//! that share no edge never lowers `Q` (it removes the null-model cross term
//! `2γ Σ_tot(A) Σ_tot(B) / (2m)²`), so the final partition is at least as modular and every
//! subsystem is connected.
//!
//! **Determinism.** Nodes are visited in id order (ids follow node keys), candidate
//! communities in increasing id, ties keep the current community, and the final communities
//! are numbered by their smallest member. Identical inputs give identical subsystems
//! whatever the order the facts were collected in.
//!
//! **Labels** state how they were derived: the longest common qualified-name prefix of the
//! members that ends at a segment boundary; else the container (package or project) that
//! holds most of the members' type-level units; else the member with the largest weighted
//! degree.

use std::collections::{BTreeMap, HashMap};

use crate::arch::graph::{ArchModel, DepKind, KindSet, Level, NodeId};

#[derive(Debug, Clone)]
pub struct ClusterConfig {
    pub kinds: KindSet,
    /// Multiplier of each kind's weight (indexed by `DepKind::index`).
    pub kind_weights: [f64; DepKind::COUNT],
    pub include_external: bool,
    /// Resolution `γ` of the modularity (1.0 is Newman–Girvan modularity).
    pub resolution: f64,
    /// A move must improve the gain by more than this.
    pub min_gain: f64,
    /// Local-moving passes per level.
    pub max_passes: u32,
    /// Aggregation levels.
    pub max_levels: u32,
}

impl Default for ClusterConfig {
    fn default() -> Self {
        Self {
            kinds: KindSet::ALL,
            kind_weights: [1.0; DepKind::COUNT],
            include_external: false,
            resolution: 1.0,
            min_gain: 1e-12,
            max_passes: 128,
            max_levels: 64,
        }
    }
}

/// How a community label was derived.
#[derive(Debug, Clone, PartialEq)]
pub enum LabelBasis {
    /// The community has one member; the label is its name.
    Singleton,
    /// The longest common qualified-name prefix of the members.
    CommonPrefix,
    /// The container at `level` that holds `share` of the members' type-level units.
    DominantContainer {
        container: NodeId,
        level: Level,
        share: f64,
    },
    /// The member with the largest weighted degree.
    Hub { node: NodeId },
}

#[derive(Debug, Clone)]
pub struct Community {
    pub id: u32,
    pub members: Vec<NodeId>,
    /// Undirected weight of the dependencies between members.
    pub internal_weight: f64,
    /// Weighted degree of the members (`Σ_tot`).
    pub total_degree: f64,
    pub label: String,
    pub label_basis: LabelBasis,
}

#[derive(Debug, Clone)]
pub struct ClusterResult {
    pub level: Level,
    /// Community of each node of the graph, `None` for excluded nodes.
    pub assignment: Vec<Option<u32>>,
    pub communities: Vec<Community>,
    pub modularity: f64,
    /// Total undirected weight `m`.
    pub total_weight: f64,
    /// Assignment after each Louvain aggregation level, before the refinement.
    pub hierarchy: Vec<Vec<Option<u32>>>,
    pub excluded: Vec<NodeId>,
    /// Number of extra communities the refinement created by splitting disconnected ones.
    pub refinement_splits: u32,
}

impl ClusterResult {
    pub fn community_of(&self, node: NodeId) -> Option<u32> {
        self.assignment.get(node.index()).copied().flatten()
    }
}

/// Undirected weighted graph with self loops: `self_loop[i] = A_ii`, `degree[i] = k_i`.
#[derive(Debug, Clone)]
struct UGraph {
    adj: Vec<Vec<(u32, f64)>>,
    self_loop: Vec<f64>,
    degree: Vec<f64>,
    two_m: f64,
}

impl UGraph {
    fn from_pairs(n: usize, pairs: BTreeMap<(u32, u32), f64>, self_loop: Vec<f64>) -> Self {
        let mut adj: Vec<Vec<(u32, f64)>> = vec![Vec::new(); n];
        for ((a, b), w) in pairs {
            adj[a as usize].push((b, w));
            adj[b as usize].push((a, w));
        }
        for list in &mut adj {
            list.sort_by_key(|&(j, _)| j);
        }
        let degree: Vec<f64> = (0..n)
            .map(|i| self_loop[i] + adj[i].iter().map(|&(_, w)| w).sum::<f64>())
            .collect();
        let two_m = degree.iter().sum();
        Self {
            adj,
            self_loop,
            degree,
            two_m,
        }
    }

    fn len(&self) -> usize {
        self.adj.len()
    }
}

/// Clusters the nodes of `model`'s graph at `level`.
pub fn louvain(model: &ArchModel, level: Level, config: &ClusterConfig) -> ClusterResult {
    let graph = model.graph(level);
    let n_all = graph.node_count();

    let mut local = vec![None; n_all];
    let mut nodes: Vec<NodeId> = Vec::new();
    let mut excluded = Vec::new();
    for node in &graph.nodes {
        if graph.included(node.id, config.include_external) {
            local[node.id.index()] = Some(nodes.len() as u32);
            nodes.push(node.id);
        } else {
            excluded.push(node.id);
        }
    }
    let n = nodes.len();

    let mut pairs: BTreeMap<(u32, u32), f64> = BTreeMap::new();
    for e in &graph.edges {
        if !config.kinds.contains(e.kind) {
            continue;
        }
        let (Some(a), Some(b)) = (local[e.from.index()], local[e.to.index()]) else {
            continue;
        };
        if a == b {
            continue;
        }
        let w = f64::from(e.weight()) * config.kind_weights[e.kind.index()];
        if w.is_nan() || w <= 0.0 {
            continue;
        }
        *pairs.entry((a.min(b), a.max(b))).or_insert(0.0) += w;
    }
    let base = UGraph::from_pairs(n, pairs, vec![0.0; n]);

    // ── Louvain levels ──
    let mut membership: Vec<u32> = (0..n as u32).collect();
    let mut hierarchy: Vec<Vec<Option<u32>>> = Vec::new();
    let mut current = base.clone();
    for _ in 0..config.max_levels {
        let mut comm: Vec<u32> = (0..current.len() as u32).collect();
        if !local_moving(&current, &mut comm, config) {
            break;
        }
        let (next, node_dense) = aggregate(&current, &comm);
        for m in membership.iter_mut() {
            *m = node_dense[*m as usize];
        }
        hierarchy.push(expand(&membership, &local));
        let reduced = next.len() < current.len();
        current = next;
        if !reduced {
            break;
        }
    }

    // ── Connectivity refinement, numbering by smallest member ──
    let (assignment_local, community_count, refinement_splits) =
        split_disconnected(&base, &membership);
    let (modularity, inside, total) =
        modularity(&base, &assignment_local, community_count, config.resolution);

    let mut members: Vec<Vec<NodeId>> = vec![Vec::new(); community_count];
    for (i, &c) in assignment_local.iter().enumerate() {
        members[c as usize].push(nodes[i]);
    }
    let labels = label_communities(model, level, &members, &base, &local);
    let communities: Vec<Community> = members
        .into_iter()
        .zip(labels)
        .enumerate()
        .map(|(c, (members, (label, label_basis)))| Community {
            id: c as u32,
            members,
            internal_weight: inside[c] / 2.0,
            total_degree: total[c],
            label,
            label_basis,
        })
        .collect();

    ClusterResult {
        level,
        assignment: expand(&assignment_local, &local),
        communities,
        modularity,
        total_weight: base.two_m / 2.0,
        hierarchy,
        excluded,
        refinement_splits,
    }
}

/// Maps a per-included-node assignment back to every node of the graph.
fn expand(assignment: &[u32], local: &[Option<u32>]) -> Vec<Option<u32>> {
    local
        .iter()
        .map(|l| l.map(|i| assignment[i as usize]))
        .collect()
}

/// One Louvain local-moving phase. Returns whether any node moved.
fn local_moving(g: &UGraph, comm: &mut [u32], config: &ClusterConfig) -> bool {
    let n = g.len();
    if n == 0 || g.two_m <= 0.0 {
        return false;
    }
    let mut tot = vec![0.0f64; n];
    for i in 0..n {
        tot[comm[i] as usize] += g.degree[i];
    }
    let mut weight_to = vec![0.0f64; n];
    let mut touched_flag = vec![false; n];
    let mut touched: Vec<u32> = Vec::new();
    let mut moved_any = false;

    for _ in 0..config.max_passes {
        let mut moved = false;
        for i in 0..n {
            let ci = comm[i];
            let ki = g.degree[i];
            for &(j, w) in &g.adj[i] {
                let c = comm[j as usize];
                if !touched_flag[c as usize] {
                    touched_flag[c as usize] = true;
                    touched.push(c);
                }
                weight_to[c as usize] += w;
            }
            tot[ci as usize] -= ki;

            let scale = config.resolution * ki / g.two_m;
            let mut best = ci;
            let mut best_gain = weight_to[ci as usize] - tot[ci as usize] * scale;
            touched.sort_unstable();
            for &c in &touched {
                if c == ci {
                    continue;
                }
                let gain = weight_to[c as usize] - tot[c as usize] * scale;
                if gain > best_gain + config.min_gain {
                    best = c;
                    best_gain = gain;
                }
            }

            tot[best as usize] += ki;
            if best != ci {
                comm[i] = best;
                moved = true;
            }
            for &c in &touched {
                weight_to[c as usize] = 0.0;
                touched_flag[c as usize] = false;
            }
            touched.clear();
        }
        if !moved {
            break;
        }
        moved_any = true;
    }
    moved_any
}

/// Contracts communities into nodes. Returns the aggregated graph and the aggregated node of
/// every node (communities numbered by first appearance in node order).
fn aggregate(g: &UGraph, comm: &[u32]) -> (UGraph, Vec<u32>) {
    let n = g.len();
    let mut dense = vec![u32::MAX; n];
    let mut node_dense = vec![0u32; n];
    let mut next = 0u32;
    for i in 0..n {
        let c = comm[i] as usize;
        if dense[c] == u32::MAX {
            dense[c] = next;
            next += 1;
        }
        node_dense[i] = dense[c];
    }
    let k = next as usize;
    let mut self_loop = vec![0.0f64; k];
    let mut pairs: BTreeMap<(u32, u32), f64> = BTreeMap::new();
    for i in 0..n {
        let ci = node_dense[i];
        self_loop[ci as usize] += g.self_loop[i];
        for &(j, w) in &g.adj[i] {
            let cj = node_dense[j as usize];
            if ci == cj {
                // Seen once from each endpoint: A_cc counts the internal edge twice.
                self_loop[ci as usize] += w;
            } else if ci < cj {
                *pairs.entry((ci, cj)).or_insert(0.0) += w;
            }
        }
    }
    (UGraph::from_pairs(k, pairs, self_loop), node_dense)
}

/// Splits every community into its connected components in `g`. Components are numbered in
/// order of their smallest node. Returns the assignment, the community count and the number
/// of extra communities created.
fn split_disconnected(g: &UGraph, membership: &[u32]) -> (Vec<u32>, usize, u32) {
    let n = membership.len();
    let mut result = vec![u32::MAX; n];
    let mut parts: HashMap<u32, u32> = HashMap::new();
    let mut next = 0u32;
    let mut stack: Vec<u32> = Vec::new();
    for start in 0..n {
        if result[start] != u32::MAX {
            continue;
        }
        let c = membership[start];
        let id = next;
        next += 1;
        *parts.entry(c).or_insert(0) += 1;
        result[start] = id;
        stack.push(start as u32);
        while let Some(v) = stack.pop() {
            for &(w, _) in &g.adj[v as usize] {
                let wi = w as usize;
                if result[wi] == u32::MAX && membership[wi] == c {
                    result[wi] = id;
                    stack.push(w);
                }
            }
        }
    }
    let splits = parts.values().map(|&p| p.saturating_sub(1)).sum();
    (result, next as usize, splits)
}

/// Modularity of an assignment with `k` communities, and each community's `Σ_in` and
/// `Σ_tot`.
fn modularity(g: &UGraph, assignment: &[u32], k: usize, resolution: f64) -> (f64, Vec<f64>, Vec<f64>) {
    let mut inside = vec![0.0f64; k];
    let mut total = vec![0.0f64; k];
    for i in 0..g.len() {
        let c = assignment[i] as usize;
        total[c] += g.degree[i];
        inside[c] += g.self_loop[i];
        for &(j, w) in &g.adj[i] {
            if assignment[j as usize] as usize == c {
                inside[c] += w;
            }
        }
    }
    if g.two_m <= 0.0 {
        return (0.0, inside, total);
    }
    let q = (0..k)
        .map(|c| inside[c] / g.two_m - resolution * (total[c] / g.two_m).powi(2))
        .sum();
    (q, inside, total)
}

// ── Labels ───────────────────────────────────────────────────────────────────

fn label_communities(
    model: &ArchModel,
    level: Level,
    members: &[Vec<NodeId>],
    base: &UGraph,
    local: &[Option<u32>],
) -> Vec<(String, LabelBasis)> {
    let graph = model.graph(level);
    let container_level = level.container();

    // Votes of type-level units for containers, per community.
    let mut community_of_node: HashMap<NodeId, usize> = HashMap::new();
    for (c, list) in members.iter().enumerate() {
        for &m in list {
            community_of_node.insert(m, c);
        }
    }
    let mut votes: Vec<BTreeMap<NodeId, u32>> = vec![BTreeMap::new(); members.len()];
    let mut vote_totals = vec![0u32; members.len()];
    if let Some(cl) = container_level {
        for u in 0..model.types.node_count() as u32 {
            let unit = NodeId(u);
            let at_level = model.container_of(unit, level);
            let Some(&c) = community_of_node.get(&at_level) else {
                continue;
            };
            let container = model.container_of(unit, cl);
            *votes[c].entry(container).or_insert(0) += 1;
            vote_totals[c] += 1;
        }
    }

    members
        .iter()
        .enumerate()
        .map(|(c, list)| {
            if list.len() == 1 {
                return (graph.node(list[0]).name.clone(), LabelBasis::Singleton);
            }
            let names: Vec<&str> = list.iter().map(|&m| graph.node(m).name.as_str()).collect();
            if let Some(prefix) = common_segment_prefix(&names) {
                return (prefix, LabelBasis::CommonPrefix);
            }
            if let Some(cl) = container_level {
                // Most votes, ties to the smallest container id (BTreeMap order).
                let mut best: Option<(NodeId, u32)> = None;
                for (&container, &count) in &votes[c] {
                    if best.is_none_or(|(_, b)| count > b) {
                        best = Some((container, count));
                    }
                }
                if let Some((container, count)) = best {
                    let name = &model.graph(cl).node(container).name;
                    if !name.is_empty() && vote_totals[c] > 0 {
                        return (
                            name.clone(),
                            LabelBasis::DominantContainer {
                                container,
                                level: cl,
                                share: f64::from(count) / f64::from(vote_totals[c]),
                            },
                        );
                    }
                }
            }
            let mut hub = list[0];
            let mut hub_degree = f64::NEG_INFINITY;
            for &m in list {
                let degree = local[m.index()].map_or(0.0, |i| base.degree[i as usize]);
                if degree > hub_degree {
                    hub = m;
                    hub_degree = degree;
                }
            }
            (graph.node(hub).name.clone(), LabelBasis::Hub { node: hub })
        })
        .collect()
}

#[inline]
fn is_identifier_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

/// Longest common prefix of `names` that ends at a segment boundary of every name (the end
/// of the name or a non-identifier character), without trailing separators; `None` when it
/// contains no identifier character.
fn common_segment_prefix(names: &[&str]) -> Option<String> {
    let first: Vec<char> = names.first()?.chars().collect();
    let others: Vec<Vec<char>> = names[1..].iter().map(|n| n.chars().collect()).collect();
    let mut lcp = first.len();
    for other in &others {
        lcp = lcp.min(
            first
                .iter()
                .zip(other.iter())
                .take_while(|(a, b)| a == b)
                .count(),
        );
    }
    let at_boundary = |name: &[char], p: usize| p == name.len() || !is_identifier_char(name[p]);
    let mut p = lcp;
    while p > 0 && !(at_boundary(&first, p) && others.iter().all(|o| at_boundary(o, p))) {
        p -= 1;
    }
    while p > 0 && !is_identifier_char(first[p - 1]) {
        p -= 1;
    }
    let prefix: String = first[..p].iter().collect();
    prefix.chars().any(is_identifier_char).then_some(prefix)
}
