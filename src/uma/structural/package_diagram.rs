//! PackageDiagramExtractor — packages, inter-package dependencies and Martin's
//! package metrics (§9.2.1).
//!
//! - Package membership is taken from `parent_sym` (nearest package ancestor),
//!   never from sibling chains.
//! - A dependency P → Q exists when a type of P references a type of Q through
//!   inheritance, an attribute, a parameter or result type, an instantiation, or
//!   a validated call; `reference_count` counts those references.
//! - Dependency cycles are the strongly connected components of the package
//!   graph (Tarjan); Ca, Ce, instability I, abstractness A and the distance from
//!   the main sequence D follow Martin (1994). Undefined ratios are NaN.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::core::types::symbol::SymbolKind;
use crate::uma::bodies::MemberBodies;
use crate::uma::index::UMAIndex;
use crate::uma::types::*;

/// Package-level structure of one Phase 9 run.
pub struct PackageStructure {
    pub packages: Vec<PackageRecord>,
    pub dependencies: Vec<PackageDependencyRecord>,
    pub metrics: Vec<PackageMetricsRecord>,
}

pub struct PackageDiagramExtractor;

impl PackageDiagramExtractor {
    pub fn extract(
        index: &UMAIndex<'_>,
        bodies: &MemberBodies,
        class_syms: &[u32],
    ) -> PackageStructure {
        let mut packages = Vec::new();
        let mut type_count: HashMap<u32, usize> = HashMap::new();
        let mut top_level_count: HashMap<u32, usize> = HashMap::new();
        let mut abstract_count: HashMap<u32, usize> = HashMap::new();
        for &t in class_syms {
            let p = index.package_of(t);
            if p == UNKNOWN_SYM {
                continue;
            }
            *type_count.entry(p).or_insert(0) += 1;
            if index.enclosing_type(t) == UNKNOWN_SYM {
                *top_level_count.entry(p).or_insert(0) += 1;
            }
            if index.is_abstract_type(t) {
                *abstract_count.entry(p).or_insert(0) += 1;
            }
        }

        for sym_id in 0..index.symbol_count() {
            let id = sym_id as u32;
            if index.kind(id) != Some(SymbolKind::SK_PACKAGE) {
                continue;
            }
            let sym = match index.symbol(id) {
                Some(s) => s,
                None => continue,
            };
            let children = index.children(id);
            if !index.sta.custom_package_names.contains_key(&id)
                && children.is_empty()
                && sym.name_id == u32::MAX
            {
                continue;
            }
            let top_level_types = top_level_count.get(&id).copied().unwrap_or(0);
            let subpackages = children
                .iter()
                .filter(|&&c| index.kind(c) == Some(SymbolKind::SK_PACKAGE))
                .count();
            packages.push(PackageRecord {
                package_sym_id: id,
                name_id: sym.name_id,
                parent_package_sym: index.package_of(id),
                class_count: u16::try_from(top_level_types).unwrap_or(u16::MAX),
                subpackage_count: u16::try_from(subpackages).unwrap_or(u16::MAX),
            });
        }

        // ── Dependency edges ──
        let mut edge_refs: BTreeMap<(u32, u32), u32> = BTreeMap::new();
        for &t in class_syms {
            let from = index.package_of(t);
            if from == UNKNOWN_SYM {
                continue;
            }
            for target in Self::referenced_types(index, bodies, t) {
                let to = index.package_of(target);
                if to == UNKNOWN_SYM || to == from {
                    continue;
                }
                let e = edge_refs.entry((from, to)).or_insert(0);
                *e = e.saturating_add(1);
            }
        }

        // ── Tarjan SCC over the package graph ──
        let nodes: Vec<u32> = {
            let mut s: BTreeSet<u32> = packages.iter().map(|p| p.package_sym_id).collect();
            for &(a, b) in edge_refs.keys() {
                s.insert(a);
                s.insert(b);
            }
            s.into_iter().collect()
        };
        let pos: HashMap<u32, usize> = nodes.iter().enumerate().map(|(i, &n)| (n, i)).collect();
        let mut adj: Vec<Vec<usize>> = vec![Vec::new(); nodes.len()];
        for &(a, b) in edge_refs.keys() {
            adj[pos[&a]].push(pos[&b]);
        }
        let comp = tarjan_scc(&adj);
        let mut comp_size: HashMap<usize, usize> = HashMap::new();
        for &c in &comp {
            *comp_size.entry(c).or_insert(0) += 1;
        }

        let dependencies: Vec<PackageDependencyRecord> = edge_refs
            .iter()
            .map(|(&(a, b), &count)| {
                let ca = comp[pos[&a]];
                let in_cycle = ca == comp[pos[&b]] && comp_size.get(&ca).copied().unwrap_or(0) > 1;
                PackageDependencyRecord {
                    from_package_sym: a,
                    to_package_sym: b,
                    reference_count: count,
                    in_cycle: u8::from(in_cycle),
                }
            })
            .collect();

        // ── Martin metrics ──
        let mut afferent: HashMap<u32, u32> = HashMap::new();
        let mut efferent: HashMap<u32, u32> = HashMap::new();
        for &(a, b) in edge_refs.keys() {
            *efferent.entry(a).or_insert(0) += 1;
            *afferent.entry(b).or_insert(0) += 1;
        }
        let metrics = packages
            .iter()
            .map(|p| {
                let id = p.package_sym_id;
                let ca = afferent.get(&id).copied().unwrap_or(0);
                let ce = efferent.get(&id).copied().unwrap_or(0);
                let instability = if ca + ce == 0 {
                    f32::NAN
                } else {
                    ce as f32 / (ca + ce) as f32
                };
                let types = type_count.get(&id).copied().unwrap_or(0);
                let abstractness = if types == 0 {
                    f32::NAN
                } else {
                    abstract_count.get(&id).copied().unwrap_or(0) as f32 / types as f32
                };
                let distance = if instability.is_nan() || abstractness.is_nan() {
                    f32::NAN
                } else {
                    (abstractness + instability - 1.0).abs()
                };
                let cycle_id = match pos.get(&id) {
                    Some(&i) if comp_size.get(&comp[i]).copied().unwrap_or(0) > 1 => {
                        u32::try_from(comp[i]).unwrap_or(u32::MAX - 1)
                    }
                    _ => u32::MAX,
                };
                PackageMetricsRecord {
                    package_sym_id: id,
                    afferent: ca,
                    efferent: ce,
                    instability,
                    abstractness,
                    distance,
                    cycle_id,
                }
            })
            .collect();

        PackageStructure {
            packages,
            dependencies,
            metrics,
        }
    }

    /// User types a type references (one entry per reference).
    pub fn referenced_types(index: &UMAIndex<'_>, bodies: &MemberBodies, t: u32) -> Vec<u32> {
        let mut out = index.direct_supertypes(t);
        for f in index.fields(t) {
            out.push(index.type_of(f));
        }
        for &m in index.children(t) {
            match index.kind(m) {
                Some(SymbolKind::SK_METHOD) | Some(SymbolKind::SK_CONSTRUCTOR) => {}
                _ => continue,
            }
            out.push(index.type_of(m));
            out.extend(index.param_types(m));
            for c in index.calls_of_member(m) {
                let owner = index.enclosing_type(c.callee);
                if owner != t {
                    out.push(owner);
                }
            }
            if let Some(body) = bodies.get(m) {
                for a in &body.allocations {
                    out.push(index.resolve_type_text(a.type_text, m));
                }
            }
        }
        out.retain(|&r| r != t && index.is_user_type(r));
        out
    }
}

/// Iterative Tarjan SCC; returns the component id of each node.
fn tarjan_scc(adj: &[Vec<usize>]) -> Vec<usize> {
    let n = adj.len();
    let mut index_of = vec![usize::MAX; n];
    let mut low = vec![0usize; n];
    let mut on_stack = vec![false; n];
    let mut comp = vec![usize::MAX; n];
    let mut stack: Vec<usize> = Vec::new();
    let mut next_index = 0usize;
    let mut next_comp = 0usize;

    for root in 0..n {
        if index_of[root] != usize::MAX {
            continue;
        }
        // (node, next edge position)
        let mut work: Vec<(usize, usize)> = vec![(root, 0)];
        index_of[root] = next_index;
        low[root] = next_index;
        next_index += 1;
        stack.push(root);
        on_stack[root] = true;

        while let Some(&mut (v, ref mut ei)) = work.last_mut() {
            if *ei < adj[v].len() {
                let w = adj[v][*ei];
                *ei += 1;
                if index_of[w] == usize::MAX {
                    index_of[w] = next_index;
                    low[w] = next_index;
                    next_index += 1;
                    stack.push(w);
                    on_stack[w] = true;
                    work.push((w, 0));
                } else if on_stack[w] {
                    low[v] = low[v].min(index_of[w]);
                }
            } else {
                work.pop();
                if let Some(&(parent, _)) = work.last() {
                    low[parent] = low[parent].min(low[v]);
                }
                if low[v] == index_of[v] {
                    while let Some(w) = stack.pop() {
                        on_stack[w] = false;
                        comp[w] = next_comp;
                        if w == v {
                            break;
                        }
                    }
                    next_comp += 1;
                }
            }
        }
    }
    comp
}
