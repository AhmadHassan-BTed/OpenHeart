//! IR identifiers, source provenance and the interned type universe.
//!
//! Every IR entity is addressed by a `Copy` newtype over `u32`. Ids are dense indices into
//! the vector that owns the entity (`ModuleIr::funcs`, `FuncIr::blocks`, `FuncIr::values`,
//! ...). `u32::MAX` is reserved as the "no entity" sentinel (`INVALID`), matching the
//! sentinel convention of the symbol table and the BP AST.
//!
//! Types are hash-consed: structurally equal `IrType`s share one `TypeId`, so type equality
//! is id equality everywhere downstream.

use std::collections::HashMap;
use std::fmt;

macro_rules! ir_id {
    ($(#[$meta:meta])* $name:ident, $prefix:literal) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(pub u32);

        impl $name {
            /// Sentinel for "no entity".
            pub const INVALID: Self = Self(u32::MAX);

            /// Id of the entity at dense index `index`.
            ///
            /// Panics when the index does not fit the 32-bit id space; an IR that large
            /// cannot be addressed and silently wrapping would alias distinct entities.
            #[inline]
            pub fn new(index: usize) -> Self {
                let raw = u32::try_from(index)
                    .ok()
                    .filter(|&r| r != u32::MAX)
                    .expect(concat!(stringify!($name), " index exceeds the 32-bit id space"));
                Self(raw)
            }

            #[inline]
            pub fn index(self) -> usize {
                self.0 as usize
            }

            #[inline]
            pub fn raw(self) -> u32 {
                self.0
            }

            #[inline]
            pub fn is_valid(self) -> bool {
                self.0 != u32::MAX
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                if self.is_valid() {
                    write!(f, concat!($prefix, "{}"), self.0)
                } else {
                    f.write_str(concat!($prefix, "?"))
                }
            }
        }
    };
}

ir_id!(
    /// Interned `IrType` in a `TypeTable`.
    TypeId,
    "t"
);
ir_id!(
    /// Function of a `ModuleIr`.
    FuncId,
    "fn"
);
ir_id!(
    /// Basic block of a `FuncIr`.
    BlockId,
    "bb"
);
ir_id!(
    /// SSA value of a `FuncIr` (function-local).
    ValueId,
    "%"
);
ir_id!(
    /// Function-local instruction number: the position of the instruction when the blocks
    /// are laid out in id order (see `func::InstNumbering`).
    InstId,
    "i"
);
ir_id!(
    /// Allocation site of a `ModuleIr` (one per `Inst::Alloc`).
    AllocSiteId,
    "alloc"
);
ir_id!(
    /// Field of a `ModuleIr`.
    FieldId,
    "fld"
);
ir_id!(
    /// Module-level variable of a `ModuleIr`.
    GlobalId,
    "@g"
);
ir_id!(
    /// Call site of a `ModuleIr` (one per `Inst::Call`).
    CallSiteId,
    "cs"
);

/// Source provenance: the inclusive token range of the construct an IR entity was lowered
/// from. Token ids index the token corpus (`TokenCorpusArtifact::token_records`); `file_id`
/// is the corpus file id of those tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Span {
    pub file_id: u32,
    pub first_token: u32,
    pub last_token: u32,
}

impl Span {
    /// Provenance of a construct synthesized by lowering with no source counterpart.
    pub const NONE: Span = Span {
        file_id: u32::MAX,
        first_token: u32::MAX,
        last_token: u32::MAX,
    };

    #[inline]
    pub fn new(file_id: u32, first_token: u32, last_token: u32) -> Self {
        Self {
            file_id,
            first_token,
            last_token,
        }
    }

    #[inline]
    pub fn is_none(&self) -> bool {
        self.first_token == u32::MAX
    }

    /// Smallest span covering both. Spans of different files cannot be joined; the left
    /// span is kept in that case.
    pub fn join(self, other: Span) -> Span {
        if self.is_none() {
            return other;
        }
        if other.is_none() || other.file_id != self.file_id {
            return self;
        }
        Span {
            file_id: self.file_id,
            first_token: self.first_token.min(other.first_token),
            last_token: self.last_token.max(other.last_token),
        }
    }

    #[inline]
    pub fn contains_token(&self, token: u32) -> bool {
        !self.is_none() && self.first_token <= token && token <= self.last_token
    }

    /// True when `other` lies inside `self` (same file, nested token range).
    pub fn encloses(&self, other: &Span) -> bool {
        !self.is_none()
            && !other.is_none()
            && self.file_id == other.file_id
            && self.first_token <= other.first_token
            && other.last_token <= self.last_token
    }
}

impl fmt::Display for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_none() {
            f.write_str("<synthetic>")
        } else {
            write!(
                f,
                "f{}:t{}-t{}",
                self.file_id, self.first_token, self.last_token
            )
        }
    }
}

/// Ownership discipline of a reference type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RefKind {
    /// Non-owning, lexically bounded reference (C++ `T&`, C# `ref`, Rust `&T`).
    Borrowed,
    /// Shared ownership (garbage-collected references, `shared_ptr`, `Rc`).
    Shared,
    /// Non-owning observer that does not keep the target alive (`weak_ptr`, `WeakReference`).
    Weak,
    /// Unique ownership (`unique_ptr`, `Box`, values moved into a container).
    Owning,
}

/// Language-neutral type of an IR value or memory location.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum IrType {
    Void,
    Bool,
    Int { bits: u16, signed: bool },
    Float { bits: u16 },
    Char,
    Str,
    /// Raw address of a `TypeId` location.
    Ptr(TypeId),
    Ref { target: TypeId, kind: RefKind },
    /// Declared type: symbol id of a class, interface, enum, record or other type symbol.
    Named(u32),
    /// `len` is the static length when the array type or its allocation fixes it.
    Array { elem: TypeId, len: Option<u64> },
    Func { params: Vec<TypeId>, ret: TypeId },
    Generic { base: TypeId, args: Vec<TypeId> },
    /// Type parameter: symbol id of the type-parameter declaration.
    TypeParam(u32),
    /// The type is not derivable from the program text and the symbol table.
    Unknown,
}

/// Interning table mapping `IrType` to `TypeId`.
///
/// The first entries are fixed so the common types have constant ids.
#[derive(Debug, Clone)]
pub struct TypeTable {
    types: Vec<IrType>,
    index: HashMap<IrType, TypeId>,
}

impl Default for TypeTable {
    fn default() -> Self {
        Self::new()
    }
}

impl TypeTable {
    pub const VOID: TypeId = TypeId(0);
    pub const BOOL: TypeId = TypeId(1);
    pub const UNKNOWN: TypeId = TypeId(2);
    pub const STR: TypeId = TypeId(3);
    pub const CHAR: TypeId = TypeId(4);

    pub fn new() -> Self {
        let mut table = Self {
            types: Vec::with_capacity(64),
            index: HashMap::with_capacity(64),
        };
        // Order fixes the constant ids above.
        table.intern(IrType::Void);
        table.intern(IrType::Bool);
        table.intern(IrType::Unknown);
        table.intern(IrType::Str);
        table.intern(IrType::Char);
        table
    }

    /// Returns the id of `ty`, adding it when it is new.
    pub fn intern(&mut self, ty: IrType) -> TypeId {
        if let Some(&id) = self.index.get(&ty) {
            return id;
        }
        let id = TypeId::new(self.types.len());
        self.types.push(ty.clone());
        self.index.insert(ty, id);
        id
    }

    /// Id of `ty` when it has been interned.
    pub fn find(&self, ty: &IrType) -> Option<TypeId> {
        self.index.get(ty).copied()
    }

    /// The type behind `id`; unknown ids read as `Unknown`.
    pub fn get(&self, id: TypeId) -> &IrType {
        self.types.get(id.index()).unwrap_or(&IrType::Unknown)
    }

    pub fn len(&self) -> usize {
        self.types.len()
    }

    pub fn is_empty(&self) -> bool {
        self.types.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (TypeId, &IrType)> {
        self.types
            .iter()
            .enumerate()
            .map(|(i, t)| (TypeId::new(i), t))
    }

    pub fn named(&mut self, symbol: u32) -> TypeId {
        self.intern(IrType::Named(symbol))
    }

    pub fn int(&mut self, bits: u16, signed: bool) -> TypeId {
        self.intern(IrType::Int { bits, signed })
    }

    pub fn float(&mut self, bits: u16) -> TypeId {
        self.intern(IrType::Float { bits })
    }

    pub fn ptr_to(&mut self, target: TypeId) -> TypeId {
        self.intern(IrType::Ptr(target))
    }

    pub fn reference(&mut self, target: TypeId, kind: RefKind) -> TypeId {
        self.intern(IrType::Ref { target, kind })
    }

    pub fn array_of(&mut self, elem: TypeId, len: Option<u64>) -> TypeId {
        self.intern(IrType::Array { elem, len })
    }

    pub fn func(&mut self, params: Vec<TypeId>, ret: TypeId) -> TypeId {
        self.intern(IrType::Func { params, ret })
    }

    pub fn generic(&mut self, base: TypeId, args: Vec<TypeId>) -> TypeId {
        if args.is_empty() {
            return base;
        }
        self.intern(IrType::Generic { base, args })
    }

    #[inline]
    pub fn is_unknown(&self, id: TypeId) -> bool {
        matches!(self.get(id), IrType::Unknown)
    }

    pub fn is_integer(&self, id: TypeId) -> bool {
        matches!(self.get(id), IrType::Int { .. } | IrType::Char)
    }

    pub fn is_float(&self, id: TypeId) -> bool {
        matches!(self.get(id), IrType::Float { .. })
    }

    pub fn is_numeric(&self, id: TypeId) -> bool {
        self.is_integer(id) || self.is_float(id)
    }

    /// True for types whose values denote memory (objects, arrays, strings, functions and
    /// explicit pointers or references): the values points-to analysis tracks.
    pub fn is_reference_like(&self, id: TypeId) -> bool {
        matches!(
            self.get(id),
            IrType::Ptr(_)
                | IrType::Ref { .. }
                | IrType::Named(_)
                | IrType::Array { .. }
                | IrType::Func { .. }
                | IrType::Generic { .. }
                | IrType::TypeParam(_)
                | IrType::Str
        )
    }

    /// Element type of an array type (through references and pointers).
    pub fn element(&self, id: TypeId) -> Option<TypeId> {
        match self.get(id) {
            IrType::Array { elem, .. } => Some(*elem),
            IrType::Ref { target, .. } | IrType::Ptr(target) => match self.get(*target) {
                IrType::Array { elem, .. } => Some(*elem),
                _ => None,
            },
            _ => None,
        }
    }

    /// Target of a pointer or reference type.
    pub fn pointee(&self, id: TypeId) -> Option<TypeId> {
        match self.get(id) {
            IrType::Ptr(t) => Some(*t),
            IrType::Ref { target, .. } => Some(*target),
            _ => None,
        }
    }

    /// Declaring symbol of a nominal type, looking through references, pointers and type
    /// arguments (`Ref<List<T>>` -> `List`).
    pub fn symbol_of(&self, id: TypeId) -> Option<u32> {
        let mut cur = id;
        // Bounded by the table size: interned types only refer to earlier ids, except
        // through malformed input, which the bound turns into `None`.
        for _ in 0..=self.types.len() {
            match self.get(cur) {
                IrType::Named(sym) => return Some(*sym),
                IrType::Generic { base, .. } => cur = *base,
                IrType::Ref { target, .. } => cur = *target,
                IrType::Ptr(target) => cur = *target,
                _ => return None,
            }
        }
        None
    }

    /// Type arguments of a parameterized type (through references and pointers).
    pub fn type_args(&self, id: TypeId) -> &[TypeId] {
        match self.get(id) {
            IrType::Generic { args, .. } => args,
            IrType::Ref { target, .. } | IrType::Ptr(target) => match self.get(*target) {
                IrType::Generic { args, .. } => args,
                _ => &[],
            },
            _ => &[],
        }
    }

    /// Bit width of an integer or floating type.
    pub fn bit_width(&self, id: TypeId) -> Option<u16> {
        match self.get(id) {
            IrType::Int { bits, .. } | IrType::Float { bits } => Some(*bits),
            _ => None,
        }
    }

    /// Result type of an arithmetic operation over `a` and `b` when it follows from the
    /// operand types alone: equal types, or the wider of two numeric types (a float wins
    /// over an integer). Anything else is not derivable here.
    pub fn arithmetic_result(&mut self, a: TypeId, b: TypeId) -> TypeId {
        if a == b {
            return a;
        }
        match (self.get(a).clone(), self.get(b).clone()) {
            (IrType::Float { bits: x }, IrType::Float { bits: y }) => self.float(x.max(y)),
            (IrType::Float { .. }, IrType::Int { .. } | IrType::Char) => a,
            (IrType::Int { .. } | IrType::Char, IrType::Float { .. }) => b,
            (IrType::Int { bits: x, signed: sx }, IrType::Int { bits: y, signed: sy }) => {
                if x == y {
                    self.int(x, sx && sy)
                } else if x > y {
                    a
                } else {
                    b
                }
            }
            (IrType::Int { .. }, IrType::Char) => a,
            (IrType::Char, IrType::Int { .. }) => b,
            _ => Self::UNKNOWN,
        }
    }
}
