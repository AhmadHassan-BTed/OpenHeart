//! Monotone-framework solvers over the IR control-flow graph (Kildall 1973; Kam & Ullman
//! 1977; Cousot & Cousot 1977 for widening and narrowing).
//!
//! Three layers:
//!
//! 1. **IR binding.** Operand and constant access ([`OperandView`], [`ConstKey`]), predicate
//!    evaluation ([`pred_holds`]), the def/use walkers, copy resolution and [`TypeFacts`] with
//!    its IR implementation [`IrTypeFacts`]. Every analysis reads the IR through these.
//! 2. **CFG view.** [`IrCfg`] turns a [`FuncIr`] into dense block positions with typed edges:
//!    `Jump`, `True`/`False` of a `Branch`, `Case`/`Default` of a `Switch`, `Throw` to the
//!    block's landing block, and `Unwind` edges that leave a block at the `Call` naming the
//!    landing block (the IR places such a call last in its block). It also holds
//!    the reverse post-order, the loop heads (targets of RPO-retreating edges, a feedback
//!    vertex set for every cycle reachable from entry) and the dominator tree.
//! 3. **Solvers.** [`solve_forward`] and [`solve_backward`] run a priority worklist in reverse
//!    post-order (backward: post-order), join at merge points, widen at loop heads once a head
//!    has been visited `widen_delay` times, and optionally run descending narrowing passes.
//!    Edge transfer functions let an analysis refine the state per edge (branch conditions,
//!    phi bindings) or declare an edge infeasible.
//!
//! **Program points.** `block_in[b]` is the state before the first instruction of `b` and
//! `block_out[b]` the state after its terminator, in both directions. Per-instruction states
//! are recovered with [`replay_forward`] / [`replay_backward`], which run the same block
//! transfer the solver ran.
//!
//! **Exceptional flow.** A `Call` with `unwind: Some(h)` contributes to `h` the state reached
//! *before* the call, passed through [`ForwardTransfer::transfer_unwind`] (the call's side
//! effects may have happened; its result was never produced). Backward, the handler's
//! in-state is joined into the state after the call through
//! [`BackwardTransfer::transfer_unwind`].
//!
//! **Termination.** Lattices of finite height terminate without widening; infinite-height
//! lattices terminate through `widen`. A visit budget guards against non-monotone transfer
//! functions: when it is exhausted the result says `converged == false` and its states are not
//! a fixpoint.

use std::collections::{BTreeSet, HashMap};
use std::fmt;

use crate::analysis::lattice::Lattice;
use crate::cfg::analysis::dominators::{compute_idom_cooper, DominatorTree, UNDEFINED_IDOM};
use crate::cfg::analysis::rpo::reverse_postorder_from;
use crate::ir::func::{BlockIr, FuncIr};
use crate::ir::inst::{Callee, ComparePred, Inst, Terminator};
use crate::ir::types::{BlockId, IrType, TypeId, TypeTable, ValueId};
use crate::ir::value::{ConstVal, Operand, Place};

// ── IR binding: operands and constants ───────────────────────────────────────

/// Uniform access to instruction operands, whether an instruction stores a full
/// [`Operand`] or a plain [`ValueId`].
pub trait OperandView {
    /// The SSA value read by the operand, if any.
    fn value(&self) -> Option<ValueId>;
    /// The constant denoted by the operand, if any.
    fn constant(&self) -> Option<ConstKey>;
    /// True for `Undef`.
    fn is_undef(&self) -> bool;
}

impl OperandView for Operand {
    #[inline]
    fn value(&self) -> Option<ValueId> {
        match self {
            Operand::Value(v) => Some(*v),
            _ => None,
        }
    }

    #[inline]
    fn constant(&self) -> Option<ConstKey> {
        match self {
            Operand::Const(c) => Some(ConstKey::from_ir(c)),
            _ => None,
        }
    }

    #[inline]
    fn is_undef(&self) -> bool {
        matches!(self, Operand::Undef)
    }
}

impl OperandView for ValueId {
    #[inline]
    fn value(&self) -> Option<ValueId> {
        Some(*self)
    }

    #[inline]
    fn constant(&self) -> Option<ConstKey> {
        None
    }

    #[inline]
    fn is_undef(&self) -> bool {
        false
    }
}

/// A hashable, totally ordered copy of an IR constant (`ConstVal`). Floats are kept as their
/// IEEE-754 binary64 bit pattern, exactly as the IR stores them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ConstKey {
    Bool(bool),
    Int(i128),
    Float(u64),
    /// Interned string literal id.
    Str(u32),
    Null,
}

impl ConstKey {
    pub fn from_ir(c: &ConstVal) -> ConstKey {
        match *c {
            ConstVal::Bool(b) => ConstKey::Bool(b),
            ConstVal::Int(i) => ConstKey::Int(i),
            ConstVal::Float(bits) => ConstKey::Float(bits),
            ConstVal::Str(s) => ConstKey::Str(s),
            ConstVal::Null => ConstKey::Null,
        }
    }

    pub fn to_ir(self) -> ConstVal {
        match self {
            ConstKey::Bool(b) => ConstVal::Bool(b),
            ConstKey::Int(i) => ConstVal::Int(i),
            ConstKey::Float(bits) => ConstVal::Float(bits),
            ConstKey::Str(s) => ConstVal::Str(s),
            ConstKey::Null => ConstVal::Null,
        }
    }

    #[inline]
    pub fn as_int(self) -> Option<i128> {
        match self {
            ConstKey::Int(i) => Some(i),
            _ => None,
        }
    }

    #[inline]
    pub fn as_bool(self) -> Option<bool> {
        match self {
            ConstKey::Bool(b) => Some(b),
            _ => None,
        }
    }

    #[inline]
    pub fn as_f64(self) -> Option<f64> {
        match self {
            ConstKey::Float(bits) => Some(f64::from_bits(bits)),
            _ => None,
        }
    }

    /// True for constants that denote a reference (`null` or a string literal).
    #[inline]
    pub fn is_reference(self) -> bool {
        matches!(self, ConstKey::Null | ConstKey::Str(_))
    }
}

impl fmt::Display for ConstKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConstKey::Bool(b) => write!(f, "{}", b),
            ConstKey::Int(i) => write!(f, "{}", i),
            ConstKey::Float(bits) => write!(f, "{}", f64::from_bits(*bits)),
            ConstKey::Str(s) => write!(f, "str#{}", s),
            ConstKey::Null => f.write_str("null"),
        }
    }
}

// ── IR binding: predicates ───────────────────────────────────────────────────

/// Evaluates a comparison predicate on an ordering result; `None` (unordered operands, a
/// NaN) makes every predicate except `Ne` false.
pub fn pred_holds(pred: ComparePred, ord: Option<std::cmp::Ordering>) -> bool {
    use std::cmp::Ordering::*;
    match (pred, ord) {
        (ComparePred::Ne, None) => true,
        (_, None) => false,
        (ComparePred::Eq, Some(o)) => o == Equal,
        (ComparePred::Ne, Some(o)) => o != Equal,
        (ComparePred::Lt, Some(o)) => o == Less,
        (ComparePred::Le, Some(o)) => o != Greater,
        (ComparePred::Gt, Some(o)) => o == Greater,
        (ComparePred::Ge, Some(o)) => o != Less,
    }
}

/// True for `Eq` and `Ne`.
#[inline]
pub fn is_equality(pred: ComparePred) -> bool {
    matches!(pred, ComparePred::Eq | ComparePred::Ne)
}

// ── IR binding: type facts ───────────────────────────────────────────────────

/// What the analyses need to know about a type: its scalar class and, for numbers, the
/// width and signedness that decide wrap-around.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ScalarKind {
    Bool,
    Int { bits: u16, signed: bool },
    Float { bits: u16 },
    /// A character type; its width is a language property not carried by the IR type.
    Char,
    /// Values that denote memory: pointers, references, strings, named types, arrays,
    /// function values (the IR's reference-like types).
    Reference,
    /// Aggregates held by value.
    Aggregate,
    Void,
    /// Type parameters and types the frontend could not resolve.
    Unknown,
}

impl ScalarKind {
    /// The `(bits, signed)` pair of a fixed-width integer kind.
    pub fn int_width(self) -> Option<(u16, bool)> {
        match self {
            ScalarKind::Int { bits, signed } => Some((bits, signed)),
            _ => None,
        }
    }
}

/// Type knowledge supplied by the IR's type table and the frontend that lowered the code.
pub trait TypeFacts {
    /// The scalar class of an interned IR type.
    fn resolve(&self, ty: TypeId) -> ScalarKind;

    /// The declared or inferred type of an SSA value of `func`, when known.
    fn value_type(&self, func: &FuncIr, value: ValueId) -> Option<TypeId>;

    /// The scalar class of an SSA value (`Unknown` when its type is not known).
    fn value_kind(&self, func: &FuncIr, value: ValueId) -> ScalarKind {
        self.value_type(func, value)
            .map_or(ScalarKind::Unknown, |t| self.resolve(t))
    }

    /// True when the source language defines signed overflow and narrowing signed
    /// conversion as two's-complement wrap-around; false when it leaves them undefined or
    /// implementation-defined (the analyses then give no value to an overflowing result).
    fn signed_overflow_wraps(&self) -> bool {
        false
    }
}

/// No type knowledge: every type resolves to `Unknown`. Analyses then fall back to
/// width-independent reasoning and report the values whose results depend on a width.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoTypeFacts;

impl TypeFacts for NoTypeFacts {
    fn resolve(&self, _ty: TypeId) -> ScalarKind {
        ScalarKind::Unknown
    }

    fn value_type(&self, _func: &FuncIr, _value: ValueId) -> Option<TypeId> {
        None
    }
}

/// Type facts read from the IR: the module's type table and the per-value types the lowering
/// records in `FuncIr::values`. `signed_overflow_wraps` is the source language's rule, as
/// its frontend reports it.
#[derive(Clone, Copy, Debug)]
pub struct IrTypeFacts<'a> {
    pub types: &'a TypeTable,
    pub signed_overflow_wraps: bool,
}

impl TypeFacts for IrTypeFacts<'_> {
    fn resolve(&self, ty: TypeId) -> ScalarKind {
        match self.types.get(ty) {
            IrType::Void => ScalarKind::Void,
            IrType::Bool => ScalarKind::Bool,
            IrType::Int { bits, signed } => ScalarKind::Int {
                bits: *bits,
                signed: *signed,
            },
            IrType::Float { bits } => ScalarKind::Float { bits: *bits },
            IrType::Char => ScalarKind::Char,
            IrType::Str
            | IrType::Ptr(_)
            | IrType::Ref { .. }
            | IrType::Named(_)
            | IrType::Array { .. }
            | IrType::Func { .. }
            | IrType::Generic { .. } => ScalarKind::Reference,
            IrType::TypeParam(_) | IrType::Unknown => ScalarKind::Unknown,
        }
    }

    fn value_type(&self, func: &FuncIr, value: ValueId) -> Option<TypeId> {
        let ty = func.value_type(value);
        if self.types.is_unknown(ty) {
            None
        } else {
            Some(ty)
        }
    }

    fn signed_overflow_wraps(&self) -> bool {
        self.signed_overflow_wraps
    }
}

// ── IR binding: definitions and uses ─────────────────────────────────────────

/// The SSA value an instruction defines.
#[inline]
pub fn inst_def(inst: &Inst) -> Option<ValueId> {
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

#[inline]
fn visit_operand<O: OperandView + ?Sized>(o: &O, f: &mut impl FnMut(ValueId)) {
    if let Some(v) = o.value() {
        f(v);
    }
}

/// Calls `f` for every SSA value a place reads to form its address.
pub fn for_each_place_use(place: &Place, mut f: impl FnMut(ValueId)) {
    match place {
        Place::Local(v) | Place::Deref(v) => f(*v),
        Place::Field { base, .. } => f(*base),
        Place::Index { base, index } => {
            f(*base);
            visit_operand(index, &mut f);
        }
        Place::Global(_) | Place::Static { .. } => {}
    }
}

/// Calls `f` for every SSA value read by the callee expression.
pub fn for_each_callee_use(callee: &Callee, mut f: impl FnMut(ValueId)) {
    match callee {
        Callee::Virtual { receiver, .. } | Callee::Interface { receiver, .. } => f(*receiver),
        Callee::Indirect(v) => f(*v),
        Callee::Static(_) | Callee::Symbol(_) | Callee::Unresolved(_) => {}
    }
}

/// Calls `f` for every SSA value an instruction reads (phi operands included).
pub fn for_each_inst_use(inst: &Inst, mut f: impl FnMut(ValueId)) {
    match inst {
        Inst::Assign { src, .. } => visit_operand(src, &mut f),
        Inst::Unary { src, .. } => visit_operand(src, &mut f),
        Inst::Cast { src, .. } => visit_operand(src, &mut f),
        Inst::TypeTest { src, .. } => visit_operand(src, &mut f),
        Inst::Binary { lhs, rhs, .. } => {
            visit_operand(lhs, &mut f);
            visit_operand(rhs, &mut f);
        }
        Inst::Compare { lhs, rhs, .. } => {
            visit_operand(lhs, &mut f);
            visit_operand(rhs, &mut f);
        }
        Inst::Load { place, .. } | Inst::Release { place } => for_each_place_use(place, &mut f),
        Inst::Store { place, value } => {
            for_each_place_use(place, &mut f);
            visit_operand(value, &mut f);
        }
        Inst::Alloc { .. } => {}
        Inst::Call { callee, args, .. } => {
            for_each_callee_use(callee, &mut f);
            for a in args.iter() {
                visit_operand(a, &mut f);
            }
        }
        Inst::Phi { incoming, .. } => {
            for (_, op) in incoming.iter() {
                visit_operand(op, &mut f);
            }
        }
        Inst::MakeClosure { captures, .. } => {
            for c in captures.iter() {
                visit_operand(c, &mut f);
            }
        }
        Inst::Intrinsic { args, .. } => {
            for a in args.iter() {
                visit_operand(a, &mut f);
            }
        }
    }
}

/// Calls `f` for every SSA value a terminator reads.
pub fn for_each_term_use(term: &Terminator, mut f: impl FnMut(ValueId)) {
    match term {
        Terminator::Branch { cond, .. } => visit_operand(cond, &mut f),
        Terminator::Switch { scrutinee, .. } => visit_operand(scrutinee, &mut f),
        Terminator::Return(Some(op)) | Terminator::Throw(op) => visit_operand(op, &mut f),
        Terminator::Return(None) | Terminator::Jump(_) | Terminator::Unreachable => {}
    }
}

/// One more than the largest SSA value id defined or used in `func` (the size of dense
/// per-value tables).
pub fn value_count(func: &FuncIr) -> usize {
    let mut max: Option<u32> = None;
    let mut see = |v: ValueId| {
        max = Some(max.map_or(v.0, |m| m.max(v.0)));
    };
    for &p in &func.params {
        see(p);
    }
    for block in &func.blocks {
        for inst in &block.insts {
            if let Some(d) = inst_def(inst) {
                see(d);
            }
            for_each_inst_use(inst, &mut see);
        }
        for_each_term_use(&block.term, &mut see);
    }
    max.map_or(0, |m| m as usize + 1).max(func.values.len())
}

/// The operands a phi receives from predecessor `pred` (normally exactly one).
pub fn phi_operands_from<'a>(
    incoming: &'a [(BlockId, Operand)],
    pred: BlockId,
) -> impl Iterator<Item = &'a Operand> + 'a {
    incoming
        .iter()
        .filter(move |(b, _)| *b == pred)
        .map(|(_, op)| op)
}

/// Location of a value's definition.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DefPoint {
    /// The `index`-th parameter of the function.
    Param(usize),
    /// Instruction `index` of block position `block`.
    Inst { block: u32, index: u32 },
}

/// The definition point of every SSA value, indexed by `ValueId.0`.
pub fn def_points(func: &FuncIr) -> Vec<Option<DefPoint>> {
    let mut defs = vec![None; value_count(func)];
    for (i, &p) in func.params.iter().enumerate() {
        if let Some(slot) = defs.get_mut(p.0 as usize) {
            *slot = Some(DefPoint::Param(i));
        }
    }
    for (b, block) in func.blocks.iter().enumerate() {
        for (i, inst) in block.insts.iter().enumerate() {
            if let Some(d) = inst_def(inst) {
                if let Some(slot) = defs.get_mut(d.0 as usize) {
                    *slot = Some(DefPoint::Inst {
                        block: b as u32,
                        index: i as u32,
                    });
                }
            }
        }
    }
    defs
}

/// The defining instruction of `v`, when it is an instruction.
pub fn defining_inst<'f>(func: &'f FuncIr, defs: &[Option<DefPoint>], v: ValueId) -> Option<&'f Inst> {
    match defs.get(v.0 as usize).copied().flatten()? {
        DefPoint::Inst { block, index } => func
            .blocks
            .get(block as usize)
            .and_then(|b| b.insts.get(index as usize)),
        DefPoint::Param(_) => None,
    }
}

/// Follows `Assign` copies from `op` to the operand that originally produced the value.
/// Stops at the first non-copy definition; a bounded walk protects against malformed
/// (cyclic) copy chains.
pub fn resolve_copies<O: OperandView + ?Sized>(
    func: &FuncIr,
    defs: &[Option<DefPoint>],
    op: &O,
) -> ResolvedOperand {
    if let Some(c) = op.constant() {
        return ResolvedOperand::Const(c);
    }
    let Some(mut v) = op.value() else {
        return ResolvedOperand::Undef;
    };
    for _ in 0..defs.len().max(1) {
        match defining_inst(func, defs, v) {
            Some(Inst::Assign { src, .. }) => {
                if let Some(c) = src.constant() {
                    return ResolvedOperand::Const(c);
                }
                match src.value() {
                    Some(next) if next != v => v = next,
                    Some(_) => break,
                    None => return ResolvedOperand::Undef,
                }
            }
            _ => break,
        }
    }
    ResolvedOperand::Value(v)
}

/// An operand after copy resolution.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ResolvedOperand {
    Value(ValueId),
    Const(ConstKey),
    Undef,
}

// ── CFG view ─────────────────────────────────────────────────────────────────

/// Kind of a CFG edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EdgeKind {
    Jump,
    /// `Branch` edge taken when the condition holds.
    True,
    /// `Branch` edge taken when the condition does not hold.
    False,
    /// The `index`-th case of a `Switch`.
    Case { index: u32 },
    /// The default edge of a `Switch`.
    Default,
    /// A `Throw` terminator transferring to the block's landing block.
    Throw,
    /// Exceptional edge leaving at instruction `inst` (a `Call` with `unwind`).
    Unwind { inst: u32 },
}

impl EdgeKind {
    /// True for the edge of a call that throws (it leaves before the block's end).
    #[inline]
    pub fn is_unwind(self) -> bool {
        matches!(self, EdgeKind::Unwind { .. })
    }

    /// True for every edge taken by an exception.
    #[inline]
    pub fn is_exceptional(self) -> bool {
        matches!(self, EdgeKind::Unwind { .. } | EdgeKind::Throw)
    }
}

/// A CFG edge between block positions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CfgEdge {
    pub from: u32,
    pub to: u32,
    pub kind: EdgeKind,
}

/// The control-flow graph of one IR function over dense block positions (the index of the
/// block in `FuncIr::blocks`).
#[derive(Clone, Debug)]
pub struct IrCfg {
    /// `ids[p]` is the IR id of the block at position `p`.
    pub ids: Vec<BlockId>,
    /// Position of the entry block.
    pub entry: u32,
    pub edges: Vec<CfgEdge>,
    /// Outgoing edge indices per block, in terminator order after the unwind edges.
    pub succ_edges: Vec<Vec<u32>>,
    /// Incoming edge indices per block.
    pub pred_edges: Vec<Vec<u32>>,
    /// Distinct successor positions per block.
    pub succs: Vec<Vec<u32>>,
    /// Distinct predecessor positions per block.
    pub preds: Vec<Vec<u32>>,
    /// Reverse post-order of the blocks reachable from entry.
    pub rpo: Vec<u32>,
    /// `rpo_index[p]` (`u32::MAX` when unreachable).
    pub rpo_index: Vec<u32>,
    /// Targets of RPO-retreating edges: the widening points.
    pub loop_heads: Vec<bool>,
    /// Blocks without successors: `Return`, `Unreachable`, and `Throw` without a landing
    /// block.
    pub exits: Vec<u32>,
    /// Immediate dominators by position (`UNDEFINED_IDOM` when unreachable).
    pub idom: Vec<u32>,
    pub dom_tree: DominatorTree,
    /// Edges whose target id names no block of the function: `(from position, target id)`.
    pub dangling: Vec<(u32, BlockId)>,
    position: HashMap<BlockId, u32>,
}

impl IrCfg {
    pub fn build(func: &FuncIr) -> IrCfg {
        let n = func.blocks.len();
        let mut ids = Vec::with_capacity(n);
        let mut position: HashMap<BlockId, u32> = HashMap::with_capacity(n);
        for (p, block) in func.blocks.iter().enumerate() {
            ids.push(block.id);
            position.entry(block.id).or_insert(p as u32);
        }

        let mut builder = EdgeBuilder {
            edges: Vec::new(),
            succ_edges: vec![Vec::new(); n],
            pred_edges: vec![Vec::new(); n],
            dangling: Vec::new(),
            position: &position,
        };
        let mut exits = Vec::new();
        for (p, block) in func.blocks.iter().enumerate() {
            let p = p as u32;
            for (i, inst) in block.insts.iter().enumerate() {
                if let Inst::Call {
                    unwind: Some(handler),
                    ..
                } = inst
                {
                    builder.push(p, *handler, EdgeKind::Unwind { inst: i as u32 });
                }
            }
            match &block.term {
                Terminator::Jump(t) => builder.push(p, *t, EdgeKind::Jump),
                Terminator::Branch { then_b, else_b, .. } => {
                    builder.push(p, *then_b, EdgeKind::True);
                    builder.push(p, *else_b, EdgeKind::False);
                }
                Terminator::Switch { cases, default, .. } => {
                    for (k, (_, target)) in cases.iter().enumerate() {
                        builder.push(p, *target, EdgeKind::Case { index: k as u32 });
                    }
                    builder.push(p, *default, EdgeKind::Default);
                }
                Terminator::Throw(_) => match block.unwind {
                    Some(landing) => builder.push(p, landing, EdgeKind::Throw),
                    None => exits.push(p),
                },
                Terminator::Return(_) | Terminator::Unreachable => exits.push(p),
            }
        }
        let EdgeBuilder {
            edges,
            succ_edges,
            pred_edges,
            dangling,
            ..
        } = builder;

        let mut succs: Vec<Vec<u32>> = vec![Vec::new(); n];
        let mut preds: Vec<Vec<u32>> = vec![Vec::new(); n];
        for e in &edges {
            let (f, t) = (e.from as usize, e.to as usize);
            if !succs[f].contains(&e.to) {
                succs[f].push(e.to);
            }
            if !preds[t].contains(&e.from) {
                preds[t].push(e.from);
            }
        }

        let entry = position.get(&func.entry).copied().unwrap_or(0);
        let rpo = if n == 0 {
            Vec::new()
        } else {
            reverse_postorder_from(n, &succs, entry)
        };
        let mut rpo_index = vec![u32::MAX; n];
        for (i, &b) in rpo.iter().enumerate() {
            rpo_index[b as usize] = i as u32;
        }
        let mut loop_heads = vec![false; n];
        for e in &edges {
            let (ri_from, ri_to) = (rpo_index[e.from as usize], rpo_index[e.to as usize]);
            if ri_from != u32::MAX && ri_to != u32::MAX && ri_to <= ri_from {
                loop_heads[e.to as usize] = true;
            }
        }
        let idom = if n == 0 {
            Vec::new()
        } else {
            compute_idom_cooper(n, &preds, &rpo)
        };
        let dom_tree = DominatorTree::new(&idom, entry);

        IrCfg {
            ids,
            entry,
            edges,
            succ_edges,
            pred_edges,
            succs,
            preds,
            rpo,
            rpo_index,
            loop_heads,
            exits,
            idom,
            dom_tree,
            dangling,
            position,
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// Position of the block with IR id `id`.
    #[inline]
    pub fn position(&self, id: BlockId) -> Option<u32> {
        self.position.get(&id).copied()
    }

    /// IR id of the block at position `pos`.
    #[inline]
    pub fn block_id(&self, pos: u32) -> BlockId {
        self.ids[pos as usize]
    }

    #[inline]
    pub fn is_reachable(&self, pos: u32) -> bool {
        self.rpo_index
            .get(pos as usize)
            .is_some_and(|&i| i != u32::MAX)
    }

    #[inline]
    pub fn edge(&self, e: u32) -> &CfgEdge {
        &self.edges[e as usize]
    }

    /// The unwind edge leaving block `block` at instruction `inst`.
    pub fn unwind_edge(&self, block: u32, inst: u32) -> Option<u32> {
        self.succ_edges[block as usize]
            .iter()
            .copied()
            .find(|&e| self.edges[e as usize].kind == EdgeKind::Unwind { inst })
    }

    /// True when `a` dominates `b` (reflexive).
    #[inline]
    pub fn dominates(&self, a: u32, b: u32) -> bool {
        self.dom_tree.dominates(a, b)
    }

    /// An edge whose target dominates its source closes a natural loop.
    pub fn is_back_edge(&self, e: u32) -> bool {
        let edge = &self.edges[e as usize];
        self.is_reachable(edge.from) && self.dominates(edge.to, edge.from)
    }

    /// RPO-retreating edges whose target does not dominate their source: irreducible flow.
    pub fn irreducible_edges(&self) -> Vec<u32> {
        (0..self.edges.len() as u32)
            .filter(|&e| {
                let edge = &self.edges[e as usize];
                let (rf, rt) = (
                    self.rpo_index[edge.from as usize],
                    self.rpo_index[edge.to as usize],
                );
                rf != u32::MAX && rt != u32::MAX && rt <= rf && !self.is_back_edge(e)
            })
            .collect()
    }

    /// Immediate dominator of a reachable non-entry block.
    pub fn immediate_dominator(&self, pos: u32) -> Option<u32> {
        match self.idom.get(pos as usize).copied() {
            Some(d) if d != UNDEFINED_IDOM && d != pos => Some(d),
            _ => None,
        }
    }
}

struct EdgeBuilder<'a> {
    edges: Vec<CfgEdge>,
    succ_edges: Vec<Vec<u32>>,
    pred_edges: Vec<Vec<u32>>,
    dangling: Vec<(u32, BlockId)>,
    position: &'a HashMap<BlockId, u32>,
}

impl EdgeBuilder<'_> {
    fn push(&mut self, from: u32, target: BlockId, kind: EdgeKind) {
        match self.position.get(&target).copied() {
            Some(to) => {
                let e = self.edges.len() as u32;
                self.edges.push(CfgEdge { from, to, kind });
                self.succ_edges[from as usize].push(e);
                self.pred_edges[to as usize].push(e);
            }
            None => self.dangling.push((from, target)),
        }
    }
}

// ── Transfer functions ───────────────────────────────────────────────────────

/// An instruction's program point.
#[derive(Clone, Copy)]
pub struct InstPoint<'a> {
    pub func: &'a FuncIr,
    pub cfg: &'a IrCfg,
    /// Block position.
    pub block: u32,
    /// Instruction index within the block.
    pub index: u32,
}

/// Transfer functions of a forward analysis.
pub trait ForwardTransfer<L: Lattice> {
    /// The state on entry to the function.
    fn entry_state(&self, func: &FuncIr, cfg: &IrCfg) -> L;

    /// State before `inst` → state after it.
    fn transfer_inst(&self, at: InstPoint<'_>, inst: &Inst, state: &mut L);

    /// State before a `Call` with an unwind handler → state entering the handler.
    fn transfer_unwind(&self, _at: InstPoint<'_>, _inst: &Inst, _state: &mut L) {}

    /// State after the last instruction → state after the terminator.
    fn transfer_terminator(&self, _func: &FuncIr, _block: u32, _term: &Terminator, _state: &mut L) {}

    /// State leaving along `edge` → state arriving at its target. Returning `false` declares
    /// the edge infeasible under `state`: it then contributes nothing.
    fn transfer_edge(&self, _func: &FuncIr, _cfg: &IrCfg, _edge: &CfgEdge, _state: &mut L) -> bool {
        true
    }
}

/// Transfer functions of a backward analysis.
pub trait BackwardTransfer<L: Lattice> {
    /// The state after the terminator of an exit block (no successors).
    fn exit_state(&self, func: &FuncIr, cfg: &IrCfg, block: u32, term: &Terminator) -> L;

    /// State after `inst` → state before it.
    fn transfer_inst(&self, at: InstPoint<'_>, inst: &Inst, state: &mut L);

    /// The handler's in-state → its contribution to the state after the `Call`.
    fn transfer_unwind(&self, _at: InstPoint<'_>, _inst: &Inst, _state: &mut L) {}

    /// State after the terminator → state before it.
    fn transfer_terminator(&self, _func: &FuncIr, _block: u32, _term: &Terminator, _state: &mut L) {}

    /// The target's in-state → its contribution to the source's out-state; `false` drops the
    /// contribution.
    fn transfer_edge(&self, _func: &FuncIr, _cfg: &IrCfg, _edge: &CfgEdge, _state: &mut L) -> bool {
        true
    }
}

// ── Solver configuration and results ─────────────────────────────────────────

/// Solver tuning.
#[derive(Clone, Copy, Debug)]
pub struct SolverOptions {
    /// Number of visits of a loop head that use the join before widening takes over.
    pub widen_delay: u32,
    /// Maximum number of descending (narrowing) passes after the ascending fixpoint.
    pub narrowing_passes: u32,
    /// Visit budget per block; the total budget is this times the number of blocks.
    pub max_visits_per_block: u32,
}

impl Default for SolverOptions {
    fn default() -> Self {
        Self {
            widen_delay: 2,
            narrowing_passes: 2,
            max_visits_per_block: 4096,
        }
    }
}

/// Direction of an analysis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Forward,
    Backward,
}

/// Fixpoint of a data-flow problem.
#[derive(Clone, Debug)]
pub struct DataflowResult<L> {
    pub direction: Direction,
    /// State before the first instruction of each block.
    pub block_in: Vec<L>,
    /// State after the terminator of each block.
    pub block_out: Vec<L>,
    /// Blocks the solver evaluated (forward: some feasible path from entry reaches them;
    /// backward: reachable from entry).
    pub reached: Vec<bool>,
    /// Forward only: the state carried by each edge (`None`: infeasible or never taken).
    pub edge_state: Vec<Option<L>>,
    /// Total block evaluations, both phases.
    pub visits: u64,
    /// Number of narrowing passes that changed a state.
    pub narrowing_rounds: u32,
    /// False when the visit budget ran out; the states are then not a fixpoint.
    pub converged: bool,
}

impl<L: Lattice> DataflowResult<L> {
    fn empty(direction: Direction, n: usize, n_edges: usize) -> Self {
        Self {
            direction,
            block_in: vec![L::bottom(); n],
            block_out: vec![L::bottom(); n],
            reached: vec![false; n],
            edge_state: vec![None; n_edges],
            visits: 0,
            narrowing_rounds: 0,
            converged: true,
        }
    }

    /// The state before the first instruction of `block`.
    pub fn state_in(&self, block: u32) -> &L {
        &self.block_in[block as usize]
    }

    /// The state after the terminator of `block`.
    pub fn state_out(&self, block: u32) -> &L {
        &self.block_out[block as usize]
    }
}

/// Priority worklist over RPO indices.
struct RpoWorklist<'a> {
    cfg: &'a IrCfg,
    set: BTreeSet<u32>,
}

impl<'a> RpoWorklist<'a> {
    fn new(cfg: &'a IrCfg) -> Self {
        Self {
            cfg,
            set: BTreeSet::new(),
        }
    }

    fn push(&mut self, block: u32) {
        let ri = self.cfg.rpo_index[block as usize];
        if ri != u32::MAX {
            self.set.insert(ri);
        }
    }

    /// Earliest block in reverse post-order.
    fn pop_first(&mut self) -> Option<u32> {
        self.set.pop_first().map(|ri| self.cfg.rpo[ri as usize])
    }

    /// Latest block in reverse post-order.
    fn pop_last(&mut self) -> Option<u32> {
        self.set.pop_last().map(|ri| self.cfg.rpo[ri as usize])
    }
}

// ── Forward solver ───────────────────────────────────────────────────────────

/// Runs one block forward from `input`; stores the states of the block's unwind edges in
/// `edge_state` and returns the state after the terminator.
fn forward_block<L, T>(
    func: &FuncIr,
    cfg: &IrCfg,
    transfer: &T,
    block: u32,
    input: &L,
    edge_state: &mut [Option<L>],
    mut visit: impl FnMut(InstPoint<'_>, &Inst, &L),
) -> L
where
    L: Lattice,
    T: ForwardTransfer<L> + ?Sized,
{
    let body: &BlockIr = &func.blocks[block as usize];
    let mut state = input.clone();
    for (i, inst) in body.insts.iter().enumerate() {
        let at = InstPoint {
            func,
            cfg,
            block,
            index: i as u32,
        };
        visit(at, inst, &state);
        if let Some(e) = cfg.unwind_edge(block, i as u32) {
            let mut s = state.clone();
            transfer.transfer_unwind(at, inst, &mut s);
            let feasible = transfer.transfer_edge(func, cfg, &cfg.edges[e as usize], &mut s);
            edge_state[e as usize] = if feasible { Some(s) } else { None };
        }
        transfer.transfer_inst(at, inst, &mut state);
    }
    transfer.transfer_terminator(func, block, &body.term, &mut state);
    state
}

/// Computes the states of the non-unwind out-edges of `block` from its out-state.
fn forward_edges<L, T>(
    func: &FuncIr,
    cfg: &IrCfg,
    transfer: &T,
    block: u32,
    out: &L,
    edge_state: &mut [Option<L>],
) where
    L: Lattice,
    T: ForwardTransfer<L> + ?Sized,
{
    for &e in &cfg.succ_edges[block as usize] {
        let edge = &cfg.edges[e as usize];
        if edge.kind.is_unwind() {
            continue;
        }
        let mut s = out.clone();
        let feasible = transfer.transfer_edge(func, cfg, edge, &mut s);
        edge_state[e as usize] = if feasible { Some(s) } else { None };
    }
}

/// Solves a forward problem to its least fixpoint (with widening at loop heads), then runs
/// up to `options.narrowing_passes` descending passes.
pub fn solve_forward<L, T>(
    func: &FuncIr,
    cfg: &IrCfg,
    transfer: &T,
    options: &SolverOptions,
) -> DataflowResult<L>
where
    L: Lattice,
    T: ForwardTransfer<L> + ?Sized,
{
    let n = cfg.len();
    let mut r: DataflowResult<L> = DataflowResult::empty(Direction::Forward, n, cfg.edges.len());
    if n == 0 {
        return r;
    }
    let entry = cfg.entry;
    let boundary = transfer.entry_state(func, cfg);
    r.block_in[entry as usize] = boundary.clone();
    r.reached[entry as usize] = true;

    let budget = (options.max_visits_per_block as u64).saturating_mul(n as u64).max(1);
    let mut visits = vec![0u32; n];
    let mut worklist = RpoWorklist::new(cfg);
    worklist.push(entry);

    // ── Ascending phase ──
    while let Some(b) = worklist.pop_first() {
        r.visits += 1;
        if r.visits > budget {
            r.converged = false;
            break;
        }
        visits[b as usize] += 1;
        let input = r.block_in[b as usize].clone();
        let out = forward_block(func, cfg, transfer, b, &input, &mut r.edge_state, |_, _, _| {});
        forward_edges(func, cfg, transfer, b, &out, &mut r.edge_state);
        r.block_out[b as usize] = out;

        for &e in &cfg.succ_edges[b as usize] {
            let Some(s) = r.edge_state[e as usize].as_ref() else {
                continue;
            };
            let to = cfg.edges[e as usize].to as usize;
            let first = !r.reached[to];
            r.reached[to] = true;
            let changed = if cfg.loop_heads[to] && visits[to] >= options.widen_delay {
                r.block_in[to].widen(s)
            } else {
                r.block_in[to].join(s)
            };
            if changed || first {
                worklist.push(to as u32);
            }
        }
    }

    // ── Descending phase ──
    if r.converged {
        for _ in 0..options.narrowing_passes {
            let mut any = false;
            for &b in &cfg.rpo {
                let bi = b as usize;
                if !r.reached[bi] {
                    continue;
                }
                r.visits += 1;
                if r.visits > budget {
                    r.converged = false;
                    break;
                }
                let mut fresh = if b == entry {
                    boundary.clone()
                } else {
                    L::bottom()
                };
                for &e in &cfg.pred_edges[bi] {
                    if let Some(s) = r.edge_state[e as usize].as_ref() {
                        fresh.join(s);
                    }
                }
                let changed = if cfg.loop_heads[bi] {
                    r.block_in[bi].narrow(&fresh)
                } else if fresh != r.block_in[bi] && fresh.leq(&r.block_in[bi]) {
                    r.block_in[bi] = fresh;
                    true
                } else {
                    false
                };
                if changed {
                    any = true;
                    let input = r.block_in[bi].clone();
                    let out =
                        forward_block(func, cfg, transfer, b, &input, &mut r.edge_state, |_, _, _| {});
                    forward_edges(func, cfg, transfer, b, &out, &mut r.edge_state);
                    r.block_out[bi] = out;
                }
            }
            if !r.converged || !any {
                break;
            }
            r.narrowing_rounds += 1;
        }
    }
    r
}

/// Re-runs the transfer of `block` from its fixpoint in-state and calls `visit` with the
/// state *before* every instruction.
pub fn replay_forward<L, T>(
    func: &FuncIr,
    cfg: &IrCfg,
    transfer: &T,
    result: &DataflowResult<L>,
    block: u32,
    visit: impl FnMut(InstPoint<'_>, &Inst, &L),
) where
    L: Lattice,
    T: ForwardTransfer<L> + ?Sized,
{
    if !result.reached.get(block as usize).copied().unwrap_or(false) {
        return;
    }
    let mut scratch: Vec<Option<L>> = vec![None; cfg.edges.len()];
    let input = result.block_in[block as usize].clone();
    forward_block(func, cfg, transfer, block, &input, &mut scratch, visit);
}

// ── Backward solver ──────────────────────────────────────────────────────────

/// The out-state of `block`: exit boundary joined with the contributions of its normal
/// successors.
fn backward_out<L, T>(
    func: &FuncIr,
    cfg: &IrCfg,
    transfer: &T,
    block: u32,
    block_in: &[L],
    processed: &[bool],
) -> L
where
    L: Lattice,
    T: BackwardTransfer<L> + ?Sized,
{
    let body = &func.blocks[block as usize];
    let mut out = L::bottom();
    let mut has_normal_succ = false;
    for &e in &cfg.succ_edges[block as usize] {
        let edge = &cfg.edges[e as usize];
        if edge.kind.is_unwind() {
            continue;
        }
        has_normal_succ = true;
        if !processed[edge.to as usize] {
            continue;
        }
        let mut s = block_in[edge.to as usize].clone();
        if transfer.transfer_edge(func, cfg, edge, &mut s) {
            out.join(&s);
        }
    }
    if !has_normal_succ {
        out.join(&transfer.exit_state(func, cfg, block, &body.term));
    }
    out
}

/// Runs one block backward from its out-state; returns the in-state.
#[allow(clippy::too_many_arguments)]
fn backward_block<L, T>(
    func: &FuncIr,
    cfg: &IrCfg,
    transfer: &T,
    block: u32,
    out: &L,
    block_in: &[L],
    processed: &[bool],
    mut visit: impl FnMut(InstPoint<'_>, &Inst, &L),
) -> L
where
    L: Lattice,
    T: BackwardTransfer<L> + ?Sized,
{
    let body = &func.blocks[block as usize];
    let mut state = out.clone();
    transfer.transfer_terminator(func, block, &body.term, &mut state);
    for (i, inst) in body.insts.iter().enumerate().rev() {
        let at = InstPoint {
            func,
            cfg,
            block,
            index: i as u32,
        };
        if let Some(e) = cfg.unwind_edge(block, i as u32) {
            let edge = &cfg.edges[e as usize];
            if processed[edge.to as usize] {
                let mut s = block_in[edge.to as usize].clone();
                transfer.transfer_unwind(at, inst, &mut s);
                if transfer.transfer_edge(func, cfg, edge, &mut s) {
                    state.join(&s);
                }
            }
        }
        visit(at, inst, &state);
        transfer.transfer_inst(at, inst, &mut state);
    }
    state
}

/// Solves a backward problem over the blocks reachable from entry, then runs up to
/// `options.narrowing_passes` descending passes.
pub fn solve_backward<L, T>(
    func: &FuncIr,
    cfg: &IrCfg,
    transfer: &T,
    options: &SolverOptions,
) -> DataflowResult<L>
where
    L: Lattice,
    T: BackwardTransfer<L> + ?Sized,
{
    let n = cfg.len();
    let mut r: DataflowResult<L> = DataflowResult::empty(Direction::Backward, n, cfg.edges.len());
    if n == 0 {
        return r;
    }
    let budget = (options.max_visits_per_block as u64).saturating_mul(n as u64).max(1);
    let mut visits = vec![0u32; n];
    let mut worklist = RpoWorklist::new(cfg);
    for &b in &cfg.rpo {
        worklist.push(b);
    }

    // ── Ascending phase ──
    while let Some(b) = worklist.pop_last() {
        let bi = b as usize;
        r.visits += 1;
        if r.visits > budget {
            r.converged = false;
            break;
        }
        visits[bi] += 1;
        let out = backward_out(func, cfg, transfer, b, &r.block_in, &r.reached);
        let input = backward_block(
            func,
            cfg,
            transfer,
            b,
            &out,
            &r.block_in,
            &r.reached,
            |_, _, _| {},
        );
        r.block_out[bi] = out;
        let first = !r.reached[bi];
        r.reached[bi] = true;
        let changed = if cfg.loop_heads[bi] && visits[bi] >= options.widen_delay {
            r.block_in[bi].widen(&input)
        } else {
            r.block_in[bi].join(&input)
        };
        if changed || first {
            for &e in &cfg.pred_edges[bi] {
                worklist.push(cfg.edges[e as usize].from);
            }
        }
    }

    // ── Descending phase ──
    if r.converged {
        for _ in 0..options.narrowing_passes {
            let mut any = false;
            for &b in cfg.rpo.iter().rev() {
                let bi = b as usize;
                r.visits += 1;
                if r.visits > budget {
                    r.converged = false;
                    break;
                }
                let out = backward_out(func, cfg, transfer, b, &r.block_in, &r.reached);
                let fresh = backward_block(
                    func,
                    cfg,
                    transfer,
                    b,
                    &out,
                    &r.block_in,
                    &r.reached,
                    |_, _, _| {},
                );
                let changed = if cfg.loop_heads[bi] {
                    r.block_in[bi].narrow(&fresh)
                } else if fresh != r.block_in[bi] && fresh.leq(&r.block_in[bi]) {
                    r.block_in[bi] = fresh;
                    true
                } else {
                    false
                };
                if out.leq(&r.block_out[bi]) {
                    r.block_out[bi] = out;
                }
                any |= changed;
            }
            if !r.converged || !any {
                break;
            }
            r.narrowing_rounds += 1;
        }
    }
    r
}

/// Re-runs the backward transfer of `block` from its fixpoint out-state and calls `visit`
/// with the state *after* every instruction (exceptional successors included).
pub fn replay_backward<L, T>(
    func: &FuncIr,
    cfg: &IrCfg,
    transfer: &T,
    result: &DataflowResult<L>,
    block: u32,
    visit: impl FnMut(InstPoint<'_>, &Inst, &L),
) where
    L: Lattice,
    T: BackwardTransfer<L> + ?Sized,
{
    if !result.reached.get(block as usize).copied().unwrap_or(false) {
        return;
    }
    let out = result.block_out[block as usize].clone();
    backward_block(
        func,
        cfg,
        transfer,
        block,
        &out,
        &result.block_in,
        &result.reached,
        visit,
    );
}
