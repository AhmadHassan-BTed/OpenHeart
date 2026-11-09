//! Pre-built .NET Base Class Library model for C# name resolution (System,
//! System.Collections.Generic, System.Linq, System.Threading.Tasks, System.IO, System.Text,
//! System.Net.Http and the namespaces their signatures depend on).
//! Loaded at Phase 3 initialization with symbol_ids >= 0xC3000000.
//!
//! Every type carries its generic parameters (with variance), constraints, base type,
//! implemented interfaces and member signatures. Members are written in C# declaration
//! syntax and parsed by `lang::csharp::semantics`, so BCL types and source types share one
//! resolution path (member lookup, overload resolution, extension methods). Enum members are
//! listed by name. Qualified names use the CLR metadata form (`System.Func`2`).

use crate::core::types::symbol::{SymbolKind, SymbolRecord, SymbolVisibility};
use crate::symbol::qual_name_table::QualifiedNameTable;
use crate::symbol::std_library::{StdLibManager, StdLibStub};
use std::collections::HashMap;

pub const CS_BASE_STUB_SYMBOL_ID: u32 = 0xC300_0000;

// Key BCL types occupy the first table slots in this exact order.
pub const CS_OBJECT_STUB_SYM_ID: u32 = CS_BASE_STUB_SYMBOL_ID;
pub const CS_STRING_STUB_SYM_ID: u32 = CS_BASE_STUB_SYMBOL_ID + 1;
pub const CS_VALUE_TYPE_STUB_SYM_ID: u32 = CS_BASE_STUB_SYMBOL_ID + 2;
pub const CS_ENUM_STUB_SYM_ID: u32 = CS_BASE_STUB_SYMBOL_ID + 3;
pub const CS_ARRAY_STUB_SYM_ID: u32 = CS_BASE_STUB_SYMBOL_ID + 4;
pub const CS_DELEGATE_STUB_SYM_ID: u32 = CS_BASE_STUB_SYMBOL_ID + 5;
pub const CS_MULTICAST_DELEGATE_STUB_SYM_ID: u32 = CS_BASE_STUB_SYMBOL_ID + 6;
pub const CS_ATTRIBUTE_STUB_SYM_ID: u32 = CS_BASE_STUB_SYMBOL_ID + 7;
pub const CS_EXCEPTION_STUB_SYM_ID: u32 = CS_BASE_STUB_SYMBOL_ID + 8;
pub const CS_VOID_STUB_SYM_ID: u32 = CS_BASE_STUB_SYMBOL_ID + 9;
pub const CS_BOOLEAN_STUB_SYM_ID: u32 = CS_BASE_STUB_SYMBOL_ID + 10;
pub const CS_CHAR_STUB_SYM_ID: u32 = CS_BASE_STUB_SYMBOL_ID + 11;
pub const CS_SBYTE_STUB_SYM_ID: u32 = CS_BASE_STUB_SYMBOL_ID + 12;
pub const CS_BYTE_STUB_SYM_ID: u32 = CS_BASE_STUB_SYMBOL_ID + 13;
pub const CS_INT16_STUB_SYM_ID: u32 = CS_BASE_STUB_SYMBOL_ID + 14;
pub const CS_UINT16_STUB_SYM_ID: u32 = CS_BASE_STUB_SYMBOL_ID + 15;
pub const CS_INT32_STUB_SYM_ID: u32 = CS_BASE_STUB_SYMBOL_ID + 16;
pub const CS_UINT32_STUB_SYM_ID: u32 = CS_BASE_STUB_SYMBOL_ID + 17;
pub const CS_INT64_STUB_SYM_ID: u32 = CS_BASE_STUB_SYMBOL_ID + 18;
pub const CS_UINT64_STUB_SYM_ID: u32 = CS_BASE_STUB_SYMBOL_ID + 19;
pub const CS_INTPTR_STUB_SYM_ID: u32 = CS_BASE_STUB_SYMBOL_ID + 20;
pub const CS_UINTPTR_STUB_SYM_ID: u32 = CS_BASE_STUB_SYMBOL_ID + 21;
pub const CS_SINGLE_STUB_SYM_ID: u32 = CS_BASE_STUB_SYMBOL_ID + 22;
pub const CS_DOUBLE_STUB_SYM_ID: u32 = CS_BASE_STUB_SYMBOL_ID + 23;
pub const CS_DECIMAL_STUB_SYM_ID: u32 = CS_BASE_STUB_SYMBOL_ID + 24;
pub const CS_NULLABLE_T_STUB_SYM_ID: u32 = CS_BASE_STUB_SYMBOL_ID + 25;
pub const CS_IDISPOSABLE_STUB_SYM_ID: u32 = CS_BASE_STUB_SYMBOL_ID + 26;
pub const CS_IENUMERABLE_STUB_SYM_ID: u32 = CS_BASE_STUB_SYMBOL_ID + 27;
pub const CS_IENUMERABLE_T_STUB_SYM_ID: u32 = CS_BASE_STUB_SYMBOL_ID + 28;
pub const CS_ICOLLECTION_T_STUB_SYM_ID: u32 = CS_BASE_STUB_SYMBOL_ID + 29;
pub const CS_ILIST_T_STUB_SYM_ID: u32 = CS_BASE_STUB_SYMBOL_ID + 30;
pub const CS_LIST_T_STUB_SYM_ID: u32 = CS_BASE_STUB_SYMBOL_ID + 31;
pub const CS_DICTIONARY_STUB_SYM_ID: u32 = CS_BASE_STUB_SYMBOL_ID + 32;
pub const CS_HASHSET_STUB_SYM_ID: u32 = CS_BASE_STUB_SYMBOL_ID + 33;
pub const CS_TASK_STUB_SYM_ID: u32 = CS_BASE_STUB_SYMBOL_ID + 34;
pub const CS_TASK_T_STUB_SYM_ID: u32 = CS_BASE_STUB_SYMBOL_ID + 35;
pub const CS_ACTION_STUB_SYM_ID: u32 = CS_BASE_STUB_SYMBOL_ID + 36;
pub const CS_FUNC_1_STUB_SYM_ID: u32 = CS_BASE_STUB_SYMBOL_ID + 37;
pub const CS_EVENT_HANDLER_STUB_SYM_ID: u32 = CS_BASE_STUB_SYMBOL_ID + 38;
pub const CS_LAZY_STUB_SYM_ID: u32 = CS_BASE_STUB_SYMBOL_ID + 39;
pub const CS_SPAN_STUB_SYM_ID: u32 = CS_BASE_STUB_SYMBOL_ID + 40;
pub const CS_IEQUATABLE_STUB_SYM_ID: u32 = CS_BASE_STUB_SYMBOL_ID + 41;
pub const CS_TYPE_STUB_SYM_ID: u32 = CS_BASE_STUB_SYMBOL_ID + 42;
pub const CS_EVENT_HANDLER_T_STUB_SYM_ID: u32 = CS_BASE_STUB_SYMBOL_ID + 43;
pub const CS_ENUMERABLE_STUB_SYM_ID: u32 = CS_BASE_STUB_SYMBOL_ID + 44;
pub const CS_VALUE_TASK_STUB_SYM_ID: u32 = CS_BASE_STUB_SYMBOL_ID + 45;
pub const CS_VALUE_TASK_T_STUB_SYM_ID: u32 = CS_BASE_STUB_SYMBOL_ID + 46;
pub const CS_STRING_BUILDER_STUB_SYM_ID: u32 = CS_BASE_STUB_SYMBOL_ID + 47;

/// Category of a BCL type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BclTypeKind {
    Class,
    Struct,
    Interface,
    Enum,
    Delegate,
}

/// Bits of `BclType::flags`.
pub struct BclTypeFlags;

impl BclTypeFlags {
    pub const STATIC: u8 = 1 << 0;
    pub const SEALED: u8 = 1 << 1;
    pub const ABSTRACT: u8 = 1 << 2;
    /// `readonly struct`.
    pub const READONLY: u8 = 1 << 3;
    /// `ref struct` (Span-like, stack-only).
    pub const REF_STRUCT: u8 = 1 << 4;
    /// Enum annotated `[Flags]`.
    pub const FLAGS_ENUM: u8 = 1 << 5;
}

/// One BCL type. `type_params` entries may carry a variance prefix (`out T`, `in T`);
/// `constraints` use the `where` clause body syntax (`T : struct`); `base`, `interfaces`
/// and `members` are C# source syntax resolved against the BCL itself.
#[derive(Debug, Clone, Copy)]
pub struct BclType {
    pub qual_name: &'static str,
    pub kind: BclTypeKind,
    pub flags: u8,
    pub type_params: &'static [&'static str],
    /// C# keyword alias (`int` for `System.Int32`).
    pub keyword: Option<&'static str>,
    /// Base class, or the underlying type of an enum.
    pub base: Option<&'static str>,
    pub interfaces: &'static [&'static str],
    pub constraints: &'static [&'static str],
    pub members: &'static [&'static str],
}

impl BclType {
    /// Containing namespace (`System.Collections.Generic`).
    pub fn namespace(&self) -> &'static str {
        let q = self.qual_name;
        match q.rfind('.') {
            Some(i) => &q[..i],
            None => "",
        }
    }

    /// Metadata name without namespace (`List`1`).
    pub fn metadata_simple_name(&self) -> &'static str {
        let q = self.qual_name;
        match q.rfind('.') {
            Some(i) => &q[i + 1..],
            None => q,
        }
    }

    /// Source name without namespace and arity suffix (`List`).
    pub fn simple_name(&self) -> &'static str {
        let m = self.metadata_simple_name();
        match m.find('`') {
            Some(i) => &m[..i],
            None => m,
        }
    }

    pub fn arity(&self) -> usize {
        self.type_params.len()
    }

    pub fn has_flag(&self, flag: u8) -> bool {
        (self.flags & flag) != 0
    }

    /// Kind in the shared Phase 3 symbol alphabet. Structs and delegates are classes there;
    /// the C# semantic model keeps them distinct.
    pub fn core_kind(&self) -> SymbolKind {
        match self.kind {
            BclTypeKind::Class | BclTypeKind::Struct | BclTypeKind::Delegate => SymbolKind::SK_CLASS,
            BclTypeKind::Interface => SymbolKind::SK_INTERFACE,
            BclTypeKind::Enum => SymbolKind::SK_ENUM,
        }
    }
}

macro_rules! bcl_opt {
    () => {
        None
    };
    ($v:literal) => {
        Some($v)
    };
}

macro_rules! bcl {
    (
        $qual:literal, $kind:ident
        $(, flags = $flags:expr)?
        $(, tp = [$($tp:literal),* $(,)?])?
        $(, kw = $kw:literal)?
        $(, base = $base:literal)?
        $(, implements = [$($iface:literal),* $(,)?])?
        $(, constraints = [$($cons:literal),* $(,)?])?
        ; [$($member:literal),* $(,)?]
    ) => {
        BclType {
            qual_name: $qual,
            kind: BclTypeKind::$kind,
            flags: 0 $(| $flags)?,
            type_params: &[$($($tp),*)?],
            keyword: bcl_opt!($($kw)?),
            base: bcl_opt!($($base)?),
            interfaces: &[$($($iface),*)?],
            constraints: &[$($($cons),*)?],
            members: &[$($member),*],
        }
    };
}

/// Integral and floating-point structs share one member surface.
macro_rules! bcl_numeric {
    ($qual:literal, $kw:literal) => {
        BclType {
            qual_name: $qual,
            kind: BclTypeKind::Struct,
            flags: BclTypeFlags::READONLY,
            type_params: &[],
            keyword: Some($kw),
            base: None,
            interfaces: &[
                "IComparable",
                concat!("IComparable<", $kw, ">"),
                concat!("IEquatable<", $kw, ">"),
                "IFormattable",
            ],
            constraints: &[],
            members: &[
                concat!("const ", $kw, " MaxValue"),
                concat!("const ", $kw, " MinValue"),
                concat!("static ", $kw, " Parse(string s)"),
                concat!("static ", $kw, " Parse(string s, IFormatProvider? provider)"),
                concat!("static bool TryParse(string? s, out ", $kw, " result)"),
                concat!("int CompareTo(", $kw, " value)"),
                "int CompareTo(object? value)",
                concat!("bool Equals(", $kw, " obj)"),
                "override bool Equals(object? obj)",
                "override int GetHashCode()",
                "override string ToString()",
                "string ToString(string? format)",
                "string ToString(string? format, IFormatProvider? provider)",
            ],
        }
    };
}

/// Delegate types: only `Invoke` is listed; the semantic model synthesizes the rest.
macro_rules! bcl_delegate {
    ($qual:literal, [$($tp:literal),*], $invoke:literal) => {
        BclType {
            qual_name: $qual,
            kind: BclTypeKind::Delegate,
            flags: BclTypeFlags::SEALED,
            type_params: &[$($tp),*],
            keyword: None,
            base: None,
            interfaces: &[],
            constraints: &[],
            members: &[$invoke],
        }
    };
}

/// Exception types: the three conventional constructors over a base exception.
macro_rules! bcl_exception {
    ($qual:literal, $name:literal, $base:literal) => {
        BclType {
            qual_name: $qual,
            kind: BclTypeKind::Class,
            flags: 0,
            type_params: &[],
            keyword: None,
            base: Some($base),
            interfaces: &[],
            constraints: &[],
            members: &[
                concat!($name, "()"),
                concat!($name, "(string? message)"),
                concat!($name, "(string? message, Exception? innerException)"),
            ],
        }
    };
}
