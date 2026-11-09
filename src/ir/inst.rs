//! Three-address SSA instructions, block terminators and call targets.
//!
//! Conventions shared by every producer and consumer:
//! * Every instruction defines at most one value (`Inst::dst`). Phi instructions are
//!   grouped at the start of their block.
//! * `Call::args[i]` binds `FuncIr::params[i]` of the target. Instance calls pass the
//!   receiver as `args[0]`; for `Callee::Virtual` and `Callee::Interface` the same value
//!   is repeated in the callee so dispatch can be resolved without inspecting `args`.
//!   `CallSiteIr::receiver` records whether `args[0]` is a receiver for every callee kind.
//! * A call inside a protected region carries the landing block in `unwind`; such a call
//!   is always the last instruction of its block (the block then ends with a `Jump` to the
//!   normal continuation), so the state at the end of the block is the state at the call.
//! * `Throw` transfers to `BlockIr::unwind` of its block, or leaves the function when that
//!   is `None`. The exception value arrives in the landing block through
//!   `IntrinsicKind::Exception`.

use crate::core::types::ast::OperatorId;
use crate::ir::types::{AllocSiteId, BlockId, CallSiteId, FuncId, TypeId, ValueId};
use crate::ir::value::{ConstVal, Operand, Place};

/// Unary operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UnaryOp {
    /// Arithmetic negation.
    Neg,
    /// Unary plus (numeric promotion only).
    Plus,
    /// Logical negation of a boolean.
    Not,
    /// Bitwise complement.
    BitNot,
}

impl UnaryOp {
    pub fn from_operator(op: OperatorId) -> Option<Self> {
        Some(match op {
            OperatorId::Neg | OperatorId::Sub => UnaryOp::Neg,
            OperatorId::Pos | OperatorId::Add => UnaryOp::Plus,
            OperatorId::LogicalNot => UnaryOp::Not,
            OperatorId::BitwiseNot => UnaryOp::BitNot,
            _ => return None,
        })
    }

    pub fn mnemonic(self) -> &'static str {
        match self {
            UnaryOp::Neg => "neg",
            UnaryOp::Plus => "plus",
            UnaryOp::Not => "not",
            UnaryOp::BitNot => "bitnot",
        }
    }
}

/// Binary arithmetic and bitwise operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    /// Arithmetic (sign-propagating) right shift.
    Shr,
    /// Logical (zero-filling) right shift.
    UShr,
}

impl BinaryOp {
    /// Operator of a binary expression. Comparisons and short-circuit operators are not
    /// arithmetic and map to `None`.
    pub fn from_operator(op: OperatorId) -> Option<Self> {
        Some(match op {
            OperatorId::Add => BinaryOp::Add,
            OperatorId::Sub => BinaryOp::Sub,
            OperatorId::Mul => BinaryOp::Mul,
            OperatorId::Div => BinaryOp::Div,
            OperatorId::Mod => BinaryOp::Rem,
            OperatorId::BitwiseAnd => BinaryOp::BitAnd,
            OperatorId::BitwiseOr => BinaryOp::BitOr,
            OperatorId::BitwiseXor => BinaryOp::BitXor,
            OperatorId::Shl => BinaryOp::Shl,
            OperatorId::Shr => BinaryOp::Shr,
            OperatorId::Ushr => BinaryOp::UShr,
            _ => return None,
        })
    }

    /// Arithmetic operator applied by a compound assignment (`+=` -> `Add`).
    pub fn from_compound_assignment(op: OperatorId) -> Option<Self> {
        Some(match op {
            OperatorId::AddAssign => BinaryOp::Add,
            OperatorId::SubAssign => BinaryOp::Sub,
            OperatorId::MulAssign => BinaryOp::Mul,
            OperatorId::DivAssign => BinaryOp::Div,
            OperatorId::ModAssign => BinaryOp::Rem,
            OperatorId::AndAssign => BinaryOp::BitAnd,
            OperatorId::OrAssign => BinaryOp::BitOr,
            OperatorId::XorAssign => BinaryOp::BitXor,
            OperatorId::ShlAssign => BinaryOp::Shl,
            OperatorId::ShrAssign => BinaryOp::Shr,
            OperatorId::UshrAssign => BinaryOp::UShr,
            _ => return None,
        })
    }

    pub fn mnemonic(self) -> &'static str {
        match self {
            BinaryOp::Add => "add",
            BinaryOp::Sub => "sub",
            BinaryOp::Mul => "mul",
            BinaryOp::Div => "div",
            BinaryOp::Rem => "rem",
            BinaryOp::BitAnd => "and",
            BinaryOp::BitOr => "or",
            BinaryOp::BitXor => "xor",
            BinaryOp::Shl => "shl",
            BinaryOp::Shr => "shr",
            BinaryOp::UShr => "ushr",
        }
    }
}

/// Comparison predicates. On reference-like operands `Eq`/`Ne` compare identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ComparePred {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl ComparePred {
    pub fn from_operator(op: OperatorId) -> Option<Self> {
        Some(match op {
            OperatorId::Eq => ComparePred::Eq,
            OperatorId::NotEq => ComparePred::Ne,
            OperatorId::Lt => ComparePred::Lt,
            OperatorId::LtEq => ComparePred::Le,
            OperatorId::Gt => ComparePred::Gt,
            OperatorId::GtEq => ComparePred::Ge,
            _ => return None,
        })
    }

    /// Predicate that holds exactly when `self` does not.
    pub fn negate(self) -> Self {
        match self {
            ComparePred::Eq => ComparePred::Ne,
            ComparePred::Ne => ComparePred::Eq,
            ComparePred::Lt => ComparePred::Ge,
            ComparePred::Le => ComparePred::Gt,
            ComparePred::Gt => ComparePred::Le,
            ComparePred::Ge => ComparePred::Lt,
        }
    }

    /// Predicate with the operands exchanged (`a < b` == `b > a`).
    pub fn swap(self) -> Self {
        match self {
            ComparePred::Lt => ComparePred::Gt,
            ComparePred::Le => ComparePred::Ge,
            ComparePred::Gt => ComparePred::Lt,
            ComparePred::Ge => ComparePred::Le,
            other => other,
        }
    }

    pub fn mnemonic(self) -> &'static str {
        match self {
            ComparePred::Eq => "eq",
            ComparePred::Ne => "ne",
            ComparePred::Lt => "lt",
            ComparePred::Le => "le",
            ComparePred::Gt => "gt",
            ComparePred::Ge => "ge",
        }
    }
}

/// Storage class of an allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Region {
    Heap,
    Stack,
    Static,
}

impl Region {
    pub fn mnemonic(self) -> &'static str {
        match self {
            Region::Heap => "heap",
            Region::Stack => "stack",
            Region::Static => "static",
        }
    }
}

/// Target of a call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Callee {
    /// Direct call of a function of this module.
    Static(FuncId),
    /// Direct call of a declared symbol that has no IR body in this module (abstract,
    /// native, external, or a declaration without a body).
    Symbol(u32),
    /// Dispatch on the dynamic class of `receiver`; `method` is the statically selected
    /// method symbol.
    Virtual { receiver: ValueId, method: u32 },
    /// Dispatch through an interface method `method` of `receiver`.
    Interface { receiver: ValueId, method: u32 },
    /// Call of a function value (closure, function pointer, delegate).
    Indirect(ValueId),
    /// The target is not derivable: interned method name as written.
    Unresolved(u32),
}

impl Callee {
    /// Receiver value used for dispatch.
    pub fn receiver(&self) -> Option<ValueId> {
        match self {
            Callee::Virtual { receiver, .. } | Callee::Interface { receiver, .. } => {
                Some(*receiver)
            }
            _ => None,
        }
    }

    /// Statically selected method symbol, when one is known.
    pub fn method_symbol(&self) -> Option<u32> {
        match self {
            Callee::Symbol(s) => Some(*s),
            Callee::Virtual { method, .. } | Callee::Interface { method, .. } => Some(*method),
            _ => None,
        }
    }

    #[inline]
    pub fn is_dynamic(&self) -> bool {
        matches!(
            self,
            Callee::Virtual { .. } | Callee::Interface { .. } | Callee::Indirect(_)
        )
    }

    fn for_each_value_slot(&mut self, f: &mut dyn FnMut(&mut ValueId)) {
        match self {
            Callee::Virtual { receiver, .. } | Callee::Interface { receiver, .. } => f(receiver),
            Callee::Indirect(v) => f(v),
            Callee::Static(_) | Callee::Symbol(_) | Callee::Unresolved(_) => {}
        }
    }
}

/// Language-neutral operations that have no instruction of their own. The name of an
/// `Inst::Intrinsic` is the interned `as_str()` spelling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IntrinsicKind {
    /// `dst = eh.exception()`: the exception being propagated into this landing block.
    Exception,
    /// `monitor.enter(obj)`: acquires the lock of a guarded region; the matching release is
    /// the `Release { Deref(obj) }` on every exit of the region.
    MonitorEnter,
    /// `dst = iter.begin(iterable)`: iterator protocol start, used when the language
    /// frontend names no protocol method.
    IterBegin,
    /// `dst = iter.has_next(iterator)`.
    IterHasNext,
    /// `dst = iter.next(iterator)`.
    IterNext,
    /// `dst = array.length(array)`.
    ArrayLength,
    /// `alloc.length(array, n)`: the freshly allocated `array` has `n` elements.
    AllocLength,
    /// `assert(cond[, message])`.
    Assert,
    /// `dst = type.literal(Int(symbol) | Str(name))`: runtime object denoting a type.
    TypeLiteral,
    /// `dst = literal.opaque(Str(text))`: a literal whose value the frontend cannot derive.
    OpaqueLiteral,
    /// `dst = op.unknown(Str(operator), operands..)`: an operator the frontend cannot name.
    UnknownOperator,
    /// `dst = name.unresolved(Str(name)[, qualifier])`: a name no declaration binds.
    UnresolvedName,
    /// `dst = opaque(Str(construct), operands..)`: a construct with no lowering rule; the
    /// operands are the lowered sub-expressions, in source order.
    Opaque,
}

impl IntrinsicKind {
    pub const ALL: [IntrinsicKind; 13] = [
        IntrinsicKind::Exception,
        IntrinsicKind::MonitorEnter,
        IntrinsicKind::IterBegin,
        IntrinsicKind::IterHasNext,
        IntrinsicKind::IterNext,
        IntrinsicKind::ArrayLength,
        IntrinsicKind::AllocLength,
        IntrinsicKind::Assert,
        IntrinsicKind::TypeLiteral,
        IntrinsicKind::OpaqueLiteral,
        IntrinsicKind::UnknownOperator,
        IntrinsicKind::UnresolvedName,
        IntrinsicKind::Opaque,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            IntrinsicKind::Exception => "eh.exception",
            IntrinsicKind::MonitorEnter => "monitor.enter",
            IntrinsicKind::IterBegin => "iter.begin",
            IntrinsicKind::IterHasNext => "iter.has_next",
            IntrinsicKind::IterNext => "iter.next",
            IntrinsicKind::ArrayLength => "array.length",
            IntrinsicKind::AllocLength => "alloc.length",
            IntrinsicKind::Assert => "assert",
            IntrinsicKind::TypeLiteral => "type.literal",
            IntrinsicKind::OpaqueLiteral => "literal.opaque",
            IntrinsicKind::UnknownOperator => "op.unknown",
            IntrinsicKind::UnresolvedName => "name.unresolved",
            IntrinsicKind::Opaque => "opaque",
        }
    }

    pub fn from_name(name: &[u8]) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|k| k.as_str().as_bytes() == name)
    }

    /// True when the intrinsic marks a fact the lowering could not derive.
    pub fn is_not_derivable(self) -> bool {
        matches!(
            self,
            IntrinsicKind::OpaqueLiteral
                | IntrinsicKind::UnknownOperator
                | IntrinsicKind::UnresolvedName
                | IntrinsicKind::Opaque
        )
    }
}

/// Three-address SSA instruction.
#[derive(Debug, Clone, PartialEq)]
pub enum Inst {
    Assign {
        dst: ValueId,
        src: Operand,
    },
    Unary {
        dst: ValueId,
        op: UnaryOp,
        src: Operand,
    },
    Binary {
        dst: ValueId,
        op: BinaryOp,
        lhs: Operand,
        rhs: Operand,
    },
    Compare {
        dst: ValueId,
        pred: ComparePred,
        lhs: Operand,
        rhs: Operand,
    },
    Load {
        dst: ValueId,
        place: Place,
    },
    Store {
        place: Place,
        value: Operand,
    },
    Alloc {
        dst: ValueId,
        site: AllocSiteId,
        ty: TypeId,
        region: Region,
    },
    Call {
        dst: Option<ValueId>,
        site: CallSiteId,
        callee: Callee,
        args: Vec<Operand>,
        unwind: Option<BlockId>,
    },
    Phi {
        dst: ValueId,
        incoming: Vec<(BlockId, Operand)>,
    },
    Cast {
        dst: ValueId,
        src: Operand,
        ty: TypeId,
    },
    /// Function value of `func` closing over `captures`; the closure's first
    /// `captures.len()` parameters receive them, in order.
    MakeClosure {
        dst: ValueId,
        func: FuncId,
        captures: Vec<Operand>,
    },
    /// `dst = src is ty` (boolean). Catch dispatch and pattern tests use it.
    TypeTest {
        dst: ValueId,
        src: Operand,
        ty: TypeId,
    },
    /// End of the lifetime region of the object at `place`: destructor, dispose, close,
    /// unlock. Emitted on every exit path of the region.
    Release {
        place: Place,
    },
    Intrinsic {
        dst: Option<ValueId>,
        name: u32,
        args: Vec<Operand>,
    },
}

impl Inst {
    /// Value defined by the instruction.
    pub fn dst(&self) -> Option<ValueId> {
        match self {
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

    /// Mutable access to the defined value.
    pub fn dst_mut(&mut self) -> Option<&mut ValueId> {
        match self {
            Inst::Assign { dst, .. }
            | Inst::Unary { dst, .. }
            | Inst::Binary { dst, .. }
            | Inst::Compare { dst, .. }
            | Inst::Load { dst, .. }
            | Inst::Alloc { dst, .. }
            | Inst::Phi { dst, .. }
            | Inst::Cast { dst, .. }
            | Inst::MakeClosure { dst, .. }
            | Inst::TypeTest { dst, .. } => Some(dst),
            Inst::Call { dst, .. } | Inst::Intrinsic { dst, .. } => dst.as_mut(),
            Inst::Store { .. } | Inst::Release { .. } => None,
        }
    }

    /// Every value the instruction reads, in operand order (duplicates kept).
    pub fn uses(&self) -> Vec<ValueId> {
        let mut out = Vec::new();
        let mut copy = self.clone();
        copy.for_each_value_slot(&mut |v| out.push(*v));
        copy.for_each_operand_slot(&mut |op| {
            if let Operand::Value(v) = op {
                out.push(*v);
            }
        });
        out
    }

    /// Visits the slots that must hold a value (place bases, dispatch receivers, closure
    /// pointers). The defined value is not visited.
    pub fn for_each_value_slot(&mut self, f: &mut dyn FnMut(&mut ValueId)) {
        match self {
            Inst::Load { place, .. } | Inst::Store { place, .. } | Inst::Release { place } => {
                place.for_each_value_slot(f)
            }
            Inst::Call { callee, .. } => callee.for_each_value_slot(f),
            _ => {}
        }
    }

    /// Visits the operand slots (including the operands nested in places).
    pub fn for_each_operand_slot(&mut self, f: &mut dyn FnMut(&mut Operand)) {
        match self {
            Inst::Assign { src, .. }
            | Inst::Unary { src, .. }
            | Inst::Cast { src, .. }
            | Inst::TypeTest { src, .. } => f(src),
            Inst::Binary { lhs, rhs, .. } | Inst::Compare { lhs, rhs, .. } => {
                f(lhs);
                f(rhs);
            }
            Inst::Load { place, .. } | Inst::Release { place } => place.for_each_operand_slot(f),
            Inst::Store { place, value } => {
                place.for_each_operand_slot(f);
                f(value);
            }
            Inst::Alloc { .. } => {}
            Inst::Call { args, .. } | Inst::Intrinsic { args, .. } => {
                for a in args.iter_mut() {
                    f(a);
                }
            }
            Inst::Phi { incoming, .. } => {
                for (_, op) in incoming.iter_mut() {
                    f(op);
                }
            }
            Inst::MakeClosure { captures, .. } => {
                for c in captures.iter_mut() {
                    f(c);
                }
            }
        }
    }

    /// Visits the block references (phi predecessors and the unwind target).
    pub fn for_each_block_slot(&mut self, f: &mut dyn FnMut(&mut BlockId)) {
        match self {
            Inst::Phi { incoming, .. } => {
                for (b, _) in incoming.iter_mut() {
                    f(b);
                }
            }
            Inst::Call {
                unwind: Some(b), ..
            } => f(b),
            _ => {}
        }
    }

    /// Landing block of a call inside a protected region.
    pub fn unwind(&self) -> Option<BlockId> {
        match self {
            Inst::Call { unwind, .. } => *unwind,
            _ => None,
        }
    }

    #[inline]
    pub fn is_phi(&self) -> bool {
        matches!(self, Inst::Phi { .. })
    }

    #[inline]
    pub fn is_call(&self) -> bool {
        matches!(self, Inst::Call { .. })
    }

    /// Memory place read or written by the instruction.
    pub fn place(&self) -> Option<&Place> {
        match self {
            Inst::Load { place, .. } | Inst::Store { place, .. } | Inst::Release { place } => {
                Some(place)
            }
            _ => None,
        }
    }

    pub fn mnemonic(&self) -> &'static str {
        match self {
            Inst::Assign { .. } => "assign",
            Inst::Unary { op, .. } => op.mnemonic(),
            Inst::Binary { op, .. } => op.mnemonic(),
            Inst::Compare { .. } => "cmp",
            Inst::Load { .. } => "load",
            Inst::Store { .. } => "store",
            Inst::Alloc { .. } => "alloc",
            Inst::Call { .. } => "call",
            Inst::Phi { .. } => "phi",
            Inst::Cast { .. } => "cast",
            Inst::MakeClosure { .. } => "closure",
            Inst::TypeTest { .. } => "typetest",
            Inst::Release { .. } => "release",
            Inst::Intrinsic { .. } => "intrinsic",
        }
    }
}

/// Block terminator.
#[derive(Debug, Clone, PartialEq)]
pub enum Terminator {
    Jump(BlockId),
    Branch {
        cond: Operand,
        then_b: BlockId,
        else_b: BlockId,
    },
    /// Multi-way branch on constant case labels; unmatched values go to `default`.
    Switch {
        scrutinee: Operand,
        cases: Vec<(ConstVal, BlockId)>,
        default: BlockId,
    },
    Return(Option<Operand>),
    /// Raises the operand; see the module documentation for its exceptional successor.
    Throw(Operand),
    Unreachable,
}

impl Terminator {
    /// Normal (non-exceptional) successors, without duplicates, in branch order.
    pub fn successors(&self) -> Vec<BlockId> {
        let mut out: Vec<BlockId> = Vec::new();
        let mut push = |b: BlockId| {
            if !out.contains(&b) {
                out.push(b);
            }
        };
        match self {
            Terminator::Jump(b) => push(*b),
            Terminator::Branch { then_b, else_b, .. } => {
                push(*then_b);
                push(*else_b);
            }
            Terminator::Switch { cases, default, .. } => {
                for (_, b) in cases {
                    push(*b);
                }
                push(*default);
            }
            Terminator::Return(_) | Terminator::Throw(_) | Terminator::Unreachable => {}
        }
        out
    }

    pub fn for_each_successor_slot(&mut self, f: &mut dyn FnMut(&mut BlockId)) {
        match self {
            Terminator::Jump(b) => f(b),
            Terminator::Branch { then_b, else_b, .. } => {
                f(then_b);
                f(else_b);
            }
            Terminator::Switch { cases, default, .. } => {
                for (_, b) in cases.iter_mut() {
                    f(b);
                }
                f(default);
            }
            Terminator::Return(_) | Terminator::Throw(_) | Terminator::Unreachable => {}
        }
    }

    pub fn for_each_operand_slot(&mut self, f: &mut dyn FnMut(&mut Operand)) {
        match self {
            Terminator::Branch { cond, .. } => f(cond),
            Terminator::Switch { scrutinee, .. } => f(scrutinee),
            Terminator::Return(Some(op)) | Terminator::Throw(op) => f(op),
            Terminator::Return(None) | Terminator::Jump(_) | Terminator::Unreachable => {}
        }
    }

    /// Values the terminator reads.
    pub fn uses(&self) -> Vec<ValueId> {
        let mut out = Vec::new();
        let mut copy = self.clone();
        copy.for_each_operand_slot(&mut |op| {
            if let Operand::Value(v) = op {
                out.push(*v);
            }
        });
        out
    }

    #[inline]
    pub fn is_return(&self) -> bool {
        matches!(self, Terminator::Return(_))
    }

    pub fn mnemonic(&self) -> &'static str {
        match self {
            Terminator::Jump(_) => "jump",
            Terminator::Branch { .. } => "branch",
            Terminator::Switch { .. } => "switch",
            Terminator::Return(_) => "return",
            Terminator::Throw(_) => "throw",
            Terminator::Unreachable => "unreachable",
        }
    }
}
