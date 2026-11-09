//! Package diagram: namespace, package and module containment with import and use
//! dependencies.
//!
//! Containment comes from the declaration tree. «import» and «include» dependencies come
//! from the frontends' import declarations, lifted to the namespaces that contain the
//! importing and the imported element. «use» dependencies come from the IR: a function of
//! one namespace that instantiates, calls, accesses or tests a type of another, or whose
//! signature names one. Cycles of the resulting dependency graph are reported.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::derive::class::ir_type_uses;
use crate::derive::compiler_views::strongly_connected;
use crate::derive::model::{
    CompartmentEntry, CompartmentKind, Derivation, DiagramEdge, DiagramKind, DiagramModel,
    DiagramNode, EdgeKind, ElementId, IdBuilder, NodeKind,
};
use crate::derive::provenance::{symbol_span, Fact};
use crate::derive::{DeriveContext, SymbolCategory};
use crate::ir::types::Span;

struct PackageBuilder<'c, 'a> {
    ctx: &'c DeriveContext<'a>,
    model: DiagramModel,
    nodes: HashMap<u32, ElementId>,
}

impl<'c, 'a> PackageBuilder<'c, 'a> {
    fn package(&mut self, ns: u32) -> ElementId {
        if let Some(id) = self.nodes.get(&ns) {
            return id.clone();
        }
        let ctx = self.ctx;
        let qn = ctx.qualified_name(ns);
        let id = IdBuilder::new("pkg").text(&qn).finish();
        self.nodes.insert(ns, id.clone());
        let fact = ctx.symbol(ns);
        let parent = fact
            .as_ref()
            .and_then(|f| f.parent)
            .and_then(|p| ctx.namespace_of(p))
            .filter(|&p| p != ns)
            .map(|p| self.package(p));
        let mut node = DiagramNode::new(id.clone(), NodeKind::Package, ctx.symbol_name(ns))
            .with_qualified_name(qn)
            .with_parent(parent)
            .with_symbol(ns)
            .with_span(symbol_span(ctx, ns))
            .with_fact(Fact::Symbol(ns));
        if let Some(f) = &fact {
            if f.category == SymbolCategory::Module {
                node = node.with_stereotype("module");
            }
            if f.external {
                node = node.with_property("external", "true");
            }
        }
        let contents: Vec<CompartmentEntry> = ctx
            .program
            .members(ns)
            .into_iter()
            .filter_map(|m| ctx.symbol(m))
            .filter(|m| m.category.is_classifier() && !m.external)
            .map(|m| {
                let mut e = CompartmentEntry::named(m.name.clone());
                e.visibility = Some(m.visibility);
                e.type_text = Some(m.category.mnemonic().to_string());
                e.symbol = Some(m.id);
                e.spans.extend(m.span);
                e
            })
            .collect();
        node = node.with_compartment(CompartmentKind::Contents, contents);
        self.model.add_node(node);
        id
    }
}

/// Namespace that owns the declarations of `sym`: its nearest enclosing namespace, or for
/// a file without one, the namespace of the file's first declaration that has one.
fn owning_namespace(ctx: &DeriveContext, sym: u32) -> Option<u32> {
    if let Some(ns) = ctx.namespace_of(sym) {
        return Some(ns);
    }
    if ctx.category(sym) == Some(SymbolCategory::File) {
        for m in ctx.program.members(sym) {
            if let Some(ns) = ctx.namespace_of(m) {
                return Some(ns);
            }
        }
    }
    None
}

/// Every namespace or module of the declaration tree, sorted by qualified name.
fn project_namespaces(ctx: &DeriveContext) -> Vec<u32> {
    let mut seen = HashSet::new();
    let mut stack: Vec<u32> = ctx.program.top_level();
    let mut found = Vec::new();
    while let Some(sym) = stack.pop() {
        if !seen.insert(sym) {
            continue;
        }
        let Some(fact) = ctx.symbol(sym) else {
            continue;
        };
        if fact.category.is_namespace() && !fact.external {
            found.push(sym);
        }
        if fact.category.is_namespace() || fact.category == SymbolCategory::File {
            stack.extend(ctx.program.members(sym));
        }
    }
    found.sort_by_key(|&s| (ctx.qualified_name(s), s));
    found
}

/// The package diagram of the analysed sources.
pub fn derive(ctx: &DeriveContext) -> DiagramModel {
    let kind = DiagramKind::Package;
    let namespaces = project_namespaces(ctx);
    if namespaces.is_empty() {
        return DiagramModel::not_derivable(
            kind,
            kind.title(),
            "the frontends report no namespace, package or module declarations",
        );
    }
    let mut b = PackageBuilder {
        ctx,
        model: DiagramModel::new(kind, kind.title()),
        nodes: HashMap::new(),
    };
    for &ns in &namespaces {
        b.package(ns);
    }

    // (from, to, stereotype) -> (spans, facts, count, exact)
    let mut deps: BTreeMap<(u32, u32, &'static str), (Vec<Span>, Vec<Fact>, usize, bool)> =
        BTreeMap::new();
    for imp in ctx.program.imports() {
        let (Some(from), Some(to)) = (owning_namespace(ctx, imp.from), owning_namespace(ctx, imp.target)) else {
            continue;
        };
        if from == to {
            continue;
        }
        let e = deps
            .entry((from, to, imp.kind.mnemonic()))
            .or_insert_with(|| (Vec::new(), Vec::new(), 0, true));
        e.0.extend(imp.span);
        e.1.push(Fact::Import {
            from: imp.from,
            target: imp.target,
        });
        e.2 += 1;
    }
    for func in ctx.funcs() {
        let anchor = ctx.owner_type(func).or(func.symbol);
        let Some(from) = anchor.and_then(|s| owning_namespace(ctx, s)) else {
            continue;
        };
        for u in ir_type_uses(ctx, func) {
            let Some(to) = owning_namespace(ctx, u.target) else {
                continue;
            };
            if from == to {
                continue;
            }
            let e = deps
                .entry((from, to, "use"))
                .or_insert_with(|| (Vec::new(), Vec::new(), 0, false));
            e.0.extend(u.span);
            e.1.push(u.fact);
            e.2 += 1;
            e.3 |= u.exact;
        }
        if let Some(sym) = func.symbol {
            if let Some(f) = ctx.symbol(sym) {
                let mut types: Vec<_> = f.params.iter().filter_map(|p| p.ty).collect();
                types.extend(f.ret);
                for t in types {
                    for target in ctx.classifiers_in_type(t) {
                        let Some(to) = owning_namespace(ctx, target) else {
                            continue;
                        };
                        if from == to {
                            continue;
                        }
                        let e = deps
                            .entry((from, to, "use"))
                            .or_insert_with(|| (Vec::new(), Vec::new(), 0, false));
                        e.0.extend(f.span);
                        e.1.push(Fact::Symbol(sym));
                        e.2 += 1;
                        e.3 = true;
                    }
                }
            }
        }
    }

    let mut graph_nodes: BTreeSet<u32> = BTreeSet::new();
    let mut graph_edges: BTreeSet<(u32, u32)> = BTreeSet::new();
    for ((from, to, stereotype), (spans, facts, count, exact)) in deps {
        let s = b.package(from);
        let t = b.package(to);
        let kind = if stereotype == "use" {
            EdgeKind::Dependency
        } else {
            EdgeKind::Import
        };
        graph_nodes.insert(from);
        graph_nodes.insert(to);
        graph_edges.insert((from, to));
        b.model.add_edge(
            DiagramEdge::new(
                IdBuilder::new("pkgdep")
                    .text(s.as_str())
                    .text(t.as_str())
                    .text(stereotype)
                    .finish(),
                kind,
                s,
                t,
            )
            .with_stereotype(stereotype)
            .with_property("occurrences", count.to_string())
            .with_spans(spans)
            .with_facts(facts)
            .with_derivation(if exact {
                Derivation::Exact
            } else {
                Derivation::OverApproximate
            }),
        );
    }

    // Dependency cycles.
    let order: Vec<u32> = graph_nodes.into_iter().collect();
    let pos: HashMap<u32, usize> = order.iter().enumerate().map(|(i, &s)| (s, i)).collect();
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); order.len()];
    for (from, to) in graph_edges {
        adj[pos[&from]].push(pos[&to]);
    }
    for comp in strongly_connected(&adj) {
        if comp.len() < 2 {
            continue;
        }
        let names: Vec<String> = comp.iter().map(|&i| ctx.qualified_name(order[i])).collect();
        let members: Vec<ElementId> = comp.iter().map(|&i| b.package(order[i])).collect();
        for id in &members {
            if let Some(node) = b.model.node_mut(id) {
                node.properties
                    .push(("dependency cycle".to_string(), names.join(", ")));
            }
        }
        b.model
            .note_on(format!("cyclic dependency among {}", names.join(", ")), members);
    }

    let mut model = b.model;
    model.finalize(ctx.limits.max_spans);
    model
}
