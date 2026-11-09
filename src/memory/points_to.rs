//! Field-Sensitive Inclusion-Based (Andersen) Points-To Analysis over the IR (§M.2).
//!
//! **Constraint graph.** Every SSA value of every function instance and every abstract cell
//! is a node `n` with a points-to set `pts(n)` of abstract objects. The IR generates:
//!
//! - `Alloc`, `MakeClosure`, allocating intrinsics, string constants: `o ∈ pts(dst)`;
//! - `Assign`, `Phi`, `Cast`, `Unary`, `Binary` (pointer arithmetic keeps the pointee):
//!   the copy edge `pts(dst) ⊇ pts(src)`; a checked cast filters by dynamic type when the
//!   frontend declares casts checked;
//! - `Load dst, p.f` (complex): `∀o ∈ pts(p): pts(dst) ⊇ pts(o.f)`;
//! - `Store p.f, v` (complex): `∀o ∈ pts(p): pts(o.f) ⊇ pts(v)`; `Index` places use the
//!   summarized element slot, `Deref`/`Local` places the content slot, globals and statics
//!   their single cell;
//! - `Call`: per resolved target `pts(param_i) ⊇ pts(arg_i)`, `pts(dst) ⊇ pts(ret)`, and the
//!   callee's escaping exceptions flow to the caller's handler or out of the caller;
//!   `MakeClosure` captures are stored in the closure's capture slots and bound to the
//!   target's leading parameters whenever the closure is invoked.
//!
//! **Solver.** A worklist algorithm with *difference propagation*: each node remembers the
//! part of its set it has already pushed along its edges (`done`) and only the delta is
//! processed again, so every object crosses every edge once. Complex constraints add copy
//! edges as objects arrive; a new edge carries the full source set once. Cycles of copy
//! edges are collapsed with *lazy cycle detection* (Hardekopf & Lin, "The Ant and the
//! Grasshopper", PLDI 2007): when propagation across `n → z` leaves `pts(n) == pts(z)` for
//! an edge not checked before, a Tarjan search from `z` finds the cycle and its nodes are
//! unified (union-find with path compression). Sets are sparse bit vectors.
//!
//! **On-the-fly call graph.** Only functions reached from the entry points are analyzed. A
//! virtual call is a trigger on its receiver node: every object arriving there is
//! dispatched through its dynamic type and only that object is bound to the callee's
//! receiver parameter. Indirect calls dispatch on closure objects. Everything outside the
//! module is the *external world*: one node (the content slot of the `Unknown` object)
//! that holds every object external code can reach; under the sound external policy the
//! slots of those objects are linked with it both ways and the methods those objects
//! implement for external declarations are invoked back from it.
//!
//! **Contexts.** `ContextPolicy::CallSite` analyzes a function once per calling site,
//! `ContextPolicy::Object` once per receiver allocation point (static calls keep the
//! caller's context); with `heap_context` the objects a function allocates inherit its
//! context (1-call-site+H / 1-object+H). A per-function budget folds further contexts into
//! the insensitive instance.

use std::collections::VecDeque;

use crate::core::logger::{log_debug, log_info};
use crate::ir::func::{BlockIr, FuncIr, FuncKind};
use crate::ir::inst::{Callee, Inst, IntrinsicKind, Region, Terminator};
use crate::ir::module::ModuleIr;
use crate::ir::types::{AllocSiteId, BlockId, CallSiteId, FuncId, Span, TypeId, ValueId};
use crate::ir::value::{ConstVal, Operand, Place};
use crate::memory::callgraph::{
    CallGraph, CallGraphBuilder, CallKind, CallbackEdge, CallbackOrigin, ContextEdge, Dispatch,
    EdgePrecision, ExternalTarget, FallbackReason, FunctionInstance, InstanceId, MethodFacts,
    TypeHierarchy, UnresolvedReason,
};
use crate::memory::location::{
    is_array_type, AbstractObject, Cell, CellId, ClosureSiteId, Context, FxHashSet, HeapBase,
    LocationTable, ObjId, ObjectInfo, Slot, SparseBitSet, EMPTY_SET,
};
use crate::psa::bdd::fxhash::FxHashMap;

// ── Configuration ───────────────────────────────────────────────────────────

/// Context sensitivity of function instances (and, with `heap_context`, of objects).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ContextPolicy {
    /// One instance per function.
    Insensitive,
    /// 1-call-site sensitivity: one instance per calling site.
    CallSite,
    /// 1-object sensitivity: one instance per receiver allocation point; static calls keep
    /// the caller's context.
    Object,
}

/// Where execution can start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryPoints {
    /// The listed functions (program entry points, framework handlers, tests) as the
    /// project model derives them. Runtime-invoked initializers (`FuncKind::StaticInit`,
    /// `FuncKind::ModuleInit`) are always added.
    Explicit(Vec<FuncId>),
    /// Library view: every function that is not a closure may be called from outside with
    /// arguments of external origin.
    OpenWorld,
}

/// What code outside the module may do with what it is handed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalPolicy {
    /// External code may retain, mutate and return everything reachable from what it
    /// receives and may call back every method it can name (sound).
    Sound,
    /// External calls return objects of external origin and neither retain nor mutate their
    /// arguments. An explicit, unsound assumption; every result records it.
    ReturnUnknownOnly,
}

/// Points-to semantics of an intrinsic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntrinsicEffect {
    /// No pointer flow (arithmetic, length queries, assertions, lock acquisition).
    Pure,
    /// The result aliases argument `i`.
    ReturnsArg(u32),
    /// The result is a fresh object created by the intrinsic.
    Allocates { dynamic_type: Option<u32> },
    /// The result is an object of external origin.
    ReturnsUnknown,
    /// The result may be any argument from index `from` on, or an object of external
    /// origin (an operator whose semantics the frontend could not name).
    MergesArgs { from: u32 },
    /// Every slot of the objects argument `src` points to flows into the same slot of the
    /// objects argument `dst` points to (memcpy, array copy, aggregate assignment).
    CopiesContents { dst: u32, src: u32 },
    /// Argument `value` is stored into the element slot of the objects `array` points to.
    StoresElement { array: u32, value: u32 },
    /// The result is loaded from the element slot of the objects `array` points to.
    LoadsElement { array: u32 },
    /// The result is the next element of argument `iterable`: the element slot of an array,
    /// and an object of external origin for any other iterable (its protocol is not named).
    IteratesElements { iterable: u32 },
    /// Calls the function value in argument `callee` with the arguments at `args`
    /// (thread spawns, scheduled callbacks).
    Invokes { callee: u32, args: Vec<u32> },
    /// The result is a variadic argument of the enclosing function.
    VarArg,
    /// The result is the exception being handled.
    CurrentException,
    /// Semantics unknown: the arguments are handed to external code and the result comes
    /// from it.
    Opaque,
}

impl IntrinsicEffect {
    /// Semantics of the IR's language-neutral intrinsics (`IntrinsicKind`).
    pub fn of_kind(kind: IntrinsicKind) -> Self {
        match kind {
            IntrinsicKind::Exception => IntrinsicEffect::CurrentException,
            IntrinsicKind::MonitorEnter
            | IntrinsicKind::IterHasNext
            | IntrinsicKind::ArrayLength
            | IntrinsicKind::AllocLength
            | IntrinsicKind::Assert => IntrinsicEffect::Pure,
            // The iterator stands for its iterable: `iter.next` reads the iterable's elements.
            IntrinsicKind::IterBegin => IntrinsicEffect::ReturnsArg(0),
            IntrinsicKind::IterNext => IntrinsicEffect::IteratesElements { iterable: 0 },
            // A runtime object denoting a type, created by the runtime once per type literal.
            IntrinsicKind::TypeLiteral => IntrinsicEffect::Allocates { dynamic_type: None },
            IntrinsicKind::OpaqueLiteral | IntrinsicKind::UnresolvedName => {
                IntrinsicEffect::ReturnsUnknown
            }
            // `op.unknown(Str(operator), operands..)`.
            IntrinsicKind::UnknownOperator => IntrinsicEffect::MergesArgs { from: 1 },
            IntrinsicKind::Opaque => IntrinsicEffect::Opaque,
        }
    }
}

/// Semantics of the intrinsic named `name`: the frontend's model, else the IR's built-in
/// kind, else `Opaque`.
pub fn intrinsic_effect(module: &ModuleIr, config: &MemoryConfig, name: u32) -> IntrinsicEffect {
    match config.intrinsics.get(&name) {
        Some(e) => e.clone(),
        None => match module.intrinsic_kind(name) {
            Some(kind) => IntrinsicEffect::of_kind(kind),
            None => IntrinsicEffect::Opaque,
        },
    }
}

/// Configuration of one points-to solve.
#[derive(Debug, Clone)]
pub struct MemoryConfig {
    pub context: ContextPolicy,
    /// Qualify allocation points with the allocating instance's context.
    pub heap_context: bool,
    /// Contexts per function before further contexts fold into the insensitive instance.
    pub max_contexts_per_function: usize,
    pub entry_points: EntryPoints,
    pub external: ExternalPolicy,
    /// Supertype, method and override facts from the frontends.
    pub hierarchy: TypeHierarchy,
    /// Frontend semantics of intrinsics by interned name; they take precedence over the
    /// built-in `IntrinsicKind` semantics. Unmodelled intrinsics are `Opaque`.
    pub intrinsics: FxHashMap<u32, IntrinsicEffect>,
    /// Dynamic type of string literals (the language's string class), when the frontend
    /// declares one.
    pub string_type: Option<u32>,
    /// Casts to class types are checked at run time (JVM, CLR), so a cast passes only
    /// objects of a subtype. Must stay off for languages with unchecked pointer casts.
    pub filter_checked_casts: bool,
    /// Work budget: worklist node visits before the solve stops as `Truncated`.
    pub max_propagations: u64,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            context: ContextPolicy::Object,
            heap_context: true,
            max_contexts_per_function: 1024,
            entry_points: EntryPoints::OpenWorld,
            external: ExternalPolicy::Sound,
            hierarchy: TypeHierarchy::default(),
            intrinsics: FxHashMap::default(),
            string_type: None,
            filter_checked_casts: false,
            max_propagations: 500_000_000,
        }
    }
}

// ── Result ──────────────────────────────────────────────────────────────────

/// Whether the solve reached its fixpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SolveStatus {
    Complete,
    /// The work budget ran out: every set is a subset of the fixpoint, so facts derived
    /// from the result are partial.
    Truncated { propagations: u64 },
}

/// Solver counters.
#[derive(Debug, Clone, Copy, Default)]
pub struct SolverStats {
    pub nodes: usize,
    pub copy_edges: usize,
    pub objects: usize,
    pub cells: usize,
    pub instances: usize,
    pub propagations: u64,
    pub cycle_searches: usize,
    pub collapsed_nodes: usize,
    pub context_overflows: usize,
    /// Explicit entry points that name no function of the module.
    pub missing_entries: usize,
}

/// An intrinsic whose semantics nobody modelled (treated as external code).
#[derive(Debug, Clone, Copy)]
pub struct OpaqueIntrinsic {
    pub func: FuncId,
    pub block: BlockId,
    pub inst: u32,
    pub name: u32,
    pub span: Span,
}

/// Points-to facts and the call graph of one solve.
#[derive(Debug, Clone)]
pub struct PointsToResult {
    pub locations: LocationTable,
    pub call_graph: CallGraph,
    pub status: SolveStatus,
    pub stats: SolverStats,
    pub context: ContextPolicy,
    pub external: ExternalPolicy,
    pub opaque_intrinsics: Vec<OpaqueIntrinsic>,
    sets: Vec<SparseBitSet>,
    values: FxHashMap<(FuncId, ValueId), u32>,
    instance_values: FxHashMap<(InstanceId, ValueId), u32>,
    returns: FxHashMap<FuncId, u32>,
    thrown: FxHashMap<FuncId, u32>,
    instance_returns: Vec<u32>,
    instance_thrown: Vec<u32>,
    cells: Vec<u32>,
    instances_by_func: FxHashMap<FuncId, Vec<InstanceId>>,
    string_literal: Option<ObjId>,
    string_set: u32,
    external_cell: CellId,
}

impl PointsToResult {
    #[inline]
    fn set(&self, index: u32) -> &SparseBitSet {
        self.sets.get(index as usize).unwrap_or(&EMPTY_SET)
    }

    /// The solve reached its fixpoint.
    #[inline]
    pub fn is_complete(&self) -> bool {
        self.status == SolveStatus::Complete
    }

    /// Objects `value` of `func` may point to, over all contexts.
    pub fn points_to(&self, func: FuncId, value: ValueId) -> &SparseBitSet {
        self.values
            .get(&(func, value))
            .map(|&s| self.set(s))
            .unwrap_or(&EMPTY_SET)
    }

    /// Objects `value` may point to in one function instance.
    pub fn points_to_in(&self, instance: InstanceId, value: ValueId) -> &SparseBitSet {
        self.instance_values
            .get(&(instance, value))
            .map(|&s| self.set(s))
            .unwrap_or(&EMPTY_SET)
    }

    /// Objects an operand may denote: its value's set, the string pool for string
    /// constants, nothing for other constants.
    pub fn operand_points_to(&self, func: FuncId, op: &Operand) -> &SparseBitSet {
        match op {
            Operand::Value(v) => self.points_to(func, *v),
            Operand::Const(ConstVal::Str(_)) => self.set(self.string_set),
            _ => &EMPTY_SET,
        }
    }

    /// Objects `func` may return, over all contexts.
    pub fn return_points_to(&self, func: FuncId) -> &SparseBitSet {
        self.returns
            .get(&func)
            .map(|&s| self.set(s))
            .unwrap_or(&EMPTY_SET)
    }

    /// Exception objects that may leave `func`, over all contexts.
    pub fn thrown_points_to(&self, func: FuncId) -> &SparseBitSet {
        self.thrown
            .get(&func)
            .map(|&s| self.set(s))
            .unwrap_or(&EMPTY_SET)
    }

    pub fn return_points_to_in(&self, instance: InstanceId) -> &SparseBitSet {
        self.instance_returns
            .get(instance.0 as usize)
            .map(|&s| self.set(s))
            .unwrap_or(&EMPTY_SET)
    }

    pub fn thrown_points_to_in(&self, instance: InstanceId) -> &SparseBitSet {
        self.instance_thrown
            .get(instance.0 as usize)
            .map(|&s| self.set(s))
            .unwrap_or(&EMPTY_SET)
    }

    /// Objects a cell may hold.
    pub fn cell_points_to(&self, cell: CellId) -> &SparseBitSet {
        self.cells
            .get(cell.0 as usize)
            .map(|&s| self.set(s))
            .unwrap_or(&EMPTY_SET)
    }

    /// Objects a slot of an object may hold.
    pub fn slot_points_to(&self, obj: ObjId, slot: Slot) -> &SparseBitSet {
        match self.locations.find_cell(&Cell::Object { obj, slot }) {
            Some(c) => self.cell_points_to(c),
            None => &EMPTY_SET,
        }
    }

    /// Cells a place of `func` may denote.
    pub fn place_cells(&self, func: FuncId, place: &Place) -> Vec<CellId> {
        let slot_cells = |base: ValueId, slot: Slot| -> Vec<CellId> {
            self.points_to(func, base)
                .objects()
                .filter_map(|obj| self.locations.find_cell(&Cell::Object { obj, slot }))
                .collect()
        };
        match place {
            Place::Local(v) | Place::Deref(v) => slot_cells(*v, Slot::Content),
            Place::Field { base, field } => slot_cells(*base, Slot::Field(*field)),
            Place::Index { base, .. } => slot_cells(*base, Slot::Element),
            Place::Global(g) => self
                .locations
                .find_cell(&Cell::Global(*g))
                .into_iter()
                .collect(),
            Place::Static { owner, field } => self
                .locations
                .find_cell(&Cell::Static {
                    owner: *owner,
                    field: *field,
                })
                .into_iter()
                .collect(),
        }
    }

    /// Objects a load from `place` in `func` may produce.
    pub fn place_points_to(&self, func: FuncId, place: &Place) -> SparseBitSet {
        let mut out = SparseBitSet::new();
        for c in self.place_cells(func, place) {
            out.union_with(self.cell_points_to(c));
        }
        out
    }

    /// Everything external code may hold (the content slot of the `Unknown` object).
    pub fn external_world(&self) -> &SparseBitSet {
        self.cell_points_to(self.external_cell)
    }

    #[inline]
    pub fn external_cell(&self) -> CellId {
        self.external_cell
    }

    /// Function instances the solver created for `func`.
    pub fn instances_of(&self, func: FuncId) -> &[InstanceId] {
        self.instances_by_func
            .get(&func)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// The string-literal pool object, when the program uses string constants.
    #[inline]
    pub fn string_literal(&self) -> Option<ObjId> {
        self.string_literal
    }

    /// Distinct dynamic types of the objects `value` may point to; `None` in the list when
    /// some pointee has no derivable dynamic type.
    pub fn dynamic_types(&self, func: FuncId, value: ValueId) -> Vec<Option<u32>> {
        let mut out: Vec<Option<u32>> = self
            .points_to(func, value)
            .objects()
            .map(|o| self.locations.object(o).dynamic_type)
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// `value` may denote an object the program text does not create (an entry-point
    /// argument or an object of external origin).
    pub fn may_be_opaque(&self, func: FuncId, value: ValueId) -> bool {
        self.points_to(func, value)
            .objects()
            .any(|o| self.locations.object(o).object.base.is_opaque())
    }
}

/// Run the points-to analysis (and build the call graph) over a module.
pub fn solve(module: &ModuleIr, config: &MemoryConfig) -> PointsToResult {
    log_info("══► Starting Stage: whole-program points-to analysis and on-the-fly call graph...");
    let mut solver = Solver::new(module, config);
    solver.seed_entries();
    solver.run();
    let result = solver.finish();
    log_info(&format!(
        "Points-to solve {}: {} instances of {} reachable functions, {} objects, {} cells, {} nodes ({} collapsed by {} cycle searches), {} propagations, {} call edges.",
        if result.is_complete() { "complete" } else { "TRUNCATED at the work budget" },
        result.stats.instances,
        result.call_graph.reachable.len(),
        result.stats.objects,
        result.stats.cells,
        result.stats.nodes,
        result.stats.collapsed_nodes,
        result.stats.cycle_searches,
        result.stats.propagations,
        result.call_graph.edges.len()
    ));
    result
}

// ── Solver state ────────────────────────────────────────────────────────────

const NONE: u32 = u32::MAX;

/// A constraint-graph node.
#[derive(Debug, Default)]
struct Node {
    pts: SparseBitSet,
    /// Part of `pts` already propagated along every edge and constraint of the node.
    done: SparseBitSet,
    /// Copy successors: `pts(succ) ⊇ pts(self)`.
    copy: Vec<u32>,
    /// `∀o ∈ pts(self): pts(dst) ⊇ pts(o.slot)`.
    loads: Vec<(Slot, u32)>,
    /// `∀o ∈ pts(self): pts(o.slot) ⊇ pts(src)`.
    stores: Vec<(Slot, u32)>,
    /// Checked casts: `pts(dst) ⊇ {o ∈ pts(self) | o's type <: ty}`.
    filters: Vec<(u32, u32)>,
    /// Triggers fired once per arriving object.
    triggers: Vec<u32>,
}

/// What a trigger does for each object arriving at its node.
#[derive(Debug, Clone, Copy)]
enum TriggerKind {
    /// Virtual / interface dispatch of `method` on the receiver object.
    Dispatch { method: u32 },
    /// Non-virtual call of an instance method, contexts from the receiver objects.
    StaticReceiver { callee: usize },
    /// Call through a function value.
    Indirect,
    /// `iter.next`: the elements of the iterable object flow into `dst`.
    IterElements { dst: u32 },
    /// Copy intrinsic, source side of pair `pair`.
    CopySource { pair: u32 },
    /// Copy intrinsic, destination side of pair `pair`.
    CopyTarget { pair: u32 },
    /// The object reached the external world.
    ExternalWorld,
}

/// A trigger with the call it belongs to.
#[derive(Debug, Clone)]
struct Trigger {
    kind: TriggerKind,
    caller: Option<u32>,
    site: Option<usize>,
    call_site: Option<CallSiteId>,
    args: Vec<Option<u32>>,
    /// The node the trigger watches when it is a receiver or a called value.
    receiver: Option<u32>,
    dst: Option<u32>,
    /// The call has a landing block: callee exceptions go to the caller's handler.
    unwind: bool,
    origin: Option<CallbackOrigin>,
    precision: EdgePrecision,
    span: Span,
    fired: bool,
}

impl Trigger {
    fn effect(kind: TriggerKind) -> Self {
        Self {
            kind,
            caller: None,
            site: None,
            call_site: None,
            args: Vec::new(),
            receiver: None,
            dst: None,
            unwind: false,
            origin: None,
            precision: EdgePrecision::Exact,
            span: Span::NONE,
            fired: false,
        }
    }
}

/// How one resolved target is bound.
#[derive(Debug, Clone)]
struct CallBinding {
    caller: Option<u32>,
    site: Option<usize>,
    call_site: Option<CallSiteId>,
    args: Vec<Option<u32>>,
    /// Leading arguments that are not bound (the function value of a dispatch call that
    /// invokes a closure).
    skip_args: usize,
    /// The dispatched object, bound alone to the callee's receiver parameter.
    receiver: Option<ObjId>,
    /// The invoked closure object, whose capture slots feed the leading parameters.
    closure: Option<ObjId>,
    dst: Option<u32>,
    unwind: bool,
    precision: EdgePrecision,
    /// Invoked by external code: parameters receive the external world, results flow to it.
    from_external: bool,
    origin: Option<CallbackOrigin>,
    span: Span,
}

impl CallBinding {
    fn from_trigger(
        t: &Trigger,
        receiver: Option<ObjId>,
        closure: Option<ObjId>,
        precision: EdgePrecision,
    ) -> Self {
        Self {
            caller: t.caller,
            site: t.site,
            call_site: t.call_site,
            args: t.args.clone(),
            skip_args: 0,
            receiver,
            closure,
            dst: t.dst,
            unwind: t.unwind,
            precision,
            from_external: false,
            origin: t.origin,
            span: t.span,
        }
    }

    fn external(receiver: Option<ObjId>, closure: Option<ObjId>, origin: CallbackOrigin) -> Self {
        Self {
            caller: None,
            site: None,
            call_site: None,
            args: Vec::new(),
            skip_args: 0,
            receiver,
            closure,
            dst: None,
            unwind: false,
            precision: EdgePrecision::Exact,
            from_external: true,
            origin: Some(origin),
            span: Span::NONE,
        }
    }
}

/// A function analyzed in one context, with its interface nodes.
#[derive(Debug, Clone, Copy)]
struct InstanceData {
    func_idx: usize,
    ctx: Context,
    ret: u32,
    /// Exceptions leaving the instance.
    exc: u32,
    /// Exceptions arriving at the instance's landing blocks.
    caught: u32,
    /// Arguments beyond the declared parameters.
    varargs: u32,
}

struct Solver<'m> {
    module: &'m ModuleIr,
    cfg: &'m MemoryConfig,
    func_by_id: FxHashMap<FuncId, usize>,
    func_by_symbol: FxHashMap<u32, Vec<usize>>,
    locs: LocationTable,

    nodes: Vec<Node>,
    parent: Vec<u32>,
    queued: Vec<bool>,
    worklist: VecDeque<u32>,
    edge_set: FxHashSet<(u32, u32)>,
    lcd_checked: FxHashSet<(u32, u32)>,

    var_nodes: FxHashMap<(u32, ValueId), u32>,
    cell_nodes: Vec<u32>,
    string_obj: Option<ObjId>,
    string_node: u32,
    x_node: u32,
    external_marked: SparseBitSet,

    instances: Vec<InstanceData>,
    instance_index: FxHashMap<(usize, Context), u32>,
    contexts_per_func: Vec<u32>,
    pending: VecDeque<u32>,

    triggers: Vec<Trigger>,
    static_receivers: Vec<u32>,
    copy_pairs: Vec<(u32, u32)>,
    mirrors: FxHashMap<ObjId, Vec<ObjId>>,
    bound: FxHashSet<(u32, u32, u32, u32)>,
    external_done: FxHashSet<(u32, ExternalTarget)>,

    dispatch_cache: FxHashMap<(u32, u32), Dispatch>,
    cha_cache: FxHashMap<u32, Vec<u32>>,
    subtype_cache: FxHashMap<(u32, u32), bool>,
    callback_cache: FxHashMap<u32, Vec<(u32, u32)>>,

    cg: CallGraphBuilder,
    opaque_intrinsics: Vec<OpaqueIntrinsic>,
    opaque_seen: FxHashSet<(FuncId, BlockId, u32)>,
    stats: SolverStats,
    truncated: bool,
}

impl<'m> Solver<'m> {
    fn new(module: &'m ModuleIr, cfg: &'m MemoryConfig) -> Self {
        let mut func_by_id = FxHashMap::default();
        let mut func_by_symbol: FxHashMap<u32, Vec<usize>> = FxHashMap::default();
        for (i, f) in module.funcs.iter().enumerate() {
            func_by_id.entry(f.id).or_insert(i);
            if let Some(s) = f.symbol {
                func_by_symbol.entry(s).or_default().push(i);
            }
        }
        let mut s = Self {
            module,
            cfg,
            func_by_id,
            func_by_symbol,
            locs: LocationTable::new(),
            nodes: Vec::new(),
            parent: Vec::new(),
            queued: Vec::new(),
            worklist: VecDeque::new(),
            edge_set: FxHashSet::default(),
            lcd_checked: FxHashSet::default(),
            var_nodes: FxHashMap::default(),
            cell_nodes: Vec::new(),
            string_obj: None,
            string_node: NONE,
            x_node: NONE,
            external_marked: SparseBitSet::new(),
            instances: Vec::new(),
            instance_index: FxHashMap::default(),
            contexts_per_func: vec![0; module.funcs.len()],
            pending: VecDeque::new(),
            triggers: Vec::new(),
            static_receivers: Vec::new(),
            copy_pairs: Vec::new(),
            mirrors: FxHashMap::default(),
            bound: FxHashSet::default(),
            external_done: FxHashSet::default(),
            dispatch_cache: FxHashMap::default(),
            cha_cache: FxHashMap::default(),
            subtype_cache: FxHashMap::default(),
            callback_cache: FxHashMap::default(),
            cg: CallGraphBuilder::new(),
            opaque_intrinsics: Vec::new(),
            opaque_seen: FxHashSet::default(),
            stats: SolverStats::default(),
            truncated: false,
        };
        // The external world: the content slot of `Unknown`, holding `Unknown` itself.
        let unknown = s.locs.unknown();
        let (xc, _) = s.locs.intern_cell(Cell::Object {
            obj: unknown,
            slot: Slot::Content,
        });
        let x = s.alloc_cell_node(xc);
        s.x_node = x;
        s.add_obj(x, unknown);
        s.add_trigger(x, Trigger::effect(TriggerKind::ExternalWorld));
        s
    }

    // ── Union-find and nodes ──

    fn new_node(&mut self) -> u32 {
        let id = self.nodes.len() as u32;
        self.nodes.push(Node::default());
        self.parent.push(id);
        self.queued.push(false);
        id
    }

    fn find(&mut self, mut n: u32) -> u32 {
        let mut root = n;
        while self.parent[root as usize] != root {
            root = self.parent[root as usize];
        }
        while self.parent[n as usize] != root {
            let next = self.parent[n as usize];
            self.parent[n as usize] = root;
            n = next;
        }
        root
    }

    fn push(&mut self, n: u32) {
        let n = self.find(n);
        if !self.queued[n as usize] {
            self.queued[n as usize] = true;
            self.worklist.push_back(n);
        }
    }

    fn add_obj(&mut self, node: u32, o: ObjId) {
        let n = self.find(node);
        if self.nodes[n as usize].pts.insert(o.0) {
            self.push(n);
        }
    }

    /// `pts(to) ∪= pts(from)` for two distinct representatives.
    fn union_pts(&mut self, from: u32, to: u32) -> bool {
        let (f, t) = (from as usize, to as usize);
        if f == t {
            return false;
        }
        let (src, dst) = if f < t {
            let (lo, hi) = self.nodes.split_at_mut(t);
            (&lo[f], &mut hi[0])
        } else {
            let (lo, hi) = self.nodes.split_at_mut(f);
            (&hi[0], &mut lo[t])
        };
        dst.pts.union_with(&src.pts)
    }

    /// Copy edge `pts(to) ⊇ pts(from)`; a new edge carries the full source set once.
    fn add_edge(&mut self, from: u32, to: u32) {
        let a = self.find(from);
        let b = self.find(to);
        if a == b || !self.edge_set.insert((a, b)) {
            return;
        }
        self.nodes[a as usize].copy.push(b);
        self.stats.copy_edges += 1;
        if self.union_pts(a, b) {
            self.push(b);
        }
    }

    fn var(&mut self, inst: u32, value: ValueId) -> u32 {
        if let Some(&n) = self.var_nodes.get(&(inst, value)) {
            return n;
        }
        let n = self.new_node();
        self.var_nodes.insert((inst, value), n);
        n
    }

    fn operand_node(&mut self, inst: u32, op: &Operand) -> Option<u32> {
        match op {
            Operand::Value(v) => Some(self.var(inst, *v)),
            Operand::Const(ConstVal::Str(_)) => Some(self.string_literal_node()),
            _ => None,
        }
    }

    fn arg_node(&mut self, inst: u32, args: &[Operand], index: u32) -> Option<u32> {
        let op = args.get(index as usize)?;
        self.operand_node(inst, op)
    }

    fn flow(&mut self, inst: u32, op: &Operand, dst: u32) {
        if let Some(n) = self.operand_node(inst, op) {
            self.add_edge(n, dst);
        }
    }

    fn string_literal_node(&mut self) -> u32 {
        if self.string_node != NONE {
            return self.string_node;
        }
        let (o, _) = self.locs.intern_object(ObjectInfo {
            object: AbstractObject {
                base: HeapBase::StringLiteral,
                context: Context::Empty,
            },
            ty: None,
            dynamic_type: self.cfg.string_type,
            region: Some(Region::Static),
            func: None,
            span: Span::NONE,
        });
        self.string_obj = Some(o);
        let n = self.new_node();
        self.add_obj(n, o);
        self.string_node = n;
        n
    }

    // ── Cells ──

    fn existing_cell_node(&self, cid: CellId) -> Option<u32> {
        match self.cell_nodes.get(cid.0 as usize) {
            Some(&n) if n != NONE => Some(n),
            _ => None,
        }
    }

    fn alloc_cell_node(&mut self, cid: CellId) -> u32 {
        let idx = cid.0 as usize;
        if self.cell_nodes.len() <= idx {
            self.cell_nodes.resize(idx + 1, NONE);
        }
        let n = self.new_node();
        self.cell_nodes[idx] = n;
        n
    }

    /// Node of a cell. A new slot of an object the external world holds is linked with the
    /// external world; a new slot of a copy source is mirrored into its copy targets.
    fn cell_node(&mut self, cid: CellId) -> u32 {
        if let Some(n) = self.existing_cell_node(cid) {
            return n;
        }
        let first = self.alloc_cell_node(cid);
        let mut stack = vec![(cid, first)];
        while let Some((c, cn)) = stack.pop() {
            let Cell::Object { obj, slot } = self.locs.cell(c) else {
                continue;
            };
            if self.external_marked.contains(obj.0) {
                let x = self.x_node;
                self.add_edge(cn, x);
                self.add_edge(x, cn);
            }
            if let Some(targets) = self.mirrors.get(&obj).cloned() {
                for p in targets {
                    let (pc, _) = self.locs.intern_cell(Cell::Object { obj: p, slot });
                    let pn = match self.existing_cell_node(pc) {
                        Some(n) => n,
                        None => {
                            let n = self.alloc_cell_node(pc);
                            stack.push((pc, n));
                            n
                        }
                    };
                    self.add_edge(cn, pn);
                }
            }
        }
        first
    }

    fn slot_node(&mut self, obj: ObjId, slot: Slot) -> u32 {
        let (cid, _) = self.locs.intern_cell(Cell::Object { obj, slot });
        self.cell_node(cid)
    }

    fn fixed_cell_node(&mut self, cell: Cell) -> u32 {
        let (cid, _) = self.locs.intern_cell(cell);
        self.cell_node(cid)
    }

    /// Contents of `from` flow into the same slots of `to` (copy intrinsics).
    fn mirror(&mut self, from: ObjId, to: ObjId) {
        if from == to {
            return;
        }
        let list = self.mirrors.entry(from).or_default();
        if list.contains(&to) {
            return;
        }
        list.push(to);
        let cells: Vec<CellId> = self.locs.cells_of(from).to_vec();
        for c in cells {
            let Cell::Object { slot, .. } = self.locs.cell(c) else {
                continue;
            };
            let cn = self.cell_node(c);
            let tn = self.slot_node(to, slot);
            self.add_edge(cn, tn);
        }
    }

    /// `o` became reachable by external code.
    fn mark_external(&mut self, o: ObjId) {
        if !self.external_marked.insert(o.0) {
            return;
        }
        let x = self.x_node;
        let cells: Vec<CellId> = self.locs.cells_of(o).to_vec();
        for c in cells {
            let n = self.cell_node(c);
            self.add_edge(n, x);
            self.add_edge(x, n);
        }
        self.external_callbacks(o);
    }

    // ── Complex constraints ──

    fn add_load(&mut self, base: u32, slot: Slot, dst: u32) {
        let b = self.find(base);
        self.nodes[b as usize].loads.push((slot, dst));
        let objs: Vec<u32> = self.nodes[b as usize].pts.iter().collect();
        for o in objs {
            let c = self.slot_node(ObjId(o), slot);
            self.add_edge(c, dst);
        }
    }

    fn add_store(&mut self, base: u32, slot: Slot, src: u32) {
        let b = self.find(base);
        self.nodes[b as usize].stores.push((slot, src));
        let objs: Vec<u32> = self.nodes[b as usize].pts.iter().collect();
        for o in objs {
            let c = self.slot_node(ObjId(o), slot);
            self.add_edge(src, c);
        }
    }

    fn add_filter(&mut self, src: u32, dst: u32, ty: u32) {
        let s = self.find(src);
        self.nodes[s as usize].filters.push((dst, ty));
        let objs: Vec<u32> = self.nodes[s as usize].pts.iter().collect();
        for o in objs {
            if self.passes_filter(ObjId(o), ty) {
                self.add_obj(dst, ObjId(o));
            }
        }
    }

    fn add_trigger(&mut self, node: u32, trigger: Trigger) -> u32 {
        let t = self.triggers.len() as u32;
        self.triggers.push(trigger);
        let n = self.find(node);
        self.nodes[n as usize].triggers.push(t);
        let objs: Vec<u32> = self.nodes[n as usize].pts.iter().collect();
        for o in objs {
            self.fire(t, ObjId(o));
        }
        t
    }

    /// A checked cast lets `o` through to type `ty`: its dynamic type is a subtype, or it
    /// has no type the hierarchy can judge.
    fn passes_filter(&mut self, o: ObjId, ty: u32) -> bool {
        match self.locs.object(o).dynamic_type {
            None => true,
            Some(t) => {
                if !self.cfg.hierarchy.knows_type(t) {
                    return true;
                }
                if let Some(&r) = self.subtype_cache.get(&(t, ty)) {
                    return r;
                }
                let r = self.cfg.hierarchy.is_subtype(t, ty);
                self.subtype_cache.insert((t, ty), r);
                r
            }
        }
    }

    // ── Instances and contexts ──

    fn func_idx(&self, f: FuncId) -> Option<usize> {
        match self.module.funcs.get(f.index()) {
            Some(func) if func.id == f => Some(f.index()),
            _ => self.func_by_id.get(&f).copied(),
        }
    }

    fn instance(&mut self, func_idx: usize, ctx: Context) -> u32 {
        if let Some(&i) = self.instance_index.get(&(func_idx, ctx)) {
            return i;
        }
        let ctx = if ctx != Context::Empty
            && self.contexts_per_func[func_idx] as usize >= self.cfg.max_contexts_per_function
        {
            self.stats.context_overflows += 1;
            Context::Empty
        } else {
            ctx
        };
        if let Some(&i) = self.instance_index.get(&(func_idx, ctx)) {
            return i;
        }
        let id = self.instances.len() as u32;
        let ret = self.new_node();
        let exc = self.new_node();
        let caught = self.new_node();
        let varargs = self.new_node();
        self.instances.push(InstanceData {
            func_idx,
            ctx,
            ret,
            exc,
            caught,
            varargs,
        });
        self.instance_index.insert((func_idx, ctx), id);
        self.contexts_per_func[func_idx] += 1;
        self.cg.mark_reachable(self.module.funcs[func_idx].id);
        self.pending.push_back(id);
        id
    }

    fn heap_context(&self, inst: u32) -> Context {
        if self.cfg.heap_context {
            self.instances[inst as usize].ctx
        } else {
            Context::Empty
        }
    }

    fn callee_context(
        &self,
        caller: Option<u32>,
        call_site: Option<CallSiteId>,
        receiver: Option<ObjId>,
    ) -> Context {
        match self.cfg.context {
            ContextPolicy::Insensitive => Context::Empty,
            ContextPolicy::CallSite => call_site.map(Context::CallSite).unwrap_or(Context::Empty),
            ContextPolicy::Object => match receiver {
                Some(o) => Context::Receiver(self.locs.object(o).object.base),
                None => caller
                    .map(|c| self.instances[c as usize].ctx)
                    .unwrap_or(Context::Empty),
            },
        }
    }

    // ── Entry points ──

    fn seed_entries(&mut self) {
        let module = self.module;
        let cfg = self.cfg;
        let mut entries: Vec<usize> = Vec::new();
        match &cfg.entry_points {
            EntryPoints::Explicit(list) => {
                for &f in list {
                    match self.func_idx(f) {
                        Some(i) => entries.push(i),
                        None => self.stats.missing_entries += 1,
                    }
                }
            }
            EntryPoints::OpenWorld => {
                entries.extend((0..module.funcs.len()).filter(|&i| !module.funcs[i].is_closure));
            }
        }
        // Initializers run because the runtime runs them, not because code calls them.
        for (i, f) in module.funcs.iter().enumerate() {
            if matches!(f.kind, FuncKind::StaticInit | FuncKind::ModuleInit) {
                entries.push(i);
            }
        }
        entries.sort_unstable();
        entries.dedup();
        for idx in entries {
            self.seed_entry(idx);
        }
    }

    fn seed_entry(&mut self, idx: usize) {
        let module = self.module;
        let func = &module.funcs[idx];
        self.cg.add_entry(func.id);
        let inst = self.instance(idx, Context::Empty);
        let x = self.x_node;
        for (k, &p) in func.params.iter().enumerate() {
            let object = AbstractObject {
                base: HeapBase::EntryParam {
                    func: func.id,
                    index: k as u32,
                },
                context: Context::Empty,
            };
            let (o, _) = self.locs.intern_object(ObjectInfo {
                object,
                ty: Some(func.value_type(p)),
                dynamic_type: None,
                region: None,
                func: Some(func.id),
                span: func.span,
            });
            let n = self.var(inst, p);
            self.add_obj(n, o);
            self.add_obj(x, o);
        }
        if self.cfg.external == ExternalPolicy::Sound {
            let data = self.instances[inst as usize];
            self.add_edge(data.ret, x);
            self.add_edge(data.exc, x);
        }
    }

    // ── Constraint generation ──

    fn generate(&mut self, inst: u32) {
        let module = self.module;
        let func = &module.funcs[self.instances[inst as usize].func_idx];
        for block in &func.blocks {
            for (i, ins) in block.insts.iter().enumerate() {
                let span = block.inst_spans.get(i).copied().unwrap_or(Span::NONE);
                self.gen_inst(inst, func, block, i as u32, ins, span);
            }
            self.gen_term(inst, block);
        }
    }

    fn gen_inst(
        &mut self,
        inst: u32,
        func: &'m FuncIr,
        block: &'m BlockIr,
        index: u32,
        ins: &'m Inst,
        span: Span,
    ) {
        match ins {
            Inst::Assign { dst, src } | Inst::Unary { dst, src, .. } => {
                let d = self.var(inst, *dst);
                self.flow(inst, src, d);
            }
            Inst::Binary { dst, lhs, rhs, .. } => {
                let d = self.var(inst, *dst);
                self.flow(inst, lhs, d);
                self.flow(inst, rhs, d);
            }
            Inst::Compare { .. } | Inst::TypeTest { .. } | Inst::Release { .. } => {}
            Inst::Load { dst, place } => {
                let d = self.var(inst, *dst);
                self.gen_load(inst, place, d);
            }
            Inst::Store { place, value } => {
                if let Some(s) = self.operand_node(inst, value) {
                    self.gen_store(inst, place, s);
                }
            }
            Inst::Alloc {
                dst,
                site,
                ty,
                region,
            } => {
                let o = self.alloc_object(inst, func, *site, *ty, *region, span);
                let d = self.var(inst, *dst);
                self.add_obj(d, o);
            }
            Inst::Call {
                dst,
                site,
                callee,
                args,
                unwind,
            } => self.gen_call(inst, func, *site, callee, args, *dst, unwind.is_some(), span),
            Inst::Phi { dst, incoming } => {
                let d = self.var(inst, *dst);
                for (_, op) in incoming {
                    self.flow(inst, op, d);
                }
            }
            Inst::Cast { dst, src, ty } => {
                let d = self.var(inst, *dst);
                let Some(s) = self.operand_node(inst, src) else {
                    return;
                };
                if self.cfg.filter_checked_casts {
                    if let Some(sym) = self.module.types.symbol_of(*ty) {
                        if self.cfg.hierarchy.knows_type(sym) {
                            self.add_filter(s, d, sym);
                            return;
                        }
                    }
                }
                self.add_edge(s, d);
            }
            Inst::MakeClosure {
                dst,
                func: target,
                captures,
            } => self.gen_closure(inst, func, block.id, index, *dst, *target, captures, span),
            Inst::Intrinsic { dst, name, args } => {
                self.gen_intrinsic(inst, func, block.id, index, *dst, *name, args, span)
            }
        }
    }

    fn gen_term(&mut self, inst: u32, block: &'m BlockIr) {
        let data = self.instances[inst as usize];
        match &block.term {
            Terminator::Return(Some(op)) => self.flow(inst, op, data.ret),
            Terminator::Throw(op) => {
                let target = if block.unwind.is_some() {
                    data.caught
                } else {
                    data.exc
                };
                self.flow(inst, op, target);
            }
            Terminator::Return(None)
            | Terminator::Jump(_)
            | Terminator::Branch { .. }
            | Terminator::Switch { .. }
            | Terminator::Unreachable => {}
        }
    }

    fn alloc_object(
        &mut self,
        inst: u32,
        func: &FuncIr,
        site: AllocSiteId,
        ty: TypeId,
        region: Region,
        span: Span,
    ) -> ObjId {
        // Static storage exists once, whatever context reaches its initialization.
        let context = if matches!(region, Region::Static) {
            Context::Empty
        } else {
            self.heap_context(inst)
        };
        let object = AbstractObject {
            base: HeapBase::Site(site),
            context,
        };
        if let Some(o) = self.locs.find_object(&object) {
            return o;
        }
        let span = match self.module.alloc_sites.get(site.index()) {
            Some(a) if a.id == site && !a.span.is_none() => a.span,
            _ => span,
        };
        self.locs
            .intern_object(ObjectInfo {
                object,
                ty: Some(ty),
                dynamic_type: self.module.types.symbol_of(ty),
                region: Some(region),
                func: Some(func.id),
                span,
            })
            .0
    }

    fn gen_load(&mut self, inst: u32, place: &Place, dst: u32) {
        match place {
            Place::Local(v) | Place::Deref(v) => {
                let b = self.var(inst, *v);
                self.add_load(b, Slot::Content, dst);
            }
            Place::Field { base, field } => {
                let b = self.var(inst, *base);
                self.add_load(b, Slot::Field(*field), dst);
            }
            Place::Index { base, .. } => {
                let b = self.var(inst, *base);
                self.add_load(b, Slot::Element, dst);
            }
            Place::Global(g) => {
                let c = self.fixed_cell_node(Cell::Global(*g));
                self.add_edge(c, dst);
            }
            Place::Static { owner, field } => {
                let c = self.fixed_cell_node(Cell::Static {
                    owner: *owner,
                    field: *field,
                });
                self.add_edge(c, dst);
            }
        }
    }

    fn gen_store(&mut self, inst: u32, place: &Place, src: u32) {
        match place {
            Place::Local(v) | Place::Deref(v) => {
                let b = self.var(inst, *v);
                self.add_store(b, Slot::Content, src);
            }
            Place::Field { base, field } => {
                let b = self.var(inst, *base);
                self.add_store(b, Slot::Field(*field), src);
            }
            Place::Index { base, .. } => {
                let b = self.var(inst, *base);
                self.add_store(b, Slot::Element, src);
            }
            Place::Global(g) => {
                let c = self.fixed_cell_node(Cell::Global(*g));
                self.add_edge(src, c);
            }
            Place::Static { owner, field } => {
                let c = self.fixed_cell_node(Cell::Static {
                    owner: *owner,
                    field: *field,
                });
                self.add_edge(src, c);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn gen_closure(
        &mut self,
        inst: u32,
        func: &FuncIr,
        block: BlockId,
        index: u32,
        dst: ValueId,
        target: FuncId,
        captures: &[Operand],
        span: Span,
    ) {
        let cs = self.locs.intern_closure_site(
            func.id,
            block,
            index,
            target,
            captures.len() as u32,
            span,
        );
        let object = AbstractObject {
            base: HeapBase::Closure(cs),
            context: self.heap_context(inst),
        };
        let o = match self.locs.find_object(&object) {
            Some(o) => o,
            None => {
                // The value's type is the function type or the functional interface the
                // closure implements; the latter is its class for dispatch.
                let vt = func.value_type(dst);
                self.locs
                    .intern_object(ObjectInfo {
                        object,
                        ty: Some(vt),
                        dynamic_type: self.module.types.symbol_of(vt),
                        region: None,
                        func: Some(func.id),
                        span,
                    })
                    .0
            }
        };
        let d = self.var(inst, dst);
        self.add_obj(d, o);
        for (k, cap) in captures.iter().enumerate() {
            if let Some(n) = self.operand_node(inst, cap) {
                let c = self.slot_node(o, Slot::Capture(k as u32));
                self.add_edge(n, c);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn gen_intrinsic(
        &mut self,
        inst: u32,
        func: &FuncIr,
        block: BlockId,
        index: u32,
        dst: Option<ValueId>,
        name: u32,
        args: &[Operand],
        span: Span,
    ) {
        let effect = intrinsic_effect(self.module, self.cfg, name);
        let dst_node = dst.map(|d| self.var(inst, d));
        let data = self.instances[inst as usize];
        match effect {
            IntrinsicEffect::Pure => {}
            IntrinsicEffect::ReturnsArg(k) => {
                if let (Some(d), Some(a)) = (dst_node, self.arg_node(inst, args, k)) {
                    self.add_edge(a, d);
                }
            }
            IntrinsicEffect::Allocates { dynamic_type } => {
                if let (Some(d), Some(dv)) = (dst_node, dst) {
                    let isite = self
                        .locs
                        .intern_intrinsic_site(func.id, block, index, name, span);
                    let object = AbstractObject {
                        base: HeapBase::Intrinsic(isite),
                        context: self.heap_context(inst),
                    };
                    let o = match self.locs.find_object(&object) {
                        Some(o) => o,
                        None => {
                            let vt = func.value_type(dv);
                            self.locs
                                .intern_object(ObjectInfo {
                                    object,
                                    ty: Some(vt),
                                    dynamic_type: dynamic_type
                                        .or_else(|| self.module.types.symbol_of(vt)),
                                    region: None,
                                    func: Some(func.id),
                                    span,
                                })
                                .0
                        }
                    };
                    self.add_obj(d, o);
                }
            }
            IntrinsicEffect::ReturnsUnknown => {
                if let Some(d) = dst_node {
                    let u = self.locs.unknown();
                    self.add_obj(d, u);
                }
            }
            IntrinsicEffect::MergesArgs { from } => {
                if let Some(d) = dst_node {
                    for op in args.iter().skip(from as usize) {
                        self.flow(inst, op, d);
                    }
                    let u = self.locs.unknown();
                    self.add_obj(d, u);
                }
            }
            IntrinsicEffect::CopiesContents { dst: dk, src: sk } => {
                if let (Some(dn), Some(sn)) =
                    (self.arg_node(inst, args, dk), self.arg_node(inst, args, sk))
                {
                    let pair = self.copy_pairs.len() as u32;
                    self.copy_pairs.push((sn, dn));
                    self.add_trigger(sn, Trigger::effect(TriggerKind::CopySource { pair }));
                    self.add_trigger(dn, Trigger::effect(TriggerKind::CopyTarget { pair }));
                }
            }
            IntrinsicEffect::StoresElement { array, value } => {
                if let (Some(a), Some(v)) =
                    (self.arg_node(inst, args, array), self.arg_node(inst, args, value))
                {
                    self.add_store(a, Slot::Element, v);
                }
            }
            IntrinsicEffect::LoadsElement { array } => {
                if let (Some(d), Some(a)) = (dst_node, self.arg_node(inst, args, array)) {
                    self.add_load(a, Slot::Element, d);
                }
            }
            IntrinsicEffect::IteratesElements { iterable } => {
                if let (Some(d), Some(a)) = (dst_node, self.arg_node(inst, args, iterable)) {
                    self.add_trigger(a, Trigger::effect(TriggerKind::IterElements { dst: d }));
                }
            }
            IntrinsicEffect::Invokes {
                callee,
                args: forwarded,
            } => {
                if let Some(f) = self.arg_node(inst, args, callee) {
                    let fargs: Vec<Option<u32>> = forwarded
                        .iter()
                        .map(|&k| self.arg_node(inst, args, k))
                        .collect();
                    let t = Trigger {
                        kind: TriggerKind::Indirect,
                        caller: Some(inst),
                        site: None,
                        call_site: None,
                        args: fargs,
                        receiver: Some(f),
                        dst: dst_node,
                        unwind: false,
                        origin: Some(CallbackOrigin::Intrinsic {
                            func: func.id,
                            name,
                        }),
                        precision: EdgePrecision::Exact,
                        span,
                        fired: false,
                    };
                    self.add_trigger(f, t);
                }
            }
            IntrinsicEffect::VarArg => {
                if let Some(d) = dst_node {
                    self.add_edge(data.varargs, d);
                }
            }
            IntrinsicEffect::CurrentException => {
                if let Some(d) = dst_node {
                    self.add_edge(data.caught, d);
                }
            }
            IntrinsicEffect::Opaque => {
                if self.opaque_seen.insert((func.id, block, index)) {
                    self.opaque_intrinsics.push(OpaqueIntrinsic {
                        func: func.id,
                        block,
                        inst: index,
                        name,
                        span,
                    });
                }
                let argn: Vec<Option<u32>> =
                    args.iter().map(|a| self.operand_node(inst, a)).collect();
                self.external_call(inst, None, ExternalTarget::Name(name), &argn, None, dst_node, false);
            }
        }
    }

    // ── Calls ──

    #[allow(clippy::too_many_arguments)]
    fn gen_call(
        &mut self,
        inst: u32,
        func: &'m FuncIr,
        site: CallSiteId,
        callee: &'m Callee,
        args: &'m [Operand],
        dst: Option<ValueId>,
        unwind: bool,
        span: Span,
    ) {
        let kind = match callee {
            Callee::Static(_) => CallKind::Static,
            Callee::Symbol(_) => CallKind::Symbol,
            Callee::Virtual { .. } => CallKind::Virtual,
            Callee::Interface { .. } => CallKind::Interface,
            Callee::Indirect(_) => CallKind::Indirect,
            Callee::Unresolved(_) => CallKind::Unresolved,
        };
        let site_idx = self.cg.note_site(site, func.id, kind, span);
        let arg_nodes: Vec<Option<u32>> = args.iter().map(|a| self.operand_node(inst, a)).collect();
        let dst_node = dst.map(|d| self.var(inst, d));
        let mut call = Trigger {
            kind: TriggerKind::Indirect,
            caller: Some(inst),
            site: Some(site_idx),
            call_site: Some(site),
            args: arg_nodes,
            receiver: None,
            dst: dst_node,
            unwind,
            origin: None,
            precision: EdgePrecision::Exact,
            span,
            fired: false,
        };
        match *callee {
            Callee::Static(f) => match self.func_idx(f) {
                Some(idx) => self.direct_call(idx, call),
                None => self.external_from_call(&call, ExternalTarget::MissingFunction(f)),
            },
            Callee::Symbol(sym) => match self.func_by_symbol.get(&sym).cloned() {
                Some(list) => {
                    for idx in list {
                        self.direct_call(idx, call.clone());
                    }
                }
                None => self.external_from_call(&call, ExternalTarget::Symbol(sym)),
            },
            Callee::Virtual { receiver, method } | Callee::Interface { receiver, method } => {
                let r = self.var(inst, receiver);
                call.kind = TriggerKind::Dispatch { method };
                call.receiver = Some(r);
                self.add_trigger(r, call);
            }
            Callee::Indirect(v) => {
                let r = self.var(inst, v);
                call.receiver = Some(r);
                self.add_trigger(r, call);
            }
            Callee::Unresolved(name) => {
                let candidates: Vec<u32> = match self.module.call_sites.get(site.index()) {
                    Some(cs) if cs.id == site => cs.candidates.clone(),
                    _ => Vec::new(),
                };
                if candidates.is_empty() {
                    self.external_from_call(&call, ExternalTarget::Name(name));
                    return;
                }
                call.precision = EdgePrecision::OverApproximate(FallbackReason::AmbiguousOverload);
                for sym in candidates {
                    match self.func_by_symbol.get(&sym).cloned() {
                        Some(list) => {
                            for idx in list {
                                self.direct_call(idx, call.clone());
                            }
                        }
                        None => self.external_from_call(&call, ExternalTarget::Symbol(sym)),
                    }
                }
            }
        }
    }

    /// Statically bound call. Under object sensitivity an instance method is analyzed in
    /// the context of each receiver object (constructors, `super` and private calls).
    fn direct_call(&mut self, idx: usize, mut call: Trigger) {
        let module = self.module;
        let callee = &module.funcs[idx];
        if self.cfg.context == ContextPolicy::Object && callee.has_receiver {
            if let Some(Some(r)) = call.args.first().copied() {
                call.kind = TriggerKind::StaticReceiver { callee: idx };
                call.receiver = Some(r);
                let t = self.add_trigger(r, call);
                self.static_receivers.push(t);
                return;
            }
        }
        let ctx = self.callee_context(call.caller, call.call_site, None);
        let b = CallBinding::from_trigger(&call, None, None, call.precision);
        self.bind(idx, ctx, &b);
    }

    /// Bind one resolved target.
    fn bind(&mut self, callee_idx: usize, ctx: Context, b: &CallBinding) {
        let module = self.module;
        let callee = &module.funcs[callee_idx];
        let ci = self.instance(callee_idx, ctx);
        if let Some(site) = b.site {
            self.cg.add_edge(site, callee.id, b.precision);
            if let (Some(caller), Some(cs)) = (b.caller, b.call_site) {
                self.cg.add_context_edge(ContextEdge {
                    caller: InstanceId(caller),
                    site: cs,
                    callee: InstanceId(ci),
                });
            }
        }
        if let Some(origin) = b.origin {
            let receiver = b.receiver.or(b.closure).unwrap_or(self.locs.unknown());
            self.cg.add_callback(CallbackEdge {
                func: callee.id,
                receiver,
                origin,
                span: b.span,
            });
        }

        // The dispatched object alone is the receiver, not the whole receiver set.
        let receiver_param = if b.receiver.is_some() {
            callee.receiver()
        } else {
            None
        };
        if let (Some(o), Some(p)) = (b.receiver, receiver_param) {
            let n = self.var(ci, p);
            self.add_obj(n, o);
        }

        let key = (
            b.caller.unwrap_or(NONE),
            b.call_site.map(|c| c.0).unwrap_or(NONE),
            ci,
            b.closure.map(|o| o.0).unwrap_or(NONE),
        );
        if !self.bound.insert(key) {
            return;
        }

        // `params = captures ++ [receiver] ++ declared`: a closure's leading parameters are
        // fed from the capture slots of the invoked closure object, the arguments bind the
        // parameters after them.
        let offset = if b.closure.is_some() {
            (callee.capture_count as usize).min(callee.params.len())
        } else {
            0
        };
        if let Some(c) = b.closure {
            for (i, &p) in callee.params.iter().enumerate().take(offset) {
                let cell = self.slot_node(c, Slot::Capture(i as u32));
                let n = self.var(ci, p);
                self.add_edge(cell, n);
            }
        }
        let x = self.x_node;
        let data = self.instances[ci as usize];
        if b.from_external {
            for &p in &callee.params[offset..] {
                if Some(p) == receiver_param {
                    continue;
                }
                let n = self.var(ci, p);
                self.add_edge(x, n);
            }
            self.add_edge(data.ret, x);
            self.add_edge(data.exc, x);
            return;
        }
        for (i, a) in b.args.iter().enumerate().skip(b.skip_args) {
            let Some(an) = *a else {
                continue;
            };
            match callee.params.get(offset + i - b.skip_args) {
                Some(&p) => {
                    if Some(p) == receiver_param {
                        continue;
                    }
                    let n = self.var(ci, p);
                    self.add_edge(an, n);
                }
                None => self.add_edge(an, data.varargs),
            }
        }
        if let Some(d) = b.dst {
            self.add_edge(data.ret, d);
        }
        if let Some(caller) = b.caller {
            let c = self.instances[caller as usize];
            let target = if b.unwind { c.caught } else { c.exc };
            self.add_edge(data.exc, target);
        }
    }

    fn external_from_call(&mut self, call: &Trigger, target: ExternalTarget) {
        if let Some(caller) = call.caller {
            let args = call.args.clone();
            self.external_call(
                caller,
                call.site,
                target,
                &args,
                None,
                call.dst,
                call.unwind,
            );
        }
    }

    fn external_from_trigger(&mut self, t: u32, target: ExternalTarget) {
        if !self.external_done.insert((t, target)) {
            return;
        }
        let call = self.triggers[t as usize].clone();
        if let Some(caller) = call.caller {
            self.external_call(
                caller,
                call.site,
                target,
                &call.args,
                call.receiver,
                call.dst,
                call.unwind,
            );
        }
    }

    /// A call into code outside the module.
    #[allow(clippy::too_many_arguments)]
    fn external_call(
        &mut self,
        caller: u32,
        site: Option<usize>,
        target: ExternalTarget,
        args: &[Option<u32>],
        receiver: Option<u32>,
        dst: Option<u32>,
        unwind: bool,
    ) {
        if let Some(s) = site {
            self.cg.add_external(s, target);
        }
        let x = self.x_node;
        let unknown = self.locs.unknown();
        let data = self.instances[caller as usize];
        let exc = if unwind { data.caught } else { data.exc };
        match self.cfg.external {
            ExternalPolicy::Sound => {
                for a in args.iter().flatten() {
                    self.add_edge(*a, x);
                }
                if let Some(r) = receiver {
                    self.add_edge(r, x);
                }
                if let Some(d) = dst {
                    self.add_edge(x, d);
                }
                self.add_edge(x, exc);
            }
            ExternalPolicy::ReturnUnknownOnly => {
                if let Some(d) = dst {
                    self.add_obj(d, unknown);
                }
                self.add_obj(exc, unknown);
            }
        }
    }

    // ── Triggers ──

    fn fire(&mut self, t: u32, o: ObjId) {
        match self.triggers[t as usize].kind {
            TriggerKind::Dispatch { method } => self.fire_dispatch(t, method, o),
            TriggerKind::StaticReceiver { callee } => {
                self.triggers[t as usize].fired = true;
                let call = self.triggers[t as usize].clone();
                let ctx = self.callee_context(call.caller, call.call_site, Some(o));
                let b = CallBinding::from_trigger(&call, Some(o), None, call.precision);
                self.bind(callee, ctx, &b);
            }
            TriggerKind::Indirect => {
                let base = self.locs.object(o).object.base;
                match base {
                    HeapBase::Closure(cs) => self.invoke_closure(t, cs, o, 0),
                    b if b.is_opaque() => self.external_from_trigger(t, ExternalTarget::FunctionValue),
                    _ => {
                        if let Some(s) = self.triggers[t as usize].site {
                            self.cg
                                .add_unresolved(s, UnresolvedReason::NonCallableValue { object: o });
                        }
                    }
                }
            }
            TriggerKind::IterElements { dst } => {
                let c = self.slot_node(o, Slot::Element);
                self.add_edge(c, dst);
                let info = self.locs.object(o);
                let is_array = info.ty.map(|ty| is_array_type(self.module, ty)).unwrap_or(false);
                if !is_array && !info.object.base.is_opaque() {
                    // The elements of a non-array iterable are not derivable without its
                    // iteration protocol.
                    let u = self.locs.unknown();
                    self.add_obj(dst, u);
                }
            }
            TriggerKind::CopySource { pair } => {
                let dn = self.copy_pairs[pair as usize].1;
                let d = self.find(dn);
                let targets: Vec<u32> = self.nodes[d as usize].pts.iter().collect();
                for p in targets {
                    self.mirror(o, ObjId(p));
                }
            }
            TriggerKind::CopyTarget { pair } => {
                let sn = self.copy_pairs[pair as usize].0;
                let s = self.find(sn);
                let sources: Vec<u32> = self.nodes[s as usize].pts.iter().collect();
                for q in sources {
                    self.mirror(ObjId(q), o);
                }
            }
            TriggerKind::ExternalWorld => self.mark_external(o),
        }
    }

    fn fire_dispatch(&mut self, t: u32, method: u32, o: ObjId) {
        let Some(site) = self.triggers[t as usize].site else {
            return;
        };
        let (base, dynamic_type) = {
            let info = self.locs.object(o);
            (info.object.base, info.dynamic_type)
        };
        let facts = self.cfg.hierarchy.method(method).copied();

        // A closure implements exactly the abstract method of its functional type: calling
        // that method invokes the closure. Its concrete (default) methods dispatch normally.
        if let HeapBase::Closure(cs) = base {
            match facts {
                Some(f) if f.is_abstract => {
                    self.invoke_closure(t, cs, o, 1);
                    return;
                }
                None => {
                    self.cg
                        .add_unresolved(site, UnresolvedReason::MethodFactsMissing { method });
                    self.invoke_closure(t, cs, o, 1);
                    self.external_from_trigger(t, ExternalTarget::Symbol(method));
                    return;
                }
                Some(_) => {}
            }
        }

        let ty = match dynamic_type {
            Some(ty) if !base.is_opaque() => ty,
            _ => {
                self.dispatch_unknown_receiver(t, site, method, o, facts);
                return;
            }
        };
        match self.dispatch_cached(ty, method) {
            Dispatch::Resolved { targets, exact } => {
                let precision = if exact {
                    EdgePrecision::Exact
                } else {
                    EdgePrecision::OverApproximate(FallbackReason::AmbiguousOverride)
                };
                for sym in targets {
                    self.bind_symbol(t, sym, o, precision);
                }
            }
            Dispatch::NoImplementation => {
                self.cg.add_unresolved(
                    site,
                    UnresolvedReason::NoImplementation { dynamic_type: ty },
                );
                self.external_from_trigger(t, ExternalTarget::Symbol(method));
            }
            Dispatch::UnknownType => {
                self.cg.add_unresolved(
                    site,
                    UnresolvedReason::HierarchyFactsMissing { dynamic_type: ty },
                );
                self.external_from_trigger(t, ExternalTarget::Symbol(method));
            }
        }
    }

    /// Class-hierarchy analysis for a receiver object of unknown type.
    fn dispatch_unknown_receiver(
        &mut self,
        t: u32,
        site: usize,
        method: u32,
        o: ObjId,
        facts: Option<MethodFacts>,
    ) {
        let precision = EdgePrecision::OverApproximate(FallbackReason::ReceiverTypeUnknown);
        match facts {
            None => {
                self.cg
                    .add_unresolved(site, UnresolvedReason::MethodFactsMissing { method });
                if self.func_by_symbol.contains_key(&method) {
                    self.bind_symbol(t, method, o, precision);
                }
                self.external_from_trigger(t, ExternalTarget::Symbol(method));
            }
            Some(f) => {
                let targets = self.cha_cached(method);
                if targets.is_empty() && !f.is_external {
                    self.cg.add_unresolved(
                        site,
                        UnresolvedReason::NoImplementation {
                            dynamic_type: f.declaring_type,
                        },
                    );
                }
                for sym in targets {
                    self.bind_symbol(t, sym, o, precision);
                }
                // Objects of external origin may be instances of external implementations.
                if f.is_external {
                    self.external_from_trigger(t, ExternalTarget::Symbol(method));
                }
            }
        }
    }

    fn bind_symbol(&mut self, t: u32, sym: u32, o: ObjId, precision: EdgePrecision) {
        match self.func_by_symbol.get(&sym).cloned() {
            Some(list) => {
                let call = self.triggers[t as usize].clone();
                let ctx = self.callee_context(call.caller, call.call_site, Some(o));
                let b = CallBinding::from_trigger(&call, Some(o), None, precision);
                for idx in list {
                    self.bind(idx, ctx, &b);
                }
            }
            None => self.external_from_trigger(t, ExternalTarget::Symbol(sym)),
        }
    }

    /// Invoke closure object `o` of closure site `cs`, skipping `skip` leading arguments.
    fn invoke_closure(&mut self, t: u32, cs: ClosureSiteId, o: ObjId, skip: usize) {
        let target = self.locs.closure_site(cs).target;
        match self.func_idx(target) {
            Some(idx) => {
                let call = self.triggers[t as usize].clone();
                let ctx = self.callee_context(call.caller, call.call_site, Some(o));
                let mut b = CallBinding::from_trigger(&call, None, Some(o), EdgePrecision::Exact);
                b.skip_args = skip.min(b.args.len());
                self.bind(idx, ctx, &b);
            }
            None => self.external_from_trigger(t, ExternalTarget::MissingFunction(target)),
        }
    }

    /// Methods of `o` external code may call once it holds `o`.
    fn external_callbacks(&mut self, o: ObjId) {
        let (base, dynamic_type) = {
            let info = self.locs.object(o);
            (info.object.base, info.dynamic_type)
        };
        if let HeapBase::Closure(cs) = base {
            let target = self.locs.closure_site(cs).target;
            if let Some(idx) = self.func_idx(target) {
                let ctx = self.callee_context(None, None, Some(o));
                let b = CallBinding::external(None, Some(o), CallbackOrigin::ExternalFunctionValue);
                self.bind(idx, ctx, &b);
            }
            return;
        }
        let Some(ty) = dynamic_type else {
            return;
        };
        if base.is_opaque() {
            return;
        }
        let entries = match self.callback_cache.get(&ty) {
            Some(v) => v.clone(),
            None => {
                let by_symbol = &self.func_by_symbol;
                let v = self
                    .cfg
                    .hierarchy
                    .external_entry_methods(ty, &|m: u32| by_symbol.contains_key(&m));
                self.callback_cache.insert(ty, v.clone());
                v
            }
        };
        for (implementation, declaration) in entries {
            let Some(list) = self.func_by_symbol.get(&implementation).cloned() else {
                continue;
            };
            let ctx = self.callee_context(None, None, Some(o));
            let b = CallBinding::external(
                Some(o),
                None,
                CallbackOrigin::ExternalOverride { declaration },
            );
            for idx in list {
                self.bind(idx, ctx, &b);
            }
        }
    }

    fn dispatch_cached(&mut self, ty: u32, method: u32) -> Dispatch {
        if let Some(d) = self.dispatch_cache.get(&(ty, method)) {
            return d.clone();
        }
        let d = self.cfg.hierarchy.dispatch(ty, method);
        self.dispatch_cache.insert((ty, method), d.clone());
        d
    }

    fn cha_cached(&mut self, method: u32) -> Vec<u32> {
        if let Some(v) = self.cha_cache.get(&method) {
            return v.clone();
        }
        let v = self.cfg.hierarchy.cha_targets(method);
        self.cha_cache.insert(method, v.clone());
        v
    }

    // ── Fixpoint ──

    fn run(&mut self) {
        loop {
            while let Some(i) = self.pending.pop_front() {
                self.generate(i);
            }
            match self.worklist.pop_front() {
                Some(n) => {
                    self.queued[n as usize] = false;
                    if self.find(n) != n {
                        continue;
                    }
                    self.stats.propagations += 1;
                    if self.stats.propagations > self.cfg.max_propagations {
                        self.truncated = true;
                        return;
                    }
                    self.process(n);
                }
                None => {
                    if !self.flush_static_receivers() {
                        return;
                    }
                }
            }
        }
    }

    /// Non-virtual instance calls whose receiver set stayed empty are bound once in the
    /// caller's context, so the statically named callee is analyzed.
    fn flush_static_receivers(&mut self) -> bool {
        let mut any = false;
        let pending: Vec<u32> = self.static_receivers.clone();
        for t in pending {
            if self.triggers[t as usize].fired {
                continue;
            }
            self.triggers[t as usize].fired = true;
            let call = self.triggers[t as usize].clone();
            if let TriggerKind::StaticReceiver { callee } = call.kind {
                let ctx = self.callee_context(call.caller, call.call_site, None);
                let b = CallBinding::from_trigger(
                    &call,
                    None,
                    None,
                    EdgePrecision::OverApproximate(FallbackReason::NoReceiverObject),
                );
                self.bind(callee, ctx, &b);
                any = true;
            }
        }
        any
    }

    fn process(&mut self, n: u32) {
        let delta = {
            let node = &self.nodes[n as usize];
            node.pts.difference(&node.done)
        };
        if delta.is_empty() {
            return;
        }
        self.nodes[n as usize].done.union_with(&delta);
        let objs: Vec<u32> = delta.iter().collect();

        // Complex constraints. Nothing below merges nodes, so `n` stays a representative
        // and its constraint lists can be taken out and put back.
        let loads = std::mem::take(&mut self.nodes[n as usize].loads);
        for &(slot, dst) in &loads {
            for &o in &objs {
                let c = self.slot_node(ObjId(o), slot);
                self.add_edge(c, dst);
            }
        }
        Self::restore(&mut self.nodes[n as usize].loads, loads);

        let stores = std::mem::take(&mut self.nodes[n as usize].stores);
        for &(slot, src) in &stores {
            for &o in &objs {
                let c = self.slot_node(ObjId(o), slot);
                self.add_edge(src, c);
            }
        }
        Self::restore(&mut self.nodes[n as usize].stores, stores);

        let filters = std::mem::take(&mut self.nodes[n as usize].filters);
        for &(dst, ty) in &filters {
            for &o in &objs {
                if self.passes_filter(ObjId(o), ty) {
                    self.add_obj(dst, ObjId(o));
                }
            }
        }
        Self::restore(&mut self.nodes[n as usize].filters, filters);

        let triggers = std::mem::take(&mut self.nodes[n as usize].triggers);
        for &t in &triggers {
            for &o in &objs {
                self.fire(t, ObjId(o));
            }
        }
        Self::restore(&mut self.nodes[n as usize].triggers, triggers);

        // Copy edges, with lazy cycle detection. The list is snapshotted (not taken) so a
        // cycle search sees the node's own edges.
        let succs: Vec<u32> = self.nodes[n as usize].copy.clone();
        for z0 in succs {
            let src = self.find(n);
            let z = self.find(z0);
            if z == src {
                continue;
            }
            if self.nodes[z as usize].pts.union_with(&delta) {
                self.push(z);
            }
            if !self.lcd_checked.contains(&(src, z))
                && self.nodes[z as usize].pts == self.nodes[src as usize].pts
            {
                self.lcd_checked.insert((src, z));
                self.collapse_cycles_from(z);
            }
        }
    }

    fn restore<T>(slot: &mut Vec<T>, mut taken: Vec<T>) {
        taken.append(slot);
        *slot = taken;
    }

    /// Tarjan search over copy edges from `root`; every non-trivial SCC is unified.
    fn collapse_cycles_from(&mut self, root: u32) {
        self.stats.cycle_searches += 1;
        let root = self.find(root);
        let mut index: FxHashMap<u32, u32> = FxHashMap::default();
        let mut low: FxHashMap<u32, u32> = FxHashMap::default();
        let mut on_stack: FxHashSet<u32> = FxHashSet::default();
        let mut stack: Vec<u32> = Vec::new();
        let mut frames: Vec<(u32, usize)> = Vec::new();
        let mut counter = 0u32;
        let mut sccs: Vec<Vec<u32>> = Vec::new();

        index.insert(root, counter);
        low.insert(root, counter);
        counter += 1;
        stack.push(root);
        on_stack.insert(root);
        frames.push((root, 0));

        while let Some(&(v, next)) = frames.last() {
            if next < self.nodes[v as usize].copy.len() {
                if let Some(top) = frames.last_mut() {
                    top.1 += 1;
                }
                let raw = self.nodes[v as usize].copy[next];
                let w = self.find(raw);
                if w == v {
                    continue;
                }
                match index.get(&w).copied() {
                    None => {
                        index.insert(w, counter);
                        low.insert(w, counter);
                        counter += 1;
                        stack.push(w);
                        on_stack.insert(w);
                        frames.push((w, 0));
                    }
                    Some(iw) => {
                        if on_stack.contains(&w) {
                            let lv = low[&v].min(iw);
                            low.insert(v, lv);
                        }
                    }
                }
                continue;
            }
            frames.pop();
            let lv = low[&v];
            if let Some(&(p, _)) = frames.last() {
                let lp = low[&p].min(lv);
                low.insert(p, lp);
            }
            if lv == index[&v] {
                let mut comp = Vec::new();
                while let Some(w) = stack.pop() {
                    on_stack.remove(&w);
                    comp.push(w);
                    if w == v {
                        break;
                    }
                }
                if comp.len() > 1 {
                    sccs.push(comp);
                }
            }
        }
        for comp in sccs {
            self.unify(&comp);
        }
    }

    /// Merge the nodes of a copy cycle into one representative.
    fn unify(&mut self, members: &[u32]) {
        let r = self.find(members[0]);
        for &m0 in &members[1..] {
            let m = self.find(m0);
            if m == r {
                continue;
            }
            let node = std::mem::take(&mut self.nodes[m as usize]);
            self.parent[m as usize] = r;
            let rn = &mut self.nodes[r as usize];
            rn.pts.union_with(&node.pts);
            // Objects are known propagated only where every merged node propagated them.
            rn.done.intersect_with(&node.done);
            rn.copy.extend(node.copy);
            rn.loads.extend(node.loads);
            rn.stores.extend(node.stores);
            rn.filters.extend(node.filters);
            rn.triggers.extend(node.triggers);
            self.stats.collapsed_nodes += 1;
        }
        let copy = std::mem::take(&mut self.nodes[r as usize].copy);
        let mut normalized: Vec<u32> = Vec::with_capacity(copy.len());
        for z in copy {
            let z = self.find(z);
            if z != r {
                normalized.push(z);
            }
        }
        normalized.sort_unstable();
        normalized.dedup();
        let rn = &mut self.nodes[r as usize];
        rn.copy = normalized;
        rn.loads.sort_unstable();
        rn.loads.dedup();
        rn.stores.sort_unstable();
        rn.stores.dedup();
        rn.filters.sort_unstable();
        rn.filters.dedup();
        rn.triggers.sort_unstable();
        rn.triggers.dedup();
        self.queued[r as usize] = false;
        self.push(r);
    }

    // ── Result ──

    /// Index of the canonical set of `node`'s representative (0 is the empty set).
    fn canonical_set(
        &mut self,
        node: u32,
        sets: &mut Vec<SparseBitSet>,
        by_rep: &mut FxHashMap<u32, u32>,
    ) -> u32 {
        let r = self.find(node);
        if self.nodes[r as usize].pts.is_empty() {
            return 0;
        }
        if let Some(&s) = by_rep.get(&r) {
            return s;
        }
        let s = sets.len() as u32;
        sets.push(self.nodes[r as usize].pts.clone());
        by_rep.insert(r, s);
        s
    }

    /// Merge per-key set indices: one distinct set is shared, several are united.
    fn merge_sets<K: Ord + Copy + std::hash::Hash + Eq>(
        mut entries: Vec<(K, u32)>,
        sets: &mut Vec<SparseBitSet>,
    ) -> FxHashMap<K, u32> {
        entries.sort_unstable();
        entries.dedup();
        let mut out = FxHashMap::default();
        let mut i = 0;
        while i < entries.len() {
            let key = entries[i].0;
            let mut j = i;
            while j < entries.len() && entries[j].0 == key {
                j += 1;
            }
            if j - i == 1 {
                out.insert(key, entries[i].1);
            } else {
                let mut u = SparseBitSet::new();
                for e in &entries[i..j] {
                    u.union_with(&sets[e.1 as usize]);
                }
                out.insert(key, sets.len() as u32);
                sets.push(u);
            }
            i = j;
        }
        out
    }

    fn finish(mut self) -> PointsToResult {
        let module = self.module;
        let mut sets: Vec<SparseBitSet> = vec![SparseBitSet::new()];
        let mut by_rep: FxHashMap<u32, u32> = FxHashMap::default();

        let var_keys: Vec<((u32, ValueId), u32)> =
            self.var_nodes.iter().map(|(&k, &n)| (k, n)).collect();
        let mut instance_values = FxHashMap::default();
        let mut per_func: Vec<((FuncId, ValueId), u32)> = Vec::with_capacity(var_keys.len());
        for ((inst, value), node) in var_keys {
            let s = self.canonical_set(node, &mut sets, &mut by_rep);
            if s == 0 {
                continue;
            }
            instance_values.insert((InstanceId(inst), value), s);
            let func = module.funcs[self.instances[inst as usize].func_idx].id;
            per_func.push(((func, value), s));
        }
        let values = Self::merge_sets(per_func, &mut sets);

        let mut instance_returns = Vec::with_capacity(self.instances.len());
        let mut instance_thrown = Vec::with_capacity(self.instances.len());
        let mut ret_entries = Vec::new();
        let mut exc_entries = Vec::new();
        let mut instances_by_func: FxHashMap<FuncId, Vec<InstanceId>> = FxHashMap::default();
        let mut instances = Vec::with_capacity(self.instances.len());
        for i in 0..self.instances.len() {
            let data = self.instances[i];
            let func = module.funcs[data.func_idx].id;
            let r = self.canonical_set(data.ret, &mut sets, &mut by_rep);
            let e = self.canonical_set(data.exc, &mut sets, &mut by_rep);
            instance_returns.push(r);
            instance_thrown.push(e);
            if r != 0 {
                ret_entries.push((func, r));
            }
            if e != 0 {
                exc_entries.push((func, e));
            }
            instances_by_func
                .entry(func)
                .or_default()
                .push(InstanceId(i as u32));
            instances.push(FunctionInstance {
                func,
                context: data.ctx,
            });
        }
        let returns = Self::merge_sets(ret_entries, &mut sets);
        let thrown = Self::merge_sets(exc_entries, &mut sets);

        let mut cells = vec![0u32; self.locs.cell_count()];
        for (c, slot) in cells.iter_mut().enumerate() {
            if let Some(&n) = self.cell_nodes.get(c) {
                if n != NONE {
                    *slot = self.canonical_set(n, &mut sets, &mut by_rep);
                }
            }
        }
        let string_set = if self.string_node != NONE {
            let n = self.string_node;
            self.canonical_set(n, &mut sets, &mut by_rep)
        } else {
            0
        };
        let unknown = self.locs.unknown();
        let external_cell = self
            .locs
            .find_cell(&Cell::Object {
                obj: unknown,
                slot: Slot::Content,
            })
            .unwrap_or(CellId(0));

        let status = if self.truncated {
            SolveStatus::Truncated {
                propagations: self.stats.propagations,
            }
        } else {
            SolveStatus::Complete
        };
        let mut stats = self.stats;
        stats.nodes = self.nodes.len();
        stats.objects = self.locs.object_count();
        stats.cells = self.locs.cell_count();
        stats.instances = instances.len();
        if stats.missing_entries > 0 {
            log_debug(&format!(
                "Points-to: {} explicit entry points name no function of the module.",
                stats.missing_entries
            ));
        }

        let call_graph = self.cg.finish(instances, self.truncated);
        PointsToResult {
            locations: self.locs,
            call_graph,
            status,
            stats,
            context: self.cfg.context,
            external: self.cfg.external,
            opaque_intrinsics: self.opaque_intrinsics,
            sets,
            values,
            instance_values,
            returns,
            thrown,
            instance_returns,
            instance_thrown,
            cells,
            instances_by_func,
            string_literal: self.string_obj,
            string_set,
            external_cell,
        }
    }
}
