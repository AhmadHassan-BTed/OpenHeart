//! Sparse conditional constant propagation (Wegman & Zadeck 1991) on SSA IR.
//!
//! Two worklists run to a common fixpoint:
//!
//! - the **flow worklist** holds CFG edges that became executable; a block is evaluated the
//!   first time one of its in-edges becomes executable, and its phis again whenever another
//!   in-edge does;
//! - the **SSA worklist** holds values whose lattice value rose; every executable use of such
//!   a value is re-evaluated.
//!
//! Values live in the flat lattice `⊥ (no evidence yet) < constant < ⊤ (not constant)`.
//! A phi joins only the operands whose incoming edge is executable, and a branch on a constant
//! makes only one out-edge executable, so constants that hold on every *executable* path are
//! found even through loops and dead branches.
//!
//! **Folding is width-exact or absent.** Integer results are folded with the width and
//! signedness the [`TypeFacts`] report: unsigned arithmetic wraps; signed overflow wraps only
//! when the language defines it to ([`TypeFacts::signed_overflow_wraps`]). When the width is
//! not known, a result is folded only if it is the same in *every* integer type able to hold
//! the operands; otherwise the value becomes `⊤` and is listed in
//! [`SccpResult::type_dependent`]. Floating-point arithmetic is folded only for a known width.
//! Division by zero, shifts by at least the width and unresolved casts give `⊤`.

use crate::analysis::conditions::fold_const_compare;
use crate::analysis::lattice::{BitSet, Flat, Lattice};
use crate::analysis::reaching::{UseSite, ValueDefUse};
use crate::analysis::solver::{
    phi_operands_from, value_count, ArithOp, CmpOp, ConstKey, EdgeKind, IrCfg, OperandView,
    ScalarKind, TypeFacts, UnaryOp,
};
use crate::ir::{FuncIr, Inst, Terminator, TypeId, ValueId};

/// Outcome of folding one operation on constants.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fold {
    Const(ConstKey),
    /// Not a single constant (trap, undefined behaviour, uninterpreted operator).
    Unknown,
    /// Would be constant, but the value depends on a width or overflow rule that is not
    /// known.
    TypeDependent,
}

// ── Integer width helpers ────────────────────────────────────────────────────

#[inline]
fn bit_len(x: i128) -> u32 {
    128 - x.leading_zeros()
}

/// Bits of the narrowest signed type holding `x`.
#[inline]
fn signed_bits(x: i128) -> u32 {
    if x >= 0 {
        bit_len(x) + 1
    } else {
        bit_len(!x) + 1
    }
}

/// Bits of the narrowest unsigned type holding the non-negative `x`.
#[inline]
fn unsigned_bits(x: i128) -> u32 {
    bit_len(x).max(1)
}

#[inline]
fn fits_signed(r: i128, bits: u32) -> bool {
    if bits >= 128 {
        return true;
    }
    let half = 1i128 << (bits - 1);
    -half <= r && r < half
}

#[inline]
fn fits_unsigned(r: i128, bits: u32) -> bool {
    r >= 0 && (bits >= 127 || r < (1i128 << bits))
}

/// True when `r` is representable in every integer type, of any width and signedness, that
/// can represent all of `operands`.
fn same_in_every_holder(r: i128, operands: &[i128]) -> bool {
    let ws = operands.iter().map(|&x| signed_bits(x)).max().unwrap_or(1);
    if operands.iter().any(|&x| x < 0) {
        fits_signed(r, ws)
    } else {
        let wu = operands.iter().map(|&x| unsigned_bits(x)).max().unwrap_or(1);
        fits_signed(r, ws) && fits_unsigned(r, wu)
    }
}

/// Two's-complement sign extension of the low `bits` bits.
#[inline]
fn wrap_signed(x: i128, bits: u16) -> i128 {
    if bits == 0 || bits >= 128 {
        return x;
    }
    let shift = 128 - bits as u32;
    (x << shift) >> shift
}

/// The low `bits` bits as an unsigned value (`None` when it does not fit an `i128`).
#[inline]
fn wrap_unsigned(x: i128, bits: u16) -> Option<i128> {
    if bits >= 128 {
        return if x >= 0 { Some(x) } else { None };
    }
    if bits == 0 {
        return Some(0);
    }
    let shift = 128 - bits as u32;
    Some((((x as u128) << shift) >> shift) as i128)
}

// ── Constant folder ──────────────────────────────────────────────────────────

/// Folds IR operations on constants under the [`TypeFacts`] of one function.
pub struct ConstantFolder<'a> {
    pub func: &'a FuncIr,
    pub types: &'a dyn TypeFacts,
}

impl ConstantFolder<'_> {
    /// Fixes an exact integer result to the type of `dst`.
    fn int_result(&self, dst: ValueId, exact: Option<i128>, operands: &[i128]) -> Fold {
        match self.types.value_kind(self.func, dst).int_width() {
            Some((bits, signed)) => {
                // An exact result outside `i128` belongs to no supported width.
                let Some(x) = exact else {
                    return Fold::Unknown;
                };
                if signed {
                    if fits_signed(x, bits as u32) {
                        Fold::Const(ConstKey::Int(x))
                    } else if self.types.signed_overflow_wraps() {
                        Fold::Const(ConstKey::Int(wrap_signed(x, bits)))
                    } else {
                        Fold::Unknown
                    }
                } else {
                    match wrap_unsigned(x, bits) {
                        Some(w) => Fold::Const(ConstKey::Int(w)),
                        None => Fold::Unknown,
                    }
                }
            }
            None => match exact {
                Some(x) if same_in_every_holder(x, operands) => Fold::Const(ConstKey::Int(x)),
                _ => Fold::TypeDependent,
            },
        }
    }

    /// The width bounding shift distances for the type of `dst`, when known.
    fn shift_limit(&self, dst: ValueId) -> Option<u16> {
        self.types.value_kind(self.func, dst).int_width().map(|(b, _)| b)
    }

    pub fn binary(&self, dst: ValueId, op: ArithOp, a: ConstKey, b: ConstKey) -> Fold {
        match (a, b) {
            (ConstKey::Int(x), ConstKey::Int(y)) => self.int_binary(dst, op, x, y),
            (ConstKey::Bool(x), ConstKey::Bool(y)) => match op {
                ArithOp::And => Fold::Const(ConstKey::Bool(x & y)),
                ArithOp::Or => Fold::Const(ConstKey::Bool(x | y)),
                ArithOp::Xor => Fold::Const(ConstKey::Bool(x ^ y)),
                _ => Fold::Unknown,
            },
            (ConstKey::Float(x), ConstKey::Float(y)) => self.float_binary(dst, op, x, y),
            _ => Fold::Unknown,
        }
    }

    fn int_binary(&self, dst: ValueId, op: ArithOp, x: i128, y: i128) -> Fold {
        let both = [x, y];
        match op {
            ArithOp::Add => self.int_result(dst, x.checked_add(y), &both),
            ArithOp::Sub => self.int_result(dst, x.checked_sub(y), &both),
            ArithOp::Mul => self.int_result(dst, x.checked_mul(y), &both),
            ArithOp::Div | ArithOp::Rem => {
                if y == 0 {
                    return Fold::Unknown;
                }
                let exact = if op == ArithOp::Div {
                    x.checked_div(y)
                } else {
                    x.checked_rem(y)
                };
                // The quotient of MIN / -1 overflows a signed type; its remainder is defined
                // only where the quotient is.
                if let Some((bits, true)) = self.types.value_kind(self.func, dst).int_width() {
                    let min = if bits >= 128 {
                        i128::MIN
                    } else {
                        -(1i128 << (bits - 1))
                    };
                    if x == min && y == -1 && !self.types.signed_overflow_wraps() {
                        return Fold::Unknown;
                    }
                }
                self.int_result(dst, exact, &both)
            }
            ArithOp::And => self.int_result(dst, Some(x & y), &both),
            ArithOp::Or => self.int_result(dst, Some(x | y), &both),
            ArithOp::Xor => self.int_result(dst, Some(x ^ y), &both),
            ArithOp::Shl | ArithOp::Shr => {
                if y < 0 {
                    return Fold::Unknown;
                }
                match self.shift_limit(dst) {
                    Some(bits) if y >= bits as i128 => Fold::Unknown,
                    _ => {}
                }
                if y >= 127 {
                    return Fold::Unknown;
                }
                let exact = if op == ArithOp::Shl {
                    x.checked_mul(1i128 << y)
                } else {
                    Some(x >> y)
                };
                self.int_result(dst, exact, &[x])
            }
        }
    }

    fn float_binary(&self, dst: ValueId, op: ArithOp, x: u64, y: u64) -> Fold {
        let (a, b) = (f64::from_bits(x), f64::from_bits(y));
        match self.types.value_kind(self.func, dst) {
            ScalarKind::Float { bits: 64 } => {
                let r = match op {
                    ArithOp::Add => a + b,
                    ArithOp::Sub => a - b,
                    ArithOp::Mul => a * b,
                    ArithOp::Div => a / b,
                    ArithOp::Rem => a % b,
                    _ => return Fold::Unknown,
                };
                Fold::Const(ConstKey::Float(r.to_bits()))
            }
            ScalarKind::Float { bits: 32 } => {
                let (a, b) = (a as f32, b as f32);
                let r = match op {
                    ArithOp::Add => a + b,
                    ArithOp::Sub => a - b,
                    ArithOp::Mul => a * b,
                    ArithOp::Div => a / b,
                    ArithOp::Rem => a % b,
                    _ => return Fold::Unknown,
                };
                Fold::Const(ConstKey::Float((r as f64).to_bits()))
            }
            ScalarKind::Float { .. } => Fold::Unknown,
            _ => Fold::TypeDependent,
        }
    }

    pub fn unary(&self, dst: ValueId, op: UnaryOp, a: ConstKey) -> Fold {
        match (op, a) {
            (UnaryOp::Not, ConstKey::Bool(b)) => Fold::Const(ConstKey::Bool(!b)),
            (UnaryOp::Neg, ConstKey::Int(x)) => self.int_result(dst, x.checked_neg(), &[x]),
            (UnaryOp::Neg, ConstKey::Float(bits)) => Fold::Const(ConstKey::Float(bits ^ (1u64 << 63))),
            _ => Fold::Unknown,
        }
    }

    pub fn compare(&self, op: CmpOp, a: ConstKey, b: ConstKey) -> Fold {
        match fold_const_compare(op, a, b) {
            Some(r) => Fold::Const(ConstKey::Bool(r)),
            None => Fold::Unknown,
        }
    }

    /// Conversions whose result every language that permits them agrees on.
    pub fn cast(&self, ty: TypeId, a: ConstKey) -> Fold {
        let target = self.types.resolve(ty);
        match (target, a) {
            (ScalarKind::Unknown, _) => Fold::TypeDependent,
            (ScalarKind::Int { bits, signed }, ConstKey::Int(x)) => {
                if signed {
                    if fits_signed(x, bits as u32) {
                        Fold::Const(ConstKey::Int(x))
                    } else if self.types.signed_overflow_wraps() {
                        Fold::Const(ConstKey::Int(wrap_signed(x, bits)))
                    } else {
                        Fold::Unknown
                    }
                } else {
                    wrap_unsigned(x, bits).map_or(Fold::Unknown, |w| Fold::Const(ConstKey::Int(w)))
                }
            }
            (ScalarKind::Int { .. }, ConstKey::Bool(b)) => Fold::Const(ConstKey::Int(b as i128)),
            (ScalarKind::Int { bits, signed }, ConstKey::Float(f)) => {
                let v = f64::from_bits(f);
                if !v.is_finite() {
                    return Fold::Unknown;
                }
                let t = v.trunc();
                if t < i128::MIN as f64 || t >= i128::MAX as f64 {
                    return Fold::Unknown;
                }
                let x = t as i128;
                let fits = if signed {
                    fits_signed(x, bits as u32)
                } else {
                    fits_unsigned(x, bits as u32)
                };
                if fits {
                    Fold::Const(ConstKey::Int(x))
                } else {
                    Fold::Unknown
                }
            }
            (ScalarKind::Float { bits: 64 }, ConstKey::Int(x)) => {
                Fold::Const(ConstKey::Float((x as f64).to_bits()))
            }
            (ScalarKind::Float { bits: 32 }, ConstKey::Int(x)) => {
                Fold::Const(ConstKey::Float(((x as f32) as f64).to_bits()))
            }
            (ScalarKind::Float { bits: 64 }, ConstKey::Float(f)) => Fold::Const(ConstKey::Float(f)),
            (ScalarKind::Float { bits: 32 }, ConstKey::Float(f)) => {
                Fold::Const(ConstKey::Float(((f64::from_bits(f) as f32) as f64).to_bits()))
            }
            (ScalarKind::Bool, ConstKey::Bool(b)) => Fold::Const(ConstKey::Bool(b)),
            (ScalarKind::Bool, ConstKey::Int(x)) => Fold::Const(ConstKey::Bool(x != 0)),
            (ScalarKind::Reference, ConstKey::Null) => Fold::Const(ConstKey::Null),
            (ScalarKind::Reference, ConstKey::Str(s)) => Fold::Const(ConstKey::Str(s)),
            _ => Fold::Unknown,
        }
    }
}

// ── SCCP ─────────────────────────────────────────────────────────────────────

/// Result of SCCP on one function.
#[derive(Clone, Debug)]
pub struct SccpResult {
    /// Lattice value per SSA value (`ValueId.0`); `Bottom` for values never defined on an
    /// executable path.
    pub values: Vec<Flat<ConstKey>>,
    /// Blocks reachable along executable edges.
    pub executable_blocks: Vec<bool>,
    /// Per `IrCfg` edge.
    pub executable_edges: Vec<bool>,
    /// `(block, edge)`: branches and switches whose constant operand leaves exactly one
    /// executable successor.
    pub decided: Vec<(u32, u32)>,
    /// Values that are `⊤` only because a width or overflow rule was not known.
    pub type_dependent: BitSet,
}

impl SccpResult {
    /// The constant value of `v`, if SCCP proved one.
    pub fn constant(&self, v: ValueId) -> Option<ConstKey> {
        match self.values.get(v.0 as usize) {
            Some(Flat::Value(c)) => Some(*c),
            _ => None,
        }
    }

    /// Reachable blocks SCCP proved dead.
    pub fn dead_blocks(&self, cfg: &IrCfg) -> Vec<u32> {
        cfg.rpo
            .iter()
            .copied()
            .filter(|&b| !self.executable_blocks[b as usize])
            .collect()
    }
}

struct Sccp<'a> {
    func: &'a FuncIr,
    cfg: &'a IrCfg,
    du: ValueDefUse,
    folder: ConstantFolder<'a>,
    values: Vec<Flat<ConstKey>>,
    type_dependent: BitSet,
    exec_edges: Vec<bool>,
    exec_blocks: Vec<bool>,
    flow: Vec<u32>,
    ssa: Vec<ValueId>,
}

impl Sccp<'_> {
    fn operand<O: OperandView + ?Sized>(&self, op: &O) -> Flat<ConstKey> {
        if let Some(c) = op.constant() {
            return Flat::Value(c);
        }
        match op.value() {
            Some(v) => self.values.get(v.0 as usize).copied().unwrap_or(Flat::Top),
            None => Flat::Bottom,
        }
    }

    fn raise(&mut self, dst: ValueId, value: Flat<ConstKey>) {
        if let Some(slot) = self.values.get_mut(dst.0 as usize) {
            if slot.join(&value) {
                self.ssa.push(dst);
            }
        }
    }

    fn raise_fold(&mut self, dst: ValueId, fold: Fold) {
        match fold {
            Fold::Const(c) => self.raise(dst, Flat::Value(c)),
            Fold::Unknown => self.raise(dst, Flat::Top),
            Fold::TypeDependent => {
                self.type_dependent.insert(dst.0 as usize);
                self.raise(dst, Flat::Top);
            }
        }
    }

    fn mark_edge(&mut self, e: u32) {
        if !self.exec_edges[e as usize] {
            self.flow.push(e);
        }
    }

    fn mark_kind(&mut self, block: u32, pred: impl Fn(EdgeKind) -> bool) {
        let edges: Vec<u32> = self.cfg.succ_edges[block as usize]
            .iter()
            .copied()
            .filter(|&e| pred(self.cfg.edges[e as usize].kind))
            .collect();
        for e in edges {
            self.mark_edge(e);
        }
    }

    fn visit_block(&mut self, block: u32) {
        let n = self.func.blocks[block as usize].insts.len();
        for i in 0..n {
            self.eval_inst(block, i);
        }
        self.mark_kind(block, |k| k.is_unwind());
        self.eval_term(block);
    }

    fn visit_phis(&mut self, block: u32) {
        let n = self.func.blocks[block as usize].insts.len();
        for i in 0..n {
            if matches!(self.func.blocks[block as usize].insts[i], Inst::Phi { .. }) {
                self.eval_inst(block, i);
            }
        }
    }

    fn eval_inst(&mut self, block: u32, index: usize) {
        let func = self.func;
        let inst = &func.blocks[block as usize].insts[index];
        match inst {
            Inst::Phi { dst, incoming } => {
                let mut preds: Vec<u32> = Vec::new();
                for &e in &self.cfg.pred_edges[block as usize] {
                    let from = self.cfg.edges[e as usize].from;
                    if self.exec_edges[e as usize] && !preds.contains(&from) {
                        preds.push(from);
                    }
                }
                let mut acc = Flat::Bottom;
                for p in preds {
                    for op in phi_operands_from(incoming, self.cfg.block_id(p)) {
                        acc.join(&self.operand(op));
                    }
                }
                self.raise(*dst, acc);
            }
            Inst::Assign { dst, src } => {
                let v = self.operand(src);
                self.raise(*dst, v);
            }
            Inst::Unary { dst, op, src } => {
                let fold = match (UnaryOp::from_ir(op), self.operand(src)) {
                    (_, Flat::Bottom) => return,
                    (Some(u), Flat::Value(c)) => self.folder.unary(*dst, u, c),
                    _ => Fold::Unknown,
                };
                self.raise_fold(*dst, fold);
            }
            Inst::Binary { dst, op, lhs, rhs } => {
                let (a, b) = (self.operand(lhs), self.operand(rhs));
                let arith = ArithOp::from_ir(op);
                // Absorbing elements decide the result whatever the other operand is.
                if let Some(op) = arith {
                    if let Some(c) = absorbing(op, &a).or_else(|| absorbing(op, &b)) {
                        self.raise(*dst, Flat::Value(c));
                        return;
                    }
                }
                let fold = match (arith, a, b) {
                    (_, Flat::Bottom, _) | (_, _, Flat::Bottom) => return,
                    (Some(op), Flat::Value(x), Flat::Value(y)) => self.folder.binary(*dst, op, x, y),
                    _ => Fold::Unknown,
                };
                self.raise_fold(*dst, fold);
            }
            Inst::Compare { dst, pred, lhs, rhs } => {
                let fold = match (CmpOp::from_ir(pred), self.operand(lhs), self.operand(rhs)) {
                    (_, Flat::Bottom, _) | (_, _, Flat::Bottom) => return,
                    (Some(op), Flat::Value(x), Flat::Value(y)) => self.folder.compare(op, x, y),
                    _ => Fold::Unknown,
                };
                self.raise_fold(*dst, fold);
            }
            Inst::Cast { dst, src, ty } => {
                let fold = match self.operand(src) {
                    Flat::Bottom => return,
                    Flat::Value(c) => self.folder.cast(*ty, c),
                    Flat::Top => Fold::Unknown,
                };
                self.raise_fold(*dst, fold);
            }
            Inst::Load { dst, .. }
            | Inst::Alloc { dst, .. }
            | Inst::MakeClosure { dst, .. }
            | Inst::TypeTest { dst, .. } => self.raise(*dst, Flat::Top),
            Inst::Call { dst: Some(d), .. } | Inst::Intrinsic { dst: Some(d), .. } => {
                self.raise(*d, Flat::Top)
            }
            Inst::Call { dst: None, .. }
            | Inst::Intrinsic { dst: None, .. }
            | Inst::Store { .. }
            | Inst::Release { .. } => {}
        }
    }

    fn eval_term(&mut self, block: u32) {
        let func = self.func;
        match &func.blocks[block as usize].term {
            Terminator::Jump(_) => self.mark_kind(block, |k| k == EdgeKind::Jump),
            Terminator::Branch { cond, .. } => match self.operand(cond) {
                Flat::Bottom => {}
                Flat::Value(ConstKey::Bool(true)) => self.mark_kind(block, |k| k == EdgeKind::True),
                Flat::Value(ConstKey::Bool(false)) => {
                    self.mark_kind(block, |k| k == EdgeKind::False)
                }
                _ => self.mark_kind(block, |k| matches!(k, EdgeKind::True | EdgeKind::False)),
            },
            Terminator::Switch {
                scrutinee, cases, ..
            } => match self.operand(scrutinee) {
                Flat::Bottom => {}
                Flat::Value(c) => {
                    match cases.iter().position(|(v, _)| ConstKey::from_ir(v) == c) {
                        Some(k) => {
                            let k = k as u32;
                            self.mark_kind(block, move |kind| kind == EdgeKind::Case { index: k })
                        }
                        None => self.mark_kind(block, |k| k == EdgeKind::Default),
                    }
                }
                Flat::Top => self.mark_kind(block, |k| {
                    matches!(k, EdgeKind::Case { .. } | EdgeKind::Default)
                }),
            },
            Terminator::Return(_) | Terminator::Throw(_) | Terminator::Unreachable => {}
        }
    }

    fn run(&mut self) {
        if self.cfg.is_empty() {
            return;
        }
        let entry = self.cfg.entry;
        self.exec_blocks[entry as usize] = true;
        self.visit_block(entry);
        loop {
            if let Some(e) = self.flow.pop() {
                if self.exec_edges[e as usize] {
                    continue;
                }
                self.exec_edges[e as usize] = true;
                let to = self.cfg.edges[e as usize].to;
                if !self.exec_blocks[to as usize] {
                    self.exec_blocks[to as usize] = true;
                    self.visit_block(to);
                } else {
                    self.visit_phis(to);
                }
                continue;
            }
            if let Some(v) = self.ssa.pop() {
                let sites: Vec<UseSite> = self.du.uses_of(v).to_vec();
                for site in sites {
                    match site {
                        UseSite::Inst { block, index } if self.exec_blocks[block as usize] => {
                            self.eval_inst(block, index as usize)
                        }
                        UseSite::Terminator { block } if self.exec_blocks[block as usize] => {
                            self.eval_term(block)
                        }
                        _ => {}
                    }
                }
                continue;
            }
            break;
        }
    }
}

/// The constant result of `op` when one operand is an absorbing element: `false ∧ x`,
/// `true ∨ x`, `0 & x`, `0 · x` (integers).
fn absorbing(op: ArithOp, v: &Flat<ConstKey>) -> Option<ConstKey> {
    match (op, v) {
        (ArithOp::And, Flat::Value(ConstKey::Bool(false))) => Some(ConstKey::Bool(false)),
        (ArithOp::Or, Flat::Value(ConstKey::Bool(true))) => Some(ConstKey::Bool(true)),
        (ArithOp::And, Flat::Value(ConstKey::Int(0))) | (ArithOp::Mul, Flat::Value(ConstKey::Int(0))) => {
            Some(ConstKey::Int(0))
        }
        _ => None,
    }
}

/// Runs SCCP on `func`. `params` gives known argument values by parameter position (for
/// example from an interprocedural constant analysis); parameters without one are `⊤`.
pub fn run(
    func: &FuncIr,
    cfg: &IrCfg,
    types: &dyn TypeFacts,
    params: Option<&[Flat<ConstKey>]>,
) -> SccpResult {
    let n_values = value_count(func);
    let mut values = vec![Flat::Bottom; n_values];
    for (i, p) in func.params.iter().enumerate() {
        let v = params
            .and_then(|ps| ps.get(i).copied())
            .unwrap_or(Flat::Top);
        if let Some(slot) = values.get_mut(p.0 as usize) {
            *slot = v;
        }
    }
    let mut sccp = Sccp {
        func,
        cfg,
        du: ValueDefUse::build(func),
        folder: ConstantFolder { func, types },
        values,
        type_dependent: BitSet::new(),
        exec_edges: vec![false; cfg.edges.len()],
        exec_blocks: vec![false; cfg.len()],
        flow: Vec::new(),
        ssa: Vec::new(),
    };
    sccp.run();

    let mut decided = Vec::new();
    for &b in &cfg.rpo {
        if !sccp.exec_blocks[b as usize] {
            continue;
        }
        if !matches!(
            func.blocks[b as usize].term,
            Terminator::Branch { .. } | Terminator::Switch { .. }
        ) {
            continue;
        }
        let normal: Vec<u32> = cfg.succ_edges[b as usize]
            .iter()
            .copied()
            .filter(|&e| !cfg.edges[e as usize].kind.is_unwind())
            .collect();
        let live: Vec<u32> = normal
            .iter()
            .copied()
            .filter(|&e| sccp.exec_edges[e as usize])
            .collect();
        if normal.len() >= 2 && live.len() == 1 {
            decided.push((b, live[0]));
        }
    }
    SccpResult {
        values: sccp.values,
        executable_blocks: sccp.exec_blocks,
        executable_edges: sccp.exec_edges,
        decided,
        type_dependent: sccp.type_dependent,
    }
}
