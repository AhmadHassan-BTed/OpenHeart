//! Class diagram derived from declarations, the IR's memory layout and the IR's uses.
//!
//! * Classifiers, attributes, operations and signatures come from the frontends'
//!   declarations; attribute types and multiplicities from the IR types of the fields.
//! * Generalization and realization come from the declared supertypes.
//! * Composition, aggregation and association come from how a field stores what it refers
//!   to: a value stored inline or through an owning reference is a part of its holder
//!   (composition); for shared and raw references the ownership facts of the memory model
//!   decide (exclusive ownership: composition, shared ownership: aggregation, none:
//!   association); borrowed and weak references are associations. Container element types,
//!   identified by the frontend's container shapes, give multiplicities and qualifiers.
//! * Usage dependencies come from the IR: allocations («instantiate»), calls («call»),
//!   field and static accesses, casts and type tests («use»), and operation signatures.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::derive::model::{
    CompartmentEntry, CompartmentKind, Derivation, DiagramEdge, DiagramKind, DiagramModel,
    DiagramNode, EdgeEnd, EdgeKind, ElementId, EntryParam, IdBuilder, Multiplicity, NodeKind,
};
use crate::derive::provenance::{inst_span, symbol_span, Fact};
use crate::derive::{DeriveContext, Modifiers, SuperRelation, SymbolCategory, SymbolFact};
use crate::ir::func::FuncIr;
use crate::ir::inst::{Callee, Inst};
use crate::ir::types::{FieldId, IrType, RefKind, Span, TypeId};
use crate::ir::value::Place;

// ── IR uses of types ─────────────────────────────────────────────────────────

/// How a function uses another type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum UseKind {
    Instantiate,
    Call,
    FieldAccess,
    StaticAccess,
    TypeCheck,
    Signature,
}

impl UseKind {
    /// UML standard stereotype of the usage dependency.
    pub fn stereotype(self) -> &'static str {
        match self {
            UseKind::Instantiate => "instantiate",
            UseKind::Call => "call",
            UseKind::FieldAccess
            | UseKind::StaticAccess
            | UseKind::TypeCheck
            | UseKind::Signature => "use",
        }
    }
}

/// One use of a classifier by a function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeUse {
    pub target: u32,
    pub kind: UseKind,
    pub span: Option<Span>,
    pub fact: Fact,
    /// The use follows from a statically named entity (no class-hierarchy fallback).
    pub exact: bool,
}

/// Every classifier a function's IR uses, in instruction order.
pub fn ir_type_uses(ctx: &DeriveContext, func: &FuncIr) -> Vec<TypeUse> {
    let mut out = Vec::new();
    let fid = func.id.raw();
    for (b, block) in func.blocks.iter().enumerate() {
        for (k, inst) in block.insts.iter().enumerate() {
            let span = inst_span(block, k);
            let fact = Fact::Inst {
                func: fid,
                block: b as u32,
                index: k as u32,
            };
            match inst {
                Inst::Alloc { ty, .. } => {
                    if let Some(sym) = ctx.named_symbol(*ty).filter(|s| ctx.is_classifier(*s)) {
                        out.push(TypeUse {
                            target: sym,
                            kind: UseKind::Instantiate,
                            span,
                            fact,
                            exact: true,
                        });
                    }
                }
                Inst::Call { site, callee, .. } => {
                    let mut owners: Vec<(u32, bool)> = Vec::new();
                    for &t in ctx.callees(*site) {
                        if let Some(o) = ctx.func(t).and_then(|tf| ctx.owner_type(tf)) {
                            owners.push((o, ctx.edge_is_exact(*site, t)));
                        }
                    }
                    if owners.is_empty() {
                        let declared = match callee {
                            Callee::Static(f) => ctx.func(*f).and_then(|tf| ctx.owner_type(tf)),
                            other => other
                                .method_symbol()
                                .and_then(|m| ctx.symbol(m))
                                .and_then(|m| m.parent)
                                .and_then(|p| ctx.classifier_of(p)),
                        };
                        if let Some(o) = declared {
                            owners.push((o, true));
                        }
                    }
                    owners.sort_unstable();
                    owners.dedup_by_key(|o| o.0);
                    for (o, exact) in owners {
                        out.push(TypeUse {
                            target: o,
                            kind: UseKind::Call,
                            span,
                            fact: Fact::CallSite(site.raw()),
                            exact,
                        });
                    }
                }
                Inst::Load { place, .. } | Inst::Store { place, .. } => match place {
                    Place::Field { field, .. } => {
                        if let Some(owner) = ctx
                            .field(*field)
                            .map(|f| f.owner_symbol)
                            .filter(|&o| o != u32::MAX)
                            .and_then(|o| ctx.classifier_of(o))
                        {
                            out.push(TypeUse {
                                target: owner,
                                kind: UseKind::FieldAccess,
                                span,
                                fact: Fact::Field(field.raw()),
                                exact: true,
                            });
                        }
                    }
                    Place::Static { owner, field } => {
                        if let Some(o) = ctx.classifier_of(*owner) {
                            out.push(TypeUse {
                                target: o,
                                kind: UseKind::StaticAccess,
                                span,
                                fact: Fact::Field(field.raw()),
                                exact: true,
                            });
                        }
                    }
                    _ => {}
                },
                Inst::Cast { ty, .. } | Inst::TypeTest { ty, .. } => {
                    for sym in ctx.classifiers_in_type(*ty) {
                        out.push(TypeUse {
                            target: sym,
                            kind: UseKind::TypeCheck,
                            span,
                            fact,
                            exact: true,
                        });
                    }
                }
                _ => {}
            }
        }
    }
    out
}

// ── Field storage ────────────────────────────────────────────────────────────

/// How a field holds what it refers to, from the IR type nearest to the target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Storage {
    /// The value is laid out inside the holder.
    Inline,
    /// Unique ownership.
    Owning,
    /// Shared ownership or a garbage-collected reference.
    Shared,
    Weak,
    Borrowed,
    /// Raw address.
    Raw,
}

impl Storage {
    pub fn mnemonic(self) -> &'static str {
        match self {
            Storage::Inline => "inline",
            Storage::Owning => "owning",
            Storage::Shared => "shared",
            Storage::Weak => "weak",
            Storage::Borrowed => "borrowed",
            Storage::Raw => "raw",
        }
    }
}

/// A classifier a field refers to, with how many and how.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldPart {
    pub target: u32,
    pub multiplicity: Multiplicity,
    pub storage: Storage,
    /// Key types of a map-like container (qualified association).
    pub qualifier: Option<String>,
    /// The field refers to the target through a single reference (no array or container).
    pub direct: bool,
    /// Container type the target is an element of.
    pub container: Option<u32>,
}

/// Classifiers a field of type `ty` refers to.
pub fn field_parts(ctx: &DeriveContext, ty: TypeId) -> Vec<FieldPart> {
    let mut out = Vec::new();
    walk_part(
        ctx,
        ty,
        PartState {
            multiplicity: Multiplicity::ONE,
            storage: Storage::Inline,
            qualifier: None,
            direct: true,
            container: None,
        },
        0,
        &mut out,
    );
    out
}

#[derive(Clone)]
struct PartState {
    multiplicity: Multiplicity,
    storage: Storage,
    qualifier: Option<String>,
    direct: bool,
    container: Option<u32>,
}

fn walk_part(ctx: &DeriveContext, ty: TypeId, st: PartState, depth: usize, out: &mut Vec<FieldPart>) {
    if depth > 12 {
        return;
    }
    match ctx.ir_type(ty) {
        Some(IrType::Named(sym)) => {
            if ctx.is_classifier(*sym) {
                out.push(FieldPart {
                    target: *sym,
                    multiplicity: st.multiplicity,
                    storage: st.storage,
                    qualifier: st.qualifier,
                    direct: st.direct,
                    container: st.container,
                });
            }
        }
        Some(IrType::Ptr(t)) => {
            let next = PartState {
                multiplicity: st.multiplicity.times(Multiplicity::ZERO_OR_ONE),
                storage: Storage::Raw,
                ..st
            };
            walk_part(ctx, *t, next, depth + 1, out);
        }
        Some(IrType::Ref { target, kind }) => {
            let (m, storage) = match kind {
                RefKind::Borrowed => (Multiplicity::ONE, Storage::Borrowed),
                RefKind::Shared => (Multiplicity::ZERO_OR_ONE, Storage::Shared),
                RefKind::Weak => (Multiplicity::ZERO_OR_ONE, Storage::Weak),
                RefKind::Owning => (Multiplicity::ZERO_OR_ONE, Storage::Owning),
            };
            let next = PartState {
                multiplicity: st.multiplicity.times(m),
                storage,
                ..st
            };
            walk_part(ctx, *target, next, depth + 1, out);
        }
        Some(IrType::Array { elem, len }) => {
            let m = match len {
                Some(n) => Multiplicity::exactly(*n),
                None => Multiplicity::MANY,
            };
            let next = PartState {
                multiplicity: st.multiplicity.times(m),
                direct: false,
                ..st
            };
            walk_part(ctx, *elem, next, depth + 1, out);
        }
        Some(IrType::Generic { base, args }) => {
            let shape = ctx
                .named_symbol(*base)
                .and_then(|b| ctx.program.container_shape(b).map(|s| (b, s)));
            match shape {
                Some((b, shape)) => {
                    let qualifier = if shape.key_args.is_empty() {
                        st.qualifier.clone()
                    } else {
                        Some(
                            shape
                                .key_args
                                .iter()
                                .filter_map(|&i| args.get(i))
                                .map(|t| ctx.render_type(*t))
                                .collect::<Vec<_>>()
                                .join(", "),
                        )
                    };
                    for &i in &shape.element_args {
                        if let Some(&a) = args.get(i) {
                            let next = PartState {
                                multiplicity: st.multiplicity.times(shape.multiplicity),
                                storage: st.storage,
                                qualifier: qualifier.clone(),
                                direct: false,
                                container: Some(b),
                            };
                            walk_part(ctx, a, next, depth + 1, out);
                        }
                    }
                }
                None => walk_part(ctx, *base, st, depth + 1, out),
            }
        }
        _ => {}
    }
}

/// Multiplicity of an attribute of non-classifier type (arrays and containers of values).
fn attribute_multiplicity(ctx: &DeriveContext, ty: TypeId) -> Option<Multiplicity> {
    let mut m = Multiplicity::ONE;
    let mut cur = ty;
    for _ in 0..12 {
        match ctx.ir_type(cur)? {
            IrType::Ref { target, kind } => {
                if *kind != RefKind::Borrowed {
                    m = m.times(Multiplicity::ZERO_OR_ONE);
                }
                cur = *target;
            }
            IrType::Ptr(t) => {
                m = m.times(Multiplicity::ZERO_OR_ONE);
                cur = *t;
            }
            IrType::Array { elem, len } => {
                m = m.times(match len {
                    Some(n) => Multiplicity::exactly(*n),
                    None => Multiplicity::MANY,
                });
                cur = *elem;
            }
            IrType::Generic { base, args } => {
                let shape = ctx
                    .named_symbol(*base)
                    .and_then(|b| ctx.program.container_shape(b));
                match shape {
                    Some(shape) => match shape.element_args.first().and_then(|&i| args.get(i)) {
                        Some(&a) => {
                            m = m.times(shape.multiplicity);
                            cur = a;
                        }
                        None => break,
                    },
                    None => break,
                }
            }
            _ => break,
        }
    }
    if m == Multiplicity::ONE {
        None
    } else {
        Some(m)
    }
}

// ── Builder ──────────────────────────────────────────────────────────────────

/// Project classifiers in declaration-tree order, sorted by qualified name.
pub fn project_classifiers(ctx: &DeriveContext) -> Vec<u32> {
    let mut seen = HashSet::new();
    let mut stack: Vec<u32> = ctx.program.top_level();
    stack.reverse();
    let mut found = Vec::new();
    while let Some(sym) = stack.pop() {
        if !seen.insert(sym) {
            continue;
        }
        let Some(fact) = ctx.symbol(sym) else {
            continue;
        };
        if fact.category.is_classifier() && !fact.external {
            found.push(sym);
        }
        if fact.category.is_namespace()
            || fact.category.is_classifier()
            || fact.category == SymbolCategory::File
        {
            let mut members = ctx.program.members(sym);
            members.reverse();
            stack.extend(members);
        }
    }
    found.sort_by_key(|&s| (ctx.qualified_name(s), s));
    found
}

pub(crate) fn classifier_node_kind(category: SymbolCategory) -> (NodeKind, Option<&'static str>) {
    match category {
        SymbolCategory::Interface => (NodeKind::Interface, None),
        SymbolCategory::Trait => (NodeKind::Interface, Some("trait")),
        SymbolCategory::Enum => (NodeKind::Enumeration, None),
        SymbolCategory::Annotation => (NodeKind::Annotation, None),
        SymbolCategory::Struct => (NodeKind::Class, Some("struct")),
        SymbolCategory::Union => (NodeKind::Class, Some("union")),
        SymbolCategory::Record => (NodeKind::Class, Some("record")),
        SymbolCategory::Delegate => (NodeKind::Class, Some("delegate")),
        _ => (NodeKind::Class, None),
    }
}

struct ClassBuilder<'c, 'a> {
    ctx: &'c DeriveContext<'a>,
    model: DiagramModel,
    nodes: HashMap<u32, ElementId>,
    ids: HashMap<ElementId, u32>,
    packages: HashMap<u32, ElementId>,
    related: HashSet<(u32, u32)>,
}

impl<'c, 'a> ClassBuilder<'c, 'a> {
    fn package(&mut self, ns: u32) -> ElementId {
        if let Some(id) = self.packages.get(&ns) {
            return id.clone();
        }
        let ctx = self.ctx;
        let qn = ctx.qualified_name(ns);
        let id = IdBuilder::new("pkg").text(&qn).finish();
        self.packages.insert(ns, id.clone());
        let parent = ctx
            .symbol(ns)
            .and_then(|s| s.parent)
            .and_then(|p| ctx.namespace_of(p))
            .filter(|&p| p != ns)
            .map(|p| self.package(p));
        self.model.add_node(
            DiagramNode::new(id.clone(), NodeKind::Package, ctx.symbol_name(ns))
                .with_qualified_name(qn)
                .with_parent(parent)
                .with_symbol(ns)
                .with_span(symbol_span(ctx, ns))
                .with_fact(Fact::Symbol(ns)),
        );
        id
    }

    fn classifier(&mut self, sym: u32) -> ElementId {
        if let Some(id) = self.nodes.get(&sym) {
            return id.clone();
        }
        let ctx = self.ctx;
        let fact = ctx.symbol(sym);
        let qn = ctx.qualified_name(sym);
        let category = fact.as_ref().map(|f| f.category).unwrap_or(SymbolCategory::Class);
        let mut id = IdBuilder::new("class").text(&qn).text(category.mnemonic()).finish();
        if self.ids.get(&id).map_or(false, |&other| other != sym) {
            id = IdBuilder::new("class")
                .text(&qn)
                .text(category.mnemonic())
                .num(sym as u64)
                .finish();
        }
        self.nodes.insert(sym, id.clone());
        self.ids.insert(id.clone(), sym);
        let parent = fact
            .as_ref()
            .and_then(|f| f.parent)
            .and_then(|p| ctx.namespace_of(p))
            .map(|ns| self.package(ns));
        let (kind, stereotype) = classifier_node_kind(category);
        let mut node = DiagramNode::new(id.clone(), kind, ctx.symbol_name(sym))
            .with_qualified_name(qn)
            .with_parent(parent)
            .with_symbol(sym)
            .with_span(symbol_span(ctx, sym))
            .with_fact(Fact::Symbol(sym));
        if let Some(s) = stereotype {
            node = node.with_stereotype(s);
        }
        match &fact {
            Some(f) => {
                if f.external {
                    node = node.with_property("external", "true");
                }
                if f.modifiers.has(Modifiers::ABSTRACT) && !f.category.is_interface_like() {
                    node = node.with_property("isAbstract", "true");
                }
                if f.modifiers.has(Modifiers::FINAL) || f.modifiers.has(Modifiers::SEALED) {
                    node = node.with_property("isLeaf", "true");
                }
                if !f.type_params.is_empty() {
                    let names: Vec<String> =
                        f.type_params.iter().map(|&t| ctx.symbol_name(t)).collect();
                    node = node.with_property("template parameters", names.join(", "));
                }
                let marker = f.visibility.marker();
                if !marker.is_empty() {
                    node = node.with_property("visibility", marker);
                }
            }
            None => {
                node = node.with_derivation(Derivation::partial(
                    "a referenced type has no declaration facts",
                ));
            }
        }
        self.model.add_node(node);
        id
    }

    fn type_text(&self, explicit: &Option<String>, ty: Option<TypeId>) -> Option<String> {
        explicit
            .clone()
            .or_else(|| ty.map(|t| self.ctx.render_type(t)))
    }

    fn is_void(&self, ty: Option<TypeId>) -> bool {
        matches!(ty.and_then(|t| self.ctx.ir_type(t)), Some(IrType::Void))
    }

    fn field_type(&self, member: &SymbolFact) -> Option<TypeId> {
        self.ctx
            .field_of_symbol(member.id)
            .and_then(|f| self.ctx.field(f))
            .map(|f| f.ty)
            .or(member.ty)
    }

    fn members(&mut self, sym: u32) {
        let ctx = self.ctx;
        let id = self.classifier(sym);
        let mut attributes = Vec::new();
        let mut operations = Vec::new();
        let mut literals = Vec::new();
        let mut declared_fields: BTreeSet<FieldId> = BTreeSet::new();
        for m in ctx.program.members(sym) {
            let Some(f) = ctx.symbol(m) else {
                continue;
            };
            if f.modifiers.has(Modifiers::SYNTHETIC) {
                continue;
            }
            let mut entry = CompartmentEntry::named(f.name.clone());
            entry.visibility = Some(f.visibility);
            entry.symbol = Some(m);
            entry.is_static = f.modifiers.has(Modifiers::STATIC);
            entry.is_abstract = f.modifiers.has(Modifiers::ABSTRACT);
            entry.is_read_only = f.modifiers.has(Modifiers::FINAL)
                || f.modifiers.has(Modifiers::CONST)
                || f.modifiers.has(Modifiers::READ_ONLY);
            if let Some(s) = f.span {
                entry.spans.push(s);
            }
            match f.category {
                c if c.is_attribute() => {
                    let field_id = ctx.field_of_symbol(m);
                    if let Some(fid) = field_id {
                        declared_fields.insert(fid);
                        if ctx.field(fid).map_or(false, |fi| fi.is_static) {
                            entry.is_static = true;
                        }
                    }
                    let ty = self.field_type(&f);
                    let has_parts = ty.map_or(false, |t| !field_parts(ctx, t).is_empty());
                    if has_parts {
                        // Shown as the role of an association end.
                        continue;
                    }
                    entry.type_text = self.type_text(&f.type_text, ty);
                    entry.multiplicity = ty.and_then(|t| attribute_multiplicity(ctx, t));
                    if ty.is_none() {
                        entry.derivation = Derivation::partial("an attribute has no derivable type");
                    }
                    attributes.push(entry);
                }
                SymbolCategory::EnumConstant => {
                    entry.visibility = None;
                    literals.push(entry);
                }
                c if c.is_callable() => {
                    entry.params = Some(
                        f.params
                            .iter()
                            .map(|p| EntryParam {
                                name: p.name.clone(),
                                type_text: self.type_text(&p.type_text, p.ty),
                            })
                            .collect(),
                    );
                    let returns = !matches!(
                        c,
                        SymbolCategory::Constructor | SymbolCategory::Destructor | SymbolCategory::Initializer
                    );
                    if returns && !self.is_void(f.ret) {
                        entry.type_text = self.type_text(&f.ret_text, f.ret);
                    }
                    operations.push(entry);
                }
                _ => {}
            }
        }
        // Fields the IR accesses on this type that no declaration of it binds.
        for &fid in ctx.fields_of_owner(sym) {
            let Some(fi) = ctx.field(fid) else {
                continue;
            };
            if fi.synthetic || fi.symbol.is_some() || declared_fields.contains(&fid) {
                continue;
            }
            let mut entry = CompartmentEntry::named(ctx.text(fi.name));
            entry.type_text = Some(ctx.render_type(fi.ty));
            entry.is_static = fi.is_static;
            entry.derivation = Derivation::partial(
                "a field is accessed in the IR but no declaration in the analysed sources binds it",
            );
            attributes.push(entry);
        }
        if let Some(node) = self.model.node_mut(&id) {
            for (kind, entries) in [
                (CompartmentKind::Literals, literals),
                (CompartmentKind::Attributes, attributes),
                (CompartmentKind::Operations, operations),
            ] {
                if !entries.is_empty() {
                    node.compartments
                        .push(crate::derive::model::Compartment { kind, entries });
                }
            }
        }
    }

    fn hierarchy(&mut self, sym: u32) {
        let ctx = self.ctx;
        let source = self.classifier(sym);
        for sup in ctx.program.supertypes(sym) {
            if sup.target == sym {
                continue;
            }
            let target = self.classifier(sup.target);
            let kind = match sup.relation {
                SuperRelation::Extends => EdgeKind::Generalization,
                SuperRelation::Implements => EdgeKind::Realization,
            };
            self.related.insert((sym, sup.target));
            self.model.add_edge(
                DiagramEdge::new(
                    IdBuilder::new("gen")
                        .text(source.as_str())
                        .text(target.as_str())
                        .text(kind.mnemonic())
                        .finish(),
                    kind,
                    source.clone(),
                    target,
                )
                .with_span(sup.span)
                .with_fact(Fact::Supertype {
                    sub: sym,
                    sup: sup.target,
                }),
            );
        }
    }

    fn nesting(&mut self, sym: u32) {
        let ctx = self.ctx;
        let Some(parent) = ctx.symbol(sym).and_then(|s| s.parent) else {
            return;
        };
        if !ctx.is_classifier(parent) {
            return;
        }
        let inner = self.classifier(sym);
        let outer = self.classifier(parent);
        self.related.insert((sym, parent));
        self.model.add_edge(
            DiagramEdge::new(
                IdBuilder::new("nest").text(inner.as_str()).text(outer.as_str()).finish(),
                EdgeKind::Nesting,
                inner,
                outer,
            )
            .with_fact(Fact::Symbol(sym)),
        );
    }

    fn field_relationships(&mut self, sym: u32) {
        let ctx = self.ctx;
        let source = self.classifier(sym);
        for m in ctx.program.members(sym) {
            let Some(f) = ctx.symbol(m) else {
                continue;
            };
            if !f.category.is_attribute() || f.modifiers.has(Modifiers::SYNTHETIC) {
                continue;
            }
            let Some(ty) = self.field_type(&f) else {
                continue;
            };
            let field_id = ctx.field_of_symbol(m);
            for part in field_parts(ctx, ty) {
                let target = self.classifier(part.target);
                let (kind, derivation, ownership_fact) = relationship(ctx, field_id, &part);
                let mut multiplicity = part.multiplicity;
                let never_null = field_id
                    .and_then(|fid| ctx.fields.and_then(|ff| ff.never_null(fid)))
                    .unwrap_or(false);
                if part.direct && never_null && part.storage != Storage::Inline {
                    multiplicity = multiplicity.with_lower(1);
                }
                let source_end = EdgeEnd {
                    role: None,
                    multiplicity: if part.storage == Storage::Inline && kind == EdgeKind::Composition {
                        Some(Multiplicity::ONE)
                    } else {
                        None
                    },
                    navigable: false,
                    qualifier: None,
                };
                let target_end = EdgeEnd {
                    role: Some(f.name.clone()),
                    multiplicity: Some(multiplicity),
                    navigable: true,
                    qualifier: part.qualifier.clone(),
                };
                let mut edge = DiagramEdge::new(
                    IdBuilder::new("assoc")
                        .text(source.as_str())
                        .text(&f.qualified_name)
                        .text(target.as_str())
                        .finish(),
                    kind,
                    source.clone(),
                    target,
                )
                .with_ends(Some(source_end), Some(target_end))
                .with_property("storage", part.storage.mnemonic())
                .with_span(f.span)
                .with_fact(Fact::Symbol(m))
                .with_derivation(derivation);
                if let Some(fid) = field_id {
                    edge = edge.with_fact(Fact::Field(fid.raw()));
                    if never_null {
                        edge = edge.with_fact(Fact::Nullness { field: fid.raw() });
                    }
                }
                if let Some(fact) = ownership_fact {
                    edge = edge.with_fact(fact);
                }
                if let Some(c) = part.container {
                    edge = edge.with_fact(Fact::Container(c));
                }
                if f.modifiers.has(Modifiers::STATIC) {
                    edge = edge.with_property("isStatic", "true");
                }
                self.related.insert((sym, part.target));
                self.model.add_edge(edge);
            }
        }
    }

    fn dependencies(&mut self, classifiers: &[u32]) {
        let ctx = self.ctx;
        let set: HashSet<u32> = classifiers.iter().copied().collect();
        // (source, target, stereotype) -> (spans, facts, exact, count)
        let mut agg: BTreeMap<(u32, u32, &'static str), (Vec<Span>, Vec<Fact>, bool, usize)> =
            BTreeMap::new();
        let mut record = |source: u32, u: TypeUse| {
            let e = agg
                .entry((source, u.target, u.kind.stereotype()))
                .or_insert_with(|| (Vec::new(), Vec::new(), false, 0));
            if let Some(s) = u.span {
                e.0.push(s);
            }
            e.1.push(u.fact);
            e.2 |= u.exact;
            e.3 += 1;
        };
        for func in ctx.funcs() {
            let Some(owner) = ctx.owner_type(func) else {
                continue;
            };
            let Some(owner) = ctx.classifier_of(owner).filter(|o| set.contains(o)) else {
                continue;
            };
            for u in ir_type_uses(ctx, func) {
                record(owner, u);
            }
        }
        for &c in classifiers {
            for m in ctx.program.members(c) {
                let Some(f) = ctx.symbol(m) else {
                    continue;
                };
                if !f.category.is_callable() {
                    continue;
                }
                let mut types: Vec<TypeId> = f.params.iter().filter_map(|p| p.ty).collect();
                types.extend(f.ret);
                for t in types {
                    for target in ctx.classifiers_in_type(t) {
                        record(
                            c,
                            TypeUse {
                                target,
                                kind: UseKind::Signature,
                                span: f.span,
                                fact: Fact::Symbol(m),
                                exact: true,
                            },
                        );
                    }
                }
            }
        }
        for ((source, target, stereotype), (spans, facts, exact, count)) in agg {
            if source == target || self.related.contains(&(source, target)) {
                continue;
            }
            let s = self.classifier(source);
            let t = self.classifier(target);
            self.model.add_edge(
                DiagramEdge::new(
                    IdBuilder::new("dep")
                        .text(s.as_str())
                        .text(t.as_str())
                        .text(stereotype)
                        .finish(),
                    EdgeKind::Dependency,
                    s,
                    t,
                )
                .with_stereotype(stereotype)
                .with_property("uses", count.to_string())
                .with_spans(spans)
                .with_facts(facts)
                .with_derivation(if exact {
                    Derivation::Exact
                } else {
                    Derivation::OverApproximate
                }),
            );
        }
    }
}

/// Relationship kind of a field part, its derivation and the ownership fact it rests on.
fn relationship(
    ctx: &DeriveContext,
    field: Option<FieldId>,
    part: &FieldPart,
) -> (EdgeKind, Derivation, Option<Fact>) {
    match part.storage {
        Storage::Inline | Storage::Owning => (EdgeKind::Composition, Derivation::Exact, None),
        Storage::Weak | Storage::Borrowed => (EdgeKind::Association, Derivation::Exact, None),
        Storage::Shared | Storage::Raw => {
            let fact = field.and_then(|f| ctx.fields.and_then(|ff| ff.ownership(f)).map(|o| (f, o)));
            match fact {
                Some((f, o)) => {
                    let kind = match o.kind {
                        crate::derive::PartOwnership::Exclusive => EdgeKind::Composition,
                        crate::derive::PartOwnership::Shared => EdgeKind::Aggregation,
                        crate::derive::PartOwnership::NonOwning => EdgeKind::Association,
                    };
                    let derivation = if o.exact {
                        Derivation::Exact
                    } else {
                        Derivation::OverApproximate
                    };
                    (kind, derivation, Some(Fact::Ownership { field: f.raw() }))
                }
                None => (EdgeKind::Association, Derivation::Exact, None),
            }
        }
    }
}

/// The class diagram of the analysed sources.
pub fn derive(ctx: &DeriveContext) -> DiagramModel {
    let classifiers = project_classifiers(ctx);
    if classifiers.is_empty() {
        return DiagramModel::not_derivable(
            DiagramKind::Class,
            DiagramKind::Class.title(),
            "the frontends report no type declarations in the analysed sources",
        );
    }
    let mut b = ClassBuilder {
        ctx,
        model: DiagramModel::new(DiagramKind::Class, DiagramKind::Class.title()),
        nodes: HashMap::new(),
        ids: HashMap::new(),
        packages: HashMap::new(),
        related: HashSet::new(),
    };
    for &c in &classifiers {
        b.classifier(c);
    }
    for &c in &classifiers {
        b.members(c);
        b.hierarchy(c);
        b.nesting(c);
    }
    for &c in &classifiers {
        b.field_relationships(c);
    }
    b.dependencies(&classifiers);
    let mut model = b.model;
    if ctx.fields.is_none() {
        model.note(
            "ownership facts are unavailable: shared and raw references are shown as associations; only inline and owning storage is shown as composition",
        );
    }
    model.finalize(ctx.limits.max_spans);
    model
}
