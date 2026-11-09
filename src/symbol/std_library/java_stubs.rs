//! Built-in library catalog: the JDK and Kotlin standard-library types that Phase 3 can bind
//! without a classpath.
//!
//! `java.lang` is listed completely (public top-level types of JDK 21), which makes the
//! implicit `import java.lang.*` exact: a simple name that is not in this list is never bound
//! to `java.lang`. Other packages are listed partially; a name the catalog does not know is
//! resolved through the import declarations instead (see `NameResolver`). Every entry records
//! its kind, generic arity, direct supertypes and the category used by association inference.
//! Members are not modelled; binding library members requires a classpath reader.

use crate::core::types::symbol::SymbolKind;
use crate::ingestion::interner::StringInterner;
use crate::symbol::model::Dialect;
use std::collections::HashMap;

/// How a library type participates in UML association inference.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LibraryCategory {
    /// Value types rendered as attributes, never as associations.
    Value = 0,
    /// Iterable/collection-like: elements are type argument 0 (multiplicity 0..*).
    Collection = 1,
    /// Map-like: values are type argument 1 (multiplicity 0..*).
    Map = 2,
    /// Optional-like: type argument 0 with multiplicity 0..1.
    Optional = 3,
    /// Any other library type.
    Other = 4,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LibraryType {
    pub fqn: &'static str,
    pub kind: SymbolKind,
    pub type_params: u8,
    pub category: LibraryCategory,
    /// Direct supertypes by qualified name (implicit `Object` omitted).
    pub supertypes: &'static [&'static str],
}

impl LibraryType {
    pub fn simple_name(&self) -> &'static str {
        let fqn: &'static str = self.fqn;
        fqn.rsplit('.').next().unwrap_or(fqn)
    }

    /// Package part of the qualified name (segments before the first capitalized one).
    pub fn package(&self) -> &'static str {
        let fqn: &'static str = self.fqn;
        let mut end = 0usize;
        let mut pos = 0usize;
        for seg in fqn.split('.') {
            if seg.chars().next().is_some_and(|c| c.is_uppercase()) {
                break;
            }
            end = pos + seg.len();
            pos = end + 1;
        }
        &fqn[..end]
    }

    pub fn is_interface(&self) -> bool {
        matches!(
            self.kind,
            SymbolKind::SK_INTERFACE | SymbolKind::SK_ANNOTATION_TYPE
        )
    }
}

const C: SymbolKind = SymbolKind::SK_CLASS;
const I: SymbolKind = SymbolKind::SK_INTERFACE;
const E: SymbolKind = SymbolKind::SK_ENUM;
const A: SymbolKind = SymbolKind::SK_ANNOTATION_TYPE;

const VAL: LibraryCategory = LibraryCategory::Value;
const COL: LibraryCategory = LibraryCategory::Collection;
const MAP: LibraryCategory = LibraryCategory::Map;
const OPT: LibraryCategory = LibraryCategory::Optional;
const OTH: LibraryCategory = LibraryCategory::Other;

const fn lib(
    fqn: &'static str,
    kind: SymbolKind,
    type_params: u8,
    category: LibraryCategory,
    supertypes: &'static [&'static str],
) -> LibraryType {
    LibraryType {
        fqn,
        kind,
        type_params,
        category,
        supertypes,
    }
}

const EXC: &[&str] = &["java.lang.Exception"];
const RTE: &[&str] = &["java.lang.RuntimeException"];
const ERR: &[&str] = &["java.lang.Error"];
const LINK: &[&str] = &["java.lang.LinkageError"];
const ICCE: &[&str] = &["java.lang.IncompatibleClassChangeError"];
const VME: &[&str] = &["java.lang.VirtualMachineError"];
const ROE: &[&str] = &["java.lang.ReflectiveOperationException"];
const IAE: &[&str] = &["java.lang.IllegalArgumentException"];
const IOOBE: &[&str] = &["java.lang.IndexOutOfBoundsException"];
const NUM: &[&str] = &["java.lang.Number", "java.lang.Comparable"];
const IOE: &[&str] = &["java.io.IOException"];

static CATALOG: &[LibraryType] = &[
    // ── java.lang (complete) ──
    lib("java.lang.Object", C, 0, OTH, &[]),
    lib("java.lang.String", C, 0, VAL, &["java.lang.CharSequence", "java.lang.Comparable", "java.io.Serializable"]),
    lib("java.lang.CharSequence", I, 0, VAL, &[]),
    lib("java.lang.Comparable", I, 1, OTH, &[]),
    lib("java.lang.Iterable", I, 1, COL, &[]),
    lib("java.lang.AutoCloseable", I, 0, OTH, &[]),
    lib("java.lang.Cloneable", I, 0, OTH, &[]),
    lib("java.lang.Runnable", I, 0, OTH, &[]),
    lib("java.lang.Appendable", I, 0, OTH, &[]),
    lib("java.lang.Readable", I, 0, OTH, &[]),
    lib("java.lang.ProcessHandle", I, 0, OTH, &["java.lang.Comparable"]),
    lib("java.lang.Boolean", C, 0, VAL, &["java.lang.Comparable", "java.io.Serializable"]),
    lib("java.lang.Character", C, 0, VAL, &["java.lang.Comparable", "java.io.Serializable"]),
    lib("java.lang.Number", C, 0, VAL, &["java.io.Serializable"]),
    lib("java.lang.Byte", C, 0, VAL, NUM),
    lib("java.lang.Short", C, 0, VAL, NUM),
    lib("java.lang.Integer", C, 0, VAL, NUM),
    lib("java.lang.Long", C, 0, VAL, NUM),
    lib("java.lang.Float", C, 0, VAL, NUM),
    lib("java.lang.Double", C, 0, VAL, NUM),
    lib("java.lang.Void", C, 0, VAL, &[]),
    lib("java.lang.Math", C, 0, OTH, &[]),
    lib("java.lang.StrictMath", C, 0, OTH, &[]),
    lib("java.lang.System", C, 0, OTH, &[]),
    lib("java.lang.Runtime", C, 0, OTH, &[]),
    lib("java.lang.RuntimePermission", C, 0, OTH, &[]),
    lib("java.lang.SecurityManager", C, 0, OTH, &[]),
    lib("java.lang.Thread", C, 0, OTH, &["java.lang.Runnable"]),
    lib("java.lang.ThreadGroup", C, 0, OTH, &[]),
    lib("java.lang.ThreadLocal", C, 1, OTH, &[]),
    lib("java.lang.InheritableThreadLocal", C, 1, OTH, &["java.lang.ThreadLocal"]),
    lib("java.lang.ScopedValue", C, 1, OTH, &[]),
    lib("java.lang.Class", C, 1, OTH, &[]),
    lib("java.lang.ClassLoader", C, 0, OTH, &[]),
    lib("java.lang.ClassValue", C, 1, OTH, &[]),
    lib("java.lang.Enum", C, 1, OTH, &["java.lang.Comparable", "java.io.Serializable"]),
    lib("java.lang.Record", C, 0, OTH, &[]),
    lib("java.lang.Module", C, 0, OTH, &[]),
    lib("java.lang.ModuleLayer", C, 0, OTH, &[]),
    lib("java.lang.Package", C, 0, OTH, &[]),
    lib("java.lang.Process", C, 0, OTH, &[]),
    lib("java.lang.ProcessBuilder", C, 0, OTH, &[]),
    lib("java.lang.StackTraceElement", C, 0, VAL, &["java.io.Serializable"]),
    lib("java.lang.StackWalker", C, 0, OTH, &[]),
    lib("java.lang.StringBuilder", C, 0, OTH, &["java.lang.CharSequence", "java.lang.Appendable"]),
    lib("java.lang.StringBuffer", C, 0, OTH, &["java.lang.CharSequence", "java.lang.Appendable"]),
    lib("java.lang.Throwable", C, 0, OTH, &["java.io.Serializable"]),
    lib("java.lang.Exception", C, 0, OTH, &["java.lang.Throwable"]),
    lib("java.lang.RuntimeException", C, 0, OTH, EXC),
    lib("java.lang.Error", C, 0, OTH, &["java.lang.Throwable"]),
    lib("java.lang.ArithmeticException", C, 0, OTH, RTE),
    lib("java.lang.ArrayStoreException", C, 0, OTH, RTE),
    lib("java.lang.ClassCastException", C, 0, OTH, RTE),
    lib("java.lang.EnumConstantNotPresentException", C, 0, OTH, RTE),
    lib("java.lang.IllegalArgumentException", C, 0, OTH, RTE),
    lib("java.lang.IllegalCallerException", C, 0, OTH, RTE),
    lib("java.lang.IllegalMonitorStateException", C, 0, OTH, RTE),
    lib("java.lang.IllegalStateException", C, 0, OTH, RTE),
    lib("java.lang.IllegalThreadStateException", C, 0, OTH, IAE),
    lib("java.lang.IndexOutOfBoundsException", C, 0, OTH, RTE),
    lib("java.lang.ArrayIndexOutOfBoundsException", C, 0, OTH, IOOBE),
    lib("java.lang.StringIndexOutOfBoundsException", C, 0, OTH, IOOBE),
    lib("java.lang.LayerInstantiationException", C, 0, OTH, RTE),
    lib("java.lang.MatchException", C, 0, OTH, RTE),
    lib("java.lang.NegativeArraySizeException", C, 0, OTH, RTE),
    lib("java.lang.NullPointerException", C, 0, OTH, RTE),
    lib("java.lang.NumberFormatException", C, 0, OTH, IAE),
    lib("java.lang.SecurityException", C, 0, OTH, RTE),
    lib("java.lang.TypeNotPresentException", C, 0, OTH, RTE),
    lib("java.lang.UnsupportedOperationException", C, 0, OTH, RTE),
    lib("java.lang.WrongThreadException", C, 0, OTH, RTE),
    lib("java.lang.CloneNotSupportedException", C, 0, OTH, EXC),
    lib("java.lang.InterruptedException", C, 0, OTH, EXC),
    lib("java.lang.ReflectiveOperationException", C, 0, OTH, EXC),
    lib("java.lang.ClassNotFoundException", C, 0, OTH, ROE),
    lib("java.lang.IllegalAccessException", C, 0, OTH, ROE),
    lib("java.lang.InstantiationException", C, 0, OTH, ROE),
    lib("java.lang.NoSuchFieldException", C, 0, OTH, ROE),
    lib("java.lang.NoSuchMethodException", C, 0, OTH, ROE),
    lib("java.lang.AssertionError", C, 0, OTH, ERR),
    lib("java.lang.LinkageError", C, 0, OTH, ERR),
    lib("java.lang.ThreadDeath", C, 0, OTH, ERR),
    lib("java.lang.VirtualMachineError", C, 0, OTH, ERR),
    lib("java.lang.InternalError", C, 0, OTH, VME),
    lib("java.lang.OutOfMemoryError", C, 0, OTH, VME),
    lib("java.lang.StackOverflowError", C, 0, OTH, VME),
    lib("java.lang.UnknownError", C, 0, OTH, VME),
    lib("java.lang.BootstrapMethodError", C, 0, OTH, LINK),
    lib("java.lang.ClassCircularityError", C, 0, OTH, LINK),
    lib("java.lang.ClassFormatError", C, 0, OTH, LINK),
    lib("java.lang.UnsupportedClassVersionError", C, 0, OTH, &["java.lang.ClassFormatError"]),
    lib("java.lang.ExceptionInInitializerError", C, 0, OTH, LINK),
    lib("java.lang.IncompatibleClassChangeError", C, 0, OTH, LINK),
    lib("java.lang.AbstractMethodError", C, 0, OTH, ICCE),
    lib("java.lang.IllegalAccessError", C, 0, OTH, ICCE),
    lib("java.lang.InstantiationError", C, 0, OTH, ICCE),
    lib("java.lang.NoSuchFieldError", C, 0, OTH, ICCE),
    lib("java.lang.NoSuchMethodError", C, 0, OTH, ICCE),
    lib("java.lang.NoClassDefFoundError", C, 0, OTH, LINK),
    lib("java.lang.UnsatisfiedLinkError", C, 0, OTH, LINK),
    lib("java.lang.VerifyError", C, 0, OTH, LINK),
    lib("java.lang.Deprecated", A, 0, OTH, &[]),
    lib("java.lang.FunctionalInterface", A, 0, OTH, &[]),
    lib("java.lang.Override", A, 0, OTH, &[]),
    lib("java.lang.SafeVarargs", A, 0, OTH, &[]),
    lib("java.lang.SuppressWarnings", A, 0, OTH, &[]),
    // ── java.lang.annotation / java.lang.reflect ──
    lib("java.lang.annotation.Annotation", I, 0, OTH, &[]),
    lib("java.lang.annotation.Documented", A, 0, OTH, &[]),
    lib("java.lang.annotation.Inherited", A, 0, OTH, &[]),
    lib("java.lang.annotation.Repeatable", A, 0, OTH, &[]),
    lib("java.lang.annotation.Retention", A, 0, OTH, &[]),
    lib("java.lang.annotation.Target", A, 0, OTH, &[]),
    lib("java.lang.annotation.ElementType", E, 0, VAL, &[]),
    lib("java.lang.annotation.RetentionPolicy", E, 0, VAL, &[]),
    lib("java.lang.reflect.Method", C, 0, OTH, &[]),
    lib("java.lang.reflect.Field", C, 0, OTH, &[]),
    lib("java.lang.reflect.Constructor", C, 1, OTH, &[]),
    lib("java.lang.reflect.InvocationHandler", I, 0, OTH, &[]),
    lib("java.lang.reflect.InvocationTargetException", C, 0, OTH, ROE),
    lib("java.lang.reflect.Proxy", C, 0, OTH, &[]),
    // ── java.io ──
    lib("java.io.Serializable", I, 0, OTH, &[]),
    lib("java.io.Closeable", I, 0, OTH, &["java.lang.AutoCloseable"]),
    lib("java.io.Flushable", I, 0, OTH, &[]),
    lib("java.io.File", C, 0, VAL, &["java.io.Serializable", "java.lang.Comparable"]),
    lib("java.io.Console", C, 0, OTH, &["java.io.Flushable"]),
    lib("java.io.InputStream", C, 0, OTH, &["java.io.Closeable"]),
    lib("java.io.OutputStream", C, 0, OTH, &["java.io.Closeable", "java.io.Flushable"]),
    lib("java.io.FileInputStream", C, 0, OTH, &["java.io.InputStream"]),
    lib("java.io.FileOutputStream", C, 0, OTH, &["java.io.OutputStream"]),
    lib("java.io.ByteArrayInputStream", C, 0, OTH, &["java.io.InputStream"]),
    lib("java.io.ByteArrayOutputStream", C, 0, OTH, &["java.io.OutputStream"]),
    lib("java.io.ObjectInputStream", C, 0, OTH, &["java.io.InputStream"]),
    lib("java.io.ObjectOutputStream", C, 0, OTH, &["java.io.OutputStream"]),
    lib("java.io.PrintStream", C, 0, OTH, &["java.io.OutputStream", "java.lang.Appendable"]),
    lib("java.io.Reader", C, 0, OTH, &["java.lang.Readable", "java.io.Closeable"]),
    lib("java.io.Writer", C, 0, OTH, &["java.lang.Appendable", "java.io.Closeable", "java.io.Flushable"]),
    lib("java.io.BufferedReader", C, 0, OTH, &["java.io.Reader"]),
    lib("java.io.BufferedWriter", C, 0, OTH, &["java.io.Writer"]),
    lib("java.io.InputStreamReader", C, 0, OTH, &["java.io.Reader"]),
    lib("java.io.OutputStreamWriter", C, 0, OTH, &["java.io.Writer"]),
    lib("java.io.FileReader", C, 0, OTH, &["java.io.InputStreamReader"]),
    lib("java.io.FileWriter", C, 0, OTH, &["java.io.OutputStreamWriter"]),
    lib("java.io.PrintWriter", C, 0, OTH, &["java.io.Writer"]),
    lib("java.io.StringReader", C, 0, OTH, &["java.io.Reader"]),
    lib("java.io.StringWriter", C, 0, OTH, &["java.io.Writer"]),
    lib("java.io.IOException", C, 0, OTH, EXC),
    lib("java.io.FileNotFoundException", C, 0, OTH, IOE),
    lib("java.io.EOFException", C, 0, OTH, IOE),
    lib("java.io.UncheckedIOException", C, 0, OTH, RTE),
    // ── java.util ──
    lib("java.util.Collection", I, 1, COL, &["java.lang.Iterable"]),
    lib("java.util.SequencedCollection", I, 1, COL, &["java.util.Collection"]),
    lib("java.util.List", I, 1, COL, &["java.util.SequencedCollection"]),
    lib("java.util.Set", I, 1, COL, &["java.util.Collection"]),
    lib("java.util.SortedSet", I, 1, COL, &["java.util.Set"]),
    lib("java.util.NavigableSet", I, 1, COL, &["java.util.SortedSet"]),
    lib("java.util.Queue", I, 1, COL, &["java.util.Collection"]),
    lib("java.util.Deque", I, 1, COL, &["java.util.Queue", "java.util.SequencedCollection"]),
    lib("java.util.ArrayList", C, 1, COL, &["java.util.List"]),
    lib("java.util.LinkedList", C, 1, COL, &["java.util.List", "java.util.Deque"]),
    lib("java.util.Vector", C, 1, COL, &["java.util.List"]),
    lib("java.util.Stack", C, 1, COL, &["java.util.Vector"]),
    lib("java.util.HashSet", C, 1, COL, &["java.util.Set"]),
    lib("java.util.LinkedHashSet", C, 1, COL, &["java.util.HashSet"]),
    lib("java.util.TreeSet", C, 1, COL, &["java.util.NavigableSet"]),
    lib("java.util.EnumSet", C, 1, COL, &["java.util.Set"]),
    lib("java.util.ArrayDeque", C, 1, COL, &["java.util.Deque"]),
    lib("java.util.PriorityQueue", C, 1, COL, &["java.util.Queue"]),
    lib("java.util.Map", I, 2, MAP, &[]),
    lib("java.util.Map.Entry", I, 2, OTH, &[]),
    lib("java.util.SequencedMap", I, 2, MAP, &["java.util.Map"]),
    lib("java.util.SortedMap", I, 2, MAP, &["java.util.SequencedMap"]),
    lib("java.util.NavigableMap", I, 2, MAP, &["java.util.SortedMap"]),
    lib("java.util.HashMap", C, 2, MAP, &["java.util.Map"]),
    lib("java.util.LinkedHashMap", C, 2, MAP, &["java.util.HashMap", "java.util.SequencedMap"]),
    lib("java.util.TreeMap", C, 2, MAP, &["java.util.NavigableMap"]),
    lib("java.util.Hashtable", C, 2, MAP, &["java.util.Map"]),
    lib("java.util.WeakHashMap", C, 2, MAP, &["java.util.Map"]),
    lib("java.util.IdentityHashMap", C, 2, MAP, &["java.util.Map"]),
    lib("java.util.EnumMap", C, 2, MAP, &["java.util.Map"]),
    lib("java.util.Properties", C, 0, OTH, &["java.util.Hashtable"]),
    lib("java.util.Iterator", I, 1, OTH, &[]),
    lib("java.util.ListIterator", I, 1, OTH, &["java.util.Iterator"]),
    lib("java.util.Spliterator", I, 1, OTH, &[]),
    lib("java.util.Comparator", I, 1, OTH, &[]),
    lib("java.util.Optional", C, 1, OPT, &[]),
    lib("java.util.OptionalInt", C, 0, VAL, &[]),
    lib("java.util.OptionalLong", C, 0, VAL, &[]),
    lib("java.util.OptionalDouble", C, 0, VAL, &[]),
    lib("java.util.Collections", C, 0, OTH, &[]),
    lib("java.util.Arrays", C, 0, OTH, &[]),
    lib("java.util.Objects", C, 0, OTH, &[]),
    lib("java.util.UUID", C, 0, VAL, &["java.lang.Comparable", "java.io.Serializable"]),
    lib("java.util.Date", C, 0, VAL, &["java.lang.Comparable", "java.io.Serializable"]),
    lib("java.util.Calendar", C, 0, VAL, &["java.lang.Comparable", "java.io.Serializable"]),
    lib("java.util.GregorianCalendar", C, 0, VAL, &["java.util.Calendar"]),
    lib("java.util.TimeZone", C, 0, VAL, &["java.io.Serializable"]),
    lib("java.util.Locale", C, 0, VAL, &["java.io.Serializable"]),
    lib("java.util.Currency", C, 0, VAL, &["java.io.Serializable"]),
    lib("java.util.Random", C, 0, OTH, &[]),
    lib("java.util.Scanner", C, 0, OTH, &["java.util.Iterator", "java.io.Closeable"]),
    lib("java.util.StringJoiner", C, 0, OTH, &[]),
    lib("java.util.StringTokenizer", C, 0, OTH, &[]),
    lib("java.util.BitSet", C, 0, VAL, &[]),
    lib("java.util.Base64", C, 0, OTH, &[]),
    lib("java.util.Timer", C, 0, OTH, &[]),
    lib("java.util.TimerTask", C, 0, OTH, &["java.lang.Runnable"]),
    lib("java.util.Formatter", C, 0, OTH, &["java.io.Closeable", "java.io.Flushable"]),
    lib("java.util.NoSuchElementException", C, 0, OTH, RTE),
    lib("java.util.ConcurrentModificationException", C, 0, OTH, RTE),
    // ── java.util.concurrent ──
    lib("java.util.concurrent.ConcurrentMap", I, 2, MAP, &["java.util.Map"]),
    lib("java.util.concurrent.ConcurrentHashMap", C, 2, MAP, &["java.util.concurrent.ConcurrentMap"]),
    lib("java.util.concurrent.CopyOnWriteArrayList", C, 1, COL, &["java.util.List"]),
    lib("java.util.concurrent.CopyOnWriteArraySet", C, 1, COL, &["java.util.Set"]),
    lib("java.util.concurrent.BlockingQueue", I, 1, COL, &["java.util.Queue"]),
    lib("java.util.concurrent.LinkedBlockingQueue", C, 1, COL, &["java.util.concurrent.BlockingQueue"]),
    lib("java.util.concurrent.ArrayBlockingQueue", C, 1, COL, &["java.util.concurrent.BlockingQueue"]),
    lib("java.util.concurrent.ConcurrentLinkedQueue", C, 1, COL, &["java.util.Queue"]),
    lib("java.util.concurrent.Executor", I, 0, OTH, &[]),
    lib("java.util.concurrent.ExecutorService", I, 0, OTH, &["java.util.concurrent.Executor", "java.lang.AutoCloseable"]),
    lib("java.util.concurrent.ScheduledExecutorService", I, 0, OTH, &["java.util.concurrent.ExecutorService"]),
    lib("java.util.concurrent.Executors", C, 0, OTH, &[]),
    lib("java.util.concurrent.Callable", I, 1, OTH, &[]),
    lib("java.util.concurrent.Future", I, 1, OTH, &[]),
    lib("java.util.concurrent.ScheduledFuture", I, 1, OTH, &["java.util.concurrent.Future"]),
    lib("java.util.concurrent.CompletionStage", I, 1, OTH, &[]),
    lib("java.util.concurrent.CompletableFuture", C, 1, OTH, &["java.util.concurrent.Future", "java.util.concurrent.CompletionStage"]),
    lib("java.util.concurrent.TimeUnit", E, 0, VAL, &[]),
    lib("java.util.concurrent.CountDownLatch", C, 0, OTH, &[]),
    lib("java.util.concurrent.Semaphore", C, 0, OTH, &[]),
    lib("java.util.concurrent.ForkJoinPool", C, 0, OTH, &["java.util.concurrent.ExecutorService"]),
    lib("java.util.concurrent.ThreadLocalRandom", C, 0, OTH, &["java.util.Random"]),
    lib("java.util.concurrent.ExecutionException", C, 0, OTH, EXC),
    lib("java.util.concurrent.TimeoutException", C, 0, OTH, EXC),
    lib("java.util.concurrent.atomic.AtomicBoolean", C, 0, VAL, &[]),
    lib("java.util.concurrent.atomic.AtomicInteger", C, 0, VAL, &["java.lang.Number"]),
    lib("java.util.concurrent.atomic.AtomicLong", C, 0, VAL, &["java.lang.Number"]),
    lib("java.util.concurrent.atomic.AtomicReference", C, 1, OTH, &[]),
    lib("java.util.concurrent.locks.Lock", I, 0, OTH, &[]),
    lib("java.util.concurrent.locks.ReadWriteLock", I, 0, OTH, &[]),
    lib("java.util.concurrent.locks.ReentrantLock", C, 0, OTH, &["java.util.concurrent.locks.Lock"]),
    lib("java.util.concurrent.locks.ReentrantReadWriteLock", C, 0, OTH, &["java.util.concurrent.locks.ReadWriteLock"]),
    // ── java.util.function / stream / regex / logging ──
    lib("java.util.function.Function", I, 2, OTH, &[]),
    lib("java.util.function.BiFunction", I, 3, OTH, &[]),
    lib("java.util.function.UnaryOperator", I, 1, OTH, &["java.util.function.Function"]),
    lib("java.util.function.BinaryOperator", I, 1, OTH, &["java.util.function.BiFunction"]),
    lib("java.util.function.Supplier", I, 1, OTH, &[]),
    lib("java.util.function.Consumer", I, 1, OTH, &[]),
    lib("java.util.function.BiConsumer", I, 2, OTH, &[]),
    lib("java.util.function.Predicate", I, 1, OTH, &[]),
    lib("java.util.function.BiPredicate", I, 2, OTH, &[]),
    lib("java.util.function.IntFunction", I, 1, OTH, &[]),
    lib("java.util.function.ToIntFunction", I, 1, OTH, &[]),
    lib("java.util.function.IntPredicate", I, 0, OTH, &[]),
    lib("java.util.function.IntUnaryOperator", I, 0, OTH, &[]),
    lib("java.util.function.IntBinaryOperator", I, 0, OTH, &[]),
    lib("java.util.function.BooleanSupplier", I, 0, OTH, &[]),
    lib("java.util.function.IntSupplier", I, 0, OTH, &[]),
    lib("java.util.function.LongSupplier", I, 0, OTH, &[]),
    lib("java.util.function.DoubleSupplier", I, 0, OTH, &[]),
    lib("java.util.stream.BaseStream", I, 2, OTH, &["java.lang.AutoCloseable"]),
    lib("java.util.stream.Stream", I, 1, OTH, &["java.util.stream.BaseStream"]),
    lib("java.util.stream.IntStream", I, 0, OTH, &["java.util.stream.BaseStream"]),
    lib("java.util.stream.LongStream", I, 0, OTH, &["java.util.stream.BaseStream"]),
    lib("java.util.stream.DoubleStream", I, 0, OTH, &["java.util.stream.BaseStream"]),
    lib("java.util.stream.Collector", I, 3, OTH, &[]),
    lib("java.util.stream.Collectors", C, 0, OTH, &[]),
    lib("java.util.regex.Pattern", C, 0, VAL, &["java.io.Serializable"]),
    lib("java.util.regex.Matcher", C, 0, OTH, &[]),
    lib("java.util.logging.Logger", C, 0, OTH, &[]),
    lib("java.util.logging.Level", C, 0, VAL, &["java.io.Serializable"]),
    // ── java.time / java.math ──
    lib("java.time.Instant", C, 0, VAL, &["java.lang.Comparable", "java.io.Serializable"]),
    lib("java.time.Duration", C, 0, VAL, &["java.lang.Comparable", "java.io.Serializable"]),
    lib("java.time.Period", C, 0, VAL, &["java.io.Serializable"]),
    lib("java.time.LocalDate", C, 0, VAL, &["java.lang.Comparable", "java.io.Serializable"]),
    lib("java.time.LocalTime", C, 0, VAL, &["java.lang.Comparable", "java.io.Serializable"]),
    lib("java.time.LocalDateTime", C, 0, VAL, &["java.lang.Comparable", "java.io.Serializable"]),
    lib("java.time.ZonedDateTime", C, 0, VAL, &["java.lang.Comparable", "java.io.Serializable"]),
    lib("java.time.OffsetDateTime", C, 0, VAL, &["java.lang.Comparable", "java.io.Serializable"]),
    lib("java.time.OffsetTime", C, 0, VAL, &["java.lang.Comparable", "java.io.Serializable"]),
    lib("java.time.Year", C, 0, VAL, &["java.lang.Comparable", "java.io.Serializable"]),
    lib("java.time.YearMonth", C, 0, VAL, &["java.lang.Comparable", "java.io.Serializable"]),
    lib("java.time.MonthDay", C, 0, VAL, &["java.lang.Comparable", "java.io.Serializable"]),
    lib("java.time.ZoneId", C, 0, VAL, &["java.io.Serializable"]),
    lib("java.time.ZoneOffset", C, 0, VAL, &["java.time.ZoneId", "java.lang.Comparable"]),
    lib("java.time.Clock", C, 0, OTH, &[]),
    lib("java.time.DayOfWeek", E, 0, VAL, &[]),
    lib("java.time.Month", E, 0, VAL, &[]),
    lib("java.time.format.DateTimeFormatter", C, 0, VAL, &[]),
    lib("java.time.temporal.ChronoUnit", E, 0, VAL, &[]),
    lib("java.math.BigDecimal", C, 0, VAL, NUM),
    lib("java.math.BigInteger", C, 0, VAL, NUM),
    lib("java.math.MathContext", C, 0, VAL, &["java.io.Serializable"]),
    lib("java.math.RoundingMode", E, 0, VAL, &[]),
    // ── java.nio / java.net / java.sql ──
    lib("java.nio.file.Path", I, 0, VAL, &["java.lang.Comparable"]),
    lib("java.nio.file.Paths", C, 0, OTH, &[]),
    lib("java.nio.file.Files", C, 0, OTH, &[]),
    lib("java.nio.charset.Charset", C, 0, VAL, &["java.lang.Comparable"]),
    lib("java.nio.charset.StandardCharsets", C, 0, OTH, &[]),
    lib("java.net.URI", C, 0, VAL, &["java.lang.Comparable", "java.io.Serializable"]),
    lib("java.net.URL", C, 0, VAL, &["java.io.Serializable"]),
    lib("java.net.InetAddress", C, 0, VAL, &["java.io.Serializable"]),
    lib("java.net.Socket", C, 0, OTH, &["java.io.Closeable"]),
    lib("java.net.ServerSocket", C, 0, OTH, &["java.io.Closeable"]),
    lib("java.net.HttpURLConnection", C, 0, OTH, &[]),
    lib("java.sql.Connection", I, 0, OTH, &["java.lang.AutoCloseable"]),
    lib("java.sql.Statement", I, 0, OTH, &["java.lang.AutoCloseable"]),
    lib("java.sql.PreparedStatement", I, 0, OTH, &["java.sql.Statement"]),
    lib("java.sql.ResultSet", I, 0, OTH, &["java.lang.AutoCloseable"]),
    lib("java.sql.DriverManager", C, 0, OTH, &[]),
    lib("java.sql.SQLException", C, 0, OTH, EXC),
    lib("java.sql.Timestamp", C, 0, VAL, &["java.util.Date"]),
    lib("java.sql.Date", C, 0, VAL, &["java.util.Date"]),
    // ── Kotlin standard library ──
    lib("kotlin.Any", C, 0, OTH, &[]),
    lib("kotlin.Unit", C, 0, VAL, &[]),
    lib("kotlin.Nothing", C, 0, VAL, &[]),
    lib("kotlin.String", C, 0, VAL, &["kotlin.CharSequence", "kotlin.Comparable"]),
    lib("kotlin.CharSequence", I, 0, VAL, &[]),
    lib("kotlin.Comparable", I, 1, OTH, &[]),
    lib("kotlin.Number", C, 0, VAL, &[]),
    lib("kotlin.Int", C, 0, VAL, &["kotlin.Number", "kotlin.Comparable"]),
    lib("kotlin.Long", C, 0, VAL, &["kotlin.Number", "kotlin.Comparable"]),
    lib("kotlin.Short", C, 0, VAL, &["kotlin.Number", "kotlin.Comparable"]),
    lib("kotlin.Byte", C, 0, VAL, &["kotlin.Number", "kotlin.Comparable"]),
    lib("kotlin.Double", C, 0, VAL, &["kotlin.Number", "kotlin.Comparable"]),
    lib("kotlin.Float", C, 0, VAL, &["kotlin.Number", "kotlin.Comparable"]),
    lib("kotlin.Boolean", C, 0, VAL, &["kotlin.Comparable"]),
    lib("kotlin.Char", C, 0, VAL, &["kotlin.Comparable"]),
    lib("kotlin.Array", C, 1, COL, &[]),
    lib("kotlin.IntArray", C, 0, VAL, &[]),
    lib("kotlin.LongArray", C, 0, VAL, &[]),
    lib("kotlin.DoubleArray", C, 0, VAL, &[]),
    lib("kotlin.BooleanArray", C, 0, VAL, &[]),
    lib("kotlin.ByteArray", C, 0, VAL, &[]),
    lib("kotlin.CharArray", C, 0, VAL, &[]),
    lib("kotlin.Pair", C, 2, OTH, &[]),
    lib("kotlin.Triple", C, 3, OTH, &[]),
    lib("kotlin.Lazy", I, 1, OTH, &[]),
    lib("kotlin.Result", C, 1, OTH, &[]),
    lib("kotlin.Throwable", C, 0, OTH, &[]),
    lib("kotlin.Exception", C, 0, OTH, &["kotlin.Throwable"]),
    lib("kotlin.RuntimeException", C, 0, OTH, &["kotlin.Exception"]),
    lib("kotlin.IllegalArgumentException", C, 0, OTH, &["kotlin.RuntimeException"]),
    lib("kotlin.IllegalStateException", C, 0, OTH, &["kotlin.RuntimeException"]),
    lib("kotlin.collections.Iterable", I, 1, COL, &[]),
    lib("kotlin.collections.MutableIterable", I, 1, COL, &["kotlin.collections.Iterable"]),
    lib("kotlin.collections.Collection", I, 1, COL, &["kotlin.collections.Iterable"]),
    lib("kotlin.collections.MutableCollection", I, 1, COL, &["kotlin.collections.Collection", "kotlin.collections.MutableIterable"]),
    lib("kotlin.collections.List", I, 1, COL, &["kotlin.collections.Collection"]),
    lib("kotlin.collections.MutableList", I, 1, COL, &["kotlin.collections.List", "kotlin.collections.MutableCollection"]),
    lib("kotlin.collections.Set", I, 1, COL, &["kotlin.collections.Collection"]),
    lib("kotlin.collections.MutableSet", I, 1, COL, &["kotlin.collections.Set", "kotlin.collections.MutableCollection"]),
    lib("kotlin.collections.Map", I, 2, MAP, &[]),
    lib("kotlin.collections.MutableMap", I, 2, MAP, &["kotlin.collections.Map"]),
    lib("kotlin.collections.ArrayList", C, 1, COL, &["kotlin.collections.MutableList"]),
    lib("kotlin.collections.HashMap", C, 2, MAP, &["kotlin.collections.MutableMap"]),
    lib("kotlin.collections.LinkedHashMap", C, 2, MAP, &["kotlin.collections.HashMap"]),
    lib("kotlin.collections.HashSet", C, 1, COL, &["kotlin.collections.MutableSet"]),
    lib("kotlin.collections.LinkedHashSet", C, 1, COL, &["kotlin.collections.HashSet"]),
    lib("kotlin.collections.Iterator", I, 1, OTH, &[]),
    lib("kotlin.sequences.Sequence", I, 1, COL, &[]),
    lib("kotlin.text.StringBuilder", C, 0, OTH, &["kotlin.CharSequence"]),
    lib("kotlin.text.Regex", C, 0, VAL, &[]),
    lib("kotlin.ranges.IntRange", C, 0, VAL, &[]),
    lib("kotlin.ranges.LongRange", C, 0, VAL, &[]),
];

// ── Legacy stub pseudo-ids ──────────────────────────────────────────────────
//
// Earlier versions of Phase 3 bound twelve JDK types to pseudo-ids >= 0xC000_0000 that were not
// symbols of the table. Phase 3 now materializes every library type it binds as an
// SK_EXTERNAL symbol with its qualified name (see `SymbolTableBuilder::materialize_library`),
// so `type_id` never holds these ids. They are kept, with their original values, for readers
// that still recognise them.

pub const BASE_STUB_SYMBOL_ID: u32 = 0xC000_0000;
pub const OBJECT_STUB_SYM_ID: u32 = BASE_STUB_SYMBOL_ID;
pub const STRING_STUB_SYM_ID: u32 = BASE_STUB_SYMBOL_ID + 1;
pub const INTEGER_STUB_SYM_ID: u32 = BASE_STUB_SYMBOL_ID + 2;
pub const BOOLEAN_STUB_SYM_ID: u32 = BASE_STUB_SYMBOL_ID + 3;
pub const LIST_STUB_SYM_ID: u32 = BASE_STUB_SYMBOL_ID + 4;
pub const MAP_STUB_SYM_ID: u32 = BASE_STUB_SYMBOL_ID + 5;
pub const SET_STUB_SYM_ID: u32 = BASE_STUB_SYMBOL_ID + 6;
pub const COLLECTION_STUB_SYM_ID: u32 = BASE_STUB_SYMBOL_ID + 7;
pub const ITERABLE_STUB_SYM_ID: u32 = BASE_STUB_SYMBOL_ID + 8;
pub const EXCEPTION_STUB_SYM_ID: u32 = BASE_STUB_SYMBOL_ID + 9;
pub const RUNTIME_EXCEPTION_STUB_SYM_ID: u32 = BASE_STUB_SYMBOL_ID + 10;
pub const THROWABLE_STUB_SYM_ID: u32 = BASE_STUB_SYMBOL_ID + 11;

/// A legacy stub pseudo-id with the names it stood for.
#[derive(Debug, Clone)]
pub struct StdLibStub {
    pub symbol_id: u32,
    pub simple_name: &'static str,
    pub qual_name: &'static str,
    pub kind: SymbolKind,
}

/// The legacy stub pseudo-ids and their names (see the note above).
pub fn get_java_stubs() -> Vec<StdLibStub> {
    [
        (OBJECT_STUB_SYM_ID, "java.lang.Object"),
        (STRING_STUB_SYM_ID, "java.lang.String"),
        (INTEGER_STUB_SYM_ID, "java.lang.Integer"),
        (BOOLEAN_STUB_SYM_ID, "java.lang.Boolean"),
        (LIST_STUB_SYM_ID, "java.util.List"),
        (MAP_STUB_SYM_ID, "java.util.Map"),
        (SET_STUB_SYM_ID, "java.util.Set"),
        (COLLECTION_STUB_SYM_ID, "java.util.Collection"),
        (ITERABLE_STUB_SYM_ID, "java.lang.Iterable"),
        (EXCEPTION_STUB_SYM_ID, "java.lang.Exception"),
        (RUNTIME_EXCEPTION_STUB_SYM_ID, "java.lang.RuntimeException"),
        (THROWABLE_STUB_SYM_ID, "java.lang.Throwable"),
    ]
    .iter()
    .filter_map(|&(symbol_id, fqn)| {
        let lib = CATALOG.iter().find(|t| t.fqn == fqn)?;
        Some(StdLibStub {
            symbol_id,
            simple_name: lib.simple_name(),
            qual_name: lib.fqn,
            kind: lib.kind,
        })
    })
    .collect()
}

const JAVA_IMPLICIT: &[&str] = &["java.lang"];
const KOTLIN_IMPLICIT: &[&str] = &[
    "kotlin",
    "kotlin.annotation",
    "kotlin.collections",
    "kotlin.comparisons",
    "kotlin.io",
    "kotlin.ranges",
    "kotlin.sequences",
    "kotlin.text",
    "java.lang",
    "kotlin.jvm",
];

#[derive(Debug, Clone)]
pub struct StdLibManager {
    pub types: Vec<LibraryType>,
    by_fqn: HashMap<&'static str, usize>,
    name_ids: Vec<u32>,
}

impl Default for StdLibManager {
    fn default() -> Self {
        Self::new()
    }
}

impl StdLibManager {
    pub fn new() -> Self {
        let types: Vec<LibraryType> = CATALOG.to_vec();
        let by_fqn = types.iter().enumerate().map(|(i, t)| (t.fqn, i)).collect();
        let name_ids = vec![u32::MAX; types.len()];
        Self {
            types,
            by_fqn,
            name_ids,
        }
    }

    /// Binds each catalog entry to the corpus interner id of its simple name (`u32::MAX` when
    /// no token of the corpus spells it), so materialized library symbols carry a name_id.
    pub fn bind_names(&mut self, interner: &StringInterner) {
        for (i, t) in self.types.iter().enumerate() {
            self.name_ids[i] = interner.find_id(t.simple_name().as_bytes());
        }
    }

    pub fn lookup_fqn(&self, fqn: &str) -> Option<usize> {
        self.by_fqn.get(fqn).copied()
    }

    pub fn get(&self, idx: usize) -> Option<&LibraryType> {
        self.types.get(idx)
    }

    pub fn name_id(&self, idx: usize) -> u32 {
        self.name_ids.get(idx).copied().unwrap_or(u32::MAX)
    }

    /// Packages imported on demand by every compilation unit of the dialect.
    pub fn implicit_packages(&self, dialect: Dialect) -> &'static [&'static str] {
        match dialect {
            Dialect::Java => JAVA_IMPLICIT,
            Dialect::Kotlin => KOTLIN_IMPLICIT,
            Dialect::Other => &[],
        }
    }
}
