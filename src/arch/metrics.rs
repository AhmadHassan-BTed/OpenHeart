//! Architecture metrics computed from the dependency graphs and the body facts.
//!
//! - **Martin's metrics** (Martin 1994): for a group `G` of type-level units (a package, a
//!   project, a file, a recovered subsystem), afferent coupling `Ca` is the number of units
//!   outside `G` that depend on units inside `G`, efferent coupling `Ce` the number of units
//!   inside `G` that depend on units outside `G`, instability `I = Ce / (Ca + Ce)`,
//!   abstractness `A = abstract types / types` and distance from the main sequence
//!   `D = |A + I − 1|`. Martin counts classes; the language-neutral unit is the type-level unit
//!   (types, free functions, module variables, file-scope code), while `A` counts types only.
//!   `I` is undefined without couplings, `A` without types, and `D` when either is.
//! - **Fan-in / fan-out** per node: distinct nodes that depend on it / that it depends on,
//!   with summed weights; per function: distinct callers / callees of the call facts.
//! - **LCOM4** (Hitz & Montazeri 1995) per type: the number of connected components of the
//!   graph whose vertices are the type's method bodies, two methods being connected when
//!   both access one instance field of the type or one calls the other. A closure counts as
//!   part of the member whose body creates it; constructors and initializers are excluded
//!   unless configured, since they touch every field by construction.
//! - **Cycles** per level: non-trivial strongly connected components, the nodes, edges and
//!   weight inside them, mutual pairs, and the number of elementary cycles (Johnson 1975),
//!   counted up to a limit and reported as complete or truncated.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::arch::dsm::strongly_connected_components;
use crate::arch::graph::{
    ArchModel, CallTarget, Certainty, DependencyGraph, KindSet, Level, MemberRole, NodeId,
    NodeKind, Strength, WeightedDigraph,
};
use crate::ir::types::{FieldId, FuncId};

#[derive(Debug, Clone)]
pub struct MetricsConfig {
    pub kinds: KindSet,
    pub include_external: bool,
    /// Count constructors and initializers as LCOM4 methods.
    pub lcom_include_constructors: bool,
    /// Elementary cycles counted per level before the count stops.
    pub max_elementary_cycles: u64,
}

impl Default for MetricsConfig {
    fn default() -> Self {
        Self {
            kinds: KindSet::STATIC,
            include_external: false,
            lcom_include_constructors: false,
            max_elementary_cycles: 100_000,
        }
    }
}

// ── Martin's metrics ─────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct MartinMetrics {
    /// Group id: the node id at the measured level, or the partition's group id.
    pub group: u32,
    pub units: u32,
    pub types: u32,
    pub abstract_types: u32,
    /// `Ca`: units outside the group that depend on units inside it.
    pub afferent: u32,
    /// `Ce`: units inside the group that depend on units outside it.
    pub efferent: u32,
    pub instability: Option<f64>,
    pub abstractness: Option<f64>,
    pub distance: Option<f64>,
    /// Index of the group dependency cycle containing the group, `None` when acyclic.
    pub cycle: Option<u32>,
}

/// Martin's metrics of the nodes of `level` that contain project units.
pub fn martin(model: &ArchModel, level: Level, config: &MetricsConfig) -> Vec<MartinMetrics> {
    let groups = model.graph(level);
    let group_of: Vec<Option<u32>> = (0..model.types.node_count() as u32)
        .map(|u| {
            let g = model.container_of(NodeId(u), level);
            (!groups.node(g).external).then_some(g.0)
        })
        .collect();
    martin_for_partition(model, &group_of, groups.node_count(), config)
}

/// Martin's metrics of an arbitrary partition of the type-level units (`group_of[unit]`,
/// `None` for units outside every group), e.g. recovered subsystems.
pub fn martin_for_partition(
    model: &ArchModel,
    group_of: &[Option<u32>],
    group_count: usize,
    config: &MetricsConfig,
) -> Vec<MartinMetrics> {
    let types = &model.types;
    let included = |u: NodeId| types.included(u, config.include_external);

    let mut units = vec![0u32; group_count];
    let mut type_count = vec![0u32; group_count];
    let mut abstract_count = vec![0u32; group_count];
    for node in &types.nodes {
        let Some(g) = group_of.get(node.id.index()).copied().flatten() else {
            continue;
        };
        if !included(node.id) {
            continue;
        }
        units[g as usize] += 1;
        if node.kind == NodeKind::Type {
            type_count[g as usize] += 1;
            if node.is_abstract {
                abstract_count[g as usize] += 1;
            }
        }
    }

    let mut afferent: BTreeSet<(u32, u32)> = BTreeSet::new();
    let mut efferent: BTreeSet<(u32, u32)> = BTreeSet::new();
    let mut group_edges: Vec<(u32, u32, u64)> = Vec::new();
    for e in &types.edges {
        if !config.kinds.contains(e.kind) || !included(e.from) || !included(e.to) {
            continue;
        }
        let gf = group_of.get(e.from.index()).copied().flatten();
        let gt = group_of.get(e.to.index()).copied().flatten();
        if gf == gt {
            continue;
        }
        if let Some(gt) = gt {
            afferent.insert((gt, e.from.0));
        }
        if let Some(gf) = gf {
            efferent.insert((gf, e.from.0));
        }
        if let (Some(gf), Some(gt)) = (gf, gt) {
            group_edges.push((gf, gt, u64::from(e.weight())));
        }
    }
    let mut ca = vec![0u32; group_count];
    let mut ce = vec![0u32; group_count];
    for &(g, _) in &afferent {
        ca[g as usize] += 1;
    }
    for &(g, _) in &efferent {
        ce[g as usize] += 1;
    }

    let group_graph = WeightedDigraph::from_edges(group_count, group_edges);
    let cycle_of = cyclic_component_index(&group_graph);

    (0..group_count)
        .filter(|&g| units[g] > 0)
        .map(|g| {
            let instability = (ca[g] + ce[g] > 0)
                .then(|| f64::from(ce[g]) / f64::from(ca[g] + ce[g]));
            let abstractness =
                (type_count[g] > 0).then(|| f64::from(abstract_count[g]) / f64::from(type_count[g]));
            let distance = match (abstractness, instability) {
                (Some(a), Some(i)) => Some((a + i - 1.0).abs()),
                _ => None,
            };
            MartinMetrics {
                group: g as u32,
                units: units[g],
                types: type_count[g],
                abstract_types: abstract_count[g],
                afferent: ca[g],
                efferent: ce[g],
                instability,
                abstractness,
                distance,
                cycle: cycle_of[g],
            }
        })
        .collect()
}

/// Index of the non-trivial component of each node (components numbered by their smallest
/// member), `None` for nodes on no cycle.
fn cyclic_component_index(g: &WeightedDigraph) -> Vec<Option<u32>> {
    let comps = strongly_connected_components(g);
    let mut cyclic: Vec<&Vec<u32>> = comps.members.iter().filter(|m| m.len() > 1).collect();
    cyclic.sort_by_key(|m| m[0]);
    let mut out = vec![None; g.node_count()];
    for (i, members) in cyclic.iter().enumerate() {
        for &v in members.iter() {
            out[v as usize] = Some(i as u32);
        }
    }
    out
}

// ── Fan-in / fan-out ─────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Fan {
    pub node: NodeId,
    pub fan_in: u32,
    pub fan_out: u32,
    pub weighted_in: u64,
    pub weighted_out: u64,
}

/// Fan-in and fan-out of every included node of `graph`.
pub fn fan(graph: &DependencyGraph, kinds: KindSet, include_external: bool) -> Vec<Fan> {
    let adjacency = graph.adjacency(kinds, include_external);
    let reverse = adjacency.transpose();
    graph
        .nodes
        .iter()
        .filter(|n| graph.included(n.id, include_external))
        .map(|n| Fan {
            node: n.id,
            fan_in: reverse.out_degree(n.id.0) as u32,
            fan_out: adjacency.out_degree(n.id.0) as u32,
            weighted_in: reverse.out_weight(n.id.0),
            weighted_out: adjacency.out_weight(n.id.0),
        })
        .collect()
}

#[derive(Debug, Clone)]
pub struct FunctionFan {
    pub func: FuncId,
    /// Distinct calling functions.
    pub fan_in: u32,
    /// Distinct callees: lowered functions, and declarations without a lowered body.
    pub fan_out: u32,
    /// Callees among `fan_out` that have no lowered body (external or unlowered).
    pub unlowered_callees: u32,
    /// Call sites with no derivable target.
    pub unresolved_calls: u32,
    pub certainty: Certainty,
}

/// Call fan-in and fan-out of every IR function. A site resolved by the call facts counts
/// its targets; an unresolved site counts the callee it names, if any.
pub fn function_fan(model: &ArchModel) -> Vec<FunctionFan> {
    let mut by_site: HashMap<u32, Vec<(FuncId, Strength)>> = HashMap::new();
    for c in &model.calls {
        by_site.entry(c.site.0).or_default().push((c.callee, c.strength));
    }
    let func_of_symbol: HashMap<u32, FuncId> = model
        .functions
        .iter()
        .filter_map(|f| f.symbol.map(|s| (s, f.func)))
        .collect();

    let mut callers: HashMap<FuncId, HashSet<FuncId>> = HashMap::new();
    let mut rows: Vec<(FuncId, HashSet<FuncId>, HashSet<u32>, u32, bool)> = Vec::new();
    for f in &model.functions {
        let mut callees: HashSet<FuncId> = HashSet::new();
        let mut unlowered: HashSet<u32> = HashSet::new();
        let mut unresolved = 0u32;
        let mut over = false;
        for call in &f.calls {
            if let Some(targets) = by_site.get(&call.site.0) {
                for &(callee, strength) in targets {
                    callees.insert(callee);
                    callers.entry(callee).or_default().insert(f.func);
                    over |= strength != Strength::Exact;
                }
                continue;
            }
            match call.target {
                CallTarget::Function(callee) => {
                    callees.insert(callee);
                    callers.entry(callee).or_default().insert(f.func);
                }
                CallTarget::Symbol(sym) => match func_of_symbol.get(&sym) {
                    Some(&callee) => {
                        callees.insert(callee);
                        callers.entry(callee).or_default().insert(f.func);
                    }
                    None => {
                        unlowered.insert(sym);
                    }
                },
                CallTarget::Indirect | CallTarget::Unresolved(_) => unresolved += 1,
            }
        }
        rows.push((f.func, callees, unlowered, unresolved, over));
    }

    rows.into_iter()
        .map(|(func, callees, unlowered, unresolved, over)| {
            let certainty = if unresolved > 0 {
                Certainty::Partial {
                    reason: format!("{} call site(s) have no derivable target", unresolved),
                }
            } else if !model.coverage.call_graph {
                Certainty::Partial {
                    reason: "runtime dispatch targets were not computed (no points-to call graph); declared callees are counted"
                        .to_string(),
                }
            } else if over {
                Certainty::OverApproximate
            } else {
                Certainty::Exact
            };
            FunctionFan {
                func,
                fan_in: callers.get(&func).map_or(0, |s| s.len() as u32),
                fan_out: (callees.len() + unlowered.len()) as u32,
                unlowered_callees: unlowered.len() as u32,
                unresolved_calls: unresolved,
                certainty,
            }
        })
        .collect()
}

// ── LCOM4 ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Cohesion {
    pub type_node: NodeId,
    /// Method bodies considered.
    pub methods: u32,
    /// Instance fields of the type (non-synthetic, non-static).
    pub fields: u32,
    /// Number of connected components; `None` when it is undefined or not derivable.
    pub lcom4: Option<u32>,
    /// Methods of each component, sorted.
    pub components: Vec<Vec<FuncId>>,
    pub certainty: Certainty,
}

/// LCOM4 of every project type.
pub fn lcom4(model: &ArchModel, config: &MetricsConfig) -> Vec<Cohesion> {
    let project_types: Vec<NodeId> = model
        .types
        .nodes
        .iter()
        .filter(|n| n.kind == NodeKind::Type && !n.external)
        .map(|n| n.id)
        .collect();
    if !model.coverage.ir {
        return project_types
            .into_iter()
            .map(|t| Cohesion {
                type_node: t,
                methods: 0,
                fields: 0,
                lcom4: None,
                components: Vec::new(),
                certainty: Certainty::NotDerivable {
                    reason: "field accesses and calls of method bodies need the IR".to_string(),
                },
            })
            .collect();
    }

    let mut functions_of: HashMap<NodeId, Vec<usize>> = HashMap::new();
    for (i, f) in model.functions.iter().enumerate() {
        functions_of.entry(f.owner).or_default().push(i);
    }
    let mut fields_of: HashMap<NodeId, BTreeSet<FieldId>> = HashMap::new();
    for f in &model.fields {
        if !f.synthetic && !f.is_static {
            fields_of.entry(f.owner).or_default().insert(f.field);
        }
    }

    project_types
        .into_iter()
        .map(|t| {
            let empty = Vec::new();
            let funcs = functions_of.get(&t).unwrap_or(&empty);
            let fields: Vec<FieldId> = fields_of
                .get(&t)
                .map(|s| s.iter().copied().collect())
                .unwrap_or_default();
            cohesion_of(model, t, funcs, &fields, config)
        })
        .collect()
}

fn cohesion_of(
    model: &ArchModel,
    type_node: NodeId,
    funcs: &[usize],
    fields: &[FieldId],
    config: &MetricsConfig,
) -> Cohesion {
    let is_method = |role: MemberRole| match role {
        MemberRole::Method => true,
        MemberRole::Constructor | MemberRole::Initializer => config.lcom_include_constructors,
        MemberRole::Closure | MemberRole::Function => false,
    };
    let methods: Vec<usize> = funcs
        .iter()
        .copied()
        .filter(|&i| is_method(model.functions[i].role))
        .collect();
    let m = methods.len();
    let method_index: HashMap<FuncId, usize> = methods
        .iter()
        .enumerate()
        .map(|(k, &i)| (model.functions[i].func, k))
        .collect();
    let method_of_symbol: HashMap<u32, usize> = methods
        .iter()
        .enumerate()
        .filter_map(|(k, &i)| model.functions[i].symbol.map(|s| (s, k)))
        .collect();
    // Members whose bodies are excluded (constructors, initializers) keep their closures out.
    let excluded_symbols: HashSet<u32> = funcs
        .iter()
        .filter(|&&i| {
            let f = &model.functions[i];
            matches!(f.role, MemberRole::Constructor | MemberRole::Initializer)
                && !config.lcom_include_constructors
        })
        .filter_map(|&i| model.functions[i].symbol)
        .collect();

    if m == 0 {
        return Cohesion {
            type_node,
            methods: 0,
            fields: fields.len() as u32,
            lcom4: None,
            components: Vec::new(),
            certainty: Certainty::NotDerivable {
                reason: "LCOM4 is undefined for a type without method bodies".to_string(),
            },
        };
    }

    // Union-find over methods followed by one slot per field.
    let field_slot: HashMap<FieldId, usize> = fields
        .iter()
        .enumerate()
        .map(|(k, &f)| (f, m + k))
        .collect();
    let mut parent: Vec<usize> = (0..m + fields.len()).collect();
    let mut unattributed = 0u32;

    let connect = |member: usize, fi: usize, parent: &mut Vec<usize>| {
        let f = &model.functions[fi];
        for field in &f.instance_fields {
            if let Some(&slot) = field_slot.get(field) {
                union(parent, member, slot);
            }
        }
        for callee in &f.static_callees {
            if let Some(&k) = method_index.get(callee) {
                union(parent, member, k);
            }
        }
        for sym in &f.symbol_callees {
            if let Some(&k) = method_of_symbol.get(sym) {
                union(parent, member, k);
            }
        }
    };

    for (k, &i) in methods.iter().enumerate() {
        connect(k, i, &mut parent);
    }
    for &i in funcs {
        let f = &model.functions[i];
        if f.role != MemberRole::Closure {
            continue;
        }
        match f.enclosing_member {
            Some(sym) if excluded_symbols.contains(&sym) => {}
            Some(sym) => match method_of_symbol.get(&sym) {
                Some(&k) => connect(k, i, &mut parent),
                None => unattributed += 1,
            },
            None => unattributed += 1,
        }
    }

    let mut groups: BTreeMap<usize, Vec<FuncId>> = BTreeMap::new();
    for (k, &i) in methods.iter().enumerate() {
        let root = find(&mut parent, k);
        groups.entry(root).or_default().push(model.functions[i].func);
    }
    let mut components: Vec<Vec<FuncId>> = groups
        .into_values()
        .map(|mut v| {
            v.sort_unstable();
            v
        })
        .collect();
    components.sort();

    Cohesion {
        type_node,
        methods: m as u32,
        fields: fields.len() as u32,
        lcom4: Some(components.len() as u32),
        components,
        certainty: if unattributed > 0 {
            Certainty::Partial {
                reason: format!(
                    "{} closure(s) of the type could not be attributed to the member that creates them",
                    unattributed
                ),
            }
        } else {
            Certainty::Exact
        },
    }
}

fn find(parent: &mut [usize], mut x: usize) -> usize {
    while parent[x] != x {
        parent[x] = parent[parent[x]];
        x = parent[x];
    }
    x
}

fn union(parent: &mut [usize], a: usize, b: usize) {
    let (ra, rb) = (find(parent, a), find(parent, b));
    if ra != rb {
        let (lo, hi) = if ra < rb { (ra, rb) } else { (rb, ra) };
        parent[hi] = lo;
    }
}

// ── Cycles ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct CycleMetrics {
    pub level: Level,
    pub cyclic_components: u32,
    pub nodes_in_cycles: u32,
    pub edges_in_cycles: u32,
    pub weight_in_cycles: u64,
    pub largest_component: u32,
    /// Unordered node pairs that depend on each other directly.
    pub mutual_pairs: u32,
    pub elementary_cycles: u64,
    /// False when the count stopped at the limit (the true number is larger).
    pub elementary_cycles_complete: bool,
    /// Members of each non-trivial component, sorted, components by smallest member.
    pub components: Vec<Vec<NodeId>>,
}

/// Cycle statistics of `graph` over `kinds`.
pub fn cycles(
    graph: &DependencyGraph,
    kinds: KindSet,
    include_external: bool,
    max_elementary_cycles: u64,
) -> CycleMetrics {
    let g = graph.adjacency(kinds, include_external);
    let comps = strongly_connected_components(&g);
    let mut cyclic: Vec<Vec<u32>> = comps
        .members
        .iter()
        .filter(|m| m.len() > 1)
        .cloned()
        .collect();
    cyclic.sort_by_key(|m| m[0]);

    let mut edges_in = 0u32;
    let mut weight_in = 0u64;
    let mut mutual = 0u32;
    for u in 0..g.node_count() as u32 {
        for (v, w) in g.successors(u) {
            let cu = comps.component[u as usize];
            if cu == comps.component[v as usize] && comps.is_cyclic(cu) {
                edges_in += 1;
                weight_in += w;
            }
            if u < v && g.weight(v, u) > 0 {
                mutual += 1;
            }
        }
    }
    let (elementary, complete) = count_elementary_cycles(&g, &cyclic, max_elementary_cycles);

    CycleMetrics {
        level: graph.level,
        cyclic_components: cyclic.len() as u32,
        nodes_in_cycles: cyclic.iter().map(|m| m.len() as u32).sum(),
        edges_in_cycles: edges_in,
        weight_in_cycles: weight_in,
        largest_component: cyclic.iter().map(|m| m.len() as u32).max().unwrap_or(0),
        mutual_pairs: mutual,
        elementary_cycles: elementary,
        elementary_cycles_complete: complete,
        components: cyclic
            .into_iter()
            .map(|m| m.into_iter().map(NodeId).collect())
            .collect(),
    }
}

/// Johnson's algorithm, counting only: for each strongly connected component, the cycles
/// through its smallest node are enumerated with the blocking discipline, the node is
/// removed, and the components of the rest are processed in turn. Stops at `limit`.
fn count_elementary_cycles(g: &WeightedDigraph, components: &[Vec<u32>], limit: u64) -> (u64, bool) {
    let mut count = 0u64;
    let mut work: Vec<Vec<u32>> = components.to_vec();
    while let Some(comp) = work.pop() {
        let k = comp.len();
        let local = |v: u32| comp.binary_search(&v).ok();
        let adj: Vec<Vec<usize>> = comp
            .iter()
            .map(|&v| g.targets(v).iter().filter_map(|&w| local(w)).collect())
            .collect();

        let start = 0usize;
        let mut blocked = vec![false; k];
        let mut closed = vec![false; k];
        let mut b_sets: Vec<Vec<usize>> = vec![Vec::new(); k];
        let mut path: Vec<usize> = vec![start];
        let mut stack: Vec<(usize, usize)> = vec![(start, 0)];
        blocked[start] = true;
        while let Some(top) = stack.len().checked_sub(1) {
            let (v, i) = stack[top];
            if i < adj[v].len() {
                stack[top].1 = i + 1;
                let w = adj[v][i];
                if w == start {
                    count += 1;
                    if count >= limit {
                        return (count, false);
                    }
                    for &p in &path {
                        closed[p] = true;
                    }
                } else if !blocked[w] {
                    path.push(w);
                    stack.push((w, 0));
                    closed[w] = false;
                    blocked[w] = true;
                }
                continue;
            }
            if closed[v] {
                unblock(v, &mut blocked, &mut b_sets);
            } else {
                for &w in &adj[v] {
                    if !b_sets[w].contains(&v) {
                        b_sets[w].push(v);
                    }
                }
            }
            stack.pop();
            path.pop();
        }

        // Remove the start node and continue with the components of the rest.
        let rest: Vec<u32> = comp[1..].to_vec();
        let mut sub_edges: Vec<(u32, u32, u64)> = Vec::new();
        for (a, &v) in rest.iter().enumerate() {
            for (w, weight) in g.successors(v) {
                if let Ok(b) = rest.binary_search(&w) {
                    sub_edges.push((a as u32, b as u32, weight));
                }
            }
        }
        let sub = WeightedDigraph::from_edges(rest.len(), sub_edges);
        let sub_comps = strongly_connected_components(&sub);
        for members in sub_comps.members {
            if members.len() > 1 {
                work.push(members.into_iter().map(|i| rest[i as usize]).collect());
            }
        }
    }
    (count, true)
}

fn unblock(v: usize, blocked: &mut [bool], b_sets: &mut [Vec<usize>]) {
    let mut stack = vec![v];
    while let Some(u) = stack.pop() {
        if blocked[u] {
            blocked[u] = false;
            stack.append(&mut b_sets[u]);
        }
    }
}

// ── All metrics ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ArchMetrics {
    pub packages: Vec<MartinMetrics>,
    pub projects: Vec<MartinMetrics>,
    pub files: Vec<MartinMetrics>,
    pub fan: Vec<(Level, Vec<Fan>)>,
    pub functions: Vec<FunctionFan>,
    pub cohesion: Vec<Cohesion>,
    pub cycles: Vec<CycleMetrics>,
}

pub fn compute(model: &ArchModel, config: &MetricsConfig) -> ArchMetrics {
    ArchMetrics {
        packages: martin(model, Level::Package, config),
        projects: martin(model, Level::Project, config),
        files: martin(model, Level::File, config),
        fan: Level::ALL
            .iter()
            .map(|&l| (l, fan(model.graph(l), config.kinds, config.include_external)))
            .collect(),
        functions: function_fan(model),
        cohesion: lcom4(model, config),
        cycles: Level::ALL
            .iter()
            .map(|&l| {
                cycles(
                    model.graph(l),
                    config.kinds,
                    config.include_external,
                    config.max_elementary_cycles,
                )
            })
            .collect(),
    }
}
