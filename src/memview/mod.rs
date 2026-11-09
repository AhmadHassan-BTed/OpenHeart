//! Memory-execution views and memory-safety facts.
//!
//! Every view in this module is computed from the explicit memory operations of the shared
//! IR (`Alloc`, `Load`, `Store`, `Release`, `Call`, `MakeClosure`) by abstract
//! interpretation. Nothing is matched by name: language knowledge (what a call frees,
//! whether a fresh object is zeroed, which calls acquire a lock or start a thread) enters
//! only through [`MemoryFacts`], which the `src/memory`, `src/analysis`, `src/abi` and
//! `src/lang` layers implement. When a fact is neither in the IR nor supplied by an
//! oracle, the result says so through a [`Derivation`] tag instead of guessing.
//!
//! Abstract objects follow the allocation-site abstraction with recency
//! (Balakrishnan & Reps 2006): each site has a *recent* object (the last instance, strongly
//! updatable) and an *old* summary object (every earlier instance, weakly updated).

pub mod frames;
pub mod heap_shape;
pub mod lifetime_view;
pub mod ownership_view;
pub mod races;
pub mod safety;

use std::collections::{HashMap, HashSet, VecDeque};

use serde::{Deserialize, Serialize};

use crate::ir::{
    AllocSiteId, CallSiteId, Callee, ConstVal, FuncId, FuncIr, Inst, IrType, ModuleIr, Operand,
    Place, Span, Terminator, TypeId, ValueId,
};

/// Sentinel used for "no position" in raw-id fields.
pub const NO_ID: u32 = u32::MAX;

// ─── Provenance ─────────────────────────────────────────────────────────────────

/// Source provenance of a memory fact, copied from the IR [`Span`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SrcSpan {
    pub file_id: u32,
    pub first_token: u32,
    pub last_token: u32,
}

impl SrcSpan {
    pub const NONE: SrcSpan = SrcSpan {
        file_id: NO_ID,
        first_token: NO_ID,
        last_token: NO_ID,
    };

    pub fn from_ir(span: &Span) -> Self {
        Self {
            file_id: span.file_id,
            first_token: span.first_token,
            last_token: span.last_token,
        }
    }

    pub fn is_none(&self) -> bool {
        self.file_id == NO_ID
    }
}

/// How a memory fact relates to the program's semantics.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "tier")]
pub enum Derivation {
    /// Holds on every execution the abstraction represents.
    Exact,
    /// A sound over-approximation: may include facts no execution exhibits.
    OverApproximate,
    /// Derived from incomplete inputs; `reason` names what is missing.
    Partial { reason: String },
    /// Cannot be derived from the IR or the supplied facts.
    NotDerivable { reason: String },
}

impl Derivation {
    pub fn partial(reason: impl Into<String>) -> Self {
        Self::Partial {
            reason: reason.into(),
        }
    }

    pub fn not_derivable(reason: impl Into<String>) -> Self {
        Self::NotDerivable {
            reason: reason.into(),
        }
    }

    fn rank(&self) -> u8 {
        match self {
            Self::Exact => 0,
            Self::OverApproximate => 1,
            Self::Partial { .. } => 2,
            Self::NotDerivable { .. } => 3,
        }
    }

    /// The weaker of two tiers (`Exact` < `OverApproximate` < `Partial` < `NotDerivable`).
    /// Two partial tiers keep both reasons.
    pub fn weakest(self, other: Derivation) -> Derivation {
        match (self, other) {
            (Self::Partial { reason: a }, Self::Partial { reason: b }) if a != b => {
                Self::Partial {
                    reason: format!("{a}; {b}"),
                }
            }
            (a, b) => {
                if a.rank() >= b.rank() {
                    a
                } else {
                    b
                }
            }
        }
    }

    pub fn is_definite(&self) -> bool {
        matches!(self, Self::Exact)
    }
}

/// An instruction position: `index == insts.len()` addresses the block terminator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ProgramPoint {
    pub func: u32,
    pub block: u32,
    pub index: u32,
}

// ─── Abstract memory vocabulary ─────────────────────────────────────────────────

/// An abstract memory object (or a pseudo-object carried in points-to sets).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "obj")]
pub enum AbsObj {
    /// Instances of an allocation site; `old` selects the recency summary object.
    Site { site: u32, old: bool },
    /// Objects allocated by a call without an IR body whose effects say it returns fresh
    /// storage; the call site is the allocation site.
    CallAlloc { call_site: u32, old: bool },
    /// Memory the function did not allocate and cannot see: the pointee of a parameter, the
    /// result of an unmodeled call or a load from unmodeled memory, named by the SSA value
    /// that first produced it.
    Ext { func: u32, value: u32 },
    /// The environment object of a closure built by `MakeClosure`.
    Closure { func: u32, value: u32 },
    /// The storage cell of a global.
    Global(u32),
    /// The storage cell of a static field.
    Static { owner: u32, field: u32 },
    /// The null reference.
    Null,
    /// An uninitialized value (an IR `Undef`, or uninitialized memory contents).
    Undef,
    /// Contents that the analysis does not model at this point.
    Unknown,
}

impl AbsObj {
    /// True for real storage (not `Null`, `Undef` or `Unknown`).
    pub fn is_storage(&self) -> bool {
        !matches!(self, Self::Null | Self::Undef | Self::Unknown)
    }

    pub fn site(&self) -> Option<u32> {
        match self {
            Self::Site { site, .. } => Some(*site),
            _ => None,
        }
    }

    /// True for objects created by an allocation the analysis observed.
    pub fn is_allocation(&self) -> bool {
        matches!(self, Self::Site { .. } | Self::CallAlloc { .. })
    }

    /// True for recency summary objects (every instance but the most recent).
    pub fn is_old(&self) -> bool {
        matches!(
            self,
            Self::Site { old: true, .. } | Self::CallAlloc { old: true, .. }
        )
    }

    /// The recency summary object of an allocation object (identity otherwise).
    pub fn as_old(&self) -> AbsObj {
        match *self {
            Self::Site { site, .. } => Self::Site { site, old: true },
            Self::CallAlloc { call_site, .. } => Self::CallAlloc {
                call_site,
                old: true,
            },
            other => other,
        }
    }

    /// The most-recent object of an allocation object (identity otherwise).
    pub fn as_recent(&self) -> AbsObj {
        match *self {
            Self::Site { site, .. } => Self::Site { site, old: false },
            Self::CallAlloc { call_site, .. } => Self::CallAlloc {
                call_site,
                old: false,
            },
            other => other,
        }
    }

    /// A stable textual key, used by every view as node identity.
    pub fn key(&self) -> String {
        match self {
            Self::Site { site, old: false } => format!("site{site}"),
            Self::Site { site, old: true } => format!("site{site}.old"),
            Self::CallAlloc {
                call_site,
                old: false,
            } => format!("call{call_site}"),
            Self::CallAlloc {
                call_site,
                old: true,
            } => format!("call{call_site}.old"),
            Self::Ext { func, value } => format!("ext.f{func}.v{value}"),
            Self::Closure { func, value } => format!("closure.f{func}.v{value}"),
            Self::Global(g) => format!("global{g}"),
            Self::Static { owner, field } => format!("static.s{owner}.f{field}"),
            Self::Null => "null".to_string(),
            Self::Undef => "undef".to_string(),
            Self::Unknown => "unknown".to_string(),
        }
    }
}

/// A slot inside an abstract object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "slot")]
pub enum FieldKey {
    /// The object's whole storage (a scalar slot, a global cell, `*p`).
    Whole,
    /// A declared field.
    Field(u32),
    /// Every element of an array (indices are summarized).
    Elem,
    /// The `i`-th captured value of a closure environment.
    Capture(u32),
}

/// Nullness of a reference value at a program point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Nullness {
    Null,
    NonNull,
    MaybeNull,
}

/// Contents of a freshly allocated object before any store.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InitKind {
    /// Every slot holds zero / null / false.
    Zeroed,
    /// Slots hold indeterminate values.
    Uninitialized,
}

/// What an IR `Release` does to the released objects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseKind {
    /// The storage is returned to the allocator (free, delete, owning drop).
    Deallocate,
    /// A dispose / close protocol ends the object's usable life; storage stays reachable.
    Dispose,
    /// A lock or monitor is released; the object stays live.
    Unlock,
    /// Stack storage whose scope ends.
    EndOfScope,
}

/// The memory effect of a call or intrinsic on its arguments, from the language layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "effect")]
pub enum CallEffect {
    /// The objects of argument `arg` are deallocated.
    Free { arg: usize },
    /// The objects of argument `arg` are disposed.
    Dispose { arg: usize },
    /// The lock designated by argument `arg` is acquired.
    Acquire { arg: usize },
    /// The lock designated by argument `arg` is released.
    Unlock { arg: usize },
    /// A thread starts executing the function or closure passed as argument `arg`.
    Spawn { arg: usize },
    /// The thread designated by argument `arg` is joined.
    Join { arg: usize },
    /// The objects of argument `arg` become reachable from memory the caller cannot see.
    Escapes { arg: usize },
    /// The objects of argument `arg` are read.
    Reads { arg: usize },
    /// The objects of argument `arg` are written.
    Writes { arg: usize },
    /// The call returns a fresh heap object.
    AllocatesResult { init: Option<InitKind> },
    /// The call returns its argument `arg` (identity, fluent setters).
    ReturnsArg { arg: usize },
    /// The listed effects are complete: no other effect on memory reachable from arguments.
    Complete,
}

/// Facts that the IR alone does not carry. Every method answers `None` for "not supplied";
/// the views then fall back to what the IR proves and weaken their [`Derivation`].
///
/// Implemented by `memory::points_to::PointsToResult` (callees, thread escape),
/// `analysis::interval` / `analysis::nullness` (value facts), `abi` layouts (array
/// lengths) and the `lang` frontends (call, release and allocation semantics).
pub trait MemoryFacts {
    /// Possible targets of a call site that is not `Callee::Static`.
    fn callees(&self, _site: CallSiteId) -> Option<Vec<FuncId>> {
        None
    }
    /// Memory effects of a call site whose callee has no IR body (external, unresolved).
    fn call_effects(&self, _func: FuncId, _site: CallSiteId) -> Option<Vec<CallEffect>> {
        None
    }
    /// Memory effects of an intrinsic, by interned intrinsic name.
    fn intrinsic_effects(&self, _func: FuncId, _name: u32) -> Option<Vec<CallEffect>> {
        None
    }
    /// What the `Release` at `point` does.
    fn release_kind(&self, _point: ProgramPoint) -> Option<ReleaseKind> {
        None
    }
    /// Initial contents of objects of an allocation site.
    fn init_kind(&self, _site: AllocSiteId) -> Option<InitKind> {
        None
    }
    /// Whether objects of an allocation site must be released explicitly.
    fn requires_release(&self, _site: AllocSiteId) -> Option<bool> {
        None
    }
    /// Element count range of array objects of an allocation site (from layouts).
    fn array_length(&self, _site: AllocSiteId) -> Option<(i128, i128)> {
        None
    }
    /// Integer range of `value` just before `point`.
    fn interval(&self, _point: ProgramPoint, _value: ValueId) -> Option<(i128, i128)> {
        None
    }
    /// Nullness of `value` just before `point` (branch-refined).
    fn nullness(&self, _point: ProgramPoint, _value: ValueId) -> Option<Nullness> {
        None
    }
    /// Functions that start executing on the program's initial thread.
    fn entry_points(&self) -> Option<Vec<FuncId>> {
        None
    }
}

/// No external facts: every view is derived from the IR alone.
#[derive(Debug, Clone, Copy, Default)]
pub struct IrOnly;

impl MemoryFacts for IrOnly {}

// ─── Operand access ─────────────────────────────────────────────────────────────

/// Uniform access to instruction operands, whether a field holds an [`Operand`] or a
/// bare [`ValueId`].
pub(crate) trait OperandRef {
    fn value(&self) -> Option<ValueId>;
    fn constant(&self) -> Option<&ConstVal>;
    fn is_undef(&self) -> bool;
}

impl OperandRef for Operand {
    fn value(&self) -> Option<ValueId> {
        match self {
            Operand::Value(v) => Some(*v),
            _ => None,
        }
    }
    fn constant(&self) -> Option<&ConstVal> {
        match self {
            Operand::Const(c) => Some(c),
            _ => None,
        }
    }
    fn is_undef(&self) -> bool {
        matches!(self, Operand::Undef)
    }
}

impl OperandRef for ValueId {
    fn value(&self) -> Option<ValueId> {
        Some(*self)
    }
    fn constant(&self) -> Option<&ConstVal> {
        None
    }
    fn is_undef(&self) -> bool {
        false
    }
}

/// The values an instruction reads, in operand order (place bases included).
pub(crate) fn inst_uses(inst: &Inst) -> Vec<ValueId> {
    fn push(out: &mut Vec<ValueId>, o: &dyn OperandRef) {
        if let Some(v) = o.value() {
            out.push(v);
        }
    }
    let mut out = Vec::new();
    match inst {
        Inst::Assign { src, .. } => push(&mut out, src),
        Inst::Unary { src, .. } => push(&mut out, src),
        Inst::Binary { lhs, rhs, .. } | Inst::Compare { lhs, rhs, .. } => {
            push(&mut out, lhs);
            push(&mut out, rhs);
        }
        Inst::Load { place, .. } | Inst::Release { place } => place_uses(place, &mut out),
        Inst::Store { place, value } => {
            place_uses(place, &mut out);
            if let Some(v) = value.value() {
                out.push(v);
            }
        }
        Inst::Alloc { .. } => {}
        Inst::Call { callee, args, .. } => {
            match callee {
                Callee::Virtual { receiver, .. } | Callee::Interface { receiver, .. } => {
                    out.push(*receiver)
                }
                Callee::Indirect(v) => out.push(*v),
                _ => {}
            }
            for a in args {
                if let Some(v) = a.value() {
                    out.push(v);
                }
            }
        }
        Inst::Phi { incoming, .. } => {
            for (_, op) in incoming {
                if let Some(v) = op.value() {
                    out.push(v);
                }
            }
        }
        Inst::Cast { src, .. } | Inst::TypeTest { src, .. } => push(&mut out, src),
        Inst::MakeClosure { captures, .. } => {
            for c in captures {
                if let Some(v) = c.value() {
                    out.push(v);
                }
            }
        }
        Inst::Intrinsic { args, .. } => {
            for a in args {
                if let Some(v) = a.value() {
                    out.push(v);
                }
            }
        }
    }
    out
}

pub(crate) fn place_uses(place: &Place, out: &mut Vec<ValueId>) {
    match place {
        Place::Local(v) | Place::Deref(v) => out.push(*v),
        Place::Field { base, .. } => out.push(*base),
        Place::Index { base, index } => {
            out.push(*base);
            if let Some(v) = index.value() {
                out.push(v);
            }
        }
        Place::Global(_) | Place::Static { .. } => {}
    }
}

/// The value an instruction defines, if any.
pub(crate) fn inst_def(inst: &Inst) -> Option<ValueId> {
    match inst {
        Inst::Assign { dst, .. }
        | Inst::Unary { dst, .. }
        | Inst::Binary { dst, .. }
        | Inst::Compare { dst, .. }
        | Inst::Load { dst, .. }
        | Inst::Alloc { dst, .. }
        | Inst::Phi { dst, .. }
        | Inst::Cast { dst, .. }
        | Inst::MakeClosure { dst, .. }
        | Inst::TypeTest { dst, .. } => Some(*dst),
        Inst::Call { dst, .. } | Inst::Intrinsic { dst, .. } => *dst,
        Inst::Store { .. } | Inst::Release { .. } => None,
    }
}

// ─── Module index ───────────────────────────────────────────────────────────────

/// Per-function control-flow facts used by every analysis in the module.
#[derive(Debug, Clone, Default)]
pub struct FuncIndex {
    /// Function id.
    pub id: u32,
    /// Position of the function in `ModuleIr::funcs`.
    pub pos: usize,
    /// Block id -> position in `FuncIr::blocks`.
    pub block_pos: HashMap<u32, usize>,
    /// Successor block positions, normal and exceptional edges together.
    pub succs: Vec<Vec<usize>>,
    pub preds: Vec<Vec<usize>>,
    /// Reverse post-order of the blocks reachable from the entry.
    pub rpo: Vec<usize>,
    /// Position of each block in `rpo` (`usize::MAX` when unreachable).
    pub rpo_num: Vec<usize>,
    /// Immediate dominator of each block position (`usize::MAX` when unreachable).
    pub idom: Vec<usize>,
    /// True when the block lies on a CFG cycle (its instructions may run repeatedly).
    pub in_cycle: Vec<bool>,
    /// Edges `(from, to)` whose target dominates their source.
    pub back_edges: HashSet<(usize, usize)>,
    /// SSA value -> (block position, instruction index) of its definition.
    pub def_site: HashMap<u32, (usize, usize)>,
    /// Parameter value -> parameter index.
    pub param_index: HashMap<u32, usize>,
    /// Position of the entry block.
    pub entry: usize,
}

impl FuncIndex {
    fn build(pos: usize, func: &FuncIr) -> Self {
        let n = func.blocks.len();
        let mut block_pos = HashMap::with_capacity(n);
        for (i, b) in func.blocks.iter().enumerate() {
            block_pos.insert(b.id.0, i);
        }
        let mut succs = vec![Vec::new(); n];
        let mut def_site = HashMap::new();
        for (bi, b) in func.blocks.iter().enumerate() {
            let mut out: Vec<usize> = Vec::new();
            let mut add = |id: u32, out: &mut Vec<usize>| {
                if let Some(&p) = block_pos.get(&id) {
                    if !out.contains(&p) {
                        out.push(p);
                    }
                }
            };
            for (ii, inst) in b.insts.iter().enumerate() {
                if let Inst::Call {
                    unwind: Some(u), ..
                } = inst
                {
                    add(u.0, &mut out);
                }
                if let Some(d) = inst_def(inst) {
                    def_site.insert(d.0, (bi, ii));
                }
            }
            match &b.term {
                Terminator::Jump(t) => add(t.0, &mut out),
                Terminator::Branch { then_b, else_b, .. } => {
                    add(then_b.0, &mut out);
                    add(else_b.0, &mut out);
                }
                Terminator::Switch { cases, default, .. } => {
                    for (_, t) in cases {
                        add(t.0, &mut out);
                    }
                    add(default.0, &mut out);
                }
                Terminator::Return(_) | Terminator::Throw(_) | Terminator::Unreachable => {}
            }
            succs[bi] = out;
        }
        let mut preds = vec![Vec::new(); n];
        for (b, ss) in succs.iter().enumerate() {
            for &s in ss {
                preds[s].push(b);
            }
        }
        let entry = block_pos.get(&func.entry.0).copied().unwrap_or(0);

        // Iterative DFS post-order from the entry.
        let mut rpo = Vec::with_capacity(n);
        if n > 0 {
            let mut seen = vec![false; n];
            let mut stack: Vec<(usize, usize)> = vec![(entry, 0)];
            seen[entry] = true;
            while let Some(&mut (b, ref mut next)) = stack.last_mut() {
                if *next < succs[b].len() {
                    let s = succs[b][*next];
                    *next += 1;
                    if !seen[s] {
                        seen[s] = true;
                        stack.push((s, 0));
                    }
                } else {
                    rpo.push(b);
                    stack.pop();
                }
            }
            rpo.reverse();
        }
        let mut rpo_num = vec![usize::MAX; n];
        for (i, &b) in rpo.iter().enumerate() {
            rpo_num[b] = i;
        }

        // Cooper-Harvey-Kennedy dominators over block positions.
        let mut idom = vec![usize::MAX; n];
        if !rpo.is_empty() {
            idom[entry] = entry;
            let mut changed = true;
            while changed {
                changed = false;
                for &b in rpo.iter().skip(1) {
                    let mut new_idom = usize::MAX;
                    for &p in &preds[b] {
                        if idom[p] == usize::MAX {
                            continue;
                        }
                        new_idom = if new_idom == usize::MAX {
                            p
                        } else {
                            let (mut x, mut y) = (p, new_idom);
                            while x != y {
                                while rpo_num[x] > rpo_num[y] {
                                    x = idom[x];
                                }
                                while rpo_num[y] > rpo_num[x] {
                                    y = idom[y];
                                }
                            }
                            x
                        };
                    }
                    if new_idom != usize::MAX && idom[b] != new_idom {
                        idom[b] = new_idom;
                        changed = true;
                    }
                }
            }
        }

        // A block is on a cycle when it reaches itself; computed per strongly connected
        // component (Tarjan, iterative).
        let in_cycle = blocks_on_cycles(&succs);

        let mut fi = FuncIndex {
            id: func.id.0,
            pos,
            block_pos,
            succs,
            preds,
            rpo,
            rpo_num,
            idom,
            in_cycle,
            back_edges: HashSet::new(),
            def_site,
            param_index: func
                .params
                .iter()
                .enumerate()
                .map(|(i, p)| (p.0, i))
                .collect(),
            entry,
        };
        let mut back = HashSet::new();
        for (b, ss) in fi.succs.iter().enumerate() {
            for &s in ss {
                if fi.dominates(s, b) {
                    back.insert((b, s));
                }
            }
        }
        fi.back_edges = back;
        fi
    }

    /// Whether block position `a` dominates block position `b`.
    pub fn dominates(&self, a: usize, b: usize) -> bool {
        if self.idom.get(b).copied().unwrap_or(usize::MAX) == usize::MAX {
            return false;
        }
        let mut cur = b;
        loop {
            if cur == a {
                return true;
            }
            let up = self.idom[cur];
            if up == cur || up == usize::MAX {
                return false;
            }
            cur = up;
        }
    }

    /// Whether the block position is reachable from the entry.
    pub fn reachable(&self, b: usize) -> bool {
        self.rpo_num.get(b).copied().unwrap_or(usize::MAX) != usize::MAX
    }

    /// Shortest block path `from ..= to` along CFG edges (breadth-first), if one exists.
    pub fn block_path(&self, from: usize, to: usize) -> Option<Vec<usize>> {
        let n = self.succs.len();
        if from >= n || to >= n {
            return None;
        }
        if from == to {
            return Some(vec![from]);
        }
        let mut parent = vec![usize::MAX; n];
        let mut queue = VecDeque::new();
        parent[from] = from;
        queue.push_back(from);
        while let Some(b) = queue.pop_front() {
            for &s in &self.succs[b] {
                if parent[s] == usize::MAX {
                    parent[s] = b;
                    if s == to {
                        let mut path = vec![to];
                        let mut cur = to;
                        while cur != from {
                            cur = parent[cur];
                            path.push(cur);
                        }
                        path.reverse();
                        return Some(path);
                    }
                    queue.push_back(s);
                }
            }
        }
        None
    }

    /// Shortest path that leaves `from` and comes back to it (for points that repeat).
    pub fn cycle_through(&self, from: usize) -> Option<Vec<usize>> {
        self.succs.get(from)?.iter().find_map(|&s| {
            self.block_path(s, from).map(|mut p| {
                p.insert(0, from);
                p
            })
        })
    }
}

/// Marks every node that lies on a cycle of the graph (iterative Tarjan SCC).
pub(crate) fn blocks_on_cycles(succs: &[Vec<usize>]) -> Vec<bool> {
    let n = succs.len();
    let mut index = vec![usize::MAX; n];
    let mut low = vec![0usize; n];
    let mut on_stack = vec![false; n];
    let mut stack: Vec<usize> = Vec::new();
    let mut result = vec![false; n];
    let mut counter = 0usize;
    for root in 0..n {
        if index[root] != usize::MAX {
            continue;
        }
        let mut call: Vec<(usize, usize)> = vec![(root, 0)];
        index[root] = counter;
        low[root] = counter;
        counter += 1;
        stack.push(root);
        on_stack[root] = true;
        while let Some(&mut (v, ref mut next)) = call.last_mut() {
            if *next < succs[v].len() {
                let w = succs[v][*next];
                *next += 1;
                if index[w] == usize::MAX {
                    index[w] = counter;
                    low[w] = counter;
                    counter += 1;
                    stack.push(w);
                    on_stack[w] = true;
                    call.push((w, 0));
                } else if on_stack[w] {
                    low[v] = low[v].min(index[w]);
                }
            } else {
                call.pop();
                if let Some(&(parent, _)) = call.last() {
                    low[parent] = low[parent].min(low[v]);
                }
                if low[v] == index[v] {
                    let mut members = Vec::new();
                    while let Some(w) = stack.pop() {
                        on_stack[w] = false;
                        members.push(w);
                        if w == v {
                            break;
                        }
                    }
                    let cyclic = members.len() > 1 || succs[v].contains(&v);
                    if cyclic {
                        for m in members {
                            result[m] = true;
                        }
                    }
                }
            }
        }
    }
    result
}

/// Resolved call edges plus the module-wide lookup tables.
#[derive(Debug, Clone, Default)]
pub struct ModuleIndex {
    pub funcs: Vec<FuncIndex>,
    /// Function id -> position in `ModuleIr::funcs`.
    pub func_pos: HashMap<u32, usize>,
    /// Allocation site id -> position in `ModuleIr::alloc_sites`.
    pub site_pos: HashMap<u32, usize>,
    /// Call site id -> position in `ModuleIr::call_sites`.
    pub call_site_pos: HashMap<u32, usize>,
    /// Field id -> position in `ModuleIr::fields`.
    pub field_pos: HashMap<u32, usize>,
    /// Call site id -> callee function ids with IR bodies.
    pub callees: HashMap<u32, Vec<u32>>,
    /// Call sites whose targets could not be resolved to IR bodies.
    pub unresolved_calls: HashSet<u32>,
    /// Function id -> call sites that may invoke it.
    pub callers: HashMap<u32, Vec<u32>>,
    /// Functions inside a recursive call-graph component.
    pub recursive: HashSet<u32>,
    /// Function ids, callees before callers (reverse topological over SCCs).
    pub bottom_up: Vec<u32>,
    /// Allocation site id -> (function position, block position, instruction index).
    pub site_inst: HashMap<u32, (usize, usize, usize)>,
    /// Call site id -> (function position, block position, instruction index).
    pub call_inst: HashMap<u32, (usize, usize, usize)>,
}

impl ModuleIndex {
    pub fn build(module: &ModuleIr, facts: &dyn MemoryFacts) -> Self {
        let mut idx = ModuleIndex::default();
        for (i, f) in module.funcs.iter().enumerate() {
            idx.func_pos.insert(f.id.0, i);
            idx.funcs.push(FuncIndex::build(i, f));
        }
        for (i, s) in module.alloc_sites.iter().enumerate() {
            idx.site_pos.insert(s.id.0, i);
        }
        for (i, c) in module.call_sites.iter().enumerate() {
            idx.call_site_pos.insert(c.id.0, i);
        }
        for (i, fl) in module.fields.iter().enumerate() {
            idx.field_pos.insert(fl.id.0, i);
        }

        // Closure values per function: an indirect call through a value defined by
        // `MakeClosure` (directly or through copies) targets that closure's body.
        for (fp, func) in module.funcs.iter().enumerate() {
            let closures = closure_targets(func);
            for (bp, block) in func.blocks.iter().enumerate() {
                for (ii, inst) in block.insts.iter().enumerate() {
                    match inst {
                        Inst::Alloc { site, .. } => {
                            idx.site_inst.insert(site.0, (fp, bp, ii));
                        }
                        Inst::Call { site, callee, .. } => {
                            idx.call_inst.insert(site.0, (fp, bp, ii));
                            let mut targets: Vec<u32> = Vec::new();
                            match callee {
                                Callee::Static(f) => targets.push(f.0),
                                Callee::Indirect(v) => {
                                    if let Some(ts) = closures.get(&v.0) {
                                        targets.extend(ts.iter().copied());
                                    }
                                }
                                _ => {}
                            }
                            if let Some(extra) = facts.callees(*site) {
                                for f in extra {
                                    if !targets.contains(&f.0) {
                                        targets.push(f.0);
                                    }
                                }
                            }
                            targets.retain(|t| idx.func_pos.contains_key(t));
                            if targets.is_empty() {
                                idx.unresolved_calls.insert(site.0);
                            }
                            for &t in &targets {
                                idx.callers.entry(t).or_default().push(site.0);
                            }
                            idx.callees.insert(site.0, targets);
                        }
                        _ => {}
                    }
                }
            }
        }

        // Call-graph SCCs: recursion flags and a callee-first order.
        let n = module.funcs.len();
        let mut succs = vec![Vec::new(); n];
        for (site, targets) in &idx.callees {
            if let Some(&(fp, _, _)) = idx.call_inst.get(site) {
                for t in targets {
                    if let Some(&tp) = idx.func_pos.get(t) {
                        if !succs[fp].contains(&tp) {
                            succs[fp].push(tp);
                        }
                    }
                }
            }
        }
        for s in &mut succs {
            s.sort_unstable();
        }
        let cyclic = blocks_on_cycles(&succs);
        for (p, &c) in cyclic.iter().enumerate() {
            if c {
                idx.recursive.insert(module.funcs[p].id.0);
            }
        }
        let mut seen = vec![false; n];
        for root in 0..n {
            if seen[root] {
                continue;
            }
            let mut stack: Vec<(usize, usize)> = vec![(root, 0)];
            seen[root] = true;
            while let Some(&mut (v, ref mut next)) = stack.last_mut() {
                if *next < succs[v].len() {
                    let w = succs[v][*next];
                    *next += 1;
                    if !seen[w] {
                        seen[w] = true;
                        stack.push((w, 0));
                    }
                } else {
                    idx.bottom_up.push(module.funcs[v].id.0);
                    stack.pop();
                }
            }
        }
        idx
    }

    pub fn func<'m>(&self, module: &'m ModuleIr, id: u32) -> Option<(&'m FuncIr, &FuncIndex)> {
        let p = *self.func_pos.get(&id)?;
        Some((&module.funcs[p], &self.funcs[p]))
    }

    /// Source span of an instruction or terminator.
    pub fn span_at(&self, module: &ModuleIr, point: ProgramPoint) -> SrcSpan {
        let Some((func, fi)) = self.func(module, point.func) else {
            return SrcSpan::NONE;
        };
        let Some(&bp) = fi.block_pos.get(&point.block) else {
            return SrcSpan::from_ir(&func.span);
        };
        let block = &func.blocks[bp];
        let i = point.index as usize;
        if i < block.insts.len() {
            block
                .inst_spans
                .get(i)
                .map(SrcSpan::from_ir)
                .unwrap_or_else(|| SrcSpan::from_ir(&block.term_span))
        } else {
            SrcSpan::from_ir(&block.term_span)
        }
    }

    /// Span of an allocation site.
    pub fn site_span(&self, module: &ModuleIr, site: u32) -> SrcSpan {
        self.site_pos
            .get(&site)
            .map(|&p| SrcSpan::from_ir(&module.alloc_sites[p].span))
            .unwrap_or(SrcSpan::NONE)
    }

    /// Declared type of an allocation site.
    pub fn site_type(&self, module: &ModuleIr, site: u32) -> Option<TypeId> {
        self.site_pos.get(&site).map(|&p| module.alloc_sites[p].ty)
    }

    /// Function that owns an allocation site.
    pub fn site_func(&self, module: &ModuleIr, site: u32) -> Option<u32> {
        self.site_pos
            .get(&site)
            .map(|&p| module.alloc_sites[p].func.0)
    }

    /// Static element count of an array type, when the type fixes it.
    pub fn fixed_array_len(module: &ModuleIr, ty: TypeId) -> Option<u64> {
        let mut cur = ty;
        // Peel references and generic wrappers down to the storage type (bounded: type
        // tables are acyclic for these constructors).
        for _ in 0..16 {
            match module.types.get(cur)? {
                IrType::Array { len, .. } => return *len,
                IrType::Ref { target, .. } => cur = *target,
                _ => return None,
            }
        }
        None
    }

    /// Program point of an instruction position.
    pub fn point(&self, module: &ModuleIr, fp: usize, bp: usize, ii: usize) -> ProgramPoint {
        let func = &module.funcs[fp];
        ProgramPoint {
            func: func.id.0,
            block: func.blocks[bp].id.0,
            index: ii as u32,
        }
    }

    /// Expands a block path into the program points of its terminators, ending at `end`.
    pub fn path_points(
        &self,
        module: &ModuleIr,
        fp: usize,
        blocks: &[usize],
        start: ProgramPoint,
        end: ProgramPoint,
    ) -> Vec<ProgramPoint> {
        let func = &module.funcs[fp];
        let mut out = vec![start];
        for (i, &b) in blocks.iter().enumerate() {
            let block = &func.blocks[b];
            let p = ProgramPoint {
                func: func.id.0,
                block: block.id.0,
                index: block.insts.len() as u32,
            };
            let is_last = i + 1 == blocks.len();
            if is_last {
                break;
            }
            if p != start {
                out.push(p);
            }
        }
        if out.last() != Some(&end) {
            out.push(end);
        }
        out
    }
}

/// Maps each SSA value that holds a closure (through copies, casts and phis) to the
/// closure bodies it may denote.
fn closure_targets(func: &FuncIr) -> HashMap<u32, Vec<u32>> {
    let mut map: HashMap<u32, Vec<u32>> = HashMap::new();
    let mut changed = true;
    while changed {
        changed = false;
        for block in &func.blocks {
            for inst in &block.insts {
                let (dst, srcs): (u32, Vec<u32>) = match inst {
                    Inst::MakeClosure { dst, func: f, .. } => {
                        let e = map.entry(dst.0).or_default();
                        if !e.contains(&f.0) {
                            e.push(f.0);
                            changed = true;
                        }
                        continue;
                    }
                    Inst::Assign { dst, src } => {
                        (dst.0, src.value().map(|v| v.0).into_iter().collect())
                    }
                    Inst::Cast { dst, src, .. } => {
                        (dst.0, src.value().map(|v| v.0).into_iter().collect())
                    }
                    Inst::Phi { dst, incoming } => (
                        dst.0,
                        incoming.iter().filter_map(|(_, o)| o.value().map(|v| v.0)).collect(),
                    ),
                    _ => continue,
                };
                let mut add: Vec<u32> = Vec::new();
                for s in srcs {
                    if let Some(ts) = map.get(&s) {
                        add.extend(ts.iter().copied());
                    }
                }
                if add.is_empty() {
                    continue;
                }
                let e = map.entry(dst).or_default();
                for t in add {
                    if !e.contains(&t) {
                        e.push(t);
                        changed = true;
                    }
                }
            }
        }
    }
    map
}

/// The raw id of a function or `NO_ID`.
pub(crate) fn func_raw(f: Option<FuncId>) -> u32 {
    f.map(|f| f.0).unwrap_or(NO_ID)
}
