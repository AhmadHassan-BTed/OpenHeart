//! Structural view builders: class, object, package, component, composite structure,
//! deployment and profile diagrams (§10.4, from the Phase 9 UMA records).

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::scpg::diagram::context::DiagramContext;
use crate::scpg::diagram::model::{
    AliasTable, EdgeKind, NodeKind, Provenance, ViewEdge, ViewKind, ViewModel, ViewNode,
};
use crate::symbol::model::OccurrenceRole;
use crate::scpg::diagram::views::{
    classifier_node, ensure_package_chain, function_label, multiplicity, settle_provenance,
    type_display, TypeIndex,
};
use crate::uma::types::*;

/// Restricts a model to the focus (matching nodes and their direct neighbours).
fn apply_focus(ctx: &DiagramContext, model: &mut ViewModel) {
    let focus = match ctx.options.focus.as_deref() {
        Some(f) if !f.is_empty() => f.to_string(),
        _ => return,
    };
    let seeds: HashSet<String> = model
        .nodes
        .iter()
        .filter(|n| {
            n.qualified_name
                .as_deref()
                .map(|q| ctx.matches_focus(q))
                .unwrap_or(false)
        })
        .map(|n| n.key.clone())
        .collect();
    if seeds.is_empty() {
        model.note(format!(
            "The focus '{}' matched no element of this view; the whole view is shown.",
            focus
        ));
        return;
    }
    model.retain_neighborhood(&seeds);
}

fn fmt_metric(value: f32) -> String {
    if value.is_finite() {
        format!("{:.2}", value)
    } else {
        "n/a".to_string()
    }
}

// ── 1. CLASS DIAGRAM ─────────────────────────────────────────────────────────

pub fn class_view(ctx: &DiagramContext) -> ViewModel {
    let types = TypeIndex::build(ctx);
    if types.is_empty() {
        return ViewModel::unavailable(
            ViewKind::Class,
            "Phase 9 recorded no classifiers for the analyzed sources.",
        );
    }
    let mut model = ViewModel::new(ViewKind::Class);
    model.source("sta");
    model.source("uma");
    let mut aliases = AliasTable::default();

    for &sym in &types.syms {
        let class_rec = match ctx.class_record(sym) {
            Some(c) => c,
            None => continue,
        };
        let parent = match types.package(sym) {
            Some(pkg) => {
                let pkg = pkg.to_string();
                ensure_package_chain(&mut model, &mut aliases, &pkg)
            }
            None => None,
        };
        if let Some(node) = classifier_node(ctx, &types, &mut aliases, class_rec, parent) {
            model.add_node(node);
        }
    }

    let mut unknown_params = 0usize;
    for &sym in &types.syms {
        let class_rec = match ctx.class_record(sym) {
            Some(c) => c,
            None => continue,
        };
        let src = match types.key(sym) {
            Some(k) => k.to_string(),
            None => continue,
        };
        add_class_relations(ctx, &types, &mut model, class_rec, &src);
        unknown_params += class_rec
            .methods
            .iter()
            .filter(|m| m.param_syms.len() != m.param_count as usize)
            .count();
    }

    if unknown_params > 0 {
        model.note(format!(
            "Parameter names and types are not recorded for {} operation(s); only their arity is shown.",
            unknown_params
        ));
    }
    if ctx.uma.design_patterns.iter().any(|d| d.class_sym != UNKNOWN_SYM) {
        let mut text = "Pattern stereotypes «Pattern N%» list every detected pattern of a class with the Phase 9 structural-evidence score, which is uncalibrated.".to_string();
        if ctx.options.min_pattern_score > 0 {
            text.push_str(&format!(
                " Detections scoring below {}% are not shown.",
                ctx.options.min_pattern_score
            ));
        }
        model.note(text);
    }
    if ctx.uma.classes.iter().any(|c| c.fields.iter().any(|f| f.is_collection != 0)) {
        model.note("Attributes whose declared type is a collection or array show that type as written; their element multiplicity is shown on the association end when the element type is a declared classifier.");
    }
    apply_focus(ctx, &mut model);
    settle_provenance(&mut model);
    model
}

fn add_class_relations(
    ctx: &DiagramContext,
    types: &TypeIndex,
    model: &mut ViewModel,
    class_rec: &ClassRecord,
    src: &str,
) {
    let src_is_interface = class_rec.stereotype == STEREOTYPE_INTERFACE;
    let mut supertypes: BTreeSet<u32> = class_rec.generalization_syms.iter().copied().collect();
    if class_rec.extends_sym != UNKNOWN_SYM {
        supertypes.insert(class_rec.extends_sym);
    }
    supertypes.extend(class_rec.implements_syms.iter().copied());
    let implemented: HashSet<u32> = class_rec.implements_syms.iter().copied().collect();

    for &sup in &supertypes {
        if sup == class_rec.sym_id {
            continue;
        }
        let dst = match types.key(sup) {
            Some(k) => k.to_string(),
            None => continue,
        };
        let target_is_interface = ctx
            .class_record(sup)
            .map(|c| c.stereotype == STEREOTYPE_INTERFACE)
            .unwrap_or(false);
        let kind = if !src_is_interface && (implemented.contains(&sup) || target_is_interface) {
            EdgeKind::Realization
        } else {
            EdgeKind::Generalization
        };
        model.add_edge(ViewEdge::new(src, &dst, kind, ""));
    }

    for &inner in &class_rec.inner_classes {
        if let Some(dst) = types.key(inner) {
            let dst = dst.to_string();
            model.add_edge(ViewEdge::new(src, &dst, EdgeKind::Nesting, ""));
        }
    }

    let mut associated: HashSet<u32> = HashSet::new();
    for f in &class_rec.fields {
        if f.field_kind == FIELD_KIND_ENUM_LITERAL {
            continue;
        }
        let kind = match f.assoc_kind {
            ASSOC_ASSOCIATION => EdgeKind::Association,
            ASSOC_AGGREGATION => EdgeKind::Aggregation,
            ASSOC_COMPOSITION => EdgeKind::Composition,
            ASSOC_DEPENDENCY => EdgeKind::Dependency,
            _ => continue,
        };
        let target_sym =
            if f.element_type_sym_id != UNKNOWN_SYM && types.contains(f.element_type_sym_id) {
                f.element_type_sym_id
            } else {
                f.type_sym_id
            };
        let dst = match types.key(target_sym) {
            Some(k) => k.to_string(),
            None => continue,
        };
        associated.insert(target_sym);
        let role = ctx.simple_name(f.field_sym_id);
        let discriminator = f.field_sym_id.to_string();
        let mut edge = ViewEdge::new(src, &dst, kind, &discriminator)
            .with_span(ctx.symbol_span(f.field_sym_id));
        if kind == EdgeKind::Dependency {
            edge = edge.with_label(Some("«use»".to_string()));
        } else {
            edge = edge
                .with_role(role)
                .with_multiplicity(None, multiplicity(f.mult_min, f.mult_max));
        }
        model.add_edge(edge);
    }

    for &target in &class_rec.association_syms {
        if associated.contains(&target) {
            continue;
        }
        if let Some(dst) = types.key(target) {
            let dst = dst.to_string();
            associated.insert(target);
            model.add_edge(ViewEdge::new(src, &dst, EdgeKind::Association, "assoc"));
        }
    }

    for &target in &class_rec.dependency_syms {
        if target == class_rec.sym_id || associated.contains(&target) || supertypes.contains(&target)
        {
            continue;
        }
        if let Some(dst) = types.key(target) {
            let dst = dst.to_string();
            model.add_edge(
                ViewEdge::new(src, &dst, EdgeKind::Dependency, "use")
                    .with_label(Some("«use»".to_string())),
            );
        }
    }
}

// ── 2. OBJECT DIAGRAM ────────────────────────────────────────────────────────

pub fn object_view(ctx: &DiagramContext) -> ViewModel {
    if ctx.uma.objects.is_empty() {
        return ViewModel::unavailable(
            ViewKind::Object,
            "Phase 9 recorded no allocation sites (`new` expressions) in the analyzed sources.",
        );
    }
    let mut model = ViewModel::new(ViewKind::Object);
    model.source("uma");
    let mut keys: Vec<String> = Vec::with_capacity(ctx.uma.objects.len());

    for obj in &ctx.uma.objects {
        let group = if obj.containing_method_sym != UNKNOWN_SYM {
            let key = format!("objgrp:{}", obj.containing_method_sym);
            if !model.has_node(&key) {
                model.add_node(
                    ViewNode::new(
                        key.clone(),
                        format!("grp_{}", obj.containing_method_sym),
                        function_label(ctx, obj.containing_method_sym),
                        NodeKind::Group,
                    )
                    .with_qualified_name(ctx.member_fqn(obj.containing_method_sym))
                    .with_span(ctx.symbol_span(obj.containing_method_sym))
                    .with_sym(obj.containing_method_sym),
                );
            }
            Some(key)
        } else {
            None
        };
        let type_name = type_display(ctx, obj.type_sym_id)
            .unwrap_or_else(|| "<unresolved type>".to_string());
        let label = ctx
            .uma
            .label(obj.label_text_id)
            .map(str::to_string)
            .unwrap_or_else(|| format!(": {}", type_name));
        let key = format!("obj:{}:{}", obj.containing_method_sym, obj.alloc_token);
        let span = ctx.span_of_tokens(obj.alloc_token, obj.alloc_token);
        let mut details = Vec::new();
        if let Some(s) = &span {
            details.push(format!(
                "allocated at {}:{}",
                s.path.as_deref().unwrap_or("<unknown file>"),
                s.line_start
            ));
        }
        if obj.assigned_field_sym != UNKNOWN_SYM {
            if let Some(field) = ctx.member_label(obj.assigned_field_sym) {
                details.push(format!("stored in {}", field));
            }
        }
        model.add_node(
            ViewNode::new(
                key.clone(),
                format!("obj_{}_{}", obj.containing_method_sym, obj.alloc_token),
                label,
                NodeKind::Object,
            )
            .with_parent(group)
            .with_qualified_name(ctx.type_fqn(obj.type_sym_id))
            .with_span(span)
            .with_details(details)
            .with_sym(obj.type_sym_id),
        );
        keys.push(key);
    }

    for link in &ctx.uma.object_links {
        let (from, to) = match (
            keys.get(link.from_object as usize),
            keys.get(link.to_object as usize),
        ) {
            (Some(f), Some(t)) => (f.clone(), t.clone()),
            _ => continue,
        };
        let label = ctx.simple_name(link.via_field_sym);
        model.add_edge(
            ViewEdge::new(&from, &to, EdgeKind::Link, &link.via_field_sym.to_string())
                .with_label(label.clone())
                .with_role(label),
        );
    }
    if ctx.uma.object_links.is_empty() {
        model.note("No field points-to links between allocation sites were recorded.");
    }
    model.note("Each object is one allocation site; runtime instance counts are not modelled.");
    apply_focus(ctx, &mut model);
    model
}

// ── 3. PACKAGE DIAGRAM ───────────────────────────────────────────────────────

/// Upper bound on package parent chains followed through UMA package records.
const MAX_PACKAGE_DEPTH: usize = 64;

/// Fully qualified name of a package record: the canonical Phase 3 name, else the segment
/// names of the record's parent chain joined from the root. A lone segment name is never
/// used for a nested package, so `a.b.c` yields one tree with one root.
fn package_name(
    ctx: &DiagramContext,
    pkg: &PackageRecord,
    by_sym: &HashMap<u32, &PackageRecord>,
) -> String {
    if let Some(name) = ctx.package_name(pkg.package_sym_id) {
        return name;
    }
    let mut segments: Vec<String> = Vec::new();
    let mut curr = Some(pkg);
    let mut seen: HashSet<u32> = HashSet::new();
    while let Some(p) = curr {
        if !seen.insert(p.package_sym_id) || segments.len() >= MAX_PACKAGE_DEPTH {
            break;
        }
        if let Some(full) = ctx.package_name(p.package_sym_id) {
            segments.push(full);
            break;
        }
        let segment = ctx
            .text(p.name_id)
            .or_else(|| ctx.simple_name(p.package_sym_id))
            .unwrap_or_else(|| format!("<package #{}>", p.package_sym_id));
        segments.push(segment);
        curr = if p.parent_package_sym == UNKNOWN_SYM {
            None
        } else {
            by_sym.get(&p.parent_package_sym).copied()
        };
    }
    segments.reverse();
    segments.join(".")
}

pub fn package_view(ctx: &DiagramContext) -> ViewModel {
    let mut model = ViewModel::new(ViewKind::Package);
    model.source("uma");
    let mut aliases = AliasTable::default();

    if ctx.uma.packages.is_empty() {
        let types = TypeIndex::build(ctx);
        let packages: BTreeSet<String> = types
            .syms
            .iter()
            .filter_map(|&s| types.package(s).map(str::to_string))
            .collect();
        if packages.is_empty() {
            return ViewModel::unavailable(
                ViewKind::Package,
                "No package declarations were found in the analyzed sources.",
            );
        }
        for pkg in &packages {
            ensure_package_chain(&mut model, &mut aliases, pkg);
        }
        model.note(
            "Phase 9 recorded no package records; packages are taken from the declared types and no dependencies are shown.",
        );
        return model;
    }

    let by_sym: HashMap<u32, &PackageRecord> = ctx
        .uma
        .packages
        .iter()
        .map(|p| (p.package_sym_id, p))
        .collect();
    let mut named: Vec<(String, &PackageRecord)> = ctx
        .uma
        .packages
        .iter()
        .map(|p| (package_name(ctx, p, &by_sym), p))
        .collect();
    named.sort_by(|a, b| a.0.cmp(&b.0));

    let mut key_by_sym: HashMap<u32, String> = HashMap::new();
    for (fqn, pkg) in &named {
        if let Some(key) = ensure_package_chain(&mut model, &mut aliases, fqn) {
            if let Some(node) = model.node_mut(&key) {
                node.sym_id = Some(pkg.package_sym_id);
                node.details.push(format!(
                    "{} type(s), {} subpackage(s)",
                    pkg.class_count, pkg.subpackage_count
                ));
            }
            key_by_sym.insert(pkg.package_sym_id, key);
        }
    }

    let mut cycles: BTreeSet<u32> = BTreeSet::new();
    for m in &ctx.uma.package_metrics {
        let key = match key_by_sym.get(&m.package_sym_id) {
            Some(k) => k.clone(),
            None => continue,
        };
        if let Some(node) = model.node_mut(&key) {
            node.details.push(format!(
                "Ca={} Ce={} I={} A={} D={}",
                m.afferent,
                m.efferent,
                fmt_metric(m.instability),
                fmt_metric(m.abstractness),
                fmt_metric(m.distance)
            ));
            if m.cycle_id != u32::MAX {
                cycles.insert(m.cycle_id);
                let stereotype = format!("cycle {}", m.cycle_id);
                if !node.stereotypes.contains(&stereotype) {
                    node.stereotypes.push(stereotype);
                }
            }
        }
    }

    for dep in &ctx.uma.package_dependencies {
        let (src, dst) = match (
            key_by_sym.get(&dep.from_package_sym),
            key_by_sym.get(&dep.to_package_sym),
        ) {
            (Some(s), Some(d)) if s != d => (s.clone(), d.clone()),
            _ => continue,
        };
        let mut label = format!("«use» ×{}", dep.reference_count);
        if dep.in_cycle != 0 {
            label.push_str(" (cycle)");
        }
        model.add_edge(
            ViewEdge::new(&src, &dst, EdgeKind::Dependency, "")
                .with_label(Some(label))
                .with_property("references", dep.reference_count.to_string()),
        );
    }

    if !cycles.is_empty() {
        model.note(format!(
            "{} package dependency cycle(s) detected (Tarjan SCC); packages in a cycle carry a «cycle N» stereotype.",
            cycles.len()
        ));
    }
    if !ctx.uma.package_dependencies.is_empty() {
        model.note("A «use» dependency P → Q aggregates every reference from a type of P to a type of Q (inheritance, attributes, parameters, results, instantiations, calls); ×N is the reference count.");
    }
    if !ctx.uma.package_metrics.is_empty() {
        model.note("Metrics (Martin): Ca/Ce afferent/efferent coupling, I instability, A abstractness, D distance from the main sequence.");
    }
    apply_focus(ctx, &mut model);
    model
}

// ── 4. COMPONENT DIAGRAM ─────────────────────────────────────────────────────

pub fn component_view(ctx: &DiagramContext) -> ViewModel {
    if ctx.uma.components.is_empty() {
        return ViewModel::unavailable(
            ViewKind::Component,
            "Phase 9 recorded no components (packages or dependency-injection beans).",
        );
    }
    let types = TypeIndex::build(ctx);
    let mut model = ViewModel::new(ViewKind::Component);
    model.source("uma");
    let mut aliases = AliasTable::default();

    let mut named: Vec<(String, &ComponentRecord)> = ctx
        .uma
        .components
        .iter()
        .map(|c| {
            let name = ctx
                .qualified_name(c.component_sym_id)
                .or_else(|| ctx.text(c.name_id))
                .unwrap_or_else(|| format!("<component #{}>", c.component_sym_id));
            (name, c)
        })
        .collect();
    named.sort_by(|a, b| a.0.cmp(&b.0));

    for (name, comp) in &named {
        let mut key = format!("comp:{}", name);
        if model.has_node(&key) {
            key = format!("comp:{}#{}", name, comp.component_sym_id);
        }
        let label = ctx
            .display_name(comp.component_sym_id)
            .unwrap_or_else(|| name.clone());
        let alias = aliases.alias("comp_", name, &key);
        let stereotype = if comp.component_kind == COMPONENT_KIND_BEAN {
            "bean"
        } else {
            "package"
        };
        model.add_node(
            ViewNode::new(key.clone(), alias, label, NodeKind::Component)
                .with_qualified_name(Some(name.clone()))
                .with_stereotype(stereotype)
                .with_span(ctx.symbol_span(comp.component_sym_id))
                .with_sym(comp.component_sym_id),
        );
        for (&iface, edge_kind) in comp
            .provided_interfaces
            .iter()
            .map(|i| (i, EdgeKind::Provides))
            .chain(comp.required_interfaces.iter().map(|i| (i, EdgeKind::Requires)))
        {
            let iface_name = types
                .fqn(iface)
                .map(str::to_string)
                .or_else(|| ctx.type_fqn(iface))
                .unwrap_or_else(|| format!("<interface #{}>", iface));
            let iface_key = format!("iface:{}", iface_name);
            if !model.has_node(&iface_key) {
                let iface_alias = aliases.alias("iface_", &iface_name, &iface_key);
                let iface_label = ctx
                    .nested_type_name(iface)
                    .unwrap_or_else(|| iface_name.clone());
                model.add_node(
                    ViewNode::new(
                        iface_key.clone(),
                        iface_alias,
                        iface_label,
                        NodeKind::ProvidedInterface,
                    )
                    .with_qualified_name(Some(iface_name.clone()))
                    .with_span(ctx.symbol_span(iface))
                    .with_sym(iface),
                );
            }
            let label = if edge_kind == EdgeKind::Requires {
                Some("«use»".to_string())
            } else {
                None
            };
            model.add_edge(ViewEdge::new(&key, &iface_key, edge_kind, "").with_label(label));
        }
    }
    model.note("Package components group the types of one package; bean components are classes annotated as framework components. Provided interfaces are realized by the component, required ones are used by it and declared elsewhere.");
    apply_focus(ctx, &mut model);
    model
}

// ── 5. COMPOSITE STRUCTURE DIAGRAM ───────────────────────────────────────────

pub fn composite_view(ctx: &DiagramContext) -> ViewModel {
    if ctx.uma.composite_structures.is_empty() {
        return ViewModel::unavailable(
            ViewKind::CompositeStructure,
            "Phase 9 recorded no composite structures (classes owning parts).",
        );
    }
    let types = TypeIndex::build(ctx);
    let mut model = ViewModel::new(ViewKind::CompositeStructure);
    model.source("uma");
    let mut aliases = AliasTable::default();

    let mut ordered: Vec<(&str, &CompositeStructureRecord)> = ctx
        .uma
        .composite_structures
        .iter()
        .filter_map(|cs| types.key(cs.class_sym_id).map(|k| (k, cs)))
        .collect();
    ordered.sort_by(|a, b| a.0.cmp(b.0));

    for (class_key, cs) in ordered {
        let owner_key = format!("sc:{}", class_key);
        if model.has_node(&owner_key) {
            continue;
        }
        let owner_alias = aliases.alias("sc_", class_key, &owner_key);
        let owner_label = ctx
            .nested_type_name(cs.class_sym_id)
            .unwrap_or_else(|| class_key.to_string());
        model.add_node(
            ViewNode::new(
                owner_key.clone(),
                owner_alias,
                owner_label,
                NodeKind::StructuredClass,
            )
            .with_qualified_name(types.fqn(cs.class_sym_id).map(str::to_string))
            .with_span(ctx.symbol_span(cs.class_sym_id))
            .with_sym(cs.class_sym_id),
        );

        let mut part_keys: Vec<String> = Vec::with_capacity(cs.parts.len());
        for (idx, part) in cs.parts.iter().enumerate() {
            let type_name = type_display(ctx, part.type_sym_id).unwrap_or_else(|| "?".to_string());
            let label = if part.is_nested_type != 0 {
                format!("{} (nested type)", type_name)
            } else {
                let role = ctx
                    .simple_name(part.part_sym)
                    .unwrap_or_else(|| format!("part{}", idx));
                match multiplicity(part.mult_min, part.mult_max) {
                    Some(m) => format!("{} : {} [{}]", role, type_name, m),
                    None => format!("{} : {}", role, type_name),
                }
            };
            let key = format!("part:{}:{}", class_key, part.part_sym);
            let alias = aliases.alias("part_", &format!("{}_{}", class_key, label), &key);
            model.add_node(
                ViewNode::new(key.clone(), alias, label, NodeKind::Part)
                    .with_parent(Some(owner_key.clone()))
                    .with_qualified_name(ctx.qualified_name(part.part_sym))
                    .with_span(ctx.symbol_span(part.part_sym))
                    .with_sym(part.part_sym),
            );
            part_keys.push(key);
        }

        let resolve = |part_ref: u32| -> Option<String> {
            cs.parts
                .iter()
                .position(|p| p.part_sym == part_ref)
                .or_else(|| {
                    if (part_ref as usize) < cs.parts.len() {
                        Some(part_ref as usize)
                    } else {
                        None
                    }
                })
                .and_then(|i| part_keys.get(i).cloned())
        };
        for connector in &cs.connectors {
            let (from, to) = match (resolve(connector.from_part), resolve(connector.to_part)) {
                (Some(f), Some(t)) => (f, t),
                _ => continue,
            };
            let label = ctx
                .simple_name(connector.via_method_sym)
                .map(|m| format!("via {}()", m));
            model.add_edge(
                ViewEdge::new(
                    &from,
                    &to,
                    EdgeKind::Connector,
                    &connector.via_method_sym.to_string(),
                )
                .with_label(label),
            );
        }
    }
    apply_focus(ctx, &mut model);
    model
}

// ── 6. DEPLOYMENT DIAGRAM ────────────────────────────────────────────────────

pub fn deployment_view(_ctx: &DiagramContext) -> ViewModel {
    ViewModel::unavailable(
        ViewKind::Deployment,
        "Deployment topology is not derivable from source code: no deployment descriptors (Dockerfile, compose, Kubernetes or Helm manifests) or build modules are part of the analyzed artifacts.",
    )
}

// ── 7. PROFILE DIAGRAM ───────────────────────────────────────────────────────

fn ensure_metaclass(model: &mut ViewModel, name: &str) -> String {
    let key = format!("meta:{}", name);
    if !model.has_node(&key) {
        model.add_node(
            ViewNode::new(
                key.clone(),
                format!("meta_{}", name),
                name,
                NodeKind::Metaclass,
            )
            .with_qualified_name(Some(format!("UML::{}", name))),
        );
    }
    key
}

/// Maximum applying classifiers listed on one stereotype.
const MAX_LISTED_APPLICATIONS: usize = 12;

/// Applications of annotation types: (applying classifier names, unattributed count).
type Applications = (BTreeSet<String>, usize);

/// Classifiers that apply each annotation type in `annotations`: every type-reference
/// occurrence of an annotation type is attributed to the innermost declared classifier whose
/// declaration token range contains it; occurrences outside every classifier are counted.
fn annotation_applications(
    ctx: &DiagramContext,
    types: &TypeIndex,
    annotations: &HashSet<u32>,
) -> HashMap<u32, Applications> {
    let mut result: HashMap<u32, Applications> = HashMap::new();
    if annotations.is_empty() {
        return result;
    }
    let mut ranges: Vec<(u32, u32, u32)> = Vec::new();
    for &sym in &types.syms {
        if let Some(rec) = ctx.symbol(sym) {
            if rec.first_token_id != u32::MAX && rec.last_token_id >= rec.first_token_id {
                ranges.push((rec.first_token_id, rec.last_token_id, sym));
            }
        }
    }
    for occ in &ctx.sta.occurrences {
        if !annotations.contains(&occ.symbol) || occ.role != OccurrenceRole::TypeReference as u8 {
            continue;
        }
        let owner = ranges
            .iter()
            .filter(|&&(first, last, sym)| {
                first <= occ.token && occ.token <= last && sym != occ.symbol
            })
            .min_by_key(|&&(first, last, _)| last - first)
            .map(|&(_, _, sym)| sym);
        let entry = result.entry(occ.symbol).or_default();
        match owner.and_then(|s| ctx.nested_type_name(s)) {
            Some(name) => {
                entry.0.insert(name);
            }
            None => entry.1 += 1,
        }
    }
    result
}

pub fn profile_view(ctx: &DiagramContext) -> ViewModel {
    let types = TypeIndex::build(ctx);
    let mut model = ViewModel::new(ViewKind::Profile);
    model.source("uma");
    model.source("sta");
    let mut aliases = AliasTable::default();
    let annotation_types: HashSet<u32> = types
        .syms
        .iter()
        .copied()
        .filter(|&s| {
            ctx.class_record(s)
                .map(|c| c.stereotype == STEREOTYPE_ANNOTATION)
                .unwrap_or(false)
        })
        .collect();
    let applications = annotation_applications(ctx, &types, &annotation_types);
    let no_applications: Applications = (BTreeSet::new(), 0);

    for &sym in &types.syms {
        let class_rec = match ctx.class_record(sym) {
            Some(c) if c.stereotype == STEREOTYPE_ANNOTATION => c,
            _ => continue,
        };
        let fqn = types.fqn(sym).unwrap_or("").to_string();
        let key = format!("st:{}", types.key(sym).unwrap_or(fqn.as_str()));
        let alias = aliases.alias("st_", &fqn, &key);
        let label = ctx.nested_type_name(sym).unwrap_or_else(|| fqn.clone());
        let mut details = vec!["declared annotation type".to_string()];
        let (applied, unattributed) = applications.get(&sym).unwrap_or(&no_applications);
        let unattributed = *unattributed;
        if !applied.is_empty() {
            let listed: Vec<&str> = applied
                .iter()
                .take(MAX_LISTED_APPLICATIONS)
                .map(|s| s.as_str())
                .collect();
            let mut text = format!("applied in: {}", listed.join(", "));
            if applied.len() > listed.len() {
                text.push_str(&format!(" and {} more", applied.len() - listed.len()));
            }
            details.push(text);
        }
        if unattributed > 0 {
            details.push(format!(
                "{} application(s) outside declared classifiers",
                unattributed
            ));
        }
        model.add_node(
            ViewNode::new(key.clone(), alias, label, NodeKind::Stereotype)
                .with_qualified_name(Some(fqn))
                .with_span(ctx.span_of_link(&class_rec.uml_link).or_else(|| ctx.symbol_span(sym)))
                .with_sym(sym)
                .with_details(details),
        );
        let meta = ensure_metaclass(&mut model, "Element");
        model.add_edge(
            ViewEdge::new(&key, &meta, EdgeKind::Extension, "")
                .with_label(Some("«extension»".to_string())),
        );
    }

    let mut by_kind: BTreeMap<u8, (usize, u8, u8)> = BTreeMap::new();
    for inst in &ctx.uma.pattern_instances {
        let entry = by_kind
            .entry(inst.pattern_kind)
            .or_insert((0, u8::MAX, 0));
        entry.0 += 1;
        entry.1 = entry.1.min(inst.score);
        entry.2 = entry.2.max(inst.score);
    }
    for (kind, (count, min_score, max_score)) in by_kind {
        let name = pattern_kind_name(kind);
        if name.is_empty() {
            continue;
        }
        let key = format!("st:pattern:{}", name);
        let mut details = vec![format!("{} detected instance(s)", count)];
        details.push(if min_score == max_score {
            format!("evidence score {}%", max_score)
        } else {
            format!("evidence score {}–{}%", min_score, max_score)
        });
        model.add_node(
            ViewNode::new(key.clone(), format!("st_pattern_{}", name), name, NodeKind::Stereotype)
                .with_qualified_name(Some(format!("GoF::{}", name)))
                .with_details(details)
                .with_provenance(Provenance::Heuristic),
        );
        let meta = ensure_metaclass(&mut model, "Class");
        model.add_edge(
            ViewEdge::new(&key, &meta, EdgeKind::Extension, "")
                .with_label(Some("«extension»".to_string()))
                .with_provenance(Provenance::Heuristic),
        );
    }

    if model.nodes.is_empty() {
        return ViewModel::unavailable(
            ViewKind::Profile,
            "The analyzed sources declare no annotation types and Phase 9 detected no design-pattern instances, so no stereotypes are applied.",
        );
    }
    model.note("Annotation types extend the root metaclass Element because their @Target is not recorded; each application is attributed to the innermost declared classifier containing the annotation. Pattern stereotypes come from Phase 9 structural-evidence detection (scores are uncalibrated).");
    settle_provenance(&mut model);
    model
}
