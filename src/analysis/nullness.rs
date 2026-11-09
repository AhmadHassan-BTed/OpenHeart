//! Nullness of references (forward, flow- and path-sensitive within a function).
//!
//! Each SSA reference carries a set of [`NullFacts`]:
//!
//! - `NULL` — a null value reaches it on some path (a `null` constant, a copy of one, a
//!   source declared to return null);
//! - `NON_NULL` — a non-null reference reaches it on some path (an allocation, a closure, a
//!   string literal, a source declared non-null);
//! - `UNDECLARED` — it comes from a source whose nullability is not known (a parameter, a
//!   field load or a call result the [`NullabilityFacts`] cannot classify).
//!
//! Facts change along the flow: on the edges of `x == null` / `x != null` the tested value
//! is refined to `NULL` or `NON_NULL` (and an edge whose refinement is empty is infeasible);
//! after a field access, element access, pointer dereference or virtual call through `x`, the
//! normal continuation knows `x` is `NON_NULL`, and a dereference of a value that can only
//! be null never completes normally.
//!
//! Every dereference gets a [`DerefVerdict`]; branches whose null test the facts decide are
//! reported as decided.

use std::fmt;

use crate::analysis::lattice::{Lattice, MapLattice, Product};
use crate::analysis::solver::{
    def_points, defining_inst, phi_operands_from, replay_forward, resolve_copies, solve_forward,
    CfgEdge, CmpOp, ConstKey, DataflowResult, DefPoint, EdgeKind, ForwardTransfer, InstPoint,
    IrCfg, OperandView, ResolvedOperand, SolverOptions, UnaryOp,
};
use crate::ir::{Callee, CallSiteId, FuncIr, Inst, Place, Terminator, ValueId};

// ── Facts ────────────────────────────────────────────────────────────────────

/// A set of nullness facts (a three-element powerset).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct NullFacts(u8);

impl NullFacts {
    pub const NONE: NullFacts = NullFacts(0);
    pub const NULL: NullFacts = NullFacts(1);
    pub const NON_NULL: NullFacts = NullFacts(2);
    pub const UNDECLARED: NullFacts = NullFacts(4);

    #[inline]
    pub fn contains(self, other: NullFacts) -> bool {
        self.0 & other.0 == other.0
    }

    #[inline]
    pub fn intersects(self, other: NullFacts) -> bool {
        self.0 & other.0 != 0
    }

    #[inline]
    pub fn union(self, other: NullFacts) -> NullFacts {
        NullFacts(self.0 | other.0)
    }

    /// The facts when the value is known to be null: empty if it can never be.
    #[inline]
    pub fn assume_null(self) -> NullFacts {
        if self.intersects(NullFacts(NullFacts::NULL.0 | NullFacts::UNDECLARED.0)) {
            NullFacts::NULL
        } else {
            NullFacts::NONE
        }
    }

    /// The facts when the value is known to be non-null: empty if it can never be.
    #[inline]
    pub fn assume_non_null(self) -> NullFacts {
        if self.intersects(NullFacts(NullFacts::NON_NULL.0 | NullFacts::UNDECLARED.0)) {
            NullFacts::NON_NULL
        } else {
            NullFacts::NONE
        }
    }
}

impl fmt::Debug for NullFacts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts = Vec::new();
        if self.contains(NullFacts::NULL) {
            parts.push("null");
        }
        if self.contains(NullFacts::NON_NULL) {
            parts.push("non-null");
        }
        if self.contains(NullFacts::UNDECLARED) {
            parts.push("undeclared");
        }
        write!(f, "{{{}}}", parts.join(", "))
    }
}

impl Lattice for NullFacts {
    fn bottom() -> Self {
        NullFacts::NONE
    }

    fn is_bottom(&self) -> bool {
        self.0 == 0
    }

    fn join(&mut self, other: &Self) -> bool {
        let next = self.0 | other.0;
        let changed = next != self.0;
        self.0 = next;
        changed
    }

    fn meet(&mut self, other: &Self) -> bool {
        let next = self.0 & other.0;
        let changed = next != self.0;
        self.0 = next;
        changed
    }

    fn leq(&self, other: &Self) -> bool {
        self.0 & !other.0 == 0
    }
}

/// Declared nullability of values entering a function, supplied by the frontend (type
/// annotations, non-null types, the receiver convention of the language).
pub trait NullabilityFacts {
    /// Facts of the `index`-th parameter on entry.
    fn param(&self, func: &FuncIr, index: usize) -> NullFacts;
    /// Facts of a value loaded from `place`.
    fn load(&self, func: &FuncIr, place: &Place) -> NullFacts;
    /// Facts of the value returned by the call at `site`.
    fn call_result(&self, func: &FuncIr, site: CallSiteId, callee: &Callee) -> NullFacts;
    /// Facts of the value produced by the intrinsic `name`.
    fn intrinsic_result(&self, _func: &FuncIr, _name: u32) -> NullFacts {
        NullFacts::UNDECLARED
    }
}

/// No declared nullability: every incoming value is `UNDECLARED`.
#[derive(Clone, Copy, Debug, Default)]
pub struct UndeclaredNullability;

impl NullabilityFacts for UndeclaredNullability {
    fn param(&self, _func: &FuncIr, _index: usize) -> NullFacts {
        NullFacts::UNDECLARED
    }

    fn load(&self, _func: &FuncIr, _place: &Place) -> NullFacts {
        NullFacts::UNDECLARED
    }

    fn call_result(&self, _func: &FuncIr, _site: CallSiteId, _callee: &Callee) -> NullFacts {
        NullFacts::UNDECLARED
    }
}

/// The state: a reachability flag and the facts of every tracked reference. Untracked
/// values (non-references) are absent.
pub type NullEnv = Product<bool, MapLattice<ValueId, NullFacts>>;

fn unreached() -> NullEnv {
    Product(false, MapLattice::new())
}

/// Facts of a value in `env`; untracked values count as `UNDECLARED`.
fn facts_of(env: &NullEnv, v: ValueId) -> NullFacts {
    env.1.get(&v).copied().unwrap_or(NullFacts::UNDECLARED)
}

// ── Dereferences ─────────────────────────────────────────────────────────────

/// How a value is dereferenced.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DerefKind {
    Field,
    Element,
    Pointer,
    /// Receiver of a virtual or interface call.
    Receiver,
}

/// Verdict on one dereference.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DerefVerdict {
    /// Only non-null values reach it.
    Safe,
    /// Only null reaches it: it always fails.
    DefinitelyNull,
    /// Null reaches it on some path.
    NullOnSomePath,
    /// A value of undeclared nullability reaches it; no verdict is derivable.
    Undetermined,
}

/// One dereference and its verdict.
#[derive(Clone, Copy, Debug)]
pub struct DerefCheck {
    pub block: u32,
    pub index: u32,
    pub value: ValueId,
    pub kind: DerefKind,
    pub facts: NullFacts,
    pub verdict: DerefVerdict,
}

fn verdict(facts: NullFacts) -> DerefVerdict {
    if facts == NullFacts::NULL {
        DerefVerdict::DefinitelyNull
    } else if facts.contains(NullFacts::NULL) {
        DerefVerdict::NullOnSomePath
    } else if facts.contains(NullFacts::UNDECLARED) {
        DerefVerdict::Undetermined
    } else {
        DerefVerdict::Safe
    }
}

fn place_deref(place: &Place) -> Option<(ValueId, DerefKind)> {
    match place {
        Place::Field { base, .. } => Some((*base, DerefKind::Field)),
        Place::Index { base, .. } => Some((*base, DerefKind::Element)),
        Place::Deref(v) => Some((*v, DerefKind::Pointer)),
        Place::Local(_) | Place::Global(_) | Place::Static { .. } => None,
    }
}

/// The values an instruction dereferences.
pub fn derefs_of(inst: &Inst) -> Vec<(ValueId, DerefKind)> {
    match inst {
        Inst::Load { place, .. } | Inst::Store { place, .. } => place_deref(place).into_iter().collect(),
        Inst::Call { callee, .. } => match callee {
            Callee::Virtual { receiver, .. } | Callee::Interface { receiver, .. } => {
                vec![(*receiver, DerefKind::Receiver)]
            }
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

// ── Transfer functions ───────────────────────────────────────────────────────

/// Nullness transfer functions for one function.
pub struct NullnessTransfer<'a> {
    func: &'a FuncIr,
    defs: Vec<Option<DefPoint>>,
    facts: &'a dyn NullabilityFacts,
}

impl<'a> NullnessTransfer<'a> {
    pub fn new(func: &'a FuncIr, facts: &'a dyn NullabilityFacts) -> Self {
        Self {
            func,
            defs: def_points(func),
            facts,
        }
    }

    /// Facts of an operand; `None` for untracked (non-reference) operands.
    fn operand<O: OperandView + ?Sized>(&self, env: &NullEnv, op: &O) -> Option<NullFacts> {
        if let Some(c) = op.constant() {
            return match c {
                ConstKey::Null => Some(NullFacts::NULL),
                ConstKey::Str(_) => Some(NullFacts::NON_NULL),
                _ => None,
            };
        }
        op.value().and_then(|v| env.1.get(&v).copied())
    }

    fn bind(env: &mut NullEnv, v: ValueId, facts: Option<NullFacts>) {
        match facts {
            Some(f) => env.1.set(v, f),
            None => {
                env.1.remove(&v);
            }
        }
    }

    /// Refines `v` with `refine`; returns false when the state became unreachable.
    fn refine(env: &mut NullEnv, v: ValueId, refine: fn(NullFacts) -> NullFacts) -> bool {
        let next = refine(facts_of(env, v));
        if next.is_bottom() {
            *env = unreached();
            return false;
        }
        env.1.set(v, next);
        true
    }

    /// Refines the state under "`cond` has truth value `truth`".
    fn assume<O: OperandView + ?Sized>(&self, env: &mut NullEnv, cond: &O, truth: bool, depth: u32) -> bool {
        if depth > 16 {
            return true;
        }
        let ResolvedOperand::Value(v) = resolve_copies(self.func, &self.defs, cond) else {
            return true;
        };
        match defining_inst(self.func, &self.defs, v) {
            Some(Inst::Compare { pred, lhs, rhs, .. }) => {
                let op = match CmpOp::from_ir(pred) {
                    Some(op @ (CmpOp::Eq | CmpOp::Ne)) => op,
                    _ => return true,
                };
                let l = resolve_copies(self.func, &self.defs, lhs);
                let r = resolve_copies(self.func, &self.defs, rhs);
                let tested = match (l, r) {
                    (ResolvedOperand::Value(x), ResolvedOperand::Const(ConstKey::Null))
                    | (ResolvedOperand::Const(ConstKey::Null), ResolvedOperand::Value(x)) => x,
                    _ => return true,
                };
                let is_null = (op == CmpOp::Eq) == truth;
                let refine: fn(NullFacts) -> NullFacts = if is_null {
                    NullFacts::assume_null
                } else {
                    NullFacts::assume_non_null
                };
                let mut targets = vec![tested];
                for o in [lhs.value(), rhs.value()].into_iter().flatten() {
                    if !targets.contains(&o) {
                        targets.push(o);
                    }
                }
                for t in targets {
                    if !Self::refine(env, t, refine) {
                        return false;
                    }
                }
                true
            }
            Some(Inst::Unary { op, src, .. }) if UnaryOp::from_ir(op) == Some(UnaryOp::Not) => {
                self.assume(env, src, !truth, depth + 1)
            }
            _ => true,
        }
    }

    fn bind_phis(&self, cfg: &IrCfg, edge: &CfgEdge, env: &mut NullEnv) {
        let pred = cfg.block_id(edge.from);
        let mut bindings: Vec<(ValueId, Option<NullFacts>)> = Vec::new();
        for inst in &self.func.blocks[edge.to as usize].insts {
            if let Inst::Phi { dst, incoming } = inst {
                let mut acc: Option<NullFacts> = None;
                for op in phi_operands_from(incoming, pred) {
                    if let Some(f) = self.operand(env, op) {
                        acc = Some(acc.unwrap_or(NullFacts::NONE).union(f));
                    } else if op.value().is_some() {
                        acc = Some(acc.unwrap_or(NullFacts::NONE).union(NullFacts::UNDECLARED));
                    }
                }
                bindings.push((*dst, acc));
            }
        }
        for (dst, f) in bindings {
            Self::bind(env, dst, f);
        }
    }
}

impl ForwardTransfer<NullEnv> for NullnessTransfer<'_> {
    fn entry_state(&self, func: &FuncIr, _cfg: &IrCfg) -> NullEnv {
        let mut env = Product(true, MapLattice::new());
        for (i, &p) in func.params.iter().enumerate() {
            env.1.set(p, self.facts.param(func, i));
        }
        env
    }

    fn transfer_inst(&self, _at: InstPoint<'_>, inst: &Inst, state: &mut NullEnv) {
        if !state.0 {
            return;
        }
        match inst {
            Inst::Assign { dst, src } => {
                let f = self.operand(state, src);
                Self::bind(state, *dst, f);
            }
            Inst::Cast { dst, src, .. } => {
                let f = self.operand(state, src);
                Self::bind(state, *dst, f);
            }
            Inst::Alloc { dst, .. } | Inst::MakeClosure { dst, .. } => {
                state.1.set(*dst, NullFacts::NON_NULL)
            }
            Inst::Load { dst, place } => {
                let f = self.facts.load(self.func, place);
                state.1.set(*dst, f);
            }
            Inst::Call {
                dst: Some(d),
                site,
                callee,
                ..
            } => {
                let f = self.facts.call_result(self.func, *site, callee);
                state.1.set(*d, f);
            }
            Inst::Intrinsic {
                dst: Some(d), name, ..
            } => {
                let f = self.facts.intrinsic_result(self.func, *name);
                state.1.set(*d, f);
            }
            _ => {}
        }
        // The normal continuation of a dereference knows the base is not null.
        for (v, _) in derefs_of(inst) {
            if !Self::refine(state, v, NullFacts::assume_non_null) {
                return;
            }
        }
    }

    fn transfer_edge(&self, func: &FuncIr, cfg: &IrCfg, edge: &CfgEdge, state: &mut NullEnv) -> bool {
        if !state.0 {
            return false;
        }
        let feasible = match (&func.blocks[edge.from as usize].term, edge.kind) {
            (Terminator::Branch { cond, .. }, EdgeKind::True) => self.assume(state, cond, true, 0),
            (Terminator::Branch { cond, .. }, EdgeKind::False) => self.assume(state, cond, false, 0),
            _ => true,
        };
        if !feasible || !state.0 {
            return false;
        }
        self.bind_phis(cfg, edge, state);
        true
    }
}

// ── Results ──────────────────────────────────────────────────────────────────

/// Nullness of one function.
#[derive(Clone, Debug)]
pub struct NullnessResult {
    pub dataflow: DataflowResult<NullEnv>,
    /// Every dereference in a reached block, in block order.
    pub derefs: Vec<DerefCheck>,
    /// `(block, edge)`: null tests whose outcome the facts decide (only `edge` is feasible).
    pub decided_tests: Vec<(u32, u32)>,
    /// Facts of each value returned, joined over all reached `Return`s.
    pub returns: NullFacts,
}

/// Runs the nullness analysis on `func`.
pub fn analyze(
    func: &FuncIr,
    cfg: &IrCfg,
    facts: &dyn NullabilityFacts,
    options: &SolverOptions,
) -> NullnessResult {
    let transfer = NullnessTransfer::new(func, facts);
    let dataflow = solve_forward(func, cfg, &transfer, options);

    let mut derefs = Vec::new();
    for &b in &cfg.rpo {
        replay_forward(func, cfg, &transfer, &dataflow, b, |at, inst, state| {
            if !state.0 {
                return;
            }
            for (v, kind) in derefs_of(inst) {
                let f = facts_of(state, v);
                derefs.push(DerefCheck {
                    block: at.block,
                    index: at.index,
                    value: v,
                    kind,
                    facts: f,
                    verdict: verdict(f),
                });
            }
        });
    }

    let mut decided_tests = Vec::new();
    let mut returns = NullFacts::NONE;
    for &b in &cfg.rpo {
        let bi = b as usize;
        if !dataflow.reached[bi] || !dataflow.block_out[bi].0 {
            continue;
        }
        match &func.blocks[bi].term {
            Terminator::Branch { .. } => {
                let edges: Vec<u32> = cfg.succ_edges[bi]
                    .iter()
                    .copied()
                    .filter(|&e| matches!(cfg.edges[e as usize].kind, EdgeKind::True | EdgeKind::False))
                    .collect();
                let live: Vec<u32> = edges
                    .iter()
                    .copied()
                    .filter(|&e| dataflow.edge_state[e as usize].is_some())
                    .collect();
                if edges.len() == 2 && live.len() == 1 {
                    decided_tests.push((b, live[0]));
                }
            }
            Terminator::Return(Some(op)) => {
                let f = match transfer.operand(&dataflow.block_out[bi], op) {
                    Some(f) => f,
                    None if op.value().is_some() => NullFacts::UNDECLARED,
                    None => NullFacts::NONE,
                };
                returns = returns.union(f);
            }
            _ => {}
        }
    }

    NullnessResult {
        dataflow,
        derefs,
        decided_tests,
        returns,
    }
}
