//! Pre-built Kotlin Standard Library Stub Symbols (`kotlin`, `kotlin.collections`,
//! `kotlin.ranges`, `kotlin.text`, `kotlin.sequences`, `kotlin.comparisons`, `kotlin.io`,
//! `kotlin.coroutines`, `kotlin.properties`, `kotlin.reflect`, `kotlin.jvm` and the
//! `kotlinx.coroutines` basics). Loaded at Phase 3 initialization for Kotlin files.
//!
//! Classifier stubs take symbol ids `KOTLIN_BASE_STUB_SYMBOL_ID + index` (index into
//! [`KOTLIN_TYPES`]), callable stubs `KOTLIN_CALLABLE_STUB_BASE_ID + index` (index into
//! [`KOTLIN_CALLABLES`]). The Kotlin range `0xC020_0000..0xC030_0000` follows the scheme
//! `0xC000_0000 | (LangId << 20)` and stays clear of the Java stubs at `0xC000_0000`.
//!
//! Each classifier records its supertypes and, for built-in types the JVM maps to a Java class
//! (`kotlin.String` ↔ `java.lang.String`, `kotlin.collections.MutableList` ↔ `java.util.List`)
//! or that are JVM type aliases (`kotlin.collections.ArrayList` = `java.util.ArrayList`), that
//! Java class. Each callable records whether it is a top-level function, an extension (with its
//! receiver) or a member (with its owner), its argument-count range and, when it does not depend
//! on type arguments, its result classifier.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use crate::core::types::symbol::{SymbolKind, SymbolModifiers, SymbolRecord, SymbolVisibility};
use crate::lang::kotlin::semantics::{
    default_import_tiers, KotlinClassifierKind, KotlinDeclarationIndex, KotlinExtension,
    KotlinMember, KotlinMemberKind, KotlinPlatform,
};
use crate::symbol::qual_name_table::QualifiedNameTable;

pub const KOTLIN_BASE_STUB_SYMBOL_ID: u32 = 0xC020_0000;
pub const KOTLIN_CALLABLE_STUB_BASE_ID: u32 = KOTLIN_BASE_STUB_SYMBOL_ID + 0x0001_0000;
/// First id past the Kotlin stub range.
pub const KOTLIN_STUB_ID_LIMIT: u32 = KOTLIN_BASE_STUB_SYMBOL_ID + 0x0010_0000;

pub const KOTLIN_ANY_STUB_SYM_ID: u32 = KOTLIN_BASE_STUB_SYMBOL_ID;
pub const KOTLIN_NOTHING_STUB_SYM_ID: u32 = KOTLIN_BASE_STUB_SYMBOL_ID + 1;
pub const KOTLIN_UNIT_STUB_SYM_ID: u32 = KOTLIN_BASE_STUB_SYMBOL_ID + 2;
pub const KOTLIN_STRING_STUB_SYM_ID: u32 = KOTLIN_BASE_STUB_SYMBOL_ID + 3;
pub const KOTLIN_CHAR_SEQUENCE_STUB_SYM_ID: u32 = KOTLIN_BASE_STUB_SYMBOL_ID + 4;
pub const KOTLIN_NUMBER_STUB_SYM_ID: u32 = KOTLIN_BASE_STUB_SYMBOL_ID + 5;
pub const KOTLIN_BOOLEAN_STUB_SYM_ID: u32 = KOTLIN_BASE_STUB_SYMBOL_ID + 6;
pub const KOTLIN_CHAR_STUB_SYM_ID: u32 = KOTLIN_BASE_STUB_SYMBOL_ID + 7;
pub const KOTLIN_BYTE_STUB_SYM_ID: u32 = KOTLIN_BASE_STUB_SYMBOL_ID + 8;
pub const KOTLIN_SHORT_STUB_SYM_ID: u32 = KOTLIN_BASE_STUB_SYMBOL_ID + 9;
pub const KOTLIN_INT_STUB_SYM_ID: u32 = KOTLIN_BASE_STUB_SYMBOL_ID + 10;
pub const KOTLIN_LONG_STUB_SYM_ID: u32 = KOTLIN_BASE_STUB_SYMBOL_ID + 11;
pub const KOTLIN_FLOAT_STUB_SYM_ID: u32 = KOTLIN_BASE_STUB_SYMBOL_ID + 12;
pub const KOTLIN_DOUBLE_STUB_SYM_ID: u32 = KOTLIN_BASE_STUB_SYMBOL_ID + 13;
pub const KOTLIN_THROWABLE_STUB_SYM_ID: u32 = KOTLIN_BASE_STUB_SYMBOL_ID + 14;

/// Bits of [`KotlinTypeStub::flags`].
pub struct KotlinTypeFlags;

impl KotlinTypeFlags {
    /// The JVM maps the type to `jvm_peer` (a platform type mapping, not an alias).
    pub const MAPPED: u16 = 1 << 0;
    /// On the JVM the type is a `typealias` of `jvm_peer`.
    pub const TYPEALIAS: u16 = 1 << 1;
    /// A non-null value is a JVM primitive; `jvm_peer` is the boxed class.
    pub const PRIMITIVE: u16 = 1 << 2;
    /// Read-only collection interface (`List`, `Set`, `Map`, ...).
    pub const READ_ONLY_COLLECTION: u16 = 1 << 3;
    /// Mutable collection interface or implementation.
    pub const MUTABLE_COLLECTION: u16 = 1 << 4;
    /// Singleton `object`.
    pub const OBJECT: u16 = 1 << 5;
    /// Companion object (named `<Type>.Companion`).
    pub const COMPANION: u16 = 1 << 6;
    /// Function interface (`kotlin.Function`, `kotlin.jvm.functions.FunctionN`).
    pub const FUNCTIONAL: u16 = 1 << 7;
    /// Array type (`Array<T>`, `IntArray`, ...).
    pub const ARRAY: u16 = 1 << 8;
    /// Unsigned integer type.
    pub const UNSIGNED: u16 = 1 << 9;
    /// Range or progression type.
    pub const RANGE: u16 = 1 << 10;
    /// Throwable type.
    pub const THROWABLE: u16 = 1 << 11;
    /// `value class`.
    pub const VALUE: u16 = 1 << 12;
    /// Exists on the JVM platform only.
    pub const JVM_ONLY: u16 = 1 << 13;
}

/// A built-in Kotlin classifier.
#[derive(Debug, Clone, Copy)]
pub struct KotlinTypeStub {
    pub package: &'static str,
    /// Simple name, or `Outer.Inner` for a nested classifier.
    pub name: &'static str,
    pub kind: SymbolKind,
    pub type_params: u8,
    /// `KotlinTypeFlags` bits.
    pub flags: u16,
    /// JVM class the type maps to (`MAPPED`) or aliases (`TYPEALIAS`).
    pub jvm_peer: Option<&'static str>,
    /// Direct supertypes (qualified names).
    pub supertypes: &'static [&'static str],
}

impl KotlinTypeStub {
    pub fn qual_name(&self) -> String {
        format!("{}.{}", self.package, self.name)
    }

    /// Simple name (last segment of a nested name).
    pub fn simple_name(&self) -> &'static str {
        self.name.rsplit('.').next().unwrap_or(self.name)
    }

    pub fn classifier_kind(&self) -> KotlinClassifierKind {
        if self.flags & KotlinTypeFlags::COMPANION != 0 {
            return KotlinClassifierKind::CompanionObject;
        }
        if self.flags & KotlinTypeFlags::OBJECT != 0 {
            return KotlinClassifierKind::Object;
        }
        match self.kind {
            SymbolKind::SK_INTERFACE => KotlinClassifierKind::Interface,
            SymbolKind::SK_ENUM => KotlinClassifierKind::Enum,
            SymbolKind::SK_ANNOTATION_TYPE => KotlinClassifierKind::Annotation,
            _ if self.flags & KotlinTypeFlags::TYPEALIAS != 0 => KotlinClassifierKind::TypeAlias,
            _ => KotlinClassifierKind::Class,
        }
    }
}

/// Whether a callable is a function or a property.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KotlinCallableKind {
    Function,
    Property,
}

/// Bits of [`KotlinCallableStub::flags`].
pub struct KotlinCallableFlags;

impl KotlinCallableFlags {
    pub const SUSPEND: u16 = 1 << 0;
    pub const INLINE: u16 = 1 << 1;
    pub const INFIX: u16 = 1 << 2;
    pub const OPERATOR: u16 = 1 << 3;
    /// The extension receiver is nullable (`T?.f()` or an unbounded type parameter).
    pub const NULLABLE_RECEIVER: u16 = 1 << 4;
    /// Declares a `reified` type parameter.
    pub const REIFIED: u16 = 1 << 5;
    /// The last value parameter is `vararg`.
    pub const VARARG: u16 = 1 << 6;
    /// Exists on the JVM platform only.
    pub const JVM_ONLY: u16 = 1 << 7;
}

/// `max_args` of a callable whose `vararg` parameter accepts any number of arguments.
pub const VARIADIC: u8 = u8::MAX;

/// A built-in Kotlin function or property.
#[derive(Debug, Clone, Copy)]
pub struct KotlinCallableStub {
    /// Declaring package (empty for members, which belong to `owner`).
    pub package: &'static str,
    pub name: &'static str,
    pub kind: KotlinCallableKind,
    /// Declaring classifier (qualified) of a member.
    pub owner: Option<&'static str>,
    /// Receiver type (qualified) of an extension.
    pub receiver: Option<&'static str>,
    /// `KotlinCallableFlags` bits.
    pub flags: u16,
    /// Fewest value arguments a call passes (parameters without defaults).
    pub min_args: u8,
    /// Most value arguments a call passes (`VARIADIC` for `vararg`).
    pub max_args: u8,
    /// Result classifier, or `None` when it is a type argument of the call.
    pub returns: Option<&'static str>,
}

impl KotlinCallableStub {
    /// `owner.name` for members, `package.name` otherwise.
    pub fn qual_name(&self) -> String {
        match self.owner {
            Some(owner) => format!("{}.{}", owner, self.name),
            None => format!("{}.{}", self.package, self.name),
        }
    }

    pub fn is_extension(&self) -> bool {
        self.receiver.is_some()
    }

    pub fn is_top_level(&self) -> bool {
        self.owner.is_none() && self.receiver.is_none()
    }

    /// True when a call with `args` value arguments fits the parameter count range.
    pub fn accepts_arity(&self, args: usize) -> bool {
        args >= self.min_args as usize && (self.max_args == VARIADIC || args <= self.max_args as usize)
    }

    fn member_kind(&self) -> KotlinMemberKind {
        match self.kind {
            KotlinCallableKind::Function => KotlinMemberKind::Function,
            KotlinCallableKind::Property => KotlinMemberKind::Property,
        }
    }
}

// ── Table vocabulary ──

const C: SymbolKind = SymbolKind::SK_CLASS;
const I: SymbolKind = SymbolKind::SK_INTERFACE;
const E: SymbolKind = SymbolKind::SK_ENUM;
const A: SymbolKind = SymbolKind::SK_ANNOTATION_TYPE;

const MAP: u16 = KotlinTypeFlags::MAPPED;
const ALIAS: u16 = KotlinTypeFlags::TYPEALIAS;
const PRIM: u16 = KotlinTypeFlags::PRIMITIVE;
const RO: u16 = KotlinTypeFlags::READ_ONLY_COLLECTION;
const MUT: u16 = KotlinTypeFlags::MUTABLE_COLLECTION;
const OBJ: u16 = KotlinTypeFlags::OBJECT;
const COMP: u16 = KotlinTypeFlags::COMPANION | KotlinTypeFlags::OBJECT;
const FUN: u16 = KotlinTypeFlags::FUNCTIONAL;
const ARR: u16 = KotlinTypeFlags::ARRAY;
const UNS: u16 = KotlinTypeFlags::UNSIGNED | KotlinTypeFlags::VALUE;
const RNG: u16 = KotlinTypeFlags::RANGE;
const THR: u16 = KotlinTypeFlags::THROWABLE;
const VAL: u16 = KotlinTypeFlags::VALUE;
const JVM: u16 = KotlinTypeFlags::JVM_ONLY;

const K: &str = "kotlin";
const KA: &str = "kotlin.annotation";
const KJ: &str = "kotlin.jvm";
const KJF: &str = "kotlin.jvm.functions";
const KC: &str = "kotlin.collections";
const KR: &str = "kotlin.ranges";
const KT: &str = "kotlin.text";
const KS: &str = "kotlin.sequences";
const KCMP: &str = "kotlin.comparisons";
const KIO: &str = "kotlin.io";
const KCO: &str = "kotlin.coroutines";
const KCC: &str = "kotlin.concurrent";
const KP: &str = "kotlin.properties";
const KRF: &str = "kotlin.reflect";
const KTM: &str = "kotlin.time";
const KX: &str = "kotlinx.coroutines";
const KXF: &str = "kotlinx.coroutines.flow";
const KXC: &str = "kotlinx.coroutines.channels";
const KXS: &str = "kotlinx.coroutines.sync";

// Qualified names used as supertypes, receivers and results.
const ANY: &str = "kotlin.Any";
const NOTHING: &str = "kotlin.Nothing";
const UNIT: &str = "kotlin.Unit";
const STRING: &str = "kotlin.String";
const CHAR_SEQUENCE: &str = "kotlin.CharSequence";
const NUMBER: &str = "kotlin.Number";
const BOOLEAN: &str = "kotlin.Boolean";
const CHAR: &str = "kotlin.Char";
const INT: &str = "kotlin.Int";
const LONG: &str = "kotlin.Long";
const FLOAT: &str = "kotlin.Float";
const DOUBLE: &str = "kotlin.Double";
const COMPARABLE: &str = "kotlin.Comparable";
const COMPARATOR: &str = "kotlin.Comparator";
const THROWABLE: &str = "kotlin.Throwable";
const ERROR: &str = "kotlin.Error";
const EXCEPTION: &str = "kotlin.Exception";
const RUNTIME_EXCEPTION: &str = "kotlin.RuntimeException";
const ILLEGAL_ARGUMENT: &str = "kotlin.IllegalArgumentException";
const ILLEGAL_STATE: &str = "kotlin.IllegalStateException";
const NO_SUCH_ELEMENT: &str = "kotlin.NoSuchElementException";
const PAIR: &str = "kotlin.Pair";
const LAZY: &str = "kotlin.Lazy";
const RESULT: &str = "kotlin.Result";
const FUNCTION: &str = "kotlin.Function";
const ARRAY: &str = "kotlin.Array";
const BOOLEAN_ARRAY: &str = "kotlin.BooleanArray";
const BYTE_ARRAY: &str = "kotlin.ByteArray";
const CHAR_ARRAY: &str = "kotlin.CharArray";
const SHORT_ARRAY: &str = "kotlin.ShortArray";
const INT_ARRAY: &str = "kotlin.IntArray";
const LONG_ARRAY: &str = "kotlin.LongArray";
const FLOAT_ARRAY: &str = "kotlin.FloatArray";
const DOUBLE_ARRAY: &str = "kotlin.DoubleArray";
const ITERABLE: &str = "kotlin.collections.Iterable";
const MUTABLE_ITERABLE: &str = "kotlin.collections.MutableIterable";
const COLLECTION: &str = "kotlin.collections.Collection";
const MUTABLE_COLLECTION: &str = "kotlin.collections.MutableCollection";
const LIST: &str = "kotlin.collections.List";
const MUTABLE_LIST: &str = "kotlin.collections.MutableList";
const SET: &str = "kotlin.collections.Set";
const MUTABLE_SET: &str = "kotlin.collections.MutableSet";
const MAP_T: &str = "kotlin.collections.Map";
const MUTABLE_MAP: &str = "kotlin.collections.MutableMap";
const MAP_ENTRY: &str = "kotlin.collections.Map.Entry";
const ITERATOR: &str = "kotlin.collections.Iterator";
const MUTABLE_ITERATOR: &str = "kotlin.collections.MutableIterator";
const LIST_ITERATOR: &str = "kotlin.collections.ListIterator";
const MUTABLE_LIST_ITERATOR: &str = "kotlin.collections.MutableListIterator";
const ARRAY_LIST: &str = "kotlin.collections.ArrayList";
const HASH_MAP: &str = "kotlin.collections.HashMap";
const HASH_SET: &str = "kotlin.collections.HashSet";
const LINKED_HASH_MAP: &str = "kotlin.collections.LinkedHashMap";
const LINKED_HASH_SET: &str = "kotlin.collections.LinkedHashSet";
const RANDOM_ACCESS: &str = "kotlin.collections.RandomAccess";
const ABSTRACT_COLLECTION: &str = "kotlin.collections.AbstractCollection";
const ABSTRACT_MUTABLE_LIST: &str = "kotlin.collections.AbstractMutableList";
const GROUPING: &str = "kotlin.collections.Grouping";
const INDEXED_VALUE: &str = "kotlin.collections.IndexedValue";
const CLOSED_RANGE: &str = "kotlin.ranges.ClosedRange";
const OPEN_END_RANGE: &str = "kotlin.ranges.OpenEndRange";
const INT_PROGRESSION: &str = "kotlin.ranges.IntProgression";
const LONG_PROGRESSION: &str = "kotlin.ranges.LongProgression";
const CHAR_PROGRESSION: &str = "kotlin.ranges.CharProgression";
const INT_RANGE: &str = "kotlin.ranges.IntRange";
const LONG_RANGE: &str = "kotlin.ranges.LongRange";
const CHAR_RANGE: &str = "kotlin.ranges.CharRange";
const REGEX: &str = "kotlin.text.Regex";
const MATCH_RESULT: &str = "kotlin.text.MatchResult";
const MATCH_GROUP_COLLECTION: &str = "kotlin.text.MatchGroupCollection";
const STRING_BUILDER: &str = "kotlin.text.StringBuilder";
const APPENDABLE: &str = "kotlin.text.Appendable";
const SEQUENCE: &str = "kotlin.sequences.Sequence";
const COROUTINE_CONTEXT: &str = "kotlin.coroutines.CoroutineContext";
const CONTEXT_ELEMENT: &str = "kotlin.coroutines.CoroutineContext.Element";
const CONTINUATION: &str = "kotlin.coroutines.Continuation";
const CONTINUATION_INTERCEPTOR: &str = "kotlin.coroutines.ContinuationInterceptor";
const ABSTRACT_CONTEXT_ELEMENT: &str = "kotlin.coroutines.AbstractCoroutineContextElement";
const READ_ONLY_PROPERTY: &str = "kotlin.properties.ReadOnlyProperty";
const READ_WRITE_PROPERTY: &str = "kotlin.properties.ReadWriteProperty";
const KCALLABLE: &str = "kotlin.reflect.KCallable";
const KPROPERTY: &str = "kotlin.reflect.KProperty";
const KPROPERTY0: &str = "kotlin.reflect.KProperty0";
const KPROPERTY1: &str = "kotlin.reflect.KProperty1";
const KMUTABLE_PROPERTY: &str = "kotlin.reflect.KMutableProperty";
const SCOPE: &str = "kotlinx.coroutines.CoroutineScope";
const JOB: &str = "kotlinx.coroutines.Job";
const DEFERRED: &str = "kotlinx.coroutines.Deferred";
const COMPLETABLE_JOB: &str = "kotlinx.coroutines.CompletableJob";
const COMPLETABLE_DEFERRED: &str = "kotlinx.coroutines.CompletableDeferred";
const DISPATCHER: &str = "kotlinx.coroutines.CoroutineDispatcher";
const MAIN_DISPATCHER: &str = "kotlinx.coroutines.MainCoroutineDispatcher";
const DISPOSABLE_HANDLE: &str = "kotlinx.coroutines.DisposableHandle";
const EXCEPTION_HANDLER: &str = "kotlinx.coroutines.CoroutineExceptionHandler";
const CANCELLATION: &str = "kotlinx.coroutines.CancellationException";
const FLOW: &str = "kotlinx.coroutines.flow.Flow";
const FLOW_COLLECTOR: &str = "kotlinx.coroutines.flow.FlowCollector";
const SHARED_FLOW: &str = "kotlinx.coroutines.flow.SharedFlow";
const STATE_FLOW: &str = "kotlinx.coroutines.flow.StateFlow";
const MUTABLE_SHARED_FLOW: &str = "kotlinx.coroutines.flow.MutableSharedFlow";
const MUTABLE_STATE_FLOW: &str = "kotlinx.coroutines.flow.MutableStateFlow";
const SEND_CHANNEL: &str = "kotlinx.coroutines.channels.SendChannel";
const RECEIVE_CHANNEL: &str = "kotlinx.coroutines.channels.ReceiveChannel";
const CHANNEL: &str = "kotlinx.coroutines.channels.Channel";
const CHANNEL_RESULT: &str = "kotlinx.coroutines.channels.ChannelResult";
const MUTEX: &str = "kotlinx.coroutines.sync.Mutex";
const SEMAPHORE: &str = "kotlinx.coroutines.sync.Semaphore";

#[allow(clippy::too_many_arguments)]
const fn ty(
    package: &'static str,
    name: &'static str,
    kind: SymbolKind,
    type_params: u8,
    flags: u16,
    jvm_peer: Option<&'static str>,
    supertypes: &'static [&'static str],
) -> KotlinTypeStub {
    KotlinTypeStub {
        package,
        name,
        kind,
        type_params,
        flags,
        jvm_peer,
        supertypes,
    }
}

/// The built-in classifiers. The first entries are fixed: their ids are the
/// `KOTLIN_*_STUB_SYM_ID` constants (checked at compile time below).
pub const KOTLIN_TYPES: &[KotlinTypeStub] = &[
    // ── kotlin: fixed-id entries ──
    ty(K, "Any", C, 0, MAP, Some("java.lang.Object"), &[]),
    ty(K, "Nothing", C, 0, 0, None, &[]),
    ty(K, "Unit", C, 0, OBJ, None, &[ANY]),
    ty(K, "String", C, 0, MAP, Some("java.lang.String"), &[COMPARABLE, CHAR_SEQUENCE]),
    ty(K, "CharSequence", I, 0, MAP, Some("java.lang.CharSequence"), &[]),
    ty(K, "Number", C, 0, MAP, Some("java.lang.Number"), &[]),
    ty(K, "Boolean", C, 0, MAP | PRIM, Some("java.lang.Boolean"), &[COMPARABLE]),
    ty(K, "Char", C, 0, MAP | PRIM, Some("java.lang.Character"), &[COMPARABLE]),
    ty(K, "Byte", C, 0, MAP | PRIM, Some("java.lang.Byte"), &[NUMBER, COMPARABLE]),
    ty(K, "Short", C, 0, MAP | PRIM, Some("java.lang.Short"), &[NUMBER, COMPARABLE]),
    ty(K, "Int", C, 0, MAP | PRIM, Some("java.lang.Integer"), &[NUMBER, COMPARABLE]),
    ty(K, "Long", C, 0, MAP | PRIM, Some("java.lang.Long"), &[NUMBER, COMPARABLE]),
    ty(K, "Float", C, 0, MAP | PRIM, Some("java.lang.Float"), &[NUMBER, COMPARABLE]),
    ty(K, "Double", C, 0, MAP | PRIM, Some("java.lang.Double"), &[NUMBER, COMPARABLE]),
    ty(K, "Throwable", C, 0, MAP | THR, Some("java.lang.Throwable"), &[]),
    // ── kotlin ──
    ty(K, "UByte", C, 0, UNS, None, &[COMPARABLE]),
    ty(K, "UShort", C, 0, UNS, None, &[COMPARABLE]),
    ty(K, "UInt", C, 0, UNS, None, &[COMPARABLE]),
    ty(K, "ULong", C, 0, UNS, None, &[COMPARABLE]),
    ty(K, "Array", C, 1, ARR, None, &[]),
    ty(K, "BooleanArray", C, 0, ARR, None, &[]),
    ty(K, "ByteArray", C, 0, ARR, None, &[]),
    ty(K, "CharArray", C, 0, ARR, None, &[]),
    ty(K, "ShortArray", C, 0, ARR, None, &[]),
    ty(K, "IntArray", C, 0, ARR, None, &[]),
    ty(K, "LongArray", C, 0, ARR, None, &[]),
    ty(K, "FloatArray", C, 0, ARR, None, &[]),
    ty(K, "DoubleArray", C, 0, ARR, None, &[]),
    ty(K, "Comparable", I, 1, MAP, Some("java.lang.Comparable"), &[]),
    ty(K, "Comparator", I, 1, ALIAS, Some("java.util.Comparator"), &[]),
    ty(K, "Enum", C, 1, MAP, Some("java.lang.Enum"), &[COMPARABLE]),
    ty(K, "Annotation", I, 0, MAP, Some("java.lang.annotation.Annotation"), &[]),
    ty(K, "Cloneable", I, 0, MAP | JVM, Some("java.lang.Cloneable"), &[]),
    ty(K, "Exception", C, 0, ALIAS | THR, Some("java.lang.Exception"), &[THROWABLE]),
    ty(K, "Error", C, 0, ALIAS | THR, Some("java.lang.Error"), &[THROWABLE]),
    ty(K, "RuntimeException", C, 0, ALIAS | THR, Some("java.lang.RuntimeException"), &[EXCEPTION]),
    ty(K, "IllegalArgumentException", C, 0, ALIAS | THR, Some("java.lang.IllegalArgumentException"), &[RUNTIME_EXCEPTION]),
    ty(K, "IllegalStateException", C, 0, ALIAS | THR, Some("java.lang.IllegalStateException"), &[RUNTIME_EXCEPTION]),
    ty(K, "UnsupportedOperationException", C, 0, ALIAS | THR, Some("java.lang.UnsupportedOperationException"), &[RUNTIME_EXCEPTION]),
    ty(K, "IndexOutOfBoundsException", C, 0, ALIAS | THR, Some("java.lang.IndexOutOfBoundsException"), &[RUNTIME_EXCEPTION]),
    ty(K, "ArithmeticException", C, 0, ALIAS | THR, Some("java.lang.ArithmeticException"), &[RUNTIME_EXCEPTION]),
    ty(K, "ClassCastException", C, 0, ALIAS | THR, Some("java.lang.ClassCastException"), &[RUNTIME_EXCEPTION]),
    ty(K, "NullPointerException", C, 0, ALIAS | THR, Some("java.lang.NullPointerException"), &[RUNTIME_EXCEPTION]),
    ty(K, "NumberFormatException", C, 0, ALIAS | THR, Some("java.lang.NumberFormatException"), &[ILLEGAL_ARGUMENT]),
    ty(K, "NoSuchElementException", C, 0, ALIAS | THR, Some("java.util.NoSuchElementException"), &[RUNTIME_EXCEPTION]),
    ty(K, "ConcurrentModificationException", C, 0, ALIAS | THR, Some("java.util.ConcurrentModificationException"), &[RUNTIME_EXCEPTION]),
    ty(K, "AssertionError", C, 0, ALIAS | THR, Some("java.lang.AssertionError"), &[ERROR]),
    ty(K, "NotImplementedError", C, 0, THR, None, &[ERROR]),
    ty(K, "UninitializedPropertyAccessException", C, 0, THR, None, &[RUNTIME_EXCEPTION]),
    ty(K, "NoWhenBranchMatchedException", C, 0, THR, None, &[RUNTIME_EXCEPTION]),
    ty(K, "Pair", C, 2, 0, None, &[]),
    ty(K, "Triple", C, 3, 0, None, &[]),
    ty(K, "Lazy", I, 1, 0, None, &[]),
    ty(K, "LazyThreadSafetyMode", E, 0, 0, None, &[]),
    ty(K, "Result", C, 1, VAL, None, &[]),
    ty(K, "Result.Companion", C, 0, COMP, None, &[]),
    ty(K, "Function", I, 1, FUN, None, &[]),
    ty(K, "KotlinVersion", C, 0, 0, None, &[COMPARABLE]),
    ty(K, "Deprecated", A, 0, 0, None, &[]),
    ty(K, "DeprecationLevel", E, 0, 0, None, &[]),
    ty(K, "ReplaceWith", A, 0, 0, None, &[]),
    ty(K, "Suppress", A, 0, 0, None, &[]),
    ty(K, "PublishedApi", A, 0, 0, None, &[]),
    ty(K, "DslMarker", A, 0, 0, None, &[]),
    ty(K, "OptIn", A, 0, 0, None, &[]),
    ty(K, "RequiresOptIn", A, 0, 0, None, &[]),
    ty(K, "UnsafeVariance", A, 0, 0, None, &[]),
    ty(K, "Throws", A, 0, ALIAS | JVM, Some("kotlin.jvm.Throws"), &[]),
    ty(K, "Byte.Companion", C, 0, COMP, None, &[]),
    ty(K, "Short.Companion", C, 0, COMP, None, &[]),
    ty(K, "Int.Companion", C, 0, COMP, None, &[]),
    ty(K, "Long.Companion", C, 0, COMP, None, &[]),
    ty(K, "Float.Companion", C, 0, COMP, None, &[]),
    ty(K, "Double.Companion", C, 0, COMP, None, &[]),
    ty(K, "Char.Companion", C, 0, COMP, None, &[]),
    ty(K, "String.Companion", C, 0, COMP, None, &[]),
    // ── kotlin.annotation ──
    ty(KA, "Target", A, 0, 0, None, &[]),
    ty(KA, "Retention", A, 0, 0, None, &[]),
    ty(KA, "Repeatable", A, 0, 0, None, &[]),
    ty(KA, "MustBeDocumented", A, 0, 0, None, &[]),
    ty(KA, "AnnotationTarget", E, 0, 0, None, &[]),
    ty(KA, "AnnotationRetention", E, 0, 0, None, &[]),
    // ── kotlin.jvm ──
    ty(KJ, "JvmStatic", A, 0, JVM, None, &[]),
    ty(KJ, "JvmField", A, 0, JVM, None, &[]),
    ty(KJ, "JvmOverloads", A, 0, JVM, None, &[]),
    ty(KJ, "JvmName", A, 0, JVM, None, &[]),
    ty(KJ, "JvmInline", A, 0, JVM, None, &[]),
    ty(KJ, "JvmRecord", A, 0, JVM, None, &[]),
    ty(KJ, "JvmSuppressWildcards", A, 0, JVM, None, &[]),
    ty(KJ, "JvmWildcard", A, 0, JVM, None, &[]),
    ty(KJ, "JvmSynthetic", A, 0, JVM, None, &[]),
    ty(KJ, "JvmMultifileClass", A, 0, JVM, None, &[]),
    ty(KJ, "Throws", A, 0, JVM, None, &[]),
    ty(KJ, "Volatile", A, 0, JVM, None, &[]),
    ty(KJ, "Transient", A, 0, JVM, None, &[]),
    ty(KJ, "Synchronized", A, 0, JVM, None, &[]),
    ty(KJ, "Strictfp", A, 0, JVM, None, &[]),
    ty(KCC, "Volatile", A, 0, 0, None, &[]),
    // ── kotlin.jvm.functions: Function0 .. Function22 ──
    ty(KJF, "Function0", I, 1, FUN | JVM, None, &[FUNCTION]),
    ty(KJF, "Function1", I, 2, FUN | JVM, None, &[FUNCTION]),
    ty(KJF, "Function2", I, 3, FUN | JVM, None, &[FUNCTION]),
    ty(KJF, "Function3", I, 4, FUN | JVM, None, &[FUNCTION]),
    ty(KJF, "Function4", I, 5, FUN | JVM, None, &[FUNCTION]),
    ty(KJF, "Function5", I, 6, FUN | JVM, None, &[FUNCTION]),
    ty(KJF, "Function6", I, 7, FUN | JVM, None, &[FUNCTION]),
    ty(KJF, "Function7", I, 8, FUN | JVM, None, &[FUNCTION]),
    ty(KJF, "Function8", I, 9, FUN | JVM, None, &[FUNCTION]),
    ty(KJF, "Function9", I, 10, FUN | JVM, None, &[FUNCTION]),
    ty(KJF, "Function10", I, 11, FUN | JVM, None, &[FUNCTION]),
    ty(KJF, "Function11", I, 12, FUN | JVM, None, &[FUNCTION]),
    ty(KJF, "Function12", I, 13, FUN | JVM, None, &[FUNCTION]),
    ty(KJF, "Function13", I, 14, FUN | JVM, None, &[FUNCTION]),
    ty(KJF, "Function14", I, 15, FUN | JVM, None, &[FUNCTION]),
    ty(KJF, "Function15", I, 16, FUN | JVM, None, &[FUNCTION]),
    ty(KJF, "Function16", I, 17, FUN | JVM, None, &[FUNCTION]),
    ty(KJF, "Function17", I, 18, FUN | JVM, None, &[FUNCTION]),
    ty(KJF, "Function18", I, 19, FUN | JVM, None, &[FUNCTION]),
    ty(KJF, "Function19", I, 20, FUN | JVM, None, &[FUNCTION]),
    ty(KJF, "Function20", I, 21, FUN | JVM, None, &[FUNCTION]),
    ty(KJF, "Function21", I, 22, FUN | JVM, None, &[FUNCTION]),
    ty(KJF, "Function22", I, 23, FUN | JVM, None, &[FUNCTION]),
    // ── kotlin.collections ──
    ty(KC, "Iterable", I, 1, MAP | RO, Some("java.lang.Iterable"), &[]),
    ty(KC, "MutableIterable", I, 1, MAP | MUT, Some("java.lang.Iterable"), &[ITERABLE]),
    ty(KC, "Collection", I, 1, MAP | RO, Some("java.util.Collection"), &[ITERABLE]),
    ty(KC, "MutableCollection", I, 1, MAP | MUT, Some("java.util.Collection"), &[COLLECTION, MUTABLE_ITERABLE]),
    ty(KC, "List", I, 1, MAP | RO, Some("java.util.List"), &[COLLECTION]),
    ty(KC, "MutableList", I, 1, MAP | MUT, Some("java.util.List"), &[LIST, MUTABLE_COLLECTION]),
    ty(KC, "Set", I, 1, MAP | RO, Some("java.util.Set"), &[COLLECTION]),
    ty(KC, "MutableSet", I, 1, MAP | MUT, Some("java.util.Set"), &[SET, MUTABLE_COLLECTION]),
    ty(KC, "Map", I, 2, MAP | RO, Some("java.util.Map"), &[]),
    ty(KC, "MutableMap", I, 2, MAP | MUT, Some("java.util.Map"), &[MAP_T]),
    ty(KC, "Map.Entry", I, 2, MAP | RO, Some("java.util.Map.Entry"), &[]),
    ty(KC, "MutableMap.MutableEntry", I, 2, MAP | MUT, Some("java.util.Map.Entry"), &[MAP_ENTRY]),
    ty(KC, "Iterator", I, 1, MAP | RO, Some("java.util.Iterator"), &[]),
    ty(KC, "MutableIterator", I, 1, MAP | MUT, Some("java.util.Iterator"), &[ITERATOR]),
    ty(KC, "ListIterator", I, 1, MAP | RO, Some("java.util.ListIterator"), &[ITERATOR]),
    ty(KC, "MutableListIterator", I, 1, MAP | MUT, Some("java.util.ListIterator"), &[LIST_ITERATOR, MUTABLE_ITERATOR]),
    ty(KC, "ArrayList", C, 1, ALIAS | MUT, Some("java.util.ArrayList"), &[MUTABLE_LIST, RANDOM_ACCESS]),
    ty(KC, "HashMap", C, 2, ALIAS | MUT, Some("java.util.HashMap"), &[MUTABLE_MAP]),
    ty(KC, "HashSet", C, 1, ALIAS | MUT, Some("java.util.HashSet"), &[MUTABLE_SET]),
    ty(KC, "LinkedHashMap", C, 2, ALIAS | MUT, Some("java.util.LinkedHashMap"), &[HASH_MAP]),
    ty(KC, "LinkedHashSet", C, 1, ALIAS | MUT, Some("java.util.LinkedHashSet"), &[HASH_SET]),
    ty(KC, "RandomAccess", I, 0, ALIAS, Some("java.util.RandomAccess"), &[]),
    ty(KC, "AbstractCollection", C, 1, RO, None, &[COLLECTION]),
    ty(KC, "AbstractList", C, 1, RO, None, &[ABSTRACT_COLLECTION, LIST]),
    ty(KC, "AbstractSet", C, 1, RO, None, &[ABSTRACT_COLLECTION, SET]),
    ty(KC, "AbstractMap", C, 2, RO, None, &[MAP_T]),
    ty(KC, "AbstractMutableCollection", C, 1, MUT, None, &[MUTABLE_COLLECTION]),
    ty(KC, "AbstractMutableList", C, 1, MUT, None, &[MUTABLE_LIST]),
    ty(KC, "AbstractMutableSet", C, 1, MUT, None, &[MUTABLE_SET]),
    ty(KC, "AbstractMutableMap", C, 2, MUT, None, &[MUTABLE_MAP]),
    ty(KC, "AbstractIterator", C, 1, 0, None, &[ITERATOR]),
    ty(KC, "ArrayDeque", C, 1, MUT, None, &[ABSTRACT_MUTABLE_LIST]),
    ty(KC, "Grouping", I, 2, 0, None, &[]),
    ty(KC, "IndexedValue", C, 1, 0, None, &[]),
    ty(KC, "IntIterator", C, 0, 0, None, &[ITERATOR]),
    ty(KC, "LongIterator", C, 0, 0, None, &[ITERATOR]),
    ty(KC, "CharIterator", C, 0, 0, None, &[ITERATOR]),
    // ── kotlin.ranges ──
    ty(KR, "ClosedRange", I, 1, RNG, None, &[]),
    ty(KR, "OpenEndRange", I, 1, RNG, None, &[]),
    ty(KR, "ClosedFloatingPointRange", I, 1, RNG, None, &[CLOSED_RANGE]),
    ty(KR, "IntProgression", C, 0, RNG, None, &[ITERABLE]),
    ty(KR, "LongProgression", C, 0, RNG, None, &[ITERABLE]),
    ty(KR, "CharProgression", C, 0, RNG, None, &[ITERABLE]),
    ty(KR, "UIntProgression", C, 0, RNG, None, &[ITERABLE]),
    ty(KR, "ULongProgression", C, 0, RNG, None, &[ITERABLE]),
    ty(KR, "IntRange", C, 0, RNG, None, &[INT_PROGRESSION, CLOSED_RANGE, OPEN_END_RANGE]),
    ty(KR, "LongRange", C, 0, RNG, None, &[LONG_PROGRESSION, CLOSED_RANGE, OPEN_END_RANGE]),
    ty(KR, "CharRange", C, 0, RNG, None, &[CHAR_PROGRESSION, CLOSED_RANGE, OPEN_END_RANGE]),
    ty(KR, "UIntRange", C, 0, RNG, None, &["kotlin.ranges.UIntProgression", CLOSED_RANGE, OPEN_END_RANGE]),
    ty(KR, "ULongRange", C, 0, RNG, None, &["kotlin.ranges.ULongProgression", CLOSED_RANGE, OPEN_END_RANGE]),
    // ── kotlin.text ──
    ty(KT, "Regex", C, 0, 0, None, &[]),
    ty(KT, "RegexOption", E, 0, 0, None, &[]),
    ty(KT, "MatchResult", I, 0, 0, None, &[]),
    ty(KT, "MatchResult.Destructured", C, 0, 0, None, &[]),
    ty(KT, "MatchGroup", C, 0, 0, None, &[]),
    ty(KT, "MatchGroupCollection", I, 0, 0, None, &[COLLECTION]),
    ty(KT, "MatchNamedGroupCollection", I, 0, 0, None, &[MATCH_GROUP_COLLECTION]),
    ty(KT, "StringBuilder", C, 0, ALIAS, Some("java.lang.StringBuilder"), &[APPENDABLE, CHAR_SEQUENCE]),
    ty(KT, "Appendable", I, 0, ALIAS, Some("java.lang.Appendable"), &[]),
    ty(KT, "CharCategory", E, 0, 0, None, &[]),
    ty(KT, "CharDirectionality", E, 0, 0, None, &[]),
    ty(KT, "Charsets", C, 0, OBJ | JVM, None, &[]),
    ty(KT, "Typography", C, 0, OBJ, None, &[]),
    // ── kotlin.sequences ──
    ty(KS, "Sequence", I, 1, 0, None, &[]),
    ty(KS, "SequenceScope", C, 1, 0, None, &[]),
    // ── kotlin.coroutines ──
    ty(KCO, "Continuation", I, 1, 0, None, &[]),
    ty(KCO, "CoroutineContext", I, 0, 0, None, &[]),
    ty(KCO, "CoroutineContext.Element", I, 0, 0, None, &[COROUTINE_CONTEXT]),
    ty(KCO, "CoroutineContext.Key", I, 1, 0, None, &[]),
    ty(KCO, "ContinuationInterceptor", I, 0, 0, None, &[CONTEXT_ELEMENT]),
    ty(KCO, "AbstractCoroutineContextElement", C, 0, 0, None, &[CONTEXT_ELEMENT]),
    ty(KCO, "EmptyCoroutineContext", C, 0, OBJ, None, &[COROUTINE_CONTEXT]),
    ty(KCO, "RestrictsSuspension", A, 0, 0, None, &[]),
    // ── kotlin.properties ──
    ty(KP, "ReadOnlyProperty", I, 2, 0, None, &[]),
    ty(KP, "ReadWriteProperty", I, 2, 0, None, &[READ_ONLY_PROPERTY]),
    ty(KP, "PropertyDelegateProvider", I, 2, 0, None, &[]),
    ty(KP, "Delegates", C, 0, OBJ, None, &[]),
    ty(KP, "ObservableProperty", C, 1, 0, None, &[READ_WRITE_PROPERTY]),
    // ── kotlin.reflect ──
    ty(KRF, "KClass", I, 1, 0, None, &[]),
    ty(KRF, "KCallable", I, 1, 0, None, &[]),
    ty(KRF, "KFunction", I, 1, 0, None, &[KCALLABLE, FUNCTION]),
    ty(KRF, "KProperty", I, 1, 0, None, &[KCALLABLE]),
    ty(KRF, "KProperty0", I, 1, 0, None, &[KPROPERTY]),
    ty(KRF, "KProperty1", I, 2, 0, None, &[KPROPERTY]),
    ty(KRF, "KMutableProperty", I, 1, 0, None, &[KPROPERTY]),
    ty(KRF, "KMutableProperty0", I, 1, 0, None, &[KPROPERTY0, KMUTABLE_PROPERTY]),
    ty(KRF, "KMutableProperty1", I, 2, 0, None, &[KPROPERTY1, KMUTABLE_PROPERTY]),
    ty(KRF, "KType", I, 0, 0, None, &[]),
    // ── kotlin.time ──
    ty(KTM, "Duration", C, 0, VAL, None, &[COMPARABLE]),
    ty(KTM, "DurationUnit", E, 0, 0, None, &[]),
    // ── kotlinx.coroutines ──
    ty(KX, "CoroutineScope", I, 0, 0, None, &[]),
    ty(KX, "Job", I, 0, 0, None, &[CONTEXT_ELEMENT]),
    ty(KX, "Deferred", I, 1, 0, None, &[JOB]),
    ty(KX, "CompletableJob", I, 0, 0, None, &[JOB]),
    ty(KX, "CompletableDeferred", I, 1, 0, None, &[DEFERRED]),
    ty(KX, "DisposableHandle", I, 0, 0, None, &[]),
    ty(KX, "CoroutineDispatcher", C, 0, 0, None, &[ABSTRACT_CONTEXT_ELEMENT, CONTINUATION_INTERCEPTOR]),
    ty(KX, "MainCoroutineDispatcher", C, 0, 0, None, &[DISPATCHER]),
    ty(KX, "Dispatchers", C, 0, OBJ, None, &[]),
    ty(KX, "GlobalScope", C, 0, OBJ, None, &[SCOPE]),
    ty(KX, "NonCancellable", C, 0, OBJ, None, &[JOB]),
    ty(KX, "CoroutineStart", E, 0, 0, None, &[]),
    ty(KX, "CoroutineName", C, 0, 0, None, &[ABSTRACT_CONTEXT_ELEMENT]),
    ty(KX, "CoroutineExceptionHandler", I, 0, 0, None, &[CONTEXT_ELEMENT]),
    ty(KX, "CancellationException", C, 0, ALIAS | THR, Some("java.util.concurrent.CancellationException"), &[ILLEGAL_STATE]),
    ty(KX, "TimeoutCancellationException", C, 0, THR, None, &[CANCELLATION]),
    ty(KX, "DelicateCoroutinesApi", A, 0, 0, None, &[]),
    ty(KX, "ExperimentalCoroutinesApi", A, 0, 0, None, &[]),
    // ── kotlinx.coroutines.flow ──
    ty(KXF, "Flow", I, 1, 0, None, &[]),
    ty(KXF, "FlowCollector", I, 1, 0, None, &[]),
    ty(KXF, "SharedFlow", I, 1, 0, None, &[FLOW]),
    ty(KXF, "StateFlow", I, 1, 0, None, &[SHARED_FLOW]),
    ty(KXF, "MutableSharedFlow", I, 1, 0, None, &[SHARED_FLOW, FLOW_COLLECTOR]),
    ty(KXF, "MutableStateFlow", I, 1, 0, None, &[STATE_FLOW, MUTABLE_SHARED_FLOW]),
    ty(KXF, "SharingStarted", I, 0, 0, None, &[]),
    // ── kotlinx.coroutines.channels ──
    ty(KXC, "SendChannel", I, 1, 0, None, &[]),
    ty(KXC, "ReceiveChannel", I, 1, 0, None, &[]),
    ty(KXC, "Channel", I, 1, 0, None, &[SEND_CHANNEL, RECEIVE_CHANNEL]),
    ty(KXC, "ProducerScope", I, 1, 0, None, &[SCOPE, SEND_CHANNEL]),
    ty(KXC, "ChannelResult", C, 1, VAL, None, &[]),
    ty(KXC, "BufferOverflow", E, 0, 0, None, &[]),
    ty(KXC, "ClosedReceiveChannelException", C, 0, THR, None, &[NO_SUCH_ELEMENT]),
    ty(KXC, "ClosedSendChannelException", C, 0, THR, None, &[ILLEGAL_STATE]),
    // ── kotlinx.coroutines.sync ──
    ty(KXS, "Mutex", I, 0, 0, None, &[]),
    ty(KXS, "Semaphore", I, 0, 0, None, &[]),
];

const fn str_eq(a: &str, b: &str) -> bool {
    let a = a.as_bytes();
    let b = b.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

const fn is_type_at(index: usize, package: &str, name: &str) -> bool {
    str_eq(KOTLIN_TYPES[index].package, package) && str_eq(KOTLIN_TYPES[index].name, name)
}

// The fixed ids name the entries they claim to.
const _: () = {
    assert!(is_type_at(0, "kotlin", "Any"));
    assert!(is_type_at(1, "kotlin", "Nothing"));
    assert!(is_type_at(2, "kotlin", "Unit"));
    assert!(is_type_at(3, "kotlin", "String"));
    assert!(is_type_at(4, "kotlin", "CharSequence"));
    assert!(is_type_at(5, "kotlin", "Number"));
    assert!(is_type_at(6, "kotlin", "Boolean"));
    assert!(is_type_at(7, "kotlin", "Char"));
    assert!(is_type_at(8, "kotlin", "Byte"));
    assert!(is_type_at(9, "kotlin", "Short"));
    assert!(is_type_at(10, "kotlin", "Int"));
    assert!(is_type_at(11, "kotlin", "Long"));
    assert!(is_type_at(12, "kotlin", "Float"));
    assert!(is_type_at(13, "kotlin", "Double"));
    assert!(is_type_at(14, "kotlin", "Throwable"));
    assert!(KOTLIN_TYPES.len() <= (KOTLIN_CALLABLE_STUB_BASE_ID - KOTLIN_BASE_STUB_SYMBOL_ID) as usize);
};

// ── Callable vocabulary ──

const SUS: u16 = KotlinCallableFlags::SUSPEND;
const INL: u16 = KotlinCallableFlags::INLINE;
const INF: u16 = KotlinCallableFlags::INFIX;
const OPR: u16 = KotlinCallableFlags::OPERATOR;
const NUL: u16 = KotlinCallableFlags::NULLABLE_RECEIVER;
const REI: u16 = KotlinCallableFlags::REIFIED;
const VAR: u16 = KotlinCallableFlags::VARARG;
const JV: u16 = KotlinCallableFlags::JVM_ONLY;
const V: u8 = VARIADIC;

/// Top-level function.
const fn top_fn(package: &'static str, name: &'static str, flags: u16, min_args: u8, max_args: u8, returns: Option<&'static str>) -> KotlinCallableStub {
    KotlinCallableStub {
        package,
        name,
        kind: KotlinCallableKind::Function,
        owner: None,
        receiver: None,
        flags,
        min_args,
        max_args,
        returns,
    }
}

/// Top-level property.
const fn top_prop(package: &'static str, name: &'static str, flags: u16, returns: Option<&'static str>) -> KotlinCallableStub {
    KotlinCallableStub {
        package,
        name,
        kind: KotlinCallableKind::Property,
        owner: None,
        receiver: None,
        flags,
        min_args: 0,
        max_args: 0,
        returns,
    }
}

/// Extension function.
#[allow(clippy::too_many_arguments)]
const fn ext_fn(package: &'static str, receiver: &'static str, name: &'static str, flags: u16, min_args: u8, max_args: u8, returns: Option<&'static str>) -> KotlinCallableStub {
    KotlinCallableStub {
        package,
        name,
        kind: KotlinCallableKind::Function,
        owner: None,
        receiver: Some(receiver),
        flags,
        min_args,
        max_args,
        returns,
    }
}

/// Extension property.
const fn ext_prop(package: &'static str, receiver: &'static str, name: &'static str, flags: u16, returns: Option<&'static str>) -> KotlinCallableStub {
    KotlinCallableStub {
        package,
        name,
        kind: KotlinCallableKind::Property,
        owner: None,
        receiver: Some(receiver),
        flags,
        min_args: 0,
        max_args: 0,
        returns,
    }
}

/// Member function.
const fn mem_fn(owner: &'static str, name: &'static str, flags: u16, min_args: u8, max_args: u8, returns: Option<&'static str>) -> KotlinCallableStub {
    KotlinCallableStub {
        package: "",
        name,
        kind: KotlinCallableKind::Function,
        owner: Some(owner),
        receiver: None,
        flags,
        min_args,
        max_args,
        returns,
    }
}

/// Member property.
const fn mem_prop(owner: &'static str, name: &'static str, returns: Option<&'static str>) -> KotlinCallableStub {
    KotlinCallableStub {
        package: "",
        name,
        kind: KotlinCallableKind::Property,
        owner: Some(owner),
        receiver: None,
        flags: 0,
        min_args: 0,
        max_args: 0,
        returns,
    }
}

/// The built-in functions and properties.
pub const KOTLIN_CALLABLES: &[KotlinCallableStub] = &[
    // ── kotlin: top-level ──
    top_fn(K, "lazy", 0, 1, 2, Some(LAZY)),
    top_fn(K, "run", INL, 1, 1, None),
    top_fn(K, "with", INL, 2, 2, None),
    top_fn(K, "repeat", INL, 2, 2, Some(UNIT)),
    top_fn(K, "require", INL, 1, 2, Some(UNIT)),
    top_fn(K, "requireNotNull", INL, 1, 2, None),
    top_fn(K, "check", INL, 1, 2, Some(UNIT)),
    top_fn(K, "checkNotNull", INL, 1, 2, None),
    top_fn(K, "error", 0, 1, 1, Some(NOTHING)),
    top_fn(K, "TODO", INL, 0, 1, Some(NOTHING)),
    top_fn(K, "runCatching", INL, 1, 1, Some(RESULT)),
    top_fn(K, "synchronized", INL | JV, 2, 2, None),
    top_fn(K, "assert", INL | JV, 1, 2, Some(UNIT)),
    top_fn(K, "arrayOf", INL | REI | VAR, 0, V, Some(ARRAY)),
    top_fn(K, "arrayOfNulls", 0, 1, 1, Some(ARRAY)),
    top_fn(K, "emptyArray", INL | REI, 0, 0, Some(ARRAY)),
    top_fn(K, "booleanArrayOf", VAR, 0, V, Some(BOOLEAN_ARRAY)),
    top_fn(K, "byteArrayOf", VAR, 0, V, Some(BYTE_ARRAY)),
    top_fn(K, "charArrayOf", VAR, 0, V, Some(CHAR_ARRAY)),
    top_fn(K, "shortArrayOf", VAR, 0, V, Some(SHORT_ARRAY)),
    top_fn(K, "intArrayOf", VAR, 0, V, Some(INT_ARRAY)),
    top_fn(K, "longArrayOf", VAR, 0, V, Some(LONG_ARRAY)),
    top_fn(K, "floatArrayOf", VAR, 0, V, Some(FLOAT_ARRAY)),
    top_fn(K, "doubleArrayOf", VAR, 0, V, Some(DOUBLE_ARRAY)),
    top_fn(K, "enumValues", INL | REI, 0, 0, Some(ARRAY)),
    top_fn(K, "enumValueOf", INL | REI, 1, 1, None),
    // ── kotlin: scope functions and other extensions ──
    ext_fn(K, ANY, "let", INL | NUL, 1, 1, None),
    ext_fn(K, ANY, "also", INL | NUL, 1, 1, None),
    ext_fn(K, ANY, "apply", INL | NUL, 1, 1, None),
    ext_fn(K, ANY, "run", INL | NUL, 1, 1, None),
    ext_fn(K, ANY, "takeIf", INL | NUL, 1, 1, None),
    ext_fn(K, ANY, "takeUnless", INL | NUL, 1, 1, None),
    ext_fn(K, ANY, "to", INF | NUL, 1, 1, Some(PAIR)),
    ext_fn(K, ANY, "runCatching", INL | NUL, 1, 1, Some(RESULT)),
    ext_fn(K, ANY, "toString", NUL, 0, 0, Some(STRING)),
    ext_fn(K, ANY, "hashCode", NUL, 0, 0, Some(INT)),
    ext_fn(K, STRING, "plus", OPR | NUL, 1, 1, Some(STRING)),
    ext_fn(K, RESULT, "getOrThrow", INL, 0, 0, None),
    ext_fn(K, RESULT, "getOrElse", INL, 1, 1, None),
    ext_fn(K, RESULT, "getOrDefault", INL, 1, 1, None),
    ext_fn(K, RESULT, "onSuccess", INL, 1, 1, Some(RESULT)),
    ext_fn(K, RESULT, "onFailure", INL, 1, 1, Some(RESULT)),
    ext_fn(K, RESULT, "fold", INL, 2, 2, None),
    ext_fn(K, RESULT, "map", INL, 1, 1, Some(RESULT)),
    ext_fn(K, RESULT, "recover", INL, 1, 1, Some(RESULT)),
    ext_fn(K, THROWABLE, "stackTraceToString", 0, 0, 0, Some(STRING)),
    ext_fn(K, THROWABLE, "addSuppressed", 0, 1, 1, Some(UNIT)),
    ext_fn(K, "java.lang.AutoCloseable", "use", INL | NUL | JV, 1, 1, None),
    // ── kotlin: members ──
    mem_fn(ANY, "equals", OPR, 1, 1, Some(BOOLEAN)),
    mem_fn(ANY, "hashCode", 0, 0, 0, Some(INT)),
    mem_fn(ANY, "toString", 0, 0, 0, Some(STRING)),
    mem_prop(STRING, "length", Some(INT)),
    mem_fn(STRING, "get", OPR, 1, 1, Some(CHAR)),
    mem_fn(STRING, "plus", OPR, 1, 1, Some(STRING)),
    mem_fn(STRING, "compareTo", OPR, 1, 1, Some(INT)),
    mem_fn(STRING, "subSequence", 0, 2, 2, Some(CHAR_SEQUENCE)),
    mem_prop(CHAR_SEQUENCE, "length", Some(INT)),
    mem_fn(CHAR_SEQUENCE, "get", OPR, 1, 1, Some(CHAR)),
    mem_fn(CHAR_SEQUENCE, "subSequence", 0, 2, 2, Some(CHAR_SEQUENCE)),
    mem_fn(COMPARABLE, "compareTo", OPR, 1, 1, Some(INT)),
    mem_fn(COMPARATOR, "compare", 0, 2, 2, Some(INT)),
    mem_prop("kotlin.Enum", "name", Some(STRING)),
    mem_prop("kotlin.Enum", "ordinal", Some(INT)),
    mem_fn("kotlin.Enum", "compareTo", OPR, 1, 1, Some(INT)),
    mem_prop(THROWABLE, "message", Some(STRING)),
    mem_prop(THROWABLE, "cause", Some(THROWABLE)),
    mem_prop(LAZY, "value", None),
    mem_fn(LAZY, "isInitialized", 0, 0, 0, Some(BOOLEAN)),
    mem_prop(PAIR, "first", None),
    mem_prop(PAIR, "second", None),
    mem_prop("kotlin.Triple", "first", None),
    mem_prop("kotlin.Triple", "second", None),
    mem_prop("kotlin.Triple", "third", None),
    mem_prop(RESULT, "isSuccess", Some(BOOLEAN)),
    mem_prop(RESULT, "isFailure", Some(BOOLEAN)),
    mem_fn(RESULT, "getOrNull", 0, 0, 0, None),
    mem_fn(RESULT, "exceptionOrNull", 0, 0, 0, Some(THROWABLE)),
    mem_fn("kotlin.Result.Companion", "success", 0, 1, 1, Some(RESULT)),
    mem_fn("kotlin.Result.Companion", "failure", 0, 1, 1, Some(RESULT)),
    mem_prop(ARRAY, "size", Some(INT)),
    mem_fn(ARRAY, "get", OPR, 1, 1, None),
    mem_fn(ARRAY, "set", OPR, 2, 2, Some(UNIT)),
    mem_fn(ARRAY, "iterator", OPR, 0, 0, Some(ITERATOR)),
    mem_prop(INT_ARRAY, "size", Some(INT)),
    mem_fn(INT_ARRAY, "get", OPR, 1, 1, Some(INT)),
    mem_fn(INT_ARRAY, "set", OPR, 2, 2, Some(UNIT)),
    mem_fn(INT_ARRAY, "iterator", OPR, 0, 0, Some("kotlin.collections.IntIterator")),
    mem_fn(BOOLEAN, "not", OPR, 0, 0, Some(BOOLEAN)),
    mem_fn(BOOLEAN, "and", INF, 1, 1, Some(BOOLEAN)),
    mem_fn(BOOLEAN, "or", INF, 1, 1, Some(BOOLEAN)),
    mem_fn(BOOLEAN, "xor", INF, 1, 1, Some(BOOLEAN)),
    mem_prop(CHAR, "code", Some(INT)),
    mem_fn(CHAR, "rangeTo", OPR, 1, 1, Some(CHAR_RANGE)),
    mem_fn(CHAR, "plus", OPR, 1, 1, Some(CHAR)),
    mem_fn(CHAR, "minus", OPR, 1, 1, None),
    mem_fn(CHAR, "compareTo", OPR, 1, 1, Some(INT)),
    mem_fn(INT, "plus", OPR, 1, 1, None),
    mem_fn(INT, "minus", OPR, 1, 1, None),
    mem_fn(INT, "times", OPR, 1, 1, None),
    mem_fn(INT, "div", OPR, 1, 1, None),
    mem_fn(INT, "rem", OPR, 1, 1, None),
    mem_fn(INT, "compareTo", OPR, 1, 1, Some(INT)),
    mem_fn(INT, "rangeTo", OPR, 1, 1, None),
    mem_fn(INT, "rangeUntil", OPR, 1, 1, None),
    mem_fn(INT, "inc", OPR, 0, 0, Some(INT)),
    mem_fn(INT, "dec", OPR, 0, 0, Some(INT)),
    mem_fn(INT, "unaryMinus", OPR, 0, 0, Some(INT)),
    mem_fn(INT, "and", INF, 1, 1, Some(INT)),
    mem_fn(INT, "or", INF, 1, 1, Some(INT)),
    mem_fn(INT, "xor", INF, 1, 1, Some(INT)),
    mem_fn(INT, "shl", INF, 1, 1, Some(INT)),
    mem_fn(INT, "shr", INF, 1, 1, Some(INT)),
    mem_fn(INT, "ushr", INF, 1, 1, Some(INT)),
    mem_fn(INT, "inv", 0, 0, 0, Some(INT)),
    mem_fn(INT, "toByte", 0, 0, 0, Some("kotlin.Byte")),
    mem_fn(INT, "toShort", 0, 0, 0, Some("kotlin.Short")),
    mem_fn(INT, "toInt", 0, 0, 0, Some(INT)),
    mem_fn(INT, "toLong", 0, 0, 0, Some(LONG)),
    mem_fn(INT, "toFloat", 0, 0, 0, Some(FLOAT)),
    mem_fn(INT, "toDouble", 0, 0, 0, Some(DOUBLE)),
    mem_fn(INT, "toChar", 0, 0, 0, Some(CHAR)),
    mem_fn(LONG, "plus", OPR, 1, 1, None),
    mem_fn(LONG, "minus", OPR, 1, 1, None),
    mem_fn(LONG, "times", OPR, 1, 1, None),
    mem_fn(LONG, "div", OPR, 1, 1, None),
    mem_fn(LONG, "rem", OPR, 1, 1, None),
    mem_fn(LONG, "compareTo", OPR, 1, 1, Some(INT)),
    mem_fn(LONG, "rangeTo", OPR, 1, 1, Some(LONG_RANGE)),
    mem_fn(LONG, "toInt", 0, 0, 0, Some(INT)),
    mem_fn(LONG, "toDouble", 0, 0, 0, Some(DOUBLE)),
    mem_fn(DOUBLE, "plus", OPR, 1, 1, Some(DOUBLE)),
    mem_fn(DOUBLE, "minus", OPR, 1, 1, Some(DOUBLE)),
    mem_fn(DOUBLE, "times", OPR, 1, 1, Some(DOUBLE)),
    mem_fn(DOUBLE, "div", OPR, 1, 1, Some(DOUBLE)),
    mem_fn(DOUBLE, "compareTo", OPR, 1, 1, Some(INT)),
    mem_fn(DOUBLE, "toInt", 0, 0, 0, Some(INT)),
    mem_fn(DOUBLE, "toLong", 0, 0, 0, Some(LONG)),
    mem_fn(NUMBER, "toInt", 0, 0, 0, Some(INT)),
    mem_fn(NUMBER, "toLong", 0, 0, 0, Some(LONG)),
    mem_fn(NUMBER, "toDouble", 0, 0, 0, Some(DOUBLE)),
    mem_fn(NUMBER, "toFloat", 0, 0, 0, Some(FLOAT)),
    mem_prop("kotlin.Byte.Companion", "MIN_VALUE", Some("kotlin.Byte")),
    mem_prop("kotlin.Byte.Companion", "MAX_VALUE", Some("kotlin.Byte")),
    mem_prop("kotlin.Short.Companion", "MIN_VALUE", Some("kotlin.Short")),
    mem_prop("kotlin.Short.Companion", "MAX_VALUE", Some("kotlin.Short")),
    mem_prop("kotlin.Int.Companion", "MIN_VALUE", Some(INT)),
    mem_prop("kotlin.Int.Companion", "MAX_VALUE", Some(INT)),
    mem_prop("kotlin.Int.Companion", "SIZE_BYTES", Some(INT)),
    mem_prop("kotlin.Int.Companion", "SIZE_BITS", Some(INT)),
    mem_prop("kotlin.Long.Companion", "MIN_VALUE", Some(LONG)),
    mem_prop("kotlin.Long.Companion", "MAX_VALUE", Some(LONG)),
    mem_prop("kotlin.Float.Companion", "MIN_VALUE", Some(FLOAT)),
    mem_prop("kotlin.Float.Companion", "MAX_VALUE", Some(FLOAT)),
    mem_prop("kotlin.Float.Companion", "POSITIVE_INFINITY", Some(FLOAT)),
    mem_prop("kotlin.Float.Companion", "NEGATIVE_INFINITY", Some(FLOAT)),
    mem_prop("kotlin.Float.Companion", "NaN", Some(FLOAT)),
    mem_prop("kotlin.Double.Companion", "MIN_VALUE", Some(DOUBLE)),
    mem_prop("kotlin.Double.Companion", "MAX_VALUE", Some(DOUBLE)),
    mem_prop("kotlin.Double.Companion", "POSITIVE_INFINITY", Some(DOUBLE)),
    mem_prop("kotlin.Double.Companion", "NEGATIVE_INFINITY", Some(DOUBLE)),
    mem_prop("kotlin.Double.Companion", "NaN", Some(DOUBLE)),
    mem_prop("kotlin.Char.Companion", "MIN_VALUE", Some(CHAR)),
    mem_prop("kotlin.Char.Companion", "MAX_VALUE", Some(CHAR)),
    // ── kotlin.io ──
    top_fn(KIO, "println", 0, 0, 1, Some(UNIT)),
    top_fn(KIO, "print", 0, 1, 1, Some(UNIT)),
    top_fn(KIO, "readln", 0, 0, 0, Some(STRING)),
    top_fn(KIO, "readlnOrNull", 0, 0, 0, Some(STRING)),
    top_fn(KIO, "readLine", 0, 0, 0, Some(STRING)),
    ext_fn(KIO, "java.io.Closeable", "use", INL | NUL | JV, 1, 1, None),
    ext_fn(KIO, "java.io.File", "readText", JV, 0, 1, Some(STRING)),
    ext_fn(KIO, "java.io.File", "writeText", JV, 1, 2, Some(UNIT)),
    ext_fn(KIO, "java.io.File", "readLines", JV, 0, 1, Some(LIST)),
    ext_fn(KIO, "java.io.File", "forEachLine", JV, 1, 2, Some(UNIT)),
    ext_fn(KIO, "java.io.File", "bufferedReader", JV, 0, 2, Some("java.io.BufferedReader")),
    // ── kotlin.collections: builders ──
    top_fn(KC, "listOf", VAR, 0, V, Some(LIST)),
    top_fn(KC, "listOfNotNull", VAR, 0, V, Some(LIST)),
    top_fn(KC, "mutableListOf", VAR, 0, V, Some(MUTABLE_LIST)),
    top_fn(KC, "arrayListOf", VAR, 0, V, Some(ARRAY_LIST)),
    top_fn(KC, "emptyList", 0, 0, 0, Some(LIST)),
    top_fn(KC, "setOf", VAR, 0, V, Some(SET)),
    top_fn(KC, "mutableSetOf", VAR, 0, V, Some(MUTABLE_SET)),
    top_fn(KC, "hashSetOf", VAR, 0, V, Some(HASH_SET)),
    top_fn(KC, "linkedSetOf", VAR, 0, V, Some(LINKED_HASH_SET)),
    top_fn(KC, "sortedSetOf", VAR | JV, 0, V, Some("java.util.TreeSet")),
    top_fn(KC, "emptySet", 0, 0, 0, Some(SET)),
    top_fn(KC, "mapOf", VAR, 0, V, Some(MAP_T)),
    top_fn(KC, "mutableMapOf", VAR, 0, V, Some(MUTABLE_MAP)),
    top_fn(KC, "hashMapOf", VAR, 0, V, Some(HASH_MAP)),
    top_fn(KC, "linkedMapOf", VAR, 0, V, Some(LINKED_HASH_MAP)),
    top_fn(KC, "sortedMapOf", VAR | JV, 0, V, Some("java.util.SortedMap")),
    top_fn(KC, "emptyMap", 0, 0, 0, Some(MAP_T)),
    top_fn(KC, "buildList", INL, 1, 2, Some(LIST)),
    top_fn(KC, "buildSet", INL, 1, 2, Some(SET)),
    top_fn(KC, "buildMap", INL, 1, 2, Some(MAP_T)),
    top_fn(KC, "List", INL, 2, 2, Some(LIST)),
    top_fn(KC, "MutableList", INL, 2, 2, Some(MUTABLE_LIST)),
    // ── kotlin.collections: Iterable extensions ──
    ext_fn(KC, ITERABLE, "map", INL, 1, 1, Some(LIST)),
    ext_fn(KC, ITERABLE, "mapNotNull", INL, 1, 1, Some(LIST)),
    ext_fn(KC, ITERABLE, "mapIndexed", INL, 1, 1, Some(LIST)),
    ext_fn(KC, ITERABLE, "mapIndexedNotNull", INL, 1, 1, Some(LIST)),
    ext_fn(KC, ITERABLE, "flatMap", INL, 1, 1, Some(LIST)),
    ext_fn(KC, ITERABLE, "flatten", 0, 0, 0, Some(LIST)),
    ext_fn(KC, ITERABLE, "filter", INL, 1, 1, Some(LIST)),
    ext_fn(KC, ITERABLE, "filterNot", INL, 1, 1, Some(LIST)),
    ext_fn(KC, ITERABLE, "filterIndexed", INL, 1, 1, Some(LIST)),
    ext_fn(KC, ITERABLE, "filterNotNull", 0, 0, 0, Some(LIST)),
    ext_fn(KC, ITERABLE, "filterIsInstance", INL | REI, 0, 1, Some(LIST)),
    ext_fn(KC, ITERABLE, "forEach", INL, 1, 1, Some(UNIT)),
    ext_fn(KC, ITERABLE, "forEachIndexed", INL, 1, 1, Some(UNIT)),
    ext_fn(KC, ITERABLE, "onEach", INL, 1, 1, None),
    ext_fn(KC, ITERABLE, "first", INL, 0, 1, None),
    ext_fn(KC, ITERABLE, "firstOrNull", INL, 0, 1, None),
    ext_fn(KC, ITERABLE, "last", INL, 0, 1, None),
    ext_fn(KC, ITERABLE, "lastOrNull", INL, 0, 1, None),
    ext_fn(KC, ITERABLE, "single", INL, 0, 1, None),
    ext_fn(KC, ITERABLE, "singleOrNull", INL, 0, 1, None),
    ext_fn(KC, ITERABLE, "find", INL, 1, 1, None),
    ext_fn(KC, ITERABLE, "findLast", INL, 1, 1, None),
    ext_fn(KC, ITERABLE, "any", INL, 0, 1, Some(BOOLEAN)),
    ext_fn(KC, ITERABLE, "all", INL, 1, 1, Some(BOOLEAN)),
    ext_fn(KC, ITERABLE, "none", INL, 0, 1, Some(BOOLEAN)),
    ext_fn(KC, ITERABLE, "count", INL, 0, 1, Some(INT)),
    ext_fn(KC, ITERABLE, "sumOf", INL, 1, 1, None),
    ext_fn(KC, ITERABLE, "sum", 0, 0, 0, None),
    ext_fn(KC, ITERABLE, "average", 0, 0, 0, Some(DOUBLE)),
    ext_fn(KC, ITERABLE, "fold", INL, 2, 2, None),
    ext_fn(KC, ITERABLE, "foldIndexed", INL, 2, 2, None),
    ext_fn(KC, ITERABLE, "reduce", INL, 1, 1, None),
    ext_fn(KC, ITERABLE, "groupBy", INL, 1, 2, Some(MAP_T)),
    ext_fn(KC, ITERABLE, "groupingBy", INL, 1, 1, Some(GROUPING)),
    ext_fn(KC, ITERABLE, "associate", INL, 1, 1, Some(MAP_T)),
    ext_fn(KC, ITERABLE, "associateBy", INL, 1, 2, Some(MAP_T)),
    ext_fn(KC, ITERABLE, "associateWith", INL, 1, 1, Some(MAP_T)),
    ext_fn(KC, ITERABLE, "partition", INL, 1, 1, Some(PAIR)),
    ext_fn(KC, ITERABLE, "sortedBy", INL, 1, 1, Some(LIST)),
    ext_fn(KC, ITERABLE, "sortedByDescending", INL, 1, 1, Some(LIST)),
    ext_fn(KC, ITERABLE, "sorted", 0, 0, 0, Some(LIST)),
    ext_fn(KC, ITERABLE, "sortedDescending", 0, 0, 0, Some(LIST)),
    ext_fn(KC, ITERABLE, "sortedWith", 0, 1, 1, Some(LIST)),
    ext_fn(KC, ITERABLE, "reversed", 0, 0, 0, Some(LIST)),
    ext_fn(KC, ITERABLE, "shuffled", 0, 0, 1, Some(LIST)),
    ext_fn(KC, ITERABLE, "distinct", 0, 0, 0, Some(LIST)),
    ext_fn(KC, ITERABLE, "distinctBy", INL, 1, 1, Some(LIST)),
    ext_fn(KC, ITERABLE, "toList", 0, 0, 0, Some(LIST)),
    ext_fn(KC, ITERABLE, "toMutableList", 0, 0, 0, Some(MUTABLE_LIST)),
    ext_fn(KC, ITERABLE, "toSet", 0, 0, 0, Some(SET)),
    ext_fn(KC, ITERABLE, "toMutableSet", 0, 0, 0, Some(MUTABLE_SET)),
    ext_fn(KC, ITERABLE, "toHashSet", 0, 0, 0, Some(HASH_SET)),
    ext_fn(KC, ITERABLE, "toMap", 0, 0, 1, Some(MAP_T)),
    ext_fn(KC, ITERABLE, "joinToString", 0, 0, 6, Some(STRING)),
    ext_fn(KC, ITERABLE, "zip", 0, 1, 2, Some(LIST)),
    ext_fn(KC, ITERABLE, "take", 0, 1, 1, Some(LIST)),
    ext_fn(KC, ITERABLE, "drop", 0, 1, 1, Some(LIST)),
    ext_fn(KC, ITERABLE, "takeWhile", INL, 1, 1, Some(LIST)),
    ext_fn(KC, ITERABLE, "dropWhile", INL, 1, 1, Some(LIST)),
    ext_fn(KC, ITERABLE, "chunked", 0, 1, 2, Some(LIST)),
    ext_fn(KC, ITERABLE, "windowed", 0, 1, 4, Some(LIST)),
    ext_fn(KC, ITERABLE, "withIndex", 0, 0, 0, Some(ITERABLE)),
    ext_fn(KC, ITERABLE, "asSequence", 0, 0, 0, Some(SEQUENCE)),
    ext_fn(KC, ITERABLE, "maxOrNull", 0, 0, 0, None),
    ext_fn(KC, ITERABLE, "minOrNull", 0, 0, 0, None),
    ext_fn(KC, ITERABLE, "maxByOrNull", INL, 1, 1, None),
    ext_fn(KC, ITERABLE, "minByOrNull", INL, 1, 1, None),
    ext_fn(KC, ITERABLE, "maxOf", INL, 1, 1, None),
    ext_fn(KC, ITERABLE, "minOf", INL, 1, 1, None),
    ext_fn(KC, ITERABLE, "contains", OPR, 1, 1, Some(BOOLEAN)),
    ext_fn(KC, ITERABLE, "indexOf", 0, 1, 1, Some(INT)),
    ext_fn(KC, ITERABLE, "elementAt", 0, 1, 1, None),
    ext_fn(KC, ITERABLE, "plus", OPR, 1, 1, Some(LIST)),
    ext_fn(KC, ITERABLE, "minus", OPR, 1, 1, Some(LIST)),
    ext_fn(KC, ITERABLE, "union", INF, 1, 1, Some(SET)),
    ext_fn(KC, ITERABLE, "intersect", INF, 1, 1, Some(SET)),
    ext_fn(KC, ITERABLE, "subtract", INF, 1, 1, Some(SET)),
    // ── kotlin.collections: Collection / List / MutableList extensions ──
    ext_fn(KC, COLLECTION, "isNotEmpty", INL, 0, 0, Some(BOOLEAN)),
    ext_fn(KC, COLLECTION, "isNullOrEmpty", INL | NUL, 0, 0, Some(BOOLEAN)),
    ext_fn(KC, COLLECTION, "orEmpty", INL | NUL, 0, 0, Some(COLLECTION)),
    ext_fn(KC, COLLECTION, "toTypedArray", INL | REI, 0, 0, Some(ARRAY)),
    ext_fn(KC, COLLECTION, "random", 0, 0, 1, None),
    ext_fn(KC, COLLECTION, "randomOrNull", 0, 0, 1, None),
    ext_fn(KC, COLLECTION, "ifEmpty", INL, 1, 1, None),
    ext_prop(KC, COLLECTION, "indices", 0, Some(INT_RANGE)),
    ext_fn(KC, LIST, "orEmpty", INL | NUL, 0, 0, Some(LIST)),
    ext_fn(KC, LIST, "getOrNull", 0, 1, 1, None),
    ext_fn(KC, LIST, "getOrElse", INL, 2, 2, None),
    ext_fn(KC, LIST, "asReversed", 0, 0, 0, Some(LIST)),
    ext_fn(KC, LIST, "binarySearch", 0, 0, 3, Some(INT)),
    ext_fn(KC, LIST, "slice", 0, 1, 1, Some(LIST)),
    ext_fn(KC, LIST, "takeLast", 0, 1, 1, Some(LIST)),
    ext_fn(KC, LIST, "dropLast", 0, 1, 1, Some(LIST)),
    ext_fn(KC, LIST, "component1", INL | OPR, 0, 0, None),
    ext_fn(KC, LIST, "component2", INL | OPR, 0, 0, None),
    ext_fn(KC, LIST, "component3", INL | OPR, 0, 0, None),
    ext_fn(KC, LIST, "component4", INL | OPR, 0, 0, None),
    ext_fn(KC, LIST, "component5", INL | OPR, 0, 0, None),
    ext_prop(KC, LIST, "lastIndex", 0, Some(INT)),
    ext_fn(KC, SET, "orEmpty", INL | NUL, 0, 0, Some(SET)),
    ext_fn(KC, MUTABLE_LIST, "sortBy", INL, 1, 1, Some(UNIT)),
    ext_fn(KC, MUTABLE_LIST, "sortByDescending", INL, 1, 1, Some(UNIT)),
    ext_fn(KC, MUTABLE_LIST, "sortWith", 0, 1, 1, Some(UNIT)),
    ext_fn(KC, MUTABLE_LIST, "sort", 0, 0, 0, Some(UNIT)),
    ext_fn(KC, MUTABLE_LIST, "shuffle", 0, 0, 1, Some(UNIT)),
    ext_fn(KC, MUTABLE_LIST, "reverse", 0, 0, 0, Some(UNIT)),
    ext_fn(KC, MUTABLE_LIST, "fill", 0, 1, 1, Some(UNIT)),
    ext_fn(KC, MUTABLE_LIST, "removeFirst", 0, 0, 0, None),
    ext_fn(KC, MUTABLE_LIST, "removeLast", 0, 0, 0, None),
    ext_fn(KC, MUTABLE_LIST, "removeFirstOrNull", 0, 0, 0, None),
    ext_fn(KC, MUTABLE_LIST, "removeLastOrNull", 0, 0, 0, None),
    ext_fn(KC, MUTABLE_COLLECTION, "plusAssign", INL | OPR, 1, 1, Some(UNIT)),
    ext_fn(KC, MUTABLE_COLLECTION, "minusAssign", INL | OPR, 1, 1, Some(UNIT)),
    ext_fn(KC, MUTABLE_COLLECTION, "addAll", 0, 1, 1, Some(BOOLEAN)),
    ext_fn(KC, MUTABLE_ITERABLE, "removeAll", 0, 1, 1, Some(BOOLEAN)),
    ext_fn(KC, MUTABLE_ITERABLE, "retainAll", 0, 1, 1, Some(BOOLEAN)),
    // ── kotlin.collections: Map extensions ──
    ext_fn(KC, MAP_T, "getOrElse", INL, 2, 2, None),
    ext_fn(KC, MAP_T, "getValue", 0, 1, 1, None),
    ext_fn(KC, MAP_T, "filter", INL, 1, 1, Some(MAP_T)),
    ext_fn(KC, MAP_T, "filterNot", INL, 1, 1, Some(MAP_T)),
    ext_fn(KC, MAP_T, "filterKeys", INL, 1, 1, Some(MAP_T)),
    ext_fn(KC, MAP_T, "filterValues", INL, 1, 1, Some(MAP_T)),
    ext_fn(KC, MAP_T, "mapValues", INL, 1, 1, Some(MAP_T)),
    ext_fn(KC, MAP_T, "mapKeys", INL, 1, 1, Some(MAP_T)),
    ext_fn(KC, MAP_T, "map", INL, 1, 1, Some(LIST)),
    ext_fn(KC, MAP_T, "mapNotNull", INL, 1, 1, Some(LIST)),
    ext_fn(KC, MAP_T, "flatMap", INL, 1, 1, Some(LIST)),
    ext_fn(KC, MAP_T, "forEach", INL, 1, 1, Some(UNIT)),
    ext_fn(KC, MAP_T, "toList", 0, 0, 0, Some(LIST)),
    ext_fn(KC, MAP_T, "toMutableMap", 0, 0, 0, Some(MUTABLE_MAP)),
    ext_fn(KC, MAP_T, "toSortedMap", JV, 0, 1, Some("java.util.SortedMap")),
    ext_fn(KC, MAP_T, "plus", OPR, 1, 1, Some(MAP_T)),
    ext_fn(KC, MAP_T, "minus", OPR, 1, 1, Some(MAP_T)),
    ext_fn(KC, MAP_T, "isNotEmpty", INL, 0, 0, Some(BOOLEAN)),
    ext_fn(KC, MAP_T, "isNullOrEmpty", INL | NUL, 0, 0, Some(BOOLEAN)),
    ext_fn(KC, MAP_T, "orEmpty", INL | NUL, 0, 0, Some(MAP_T)),
    ext_fn(KC, MAP_T, "count", INL, 0, 1, Some(INT)),
    ext_fn(KC, MAP_T, "any", INL, 0, 1, Some(BOOLEAN)),
    ext_fn(KC, MAP_T, "all", INL, 1, 1, Some(BOOLEAN)),
    ext_fn(KC, MAP_T, "none", INL, 0, 1, Some(BOOLEAN)),
    ext_fn(KC, MAP_T, "maxByOrNull", INL, 1, 1, Some(MAP_ENTRY)),
    ext_fn(KC, MAP_T, "minByOrNull", INL, 1, 1, Some(MAP_ENTRY)),
    ext_fn(KC, MAP_T, "contains", INL | OPR, 1, 1, Some(BOOLEAN)),
    ext_fn(KC, MAP_T, "asSequence", 0, 0, 0, Some(SEQUENCE)),
    ext_fn(KC, MAP_T, "iterator", INL | OPR, 0, 0, Some(ITERATOR)),
    ext_fn(KC, MAP_T, "withDefault", 0, 1, 1, Some(MAP_T)),
    ext_fn(KC, MAP_ENTRY, "component1", INL | OPR, 0, 0, None),
    ext_fn(KC, MAP_ENTRY, "component2", INL | OPR, 0, 0, None),
    ext_fn(KC, MAP_ENTRY, "toPair", 0, 0, 0, Some(PAIR)),
    ext_fn(KC, MUTABLE_MAP, "getOrPut", INL, 2, 2, None),
    ext_fn(KC, MUTABLE_MAP, "set", INL | OPR, 2, 2, Some(UNIT)),
    ext_fn(KC, MUTABLE_MAP, "plusAssign", INL | OPR, 1, 1, Some(UNIT)),
    ext_fn(KC, MUTABLE_MAP, "minusAssign", INL | OPR, 1, 1, Some(UNIT)),
    ext_fn(KC, MUTABLE_MAP, "putAll", 0, 1, 1, Some(UNIT)),
    // ── kotlin.collections: Array extensions ──
    ext_fn(KC, ARRAY, "forEach", INL, 1, 1, Some(UNIT)),
    ext_fn(KC, ARRAY, "forEachIndexed", INL, 1, 1, Some(UNIT)),
    ext_fn(KC, ARRAY, "map", INL, 1, 1, Some(LIST)),
    ext_fn(KC, ARRAY, "mapNotNull", INL, 1, 1, Some(LIST)),
    ext_fn(KC, ARRAY, "filter", INL, 1, 1, Some(LIST)),
    ext_fn(KC, ARRAY, "toList", 0, 0, 0, Some(LIST)),
    ext_fn(KC, ARRAY, "toMutableList", 0, 0, 0, Some(MUTABLE_LIST)),
    ext_fn(KC, ARRAY, "toSet", 0, 0, 0, Some(SET)),
    ext_fn(KC, ARRAY, "asList", 0, 0, 0, Some(LIST)),
    ext_fn(KC, ARRAY, "isEmpty", INL, 0, 0, Some(BOOLEAN)),
    ext_fn(KC, ARRAY, "isNotEmpty", INL, 0, 0, Some(BOOLEAN)),
    ext_fn(KC, ARRAY, "contains", OPR, 1, 1, Some(BOOLEAN)),
    ext_fn(KC, ARRAY, "indexOf", 0, 1, 1, Some(INT)),
    ext_fn(KC, ARRAY, "joinToString", 0, 0, 6, Some(STRING)),
    ext_fn(KC, ARRAY, "first", INL, 0, 1, None),
    ext_fn(KC, ARRAY, "last", INL, 0, 1, None),
    ext_fn(KC, ARRAY, "firstOrNull", INL, 0, 1, None),
    ext_fn(KC, ARRAY, "lastOrNull", INL, 0, 1, None),
    ext_fn(KC, ARRAY, "getOrNull", 0, 1, 1, None),
    ext_fn(KC, ARRAY, "any", INL, 0, 1, Some(BOOLEAN)),
    ext_fn(KC, ARRAY, "all", INL, 1, 1, Some(BOOLEAN)),
    ext_fn(KC, ARRAY, "none", INL, 0, 1, Some(BOOLEAN)),
    ext_fn(KC, ARRAY, "count", INL, 0, 1, Some(INT)),
    ext_fn(KC, ARRAY, "sumOf", INL, 1, 1, None),
    ext_fn(KC, ARRAY, "sorted", 0, 0, 0, Some(LIST)),
    ext_fn(KC, ARRAY, "sortedArray", 0, 0, 0, Some(ARRAY)),
    ext_fn(KC, ARRAY, "reversed", 0, 0, 0, Some(LIST)),
    ext_fn(KC, ARRAY, "copyOf", 0, 0, 1, Some(ARRAY)),
    ext_fn(KC, ARRAY, "fill", 0, 1, 3, Some(UNIT)),
    ext_fn(KC, ARRAY, "plus", OPR, 1, 1, Some(ARRAY)),
    ext_fn(KC, ARRAY, "withIndex", 0, 0, 0, Some(ITERABLE)),
    ext_fn(KC, ARRAY, "asSequence", 0, 0, 0, Some(SEQUENCE)),
    ext_prop(KC, ARRAY, "indices", 0, Some(INT_RANGE)),
    ext_prop(KC, ARRAY, "lastIndex", 0, Some(INT)),
    ext_fn(KC, INT_ARRAY, "sum", 0, 0, 0, Some(INT)),
    ext_fn(KC, INT_ARRAY, "average", 0, 0, 0, Some(DOUBLE)),
    ext_fn(KC, INT_ARRAY, "toList", 0, 0, 0, Some(LIST)),
    ext_fn(KC, INT_ARRAY, "maxOrNull", 0, 0, 0, Some(INT)),
    ext_fn(KC, INT_ARRAY, "minOrNull", 0, 0, 0, Some(INT)),
    ext_fn(KC, INT_ARRAY, "sorted", 0, 0, 0, Some(LIST)),
    ext_fn(KC, INT_ARRAY, "forEach", INL, 1, 1, Some(UNIT)),
    ext_fn(KC, INT_ARRAY, "map", INL, 1, 1, Some(LIST)),
    ext_fn(KC, INT_ARRAY, "contains", OPR, 1, 1, Some(BOOLEAN)),
    // ── kotlin.collections: members ──
    mem_fn(ITERABLE, "iterator", OPR, 0, 0, Some(ITERATOR)),
    mem_fn(MUTABLE_ITERABLE, "iterator", OPR, 0, 0, Some(MUTABLE_ITERATOR)),
    mem_prop(COLLECTION, "size", Some(INT)),
    mem_fn(COLLECTION, "isEmpty", 0, 0, 0, Some(BOOLEAN)),
    mem_fn(COLLECTION, "contains", OPR, 1, 1, Some(BOOLEAN)),
    mem_fn(COLLECTION, "containsAll", 0, 1, 1, Some(BOOLEAN)),
    mem_fn(MUTABLE_COLLECTION, "add", 0, 1, 1, Some(BOOLEAN)),
    mem_fn(MUTABLE_COLLECTION, "remove", 0, 1, 1, Some(BOOLEAN)),
    mem_fn(MUTABLE_COLLECTION, "addAll", 0, 1, 1, Some(BOOLEAN)),
    mem_fn(MUTABLE_COLLECTION, "removeAll", 0, 1, 1, Some(BOOLEAN)),
    mem_fn(MUTABLE_COLLECTION, "retainAll", 0, 1, 1, Some(BOOLEAN)),
    mem_fn(MUTABLE_COLLECTION, "clear", 0, 0, 0, Some(UNIT)),
    mem_fn(LIST, "get", OPR, 1, 1, None),
    mem_fn(LIST, "indexOf", 0, 1, 1, Some(INT)),
    mem_fn(LIST, "lastIndexOf", 0, 1, 1, Some(INT)),
    mem_fn(LIST, "subList", 0, 2, 2, Some(LIST)),
    mem_fn(LIST, "listIterator", 0, 0, 1, Some(LIST_ITERATOR)),
    mem_fn(MUTABLE_LIST, "add", 0, 1, 2, None),
    mem_fn(MUTABLE_LIST, "addAll", 0, 1, 2, Some(BOOLEAN)),
    mem_fn(MUTABLE_LIST, "set", OPR, 2, 2, None),
    mem_fn(MUTABLE_LIST, "removeAt", 0, 1, 1, None),
    mem_fn(MUTABLE_LIST, "subList", 0, 2, 2, Some(MUTABLE_LIST)),
    mem_fn(MUTABLE_LIST, "listIterator", 0, 0, 1, Some(MUTABLE_LIST_ITERATOR)),
    mem_prop(MAP_T, "size", Some(INT)),
    mem_fn(MAP_T, "isEmpty", 0, 0, 0, Some(BOOLEAN)),
    mem_fn(MAP_T, "containsKey", 0, 1, 1, Some(BOOLEAN)),
    mem_fn(MAP_T, "containsValue", 0, 1, 1, Some(BOOLEAN)),
    mem_fn(MAP_T, "get", OPR, 1, 1, None),
    mem_fn(MAP_T, "getOrDefault", 0, 2, 2, None),
    mem_prop(MAP_T, "keys", Some(SET)),
    mem_prop(MAP_T, "values", Some(COLLECTION)),
    mem_prop(MAP_T, "entries", Some(SET)),
    mem_fn(MUTABLE_MAP, "put", 0, 2, 2, None),
    mem_fn(MUTABLE_MAP, "remove", 0, 1, 2, None),
    mem_fn(MUTABLE_MAP, "putAll", 0, 1, 1, Some(UNIT)),
    mem_fn(MUTABLE_MAP, "clear", 0, 0, 0, Some(UNIT)),
    mem_prop(MUTABLE_MAP, "keys", Some(MUTABLE_SET)),
    mem_prop(MUTABLE_MAP, "values", Some(MUTABLE_COLLECTION)),
    mem_prop(MUTABLE_MAP, "entries", Some(MUTABLE_SET)),
    mem_prop(MAP_ENTRY, "key", None),
    mem_prop(MAP_ENTRY, "value", None),
    mem_fn("kotlin.collections.MutableMap.MutableEntry", "setValue", 0, 1, 1, None),
    mem_fn(ITERATOR, "next", OPR, 0, 0, None),
    mem_fn(ITERATOR, "hasNext", OPR, 0, 0, Some(BOOLEAN)),
    mem_fn(MUTABLE_ITERATOR, "remove", 0, 0, 0, Some(UNIT)),
    mem_fn(LIST_ITERATOR, "hasPrevious", 0, 0, 0, Some(BOOLEAN)),
    mem_fn(LIST_ITERATOR, "previous", 0, 0, 0, None),
    mem_fn(LIST_ITERATOR, "nextIndex", 0, 0, 0, Some(INT)),
    mem_fn(LIST_ITERATOR, "previousIndex", 0, 0, 0, Some(INT)),
    mem_fn("kotlin.collections.ArrayDeque", "addFirst", 0, 1, 1, Some(UNIT)),
    mem_fn("kotlin.collections.ArrayDeque", "addLast", 0, 1, 1, Some(UNIT)),
    mem_fn("kotlin.collections.ArrayDeque", "removeFirst", 0, 0, 0, None),
    mem_fn("kotlin.collections.ArrayDeque", "removeLast", 0, 0, 0, None),
    mem_prop(INDEXED_VALUE, "index", Some(INT)),
    mem_prop(INDEXED_VALUE, "value", None),
    // ── kotlin.ranges ──
    ext_fn(KR, INT, "until", INF, 1, 1, Some(INT_RANGE)),
    ext_fn(KR, INT, "downTo", INF, 1, 1, Some(INT_PROGRESSION)),
    ext_fn(KR, LONG, "until", INF, 1, 1, Some(LONG_RANGE)),
    ext_fn(KR, LONG, "downTo", INF, 1, 1, Some(LONG_PROGRESSION)),
    ext_fn(KR, CHAR, "until", INF, 1, 1, Some(CHAR_RANGE)),
    ext_fn(KR, CHAR, "downTo", INF, 1, 1, Some(CHAR_PROGRESSION)),
    ext_fn(KR, INT_PROGRESSION, "step", INF, 1, 1, Some(INT_PROGRESSION)),
    ext_fn(KR, INT_PROGRESSION, "reversed", 0, 0, 0, Some(INT_PROGRESSION)),
    ext_fn(KR, LONG_PROGRESSION, "step", INF, 1, 1, Some(LONG_PROGRESSION)),
    ext_fn(KR, CHAR_PROGRESSION, "step", INF, 1, 1, Some(CHAR_PROGRESSION)),
    ext_fn(KR, INT_RANGE, "random", 0, 0, 1, Some(INT)),
    ext_fn(KR, INT_RANGE, "randomOrNull", 0, 0, 1, Some(INT)),
    ext_fn(KR, COMPARABLE, "coerceIn", 0, 1, 2, None),
    ext_fn(KR, COMPARABLE, "coerceAtLeast", 0, 1, 1, None),
    ext_fn(KR, COMPARABLE, "coerceAtMost", 0, 1, 1, None),
    ext_fn(KR, COMPARABLE, "rangeTo", OPR, 1, 1, Some(CLOSED_RANGE)),
    ext_fn(KR, COMPARABLE, "rangeUntil", OPR, 1, 1, Some(OPEN_END_RANGE)),
    mem_prop(CLOSED_RANGE, "start", None),
    mem_prop(CLOSED_RANGE, "endInclusive", None),
    mem_fn(CLOSED_RANGE, "contains", OPR, 1, 1, Some(BOOLEAN)),
    mem_fn(CLOSED_RANGE, "isEmpty", 0, 0, 0, Some(BOOLEAN)),
    mem_prop(OPEN_END_RANGE, "start", None),
    mem_prop(OPEN_END_RANGE, "endExclusive", None),
    mem_fn(OPEN_END_RANGE, "contains", OPR, 1, 1, Some(BOOLEAN)),
    mem_fn(OPEN_END_RANGE, "isEmpty", 0, 0, 0, Some(BOOLEAN)),
    mem_prop(INT_PROGRESSION, "first", Some(INT)),
    mem_prop(INT_PROGRESSION, "last", Some(INT)),
    mem_prop(INT_PROGRESSION, "step", Some(INT)),
    mem_fn(INT_PROGRESSION, "isEmpty", 0, 0, 0, Some(BOOLEAN)),
    mem_fn(INT_PROGRESSION, "iterator", OPR, 0, 0, Some("kotlin.collections.IntIterator")),
    // ── kotlin.text ──
    top_fn(KT, "buildString", INL, 1, 2, Some(STRING)),
    ext_fn(KT, STRING, "trim", INL, 0, V, Some(STRING)),
    ext_fn(KT, STRING, "trimStart", INL, 0, V, Some(STRING)),
    ext_fn(KT, STRING, "trimEnd", INL, 0, V, Some(STRING)),
    ext_fn(KT, CHAR_SEQUENCE, "split", VAR, 0, V, Some(LIST)),
    ext_fn(KT, CHAR_SEQUENCE, "lines", 0, 0, 0, Some(LIST)),
    ext_fn(KT, CHAR_SEQUENCE, "isBlank", 0, 0, 0, Some(BOOLEAN)),
    ext_fn(KT, CHAR_SEQUENCE, "isNotBlank", INL, 0, 0, Some(BOOLEAN)),
    ext_fn(KT, CHAR_SEQUENCE, "isEmpty", INL, 0, 0, Some(BOOLEAN)),
    ext_fn(KT, CHAR_SEQUENCE, "isNotEmpty", INL, 0, 0, Some(BOOLEAN)),
    ext_fn(KT, CHAR_SEQUENCE, "isNullOrBlank", INL | NUL, 0, 0, Some(BOOLEAN)),
    ext_fn(KT, CHAR_SEQUENCE, "isNullOrEmpty", INL | NUL, 0, 0, Some(BOOLEAN)),
    ext_fn(KT, STRING, "orEmpty", INL | NUL, 0, 0, Some(STRING)),
    ext_fn(KT, CHAR_SEQUENCE, "ifEmpty", INL, 1, 1, None),
    ext_fn(KT, CHAR_SEQUENCE, "ifBlank", INL, 1, 1, None),
    ext_fn(KT, STRING, "substring", INL, 1, 2, Some(STRING)),
    ext_fn(KT, STRING, "substringBefore", 0, 1, 2, Some(STRING)),
    ext_fn(KT, STRING, "substringAfter", 0, 1, 2, Some(STRING)),
    ext_fn(KT, STRING, "substringBeforeLast", 0, 1, 2, Some(STRING)),
    ext_fn(KT, STRING, "substringAfterLast", 0, 1, 2, Some(STRING)),
    ext_fn(KT, STRING, "startsWith", 0, 1, 3, Some(BOOLEAN)),
    ext_fn(KT, STRING, "endsWith", 0, 1, 2, Some(BOOLEAN)),
    ext_fn(KT, CHAR_SEQUENCE, "contains", OPR, 1, 2, Some(BOOLEAN)),
    ext_fn(KT, STRING, "replace", 0, 2, 3, Some(STRING)),
    ext_fn(KT, STRING, "replaceFirst", 0, 2, 3, Some(STRING)),
    ext_fn(KT, CHAR_SEQUENCE, "indexOf", 0, 1, 3, Some(INT)),
    ext_fn(KT, CHAR_SEQUENCE, "lastIndexOf", 0, 1, 3, Some(INT)),
    ext_fn(KT, STRING, "toInt", 0, 0, 1, Some(INT)),
    ext_fn(KT, STRING, "toIntOrNull", 0, 0, 1, Some(INT)),
    ext_fn(KT, STRING, "toLong", 0, 0, 1, Some(LONG)),
    ext_fn(KT, STRING, "toLongOrNull", 0, 0, 1, Some(LONG)),
    ext_fn(KT, STRING, "toDouble", 0, 0, 0, Some(DOUBLE)),
    ext_fn(KT, STRING, "toDoubleOrNull", 0, 0, 0, Some(DOUBLE)),
    ext_fn(KT, STRING, "toFloat", 0, 0, 0, Some(FLOAT)),
    ext_fn(KT, STRING, "toBoolean", 0, 0, 0, Some(BOOLEAN)),
    ext_fn(KT, STRING, "lowercase", 0, 0, 1, Some(STRING)),
    ext_fn(KT, STRING, "uppercase", 0, 0, 1, Some(STRING)),
    ext_fn(KT, STRING, "replaceFirstChar", INL, 1, 1, Some(STRING)),
    ext_fn(KT, STRING, "padStart", 0, 1, 2, Some(STRING)),
    ext_fn(KT, STRING, "padEnd", 0, 1, 2, Some(STRING)),
    ext_fn(KT, CHAR_SEQUENCE, "repeat", 0, 1, 1, Some(STRING)),
    ext_fn(KT, STRING, "reversed", INL, 0, 0, Some(STRING)),
    ext_fn(KT, STRING, "toRegex", 0, 0, 1, Some(REGEX)),
    ext_fn(KT, STRING, "removePrefix", 0, 1, 1, Some(STRING)),
    ext_fn(KT, STRING, "removeSuffix", 0, 1, 1, Some(STRING)),
    ext_fn(KT, STRING, "removeSurrounding", 0, 1, 2, Some(STRING)),
    ext_fn(KT, STRING, "trimIndent", 0, 0, 0, Some(STRING)),
    ext_fn(KT, STRING, "trimMargin", 0, 0, 1, Some(STRING)),
    ext_fn(KT, STRING, "toCharArray", INL, 0, 2, Some(CHAR_ARRAY)),
    ext_fn(KT, STRING, "toByteArray", INL | JV, 0, 1, Some(BYTE_ARRAY)),
    ext_fn(KT, STRING, "encodeToByteArray", 0, 0, 0, Some(BYTE_ARRAY)),
    ext_fn(KT, STRING, "format", INL | VAR | JV, 0, V, Some(STRING)),
    ext_fn(KT, "kotlin.String.Companion", "format", INL | VAR | JV, 1, V, Some(STRING)),
    ext_fn(KT, STRING, "filter", INL, 1, 1, Some(STRING)),
    ext_fn(KT, STRING, "equals", NUL, 1, 2, Some(BOOLEAN)),
    ext_fn(KT, CHAR_SEQUENCE, "first", INL, 0, 1, Some(CHAR)),
    ext_fn(KT, CHAR_SEQUENCE, "last", INL, 0, 1, Some(CHAR)),
    ext_fn(KT, CHAR_SEQUENCE, "firstOrNull", INL, 0, 1, Some(CHAR)),
    ext_fn(KT, CHAR_SEQUENCE, "lastOrNull", INL, 0, 1, Some(CHAR)),
    ext_fn(KT, CHAR_SEQUENCE, "map", INL, 1, 1, Some(LIST)),
    ext_fn(KT, CHAR_SEQUENCE, "forEach", INL, 1, 1, Some(UNIT)),
    ext_fn(KT, CHAR_SEQUENCE, "count", INL, 0, 1, Some(INT)),
    ext_fn(KT, CHAR_SEQUENCE, "any", INL, 0, 1, Some(BOOLEAN)),
    ext_fn(KT, CHAR_SEQUENCE, "all", INL, 1, 1, Some(BOOLEAN)),
    ext_fn(KT, CHAR_SEQUENCE, "none", INL, 0, 1, Some(BOOLEAN)),
    ext_fn(KT, CHAR_SEQUENCE, "matches", INF, 1, 1, Some(BOOLEAN)),
    ext_fn(KT, CHAR_SEQUENCE, "chunked", 0, 1, 2, Some(LIST)),
    ext_fn(KT, CHAR_SEQUENCE, "windowed", 0, 1, 4, Some(LIST)),
    ext_fn(KT, CHAR_SEQUENCE, "lineSequence", 0, 0, 0, Some(SEQUENCE)),
    ext_fn(KT, CHAR_SEQUENCE, "asSequence", 0, 0, 0, Some(SEQUENCE)),
    ext_fn(KT, CHAR, "isDigit", 0, 0, 0, Some(BOOLEAN)),
    ext_fn(KT, CHAR, "isLetter", 0, 0, 0, Some(BOOLEAN)),
    ext_fn(KT, CHAR, "isLetterOrDigit", 0, 0, 0, Some(BOOLEAN)),
    ext_fn(KT, CHAR, "isWhitespace", 0, 0, 0, Some(BOOLEAN)),
    ext_fn(KT, CHAR, "isUpperCase", 0, 0, 0, Some(BOOLEAN)),
    ext_fn(KT, CHAR, "isLowerCase", 0, 0, 0, Some(BOOLEAN)),
    ext_fn(KT, CHAR, "uppercaseChar", 0, 0, 0, Some(CHAR)),
    ext_fn(KT, CHAR, "lowercaseChar", 0, 0, 0, Some(CHAR)),
    ext_fn(KT, CHAR, "uppercase", 0, 0, 0, Some(STRING)),
    ext_fn(KT, CHAR, "lowercase", 0, 0, 0, Some(STRING)),
    ext_fn(KT, CHAR, "titlecase", 0, 0, 0, Some(STRING)),
    ext_fn(KT, CHAR, "digitToInt", 0, 0, 1, Some(INT)),
    ext_fn(KT, CHAR, "digitToIntOrNull", 0, 0, 1, Some(INT)),
    ext_fn(KT, STRING_BUILDER, "appendLine", INL, 0, 1, Some(STRING_BUILDER)),
    ext_fn(KT, STRING_BUILDER, "clear", INL | JV, 0, 0, Some(STRING_BUILDER)),
    mem_fn(STRING_BUILDER, "append", 0, 1, 1, Some(STRING_BUILDER)),
    mem_fn(STRING_BUILDER, "insert", 0, 2, 2, Some(STRING_BUILDER)),
    mem_fn(STRING_BUILDER, "reverse", 0, 0, 0, Some(STRING_BUILDER)),
    mem_fn(STRING_BUILDER, "deleteCharAt", 0, 1, 1, Some(STRING_BUILDER)),
    mem_prop(STRING_BUILDER, "length", Some(INT)),
    mem_fn(REGEX, "find", 0, 1, 2, Some(MATCH_RESULT)),
    mem_fn(REGEX, "findAll", 0, 1, 2, Some(SEQUENCE)),
    mem_fn(REGEX, "matches", INF, 1, 1, Some(BOOLEAN)),
    mem_fn(REGEX, "containsMatchIn", 0, 1, 1, Some(BOOLEAN)),
    mem_fn(REGEX, "matchEntire", 0, 1, 1, Some(MATCH_RESULT)),
    mem_fn(REGEX, "replace", 0, 2, 2, Some(STRING)),
    mem_fn(REGEX, "replaceFirst", 0, 2, 2, Some(STRING)),
    mem_fn(REGEX, "split", 0, 1, 2, Some(LIST)),
    mem_prop(REGEX, "pattern", Some(STRING)),
    mem_prop(REGEX, "options", Some(SET)),
    mem_prop(MATCH_RESULT, "value", Some(STRING)),
    mem_prop(MATCH_RESULT, "groupValues", Some(LIST)),
    mem_prop(MATCH_RESULT, "groups", Some(MATCH_GROUP_COLLECTION)),
    mem_prop(MATCH_RESULT, "range", Some(INT_RANGE)),
    mem_prop(MATCH_RESULT, "destructured", Some("kotlin.text.MatchResult.Destructured")),
    mem_fn(MATCH_RESULT, "next", 0, 0, 0, Some(MATCH_RESULT)),
    // ── kotlin.sequences ──
    top_fn(KS, "sequence", 0, 1, 1, Some(SEQUENCE)),
    top_fn(KS, "generateSequence", 0, 1, 2, Some(SEQUENCE)),
    top_fn(KS, "sequenceOf", VAR, 0, V, Some(SEQUENCE)),
    top_fn(KS, "emptySequence", 0, 0, 0, Some(SEQUENCE)),
    ext_fn(KS, SEQUENCE, "map", 0, 1, 1, Some(SEQUENCE)),
    ext_fn(KS, SEQUENCE, "mapIndexed", 0, 1, 1, Some(SEQUENCE)),
    ext_fn(KS, SEQUENCE, "mapNotNull", 0, 1, 1, Some(SEQUENCE)),
    ext_fn(KS, SEQUENCE, "flatMap", 0, 1, 1, Some(SEQUENCE)),
    ext_fn(KS, SEQUENCE, "filter", 0, 1, 1, Some(SEQUENCE)),
    ext_fn(KS, SEQUENCE, "filterNot", 0, 1, 1, Some(SEQUENCE)),
    ext_fn(KS, SEQUENCE, "filterNotNull", 0, 0, 0, Some(SEQUENCE)),
    ext_fn(KS, SEQUENCE, "take", 0, 1, 1, Some(SEQUENCE)),
    ext_fn(KS, SEQUENCE, "drop", 0, 1, 1, Some(SEQUENCE)),
    ext_fn(KS, SEQUENCE, "takeWhile", 0, 1, 1, Some(SEQUENCE)),
    ext_fn(KS, SEQUENCE, "dropWhile", 0, 1, 1, Some(SEQUENCE)),
    ext_fn(KS, SEQUENCE, "distinct", 0, 0, 0, Some(SEQUENCE)),
    ext_fn(KS, SEQUENCE, "distinctBy", 0, 1, 1, Some(SEQUENCE)),
    ext_fn(KS, SEQUENCE, "sorted", 0, 0, 0, Some(SEQUENCE)),
    ext_fn(KS, SEQUENCE, "sortedBy", INL, 1, 1, Some(SEQUENCE)),
    ext_fn(KS, SEQUENCE, "onEach", 0, 1, 1, Some(SEQUENCE)),
    ext_fn(KS, SEQUENCE, "chunked", 0, 1, 2, Some(SEQUENCE)),
    ext_fn(KS, SEQUENCE, "windowed", 0, 1, 4, Some(SEQUENCE)),
    ext_fn(KS, SEQUENCE, "zip", 0, 1, 2, Some(SEQUENCE)),
    ext_fn(KS, SEQUENCE, "withIndex", 0, 0, 0, Some(SEQUENCE)),
    ext_fn(KS, SEQUENCE, "plus", OPR, 1, 1, Some(SEQUENCE)),
    ext_fn(KS, SEQUENCE, "constrainOnce", 0, 0, 0, Some(SEQUENCE)),
    ext_fn(KS, SEQUENCE, "asIterable", 0, 0, 0, Some(ITERABLE)),
    ext_fn(KS, SEQUENCE, "toList", 0, 0, 0, Some(LIST)),
    ext_fn(KS, SEQUENCE, "toMutableList", 0, 0, 0, Some(MUTABLE_LIST)),
    ext_fn(KS, SEQUENCE, "toSet", 0, 0, 0, Some(SET)),
    ext_fn(KS, SEQUENCE, "first", INL, 0, 1, None),
    ext_fn(KS, SEQUENCE, "firstOrNull", INL, 0, 1, None),
    ext_fn(KS, SEQUENCE, "last", INL, 0, 1, None),
    ext_fn(KS, SEQUENCE, "forEach", INL, 1, 1, Some(UNIT)),
    ext_fn(KS, SEQUENCE, "count", INL, 0, 1, Some(INT)),
    ext_fn(KS, SEQUENCE, "any", INL, 0, 1, Some(BOOLEAN)),
    ext_fn(KS, SEQUENCE, "all", INL, 1, 1, Some(BOOLEAN)),
    ext_fn(KS, SEQUENCE, "none", INL, 0, 1, Some(BOOLEAN)),
    ext_fn(KS, SEQUENCE, "sumOf", INL, 1, 1, None),
    ext_fn(KS, SEQUENCE, "fold", INL, 2, 2, None),
    ext_fn(KS, SEQUENCE, "reduce", INL, 1, 1, None),
    ext_fn(KS, SEQUENCE, "groupBy", INL, 1, 2, Some(MAP_T)),
    ext_fn(KS, SEQUENCE, "associateBy", INL, 1, 2, Some(MAP_T)),
    ext_fn(KS, SEQUENCE, "joinToString", 0, 0, 6, Some(STRING)),
    mem_fn(SEQUENCE, "iterator", OPR, 0, 0, Some(ITERATOR)),
    mem_fn("kotlin.sequences.SequenceScope", "yield", SUS, 1, 1, Some(UNIT)),
    mem_fn("kotlin.sequences.SequenceScope", "yieldAll", SUS, 1, 1, Some(UNIT)),
    // ── kotlin.comparisons ──
    top_fn(KCMP, "compareBy", VAR, 0, V, Some(COMPARATOR)),
    top_fn(KCMP, "compareByDescending", INL, 1, 2, Some(COMPARATOR)),
    top_fn(KCMP, "naturalOrder", 0, 0, 0, Some(COMPARATOR)),
    top_fn(KCMP, "reverseOrder", 0, 0, 0, Some(COMPARATOR)),
    top_fn(KCMP, "nullsFirst", 0, 0, 1, Some(COMPARATOR)),
    top_fn(KCMP, "nullsLast", 0, 0, 1, Some(COMPARATOR)),
    top_fn(KCMP, "maxOf", VAR, 1, V, None),
    top_fn(KCMP, "minOf", VAR, 1, V, None),
    top_fn(KCMP, "compareValues", 0, 2, 2, Some(INT)),
    top_fn(KCMP, "compareValuesBy", VAR, 2, V, Some(INT)),
    ext_fn(KCMP, COMPARATOR, "thenBy", INL, 1, 1, Some(COMPARATOR)),
    ext_fn(KCMP, COMPARATOR, "thenByDescending", INL, 1, 1, Some(COMPARATOR)),
    ext_fn(KCMP, COMPARATOR, "thenComparator", INL, 1, 1, Some(COMPARATOR)),
    ext_fn(KCMP, COMPARATOR, "then", INF, 1, 1, Some(COMPARATOR)),
    ext_fn(KCMP, COMPARATOR, "reversed", 0, 0, 0, Some(COMPARATOR)),
    // ── kotlin.properties ──
    mem_fn("kotlin.properties.Delegates", "notNull", 0, 0, 0, Some(READ_WRITE_PROPERTY)),
    mem_fn("kotlin.properties.Delegates", "observable", INL, 2, 2, Some(READ_WRITE_PROPERTY)),
    mem_fn("kotlin.properties.Delegates", "vetoable", INL, 2, 2, Some(READ_WRITE_PROPERTY)),
    mem_fn(READ_ONLY_PROPERTY, "getValue", OPR, 2, 2, None),
    mem_fn(READ_WRITE_PROPERTY, "setValue", OPR, 3, 3, Some(UNIT)),
    // ── kotlin.coroutines ──
    top_fn(KCO, "suspendCoroutine", SUS | INL, 1, 1, None),
    top_prop(KCO, "coroutineContext", SUS, Some(COROUTINE_CONTEXT)),
    ext_fn(KCO, CONTINUATION, "resume", INL, 1, 1, Some(UNIT)),
    ext_fn(KCO, CONTINUATION, "resumeWithException", INL, 1, 1, Some(UNIT)),
    mem_fn(CONTINUATION, "resumeWith", 0, 1, 1, Some(UNIT)),
    mem_prop(CONTINUATION, "context", Some(COROUTINE_CONTEXT)),
    mem_fn(COROUTINE_CONTEXT, "get", OPR, 1, 1, None),
    mem_fn(COROUTINE_CONTEXT, "plus", OPR, 1, 1, Some(COROUTINE_CONTEXT)),
    mem_fn(COROUTINE_CONTEXT, "minusKey", 0, 1, 1, Some(COROUTINE_CONTEXT)),
    mem_fn(COROUTINE_CONTEXT, "fold", 0, 2, 2, None),
    // ── kotlinx.coroutines ──
    top_fn(KX, "runBlocking", JV, 1, 2, None),
    top_fn(KX, "coroutineScope", SUS, 1, 1, None),
    top_fn(KX, "supervisorScope", SUS, 1, 1, None),
    top_fn(KX, "withContext", SUS, 2, 2, None),
    top_fn(KX, "delay", SUS, 1, 1, Some(UNIT)),
    top_fn(KX, "yield", SUS, 0, 0, Some(UNIT)),
    top_fn(KX, "withTimeout", SUS, 2, 2, None),
    top_fn(KX, "withTimeoutOrNull", SUS, 2, 2, None),
    top_fn(KX, "awaitAll", SUS | VAR, 0, V, Some(LIST)),
    top_fn(KX, "joinAll", SUS | VAR, 0, V, Some(UNIT)),
    top_fn(KX, "CoroutineScope", 0, 1, 1, Some(SCOPE)),
    top_fn(KX, "MainScope", 0, 0, 0, Some(SCOPE)),
    top_fn(KX, "SupervisorJob", 0, 0, 1, Some(COMPLETABLE_JOB)),
    top_fn(KX, "Job", 0, 0, 1, Some(COMPLETABLE_JOB)),
    top_fn(KX, "CompletableDeferred", 0, 0, 1, Some(COMPLETABLE_DEFERRED)),
    top_fn(KX, "CoroutineExceptionHandler", INL, 1, 1, Some(EXCEPTION_HANDLER)),
    top_fn(KX, "suspendCancellableCoroutine", SUS | INL, 1, 1, None),
    top_fn(KX, "currentCoroutineContext", SUS, 0, 0, Some(COROUTINE_CONTEXT)),
    ext_fn(KX, SCOPE, "launch", 0, 1, 3, Some(JOB)),
    ext_fn(KX, SCOPE, "async", 0, 1, 3, Some(DEFERRED)),
    ext_fn(KX, SCOPE, "cancel", 0, 0, 1, Some(UNIT)),
    ext_fn(KX, SCOPE, "ensureActive", 0, 0, 0, Some(UNIT)),
    ext_fn(KX, SCOPE, "plus", OPR, 1, 1, Some(SCOPE)),
    ext_prop(KX, SCOPE, "isActive", 0, Some(BOOLEAN)),
    ext_fn(KX, COLLECTION, "awaitAll", SUS, 0, 0, Some(LIST)),
    ext_fn(KX, COLLECTION, "joinAll", SUS, 0, 0, Some(UNIT)),
    ext_fn(KX, JOB, "cancelAndJoin", SUS, 0, 0, Some(UNIT)),
    ext_fn(KX, JOB, "ensureActive", 0, 0, 0, Some(UNIT)),
    ext_fn(KX, JOB, "cancelChildren", 0, 0, 1, Some(UNIT)),
    ext_fn(KX, COROUTINE_CONTEXT, "cancel", 0, 0, 1, Some(UNIT)),
    ext_fn(KX, COROUTINE_CONTEXT, "ensureActive", 0, 0, 0, Some(UNIT)),
    ext_prop(KX, COROUTINE_CONTEXT, "isActive", 0, Some(BOOLEAN)),
    ext_prop(KX, COROUTINE_CONTEXT, "job", 0, Some(JOB)),
    ext_fn(KX, "java.util.concurrent.Executor", "asCoroutineDispatcher", JV, 0, 0, Some(DISPATCHER)),
    mem_fn(JOB, "cancel", 0, 0, 1, Some(UNIT)),
    mem_fn(JOB, "join", SUS, 0, 0, Some(UNIT)),
    mem_fn(JOB, "start", 0, 0, 0, Some(BOOLEAN)),
    mem_fn(JOB, "invokeOnCompletion", 0, 1, 1, Some(DISPOSABLE_HANDLE)),
    mem_prop(JOB, "isActive", Some(BOOLEAN)),
    mem_prop(JOB, "isCompleted", Some(BOOLEAN)),
    mem_prop(JOB, "isCancelled", Some(BOOLEAN)),
    mem_prop(JOB, "children", Some(SEQUENCE)),
    mem_fn(DEFERRED, "await", SUS, 0, 0, None),
    mem_fn(DEFERRED, "getCompleted", 0, 0, 0, None),
    mem_fn(COMPLETABLE_JOB, "complete", 0, 0, 0, Some(BOOLEAN)),
    mem_fn(COMPLETABLE_JOB, "completeExceptionally", 0, 1, 1, Some(BOOLEAN)),
    mem_fn(COMPLETABLE_DEFERRED, "complete", 0, 1, 1, Some(BOOLEAN)),
    mem_fn(COMPLETABLE_DEFERRED, "completeExceptionally", 0, 1, 1, Some(BOOLEAN)),
    mem_prop(SCOPE, "coroutineContext", Some(COROUTINE_CONTEXT)),
    mem_prop("kotlinx.coroutines.Dispatchers", "Default", Some(DISPATCHER)),
    mem_prop("kotlinx.coroutines.Dispatchers", "Main", Some(MAIN_DISPATCHER)),
    mem_prop("kotlinx.coroutines.Dispatchers", "IO", Some(DISPATCHER)),
    mem_prop("kotlinx.coroutines.Dispatchers", "Unconfined", Some(DISPATCHER)),
    mem_fn(DISPOSABLE_HANDLE, "dispose", 0, 0, 0, Some(UNIT)),
    // ── kotlinx.coroutines.flow ──
    top_fn(KXF, "flow", 0, 1, 1, Some(FLOW)),
    top_fn(KXF, "flowOf", VAR, 0, V, Some(FLOW)),
    top_fn(KXF, "emptyFlow", 0, 0, 0, Some(FLOW)),
    top_fn(KXF, "channelFlow", 0, 1, 1, Some(FLOW)),
    top_fn(KXF, "callbackFlow", 0, 1, 1, Some(FLOW)),
    top_fn(KXF, "combine", VAR, 3, V, Some(FLOW)),
    top_fn(KXF, "MutableStateFlow", 0, 1, 1, Some(MUTABLE_STATE_FLOW)),
    top_fn(KXF, "MutableSharedFlow", 0, 0, 3, Some(MUTABLE_SHARED_FLOW)),
    ext_fn(KXF, ITERABLE, "asFlow", 0, 0, 0, Some(FLOW)),
    ext_fn(KXF, SEQUENCE, "asFlow", 0, 0, 0, Some(FLOW)),
    ext_fn(KXF, ARRAY, "asFlow", 0, 0, 0, Some(FLOW)),
    ext_fn(KXF, FLOW, "map", 0, 1, 1, Some(FLOW)),
    ext_fn(KXF, FLOW, "mapNotNull", 0, 1, 1, Some(FLOW)),
    ext_fn(KXF, FLOW, "mapLatest", 0, 1, 1, Some(FLOW)),
    ext_fn(KXF, FLOW, "filter", 0, 1, 1, Some(FLOW)),
    ext_fn(KXF, FLOW, "filterNot", 0, 1, 1, Some(FLOW)),
    ext_fn(KXF, FLOW, "filterNotNull", 0, 0, 0, Some(FLOW)),
    ext_fn(KXF, FLOW, "onEach", 0, 1, 1, Some(FLOW)),
    ext_fn(KXF, FLOW, "transform", 0, 1, 1, Some(FLOW)),
    ext_fn(KXF, FLOW, "take", 0, 1, 1, Some(FLOW)),
    ext_fn(KXF, FLOW, "drop", 0, 1, 1, Some(FLOW)),
    ext_fn(KXF, FLOW, "catch", 0, 1, 1, Some(FLOW)),
    ext_fn(KXF, FLOW, "onStart", 0, 1, 1, Some(FLOW)),
    ext_fn(KXF, FLOW, "onCompletion", 0, 1, 1, Some(FLOW)),
    ext_fn(KXF, FLOW, "flatMapLatest", 0, 1, 1, Some(FLOW)),
    ext_fn(KXF, FLOW, "flatMapConcat", 0, 1, 1, Some(FLOW)),
    ext_fn(KXF, FLOW, "flatMapMerge", 0, 1, 2, Some(FLOW)),
    ext_fn(KXF, FLOW, "debounce", 0, 1, 1, Some(FLOW)),
    ext_fn(KXF, FLOW, "sample", 0, 1, 1, Some(FLOW)),
    ext_fn(KXF, FLOW, "distinctUntilChanged", 0, 0, 1, Some(FLOW)),
    ext_fn(KXF, FLOW, "distinctUntilChangedBy", 0, 1, 1, Some(FLOW)),
    ext_fn(KXF, FLOW, "zip", 0, 2, 2, Some(FLOW)),
    ext_fn(KXF, FLOW, "combine", 0, 2, 2, Some(FLOW)),
    ext_fn(KXF, FLOW, "flowOn", 0, 1, 1, Some(FLOW)),
    ext_fn(KXF, FLOW, "buffer", 0, 0, 2, Some(FLOW)),
    ext_fn(KXF, FLOW, "conflate", 0, 0, 0, Some(FLOW)),
    ext_fn(KXF, FLOW, "retry", 0, 0, 2, Some(FLOW)),
    ext_fn(KXF, FLOW, "launchIn", 0, 1, 1, Some(JOB)),
    ext_fn(KXF, FLOW, "stateIn", 0, 1, 3, Some(STATE_FLOW)),
    ext_fn(KXF, FLOW, "shareIn", 0, 2, 3, Some(SHARED_FLOW)),
    ext_fn(KXF, FLOW, "collect", SUS, 0, 0, Some(UNIT)),
    ext_fn(KXF, FLOW, "collectLatest", SUS, 1, 1, Some(UNIT)),
    ext_fn(KXF, FLOW, "collectIndexed", SUS | INL, 1, 1, Some(UNIT)),
    ext_fn(KXF, FLOW, "toList", SUS, 0, 1, Some(LIST)),
    ext_fn(KXF, FLOW, "toSet", SUS, 0, 1, Some(SET)),
    ext_fn(KXF, FLOW, "first", SUS, 0, 1, None),
    ext_fn(KXF, FLOW, "firstOrNull", SUS, 0, 1, None),
    ext_fn(KXF, FLOW, "single", SUS, 0, 0, None),
    ext_fn(KXF, FLOW, "singleOrNull", SUS, 0, 0, None),
    ext_fn(KXF, FLOW, "reduce", SUS, 1, 1, None),
    ext_fn(KXF, FLOW, "fold", SUS | INL, 2, 2, None),
    ext_fn(KXF, FLOW, "count", SUS, 0, 1, Some(INT)),
    ext_fn(KXF, MUTABLE_STATE_FLOW, "update", INL, 1, 1, Some(UNIT)),
    ext_fn(KXF, MUTABLE_STATE_FLOW, "getAndUpdate", INL, 1, 1, None),
    ext_fn(KXF, MUTABLE_STATE_FLOW, "updateAndGet", INL, 1, 1, None),
    ext_fn(KXF, MUTABLE_STATE_FLOW, "asStateFlow", 0, 0, 0, Some(STATE_FLOW)),
    ext_fn(KXF, MUTABLE_SHARED_FLOW, "asSharedFlow", 0, 0, 0, Some(SHARED_FLOW)),
    mem_fn(FLOW, "collect", SUS, 1, 1, Some(UNIT)),
    mem_fn(FLOW_COLLECTOR, "emit", SUS, 1, 1, Some(UNIT)),
    mem_prop(SHARED_FLOW, "replayCache", Some(LIST)),
    mem_prop(STATE_FLOW, "value", None),
    mem_prop(MUTABLE_STATE_FLOW, "value", None),
    mem_fn(MUTABLE_STATE_FLOW, "compareAndSet", 0, 2, 2, Some(BOOLEAN)),
    mem_fn(MUTABLE_SHARED_FLOW, "emit", SUS, 1, 1, Some(UNIT)),
    mem_fn(MUTABLE_SHARED_FLOW, "tryEmit", 0, 1, 1, Some(BOOLEAN)),
    mem_prop(MUTABLE_SHARED_FLOW, "subscriptionCount", Some(STATE_FLOW)),
    // ── kotlinx.coroutines.channels ──
    top_fn(KXC, "Channel", 0, 0, 3, Some(CHANNEL)),
    ext_fn(KXC, SCOPE, "produce", 0, 1, 3, Some(RECEIVE_CHANNEL)),
    ext_fn(KXC, RECEIVE_CHANNEL, "consumeEach", SUS | INL, 1, 1, Some(UNIT)),
    ext_fn(KXC, RECEIVE_CHANNEL, "receiveAsFlow", 0, 0, 0, Some(FLOW)),
    ext_fn(KXC, RECEIVE_CHANNEL, "consumeAsFlow", 0, 0, 0, Some(FLOW)),
    ext_fn(KXC, RECEIVE_CHANNEL, "toList", SUS, 0, 0, Some(LIST)),
    mem_fn(SEND_CHANNEL, "send", SUS, 1, 1, Some(UNIT)),
    mem_fn(SEND_CHANNEL, "trySend", 0, 1, 1, Some(CHANNEL_RESULT)),
    mem_fn(SEND_CHANNEL, "close", 0, 0, 1, Some(BOOLEAN)),
    mem_prop(SEND_CHANNEL, "isClosedForSend", Some(BOOLEAN)),
    mem_fn(RECEIVE_CHANNEL, "receive", SUS, 0, 0, None),
    mem_fn(RECEIVE_CHANNEL, "tryReceive", 0, 0, 0, Some(CHANNEL_RESULT)),
    mem_fn(RECEIVE_CHANNEL, "receiveCatching", SUS, 0, 0, Some(CHANNEL_RESULT)),
    mem_fn(RECEIVE_CHANNEL, "iterator", OPR, 0, 0, Some("kotlinx.coroutines.channels.ChannelIterator")),
    mem_fn(RECEIVE_CHANNEL, "cancel", 0, 0, 1, Some(UNIT)),
    mem_prop(RECEIVE_CHANNEL, "isClosedForReceive", Some(BOOLEAN)),
    // ── kotlinx.coroutines.sync ──
    top_fn(KXS, "Mutex", 0, 0, 1, Some(MUTEX)),
    top_fn(KXS, "Semaphore", 0, 1, 2, Some(SEMAPHORE)),
    ext_fn(KXS, MUTEX, "withLock", SUS | INL, 1, 2, None),
    ext_fn(KXS, SEMAPHORE, "withPermit", SUS | INL, 1, 1, None),
    mem_fn(MUTEX, "lock", SUS, 0, 1, Some(UNIT)),
    mem_fn(MUTEX, "unlock", 0, 0, 1, Some(UNIT)),
    mem_fn(MUTEX, "tryLock", 0, 0, 1, Some(BOOLEAN)),
    mem_prop(MUTEX, "isLocked", Some(BOOLEAN)),
    mem_fn(SEMAPHORE, "acquire", SUS, 0, 0, Some(UNIT)),
    mem_fn(SEMAPHORE, "release", 0, 0, 0, Some(UNIT)),
    mem_fn(SEMAPHORE, "tryAcquire", 0, 0, 0, Some(BOOLEAN)),
    mem_prop(SEMAPHORE, "availablePermits", Some(INT)),
];

const _: () = assert!(
    KOTLIN_CALLABLES.len() <= (KOTLIN_STUB_ID_LIMIT - KOTLIN_CALLABLE_STUB_BASE_ID) as usize
);

/// Symbol id of the classifier stub at `index` of [`KOTLIN_TYPES`].
pub fn type_stub_id(index: usize) -> u32 {
    KOTLIN_BASE_STUB_SYMBOL_ID + index as u32
}

/// Symbol id of the callable stub at `index` of [`KOTLIN_CALLABLES`].
pub fn callable_stub_id(index: usize) -> u32 {
    KOTLIN_CALLABLE_STUB_BASE_ID + index as u32
}

/// True for ids in the Kotlin stub range.
pub fn is_kotlin_stub_id(symbol_id: u32) -> bool {
    (KOTLIN_BASE_STUB_SYMBOL_ID..KOTLIN_STUB_ID_LIMIT).contains(&symbol_id)
}

/// Classifier stub by qualified name (`kotlin.collections.Map.Entry`), with its symbol id.
pub fn type_stub_by_qual_name(qual_name: &str) -> Option<(u32, &'static KotlinTypeStub)> {
    static INDEX: OnceLock<HashMap<String, usize>> = OnceLock::new();
    let index = INDEX.get_or_init(|| {
        KOTLIN_TYPES
            .iter()
            .enumerate()
            .map(|(i, stub)| (stub.qual_name(), i))
            .collect()
    });
    index
        .get(qual_name)
        .map(|&i| (type_stub_id(i), &KOTLIN_TYPES[i]))
}

fn stub_record(symbol_id: u32, qual_name_id: u32, kind: SymbolKind) -> SymbolRecord {
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
        kind: kind as u8,
        visibility: SymbolVisibility::Public as u8,
        type_param_count: 0,
        flags: 0,
        first_token_id: u32::MAX,
        last_token_id: u32::MAX,
        _reserved: 0,
    }
}

/// The Kotlin library model for one platform: symbol records for every stub plus the
/// indexes the Kotlin resolvers query.
#[derive(Debug, Clone)]
pub struct KotlinStdLibManager {
    /// Qualified names, and the simple names visible through the default imports, of the
    /// classifier stubs.
    pub stubs: HashMap<String, u32>,
    /// Records of every classifier and callable stub available on the platform.
    pub stub_records: HashMap<u32, SymbolRecord>,
    platform: KotlinPlatform,
    types_by_qual: HashMap<String, usize>,
    top_level_by_qual: HashMap<String, Vec<usize>>,
    members_by_owner: HashMap<&'static str, HashMap<&'static str, Vec<usize>>>,
    extensions_by_name: HashMap<&'static str, Vec<usize>>,
    /// Simple names that two default-imported packages of the same tier declare.
    ambiguous_simple_names: HashSet<String>,
}

impl KotlinStdLibManager {
    /// The JVM library model.
    pub fn new(qual_table: &mut QualifiedNameTable) -> Self {
        Self::with_platform(qual_table, KotlinPlatform::Jvm)
    }

    pub fn with_platform(qual_table: &mut QualifiedNameTable, platform: KotlinPlatform) -> Self {
        let on_platform = |flags: u16, jvm_bit: u16| platform == KotlinPlatform::Jvm || flags & jvm_bit == 0;

        let mut stubs = HashMap::new();
        let mut stub_records = HashMap::new();
        let mut types_by_qual = HashMap::new();

        for (i, stub) in KOTLIN_TYPES.iter().enumerate() {
            if !on_platform(stub.flags, KotlinTypeFlags::JVM_ONLY) {
                continue;
            }
            let qual = stub.qual_name();
            let symbol_id = type_stub_id(i);
            let mut record = stub_record(symbol_id, qual_table.get_or_intern(&qual), stub.kind);
            record.type_param_count = stub.type_params;
            if stub.kind == SymbolKind::SK_INTERFACE {
                record.modifiers |= SymbolModifiers::ABSTRACT;
            }
            stub_records.insert(symbol_id, record);
            stubs.insert(qual.clone(), symbol_id);
            types_by_qual.insert(qual, i);
        }

        // Simple names through the default imports: an earlier tier hides a later one; two
        // declarations in the same tier make the simple name ambiguous.
        let mut ambiguous_simple_names = HashSet::new();
        for tier in default_import_tiers(platform) {
            let mut tier_names: HashMap<&'static str, Vec<u32>> = HashMap::new();
            for (i, stub) in KOTLIN_TYPES.iter().enumerate() {
                if tier.contains(&stub.package) && on_platform(stub.flags, KotlinTypeFlags::JVM_ONLY) {
                    tier_names.entry(stub.name).or_default().push(type_stub_id(i));
                }
            }
            for (name, ids) in tier_names {
                if stubs.contains_key(name) || ambiguous_simple_names.contains(name) {
                    continue;
                }
                if ids.len() == 1 {
                    stubs.insert(name.to_string(), ids[0]);
                } else {
                    ambiguous_simple_names.insert(name.to_string());
                }
            }
        }

        let mut top_level_by_qual: HashMap<String, Vec<usize>> = HashMap::new();
        let mut members_by_owner: HashMap<&'static str, HashMap<&'static str, Vec<usize>>> =
            HashMap::new();
        let mut extensions_by_name: HashMap<&'static str, Vec<usize>> = HashMap::new();

        for (i, stub) in KOTLIN_CALLABLES.iter().enumerate() {
            if !on_platform(stub.flags, KotlinCallableFlags::JVM_ONLY) {
                continue;
            }
            let qual = stub.qual_name();
            let symbol_id = callable_stub_id(i);
            let kind = match stub.kind {
                KotlinCallableKind::Function => SymbolKind::SK_METHOD,
                KotlinCallableKind::Property => SymbolKind::SK_FIELD,
            };
            let mut record = stub_record(symbol_id, qual_table.get_or_intern(&qual), kind);
            record.param_count = u16::from(stub.min_args);
            if stub.flags & KotlinCallableFlags::VARARG != 0 {
                record.modifiers |= SymbolModifiers::VARARGS;
            }
            match stub.owner {
                Some(owner) => {
                    if let Some(&owner_index) = types_by_qual.get(owner) {
                        record.parent_sym = type_stub_id(owner_index);
                    }
                    members_by_owner
                        .entry(owner)
                        .or_default()
                        .entry(stub.name)
                        .or_default()
                        .push(i);
                }
                None => {
                    // Top-level functions and extensions compile to static methods of the
                    // file facade class and are dispatched statically.
                    record.modifiers |= SymbolModifiers::STATIC;
                    if stub.receiver.is_some() {
                        extensions_by_name.entry(stub.name).or_default().push(i);
                    } else {
                        top_level_by_qual.entry(qual).or_default().push(i);
                    }
                }
            }
            stub_records.insert(symbol_id, record);
        }

        Self {
            stubs,
            stub_records,
            platform,
            types_by_qual,
            top_level_by_qual,
            members_by_owner,
            extensions_by_name,
            ambiguous_simple_names,
        }
    }

    pub fn platform(&self) -> KotlinPlatform {
        self.platform
    }

    /// Classifier stub id by qualified name, or by a simple name the default imports make
    /// visible (`String`, `List`, `Regex`). Names of other packages (`Job`, `Flow`) need an
    /// import and are found only by qualified name.
    pub fn lookup(&self, name: &str) -> Option<u32> {
        self.stubs.get(name).copied()
    }

    /// True when two default-imported packages of the same tier declare `simple_name`.
    pub fn is_ambiguous_default_name(&self, simple_name: &str) -> bool {
        self.ambiguous_simple_names.contains(simple_name)
    }

    /// Classifier stub of a symbol id.
    pub fn type_stub(&self, symbol_id: u32) -> Option<&'static KotlinTypeStub> {
        let index = symbol_id.checked_sub(KOTLIN_BASE_STUB_SYMBOL_ID)? as usize;
        if symbol_id >= KOTLIN_CALLABLE_STUB_BASE_ID || !self.stub_records.contains_key(&symbol_id) {
            return None;
        }
        KOTLIN_TYPES.get(index)
    }

    /// Callable stub of a symbol id.
    pub fn callable_stub(&self, symbol_id: u32) -> Option<&'static KotlinCallableStub> {
        let index = symbol_id.checked_sub(KOTLIN_CALLABLE_STUB_BASE_ID)? as usize;
        if !self.stub_records.contains_key(&symbol_id) {
            return None;
        }
        KOTLIN_CALLABLES.get(index)
    }

    /// Classifier stub by qualified name, when available on this platform.
    pub fn type_by_qual_name(&self, qual_name: &str) -> Option<(u32, &'static KotlinTypeStub)> {
        self.types_by_qual
            .get(qual_name)
            .map(|&i| (type_stub_id(i), &KOTLIN_TYPES[i]))
    }

    /// Top-level (non-extension) functions and properties with this qualified name.
    pub fn top_level_callables(&self, qual_name: &str) -> Vec<(u32, &'static KotlinCallableStub)> {
        self.top_level_by_qual
            .get(qual_name)
            .map(|v| v.iter().map(|&i| (callable_stub_id(i), &KOTLIN_CALLABLES[i])).collect())
            .unwrap_or_default()
    }

    /// Members named `name` declared in `owner`.
    pub fn member_callables(&self, owner: &str, name: &str) -> Vec<(u32, &'static KotlinCallableStub)> {
        self.members_by_owner
            .get(owner)
            .and_then(|names| names.get(name))
            .map(|v| v.iter().map(|&i| (callable_stub_id(i), &KOTLIN_CALLABLES[i])).collect())
            .unwrap_or_default()
    }

    /// Extension functions and properties named `name`, on any receiver.
    pub fn extension_callables(&self, name: &str) -> Vec<(u32, &'static KotlinCallableStub)> {
        self.extensions_by_name
            .get(name)
            .map(|v| v.iter().map(|&i| (callable_stub_id(i), &KOTLIN_CALLABLES[i])).collect())
            .unwrap_or_default()
    }

    /// Stub id of the callable an extension resolution chose (same package, name and
    /// receiver), when it is a library extension.
    pub fn extension_stub_id(&self, extension: &KotlinExtension) -> Option<u32> {
        self.extension_callables(&extension.name)
            .into_iter()
            .find(|(_, stub)| {
                stub.package == extension.package && stub.receiver == Some(extension.receiver.as_str())
            })
            .map(|(id, _)| id)
    }
}

impl KotlinDeclarationIndex for KotlinStdLibManager {
    fn classifier_kind(&self, qual_name: &str) -> Option<KotlinClassifierKind> {
        self.type_by_qual_name(qual_name).map(|(_, stub)| stub.classifier_kind())
    }

    fn has_callable(&self, qual_name: &str) -> bool {
        self.top_level_by_qual.contains_key(qual_name)
    }

    fn members_named(&self, owner: &str, name: &str) -> Vec<KotlinMember> {
        self.member_callables(owner, name)
            .into_iter()
            .map(|(_, stub)| KotlinMember {
                owner: owner.to_string(),
                name: stub.name.to_string(),
                kind: stub.member_kind(),
            })
            .collect()
    }

    fn supertypes(&self, classifier: &str) -> Vec<String> {
        self.type_by_qual_name(classifier)
            .map(|(_, stub)| stub.supertypes.iter().map(|s| s.to_string()).collect())
            .unwrap_or_default()
    }

    fn companion_object(&self, classifier: &str) -> Option<String> {
        let companion = format!("{}.Companion", classifier);
        self.type_by_qual_name(&companion)
            .filter(|(_, stub)| stub.flags & KotlinTypeFlags::COMPANION != 0)
            .map(|_| companion)
    }

    fn extensions_named(&self, name: &str) -> Vec<KotlinExtension> {
        self.extension_callables(name)
            .into_iter()
            .filter_map(|(_, stub)| {
                stub.receiver.map(|receiver| KotlinExtension {
                    package: stub.package.to_string(),
                    name: stub.name.to_string(),
                    receiver: receiver.to_string(),
                    receiver_nullable: stub.flags & KotlinCallableFlags::NULLABLE_RECEIVER != 0,
                    kind: stub.member_kind(),
                })
            })
            .collect()
    }

    fn typealias_target(&self, qual_name: &str) -> Option<String> {
        self.type_by_qual_name(qual_name)
            .filter(|(_, stub)| stub.flags & KotlinTypeFlags::TYPEALIAS != 0)
            .and_then(|(_, stub)| stub.jvm_peer.map(|p| p.to_string()))
    }
}
