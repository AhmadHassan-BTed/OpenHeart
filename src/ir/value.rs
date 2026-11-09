//! Operands, constants, memory places and per-value metadata.
//!
//! The IR keeps scalar data flow in SSA values and every access to memory explicit: a
//! `Place` names an abstract memory location (a stack slot, an object field, an array
//! element, a module variable, a static field or the target of a pointer) and only
//! `Inst::Load`, `Inst::Store` and `Inst::Release` touch places.

use crate::ir::types::{FieldId, GlobalId, TypeId, ValueId};

/// Compile-time constant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConstVal {
    Bool(bool),
    Int(i128),
    /// IEEE-754 binary64 bit pattern (`f64::to_bits`), so constants stay `Eq + Hash`.
    Float(u64),
    /// Interned string (`ModuleIr::strings`).
    Str(u32),
    Null,
}

impl ConstVal {
    #[inline]
    pub fn float(value: f64) -> Self {
        ConstVal::Float(value.to_bits())
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            ConstVal::Float(bits) => Some(f64::from_bits(*bits)),
            ConstVal::Int(i) => Some(*i as f64),
            _ => None,
        }
    }

    pub fn as_int(&self) -> Option<i128> {
        match self {
            ConstVal::Int(i) => Some(*i),
            ConstVal::Bool(b) => Some(*b as i128),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            ConstVal::Bool(b) => Some(*b),
            _ => None,
        }
    }

    #[inline]
    pub fn is_null(&self) -> bool {
        matches!(self, ConstVal::Null)
    }
}

/// Instruction operand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Operand {
    Value(ValueId),
    Const(ConstVal),
    /// No definition reaches this use (read of an uninitialised variable, or a value
    /// that only flows in from an unreachable path).
    Undef,
}

impl Operand {
    #[inline]
    pub fn as_value(&self) -> Option<ValueId> {
        match self {
            Operand::Value(v) => Some(*v),
            _ => None,
        }
    }

    #[inline]
    pub fn as_const(&self) -> Option<ConstVal> {
        match self {
            Operand::Const(c) => Some(*c),
            _ => None,
        }
    }

    #[inline]
    pub fn is_undef(&self) -> bool {
        matches!(self, Operand::Undef)
    }

    #[inline]
    pub fn bool(value: bool) -> Self {
        Operand::Const(ConstVal::Bool(value))
    }

    #[inline]
    pub fn int(value: i128) -> Self {
        Operand::Const(ConstVal::Int(value))
    }

    #[inline]
    pub fn null() -> Self {
        Operand::Const(ConstVal::Null)
    }
}

impl From<ValueId> for Operand {
    fn from(v: ValueId) -> Self {
        Operand::Value(v)
    }
}

impl From<ConstVal> for Operand {
    fn from(c: ConstVal) -> Self {
        Operand::Const(c)
    }
}

/// Abstract memory location.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Place {
    /// Stack slot created by an `Inst::Alloc` with `Region::Stack` whose `dst` is the value
    /// (address-taken or by-reference-captured locals, stack objects).
    Local(ValueId),
    /// Field `field` of the object `base` refers to.
    Field { base: ValueId, field: FieldId },
    /// Element `index` of the array `base` refers to.
    Index { base: ValueId, index: Operand },
    /// Module-level variable.
    Global(GlobalId),
    /// The location the pointer or reference `base` designates.
    Deref(ValueId),
    /// Static (per-type) field `field` of the type symbol `owner`.
    Static { owner: u32, field: FieldId },
}

impl Place {
    /// The value whose target this place is relative to.
    pub fn base(&self) -> Option<ValueId> {
        match self {
            Place::Local(v) | Place::Deref(v) => Some(*v),
            Place::Field { base, .. } | Place::Index { base, .. } => Some(*base),
            Place::Global(_) | Place::Static { .. } => None,
        }
    }

    /// Field accessed by the place, for field and static places.
    pub fn field(&self) -> Option<FieldId> {
        match self {
            Place::Field { field, .. } | Place::Static { field, .. } => Some(*field),
            _ => None,
        }
    }

    /// Values the place reads to compute its address.
    pub fn uses(&self) -> Vec<ValueId> {
        let mut out = Vec::with_capacity(2);
        match self {
            Place::Local(v) | Place::Deref(v) => out.push(*v),
            Place::Field { base, .. } => out.push(*base),
            Place::Index { base, index } => {
                out.push(*base);
                if let Operand::Value(i) = index {
                    out.push(*i);
                }
            }
            Place::Global(_) | Place::Static { .. } => {}
        }
        out
    }

    /// Visits every value slot of the place (the address operands that must stay values).
    pub fn for_each_value_slot(&mut self, f: &mut dyn FnMut(&mut ValueId)) {
        match self {
            Place::Local(v) | Place::Deref(v) => f(v),
            Place::Field { base, .. } | Place::Index { base, .. } => f(base),
            Place::Global(_) | Place::Static { .. } => {}
        }
    }

    /// Visits every operand slot of the place.
    pub fn for_each_operand_slot(&mut self, f: &mut dyn FnMut(&mut Operand)) {
        if let Place::Index { index, .. } = self {
            f(index);
        }
    }

    /// True when the place denotes memory that outlives the current activation regardless
    /// of the base value (module variables and static fields).
    pub fn is_static_storage(&self) -> bool {
        matches!(self, Place::Global(_) | Place::Static { .. })
    }
}

/// Metadata of one SSA value: its type and the source variable it versions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValueInfo {
    pub ty: TypeId,
    /// Interned name of the source variable this value is a version of (`None` for
    /// temporaries).
    pub name: Option<u32>,
    /// Symbol id of that source variable, when the symbol table declares it.
    pub symbol: Option<u32>,
}

impl ValueInfo {
    #[inline]
    pub fn temp(ty: TypeId) -> Self {
        Self {
            ty,
            name: None,
            symbol: None,
        }
    }
}
