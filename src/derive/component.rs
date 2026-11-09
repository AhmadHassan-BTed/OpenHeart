//! Component diagram from the project model: build units (projects, targets, assemblies,
//! packages) with the dependencies their build descriptors declare, the dependencies the
//! IR shows between the units' code, and the interfaces units provide to and require
//! from each other.
//!
//! A unit owns the code of its source files. An interface-like type is *provided* by the
//! units whose classes realize it and *required* by the units whose code uses it; only
//! interfaces that cross a unit boundary are shown. A declared dependency without an
//! observed use, and an observed use without a declared dependency, are both marked.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::derive::class::ir_type_uses;
use crate::derive::model::{
    Derivation, DiagramEdge, DiagramKind, DiagramModel, DiagramNode, EdgeKind, ElementId,
    IdBuilder, NodeKind,
};
use crate::derive::provenance::{symbol_span, Fact};
use crate::derive::{DeriveContext, ProjectUnitFact, SuperRelation};
use crate::ir::types::Span;

/// Unit owning each corpus file (the lowest unit id when several list the file).
pub(crate) fn file_units(units: &[ProjectUnitFact]) -> HashMap<u32, u32> {
    let mut sorted: Vec<&ProjectUnitFact> = units.iter().collect();
    sorted.sort_by_key(|u| u.id);
    let mut out = HashMap::new();
    for u in sorted {
        for &f in &u.files {
            out.entry(f).or_insert(u.id);
        }
    }
    out
}

fn unit_id(unit: &ProjectUnitFact) -> ElementId {
    IdBuilder::new("unit")
        .text(unit.kind.mnemonic())
        .text(&unit.name)
        .text(unit.descriptor.as_deref().unwrap_or(""))
        .finish()
}

/// The component diagram of the project model.
pub fn derive(ctx: &DeriveContext) -> DiagramModel {
    let kind = DiagramKind::Component;
    let Some(project) = ctx.project else {
        return DiagramModel::not_derivable(
            kind,
            kind.title(),
            "the project model is unavailable: components are the build units the build descriptors declare",
        );
    };
    let mut units = project.units();
    units.sort_by_key(|u| u.id);
    if units.is_empty() {
        return DiagramModel::not_derivable(
            kind,
            kind.title(),
            "the build descriptors declare no build unit",
        );
    }
    let mut model = DiagramModel::new(kind, kind.title());
    let ids: BTreeMap<u32, ElementId> = units.iter().map(|u| (u.id, unit_id(u))).collect();
    let files = file_units(&units);
    let unit_of_symbol = |sym: u32| -> Option<u32> {
        let span = symbol_span(ctx, sym)?;
        files.get(&span.file_id).copied()
    };

    for u in &units {
        let mut node = DiagramNode::new(ids[&u.id].clone(), NodeKind::Component, u.name.clone())
            .with_stereotype(u.kind.mnemonic())
            .with_parent(u.parent.and_then(|p| ids.get(&p).cloned()))
            .with_property("source files", u.files.len().to_string())
            .with_fact(Fact::ProjectUnit(u.id));
        if let Some(d) = &u.descriptor {
            node = node.with_property("descriptor", d.clone());
        }
        if let Some(v) = &u.version {
            node = node.with_property("version", v.clone());
        }
        model.add_node(node);
    }

    // Observed unit-to-unit uses.
    let mut observed: BTreeMap<(u32, u32), (Vec<Span>, Vec<Fact>, bool)> = BTreeMap::new();
    // Interface-like types: who realizes them and who uses them.
    let mut providers: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
    let mut requirers: BTreeMap<u32, (BTreeSet<u32>, Vec<Span>)> = BTreeMap::new();
    let is_interface = |sym: u32| ctx.category(sym).map_or(false, |c| c.is_interface_like());

    for func in ctx.funcs() {
        let Some(from) = files.get(&func.span.file_id).copied() else {
            continue;
        };
        for u in ir_type_uses(ctx, func) {
            if is_interface(u.target) {
                let e = requirers.entry(u.target).or_default();
                e.0.insert(from);
                e.1.extend(u.span);
            }
            let Some(to) = unit_of_symbol(u.target) else {
                continue;
            };
            if to == from {
                continue;
            }
            let e = observed
                .entry((from, to))
                .or_insert_with(|| (Vec::new(), Vec::new(), false));
            e.0.extend(u.span);
            e.1.push(u.fact);
            e.2 |= u.exact;
        }
    }
    for sym in crate::derive::class::project_classifiers(ctx) {
        let Some(unit) = unit_of_symbol(sym) else {
            continue;
        };
        for sup in ctx.program.supertypes(sym) {
            if sup.relation == SuperRelation::Implements || is_interface(sup.target) {
                providers.entry(sup.target).or_default().insert(unit);
                if let Some(to) = unit_of_symbol(sup.target) {
                    if to != unit {
                        let e = observed
                            .entry((unit, to))
                            .or_insert_with(|| (Vec::new(), Vec::new(), false));
                        e.0.extend(sup.span);
                        e.1.push(Fact::Supertype {
                            sub: sym,
                            sup: sup.target,
                        });
                        e.2 = true;
                    }
                }
            }
        }
    }

    // Declared dependencies, merged with the observed ones.
    let mut deps = project.dependencies();
    deps.sort_by(|a, b| (a.from, a.to, a.scope).cmp(&(b.from, b.to, b.scope)));
    let mut declared_pairs: BTreeSet<(u32, u32)> = BTreeSet::new();
    for d in &deps {
        let (Some(s), Some(t)) = (ids.get(&d.from), ids.get(&d.to)) else {
            continue;
        };
        declared_pairs.insert((d.from, d.to));
        let seen = observed.get(&(d.from, d.to));
        let mut edge = DiagramEdge::new(
            IdBuilder::new("unitdep")
                .text(s.as_str())
                .text(t.as_str())
                .text(d.scope.mnemonic())
                .finish(),
            EdgeKind::Dependency,
            s.clone(),
            t.clone(),
        )
        .with_stereotype(d.scope.mnemonic())
        .with_property("declared", "true")
        .with_property("observed in code", if seen.is_some() { "true" } else { "false" })
        .with_fact(Fact::UnitDependency {
            from: d.from,
            to: d.to,
        });
        if let Some(v) = &d.version {
            edge = edge.with_property("version", v.clone());
        }
        if let Some(desc) = &d.descriptor {
            edge = edge.with_property("descriptor", desc.clone());
        }
        if let Some((spans, facts, _)) = seen {
            edge = edge.with_spans(spans.iter().copied()).with_facts(facts.iter().copied());
        }
        model.add_edge(edge);
    }
    for ((from, to), (spans, facts, exact)) in observed {
        if declared_pairs.contains(&(from, to)) {
            continue;
        }
        let (Some(s), Some(t)) = (ids.get(&from), ids.get(&to)) else {
            continue;
        };
        model.add_edge(
            DiagramEdge::new(
                IdBuilder::new("unitdep")
                    .text(s.as_str())
                    .text(t.as_str())
                    .text("use")
                    .finish(),
                EdgeKind::Dependency,
                s.clone(),
                t.clone(),
            )
            .with_stereotype("use")
            .with_property("declared", "false")
            .with_property("observed in code", "true")
            .with_spans(spans)
            .with_facts(facts)
            .with_derivation(if exact {
                Derivation::Exact
            } else {
                Derivation::OverApproximate
            }),
        );
    }

    // Interfaces crossing unit boundaries.
    let mut interfaces: BTreeSet<u32> = providers.keys().copied().collect();
    interfaces.extend(requirers.keys().copied());
    for iface in interfaces {
        let provided: BTreeSet<u32> = providers.get(&iface).cloned().unwrap_or_default();
        let (required, spans) = requirers
            .get(&iface)
            .map(|(r, s)| (r.clone(), s.clone()))
            .unwrap_or_default();
        let crosses = required.iter().any(|r| provided.iter().any(|p| p != r))
            || (provided.is_empty()
                && unit_of_symbol(iface).map_or(false, |home| required.iter().any(|r| *r != home)));
        if !crosses {
            continue;
        }
        let qn = ctx.qualified_name(iface);
        let iid = IdBuilder::new("iface").text(&qn).finish();
        model.add_node(
            DiagramNode::new(iid.clone(), NodeKind::Interface, ctx.symbol_name(iface))
                .with_qualified_name(qn)
                .with_symbol(iface)
                .with_span(symbol_span(ctx, iface))
                .with_fact(Fact::Symbol(iface)),
        );
        for p in &provided {
            if let Some(s) = ids.get(p) {
                model.add_edge(
                    DiagramEdge::new(
                        IdBuilder::new("provides").text(s.as_str()).text(iid.as_str()).finish(),
                        EdgeKind::Provides,
                        s.clone(),
                        iid.clone(),
                    )
                    .with_fact(Fact::Symbol(iface)),
                );
            }
        }
        for r in &required {
            if provided.contains(r) {
                continue;
            }
            if let Some(s) = ids.get(r) {
                model.add_edge(
                    DiagramEdge::new(
                        IdBuilder::new("requires").text(s.as_str()).text(iid.as_str()).finish(),
                        EdgeKind::Requires,
                        s.clone(),
                        iid.clone(),
                    )
                    .with_spans(spans.iter().copied())
                    .with_fact(Fact::Symbol(iface)),
                );
            }
        }
    }
    model.finalize(ctx.limits.max_spans);
    model
}
