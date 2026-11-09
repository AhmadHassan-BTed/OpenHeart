//! Symbolic state: values, the hash-consed expression arena, symbols, stack frames, the
//! heap and the path condition of one execution path.
//!
//! **Values.** A [`SymValue`] is a concrete constant, a pointer into a symbolic object, a
//! function value, or a symbolic expression [`ExprId`]. Expressions are interned in the
//! [`SymContext`] shared by every path of one exploration, so structurally equal
//! expressions over the same symbols are one expression everywhere: two tests of `x + 1 < y`
//! on different paths constrain the same solver variable.
//!
//! **Arithmetic is the program's, not the solver's.** Every integral expression carries the
//! value class of its IR type. Fixed-width operations whose overflow wraps are encoded
//! exactly as `t = raw − 2ʷ·q` with a quotient variable `q` (two's-complement modulo);
//! operations whose overflow is undefined are encoded as `t = raw` inside the type range
//! (the path assumes, as the compiler does, that undefined behaviour does not happen);
//! operations whose semantics the frontend did not state are evaluated only where no
//! overflow occurs. Non-linear operations are evaluable terms with the sound range facts
//! they imply (`0 ≤ x & m ≤ m`, `|x % c| < |c|`, ...). What the language does on overflow,
//! how it rounds division and how wide `char` is come from [`ArithmeticFacts`], which the
//! frontend supplies; nothing here assumes a language.

use std::collections::{BTreeSet, HashMap};

use crate::analysis::conditions::Cond;
use crate::ir::inst::{BinaryOp, ComparePred, UnaryOp};
use crate::ir::types::{
    AllocSiteId, BlockId, CallSiteId, FuncId, IrType, Span, TypeId, TypeTable, ValueId,
};
use crate::symex::memory::{CellKey, Fault, Heap, StaticKey};
use crate::symex::solver::{
    Atom, Domain, LinExpr, RefTerm, TermInfo, TermKind, TermOracle, UnknownReason, Var,
};
use crate::symex::trace::TraceBuf;

// ── Identities ───────────────────────────────────────────────────────────────

/// A fresh symbol: an input of the exploration or a value nothing determines.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SymId(pub u32);

/// An interned symbolic expression.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ExprId(pub u32);

impl ExprId {
    /// The solver variable standing for this expression.
    #[inline]
    pub fn var(self) -> Var {
        Var(self.0)
    }
}

/// Where a symbolic object comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ObjectOrigin {
    /// An `Inst::Alloc` of the program.
    Alloc(AllocSiteId),
    /// The object a symbolic reference designates when the exploration starts (an entry
    /// parameter, an initial static) or after opaque code produced it.
    Input(SymId),
    /// The environment of a `MakeClosure`.
    Closure(FuncId),
}

/// A symbolic heap object: allocation origin plus the dynamic instance number of that
/// origin along the path (the second allocation at a site in a loop is instance 1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ObjectId {
    pub origin: ObjectOrigin,
    pub instance: u32,
}

/// An integer that is either known or symbolic (pointer offsets, indices, sizes).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SymInt {
    Const(i128),
    Expr(ExprId),
}

impl SymInt {
    pub fn as_const(&self) -> Option<i128> {
        match self {
            SymInt::Const(c) => Some(*c),
            SymInt::Expr(_) => None,
        }
    }

    pub fn value(&self) -> SymValue {
        match self {
            SymInt::Const(c) => SymValue::Int(*c),
            SymInt::Expr(e) => SymValue::Expr(*e),
        }
    }
}

/// An address inside a symbolic object. `offset` counts bytes in byte-addressed objects
/// and elements in object/field-level objects.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Pointer {
    pub object: ObjectId,
    pub offset: SymInt,
    /// Type of the location designated, when known (sizes loads and stores, scales
    /// pointer arithmetic).
    pub pointee: Option<TypeId>,
}

/// A symbolic value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SymValue {
    /// No definition reaches the use.
    Undef,
    Bool(bool),
    Int(i128),
    /// IEEE-754 binary64 bit pattern.
    Float(u64),
    /// Interned string literal (`ModuleIr::strings`).
    Str(u32),
    Null,
    Ptr(Pointer),
    /// A function value of the module.
    Func(FuncId),
    Expr(ExprId),
}

impl SymValue {
    pub fn is_concrete(&self) -> bool {
        !matches!(self, SymValue::Expr(_) | SymValue::Undef)
    }

    pub fn as_int(&self) -> Option<i128> {
        match self {
            SymValue::Int(i) => Some(*i),
            SymValue::Bool(b) => Some(*b as i128),
            _ => None,
        }
    }

    pub fn as_sym_int(&self) -> Option<SymInt> {
        match self {
            SymValue::Int(i) => Some(SymInt::Const(*i)),
            SymValue::Bool(b) => Some(SymInt::Const(*b as i128)),
            SymValue::Expr(e) => Some(SymInt::Expr(*e)),
            _ => None,
        }
    }
}

// ── Value classes and language arithmetic ────────────────────────────────────

/// The scalar class of a value, derived from its IR type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ValueClass {
    Bool,
    Int { bits: u16, signed: bool },
    /// An integer whose width the IR does not state.
    IntAnyWidth,
    Float { bits: u16 },
    /// Pointers, references, objects, arrays, strings, function values.
    Ref,
    /// `Void` and other non-scalar types.
    Other,
    Unknown,
}

impl ValueClass {
    /// The class of values of type `ty`.
    pub fn of_type(types: &TypeTable, ty: TypeId, facts: &ArithmeticFacts) -> Self {
        if ty.index() >= types.len() {
            return ValueClass::Unknown;
        }
        match types.get(ty) {
            IrType::Bool => ValueClass::Bool,
            IrType::Int { bits, signed } => ValueClass::Int {
                bits: *bits,
                signed: *signed,
            },
            IrType::Char => match facts.char_class {
                Some((bits, signed)) => ValueClass::Int { bits, signed },
                None => ValueClass::IntAnyWidth,
            },
            IrType::Float { bits } => ValueClass::Float { bits: *bits },
            IrType::Ptr(_)
            | IrType::Ref { .. }
            | IrType::Named(_)
            | IrType::Array { .. }
            | IrType::Func { .. }
            | IrType::Generic { .. }
            | IrType::TypeParam(_)
            | IrType::Str => ValueClass::Ref,
            IrType::Void => ValueClass::Other,
            IrType::Unknown => ValueClass::Unknown,
        }
    }

    /// Booleans and integers.
    pub fn is_integral(&self) -> bool {
        matches!(
            self,
            ValueClass::Bool | ValueClass::Int { .. } | ValueClass::IntAnyWidth
        )
    }

    pub fn is_float(&self) -> bool {
        matches!(self, ValueClass::Float { .. })
    }
}

/// What happens when integer arithmetic leaves the range of its type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Overflow {
    /// Two's-complement wrap-around (JVM, CLR unchecked, C/C++ unsigned).
    Wraps,
    /// Undefined behaviour: a valid execution never overflows (C/C++ signed).
    Undefined,
    /// Integers are unbounded (arbitrary precision).
    Unbounded,
    /// The frontend did not state it; only non-overflowing results are derived.
    Unspecified,
}

/// How integer division rounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DivisionRounding {
    TowardZero,
    Floor,
    Unspecified,
}

/// What a shift by a count outside `[0, width)` does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ShiftCount {
    /// The count is reduced modulo the width.
    Masked,
    /// Undefined behaviour.
    Undefined,
    Unspecified,
}

/// Arithmetic semantics of the source language, supplied by its frontend.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ArithmeticFacts {
    pub signed_overflow: Overflow,
    pub unsigned_overflow: Overflow,
    pub division: DivisionRounding,
    pub shift_count: ShiftCount,
    /// Width and signedness of `IrType::Char`.
    pub char_class: Option<(u16, bool)>,
}

impl ArithmeticFacts {
    /// No stated semantics: only results that are the same under every semantics are
    /// derived.
    pub const UNSPECIFIED: ArithmeticFacts = ArithmeticFacts {
        signed_overflow: Overflow::Unspecified,
        unsigned_overflow: Overflow::Unspecified,
        division: DivisionRounding::Unspecified,
        shift_count: ShiftCount::Unspecified,
        char_class: None,
    };

    pub fn overflow(&self, signed: bool) -> Overflow {
        if signed {
            self.signed_overflow
        } else {
            self.unsigned_overflow
        }
    }
}

/// Why an arithmetic operation has no value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ArithFault {
    DivisionByZero,
    /// Overflow under `Overflow::Undefined`.
    Overflow,
    /// Shift count outside the width under `ShiftCount::Undefined`.
    ShiftOutOfRange,
    /// The result depends on a semantics the frontend did not state.
    NotDerivable,
}

/// Result of building an operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArithOutcome {
    Value(SymValue),
    Fault(ArithFault),
}

/// `2^bits`, for widths whose modulus fits an i128.
fn modulus(bits: u16) -> Option<i128> {
    if bits == 0 || bits > 126 {
        None
    } else {
        Some(1i128 << bits)
    }
}

/// Range of an integer type.
pub fn int_range(bits: u16, signed: bool) -> Option<(i128, i128)> {
    let m = modulus(bits)?;
    Some(if signed { (-(m / 2), m / 2 - 1) } else { (0, m - 1) })
}

/// Two's-complement reduction of `x` into the type.
pub fn wrap(x: i128, bits: u16, signed: bool) -> Option<i128> {
    let m = modulus(bits)?;
    let r = x.rem_euclid(m);
    Some(if signed && r >= m / 2 { r - m } else { r })
}

/// Range every integer type of at least 8 bits, signed or not, can represent.
const PORTABLE_RANGE: (i128, i128) = (0, 127);

// ── Symbols and expressions ──────────────────────────────────────────────────

/// Why a symbol exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SymbolOrigin {
    /// Parameter `index` of the exploration's entry function.
    EntryParam { func: FuncId, index: u32 },
    /// Value of a module variable or static field the path has not written.
    StaticContents(StaticKey),
    /// Contents of a cell of an object whose contents the path does not determine (an
    /// input object, or an object opaque code may have modified).
    Contents { object: ObjectId, cell: CellKey },
    /// Result of a call whose callee was not executed.
    CallResult { site: CallSiteId },
    /// Exception raised by a call whose callee was not executed.
    Exception { site: CallSiteId },
    /// Value of an SSA value of a summarized loop after any number of iterations.
    LoopValue { func: FuncId, head: BlockId, value: ValueId },
    /// Read of memory never written (its contents are indeterminate).
    Uninitialized { object: ObjectId },
    /// Read whose value the memory model could not decide (overlapping or symbolic-index
    /// writes, reinterpretation of bytes).
    Undecided { func: FuncId, span: Span },
    /// Result of an operation that is not derivable (opaque intrinsic, unknown operator,
    /// arithmetic under unstated semantics).
    NotDerivable { func: FuncId, span: Span },
    /// An `Operand::Undef`.
    Undefined { func: FuncId, span: Span },
}

/// Description of one symbol.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SymbolInfo {
    pub origin: SymbolOrigin,
    pub class: ValueClass,
    pub ty: Option<TypeId>,
}

/// An interned expression node.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ExprNode {
    Symbol(SymId),
    Unary {
        op: UnaryOp,
        a: SymValue,
        class: ValueClass,
    },
    Binary {
        op: BinaryOp,
        a: SymValue,
        b: SymValue,
        class: ValueClass,
    },
    /// `pred` is `Eq`, `Ne`, `Lt` or `Le` (`Gt`/`Ge` are swapped into `Lt`/`Le`); `class` is
    /// the class of the operands.
    Compare {
        pred: ComparePred,
        a: SymValue,
        b: SymValue,
        class: ValueClass,
    },
    Cast {
        a: SymValue,
        to: ValueClass,
        ty: TypeId,
    },
    TypeTest {
        a: SymValue,
        ty: TypeId,
    },
    /// The wrap-around quotient `q` of the fixed-width operation `of`.
    Quotient(ExprId),
}

// ── Shared context ───────────────────────────────────────────────────────────

/// Maximum nesting depth of expression evaluation under a model.
const MAX_EVAL_DEPTH: u32 = 256;

/// How an operation's result relates to its exact (mathematical) value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Encoding {
    /// `t = raw − m·q`: two's-complement modulo.
    Wrap { m: i128 },
    /// `t = raw` (inside the type range).
    Exact,
    /// Only evaluable under a model.
    Evaluate,
}

/// Expressions, symbols and object constants shared by every path of one exploration.
#[derive(Debug)]
pub struct SymContext {
    nodes: Vec<ExprNode>,
    index: HashMap<ExprNode, ExprId>,
    symbols: Vec<SymbolInfo>,
    objects: Vec<ObjectId>,
    object_index: HashMap<ObjectId, u32>,
    pub arith: ArithmeticFacts,
}

impl SymContext {
    pub fn new(arith: ArithmeticFacts) -> Self {
        Self {
            nodes: Vec::new(),
            index: HashMap::new(),
            symbols: Vec::new(),
            objects: Vec::new(),
            object_index: HashMap::new(),
            arith,
        }
    }

    #[inline]
    pub fn node(&self, e: ExprId) -> &ExprNode {
        &self.nodes[e.0 as usize]
    }

    pub fn expr_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn symbol(&self, s: SymId) -> &SymbolInfo {
        &self.symbols[s.0 as usize]
    }

    pub fn symbol_count(&self) -> usize {
        self.symbols.len()
    }

    /// The symbol an expression is, when it is a bare symbol.
    pub fn symbol_of(&self, e: ExprId) -> Option<SymId> {
        match self.node(e) {
            ExprNode::Symbol(s) => Some(*s),
            _ => None,
        }
    }

    fn intern(&mut self, node: ExprNode) -> ExprId {
        if let Some(&e) = self.index.get(&node) {
            return e;
        }
        let e = ExprId(
            u32::try_from(self.nodes.len()).expect("expression arena exceeds the 32-bit id space"),
        );
        self.nodes.push(node);
        self.index.insert(node, e);
        e
    }

    /// A new symbol.
    pub fn fresh(&mut self, origin: SymbolOrigin, class: ValueClass, ty: Option<TypeId>) -> SymValue {
        let s = SymId(
            u32::try_from(self.symbols.len()).expect("symbol table exceeds the 32-bit id space"),
        );
        self.symbols.push(SymbolInfo { origin, class, ty });
        SymValue::Expr(self.intern(ExprNode::Symbol(s)))
    }

    /// The solver constant denoting `obj`.
    pub fn object_const(&mut self, obj: ObjectId) -> u32 {
        if let Some(&k) = self.object_index.get(&obj) {
            return k;
        }
        let k = self.objects.len() as u32;
        self.objects.push(obj);
        self.object_index.insert(obj, k);
        k
    }

    /// The object a solver constant denotes.
    pub fn object_of_const(&self, k: u32) -> Option<ObjectId> {
        self.objects.get(k as usize).copied()
    }

    pub fn class_of(&self, v: &SymValue) -> ValueClass {
        match v {
            SymValue::Undef => ValueClass::Unknown,
            SymValue::Bool(_) => ValueClass::Bool,
            SymValue::Int(_) => ValueClass::IntAnyWidth,
            SymValue::Float(_) => ValueClass::Float { bits: 64 },
            SymValue::Str(_) | SymValue::Null | SymValue::Ptr(_) | SymValue::Func(_) => ValueClass::Ref,
            SymValue::Expr(e) => self.expr_class(*e),
        }
    }

    pub fn expr_class(&self, e: ExprId) -> ValueClass {
        match self.node(e) {
            ExprNode::Symbol(s) => self.symbols[s.0 as usize].class,
            ExprNode::Unary { op: UnaryOp::Not, .. } => ValueClass::Bool,
            ExprNode::Unary { class, .. } | ExprNode::Binary { class, .. } => *class,
            ExprNode::Compare { .. } | ExprNode::TypeTest { .. } => ValueClass::Bool,
            ExprNode::Cast { to, .. } => *to,
            ExprNode::Quotient(_) => ValueClass::IntAnyWidth,
        }
    }

    /// Range of values of `class` under the language's arithmetic.
    pub fn class_domain(&self, class: ValueClass) -> Domain {
        match class {
            ValueClass::Bool => Domain::range(0, 1),
            ValueClass::Int { bits, signed } => {
                if self.arith.overflow(signed) == Overflow::Unbounded {
                    Domain::UNBOUNDED
                } else {
                    int_range(bits, signed)
                        .map(|(lo, hi)| Domain::range(lo, hi))
                        .unwrap_or(Domain::UNBOUNDED)
                }
            }
            _ => Domain::UNBOUNDED,
        }
    }

    // ── Concrete arithmetic ──────────────────────────────────────────────────

    /// Apply `class`'s overflow semantics to an exact result.
    fn fit(&self, raw: i128, class: ValueClass) -> Result<i128, ArithFault> {
        match class {
            ValueClass::Bool => {
                if raw == 0 || raw == 1 {
                    Ok(raw)
                } else {
                    Err(ArithFault::NotDerivable)
                }
            }
            ValueClass::Int { bits, signed } => {
                let overflow = self.arith.overflow(signed);
                let (lo, hi) = match int_range(bits, signed) {
                    Some(r) => r,
                    None if overflow == Overflow::Unbounded => return Ok(raw),
                    None => return Err(ArithFault::NotDerivable),
                };
                if raw >= lo && raw <= hi {
                    return Ok(raw);
                }
                match overflow {
                    Overflow::Wraps => wrap(raw, bits, signed).ok_or(ArithFault::NotDerivable),
                    Overflow::Undefined => Err(ArithFault::Overflow),
                    Overflow::Unbounded => Ok(raw),
                    Overflow::Unspecified => Err(ArithFault::NotDerivable),
                }
            }
            _ => {
                if self.arith.signed_overflow == Overflow::Unbounded
                    || (raw >= PORTABLE_RANGE.0 && raw <= PORTABLE_RANGE.1)
                {
                    Ok(raw)
                } else {
                    Err(ArithFault::NotDerivable)
                }
            }
        }
    }

    fn width(class: ValueClass) -> Option<u16> {
        match class {
            ValueClass::Int { bits, .. } => Some(bits),
            _ => None,
        }
    }

    /// The effective shift count of `y` for a shift in `class`.
    fn shift_amount(&self, y: i128, class: ValueClass) -> Result<u32, ArithFault> {
        let bits = match Self::width(class) {
            Some(b) => b as i128,
            None => {
                return if (0..8).contains(&y) {
                    Ok(y as u32)
                } else {
                    Err(ArithFault::NotDerivable)
                }
            }
        };
        if bits > 126 {
            return Err(ArithFault::NotDerivable);
        }
        if y >= 0 && y < bits {
            return Ok(y as u32);
        }
        match self.arith.shift_count {
            ShiftCount::Masked => Ok((y & (bits - 1)) as u32),
            ShiftCount::Undefined => Err(ArithFault::ShiftOutOfRange),
            ShiftCount::Unspecified => Err(ArithFault::NotDerivable),
        }
    }

    /// `x op y` in `class`.
    pub fn int_binary(&self, op: BinaryOp, x: i128, y: i128, class: ValueClass) -> Result<i128, ArithFault> {
        let overflowed = ArithFault::NotDerivable;
        let raw = match op {
            BinaryOp::Add => x.checked_add(y).ok_or(overflowed)?,
            BinaryOp::Sub => x.checked_sub(y).ok_or(overflowed)?,
            BinaryOp::Mul => x.checked_mul(y).ok_or(overflowed)?,
            BinaryOp::Div => {
                if y == 0 {
                    return Err(ArithFault::DivisionByZero);
                }
                let exact = x % y == 0;
                match self.arith.division {
                    DivisionRounding::TowardZero => x.checked_div(y).ok_or(overflowed)?,
                    DivisionRounding::Floor => {
                        let q = x.checked_div(y).ok_or(overflowed)?;
                        if !exact && ((x < 0) != (y < 0)) {
                            q - 1
                        } else {
                            q
                        }
                    }
                    DivisionRounding::Unspecified => {
                        if exact || (x >= 0 && y > 0) {
                            x.checked_div(y).ok_or(overflowed)?
                        } else {
                            return Err(ArithFault::NotDerivable);
                        }
                    }
                }
            }
            BinaryOp::Rem => {
                if y == 0 {
                    return Err(ArithFault::DivisionByZero);
                }
                let r = x.checked_rem(y).ok_or(overflowed)?;
                match self.arith.division {
                    DivisionRounding::TowardZero => r,
                    DivisionRounding::Floor => {
                        if r != 0 && ((r < 0) != (y < 0)) {
                            r + y
                        } else {
                            r
                        }
                    }
                    DivisionRounding::Unspecified => {
                        if r == 0 || (x >= 0 && y > 0) {
                            r
                        } else {
                            return Err(ArithFault::NotDerivable);
                        }
                    }
                }
            }
            BinaryOp::BitAnd => x & y,
            BinaryOp::BitOr => x | y,
            BinaryOp::BitXor => x ^ y,
            BinaryOp::Shl => {
                let k = self.shift_amount(y, class)?;
                x.checked_mul(1i128 << k).ok_or(overflowed)?
            }
            BinaryOp::Shr => {
                let k = self.shift_amount(y, class)?;
                x >> k
            }
            BinaryOp::UShr => {
                let k = self.shift_amount(y, class)?;
                return match class {
                    ValueClass::Int { bits, signed } => {
                        let m = modulus(bits).ok_or(ArithFault::NotDerivable)?;
                        wrap(x.rem_euclid(m) >> k, bits, signed).ok_or(ArithFault::NotDerivable)
                    }
                    _ if x >= 0 => self.fit(x >> k, class),
                    _ => Err(ArithFault::NotDerivable),
                };
            }
        };
        self.fit(raw, class)
    }

    /// `op x` in `class`.
    pub fn int_unary(&self, op: UnaryOp, x: i128, class: ValueClass) -> Result<i128, ArithFault> {
        match op {
            UnaryOp::Plus => Ok(x),
            UnaryOp::Not => Ok((x == 0) as i128),
            UnaryOp::Neg => self.fit(x.checked_neg().ok_or(ArithFault::NotDerivable)?, class),
            UnaryOp::BitNot => match class {
                ValueClass::Int { signed: true, .. } => Ok(!x),
                ValueClass::Int { bits, signed: false } => {
                    let m = modulus(bits).ok_or(ArithFault::NotDerivable)?;
                    Ok(m - 1 - x)
                }
                _ => Err(ArithFault::NotDerivable),
            },
        }
    }

    /// True when conversions into an integer type of this signedness are modular.
    fn conversion_wraps(&self, signed: bool) -> bool {
        self.arith.overflow(signed) == Overflow::Wraps
    }

    /// Convert the integer `x` into class `to`.
    pub fn int_convert(&self, x: i128, to: ValueClass) -> Result<i128, ArithFault> {
        match to {
            ValueClass::Bool => Ok((x != 0) as i128),
            ValueClass::Int { bits, signed } => match int_range(bits, signed) {
                Some((lo, hi)) if x >= lo && x <= hi => Ok(x),
                Some(_) if self.conversion_wraps(signed) => {
                    wrap(x, bits, signed).ok_or(ArithFault::NotDerivable)
                }
                None if self.arith.overflow(signed) == Overflow::Unbounded => Ok(x),
                _ => Err(ArithFault::NotDerivable),
            },
            ValueClass::IntAnyWidth | ValueClass::Unknown => self.fit(x, to),
            _ => Err(ArithFault::NotDerivable),
        }
    }

    fn float_binary(&self, op: BinaryOp, x: f64, y: f64, bits: u16) -> Option<f64> {
        let r = match op {
            BinaryOp::Add => x + y,
            BinaryOp::Sub => x - y,
            BinaryOp::Mul => x * y,
            BinaryOp::Div => x / y,
            BinaryOp::Rem => match self.arith.division {
                DivisionRounding::TowardZero => x % y,
                DivisionRounding::Floor => x - y * (x / y).floor(),
                DivisionRounding::Unspecified if x >= 0.0 && y > 0.0 => x % y,
                DivisionRounding::Unspecified => return None,
            },
            _ => return None,
        };
        Some(round_float(r, bits))
    }

    fn int_value(r: i128, class: ValueClass) -> SymValue {
        match class {
            ValueClass::Bool => SymValue::Bool(r != 0),
            _ => SymValue::Int(r),
        }
    }

    // ── Expression constructors ──────────────────────────────────────────────

    /// `a op b` in `class` (the class of the result).
    pub fn binary(&mut self, op: BinaryOp, a: SymValue, b: SymValue, class: ValueClass) -> ArithOutcome {
        if let (Some(x), Some(y)) = (a.as_int(), b.as_int()) {
            if class.is_integral() || class == ValueClass::Unknown {
                return match self.int_binary(op, x, y, class) {
                    Ok(r) => ArithOutcome::Value(Self::int_value(r, class)),
                    Err(f) => ArithOutcome::Fault(f),
                };
            }
        }
        if let ValueClass::Float { bits } = class {
            if let (SymValue::Float(x), SymValue::Float(y)) = (a, b) {
                return match self.float_binary(op, f64::from_bits(x), f64::from_bits(y), bits) {
                    Some(r) => ArithOutcome::Value(SymValue::Float(r.to_bits())),
                    None => ArithOutcome::Fault(ArithFault::NotDerivable),
                };
            }
        }
        if class.is_integral() {
            if matches!(op, BinaryOp::Div | BinaryOp::Rem) && b.as_int() == Some(0) {
                return ArithOutcome::Fault(ArithFault::DivisionByZero);
            }
            if let Some(v) = Self::identity(op, a, b) {
                return ArithOutcome::Value(v);
            }
        }
        let e = self.intern(ExprNode::Binary { op, a, b, class });
        self.attach_quotient(e);
        ArithOutcome::Value(SymValue::Expr(e))
    }

    /// Algebraic identities that hold under every overflow semantics.
    fn identity(op: BinaryOp, a: SymValue, b: SymValue) -> Option<SymValue> {
        let (ca, cb) = (a.as_int(), b.as_int());
        let same = a == b && matches!(a, SymValue::Expr(_));
        match op {
            BinaryOp::Add if cb == Some(0) => Some(a),
            BinaryOp::Add if ca == Some(0) => Some(b),
            BinaryOp::Sub if cb == Some(0) => Some(a),
            BinaryOp::Sub if same => Some(SymValue::Int(0)),
            BinaryOp::Mul if cb == Some(1) => Some(a),
            BinaryOp::Mul if ca == Some(1) => Some(b),
            BinaryOp::Mul if ca == Some(0) || cb == Some(0) => Some(SymValue::Int(0)),
            BinaryOp::Div if cb == Some(1) => Some(a),
            BinaryOp::Rem if cb == Some(1) => Some(SymValue::Int(0)),
            BinaryOp::BitAnd if ca == Some(0) || cb == Some(0) => Some(SymValue::Int(0)),
            BinaryOp::BitAnd | BinaryOp::BitOr if same => Some(a),
            BinaryOp::BitOr | BinaryOp::BitXor if cb == Some(0) => Some(a),
            BinaryOp::BitOr | BinaryOp::BitXor if ca == Some(0) => Some(b),
            BinaryOp::BitXor if same => Some(SymValue::Int(0)),
            BinaryOp::Shl | BinaryOp::Shr | BinaryOp::UShr if cb == Some(0) => Some(a),
            _ => None,
        }
    }

    /// `op a` in `class`.
    pub fn unary(&mut self, op: UnaryOp, a: SymValue, class: ValueClass) -> ArithOutcome {
        match (op, a) {
            (UnaryOp::Plus, _) => return ArithOutcome::Value(a),
            (UnaryOp::Not, SymValue::Bool(b)) => return ArithOutcome::Value(SymValue::Bool(!b)),
            (UnaryOp::Not, SymValue::Int(i)) => return ArithOutcome::Value(SymValue::Bool(i == 0)),
            (UnaryOp::Not, SymValue::Null) => return ArithOutcome::Value(SymValue::Bool(true)),
            (UnaryOp::Not, SymValue::Ptr(_) | SymValue::Func(_) | SymValue::Str(_)) => {
                return ArithOutcome::Value(SymValue::Bool(false))
            }
            (UnaryOp::Not, SymValue::Expr(e)) => return ArithOutcome::Value(self.not(e)),
            (UnaryOp::Neg, SymValue::Float(bits)) => {
                return ArithOutcome::Value(SymValue::Float((-f64::from_bits(bits)).to_bits()))
            }
            (UnaryOp::Neg | UnaryOp::BitNot, SymValue::Int(_) | SymValue::Bool(_)) => {
                let x = a.as_int().unwrap_or(0);
                return match self.int_unary(op, x, class) {
                    Ok(r) => ArithOutcome::Value(Self::int_value(r, class)),
                    Err(f) => ArithOutcome::Fault(f),
                };
            }
            _ => {}
        }
        let e = self.intern(ExprNode::Unary { op, a, class });
        self.attach_quotient(e);
        ArithOutcome::Value(SymValue::Expr(e))
    }

    /// Logical negation of a Boolean expression.
    fn not(&mut self, e: ExprId) -> SymValue {
        match *self.node(e) {
            ExprNode::Unary {
                op: UnaryOp::Not,
                a,
                ..
            } if self.class_of(&a) == ValueClass::Bool => a,
            ExprNode::Compare { pred, a, b, class } => {
                let total = class.is_integral() || class == ValueClass::Ref;
                match pred {
                    ComparePred::Eq => self.compare(ComparePred::Ne, a, b, class),
                    ComparePred::Ne => self.compare(ComparePred::Eq, a, b, class),
                    // ¬(a < b) ≡ b ≤ a and ¬(a ≤ b) ≡ b < a hold in a total order only.
                    ComparePred::Lt if total => self.compare(ComparePred::Le, b, a, class),
                    ComparePred::Le if total => self.compare(ComparePred::Lt, b, a, class),
                    _ => SymValue::Expr(self.intern(ExprNode::Unary {
                        op: UnaryOp::Not,
                        a: SymValue::Expr(e),
                        class: ValueClass::Bool,
                    })),
                }
            }
            _ => SymValue::Expr(self.intern(ExprNode::Unary {
                op: UnaryOp::Not,
                a: SymValue::Expr(e),
                class: ValueClass::Bool,
            })),
        }
    }

    /// `a pred b` where `class` is the class of the operands.
    pub fn compare(&mut self, pred: ComparePred, a: SymValue, b: SymValue, class: ValueClass) -> SymValue {
        if let Some(r) = Self::fold_compare(pred, &a, &b, class) {
            return SymValue::Bool(r);
        }
        let (pred, a, b) = match pred {
            ComparePred::Gt => (ComparePred::Lt, b, a),
            ComparePred::Ge => (ComparePred::Le, b, a),
            p => (p, a, b),
        };
        let (a, b) = if matches!(pred, ComparePred::Eq | ComparePred::Ne) && b < a {
            (b, a)
        } else {
            (a, b)
        };
        SymValue::Expr(self.intern(ExprNode::Compare { pred, a, b, class }))
    }

    fn fold_compare(pred: ComparePred, a: &SymValue, b: &SymValue, class: ValueClass) -> Option<bool> {
        use SymValue as V;
        let eq_only = |equal: bool| match pred {
            ComparePred::Eq => Some(equal),
            ComparePred::Ne => Some(!equal),
            _ => None,
        };
        match (a, b) {
            (V::Float(x), V::Float(y)) => {
                let (x, y) = (f64::from_bits(*x), f64::from_bits(*y));
                Some(match pred {
                    ComparePred::Eq => x == y,
                    ComparePred::Ne => x != y,
                    ComparePred::Lt => x < y,
                    ComparePred::Le => x <= y,
                    ComparePred::Gt => x > y,
                    ComparePred::Ge => x >= y,
                })
            }
            _ if !class.is_float() && a.as_int().is_some() && b.as_int().is_some() => {
                let (x, y) = (a.as_int()?, b.as_int()?);
                Some(cmp_ints(pred, x, y))
            }
            (V::Null, V::Null) => eq_only(true),
            (V::Null, V::Ptr(_) | V::Func(_) | V::Str(_)) | (V::Ptr(_) | V::Func(_) | V::Str(_), V::Null) => {
                eq_only(false)
            }
            (V::Ptr(_) | V::Func(_), V::Int(0)) | (V::Int(0), V::Ptr(_) | V::Func(_)) => eq_only(false),
            (V::Ptr(p), V::Ptr(q)) => {
                if p.object != q.object {
                    return eq_only(false);
                }
                match (p.offset, q.offset) {
                    (SymInt::Const(x), SymInt::Const(y)) => Some(cmp_ints(pred, x, y)),
                    (x, y) if x == y => Some(cmp_ints(pred, 0, 0)),
                    _ => None,
                }
            }
            (V::Func(f), V::Func(g)) => eq_only(f == g),
            (V::Str(x), V::Str(y)) if x != y => eq_only(false),
            (V::Expr(x), V::Expr(y)) if x == y && (class.is_integral() || class == ValueClass::Ref) => {
                Some(cmp_ints(pred, 0, 0))
            }
            _ => None,
        }
    }

    /// Conversion of `a` to class `to` (IR type `ty`).
    pub fn cast(&mut self, a: SymValue, to: ValueClass, ty: TypeId) -> ArithOutcome {
        if to == ValueClass::Unknown {
            return ArithOutcome::Value(a);
        }
        match a {
            SymValue::Int(_) | SymValue::Bool(_) if to.is_integral() => {
                let x = a.as_int().unwrap_or(0);
                return match self.int_convert(x, to) {
                    Ok(r) => ArithOutcome::Value(Self::int_value(r, to)),
                    Err(f) => ArithOutcome::Fault(f),
                };
            }
            SymValue::Int(i) => {
                if let ValueClass::Float { bits } = to {
                    return ArithOutcome::Value(SymValue::Float(round_float(i as f64, bits).to_bits()));
                }
            }
            SymValue::Float(bits) => {
                let f = f64::from_bits(bits);
                match to {
                    ValueClass::Float { bits: b } => {
                        return ArithOutcome::Value(SymValue::Float(round_float(f, b).to_bits()))
                    }
                    ValueClass::Bool => return ArithOutcome::Value(SymValue::Bool(f != 0.0)),
                    ValueClass::Int { bits: b, signed } => {
                        let t = f.trunc();
                        return match int_range(b, signed) {
                            Some((lo, hi)) if f.is_finite() && t >= lo as f64 && t <= hi as f64 => {
                                ArithOutcome::Value(SymValue::Int(t as i128))
                            }
                            _ => ArithOutcome::Fault(ArithFault::NotDerivable),
                        };
                    }
                    _ => {}
                }
            }
            SymValue::Null | SymValue::Ptr(_) | SymValue::Func(_) | SymValue::Str(_) if to == ValueClass::Ref => {
                return ArithOutcome::Value(a)
            }
            SymValue::Expr(_) if self.class_of(&a) == to => return ArithOutcome::Value(a),
            _ => {}
        }
        let e = self.intern(ExprNode::Cast { a, to, ty });
        self.attach_quotient(e);
        ArithOutcome::Value(SymValue::Expr(e))
    }

    /// `a is ty`, undecided.
    pub fn type_test(&mut self, a: SymValue, ty: TypeId) -> SymValue {
        SymValue::Expr(self.intern(ExprNode::TypeTest { a, ty }))
    }

    fn attach_quotient(&mut self, e: ExprId) {
        let node = *self.node(e);
        let class = self.expr_class(e);
        if self.raw_linear(&node).is_some() {
            if let Encoding::Wrap { .. } = self.encoding(&node, class) {
                self.intern(ExprNode::Quotient(e));
            }
        }
    }

    // ── Atoms ────────────────────────────────────────────────────────────────

    /// The linear form of an integral value: a constant, or the variable of its expression.
    pub fn linear(&self, v: &SymValue) -> Option<LinExpr> {
        match v {
            SymValue::Int(i) => Some(LinExpr::constant(*i)),
            SymValue::Bool(b) => Some(LinExpr::constant(*b as i128)),
            SymValue::Expr(e) if self.expr_class(*e).is_integral() => Some(LinExpr::var(e.var())),
            _ => None,
        }
    }

    pub fn linear_int(&self, v: SymInt) -> Option<LinExpr> {
        self.linear(&v.value())
    }

    /// The reference term of a reference-valued value.
    pub fn ref_term(&mut self, v: &SymValue) -> Option<RefTerm> {
        match v {
            SymValue::Null | SymValue::Int(0) => Some(RefTerm::Null),
            SymValue::Ptr(p) if p.offset == SymInt::Const(0) => {
                Some(RefTerm::Object(self.object_const(p.object)))
            }
            SymValue::Expr(e) => match self.expr_class(*e) {
                ValueClass::Ref | ValueClass::Unknown => Some(RefTerm::Var(e.var())),
                _ => None,
            },
            _ => None,
        }
    }

    /// The atom stating that the Boolean `v` has truth value `positive`; `None` for
    /// `Undef`.
    pub fn truth(&mut self, v: SymValue, positive: bool) -> Option<Atom> {
        Some(match v {
            SymValue::Undef => return None,
            SymValue::Bool(b) => Atom::Const(b == positive),
            SymValue::Int(i) => Atom::Const((i != 0) == positive),
            SymValue::Null => Atom::Const(!positive),
            SymValue::Ptr(_) | SymValue::Func(_) | SymValue::Str(_) => Atom::Const(positive),
            SymValue::Float(bits) => Atom::Const((f64::from_bits(bits) != 0.0) == positive),
            SymValue::Expr(e) => self.expr_truth(e, positive),
        })
    }

    fn expr_truth(&mut self, e: ExprId, positive: bool) -> Atom {
        match *self.node(e) {
            ExprNode::Unary {
                op: UnaryOp::Not,
                a,
                ..
            } => match self.truth(a, !positive) {
                Some(atom) => atom,
                None => Atom::Holds(e.var(), positive),
            },
            ExprNode::Compare { pred, a, b, class } => match self.compare_atom(pred, a, b, class) {
                Some(atom) if positive => atom,
                Some(atom) => atom.negate().unwrap_or(Atom::Holds(e.var(), positive)),
                None => Atom::Holds(e.var(), positive),
            },
            _ => {
                let class = self.expr_class(e);
                if class.is_integral() {
                    let l = LinExpr::var(e.var());
                    if positive {
                        Atom::Ne(l).canonical()
                    } else {
                        Atom::Eq(l).canonical()
                    }
                } else if class == ValueClass::Ref {
                    let atom = Atom::RefNe(RefTerm::Var(e.var()), RefTerm::Null);
                    if positive {
                        atom
                    } else {
                        Atom::RefEq(RefTerm::Var(e.var()), RefTerm::Null)
                    }
                } else {
                    Atom::Holds(e.var(), positive)
                }
            }
        }
    }

    fn compare_atom(&mut self, pred: ComparePred, a: SymValue, b: SymValue, class: ValueClass) -> Option<Atom> {
        let integral_operands = self.linear(&a).is_some() && self.linear(&b).is_some();
        if class.is_integral() || (class == ValueClass::Unknown && integral_operands) {
            let (la, lb) = (self.linear(&a)?, self.linear(&b)?);
            return match pred {
                ComparePred::Eq => Atom::eq(&la, &lb),
                ComparePred::Ne => Atom::ne(&la, &lb),
                ComparePred::Lt => Atom::lt(&la, &lb),
                ComparePred::Le => Atom::le(&la, &lb),
                ComparePred::Gt => Atom::lt(&lb, &la),
                ComparePred::Ge => Atom::le(&lb, &la),
            };
        }
        if class == ValueClass::Ref || class == ValueClass::Unknown {
            let (ra, rb) = (self.ref_term(&a)?, self.ref_term(&b)?);
            return match pred {
                ComparePred::Eq => Some(Atom::RefEq(ra, rb).canonical()),
                ComparePred::Ne => Some(Atom::RefNe(ra, rb).canonical()),
                _ => None,
            };
        }
        None
    }

    // ── Exact encodings ──────────────────────────────────────────────────────

    /// The exact mathematical value of a linear operation over its operands' linear forms.
    fn raw_linear(&self, node: &ExprNode) -> Option<LinExpr> {
        match *node {
            ExprNode::Binary { op, a, b, class } if class.is_integral() => {
                let (la, lb) = (self.linear(&a)?, self.linear(&b)?);
                match op {
                    BinaryOp::Add => la.checked_add(&lb),
                    BinaryOp::Sub => la.checked_sub(&lb),
                    BinaryOp::Mul if lb.is_constant() => la.checked_scale(lb.constant),
                    BinaryOp::Mul if la.is_constant() => lb.checked_scale(la.constant),
                    BinaryOp::Shl => {
                        let k = b.as_int()?;
                        let bits = Self::width(class)? as i128;
                        if k < 0 || k >= bits || k > 125 {
                            None
                        } else {
                            la.checked_scale(1i128 << k)
                        }
                    }
                    _ => None,
                }
            }
            ExprNode::Unary { op, a, class } if class.is_integral() => {
                let la = self.linear(&a)?;
                match op {
                    UnaryOp::Neg => la.checked_scale(-1),
                    // ~x = −x − 1 in two's complement; the wrap encoding maps it into range.
                    UnaryOp::BitNot => la.checked_scale(-1)?.checked_add_const(-1),
                    UnaryOp::Plus => Some(la),
                    UnaryOp::Not => None,
                }
            }
            ExprNode::Cast { a, to, .. } if to.is_integral() && to != ValueClass::Bool => {
                if self.class_of(&a).is_integral() {
                    self.linear(&a)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// How the result of `node` (of class `class`) relates to its raw value.
    fn encoding(&self, node: &ExprNode, class: ValueClass) -> Encoding {
        match (node, class) {
            (ExprNode::Cast { a, .. }, ValueClass::Int { bits, signed }) => {
                let from = self.class_of(a);
                let fits = match (from, int_range(bits, signed)) {
                    (ValueClass::Bool, Some(_)) => true,
                    (ValueClass::Int { bits: fb, signed: fs }, Some((lo, hi))) => {
                        int_range(fb, fs).is_some_and(|(flo, fhi)| flo >= lo && fhi <= hi)
                    }
                    _ => false,
                };
                if fits {
                    Encoding::Exact
                } else if self.conversion_wraps(signed) {
                    modulus(bits).map_or(Encoding::Evaluate, |m| Encoding::Wrap { m })
                } else if self.arith.overflow(signed) == Overflow::Unbounded {
                    Encoding::Exact
                } else {
                    Encoding::Evaluate
                }
            }
            (ExprNode::Cast { .. }, _) => Encoding::Evaluate,
            (_, ValueClass::Int { bits, signed }) => match self.arith.overflow(signed) {
                Overflow::Wraps => modulus(bits).map_or(Encoding::Evaluate, |m| Encoding::Wrap { m }),
                Overflow::Undefined | Overflow::Unbounded => Encoding::Exact,
                Overflow::Unspecified => Encoding::Evaluate,
            },
            (_, ValueClass::IntAnyWidth | ValueClass::Unknown)
                if self.arith.signed_overflow == Overflow::Unbounded =>
            {
                Encoding::Exact
            }
            _ => Encoding::Evaluate,
        }
    }

    /// Value range of a linear expression from its variables' domains.
    fn linear_range(&self, e: &LinExpr) -> Option<(i128, i128)> {
        let (mut lo, mut hi) = (e.constant, e.constant);
        for &(v, c) in &e.terms {
            let d = self.class_domain(self.expr_class(ExprId(v.0)));
            let (dl, dh) = (d.lo?, d.hi?);
            let (x, y) = (c.checked_mul(dl)?, c.checked_mul(dh)?);
            lo = lo.checked_add(x.min(y))?;
            hi = hi.checked_add(x.max(y))?;
        }
        Some((lo, hi))
    }

    /// Range of the wrap quotient of `of`.
    fn quotient_domain(&self, of: ExprId) -> Domain {
        let node = *self.node(of);
        let class = self.expr_class(of);
        let (m, t) = match (self.encoding(&node, class), self.class_domain(class)) {
            (Encoding::Wrap { m }, Domain { lo: Some(l), hi: Some(h) }) => (m, (l, h)),
            _ => return Domain::UNBOUNDED,
        };
        let raw = match self.raw_linear(&node).and_then(|r| self.linear_range(&r)) {
            Some(r) => r,
            None => return Domain::UNBOUNDED,
        };
        // raw = t + m·q  ⇒  q ∈ [⌈(raw_lo − t_hi) / m⌉, ⌊(raw_hi − t_lo) / m⌋]
        match (raw.0.checked_sub(t.1), raw.1.checked_sub(t.0)) {
            (Some(a), Some(b)) => Domain::range(
                crate::symex::solver::ceil_div(a, m),
                crate::symex::solver::floor_div(b, m),
            ),
            _ => Domain::UNBOUNDED,
        }
    }

    /// Exact floor-division definitions: shifts right by a constant and, when division
    /// rounds down, division by a positive constant.
    fn floor_definition(&self, node: &ExprNode, t: &LinExpr) -> Option<Vec<Atom>> {
        let (a, divisor) = match *node {
            ExprNode::Binary { op, a, b, class } if class.is_integral() => {
                let k = b.as_int()?;
                let bits_known = Self::width(class).is_some()
                    || self.arith.signed_overflow == Overflow::Unbounded;
                if !bits_known {
                    return None;
                }
                match (op, class) {
                    (BinaryOp::Shr, _) | (BinaryOp::UShr, ValueClass::Int { signed: false, .. }) => {
                        let bits = Self::width(class).map_or(126, |b| b as i128);
                        if k < 0 || k >= bits.min(126) {
                            return None;
                        }
                        (a, 1i128 << k)
                    }
                    (BinaryOp::Div, _) if k > 0 && self.arith.division == DivisionRounding::Floor => (a, k),
                    _ => return None,
                }
            }
            _ => return None,
        };
        let la = self.linear(&a)?;
        // t = ⌊a / d⌋  ⇔  d·t − a ≤ 0  ∧  a − d·t − (d − 1) ≤ 0
        let dt = t.checked_scale(divisor)?;
        let lower = dt.checked_sub(&la)?;
        let upper = la.checked_sub(&dt)?.checked_add_const(-(divisor - 1))?;
        Some(vec![Atom::Le(lower).canonical(), Atom::Le(upper).canonical()])
    }

    /// Sound range facts of evaluable non-linear operations.
    fn range_facts(&self, node: &ExprNode, t: &LinExpr) -> Vec<Atom> {
        fn bound(out: &mut Vec<Atom>, t: &LinExpr, lo: i128, hi: i128) {
            // lo ≤ t  ∧  t ≤ hi
            if let Some(e) = LinExpr::constant(lo).checked_sub(t) {
                out.push(Atom::Le(e).canonical());
            }
            if let Some(e) = t.checked_add_const(-hi) {
                out.push(Atom::Le(e).canonical());
            }
        }
        let mut out = Vec::new();
        let ExprNode::Binary { op, a, b, class } = *node else {
            return out;
        };
        if !class.is_integral() {
            return out;
        }
        match op {
            BinaryOp::Rem => {
                if let Some(c) = b.as_int().filter(|&c| c != 0 && c != i128::MIN) {
                    let r = c.abs() - 1;
                    match self.arith.division {
                        DivisionRounding::Floor if c > 0 => bound(&mut out, t, 0, r),
                        DivisionRounding::Floor => bound(&mut out, t, -r, 0),
                        DivisionRounding::TowardZero | DivisionRounding::Unspecified => {
                            bound(&mut out, t, -r, r)
                        }
                    }
                }
            }
            BinaryOp::BitAnd => {
                let mask = [a.as_int(), b.as_int()]
                    .into_iter()
                    .flatten()
                    .filter(|&m| m >= 0)
                    .min();
                if let Some(m) = mask {
                    bound(&mut out, t, 0, m);
                }
            }
            BinaryOp::Div if self.arith.division == DivisionRounding::TowardZero => {
                // a = c·t + r with |r| < |c|  ⇒  |a − c·t| ≤ |c| − 1
                if let (Some(c), Some(la)) = (b.as_int(), self.linear(&a)) {
                    if c != 0 && c != -1 && c != i128::MIN {
                        let r = c.abs() - 1;
                        if let Some(ct) = t.checked_scale(c) {
                            if let Some(diff) = la.checked_sub(&ct) {
                                if let (Some(up), Some(down)) = (
                                    diff.checked_add_const(-r),
                                    diff.checked_scale(-1).and_then(|d| d.checked_add_const(-r)),
                                ) {
                                    out.push(Atom::Le(up).canonical());
                                    out.push(Atom::Le(down).canonical());
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        out
    }

    /// Solver variables of the integral operands of a node.
    fn operand_vars(&self, node: &ExprNode) -> Vec<Var> {
        let mut out = Vec::new();
        let mut push = |v: &SymValue| {
            if let SymValue::Expr(e) = v {
                out.push(e.var());
            }
        };
        match node {
            ExprNode::Unary { a, .. } | ExprNode::Cast { a, .. } | ExprNode::TypeTest { a, .. } => push(a),
            ExprNode::Binary { a, b, .. } | ExprNode::Compare { a, b, .. } => {
                push(a);
                push(b);
            }
            ExprNode::Quotient(of) => out.push(of.var()),
            ExprNode::Symbol(_) => {}
        }
        out
    }

    fn evaluable(&self, node: &ExprNode) -> bool {
        match node {
            ExprNode::Symbol(_) => true,
            ExprNode::Unary { a, .. } | ExprNode::Cast { a, .. } => self.class_of(a).is_integral(),
            ExprNode::Binary { class, .. } => class.is_integral(),
            ExprNode::Compare { class, a, b, .. } => {
                class.is_integral() || (self.linear(a).is_some() && self.linear(b).is_some())
            }
            ExprNode::TypeTest { .. } => false,
            ExprNode::Quotient(_) => true,
        }
    }

    fn describe_operation(&self, e: ExprId, node: &ExprNode) -> TermInfo {
        let class = self.expr_class(e);
        let domain = self.class_domain(class);
        let operands = self.operand_vars(node);
        let t = LinExpr::var(e.var());
        if let Some(raw) = self.raw_linear(node) {
            match self.encoding(node, class) {
                Encoding::Wrap { m } => {
                    if let Some(q) = self.index.get(&ExprNode::Quotient(e)) {
                        // t − raw + m·q = 0
                        let def = t
                            .checked_sub(&raw)
                            .and_then(|d| d.checked_add(&LinExpr::term(q.var(), m)));
                        if let Some(def) = def {
                            let mut ops = operands;
                            ops.push(q.var());
                            return TermInfo {
                                domain,
                                kind: TermKind::Defined,
                                definition: vec![Atom::Eq(def).canonical()],
                                operands: ops,
                            };
                        }
                    }
                }
                Encoding::Exact => {
                    if let Some(def) = t.checked_sub(&raw) {
                        return TermInfo {
                            domain,
                            kind: TermKind::Defined,
                            definition: vec![Atom::Eq(def).canonical()],
                            operands,
                        };
                    }
                }
                Encoding::Evaluate => {}
            }
        }
        if let Some(definition) = self.floor_definition(node, &t) {
            return TermInfo {
                domain,
                kind: TermKind::Defined,
                definition,
                operands,
            };
        }
        let kind = if self.evaluable(node) {
            TermKind::Computed
        } else {
            TermKind::Opaque
        };
        TermInfo {
            domain,
            kind,
            definition: self.range_facts(node, &t),
            operands,
        }
    }

    fn eval_value(&self, v: &SymValue, value: &dyn Fn(Var) -> Option<i128>, depth: u32) -> Option<i128> {
        match v {
            SymValue::Int(i) => Some(*i),
            SymValue::Bool(b) => Some(*b as i128),
            SymValue::Expr(e) => self.eval(*e, value, depth + 1),
            _ => None,
        }
    }

    /// Value of `e` given values of the symbols it depends on.
    pub fn eval(&self, e: ExprId, value: &dyn Fn(Var) -> Option<i128>, depth: u32) -> Option<i128> {
        if depth > MAX_EVAL_DEPTH {
            return None;
        }
        let node = *self.nodes.get(e.0 as usize)?;
        match node {
            ExprNode::Symbol(_) => value(e.var()),
            ExprNode::Quotient(of) => {
                let of_node = *self.nodes.get(of.0 as usize)?;
                let m = match self.encoding(&of_node, self.expr_class(of)) {
                    Encoding::Wrap { m } => m,
                    _ => return None,
                };
                let raw = self.raw_linear(&of_node)?;
                let r = raw.eval(&|w| self.eval(ExprId(w.0), value, depth + 1))?;
                let t = self.eval(of, value, depth + 1)?;
                Some((r - t) / m)
            }
            ExprNode::Binary { op, a, b, class } => {
                let (x, y) = (self.eval_value(&a, value, depth)?, self.eval_value(&b, value, depth)?);
                self.int_binary(op, x, y, class).ok()
            }
            ExprNode::Unary { op, a, class } => {
                let x = self.eval_value(&a, value, depth)?;
                self.int_unary(op, x, class).ok()
            }
            ExprNode::Compare { pred, a, b, .. } => {
                let (x, y) = (self.eval_value(&a, value, depth)?, self.eval_value(&b, value, depth)?);
                Some(cmp_ints(pred, x, y) as i128)
            }
            ExprNode::Cast { a, to, .. } => {
                if !self.class_of(&a).is_integral() {
                    return None;
                }
                let x = self.eval_value(&a, value, depth)?;
                self.int_convert(x, to).ok()
            }
            ExprNode::TypeTest { .. } => None,
        }
    }
}

impl TermOracle for SymContext {
    fn describe(&self, v: Var) -> TermInfo {
        let e = ExprId(v.0);
        let node = match self.nodes.get(e.0 as usize) {
            Some(n) => *n,
            None => {
                return TermInfo {
                    domain: Domain::UNBOUNDED,
                    kind: TermKind::Opaque,
                    definition: Vec::new(),
                    operands: Vec::new(),
                }
            }
        };
        match node {
            ExprNode::Symbol(s) => TermInfo::free(self.class_domain(self.symbols[s.0 as usize].class)),
            ExprNode::Quotient(of) => TermInfo {
                domain: self.quotient_domain(of),
                kind: TermKind::Defined,
                definition: Vec::new(),
                operands: vec![of.var()],
            },
            _ => self.describe_operation(e, &node),
        }
    }

    fn compute(&self, v: Var, value: &dyn Fn(Var) -> Option<i128>) -> Option<i128> {
        self.eval(ExprId(v.0), value, 0)
    }
}

fn cmp_ints(pred: ComparePred, x: i128, y: i128) -> bool {
    match pred {
        ComparePred::Eq => x == y,
        ComparePred::Ne => x != y,
        ComparePred::Lt => x < y,
        ComparePred::Le => x <= y,
        ComparePred::Gt => x > y,
        ComparePred::Ge => x >= y,
    }
}

/// Round an f64 result to a float type of `bits`.
fn round_float(x: f64, bits: u16) -> f64 {
    if bits == 32 {
        (x as f32) as f64
    } else {
        x
    }
}

// ── Path condition ───────────────────────────────────────────────────────────

/// Decided status of a path condition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Feasibility {
    /// Every check along the path found a verified model.
    Feasible,
    /// Some check was undecided: the path is kept, not confirmed.
    Unknown(UnknownReason),
}

/// Why a constraint is part of the path condition.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ConstraintKind {
    /// Outcome of a two-way branch.
    Branch,
    /// Outcome of a multi-way branch.
    Switch,
    /// A dereferenced reference is not null.
    NonNull,
    /// An access lies inside its object.
    InBounds,
    /// A divisor is not zero.
    NonZeroDivisor,
    /// Fixed-width arithmetic with undefined overflow did not overflow.
    NoOverflow,
    /// A symbolic reference designates a materialized input object, or does not.
    Binding,
    /// A checked assertion held.
    Assertion,
    /// A dynamic type test held or failed.
    TypeTest,
}

/// One conjunct of a path condition with where and why it was added.
#[derive(Clone, Debug)]
pub struct Constraint {
    pub atom: Atom,
    pub kind: ConstraintKind,
    pub func: FuncId,
    pub span: Span,
    /// Canonical branch literal (`analysis::conditions`) of a branch constraint, when the
    /// function's conditions were supplied.
    pub cond: Option<Cond>,
}

/// The conjunction of constraints along one path.
#[derive(Clone, Debug)]
pub struct PathCondition {
    constraints: Vec<Constraint>,
    atoms: Vec<Atom>,
    pub feasibility: Feasibility,
}

impl Default for PathCondition {
    fn default() -> Self {
        Self {
            constraints: Vec::new(),
            atoms: Vec::new(),
            feasibility: Feasibility::Feasible,
        }
    }
}

impl PathCondition {
    pub fn atoms(&self) -> &[Atom] {
        &self.atoms
    }

    pub fn constraints(&self) -> &[Constraint] {
        &self.constraints
    }

    /// True while every check along the path was decided satisfiable.
    pub fn is_decided_sat(&self) -> bool {
        self.feasibility == Feasibility::Feasible
    }

    /// Add a conjunct; trivially true and repeated atoms are not stored again.
    pub fn push(&mut self, c: Constraint) {
        if c.atom == Atom::Const(true) || self.atoms.contains(&c.atom) {
            return;
        }
        self.atoms.push(c.atom.clone());
        self.constraints.push(c);
    }

    /// Record that a check along the path was undecided.
    pub fn degrade(&mut self, reason: UnknownReason) {
        if self.feasibility == Feasibility::Feasible {
            self.feasibility = Feasibility::Unknown(reason);
        }
    }
}

// ── Approximations ───────────────────────────────────────────────────────────

/// Why a call was not executed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OpaqueReason {
    /// The callee is a declared symbol without an IR body.
    NoBody,
    /// The target is not derivable (`Callee::Unresolved`, an unknown function value).
    Unresolved,
    /// The call graph lists no target for the site.
    NoTargets,
    /// The call-depth budget was exhausted.
    DepthLimit,
}

/// A point where the path's facts stop being exact.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Approximation {
    /// The loop was summarized after `Budget::loop_unroll` iterations: values and memory it
    /// writes were replaced by fresh symbols.
    LoopSummarized { func: FuncId, head: BlockId },
    /// The call was not executed: its result is a fresh symbol and the memory reachable
    /// from its arguments and every static was invalidated.
    CallNotExecuted { site: CallSiteId, reason: OpaqueReason },
    /// One of several call-graph targets was followed for a dispatch the path does not
    /// decide (the receiver's dynamic type is not known).
    DispatchCandidate { site: CallSiteId, target: FuncId },
    /// An exception edge of a call that was not executed was followed.
    ExceptionAssumed { site: CallSiteId },
    /// A memory read could not be decided and produced a fresh symbol.
    UndecidedMemory { func: FuncId, span: Span },
    /// An operation is not derivable and produced a fresh symbol.
    NotDerivable { func: FuncId, span: Span },
    /// More input references could alias than `MemoryPolicy::max_alias_candidates`; the
    /// remaining ones are taken to be distinct objects.
    AliasingCut { func: FuncId, span: Span },
}

// ── Frames and states ────────────────────────────────────────────────────────

/// One activation record.
#[derive(Clone, Debug)]
pub struct Frame {
    pub func: FuncId,
    /// Activation number, unique along the path.
    pub activation: u32,
    /// SSA values (parameters are values too).
    pub values: HashMap<ValueId, SymValue>,
    pub block: BlockId,
    pub prev_block: Option<BlockId>,
    /// Index of the next instruction of `block` to execute.
    pub inst: usize,
    /// Call site in the caller that created this activation.
    pub call_site: Option<CallSiteId>,
    /// Caller value receiving the return value.
    pub ret_dst: Option<ValueId>,
    /// Landing block in the caller for an exception leaving this activation.
    pub caller_unwind: Option<BlockId>,
    /// The return slot, filled by `Return` before the frame is popped.
    pub ret_value: Option<SymValue>,
    /// Stack-region objects allocated by this activation.
    pub stack_objects: Vec<ObjectId>,
    /// Entries of each loop head in this activation.
    pub loop_visits: HashMap<BlockId, u32>,
    /// Loop heads whose state was summarized in this activation.
    pub summarized_loops: BTreeSet<BlockId>,
    /// Exception delivered to the current landing block (`IntrinsicKind::Exception`).
    pub exception: Option<SymValue>,
}

impl Frame {
    pub fn new(func: FuncId, activation: u32, entry: BlockId) -> Self {
        Self {
            func,
            activation,
            values: HashMap::new(),
            block: entry,
            prev_block: None,
            inst: 0,
            call_site: None,
            ret_dst: None,
            caller_unwind: None,
            ret_value: None,
            stack_objects: Vec::new(),
            loop_visits: HashMap::new(),
            summarized_loops: BTreeSet::new(),
            exception: None,
        }
    }
}

/// The complete symbolic state of one path.
#[derive(Debug)]
pub struct SymState {
    /// Path id, unique within one exploration.
    pub id: u32,
    pub frames: Vec<Frame>,
    pub heap: Heap,
    pub pc: PathCondition,
    pub trace: TraceBuf,
    /// Instructions and terminators executed along the path.
    pub steps: usize,
    pub approximations: Vec<Approximation>,
    pub faults: Vec<Fault>,
    next_activation: u32,
}

impl SymState {
    pub fn new(id: u32) -> Self {
        Self {
            id,
            frames: Vec::new(),
            heap: Heap::default(),
            pc: PathCondition::default(),
            trace: TraceBuf::default(),
            steps: 0,
            approximations: Vec::new(),
            faults: Vec::new(),
            next_activation: 0,
        }
    }

    /// A copy continuing independently under path id `id`; the trace prefix is shared.
    pub fn fork(&mut self, id: u32) -> SymState {
        let trace = self.trace.fork();
        SymState {
            id,
            frames: self.frames.clone(),
            heap: self.heap.clone(),
            pc: self.pc.clone(),
            trace,
            steps: self.steps,
            approximations: self.approximations.clone(),
            faults: self.faults.clone(),
            next_activation: self.next_activation,
        }
    }

    pub fn allocate_activation(&mut self) -> u32 {
        let a = self.next_activation;
        self.next_activation += 1;
        a
    }

    #[inline]
    pub fn depth(&self) -> usize {
        self.frames.len()
    }

    /// The innermost frame. A state is only stepped while it has one.
    #[inline]
    pub fn top(&self) -> &Frame {
        self.frames.last().expect("symbolic state without a frame")
    }

    #[inline]
    pub fn top_mut(&mut self) -> &mut Frame {
        self.frames.last_mut().expect("symbolic state without a frame")
    }

    /// Value of an SSA value of the innermost frame.
    pub fn value(&self, v: ValueId) -> Option<SymValue> {
        self.frames.last().and_then(|f| f.values.get(&v).copied())
    }

    pub fn set_value(&mut self, v: ValueId, value: SymValue) {
        self.top_mut().values.insert(v, value);
    }

    pub fn approximate(&mut self, a: Approximation) {
        if !self.approximations.contains(&a) {
            self.approximations.push(a);
        }
    }
}
