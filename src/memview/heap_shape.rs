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
    AllocSiteId, Callee, ConstVal, FuncId, FuncIr, Inst, IrType, ModuleIr, Operand, Place,
    RefKind, Region, Terminator,
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
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Program points in the callee (transitively) without modeled semantics.
    pub unmodeled: u32,
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

impl Default for Derivation {
    fn default() -> Self {
        Derivation::Exact
    }
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
                let Some(fh) = analyze_func(module, &index, &sem, facts, &summaries, f, None)
                else {
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
            runner.run_block(bp, st, Some(visitor));
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
    _visitor: Option<&mut dyn FnMut(ProgramPoint, Step<'_>, &HeapState)>,
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
    callee_unmodeled: u32,
}
