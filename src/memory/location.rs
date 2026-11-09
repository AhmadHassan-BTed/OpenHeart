//! Abstract Memory Locations (§M.1).
//!
//! The memory model is the one inclusion-based pointer analyses are defined over:
//!
//! - an **abstract object** stands for every concrete object created at one allocation point
//!   (an `Alloc` instruction, a `MakeClosure`, an allocating intrinsic) in one k = 1 context,
//!   plus four kinds of objects the program text does not allocate: the objects passed into
//!   an entry point's parameters, the string-literal pool and the explicit `Unknown` object
//!   that summarizes everything created by code outside the module;
//! - an **abstract cell** is a location that holds a value: a slot of an object (a declared
//!   field, the one summarized element of an array-like object, the whole pointee reached by
//!   `Deref`, a captured variable of a closure), a global or a static field.
//!
//! Points-to sets are sets of `ObjId`, stored as sparse bit sets. Cells are created on demand
//! by the solver, so the table only ever contains the cells the program actually touches.

use std::collections::HashSet;

use crate::ir::inst::Region;
use crate::ir::module::ModuleIr;
use crate::ir::types::{AllocSiteId, BlockId, CallSiteId, FieldId, FuncId, GlobalId, IrType, Span, TypeId};
use crate::psa::bdd::fxhash::{FxBuildHasher, FxHashMap};

/// `HashSet` using the Fx hash (integer-keyed solver tables).
pub type FxHashSet<K> = HashSet<K, FxBuildHasher>;

// ── Ids ─────────────────────────────────────────────────────────────────────

/// Dense id of an abstract object: the element type of every points-to set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ObjId(pub u32);

/// Dense id of an abstract cell (a location that holds a value).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CellId(pub u32);

/// Dense id of a `MakeClosure` instruction (closure and function-value creation point).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ClosureSiteId(pub u32);

/// Dense id of an `Intrinsic` instruction whose modelled effect allocates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct IntrinsicSiteId(pub u32);

// ── Objects ─────────────────────────────────────────────────────────────────

/// What an abstract object summarizes, without its context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum HeapBase {
    /// Objects created by one `Alloc` instruction.
    Site(AllocSiteId),
    /// Closures / function values created by one `MakeClosure` instruction.
    Closure(ClosureSiteId),
    /// Objects created by one allocating intrinsic.
    Intrinsic(IntrinsicSiteId),
    /// The object(s) an outside caller passes as parameter `index` of entry point `func`.
    EntryParam { func: FuncId, index: u32 },
    /// The pool of string literals (`ConstVal::Str`).
    StringLiteral,
    /// Everything created by code outside the module: external libraries, the runtime,
    /// unmodelled intrinsics. Its slots are the external world itself.
    Unknown,
}

impl HeapBase {
    /// The program text does not show how the object is built or what its slots hold.
    #[inline]
    pub fn is_opaque(&self) -> bool {
        matches!(self, HeapBase::EntryParam { .. } | HeapBase::Unknown)
    }
}

/// k = 1 context of a function instance or of a heap object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Context {
    /// Context-insensitive (also the context of entry points).
    Empty,
    /// 1-call-site sensitivity: the call site that created the instance.
    CallSite(CallSiteId),
    /// 1-object sensitivity: the allocation point of the receiver object.
    Receiver(HeapBase),
}

/// An abstract object: an allocation point qualified by a context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AbstractObject {
    pub base: HeapBase,
    pub context: Context,
}

/// Facts recorded for every abstract object when it is created.
#[derive(Debug, Clone)]
pub struct ObjectInfo {
    pub object: AbstractObject,
    /// IR type of the allocated object, when the allocation names one.
    pub ty: Option<TypeId>,
    /// Class symbol used for dynamic dispatch (`IrType::Named` of `ty`, or the type a
    /// frontend supplied for literals and intrinsic allocations). `None`: not derivable.
    pub dynamic_type: Option<u32>,
    /// Storage region of the allocation. `None` for objects the program does not allocate.
    pub region: Option<Region>,
    /// Function containing the allocation point.
    pub func: Option<FuncId>,
    /// Source span of the allocation point (`Span::NONE` when synthesized).
    pub span: Span,
}

// ── Cells ───────────────────────────────────────────────────────────────────

/// A value-holding slot of an abstract object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Slot {
    /// A declared field (`Place::Field`).
    Field(FieldId),
    /// The single summarized element of an array-like object (`Place::Index`).
    Element,
    /// The whole pointee reached through a raw dereference (`Place::Deref`).
    Content,
    /// Captured variable `index` of a closure object (`MakeClosure.captures`).
    Capture(u32),
}

/// An abstract cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Cell {
    /// A slot of an abstract object.
    Object { obj: ObjId, slot: Slot },
    /// A module-level global (`Place::Global`).
    Global(GlobalId),
    /// A static field of a type (`Place::Static`).
    Static { owner: u32, field: FieldId },
}

impl Cell {
    /// The object a slot cell belongs to.
    #[inline]
    pub fn object(&self) -> Option<ObjId> {
        match self {
            Cell::Object { obj, .. } => Some(*obj),
            _ => None,
        }
    }

    /// Cells that exist once per program rather than per object.
    #[inline]
    pub fn is_global(&self) -> bool {
        matches!(self, Cell::Global(_) | Cell::Static { .. })
    }
}

/// Any abstract memory location: an object or a cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Location {
    Object(ObjId),
    Cell(CellId),
}

// ── Creation points without an IR id ────────────────────────────────────────

/// A `MakeClosure` instruction.
#[derive(Debug, Clone, Copy)]
pub struct ClosureSite {
    pub id: ClosureSiteId,
    /// Function containing the instruction.
    pub func: FuncId,
    pub block: BlockId,
    /// Index of the instruction in its block.
    pub inst: u32,
    /// The function the closure value calls.
    pub target: FuncId,
    /// Number of captured operands (bound to the target's leading parameters).
    pub captures: u32,
    pub span: Span,
}

/// An allocating `Intrinsic` instruction.
#[derive(Debug, Clone, Copy)]
pub struct IntrinsicSite {
    pub id: IntrinsicSiteId,
    pub func: FuncId,
    pub block: BlockId,
    pub inst: u32,
    /// Interned intrinsic name.
    pub name: u32,
    pub span: Span,
}

// ── Location table ──────────────────────────────────────────────────────────

/// Interner and fact store for every abstract object and cell of one analysis run.
#[derive(Debug, Clone)]
pub struct LocationTable {
    objects: Vec<ObjectInfo>,
    object_index: FxHashMap<AbstractObject, ObjId>,
    cells: Vec<Cell>,
    cell_index: FxHashMap<Cell, CellId>,
    object_cells: Vec<Vec<CellId>>,
    closure_sites: Vec<ClosureSite>,
    closure_index: FxHashMap<(FuncId, BlockId, u32), ClosureSiteId>,
    intrinsic_sites: Vec<IntrinsicSite>,
    intrinsic_index: FxHashMap<(FuncId, BlockId, u32), IntrinsicSiteId>,
    unknown: ObjId,
}

impl Default for LocationTable {
    fn default() -> Self {
        Self::new()
    }
}

impl LocationTable {
    /// A table holding only the `Unknown` object (always `ObjId(0)`).
    pub fn new() -> Self {
        let mut table = Self {
            objects: Vec::new(),
            object_index: FxHashMap::default(),
            cells: Vec::new(),
            cell_index: FxHashMap::default(),
            object_cells: Vec::new(),
            closure_sites: Vec::new(),
            closure_index: FxHashMap::default(),
            intrinsic_sites: Vec::new(),
            intrinsic_index: FxHashMap::default(),
            unknown: ObjId(0),
        };
        let unknown = AbstractObject {
            base: HeapBase::Unknown,
            context: Context::Empty,
        };
        let (id, _) = table.intern_object(ObjectInfo {
            object: unknown,
            ty: None,
            dynamic_type: None,
            region: None,
            func: None,
            span: Span::NONE,
        });
        table.unknown = id;
        table
    }

    /// The object summarizing everything outside the module.
    #[inline]
    pub fn unknown(&self) -> ObjId {
        self.unknown
    }

    /// Intern an object; returns its id and whether it was created by this call. The facts
    /// of an existing object are kept.
    pub fn intern_object(&mut self, info: ObjectInfo) -> (ObjId, bool) {
        if let Some(&id) = self.object_index.get(&info.object) {
            return (id, false);
        }
        let id = ObjId(self.objects.len() as u32);
        self.object_index.insert(info.object, id);
        self.objects.push(info);
        self.object_cells.push(Vec::new());
        (id, true)
    }

    /// Look up an object without creating it.
    #[inline]
    pub fn find_object(&self, object: &AbstractObject) -> Option<ObjId> {
        self.object_index.get(object).copied()
    }

    #[inline]
    pub fn object(&self, id: ObjId) -> &ObjectInfo {
        &self.objects[id.0 as usize]
    }

    #[inline]
    pub fn object_count(&self) -> usize {
        self.objects.len()
    }

    /// All objects in id order.
    pub fn objects(&self) -> impl Iterator<Item = (ObjId, &ObjectInfo)> {
        self.objects
            .iter()
            .enumerate()
            .map(|(i, info)| (ObjId(i as u32), info))
    }

    /// Intern a cell; returns its id and whether it was created by this call.
    pub fn intern_cell(&mut self, cell: Cell) -> (CellId, bool) {
        if let Some(&id) = self.cell_index.get(&cell) {
            return (id, false);
        }
        let id = CellId(self.cells.len() as u32);
        self.cell_index.insert(cell, id);
        self.cells.push(cell);
        if let Some(obj) = cell.object() {
            self.object_cells[obj.0 as usize].push(id);
        }
        (id, true)
    }

    /// Look up a cell without creating it.
    #[inline]
    pub fn find_cell(&self, cell: &Cell) -> Option<CellId> {
        self.cell_index.get(cell).copied()
    }

    #[inline]
    pub fn cell(&self, id: CellId) -> Cell {
        self.cells[id.0 as usize]
    }

    #[inline]
    pub fn cell_count(&self) -> usize {
        self.cells.len()
    }

    /// All cells in id order.
    pub fn cells(&self) -> impl Iterator<Item = (CellId, Cell)> + '_ {
        self.cells
            .iter()
            .enumerate()
            .map(|(i, c)| (CellId(i as u32), *c))
    }

    /// The slot cells of one object that the program touches.
    #[inline]
    pub fn cells_of(&self, obj: ObjId) -> &[CellId] {
        &self.object_cells[obj.0 as usize]
    }

    /// Intern the closure creation point at (`func`, `block`, `inst`).
    pub fn intern_closure_site(
        &mut self,
        func: FuncId,
        block: BlockId,
        inst: u32,
        target: FuncId,
        captures: u32,
        span: Span,
    ) -> ClosureSiteId {
        if let Some(&id) = self.closure_index.get(&(func, block, inst)) {
            return id;
        }
        let id = ClosureSiteId(self.closure_sites.len() as u32);
        self.closure_index.insert((func, block, inst), id);
        self.closure_sites.push(ClosureSite {
            id,
            func,
            block,
            inst,
            target,
            captures,
            span,
        });
        id
    }

    #[inline]
    pub fn closure_site(&self, id: ClosureSiteId) -> &ClosureSite {
        &self.closure_sites[id.0 as usize]
    }

    /// The closure creation point at an instruction position, if the solver reached it.
    #[inline]
    pub fn closure_site_at(&self, func: FuncId, block: BlockId, inst: u32) -> Option<ClosureSiteId> {
        self.closure_index.get(&(func, block, inst)).copied()
    }

    pub fn closure_sites(&self) -> &[ClosureSite] {
        &self.closure_sites
    }

    /// Intern the allocating intrinsic at (`func`, `block`, `inst`).
    pub fn intern_intrinsic_site(
        &mut self,
        func: FuncId,
        block: BlockId,
        inst: u32,
        name: u32,
        span: Span,
    ) -> IntrinsicSiteId {
        if let Some(&id) = self.intrinsic_index.get(&(func, block, inst)) {
            return id;
        }
        let id = IntrinsicSiteId(self.intrinsic_sites.len() as u32);
        self.intrinsic_index.insert((func, block, inst), id);
        self.intrinsic_sites.push(IntrinsicSite {
            id,
            func,
            block,
            inst,
            name,
            span,
        });
        id
    }

    #[inline]
    pub fn intrinsic_site(&self, id: IntrinsicSiteId) -> &IntrinsicSite {
        &self.intrinsic_sites[id.0 as usize]
    }

    pub fn intrinsic_sites(&self) -> &[IntrinsicSite] {
        &self.intrinsic_sites
    }

    /// All objects (every context) created at one allocation site.
    pub fn objects_of_site(&self, site: AllocSiteId) -> Vec<ObjId> {
        self.objects()
            .filter(|(_, info)| info.object.base == HeapBase::Site(site))
            .map(|(id, _)| id)
            .collect()
    }

    /// All objects whose dynamic type is `type_symbol`.
    pub fn objects_of_type(&self, type_symbol: u32) -> Vec<ObjId> {
        self.objects()
            .filter(|(_, info)| info.dynamic_type == Some(type_symbol))
            .map(|(id, _)| id)
            .collect()
    }

    /// Stable human-readable name of an object for logs and diagnostics.
    pub fn describe_object(&self, id: ObjId) -> String {
        let info = self.object(id);
        let base = describe_base(info.object.base);
        match info.object.context {
            Context::Empty => base,
            Context::CallSite(cs) => format!("{}[cs#{}]", base, cs.0),
            Context::Receiver(r) => format!("{}[recv:{}]", base, describe_base(r)),
        }
    }

    /// Stable human-readable name of a cell for logs and diagnostics.
    pub fn describe_cell(&self, id: CellId) -> String {
        match self.cell(id) {
            Cell::Object { obj, slot } => {
                let o = self.describe_object(obj);
                match slot {
                    Slot::Field(f) => format!("{}.field#{}", o, f.0),
                    Slot::Element => format!("{}[*]", o),
                    Slot::Content => format!("*{}", o),
                    Slot::Capture(i) => format!("{}.capture{}", o, i),
                }
            }
            Cell::Global(g) => format!("global#{}", g.0),
            Cell::Static { owner, field } => format!("static#{}.field#{}", owner, field.0),
        }
    }
}

fn describe_base(base: HeapBase) -> String {
    match base {
        HeapBase::Site(s) => format!("alloc#{}", s.0),
        HeapBase::Closure(c) => format!("closure#{}", c.0),
        HeapBase::Intrinsic(i) => format!("intrinsic#{}", i.0),
        HeapBase::EntryParam { func, index } => format!("param{}@fn#{}", index, func.0),
        HeapBase::StringLiteral => "string-literal".to_string(),
        HeapBase::Unknown => "unknown".to_string(),
    }
}

// ── IR type helpers ─────────────────────────────────────────────────────────

/// The type is an array (the only objects whose element slot the IR writes through
/// `Place::Index`).
pub(crate) fn is_array_type(module: &ModuleIr, id: TypeId) -> bool {
    matches!(module.types.get(id), IrType::Array { .. })
}

// ── Sparse bit set ──────────────────────────────────────────────────────────

/// Sorted sparse bit vector: a list of (word index, 64-bit word) pairs with no zero words.
///
/// Points-to sets are small relative to the object universe and cluster by allocation
/// order (objects of one function are interned together), so sorted 64-bit words give
/// O(n + m) union/difference with little memory, and equality is a slice comparison.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct SparseBitSet {
    words: Vec<(u32, u64)>,
}

impl SparseBitSet {
    /// The empty set.
    pub const fn new() -> Self {
        Self { words: Vec::new() }
    }

    /// The set `{e}`.
    pub fn singleton(e: u32) -> Self {
        let (w, b) = Self::split(e);
        Self { words: vec![(w, b)] }
    }

    #[inline]
    fn split(e: u32) -> (u32, u64) {
        (e >> 6, 1u64 << (e & 63))
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }

    /// Number of elements.
    pub fn len(&self) -> usize {
        self.words.iter().map(|(_, b)| b.count_ones() as usize).sum()
    }

    pub fn clear(&mut self) {
        self.words.clear();
    }

    pub fn contains(&self, e: u32) -> bool {
        let (w, b) = Self::split(e);
        match self.words.binary_search_by_key(&w, |&(k, _)| k) {
            Ok(i) => self.words[i].1 & b != 0,
            Err(_) => false,
        }
    }

    /// Insert `e`; returns whether the set changed.
    pub fn insert(&mut self, e: u32) -> bool {
        let (w, b) = Self::split(e);
        match self.words.binary_search_by_key(&w, |&(k, _)| k) {
            Ok(i) => {
                let old = self.words[i].1;
                self.words[i].1 = old | b;
                old & b == 0
            }
            Err(i) => {
                self.words.insert(i, (w, b));
                true
            }
        }
    }

    /// Remove `e`; returns whether the set changed.
    pub fn remove(&mut self, e: u32) -> bool {
        let (w, b) = Self::split(e);
        match self.words.binary_search_by_key(&w, |&(k, _)| k) {
            Ok(i) => {
                let old = self.words[i].1;
                if old & b == 0 {
                    return false;
                }
                let new = old & !b;
                if new == 0 {
                    self.words.remove(i);
                } else {
                    self.words[i].1 = new;
                }
                true
            }
            Err(_) => false,
        }
    }

    /// `self ⊆ other`.
    pub fn is_subset(&self, other: &Self) -> bool {
        let (mut i, mut j) = (0, 0);
        while i < self.words.len() {
            let (wa, ba) = self.words[i];
            while j < other.words.len() && other.words[j].0 < wa {
                j += 1;
            }
            if j == other.words.len() || other.words[j].0 != wa || ba & !other.words[j].1 != 0 {
                return false;
            }
            i += 1;
        }
        true
    }

    /// `self ∪= other`; returns whether the set changed. Allocates only on change.
    pub fn union_with(&mut self, other: &Self) -> bool {
        if other.words.is_empty() {
            return false;
        }
        if self.words.is_empty() {
            self.words = other.words.clone();
            return true;
        }
        if other.is_subset(self) {
            return false;
        }
        let mut merged = Vec::with_capacity(self.words.len() + other.words.len());
        let (mut i, mut j) = (0, 0);
        while i < self.words.len() && j < other.words.len() {
            let (wa, ba) = self.words[i];
            let (wb, bb) = other.words[j];
            if wa < wb {
                merged.push((wa, ba));
                i += 1;
            } else if wb < wa {
                merged.push((wb, bb));
                j += 1;
            } else {
                merged.push((wa, ba | bb));
                i += 1;
                j += 1;
            }
        }
        merged.extend_from_slice(&self.words[i..]);
        merged.extend_from_slice(&other.words[j..]);
        self.words = merged;
        true
    }

    /// `self \ other`.
    pub fn difference(&self, other: &Self) -> Self {
        let mut out = Vec::with_capacity(self.words.len());
        let mut j = 0;
        for &(wa, ba) in &self.words {
            while j < other.words.len() && other.words[j].0 < wa {
                j += 1;
            }
            let bits = if j < other.words.len() && other.words[j].0 == wa {
                ba & !other.words[j].1
            } else {
                ba
            };
            if bits != 0 {
                out.push((wa, bits));
            }
        }
        Self { words: out }
    }

    /// `self ∩ other`.
    pub fn intersection(&self, other: &Self) -> Self {
        let mut out = Vec::new();
        let (mut i, mut j) = (0, 0);
        while i < self.words.len() && j < other.words.len() {
            let (wa, ba) = self.words[i];
            let (wb, bb) = other.words[j];
            if wa < wb {
                i += 1;
            } else if wb < wa {
                j += 1;
            } else {
                if ba & bb != 0 {
                    out.push((wa, ba & bb));
                }
                i += 1;
                j += 1;
            }
        }
        Self { words: out }
    }

    /// `self ∩= other`.
    pub fn intersect_with(&mut self, other: &Self) {
        *self = self.intersection(other);
    }

    /// `self ∩ other ≠ ∅`.
    pub fn intersects(&self, other: &Self) -> bool {
        let (mut i, mut j) = (0, 0);
        while i < self.words.len() && j < other.words.len() {
            let (wa, ba) = self.words[i];
            let (wb, bb) = other.words[j];
            if wa < wb {
                i += 1;
            } else if wb < wa {
                j += 1;
            } else {
                if ba & bb != 0 {
                    return true;
                }
                i += 1;
                j += 1;
            }
        }
        false
    }

    /// Smallest element.
    pub fn first(&self) -> Option<u32> {
        self.words
            .first()
            .map(|&(w, b)| (w << 6) | b.trailing_zeros())
    }

    /// Elements in increasing order.
    pub fn iter(&self) -> SparseBitIter<'_> {
        SparseBitIter {
            words: &self.words,
            index: 0,
            current: self.words.first().map(|&(_, b)| b).unwrap_or(0),
        }
    }

    /// Elements as object ids.
    pub fn objects(&self) -> impl Iterator<Item = ObjId> + '_ {
        self.iter().map(ObjId)
    }
}

impl FromIterator<u32> for SparseBitSet {
    fn from_iter<I: IntoIterator<Item = u32>>(iter: I) -> Self {
        let mut elems: Vec<u32> = iter.into_iter().collect();
        elems.sort_unstable();
        elems.dedup();
        let mut words: Vec<(u32, u64)> = Vec::new();
        for e in elems {
            let (w, b) = Self::split(e);
            match words.last_mut() {
                Some(last) if last.0 == w => last.1 |= b,
                _ => words.push((w, b)),
            }
        }
        Self { words }
    }
}

/// Iterator over a [`SparseBitSet`].
pub struct SparseBitIter<'a> {
    words: &'a [(u32, u64)],
    index: usize,
    current: u64,
}

impl Iterator for SparseBitIter<'_> {
    type Item = u32;

    #[inline]
    fn next(&mut self) -> Option<u32> {
        loop {
            if self.index >= self.words.len() {
                return None;
            }
            if self.current != 0 {
                let bit = self.current.trailing_zeros();
                self.current &= self.current - 1;
                return Some((self.words[self.index].0 << 6) | bit);
            }
            self.index += 1;
            if self.index < self.words.len() {
                self.current = self.words[self.index].1;
            }
        }
    }
}

/// The shared empty set returned by lookups of locations that hold nothing.
pub static EMPTY_SET: SparseBitSet = SparseBitSet::new();
