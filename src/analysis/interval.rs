//! Integer interval analysis (Cousot & Cousot 1977) on SSA IR.
//!
//! The state is an environment from SSA values to intervals in which an *absent* value is
//! unconstrained (`[−∞, +∞]`), plus a distinguished `Unreached` state. Although SSA values
//! never change, the environment is per program point because branches refine values:
//! on the `True` edge of `if (i < n)` the analysis knows `i ≤ n.hi − 1` and `n ≥ i.lo + 1`.
//! Phis are evaluated on their incoming edges (all phis of a block in parallel), so each
//! incoming value is taken with the refinement of its own edge.
//!
//! Booleans are encoded as `[0, 1]`; comparisons whose operand ranges decide them become
//! `[0, 0]` or `[1, 1]`, and an edge whose condition is refuted is infeasible.
//!
//! Loops terminate by widening at loop heads; the descending pass then recovers bounds such
//! as the loop limit. Fixed-width wrap-around is applied when [`TypeFacts`] give the width;
//! values without a known width are computed over the mathematical integers and listed in
//! [`IntervalResult::width_unknown`].

use std::collections::BTreeMap;

use crate::analysis::lattice::{BitSet, Interval, Lattice};
use crate::analysis::solver::{
    def_points, defining_inst, phi_operands_from, replay_forward, resolve_copies, solve_forward,
    value_count, ArithOp, CfgEdge, CmpOp, ConstKey, DataflowResult, DefPoint, EdgeKind,
    ForwardTransfer, InstPoint, IrCfg, OperandView, ResolvedOperand, ScalarKind, SolverOptions,
    TypeFacts, UnaryOp,
};
use crate::ir::{FuncIr, Inst, Terminator, ValueId};

// ── Environment lattice ──────────────────────────────────────────────────────

/// Interval environment; absent values are unconstrained.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum IntervalEnv {
    Unreached,
    /// Values with a constrained (neither empty nor full) interval.
    Env(BTreeMap<ValueId, Interval>),
}

impl IntervalEnv {
    pub fn unconstrained() -> IntervalEnv {
        IntervalEnv::Env(BTreeMap::new())
    }

    /// The interval of `v` (`⊥` when unreached).
    pub fn get(&self, v: ValueId) -> Interval {
        match self {
            IntervalEnv::Unreached => Interval::Bottom,
            IntervalEnv::Env(m) => m.get(&v).copied().unwrap_or_else(Interval::top),
        }
    }

    /// Strong update of `v`; an empty interval makes the state unreached.
    pub fn set(&mut self, v: ValueId, i: Interval) {
        if let IntervalEnv::Env(m) = self {
            if i.is_bottom() {
                *self = IntervalEnv::Unreached;
            } else if i.is_top() {
                m.remove(&v);
            } else {
                m.insert(v, i);
            }
        }
    }

    /// `v := v ⊓ i`; returns false when the state became unreached.
    pub fn refine(&mut self, v: ValueId, i: Interval) -> bool {
        let next = self.get(v).intersect(&i);
        self.set(v, next);
        !self.is_bottom()
    }
}

impl Lattice for IntervalEnv {
    fn bottom() -> Self {
        IntervalEnv::Unreached
    }

    fn is_bottom(&self) -> bool {
        matches!(self, IntervalEnv::Unreached)
    }

    fn join(&mut self, other: &Self) -> bool {
        let IntervalEnv::Env(b) = other else {
            return false;
        };
        match self {
            IntervalEnv::Unreached => {
                *self = other.clone();
                true
            }
            IntervalEnv::Env(a) => {
                let mut changed = false;
                a.retain(|k, v| match b.get(k) {
                    Some(w) => {
                        let h = v.hull(w);
                        changed |= h != *v;
                        *v = h;
                        !h.is_top()
                    }
                    None => {
                        changed = true;
                        false
                    }
                });
                changed
            }
        }
    }

    fn meet(&mut self, other: &Self) -> bool {
        let IntervalEnv::Env(b) = other else {
            let changed = !self.is_bottom();
            *self = IntervalEnv::Unreached;
            return changed;
        };
        let (changed, empty) = match self {
            IntervalEnv::Unreached => return false,
            IntervalEnv::Env(a) => {
                let mut changed = false;
                let mut empty = false;
                for (k, w) in b {
                    let cur = a.get(k).copied().unwrap_or_else(Interval::top);
                    let next = cur.intersect(w);
                    if next != cur {
                        changed = true;
                        empty |= next.is_bottom();
                        a.insert(*k, next);
                    }
                }
                (changed, empty)
            }
        };
        if empty {
            *self = IntervalEnv::Unreached;
        }
        changed
    }

    fn leq(&self, other: &Self) -> bool {
        match (self, other) {
            (IntervalEnv::Unreached, _) => true,
            (_, IntervalEnv::Unreached) => false,
            (IntervalEnv::Env(a), IntervalEnv::Env(b)) => b
                .iter()
                .all(|(k, w)| a.get(k).is_some_and(|v| v.leq(w))),
        }
    }

    fn widen(&mut self, newer: &Self) -> bool {
        let IntervalEnv::Env(b) = newer else {
            return false;
        };
        match self {
            IntervalEnv::Unreached => {
                *self = newer.clone();
                true
            }
            IntervalEnv::Env(a) => {
                let mut changed = false;
                a.retain(|k, v| match b.get(k) {
                    Some(w) => {
                        changed |= v.widen(w);
                        !v.is_top()
                    }
                    None => {
                        changed = true;
                        false
                    }
                });
                changed
            }
        }
    }

    fn narrow(&mut self, newer: &Self) -> bool {
        let IntervalEnv::Env(b) = newer else {
            let changed = !self.is_bottom();
            *self = IntervalEnv::Unreached;
            return changed;
        };
        let (changed, empty) = match self {
            IntervalEnv::Unreached => return false,
            IntervalEnv::Env(a) => {
                let mut changed = false;
                let top = Interval::top();
                for (k, v) in a.iter_mut() {
                    changed |= v.narrow(b.get(k).unwrap_or(&top));
                }
                for (k, w) in b {
                    if !a.contains_key(k) {
                        a.insert(*k, *w);
                        changed = true;
                    }
                }
                let empty = a.values().any(|v| v.is_bottom());
                a.retain(|_, v| !v.is_top());
                (changed, empty)
            }
        };
        if empty {
            *self = IntervalEnv::Unreached;
        }
        changed
    }
}

// ── Transfer functions ───────────────────────────────────────────────────────

/// Interval transfer functions for one function.
pub struct IntervalTransfer<'a> {
    func: &'a FuncIr,
    defs: Vec<Option<DefPoint>>,
    types: &'a dyn TypeFacts,
}

impl<'a> IntervalTransfer<'a> {
    pub fn new(func: &'a FuncIr, types: &'a dyn TypeFacts) -> Self {
        Self {
            func,
            defs: def_points(func),
            types,
        }
    }

    /// The interval of an operand; non-integer constants are unconstrained.
    fn operand<O: OperandView + ?Sized>(&self, env: &IntervalEnv, op: &O) -> Interval {
        if let Some(c) = op.constant() {
            return const_interval(c).unwrap_or_else(Interval::top);
        }
        match op.value() {
            Some(v) => env.get(v),
            None => Interval::top(),
        }
    }

    /// The range of a value of the declared type of `v` (unconstrained when not an integer).
    fn declared_range(&self, v: ValueId) -> Interval {
        match self.types.value_kind(self.func, v) {
            ScalarKind::Int { bits, signed } => Interval::type_range(bits, signed),
            ScalarKind::Bool => Interval::range(0, 1),
            _ => Interval::top(),
        }
    }

    /// Fits an arithmetic result to the width of `dst`; the flag is true when the width is
    /// unknown and the result is finite (it then assumes no wrap-around).
    fn fit(&self, dst: ValueId, i: Interval) -> (Interval, bool) {
        match self.types.value_kind(self.func, dst) {
            ScalarKind::Int { bits, signed } => (i.wrap_to(bits, signed), false),
            ScalarKind::Float { .. } | ScalarKind::Reference | ScalarKind::Aggregate => {
                (Interval::top(), false)
            }
            _ => (i, !i.is_top() && !i.is_bottom()),
        }
    }

    /// The interval an instruction assigns to its definition, and whether it rests on an
    /// unknown width. `None` for instructions without a definition and for phis (bound on
    /// edges).
    pub fn def_interval(&self, env: &IntervalEnv, inst: &Inst) -> Option<(ValueId, Interval, bool)> {
        let out = match inst {
            Inst::Phi { .. } | Inst::Store { .. } | Inst::Release { .. } => return None,
            Inst::Assign { dst, src } => (*dst, self.operand(env, src), false),
            Inst::Unary { dst, op, src } => {
                let a = self.operand(env, src);
                match UnaryOp::from_ir(op) {
                    Some(UnaryOp::Neg) => {
                        let (i, w) = self.fit(*dst, a.neg());
                        (*dst, i, w)
                    }
                    Some(UnaryOp::Not) if a.leq(&Interval::range(0, 1)) => {
                        (*dst, Interval::constant(1).sub(&a), false)
                    }
                    _ => (*dst, self.declared_range(*dst), false),
                }
            }
            Inst::Binary { dst, op, lhs, rhs } => {
                let (a, b) = (self.operand(env, lhs), self.operand(env, rhs));
                let raw = match ArithOp::from_ir(op) {
                    Some(ArithOp::Add) => a.add(&b),
                    Some(ArithOp::Sub) => a.sub(&b),
                    Some(ArithOp::Mul) => a.mul(&b),
                    Some(ArithOp::Div) => a.div(&b),
                    Some(ArithOp::Rem) => a.rem(&b),
                    Some(ArithOp::And) => a.bit_and(&b),
                    Some(ArithOp::Or) => a.bit_or_xor(&b, false),
                    Some(ArithOp::Xor) => a.bit_or_xor(&b, true),
                    Some(ArithOp::Shl) => a.shl(&b),
                    Some(ArithOp::Shr) => a.shr(&b),
                    None => Interval::top(),
                };
                let (i, w) = self.fit(*dst, raw);
                (*dst, i, w)
            }
            Inst::Compare { dst, pred, lhs, rhs } => {
                let decided = self.decide(CmpOp::from_ir(pred), lhs, rhs, env);
                let i = match decided {
                    Some(true) => Interval::constant(1),
                    Some(false) => Interval::constant(0),
                    None => Interval::range(0, 1),
                };
                (*dst, i, false)
            }
            Inst::Cast { dst, src, ty } => {
                let a = self.operand(env, src);
                let i = match self.types.resolve(*ty) {
                    ScalarKind::Int { bits, signed } => a.wrap_to(bits, signed),
                    ScalarKind::Bool => Interval::range(0, 1),
                    _ => Interval::top(),
                };
                (*dst, i, false)
            }
            Inst::TypeTest { dst, .. } => (*dst, Interval::range(0, 1), false),
            Inst::Load { dst, .. } | Inst::Alloc { dst, .. } | Inst::MakeClosure { dst, .. } => {
                (*dst, self.declared_range(*dst), false)
            }
            Inst::Call { dst, .. } | Inst::Intrinsic { dst, .. } => {
                let d = (*dst)?;
                (d, self.declared_range(d), false)
            }
        };
        Some(out)
    }

    /// Decides `lhs op rhs` from the operand intervals, for integer and Boolean operands.
    fn decide<L: OperandView + ?Sized, R: OperandView + ?Sized>(
        &self,
        op: Option<CmpOp>,
        lhs: &L,
        rhs: &R,
        env: &IntervalEnv,
    ) -> Option<bool> {
        let op = op?;
        if !integral_operand(lhs) || !integral_operand(rhs) {
            return None;
        }
        let (a, b) = (self.operand(env, lhs), self.operand(env, rhs));
        match op {
            CmpOp::Eq => a.decide_eq(&b),
            CmpOp::Ne => a.decide_eq(&b).map(|r| !r),
            CmpOp::Lt => a.decide_lt(&b),
            CmpOp::Le => a.decide_le(&b),
            CmpOp::Gt => b.decide_lt(&a),
            CmpOp::Ge => b.decide_le(&a),
        }
    }

    /// Refines `env` under the assumption that operand `cond` has truth value `truth`;
    /// returns false when the assumption is refuted.
    fn assume<O: OperandView + ?Sized>(&self, env: &mut IntervalEnv, cond: &O, truth: bool, depth: u32) -> bool {
        if depth > 16 {
            return true;
        }
        let root = resolve_copies(self.func, &self.defs, cond);
        let v = match root {
            ResolvedOperand::Const(ConstKey::Bool(b)) => return b == truth,
            ResolvedOperand::Const(_) | ResolvedOperand::Undef => return true,
            ResolvedOperand::Value(v) => v,
        };
        // The condition value itself is 1 or 0 on the edge, when it is known to be Boolean
        // (a branch on a non-Boolean value says nothing about its magnitude here).
        let def = defining_inst(self.func, &self.defs, v);
        let boolean = matches!(def, Some(Inst::Compare { .. }) | Some(Inst::TypeTest { .. }))
            || self.types.value_kind(self.func, v) == ScalarKind::Bool
            || env.get(v).leq(&Interval::range(0, 1));
        if boolean {
            let truth_value = Interval::constant(truth as i128);
            if let Some(direct) = cond.value() {
                if !env.refine(direct, truth_value) {
                    return false;
                }
            }
            if !env.refine(v, truth_value) {
                return false;
            }
        }
        match def {
            Some(Inst::Compare { pred, lhs, rhs, .. }) => {
                let Some(op) = CmpOp::from_ir(pred) else {
                    return true;
                };
                if !integral_operand(lhs) || !integral_operand(rhs) {
                    return true;
                }
                let op = if truth { op } else { op.negate() };
                self.assume_compare(env, op, lhs, rhs)
            }
            Some(Inst::Unary { op, src, .. }) if UnaryOp::from_ir(op) == Some(UnaryOp::Not) => {
                let inner = self.operand(env, src);
                if inner.leq(&Interval::range(0, 1)) {
                    self.assume(env, src, !truth, depth + 1)
                } else {
                    true
                }
            }
            _ => true,
        }
    }

    fn assume_compare<L: OperandView + ?Sized, R: OperandView + ?Sized>(
        &self,
        env: &mut IntervalEnv,
        op: CmpOp,
        lhs: &L,
        rhs: &R,
    ) -> bool {
        let (a, b) = (self.operand(env, lhs), self.operand(env, rhs));
        let (a2, b2) = match op {
            CmpOp::Lt => Interval::refine_lt(&a, &b),
            CmpOp::Le => Interval::refine_le(&a, &b),
            CmpOp::Gt => {
                let (b2, a2) = Interval::refine_lt(&b, &a);
                (a2, b2)
            }
            CmpOp::Ge => {
                let (b2, a2) = Interval::refine_le(&b, &a);
                (a2, b2)
            }
            CmpOp::Eq => Interval::refine_eq(&a, &b),
            CmpOp::Ne => Interval::refine_ne(&a, &b),
        };
        if a2.is_bottom() || b2.is_bottom() {
            *env = IntervalEnv::Unreached;
            return false;
        }
        for (op_view, refined) in [(lhs.value(), a2), (rhs.value(), b2)] {
            if let Some(x) = op_view {
                if !env.refine(x, refined) {
                    return false;
                }
                if let ResolvedOperand::Value(root) = resolve_copies(self.func, &self.defs, &x) {
                    if root != x && !env.refine(root, refined) {
                        return false;
                    }
                }
            }
        }
        true
    }

    /// Binds the phis of `edge.to` for the incoming edge, in parallel.
    fn bind_phis(&self, cfg: &IrCfg, edge: &CfgEdge, env: &mut IntervalEnv) {
        let pred = cfg.block_id(edge.from);
        let mut bindings: Vec<(ValueId, Interval)> = Vec::new();
        for inst in &self.func.blocks[edge.to as usize].insts {
            if let Inst::Phi { dst, incoming } = inst {
                let mut acc = Interval::Bottom;
                let mut any = false;
                for op in phi_operands_from(incoming, pred) {
                    any = true;
                    acc = acc.hull(&self.operand(env, op));
                }
                bindings.push((*dst, if any { acc } else { Interval::top() }));
            }
        }
        for (dst, i) in bindings {
            if i.is_bottom() {
                // An undefined incoming value constrains nothing.
                env.set(dst, Interval::top());
            } else {
                env.set(dst, i);
            }
        }
    }
}

/// True unless the operand is a non-integer, non-Boolean constant (the comparison is then
/// over floats, strings or references and intervals say nothing about it).
fn integral_operand<O: OperandView + ?Sized>(op: &O) -> bool {
    match op.constant() {
        Some(c) => const_interval(c).is_some(),
        None => true,
    }
}

fn const_interval(c: ConstKey) -> Option<Interval> {
    match c {
        ConstKey::Int(i) => Some(Interval::constant(i)),
        ConstKey::Bool(b) => Some(Interval::constant(b as i128)),
        _ => None,
    }
}

impl ForwardTransfer<IntervalEnv> for IntervalTransfer<'_> {
    fn entry_state(&self, func: &FuncIr, _cfg: &IrCfg) -> IntervalEnv {
        let mut env = IntervalEnv::unconstrained();
        for &p in &func.params {
            env.set(p, self.declared_range(p));
        }
        env
    }

    fn transfer_inst(&self, _at: InstPoint<'_>, inst: &Inst, state: &mut IntervalEnv) {
        if state.is_bottom() {
            return;
        }
        if let Some((dst, i, _)) = self.def_interval(state, inst) {
            state.set(dst, i);
        }
    }

    fn transfer_edge(&self, func: &FuncIr, cfg: &IrCfg, edge: &CfgEdge, state: &mut IntervalEnv) -> bool {
        if state.is_bottom() {
            return false;
        }
        let feasible = match (&func.blocks[edge.from as usize].term, edge.kind) {
            (Terminator::Branch { cond, .. }, EdgeKind::True) => self.assume(state, cond, true, 0),
            (Terminator::Branch { cond, .. }, EdgeKind::False) => self.assume(state, cond, false, 0),
            (Terminator::Switch { scrutinee, cases, .. }, EdgeKind::Case { index }) => {
                match cases.get(index as usize).map(|(c, _)| ConstKey::from_ir(c)) {
                    Some(c) => match (const_interval(c), scrutinee.value()) {
                        (Some(ci), Some(v)) => state.refine(v, ci),
                        (Some(ci), None) => self.operand(state, scrutinee).intersect(&ci) != Interval::Bottom,
                        _ => true,
                    },
                    None => true,
                }
            }
            (Terminator::Switch { scrutinee, cases, .. }, EdgeKind::Default) => {
                let mut ok = true;
                if let Some(v) = scrutinee.value() {
                    for (c, _) in cases {
                        if let Some(ci) = const_interval(ConstKey::from_ir(c)) {
                            let (refined, _) = Interval::refine_ne(&state.get(v), &ci);
                            if !state.refine(v, refined) {
                                ok = false;
                                break;
                            }
                        }
                    }
                }
                ok
            }
            _ => true,
        };
        if !feasible || state.is_bottom() {
            return false;
        }
        self.bind_phis(cfg, edge, state);
        !state.is_bottom()
    }
}

// ── Results ──────────────────────────────────────────────────────────────────

/// Interval analysis of one function.
#[derive(Clone, Debug)]
pub struct IntervalResult {
    pub dataflow: DataflowResult<IntervalEnv>,
    /// Range of each SSA value at its definition (`ValueId.0`); `⊥` when it is defined on no
    /// feasible path.
    pub ranges: Vec<Interval>,
    /// Values whose range was computed without knowing their integer width.
    pub width_unknown: BitSet,
    /// Edges leaving reached blocks that no feasible state takes.
    pub infeasible_edges: Vec<u32>,
}

impl IntervalResult {
    pub fn range(&self, v: ValueId) -> Interval {
        self.ranges.get(v.0 as usize).copied().unwrap_or_else(Interval::top)
    }
}

/// Runs the interval analysis on `func`.
pub fn analyze(
    func: &FuncIr,
    cfg: &IrCfg,
    types: &dyn TypeFacts,
    options: &SolverOptions,
) -> IntervalResult {
    let transfer = IntervalTransfer::new(func, types);
    let dataflow = solve_forward(func, cfg, &transfer, options);

    let mut ranges = vec![Interval::Bottom; value_count(func)];
    let mut width_unknown = BitSet::new();
    if let Some(entry) = dataflow.block_in.get(cfg.entry as usize) {
        for &p in &func.params {
            if let Some(slot) = ranges.get_mut(p.0 as usize) {
                *slot = entry.get(p);
            }
        }
    }
    for &b in &cfg.rpo {
        replay_forward(func, cfg, &transfer, &dataflow, b, |_, inst, state| {
            if let Inst::Phi { dst, .. } = inst {
                if let Some(slot) = ranges.get_mut(dst.0 as usize) {
                    *slot = slot.hull(&state.get(*dst));
                }
                return;
            }
            if let Some((dst, i, w)) = transfer.def_interval(state, inst) {
                if let Some(slot) = ranges.get_mut(dst.0 as usize) {
                    *slot = slot.hull(&i);
                }
                if w {
                    width_unknown.insert(dst.0 as usize);
                }
            }
        });
    }

    let infeasible_edges = (0..cfg.edges.len() as u32)
        .filter(|&e| {
            let from = cfg.edges[e as usize].from as usize;
            dataflow.reached[from] && dataflow.edge_state[e as usize].is_none()
        })
        .collect();

    IntervalResult {
        dataflow,
        ranges,
        width_unknown,
        infeasible_edges,
    }
}
