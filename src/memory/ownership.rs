//! Ownership and Lifetime Facts (§M.5): composition, aggregation and association from what
//! the program does with its objects.
//!
//! UML 2.5 composite aggregation means the part belongs to one composite at a time and is
//! destroyed with it. That is a statement about lifetimes and holders, which the memory
//! model answers directly:
//!
//! - **who allocates** — the allocating function of every part, and whether the value a
//!   store writes is *fresh* (comes from an allocation in that function, or from a call
//!   whose every target returns fresh objects) and is stored into the receiver or another
//!   fresh object (a *constructive* store);
//! - **who holds** — every cell whose points-to set contains the part;
//! - **who exposes** — functions that return a part they did not just create;
//! - **who releases** — `Release` instructions (destructor, dispose, close) over the part;
//! - the declaration: the field's `RefKind` (`Owning`, `Weak`, `Borrowed`) and, through a
//!   frontend hook, whether the field embeds its value (C/C++ by-value members, C# structs).
//!
//! A field is a **composition** when its storage or declaration makes it one (embedded,
//! `Owning`), or when every part it holds is allocated constructively by the owner, held by
//! nothing but this field and never handed out. Parts held through arrays or containers
//! make the field multi-valued; owner-created or retained multi-valued parts that fail the
//! exclusivity test are a shared **aggregation**; any other reference is an
//! **association**. A field no reachable code stores an object into is **not derivable**.

use crate::ir::func::FuncIr;
use crate::ir::inst::{Inst, Terminator};
use crate::ir::module::{FieldIr, ModuleIr};
use crate::ir::types::{CallSiteId, FieldId, FuncId, IrType, RefKind, Span, TypeId, ValueId};
use crate::ir::value::{ConstVal, Operand, Place};
use crate::memory::escape::{EscapeResult, EscapeState};
use crate::memory::location::{is_array_type, Cell, CellId, FxHashSet, ObjId, Slot, SparseBitSet};
use crate::memory::points_to::{intrinsic_effect, IntrinsicEffect, MemoryConfig, PointsToResult};
use crate::psa::bdd::fxhash::FxHashMap;

// ── Frontend hooks ──────────────────────────────────────────────────────────

/// How a field stores its value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FieldStorage {
    /// The value is part of the owner's storage (by-value member, value-type field): its
    /// lifetime is the owner's.
    Embedded,
    /// The field holds a reference to a separately allocated object.
    Reference,
}

/// Language facts the ownership analysis cannot read from the IR alone.
pub trait OwnershipHooks {
    /// Storage class of a field. The IR's `Ref`/`Ptr` types already say "reference"; what
    /// a by-value `Named` field means is language knowledge.
    fn field_storage(&self, _module: &ModuleIr, _field: &FieldIr) -> FieldStorage {
        FieldStorage::Reference
    }

    /// Element types of a container type whose implementation the module does not lower
    /// (a library collection), read from its declaration (`List<Part>` → `Part`).
    fn container_elements(&self, _module: &ModuleIr, _ty: TypeId) -> Option<Vec<TypeId>> {
        None
    }
}

/// No language facts beyond the IR.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoOwnershipHooks;

impl OwnershipHooks for NoOwnershipHooks {}

// ── Facts ───────────────────────────────────────────────────────────────────

/// Kind of the relationship a field realizes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OwnershipKind {
    Composition,
    Aggregation,
    Association,
    /// No reachable code stores an object into the field; its relationship kind is not
    /// derivable from the program's behaviour.
    NotDerivable,
}

/// The facts a classification rests on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OwnershipBasis {
    /// The field embeds its value.
    EmbeddedValue,
    /// The field's type is an owning reference.
    DeclaredOwning,
    /// The field's type is a non-owning reference (`Weak`, `Borrowed`).
    DeclaredNonOwning(RefKind),
    /// Every part is created by the owner's own code.
    OwnerAllocated,
    /// Some part is stored from a value the storing code did not create.
    ExternallySupplied,
    /// Every part is held by this field only.
    Exclusive,
    /// Some part is also held by another cell (another field, a global, a container, the
    /// external world).
    SharedParts,
    /// Some part is returned by a function that did not create it.
    ExposedByReturn,
    /// The field holds parts through arrays or containers.
    MultiValued,
    /// Some part is released (destroyed, disposed) by the owner's code.
    ReleasedByOwner,
    /// Some part is of external origin; its holders are not fully known.
    ExternalParts,
    /// Static (per-type) storage.
    StaticField,
    /// No object is ever stored.
    NoStoredObjects,
}

/// Precision of a classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OwnershipPrecision {
    Exact,
    /// The facts are incomplete: truncated solve, parts of external origin, or container
    /// contents the module does not lower.
    Partial,
}

/// A store of objects into a field or element slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoreFact {
    pub func: FuncId,
    pub span: Span,
    /// The stored value was created by the storing function.
    pub fresh: bool,
    /// Fresh, and stored into the storing function's receiver or into another object the
    /// function just created.
    pub constructive: bool,
}

/// A `Release` of objects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReleaseFact {
    pub func: FuncId,
    pub span: Span,
    /// Released through a field of the releasing function's receiver.
    pub by_owner: bool,
}

/// Relationship facts of one field.
#[derive(Debug, Clone)]
pub struct FieldOwnership {
    pub field: FieldId,
    /// Declaring type (`u32::MAX` when not derivable).
    pub owner_type: u32,
    pub kind: OwnershipKind,
    pub basis: Vec<OwnershipBasis>,
    pub precision: OwnershipPrecision,
    /// Dynamic types of the parts (or the declared element types of a container).
    pub part_types: Vec<u32>,
    /// The part objects (array and container elements for multi-valued fields).
    pub parts: Vec<ObjId>,
    /// More than one part per owner is derivable (arrays, containers).
    pub many: bool,
    pub stores: Vec<StoreFact>,
    pub releases: Vec<ReleaseFact>,
    /// Functions that return a part they did not create.
    pub exposed_by: Vec<FuncId>,
    /// The field was synthesized by lowering (closure environments).
    pub synthetic: bool,
}

/// Lifetime facts of one allocated object.
#[derive(Debug, Clone)]
pub struct AllocationOwnership {
    pub object: ObjId,
    pub allocator: Option<FuncId>,
    /// Every cell that may hold the object.
    pub holders: Vec<CellId>,
    /// The single (owner object, field) holding the object, when there is exactly one.
    pub owner: Option<(ObjId, FieldId)>,
    pub stored_by: Vec<StoreFact>,
    pub released_by: Vec<ReleaseFact>,
    pub exposed_by: Vec<FuncId>,
    pub escape: EscapeState,
}

/// Ownership facts of a module.
#[derive(Debug, Clone)]
pub struct OwnershipResult {
    pub fields: Vec<FieldOwnership>,
    pub allocations: Vec<AllocationOwnership>,
    /// Functions every return of which yields a freshly created object (factories).
    pub fresh_returning: Vec<FuncId>,
    /// Derived from a truncated points-to solve.
    pub partial: bool,
    field_index: FxHashMap<FieldId, usize>,
    allocation_index: FxHashMap<ObjId, usize>,
}

impl OwnershipResult {
    pub fn field(&self, field: FieldId) -> Option<&FieldOwnership> {
        self.field_index.get(&field).map(|&i| &self.fields[i])
    }

    pub fn allocation(&self, obj: ObjId) -> Option<&AllocationOwnership> {
        self.allocation_index.get(&obj).map(|&i| &self.allocations[i])
    }

    /// (creator type, created type, span) for every allocation whose allocating function is
    /// a member of a type: the UML «create» dependencies.
    pub fn creations(&self, module: &ModuleIr, pts: &PointsToResult) -> Vec<(u32, u32, Span)> {
        let mut out = Vec::new();
        for a in &self.allocations {
            let info = pts.locations.object(a.object);
            let (Some(f), Some(created)) = (a.allocator, info.dynamic_type) else {
                continue;
            };
            let Some(creator) = module
                .funcs
                .get(f.index())
                .filter(|x| x.id == f)
                .and_then(|x| x.enclosing_type)
            else {
                continue;
            };
            out.push((creator, created, info.span));
        }
        out.sort_unstable();
        out.dedup();
        out
    }
}

// ── Analysis ────────────────────────────────────────────────────────────────

/// Derive ownership and lifetime facts.
pub fn analyze(
    module: &ModuleIr,
    pts: &PointsToResult,
    escape: &EscapeResult,
    memory: &MemoryConfig,
    hooks: &dyn OwnershipHooks,
) -> OwnershipResult {
    let funcs: Vec<&FuncIr> = pts
        .call_graph
        .reachable
        .iter()
        .filter_map(|&f| module.funcs.get(f.index()).filter(|x| x.id == f))
        .collect();

    let fresh = Freshness::compute(module, pts, memory, &funcs);

    // Holders of every object.
    let mut holders: FxHashMap<ObjId, Vec<CellId>> = FxHashMap::default();
    for (cid, _) in pts.locations.cells() {
        for o in pts.cell_points_to(cid).objects() {
            holders.entry(o).or_default().push(cid);
        }
    }

    // Stores, releases and exposures, from reachable code.
    let mut field_stores: FxHashMap<FieldId, Vec<StoreFact>> = FxHashMap::default();
    let mut field_releases: FxHashMap<FieldId, Vec<ReleaseFact>> = FxHashMap::default();
    let mut object_stores: FxHashMap<ObjId, Vec<StoreFact>> = FxHashMap::default();
    let mut object_releases: FxHashMap<ObjId, Vec<ReleaseFact>> = FxHashMap::default();
    let mut exposed: FxHashMap<ObjId, Vec<FuncId>> = FxHashMap::default();
    let mut constructive_parts: FxHashMap<FieldId, SparseBitSet> = FxHashMap::default();

    for func in &funcs {
        let fresh_values = fresh.values.get(&func.id);
        let is_fresh = |v: ValueId| fresh_values.map(|s| s.contains(&v)).unwrap_or(false);
        let receiver = func.receiver();
        for block in &func.blocks {
            for (i, inst) in block.insts.iter().enumerate() {
                let span = block.inst_spans.get(i).copied().unwrap_or(Span::NONE);
                match inst {
                    Inst::Store { place, value } => {
                        let Some(base) = place.base() else {
                            continue;
                        };
                        let value_fresh = matches!(value, Operand::Value(v) if is_fresh(*v));
                        let fact = StoreFact {
                            func: func.id,
                            span,
                            fresh: value_fresh,
                            constructive: value_fresh
                                && (Some(base) == receiver || is_fresh(base)),
                        };
                        let parts = pts.operand_points_to(func.id, value);
                        if let Place::Field { field, .. } = place {
                            field_stores.entry(*field).or_default().push(fact);
                            if fact.constructive {
                                constructive_parts
                                    .entry(*field)
                                    .or_default()
                                    .union_with(parts);
                            }
                        }
                        for p in parts.objects() {
                            object_stores.entry(p).or_default().push(fact);
                        }
                    }
                    Inst::Release { place } => {
                        let by_owner = matches!(place, Place::Field { base, .. } if Some(*base) == receiver);
                        let fact = ReleaseFact {
                            func: func.id,
                            span,
                            by_owner,
                        };
                        if let Place::Field { field, .. } = place {
                            field_releases.entry(*field).or_default().push(fact);
                        }
                        for o in released_objects(pts, func.id, place).objects() {
                            object_releases.entry(o).or_default().push(fact);
                        }
                    }
                    _ => {}
                }
            }
            if let Terminator::Return(Some(Operand::Value(v))) = &block.term {
                if !is_fresh(*v) {
                    for o in pts.points_to(func.id, *v).objects() {
                        let list = exposed.entry(o).or_default();
                        if !list.contains(&func.id) {
                            list.push(func.id);
                        }
                    }
                }
            }
        }
    }

    // Fields.
    let mut fields = Vec::with_capacity(module.fields.len());
    let mut field_index = FxHashMap::default();
    let mut cells_by_field: FxHashMap<FieldId, Vec<CellId>> = FxHashMap::default();
    for (cid, cell) in pts.locations.cells() {
        if let Cell::Object {
            slot: Slot::Field(f),
            ..
        } = cell
        {
            cells_by_field.entry(f).or_default().push(cid);
        }
    }
    let ctx = FieldContext {
        module,
        pts,
        holders: &holders,
        exposed: &exposed,
        object_stores: &object_stores,
        hooks,
    };
    for field in &module.fields {
        let fo = ctx.classify(
            field,
            cells_by_field.get(&field.id).map(|v| v.as_slice()).unwrap_or(&[]),
            field_stores.remove(&field.id).unwrap_or_default(),
            field_releases.remove(&field.id).unwrap_or_default(),
            constructive_parts.get(&field.id),
        );
        field_index.insert(field.id, fields.len());
        fields.push(fo);
    }

    // Allocations.
    let mut allocations = Vec::new();
    let mut allocation_index = FxHashMap::default();
    for (id, info) in pts.locations.objects() {
        if info.object.base.is_opaque() {
            continue;
        }
        let hs = holders.get(&id).cloned().unwrap_or_default();
        let owner = single_owner(pts, &hs);
        allocation_index.insert(id, allocations.len());
        allocations.push(AllocationOwnership {
            object: id,
            allocator: info.func,
            holders: hs,
            owner,
            stored_by: object_stores.get(&id).cloned().unwrap_or_default(),
            released_by: object_releases.get(&id).cloned().unwrap_or_default(),
            exposed_by: exposed.get(&id).cloned().unwrap_or_default(),
            escape: escape.state(id),
        });
    }

    let mut fresh_returning: Vec<FuncId> = fresh.returns_fresh.iter().copied().collect();
    fresh_returning.sort_unstable();

    OwnershipResult {
        fields,
        allocations,
        fresh_returning,
        partial: !pts.is_complete() || escape.partial,
        field_index,
        allocation_index,
    }
}

/// Objects whose lifetime a `Release` of `place` ends: the objects the place designates and
/// the objects stored in it (a stack slot of reference type releases its content).
fn released_objects(pts: &PointsToResult, func: FuncId, place: &Place) -> SparseBitSet {
    match place {
        Place::Local(v) | Place::Deref(v) => {
            let mut out = pts.points_to(func, *v).clone();
            out.union_with(&pts.place_points_to(func, place));
            out
        }
        _ => pts.place_points_to(func, place),
    }
}

/// The unique (owner, field) among the holders, when every holder is the same field slot of
/// the same object.
fn single_owner(pts: &PointsToResult, holders: &[CellId]) -> Option<(ObjId, FieldId)> {
    let mut found: Option<(ObjId, FieldId)> = None;
    for &c in holders {
        match pts.locations.cell(c) {
            Cell::Object {
                obj,
                slot: Slot::Field(f),
            } => match found {
                None => found = Some((obj, f)),
                Some(prev) if prev == (obj, f) => {}
                Some(_) => return None,
            },
            _ => return None,
        }
    }
    found
}

struct FieldContext<'a> {
    module: &'a ModuleIr,
    pts: &'a PointsToResult,
    holders: &'a FxHashMap<ObjId, Vec<CellId>>,
    exposed: &'a FxHashMap<ObjId, Vec<FuncId>>,
    object_stores: &'a FxHashMap<ObjId, Vec<StoreFact>>,
    hooks: &'a dyn OwnershipHooks,
}

impl FieldContext<'_> {
    fn classify(
        &self,
        field: &FieldIr,
        cells: &[CellId],
        stores: Vec<StoreFact>,
        releases: Vec<ReleaseFact>,
        constructive: Option<&SparseBitSet>,
    ) -> FieldOwnership {
        let mut basis = Vec::new();
        let mut precision = if self.pts.is_complete() {
            OwnershipPrecision::Exact
        } else {
            OwnershipPrecision::Partial
        };

        // Direct parts, then array / container elements behind them.
        let mut direct = SparseBitSet::new();
        for &c in cells {
            direct.union_with(self.pts.cell_points_to(c));
        }
        let mut parts = SparseBitSet::new();
        let mut carriers: FxHashSet<CellId> = cells.iter().copied().collect();
        // Elements of arrays the owner stored constructively.
        let mut owned_array_elements = SparseBitSet::new();
        let mut many = false;
        let mut declared_elements: Vec<u32> = Vec::new();
        if let Some(elems) = self.hooks.container_elements(self.module, field.ty) {
            many = true;
            declared_elements.extend(elems.iter().filter_map(|&t| self.module.types.symbol_of(t)));
        }
        for o in direct.objects() {
            let info = self.pts.locations.object(o);
            let is_array = info.ty.map(|t| is_array_type(self.module, t)).unwrap_or(false);
            if is_array {
                many = true;
                if let Some(c) = self.pts.locations.find_cell(&Cell::Object {
                    obj: o,
                    slot: Slot::Element,
                }) {
                    carriers.insert(c);
                    let elements = self.pts.cell_points_to(c);
                    parts.union_with(elements);
                    if constructive.map(|k| k.contains(o.0)).unwrap_or(false) {
                        owned_array_elements.union_with(elements);
                    }
                }
            } else if many && !declared_elements.is_empty() {
                // A container whose contents the module does not model: the declaration
                // names its element types.
                precision = OwnershipPrecision::Partial;
            } else {
                parts.insert(o.0);
            }
        }

        let ref_kind = match self.module.types.get(field.ty) {
            IrType::Ref { kind, .. } => Some(*kind),
            _ => None,
        };
        let storage = self.hooks.field_storage(self.module, field);

        let mut part_types: Vec<u32> = parts
            .objects()
            .filter_map(|o| self.pts.locations.object(o).dynamic_type)
            .chain(declared_elements.iter().copied())
            .collect();
        part_types.sort_unstable();
        part_types.dedup();

        let external_parts = parts
            .objects()
            .any(|o| self.pts.locations.object(o).object.base.is_opaque());
        if external_parts {
            basis.push(OwnershipBasis::ExternalParts);
            precision = OwnershipPrecision::Partial;
        }
        if many {
            basis.push(OwnershipBasis::MultiValued);
        }
        let released_by_owner = releases.iter().any(|r| r.by_owner);
        if released_by_owner {
            basis.push(OwnershipBasis::ReleasedByOwner);
        }

        let mut exposed_by: Vec<FuncId> = parts
            .objects()
            .flat_map(|o| self.exposed.get(&o).cloned().unwrap_or_default())
            .collect();
        exposed_by.sort_unstable();
        exposed_by.dedup();

        let kind = if field.is_static {
            basis.push(OwnershipBasis::StaticField);
            OwnershipKind::Association
        } else if storage == FieldStorage::Embedded {
            basis.push(OwnershipBasis::EmbeddedValue);
            OwnershipKind::Composition
        } else if ref_kind == Some(RefKind::Owning) {
            basis.push(OwnershipBasis::DeclaredOwning);
            OwnershipKind::Composition
        } else if let Some(k @ (RefKind::Weak | RefKind::Borrowed)) = ref_kind {
            basis.push(OwnershipBasis::DeclaredNonOwning(k));
            OwnershipKind::Association
        } else if parts.is_empty() {
            basis.push(OwnershipBasis::NoStoredObjects);
            OwnershipKind::NotDerivable
        } else {
            // Behavioural classification.
            let exclusive = parts.objects().all(|p| {
                self.holders
                    .get(&p)
                    .map(|hs| hs.iter().all(|h| carriers.contains(h)))
                    .unwrap_or(true)
            });
            // A direct part is owner-made when the owner stored it constructively and every
            // store of it anywhere is constructive; an element when it sits in an array the
            // owner made and every store of it writes a value its storing code created.
            let owner_allocated = parts.objects().all(|p| {
                let stores = self.object_stores.get(&p).map(|v| v.as_slice()).unwrap_or(&[]);
                if stores.is_empty() {
                    return false;
                }
                if owned_array_elements.contains(p.0) {
                    stores.iter().all(|s| s.fresh)
                } else {
                    constructive.map(|c| c.contains(p.0)).unwrap_or(false)
                        && stores.iter().all(|s| s.constructive)
                }
            });
            let is_exposed = !exposed_by.is_empty();
            basis.push(if exclusive {
                OwnershipBasis::Exclusive
            } else {
                OwnershipBasis::SharedParts
            });
            basis.push(if owner_allocated {
                OwnershipBasis::OwnerAllocated
            } else {
                OwnershipBasis::ExternallySupplied
            });
            if is_exposed {
                basis.push(OwnershipBasis::ExposedByReturn);
            }
            if exclusive && owner_allocated && !is_exposed && !external_parts {
                OwnershipKind::Composition
            } else if many || (owner_allocated && !external_parts) {
                OwnershipKind::Aggregation
            } else {
                OwnershipKind::Association
            }
        };

        FieldOwnership {
            field: field.id,
            owner_type: field.owner_symbol,
            kind,
            basis,
            precision,
            part_types,
            parts: parts.objects().collect(),
            many,
            stores,
            releases,
            exposed_by,
            synthetic: field.synthetic,
        }
    }
}

// ── Freshness ───────────────────────────────────────────────────────────────

/// Values created by their function: greatest fixpoint over allocations, copies, phis and
/// calls whose every target returns fresh objects.
struct Freshness {
    values: FxHashMap<FuncId, FxHashSet<ValueId>>,
    returns_fresh: FxHashSet<FuncId>,
}

/// How a value is defined, for freshness.
enum FreshDef {
    /// Allocation or allocating intrinsic.
    Alloc,
    /// Copy of the listed operands (assign, cast, phi).
    Copy(Vec<Operand>),
    /// Result of a call.
    Call(CallSiteId),
}

impl Freshness {
    fn compute(
        module: &ModuleIr,
        pts: &PointsToResult,
        memory: &MemoryConfig,
        funcs: &[&FuncIr],
    ) -> Self {
        let mut defs: FxHashMap<FuncId, FxHashMap<ValueId, FreshDef>> = FxHashMap::default();
        for func in funcs {
            let mut d: FxHashMap<ValueId, FreshDef> = FxHashMap::default();
            for block in &func.blocks {
                for inst in &block.insts {
                    match inst {
                        Inst::Alloc { dst, .. } => {
                            d.insert(*dst, FreshDef::Alloc);
                        }
                        Inst::MakeClosure { dst, .. } => {
                            d.insert(*dst, FreshDef::Alloc);
                        }
                        Inst::Assign { dst, src } | Inst::Cast { dst, src, .. } => {
                            d.insert(*dst, FreshDef::Copy(vec![*src]));
                        }
                        Inst::Phi { dst, incoming } => {
                            d.insert(
                                *dst,
                                FreshDef::Copy(incoming.iter().map(|(_, op)| *op).collect()),
                            );
                        }
                        Inst::Call {
                            dst: Some(dst),
                            site,
                            ..
                        } => {
                            d.insert(*dst, FreshDef::Call(*site));
                        }
                        Inst::Intrinsic {
                            dst: Some(dst),
                            name,
                            ..
                        } => {
                            if matches!(
                                intrinsic_effect(module, memory, *name),
                                IntrinsicEffect::Allocates { .. }
                            ) {
                                d.insert(*dst, FreshDef::Alloc);
                            }
                        }
                        _ => {}
                    }
                }
            }
            defs.insert(func.id, d);
        }

        let mut returns_fresh: FxHashSet<FuncId> = funcs
            .iter()
            .filter(|f| {
                f.blocks
                    .iter()
                    .any(|b| matches!(b.term, Terminator::Return(Some(Operand::Value(_)))))
            })
            .map(|f| f.id)
            .collect();
        let mut values: FxHashMap<FuncId, FxHashSet<ValueId>> = FxHashMap::default();

        loop {
            values.clear();
            for func in funcs {
                let d = &defs[&func.id];
                let mut fresh: FxHashSet<ValueId> = d.keys().copied().collect();
                loop {
                    let mut removed = Vec::new();
                    for &v in &fresh {
                        let ok = match &d[&v] {
                            FreshDef::Alloc => true,
                            FreshDef::Copy(ops) => ops.iter().all(|op| match op {
                                Operand::Value(w) => fresh.contains(w),
                                // Null and undefined contribute no object; a string
                                // constant is the shared literal pool.
                                Operand::Const(ConstVal::Str(_)) => false,
                                Operand::Const(_) | Operand::Undef => true,
                            }),
                            FreshDef::Call(site) => match pts.call_graph.site(*site) {
                                Some(r) => {
                                    !r.targets.is_empty()
                                        && r.external.is_empty()
                                        && r.unresolved.is_empty()
                                        && r.targets.iter().all(|(f, _)| returns_fresh.contains(f))
                                }
                                None => false,
                            },
                        };
                        if !ok {
                            removed.push(v);
                        }
                    }
                    if removed.is_empty() {
                        break;
                    }
                    for v in removed {
                        fresh.remove(&v);
                    }
                }
                values.insert(func.id, fresh);
            }
            let mut changed = false;
            let current: Vec<FuncId> = returns_fresh.iter().copied().collect();
            for f in current {
                let Some(func) = funcs.iter().find(|x| x.id == f) else {
                    continue;
                };
                let fresh = &values[&f];
                let all = func.blocks.iter().all(|b| match &b.term {
                    Terminator::Return(Some(Operand::Value(v))) => fresh.contains(v),
                    Terminator::Return(Some(Operand::Const(ConstVal::Str(_)))) => false,
                    _ => true,
                });
                if !all {
                    returns_fresh.remove(&f);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        Self {
            values,
            returns_fresh,
        }
    }
}
