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

/// The BCL table. Slots 0..=47 hold the key types in the order of the `CS_*_STUB_SYM_ID`
/// constants; a type's symbol id is `CS_BASE_STUB_SYMBOL_ID + index`.
pub static BCL_TYPES: &[BclType] = &[
    // 0
    bcl!("System.Object", Class, kw = "object"; [
        "Object()",
        "virtual bool Equals(object? obj)",
        "static bool Equals(object? objA, object? objB)",
        "static bool ReferenceEquals(object? objA, object? objB)",
        "virtual int GetHashCode()",
        "Type GetType()",
        "virtual string? ToString()",
        "protected object MemberwiseClone()",
        "protected virtual void Finalize()",
    ]),
    // 1
    bcl!("System.String", Class, flags = BclTypeFlags::SEALED, kw = "string",
        implements = ["IEnumerable<char>", "IComparable", "IComparable<string>", "IEquatable<string>", "ICloneable"]; [
        "String(char[] value)",
        "String(char c, int count)",
        "String(ReadOnlySpan<char> value)",
        "static readonly string Empty",
        "int Length { get; }",
        "char this[int index] { get; }",
        "bool Contains(string value)",
        "bool Contains(char value)",
        "bool Contains(string value, StringComparison comparisonType)",
        "bool StartsWith(string value)",
        "bool StartsWith(char value)",
        "bool StartsWith(string value, StringComparison comparisonType)",
        "bool EndsWith(string value)",
        "bool EndsWith(char value)",
        "bool EndsWith(string value, StringComparison comparisonType)",
        "int IndexOf(char value)",
        "int IndexOf(char value, int startIndex)",
        "int IndexOf(string value)",
        "int IndexOf(string value, int startIndex)",
        "int IndexOf(string value, StringComparison comparisonType)",
        "int IndexOfAny(char[] anyOf)",
        "int LastIndexOf(char value)",
        "int LastIndexOf(string value)",
        "string Substring(int startIndex)",
        "string Substring(int startIndex, int length)",
        "string Replace(string oldValue, string? newValue)",
        "string Replace(char oldChar, char newChar)",
        "string[] Split(char separator, StringSplitOptions options = StringSplitOptions.None)",
        "string[] Split(params char[]? separator)",
        "string[] Split(string? separator, StringSplitOptions options = StringSplitOptions.None)",
        "string[] Split(string[]? separator, StringSplitOptions options)",
        "string Trim()",
        "string Trim(char trimChar)",
        "string Trim(params char[]? trimChars)",
        "string TrimStart()",
        "string TrimStart(params char[]? trimChars)",
        "string TrimEnd()",
        "string TrimEnd(params char[]? trimChars)",
        "string ToUpper()",
        "string ToLower()",
        "string ToUpperInvariant()",
        "string ToLowerInvariant()",
        "string PadLeft(int totalWidth)",
        "string PadLeft(int totalWidth, char paddingChar)",
        "string PadRight(int totalWidth)",
        "string PadRight(int totalWidth, char paddingChar)",
        "string Insert(int startIndex, string value)",
        "string Remove(int startIndex)",
        "string Remove(int startIndex, int count)",
        "char[] ToCharArray()",
        "bool Equals(string? value)",
        "bool Equals(string? value, StringComparison comparisonType)",
        "override bool Equals(object? obj)",
        "override int GetHashCode()",
        "override string ToString()",
        "int CompareTo(string? strB)",
        "int CompareTo(object? value)",
        "object Clone()",
        "IEnumerator<char> GetEnumerator()",
        "static bool IsNullOrEmpty(string? value)",
        "static bool IsNullOrWhiteSpace(string? value)",
        "static string Join(string? separator, params string?[] value)",
        "static string Join(string? separator, params object?[] values)",
        "static string Join(string? separator, IEnumerable<string?> values)",
        "static string Join(char separator, params string?[] value)",
        "static string Join<T>(string? separator, IEnumerable<T> values)",
        "static string Concat(string? str0, string? str1)",
        "static string Concat(string? str0, string? str1, string? str2)",
        "static string Concat(params string?[] values)",
        "static string Concat(object? arg0)",
        "static string Concat(IEnumerable<string?> values)",
        "static string Format(string format, object? arg0)",
        "static string Format(string format, object? arg0, object? arg1)",
        "static string Format(string format, params object?[] args)",
        "static string Format(IFormatProvider? provider, string format, params object?[] args)",
        "static int Compare(string? strA, string? strB)",
        "static int Compare(string? strA, string? strB, bool ignoreCase)",
        "static int Compare(string? strA, string? strB, StringComparison comparisonType)",
        "static int CompareOrdinal(string? strA, string? strB)",
        "static bool Equals(string? a, string? b)",
        "static bool Equals(string? a, string? b, StringComparison comparisonType)",
        "static bool operator ==(string? a, string? b)",
        "static bool operator !=(string? a, string? b)",
        "static implicit operator ReadOnlySpan<char>(string? value)",
    ]),
    // 2
    bcl!("System.ValueType", Class, flags = BclTypeFlags::ABSTRACT; [
        "protected ValueType()",
        "override bool Equals(object? obj)",
        "override int GetHashCode()",
        "override string? ToString()",
    ]),
    // 3
    bcl!("System.Enum", Class, flags = BclTypeFlags::ABSTRACT, base = "ValueType",
        implements = ["IComparable", "IFormattable"]; [
        "protected Enum()",
        "bool HasFlag(Enum flag)",
        "int CompareTo(object? target)",
        "override bool Equals(object? obj)",
        "override int GetHashCode()",
        "override string ToString()",
        "string ToString(string? format)",
        "string ToString(string? format, IFormatProvider? provider)",
        "static string? GetName(Type enumType, object value)",
        "static string? GetName<TEnum>(TEnum value) where TEnum : struct, Enum",
        "static string[] GetNames(Type enumType)",
        "static string[] GetNames<TEnum>() where TEnum : struct, Enum",
        "static Array GetValues(Type enumType)",
        "static TEnum[] GetValues<TEnum>() where TEnum : struct, Enum",
        "static Type GetUnderlyingType(Type enumType)",
        "static object Parse(Type enumType, string value)",
        "static object Parse(Type enumType, string value, bool ignoreCase)",
        "static TEnum Parse<TEnum>(string value) where TEnum : struct",
        "static TEnum Parse<TEnum>(string value, bool ignoreCase) where TEnum : struct",
        "static bool TryParse<TEnum>(string? value, out TEnum result) where TEnum : struct",
        "static bool TryParse<TEnum>(string? value, bool ignoreCase, out TEnum result) where TEnum : struct",
        "static bool IsDefined(Type enumType, object value)",
        "static bool IsDefined<TEnum>(TEnum value) where TEnum : struct, Enum",
        "static object ToObject(Type enumType, int value)",
    ]),
    // 4
    bcl!("System.Array", Class, flags = BclTypeFlags::ABSTRACT,
        implements = ["IEnumerable", "ICloneable"]; [
        "int Length { get; }",
        "long LongLength { get; }",
        "int Rank { get; }",
        "int GetLength(int dimension)",
        "int GetLowerBound(int dimension)",
        "int GetUpperBound(int dimension)",
        "object? GetValue(int index)",
        "object? GetValue(params int[] indices)",
        "void SetValue(object? value, int index)",
        "void CopyTo(Array array, int index)",
        "object Clone()",
        "IEnumerator GetEnumerator()",
        "static void Sort(Array array)",
        "static void Sort<T>(T[] array)",
        "static void Sort<T>(T[] array, Comparison<T> comparison)",
        "static void Sort<T>(T[] array, IComparer<T>? comparer)",
        "static void Sort<TKey, TValue>(TKey[] keys, TValue[]? items)",
        "static int IndexOf<T>(T[] array, T value)",
        "static int LastIndexOf<T>(T[] array, T value)",
        "static void Reverse(Array array)",
        "static void Reverse<T>(T[] array)",
        "static T[] Empty<T>()",
        "static void Resize<T>(ref T[]? array, int newSize)",
        "static void Copy(Array sourceArray, Array destinationArray, int length)",
        "static void Copy(Array sourceArray, int sourceIndex, Array destinationArray, int destinationIndex, int length)",
        "static void Clear(Array array)",
        "static void Clear(Array array, int index, int length)",
        "static void Fill<T>(T[] array, T value)",
        "static bool Exists<T>(T[] array, Predicate<T> match)",
        "static T? Find<T>(T[] array, Predicate<T> match)",
        "static T[] FindAll<T>(T[] array, Predicate<T> match)",
        "static int FindIndex<T>(T[] array, Predicate<T> match)",
        "static T? FindLast<T>(T[] array, Predicate<T> match)",
        "static bool TrueForAll<T>(T[] array, Predicate<T> match)",
        "static int BinarySearch<T>(T[] array, T value)",
        "static void ForEach<T>(T[] array, Action<T> action)",
        "static TOutput[] ConvertAll<TInput, TOutput>(TInput[] array, Converter<TInput, TOutput> converter)",
        "static Array CreateInstance(Type elementType, int length)",
        "static ReadOnlyCollection<T> AsReadOnly<T>(T[] array)",
    ]),
    // 5
    bcl!("System.Delegate", Class, flags = BclTypeFlags::ABSTRACT, implements = ["ICloneable"]; [
        "object? Target { get; }",
        "object? DynamicInvoke(params object?[]? args)",
        "virtual object Clone()",
        "virtual Delegate[] GetInvocationList()",
        "override bool Equals(object? obj)",
        "override int GetHashCode()",
        "static Delegate? Combine(Delegate? a, Delegate? b)",
        "static Delegate? Combine(params Delegate?[]? delegates)",
        "static Delegate? Remove(Delegate? source, Delegate? value)",
        "static Delegate? RemoveAll(Delegate? source, Delegate? value)",
        "static bool operator ==(Delegate? d1, Delegate? d2)",
        "static bool operator !=(Delegate? d1, Delegate? d2)",
    ]),
    // 6
    bcl!("System.MulticastDelegate", Class, flags = BclTypeFlags::ABSTRACT, base = "Delegate"; [
        "sealed override Delegate[] GetInvocationList()",
        "sealed override bool Equals(object? obj)",
        "sealed override int GetHashCode()",
        "static bool operator ==(MulticastDelegate? d1, MulticastDelegate? d2)",
        "static bool operator !=(MulticastDelegate? d1, MulticastDelegate? d2)",
    ]),
    // 7
    bcl!("System.Attribute", Class, flags = BclTypeFlags::ABSTRACT; [
        "protected Attribute()",
        "virtual object TypeId { get; }",
        "virtual bool IsDefaultAttribute()",
        "virtual bool Match(object? obj)",
        "override bool Equals(object? obj)",
        "override int GetHashCode()",
        "static Attribute? GetCustomAttribute(Type element, Type attributeType)",
        "static Attribute[] GetCustomAttributes(Type element)",
        "static bool IsDefined(Type element, Type attributeType)",
    ]),
    // 8
    bcl!("System.Exception", Class; [
        "Exception()",
        "Exception(string? message)",
        "Exception(string? message, Exception? innerException)",
        "virtual string Message { get; }",
        "Exception? InnerException { get; }",
        "virtual string? StackTrace { get; }",
        "virtual string? Source { get; set; }",
        "virtual string? HelpLink { get; set; }",
        "int HResult { get; set; }",
        "virtual Exception GetBaseException()",
        "override string ToString()",
    ]),
    // 9
    bcl!("System.Void", Struct, kw = "void"; []),
    // 10
    bcl!("System.Boolean", Struct, flags = BclTypeFlags::READONLY, kw = "bool",
        implements = ["IComparable", "IComparable<bool>", "IEquatable<bool>"]; [
        "static readonly string TrueString",
        "static readonly string FalseString",
        "static bool Parse(string value)",
        "static bool TryParse(string? value, out bool result)",
        "int CompareTo(bool value)",
        "int CompareTo(object? obj)",
        "bool Equals(bool obj)",
        "override bool Equals(object? obj)",
        "override int GetHashCode()",
        "override string ToString()",
    ]),
    // 11
    bcl!("System.Char", Struct, flags = BclTypeFlags::READONLY, kw = "char",
        implements = ["IComparable", "IComparable<char>", "IEquatable<char>"]; [
        "const char MaxValue",
        "const char MinValue",
        "static bool IsDigit(char c)",
        "static bool IsLetter(char c)",
        "static bool IsLetterOrDigit(char c)",
        "static bool IsWhiteSpace(char c)",
        "static bool IsUpper(char c)",
        "static bool IsLower(char c)",
        "static bool IsPunctuation(char c)",
        "static bool IsControl(char c)",
        "static bool IsNumber(char c)",
        "static bool IsSurrogate(char c)",
        "static char ToUpper(char c)",
        "static char ToLower(char c)",
        "static char ToUpperInvariant(char c)",
        "static char ToLowerInvariant(char c)",
        "static double GetNumericValue(char c)",
        "static char Parse(string s)",
        "static bool TryParse(string? s, out char result)",
        "int CompareTo(char value)",
        "bool Equals(char obj)",
        "override bool Equals(object? obj)",
        "override int GetHashCode()",
        "override string ToString()",
        "static string ToString(char c)",
    ]),
    // 12..=24
    bcl_numeric!("System.SByte", "sbyte"),
    bcl_numeric!("System.Byte", "byte"),
    bcl_numeric!("System.Int16", "short"),
    bcl_numeric!("System.UInt16", "ushort"),
    bcl_numeric!("System.Int32", "int"),
    bcl_numeric!("System.UInt32", "uint"),
    bcl_numeric!("System.Int64", "long"),
    bcl_numeric!("System.UInt64", "ulong"),
    bcl_numeric!("System.IntPtr", "nint"),
    bcl_numeric!("System.UIntPtr", "nuint"),
    bcl_numeric!("System.Single", "float"),
    bcl_numeric!("System.Double", "double"),
    bcl_numeric!("System.Decimal", "decimal"),
    // 25
    bcl!("System.Nullable`1", Struct, tp = ["T"], constraints = ["T : struct"]; [
        "Nullable(T value)",
        "bool HasValue { get; }",
        "T Value { get; }",
        "T GetValueOrDefault()",
        "T GetValueOrDefault(T defaultValue)",
        "override bool Equals(object? other)",
        "override int GetHashCode()",
        "override string? ToString()",
        "static implicit operator T?(T value)",
        "static explicit operator T(T? value)",
    ]),
    // 26
    bcl!("System.IDisposable", Interface; [
        "void Dispose()",
    ]),
    // 27
    bcl!("System.Collections.IEnumerable", Interface; [
        "IEnumerator GetEnumerator()",
    ]),
    // 28
    bcl!("System.Collections.Generic.IEnumerable`1", Interface, tp = ["out T"], implements = ["IEnumerable"]; [
        "new IEnumerator<T> GetEnumerator()",
    ]),
    // 29
    bcl!("System.Collections.Generic.ICollection`1", Interface, tp = ["T"], implements = ["IEnumerable<T>"]; [
        "int Count { get; }",
        "bool IsReadOnly { get; }",
        "void Add(T item)",
        "void Clear()",
        "bool Contains(T item)",
        "void CopyTo(T[] array, int arrayIndex)",
        "bool Remove(T item)",
    ]),
    // 30
    bcl!("System.Collections.Generic.IList`1", Interface, tp = ["T"], implements = ["ICollection<T>", "IEnumerable<T>"]; [
        "T this[int index] { get; set; }",
        "int IndexOf(T item)",
        "void Insert(int index, T item)",
        "void RemoveAt(int index)",
    ]),
    // 31
    bcl!("System.Collections.Generic.List`1", Class, tp = ["T"],
        implements = ["IList<T>", "ICollection<T>", "IEnumerable<T>", "IReadOnlyList<T>", "IReadOnlyCollection<T>"]; [
        "List()",
        "List(int capacity)",
        "List(IEnumerable<T> collection)",
        "int Count { get; }",
        "int Capacity { get; set; }",
        "T this[int index] { get; set; }",
        "void Add(T item)",
        "void AddRange(IEnumerable<T> collection)",
        "void Insert(int index, T item)",
        "void InsertRange(int index, IEnumerable<T> collection)",
        "bool Remove(T item)",
        "void RemoveAt(int index)",
        "int RemoveAll(Predicate<T> match)",
        "void RemoveRange(int index, int count)",
        "void Clear()",
        "bool Contains(T item)",
        "int IndexOf(T item)",
        "int IndexOf(T item, int index)",
        "int LastIndexOf(T item)",
        "T? Find(Predicate<T> match)",
        "List<T> FindAll(Predicate<T> match)",
        "int FindIndex(Predicate<T> match)",
        "T? FindLast(Predicate<T> match)",
        "int FindLastIndex(Predicate<T> match)",
        "bool Exists(Predicate<T> match)",
        "bool TrueForAll(Predicate<T> match)",
        "void ForEach(Action<T> action)",
        "void Sort()",
        "void Sort(Comparison<T> comparison)",
        "void Sort(IComparer<T>? comparer)",
        "void Reverse()",
        "void Reverse(int index, int count)",
        "T[] ToArray()",
        "List<T> GetRange(int index, int count)",
        "List<T> Slice(int start, int length)",
        "List<TOutput> ConvertAll<TOutput>(Converter<T, TOutput> converter)",
        "int BinarySearch(T item)",
        "int BinarySearch(T item, IComparer<T>? comparer)",
        "void CopyTo(T[] array)",
        "void CopyTo(T[] array, int arrayIndex)",
        "ReadOnlyCollection<T> AsReadOnly()",
        "IEnumerator<T> GetEnumerator()",
        "int EnsureCapacity(int capacity)",
        "void TrimExcess()",
    ]),
    // 32
    bcl!("System.Collections.Generic.Dictionary`2", Class, tp = ["TKey", "TValue"],
        implements = [
            "IDictionary<TKey, TValue>",
            "ICollection<KeyValuePair<TKey, TValue>>",
            "IEnumerable<KeyValuePair<TKey, TValue>>",
            "IReadOnlyDictionary<TKey, TValue>",
            "IReadOnlyCollection<KeyValuePair<TKey, TValue>>",
        ],
        constraints = ["TKey : notnull"]; [
        "Dictionary()",
        "Dictionary(int capacity)",
        "Dictionary(IEqualityComparer<TKey>? comparer)",
        "Dictionary(int capacity, IEqualityComparer<TKey>? comparer)",
        "Dictionary(IDictionary<TKey, TValue> dictionary)",
        "Dictionary(IEnumerable<KeyValuePair<TKey, TValue>> collection)",
        "int Count { get; }",
        "TValue this[TKey key] { get; set; }",
        "ICollection<TKey> Keys { get; }",
        "ICollection<TValue> Values { get; }",
        "IEqualityComparer<TKey> Comparer { get; }",
        "void Add(TKey key, TValue value)",
        "bool TryAdd(TKey key, TValue value)",
        "bool Remove(TKey key)",
        "bool Remove(TKey key, out TValue value)",
        "bool ContainsKey(TKey key)",
        "bool ContainsValue(TValue value)",
        "bool TryGetValue(TKey key, out TValue value)",
        "void Clear()",
        "IEnumerator<KeyValuePair<TKey, TValue>> GetEnumerator()",
        "int EnsureCapacity(int capacity)",
        "void TrimExcess()",
    ]),
    // 33
    bcl!("System.Collections.Generic.HashSet`1", Class, tp = ["T"],
        implements = ["ISet<T>", "ICollection<T>", "IEnumerable<T>", "IReadOnlySet<T>", "IReadOnlyCollection<T>"]; [
        "HashSet()",
        "HashSet(int capacity)",
        "HashSet(IEnumerable<T> collection)",
        "HashSet(IEqualityComparer<T>? comparer)",
        "HashSet(IEnumerable<T> collection, IEqualityComparer<T>? comparer)",
        "int Count { get; }",
        "IEqualityComparer<T> Comparer { get; }",
        "bool Add(T item)",
        "bool Remove(T item)",
        "int RemoveWhere(Predicate<T> match)",
        "bool Contains(T item)",
        "void Clear()",
        "void CopyTo(T[] array)",
        "void CopyTo(T[] array, int arrayIndex)",
        "void UnionWith(IEnumerable<T> other)",
        "void IntersectWith(IEnumerable<T> other)",
        "void ExceptWith(IEnumerable<T> other)",
        "void SymmetricExceptWith(IEnumerable<T> other)",
        "bool IsSubsetOf(IEnumerable<T> other)",
        "bool IsProperSubsetOf(IEnumerable<T> other)",
        "bool IsSupersetOf(IEnumerable<T> other)",
        "bool IsProperSupersetOf(IEnumerable<T> other)",
        "bool Overlaps(IEnumerable<T> other)",
        "bool SetEquals(IEnumerable<T> other)",
        "bool TryGetValue(T equalValue, out T actualValue)",
        "IEnumerator<T> GetEnumerator()",
        "int EnsureCapacity(int capacity)",
        "void TrimExcess()",
    ]),
    // 34
    bcl!("System.Threading.Tasks.Task", Class, implements = ["IAsyncResult", "IDisposable"]; [
        "Task(Action action)",
        "Task(Action action, CancellationToken cancellationToken)",
        "Task(Action<object?> action, object? state)",
        "int Id { get; }",
        "bool IsCompleted { get; }",
        "bool IsCompletedSuccessfully { get; }",
        "bool IsFaulted { get; }",
        "bool IsCanceled { get; }",
        "TaskStatus Status { get; }",
        "AggregateException? Exception { get; }",
        "object? AsyncState { get; }",
        "static Task CompletedTask { get; }",
        "static int? CurrentId { get; }",
        "void Start()",
        "void Wait()",
        "bool Wait(int millisecondsTimeout)",
        "bool Wait(TimeSpan timeout)",
        "void Wait(CancellationToken cancellationToken)",
        "TaskAwaiter GetAwaiter()",
        "ConfiguredTaskAwaitable ConfigureAwait(bool continueOnCapturedContext)",
        "Task ContinueWith(Action<Task> continuationAction)",
        "Task ContinueWith(Action<Task> continuationAction, CancellationToken cancellationToken)",
        "Task<TResult> ContinueWith<TResult>(Func<Task, TResult> continuationFunction)",
        "Task WaitAsync(TimeSpan timeout)",
        "Task WaitAsync(CancellationToken cancellationToken)",
        "void Dispose()",
        "static Task Run(Action action)",
        "static Task Run(Action action, CancellationToken cancellationToken)",
        "static Task Run(Func<Task?> function)",
        "static Task Run(Func<Task?> function, CancellationToken cancellationToken)",
        "static Task<TResult> Run<TResult>(Func<TResult> function)",
        "static Task<TResult> Run<TResult>(Func<TResult> function, CancellationToken cancellationToken)",
        "static Task<TResult> Run<TResult>(Func<Task<TResult>?> function)",
        "static Task<TResult> Run<TResult>(Func<Task<TResult>?> function, CancellationToken cancellationToken)",
        "static Task Delay(int millisecondsDelay)",
        "static Task Delay(int millisecondsDelay, CancellationToken cancellationToken)",
        "static Task Delay(TimeSpan delay)",
        "static Task Delay(TimeSpan delay, CancellationToken cancellationToken)",
        "static Task WhenAll(IEnumerable<Task> tasks)",
        "static Task WhenAll(params Task[] tasks)",
        "static Task<TResult[]> WhenAll<TResult>(IEnumerable<Task<TResult>> tasks)",
        "static Task<TResult[]> WhenAll<TResult>(params Task<TResult>[] tasks)",
        "static Task<Task> WhenAny(IEnumerable<Task> tasks)",
        "static Task<Task> WhenAny(params Task[] tasks)",
        "static Task<Task<TResult>> WhenAny<TResult>(IEnumerable<Task<TResult>> tasks)",
        "static Task<Task<TResult>> WhenAny<TResult>(params Task<TResult>[] tasks)",
        "static void WaitAll(params Task[] tasks)",
        "static bool WaitAll(Task[] tasks, int millisecondsTimeout)",
        "static int WaitAny(params Task[] tasks)",
        "static Task<TResult> FromResult<TResult>(TResult result)",
        "static Task FromException(Exception exception)",
        "static Task<TResult> FromException<TResult>(Exception exception)",
        "static Task FromCanceled(CancellationToken cancellationToken)",
        "static Task<TResult> FromCanceled<TResult>(CancellationToken cancellationToken)",
        "static YieldAwaitable Yield()",
    ]),
    // 35
    bcl!("System.Threading.Tasks.Task`1", Class, tp = ["TResult"], base = "Task"; [
        "Task(Func<TResult> function)",
        "Task(Func<TResult> function, CancellationToken cancellationToken)",
        "TResult Result { get; }",
        "new TaskAwaiter<TResult> GetAwaiter()",
        "new ConfiguredTaskAwaitable<TResult> ConfigureAwait(bool continueOnCapturedContext)",
        "Task ContinueWith(Action<Task<TResult>> continuationAction)",
        "Task<TNewResult> ContinueWith<TNewResult>(Func<Task<TResult>, TNewResult> continuationFunction)",
        "new Task<TResult> WaitAsync(TimeSpan timeout)",
        "new Task<TResult> WaitAsync(CancellationToken cancellationToken)",
    ]),
    // 36
    bcl_delegate!("System.Action", [], "void Invoke()"),
    // 37
    bcl_delegate!("System.Func`1", ["out TResult"], "TResult Invoke()"),
    // 38
    bcl_delegate!("System.EventHandler", [], "void Invoke(object? sender, EventArgs e)"),
    // 39
    bcl!("System.Lazy`1", Class, tp = ["T"]; [
        "Lazy()",
        "Lazy(bool isThreadSafe)",
        "Lazy(Func<T> valueFactory)",
        "Lazy(Func<T> valueFactory, bool isThreadSafe)",
        "Lazy(Func<T> valueFactory, LazyThreadSafetyMode mode)",
        "Lazy(LazyThreadSafetyMode mode)",
        "Lazy(T value)",
        "bool IsValueCreated { get; }",
        "T Value { get; }",
        "override string? ToString()",
    ]),
    // 40
    bcl!("System.Span`1", Struct, flags = BclTypeFlags::READONLY | BclTypeFlags::REF_STRUCT, tp = ["T"]; [
        "Span(T[]? array)",
        "Span(T[]? array, int start, int length)",
        "Span(ref T reference)",
        "int Length { get; }",
        "bool IsEmpty { get; }",
        "ref T this[int index] { get; }",
        "static Span<T> Empty { get; }",
        "void Clear()",
        "void Fill(T value)",
        "void CopyTo(Span<T> destination)",
        "bool TryCopyTo(Span<T> destination)",
        "Span<T> Slice(int start)",
        "Span<T> Slice(int start, int length)",
        "T[] ToArray()",
        "ref T GetPinnableReference()",
        "override string ToString()",
        "static implicit operator Span<T>(T[]? array)",
        "static implicit operator Span<T>(ArraySegment<T> segment)",
        "static implicit operator ReadOnlySpan<T>(Span<T> span)",
        "static bool operator ==(Span<T> left, Span<T> right)",
        "static bool operator !=(Span<T> left, Span<T> right)",
    ]),
    // 41
    bcl!("System.IEquatable`1", Interface, tp = ["T"]; [
        "bool Equals(T? other)",
    ]),
    // 42
    bcl!("System.Type", Class, flags = BclTypeFlags::ABSTRACT; [
        "abstract string Name { get; }",
        "abstract string? FullName { get; }",
        "abstract string? Namespace { get; }",
        "abstract string? AssemblyQualifiedName { get; }",
        "abstract Type? BaseType { get; }",
        "bool IsClass { get; }",
        "bool IsInterface { get; }",
        "bool IsValueType { get; }",
        "bool IsEnum { get; }",
        "bool IsArray { get; }",
        "bool IsPrimitive { get; }",
        "bool IsPublic { get; }",
        "bool IsAbstract { get; }",
        "bool IsSealed { get; }",
        "virtual bool IsGenericType { get; }",
        "virtual bool IsGenericTypeDefinition { get; }",
        "virtual Type[] GetGenericArguments()",
        "virtual Type GetGenericTypeDefinition()",
        "abstract Type[] GetInterfaces()",
        "abstract Type? GetElementType()",
        "virtual bool IsAssignableFrom(Type? c)",
        "virtual bool IsInstanceOfType(object? o)",
        "virtual bool IsSubclassOf(Type c)",
        "virtual Type MakeGenericType(params Type[] typeArguments)",
        "virtual Type MakeArrayType()",
        "static Type? GetType(string typeName)",
        "static Type? GetType(string typeName, bool throwOnError)",
        "override string ToString()",
        "static bool operator ==(Type? left, Type? right)",
        "static bool operator !=(Type? left, Type? right)",
    ]),
    // 43
    bcl_delegate!("System.EventHandler`1", ["TEventArgs"], "void Invoke(object? sender, TEventArgs e)"),
    // 44
    bcl!("System.Linq.Enumerable", Class, flags = BclTypeFlags::STATIC; [
        "static IEnumerable<TSource> Where<TSource>(this IEnumerable<TSource> source, Func<TSource, bool> predicate)",
        "static IEnumerable<TSource> Where<TSource>(this IEnumerable<TSource> source, Func<TSource, int, bool> predicate)",
        "static IEnumerable<TResult> Select<TSource, TResult>(this IEnumerable<TSource> source, Func<TSource, TResult> selector)",
        "static IEnumerable<TResult> Select<TSource, TResult>(this IEnumerable<TSource> source, Func<TSource, int, TResult> selector)",
        "static IEnumerable<TResult> SelectMany<TSource, TResult>(this IEnumerable<TSource> source, Func<TSource, IEnumerable<TResult>> selector)",
        "static IEnumerable<TResult> SelectMany<TSource, TCollection, TResult>(this IEnumerable<TSource> source, Func<TSource, IEnumerable<TCollection>> collectionSelector, Func<TSource, TCollection, TResult> resultSelector)",
        "static TSource First<TSource>(this IEnumerable<TSource> source)",
        "static TSource First<TSource>(this IEnumerable<TSource> source, Func<TSource, bool> predicate)",
        "static TSource? FirstOrDefault<TSource>(this IEnumerable<TSource> source)",
        "static TSource? FirstOrDefault<TSource>(this IEnumerable<TSource> source, Func<TSource, bool> predicate)",
        "static TSource FirstOrDefault<TSource>(this IEnumerable<TSource> source, TSource defaultValue)",
        "static TSource Last<TSource>(this IEnumerable<TSource> source)",
        "static TSource Last<TSource>(this IEnumerable<TSource> source, Func<TSource, bool> predicate)",
        "static TSource? LastOrDefault<TSource>(this IEnumerable<TSource> source)",
        "static TSource? LastOrDefault<TSource>(this IEnumerable<TSource> source, Func<TSource, bool> predicate)",
        "static TSource Single<TSource>(this IEnumerable<TSource> source)",
        "static TSource Single<TSource>(this IEnumerable<TSource> source, Func<TSource, bool> predicate)",
        "static TSource? SingleOrDefault<TSource>(this IEnumerable<TSource> source)",
        "static TSource? SingleOrDefault<TSource>(this IEnumerable<TSource> source, Func<TSource, bool> predicate)",
        "static TSource ElementAt<TSource>(this IEnumerable<TSource> source, int index)",
        "static TSource? ElementAtOrDefault<TSource>(this IEnumerable<TSource> source, int index)",
        "static bool Any<TSource>(this IEnumerable<TSource> source)",
        "static bool Any<TSource>(this IEnumerable<TSource> source, Func<TSource, bool> predicate)",
        "static bool All<TSource>(this IEnumerable<TSource> source, Func<TSource, bool> predicate)",
        "static int Count<TSource>(this IEnumerable<TSource> source)",
        "static int Count<TSource>(this IEnumerable<TSource> source, Func<TSource, bool> predicate)",
        "static long LongCount<TSource>(this IEnumerable<TSource> source)",
        "static bool Contains<TSource>(this IEnumerable<TSource> source, TSource value)",
        "static bool Contains<TSource>(this IEnumerable<TSource> source, TSource value, IEqualityComparer<TSource>? comparer)",
        "static int Sum(this IEnumerable<int> source)",
        "static long Sum(this IEnumerable<long> source)",
        "static double Sum(this IEnumerable<double> source)",
        "static decimal Sum(this IEnumerable<decimal> source)",
        "static int Sum<TSource>(this IEnumerable<TSource> source, Func<TSource, int> selector)",
        "static long Sum<TSource>(this IEnumerable<TSource> source, Func<TSource, long> selector)",
        "static double Sum<TSource>(this IEnumerable<TSource> source, Func<TSource, double> selector)",
        "static decimal Sum<TSource>(this IEnumerable<TSource> source, Func<TSource, decimal> selector)",
        "static double Average(this IEnumerable<int> source)",
        "static double Average(this IEnumerable<double> source)",
        "static decimal Average(this IEnumerable<decimal> source)",
        "static double Average<TSource>(this IEnumerable<TSource> source, Func<TSource, int> selector)",
        "static double Average<TSource>(this IEnumerable<TSource> source, Func<TSource, double> selector)",
        "static decimal Average<TSource>(this IEnumerable<TSource> source, Func<TSource, decimal> selector)",
        "static TSource? Min<TSource>(this IEnumerable<TSource> source)",
        "static TResult? Min<TSource, TResult>(this IEnumerable<TSource> source, Func<TSource, TResult> selector)",
        "static TSource? Max<TSource>(this IEnumerable<TSource> source)",
        "static TResult? Max<TSource, TResult>(this IEnumerable<TSource> source, Func<TSource, TResult> selector)",
        "static TSource? MinBy<TSource, TKey>(this IEnumerable<TSource> source, Func<TSource, TKey> keySelector)",
        "static TSource? MaxBy<TSource, TKey>(this IEnumerable<TSource> source, Func<TSource, TKey> keySelector)",
        "static IOrderedEnumerable<TSource> OrderBy<TSource, TKey>(this IEnumerable<TSource> source, Func<TSource, TKey> keySelector)",
        "static IOrderedEnumerable<TSource> OrderBy<TSource, TKey>(this IEnumerable<TSource> source, Func<TSource, TKey> keySelector, IComparer<TKey>? comparer)",
        "static IOrderedEnumerable<TSource> OrderByDescending<TSource, TKey>(this IEnumerable<TSource> source, Func<TSource, TKey> keySelector)",
        "static IOrderedEnumerable<TSource> ThenBy<TSource, TKey>(this IOrderedEnumerable<TSource> source, Func<TSource, TKey> keySelector)",
        "static IOrderedEnumerable<TSource> ThenByDescending<TSource, TKey>(this IOrderedEnumerable<TSource> source, Func<TSource, TKey> keySelector)",
        "static IOrderedEnumerable<T> Order<T>(this IEnumerable<T> source)",
        "static IEnumerable<IGrouping<TKey, TSource>> GroupBy<TSource, TKey>(this IEnumerable<TSource> source, Func<TSource, TKey> keySelector)",
        "static IEnumerable<IGrouping<TKey, TElement>> GroupBy<TSource, TKey, TElement>(this IEnumerable<TSource> source, Func<TSource, TKey> keySelector, Func<TSource, TElement> elementSelector)",
        "static IEnumerable<TResult> Join<TOuter, TInner, TKey, TResult>(this IEnumerable<TOuter> outer, IEnumerable<TInner> inner, Func<TOuter, TKey> outerKeySelector, Func<TInner, TKey> innerKeySelector, Func<TOuter, TInner, TResult> resultSelector)",
        "static IEnumerable<TResult> GroupJoin<TOuter, TInner, TKey, TResult>(this IEnumerable<TOuter> outer, IEnumerable<TInner> inner, Func<TOuter, TKey> outerKeySelector, Func<TInner, TKey> innerKeySelector, Func<TOuter, IEnumerable<TInner>, TResult> resultSelector)",
        "static IEnumerable<TSource> Distinct<TSource>(this IEnumerable<TSource> source)",
        "static IEnumerable<TSource> Distinct<TSource>(this IEnumerable<TSource> source, IEqualityComparer<TSource>? comparer)",
        "static IEnumerable<TSource> DistinctBy<TSource, TKey>(this IEnumerable<TSource> source, Func<TSource, TKey> keySelector)",
        "static IEnumerable<TSource> Union<TSource>(this IEnumerable<TSource> first, IEnumerable<TSource> second)",
        "static IEnumerable<TSource> Intersect<TSource>(this IEnumerable<TSource> first, IEnumerable<TSource> second)",
        "static IEnumerable<TSource> Except<TSource>(this IEnumerable<TSource> first, IEnumerable<TSource> second)",
        "static IEnumerable<TSource> Concat<TSource>(this IEnumerable<TSource> first, IEnumerable<TSource> second)",
        "static IEnumerable<TSource> Append<TSource>(this IEnumerable<TSource> source, TSource element)",
        "static IEnumerable<TSource> Prepend<TSource>(this IEnumerable<TSource> source, TSource element)",
        "static IEnumerable<TResult> Zip<TFirst, TSecond, TResult>(this IEnumerable<TFirst> first, IEnumerable<TSecond> second, Func<TFirst, TSecond, TResult> resultSelector)",
        "static IEnumerable<(TFirst First, TSecond Second)> Zip<TFirst, TSecond>(this IEnumerable<TFirst> first, IEnumerable<TSecond> second)",
        "static IEnumerable<TSource> Skip<TSource>(this IEnumerable<TSource> source, int count)",
        "static IEnumerable<TSource> SkipWhile<TSource>(this IEnumerable<TSource> source, Func<TSource, bool> predicate)",
        "static IEnumerable<TSource> SkipLast<TSource>(this IEnumerable<TSource> source, int count)",
        "static IEnumerable<TSource> Take<TSource>(this IEnumerable<TSource> source, int count)",
        "static IEnumerable<TSource> TakeWhile<TSource>(this IEnumerable<TSource> source, Func<TSource, bool> predicate)",
        "static IEnumerable<TSource> TakeLast<TSource>(this IEnumerable<TSource> source, int count)",
        "static IEnumerable<TSource[]> Chunk<TSource>(this IEnumerable<TSource> source, int size)",
        "static IEnumerable<TSource> Reverse<TSource>(this IEnumerable<TSource> source)",
        "static IEnumerable<TSource?> DefaultIfEmpty<TSource>(this IEnumerable<TSource> source)",
        "static bool SequenceEqual<TSource>(this IEnumerable<TSource> first, IEnumerable<TSource> second)",
        "static TSource Aggregate<TSource>(this IEnumerable<TSource> source, Func<TSource, TSource, TSource> func)",
        "static TAccumulate Aggregate<TSource, TAccumulate>(this IEnumerable<TSource> source, TAccumulate seed, Func<TAccumulate, TSource, TAccumulate> func)",
        "static IEnumerable<TResult> Cast<TResult>(this IEnumerable source)",
        "static IEnumerable<TResult> OfType<TResult>(this IEnumerable source)",
        "static IEnumerable<TSource> AsEnumerable<TSource>(this IEnumerable<TSource> source)",
        "static List<TSource> ToList<TSource>(this IEnumerable<TSource> source)",
        "static TSource[] ToArray<TSource>(this IEnumerable<TSource> source)",
        "static HashSet<TSource> ToHashSet<TSource>(this IEnumerable<TSource> source)",
        "static Dictionary<TKey, TSource> ToDictionary<TSource, TKey>(this IEnumerable<TSource> source, Func<TSource, TKey> keySelector) where TKey : notnull",
        "static Dictionary<TKey, TElement> ToDictionary<TSource, TKey, TElement>(this IEnumerable<TSource> source, Func<TSource, TKey> keySelector, Func<TSource, TElement> elementSelector) where TKey : notnull",
        "static ILookup<TKey, TSource> ToLookup<TSource, TKey>(this IEnumerable<TSource> source, Func<TSource, TKey> keySelector)",
        "static IEnumerable<int> Range(int start, int count)",
        "static IEnumerable<TResult> Repeat<TResult>(TResult element, int count)",
        "static IEnumerable<TResult> Empty<TResult>()",
    ]),
    // 45
    bcl!("System.Threading.Tasks.ValueTask", Struct, flags = BclTypeFlags::READONLY, implements = ["IEquatable<ValueTask>"]; [
        "ValueTask(Task task)",
        "bool IsCompleted { get; }",
        "bool IsCompletedSuccessfully { get; }",
        "bool IsFaulted { get; }",
        "bool IsCanceled { get; }",
        "static ValueTask CompletedTask { get; }",
        "Task AsTask()",
        "ValueTask Preserve()",
        "bool Equals(ValueTask other)",
        "override bool Equals(object? obj)",
        "override int GetHashCode()",
        "static ValueTask<TResult> FromResult<TResult>(TResult result)",
        "static ValueTask FromException(Exception exception)",
        "static ValueTask FromCanceled(CancellationToken cancellationToken)",
    ]),
    // 46
    bcl!("System.Threading.Tasks.ValueTask`1", Struct, flags = BclTypeFlags::READONLY, tp = ["TResult"],
        implements = ["IEquatable<ValueTask<TResult>>"]; [
        "ValueTask(TResult result)",
        "ValueTask(Task<TResult> task)",
        "TResult Result { get; }",
        "bool IsCompleted { get; }",
        "bool IsCompletedSuccessfully { get; }",
        "bool IsFaulted { get; }",
        "bool IsCanceled { get; }",
        "Task<TResult> AsTask()",
        "ValueTask<TResult> Preserve()",
        "bool Equals(ValueTask<TResult> other)",
        "override bool Equals(object? obj)",
        "override int GetHashCode()",
        "override string? ToString()",
    ]),
    // 47
    bcl!("System.Text.StringBuilder", Class, flags = BclTypeFlags::SEALED; [
        "StringBuilder()",
        "StringBuilder(int capacity)",
        "StringBuilder(string? value)",
        "StringBuilder(string? value, int capacity)",
        "int Length { get; set; }",
        "int Capacity { get; set; }",
        "int MaxCapacity { get; }",
        "char this[int index] { get; set; }",
        "StringBuilder Append(string? value)",
        "StringBuilder Append(char value)",
        "StringBuilder Append(char value, int repeatCount)",
        "StringBuilder Append(bool value)",
        "StringBuilder Append(int value)",
        "StringBuilder Append(long value)",
        "StringBuilder Append(double value)",
        "StringBuilder Append(decimal value)",
        "StringBuilder Append(object? value)",
        "StringBuilder Append(char[]? value)",
        "StringBuilder Append(string? value, int startIndex, int count)",
        "StringBuilder AppendLine()",
        "StringBuilder AppendLine(string? value)",
        "StringBuilder AppendFormat(string format, object? arg0)",
        "StringBuilder AppendFormat(string format, params object?[] args)",
        "StringBuilder AppendJoin(string? separator, params object?[] values)",
        "StringBuilder AppendJoin<T>(string? separator, IEnumerable<T> values)",
        "StringBuilder Insert(int index, string? value)",
        "StringBuilder Insert(int index, char value)",
        "StringBuilder Insert(int index, object? value)",
        "StringBuilder Remove(int startIndex, int length)",
        "StringBuilder Replace(string oldValue, string? newValue)",
        "StringBuilder Replace(char oldChar, char newChar)",
        "StringBuilder Clear()",
        "int EnsureCapacity(int capacity)",
        "bool Equals(StringBuilder? sb)",
        "override string ToString()",
        "string ToString(int startIndex, int length)",
    ]),
    // ── System ──
    bcl!("System.IAsyncDisposable", Interface; [
        "ValueTask DisposeAsync()",
    ]),
    bcl!("System.IComparable", Interface; [
        "int CompareTo(object? obj)",
    ]),
    bcl!("System.IComparable`1", Interface, tp = ["in T"]; [
        "int CompareTo(T? other)",
    ]),
    bcl!("System.IFormatProvider", Interface; [
        "object? GetFormat(Type? formatType)",
    ]),
    bcl!("System.IFormattable", Interface; [
        "string ToString(string? format, IFormatProvider? formatProvider)",
    ]),
    bcl!("System.ICloneable", Interface; [
        "object Clone()",
    ]),
    bcl!("System.IServiceProvider", Interface; [
        "object? GetService(Type serviceType)",
    ]),
    bcl!("System.IAsyncResult", Interface; [
        "bool IsCompleted { get; }",
        "bool CompletedSynchronously { get; }",
        "object? AsyncState { get; }",
    ]),
    bcl!("System.IObservable`1", Interface, tp = ["out T"]; [
        "IDisposable Subscribe(IObserver<T> observer)",
    ]),
    bcl!("System.IObserver`1", Interface, tp = ["in T"]; [
        "void OnNext(T value)",
        "void OnError(Exception error)",
        "void OnCompleted()",
    ]),
    bcl!("System.IProgress`1", Interface, tp = ["in T"]; [
        "void Report(T value)",
    ]),
    bcl!("System.EventArgs", Class; [
        "EventArgs()",
        "static readonly EventArgs Empty",
    ]),
    bcl_delegate!("System.AsyncCallback", [], "void Invoke(IAsyncResult ar)"),
    bcl_delegate!("System.Action`1", ["in T"], "void Invoke(T obj)"),
    bcl_delegate!("System.Action`2", ["in T1", "in T2"], "void Invoke(T1 arg1, T2 arg2)"),
    bcl_delegate!("System.Action`3", ["in T1", "in T2", "in T3"], "void Invoke(T1 arg1, T2 arg2, T3 arg3)"),
    bcl_delegate!("System.Action`4", ["in T1", "in T2", "in T3", "in T4"], "void Invoke(T1 arg1, T2 arg2, T3 arg3, T4 arg4)"),
    bcl_delegate!("System.Func`2", ["in T", "out TResult"], "TResult Invoke(T arg)"),
    bcl_delegate!("System.Func`3", ["in T1", "in T2", "out TResult"], "TResult Invoke(T1 arg1, T2 arg2)"),
    bcl_delegate!("System.Func`4", ["in T1", "in T2", "in T3", "out TResult"], "TResult Invoke(T1 arg1, T2 arg2, T3 arg3)"),
    bcl_delegate!("System.Func`5", ["in T1", "in T2", "in T3", "in T4", "out TResult"], "TResult Invoke(T1 arg1, T2 arg2, T3 arg3, T4 arg4)"),
    bcl_delegate!("System.Predicate`1", ["in T"], "bool Invoke(T obj)"),
    bcl_delegate!("System.Comparison`1", ["in T"], "int Invoke(T x, T y)"),
    bcl_delegate!("System.Converter`2", ["in TInput", "out TOutput"], "TOutput Invoke(TInput input)"),
    bcl!("System.Nullable", Class, flags = BclTypeFlags::STATIC; [
        "static Type? GetUnderlyingType(Type nullableType)",
        "static int Compare<T>(T? n1, T? n2) where T : struct",
        "static bool Equals<T>(T? n1, T? n2) where T : struct",
    ]),
    bcl!("System.ReadOnlySpan`1", Struct, flags = BclTypeFlags::READONLY | BclTypeFlags::REF_STRUCT, tp = ["T"]; [
        "ReadOnlySpan(T[]? array)",
        "ReadOnlySpan(T[]? array, int start, int length)",
        "int Length { get; }",
        "bool IsEmpty { get; }",
        "ref readonly T this[int index] { get; }",
        "static ReadOnlySpan<T> Empty { get; }",
        "void CopyTo(Span<T> destination)",
        "bool TryCopyTo(Span<T> destination)",
        "ReadOnlySpan<T> Slice(int start)",
        "ReadOnlySpan<T> Slice(int start, int length)",
        "T[] ToArray()",
        "override string ToString()",
        "static implicit operator ReadOnlySpan<T>(T[]? array)",
        "static implicit operator ReadOnlySpan<T>(ArraySegment<T> segment)",
    ]),
    bcl!("System.Memory`1", Struct, flags = BclTypeFlags::READONLY, tp = ["T"], implements = ["IEquatable<Memory<T>>"]; [
        "Memory(T[]? array)",
        "Memory(T[]? array, int start, int length)",
        "int Length { get; }",
        "bool IsEmpty { get; }",
        "Span<T> Span { get; }",
        "static Memory<T> Empty { get; }",
        "Memory<T> Slice(int start)",
        "Memory<T> Slice(int start, int length)",
        "T[] ToArray()",
        "bool Equals(Memory<T> other)",
        "static implicit operator Memory<T>(T[]? array)",
        "static implicit operator ReadOnlyMemory<T>(Memory<T> memory)",
    ]),
    bcl!("System.ReadOnlyMemory`1", Struct, flags = BclTypeFlags::READONLY, tp = ["T"], implements = ["IEquatable<ReadOnlyMemory<T>>"]; [
        "ReadOnlyMemory(T[]? array)",
        "ReadOnlyMemory(T[]? array, int start, int length)",
        "int Length { get; }",
        "bool IsEmpty { get; }",
        "ReadOnlySpan<T> Span { get; }",
        "static ReadOnlyMemory<T> Empty { get; }",
        "ReadOnlyMemory<T> Slice(int start)",
        "ReadOnlyMemory<T> Slice(int start, int length)",
        "T[] ToArray()",
        "bool Equals(ReadOnlyMemory<T> other)",
        "static implicit operator ReadOnlyMemory<T>(T[]? array)",
    ]),
    bcl!("System.ArraySegment`1", Struct, flags = BclTypeFlags::READONLY, tp = ["T"],
        implements = ["IList<T>", "IReadOnlyList<T>", "IEnumerable<T>"]; [
        "ArraySegment(T[] array)",
        "ArraySegment(T[] array, int offset, int count)",
        "T[]? Array { get; }",
        "int Offset { get; }",
        "int Count { get; }",
        "T this[int index] { get; set; }",
        "ArraySegment<T> Slice(int index)",
        "ArraySegment<T> Slice(int index, int count)",
        "T[] ToArray()",
        "static implicit operator ArraySegment<T>(T[] array)",
    ]),
    bcl!("System.Index", Struct, flags = BclTypeFlags::READONLY, implements = ["IEquatable<Index>"]; [
        "Index(int value, bool fromEnd = false)",
        "int Value { get; }",
        "bool IsFromEnd { get; }",
        "static Index Start { get; }",
        "static Index End { get; }",
        "static Index FromStart(int value)",
        "static Index FromEnd(int value)",
        "int GetOffset(int length)",
        "bool Equals(Index other)",
        "static implicit operator Index(int value)",
    ]),
    bcl!("System.Range", Struct, flags = BclTypeFlags::READONLY, implements = ["IEquatable<Range>"]; [
        "Range(Index start, Index end)",
        "Index Start { get; }",
        "Index End { get; }",
        "static Range All { get; }",
        "static Range StartAt(Index start)",
        "static Range EndAt(Index end)",
        "bool Equals(Range other)",
    ]),
    bcl!("System.ValueTuple`1", Struct, tp = ["T1"], implements = ["IEquatable<ValueTuple<T1>>"]; [
        "ValueTuple(T1 item1)",
        "T1 Item1",
        "bool Equals(ValueTuple<T1> other)",
    ]),
    bcl!("System.ValueTuple`2", Struct, tp = ["T1", "T2"], implements = ["IEquatable<ValueTuple<T1, T2>>"]; [
        "ValueTuple(T1 item1, T2 item2)",
        "T1 Item1",
        "T2 Item2",
        "bool Equals(ValueTuple<T1, T2> other)",
    ]),
    bcl!("System.ValueTuple`3", Struct, tp = ["T1", "T2", "T3"]; [
        "ValueTuple(T1 item1, T2 item2, T3 item3)",
        "T1 Item1",
        "T2 Item2",
        "T3 Item3",
    ]),
    bcl!("System.ValueTuple`4", Struct, tp = ["T1", "T2", "T3", "T4"]; [
        "ValueTuple(T1 item1, T2 item2, T3 item3, T4 item4)",
        "T1 Item1",
        "T2 Item2",
        "T3 Item3",
        "T4 Item4",
    ]),
    bcl!("System.ValueTuple`5", Struct, tp = ["T1", "T2", "T3", "T4", "T5"]; [
        "ValueTuple(T1 item1, T2 item2, T3 item3, T4 item4, T5 item5)",
        "T1 Item1",
        "T2 Item2",
        "T3 Item3",
        "T4 Item4",
        "T5 Item5",
    ]),
    bcl!("System.ValueTuple`6", Struct, tp = ["T1", "T2", "T3", "T4", "T5", "T6"]; [
        "ValueTuple(T1 item1, T2 item2, T3 item3, T4 item4, T5 item5, T6 item6)",
        "T1 Item1",
        "T2 Item2",
        "T3 Item3",
        "T4 Item4",
        "T5 Item5",
        "T6 Item6",
    ]),
    bcl!("System.ValueTuple`7", Struct, tp = ["T1", "T2", "T3", "T4", "T5", "T6", "T7"]; [
        "ValueTuple(T1 item1, T2 item2, T3 item3, T4 item4, T5 item5, T6 item6, T7 item7)",
        "T1 Item1",
        "T2 Item2",
        "T3 Item3",
        "T4 Item4",
        "T5 Item5",
        "T6 Item6",
        "T7 Item7",
    ]),
    bcl!("System.ValueTuple`8", Struct, tp = ["T1", "T2", "T3", "T4", "T5", "T6", "T7", "TRest"],
        constraints = ["TRest : struct"]; [
        "ValueTuple(T1 item1, T2 item2, T3 item3, T4 item4, T5 item5, T6 item6, T7 item7, TRest rest)",
        "T1 Item1",
        "T2 Item2",
        "T3 Item3",
        "T4 Item4",
        "T5 Item5",
        "T6 Item6",
        "T7 Item7",
        "TRest Rest",
    ]),
    bcl!("System.DateTime", Struct, flags = BclTypeFlags::READONLY,
        implements = ["IComparable", "IComparable<DateTime>", "IEquatable<DateTime>", "IFormattable"]; [
        "DateTime(long ticks)",
        "DateTime(int year, int month, int day)",
        "DateTime(int year, int month, int day, int hour, int minute, int second)",
        "DateTime(int year, int month, int day, int hour, int minute, int second, DateTimeKind kind)",
        "static readonly DateTime MinValue",
        "static readonly DateTime MaxValue",
        "static readonly DateTime UnixEpoch",
        "static DateTime Now { get; }",
        "static DateTime UtcNow { get; }",
        "static DateTime Today { get; }",
        "int Year { get; }",
        "int Month { get; }",
        "int Day { get; }",
        "int Hour { get; }",
        "int Minute { get; }",
        "int Second { get; }",
        "int Millisecond { get; }",
        "long Ticks { get; }",
        "DayOfWeek DayOfWeek { get; }",
        "int DayOfYear { get; }",
        "DateTime Date { get; }",
        "TimeSpan TimeOfDay { get; }",
        "DateTimeKind Kind { get; }",
        "DateTime Add(TimeSpan value)",
        "DateTime AddDays(double value)",
        "DateTime AddHours(double value)",
        "DateTime AddMinutes(double value)",
        "DateTime AddSeconds(double value)",
        "DateTime AddMilliseconds(double value)",
        "DateTime AddMonths(int months)",
        "DateTime AddYears(int value)",
        "DateTime AddTicks(long value)",
        "TimeSpan Subtract(DateTime value)",
        "DateTime Subtract(TimeSpan value)",
        "DateTime ToUniversalTime()",
        "DateTime ToLocalTime()",
        "string ToShortDateString()",
        "string ToLongDateString()",
        "override string ToString()",
        "string ToString(string? format)",
        "int CompareTo(DateTime value)",
        "bool Equals(DateTime value)",
        "static DateTime Parse(string s)",
        "static bool TryParse(string? s, out DateTime result)",
        "static DateTime ParseExact(string s, string format, IFormatProvider? provider)",
        "static int DaysInMonth(int year, int month)",
        "static bool IsLeapYear(int year)",
        "static DateTime SpecifyKind(DateTime value, DateTimeKind kind)",
        "static TimeSpan operator -(DateTime d1, DateTime d2)",
        "static DateTime operator -(DateTime d, TimeSpan t)",
        "static DateTime operator +(DateTime d, TimeSpan t)",
        "static bool operator ==(DateTime d1, DateTime d2)",
        "static bool operator !=(DateTime d1, DateTime d2)",
        "static bool operator <(DateTime t1, DateTime t2)",
        "static bool operator >(DateTime t1, DateTime t2)",
        "static bool operator <=(DateTime t1, DateTime t2)",
        "static bool operator >=(DateTime t1, DateTime t2)",
    ]),
    bcl!("System.DateTimeKind", Enum; ["Unspecified", "Utc", "Local"]),
    bcl!("System.DayOfWeek", Enum; ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"]),
    bcl!("System.DateTimeOffset", Struct, flags = BclTypeFlags::READONLY,
        implements = ["IComparable", "IComparable<DateTimeOffset>", "IEquatable<DateTimeOffset>", "IFormattable"]; [
        "DateTimeOffset(DateTime dateTime)",
        "DateTimeOffset(DateTime dateTime, TimeSpan offset)",
        "static DateTimeOffset Now { get; }",
        "static DateTimeOffset UtcNow { get; }",
        "DateTime DateTime { get; }",
        "DateTime UtcDateTime { get; }",
        "DateTime LocalDateTime { get; }",
        "TimeSpan Offset { get; }",
        "long Ticks { get; }",
        "DateTimeOffset AddDays(double days)",
        "DateTimeOffset AddHours(double hours)",
        "DateTimeOffset AddMinutes(double minutes)",
        "DateTimeOffset AddSeconds(double seconds)",
        "long ToUnixTimeSeconds()",
        "long ToUnixTimeMilliseconds()",
        "static DateTimeOffset FromUnixTimeSeconds(long seconds)",
        "static DateTimeOffset FromUnixTimeMilliseconds(long milliseconds)",
        "static DateTimeOffset Parse(string input)",
        "int CompareTo(DateTimeOffset other)",
        "bool Equals(DateTimeOffset other)",
        "override string ToString()",
    ]),
    bcl!("System.TimeSpan", Struct, flags = BclTypeFlags::READONLY,
        implements = ["IComparable", "IComparable<TimeSpan>", "IEquatable<TimeSpan>", "IFormattable"]; [
        "TimeSpan(long ticks)",
        "TimeSpan(int hours, int minutes, int seconds)",
        "TimeSpan(int days, int hours, int minutes, int seconds)",
        "TimeSpan(int days, int hours, int minutes, int seconds, int milliseconds)",
        "static readonly TimeSpan Zero",
        "static readonly TimeSpan MaxValue",
        "static readonly TimeSpan MinValue",
        "const long TicksPerMillisecond",
        "const long TicksPerSecond",
        "int Days { get; }",
        "int Hours { get; }",
        "int Minutes { get; }",
        "int Seconds { get; }",
        "int Milliseconds { get; }",
        "long Ticks { get; }",
        "double TotalDays { get; }",
        "double TotalHours { get; }",
        "double TotalMinutes { get; }",
        "double TotalSeconds { get; }",
        "double TotalMilliseconds { get; }",
        "TimeSpan Add(TimeSpan ts)",
        "TimeSpan Subtract(TimeSpan ts)",
        "TimeSpan Negate()",
        "TimeSpan Duration()",
        "static TimeSpan FromDays(double value)",
        "static TimeSpan FromHours(double value)",
        "static TimeSpan FromMinutes(double value)",
        "static TimeSpan FromSeconds(double value)",
        "static TimeSpan FromMilliseconds(double value)",
        "static TimeSpan FromTicks(long value)",
        "static TimeSpan Parse(string s)",
        "static bool TryParse(string? s, out TimeSpan result)",
        "int CompareTo(TimeSpan value)",
        "bool Equals(TimeSpan obj)",
        "override string ToString()",
        "static TimeSpan operator +(TimeSpan t1, TimeSpan t2)",
        "static TimeSpan operator -(TimeSpan t1, TimeSpan t2)",
        "static bool operator ==(TimeSpan t1, TimeSpan t2)",
        "static bool operator !=(TimeSpan t1, TimeSpan t2)",
        "static bool operator <(TimeSpan t1, TimeSpan t2)",
        "static bool operator >(TimeSpan t1, TimeSpan t2)",
    ]),
    bcl!("System.Guid", Struct, flags = BclTypeFlags::READONLY,
        implements = ["IComparable", "IComparable<Guid>", "IEquatable<Guid>", "IFormattable"]; [
        "Guid(byte[] b)",
        "Guid(string g)",
        "static readonly Guid Empty",
        "static Guid NewGuid()",
        "static Guid Parse(string input)",
        "static bool TryParse(string? input, out Guid result)",
        "byte[] ToByteArray()",
        "override string ToString()",
        "string ToString(string? format)",
        "int CompareTo(Guid value)",
        "bool Equals(Guid g)",
        "static bool operator ==(Guid a, Guid b)",
        "static bool operator !=(Guid a, Guid b)",
    ]),
    bcl!("System.Math", Class, flags = BclTypeFlags::STATIC; [
        "const double PI",
        "const double E",
        "static int Abs(int value)",
        "static long Abs(long value)",
        "static double Abs(double value)",
        "static decimal Abs(decimal value)",
        "static int Max(int val1, int val2)",
        "static long Max(long val1, long val2)",
        "static double Max(double val1, double val2)",
        "static decimal Max(decimal val1, decimal val2)",
        "static int Min(int val1, int val2)",
        "static long Min(long val1, long val2)",
        "static double Min(double val1, double val2)",
        "static decimal Min(decimal val1, decimal val2)",
        "static int Clamp(int value, int min, int max)",
        "static double Clamp(double value, double min, double max)",
        "static double Pow(double x, double y)",
        "static double Sqrt(double d)",
        "static double Floor(double d)",
        "static double Ceiling(double a)",
        "static double Round(double a)",
        "static double Round(double value, int digits)",
        "static decimal Round(decimal d)",
        "static decimal Round(decimal d, int decimals)",
        "static double Truncate(double d)",
        "static int Sign(double value)",
        "static double Log(double d)",
        "static double Log10(double d)",
        "static double Exp(double d)",
        "static double Sin(double a)",
        "static double Cos(double d)",
        "static double Tan(double a)",
        "static double Atan2(double y, double x)",
    ]),
    bcl!("System.Console", Class, flags = BclTypeFlags::STATIC; [
        "static TextWriter Out { get; }",
        "static TextWriter Error { get; }",
        "static TextReader In { get; }",
        "static ConsoleColor ForegroundColor { get; set; }",
        "static void WriteLine()",
        "static void WriteLine(string? value)",
        "static void WriteLine(object? value)",
        "static void WriteLine(bool value)",
        "static void WriteLine(char value)",
        "static void WriteLine(int value)",
        "static void WriteLine(long value)",
        "static void WriteLine(double value)",
        "static void WriteLine(decimal value)",
        "static void WriteLine(string format, object? arg0)",
        "static void WriteLine(string format, object? arg0, object? arg1)",
        "static void WriteLine(string format, params object?[]? arg)",
        "static void Write(string? value)",
        "static void Write(object? value)",
        "static void Write(char value)",
        "static void Write(int value)",
        "static void Write(string format, params object?[]? arg)",
        "static string? ReadLine()",
        "static int Read()",
        "static ConsoleKeyInfo ReadKey()",
        "static void Clear()",
        "static void ResetColor()",
    ]),
    bcl!("System.ConsoleColor", Enum; [
        "Black", "DarkBlue", "DarkGreen", "DarkCyan", "DarkRed", "DarkMagenta", "DarkYellow", "Gray",
        "DarkGray", "Blue", "Green", "Cyan", "Red", "Magenta", "Yellow", "White",
    ]),
    bcl!("System.ConsoleKeyInfo", Struct, flags = BclTypeFlags::READONLY; [
        "char KeyChar { get; }",
    ]),
    bcl!("System.Convert", Class, flags = BclTypeFlags::STATIC; [
        "static int ToInt32(string? value)",
        "static int ToInt32(object? value)",
        "static int ToInt32(double value)",
        "static long ToInt64(string? value)",
        "static double ToDouble(string? value)",
        "static double ToDouble(object? value)",
        "static decimal ToDecimal(string? value)",
        "static bool ToBoolean(string? value)",
        "static bool ToBoolean(object? value)",
        "static string ToString(object? value)",
        "static string? ToString(int value)",
        "static DateTime ToDateTime(string? value)",
        "static string ToBase64String(byte[] inArray)",
        "static byte[] FromBase64String(string s)",
        "static string ToHexString(byte[] inArray)",
        "static object ChangeType(object? value, Type conversionType)",
    ]),
    bcl!("System.Environment", Class, flags = BclTypeFlags::STATIC; [
        "static string NewLine { get; }",
        "static string MachineName { get; }",
        "static string CurrentDirectory { get; set; }",
        "static int ProcessorCount { get; }",
        "static int CurrentManagedThreadId { get; }",
        "static int ExitCode { get; set; }",
        "static string? GetEnvironmentVariable(string variable)",
        "static void SetEnvironmentVariable(string variable, string? value)",
        "static string[] GetCommandLineArgs()",
        "static void Exit(int exitCode)",
    ]),
    bcl!("System.GC", Class, flags = BclTypeFlags::STATIC; [
        "static void Collect()",
        "static void SuppressFinalize(object obj)",
        "static void KeepAlive(object? obj)",
        "static long GetTotalMemory(bool forceFullCollection)",
    ]),
    bcl!("System.Activator", Class, flags = BclTypeFlags::STATIC; [
        "static object? CreateInstance(Type type)",
        "static object? CreateInstance(Type type, params object?[]? args)",
        "static T CreateInstance<T>()",
    ]),
    bcl!("System.Random", Class; [
        "Random()",
        "Random(int Seed)",
        "static Random Shared { get; }",
        "virtual int Next()",
        "virtual int Next(int maxValue)",
        "virtual int Next(int minValue, int maxValue)",
        "virtual double NextDouble()",
        "virtual void NextBytes(byte[] buffer)",
    ]),
    bcl!("System.Uri", Class; [
        "Uri(string uriString)",
        "Uri(Uri baseUri, string? relativeUri)",
        "string AbsoluteUri { get; }",
        "string AbsolutePath { get; }",
        "string Host { get; }",
        "int Port { get; }",
        "string Scheme { get; }",
        "string Query { get; }",
        "bool IsAbsoluteUri { get; }",
        "override string ToString()",
        "static bool TryCreate(string? uriString, UriKind uriKind, out Uri? result)",
        "static string EscapeDataString(string stringToEscape)",
    ]),
    bcl!("System.UriKind", Enum; ["RelativeOrAbsolute", "Absolute", "Relative"]),
    bcl!("System.StringComparison", Enum; [
        "CurrentCulture", "CurrentCultureIgnoreCase", "InvariantCulture", "InvariantCultureIgnoreCase",
        "Ordinal", "OrdinalIgnoreCase",
    ]),
    bcl!("System.StringSplitOptions", Enum, flags = BclTypeFlags::FLAGS_ENUM; ["None", "RemoveEmptyEntries", "TrimEntries"]),
    bcl!("System.StringComparer", Class, flags = BclTypeFlags::ABSTRACT,
        implements = ["IComparer<string?>", "IEqualityComparer<string?>"]; [
        "static StringComparer Ordinal { get; }",
        "static StringComparer OrdinalIgnoreCase { get; }",
        "static StringComparer InvariantCulture { get; }",
        "static StringComparer InvariantCultureIgnoreCase { get; }",
        "static StringComparer CurrentCulture { get; }",
        "static StringComparer CurrentCultureIgnoreCase { get; }",
        "abstract int Compare(string? x, string? y)",
        "abstract bool Equals(string? x, string? y)",
        "abstract int GetHashCode(string obj)",
    ]),
    bcl!("System.AttributeTargets", Enum, flags = BclTypeFlags::FLAGS_ENUM; [
        "Assembly", "Module", "Class", "Struct", "Enum", "Constructor", "Method", "Property", "Field",
        "Event", "Interface", "Parameter", "Delegate", "ReturnValue", "GenericParameter", "All",
    ]),
    bcl!("System.AttributeUsageAttribute", Class, flags = BclTypeFlags::SEALED, base = "Attribute"; [
        "AttributeUsageAttribute(AttributeTargets validOn)",
        "AttributeTargets ValidOn { get; }",
        "bool AllowMultiple { get; set; }",
        "bool Inherited { get; set; }",
    ]),
    bcl!("System.ObsoleteAttribute", Class, flags = BclTypeFlags::SEALED, base = "Attribute"; [
        "ObsoleteAttribute()",
        "ObsoleteAttribute(string? message)",
        "ObsoleteAttribute(string? message, bool error)",
        "string? Message { get; }",
        "bool IsError { get; }",
        "string? DiagnosticId { get; set; }",
    ]),
    bcl!("System.SerializableAttribute", Class, flags = BclTypeFlags::SEALED, base = "Attribute"; [
        "SerializableAttribute()",
    ]),
    bcl!("System.NonSerializedAttribute", Class, flags = BclTypeFlags::SEALED, base = "Attribute"; [
        "NonSerializedAttribute()",
    ]),
    bcl!("System.FlagsAttribute", Class, base = "Attribute"; [
        "FlagsAttribute()",
    ]),
    bcl!("System.CLSCompliantAttribute", Class, flags = BclTypeFlags::SEALED, base = "Attribute"; [
        "CLSCompliantAttribute(bool isCompliant)",
        "bool IsCompliant { get; }",
    ]),
    bcl!("System.ParamArrayAttribute", Class, flags = BclTypeFlags::SEALED, base = "Attribute"; [
        "ParamArrayAttribute()",
    ]),
    bcl!("System.ThreadStaticAttribute", Class, base = "Attribute"; [
        "ThreadStaticAttribute()",
    ]),
    bcl_exception!("System.SystemException", "SystemException", "Exception"),
    bcl!("System.ArgumentException", Class, base = "SystemException"; [
        "ArgumentException()",
        "ArgumentException(string? message)",
        "ArgumentException(string? message, Exception? innerException)",
        "ArgumentException(string? message, string? paramName)",
        "virtual string? ParamName { get; }",
        "static void ThrowIfNullOrEmpty(string? argument, string? paramName = null)",
    ]),
    bcl!("System.ArgumentNullException", Class, base = "ArgumentException"; [
        "ArgumentNullException()",
        "ArgumentNullException(string? paramName)",
        "ArgumentNullException(string? paramName, string? message)",
        "static void ThrowIfNull(object? argument, string? paramName = null)",
    ]),
    bcl!("System.ArgumentOutOfRangeException", Class, base = "ArgumentException"; [
        "ArgumentOutOfRangeException()",
        "ArgumentOutOfRangeException(string? paramName)",
        "ArgumentOutOfRangeException(string? paramName, string? message)",
        "ArgumentOutOfRangeException(string? paramName, object? actualValue, string? message)",
        "virtual object? ActualValue { get; }",
    ]),
    bcl_exception!("System.InvalidOperationException", "InvalidOperationException", "SystemException"),
    bcl_exception!("System.NotImplementedException", "NotImplementedException", "SystemException"),
    bcl_exception!("System.NotSupportedException", "NotSupportedException", "SystemException"),
    bcl_exception!("System.NullReferenceException", "NullReferenceException", "SystemException"),
    bcl_exception!("System.IndexOutOfRangeException", "IndexOutOfRangeException", "SystemException"),
    bcl_exception!("System.InvalidCastException", "InvalidCastException", "SystemException"),
    bcl_exception!("System.FormatException", "FormatException", "SystemException"),
    bcl_exception!("System.ArithmeticException", "ArithmeticException", "SystemException"),
    bcl_exception!("System.OverflowException", "OverflowException", "ArithmeticException"),
    bcl_exception!("System.DivideByZeroException", "DivideByZeroException", "ArithmeticException"),
    bcl_exception!("System.TimeoutException", "TimeoutException", "SystemException"),
    bcl_exception!("System.UnauthorizedAccessException", "UnauthorizedAccessException", "SystemException"),
    bcl_exception!("System.OutOfMemoryException", "OutOfMemoryException", "SystemException"),
    bcl_exception!("System.StackOverflowException", "StackOverflowException", "SystemException"),
    bcl!("System.ObjectDisposedException", Class, base = "InvalidOperationException"; [
        "ObjectDisposedException(string? objectName)",
        "ObjectDisposedException(string? objectName, string? message)",
        "ObjectDisposedException(string? message, Exception? innerException)",
        "string ObjectName { get; }",
        "static void ThrowIf(bool condition, object instance)",
    ]),
    bcl!("System.OperationCanceledException", Class, base = "SystemException"; [
        "OperationCanceledException()",
        "OperationCanceledException(string? message)",
        "OperationCanceledException(string? message, Exception? innerException)",
        "OperationCanceledException(CancellationToken token)",
        "OperationCanceledException(string? message, CancellationToken token)",
        "CancellationToken CancellationToken { get; }",
    ]),
    bcl!("System.AggregateException", Class, base = "Exception"; [
        "AggregateException()",
        "AggregateException(string? message)",
        "AggregateException(IEnumerable<Exception> innerExceptions)",
        "AggregateException(params Exception[] innerExceptions)",
        "AggregateException(string? message, Exception innerException)",
        "ReadOnlyCollection<Exception> InnerExceptions { get; }",
        "AggregateException Flatten()",
        "void Handle(Func<Exception, bool> predicate)",
    ]),
    // ── System.Collections ──
    bcl!("System.Collections.IEnumerator", Interface; [
        "object Current { get; }",
        "bool MoveNext()",
        "void Reset()",
    ]),
    bcl!("System.Collections.ICollection", Interface, implements = ["IEnumerable"]; [
        "int Count { get; }",
        "bool IsSynchronized { get; }",
        "object SyncRoot { get; }",
        "void CopyTo(Array array, int index)",
    ]),
    bcl!("System.Collections.IList", Interface, implements = ["ICollection", "IEnumerable"]; [
        "object? this[int index] { get; set; }",
        "bool IsReadOnly { get; }",
        "bool IsFixedSize { get; }",
        "int Add(object? value)",
        "void Clear()",
        "bool Contains(object? value)",
        "int IndexOf(object? value)",
        "void Insert(int index, object? value)",
        "void Remove(object? value)",
        "void RemoveAt(int index)",
    ]),
    // ── System.Collections.Generic ──
    bcl!("System.Collections.Generic.IEnumerator`1", Interface, tp = ["out T"], implements = ["IEnumerator", "IDisposable"]; [
        "new T Current { get; }",
    ]),
    bcl!("System.Collections.Generic.IReadOnlyCollection`1", Interface, tp = ["out T"], implements = ["IEnumerable<T>"]; [
        "int Count { get; }",
    ]),
    bcl!("System.Collections.Generic.IReadOnlyList`1", Interface, tp = ["out T"],
        implements = ["IReadOnlyCollection<T>", "IEnumerable<T>"]; [
        "T this[int index] { get; }",
    ]),
    bcl!("System.Collections.Generic.ISet`1", Interface, tp = ["T"], implements = ["ICollection<T>", "IEnumerable<T>"]; [
        "new bool Add(T item)",
        "void UnionWith(IEnumerable<T> other)",
        "void IntersectWith(IEnumerable<T> other)",
        "void ExceptWith(IEnumerable<T> other)",
        "void SymmetricExceptWith(IEnumerable<T> other)",
        "bool IsSubsetOf(IEnumerable<T> other)",
        "bool IsSupersetOf(IEnumerable<T> other)",
        "bool IsProperSubsetOf(IEnumerable<T> other)",
        "bool IsProperSupersetOf(IEnumerable<T> other)",
        "bool Overlaps(IEnumerable<T> other)",
        "bool SetEquals(IEnumerable<T> other)",
    ]),
    bcl!("System.Collections.Generic.IReadOnlySet`1", Interface, tp = ["T"], implements = ["IReadOnlyCollection<T>", "IEnumerable<T>"]; [
        "bool Contains(T item)",
        "bool IsSubsetOf(IEnumerable<T> other)",
        "bool IsSupersetOf(IEnumerable<T> other)",
        "bool Overlaps(IEnumerable<T> other)",
        "bool SetEquals(IEnumerable<T> other)",
    ]),
    bcl!("System.Collections.Generic.IDictionary`2", Interface, tp = ["TKey", "TValue"],
        implements = ["ICollection<KeyValuePair<TKey, TValue>>", "IEnumerable<KeyValuePair<TKey, TValue>>"]; [
        "TValue this[TKey key] { get; set; }",
        "ICollection<TKey> Keys { get; }",
        "ICollection<TValue> Values { get; }",
        "void Add(TKey key, TValue value)",
        "bool ContainsKey(TKey key)",
        "bool Remove(TKey key)",
        "bool TryGetValue(TKey key, out TValue value)",
    ]),
    bcl!("System.Collections.Generic.IReadOnlyDictionary`2", Interface, tp = ["TKey", "TValue"],
        implements = ["IReadOnlyCollection<KeyValuePair<TKey, TValue>>", "IEnumerable<KeyValuePair<TKey, TValue>>"]; [
        "TValue this[TKey key] { get; }",
        "IEnumerable<TKey> Keys { get; }",
        "IEnumerable<TValue> Values { get; }",
        "bool ContainsKey(TKey key)",
        "bool TryGetValue(TKey key, out TValue value)",
    ]),
    bcl!("System.Collections.Generic.KeyValuePair`2", Struct, flags = BclTypeFlags::READONLY, tp = ["TKey", "TValue"]; [
        "KeyValuePair(TKey key, TValue value)",
        "TKey Key { get; }",
        "TValue Value { get; }",
        "void Deconstruct(out TKey key, out TValue value)",
        "override string ToString()",
    ]),
    bcl!("System.Collections.Generic.Queue`1", Class, tp = ["T"], implements = ["IEnumerable<T>", "IReadOnlyCollection<T>"]; [
        "Queue()",
        "Queue(int capacity)",
        "Queue(IEnumerable<T> collection)",
        "int Count { get; }",
        "void Enqueue(T item)",
        "T Dequeue()",
        "bool TryDequeue(out T result)",
        "T Peek()",
        "bool TryPeek(out T result)",
        "bool Contains(T item)",
        "void Clear()",
        "T[] ToArray()",
        "IEnumerator<T> GetEnumerator()",
    ]),
    bcl!("System.Collections.Generic.Stack`1", Class, tp = ["T"], implements = ["IEnumerable<T>", "IReadOnlyCollection<T>"]; [
        "Stack()",
        "Stack(int capacity)",
        "Stack(IEnumerable<T> collection)",
        "int Count { get; }",
        "void Push(T item)",
        "T Pop()",
        "bool TryPop(out T result)",
        "T Peek()",
        "bool TryPeek(out T result)",
        "bool Contains(T item)",
        "void Clear()",
        "T[] ToArray()",
        "IEnumerator<T> GetEnumerator()",
    ]),
    bcl!("System.Collections.Generic.LinkedList`1", Class, tp = ["T"],
        implements = ["ICollection<T>", "IEnumerable<T>", "IReadOnlyCollection<T>"]; [
        "LinkedList()",
        "LinkedList(IEnumerable<T> collection)",
        "int Count { get; }",
        "LinkedListNode<T>? First { get; }",
        "LinkedListNode<T>? Last { get; }",
        "LinkedListNode<T> AddFirst(T value)",
        "LinkedListNode<T> AddLast(T value)",
        "LinkedListNode<T> AddAfter(LinkedListNode<T> node, T value)",
        "LinkedListNode<T> AddBefore(LinkedListNode<T> node, T value)",
        "LinkedListNode<T>? Find(T value)",
        "bool Remove(T value)",
        "void Remove(LinkedListNode<T> node)",
        "void RemoveFirst()",
        "void RemoveLast()",
        "bool Contains(T value)",
        "void Clear()",
        "IEnumerator<T> GetEnumerator()",
    ]),
    bcl!("System.Collections.Generic.LinkedListNode`1", Class, flags = BclTypeFlags::SEALED, tp = ["T"]; [
        "LinkedListNode(T value)",
        "T Value { get; set; }",
        "LinkedListNode<T>? Next { get; }",
        "LinkedListNode<T>? Previous { get; }",
        "LinkedList<T>? List { get; }",
    ]),
    bcl!("System.Collections.Generic.SortedDictionary`2", Class, tp = ["TKey", "TValue"],
        implements = ["IDictionary<TKey, TValue>", "IReadOnlyDictionary<TKey, TValue>"],
        constraints = ["TKey : notnull"]; [
        "SortedDictionary()",
        "SortedDictionary(IComparer<TKey>? comparer)",
        "SortedDictionary(IDictionary<TKey, TValue> dictionary)",
        "int Count { get; }",
        "TValue this[TKey key] { get; set; }",
        "ICollection<TKey> Keys { get; }",
        "ICollection<TValue> Values { get; }",
        "void Add(TKey key, TValue value)",
        "bool Remove(TKey key)",
        "bool ContainsKey(TKey key)",
        "bool TryGetValue(TKey key, out TValue value)",
        "void Clear()",
        "IEnumerator<KeyValuePair<TKey, TValue>> GetEnumerator()",
    ]),
    bcl!("System.Collections.Generic.SortedSet`1", Class, tp = ["T"],
        implements = ["ISet<T>", "IReadOnlySet<T>", "ICollection<T>", "IEnumerable<T>"]; [
        "SortedSet()",
        "SortedSet(IComparer<T>? comparer)",
        "SortedSet(IEnumerable<T> collection)",
        "int Count { get; }",
        "T? Min { get; }",
        "T? Max { get; }",
        "bool Add(T item)",
        "bool Remove(T item)",
        "bool Contains(T item)",
        "void Clear()",
        "IEnumerable<T> Reverse()",
        "SortedSet<T> GetViewBetween(T? lowerValue, T? upperValue)",
        "IEnumerator<T> GetEnumerator()",
    ]),
    bcl!("System.Collections.Generic.IComparer`1", Interface, tp = ["in T"]; [
        "int Compare(T? x, T? y)",
    ]),
    bcl!("System.Collections.Generic.IEqualityComparer`1", Interface, tp = ["in T"]; [
        "bool Equals(T? x, T? y)",
        "int GetHashCode(T obj)",
    ]),
    bcl!("System.Collections.Generic.Comparer`1", Class, flags = BclTypeFlags::ABSTRACT, tp = ["T"], implements = ["IComparer<T>"]; [
        "static Comparer<T> Default { get; }",
        "static Comparer<T> Create(Comparison<T> comparison)",
        "abstract int Compare(T? x, T? y)",
    ]),
    bcl!("System.Collections.Generic.EqualityComparer`1", Class, flags = BclTypeFlags::ABSTRACT, tp = ["T"],
        implements = ["IEqualityComparer<T>"]; [
        "static EqualityComparer<T> Default { get; }",
        "abstract bool Equals(T? x, T? y)",
        "abstract int GetHashCode(T obj)",
    ]),
    bcl_exception!("System.Collections.Generic.KeyNotFoundException", "KeyNotFoundException", "SystemException"),
    bcl!("System.Collections.Generic.IAsyncEnumerable`1", Interface, tp = ["out T"]; [
        "IAsyncEnumerator<T> GetAsyncEnumerator(CancellationToken cancellationToken = default)",
    ]),
    bcl!("System.Collections.Generic.IAsyncEnumerator`1", Interface, tp = ["out T"], implements = ["IAsyncDisposable"]; [
        "T Current { get; }",
        "ValueTask<bool> MoveNextAsync()",
    ]),
    // ── System.Collections.ObjectModel ──
    bcl!("System.Collections.ObjectModel.Collection`1", Class, tp = ["T"],
        implements = ["IList<T>", "ICollection<T>", "IEnumerable<T>", "IReadOnlyList<T>", "IReadOnlyCollection<T>"]; [
        "Collection()",
        "Collection(IList<T> list)",
        "int Count { get; }",
        "T this[int index] { get; set; }",
        "void Add(T item)",
        "void Clear()",
        "bool Contains(T item)",
        "int IndexOf(T item)",
        "void Insert(int index, T item)",
        "bool Remove(T item)",
        "void RemoveAt(int index)",
        "IEnumerator<T> GetEnumerator()",
        "protected virtual void InsertItem(int index, T item)",
        "protected virtual void RemoveItem(int index)",
        "protected virtual void SetItem(int index, T item)",
        "protected virtual void ClearItems()",
    ]),
    bcl!("System.Collections.ObjectModel.ReadOnlyCollection`1", Class, tp = ["T"],
        implements = ["IList<T>", "IReadOnlyList<T>", "IEnumerable<T>"]; [
        "ReadOnlyCollection(IList<T> list)",
        "int Count { get; }",
        "T this[int index] { get; }",
        "bool Contains(T value)",
        "int IndexOf(T value)",
        "IEnumerator<T> GetEnumerator()",
    ]),
    bcl!("System.Collections.ObjectModel.ObservableCollection`1", Class, tp = ["T"], base = "Collection<T>"; [
        "ObservableCollection()",
        "ObservableCollection(IEnumerable<T> collection)",
        "void Move(int oldIndex, int newIndex)",
    ]),
    // ── System.Linq ──
    bcl!("System.Linq.IGrouping`2", Interface, tp = ["out TKey", "out TElement"], implements = ["IEnumerable<TElement>"]; [
        "TKey Key { get; }",
    ]),
    bcl!("System.Linq.IOrderedEnumerable`1", Interface, tp = ["out TElement"], implements = ["IEnumerable<TElement>"]; []),
    bcl!("System.Linq.ILookup`2", Interface, tp = ["TKey", "TElement"], implements = ["IEnumerable<IGrouping<TKey, TElement>>"]; [
        "int Count { get; }",
        "IEnumerable<TElement> this[TKey key] { get; }",
        "bool Contains(TKey key)",
    ]),
    bcl!("System.Linq.IQueryable`1", Interface, tp = ["out T"], implements = ["IEnumerable<T>"]; [
        "Type ElementType { get; }",
    ]),
    // ── System.Threading ──
    bcl!("System.Threading.CancellationToken", Struct, flags = BclTypeFlags::READONLY, implements = ["IEquatable<CancellationToken>"]; [
        "CancellationToken(bool canceled)",
        "static CancellationToken None { get; }",
        "bool IsCancellationRequested { get; }",
        "bool CanBeCanceled { get; }",
        "void ThrowIfCancellationRequested()",
        "CancellationTokenRegistration Register(Action callback)",
        "bool Equals(CancellationToken other)",
    ]),
    bcl!("System.Threading.CancellationTokenRegistration", Struct, flags = BclTypeFlags::READONLY,
        implements = ["IDisposable", "IAsyncDisposable"]; [
        "void Dispose()",
        "ValueTask DisposeAsync()",
        "bool Unregister()",
    ]),
    bcl!("System.Threading.CancellationTokenSource", Class, implements = ["IDisposable"]; [
        "CancellationTokenSource()",
        "CancellationTokenSource(int millisecondsDelay)",
        "CancellationTokenSource(TimeSpan delay)",
        "CancellationToken Token { get; }",
        "bool IsCancellationRequested { get; }",
        "void Cancel()",
        "void Cancel(bool throwOnFirstException)",
        "Task CancelAsync()",
        "void CancelAfter(int millisecondsDelay)",
        "void CancelAfter(TimeSpan delay)",
        "bool TryReset()",
        "void Dispose()",
        "static CancellationTokenSource CreateLinkedTokenSource(params CancellationToken[] tokens)",
    ]),
    bcl!("System.Threading.SemaphoreSlim", Class, implements = ["IDisposable"]; [
        "SemaphoreSlim(int initialCount)",
        "SemaphoreSlim(int initialCount, int maxCount)",
        "int CurrentCount { get; }",
        "void Wait()",
        "bool Wait(int millisecondsTimeout)",
        "void Wait(CancellationToken cancellationToken)",
        "Task WaitAsync()",
        "Task WaitAsync(CancellationToken cancellationToken)",
        "Task<bool> WaitAsync(int millisecondsTimeout)",
        "Task<bool> WaitAsync(TimeSpan timeout)",
        "int Release()",
        "int Release(int releaseCount)",
        "void Dispose()",
    ]),
    bcl!("System.Threading.Interlocked", Class, flags = BclTypeFlags::STATIC; [
        "static int Increment(ref int location)",
        "static long Increment(ref long location)",
        "static int Decrement(ref int location)",
        "static long Decrement(ref long location)",
        "static int Add(ref int location1, int value)",
        "static long Add(ref long location1, long value)",
        "static int Exchange(ref int location1, int value)",
        "static T Exchange<T>(ref T location1, T value) where T : class?",
        "static int CompareExchange(ref int location1, int value, int comparand)",
        "static T CompareExchange<T>(ref T location1, T value, T comparand) where T : class?",
        "static long Read(ref long location)",
    ]),
    bcl!("System.Threading.Monitor", Class, flags = BclTypeFlags::STATIC; [
        "static void Enter(object obj)",
        "static void Enter(object obj, ref bool lockTaken)",
        "static void Exit(object obj)",
        "static bool TryEnter(object obj)",
        "static bool TryEnter(object obj, int millisecondsTimeout)",
        "static bool Wait(object obj)",
        "static void Pulse(object obj)",
        "static void PulseAll(object obj)",
    ]),
    bcl!("System.Threading.Thread", Class, flags = BclTypeFlags::SEALED; [
        "Thread(ThreadStart start)",
        "static Thread CurrentThread { get; }",
        "int ManagedThreadId { get; }",
        "string? Name { get; set; }",
        "bool IsBackground { get; set; }",
        "bool IsAlive { get; }",
        "void Start()",
        "void Join()",
        "bool Join(int millisecondsTimeout)",
        "static void Sleep(int millisecondsTimeout)",
        "static void Sleep(TimeSpan timeout)",
        "static bool Yield()",
    ]),
    bcl_delegate!("System.Threading.ThreadStart", [], "void Invoke()"),
    bcl!("System.Threading.Timeout", Class, flags = BclTypeFlags::STATIC; [
        "const int Infinite",
        "static readonly TimeSpan InfiniteTimeSpan",
    ]),
    bcl!("System.Threading.LazyThreadSafetyMode", Enum; ["None", "PublicationOnly", "ExecutionAndPublication"]),
    // ── System.Threading.Tasks ──
    bcl!("System.Threading.Tasks.TaskStatus", Enum; [
        "Created", "WaitingForActivation", "WaitingToRun", "Running", "WaitingForChildrenToComplete",
        "RanToCompletion", "Canceled", "Faulted",
    ]),
    bcl!("System.Threading.Tasks.TaskCompletionSource", Class; [
        "TaskCompletionSource()",
        "TaskCompletionSource(object? state)",
        "Task Task { get; }",
        "void SetResult()",
        "bool TrySetResult()",
        "void SetException(Exception exception)",
        "bool TrySetException(Exception exception)",
        "void SetCanceled()",
        "bool TrySetCanceled()",
    ]),
    bcl!("System.Threading.Tasks.TaskCompletionSource`1", Class, tp = ["TResult"]; [
        "TaskCompletionSource()",
        "TaskCompletionSource(object? state)",
        "Task<TResult> Task { get; }",
        "void SetResult(TResult result)",
        "bool TrySetResult(TResult result)",
        "void SetException(Exception exception)",
        "bool TrySetException(Exception exception)",
        "void SetCanceled()",
        "bool TrySetCanceled()",
        "bool TrySetCanceled(CancellationToken cancellationToken)",
    ]),
    bcl!("System.Threading.Tasks.TaskCanceledException", Class, base = "OperationCanceledException"; [
        "TaskCanceledException()",
        "TaskCanceledException(string? message)",
        "TaskCanceledException(string? message, Exception? innerException)",
        "TaskCanceledException(Task? task)",
        "Task? Task { get; }",
    ]),
    bcl!("System.Threading.Tasks.Parallel", Class, flags = BclTypeFlags::STATIC; [
        "static ParallelLoopResult For(int fromInclusive, int toExclusive, Action<int> body)",
        "static ParallelLoopResult ForEach<TSource>(IEnumerable<TSource> source, Action<TSource> body)",
        "static Task ForEachAsync<TSource>(IEnumerable<TSource> source, Func<TSource, CancellationToken, ValueTask> body)",
        "static void Invoke(params Action[] actions)",
    ]),
    bcl!("System.Threading.Tasks.ParallelLoopResult", Struct, flags = BclTypeFlags::READONLY; [
        "bool IsCompleted { get; }",
    ]),
    // ── System.Runtime.CompilerServices ──
    bcl!("System.Runtime.CompilerServices.TaskAwaiter", Struct, flags = BclTypeFlags::READONLY; [
        "bool IsCompleted { get; }",
        "void GetResult()",
        "void OnCompleted(Action continuation)",
    ]),
    bcl!("System.Runtime.CompilerServices.TaskAwaiter`1", Struct, flags = BclTypeFlags::READONLY, tp = ["TResult"]; [
        "bool IsCompleted { get; }",
        "TResult GetResult()",
        "void OnCompleted(Action continuation)",
    ]),
    bcl!("System.Runtime.CompilerServices.ConfiguredTaskAwaitable", Struct, flags = BclTypeFlags::READONLY; [
        "ConfiguredTaskAwaiter GetAwaiter()",
    ]),
    bcl!("System.Runtime.CompilerServices.ConfiguredTaskAwaitable`1", Struct, flags = BclTypeFlags::READONLY, tp = ["TResult"]; [
        "ConfiguredTaskAwaiter GetAwaiter()",
    ]),
    bcl!("System.Runtime.CompilerServices.ConfiguredTaskAwaiter", Struct, flags = BclTypeFlags::READONLY; [
        "bool IsCompleted { get; }",
        "void GetResult()",
    ]),
    bcl!("System.Runtime.CompilerServices.YieldAwaitable", Struct, flags = BclTypeFlags::READONLY; []),
    bcl!("System.Runtime.CompilerServices.CallerMemberNameAttribute", Class, flags = BclTypeFlags::SEALED, base = "Attribute"; [
        "CallerMemberNameAttribute()",
    ]),
    bcl!("System.Runtime.CompilerServices.CallerFilePathAttribute", Class, flags = BclTypeFlags::SEALED, base = "Attribute"; [
        "CallerFilePathAttribute()",
    ]),
    bcl!("System.Runtime.CompilerServices.CallerLineNumberAttribute", Class, flags = BclTypeFlags::SEALED, base = "Attribute"; [
        "CallerLineNumberAttribute()",
    ]),
    bcl!("System.Runtime.CompilerServices.InternalsVisibleToAttribute", Class, flags = BclTypeFlags::SEALED, base = "Attribute"; [
        "InternalsVisibleToAttribute(string assemblyName)",
        "string AssemblyName { get; }",
    ]),
    // ── System.IO ──
    bcl!("System.IO.Stream", Class, flags = BclTypeFlags::ABSTRACT, implements = ["IDisposable", "IAsyncDisposable"]; [
        "protected Stream()",
        "static readonly Stream Null",
        "abstract bool CanRead { get; }",
        "abstract bool CanWrite { get; }",
        "abstract bool CanSeek { get; }",
        "abstract long Length { get; }",
        "abstract long Position { get; set; }",
        "abstract int Read(byte[] buffer, int offset, int count)",
        "virtual int Read(Span<byte> buffer)",
        "abstract void Write(byte[] buffer, int offset, int count)",
        "virtual void Write(ReadOnlySpan<byte> buffer)",
        "abstract long Seek(long offset, SeekOrigin origin)",
        "abstract void SetLength(long value)",
        "abstract void Flush()",
        "virtual int ReadByte()",
        "virtual void WriteByte(byte value)",
        "void CopyTo(Stream destination)",
        "Task CopyToAsync(Stream destination)",
        "Task CopyToAsync(Stream destination, CancellationToken cancellationToken)",
        "Task<int> ReadAsync(byte[] buffer, int offset, int count)",
        "virtual Task<int> ReadAsync(byte[] buffer, int offset, int count, CancellationToken cancellationToken)",
        "virtual ValueTask<int> ReadAsync(Memory<byte> buffer, CancellationToken cancellationToken = default)",
        "Task WriteAsync(byte[] buffer, int offset, int count)",
        "virtual Task WriteAsync(byte[] buffer, int offset, int count, CancellationToken cancellationToken)",
        "virtual ValueTask WriteAsync(ReadOnlyMemory<byte> buffer, CancellationToken cancellationToken = default)",
        "Task FlushAsync()",
        "virtual Task FlushAsync(CancellationToken cancellationToken)",
        "virtual void Close()",
        "void Dispose()",
        "protected virtual void Dispose(bool disposing)",
        "virtual ValueTask DisposeAsync()",
    ]),
    bcl!("System.IO.MemoryStream", Class, base = "Stream"; [
        "MemoryStream()",
        "MemoryStream(int capacity)",
        "MemoryStream(byte[] buffer)",
        "MemoryStream(byte[] buffer, bool writable)",
        "virtual int Capacity { get; set; }",
        "override bool CanRead { get; }",
        "override bool CanWrite { get; }",
        "override bool CanSeek { get; }",
        "override long Length { get; }",
        "override long Position { get; set; }",
        "override int Read(byte[] buffer, int offset, int count)",
        "override void Write(byte[] buffer, int offset, int count)",
        "override long Seek(long offset, SeekOrigin loc)",
        "override void SetLength(long value)",
        "override void Flush()",
        "virtual byte[] ToArray()",
        "virtual byte[] GetBuffer()",
        "virtual void WriteTo(Stream stream)",
    ]),
    bcl!("System.IO.FileStream", Class, base = "Stream"; [
        "FileStream(string path, FileMode mode)",
        "FileStream(string path, FileMode mode, FileAccess access)",
        "FileStream(string path, FileMode mode, FileAccess access, FileShare share)",
        "virtual string Name { get; }",
        "override bool CanRead { get; }",
        "override bool CanWrite { get; }",
        "override bool CanSeek { get; }",
        "override long Length { get; }",
        "override long Position { get; set; }",
        "override int Read(byte[] buffer, int offset, int count)",
        "override void Write(byte[] buffer, int offset, int count)",
        "override long Seek(long offset, SeekOrigin origin)",
        "override void SetLength(long value)",
        "override void Flush()",
        "virtual void Flush(bool flushToDisk)",
    ]),
    bcl!("System.IO.TextReader", Class, flags = BclTypeFlags::ABSTRACT, implements = ["IDisposable"]; [
        "protected TextReader()",
        "static readonly TextReader Null",
        "virtual int Peek()",
        "virtual int Read()",
        "virtual string? ReadLine()",
        "virtual string ReadToEnd()",
        "virtual Task<string?> ReadLineAsync()",
        "virtual Task<string> ReadToEndAsync()",
        "virtual void Close()",
        "void Dispose()",
    ]),
    bcl!("System.IO.TextWriter", Class, flags = BclTypeFlags::ABSTRACT, implements = ["IDisposable", "IAsyncDisposable"]; [
        "protected TextWriter()",
        "static readonly TextWriter Null",
        "abstract Encoding Encoding { get; }",
        "virtual string NewLine { get; set; }",
        "virtual void Write(char value)",
        "virtual void Write(string? value)",
        "virtual void Write(object? value)",
        "virtual void Write(int value)",
        "virtual void Write(string format, params object?[] arg)",
        "virtual void WriteLine()",
        "virtual void WriteLine(string? value)",
        "virtual void WriteLine(object? value)",
        "virtual void WriteLine(int value)",
        "virtual void WriteLine(string format, params object?[] arg)",
        "virtual Task WriteAsync(string? value)",
        "virtual Task WriteLineAsync(string? value)",
        "virtual void Flush()",
        "virtual Task FlushAsync()",
        "virtual void Close()",
        "void Dispose()",
        "virtual ValueTask DisposeAsync()",
    ]),
    bcl!("System.IO.StreamReader", Class, base = "TextReader"; [
        "StreamReader(Stream stream)",
        "StreamReader(Stream stream, Encoding encoding)",
        "StreamReader(string path)",
        "StreamReader(string path, Encoding encoding)",
        "virtual Stream BaseStream { get; }",
        "virtual Encoding CurrentEncoding { get; }",
        "bool EndOfStream { get; }",
        "override int Peek()",
        "override int Read()",
        "override string? ReadLine()",
        "override string ReadToEnd()",
        "override Task<string?> ReadLineAsync()",
        "override Task<string> ReadToEndAsync()",
    ]),
    bcl!("System.IO.StreamWriter", Class, base = "TextWriter"; [
        "StreamWriter(Stream stream)",
        "StreamWriter(Stream stream, Encoding encoding)",
        "StreamWriter(string path)",
        "StreamWriter(string path, bool append)",
        "StreamWriter(string path, bool append, Encoding encoding)",
        "virtual bool AutoFlush { get; set; }",
        "virtual Stream BaseStream { get; }",
        "override Encoding Encoding { get; }",
        "override void Write(char value)",
        "override void Write(string? value)",
        "override void WriteLine(string? value)",
        "override void Flush()",
        "override Task FlushAsync()",
    ]),
    bcl!("System.IO.StringReader", Class, base = "TextReader"; [
        "StringReader(string s)",
        "override int Peek()",
        "override int Read()",
        "override string? ReadLine()",
        "override string ReadToEnd()",
    ]),
    bcl!("System.IO.StringWriter", Class, base = "TextWriter"; [
        "StringWriter()",
        "StringWriter(StringBuilder sb)",
        "override Encoding Encoding { get; }",
        "virtual StringBuilder GetStringBuilder()",
        "override void Write(char value)",
        "override void Write(string? value)",
        "override string ToString()",
    ]),
    bcl!("System.IO.File", Class, flags = BclTypeFlags::STATIC; [
        "static bool Exists(string? path)",
        "static string ReadAllText(string path)",
        "static string ReadAllText(string path, Encoding encoding)",
        "static string[] ReadAllLines(string path)",
        "static byte[] ReadAllBytes(string path)",
        "static IEnumerable<string> ReadLines(string path)",
        "static Task<string> ReadAllTextAsync(string path, CancellationToken cancellationToken = default)",
        "static Task<string[]> ReadAllLinesAsync(string path, CancellationToken cancellationToken = default)",
        "static Task<byte[]> ReadAllBytesAsync(string path, CancellationToken cancellationToken = default)",
        "static void WriteAllText(string path, string? contents)",
        "static void WriteAllText(string path, string? contents, Encoding encoding)",
        "static void WriteAllLines(string path, IEnumerable<string> contents)",
        "static void WriteAllBytes(string path, byte[] bytes)",
        "static Task WriteAllTextAsync(string path, string? contents, CancellationToken cancellationToken = default)",
        "static Task WriteAllBytesAsync(string path, byte[] bytes, CancellationToken cancellationToken = default)",
        "static void AppendAllText(string path, string? contents)",
        "static void Copy(string sourceFileName, string destFileName)",
        "static void Copy(string sourceFileName, string destFileName, bool overwrite)",
        "static void Move(string sourceFileName, string destFileName)",
        "static void Delete(string path)",
        "static FileStream Open(string path, FileMode mode)",
        "static FileStream OpenRead(string path)",
        "static FileStream OpenWrite(string path)",
        "static FileStream Create(string path)",
        "static StreamReader OpenText(string path)",
        "static StreamWriter CreateText(string path)",
        "static StreamWriter AppendText(string path)",
        "static DateTime GetLastWriteTime(string path)",
    ]),
    bcl!("System.IO.Directory", Class, flags = BclTypeFlags::STATIC; [
        "static bool Exists(string? path)",
        "static DirectoryInfo CreateDirectory(string path)",
        "static void Delete(string path)",
        "static void Delete(string path, bool recursive)",
        "static void Move(string sourceDirName, string destDirName)",
        "static string[] GetFiles(string path)",
        "static string[] GetFiles(string path, string searchPattern)",
        "static string[] GetFiles(string path, string searchPattern, SearchOption searchOption)",
        "static string[] GetDirectories(string path)",
        "static IEnumerable<string> EnumerateFiles(string path)",
        "static IEnumerable<string> EnumerateFiles(string path, string searchPattern, SearchOption searchOption)",
        "static IEnumerable<string> EnumerateDirectories(string path)",
        "static string GetCurrentDirectory()",
        "static void SetCurrentDirectory(string path)",
    ]),
    bcl!("System.IO.Path", Class, flags = BclTypeFlags::STATIC; [
        "static readonly char DirectorySeparatorChar",
        "static readonly char AltDirectorySeparatorChar",
        "static readonly char PathSeparator",
        "static string Combine(string path1, string path2)",
        "static string Combine(string path1, string path2, string path3)",
        "static string Combine(params string[] paths)",
        "static string Join(string? path1, string? path2)",
        "static string? GetFileName(string? path)",
        "static string? GetFileNameWithoutExtension(string? path)",
        "static string? GetExtension(string? path)",
        "static string? GetDirectoryName(string? path)",
        "static string GetFullPath(string path)",
        "static string GetTempPath()",
        "static string GetTempFileName()",
        "static string GetRandomFileName()",
        "static string? ChangeExtension(string? path, string? extension)",
        "static bool HasExtension(string? path)",
        "static bool IsPathRooted(string? path)",
        "static string GetRelativePath(string relativeTo, string path)",
        "static char[] GetInvalidFileNameChars()",
    ]),
    bcl!("System.IO.FileSystemInfo", Class, flags = BclTypeFlags::ABSTRACT; [
        "abstract string Name { get; }",
        "virtual string FullName { get; }",
        "string Extension { get; }",
        "abstract bool Exists { get; }",
        "DateTime CreationTime { get; set; }",
        "DateTime LastWriteTime { get; set; }",
        "abstract void Delete()",
        "void Refresh()",
    ]),
    bcl!("System.IO.FileInfo", Class, flags = BclTypeFlags::SEALED, base = "FileSystemInfo"; [
        "FileInfo(string fileName)",
        "override string Name { get; }",
        "override bool Exists { get; }",
        "long Length { get; }",
        "string? DirectoryName { get; }",
        "DirectoryInfo? Directory { get; }",
        "bool IsReadOnly { get; set; }",
        "override void Delete()",
        "FileInfo CopyTo(string destFileName)",
        "void MoveTo(string destFileName)",
        "FileStream OpenRead()",
        "FileStream OpenWrite()",
        "StreamReader OpenText()",
        "FileStream Create()",
    ]),
    bcl!("System.IO.DirectoryInfo", Class, flags = BclTypeFlags::SEALED, base = "FileSystemInfo"; [
        "DirectoryInfo(string path)",
        "override string Name { get; }",
        "override bool Exists { get; }",
        "DirectoryInfo? Parent { get; }",
        "DirectoryInfo Root { get; }",
        "void Create()",
        "DirectoryInfo CreateSubdirectory(string path)",
        "override void Delete()",
        "void Delete(bool recursive)",
        "FileInfo[] GetFiles()",
        "FileInfo[] GetFiles(string searchPattern)",
        "DirectoryInfo[] GetDirectories()",
        "IEnumerable<FileInfo> EnumerateFiles()",
        "IEnumerable<DirectoryInfo> EnumerateDirectories()",
    ]),
    bcl!("System.IO.FileMode", Enum; ["CreateNew", "Create", "Open", "OpenOrCreate", "Truncate", "Append"]),
    bcl!("System.IO.FileAccess", Enum, flags = BclTypeFlags::FLAGS_ENUM; ["Read", "Write", "ReadWrite"]),
    bcl!("System.IO.FileShare", Enum, flags = BclTypeFlags::FLAGS_ENUM; ["None", "Read", "Write", "ReadWrite", "Delete", "Inheritable"]),
    bcl!("System.IO.SeekOrigin", Enum; ["Begin", "Current", "End"]),
    bcl!("System.IO.SearchOption", Enum; ["TopDirectoryOnly", "AllDirectories"]),
    bcl_exception!("System.IO.IOException", "IOException", "SystemException"),
    bcl!("System.IO.FileNotFoundException", Class, base = "IOException"; [
        "FileNotFoundException()",
        "FileNotFoundException(string? message)",
        "FileNotFoundException(string? message, string? fileName)",
        "FileNotFoundException(string? message, Exception? innerException)",
        "string? FileName { get; }",
    ]),
    bcl_exception!("System.IO.DirectoryNotFoundException", "DirectoryNotFoundException", "IOException"),
    bcl_exception!("System.IO.EndOfStreamException", "EndOfStreamException", "IOException"),
    // ── System.Text ──
    bcl!("System.Text.Encoding", Class, flags = BclTypeFlags::ABSTRACT, implements = ["ICloneable"]; [
        "static Encoding UTF8 { get; }",
        "static Encoding ASCII { get; }",
        "static Encoding Unicode { get; }",
        "static Encoding BigEndianUnicode { get; }",
        "static Encoding UTF32 { get; }",
        "static Encoding Latin1 { get; }",
        "static Encoding Default { get; }",
        "virtual string EncodingName { get; }",
        "virtual string WebName { get; }",
        "virtual int CodePage { get; }",
        "virtual byte[] GetBytes(string s)",
        "virtual byte[] GetBytes(char[] chars)",
        "virtual string GetString(byte[] bytes)",
        "virtual string GetString(byte[] bytes, int index, int count)",
        "virtual int GetByteCount(string s)",
        "virtual byte[] GetPreamble()",
        "virtual object Clone()",
        "static Encoding GetEncoding(string name)",
        "static Encoding GetEncoding(int codepage)",
    ]),
    // ── System.Net ──
    bcl!("System.Net.HttpStatusCode", Enum, base = "int"; [
        "Continue", "SwitchingProtocols", "OK", "Created", "Accepted", "NoContent", "MovedPermanently",
        "Found", "Redirect", "NotModified", "TemporaryRedirect", "PermanentRedirect", "BadRequest",
        "Unauthorized", "PaymentRequired", "Forbidden", "NotFound", "MethodNotAllowed", "NotAcceptable",
        "RequestTimeout", "Conflict", "Gone", "PreconditionFailed", "UnsupportedMediaType",
        "UnprocessableEntity", "TooManyRequests", "InternalServerError", "NotImplemented", "BadGateway",
        "ServiceUnavailable", "GatewayTimeout",
    ]),
    // ── System.Net.Http ──
    bcl!("System.Net.Http.HttpMessageHandler", Class, flags = BclTypeFlags::ABSTRACT, implements = ["IDisposable"]; [
        "protected HttpMessageHandler()",
        "protected abstract Task<HttpResponseMessage> SendAsync(HttpRequestMessage request, CancellationToken cancellationToken)",
        "protected virtual HttpResponseMessage Send(HttpRequestMessage request, CancellationToken cancellationToken)",
        "void Dispose()",
        "protected virtual void Dispose(bool disposing)",
    ]),
    bcl!("System.Net.Http.HttpClientHandler", Class, base = "HttpMessageHandler"; [
        "HttpClientHandler()",
        "bool AllowAutoRedirect { get; set; }",
        "bool UseCookies { get; set; }",
        "bool UseProxy { get; set; }",
        "int MaxConnectionsPerServer { get; set; }",
        "protected override Task<HttpResponseMessage> SendAsync(HttpRequestMessage request, CancellationToken cancellationToken)",
    ]),
    bcl!("System.Net.Http.DelegatingHandler", Class, flags = BclTypeFlags::ABSTRACT, base = "HttpMessageHandler"; [
        "protected DelegatingHandler()",
        "protected DelegatingHandler(HttpMessageHandler innerHandler)",
        "HttpMessageHandler? InnerHandler { get; set; }",
        "protected override Task<HttpResponseMessage> SendAsync(HttpRequestMessage request, CancellationToken cancellationToken)",
    ]),
    bcl!("System.Net.Http.HttpMessageInvoker", Class, implements = ["IDisposable"]; [
        "HttpMessageInvoker(HttpMessageHandler handler)",
        "HttpMessageInvoker(HttpMessageHandler handler, bool disposeHandler)",
        "virtual Task<HttpResponseMessage> SendAsync(HttpRequestMessage request, CancellationToken cancellationToken)",
        "virtual HttpResponseMessage Send(HttpRequestMessage request, CancellationToken cancellationToken)",
        "void Dispose()",
        "protected virtual void Dispose(bool disposing)",
    ]),
    bcl!("System.Net.Http.HttpClient", Class, base = "HttpMessageInvoker"; [
        "HttpClient()",
        "HttpClient(HttpMessageHandler handler)",
        "HttpClient(HttpMessageHandler handler, bool disposeHandler)",
        "Uri? BaseAddress { get; set; }",
        "TimeSpan Timeout { get; set; }",
        "long MaxResponseContentBufferSize { get; set; }",
        "HttpRequestHeaders DefaultRequestHeaders { get; }",
        "Task<string> GetStringAsync(string? requestUri)",
        "Task<string> GetStringAsync(Uri? requestUri)",
        "Task<string> GetStringAsync(string? requestUri, CancellationToken cancellationToken)",
        "Task<byte[]> GetByteArrayAsync(string? requestUri)",
        "Task<Stream> GetStreamAsync(string? requestUri)",
        "Task<HttpResponseMessage> GetAsync(string? requestUri)",
        "Task<HttpResponseMessage> GetAsync(Uri? requestUri)",
        "Task<HttpResponseMessage> GetAsync(string? requestUri, CancellationToken cancellationToken)",
        "Task<HttpResponseMessage> GetAsync(string? requestUri, HttpCompletionOption completionOption)",
        "Task<HttpResponseMessage> PostAsync(string? requestUri, HttpContent? content)",
        "Task<HttpResponseMessage> PostAsync(Uri? requestUri, HttpContent? content)",
        "Task<HttpResponseMessage> PostAsync(string? requestUri, HttpContent? content, CancellationToken cancellationToken)",
        "Task<HttpResponseMessage> PutAsync(string? requestUri, HttpContent? content)",
        "Task<HttpResponseMessage> PutAsync(string? requestUri, HttpContent? content, CancellationToken cancellationToken)",
        "Task<HttpResponseMessage> PatchAsync(string? requestUri, HttpContent? content)",
        "Task<HttpResponseMessage> DeleteAsync(string? requestUri)",
        "Task<HttpResponseMessage> DeleteAsync(string? requestUri, CancellationToken cancellationToken)",
        "Task<HttpResponseMessage> SendAsync(HttpRequestMessage request)",
        "Task<HttpResponseMessage> SendAsync(HttpRequestMessage request, HttpCompletionOption completionOption)",
        "override Task<HttpResponseMessage> SendAsync(HttpRequestMessage request, CancellationToken cancellationToken)",
        "void CancelPendingRequests()",
    ]),
    bcl!("System.Net.Http.HttpRequestMessage", Class, implements = ["IDisposable"]; [
        "HttpRequestMessage()",
        "HttpRequestMessage(HttpMethod method, string? requestUri)",
        "HttpRequestMessage(HttpMethod method, Uri? requestUri)",
        "HttpMethod Method { get; set; }",
        "Uri? RequestUri { get; set; }",
        "HttpContent? Content { get; set; }",
        "HttpRequestHeaders Headers { get; }",
        "Version Version { get; set; }",
        "void Dispose()",
        "override string ToString()",
    ]),
    bcl!("System.Net.Http.HttpResponseMessage", Class, implements = ["IDisposable"]; [
        "HttpResponseMessage()",
        "HttpResponseMessage(HttpStatusCode statusCode)",
        "HttpStatusCode StatusCode { get; set; }",
        "string? ReasonPhrase { get; set; }",
        "bool IsSuccessStatusCode { get; }",
        "HttpContent Content { get; set; }",
        "HttpResponseHeaders Headers { get; }",
        "HttpRequestMessage? RequestMessage { get; set; }",
        "Version Version { get; set; }",
        "HttpResponseMessage EnsureSuccessStatusCode()",
        "void Dispose()",
        "override string ToString()",
    ]),
    bcl!("System.Version", Class, flags = BclTypeFlags::SEALED,
        implements = ["ICloneable", "IComparable", "IComparable<Version?>", "IEquatable<Version?>"]; [
        "Version(int major, int minor)",
        "Version(int major, int minor, int build)",
        "Version(string version)",
        "int Major { get; }",
        "int Minor { get; }",
        "int Build { get; }",
        "int Revision { get; }",
        "object Clone()",
        "override string ToString()",
        "static Version Parse(string input)",
    ]),
    bcl!("System.Net.Http.HttpContent", Class, flags = BclTypeFlags::ABSTRACT, implements = ["IDisposable"]; [
        "protected HttpContent()",
        "HttpContentHeaders Headers { get; }",
        "Task<string> ReadAsStringAsync()",
        "Task<string> ReadAsStringAsync(CancellationToken cancellationToken)",
        "Task<byte[]> ReadAsByteArrayAsync()",
        "Task<Stream> ReadAsStreamAsync()",
        "Task<Stream> ReadAsStreamAsync(CancellationToken cancellationToken)",
        "Task CopyToAsync(Stream stream)",
        "Task LoadIntoBufferAsync()",
        "void Dispose()",
        "protected virtual void Dispose(bool disposing)",
        "protected abstract Task SerializeToStreamAsync(Stream stream, TransportContext? context)",
        "protected abstract bool TryComputeLength(out long length)",
    ]),
    bcl!("System.Net.TransportContext", Class, flags = BclTypeFlags::ABSTRACT; []),
    bcl!("System.Net.Http.ByteArrayContent", Class, base = "HttpContent"; [
        "ByteArrayContent(byte[] content)",
        "ByteArrayContent(byte[] content, int offset, int count)",
        "protected override Task SerializeToStreamAsync(Stream stream, TransportContext? context)",
        "protected override bool TryComputeLength(out long length)",
    ]),
    bcl!("System.Net.Http.StringContent", Class, base = "ByteArrayContent"; [
        "StringContent(string content)",
        "StringContent(string content, Encoding? encoding)",
        "StringContent(string content, Encoding? encoding, string mediaType)",
    ]),
    bcl!("System.Net.Http.StreamContent", Class, base = "HttpContent"; [
        "StreamContent(Stream content)",
        "StreamContent(Stream content, int bufferSize)",
        "protected override Task SerializeToStreamAsync(Stream stream, TransportContext? context)",
        "protected override bool TryComputeLength(out long length)",
    ]),
    bcl!("System.Net.Http.HttpMethod", Class, implements = ["IEquatable<HttpMethod>"]; [
        "HttpMethod(string method)",
        "static HttpMethod Get { get; }",
        "static HttpMethod Post { get; }",
        "static HttpMethod Put { get; }",
        "static HttpMethod Delete { get; }",
        "static HttpMethod Patch { get; }",
        "static HttpMethod Head { get; }",
        "static HttpMethod Options { get; }",
        "string Method { get; }",
        "bool Equals(HttpMethod? other)",
        "override string ToString()",
        "static bool operator ==(HttpMethod? left, HttpMethod? right)",
        "static bool operator !=(HttpMethod? left, HttpMethod? right)",
    ]),
    bcl!("System.Net.Http.HttpRequestException", Class, base = "Exception"; [
        "HttpRequestException()",
        "HttpRequestException(string? message)",
        "HttpRequestException(string? message, Exception? inner)",
        "HttpRequestException(string? message, Exception? inner, HttpStatusCode? statusCode)",
        "HttpStatusCode? StatusCode { get; }",
    ]),
    bcl!("System.Net.Http.HttpCompletionOption", Enum; ["ResponseContentRead", "ResponseHeadersRead"]),
    // ── System.Net.Http.Headers ──
    bcl!("System.Net.Http.Headers.HttpHeaders", Class, flags = BclTypeFlags::ABSTRACT,
        implements = ["IEnumerable<KeyValuePair<string, IEnumerable<string>>>"]; [
        "protected HttpHeaders()",
        "void Add(string name, string? value)",
        "void Add(string name, IEnumerable<string?> values)",
        "bool TryAddWithoutValidation(string name, string? value)",
        "bool Contains(string name)",
        "bool Remove(string name)",
        "void Clear()",
        "IEnumerable<string> GetValues(string name)",
        "bool TryGetValues(string name, out IEnumerable<string>? values)",
        "IEnumerator<KeyValuePair<string, IEnumerable<string>>> GetEnumerator()",
        "override string ToString()",
    ]),
    bcl!("System.Net.Http.Headers.HttpRequestHeaders", Class, flags = BclTypeFlags::SEALED, base = "HttpHeaders"; [
        "AuthenticationHeaderValue? Authorization { get; set; }",
        "string? Host { get; set; }",
    ]),
    bcl!("System.Net.Http.Headers.HttpResponseHeaders", Class, flags = BclTypeFlags::SEALED, base = "HttpHeaders"; [
        "Uri? Location { get; set; }",
    ]),
    bcl!("System.Net.Http.Headers.HttpContentHeaders", Class, flags = BclTypeFlags::SEALED, base = "HttpHeaders"; [
        "long? ContentLength { get; set; }",
        "MediaTypeHeaderValue? ContentType { get; set; }",
    ]),
    bcl!("System.Net.Http.Headers.AuthenticationHeaderValue", Class; [
        "AuthenticationHeaderValue(string scheme)",
        "AuthenticationHeaderValue(string scheme, string? parameter)",
        "string Scheme { get; }",
        "string? Parameter { get; }",
    ]),
    bcl!("System.Net.Http.Headers.MediaTypeHeaderValue", Class; [
        "MediaTypeHeaderValue(string mediaType)",
        "string? MediaType { get; set; }",
        "string? CharSet { get; set; }",
    ]),
    // ── System.Net.Http.Json ──
    bcl!("System.Net.Http.Json.HttpClientJsonExtensions", Class, flags = BclTypeFlags::STATIC; [
        "static Task<TValue?> GetFromJsonAsync<TValue>(this HttpClient client, string? requestUri, CancellationToken cancellationToken = default)",
        "static Task<TValue?> GetFromJsonAsync<TValue>(this HttpClient client, Uri? requestUri, CancellationToken cancellationToken = default)",
        "static Task<HttpResponseMessage> PostAsJsonAsync<TValue>(this HttpClient client, string? requestUri, TValue value, CancellationToken cancellationToken = default)",
        "static Task<HttpResponseMessage> PutAsJsonAsync<TValue>(this HttpClient client, string? requestUri, TValue value, CancellationToken cancellationToken = default)",
        "static Task<HttpResponseMessage> PatchAsJsonAsync<TValue>(this HttpClient client, string? requestUri, TValue value, CancellationToken cancellationToken = default)",
        "static Task<TValue?> DeleteFromJsonAsync<TValue>(this HttpClient client, string? requestUri, CancellationToken cancellationToken = default)",
    ]),
    bcl!("System.Net.Http.Json.HttpContentJsonExtensions", Class, flags = BclTypeFlags::STATIC; [
        "static Task<T?> ReadFromJsonAsync<T>(this HttpContent content, CancellationToken cancellationToken = default)",
    ]),
    bcl!("System.Net.Http.Json.JsonContent", Class, flags = BclTypeFlags::SEALED, base = "HttpContent"; [
        "static JsonContent Create<T>(T inputValue)",
        "Type ObjectType { get; }",
        "object? Value { get; }",
        "protected override Task SerializeToStreamAsync(Stream stream, TransportContext? context)",
        "protected override bool TryComputeLength(out long length)",
    ]),
];

/// The full BCL table.
pub fn bcl_types() -> &'static [BclType] {
    BCL_TYPES
}

/// Symbol id of the BCL type at `index` in `BCL_TYPES`.
pub fn bcl_symbol_id(index: usize) -> u32 {
    CS_BASE_STUB_SYMBOL_ID + index as u32
}

/// Table index of a C# BCL stub symbol id, or `None` when the id is not one.
pub fn bcl_index(symbol_id: u32) -> Option<usize> {
    if symbol_id < CS_BASE_STUB_SYMBOL_ID {
        return None;
    }
    let index = (symbol_id - CS_BASE_STUB_SYMBOL_ID) as usize;
    if index < BCL_TYPES.len() {
        Some(index)
    } else {
        None
    }
}

/// The BCL type behind a stub symbol id.
pub fn bcl_type(symbol_id: u32) -> Option<&'static BclType> {
    bcl_index(symbol_id).map(|i| &BCL_TYPES[i])
}

/// Metadata name of the type a C# keyword aliases (`int` → `System.Int32`).
pub fn keyword_type(keyword: &str) -> Option<&'static str> {
    BCL_TYPES
        .iter()
        .find(|t| t.keyword == Some(keyword))
        .map(|t| t.qual_name)
}

/// Every namespace declared by the BCL model, including enclosing namespaces
/// (`System`, `System.Collections`, `System.Collections.Generic`, ...), sorted.
pub fn bcl_namespaces() -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    for t in BCL_TYPES {
        let ns = t.namespace();
        let mut end = ns.len();
        loop {
            let prefix = &ns[..end];
            if !prefix.is_empty() && !out.contains(&prefix) {
                out.push(prefix);
            }
            match prefix.rfind('.') {
                Some(i) => end = i,
                None => break,
            }
        }
    }
    out.sort_unstable();
    out
}

/// MSBuild project SDK of an SDK-style project (`<Project Sdk="...">`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DotNetSdk {
    /// `Microsoft.NET.Sdk` (console apps, class libraries).
    Base,
    /// `Microsoft.NET.Sdk.Web` (ASP.NET Core).
    Web,
    /// `Microsoft.NET.Sdk.Worker` (hosted services).
    Worker,
    /// `Microsoft.NET.Sdk.Razor` (Razor class libraries).
    Razor,
    /// `Microsoft.NET.Sdk.BlazorWebAssembly`.
    BlazorWebAssembly,
    /// `Microsoft.NET.Sdk` with `<UseWindowsForms>true</UseWindowsForms>`.
    WindowsForms,
    /// `Microsoft.NET.Sdk` with `<UseWPF>true</UseWPF>`.
    Wpf,
}

impl DotNetSdk {
    /// Maps the `Sdk` attribute value (an optional `/version` suffix is ignored).
    /// Returns `None` for SDKs whose implicit usings are not modelled.
    pub fn from_sdk_name(name: &str) -> Option<Self> {
        let base = name.split('/').next().unwrap_or(name).trim();
        match base.to_ascii_lowercase().as_str() {
            "microsoft.net.sdk" => Some(DotNetSdk::Base),
            "microsoft.net.sdk.web" => Some(DotNetSdk::Web),
            "microsoft.net.sdk.worker" => Some(DotNetSdk::Worker),
            "microsoft.net.sdk.razor" => Some(DotNetSdk::Razor),
            "microsoft.net.sdk.blazorwebassembly" => Some(DotNetSdk::BlazorWebAssembly),
            "microsoft.net.sdk.windowsdesktop" => Some(DotNetSdk::WindowsForms),
            _ => None,
        }
    }
}

const IMPLICIT_USINGS_BASE: &[&str] = &[
    "System",
    "System.Collections.Generic",
    "System.IO",
    "System.Linq",
    "System.Net.Http",
    "System.Threading",
    "System.Threading.Tasks",
];

const IMPLICIT_USINGS_WEB_EXTRA: &[&str] = &[
    "System.Net.Http.Json",
    "Microsoft.AspNetCore.Builder",
    "Microsoft.AspNetCore.Hosting",
    "Microsoft.AspNetCore.Http",
    "Microsoft.AspNetCore.Routing",
    "Microsoft.Extensions.Configuration",
    "Microsoft.Extensions.DependencyInjection",
    "Microsoft.Extensions.Hosting",
    "Microsoft.Extensions.Logging",
];

const IMPLICIT_USINGS_WORKER_EXTRA: &[&str] = &[
    "Microsoft.Extensions.Configuration",
    "Microsoft.Extensions.DependencyInjection",
    "Microsoft.Extensions.Hosting",
    "Microsoft.Extensions.Logging",
];

const IMPLICIT_USINGS_BLAZOR_WASM_EXTRA: &[&str] = &[
    "System.Net.Http.Json",
    "Microsoft.AspNetCore.Components.WebAssembly.Hosting",
    "Microsoft.Extensions.Configuration",
    "Microsoft.Extensions.DependencyInjection",
    "Microsoft.Extensions.Logging",
];

const IMPLICIT_USINGS_WINFORMS_EXTRA: &[&str] = &["System.Drawing", "System.Windows.Forms"];

/// Namespaces imported by `<ImplicitUsings>enable</ImplicitUsings>` for an SDK, as
/// generated into `obj/<project>.GlobalUsings.g.cs` (each one a `global using global::Ns;`).
pub fn implicit_global_usings(sdk: DotNetSdk) -> Vec<&'static str> {
    let extra: &[&str] = match sdk {
        DotNetSdk::Base | DotNetSdk::Razor | DotNetSdk::Wpf => &[],
        DotNetSdk::Web => IMPLICIT_USINGS_WEB_EXTRA,
        DotNetSdk::Worker => IMPLICIT_USINGS_WORKER_EXTRA,
        DotNetSdk::BlazorWebAssembly => IMPLICIT_USINGS_BLAZOR_WASM_EXTRA,
        DotNetSdk::WindowsForms => IMPLICIT_USINGS_WINFORMS_EXTRA,
    };
    let mut out: Vec<&'static str> = IMPLICIT_USINGS_BASE.to_vec();
    for ns in extra {
        if !out.contains(ns) {
            out.push(*ns);
        }
    }
    out
}

/// The BCL in the Java stub shape: one `StdLibStub` per type, keyed by metadata name.
pub fn get_csharp_stubs() -> Vec<StdLibStub> {
    BCL_TYPES
        .iter()
        .enumerate()
        .map(|(i, t)| StdLibStub {
            symbol_id: bcl_symbol_id(i),
            simple_name: t.simple_name(),
            qual_name: t.qual_name,
            kind: t.core_kind(),
        })
        .collect()
}

fn stub_record(symbol_id: u32, qual_name_id: u32, t: &BclType) -> SymbolRecord {
    SymbolRecord {
        symbol_id,
        name_id: u32::MAX,
        qual_name_id,
        type_id: u32::MAX,
        decl_node: u32::MAX,
        def_node: u32::MAX,
        parent_sym: u32::MAX,
        first_child: u32::MAX,
        next_sibling: u32::MAX,
        scope_id: u32::MAX,
        uml_meta_offset: 0,
        param_count: 0,
        modifiers: 0,
        kind: t.core_kind() as u8,
        visibility: SymbolVisibility::Public as u8,
        type_param_count: t.arity().min(u8::MAX as usize) as u8,
        flags: 0,
        first_token_id: u32::MAX,
        last_token_id: u32::MAX,
        _reserved: 0,
    }
}

/// Lookup tables over the BCL, in the shape of the Java `StdLibManager`.
#[derive(Debug, Clone)]
pub struct CsStdLibManager {
    /// Metadata name (`System.Collections.Generic.List`1`), display name
    /// (`System.Collections.Generic.List<T>`) and keyword (`int`) → stub symbol id.
    pub stubs: HashMap<String, u32>,
    pub stub_records: HashMap<u32, SymbolRecord>,
    /// (simple name, arity) → stub symbol ids, across namespaces.
    by_simple: HashMap<(String, usize), Vec<u32>>,
}

impl CsStdLibManager {
    pub fn new(qual_table: &mut QualifiedNameTable) -> Self {
        let mut stubs = HashMap::new();
        let mut stub_records = HashMap::new();
        let mut by_simple: HashMap<(String, usize), Vec<u32>> = HashMap::new();

        for (i, t) in BCL_TYPES.iter().enumerate() {
            let symbol_id = bcl_symbol_id(i);
            let qual_name_id = qual_table.get_or_intern(t.qual_name);
            stub_records.insert(symbol_id, stub_record(symbol_id, qual_name_id, t));

            stubs.insert(t.qual_name.to_string(), symbol_id);
            stubs.insert(Self::display_name(t), symbol_id);
            if let Some(kw) = t.keyword {
                stubs.insert(kw.to_string(), symbol_id);
            }
            by_simple
                .entry((t.simple_name().to_string(), t.arity()))
                .or_default()
                .push(symbol_id);
        }

        Self {
            stubs,
            stub_records,
            by_simple,
        }
    }

    /// `System.Collections.Generic.Dictionary<TKey, TValue>` (variance prefixes removed).
    pub fn display_name(t: &BclType) -> String {
        let ns = t.namespace();
        let mut s = String::new();
        if !ns.is_empty() {
            s.push_str(ns);
            s.push('.');
        }
        s.push_str(t.simple_name());
        if !t.type_params.is_empty() {
            s.push('<');
            for (i, tp) in t.type_params.iter().enumerate() {
                if i > 0 {
                    s.push_str(", ");
                }
                let name = tp.rsplit(' ').next().unwrap_or(tp);
                s.push_str(name);
            }
            s.push('>');
        }
        s
    }

    /// Exact lookup by metadata name, display name or keyword.
    pub fn lookup(&self, name: &str) -> Option<u32> {
        self.stubs.get(name).copied()
    }

    /// Types with this simple name and arity in any modelled namespace.
    pub fn lookup_simple(&self, simple_name: &str, arity: usize) -> &[u32] {
        self.by_simple
            .get(&(simple_name.to_string(), arity))
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    pub fn bcl_type(&self, symbol_id: u32) -> Option<&'static BclType> {
        bcl_type(symbol_id)
    }

    /// Makes the BCL stubs visible through the shared Phase 3 `StdLibManager`, keyed by
    /// metadata name only so that Java simple-name keys (`String`, `Object`) keep their
    /// Java binding.
    pub fn install_into(&self, std_lib: &mut StdLibManager) {
        for (i, t) in BCL_TYPES.iter().enumerate() {
            let id = bcl_symbol_id(i);
            if let Some(rec) = self.stub_records.get(&id) {
                std_lib.stub_records.insert(id, *rec);
            }
            std_lib.stubs.insert(t.qual_name.to_string(), id);
        }
    }
}
