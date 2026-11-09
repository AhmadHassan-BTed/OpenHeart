//! Object diagram: the abstract objects of the solved heap and the links between them.
//!
//! An object is an allocation point of the abstract heap (`HeapBase`) qualified by its
//! heap context; one abstract object may stand for several runtime objects. A link is an
//! object slot (field, array element, pointee, closure capture) whose cell may hold another
//! object: may-point-to facts, hence over-approximate. A slot value is shown when every
//! store into the field writes a constant. Static fields and module variables that hold
//! objects appear as roots.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::derive::model::{
    CompartmentEntry, CompartmentKind, Derivation, DiagramEdge, DiagramKind, DiagramModel,
    DiagramNode, EdgeEnd, EdgeKind, ElementId, IdBuilder, NodeKind,
};
use crate::derive::provenance::{const_text, object_span, Fact, ValueRenderer};
use crate::derive::{cap, DeriveContext, HeapFacts};
use crate::ir::inst::Inst;
use crate::ir::types::FieldId;
use crate::ir::value::{Operand, Place};
use crate::memory::location::{Cell, CellId, Context, HeapBase, LocationTable, ObjId, Slot};

/// Values every store into a field writes: `Some(constants)` when all are constants,
/// `None` when some store writes a computed value.
pub fn field_constants(ctx: &DeriveContext) -> HashMap<FieldId, Option<BTreeSet<String>>> {
    let mut out: HashMap<FieldId, Option<BTreeSet<String>>> = HashMap::new();
    for func in ctx.funcs() {
        let r = ValueRenderer::new(func);
        for block in &func.blocks {
            for inst in &block.insts {
                let Inst::Store { place, value } = inst else {
                    continue;
                };
                let field = match place {
                    Place::Field { field, .. } | Place::Static { field, .. } => *field,
                    _ => continue,
                };
                let constant = match value {
                    Operand::Const(c) => Some(const_text(ctx, c)),
                    Operand::Value(v) => match r.def(r.resolve_copy(*v)) {
                        Some(Inst::Assign {
                            src: Operand::Const(c),
                            ..
                        }) => Some(const_text(ctx, c)),
                        _ => None,
                    },
                    Operand::Undef => None,
                };
                let slot = out.entry(field).or_insert_with(|| Some(BTreeSet::new()));
                match constant {
                    Some(c) => {
                        if let Some(set) = slot.as_mut() {
                            set.insert(c);
                        }
                    }
                    None => *slot = None,
                }
            }
        }
    }
    out
}

/// Stable text identity of a heap base (function and span of the allocation point).
pub(crate) fn base_key(ctx: &DeriveContext, loc: &LocationTable, base: HeapBase) -> String {
    match base {
        HeapBase::Site(s) => match ctx.alloc_site(s) {
            Some(a) => format!(
                "alloc {} {}",
                ctx.func(a.func).map(|f| ctx.func_qualified_name(f)).unwrap_or_default(),
                a.span
            ),
            None => format!("alloc {}", s),
        },
        HeapBase::Closure(c) => {
            let site = loc.closure_site(c);
            format!(
                "closure {} {}:{}",
                ctx.func(site.func).map(|f| ctx.func_qualified_name(f)).unwrap_or_default(),
                site.block,
                site.inst
            )
        }
        HeapBase::Intrinsic(i) => {
            let site = loc.intrinsic_site(i);
            format!(
                "intrinsic {} {}:{}",
                ctx.func(site.func).map(|f| ctx.func_qualified_name(f)).unwrap_or_default(),
                site.block,
                site.inst
            )
        }
        HeapBase::EntryParam { func, index } => format!(
            "param {} of {}",
            index,
            ctx.func(func).map(|f| ctx.func_qualified_name(f)).unwrap_or_default()
        ),
        HeapBase::StringLiteral => "string literals".to_string(),
        HeapBase::Unknown => "external".to_string(),
    }
}

/// Readable text of a heap context.
pub(crate) fn context_label(ctx: &DeriveContext, loc: &LocationTable, context: Context) -> Option<String> {
    match context {
        Context::Empty => None,
        Context::CallSite(cs) => Some(match ctx.call_site(cs) {
            Some(site) => format!(
                "called at {} in {}",
                site.span,
                ctx.func(site.func).map(|f| ctx.func_qualified_name(f)).unwrap_or_default()
            ),
            None => format!("called at {}", cs),
        }),
        Context::Receiver(b) => Some(format!("receiver {}", base_key(ctx, loc, b))),
    }
}

/// Stable id of an abstract object.
pub(crate) fn object_id(ctx: &DeriveContext, loc: &LocationTable, obj: ObjId) -> ElementId {
    let info = loc.object(obj);
    IdBuilder::new("obj")
        .text(&base_key(ctx, loc, info.object.base))
        .text(&context_label(ctx, loc, info.object.context).unwrap_or_default())
        .finish()
}

/// Type name of an abstract object.
pub(crate) fn object_type(ctx: &DeriveContext, loc: &LocationTable, obj: ObjId) -> Option<String> {
    let info = loc.object(obj);
    if let Some(sym) = info.dynamic_type {
        return Some(ctx.symbol_name(sym));
    }
    info.ty.map(|t| ctx.render_type(t))
}

/// UML instance label `name : Type` of an abstract object (the name is the variable the
/// allocation is assigned to, when the IR records one).
pub(crate) fn object_label(ctx: &DeriveContext, loc: &LocationTable, obj: ObjId) -> String {
    let info = loc.object(obj);
    let ty = object_type(ctx, loc, obj);
    let name: Option<String> = match info.object.base {
        HeapBase::Site(s) => ctx.alloc_pos(s).and_then(|pos| {
            let func = ctx.func(pos.func)?;
            match ctx.inst_at(pos)? {
                Inst::Alloc { dst, .. } => ctx.value_name(func, *dst),
                _ => None,
            }
        }),
        HeapBase::EntryParam { func, index } => ctx.func(func).and_then(|f| {
            let v = *f.params.get(index as usize)?;
            ctx.value_name(f, v)
        }),
        HeapBase::Closure(c) => {
            let site = loc.closure_site(c);
            return format!(
                ": closure {}",
                ctx.func(site.target).map(|f| ctx.func_name(f)).unwrap_or_default()
            );
        }
        HeapBase::Intrinsic(i) => {
            let site = loc.intrinsic_site(i);
            return format!(": {}", ctx.text(site.name));
        }
        HeapBase::StringLiteral => {
            return format!(": {}", ty.unwrap_or_else(|| "string literal".to_string()))
        }
        HeapBase::Unknown => return "external objects".to_string(),
    };
    match (name, ty) {
        (Some(n), Some(t)) => format!("{} : {}", n, t),
        (Some(n), None) => n,
        (None, Some(t)) => format!(": {}", t),
        (None, None) => ": ?".to_string(),
    }
}

fn slot_name(ctx: &DeriveContext, slot: Slot) -> String {
    match slot {
        Slot::Field(f) => ctx
            .field(f)
            .map(|fi| ctx.text(fi.name))
            .unwrap_or_else(|| format!("{}", f)),
        Slot::Element => "[*]".to_string(),
        Slot::Content => "*".to_string(),
        Slot::Capture(i) => format!("capture {}", i),
    }
}

fn targets(heap: &dyn HeapFacts, cell: CellId) -> Vec<ObjId> {
    let mut t = heap.cell_points_to(cell);
    t.sort_unstable();
    t.dedup();
    t
}

/// The object diagram of the solved heap.
pub fn derive(ctx: &DeriveContext) -> DiagramModel {
    let kind = DiagramKind::Object;
    let Some(heap) = ctx.heap else {
        return DiagramModel::not_derivable(
            kind,
            kind.title(),
            "points-to facts are unavailable: objects are the allocation points of the abstract heap",
        );
    };
    let loc = heap.locations();
    let mut objects: Vec<ObjId> = loc.objects().map(|(id, _)| id).collect();
    objects.sort_unstable();
    if objects.is_empty() {
        return DiagramModel::not_derivable(kind, kind.title(), "the abstract heap holds no object");
    }
    let mut model = DiagramModel::new(kind, kind.title());
    let cut = cap(&mut objects, ctx.limits.max_objects);
    if cut > 0 {
        model.weaken(&Derivation::partial(format!(
            "{} abstract object(s) beyond the limit of {} are not shown",
            cut, ctx.limits.max_objects
        )));
    }
    let kept: BTreeSet<ObjId> = objects.iter().copied().collect();
    let constants = field_constants(ctx);
    let ids: BTreeMap<ObjId, ElementId> = objects
        .iter()
        .map(|&o| (o, object_id(ctx, loc, o)))
        .collect();

    for &obj in &objects {
        let info = loc.object(obj);
        let id = ids[&obj].clone();
        let mut node = DiagramNode::new(id.clone(), NodeKind::Object, object_label(ctx, loc, obj))
            .with_span(object_span(ctx, obj))
            .with_fact(Fact::Object(obj.0));
        if let HeapBase::Site(s) = info.object.base {
            node = node.with_fact(Fact::AllocSite(s.raw()));
        }
        if let Some(sym) = info.dynamic_type {
            node = node.with_symbol(sym);
        }
        match info.object.base {
            HeapBase::EntryParam { .. } => node = node.with_stereotype("parameter"),
            HeapBase::Unknown => node = node.with_stereotype("external"),
            HeapBase::StringLiteral => node = node.with_stereotype("literal"),
            HeapBase::Closure(_) => node = node.with_stereotype("closure"),
            HeapBase::Intrinsic(_) => node = node.with_stereotype("intrinsic"),
            HeapBase::Site(_) => {}
        }
        if info.object.base.is_opaque() {
            node = node.with_derivation(Derivation::partial(
                "the program text does not show how parameter and external objects are built",
            ));
        }
        if let Some(c) = context_label(ctx, loc, info.object.context) {
            node = node.with_property("context", c);
        }
        if let Some(region) = info.region {
            node = node.with_property("region", region.mnemonic());
        }
        if let Some(f) = info.func.and_then(|f| ctx.func(f)) {
            node = node.with_property("allocated in", ctx.func_qualified_name(f));
        }

        // Slots with links, then constant-valued fields without links.
        let mut slots: Vec<CompartmentEntry> = Vec::new();
        let mut linked_fields: BTreeSet<FieldId> = BTreeSet::new();
        for &cell in loc.cells_of(obj) {
            let Cell::Object { slot, .. } = loc.cell(cell) else {
                continue;
            };
            let pointees = targets(heap, cell);
            if pointees.is_empty() {
                continue;
            }
            if let Slot::Field(f) = slot {
                linked_fields.insert(f);
            }
            let role = slot_name(ctx, slot);
            for t in pointees {
                if !kept.contains(&t) {
                    continue;
                }
                model.add_edge(
                    DiagramEdge::new(
                        IdBuilder::new("link")
                            .text(id.as_str())
                            .text(&role)
                            .text(ids[&t].as_str())
                            .finish(),
                        EdgeKind::Link,
                        id.clone(),
                        ids[&t].clone(),
                    )
                    .with_label(role.clone())
                    .with_ends(
                        None,
                        Some(EdgeEnd {
                            role: Some(role.clone()),
                            navigable: true,
                            ..EdgeEnd::default()
                        }),
                    )
                    .with_fact(Fact::CellPointsTo { cell: cell.0 })
                    .with_derivation(Derivation::OverApproximate),
                );
            }
        }
        let mut type_fields: Vec<FieldId> = Vec::new();
        if let Some(sym) = info.dynamic_type {
            type_fields.extend_from_slice(ctx.fields_of_owner(sym));
            for sup in ctx.supertype_closure(sym) {
                type_fields.extend_from_slice(ctx.fields_of_owner(sup));
            }
        }
        type_fields.sort_unstable();
        type_fields.dedup();
        for f in type_fields {
            if linked_fields.contains(&f) {
                continue;
            }
            let Some(fi) = ctx.field(f) else {
                continue;
            };
            if fi.is_static {
                continue;
            }
            if let Some(Some(values)) = constants.get(&f) {
                if values.is_empty() {
                    continue;
                }
                let mut entry = CompartmentEntry::named(ctx.text(fi.name));
                entry.type_text = Some(ctx.render_type(fi.ty));
                entry.value = Some(values.iter().cloned().collect::<Vec<_>>().join(" | "));
                entry.symbol = fi.symbol;
                if values.len() > 1 {
                    entry.derivation = Derivation::OverApproximate;
                }
                slots.push(entry);
            }
        }
        node = node.with_compartment(CompartmentKind::Slots, slots);
        model.add_node(node);
    }

    // Roots: module variables and static fields holding objects.
    for (cell_id, cell) in loc.cells() {
        let (label, stereotype, fact) = match cell {
            Cell::Global(g) => (
                ctx.global(g)
                    .map(|gi| ctx.text(gi.name))
                    .unwrap_or_else(|| format!("{}", g)),
                "global",
                Fact::CellPointsTo { cell: cell_id.0 },
            ),
            Cell::Static { owner, field } => (
                format!(
                    "{}.{}",
                    ctx.symbol_name(owner),
                    ctx.field(field)
                        .map(|fi| ctx.text(fi.name))
                        .unwrap_or_else(|| format!("{}", field))
                ),
                "static",
                Fact::Field(field.raw()),
            ),
            Cell::Object { .. } => continue,
        };
        let pointees: Vec<ObjId> = targets(heap, cell_id)
            .into_iter()
            .filter(|t| kept.contains(t))
            .collect();
        if pointees.is_empty() {
            continue;
        }
        let root = IdBuilder::new("root").text(stereotype).text(&label).finish();
        model.add_node(
            DiagramNode::new(root.clone(), NodeKind::Root, label.clone())
                .with_stereotype(stereotype)
                .with_fact(fact),
        );
        for t in pointees {
            model.add_edge(
                DiagramEdge::new(
                    IdBuilder::new("link").text(root.as_str()).text(ids[&t].as_str()).finish(),
                    EdgeKind::Link,
                    root.clone(),
                    ids[&t].clone(),
                )
                .with_fact(Fact::CellPointsTo { cell: cell_id.0 })
                .with_derivation(Derivation::OverApproximate),
            );
        }
    }
    model.note("each object is an abstract object (allocation point × heap context) and may stand for several runtime objects; links are may-point-to facts");
    model.finalize(ctx.limits.max_spans);
    model
}
