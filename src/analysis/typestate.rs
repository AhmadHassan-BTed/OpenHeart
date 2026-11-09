//! Typestate: abstract values of state-carrying locations and enumeration-valued variables per
//! program point, the transitions between them, and the conditions that guard them.
//!
//! **What is tracked is derived, not named.** Within a function, a memory location is tracked
//! when the function stores a *state value* into it, releases it, or tests a value loaded
//! from it against a state value; an SSA value is tracked when it is tested against a state
//! value (an enumeration variable) or carries a tracked location's value. A *state value* is
//! a constant of the IR (integers, Booleans, `null`, string literals), a static or global the
//! frontend declares immutable ([`SymbolFacts`]), or `Released` — the end of lifetime the IR
//! makes explicit with `Release` (destructor, dispose, unlock).
//!
//! **Domain.** For every tracked key a [`StateSet`] (at most [`STATE_SET_LIMIT`] values, then
//! "any"), plus a must-set of *load links* `(v, ℓ)`: "SSA value `v` still equals the current
//! content of `ℓ`". A test of `v` refines `ℓ` through a live link; a store, release or call
//! that may write `ℓ` breaks the links. Stores are strong when the alias oracle says the name
//! is a must-alias of itself, weak otherwise, and weak into every other tracked location that
//! may alias. Locations named through an SSA value are reset when that value is redefined.
//!
//! **Results.** Each store or release of a tracked location is a [`Transition`] with the
//! location's abstract value before (`from`), the stored values (`to`), and the guard of its
//! block (control dependence over canonical condition atoms). Each test is a [`StateTest`].
//! [`summarize`] groups both by module-level variable (field, static, global) into
//! [`StateMachine`]s; whether a variable is state-like is derived from them.

use std::collections::{BTreeSet, HashMap, HashSet};

use crate::analysis::conditions::{FunctionConditions, Guard};
use crate::analysis::controldep::ControlDependence;
use crate::analysis::lattice::{BoundedSet, Lattice, MapLattice, MustSet, Product};
use crate::analysis::reaching::{AliasOracle, MemLoc};
use crate::analysis::solver::{
    def_points, defining_inst, inst_def, phi_operands_from, replay_forward, resolve_copies,
    solve_forward, CfgEdge, CmpOp, ConstKey, DataflowResult, DefPoint, EdgeKind, ForwardTransfer,
    InstPoint, IrCfg, OperandView, ResolvedOperand, SolverOptions, UnaryOp,
};
use crate::ir::{FieldId, FuncId, FuncIr, GlobalId, Inst, Place, Span, Terminator, ValueId};

/// Largest number of distinct values a state set keeps before it becomes "any".
pub const STATE_SET_LIMIT: usize = 32;

/// A value a state location can hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum StateValue {
    Const(ConstKey),
    /// The value of an immutable static field (for example an enumeration constant).
    Static { owner: u32, field: FieldId },
    /// The value of an immutable global.
    Global(GlobalId),
    /// The location's lifetime ended (`Release`).
    Released,
}

/// The abstract value of a tracked key.
pub type StateSet = BoundedSet<StateValue, STATE_SET_LIMIT>;

/// Facts about symbolic constants, supplied by the frontend from declarations.
pub trait SymbolFacts {
    /// True when the static field holds the same value from initialisation on.
    fn is_constant_static(&self, owner: u32, field: FieldId) -> bool;
    /// True when the global holds the same value from initialisation on.
    fn is_constant_global(&self, global: GlobalId) -> bool;
    /// True when the two state values certainly differ. The default knows only constants:
    /// different integers, Booleans or string literals, and `null` against a string literal.
    fn distinct(&self, a: &StateValue, b: &StateValue) -> bool {
        match (a, b) {
            (StateValue::Const(x), StateValue::Const(y)) => x != y,
            _ => false,
        }
    }
}

/// No declared constants: statics and globals are never state values.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoSymbolFacts;

impl SymbolFacts for NoSymbolFacts {
    fn is_constant_static(&self, _owner: u32, _field: FieldId) -> bool {
        false
    }

    fn is_constant_global(&self, _global: GlobalId) -> bool {
        false
    }
}

/// A key of the typestate environment.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TrackKey {
    Loc(MemLoc),
    Value(ValueId),
}

/// The typestate environment: state sets per tracked key and the live load links.
pub type TypestateEnv = Product<MapLattice<TrackKey, StateSet>, MustSet<(ValueId, MemLoc)>>;

/// A store or release of a tracked location.
#[derive(Clone, Debug)]
pub struct Transition {
    pub loc: MemLoc,
    pub block: u32,
    pub index: u32,
    pub span: Span,
    /// The location's abstract value before the write (`Top`: any value / not derivable).
    pub from: StateSet,
    /// The written value(s).
    pub to: StateSet,
    /// True for a strong update (the location certainly holds `to` afterwards).
    pub strong: bool,
    /// Execution condition of the write.
    pub guard: Guard,
}

/// A test of a tracked value against state values.
#[derive(Clone, Debug)]
pub struct StateTest {
    pub block: u32,
    pub span: Span,
    /// The tested SSA value.
    pub subject: ValueId,
    /// Locations whose current content the subject equals at the test.
    pub locations: Vec<MemLoc>,
    /// The subject's abstract value at the test.
    pub before: StateSet,
    /// Per out-edge: the edge, the value it compares with, whether the edge means "equal",
    /// and whether the edge is feasible.
    pub outcomes: Vec<TestOutcome>,
}

/// One outcome of a [`StateTest`].
#[derive(Clone, Copy, Debug)]
pub struct TestOutcome {
    pub edge: u32,
    pub value: StateValue,
    pub equal: bool,
    pub feasible: bool,
}

/// Typestate of one function.
#[derive(Clone, Debug)]
pub struct TypestateFunction {
    pub dataflow: DataflowResult<TypestateEnv>,
    pub tracked_locations: Vec<MemLoc>,
    pub transitions: Vec<Transition>,
    pub tests: Vec<StateTest>,
}

/// A precomputed test at a block's terminator.
#[derive(Clone, Debug)]
enum BlockTest {
    /// `subject == value` holds iff the branch condition has truth value `equal_on_true`.
    Branch {
        subject: ValueId,
        value: StateValue,
        equal_on_true: bool,
    },
    /// Case edges and the default edge of a switch on `subject`.
    Switch {
        subject: ValueId,
        cases: Vec<(u32, StateValue)>,
        default: Option<u32>,
    },
}

struct TypestateTransfer<'a> {
    func: &'a FuncIr,
    defs: Vec<Option<DefPoint>>,
    alias: &'a dyn AliasOracle,
    symbols: &'a dyn SymbolFacts,
    tracked_locs: BTreeSet<MemLoc>,
    tracked_values: HashSet<ValueId>,
    locs_based_on: HashMap<ValueId, Vec<MemLoc>>,
    tests: HashMap<u32, BlockTest>,
}

fn state_const(c: ConstKey) -> Option<StateValue> {
    match c {
        ConstKey::Float(_) => None,
        other => Some(StateValue::Const(other)),
    }
}

impl<'a> TypestateTransfer<'a> {
    fn loc(&self, place: &Place) -> MemLoc {
        MemLoc::from_place(self.func, &self.defs, place)
    }

    /// The state constant an operand denotes, if any.
    fn constant_of<O: OperandView + ?Sized>(&self, op: &O) -> Option<StateValue> {
        match resolve_copies(self.func, &self.defs, op) {
            ResolvedOperand::Const(c) => state_const(c),
            ResolvedOperand::Value(v) => match defining_inst(self.func, &self.defs, v) {
                Some(Inst::Load {
                    place: Place::Static { owner, field },
                    ..
                }) if self.symbols.is_constant_static(*owner, *field) => Some(StateValue::Static {
                    owner: *owner,
                    field: *field,
                }),
                Some(Inst::Load {
                    place: Place::Global(g),
                    ..
                }) if self.symbols.is_constant_global(*g) => Some(StateValue::Global(*g)),
                _ => None,
            },
            ResolvedOperand::Undef => None,
        }
    }

    /// The abstract value of an operand in `env`.
    fn value_set<O: OperandView + ?Sized>(&self, env: &TypestateEnv, op: &O) -> StateSet {
        if let Some(c) = self.constant_of(op) {
            return StateSet::singleton(c);
        }
        match op.value() {
            Some(v) => env
                .0
                .get(&TrackKey::Value(v))
                .cloned()
                .unwrap_or(BoundedSet::Top),
            None => BoundedSet::Top,
        }
    }

    /// `(subject, value, equal_on_true)` for a branch on `subject == value` / `!=`.
    fn branch_test<O: OperandView + ?Sized>(&self, cond: &O) -> Option<(ValueId, StateValue, bool)> {
        let mut positive = true;
        let mut cur = resolve_copies(self.func, &self.defs, cond);
        for _ in 0..16 {
            let ResolvedOperand::Value(v) = cur else {
                return None;
            };
            match defining_inst(self.func, &self.defs, v)? {
                Inst::Unary { op, src, .. } if UnaryOp::from_ir(op) == Some(UnaryOp::Not) => {
                    positive = !positive;
                    cur = resolve_copies(self.func, &self.defs, src);
                }
                Inst::Compare { pred, lhs, rhs, .. } => {
                    let op = CmpOp::from_ir(pred)?;
                    if !matches!(op, CmpOp::Eq | CmpOp::Ne) {
                        return None;
                    }
                    let equal_on_true = (op == CmpOp::Eq) == positive;
                    let (lc, rc) = (self.constant_of(lhs), self.constant_of(rhs));
                    let subject_of = |o: ResolvedOperand| match o {
                        ResolvedOperand::Value(x) => Some(x),
                        _ => None,
                    };
                    return match (lc, rc) {
                        (None, Some(c)) => {
                            subject_of(resolve_copies(self.func, &self.defs, lhs)).map(|x| (x, c, equal_on_true))
                        }
                        (Some(c), None) => {
                            subject_of(resolve_copies(self.func, &self.defs, rhs)).map(|x| (x, c, equal_on_true))
                        }
                        _ => None,
                    };
                }
                _ => return None,
            }
        }
        None
    }

    /// Writes `to` into `loc` and weakly into every tracked location that may alias it.
    fn write(&self, env: &mut TypestateEnv, loc: MemLoc, to: &StateSet) {
        for &other in &self.tracked_locs {
            if other == loc {
                continue;
            }
            if self.alias.may_alias(self.func, &loc, &other) {
                env.0.join_at(TrackKey::Loc(other), to);
            }
        }
        if self.alias.must_alias(self.func, &loc, &loc) {
            env.0.set(TrackKey::Loc(loc), to.clone());
        } else {
            env.0.join_at(TrackKey::Loc(loc), to);
        }
        let (alias, func) = (self.alias, self.func);
        env.1.remove_where(|(_, l)| alias.may_alias(func, l, &loc));
    }

    /// Refines `subject` (and every location linked to it) under `subject == value` or
    /// `subject != value`; false when the edge is infeasible.
    fn refine(&self, env: &mut TypestateEnv, subject: ValueId, value: StateValue, equal: bool) -> bool {
        let mut keys = vec![TrackKey::Value(subject)];
        for (d, l) in env.1.iter() {
            if *d == subject {
                keys.push(TrackKey::Loc(*l));
            }
        }
        for key in keys {
            let cur = env.0.get(&key).cloned().unwrap_or(BoundedSet::Top);
            let next = if equal {
                match cur.elements() {
                    Some(elems) if !elems.contains(&value) => {
                        if elems.iter().all(|e| self.symbols.distinct(e, &value)) {
                            StateSet::empty()
                        } else {
                            StateSet::singleton(value)
                        }
                    }
                    _ => StateSet::singleton(value),
                }
            } else {
                match cur {
                    BoundedSet::Elems(mut elems) => {
                        elems.remove(&value);
                        BoundedSet::Elems(elems)
                    }
                    BoundedSet::Top => BoundedSet::Top,
                }
            };
            if next.is_bottom() {
                return false;
            }
            env.0.set(key, next);
        }
        true
    }

    fn bind_phis(&self, cfg: &IrCfg, edge: &CfgEdge, env: &mut TypestateEnv) {
        let pred = cfg.block_id(edge.from);
        let mut bindings: Vec<(ValueId, StateSet)> = Vec::new();
        for inst in &self.func.blocks[edge.to as usize].insts {
            if let Inst::Phi { dst, incoming } = inst {
                if !self.tracked_values.contains(dst) {
                    continue;
                }
                let mut acc = StateSet::empty();
                for op in phi_operands_from(incoming, pred) {
                    acc.join(&self.value_set(env, op));
                }
                bindings.push((*dst, acc));
            }
        }
        for (dst, s) in bindings {
            env.1.remove_where(|(d, _)| *d == dst);
            env.0.set(TrackKey::Value(dst), if s.is_bottom() { BoundedSet::Top } else { s });
        }
    }
}

impl ForwardTransfer<TypestateEnv> for TypestateTransfer<'_> {
    fn entry_state(&self, func: &FuncIr, _cfg: &IrCfg) -> TypestateEnv {
        let mut env: TypestateEnv = Product(MapLattice::new(), MustSet::empty());
        let params: HashSet<ValueId> = func.params.iter().copied().collect();
        for &loc in &self.tracked_locs {
            let at_entry = match loc.base() {
                None => true,
                Some(v) => params.contains(&v) || self.defs.get(v.0 as usize).copied().flatten().is_none(),
            };
            if at_entry {
                env.0.set(TrackKey::Loc(loc), BoundedSet::Top);
            }
        }
        for &p in &func.params {
            if self.tracked_values.contains(&p) {
                env.0.set(TrackKey::Value(p), BoundedSet::Top);
            }
        }
        env
    }

    fn transfer_inst(&self, _at: InstPoint<'_>, inst: &Inst, env: &mut TypestateEnv) {
        if let Some(v) = inst_def(inst) {
            // A redefinition starts a new instance of `v`: what was known about the objects
            // named through its old instance no longer applies to names through `v`.
            if let Some(locs) = self.locs_based_on.get(&v) {
                for &l in locs {
                    env.0.set(TrackKey::Loc(l), BoundedSet::Top);
                }
            }
            env.1.remove_where(|(d, l)| *d == v || l.base() == Some(v));
        }
        match inst {
            Inst::Load { dst, place } => {
                let loc = self.loc(place);
                if self.tracked_locs.contains(&loc) {
                    let s = env
                        .0
                        .get(&TrackKey::Loc(loc))
                        .cloned()
                        .unwrap_or(BoundedSet::Top);
                    env.0.set(TrackKey::Value(*dst), s);
                    env.1.insert((*dst, loc));
                } else if self.tracked_values.contains(dst) {
                    env.0.set(TrackKey::Value(*dst), BoundedSet::Top);
                }
            }
            Inst::Assign { dst, src } => {
                if self.tracked_values.contains(dst) {
                    let s = self.value_set(env, src);
                    env.0.set(TrackKey::Value(*dst), s);
                    if let Some(x) = src.value() {
                        let linked: Vec<MemLoc> =
                            env.1.iter().filter(|(d, _)| *d == x).map(|(_, l)| *l).collect();
                        for l in linked {
                            env.1.insert((*dst, l));
                        }
                    }
                }
            }
            Inst::Store { place, value } => {
                let loc = self.loc(place);
                if self.tracked_locs.contains(&loc) {
                    let to = self.value_set(env, value);
                    self.write(env, loc, &to);
                }
            }
            Inst::Release { place } => {
                let loc = self.loc(place);
                if self.tracked_locs.contains(&loc) {
                    self.write(env, loc, &StateSet::singleton(StateValue::Released));
                }
            }
            Inst::Call {
                dst, site, callee, ..
            } => {
                for &l in &self.tracked_locs {
                    if self.alias.call_may_write(self.func, *site, callee, &l) {
                        env.0.set(TrackKey::Loc(l), BoundedSet::Top);
                    }
                }
                let (alias, func) = (self.alias, self.func);
                env.1.remove_where(|(_, l)| alias.call_may_write(func, *site, callee, l));
                if let Some(d) = dst {
                    if self.tracked_values.contains(d) {
                        env.0.set(TrackKey::Value(*d), BoundedSet::Top);
                    }
                }
            }
            Inst::Phi { .. } => {}
            other => {
                if let Some(d) = inst_def(other) {
                    if self.tracked_values.contains(&d) {
                        env.0.set(TrackKey::Value(d), BoundedSet::Top);
                    }
                }
            }
        }
    }

    fn transfer_unwind(&self, at: InstPoint<'_>, inst: &Inst, env: &mut TypestateEnv) {
        // The callee may have written before throwing; its result was never produced.
        if let Inst::Call { site, callee, .. } = inst {
            for &l in &self.tracked_locs {
                if self.alias.call_may_write(at.func, *site, callee, &l) {
                    env.0.set(TrackKey::Loc(l), BoundedSet::Top);
                }
            }
            let (alias, func) = (self.alias, at.func);
            env.1.remove_where(|(_, l)| alias.call_may_write(func, *site, callee, l));
        }
    }

    fn transfer_edge(&self, _func: &FuncIr, cfg: &IrCfg, edge: &CfgEdge, env: &mut TypestateEnv) -> bool {
        let feasible = match (self.tests.get(&edge.from), edge.kind) {
            (
                Some(BlockTest::Branch {
                    subject,
                    value,
                    equal_on_true,
                }),
                EdgeKind::True,
            ) => self.refine(env, *subject, *value, *equal_on_true),
            (
                Some(BlockTest::Branch {
                    subject,
                    value,
                    equal_on_true,
                }),
                EdgeKind::False,
            ) => self.refine(env, *subject, *value, !*equal_on_true),
            (Some(BlockTest::Switch { subject, cases, .. }), EdgeKind::Case { .. }) => {
                let e = cfg
                    .succ_edges[edge.from as usize]
                    .iter()
                    .copied()
                    .find(|&e| cfg.edges[e as usize] == *edge);
                match e.and_then(|e| cases.iter().find(|(ce, _)| *ce == e)) {
                    Some((_, v)) => self.refine(env, *subject, *v, true),
                    None => true,
                }
            }
            (Some(BlockTest::Switch { subject, cases, .. }), EdgeKind::Default) => {
                let mut ok = true;
                for (_, v) in cases {
                    if !self.refine(env, *subject, *v, false) {
                        ok = false;
                        break;
                    }
                }
                ok
            }
            _ => true,
        };
        if !feasible {
            return false;
        }
        self.bind_phis(cfg, edge, env);
        true
    }
}

/// Runs the typestate analysis on one function.
pub fn analyze_function(
    func: &FuncIr,
    cfg: &IrCfg,
    conditions: &FunctionConditions,
    cd: &ControlDependence,
    alias: &dyn AliasOracle,
    symbols: &dyn SymbolFacts,
    options: &SolverOptions,
) -> TypestateFunction {
    let defs = def_points(func);
    let mut transfer = TypestateTransfer {
        func,
        defs,
        alias,
        symbols,
        tracked_locs: BTreeSet::new(),
        tracked_values: HashSet::new(),
        locs_based_on: HashMap::new(),
        tests: HashMap::new(),
    };

    // ── 1. Tests at branch and switch terminators ──
    for (b, block) in func.blocks.iter().enumerate() {
        let b = b as u32;
        if !cfg.is_reachable(b) {
            continue;
        }
        match &block.term {
            Terminator::Branch { cond, .. } => {
                if let Some((subject, value, equal_on_true)) = transfer.branch_test(cond) {
                    transfer.tests.insert(
                        b,
                        BlockTest::Branch {
                            subject,
                            value,
                            equal_on_true,
                        },
                    );
                }
            }
            Terminator::Switch {
                scrutinee, cases, ..
            } => {
                let subject = match resolve_copies(func, &transfer.defs, scrutinee) {
                    ResolvedOperand::Value(v) if transfer.constant_of(scrutinee).is_none() => v,
                    _ => continue,
                };
                let mut case_values = Vec::new();
                for (k, (c, _)) in cases.iter().enumerate() {
                    let Some(value) = state_const(ConstKey::from_ir(c)) else {
                        continue;
                    };
                    if let Some(e) = cfg.succ_edges[b as usize]
                        .iter()
                        .copied()
                        .find(|&e| cfg.edges[e as usize].kind == EdgeKind::Case { index: k as u32 })
                    {
                        case_values.push((e, value));
                    }
                }
                let default = cfg.succ_edges[b as usize]
                    .iter()
                    .copied()
                    .find(|&e| cfg.edges[e as usize].kind == EdgeKind::Default);
                transfer.tests.insert(
                    b,
                    BlockTest::Switch {
                        subject,
                        cases: case_values,
                        default,
                    },
                );
            }
            _ => {}
        }
    }
    for test in transfer.tests.values() {
        match test {
            BlockTest::Branch { subject, .. } | BlockTest::Switch { subject, .. } => {
                transfer.tracked_values.insert(*subject);
            }
        }
    }

    // ── 2. Tracked locations and values, to a common fixpoint: a location is tracked when
    //       a state value (a constant or a tracked value) is stored into it, it is released,
    //       or a tracked value is loaded from it; a value is tracked when it is tested, loaded
    //       from a tracked location, or copied / merged from tracked or constant operands ──
    let mut changed = true;
    while changed {
        changed = false;
        for &b in &cfg.rpo {
            for inst in &func.blocks[b as usize].insts {
                match inst {
                    Inst::Load { dst, place } => {
                        let loc = transfer.loc(place);
                        if transfer.tracked_values.contains(dst) && transfer.tracked_locs.insert(loc) {
                            changed = true;
                        }
                        if transfer.tracked_locs.contains(&loc) && transfer.tracked_values.insert(*dst) {
                            changed = true;
                        }
                    }
                    Inst::Store { place, value } => {
                        let state_valued = transfer.constant_of(value).is_some()
                            || value.value().is_some_and(|v| transfer.tracked_values.contains(&v));
                        let loc = transfer.loc(place);
                        if state_valued && transfer.tracked_locs.insert(loc) {
                            changed = true;
                        }
                    }
                    Inst::Release { place } => {
                        let loc = transfer.loc(place);
                        if transfer.tracked_locs.insert(loc) {
                            changed = true;
                        }
                    }
                    Inst::Assign { dst, src } => {
                        let tracked = transfer.constant_of(src).is_some()
                            || src.value().is_some_and(|v| transfer.tracked_values.contains(&v));
                        if tracked && transfer.tracked_values.insert(*dst) {
                            changed = true;
                        }
                    }
                    Inst::Phi { dst, incoming } => {
                        let tracked = incoming.iter().any(|(_, op)| {
                            transfer.constant_of(op).is_some()
                                || op.value().is_some_and(|v| transfer.tracked_values.contains(&v))
                        });
                        if tracked && transfer.tracked_values.insert(*dst) {
                            changed = true;
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    for &loc in &transfer.tracked_locs {
        if let Some(v) = loc.base() {
            let defined_inside = matches!(
                transfer.defs.get(v.0 as usize).copied().flatten(),
                Some(DefPoint::Inst { .. })
            );
            if defined_inside {
                transfer.locs_based_on.entry(v).or_default().push(loc);
            }
        }
    }

    // ── 3. Solve ──
    let dataflow = solve_forward(func, cfg, &transfer, options);

    // ── 4. Transitions and tests ──
    let mut transitions = Vec::new();
    for &b in &cfg.rpo {
        replay_forward(func, cfg, &transfer, &dataflow, b, |at, inst, env| {
            let (place, to) = match inst {
                Inst::Store { place, value } => (place, transfer.value_set(env, value)),
                Inst::Release { place } => (place, StateSet::singleton(StateValue::Released)),
                _ => return,
            };
            let loc = transfer.loc(place);
            if !transfer.tracked_locs.contains(&loc) {
                return;
            }
            let block = &func.blocks[at.block as usize];
            let span = block
                .inst_spans
                .get(at.index as usize)
                .copied()
                .unwrap_or(block.term_span);
            transitions.push(Transition {
                loc,
                block: at.block,
                index: at.index,
                span,
                from: env
                    .0
                    .get(&TrackKey::Loc(loc))
                    .cloned()
                    .unwrap_or(BoundedSet::Top),
                to,
                strong: alias.must_alias(func, &loc, &loc),
                guard: conditions.block_guard(cfg, cd, at.block),
            });
        });
    }

    let mut tests = Vec::new();
    let mut test_blocks: Vec<u32> = transfer.tests.keys().copied().collect();
    test_blocks.sort_unstable();
    for b in test_blocks {
        let bi = b as usize;
        if !dataflow.reached[bi] {
            continue;
        }
        let env = &dataflow.block_out[bi];
        let (subject, outcomes) = match &transfer.tests[&b] {
            BlockTest::Branch {
                subject,
                value,
                equal_on_true,
            } => {
                let mut outcomes = Vec::new();
                for &e in &cfg.succ_edges[bi] {
                    let equal = match cfg.edges[e as usize].kind {
                        EdgeKind::True => *equal_on_true,
                        EdgeKind::False => !*equal_on_true,
                        _ => continue,
                    };
                    outcomes.push(TestOutcome {
                        edge: e,
                        value: *value,
                        equal,
                        feasible: dataflow.edge_state[e as usize].is_some(),
                    });
                }
                (*subject, outcomes)
            }
            BlockTest::Switch {
                subject,
                cases,
                default,
            } => {
                let mut outcomes: Vec<TestOutcome> = cases
                    .iter()
                    .map(|&(e, v)| TestOutcome {
                        edge: e,
                        value: v,
                        equal: true,
                        feasible: dataflow.edge_state[e as usize].is_some(),
                    })
                    .collect();
                if let Some(d) = default {
                    for &(_, v) in cases {
                        outcomes.push(TestOutcome {
                            edge: *d,
                            value: v,
                            equal: false,
                            feasible: dataflow.edge_state[*d as usize].is_some(),
                        });
                    }
                }
                (*subject, outcomes)
            }
        };
        let locations: Vec<MemLoc> = env
            .1
            .iter()
            .filter(|(d, _)| *d == subject)
            .map(|(_, l)| *l)
            .collect();
        tests.push(StateTest {
            block: b,
            span: func.blocks[bi].term_span,
            subject,
            locations,
            before: env
                .0
                .get(&TrackKey::Value(subject))
                .cloned()
                .unwrap_or(BoundedSet::Top),
            outcomes,
        });
    }

    TypestateFunction {
        dataflow,
        tracked_locations: transfer.tracked_locs.iter().copied().collect(),
        transitions,
        tests,
    }
}

// ── Module-level state machines ──────────────────────────────────────────────

/// A module-level variable: the per-object or per-program identity of a location.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum StateVar {
    /// An instance field, over all objects.
    Field(FieldId),
    Static { owner: u32, field: FieldId },
    Global(GlobalId),
}

impl StateVar {
    /// The module-level variable a location belongs to; locals, elements and dereferences
    /// have none.
    pub fn of(loc: &MemLoc) -> Option<StateVar> {
        match loc {
            MemLoc::Field { field, .. } => Some(StateVar::Field(*field)),
            MemLoc::Static { owner, field } => Some(StateVar::Static {
                owner: *owner,
                field: *field,
            }),
            MemLoc::Global(g) => Some(StateVar::Global(*g)),
            MemLoc::Local(_) | MemLoc::Element { .. } | MemLoc::Deref(_) => None,
        }
    }
}

/// Everything derived about one module-level state variable.
#[derive(Clone, Debug)]
pub struct StateMachine {
    pub var: StateVar,
    /// Every state value written into or compared against the variable.
    pub values: BTreeSet<StateValue>,
    pub transitions: Vec<(FuncId, Transition)>,
    pub tests: Vec<(FuncId, StateTest)>,
    /// True when every write stores a derivable finite set of values.
    pub closed: bool,
}

impl StateMachine {
    /// A variable is state-like when all its writes store derivable values, at least two
    /// distinct values occur, and some branch tests it.
    pub fn is_state_like(&self) -> bool {
        self.closed && self.values.len() >= 2 && !self.tests.is_empty()
    }
}

/// Groups the per-function results by module-level variable.
pub fn summarize(functions: &[(FuncId, &TypestateFunction)]) -> Vec<StateMachine> {
    let mut machines: Vec<StateMachine> = Vec::new();
    let mut index: HashMap<StateVar, usize> = HashMap::new();
    let mut slot = |var: StateVar, machines: &mut Vec<StateMachine>| -> usize {
        *index.entry(var).or_insert_with(|| {
            machines.push(StateMachine {
                var,
                values: BTreeSet::new(),
                transitions: Vec::new(),
                tests: Vec::new(),
                closed: true,
            });
            machines.len() - 1
        })
    };
    for (fid, result) in functions {
        for t in &result.transitions {
            let Some(var) = StateVar::of(&t.loc) else {
                continue;
            };
            let m = slot(var, &mut machines);
            match t.to.elements() {
                Some(values) => machines[m].values.extend(values.iter().copied()),
                None => machines[m].closed = false,
            }
            machines[m].transitions.push((*fid, t.clone()));
        }
        for test in &result.tests {
            for loc in &test.locations {
                let Some(var) = StateVar::of(loc) else {
                    continue;
                };
                let m = slot(var, &mut machines);
                for o in &test.outcomes {
                    machines[m].values.insert(o.value);
                }
                machines[m].tests.push((*fid, test.clone()));
            }
        }
    }
    machines.sort_by_key(|m| m.var);
    machines
}
