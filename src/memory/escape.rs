//! Escape Analysis over the Points-To Graph (§M.4).
//!
//! An object *escapes* a scope when it can still be reached after the scope ends or from
//! outside it. Reachability is the object graph of the points-to result: `o → p` when some
//! slot of `o` may hold `p`. Four states, ordered by how far an object travels:
//!
//! - `NoEscape`: reachable only from the allocating function's own values and objects. Its
//!   lifetime is bounded by the allocating frame (stack allocation, scalar replacement,
//!   lock elision are sound for it), even when callees use it during the call.
//! - `ArgEscape`: reachable from the allocating function's parameters, return value or
//!   escaping exceptions: it outlives the frame but stays inside the caller's world.
//! - `GlobalEscape`: reachable from a global or static cell, from the external world, or
//!   created outside the program: its lifetime is unbounded.
//! - `ThreadEscape`: reachable from a value handed to a thread-spawn point, or from global
//!   memory in a program that spawns threads: it may be accessed concurrently.
//!
//! Every escaping object records why: the instruction that made it a root (with its span)
//! or the object it is reachable through. Thread-spawn points are frontend facts
//! (`EscapeConfig`); nothing here knows any language's threading API.

use std::collections::VecDeque;

use crate::ir::func::FuncIr;
use crate::ir::inst::{Callee, Inst, Region, Terminator};
use crate::ir::module::ModuleIr;
use crate::ir::types::{FuncId, Span};
use crate::ir::value::Operand;
use crate::memory::callgraph::ExternalTarget;
use crate::memory::location::{FxHashSet, HeapBase, ObjId, SparseBitSet};
use crate::memory::points_to::{
    intrinsic_effect, ExternalPolicy, IntrinsicEffect, MemoryConfig, PointsToResult,
};
use crate::psa::bdd::fxhash::FxHashMap;

/// How far an object travels from its allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EscapeState {
    NoEscape,
    ArgEscape,
    GlobalEscape,
    ThreadEscape,
}

/// Why an object escapes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EscapeReason {
    /// Returned by its allocating function.
    Returned { func: FuncId, span: Span },
    /// Thrown out of its allocating function.
    Thrown { func: FuncId, span: Span },
    /// Stored into memory reachable from a parameter of its allocating function.
    ReachableFromParameter { func: FuncId },
    /// Stored into a global or static cell.
    StoredInGlobal { func: FuncId, span: Span },
    /// Reachable from a global or static cell through object slots.
    ReachableFromGlobal,
    /// Handed to code outside the module (sound external policy).
    PassedToExternal { func: FuncId, span: Span },
    /// Reachable by code outside the module.
    ReachableFromExternal,
    /// Created outside the program text (entry-point argument, external origin).
    ExternalOrigin,
    /// Static storage: string literals and `Region::Static` allocations.
    StaticStorage,
    /// Handed to a thread-spawn point.
    PassedToThread { func: FuncId, span: Span },
    /// Reachable from an object handed to a thread-spawn point.
    ReachableFromThread,
    /// Reachable from global memory while the program spawns threads.
    SharedThroughGlobal,
}

/// Escape facts of one object.
#[derive(Debug, Clone)]
pub struct ObjectEscape {
    pub state: EscapeState,
    pub reasons: Vec<EscapeReason>,
    /// The object it is reachable through on the path from the escape root; `None` when
    /// the object is a root itself or does not escape.
    pub via: Option<ObjId>,
    /// A stack object that is reachable after its frame ends: a dangling reference.
    pub outlives_frame: bool,
}

/// Escape facts of the objects one function allocates.
#[derive(Debug, Clone)]
pub struct FunctionEscape {
    pub func: FuncId,
    pub objects: Vec<(ObjId, EscapeState)>,
}

impl FunctionEscape {
    /// Objects whose lifetime is bounded by the function's frame.
    pub fn frame_local(&self) -> impl Iterator<Item = ObjId> + '_ {
        self.objects
            .iter()
            .filter(|(_, s)| *s == EscapeState::NoEscape)
            .map(|(o, _)| *o)
    }
}

/// Thread-spawn points, as frontend facts.
#[derive(Debug, Clone, Default)]
pub struct EscapeConfig {
    /// Method / function symbols whose receiver and arguments run on another thread
    /// (thread start, executor submission, task creation).
    pub spawn_symbols: FxHashSet<u32>,
    /// Interned names of unresolved callees and intrinsics with the same meaning.
    pub spawn_names: FxHashSet<u32>,
}

/// Escape facts of every abstract object.
#[derive(Debug, Clone)]
pub struct EscapeResult {
    /// Indexed by `ObjId`.
    pub objects: Vec<ObjectEscape>,
    /// One entry per reachable function that allocates.
    pub functions: Vec<FunctionEscape>,
    /// A reachable spawn point exists.
    pub spawns_threads: bool,
    /// Derived from a truncated points-to solve: states may be too low.
    pub partial: bool,
    function_index: FxHashMap<FuncId, usize>,
}

impl EscapeResult {
    #[inline]
    pub fn state(&self, o: ObjId) -> EscapeState {
        self.objects
            .get(o.0 as usize)
            .map(|e| e.state)
            .unwrap_or(EscapeState::GlobalEscape)
    }

    #[inline]
    pub fn of(&self, o: ObjId) -> Option<&ObjectEscape> {
        self.objects.get(o.0 as usize)
    }

    pub fn function(&self, func: FuncId) -> Option<&FunctionEscape> {
        self.function_index.get(&func).map(|&i| &self.functions[i])
    }

    /// Stack objects reachable after their frame ends.
    pub fn dangling(&self) -> Vec<ObjId> {
        self.objects
            .iter()
            .enumerate()
            .filter(|(_, e)| e.outlives_frame)
            .map(|(i, _)| ObjId(i as u32))
            .collect()
    }

    /// The escape path of `o`: `o`, the object it is reachable through, and so on up to
    /// the root.
    pub fn path(&self, o: ObjId) -> Vec<ObjId> {
        let mut out = vec![o];
        let mut seen: FxHashSet<ObjId> = FxHashSet::default();
        seen.insert(o);
        let mut cur = o;
        while let Some(next) = self.of(cur).and_then(|e| e.via) {
            if !seen.insert(next) {
                break;
            }
            out.push(next);
            cur = next;
        }
        out
    }
}

struct Escape<'a> {
    module: &'a ModuleIr,
    pts: &'a PointsToResult,
    memory: &'a MemoryConfig,
    config: &'a EscapeConfig,
    succ: Vec<SparseBitSet>,
    objects: Vec<ObjectEscape>,
}

/// Run escape analysis over a points-to result.
pub fn analyze(
    module: &ModuleIr,
    pts: &PointsToResult,
    memory: &MemoryConfig,
    config: &EscapeConfig,
) -> EscapeResult {
    let n = pts.locations.object_count();
    let mut succ = vec![SparseBitSet::new(); n];
    for (i, s) in succ.iter_mut().enumerate() {
        for &c in pts.locations.cells_of(ObjId(i as u32)) {
            s.union_with(pts.cell_points_to(c));
        }
    }
    let mut e = Escape {
        module,
        pts,
        memory,
        config,
        succ,
        objects: vec![
            ObjectEscape {
                state: EscapeState::NoEscape,
                reasons: Vec::new(),
                via: None,
                outlives_frame: false,
            };
            n
        ],
    };
    e.run()
}

impl<'a> Escape<'a> {
    fn run(mut self) -> EscapeResult {
        let mut global_roots: Vec<(ObjId, EscapeReason)> = Vec::new();
        let mut external_roots: Vec<(ObjId, EscapeReason)> = Vec::new();
        let mut global_cell_roots: Vec<ObjId> = Vec::new();
        let mut thread_roots: Vec<(ObjId, EscapeReason)> = Vec::new();

        // Objects the program text does not allocate, and static storage.
        for (id, info) in self.pts.locations.objects() {
            match info.object.base {
                HeapBase::EntryParam { .. } | HeapBase::Unknown => {
                    external_roots.push((id, EscapeReason::ExternalOrigin))
                }
                HeapBase::StringLiteral => global_roots.push((id, EscapeReason::StaticStorage)),
                _ => {
                    if matches!(info.region, Some(Region::Static)) {
                        global_roots.push((id, EscapeReason::StaticStorage));
                    }
                }
            }
        }

        // Instruction-level roots, with spans.
        let mut spawns = false;
        let module = self.module;
        for &f in &self.pts.call_graph.reachable {
            let Some(func) = module.funcs.get(f.index()).filter(|x| x.id == f) else {
                continue;
            };
            self.scan_function(
                func,
                &mut global_roots,
                &mut external_roots,
                &mut thread_roots,
                &mut spawns,
            );
        }

        // Everything global and static cells and the external world hold.
        for (cid, cell) in self.pts.locations.cells() {
            if cell.is_global() {
                for o in self.pts.cell_points_to(cid).objects() {
                    global_cell_roots.push(o);
                    global_roots.push((o, EscapeReason::ReachableFromGlobal));
                }
            }
        }
        for o in self.pts.external_world().objects() {
            external_roots.push((o, EscapeReason::ReachableFromExternal));
        }

        // Thread escape first (highest state), then global escape.
        self.flood(&thread_roots, EscapeState::ThreadEscape, EscapeReason::ReachableFromThread);
        self.flood(
            &external_roots,
            EscapeState::GlobalEscape,
            EscapeReason::ReachableFromExternal,
        );
        self.flood(&global_roots, EscapeState::GlobalEscape, EscapeReason::ReachableFromGlobal);
        if spawns {
            let shared: Vec<(ObjId, EscapeReason)> = global_cell_roots
                .iter()
                .map(|&o| (o, EscapeReason::SharedThroughGlobal))
                .collect();
            self.flood(&shared, EscapeState::ThreadEscape, EscapeReason::SharedThroughGlobal);
        }

        // Argument escape per allocating function.
        let mut alloc_by_func: FxHashMap<FuncId, Vec<ObjId>> = FxHashMap::default();
        for (id, info) in self.pts.locations.objects() {
            if let Some(f) = info.func {
                if !info.object.base.is_opaque() {
                    alloc_by_func.entry(f).or_default().push(id);
                }
            }
        }
        let mut functions = Vec::new();
        let mut function_index = FxHashMap::default();
        let mut funcs: Vec<FuncId> = alloc_by_func.keys().copied().collect();
        funcs.sort_unstable();
        for f in funcs {
            let Some(func) = module.funcs.get(f.index()).filter(|x| x.id == f) else {
                continue;
            };
            let allocated = &alloc_by_func[&f];
            self.arg_escape(func, allocated);
            let objects: Vec<(ObjId, EscapeState)> = allocated
                .iter()
                .map(|&o| (o, self.objects[o.0 as usize].state))
                .collect();
            function_index.insert(f, functions.len());
            functions.push(FunctionEscape { func: f, objects });
        }

        // Stack objects that outlive their frame.
        for (id, info) in self.pts.locations.objects() {
            let e = &mut self.objects[id.0 as usize];
            if matches!(info.region, Some(Region::Stack)) && e.state >= EscapeState::ArgEscape {
                e.outlives_frame = true;
            }
        }

        EscapeResult {
            objects: self.objects,
            functions,
            spawns_threads: spawns,
            partial: !self.pts.is_complete(),
            function_index,
        }
    }

    fn raise(&mut self, o: ObjId, state: EscapeState, reason: EscapeReason, via: Option<ObjId>) -> bool {
        let e = &mut self.objects[o.0 as usize];
        if !e.reasons.contains(&reason) && (state >= e.state) {
            e.reasons.push(reason);
        }
        if state > e.state {
            e.state = state;
            e.via = via;
            return true;
        }
        false
    }

    /// Raise the roots to `state` and everything reachable from them, recording the path.
    fn flood(&mut self, roots: &[(ObjId, EscapeReason)], state: EscapeState, transitive: EscapeReason) {
        let mut queue: VecDeque<ObjId> = VecDeque::new();
        for &(o, reason) in roots {
            if self.raise(o, state, reason, None) || self.objects[o.0 as usize].state == state {
                queue.push_back(o);
            }
        }
        let mut seen: FxHashSet<ObjId> = FxHashSet::default();
        while let Some(o) = queue.pop_front() {
            if !seen.insert(o) {
                continue;
            }
            let next: Vec<ObjId> = self.succ[o.0 as usize].objects().collect();
            for p in next {
                if self.raise(p, state, transitive, Some(o)) || self.objects[p.0 as usize].state == state {
                    queue.push_back(p);
                }
            }
        }
    }

    fn is_spawn_call(&self, callee: &Callee, targets: &[FuncId], external: &[ExternalTarget]) -> bool {
        if self.config.spawn_symbols.is_empty() && self.config.spawn_names.is_empty() {
            return false;
        }
        if let Some(m) = callee.method_symbol() {
            if self.config.spawn_symbols.contains(&m) {
                return true;
            }
        }
        if let Callee::Unresolved(name) = callee {
            if self.config.spawn_names.contains(name) {
                return true;
            }
        }
        let module = self.module;
        targets.iter().any(|&f| {
            module
                .funcs
                .get(f.index())
                .and_then(|x| x.symbol)
                .map(|s| self.config.spawn_symbols.contains(&s))
                .unwrap_or(false)
        }) || external.iter().any(|t| match t {
            ExternalTarget::Symbol(s) => self.config.spawn_symbols.contains(s),
            ExternalTarget::Name(n) => self.config.spawn_names.contains(n),
            _ => false,
        })
    }

    fn scan_function(
        &self,
        func: &FuncIr,
        global_roots: &mut Vec<(ObjId, EscapeReason)>,
        external_roots: &mut Vec<(ObjId, EscapeReason)>,
        thread_roots: &mut Vec<(ObjId, EscapeReason)>,
        spawns: &mut bool,
    ) {
        let sound = self.memory.external == ExternalPolicy::Sound;
        for block in &func.blocks {
            for (i, inst) in block.insts.iter().enumerate() {
                let span = block.inst_spans.get(i).copied().unwrap_or(Span::NONE);
                match inst {
                    Inst::Store { place, value } if place.is_static_storage() => {
                        for o in self.pts.operand_points_to(func.id, value).objects() {
                            global_roots.push((o, EscapeReason::StoredInGlobal { func: func.id, span }));
                        }
                    }
                    Inst::Call { site, callee, args, .. } => {
                        let (targets, external): (Vec<FuncId>, Vec<ExternalTarget>) =
                            match self.pts.call_graph.site(*site) {
                                Some(r) => (r.targets.iter().map(|&(f, _)| f).collect(), r.external.clone()),
                                None => (Vec::new(), Vec::new()),
                            };
                        if self.is_spawn_call(callee, &targets, &external) {
                            *spawns = true;
                            for a in args {
                                for o in self.pts.operand_points_to(func.id, a).objects() {
                                    thread_roots.push((o, EscapeReason::PassedToThread { func: func.id, span }));
                                }
                            }
                        }
                        if sound && !external.is_empty() {
                            for a in args {
                                for o in self.pts.operand_points_to(func.id, a).objects() {
                                    external_roots.push((o, EscapeReason::PassedToExternal { func: func.id, span }));
                                }
                            }
                        }
                    }
                    Inst::Intrinsic { name, args, .. } => {
                        let effect = intrinsic_effect(self.module, self.memory, *name);
                        let spawn = self.config.spawn_names.contains(name);
                        if spawn {
                            *spawns = true;
                            for a in args {
                                for o in self.pts.operand_points_to(func.id, a).objects() {
                                    thread_roots.push((o, EscapeReason::PassedToThread { func: func.id, span }));
                                }
                            }
                        }
                        if sound && effect == IntrinsicEffect::Opaque {
                            for a in args {
                                if matches!(a, Operand::Value(_)) {
                                    for o in self.pts.operand_points_to(func.id, a).objects() {
                                        external_roots.push((o, EscapeReason::PassedToExternal { func: func.id, span }));
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    /// Spans of the returns and escaping throws of `func`, with their operands.
    fn exits(func: &FuncIr) -> (Vec<(Operand, Span)>, Vec<(Operand, Span)>) {
        let mut returns = Vec::new();
        let mut throws = Vec::new();
        for block in &func.blocks {
            match &block.term {
                Terminator::Return(Some(op)) => returns.push((*op, block.term_span)),
                Terminator::Throw(op) if block.unwind.is_none() => {
                    throws.push((*op, block.term_span))
                }
                _ => {}
            }
        }
        (returns, throws)
    }

    /// Objects `func` allocates that are reachable from its parameters, return value or
    /// escaping exceptions outlive its frame.
    fn arg_escape(&mut self, func: &FuncIr, allocated: &[ObjId]) {
        let candidates: Vec<ObjId> = allocated
            .iter()
            .copied()
            .filter(|&o| self.objects[o.0 as usize].state == EscapeState::NoEscape)
            .collect();
        if candidates.is_empty() {
            return;
        }
        let (returns, throws) = Self::exits(func);
        // Direct roots: returned and thrown objects.
        for (op, span) in &returns {
            for o in self.pts.operand_points_to(func.id, op).objects() {
                if candidates.contains(&o) {
                    self.raise(o, EscapeState::ArgEscape, EscapeReason::Returned { func: func.id, span: *span }, None);
                }
            }
        }
        for (op, span) in &throws {
            for o in self.pts.operand_points_to(func.id, op).objects() {
                if candidates.contains(&o) {
                    self.raise(o, EscapeState::ArgEscape, EscapeReason::Thrown { func: func.id, span: *span }, None);
                }
            }
        }
        // Thrown out through callees, and everything reachable from caller-visible values.
        let mut roots = SparseBitSet::new();
        for &p in &func.params {
            roots.union_with(self.pts.points_to(func.id, p));
        }
        roots.union_with(self.pts.return_points_to(func.id));
        roots.union_with(self.pts.thrown_points_to(func.id));
        let target: FxHashSet<ObjId> = candidates.iter().copied().collect();
        let mut queue: VecDeque<(ObjId, Option<ObjId>)> = roots.objects().map(|o| (o, None)).collect();
        let mut seen: FxHashSet<ObjId> = FxHashSet::default();
        while let Some((o, via)) = queue.pop_front() {
            if !seen.insert(o) {
                continue;
            }
            if target.contains(&o) && self.objects[o.0 as usize].state == EscapeState::NoEscape {
                let reason = if self.pts.thrown_points_to(func.id).contains(o.0) && via.is_none() {
                    EscapeReason::Thrown { func: func.id, span: func.span }
                } else if self.pts.return_points_to(func.id).contains(o.0) && via.is_none() {
                    EscapeReason::Returned { func: func.id, span: func.span }
                } else {
                    EscapeReason::ReachableFromParameter { func: func.id }
                };
                self.raise(o, EscapeState::ArgEscape, reason, via);
            }
            for p in self.succ[o.0 as usize].objects() {
                if !seen.contains(&p) {
                    queue.push_back((p, Some(o)));
                }
            }
        }
    }
}

/// Objects reachable from `roots` through object slots (reflexive).
pub fn reachable_objects(pts: &PointsToResult, roots: &SparseBitSet) -> SparseBitSet {
    let mut out = roots.clone();
    let mut stack: Vec<ObjId> = roots.objects().collect();
    while let Some(o) = stack.pop() {
        for &c in pts.locations.cells_of(o) {
            for p in pts.cell_points_to(c).objects() {
                if out.insert(p.0) {
                    stack.push(p);
                }
            }
        }
    }
    out
}
