//! Flow-sensitive abstract heap and heap-shape graphs.
//!
//! The interpreter runs over the IR of every function, callees before callers, and keeps
//! at each program point: the points-to set of every SSA value, the contents of every
//! abstract object slot, the lifetime state of every object (live, released, not yet
//! allocated) and the objects that escaped to memory the function cannot see. Calls are
//! applied through bottom-up summaries (access paths of depth one: parameters and their
//! fields), iterated to a fixpoint over recursive components. Calls without an IR body use
//! the effects the language layer supplies; without those, their arguments escape and the
//! result is an opaque object, and the function's derivation records the gap.
//!
//! Heap-shape graphs are snapshots of that state at chosen program points: objects as
//! nodes, slot points-to as edges, and recursive structures (lists, trees, graphs) found
//! as cycles of the allocation-site graph.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde::{Deserialize, Serialize};

use crate::ir::{
    BlockIr, Callee, ConstVal, FuncId, FuncIr, Inst, IrType, ModuleIr, Place, RefKind, Region,
    Terminator, TypeId,
};
use crate::memview::{
    AbsObj, CallEffect, Derivation, FieldKey, FuncIndex, InitKind, MemoryFacts, ModuleIndex,
    OperandRef, ProgramPoint, ReleaseKind, SrcSpan,
};

/// A points-to set (pseudo-objects `Null`, `Undef` and `Unknown` included).
pub type PtsSet = BTreeSet<AbsObj>;

/// Lifetime bits of an object; a state is the set of possibilities on the paths reaching
/// a point.
pub const LIFE_LIVE: u8 = 1;
pub const LIFE_RELEASED: u8 = 2;
pub const LIFE_UNALLOCATED: u8 = 4;

/// Release-kind bits recorded per object.
pub const REL_DEALLOCATE: u8 = 1;
pub const REL_DISPOSE: u8 = 2;
pub const REL_END_OF_SCOPE: u8 = 4;
pub const REL_UNKNOWN: u8 = 8;

/// Upper bound on bottom-up summary rounds over recursive call-graph components.
const MAX_SUMMARY_ROUNDS: usize = 32;
/// Upper bound on visits of one block during a function's fixpoint.
const MAX_BLOCK_VISITS: usize = 256;

/// Memory region of an allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemRegion {
    Heap,
    Stack,
    Static,
}

impl MemRegion {
    fn from_ir(r: &Region) -> Self {
        match r {
            Region::Heap => Self::Heap,
            Region::Stack => Self::Stack,
            Region::Static => Self::Static,
        }
    }
}

pub(crate) fn one(o: AbsObj) -> PtsSet {
    let mut s = PtsSet::new();
    s.insert(o);
    s
}

pub(crate) fn storage(p: &PtsSet) -> impl Iterator<Item = AbsObj> + '_ {
    p.iter().copied().filter(|o| o.is_storage())
}

fn rel_bit(kind: Option<ReleaseKind>) -> u8 {
    match kind {
        Some(ReleaseKind::Deallocate) => REL_DEALLOCATE,
        Some(ReleaseKind::Dispose) => REL_DISPOSE,
        Some(ReleaseKind::EndOfScope) => REL_END_OF_SCOPE,
        Some(ReleaseKind::Unlock) => 0,
        None => REL_UNKNOWN,
    }
}

// ─── Events ─────────────────────────────────────────────────────────────────────

/// What happened to an object at a program point.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "event")]
pub enum MemEventKind {
    /// The object was created.
    Allocated { region: MemRegion },
    /// A reference to the object was stored into `container.slot`.
    StoredInto { container: AbsObj, slot: FieldKey },
    /// The object's storage was read or written (directly or by a callee).
    Accessed { write: bool, slot: FieldKey, via_call: bool },
    /// A reference to the object was passed to a call (`arg: None` is the receiver).
    Passed {
        call_site: u32,
        callees: Vec<u32>,
        arg: Option<u32>,
    },
    /// A reference to the object was captured by a closure.
    Captured { closure: AbsObj, index: u32 },
    /// A reference to the object was returned.
    Returned,
    /// A reference to the object was thrown.
    Thrown,
    /// The object escaped to memory the analysis cannot see.
    Escaped,
    /// The object was released (`kind: None` when the release semantics are not supplied).
    Released { kind: Option<ReleaseKind>, must: bool },
    /// No root reaches the object any more (function exit).
    Unreachable,
}

/// One lifetime event of an abstract object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemEvent {
    pub point: ProgramPoint,
    pub span: SrcSpan,
    pub obj: AbsObj,
    pub kind: MemEventKind,
    /// SSA value involved (the defined or the stored value), when there is one.
    pub value: Option<u32>,
}

// ─── Abstract state ─────────────────────────────────────────────────────────────

/// The abstract memory at one program point.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HeapState {
    /// False for the bottom state (point not reached yet).
    pub reached: bool,
    /// SSA value -> objects it may reference.
    pub vals: BTreeMap<u32, PtsSet>,
    /// Object slot -> contents. An absent slot holds the object's initial contents.
    pub heap: BTreeMap<(AbsObj, FieldKey), PtsSet>,
    /// Lifetime bits; absent means "not allocated" for allocation objects and "live" for
    /// every other storage object.
    pub life: BTreeMap<AbsObj, u8>,
    /// Points where each object may have been released.
    pub released_at: BTreeMap<AbsObj, BTreeSet<ProgramPoint>>,
    /// `REL_*` bits of the releases each object went through.
    pub release_kinds: BTreeMap<AbsObj, u8>,
    /// Objects passed to code the analysis cannot see.
    pub escaped: BTreeSet<AbsObj>,
}

impl HeapState {
    pub fn pts(&self, v: u32) -> PtsSet {
        self.vals.get(&v).cloned().unwrap_or_default()
    }

    pub fn life_of(&self, o: AbsObj) -> u8 {
        if !o.is_storage() {
            return 0;
        }
        match self.life.get(&o) {
            Some(&b) => b,
            None if o.is_allocation() => LIFE_UNALLOCATED,
            None => LIFE_LIVE,
        }
    }

    pub fn contents(&self, cell: &(AbsObj, FieldKey), sem: &Semantics) -> PtsSet {
        match self.heap.get(cell) {
            Some(c) => c.clone(),
            None => sem.initial(cell.0),
        }
    }

    /// Every slot entry of one object.
    pub fn cells_of(&self, o: AbsObj) -> Vec<(FieldKey, &PtsSet)> {
        self.heap
            .range((o, FieldKey::Whole)..=(o, FieldKey::Capture(u32::MAX)))
            .map(|((_, k), v)| (*k, v))
            .collect()
    }

    fn clear_cells(&mut self, o: AbsObj) {
        let keys: Vec<(AbsObj, FieldKey)> = self
            .heap
            .range((o, FieldKey::Whole)..=(o, FieldKey::Capture(u32::MAX)))
            .map(|(k, _)| *k)
            .collect();
        for k in keys {
            self.heap.remove(&k);
        }
    }

    /// Objects reachable from `roots` through slot contents.
    pub fn reachable_from(&self, roots: impl IntoIterator<Item = AbsObj>) -> BTreeSet<AbsObj> {
        let mut seen = BTreeSet::new();
        let mut stack: Vec<AbsObj> = roots.into_iter().filter(|o| o.is_storage()).collect();
        while let Some(o) = stack.pop() {
            if !seen.insert(o) {
                continue;
            }
            for (_, contents) in self.cells_of(o) {
                for c in storage(contents) {
                    if !seen.contains(&c) {
                        stack.push(c);
                    }
                }
            }
        }
        seen
    }

    /// Every storage object the state mentions.
    pub fn objects(&self) -> BTreeSet<AbsObj> {
        let mut out = BTreeSet::new();
        for s in self.vals.values() {
            out.extend(storage(s));
        }
        for ((o, _), c) in &self.heap {
            out.insert(*o);
            out.extend(storage(c));
        }
        out.extend(self.life.keys().copied());
        out
    }

    /// Joins `other` into `self`; returns whether `self` changed.
    pub fn join_from(&mut self, other: &HeapState, sem: &Semantics) -> bool {
        if !other.reached {
            return false;
        }
        if !self.reached {
            *self = other.clone();
            return true;
        }
        let before = self.clone();
        for (v, p) in &other.vals {
            self.vals.entry(*v).or_default().extend(p.iter().copied());
        }
        let keys: BTreeSet<(AbsObj, FieldKey)> = self
            .heap
            .keys()
            .chain(other.heap.keys())
            .copied()
            .collect();
        for k in keys {
            let mut merged = before.contents(&k, sem);
            merged.extend(other.contents(&k, sem));
            self.heap.insert(k, merged);
        }
        let objs: BTreeSet<AbsObj> = before
            .life
            .keys()
            .chain(other.life.keys())
            .copied()
            .collect();
        for o in objs {
            let bits = before.life_of(o) | other.life_of(o);
            self.life.insert(o, bits);
        }
        for (o, pts) in &other.released_at {
            self.released_at
                .entry(*o)
                .or_default()
                .extend(pts.iter().copied());
        }
        for (o, k) in &other.release_kinds {
            *self.release_kinds.entry(*o).or_default() |= *k;
        }
        self.escaped.extend(other.escaped.iter().copied());
        *self != before
    }

    /// Recency fold: every instance denoted by `recent` becomes part of its old summary.
    fn fold(&mut self, recent: AbsObj, sem: &Semantics) {
        let old = recent.as_old();
        if recent == old || !self.life.contains_key(&recent) {
            return;
        }
        let old_exists = self.life.contains_key(&old);
        let recent_keys: Vec<FieldKey> = self.cells_of(recent).into_iter().map(|(k, _)| k).collect();
        let old_keys: Vec<FieldKey> = self.cells_of(old).into_iter().map(|(k, _)| k).collect();
        let keys: BTreeSet<FieldKey> = recent_keys.into_iter().chain(old_keys).collect();
        for k in keys {
            let mut merged = self.contents(&(recent, k), sem);
            if old_exists {
                merged.extend(self.contents(&(old, k), sem));
            }
            self.heap.remove(&(recent, k));
            self.heap.insert((old, k), merged);
        }
        for set in self.vals.values_mut() {
            if set.remove(&recent) {
                set.insert(old);
            }
        }
        for set in self.heap.values_mut() {
            if set.remove(&recent) {
                set.insert(old);
            }
        }
        let rb = self.life.remove(&recent).unwrap_or(LIFE_UNALLOCATED);
        let ob = if old_exists { self.life_of(old) } else { 0 };
        self.life.insert(old, ob | rb);
        if let Some(r) = self.released_at.remove(&recent) {
            self.released_at.entry(old).or_default().extend(r);
        }
        if let Some(k) = self.release_kinds.remove(&recent) {
            *self.release_kinds.entry(old).or_default() |= k;
        }
        if self.escaped.remove(&recent) {
            self.escaped.insert(old);
        }
    }
}

// ─── Semantics supplied once per module ─────────────────────────────────────────

/// Allocation and external-call semantics, resolved once from the IR and the facts.
#[derive(Debug, Clone, Default)]
pub struct Semantics {
    /// Allocation site -> region of its `Alloc`.
    pub site_region: HashMap<u32, MemRegion>,
    /// Allocation site -> initial contents (`None` when not supplied).
    pub site_init: HashMap<u32, Option<InitKind>>,
    /// Call sites without IR targets -> effects supplied by the language layer.
    pub call_effects: HashMap<u32, Vec<CallEffect>>,
    /// Allocating call sites -> initial contents of the returned storage.
    pub call_alloc_init: HashMap<u32, Option<InitKind>>,
}

impl Semantics {
    fn build(module: &ModuleIr, index: &ModuleIndex, facts: &dyn MemoryFacts) -> Self {
        let mut sem = Semantics::default();
        for func in &module.funcs {
            for block in &func.blocks {
                for inst in &block.insts {
                    match inst {
                        Inst::Alloc { site, region, .. } => {
                            sem.site_region.insert(site.0, MemRegion::from_ir(region));
                            sem.site_init.insert(site.0, facts.init_kind(*site));
                        }
                        Inst::Call { site, .. } => {
                            let unresolved = index
                                .callees
                                .get(&site.0)
                                .map(|t| t.is_empty())
                                .unwrap_or(true);
                            if !unresolved {
                                continue;
                            }
                            if let Some(effects) = facts.call_effects(func.id, *site) {
                                for e in &effects {
                                    if let CallEffect::AllocatesResult { init } = e {
                                        sem.call_alloc_init.insert(site.0, *init);
                                    }
                                }
                                sem.call_effects.insert(site.0, effects);
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        sem
    }

    /// Contents of a slot that no store reached.
    pub fn initial(&self, o: AbsObj) -> PtsSet {
        let kind = match o {
            AbsObj::Site { site, .. } => self.site_init.get(&site).copied().flatten(),
            AbsObj::CallAlloc { call_site, .. } => {
                self.call_alloc_init.get(&call_site).copied().flatten()
            }
            _ => None,
        };
        one(match kind {
            Some(InitKind::Zeroed) => AbsObj::Null,
            Some(InitKind::Uninitialized) => AbsObj::Undef,
            None => AbsObj::Unknown,
        })
    }

    pub fn region(&self, o: AbsObj) -> Option<MemRegion> {
        match o {
            AbsObj::Site { site, .. } => self.site_region.get(&site).copied(),
            AbsObj::CallAlloc { .. } => Some(MemRegion::Heap),
            AbsObj::Global(_) | AbsObj::Static { .. } => Some(MemRegion::Static),
            AbsObj::Closure { .. } => Some(MemRegion::Heap),
            _ => None,
        }
    }
}

// ─── Summaries ──────────────────────────────────────────────────────────────────

/// An object as a callee summary names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "sum")]
pub enum SumObj {
    /// The objects of parameter `i`.
    Param(u32),
    /// The contents of slot `k` of the objects of parameter `i` at entry.
    ParamField(u32, FieldKey),
    /// A concrete abstract object (callee allocations, globals, null).
    Obj(AbsObj),
    /// Memory the callee could not see either.
    Fresh,
}

/// How a callee releases the objects of one parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParamRelease {
    /// Released on every exit.
    pub must: bool,
    /// `REL_*` bits.
    pub kinds: u8,
}

/// Bottom-up memory summary of a function.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FuncSummary {
    /// Objects the normal exits may return.
    pub ret: BTreeSet<SumObj>,
    /// Parameters whose objects the callee releases.
    pub released: BTreeMap<u32, ParamRelease>,
    /// Parameters whose objects escape to memory the caller cannot see.
    pub escapes: BTreeSet<u32>,
    /// Parameters whose objects the callee accesses (`true` when written).
    pub accesses: BTreeMap<u32, bool>,
    /// Slot contents the callee stores into the objects of a parameter.
    pub param_stores: BTreeMap<(u32, FieldKey), BTreeSet<SumObj>>,
    /// Lifetime bits at exit of callee allocations the caller can reach.
    pub out_life: BTreeMap<AbsObj, u8>,
    /// Recent allocation objects the callee (transitively) creates.
    pub allocates: BTreeSet<AbsObj>,
    /// The callee (transitively) reaches calls without modeled semantics.
    pub reaches_unmodeled: bool,
}

/// The facts of one function exit.
#[derive(Debug, Clone)]
pub struct ExitFact {
    pub point: ProgramPoint,
    pub span: SrcSpan,
    /// True for `Throw` exits.
    pub exceptional: bool,
    /// Objects returned or thrown.
    pub returned: PtsSet,
    /// The abstract memory at the exit, before the frame is popped.
    pub state: HeapState,
}

/// Abstract-heap results of one function.
#[derive(Debug, Clone, Default)]
pub struct FuncHeap {
    pub func: u32,
    /// In-state of every block position.
    pub block_in: Vec<HeapState>,
    /// Every value's points-to set, joined over all points.
    pub values: BTreeMap<u32, PtsSet>,
    /// Values computed by pointer arithmetic (interior references).
    pub interior: BTreeSet<u32>,
    /// Opaque objects loaded from a known slot: value -> slot it was read from.
    pub ext_origin: BTreeMap<u32, (AbsObj, FieldKey)>,
    pub events: Vec<MemEvent>,
    pub exits: Vec<ExitFact>,
    pub summary: FuncSummary,
    /// Calls and intrinsics with neither an IR body nor supplied effects.
    pub unmodeled: BTreeSet<ProgramPoint>,
    /// False when the block fixpoint hit its visit bound.
    pub converged: bool,
}

/// One instruction or terminator handed to a replay visitor.
#[derive(Debug, Clone, Copy)]
pub enum Step<'a> {
    Inst(&'a Inst),
    Term(&'a Terminator),
}

/// Module-wide abstract-heap results.
#[derive(Debug, Clone, Default)]
pub struct HeapResult {
    pub index: ModuleIndex,
    pub sem: Semantics,
    pub funcs: HashMap<u32, FuncHeap>,
    pub summaries: HashMap<u32, FuncSummary>,
    pub derivation: Derivation,
    pub notes: Vec<String>,
}

impl HeapResult {
    /// Interprets every function of the module.
    pub fn run(module: &ModuleIr, facts: &dyn MemoryFacts) -> Self {
        let index = ModuleIndex::build(module, facts);
        let sem = Semantics::build(module, &index, facts);
        let mut summaries: HashMap<u32, FuncSummary> = HashMap::new();
        let mut funcs: HashMap<u32, FuncHeap> = HashMap::new();
        let mut converged = false;
        for _ in 0..MAX_SUMMARY_ROUNDS {
            let mut changed = false;
            for &f in &index.bottom_up {
                let Some(fh) = analyze_func(module, &index, &sem, facts, &summaries, f) else {
                    continue;
                };
                if summaries.get(&f) != Some(&fh.summary) {
                    summaries.insert(f, fh.summary.clone());
                    changed = true;
                }
                funcs.insert(f, fh);
            }
            if !changed {
                converged = true;
                break;
            }
        }

        let mut derivation = Derivation::Exact;
        let mut notes = Vec::new();
        if !converged {
            derivation = derivation.weakest(Derivation::partial(format!(
                "interprocedural summaries did not stabilize within {MAX_SUMMARY_ROUNDS} rounds"
            )));
            notes.push("summary fixpoint bound reached".to_string());
        }
        let unmodeled: usize = funcs.values().map(|f| f.unmodeled.len()).sum();
        if unmodeled > 0 {
            derivation = derivation.weakest(Derivation::partial(format!(
                "{unmodeled} calls or intrinsics have neither an IR body nor supplied effects"
            )));
            notes.push(format!(
                "{unmodeled} unmodeled call points: their arguments are treated as escaping"
            ));
        }
        let unconverged: Vec<u32> = funcs
            .values()
            .filter(|f| !f.converged)
            .map(|f| f.func)
            .collect();
        if !unconverged.is_empty() {
            derivation = derivation.weakest(Derivation::partial(format!(
                "block fixpoint bound reached in {} functions",
                unconverged.len()
            )));
        }
        if !index.recursive.is_empty() {
            derivation = derivation.weakest(Derivation::OverApproximate);
        }
        if !index.unresolved_calls.is_empty() {
            notes.push(format!(
                "{} call sites have no IR target",
                index.unresolved_calls.len()
            ));
        }
        HeapResult {
            index,
            sem,
            funcs,
            summaries,
            derivation,
            notes,
        }
    }

    /// Re-runs one function from its converged block states, calling `visitor` with the
    /// state just before every instruction and terminator.
    pub fn replay(
        &self,
        module: &ModuleIr,
        facts: &dyn MemoryFacts,
        func: u32,
        visitor: &mut dyn FnMut(ProgramPoint, Step<'_>, &HeapState),
    ) {
        let Some(fh) = self.funcs.get(&func) else {
            return;
        };
        let Some((f, fi)) = self.index.func(module, func) else {
            return;
        };
        let mut runner = Runner::new(module, &self.index, &self.sem, facts, &self.summaries, f, fi);
        for &bp in &fi.rpo {
            let st = fh.block_in.get(bp).cloned().unwrap_or_default();
            if !st.reached {
                continue;
            }
            runner.run_block(bp, st, Some(&mut *visitor));
        }
    }

    /// The abstract memory just before `point`, if the point is reachable.
    pub fn state_at(
        &self,
        module: &ModuleIr,
        facts: &dyn MemoryFacts,
        point: ProgramPoint,
    ) -> Option<HeapState> {
        let mut found = None;
        self.replay(module, facts, point.func, &mut |p, _, st| {
            if p == point && found.is_none() {
                found = Some(st.clone());
            }
        });
        found
    }

    /// Every event of every function, in function then program order.
    pub fn all_events(&self) -> Vec<&MemEvent> {
        let mut ids: Vec<u32> = self.funcs.keys().copied().collect();
        ids.sort_unstable();
        ids.iter()
            .flat_map(|f| self.funcs[f].events.iter())
            .collect()
    }
}

fn analyze_func(
    module: &ModuleIr,
    index: &ModuleIndex,
    sem: &Semantics,
    facts: &dyn MemoryFacts,
    summaries: &HashMap<u32, FuncSummary>,
    func: u32,
) -> Option<FuncHeap> {
    let (f, fi) = index.func(module, func)?;
    let mut runner = Runner::new(module, index, sem, facts, summaries, f, fi);
    let n = f.blocks.len();
    let mut block_in = vec![HeapState::default(); n];
    if n == 0 {
        return Some(FuncHeap {
            func,
            converged: true,
            ..FuncHeap::default()
        });
    }
    let mut entry = HeapState {
        reached: true,
        ..HeapState::default()
    };
    for p in &f.params {
        entry.vals.insert(
            p.0,
            one(AbsObj::Ext {
                func,
                value: p.0,
            }),
        );
    }
    block_in[fi.entry] = entry;

    // Block fixpoint in reverse post-order.
    let mut work: BTreeSet<(usize, usize)> = BTreeSet::new();
    work.insert((fi.rpo_num[fi.entry], fi.entry));
    let mut visits = vec![0usize; n];
    let mut converged = true;
    runner.record = false;
    while let Some((_, bp)) = work.pop_first() {
        visits[bp] += 1;
        if visits[bp] > MAX_BLOCK_VISITS {
            converged = false;
            continue;
        }
        let st = block_in[bp].clone();
        let outs = runner.run_block(bp, st, None);
        for (s, out) in outs {
            if block_in[s].join_from(&out, sem) {
                work.insert((fi.rpo_num[s], s));
            }
        }
    }

    // Recording sweep over the converged states.
    runner.record = true;
    runner.exits.clear();
    for &bp in &fi.rpo {
        let st = block_in[bp].clone();
        if st.reached {
            runner.run_block(bp, st, None);
        }
    }
    let summary = runner.summarize();
    Some(FuncHeap {
        func,
        block_in,
        values: std::mem::take(&mut runner.values),
        interior: std::mem::take(&mut runner.interior),
        ext_origin: std::mem::take(&mut runner.ext_origin),
        events: std::mem::take(&mut runner.events),
        exits: std::mem::take(&mut runner.exits),
        summary,
        unmodeled: std::mem::take(&mut runner.unmodeled),
        converged,
    })
}

// ─── The interpreter ────────────────────────────────────────────────────────────

/// A place resolved against the current state.
enum Resolved {
    /// Slots of abstract objects.
    Cells(Vec<(AbsObj, FieldKey)>),
    /// A plain SSA value used as a place (no storage behind it).
    Direct(PtsSet),
}

struct Runner<'a> {
    module: &'a ModuleIr,
    index: &'a ModuleIndex,
    sem: &'a Semantics,
    facts: &'a dyn MemoryFacts,
    summaries: &'a HashMap<u32, FuncSummary>,
    func: &'a FuncIr,
    fi: &'a FuncIndex,
    fid: u32,
    record: bool,
    events: Vec<MemEvent>,
    exits: Vec<ExitFact>,
    values: BTreeMap<u32, PtsSet>,
    interior: BTreeSet<u32>,
    ext_origin: BTreeMap<u32, (AbsObj, FieldKey)>,
    unmodeled: BTreeSet<ProgramPoint>,
    param_access: BTreeMap<u32, bool>,
}

impl<'a> Runner<'a> {
    fn new(
        module: &'a ModuleIr,
        index: &'a ModuleIndex,
        sem: &'a Semantics,
        facts: &'a dyn MemoryFacts,
        summaries: &'a HashMap<u32, FuncSummary>,
        func: &'a FuncIr,
        fi: &'a FuncIndex,
    ) -> Self {
        Runner {
            module,
            index,
            sem,
            facts,
            summaries,
            func,
            fi,
            fid: func.id.0,
            record: false,
            events: Vec::new(),
            exits: Vec::new(),
            values: BTreeMap::new(),
            interior: BTreeSet::new(),
            ext_origin: BTreeMap::new(),
            unmodeled: BTreeSet::new(),
            param_access: BTreeMap::new(),
        }
    }

    fn point(&self, bp: usize, ii: usize) -> ProgramPoint {
        ProgramPoint {
            func: self.fid,
            block: self.func.blocks[bp].id.0,
            index: ii as u32,
        }
    }

    fn param_of(&self, o: AbsObj) -> Option<u32> {
        match o {
            AbsObj::Ext { func, value } if func == self.fid => {
                self.fi.param_index.get(&value).map(|&i| i as u32)
            }
            _ => None,
        }
    }

    fn emit(&mut self, point: ProgramPoint, obj: AbsObj, kind: MemEventKind, value: Option<u32>) {
        if !obj.is_storage() {
            return;
        }
        if let MemEventKind::Accessed { write, .. } = kind {
            if let Some(p) = self.param_of(obj) {
                let e = self.param_access.entry(p).or_insert(false);
                *e |= write;
            }
        }
        if self.record {
            self.events.push(MemEvent {
                point,
                span: self.index.span_at(self.module, point),
                obj,
                kind,
                value,
            });
        }
    }

    fn eval(&self, st: &HeapState, op: &dyn OperandRef) -> PtsSet {
        if op.is_undef() {
            return one(AbsObj::Undef);
        }
        if let Some(c) = op.constant() {
            return match c {
                ConstVal::Null => one(AbsObj::Null),
                _ => PtsSet::new(),
            };
        }
        match op.value() {
            Some(v) => st.pts(v.0),
            None => PtsSet::new(),
        }
    }

    fn define(&mut self, st: &mut HeapState, dst: u32, pts: PtsSet) {
        self.values
            .entry(dst)
            .or_default()
            .extend(pts.iter().copied());
        st.vals.insert(dst, pts);
    }

    /// Whether a slot of `o` denotes exactly one concrete location (strong update).
    fn unique(&self, o: AbsObj) -> bool {
        match o {
            AbsObj::Site { old: false, .. } | AbsObj::CallAlloc { old: false, .. } => true,
            AbsObj::Ext { func, value } | AbsObj::Closure { func, value } => {
                func == self.fid
                    && match self.fi.def_site.get(&value) {
                        Some(&(bp, _)) => !self.fi.in_cycle[bp],
                        None => true,
                    }
            }
            AbsObj::Global(_) | AbsObj::Static { .. } => true,
            _ => false,
        }
    }

    fn resolve(&self, st: &HeapState, place: &Place) -> Resolved {
        fn cells(p: &PtsSet, k: FieldKey) -> Vec<(AbsObj, FieldKey)> {
            storage(p).map(|o| (o, k)).collect()
        }
        match place {
            Place::Local(v) => {
                let p = st.pts(v.0);
                let c = cells(&p, FieldKey::Whole);
                if c.is_empty() {
                    Resolved::Direct(p)
                } else {
                    Resolved::Cells(c)
                }
            }
            Place::Deref(v) => Resolved::Cells(cells(&st.pts(v.0), FieldKey::Whole)),
            Place::Field { base, field } => {
                Resolved::Cells(cells(&st.pts(base.0), FieldKey::Field(field.0)))
            }
            Place::Index { base, .. } => Resolved::Cells(cells(&st.pts(base.0), FieldKey::Elem)),
            Place::Global(g) => Resolved::Cells(vec![(AbsObj::Global(g.0), FieldKey::Whole)]),
            Place::Static { owner, field } => Resolved::Cells(vec![(
                AbsObj::Static {
                    owner: *owner,
                    field: field.0,
                },
                FieldKey::Whole,
            )]),
        }
    }

    fn allocate(&mut self, st: &mut HeapState, recent: AbsObj) {
        st.fold(recent, self.sem);
        st.life.insert(recent, LIFE_LIVE);
        st.clear_cells(recent);
        st.released_at.remove(&recent);
        st.release_kinds.remove(&recent);
        st.escaped.remove(&recent);
    }

    /// Runs one block from `st`; returns the out-states per successor position.
    fn run_block(
        &mut self,
        bp: usize,
        mut st: HeapState,
        mut visitor: Option<&mut dyn FnMut(ProgramPoint, Step<'_>, &HeapState)>,
    ) -> Vec<(usize, HeapState)> {
        let func = self.func;
        let fi = self.fi;
        let block = &func.blocks[bp];
        let mut outs: Vec<(usize, HeapState)> = Vec::new();

        // Leading phis read their operands simultaneously.
        let mut first = 0;
        let mut phi_vals: Vec<(u32, PtsSet)> = Vec::new();
        while let Some(Inst::Phi { dst, incoming }) = block.insts.get(first) {
            let point = self.point(bp, first);
            if let Some(v) = visitor.as_deref_mut() {
                v(point, Step::Inst(&block.insts[first]), &st);
            }
            let mut p = PtsSet::new();
            for (_, op) in incoming {
                p.extend(self.eval(&st, op));
            }
            phi_vals.push((dst.0, p));
            first += 1;
        }
        for (d, p) in phi_vals {
            self.define(&mut st, d, p);
        }

        for ii in first..block.insts.len() {
            let inst = &block.insts[ii];
            let point = self.point(bp, ii);
            if let Some(v) = visitor.as_deref_mut() {
                v(point, Step::Inst(inst), &st);
            }
            let unwind_pre = match inst {
                Inst::Call {
                    unwind: Some(u), ..
                } => fi.block_pos.get(&u.0).map(|&p| (p, st.clone())),
                _ => None,
            };
            self.step(&mut st, point, inst);
            if let Some((u, mut pre)) = unwind_pre {
                // The callee may throw before or after its effects.
                pre.join_from(&st, self.sem);
                outs.push((u, pre));
            }
        }

        let tpoint = self.point(bp, block.insts.len());
        if let Some(v) = visitor.as_deref_mut() {
            v(tpoint, Step::Term(&block.term), &st);
        }
        match &block.term {
            Terminator::Return(op) => {
                let returned = match op {
                    Some(o) => self.eval(&st, o),
                    None => PtsSet::new(),
                };
                for o in storage(&returned).collect::<Vec<_>>() {
                    self.emit(tpoint, o, MemEventKind::Returned, None);
                }
                self.exit(tpoint, block, &st, returned, false);
            }
            Terminator::Throw(op) => {
                let thrown = self.eval(&st, op);
                for o in storage(&thrown).collect::<Vec<_>>() {
                    self.emit(tpoint, o, MemEventKind::Thrown, None);
                    st.escaped.insert(o);
                }
                self.exit(tpoint, block, &st, thrown, true);
            }
            _ => {}
        }
        let unwind_targets: Vec<usize> = outs.iter().map(|(u, _)| *u).collect();
        for &s in &fi.succs[bp] {
            if unwind_targets.contains(&s) && !term_targets(block, s, fi) {
                continue;
            }
            outs.push((s, st.clone()));
        }
        outs
    }

    fn exit(
        &mut self,
        point: ProgramPoint,
        block: &BlockIr,
        st: &HeapState,
        returned: PtsSet,
        exceptional: bool,
    ) {
        if !self.record {
            return;
        }
        // Objects no root reaches once the frame is popped.
        let mut roots: Vec<AbsObj> = storage(&returned).collect();
        roots.extend(st.escaped.iter().copied());
        let objects = st.objects();
        for o in &objects {
            if matches!(
                o,
                AbsObj::Ext { .. } | AbsObj::Global(_) | AbsObj::Static { .. }
            ) {
                roots.push(*o);
            }
        }
        let live = st.reachable_from(roots);
        for o in objects {
            if o.is_allocation() && st.life_of(o) & LIFE_LIVE != 0 && !live.contains(&o) {
                self.emit(point, o, MemEventKind::Unreachable, None);
            }
        }
        self.exits.push(ExitFact {
            point,
            span: SrcSpan::from_ir(&block.term_span),
            exceptional,
            returned,
            state: st.clone(),
        });
    }

    fn step(&mut self, st: &mut HeapState, point: ProgramPoint, inst: &Inst) {
        match inst {
            Inst::Assign { dst, src } => {
                let p = self.eval(st, src);
                self.define(st, dst.0, p);
            }
            Inst::Cast { dst, src, .. } => {
                let p = self.eval(st, src);
                self.define(st, dst.0, p);
            }
            Inst::Unary { dst, src, .. } => {
                let p = self.eval(st, src);
                self.define(st, dst.0, undef_only(&p));
            }
            Inst::Binary { dst, lhs, rhs, .. } => {
                let l = self.eval(st, lhs);
                let r = self.eval(st, rhs);
                let mut out = undef_only(&l);
                out.extend(undef_only(&r));
                // Pointer arithmetic: exactly one operand references storage.
                let ls = storage(&l).next().is_some();
                let rs = storage(&r).next().is_some();
                if ls != rs {
                    out.extend(storage(if ls { &l } else { &r }));
                    self.interior.insert(dst.0);
                }
                self.define(st, dst.0, out);
            }
            Inst::Compare { dst, lhs, rhs, .. } => {
                let mut out = undef_only(&self.eval(st, lhs));
                out.extend(undef_only(&self.eval(st, rhs)));
                self.define(st, dst.0, out);
            }
            Inst::TypeTest { dst, src, .. } => {
                let p = self.eval(st, src);
                self.define(st, dst.0, undef_only(&p));
            }
            Inst::Phi { dst, incoming } => {
                let mut p = PtsSet::new();
                for (_, op) in incoming {
                    p.extend(self.eval(st, op));
                }
                self.define(st, dst.0, p);
            }
            Inst::Alloc {
                dst, site, region, ..
            } => {
                let recent = AbsObj::Site {
                    site: site.0,
                    old: false,
                };
                self.allocate(st, recent);
                self.define(st, dst.0, one(recent));
                self.emit(
                    point,
                    recent,
                    MemEventKind::Allocated {
                        region: MemRegion::from_ir(region),
                    },
                    Some(dst.0),
                );
            }
            Inst::Load { dst, place } => {
                let out = match self.resolve(st, place) {
                    Resolved::Direct(p) => p,
                    Resolved::Cells(cells) => {
                        let mut out = PtsSet::new();
                        for cell in &cells {
                            out.extend(st.contents(cell, self.sem));
                            self.emit(
                                point,
                                cell.0,
                                MemEventKind::Accessed {
                                    write: false,
                                    slot: cell.1,
                                    via_call: false,
                                },
                                Some(dst.0),
                            );
                        }
                        if out.remove(&AbsObj::Unknown) {
                            out.insert(AbsObj::Ext {
                                func: self.fid,
                                value: dst.0,
                            });
                            if cells.len() == 1 {
                                self.ext_origin.insert(dst.0, cells[0]);
                            }
                        }
                        out
                    }
                };
                self.define(st, dst.0, out);
            }
            Inst::Store { place, value } => {
                let pv = self.eval(st, value);
                let stored = value.value().map(|v| v.0);
                if let Resolved::Cells(cells) = self.resolve(st, place) {
                    let strong = cells.len() == 1 && self.unique(cells[0].0);
                    for cell in cells {
                        let mut c = if strong {
                            PtsSet::new()
                        } else {
                            st.contents(&cell, self.sem)
                        };
                        c.extend(pv.iter().copied());
                        st.heap.insert(cell, c);
                        self.emit(
                            point,
                            cell.0,
                            MemEventKind::Accessed {
                                write: true,
                                slot: cell.1,
                                via_call: false,
                            },
                            stored,
                        );
                        for o in storage(&pv).collect::<Vec<_>>() {
                            self.emit(
                                point,
                                o,
                                MemEventKind::StoredInto {
                                    container: cell.0,
                                    slot: cell.1,
                                },
                                stored,
                            );
                        }
                    }
                }
            }
            Inst::MakeClosure { dst, captures, .. } => {
                let obj = AbsObj::Closure {
                    func: self.fid,
                    value: dst.0,
                };
                let strong = self.unique(obj);
                for (i, c) in captures.iter().enumerate() {
                    let p = self.eval(st, c);
                    let cell = (obj, FieldKey::Capture(i as u32));
                    let mut contents = if strong {
                        PtsSet::new()
                    } else {
                        st.contents(&cell, self.sem)
                    };
                    contents.extend(p.iter().copied());
                    st.heap.insert(cell, contents);
                    for o in storage(&p).collect::<Vec<_>>() {
                        self.emit(
                            point,
                            o,
                            MemEventKind::Captured {
                                closure: obj,
                                index: i as u32,
                            },
                            c.value().map(|v| v.0),
                        );
                    }
                }
                self.define(st, dst.0, one(obj));
            }
            Inst::Release { place } => self.release_place(st, point, place),
            Inst::Call {
                dst,
                site,
                callee,
                args,
                ..
            } => {
                let ops: Vec<&dyn OperandRef> = args.iter().map(|a| a as &dyn OperandRef).collect();
                self.call(st, point, dst.map(|d| d.0), site.0, callee, &ops);
            }
            Inst::Intrinsic { dst, name, args } => {
                let ops: Vec<&dyn OperandRef> = args.iter().map(|a| a as &dyn OperandRef).collect();
                let arg_pts: Vec<PtsSet> = ops.iter().map(|a| self.eval(st, *a)).collect();
                let d = dst.map(|d| d.0);
                let result = match self.facts.intrinsic_effects(FuncId(self.fid), *name) {
                    Some(effects) => self.apply_effects(st, point, d, None, &arg_pts, &effects),
                    None => self.unmodeled_call(st, point, d, &arg_pts),
                };
                if let Some(d) = d {
                    self.define(st, d, result);
                }
            }
        }
    }

    fn release_objects(
        &mut self,
        st: &mut HeapState,
        point: ProgramPoint,
        objs: &[AbsObj],
        kind: Option<ReleaseKind>,
        must: bool,
    ) {
        let strong = must && objs.len() == 1 && self.unique(objs[0]);
        for &o in objs {
            self.emit(point, o, MemEventKind::Released { kind, must: strong }, None);
            if kind == Some(ReleaseKind::Unlock) {
                continue;
            }
            let bits = st.life_of(o);
            let new = if strong {
                (bits & !LIFE_LIVE) | LIFE_RELEASED
            } else {
                bits | LIFE_RELEASED
            };
            st.life.insert(o, new);
            st.released_at.entry(o).or_default().insert(point);
            *st.release_kinds.entry(o).or_default() |= rel_bit(kind);
        }
    }

    fn type_is_owning(&self, ty: TypeId) -> bool {
        matches!(
            self.module.types.get(ty),
            Some(IrType::Ref {
                kind: RefKind::Owning,
                ..
            })
        )
    }

    fn field_is_owning(&self, field: u32) -> bool {
        self.index
            .field_pos
            .get(&field)
            .map(|&fp| self.type_is_owning(self.module.fields[fp].ty))
            .unwrap_or(false)
    }

    /// `Release { place }`: a stack slot ends its scope (an owning slot deallocates what
    /// it holds); a plain reference releases its objects; a field, element or global place
    /// releases the objects it holds.
    fn release_place(&mut self, st: &mut HeapState, point: ProgramPoint, place: &Place) {
        let supplied = self.facts.release_kind(point);
        let cells = match self.resolve(st, place) {
            Resolved::Direct(p) => {
                let objs: Vec<AbsObj> = storage(&p).collect();
                self.release_objects(st, point, &objs, supplied, true);
                return;
            }
            Resolved::Cells(cells) => cells,
        };
        let must = cells.len() == 1;
        let slot_place = !matches!(place, Place::Local(_) | Place::Deref(_));
        if slot_place {
            let owning = match place {
                Place::Field { field, .. } | Place::Static { field, .. } => {
                    self.field_is_owning(field.0)
                }
                _ => false,
            };
            let kind = supplied.or(if owning {
                Some(ReleaseKind::Deallocate)
            } else {
                None
            });
            let mut held: Vec<AbsObj> = Vec::new();
            for cell in &cells {
                held.extend(storage(&st.contents(cell, self.sem)));
            }
            let single = held.len() == 1;
            self.release_objects(st, point, &held, kind, must && single);
            return;
        }
        for (o, k) in cells {
            let stack = self.sem.region(o) == Some(MemRegion::Stack);
            if stack {
                let owning = o
                    .site()
                    .and_then(|s| self.index.site_type(self.module, s))
                    .map(|t| self.type_is_owning(t))
                    .unwrap_or(false);
                if owning {
                    let held: Vec<AbsObj> = storage(&st.contents(&(o, k), self.sem)).collect();
                    let single = held.len() == 1;
                    self.release_objects(
                        st,
                        point,
                        &held,
                        Some(ReleaseKind::Deallocate),
                        must && single,
                    );
                }
            }
            let kind = supplied.or(if stack {
                Some(ReleaseKind::EndOfScope)
            } else {
                None
            });
            self.release_objects(st, point, &[o], kind, must);
        }
    }

    /// Parameter-ordered actuals for callee `t` (receiver first when the callee declares
    /// it as a parameter).
    fn actuals(&self, t: u32, recv: &Option<PtsSet>, args: &[PtsSet]) -> Vec<PtsSet> {
        let n = self
            .index
            .func(self.module, t)
            .map(|(f, _)| f.params.len())
            .unwrap_or(args.len());
        match recv {
            Some(r) if n == args.len() + 1 => {
                let mut v = vec![r.clone()];
                v.extend(args.iter().cloned());
                v
            }
            _ => args.to_vec(),
        }
    }

    fn map_sum(
        &self,
        st: &HeapState,
        objs: &BTreeSet<SumObj>,
        actuals: &[PtsSet],
        dst: Option<u32>,
    ) -> PtsSet {
        let fresh = match dst {
            Some(d) => AbsObj::Ext {
                func: self.fid,
                value: d,
            },
            None => AbsObj::Unknown,
        };
        let mut out = PtsSet::new();
        for s in objs {
            match s {
                SumObj::Param(i) => {
                    if let Some(a) = actuals.get(*i as usize) {
                        out.extend(a.iter().copied());
                    }
                }
                SumObj::ParamField(i, k) => {
                    if let Some(a) = actuals.get(*i as usize) {
                        for o in storage(a) {
                            for c in st.contents(&(o, *k), self.sem) {
                                out.insert(if c == AbsObj::Unknown { fresh } else { c });
                            }
                        }
                    }
                }
                SumObj::Obj(o) => {
                    out.insert(*o);
                }
                SumObj::Fresh => {
                    out.insert(fresh);
                }
            }
        }
        out
    }

    fn escape(&mut self, st: &mut HeapState, point: ProgramPoint, objs: Vec<AbsObj>) {
        for o in objs {
            if st.escaped.insert(o) {
                self.emit(point, o, MemEventKind::Escaped, None);
            }
        }
    }

    fn call(
        &mut self,
        st: &mut HeapState,
        point: ProgramPoint,
        dst: Option<u32>,
        site: u32,
        callee: &Callee,
        args: &[&dyn OperandRef],
    ) {
        let summaries = self.summaries;
        let sem = self.sem;
        let receiver = match callee {
            Callee::Virtual { receiver, .. } | Callee::Interface { receiver, .. } => {
                Some(receiver.0)
            }
            _ => None,
        };
        let targets = self.index.callees.get(&site).cloned().unwrap_or_default();
        if let Some(r) = receiver {
            for o in storage(&st.pts(r)).collect::<Vec<_>>() {
                let kind = MemEventKind::Passed {
                    call_site: site,
                    callees: targets.clone(),
                    arg: None,
                };
                self.emit(point, o, kind, Some(r));
            }
        }
        for (i, a) in args.iter().enumerate() {
            for o in storage(&self.eval(st, *a)).collect::<Vec<_>>() {
                let kind = MemEventKind::Passed {
                    call_site: site,
                    callees: targets.clone(),
                    arg: Some(i as u32),
                };
                self.emit(point, o, kind, a.value().map(|v| v.0));
            }
        }

        let result = if targets.is_empty() {
            let arg_pts: Vec<PtsSet> = args.iter().map(|a| self.eval(st, *a)).collect();
            match sem.call_effects.get(&site) {
                Some(effects) => self.apply_effects(st, point, dst, Some(site), &arg_pts, effects),
                None => {
                    let mut all = arg_pts;
                    if let Some(r) = receiver {
                        all.push(st.pts(r));
                    }
                    self.unmodeled_call(st, point, dst, &all)
                }
            }
        } else {
            // Recency at calls: the callee's allocations make earlier instances old.
            let mut allocs: BTreeSet<AbsObj> = BTreeSet::new();
            for t in &targets {
                if let Some(s) = summaries.get(t) {
                    allocs.extend(s.allocates.iter().copied());
                }
            }
            for a in allocs {
                st.fold(a, sem);
            }
            let arg_pts: Vec<PtsSet> = args.iter().map(|a| self.eval(st, *a)).collect();
            let recv_pts = receiver.map(|r| st.pts(r));
            let single = targets.len() == 1;
            let mut result = PtsSet::new();
            for t in &targets {
                let Some(sum) = summaries.get(t) else {
                    continue;
                };
                let actuals = self.actuals(*t, &recv_pts, &arg_pts);
                for (&pi, &w) in &sum.accesses {
                    if let Some(a) = actuals.get(pi as usize) {
                        for o in storage(a).collect::<Vec<_>>() {
                            let kind = MemEventKind::Accessed {
                                write: w,
                                slot: FieldKey::Whole,
                                via_call: true,
                            };
                            self.emit(point, o, kind, None);
                        }
                    }
                }
                for (&pi, rel) in &sum.released {
                    if let Some(a) = actuals.get(pi as usize) {
                        let objs: Vec<AbsObj> = storage(a).collect();
                        let kind = kind_from_bits(rel.kinds);
                        self.release_objects(st, point, &objs, kind, single && rel.must);
                    }
                }
                for &pi in &sum.escapes {
                    if let Some(a) = actuals.get(pi as usize) {
                        self.escape(st, point, storage(a).collect());
                    }
                }
                for ((pi, k), vals) in &sum.param_stores {
                    let mapped = self.map_sum(st, vals, &actuals, None);
                    if let Some(a) = actuals.get(*pi as usize) {
                        for o in storage(a).collect::<Vec<_>>() {
                            let cell = (o, *k);
                            let mut c = st.contents(&cell, sem);
                            c.extend(mapped.iter().copied());
                            st.heap.insert(cell, c);
                        }
                    }
                }
                for (o, &bits) in &sum.out_life {
                    let new = match st.life.get(o) {
                        Some(&c) if !single => c | bits,
                        _ => bits,
                    };
                    st.life.insert(*o, new);
                }
                result.extend(self.map_sum(st, &sum.ret, &actuals, dst));
            }
            result
        };
        if let Some(d) = dst {
            self.define(st, d, result);
        }
    }

    /// Effects supplied by the language layer for a call or intrinsic without an IR body.
    fn apply_effects(
        &mut self,
        st: &mut HeapState,
        point: ProgramPoint,
        dst: Option<u32>,
        site: Option<u32>,
        args: &[PtsSet],
        effects: &[CallEffect],
    ) -> PtsSet {
        let mut handled = vec![false; args.len()];
        let complete = effects.contains(&CallEffect::Complete);
        let mut result = PtsSet::new();
        let mut has_result = false;
        let objs_of = |i: usize| -> Vec<AbsObj> {
            args.get(i).map(|p| storage(p).collect()).unwrap_or_default()
        };
        let mark = |handled: &mut Vec<bool>, i: usize| {
            if let Some(h) = handled.get_mut(i) {
                *h = true;
            }
        };
        for e in effects {
            match *e {
                CallEffect::Free { arg } => {
                    let objs = objs_of(arg);
                    self.release_objects(st, point, &objs, Some(ReleaseKind::Deallocate), true);
                    mark(&mut handled, arg);
                }
                CallEffect::Dispose { arg } => {
                    let objs = objs_of(arg);
                    self.release_objects(st, point, &objs, Some(ReleaseKind::Dispose), true);
                    mark(&mut handled, arg);
                }
                CallEffect::Acquire { arg } | CallEffect::Unlock { arg } | CallEffect::Join { arg } => {
                    mark(&mut handled, arg)
                }
                CallEffect::Reads { arg } | CallEffect::Writes { arg } => {
                    let write = matches!(e, CallEffect::Writes { .. });
                    for o in objs_of(arg) {
                        let kind = MemEventKind::Accessed {
                            write,
                            slot: FieldKey::Whole,
                            via_call: true,
                        };
                        self.emit(point, o, kind, None);
                        if write {
                            let keys: Vec<FieldKey> =
                                st.cells_of(o).into_iter().map(|(k, _)| k).collect();
                            for k in keys {
                                st.heap.entry((o, k)).or_default().insert(AbsObj::Unknown);
                            }
                        }
                    }
                    mark(&mut handled, arg);
                }
                CallEffect::Spawn { arg } | CallEffect::Escapes { arg } => {
                    self.escape(st, point, objs_of(arg));
                    mark(&mut handled, arg);
                }
                CallEffect::AllocatesResult { .. } => {
                    has_result = true;
                    match site {
                        Some(cs) => {
                            let recent = AbsObj::CallAlloc {
                                call_site: cs,
                                old: false,
                            };
                            self.allocate(st, recent);
                            result.insert(recent);
                            let kind = MemEventKind::Allocated {
                                region: MemRegion::Heap,
                            };
                            self.emit(point, recent, kind, dst);
                        }
                        None => {
                            if let Some(d) = dst {
                                result.insert(AbsObj::Ext {
                                    func: self.fid,
                                    value: d,
                                });
                            }
                        }
                    }
                }
                CallEffect::ReturnsArg { arg } => {
                    has_result = true;
                    if let Some(p) = args.get(arg) {
                        result.extend(p.iter().copied());
                    }
                }
                CallEffect::Complete => {}
            }
        }
        if !complete {
            let rest: Vec<AbsObj> = handled
                .iter()
                .enumerate()
                .filter(|(_, h)| !**h)
                .flat_map(|(i, _)| objs_of(i))
                .collect();
            self.escape(st, point, rest);
        }
        if let Some(d) = dst {
            if !has_result && !complete {
                result.insert(AbsObj::Ext {
                    func: self.fid,
                    value: d,
                });
            }
        }
        result
    }

    /// A call with no body and no effects: everything reachable from the arguments
    /// escapes and may be overwritten; the result is opaque.
    fn unmodeled_call(
        &mut self,
        st: &mut HeapState,
        point: ProgramPoint,
        dst: Option<u32>,
        args: &[PtsSet],
    ) -> PtsSet {
        self.unmodeled.insert(point);
        let roots: Vec<AbsObj> = args.iter().flat_map(|p| storage(p)).collect();
        let reach: Vec<AbsObj> = st.reachable_from(roots).into_iter().collect();
        for &o in &reach {
            let keys: Vec<FieldKey> = st.cells_of(o).into_iter().map(|(k, _)| k).collect();
            for k in keys {
                st.heap.entry((o, k)).or_default().insert(AbsObj::Unknown);
            }
        }
        self.escape(st, point, reach);
        match dst {
            Some(d) => one(AbsObj::Ext {
                func: self.fid,
                value: d,
            }),
            None => PtsSet::new(),
        }
    }

    fn to_sum(&self, o: AbsObj) -> SumObj {
        match o {
            AbsObj::Ext { func, value } if func == self.fid => {
                if let Some(&i) = self.fi.param_index.get(&value) {
                    return SumObj::Param(i as u32);
                }
                if let Some((base, k)) = self.ext_origin.get(&value) {
                    if let Some(i) = self.param_of(*base) {
                        return SumObj::ParamField(i, *k);
                    }
                }
                SumObj::Fresh
            }
            AbsObj::Ext { .. } | AbsObj::Unknown => SumObj::Fresh,
            other => SumObj::Obj(other),
        }
    }

    fn summarize(&self) -> FuncSummary {
        let mut s = FuncSummary {
            accesses: self.param_access.clone(),
            ..FuncSummary::default()
        };
        let params: Vec<AbsObj> = self
            .func
            .params
            .iter()
            .map(|p| AbsObj::Ext {
                func: self.fid,
                value: p.0,
            })
            .collect();
        let mut must_release: BTreeMap<u32, bool> = BTreeMap::new();
        for exit in &self.exits {
            let st = &exit.state;
            if !exit.exceptional {
                for o in &exit.returned {
                    s.ret.insert(self.to_sum(*o));
                }
            }
            for (i, p) in params.iter().enumerate() {
                let bits = st.life_of(*p);
                let i = i as u32;
                if bits & LIFE_RELEASED != 0 {
                    let e = s.released.entry(i).or_insert(ParamRelease {
                        must: true,
                        kinds: 0,
                    });
                    e.kinds |= st.release_kinds.get(p).copied().unwrap_or(REL_UNKNOWN);
                }
                let m = must_release.entry(i).or_insert(true);
                *m &= bits & LIFE_RELEASED != 0 && bits & LIFE_LIVE == 0;
            }
            // Escape: reachable from memory outside both frames.
            let mut roots: Vec<AbsObj> = st.escaped.iter().copied().collect();
            for o in st.objects() {
                if matches!(o, AbsObj::Global(_) | AbsObj::Static { .. }) {
                    roots.push(o);
                }
            }
            if exit.exceptional {
                roots.extend(storage(&exit.returned));
            }
            let outside = st.reachable_from(roots);
            for (i, p) in params.iter().enumerate() {
                if outside.contains(p) {
                    s.escapes.insert(i as u32);
                }
            }
            for ((o, k), contents) in &st.heap {
                if let Some(i) = self.param_of(*o) {
                    let mapped: BTreeSet<SumObj> = contents
                        .iter()
                        .filter(|c| **c != AbsObj::Unknown)
                        .map(|c| self.to_sum(*c))
                        .filter(|c| *c != SumObj::ParamField(i, *k))
                        .collect();
                    if !mapped.is_empty() {
                        s.param_stores.entry((i, *k)).or_default().extend(mapped);
                    }
                }
            }
            let mut visible: Vec<AbsObj> = storage(&exit.returned).collect();
            visible.extend(params.iter().copied());
            for o in st.reachable_from(visible) {
                if o.is_allocation() {
                    *s.out_life.entry(o).or_default() |= st.life_of(o);
                }
            }
        }
        for (i, m) in must_release {
            if let Some(r) = s.released.get_mut(&i) {
                r.must = m;
            }
        }
        s.reaches_unmodeled = !self.unmodeled.is_empty();
        for block in &self.func.blocks {
            for inst in &block.insts {
                match inst {
                    Inst::Alloc { site, .. } => {
                        s.allocates.insert(AbsObj::Site {
                            site: site.0,
                            old: false,
                        });
                    }
                    Inst::Call { site, .. } => {
                        if self.sem.call_alloc_init.contains_key(&site.0) {
                            s.allocates.insert(AbsObj::CallAlloc {
                                call_site: site.0,
                                old: false,
                            });
                        }
                        for t in self.index.callees.get(&site.0).into_iter().flatten() {
                            if let Some(cs) = self.summaries.get(t) {
                                s.allocates.extend(cs.allocates.iter().copied());
                                s.reaches_unmodeled |= cs.reaches_unmodeled;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        s
    }
}

/// Keeps only the `Undef` marker of a set (results of non-pointer operations).
fn undef_only(p: &PtsSet) -> PtsSet {
    if p.contains(&AbsObj::Undef) {
        one(AbsObj::Undef)
    } else {
        PtsSet::new()
    }
}

/// The single release kind of a `REL_*` mask, `None` when unknown or mixed.
pub fn kind_from_bits(bits: u8) -> Option<ReleaseKind> {
    match bits {
        REL_DEALLOCATE => Some(ReleaseKind::Deallocate),
        REL_DISPOSE => Some(ReleaseKind::Dispose),
        REL_END_OF_SCOPE => Some(ReleaseKind::EndOfScope),
        _ => None,
    }
}

/// Whether block position `s` is a terminator target of `block` (not only an unwind
/// target of one of its calls).
fn term_targets(block: &BlockIr, s: usize, fi: &FuncIndex) -> bool {
    let pos = |id: u32| fi.block_pos.get(&id).copied();
    match &block.term {
        Terminator::Jump(t) => pos(t.0) == Some(s),
        Terminator::Branch { then_b, else_b, .. } => {
            pos(then_b.0) == Some(s) || pos(else_b.0) == Some(s)
        }
        Terminator::Switch { cases, default, .. } => {
            pos(default.0) == Some(s) || cases.iter().any(|(_, t)| pos(t.0) == Some(s))
        }
        _ => false,
    }
}

// ─── Heap-shape graphs ──────────────────────────────────────────────────────────

/// Lifetime state of an object at a point, from its lifetime bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifeState {
    Live,
    Released,
    MaybeReleased,
    MaybeUnallocated,
    Unallocated,
}

impl LifeState {
    pub fn from_bits(bits: u8) -> Self {
        let live = bits & LIFE_LIVE != 0;
        let rel = bits & LIFE_RELEASED != 0;
        let unalloc = bits & LIFE_UNALLOCATED != 0;
        match (live, rel, unalloc) {
            (true, false, false) => Self::Live,
            (false, true, false) => Self::Released,
            (_, true, _) => Self::MaybeReleased,
            (true, false, true) => Self::MaybeUnallocated,
            (false, false, _) => Self::Unallocated,
        }
    }
}

/// One abstract object in a heap-shape graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShapeNode {
    pub key: String,
    pub obj: AbsObj,
    /// Allocation site (or allocating call site) the object comes from.
    pub site: Option<u32>,
    pub region: Option<MemRegion>,
    /// Allocated type, when the allocation site states it.
    pub type_id: Option<u32>,
    pub life: LifeState,
    /// True when the node stands for many concrete objects (recency summary, or opaque
    /// memory produced repeatedly).
    pub summary: bool,
    pub escaped: bool,
    /// SSA values that reference the object at this point.
    pub roots: Vec<u32>,
    pub spans: Vec<SrcSpan>,
    pub derivation: Derivation,
}

/// A slot of one object that references another.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShapeEdge {
    pub from: String,
    pub to: String,
    pub slot: FieldKey,
    /// Interned field name for declared fields.
    pub field_name: Option<u32>,
    /// True when the slot denotes one location holding exactly this object.
    pub definite: bool,
    pub derivation: Derivation,
}

/// Shape class of a recursive structure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StructureKind {
    /// One allocation site linked through one slot.
    SinglyLinked,
    /// One allocation site linked through two mutually inverse slots.
    DoublyLinked,
    /// One allocation site linked through two or more slots without inverse pairing.
    Tree,
    /// Several allocation sites that reference each other.
    MutuallyRecursive,
    /// A cycle of definite edges among single concrete objects.
    Cyclic,
}

/// A summarized recursive structure: a cycle of the allocation-site graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecursiveStructure {
    pub kind: StructureKind,
    /// Node keys of the structure.
    pub nodes: Vec<String>,
    /// Slots whose edges stay inside the structure.
    pub slots: Vec<FieldKey>,
    pub derivation: Derivation,
}

/// The heap at one program point.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeapShapeGraph {
    pub point: ProgramPoint,
    pub span: SrcSpan,
    pub nodes: Vec<ShapeNode>,
    pub edges: Vec<ShapeEdge>,
    pub structures: Vec<RecursiveStructure>,
    /// Slots that may hold null (or zero).
    pub null_slots: Vec<(String, FieldKey)>,
    /// Slots that may hold an uninitialized value.
    pub uninit_slots: Vec<(String, FieldKey)>,
    pub derivation: Derivation,
}

/// Heap-shape graph just before `point`; `None` when the point is unreachable.
pub fn heap_shape_at(
    result: &HeapResult,
    module: &ModuleIr,
    facts: &dyn MemoryFacts,
    point: ProgramPoint,
) -> Option<HeapShapeGraph> {
    let st = result.state_at(module, facts, point)?;
    Some(heap_shape_of(result, module, point, &st))
}

/// Heap-shape graphs at every exit of a function (before its frame is popped).
pub fn heap_shapes_at_exits(
    result: &HeapResult,
    module: &ModuleIr,
    func: u32,
) -> Vec<HeapShapeGraph> {
    result
        .funcs
        .get(&func)
        .map(|fh| {
            fh.exits
                .iter()
                .map(|e| heap_shape_of(result, module, e.point, &e.state))
                .collect()
        })
        .unwrap_or_default()
}

/// Builds the shape graph of an abstract state.
pub fn heap_shape_of(
    result: &HeapResult,
    module: &ModuleIr,
    point: ProgramPoint,
    st: &HeapState,
) -> HeapShapeGraph {
    let index = &result.index;
    let in_cycle_value = |func: u32, value: u32| -> bool {
        index
            .func(module, func)
            .and_then(|(_, fi)| fi.def_site.get(&value).map(|&(bp, _)| fi.in_cycle[bp]))
            .unwrap_or(false)
    };
    let mut roots: BTreeMap<AbsObj, Vec<u32>> = BTreeMap::new();
    for (v, p) in &st.vals {
        for o in storage(p) {
            roots.entry(o).or_default().push(*v);
        }
    }
    let objects: Vec<AbsObj> = st
        .objects()
        .into_iter()
        .filter(|o| st.life_of(*o) != LIFE_UNALLOCATED)
        .collect();

    let mut derivation = Derivation::Exact;
    let mut nodes = Vec::with_capacity(objects.len());
    for &o in &objects {
        let (site, type_id, spans) = match o {
            AbsObj::Site { site, .. } => (
                Some(site),
                index.site_type(module, site).map(|t| t.0),
                vec![index.site_span(module, site)],
            ),
            AbsObj::CallAlloc { call_site, .. } => (
                Some(call_site),
                None,
                index
                    .call_site_pos
                    .get(&call_site)
                    .map(|&p| vec![SrcSpan::from_ir(&module.call_sites[p].span)])
                    .unwrap_or_default(),
            ),
            AbsObj::Ext { func, value } | AbsObj::Closure { func, value } => {
                let span = index
                    .func(module, func)
                    .and_then(|(f, fi)| {
                        fi.def_site.get(&value).map(|&(bp, ii)| {
                            f.blocks[bp]
                                .inst_spans
                                .get(ii)
                                .map(SrcSpan::from_ir)
                                .unwrap_or(SrcSpan::NONE)
                        })
                    })
                    .unwrap_or(SrcSpan::NONE);
                (None, None, if span.is_none() { Vec::new() } else { vec![span] })
            }
            _ => (None, None, Vec::new()),
        };
        let summary = match o {
            AbsObj::Site { old, .. } | AbsObj::CallAlloc { old, .. } => old,
            AbsObj::Ext { func, value } | AbsObj::Closure { func, value } => {
                in_cycle_value(func, value)
            }
            _ => false,
        };
        let node_derivation = match o {
            AbsObj::Ext { .. } => Derivation::partial(
                "memory the function did not allocate; its contents are known only through \
                 the accesses made here",
            ),
            _ if summary => Derivation::OverApproximate,
            _ => Derivation::Exact,
        };
        derivation = derivation.weakest(node_derivation.clone());
        nodes.push(ShapeNode {
            key: o.key(),
            obj: o,
            site,
            region: result.sem.region(o),
            type_id,
            life: LifeState::from_bits(st.life_of(o)),
            summary,
            escaped: st.escaped.contains(&o),
            roots: roots.get(&o).cloned().unwrap_or_default(),
            spans,
            derivation: node_derivation,
        });
    }

    let mut edges = Vec::new();
    let mut null_slots = Vec::new();
    let mut uninit_slots = Vec::new();
    for ((o, k), contents) in &st.heap {
        if st.life_of(*o) == LIFE_UNALLOCATED {
            continue;
        }
        let field_name = match k {
            FieldKey::Field(f) => index.field_pos.get(f).map(|&p| module.fields[p].name),
            _ => None,
        };
        let targets: Vec<AbsObj> = storage(contents).collect();
        let summary_src = o.is_old();
        let definite = !summary_src && contents.len() == 1 && targets.len() == 1;
        for t in targets {
            edges.push(ShapeEdge {
                from: o.key(),
                to: t.key(),
                slot: *k,
                field_name,
                definite,
                derivation: if definite {
                    Derivation::Exact
                } else {
                    Derivation::OverApproximate
                },
            });
        }
        if contents.contains(&AbsObj::Null) {
            null_slots.push((o.key(), *k));
        }
        if contents.contains(&AbsObj::Undef) {
            uninit_slots.push((o.key(), *k));
        }
        if contents.contains(&AbsObj::Unknown) {
            derivation = derivation.weakest(Derivation::partial(
                "some slots hold contents written by code without modeled semantics",
            ));
        }
    }

    let structures = recursive_structures(&objects, st);
    for s in &structures {
        derivation = derivation.weakest(s.derivation.clone());
    }
    HeapShapeGraph {
        point,
        span: index.span_at(module, point),
        nodes,
        edges,
        structures,
        null_slots,
        uninit_slots,
        derivation,
    }
}

/// Allocation-site grouping key: recent and old objects of one site are one group.
fn group_of(o: AbsObj) -> AbsObj {
    o.as_recent()
}

/// Finds cycles of the allocation-site graph and classifies them by the slots that close
/// them.
fn recursive_structures(objects: &[AbsObj], st: &HeapState) -> Vec<RecursiveStructure> {
    let groups: Vec<AbsObj> = objects
        .iter()
        .map(|o| group_of(*o))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let pos: HashMap<AbsObj, usize> = groups.iter().enumerate().map(|(i, g)| (*g, i)).collect();
    let mut succs = vec![Vec::new(); groups.len()];
    // (from group, to group) -> slots; also concrete object edges per slot.
    let mut slot_edges: BTreeMap<(usize, usize), BTreeSet<FieldKey>> = BTreeMap::new();
    let mut concrete: BTreeSet<(AbsObj, FieldKey, AbsObj)> = BTreeSet::new();
    for ((o, k), contents) in &st.heap {
        let Some(&a) = pos.get(&group_of(*o)) else {
            continue;
        };
        for t in storage(contents) {
            let Some(&b) = pos.get(&group_of(t)) else {
                continue;
            };
            if !succs[a].contains(&b) {
                succs[a].push(b);
            }
            slot_edges.entry((a, b)).or_default().insert(*k);
            concrete.insert((*o, *k, t));
        }
    }

    let mut out = Vec::new();
    for comp in crate::memview::sccs(&succs) {
        let cyclic = comp.len() > 1 || succs[comp[0]].contains(&comp[0]);
        if !cyclic {
            continue;
        }
        let members: BTreeSet<usize> = comp.iter().copied().collect();
        let mut slots: BTreeSet<FieldKey> = BTreeSet::new();
        for ((a, b), ks) in &slot_edges {
            if members.contains(a) && members.contains(b) {
                slots.extend(ks.iter().copied());
            }
        }
        let node_keys: Vec<String> = objects
            .iter()
            .filter(|o| pos.get(&group_of(**o)).map(|g| members.contains(g)).unwrap_or(false))
            .map(|o| o.key())
            .collect();

        // A cycle among single concrete objects through definite edges is a real cycle.
        let definite_cycle = definite_object_cycle(st, &node_keys, objects);
        let (kind, derivation) = if definite_cycle {
            (StructureKind::Cyclic, Derivation::Exact)
        } else if comp.len() > 1 {
            (
                StructureKind::MutuallyRecursive,
                Derivation::OverApproximate,
            )
        } else if slots.len() == 1 {
            (StructureKind::SinglyLinked, Derivation::OverApproximate)
        } else if inverse_pair(&slots, &concrete) {
            (StructureKind::DoublyLinked, Derivation::OverApproximate)
        } else {
            (
                StructureKind::Tree,
                Derivation::partial(
                    "the allocation-site abstraction does not track sharing: a DAG or a \
                     cyclic graph built through the same slots has the same abstract shape",
                ),
            )
        };
        out.push(RecursiveStructure {
            kind,
            nodes: node_keys,
            slots: slots.into_iter().collect(),
            derivation,
        });
    }
    out
}

/// Whether two slots are inverse on every concrete edge: `a -f-> b` implies `b -g-> a`.
fn inverse_pair(slots: &BTreeSet<FieldKey>, edges: &BTreeSet<(AbsObj, FieldKey, AbsObj)>) -> bool {
    let list: Vec<FieldKey> = slots.iter().copied().collect();
    if list.len() != 2 {
        return false;
    }
    let (f, g) = (list[0], list[1]);
    let holds = |x: FieldKey, y: FieldKey| {
        let mut any = false;
        for (a, k, b) in edges {
            if *k != x {
                continue;
            }
            any = true;
            if !edges.contains(&(*b, y, *a)) {
                return false;
            }
        }
        any
    };
    holds(f, g) && holds(g, f)
}

/// Whether the definite edges among non-summary objects of a structure form a cycle.
fn definite_object_cycle(st: &HeapState, keys: &[String], objects: &[AbsObj]) -> bool {
    let members: Vec<AbsObj> = objects
        .iter()
        .copied()
        .filter(|o| keys.contains(&o.key()) && !o.is_old())
        .collect();
    let pos: HashMap<AbsObj, usize> = members.iter().enumerate().map(|(i, o)| (*o, i)).collect();
    let mut succs = vec![Vec::new(); members.len()];
    for ((o, _), contents) in &st.heap {
        let Some(&a) = pos.get(o) else {
            continue;
        };
        if contents.len() != 1 {
            continue;
        }
        if let Some(t) = contents.iter().next() {
            if let Some(&b) = pos.get(t) {
                succs[a].push(b);
            }
        }
    }
    crate::memview::blocks_on_cycles(&succs).iter().any(|c| *c)
}
