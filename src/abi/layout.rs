//! Language-neutral memory layout.
//!
//! Three layers live here:
//! - the declarations a frontend supplies ([`AbiProgram`], [`AggregateDecl`], [`FieldDecl`],
//!   [`BaseDecl`], [`VirtualMethodDecl`], [`EnumDecl`]) written in the neutral type vocabulary
//!   [`AbiType`], which is derived from IR types by [`AbiType::from_ir`];
//! - the result every object model produces ([`RecordLayout`]: size, alignment, data size,
//!   non-virtual size, field slots with offsets and bit positions, base subobjects, hidden
//!   slots such as vtable pointers and object headers, padding, dispatch tables);
//! - the [`LayoutEngine`], which dispatches each aggregate to the object model that governs it
//!   (`cxx`, `jvm`, `clr`), resolves nested aggregates recursively, substitutes type arguments
//!   and caches every instantiation.
//!
//! No layout is ever guessed: a missing declaration, an unbound type parameter, an array of
//! unknown bound or a self-containing aggregate makes the affected layout `NotDerivable` with
//! the reason recorded.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::sync::Arc;

use super::clr::{self, ClrModelConfig};
use super::cxx;
use super::jvm::{self, JvmModelConfig};
use super::target::{CScalar, ScalarLayout, TargetDesc};
use super::{FieldId, IrType, Span, TypeId};

// ── Object models and declaration vocabulary ─────────────────────────────────

/// The object model that governs the in-memory representation of an aggregate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ObjectModel {
    /// Native objects laid out by the target's C / C++ ABI (`TargetDesc::cxx_abi`).
    Native,
    /// Objects on a JVM heap, laid out by the reference VM model in `jvm`.
    Jvm,
    /// Objects on a CLR heap or CLR value types, laid out by the model in `clr`.
    Clr,
}

/// Declared category of an aggregate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AggregateKind {
    Struct,
    Class,
    Union,
    Interface,
    Enum,
    Record,
}

/// Where instances of a type live when a variable or field "holds" one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Storage {
    /// The value is stored in place (native aggregates, CLR value types).
    Inline,
    /// The value is a heap object; variables and fields hold a reference to it.
    Reference,
}

/// Declared access of a member or base.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Access {
    Public,
    Protected,
    Private,
    /// Package / internal / file-private visibility.
    Package,
}

/// A scalar whose width is either fixed by the language (`bits`) or by the target (`C`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AbiScalar {
    Bool { bits: u16 },
    Int { bits: u16, signed: bool },
    Float { bits: u16 },
    Char { bits: u16 },
    /// A C scalar class (`long`, `wchar_t`, `long double`, ...) sized by the target.
    C(CScalar),
}

impl AbiScalar {
    pub fn is_floating(&self) -> bool {
        match self {
            AbiScalar::Float { .. } => true,
            AbiScalar::C(c) => c.is_floating(),
            _ => false,
        }
    }
}

/// Neutral type vocabulary of the ABI models.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AbiType {
    Void,
    Scalar(AbiScalar),
    /// Native data pointer, native reference (`T&`) or function pointer, with its pointee.
    Pointer(Box<AbiType>),
    /// Reference into a garbage-collected heap, with its referent.
    ManagedRef(Box<AbiType>),
    /// Interior managed pointer (CLR byref); valid only in stack-only types.
    ManagedByRef(Box<AbiType>),
    /// An aggregate or enumeration by symbol, with type arguments for generic declarations.
    Named { symbol: u32, args: Vec<AbiType> },
    /// An inline array (`T[N]`); `len: None` is an array of unknown bound.
    Array { elem: Box<AbiType>, len: Option<u64> },
    /// C++ pointer to member of `class`.
    MemberPointer { class: u32, function: bool },
    /// A function type itself (not a pointer to one).
    Function,
    /// A type parameter of the enclosing generic declaration.
    TypeParam(u32),
    /// A type the frontend could not determine; the reason is kept.
    Unknown(String),
}

/// Language facts needed to translate IR types into [`AbiType`]s. Supplied by the frontend of
/// the language the IR was lowered from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IrTypeFacts {
    pub model: ObjectModel,
    /// Width of the language's boolean type in memory.
    pub bool_bits: u16,
    /// Width of the language's character type (`IrType::Char`).
    pub char_bits: u16,
    /// Representation of the language's built-in string type (`IrType::Str`).
    pub string_repr: AbiType,
    /// Every named type without a visible declaration is a reference type (true where the
    /// language has no user-defined value types).
    pub unknown_named_is_reference: bool,
}

impl AbiType {
    /// Translates an IR type. `lookup` resolves interned type ids of the same type table.
    pub fn from_ir(
        ty: &IrType,
        facts: &IrTypeFacts,
        program: &AbiProgram,
        lookup: &dyn Fn(TypeId) -> Option<IrType>,
    ) -> AbiType {
        let convert = |id: TypeId| -> AbiType {
            match lookup(id) {
                Some(inner) => AbiType::from_ir(&inner, facts, program, lookup),
                None => AbiType::Unknown(format!("type id {:?} is not in the type table", id)),
            }
        };
        let managed = facts.model != ObjectModel::Native;
        match ty {
            IrType::Void => AbiType::Void,
            IrType::Bool => AbiType::Scalar(AbiScalar::Bool { bits: facts.bool_bits }),
            IrType::Int { bits, signed } => AbiType::Scalar(AbiScalar::Int {
                bits: *bits,
                signed: *signed,
            }),
            IrType::Float { bits } => AbiType::Scalar(AbiScalar::Float { bits: *bits }),
            IrType::Char => AbiType::Scalar(AbiScalar::Char { bits: facts.char_bits }),
            IrType::Str => facts.string_repr.clone(),
            IrType::Ptr(target) => AbiType::Pointer(Box::new(convert(*target))),
            // Every reference kind is a single address in memory; the ownership discipline it
            // denotes is a lifetime fact (see `lifetime`), not a layout fact.
            IrType::Ref { target, .. } => {
                if managed {
                    AbiType::ManagedRef(Box::new(convert(*target)))
                } else {
                    AbiType::Pointer(Box::new(convert(*target)))
                }
            }
            IrType::Named(symbol) => Self::named(*symbol, Vec::new(), facts, program),
            IrType::Array { elem, len } => {
                let array = AbiType::Array {
                    elem: Box::new(convert(*elem)),
                    len: *len,
                };
                if managed {
                    AbiType::ManagedRef(Box::new(array))
                } else {
                    array
                }
            }
            IrType::Func { .. } => {
                if managed {
                    AbiType::ManagedRef(Box::new(AbiType::Function))
                } else {
                    AbiType::Function
                }
            }
            IrType::Generic { base, args } => {
                let base = convert(*base);
                let symbol = match &base {
                    AbiType::Named { symbol, .. } => Some(*symbol),
                    AbiType::ManagedRef(inner) => match inner.as_ref() {
                        AbiType::Named { symbol, .. } => Some(*symbol),
                        _ => None,
                    },
                    _ => None,
                };
                match symbol {
                    Some(symbol) => {
                        let args = args.iter().map(|a| convert(*a)).collect();
                        Self::named(symbol, args, facts, program)
                    }
                    None => AbiType::Unknown(format!(
                        "generic instantiation of a non-nominal type {:?}",
                        base
                    )),
                }
            }
            // Type variables of an erasing model are references to their bound's erasure.
            IrType::TypeParam(param) if facts.model == ObjectModel::Jvm => {
                AbiType::ManagedRef(Box::new(AbiType::TypeParam(*param)))
            }
            IrType::TypeParam(param) => AbiType::TypeParam(*param),
            IrType::Unknown => AbiType::Unknown("the IR type is unknown".to_string()),
        }
    }

    /// A named type, wrapped in a reference when its declaration (or the language) says
    /// instances are heap objects.
    fn named(symbol: u32, args: Vec<AbiType>, facts: &IrTypeFacts, program: &AbiProgram) -> AbiType {
        let named = AbiType::Named { symbol, args };
        if program.enums.contains_key(&symbol) {
            return named;
        }
        match program.aggregates.get(&symbol) {
            Some(decl) if decl.storage == Storage::Reference => AbiType::ManagedRef(Box::new(named)),
            Some(_) => named,
            None if facts.unknown_named_is_reference => AbiType::ManagedRef(Box::new(named)),
            None => named,
        }
    }

    /// Replaces type parameters bound by `params` with the matching `args`.
    pub fn substitute(&self, params: &[u32], args: &[AbiType]) -> AbiType {
        match self {
            AbiType::TypeParam(p) => match params.iter().position(|q| q == p) {
                Some(i) if i < args.len() => args[i].clone(),
                _ => self.clone(),
            },
            AbiType::Pointer(t) => AbiType::Pointer(Box::new(t.substitute(params, args))),
            AbiType::ManagedRef(t) => AbiType::ManagedRef(Box::new(t.substitute(params, args))),
            AbiType::ManagedByRef(t) => AbiType::ManagedByRef(Box::new(t.substitute(params, args))),
            AbiType::Named { symbol, args: inner } => AbiType::Named {
                symbol: *symbol,
                args: inner.iter().map(|a| a.substitute(params, args)).collect(),
            },
            AbiType::Array { elem, len } => AbiType::Array {
                elem: Box::new(elem.substitute(params, args)),
                len: *len,
            },
            other => other.clone(),
        }
    }

    /// The layout-relevant shape of a type argument: referents of pointers and references
    /// never affect layout, so instantiations that differ only there share one layout.
    pub fn canonical(&self) -> AbiType {
        match self {
            AbiType::Pointer(_) => AbiType::Pointer(Box::new(AbiType::Void)),
            AbiType::ManagedRef(_) => AbiType::ManagedRef(Box::new(AbiType::Void)),
            AbiType::ManagedByRef(_) => AbiType::ManagedByRef(Box::new(AbiType::Void)),
            AbiType::Named { symbol, args } => AbiType::Named {
                symbol: *symbol,
                args: args.iter().map(AbiType::canonical).collect(),
            },
            AbiType::Array { elem, len } => AbiType::Array {
                elem: Box::new(elem.canonical()),
                len: *len,
            },
            other => other.clone(),
        }
    }

    /// The aggregate or enum symbol this type names, looking through references and arrays.
    pub fn named_symbol(&self) -> Option<u32> {
        match self {
            AbiType::Named { symbol, .. } => Some(*symbol),
            AbiType::Pointer(t) | AbiType::ManagedRef(t) | AbiType::ManagedByRef(t) => t.named_symbol(),
            AbiType::Array { elem, .. } => elem.named_symbol(),
            _ => None,
        }
    }
}

/// How a special member function is declared.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MemberDecl {
    /// Not declared by the user; the language may declare it implicitly.
    NotDeclared,
    /// Declared and defined by the user.
    UserProvided,
    /// Explicitly defaulted on its first declaration.
    Defaulted,
    /// Explicitly deleted.
    Deleted,
}

impl MemberDecl {
    /// Declared in the source in any form (provided, defaulted or deleted).
    pub fn is_user_declared(self) -> bool {
        self != MemberDecl::NotDeclared
    }
}

/// The special members as declared in the source. Languages without value semantics leave
/// everything `NotDeclared`; the lifetime model derives the implicit ones.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecialMembers {
    pub default_ctor: MemberDecl,
    pub copy_ctor: MemberDecl,
    pub move_ctor: MemberDecl,
    pub copy_assign: MemberDecl,
    pub move_assign: MemberDecl,
    pub dtor: MemberDecl,
    pub dtor_is_virtual: bool,
    /// Any user-declared constructor other than copy and move constructors, including
    /// constructor templates.
    pub other_ctors: bool,
    /// The type is annotated to be passed in registers despite non-trivial members
    /// (`[[clang::trivial_abi]]`).
    pub trivial_abi: bool,
    /// Finalizer method of a garbage-collected type, when the type declares one.
    pub finalizer: Option<u32>,
}

impl Default for SpecialMembers {
    fn default() -> Self {
        Self {
            default_ctor: MemberDecl::NotDeclared,
            copy_ctor: MemberDecl::NotDeclared,
            move_ctor: MemberDecl::NotDeclared,
            copy_assign: MemberDecl::NotDeclared,
            move_assign: MemberDecl::NotDeclared,
            dtor: MemberDecl::NotDeclared,
            dtor_is_virtual: false,
            other_ctors: false,
            trivial_abi: false,
            finalizer: None,
        }
    }
}

impl SpecialMembers {
    /// Any constructor is user-declared.
    pub fn any_user_ctor(&self) -> bool {
        self.other_ctors
            || self.default_ctor.is_user_declared()
            || self.copy_ctor.is_user_declared()
            || self.move_ctor.is_user_declared()
    }
}

/// Managed layout request (`StructLayout` / `LayoutKind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ManagedLayoutKind {
    /// No request: the runtime default for the kind of type applies.
    Default,
    Sequential,
    Explicit,
    Auto,
}

/// Layout attributes attached to an aggregate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutAttrs {
    /// Maximum field alignment in bytes (`#pragma pack(n)`, `StructLayout.Pack`).
    pub pack: Option<u64>,
    /// Every member is packed to byte alignment (GNU `packed` on the record).
    pub packed: bool,
    /// Minimum alignment of the record (`alignas`, `aligned`, `__declspec(align)`).
    pub align: Option<u64>,
    /// Managed layout request.
    pub managed_layout: ManagedLayoutKind,
    /// Minimum size in bytes (`StructLayout.Size`).
    pub min_size: Option<u64>,
    /// Complete objects must have distinct addresses, so an empty record has nonzero size.
    pub distinct_address_required: bool,
    /// GCC `ms_struct`: Microsoft bitfield rules under the Itanium ABI.
    pub ms_struct: bool,
    /// `__declspec(empty_bases)`: Microsoft layout applies the empty base optimization.
    pub empty_bases: bool,
}

impl Default for LayoutAttrs {
    fn default() -> Self {
        Self {
            pack: None,
            packed: false,
            align: None,
            managed_layout: ManagedLayoutKind::Default,
            min_size: None,
            distinct_address_required: false,
            ms_struct: false,
            empty_bases: false,
        }
    }
}

/// A data member as declared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldDecl {
    pub symbol: u32,
    /// IR field id, when the field was lowered.
    pub field: Option<FieldId>,
    pub name: u32,
    pub ty: AbiType,
    pub bit_width: Option<u32>,
    pub is_static: bool,
    /// Introduced by the compiler (outer-instance links, captured variables, backing fields).
    pub is_synthetic: bool,
    pub access: Access,
    /// Explicit byte offset (`FieldOffset`).
    pub explicit_offset: Option<u64>,
    /// Minimum alignment requested on the member.
    pub align: Option<u64>,
    /// Packed member (GNU `packed` on the field).
    pub packed: bool,
    /// Potentially-overlapping member (`[[no_unique_address]]`).
    pub no_unique_address: bool,
    /// A reference member: stored as an address but bound once, never reseated.
    pub is_reference: bool,
    /// The member is `const` / `readonly` / `final`.
    pub is_const: bool,
    /// The declaration carries a default member initializer.
    pub has_initializer: bool,
    /// Ownership discipline of the declared reference type, when the language states one.
    pub ownership: Option<super::RefKind>,
    pub span: Span,
}

/// A direct base as declared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaseDecl {
    pub symbol: u32,
    pub args: Vec<AbiType>,
    pub is_virtual: bool,
    /// The base is an interface: it contributes dispatch, never storage.
    pub is_interface: bool,
    pub access: Access,
    pub span: Span,
}

/// A method that occupies a dynamic-dispatch slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VirtualMethodDecl {
    pub symbol: u32,
    pub name: u32,
    /// Interned signature key (name plus parameter types); equal keys may share a vcall offset.
    pub signature: u32,
    /// Methods of bases this method directly overrides (resolved by the symbol table).
    pub overrides: Vec<u32>,
    pub is_destructor: bool,
    pub is_pure: bool,
    /// Declared implicitly by the language (implicit virtual destructor).
    pub is_implicit: bool,
    /// Class symbol of a pointer or reference return type, for covariant-return adjustment.
    pub return_class: Option<u32>,
    pub span: Span,
}

/// An enumeration whose values are stored as an integer of `underlying` type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnumDecl {
    pub symbol: u32,
    pub underlying: AbiType,
    pub span: Span,
}

/// One aggregate type as the frontend resolved it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AggregateDecl {
    pub symbol: u32,
    pub name: u32,
    pub kind: AggregateKind,
    pub model: ObjectModel,
    pub storage: Storage,
    /// Type parameters bound by this declaration, in order.
    pub type_params: Vec<u32>,
    pub bases: Vec<BaseDecl>,
    /// Data members in declaration order.
    pub fields: Vec<FieldDecl>,
    /// Methods with dispatch slots, in declaration order.
    pub virtual_methods: Vec<VirtualMethodDecl>,
    pub special: SpecialMembers,
    pub attrs: LayoutAttrs,
    /// The definition (not only a declaration) is visible.
    pub is_complete: bool,
    pub is_abstract: bool,
    pub span: Span,
}

impl AggregateDecl {
    pub fn instance_fields(&self) -> impl Iterator<Item = (usize, &FieldDecl)> {
        self.fields.iter().enumerate().filter(|(_, f)| !f.is_static)
    }

    pub fn is_union(&self) -> bool {
        self.kind == AggregateKind::Union
    }

    /// Bases that contribute storage (interfaces only contribute dispatch).
    pub fn storage_bases(&self) -> impl Iterator<Item = &BaseDecl> {
        self.bases.iter().filter(|b| !b.is_interface)
    }
}

/// The declarations of a whole program, keyed by symbol id.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AbiProgram {
    pub aggregates: HashMap<u32, AggregateDecl>,
    pub enums: HashMap<u32, EnumDecl>,
}

impl AbiProgram {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_aggregate(&mut self, decl: AggregateDecl) {
        self.aggregates.insert(decl.symbol, decl);
    }

    pub fn add_enum(&mut self, decl: EnumDecl) {
        self.enums.insert(decl.symbol, decl);
    }

    /// Virtual bases of `symbol` in inheritance-graph order: for each direct base in
    /// declaration order, first that base's virtual bases, then the base itself when it is
    /// inherited virtually. This is the order in which a most-derived constructor initializes
    /// virtual bases and in which the Microsoft ABI allocates them.
    pub fn virtual_bases_in_order(&self, symbol: u32) -> Vec<u32> {
        let mut out = Vec::new();
        self.collect_virtual_bases(symbol, &mut out, &mut Vec::new());
        out
    }

    fn collect_virtual_bases(&self, symbol: u32, out: &mut Vec<u32>, stack: &mut Vec<u32>) {
        if stack.contains(&symbol) {
            return;
        }
        let Some(decl) = self.aggregates.get(&symbol) else {
            return;
        };
        stack.push(symbol);
        for base in decl.storage_bases() {
            let mut inner = Vec::new();
            self.collect_virtual_bases(base.symbol, &mut inner, stack);
            for v in inner {
                if !out.contains(&v) {
                    out.push(v);
                }
            }
            if base.is_virtual && !out.contains(&base.symbol) {
                out.push(base.symbol);
            }
        }
        stack.pop();
    }

    /// `derived` equals `base` or inherits from it (through storage or interface bases).
    pub fn is_derived_from(&self, derived: u32, base: u32) -> bool {
        let mut stack = vec![derived];
        let mut seen = Vec::new();
        while let Some(current) = stack.pop() {
            if current == base {
                return true;
            }
            if seen.contains(&current) {
                continue;
            }
            seen.push(current);
            if let Some(decl) = self.aggregates.get(&current) {
                stack.extend(decl.bases.iter().map(|b| b.symbol));
            }
        }
        false
    }
}

// ── Layout results ───────────────────────────────────────────────────────────

/// How a layout was obtained.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum LayoutDerivation {
    /// Computed by the algorithm the target ABI documents; compilers agree on it.
    Exact,
    /// Computed exactly under a named runtime model; the runtime chooses layouts when it loads
    /// the type, so another runtime or other runtime flags may differ.
    Model { model: String },
    /// Part of the layout is derivable; `reason` names what is not.
    Partial { reason: String },
    NotDerivable { reason: String },
}

impl LayoutDerivation {
    pub fn is_derivable(&self) -> bool {
        !matches!(self, LayoutDerivation::NotDerivable { .. })
    }
}

impl fmt::Display for LayoutDerivation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LayoutDerivation::Exact => f.write_str("exact"),
            LayoutDerivation::Model { model } => write!(f, "exact under {}", model),
            LayoutDerivation::Partial { reason } => write!(f, "partial: {}", reason),
            LayoutDerivation::NotDerivable { reason } => write!(f, "not derivable: {}", reason),
        }
    }
}

/// The algorithm that produced a layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LayoutScheme {
    /// Itanium C++ ABI record layout (System V C layout for C records).
    ItaniumCxx,
    /// Microsoft C++ ABI record layout (MSVC C layout for C records).
    MicrosoftCxx,
    /// JVM reference VM model.
    JvmReference,
    /// CoreCLR model.
    Clr,
    /// No layout could be computed.
    Unavailable,
}

/// What a slot holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SlotRepr {
    Scalar,
    Pointer,
    ManagedRef,
    ManagedByRef,
    BitField,
    Record(u32),
    Array,
    Enum(u32),
    MemberPointer,
}

/// Placement of one instance data member.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldSlot {
    /// Index into `AggregateDecl::fields`.
    pub decl_index: usize,
    pub symbol: u32,
    pub field: Option<FieldId>,
    /// Byte offset from the start of the object (for bitfields, of the first byte touched or
    /// of the allocation unit, see `bit_offset`).
    pub offset: u64,
    /// Bytes occupied (for bitfields, bytes spanned by the bits or by the allocation unit).
    pub size: u64,
    pub align: u64,
    /// For bitfields: the first bit, in allocation order, counted from `offset`.
    pub bit_offset: u32,
    pub bit_width: Option<u32>,
    pub repr: SlotRepr,
}

impl FieldSlot {
    /// First bit and one-past-last bit of the field, in allocation order.
    pub fn bit_range(&self) -> (u64, u64) {
        let start = self.offset * 8 + u64::from(self.bit_offset);
        match self.bit_width {
            Some(width) => (start, start + u64::from(width)),
            None => (start, start + self.size * 8),
        }
    }
}

/// Where a static data member lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StaticStorage {
    /// A global object with static storage duration.
    Global,
    /// A field of the runtime's class object (the JVM `java.lang.Class` mirror).
    ClassMirror,
    /// The runtime's per-type statics area (CLR).
    TypeStatics,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaticFieldSlot {
    pub decl_index: usize,
    pub symbol: u32,
    pub field: Option<FieldId>,
    pub storage: StaticStorage,
}

/// Placement of a base-class subobject.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaseSlot {
    pub symbol: u32,
    pub args: Vec<AbiType>,
    pub offset: u64,
    /// Non-virtual size of the base.
    pub size: u64,
    pub align: u64,
    pub is_virtual: bool,
    /// The base shares the derived class's vtable pointer (primary base, or an indirect
    /// primary virtual base under Itanium).
    pub is_primary: bool,
    pub is_empty: bool,
    /// A Microsoft vtordisp field precedes this virtual base.
    pub vtordisp: bool,
}

/// A slot the language does not declare but the ABI or runtime places in the object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HiddenSlotKind {
    /// Pointer to a virtual table / virtual function table.
    VTablePointer,
    /// Microsoft pointer to a virtual base table.
    VBTablePointer,
    /// Microsoft constructor displacement for a virtual base.
    VtorDisp { vbase: u32 },
    /// Managed object header word (lock, hash, GC state).
    MarkWord,
    /// Pointer to the runtime class metadata (JVM Klass).
    ClassPointer,
    /// Pointer to the runtime type descriptor (CLR MethodTable).
    MethodTablePointer,
    /// CLR object header (sync block index), located before the object reference.
    SyncBlock,
    /// Element count of an array object.
    ArrayLength,
    /// Per-dimension lengths and lower bounds of a multi-dimensional array.
    ArrayBounds { rank: u32 },
    /// Alignment padding the runtime inserts before array elements or string characters.
    HeaderPadding,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HiddenSlot {
    pub kind: HiddenSlotKind,
    /// Byte offset from the object address (negative for slots before it).
    pub offset: i64,
    pub size: u64,
    /// Class whose subobject holds the slot.
    pub owner: u32,
}

/// Bytes inside the object that no member, base or hidden slot occupies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PaddingRange {
    pub offset: u64,
    pub size: u64,
    /// The padding lies at or beyond the data size (reusable tail padding or final rounding).
    pub tail: bool,
}

/// Empty-class subobjects of one class at `offset`, repeated over array dimensions: each
/// `(stride, count)` pair adds `k * stride` for `k < count`, outermost dimension first.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EmptySubobject {
    pub class: u32,
    pub offset: u64,
    pub dims: Vec<(u64, u64)>,
}

impl EmptySubobject {
    pub fn single(class: u32, offset: u64) -> Self {
        Self {
            class,
            offset,
            dims: Vec::new(),
        }
    }

    /// The same subobjects shifted by `delta` and repeated `count` times every `stride` bytes.
    pub fn placed(&self, delta: u64, stride: u64, count: u64) -> Self {
        let mut dims = Vec::with_capacity(self.dims.len() + 1);
        if count > 1 && stride > 0 {
            dims.push((stride, count));
        }
        dims.extend(self.dims.iter().copied());
        Self {
            class: self.class,
            offset: self.offset + delta,
            dims,
        }
    }

    /// Calls `visit` for every offset below `bound`, in increasing order per dimension; stops
    /// early (returning false) when `visit` returns false.
    pub fn for_each_below(&self, bound: u64, visit: &mut dyn FnMut(u64) -> bool) -> bool {
        fn walk(
            at: u64,
            dims: &[(u64, u64)],
            bound: u64,
            visit: &mut dyn FnMut(u64) -> bool,
        ) -> bool {
            if at >= bound {
                return true;
            }
            match dims.split_first() {
                None => visit(at),
                Some((&(stride, count), rest)) => {
                    let count = if stride == 0 { 1 } else { count };
                    for k in 0..count {
                        let next = at + k * stride;
                        if next >= bound {
                            break;
                        }
                        if !walk(next, rest, bound, visit) {
                            return false;
                        }
                    }
                    true
                }
            }
        }
        walk(self.offset, &self.dims, bound, visit)
    }
}

/// A direct storage base in declaration order.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DirectBase {
    pub symbol: u32,
    pub args: Vec<AbiType>,
    pub is_virtual: bool,
}

/// Microsoft inheritance model of a class (selects member-pointer representations).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MsInheritance {
    Single,
    Multiple,
    Virtual,
    Unspecified,
}

/// Facts the Microsoft layout algorithm keeps per record for its derived classes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MsRecordInfo {
    pub has_own_vfptr: bool,
    pub has_extendable_vfptr: bool,
    pub has_vbptr: bool,
    pub vbptr_offset: Option<u64>,
    pub shared_vbptr_base: Option<u32>,
    pub primary_base: Option<u32>,
    pub required_alignment: u64,
    pub leads_with_zero_sized_base: bool,
    pub ends_with_zero_sized_object: bool,
    pub vtordisp_vbases: Vec<u32>,
    pub inheritance: MsInheritance,
}

/// Which table of the vtable group a dispatch table is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DispatchTableKind {
    /// Itanium primary virtual table (shared by the primary base chain).
    Primary,
    /// Itanium secondary virtual table of a base subobject.
    Secondary,
    /// Microsoft virtual function table of one vfptr.
    MsVfTable,
}

/// How `this` is adjusted before entering the final overrider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ThunkKind {
    None,
    /// Constant adjustment.
    NonVirtual,
    /// Adjustment loaded from a vcall offset (path through a virtual base).
    Virtual,
    /// Microsoft thunk that also applies a vtordisp displacement.
    VtorDisp,
}

/// Which entry point a dispatch slot designates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SlotVariant {
    Ordinary,
    /// Itanium complete-object destructor (D1).
    CompleteDtor,
    /// Itanium deleting destructor (D0).
    DeletingDtor,
    /// Microsoft scalar deleting destructor.
    ScalarDeletingDtor,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DispatchEntry {
    VCallOffset { method: u32, offset: i64 },
    VBaseOffset { vbase: u32, offset: i64 },
    OffsetToTop(i64),
    TypeInfo(u32),
    CompleteObjectLocator(u32),
    Method {
        /// The method that introduced the slot.
        slot: u32,
        variant: SlotVariant,
        /// The final overrider in the complete class.
        implementor: u32,
        implementor_class: u32,
        this_adjust: i64,
        thunk: ThunkKind,
        return_adjust: bool,
        is_pure: bool,
    },
    /// No unique final overrider exists (the program is ill-formed for this class).
    Ambiguous { slot: u32, candidates: Vec<u32> },
}

/// One virtual table of a complete class's vtable group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispatchTable {
    pub kind: DispatchTableKind,
    pub complete_class: u32,
    /// Class of the subobject whose vtable pointer addresses this table.
    pub subobject: u32,
    pub subobject_offset: u64,
    pub vptr_offset: u64,
    /// Index of the entry the vtable pointer points at.
    pub address_point: usize,
    pub entries: Vec<DispatchEntry>,
}

/// Microsoft virtual base table of one vbptr.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VbTable {
    pub complete_class: u32,
    pub subobject: u32,
    pub vbptr_offset: u64,
    /// Offset from the vbptr back to the start of its subobject.
    pub self_offset: i64,
    /// Offset from the vbptr to each virtual base, in table order.
    pub entries: Vec<(u32, i64)>,
}

/// The layout of one aggregate (or one instantiation of a generic aggregate).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordLayout {
    pub symbol: u32,
    pub args: Vec<AbiType>,
    pub model: ObjectModel,
    pub scheme: LayoutScheme,
    pub storage: Storage,
    /// `sizeof` of an inline value; heap instance size (header included) of a reference type.
    pub size: u64,
    pub align: u64,
    /// Size without tail padding (what a derived class may not overlap).
    pub data_size: u64,
    /// Size and alignment of the non-virtual part (without virtual bases).
    pub nv_size: u64,
    pub nv_align: u64,
    /// Bytes of runtime header before the first field (0 for native objects).
    pub header_size: u64,
    /// Heap size of the boxed form of a value type.
    pub boxed_size: Option<u64>,
    pub fields: Vec<FieldSlot>,
    pub statics: Vec<StaticFieldSlot>,
    /// Direct storage bases in declaration order.
    pub direct_bases: Vec<DirectBase>,
    /// Placed base subobjects: direct non-virtual bases and every virtual base.
    pub bases: Vec<BaseSlot>,
    pub hidden: Vec<HiddenSlot>,
    pub padding: Vec<PaddingRange>,
    pub primary_base: Option<u32>,
    pub primary_base_virtual: bool,
    pub is_empty: bool,
    pub is_dynamic: bool,
    pub pod_for_layout: bool,
    pub nv_empty_subobjects: Vec<EmptySubobject>,
    pub empty_subobjects: Vec<EmptySubobject>,
    pub largest_empty_subobject: u64,
    pub ms: Option<MsRecordInfo>,
    pub vtables: Vec<DispatchTable>,
    pub vbtables: Vec<VbTable>,
    /// Offsets of managed references (the GC reference map of the object).
    pub reference_offsets: Vec<u64>,
    pub derivation: LayoutDerivation,
    pub notes: Vec<String>,
    pub span: Option<Span>,
}

impl RecordLayout {
    /// An empty layout skeleton for `decl`'s symbol.
    pub fn skeleton(
        symbol: u32,
        args: &[AbiType],
        model: ObjectModel,
        scheme: LayoutScheme,
        storage: Storage,
        span: Option<Span>,
    ) -> Self {
        Self {
            symbol,
            args: args.to_vec(),
            model,
            scheme,
            storage,
            size: 0,
            align: 1,
            data_size: 0,
            nv_size: 0,
            nv_align: 1,
            header_size: 0,
            boxed_size: None,
            fields: Vec::new(),
            statics: Vec::new(),
            direct_bases: Vec::new(),
            bases: Vec::new(),
            hidden: Vec::new(),
            padding: Vec::new(),
            primary_base: None,
            primary_base_virtual: false,
            is_empty: false,
            is_dynamic: false,
            pod_for_layout: false,
            nv_empty_subobjects: Vec::new(),
            empty_subobjects: Vec::new(),
            largest_empty_subobject: 0,
            ms: None,
            vtables: Vec::new(),
            vbtables: Vec::new(),
            reference_offsets: Vec::new(),
            derivation: LayoutDerivation::Exact,
            notes: Vec::new(),
            span,
        }
    }

    pub fn not_derivable(
        symbol: u32,
        args: &[AbiType],
        model: ObjectModel,
        reason: String,
        span: Option<Span>,
    ) -> Self {
        let mut layout =
            Self::skeleton(symbol, args, model, LayoutScheme::Unavailable, Storage::Inline, span);
        layout.derivation = LayoutDerivation::NotDerivable { reason };
        layout
    }

    pub fn is_derivable(&self) -> bool {
        self.derivation.is_derivable()
    }

    pub fn reason(&self) -> Option<&str> {
        match &self.derivation {
            LayoutDerivation::NotDerivable { reason } | LayoutDerivation::Partial { reason } => {
                Some(reason)
            }
            _ => None,
        }
    }

    /// Offset of the direct or indirect non-virtual base `symbol` placed in this layout.
    pub fn base_offset(&self, symbol: u32) -> Option<u64> {
        self.bases
            .iter()
            .find(|b| b.symbol == symbol && !b.is_virtual)
            .map(|b| b.offset)
    }

    /// Offset of the virtual base `symbol` in a complete object of this type.
    pub fn vbase_offset(&self, symbol: u32) -> Option<u64> {
        self.bases
            .iter()
            .find(|b| b.symbol == symbol && b.is_virtual)
            .map(|b| b.offset)
    }

    pub fn field_by_symbol(&self, symbol: u32) -> Option<&FieldSlot> {
        self.fields.iter().find(|f| f.symbol == symbol)
    }

    /// Dynamic class whose non-virtual part is exactly one vtable pointer.
    pub fn is_nearly_empty(&self, pointer_size: u64) -> bool {
        self.is_dynamic && self.nv_size == pointer_size
    }

    /// Recomputes `padding` and `reference_offsets` from the placed slots.
    pub fn finish(&mut self) {
        let mut spans: Vec<(u64, u64)> = Vec::new();
        for f in &self.fields {
            let (start, end) = f.bit_range();
            if end > start {
                spans.push((start / 8, end.div_ceil(8)));
            }
        }
        for b in &self.bases {
            if b.size > 0 {
                spans.push((b.offset, b.offset + b.size));
            }
        }
        for h in &self.hidden {
            if h.offset >= 0 && h.size > 0 {
                let start = h.offset as u64;
                spans.push((start, start + h.size));
            }
        }
        spans.sort_unstable();
        let mut padding = Vec::new();
        let mut cursor = 0u64;
        for (start, end) in spans {
            if start > cursor {
                padding.push(PaddingRange {
                    offset: cursor,
                    size: start - cursor,
                    tail: cursor >= self.data_size,
                });
            }
            cursor = cursor.max(end);
        }
        if self.size > cursor {
            padding.push(PaddingRange {
                offset: cursor,
                size: self.size - cursor,
                tail: true,
            });
        }
        self.padding = padding;
        let mut refs: Vec<u64> = self
            .fields
            .iter()
            .filter(|f| f.repr == SlotRepr::ManagedRef)
            .map(|f| f.offset)
            .collect();
        refs.sort_unstable();
        self.reference_offsets = refs;
    }
}

/// Storage requirements of a value of some type when it occupies a slot.
#[derive(Debug, Clone)]
pub struct SlotType {
    pub size: u64,
    pub align: u64,
    pub repr: SlotRepr,
    /// Layout of the inline record (or of the array element record).
    pub record: Option<Arc<RecordLayout>>,
    /// The scalar class when the slot holds a scalar or an enum.
    pub scalar: Option<AbiScalar>,
    /// Number of elements when the slot holds an inline array (1 otherwise).
    pub count: u64,
}

impl SlotType {
    pub(crate) fn scalar(layout: ScalarLayout, scalar: AbiScalar) -> Self {
        Self {
            size: layout.size,
            align: layout.align,
            repr: SlotRepr::Scalar,
            record: None,
            scalar: Some(scalar),
            count: 1,
        }
    }

    pub(crate) fn word(size: u64, align: u64, repr: SlotRepr) -> Self {
        Self {
            size,
            align,
            repr,
            record: None,
            scalar: None,
            count: 1,
        }
    }
}

// ── Engine ───────────────────────────────────────────────────────────────────

/// Cache key of one instantiation: the symbol and its layout-canonical type arguments.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LayoutKey {
    pub symbol: u32,
    pub args: Vec<AbiType>,
}

/// Computes and caches the layout of every aggregate on one target.
pub struct LayoutEngine<'a> {
    pub program: &'a AbiProgram,
    pub target: &'a TargetDesc,
    pub jvm: JvmModelConfig,
    pub clr: ClrModelConfig,
    cache: HashMap<LayoutKey, Arc<RecordLayout>>,
    active: Vec<LayoutKey>,
    /// Virtual method symbol → (owning aggregate, index in `virtual_methods`).
    methods: HashMap<u32, (u32, usize)>,
}

impl<'a> LayoutEngine<'a> {
    pub fn new(program: &'a AbiProgram, target: &'a TargetDesc) -> Self {
        Self::with_models(
            program,
            target,
            JvmModelConfig::for_target(target),
            ClrModelConfig::for_target(target),
        )
    }

    pub fn with_models(
        program: &'a AbiProgram,
        target: &'a TargetDesc,
        jvm: JvmModelConfig,
        clr: ClrModelConfig,
    ) -> Self {
        let mut methods = HashMap::new();
        for decl in program.aggregates.values() {
            for (index, method) in decl.virtual_methods.iter().enumerate() {
                methods.insert(method.symbol, (decl.symbol, index));
            }
        }
        Self {
            program,
            target,
            jvm,
            clr,
            cache: HashMap::new(),
            active: Vec::new(),
            methods,
        }
    }

    pub fn aggregate(&self, symbol: u32) -> Option<&'a AggregateDecl> {
        let program = self.program;
        program.aggregates.get(&symbol)
    }

    /// The declaring aggregate and declaration of a virtual method.
    pub fn method(&self, method: u32) -> Option<(&'a AggregateDecl, &'a VirtualMethodDecl)> {
        let program = self.program;
        let (owner, index) = *self.methods.get(&method)?;
        let decl = program.aggregates.get(&owner)?;
        decl.virtual_methods.get(index).map(|m| (decl, m))
    }

    pub fn key_for(decl: &AggregateDecl, args: &[AbiType]) -> LayoutKey {
        // The JVM erases type arguments: every instantiation shares one class layout.
        let args = if decl.model == ObjectModel::Jvm {
            Vec::new()
        } else {
            args.iter().map(AbiType::canonical).collect()
        };
        LayoutKey {
            symbol: decl.symbol,
            args,
        }
    }

    /// A layout already computed, without computing it.
    pub fn cached(&self, symbol: u32, args: &[AbiType]) -> Option<Arc<RecordLayout>> {
        let decl = self.aggregate(symbol)?;
        self.cache.get(&Self::key_for(decl, args)).cloned()
    }

    /// The layout of `symbol` instantiated with `args`.
    pub fn layout(&mut self, symbol: u32, args: &[AbiType]) -> Arc<RecordLayout> {
        let program = self.program;
        let Some(decl) = program.aggregates.get(&symbol) else {
            return Arc::new(RecordLayout::not_derivable(
                symbol,
                args,
                ObjectModel::Native,
                format!("no declaration of type symbol {} is available", symbol),
                None,
            ));
        };
        let key = Self::key_for(decl, args);
        if let Some(hit) = self.cache.get(&key) {
            return Arc::clone(hit);
        }
        if self.active.contains(&key) {
            return Arc::new(RecordLayout::not_derivable(
                symbol,
                &key.args,
                decl.model,
                "the aggregate contains itself by value, so its size is unbounded".to_string(),
                Some(decl.span),
            ));
        }
        let layout = if !decl.is_complete {
            RecordLayout::not_derivable(
                symbol,
                &key.args,
                decl.model,
                "only a declaration is visible; layout requires the definition".to_string(),
                Some(decl.span),
            )
        } else if decl.model != ObjectModel::Jvm && decl.type_params.len() != key.args.len() {
            RecordLayout::not_derivable(
                symbol,
                &key.args,
                decl.model,
                format!(
                    "the declaration binds {} type parameters but {} type arguments were supplied",
                    decl.type_params.len(),
                    key.args.len()
                ),
                Some(decl.span),
            )
        } else {
            self.active.push(key.clone());
            let computed = match decl.model {
                ObjectModel::Native => cxx::layout_record(self, decl, &key.args),
                ObjectModel::Jvm => jvm::layout_class(self, decl),
                ObjectModel::Clr => clr::layout_type(self, decl, &key.args),
            };
            self.active.pop();
            computed
        };
        let layout = Arc::new(layout);
        self.cache.insert(key, Arc::clone(&layout));
        layout
    }

    /// Size of a managed reference held by an object of `model`.
    pub fn reference_size(&self, model: ObjectModel) -> u64 {
        match model {
            ObjectModel::Jvm => self.jvm.reference_size(),
            ObjectModel::Native | ObjectModel::Clr => self.target.pointer.size,
        }
    }

    fn scalar_slot(&self, scalar: &AbiScalar, model: ObjectModel) -> Result<SlotType, String> {
        let layout = match model {
            ObjectModel::Native => self.native_scalar(scalar)?,
            ObjectModel::Jvm => jvm::primitive_layout(scalar, self.target)?,
            ObjectModel::Clr => clr::primitive_layout(&self.clr, scalar, self.target)?,
        };
        Ok(SlotType::scalar(layout, scalar.clone()))
    }

    /// Size and alignment of a scalar under the target's C ABI.
    pub fn native_scalar(&self, scalar: &AbiScalar) -> Result<ScalarLayout, String> {
        let target = self.target;
        let found = match scalar {
            AbiScalar::Bool { bits } | AbiScalar::Char { bits } | AbiScalar::Int { bits, .. } => {
                target.int_of_bits(*bits)
            }
            AbiScalar::Float { bits } => target.float_of_bits(*bits),
            AbiScalar::C(c) => target.c_scalar(*c),
        };
        found.ok_or_else(|| format!("{} has no C scalar class for {:?}", target.triple, scalar))
    }

    /// Storage requirements of a value of type `ty` held by an object of `model`.
    pub fn slot_of(&mut self, ty: &AbiType, model: ObjectModel) -> Result<SlotType, String> {
        let pointer = self.target.pointer;
        match ty {
            AbiType::Void => Err("void has no object representation".to_string()),
            AbiType::Scalar(scalar) => self.scalar_slot(scalar, model),
            AbiType::Pointer(_) => Ok(SlotType::word(pointer.size, pointer.align, SlotRepr::Pointer)),
            AbiType::ManagedByRef(_) => Ok(SlotType::word(
                pointer.size,
                pointer.align,
                SlotRepr::ManagedByRef,
            )),
            AbiType::ManagedRef(_) => {
                let size = self.reference_size(model);
                Ok(SlotType::word(size, size, SlotRepr::ManagedRef))
            }
            AbiType::Named { symbol, args } => {
                let program = self.program;
                if let Some(decl) = program.enums.get(symbol) {
                    let mut slot = self.slot_of(&decl.underlying, model)?;
                    if slot.repr != SlotRepr::Scalar {
                        return Err(format!(
                            "enum symbol {} has a non-scalar underlying type",
                            symbol
                        ));
                    }
                    slot.repr = SlotRepr::Enum(*symbol);
                    return Ok(slot);
                }
                let Some(decl) = program.aggregates.get(symbol) else {
                    return Err(format!("no declaration of type symbol {} is available", symbol));
                };
                if decl.storage == Storage::Reference {
                    let size = self.reference_size(model);
                    return Ok(SlotType::word(size, size, SlotRepr::ManagedRef));
                }
                let record = self.layout(*symbol, args);
                if let LayoutDerivation::NotDerivable { reason } = &record.derivation {
                    return Err(format!("type symbol {}: {}", symbol, reason));
                }
                Ok(SlotType {
                    size: record.size,
                    align: record.align,
                    repr: SlotRepr::Record(*symbol),
                    record: Some(record),
                    scalar: None,
                    count: 1,
                })
            }
            AbiType::Array { elem, len } => {
                let Some(len) = len else {
                    return Err("an array of unknown bound has no inline size".to_string());
                };
                let inner = self.slot_of(elem, model)?;
                let size = inner
                    .size
                    .checked_mul(*len)
                    .ok_or_else(|| "the array size overflows 64 bits".to_string())?;
                Ok(SlotType {
                    size,
                    align: inner.align,
                    repr: SlotRepr::Array,
                    record: inner.record,
                    scalar: inner.scalar,
                    count: inner.count.saturating_mul(*len),
                })
            }
            AbiType::MemberPointer { class, function } => {
                cxx::member_pointer_slot(self, *class, *function)
            }
            AbiType::Function => Err("a function type has no object representation".to_string()),
            // Erased type variables are references to the erasure of their bound.
            AbiType::TypeParam(_) if model == ObjectModel::Jvm => {
                let size = self.reference_size(model);
                Ok(SlotType::word(size, size, SlotRepr::ManagedRef))
            }
            AbiType::TypeParam(param) => Err(format!(
                "type parameter {} is not bound to a type argument",
                param
            )),
            AbiType::Unknown(reason) => Err(reason.clone()),
        }
    }

    /// Lays out every non-generic aggregate (and every JVM class, whose generics are erased).
    /// Generic aggregates of other models are laid out only per instantiation, either through
    /// [`LayoutEngine::layout`] or as a by-product of an enclosing layout.
    pub fn layout_all(&mut self) -> LayoutTable {
        let program = self.program;
        let mut symbols: Vec<u32> = program.aggregates.keys().copied().collect();
        symbols.sort_unstable();
        let mut uninstantiated = Vec::new();
        for symbol in symbols {
            let decl = &program.aggregates[&symbol];
            if decl.type_params.is_empty() || decl.model == ObjectModel::Jvm {
                self.layout(symbol, &[]);
            } else {
                uninstantiated.push(symbol);
            }
        }
        LayoutTable::from_layouts(self.cache.values().cloned().collect(), uninstantiated)
    }
}

/// Every layout computed for a program.
#[derive(Debug, Clone, Default)]
pub struct LayoutTable {
    /// Layouts ordered by symbol, then by type arguments.
    pub records: Vec<Arc<RecordLayout>>,
    pub by_symbol: BTreeMap<u32, Vec<usize>>,
    /// Generic aggregates for which no instantiation was requested.
    pub uninstantiated_generics: Vec<u32>,
}

impl LayoutTable {
    pub fn from_layouts(mut records: Vec<Arc<RecordLayout>>, uninstantiated: Vec<u32>) -> Self {
        let mut keyed: Vec<(u32, String, Arc<RecordLayout>)> = records
            .drain(..)
            .map(|r| (r.symbol, format!("{:?}", r.args), r))
            .collect();
        keyed.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
        let mut table = LayoutTable {
            uninstantiated_generics: uninstantiated,
            ..Default::default()
        };
        for (symbol, _, record) in keyed {
            table
                .by_symbol
                .entry(symbol)
                .or_default()
                .push(table.records.len());
            table.records.push(record);
        }
        table
    }

    /// The layout of a non-generic aggregate, or the first instantiation of a generic one.
    pub fn get(&self, symbol: u32) -> Option<&Arc<RecordLayout>> {
        self.by_symbol
            .get(&symbol)
            .and_then(|ids| ids.first())
            .map(|&i| &self.records[i])
    }

    pub fn instantiations(&self, symbol: u32) -> impl Iterator<Item = &Arc<RecordLayout>> {
        self.by_symbol
            .get(&symbol)
            .into_iter()
            .flatten()
            .map(move |&i| &self.records[i])
    }
}
