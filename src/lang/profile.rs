//! Language registry for the C family (C, C++, C# / .NET).
//!
//! Maps file extensions to `LangId`, decides whether an ambiguous `.h` header is C or
//! C++, declares which pipeline phases and language features each frontend supports, and
//! discovers source and build files while skipping build-output trees and generated code.
//!
//! `.h` rule: a header is C++ when the project contains any C++ source, or when the header
//! itself uses C++-only syntax outside `#ifdef __cplusplus` regions; otherwise it is C.

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use crate::core::types::LangId;

// ── Extensions ───────────────────────────────────────────────────────────────

/// C translation units.
pub const C_SOURCE_EXTENSIONS: &[&str] = &["c"];
/// Headers that are C unless the project or the header is C++ (see [`classify_header`]).
pub const C_HEADER_EXTENSIONS: &[&str] = &["h"];
/// Every extension routed to the C frontend.
pub const C_EXTENSIONS: &[&str] = &["c", "h"];

/// C++ translation units (`.ixx` / `.cppm` are C++20 module interface units).
pub const CPP_SOURCE_EXTENSIONS: &[&str] = &["cpp", "cc", "cxx", "c++", "ixx", "cppm"];
/// C++ headers, template implementation files and inline-definition files.
pub const CPP_HEADER_EXTENSIONS: &[&str] = &["hpp", "hh", "hxx", "h++", "ipp", "tpp", "inl"];
/// Every extension routed to the C++ frontend.
pub const CPP_EXTENSIONS: &[&str] = &[
    "cpp", "cc", "cxx", "c++", "hpp", "hh", "hxx", "h++", "ipp", "tpp", "inl", "ixx", "cppm",
];

/// C# sources (`.csx` is a C# script).
pub const CSHARP_EXTENSIONS: &[&str] = &["cs", "csx"];

/// Role of a file within its language.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FileRole {
    /// Compiled on its own (`.c`, `.cpp`, `.cs`).
    TranslationUnit,
    /// Only reached through `#include` (`.h`, `.hpp`, `.inl`, ...).
    Header,
    /// C++20 module interface unit (`.ixx`, `.cppm`).
    ModuleInterface,
    /// C# script (`.csx`).
    Script,
}

/// What a file extension says about the file's language.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExtensionClass {
    /// The extension names exactly one language.
    Known { lang: LangId, role: FileRole },
    /// `.h`: C or C++, decided by [`classify_header`] / [`resolve_header_languages`].
    AmbiguousHeader,
}

/// Classifies `path` by extension. Returns `None` for files outside the C family.
///
/// Following GCC, an upper-case `.C` / `.H` names C++ — unless the whole file name is
/// upper case, the DOS-era spelling of plain C files.
pub fn classify_extension(path: &Path) -> Option<ExtensionClass> {
    let ext = path.extension()?.to_str()?;
    if ext == "C" || ext == "H" {
        let stem_has_lower = path
            .file_stem()
            .and_then(|s| s.to_str())
            .is_some_and(|s| s.chars().any(|c| c.is_ascii_lowercase()));
        if stem_has_lower {
            let role = if ext == "C" {
                FileRole::TranslationUnit
            } else {
                FileRole::Header
            };
            return Some(ExtensionClass::Known {
                lang: LangId::Cpp,
                role,
            });
        }
    }
    classify_extension_str(&ext.to_ascii_lowercase())
}

/// Classifies a lower-case extension without the dot.
pub fn classify_extension_str(ext: &str) -> Option<ExtensionClass> {
    let known = |lang, role| Some(ExtensionClass::Known { lang, role });
    match ext {
        "c" => known(LangId::C, FileRole::TranslationUnit),
        "h" => Some(ExtensionClass::AmbiguousHeader),
        "cpp" | "cc" | "cxx" | "c++" => known(LangId::Cpp, FileRole::TranslationUnit),
        "ixx" | "cppm" => known(LangId::Cpp, FileRole::ModuleInterface),
        "hpp" | "hh" | "hxx" | "h++" | "ipp" | "tpp" | "inl" => known(LangId::Cpp, FileRole::Header),
        "cs" => known(LangId::CSharp, FileRole::TranslationUnit),
        "csx" => known(LangId::CSharp, FileRole::Script),
        _ => None,
    }
}

/// True for `LangId::C`, `LangId::Cpp` and `LangId::CSharp`.
pub fn is_c_family(lang: LangId) -> bool {
    matches!(lang, LangId::C | LangId::Cpp | LangId::CSharp)
}

// ── Phases and capabilities ──────────────────────────────────────────────────

pub const PHASE_INGESTION: u8 = 1;
pub const PHASE_AST: u8 = 2;
pub const PHASE_SYMBOLS: u8 = 3;
pub const PHASE_CFG: u8 = 4;
pub const PHASE_SSA: u8 = 5;
pub const PHASE_CALL_GRAPH: u8 = 6;
pub const PHASE_TRACEABILITY: u8 = 7;
pub const PHASE_PATH_SUMMARIES: u8 = 8;
pub const PHASE_UML: u8 = 9;
pub const PHASE_UNIFIED_GRAPH: u8 = 10;

/// Phase names, indexed by `phase - 1`.
pub const PHASE_NAMES: [&str; 10] = [
    "ingestion",
    "ast",
    "symbols",
    "cfg",
    "ssa",
    "call-graph",
    "traceability",
    "path-summaries",
    "uml",
    "unified-graph",
];

/// Name of pipeline phase `phase` (1..=10).
pub fn phase_name(phase: u8) -> Option<&'static str> {
    PHASE_NAMES.get(usize::from(phase).checked_sub(1)?).copied()
}

/// A set of pipeline phases (bit `n` = phase `n`, 1..=10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct PhaseSet(u16);

impl PhaseSet {
    pub const EMPTY: PhaseSet = PhaseSet(0);
    pub const ALL: PhaseSet = PhaseSet(0b0111_1111_1110);

    pub const fn of(phases: &[u8]) -> Self {
        let mut bits = 0u16;
        let mut i = 0;
        while i < phases.len() {
            if phases[i] >= 1 && phases[i] <= 10 {
                bits |= 1u16 << phases[i];
            }
            i += 1;
        }
        PhaseSet(bits)
    }

    pub const fn contains(self, phase: u8) -> bool {
        phase >= 1 && phase <= 10 && (self.0 & (1u16 << phase)) != 0
    }

    pub const fn union(self, other: PhaseSet) -> Self {
        PhaseSet(self.0 | other.0)
    }

    pub const fn difference(self, other: PhaseSet) -> Self {
        PhaseSet(self.0 & !other.0)
    }

    pub const fn bits(self) -> u16 {
        self.0
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Phases in the set, ascending.
    pub fn iter(self) -> impl Iterator<Item = u8> {
        (1u8..=10).filter(move |p| self.contains(*p))
    }
}

/// Maturity of one phase for one language.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SupportLevel {
    Supported,
    Experimental,
    Unsupported,
}

impl SupportLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            SupportLevel::Supported => "supported",
            SupportLevel::Experimental => "experimental",
            SupportLevel::Unsupported => "unsupported",
        }
    }
}

// Language feature flags (`LanguageProfile::features`).
/// `#include`, `#define`, `#if` (C, C++). C# has conditional directives but no includes.
pub const FEATURE_PREPROCESSOR: u32 = 1 << 0;
/// Declarations split across headers and translation units.
pub const FEATURE_HEADERS: u32 = 1 << 1;
pub const FEATURE_CLASSES: u32 = 1 << 2;
pub const FEATURE_NAMESPACES: u32 = 1 << 3;
/// C++ templates (primary definitions are analyzed; instantiation is not evaluated).
pub const FEATURE_TEMPLATES: u32 = 1 << 4;
/// C# generics.
pub const FEATURE_GENERICS: u32 = 1 << 5;
pub const FEATURE_EXCEPTIONS: u32 = 1 << 6;
pub const FEATURE_MULTIPLE_INHERITANCE: u32 = 1 << 7;
pub const FEATURE_OPERATOR_OVERLOADING: u32 = 1 << 8;
/// C# properties, indexers and events.
pub const FEATURE_PROPERTIES: u32 = 1 << 9;
/// C# delegates (C and C++ use function pointers instead).
pub const FEATURE_DELEGATES: u32 = 1 << 10;
pub const FEATURE_GOTO: u32 = 1 << 11;
pub const FEATURE_POINTERS: u32 = 1 << 12;
/// C++ destructors, C# finalizers.
pub const FEATURE_DESTRUCTORS: u32 = 1 << 13;
pub const FEATURE_GARBAGE_COLLECTED: u32 = 1 << 14;
/// C++20 modules.
pub const FEATURE_MODULES: u32 = 1 << 15;
/// C# attributes, C++11 / C23 `[[attributes]]`.
pub const FEATURE_ATTRIBUTES: u32 = 1 << 16;
pub const FEATURE_LAMBDAS: u32 = 1 << 17;
/// C# `async` / `await`, C++20 coroutines.
pub const FEATURE_ASYNC: u32 = 1 << 18;
/// C# `partial` types spread across files.
pub const FEATURE_PARTIAL_TYPES: u32 = 1 << 19;
pub const FEATURE_EXTENSION_METHODS: u32 = 1 << 20;
/// C# `yield return` iterators.
pub const FEATURE_ITERATORS: u32 = 1 << 21;
/// C# `using` / `lock` statements, C++ RAII scopes.
pub const FEATURE_SCOPED_RESOURCES: u32 = 1 << 22;
/// `struct` value types with members (C structs, C++ structs, C# structs).
pub const FEATURE_VALUE_TYPES: u32 = 1 << 23;
/// Unions.
pub const FEATURE_UNIONS: u32 = 1 << 24;

/// Build inputs that describe projects of a language.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum BuildSystem {
    /// `compile_commands.json`.
    CompileDatabase,
    /// `CMakeLists.txt`.
    CMake,
    /// `.sln` / `.slnx` / `.csproj` / `.vcxproj` / `Directory.Build.props`.
    MsBuild,
}

/// Static description of one C-family language.
#[derive(Debug, Clone, Copy)]
pub struct LanguageProfile {
    pub lang: LangId,
    /// Stable identifier (`c`, `cpp`, `csharp`).
    pub id: &'static str,
    pub display_name: &'static str,
    /// Every routed extension, lower case, without the dot.
    pub extensions: &'static [&'static str],
    pub source_extensions: &'static [&'static str],
    pub header_extensions: &'static [&'static str],
    /// Phases with a frontend for this language.
    pub phases: PhaseSet,
    /// Subset of `phases` whose results are less complete than for Java.
    pub experimental_phases: PhaseSet,
    /// `FEATURE_*` flags.
    pub features: u32,
    pub build_systems: &'static [BuildSystem],
    /// Grammar crate the Phase 1 / 2 adapters parse with.
    pub grammar_crate: &'static str,
    /// Known analysis limitations, one sentence each.
    pub limitations: &'static [&'static str],
}

impl LanguageProfile {
    pub fn supports_phase(&self, phase: u8) -> bool {
        self.phases.contains(phase)
    }

    pub fn support_level(&self, phase: u8) -> SupportLevel {
        if !self.phases.contains(phase) {
            SupportLevel::Unsupported
        } else if self.experimental_phases.contains(phase) {
            SupportLevel::Experimental
        } else {
            SupportLevel::Supported
        }
    }

    pub fn has_feature(&self, feature: u32) -> bool {
        self.features & feature == feature
    }

    /// Overall maturity: `experimental` when any supported phase is experimental.
    pub fn status(&self) -> &'static str {
        if self.phases.is_empty() {
            SupportLevel::Unsupported.as_str()
        } else if self.experimental_phases.is_empty() {
            SupportLevel::Supported.as_str()
        } else {
            SupportLevel::Experimental.as_str()
        }
    }

    pub fn handles_extension(&self, ext: &str) -> bool {
        let lower = ext.to_ascii_lowercase();
        self.extensions.contains(&lower.as_str())
    }
}

const C_LIMITATIONS: &[&str] = &[
    "Code is analyzed as written: macro bodies are not expanded into the syntax tree.",
    "Regions excluded by #if under the project's known defines are reported but not parsed as active code.",
    "Calls through function pointers resolve only when the pointer's targets are assigned in the analyzed sources.",
];

const CPP_LIMITATIONS: &[&str] = &[
    "Templates are analyzed through their primary definitions; instantiation and specialization selection are not evaluated.",
    "Overload resolution uses declared parameter counts and types; implicit conversion ranking is not modeled.",
    "Code is analyzed as written: macro bodies are not expanded into the syntax tree.",
    "Argument-dependent lookup is limited to namespaces of argument types declared in the analyzed sources.",
];

const CSHARP_LIMITATIONS: &[&str] = &[
    "Source generators are not run; generated partial members are visible only when their output is on disk.",
    "Members of referenced assemblies and NuGet packages come from the bundled base-class-library model, not from metadata.",
    "dynamic-typed member accesses are not resolved.",
];

/// The C-family profiles, in `LangId` order.
pub static C_FAMILY_PROFILES: [LanguageProfile; 3] = [
    LanguageProfile {
        lang: LangId::C,
        id: "c",
        display_name: "C",
        extensions: C_EXTENSIONS,
        source_extensions: C_SOURCE_EXTENSIONS,
        header_extensions: C_HEADER_EXTENSIONS,
        phases: PhaseSet::ALL,
        experimental_phases: PhaseSet::EMPTY,
        features: FEATURE_PREPROCESSOR
            | FEATURE_HEADERS
            | FEATURE_GOTO
            | FEATURE_POINTERS
            | FEATURE_ATTRIBUTES
            | FEATURE_VALUE_TYPES
            | FEATURE_UNIONS,
        build_systems: &[BuildSystem::CompileDatabase, BuildSystem::CMake, BuildSystem::MsBuild],
        grammar_crate: "tree-sitter-c",
        limitations: C_LIMITATIONS,
    },
    LanguageProfile {
        lang: LangId::Cpp,
        id: "cpp",
        display_name: "C++",
        extensions: CPP_EXTENSIONS,
        source_extensions: CPP_SOURCE_EXTENSIONS,
        header_extensions: CPP_HEADER_EXTENSIONS,
        phases: PhaseSet::ALL,
        experimental_phases: PhaseSet::EMPTY,
        features: FEATURE_PREPROCESSOR
            | FEATURE_HEADERS
            | FEATURE_CLASSES
            | FEATURE_NAMESPACES
            | FEATURE_TEMPLATES
            | FEATURE_EXCEPTIONS
            | FEATURE_MULTIPLE_INHERITANCE
            | FEATURE_OPERATOR_OVERLOADING
            | FEATURE_GOTO
            | FEATURE_POINTERS
            | FEATURE_DESTRUCTORS
            | FEATURE_MODULES
            | FEATURE_ATTRIBUTES
            | FEATURE_LAMBDAS
            | FEATURE_ASYNC
            | FEATURE_SCOPED_RESOURCES
            | FEATURE_VALUE_TYPES
            | FEATURE_UNIONS,
        build_systems: &[BuildSystem::CompileDatabase, BuildSystem::CMake, BuildSystem::MsBuild],
        grammar_crate: "tree-sitter-cpp",
        limitations: CPP_LIMITATIONS,
    },
    LanguageProfile {
        lang: LangId::CSharp,
        id: "csharp",
        display_name: "C#",
        extensions: CSHARP_EXTENSIONS,
        source_extensions: CSHARP_EXTENSIONS,
        header_extensions: &[],
        phases: PhaseSet::ALL,
        experimental_phases: PhaseSet::EMPTY,
        features: FEATURE_CLASSES
            | FEATURE_NAMESPACES
            | FEATURE_GENERICS
            | FEATURE_EXCEPTIONS
            | FEATURE_OPERATOR_OVERLOADING
            | FEATURE_PROPERTIES
            | FEATURE_DELEGATES
            | FEATURE_GOTO
            | FEATURE_POINTERS
            | FEATURE_DESTRUCTORS
            | FEATURE_GARBAGE_COLLECTED
            | FEATURE_ATTRIBUTES
            | FEATURE_LAMBDAS
            | FEATURE_ASYNC
            | FEATURE_PARTIAL_TYPES
            | FEATURE_EXTENSION_METHODS
            | FEATURE_ITERATORS
            | FEATURE_SCOPED_RESOURCES
            | FEATURE_VALUE_TYPES,
        build_systems: &[BuildSystem::MsBuild],
        grammar_crate: "tree-sitter-c-sharp",
        limitations: CSHARP_LIMITATIONS,
    },
];

/// Profile of a C-family language.
pub fn profile(lang: LangId) -> Option<&'static LanguageProfile> {
    C_FAMILY_PROFILES.iter().find(|p| p.lang == lang)
}

/// Looks a profile up by identifier or common alias (`c`, `cpp`, `c++`, `cxx`, `cs`, `c#`,
/// `csharp`, `dotnet`), case-insensitively.
pub fn profile_by_id(id: &str) -> Option<&'static LanguageProfile> {
    let lang = match id.trim().to_ascii_lowercase().as_str() {
        "c" => LangId::C,
        "cpp" | "c++" | "cxx" | "cplusplus" => LangId::Cpp,
        "csharp" | "c#" | "cs" | "dotnet" | ".net" => LangId::CSharp,
        _ => return None,
    };
    profile(lang)
}

/// True when the C-family frontend for `lang` implements `phase`.
pub fn supports_phase(lang: LangId, phase: u8) -> bool {
    profile(lang).is_some_and(|p| p.supports_phase(phase))
}

// ── Standard and system headers ──────────────────────────────────────────────

/// ISO C standard library headers (C89 through C23).
pub const C_STANDARD_HEADERS: &[&str] = &[
    "assert.h", "complex.h", "ctype.h", "errno.h", "fenv.h", "float.h", "inttypes.h", "iso646.h",
    "limits.h", "locale.h", "math.h", "setjmp.h", "signal.h", "stdalign.h", "stdarg.h",
    "stdatomic.h", "stdbit.h", "stdbool.h", "stdckdint.h", "stddef.h", "stdint.h", "stdio.h",
    "stdlib.h", "stdnoreturn.h", "string.h", "tgmath.h", "threads.h", "time.h", "uchar.h",
    "wchar.h", "wctype.h",
];

/// ISO C++ standard library headers (C++98 through C++26), including the `<cxxx>` C wrappers.
pub const CPP_STANDARD_HEADERS: &[&str] = &[
    "algorithm", "any", "array", "atomic", "barrier", "bit", "bitset", "cassert", "ccomplex",
    "cctype", "cerrno", "cfenv", "cfloat", "charconv", "chrono", "cinttypes", "ciso646",
    "climits", "clocale", "cmath", "codecvt", "compare", "complex", "concepts",
    "condition_variable", "contracts", "coroutine", "csetjmp", "csignal", "cstdalign", "cstdarg",
    "cstdbool", "cstddef", "cstdint", "cstdio", "cstdlib", "cstring", "ctgmath", "ctime",
    "cuchar", "cwchar", "cwctype", "debugging", "deque", "exception", "execution", "expected",
    "filesystem", "flat_map", "flat_set", "format", "forward_list", "fstream", "functional",
    "future", "generator", "hazard_pointer", "initializer_list", "inplace_vector", "iomanip",
    "ios", "iosfwd", "iostream", "istream", "iterator", "latch", "limits", "linalg", "list",
    "locale", "map", "mdspan", "memory", "memory_resource", "mutex", "new", "numbers", "numeric",
    "optional", "ostream", "print", "queue", "random", "ranges", "ratio", "rcu", "regex",
    "scoped_allocator", "semaphore", "set", "shared_mutex", "source_location", "span",
    "spanstream", "sstream", "stack", "stacktrace", "stdexcept", "stdfloat", "stop_token",
    "streambuf", "string", "string_view", "strstream", "syncstream", "system_error",
    "text_encoding", "thread", "tuple", "type_traits", "typeindex", "typeinfo", "unordered_map",
    "unordered_set", "utility", "valarray", "variant", "vector", "version",
];

/// Common POSIX headers outside `sys/`, `arpa/`, `netinet/` (which are matched by prefix).
const POSIX_HEADERS: &[&str] = &[
    "aio.h", "dirent.h", "dlfcn.h", "fcntl.h", "fnmatch.h", "glob.h", "grp.h", "iconv.h",
    "langinfo.h", "libgen.h", "netdb.h", "nl_types.h", "poll.h", "pthread.h", "pwd.h", "regex.h",
    "sched.h", "search.h", "semaphore.h", "spawn.h", "strings.h", "syslog.h", "termios.h",
    "ucontext.h", "unistd.h", "utime.h", "utmpx.h", "wordexp.h", "getopt.h", "endian.h",
    "execinfo.h", "malloc.h", "alloca.h", "err.h", "features.h", "ifaddrs.h", "link.h",
    "mqueue.h", "ndbm.h", "net/if.h",
];

const POSIX_HEADER_PREFIXES: &[&str] = &["sys/", "arpa/", "netinet/", "linux/", "asm/", "bits/", "mach/"];

/// Common Windows SDK / CRT headers.
const WINDOWS_HEADERS: &[&str] = &[
    "windows.h", "windef.h", "winbase.h", "winuser.h", "wingdi.h", "winnt.h", "winerror.h",
    "winreg.h", "winsock.h", "winsock2.h", "ws2tcpip.h", "mswsock.h", "iphlpapi.h", "tchar.h",
    "process.h", "io.h", "direct.h", "conio.h", "shlobj.h", "shellapi.h", "shlwapi.h",
    "objbase.h", "combaseapi.h", "comdef.h", "comutil.h", "oleauto.h", "ole2.h", "unknwn.h",
    "atlbase.h", "atlcom.h", "atlstr.h", "afx.h", "afxwin.h", "commctrl.h", "commdlg.h",
    "crtdbg.h", "dbghelp.h", "psapi.h", "tlhelp32.h", "wincrypt.h", "bcrypt.h", "ntstatus.h",
    "sal.h", "specstrings.h", "strsafe.h", "versionhelpers.h", "wtypes.h", "minwindef.h",
    "synchapi.h", "processthreadsapi.h", "fileapi.h", "handleapi.h", "memoryapi.h", "d3d11.h",
    "d3d12.h", "dxgi.h", "wrl.h", "wrl/client.h", "mmsystem.h", "lmcons.h", "sddl.h",
];

/// Who provides a header that is not part of the analyzed sources.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SystemHeaderKind {
    CStandard,
    CppStandard,
    Posix,
    Windows,
    /// Compiler intrinsics (`<immintrin.h>`, `<intrin.h>`, `<arm_neon.h>`).
    CompilerIntrinsics,
}

/// Classifies an `#include` spelling (`stdio.h`, `vector`, `sys/types.h`) as a standard or
/// platform header. Matching is case-insensitive for Windows headers only.
pub fn system_header_kind(spelled: &str) -> Option<SystemHeaderKind> {
    let name = spelled.trim().replace('\\', "/");
    if C_STANDARD_HEADERS.contains(&name.as_str()) {
        return Some(SystemHeaderKind::CStandard);
    }
    if CPP_STANDARD_HEADERS.contains(&name.as_str()) {
        return Some(SystemHeaderKind::CppStandard);
    }
    if POSIX_HEADERS.contains(&name.as_str()) || POSIX_HEADER_PREFIXES.iter().any(|p| name.starts_with(p)) {
        return Some(SystemHeaderKind::Posix);
    }
    let lower = name.to_ascii_lowercase();
    if WINDOWS_HEADERS.contains(&lower.as_str()) {
        return Some(SystemHeaderKind::Windows);
    }
    if lower.ends_with("intrin.h") || lower == "arm_neon.h" || lower == "arm_acle.h" || lower == "cpuid.h" {
        return Some(SystemHeaderKind::CompilerIntrinsics);
    }
    None
}

// ── `.h` disambiguation ──────────────────────────────────────────────────────

/// Why a header was classified the way it was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeaderEvidence {
    /// The project contains at least one C++ source file.
    ProjectContainsCpp,
    /// The header uses a C++-only construct at `line` (outside `#ifdef __cplusplus`).
    CppSyntax { construct: &'static str, line: u32 },
    /// The header includes a C++ standard header or a C++ header file.
    IncludesCppHeader { header: String, line: u32 },
    /// No C++-only construct was found; the header is C.
    NoCppEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeaderClassification {
    pub lang: LangId,
    pub evidence: HeaderEvidence,
}

/// Decides the language of a `.h` header.
pub fn classify_header(source: &[u8], project_has_cpp: bool) -> HeaderClassification {
    if project_has_cpp {
        return HeaderClassification {
            lang: LangId::Cpp,
            evidence: HeaderEvidence::ProjectContainsCpp,
        };
    }
    match find_cpp_evidence(source) {
        Some(evidence) => HeaderClassification {
            lang: LangId::Cpp,
            evidence,
        },
        None => HeaderClassification {
            lang: LangId::C,
            evidence: HeaderEvidence::NoCppEvidence,
        },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HeaderTok<'a> {
    Ident(&'a [u8]),
    Punct(&'a [u8]),
    Str(&'a [u8]),
    Number,
}

#[derive(Debug, Clone, Copy)]
struct LexedTok<'a> {
    tok: HeaderTok<'a>,
    line: u32,
}

/// Effect of an `#if` frame on C++ evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GuardFrame {
    /// Code compiled only by C++ compilers (`#ifdef __cplusplus`): never evidence.
    CppOnly,
    /// Code compiled only by C compilers (`#ifndef __cplusplus`).
    NotCpp,
    Neutral,
}

/// Scans a header for C++-only syntax, ignoring comments, literals and regions guarded by
/// `__cplusplus`. Returns the first piece of evidence found.
pub fn find_cpp_evidence(source: &[u8]) -> Option<HeaderEvidence> {
    let mut tokens: Vec<LexedTok<'_>> = Vec::new();
    let mut frames: Vec<GuardFrame> = Vec::new();
    let mut i = 0usize;
    let mut line = 1u32;
    let mut at_line_start = true;
    let n = source.len();

    while i < n {
        let c = source[i];
        match c {
            b'\n' => {
                line += 1;
                at_line_start = true;
                i += 1;
            }
            b' ' | b'\t' | b'\r' | 0x0B | 0x0C => i += 1,
            b'\\' if splice_len(source, i) > 0 => {
                i += splice_len(source, i);
                line += 1;
            }
            b'/' if source.get(i + 1) == Some(&b'/') => {
                while i < n && source[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if source.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i < n && !(source[i] == b'*' && source.get(i + 1) == Some(&b'/')) {
                    if source[i] == b'\n' {
                        line += 1;
                    }
                    i += 1;
                }
                i = (i + 2).min(n);
            }
            b'#' if at_line_start => {
                let start_line = line;
                let (text, next, lines) = read_directive_line(source, i + 1);
                i = next;
                line += lines;
                if let Some(evidence) = process_header_directive(&text, start_line, &mut frames) {
                    if !frames.contains(&GuardFrame::CppOnly) {
                        return Some(evidence);
                    }
                }
            }
            b'"' | b'\'' => {
                let (end, lines) = skip_quoted(source, i, c);
                if !frames.contains(&GuardFrame::CppOnly) && c == b'"' {
                    tokens.push(LexedTok {
                        tok: HeaderTok::Str(&source[i..end]),
                        line,
                    });
                }
                line += lines;
                i = end;
                at_line_start = false;
            }
            b'0'..=b'9' => {
                i = skip_pp_number(source, i);
                if !frames.contains(&GuardFrame::CppOnly) {
                    tokens.push(LexedTok {
                        tok: HeaderTok::Number,
                        line,
                    });
                }
                at_line_start = false;
            }
            b'.' if source.get(i + 1).is_some_and(|d| d.is_ascii_digit()) => {
                i = skip_pp_number(source, i);
                at_line_start = false;
            }
            _ if is_ident_start(c) => {
                let start = i;
                while i < n && is_ident_continue(source[i]) {
                    i += 1;
                }
                let ident = &source[start..i];
                at_line_start = false;
                // Encoding prefixes and raw strings: `L"..."`, `u8'x'`, `R"d(...)d"`.
                if let Some(&q) = source.get(i) {
                    if (q == b'"' || q == b'\'') && is_literal_prefix(ident) {
                        let (end, lines) = if ident.ends_with(b"R") && q == b'"' {
                            skip_raw_string(source, i)
                        } else {
                            skip_quoted(source, i, q)
                        };
                        if !frames.contains(&GuardFrame::CppOnly) && q == b'"' {
                            tokens.push(LexedTok {
                                tok: HeaderTok::Str(&source[i..end]),
                                line,
                            });
                        }
                        line += lines;
                        i = end;
                        continue;
                    }
                }
                if !frames.contains(&GuardFrame::CppOnly) {
                    tokens.push(LexedTok {
                        tok: HeaderTok::Ident(ident),
                        line,
                    });
                }
            }
            _ => {
                let len = punct_len(source, i);
                if !frames.contains(&GuardFrame::CppOnly) {
                    tokens.push(LexedTok {
                        tok: HeaderTok::Punct(&source[i..i + len]),
                        line,
                    });
                }
                i += len;
                at_line_start = false;
            }
        }
    }
    scan_cpp_tokens(&tokens)
}

/// Length of a line splice (`\` + optional horizontal whitespace + newline) at `i`, or 0.
fn splice_len(source: &[u8], i: usize) -> usize {
    let mut j = i + 1;
    while j < source.len() && (source[j] == b' ' || source[j] == b'\t') {
        j += 1;
    }
    if source.get(j) == Some(&b'\r') {
        j += 1;
    }
    if source.get(j) == Some(&b'\n') {
        j + 1 - i
    } else {
        0
    }
}

/// Reads a directive's logical line after `#`, with splices joined and comments replaced
/// by a space. Returns `(text, index after the line, newlines consumed)`.
fn read_directive_line(source: &[u8], mut i: usize) -> (String, usize, u32) {
    let mut out = Vec::new();
    let mut lines = 0u32;
    let n = source.len();
    while i < n {
        let c = source[i];
        if c == b'\n' {
            break;
        }
        if c == b'\\' && splice_len(source, i) > 0 {
            i += splice_len(source, i);
            lines += 1;
            continue;
        }
        if c == b'/' && source.get(i + 1) == Some(&b'/') {
            while i < n && source[i] != b'\n' {
                i += 1;
            }
            break;
        }
        if c == b'/' && source.get(i + 1) == Some(&b'*') {
            i += 2;
            while i < n && !(source[i] == b'*' && source.get(i + 1) == Some(&b'/')) {
                if source[i] == b'\n' {
                    lines += 1;
                }
                i += 1;
            }
            i = (i + 2).min(n);
            out.push(b' ');
            continue;
        }
        if c == b'"' || c == b'\'' {
            let (end, l) = skip_quoted(source, i, c);
            out.extend_from_slice(&source[i..end]);
            lines += l;
            i = end;
            continue;
        }
        out.push(c);
        i += 1;
    }
    (String::from_utf8_lossy(&out).into_owned(), i, lines)
}

/// Skips a quoted literal starting at `i` (the quote). Stops at an unescaped newline for
/// unterminated literals. Returns `(index after the literal, newlines consumed)`.
fn skip_quoted(source: &[u8], i: usize, quote: u8) -> (usize, u32) {
    let mut j = i + 1;
    let mut lines = 0;
    while j < source.len() {
        match source[j] {
            b'\\' => {
                let splice = splice_len(source, j);
                if splice > 0 {
                    j += splice;
                    lines += 1;
                } else {
                    j += 2;
                }
            }
            b'\n' => return (j, lines),
            c if c == quote => return (j + 1, lines),
            _ => j += 1,
        }
    }
    (source.len(), lines)
}

/// Skips a C++11 raw string `"delim( ... )delim"` starting at the opening quote.
fn skip_raw_string(source: &[u8], quote: usize) -> (usize, u32) {
    let mut j = quote + 1;
    let delim_start = j;
    while j < source.len() && source[j] != b'(' && j - delim_start <= 16 {
        if matches!(source[j], b' ' | b')' | b'\\' | b'\t' | b'\n') {
            return skip_quoted(source, quote, b'"');
        }
        j += 1;
    }
    if j >= source.len() || source[j] != b'(' {
        return skip_quoted(source, quote, b'"');
    }
    let delim = &source[delim_start..j];
    let mut lines = 0;
    j += 1;
    while j < source.len() {
        if source[j] == b'\n' {
            lines += 1;
        }
        if source[j] == b')'
            && source[j + 1..].starts_with(delim)
            && source.get(j + 1 + delim.len()) == Some(&b'"')
        {
            return (j + 2 + delim.len(), lines);
        }
        j += 1;
    }
    (source.len(), lines)
}

/// Skips a preprocessing number (digits, letters, `_`, `.`, digit separators and signed
/// exponents).
fn skip_pp_number(source: &[u8], mut i: usize) -> usize {
    i += 1;
    while i < source.len() {
        let c = source[i];
        if (c == b'+' || c == b'-') && matches!(source[i - 1], b'e' | b'E' | b'p' | b'P') {
            i += 1;
        } else if c == b'\'' && source.get(i + 1).is_some_and(|d| d.is_ascii_alphanumeric()) {
            i += 2;
        } else if c.is_ascii_alphanumeric() || c == b'_' || c == b'.' {
            i += 1;
        } else {
            break;
        }
    }
    i
}

fn is_ident_start(c: u8) -> bool {
    c == b'_' || c.is_ascii_alphabetic() || c >= 0x80 || c == b'$'
}

fn is_ident_continue(c: u8) -> bool {
    is_ident_start(c) || c.is_ascii_digit()
}

fn is_literal_prefix(ident: &[u8]) -> bool {
    matches!(ident, b"L" | b"u" | b"U" | b"u8" | b"R" | b"LR" | b"uR" | b"UR" | b"u8R")
}

fn punct_len(source: &[u8], i: usize) -> usize {
    let two = &source[i..(i + 2).min(source.len())];
    match two {
        b"::" | b"[[" | b"]]" | b"->" | b"&&" | b"||" | b"==" | b"!=" | b"<=" | b">=" | b"<<"
        | b">>" | b"++" | b"--" => 2,
        _ => {
            // A multi-byte UTF-8 sequence is never split.
            let mut len = 1;
            while i + len < source.len() && (source[i + len] & 0xC0) == 0x80 {
                len += 1;
            }
            len
        }
    }
}

/// Updates the `__cplusplus` guard stack for one directive and reports include-based
/// C++ evidence.
fn process_header_directive(text: &str, line: u32, frames: &mut Vec<GuardFrame>) -> Option<HeaderEvidence> {
    let trimmed = text.trim_start();
    let name_end = trimmed
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(trimmed.len());
    let (name, rest) = trimmed.split_at(name_end);
    let rest = rest.trim();
    match name {
        "ifdef" => frames.push(if first_word(rest) == "__cplusplus" {
            GuardFrame::CppOnly
        } else {
            GuardFrame::Neutral
        }),
        "ifndef" => frames.push(if first_word(rest) == "__cplusplus" {
            GuardFrame::NotCpp
        } else {
            GuardFrame::Neutral
        }),
        "if" => frames.push(classify_cplusplus_condition(rest)),
        "elif" | "elifdef" | "elifndef" => {
            if let Some(top) = frames.last_mut() {
                *top = GuardFrame::Neutral;
            }
        }
        "else" => {
            if let Some(top) = frames.last_mut() {
                *top = match *top {
                    GuardFrame::CppOnly => GuardFrame::NotCpp,
                    GuardFrame::NotCpp => GuardFrame::CppOnly,
                    GuardFrame::Neutral => GuardFrame::Neutral,
                };
            }
        }
        "endif" => {
            frames.pop();
        }
        "include" | "include_next" | "import" => {
            let spelled = rest.trim();
            if let Some(inner) = spelled.strip_prefix('<').and_then(|s| s.split('>').next()) {
                if CPP_STANDARD_HEADERS.contains(&inner.trim()) {
                    return Some(HeaderEvidence::IncludesCppHeader {
                        header: format!("<{}>", inner.trim()),
                        line,
                    });
                }
            }
            if let Some(inner) = spelled.strip_prefix('"').and_then(|s| s.split('"').next()) {
                let is_cpp_header = Path::new(inner)
                    .extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| CPP_HEADER_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()));
                if is_cpp_header {
                    return Some(HeaderEvidence::IncludesCppHeader {
                        header: format!("\"{}\"", inner),
                        line,
                    });
                }
            }
        }
        _ => {}
    }
    None
}

fn first_word(text: &str) -> &str {
    text.split(|c: char| c.is_whitespace() || c == '(' || c == ')')
        .find(|w| !w.is_empty())
        .unwrap_or("")
}

/// `#if` condition effect: negated `__cplusplus` tests guard C-only code, positive tests
/// without `||` guard C++-only code, anything else is neutral.
fn classify_cplusplus_condition(cond: &str) -> GuardFrame {
    let Some(pos) = cond.find("__cplusplus") else {
        return GuardFrame::Neutral;
    };
    if cond.contains("||") {
        return GuardFrame::Neutral;
    }
    let before = cond[..pos].trim_end();
    let before = before
        .strip_suffix('(')
        .map(str::trim_end)
        .unwrap_or(before);
    let before = before
        .strip_suffix("defined")
        .map(str::trim_end)
        .unwrap_or(before);
    let before = before.strip_suffix('(').map(str::trim_end).unwrap_or(before);
    if before.ends_with('!') {
        GuardFrame::NotCpp
    } else {
        GuardFrame::CppOnly
    }
}

/// Finds C++-only syntax in the token stream of a header.
fn scan_cpp_tokens(tokens: &[LexedTok<'_>]) -> Option<HeaderEvidence> {
    let ident_at = |k: usize| match tokens.get(k).map(|t| t.tok) {
        Some(HeaderTok::Ident(s)) => Some(s),
        _ => None,
    };
    let punct_at = |k: usize| match tokens.get(k).map(|t| t.tok) {
        Some(HeaderTok::Punct(s)) => Some(s),
        _ => None,
    };
    let found = |construct: &'static str, k: usize| {
        Some(HeaderEvidence::CppSyntax {
            construct,
            line: tokens[k].line,
        })
    };
    let mut in_attribute = false;

    for k in 0..tokens.len() {
        match tokens[k].tok {
            HeaderTok::Punct(b"[[") => in_attribute = true,
            HeaderTok::Punct(b"]]") => in_attribute = false,
            HeaderTok::Punct(b"::") if !in_attribute => {
                return found("scope resolution operator `::`", k);
            }
            HeaderTok::Ident(word) => match word {
                b"class" => {
                    let next_is_body = matches!(punct_at(k + 2), Some(b"{") | Some(b":"))
                        || matches!(ident_at(k + 2), Some(b"final"));
                    if ident_at(k + 1).is_some() && next_is_body {
                        return found("class definition", k);
                    }
                    if k > 0 && matches!(ident_at(k - 1), Some(b"enum")) {
                        return found("scoped enumeration `enum class`", k);
                    }
                }
                b"namespace" => {
                    if ident_at(k + 1).is_some() || matches!(punct_at(k + 1), Some(b"{")) {
                        return found("namespace", k);
                    }
                }
                b"template" => {
                    if matches!(punct_at(k + 1), Some(b"<")) {
                        return found("template declaration", k);
                    }
                }
                b"virtual" => {
                    if ident_at(k + 1).is_some() || matches!(punct_at(k + 1), Some(b"~")) {
                        return found("virtual member function", k);
                    }
                }
                b"public" | b"private" | b"protected" => {
                    if matches!(punct_at(k + 1), Some(b":")) {
                        return found("access specifier", k);
                    }
                }
                b"operator" => {
                    let overloads = matches!(
                        punct_at(k + 1),
                        Some(b"=") | Some(b"+") | Some(b"-") | Some(b"*") | Some(b"/") | Some(b"%")
                            | Some(b"<") | Some(b">") | Some(b"!") | Some(b"~") | Some(b"&")
                            | Some(b"|") | Some(b"^") | Some(b"(") | Some(b"[") | Some(b"==")
                            | Some(b"!=") | Some(b"<=") | Some(b">=") | Some(b"<<") | Some(b">>")
                            | Some(b"++") | Some(b"--") | Some(b"->") | Some(b"&&") | Some(b"||")
                    ) || matches!(ident_at(k + 1), Some(b"new") | Some(b"delete"));
                    if overloads {
                        return found("operator overload", k);
                    }
                }
                b"static_cast" | b"dynamic_cast" | b"reinterpret_cast" | b"const_cast" => {
                    if matches!(punct_at(k + 1), Some(b"<")) {
                        return found("C++ cast expression", k);
                    }
                }
                b"decltype" => {
                    if matches!(punct_at(k + 1), Some(b"(")) {
                        return found("decltype", k);
                    }
                }
                b"noexcept" => return found("noexcept specification", k),
                b"co_await" | b"co_return" | b"co_yield" => return found("coroutine keyword", k),
                b"consteval" | b"constinit" => return found("consteval / constinit", k),
                b"using" => {
                    if matches!(ident_at(k + 1), Some(b"namespace")) {
                        return found("using-directive", k);
                    }
                    if ident_at(k + 1).is_some() && matches!(punct_at(k + 2), Some(b"=") | Some(b"::")) {
                        return found("using-declaration", k);
                    }
                }
                b"extern" => {
                    if let Some(HeaderTok::Str(s)) = tokens.get(k + 1).map(|t| t.tok) {
                        if matches!(s, b"\"C\"" | b"\"C++\"") {
                            return found("unguarded `extern \"C\"` linkage specification", k);
                        }
                    }
                }
                b"friend" => {
                    if matches!(ident_at(k + 1), Some(b"class") | Some(b"struct")) {
                        return found("friend declaration", k);
                    }
                }
                b"explicit" | b"mutable" => {
                    if ident_at(k + 1).is_some() {
                        return found("explicit / mutable specifier", k);
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }
    None
}

/// Language decisions for every ambiguous `.h` header of a project.
#[derive(Debug, Clone, Default)]
pub struct HeaderLanguages {
    /// The project has at least one unambiguous C++ file.
    pub project_has_cpp: bool,
    pub decisions: BTreeMap<PathBuf, HeaderClassification>,
    /// Headers that could not be read; they are classified C.
    pub unreadable: Vec<(PathBuf, String)>,
}

impl HeaderLanguages {
    pub fn lang_of(&self, path: &Path) -> Option<LangId> {
        self.decisions.get(path).map(|d| d.lang)
    }

    /// The language every `.h` resolved to, when it is the same for all of them. This is
    /// the value for an extension-wide `"h"` entry in `SourceManifest::language_overrides`.
    pub fn uniform_language(&self) -> Option<LangId> {
        if self.project_has_cpp {
            return Some(LangId::Cpp);
        }
        let mut langs = self.decisions.values().map(|d| d.lang);
        let first = langs.next()?;
        if langs.all(|l| l == first) {
            Some(first)
        } else {
            None
        }
    }

    /// Per-file language for headers whose language differs from the C default.
    pub fn cpp_headers(&self) -> impl Iterator<Item = &Path> {
        self.decisions
            .iter()
            .filter(|(_, d)| d.lang == LangId::Cpp)
            .map(|(p, _)| p.as_path())
    }
}

/// Bytes of a header inspected for C++ evidence.
pub const HEADER_SCAN_LIMIT_BYTES: u64 = 1024 * 1024;

/// Decides the language of every `.h` in `paths` (other paths are ignored).
pub fn resolve_header_languages(paths: &[PathBuf]) -> HeaderLanguages {
    let mut result = HeaderLanguages {
        project_has_cpp: paths.iter().any(|p| {
            matches!(
                classify_extension(p),
                Some(ExtensionClass::Known {
                    lang: LangId::Cpp,
                    ..
                })
            )
        }),
        ..HeaderLanguages::default()
    };
    for path in paths {
        if classify_extension(path) != Some(ExtensionClass::AmbiguousHeader) {
            continue;
        }
        if result.project_has_cpp {
            result
                .decisions
                .insert(path.clone(), classify_header(&[], true));
            continue;
        }
        match read_prefix(path, HEADER_SCAN_LIMIT_BYTES) {
            Ok(bytes) => {
                result
                    .decisions
                    .insert(path.clone(), classify_header(&bytes, false));
            }
            Err(err) => {
                result.unreadable.push((path.clone(), err.to_string()));
                result.decisions.insert(
                    path.clone(),
                    HeaderClassification {
                        lang: LangId::C,
                        evidence: HeaderEvidence::NoCppEvidence,
                    },
                );
            }
        }
    }
    result
}

/// Language and role of `path`, with `.h` decided by `headers` (C when not listed).
pub fn detect_language(path: &Path, headers: &HeaderLanguages) -> Option<(LangId, FileRole)> {
    match classify_extension(path)? {
        ExtensionClass::Known { lang, role } => Some((lang, role)),
        ExtensionClass::AmbiguousHeader => {
            let lang = headers.lang_of(path).unwrap_or(if headers.project_has_cpp {
                LangId::Cpp
            } else {
                LangId::C
            });
            Some((lang, FileRole::Header))
        }
    }
}

fn read_prefix(path: &Path, limit: u64) -> io::Result<Vec<u8>> {
    let file = fs::File::open(path)?;
    let mut bytes = Vec::new();
    file.take(limit).read_to_end(&mut bytes)?;
    Ok(bytes)
}

// ── Build output and generated files ─────────────────────────────────────────

/// Directory names produced by builds and IDEs (compared case-insensitively).
pub const BUILD_OUTPUT_DIRECTORIES: &[&str] = &[
    "bin",
    "obj",
    "build",
    "out",
    ".vs",
    "node_modules",
    "target",
    "cmakefiles",
    "vcpkg_installed",
    "ipch",
];

/// Directory-name prefixes produced by IDE CMake integrations (`cmake-build-debug`).
pub const BUILD_OUTPUT_DIRECTORY_PREFIXES: &[&str] = &["cmake-build-"];

/// Version-control metadata directories.
pub const VCS_DIRECTORIES: &[&str] = &[".git", ".hg", ".svn", ".bzr"];

/// True for a build-output directory name (`bin`, `obj`, `cmake-build-release`, ...).
pub fn is_build_output_directory(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    BUILD_OUTPUT_DIRECTORIES.contains(&lower.as_str())
        || BUILD_OUTPUT_DIRECTORY_PREFIXES.iter().any(|p| lower.starts_with(p))
}

/// True for a legacy NuGet `packages/` restore folder (contains `repositories.config` or a
/// package directory with a `.nupkg`).
pub fn is_nuget_packages_directory(path: &Path) -> bool {
    let is_named_packages = path
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.eq_ignore_ascii_case("packages"));
    if !is_named_packages {
        return false;
    }
    if path.join("repositories.config").is_file() {
        return true;
    }
    let Ok(entries) = fs::read_dir(path) else {
        return false;
    };
    for entry in entries.flatten().take(64) {
        let child = entry.path();
        if !child.is_dir() {
            continue;
        }
        let Ok(inner) = fs::read_dir(&child) else {
            continue;
        };
        let has_nupkg = inner.flatten().take(64).any(|e| {
            e.path()
                .extension()
                .and_then(|x| x.to_str())
                .is_some_and(|x| x.eq_ignore_ascii_case("nupkg"))
        });
        if has_nupkg {
            return true;
        }
    }
    false
}

/// True for a file name that code generators produce: `*.g.cs`, `*.g.i.cs`,
/// `*.designer.cs`, `*.generated.*`, Qt `moc_*` / `qrc_*` / `*.moc`, protobuf `*.pb.*`,
/// bison `*.tab.c` / `*.tab.h`, flex `lex.yy.c`, MSBuild `*.AssemblyAttributes.cs` and
/// `TemporaryGeneratedFile_*`.
pub fn is_generated_file_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".g.cs")
        || lower.ends_with(".g.i.cs")
        || lower.ends_with(".designer.cs")
        || lower.ends_with(".assemblyattributes.cs")
        || lower.contains(".generated.")
        || lower.starts_with("moc_")
        || lower.starts_with("qrc_")
        || lower.ends_with(".moc")
        || lower.contains(".pb.")
        || lower.ends_with(".tab.c")
        || lower.ends_with(".tab.h")
        || lower.ends_with(".tab.cc")
        || lower.ends_with(".tab.hh")
        || lower == "lex.yy.c"
        || lower == "lex.yy.cc"
        || lower.starts_with("temporarygeneratedfile_")
}

/// Markers code generators write into the first lines of their output.
pub const GENERATED_CONTENT_MARKERS: &[&str] = &[
    "<auto-generated",
    "<autogenerated",
    "@generated",
    "generated by the protocol buffer compiler",
    "created by: the resource compiler for qt",
    "meta object code from reading c++ file",
    "this file was automatically generated by swig",
];

/// Bytes inspected for [`GENERATED_CONTENT_MARKERS`].
pub const GENERATED_MARKER_SCAN_BYTES: u64 = 2048;

/// True when the head of a file carries a code-generator marker (case-insensitive).
pub fn has_generated_marker(head: &[u8]) -> bool {
    let limit = (GENERATED_MARKER_SCAN_BYTES as usize).min(head.len());
    let text = String::from_utf8_lossy(&head[..limit]).to_ascii_lowercase();
    GENERATED_CONTENT_MARKERS.iter().any(|m| text.contains(m))
}

// ── Discovery ────────────────────────────────────────────────────────────────

/// Why a directory or file was not descended into / not admitted.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SkipKind {
    /// `bin`, `obj`, `build`, `out`, `cmake-build-*`, `.vs`, `node_modules`, `target`, ...
    BuildOutput,
    /// `.git`, `.svn`, ...
    VersionControl,
    /// A NuGet `packages/` restore folder.
    PackageCache,
    /// Any other directory whose name starts with `.`.
    Hidden,
    /// Listed in `DiscoveryOptions::extra_excluded_directories`.
    UserExcluded,
    /// Symbolic links are never followed.
    Symlink,
    /// Deeper than `DiscoveryOptions::max_depth`.
    DepthLimit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedEntry {
    pub path: PathBuf,
    pub kind: SkipKind,
}

/// Options for [`discover`].
#[derive(Debug, Clone)]
pub struct DiscoveryOptions {
    /// Admit generated files instead of listing them in `Discovery::generated_files`.
    pub include_generated: bool,
    /// Read the head of each source to detect generator markers (`<auto-generated>`).
    pub inspect_generated_markers: bool,
    /// Additional directory names to skip (case-insensitive).
    pub extra_excluded_directories: Vec<String>,
    /// Languages to admit; empty admits all three C-family languages.
    pub languages: Vec<LangId>,
    /// Maximum directory depth below the root.
    pub max_depth: usize,
}

impl Default for DiscoveryOptions {
    fn default() -> Self {
        Self {
            include_generated: false,
            inspect_generated_markers: true,
            extra_excluded_directories: Vec::new(),
            languages: Vec::new(),
            max_depth: 64,
        }
    }
}

/// A C-family source file admitted by discovery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredFile {
    pub path: PathBuf,
    pub lang: LangId,
    pub role: FileRole,
}

/// Build inputs found during discovery.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BuildFiles {
    /// `compile_commands.json`, including copies inside skipped build directories.
    pub compile_databases: Vec<PathBuf>,
    pub cmake_lists: Vec<PathBuf>,
    /// `.sln` and `.slnx`.
    pub solutions: Vec<PathBuf>,
    /// `.csproj`, `.vcxproj`, `.fsproj`, `.vbproj`.
    pub msbuild_projects: Vec<PathBuf>,
    /// `Directory.Build.props`, `Directory.Build.targets`, `Directory.Packages.props`.
    pub msbuild_directory_files: Vec<PathBuf>,
    pub packages_configs: Vec<PathBuf>,
}

impl BuildFiles {
    pub fn is_empty(&self) -> bool {
        self.compile_databases.is_empty()
            && self.cmake_lists.is_empty()
            && self.solutions.is_empty()
            && self.msbuild_projects.is_empty()
    }

    fn record(&mut self, path: &Path) {
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            return;
        };
        let lower = name.to_ascii_lowercase();
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .unwrap_or_default();
        if lower == "compile_commands.json" {
            self.compile_databases.push(path.to_path_buf());
        } else if lower == "cmakelists.txt" {
            self.cmake_lists.push(path.to_path_buf());
        } else if ext == "sln" || ext == "slnx" {
            self.solutions.push(path.to_path_buf());
        } else if matches!(ext.as_str(), "csproj" | "vcxproj" | "fsproj" | "vbproj") {
            self.msbuild_projects.push(path.to_path_buf());
        } else if matches!(
            lower.as_str(),
            "directory.build.props" | "directory.build.targets" | "directory.packages.props"
        ) {
            self.msbuild_directory_files.push(path.to_path_buf());
        } else if lower == "packages.config" {
            self.packages_configs.push(path.to_path_buf());
        }
    }

    fn sort(&mut self) {
        for list in [
            &mut self.compile_databases,
            &mut self.cmake_lists,
            &mut self.solutions,
            &mut self.msbuild_projects,
            &mut self.msbuild_directory_files,
            &mut self.packages_configs,
        ] {
            list.sort();
            list.dedup();
        }
    }
}

/// Result of [`discover`].
#[derive(Debug, Clone, Default)]
pub struct Discovery {
    pub root: PathBuf,
    /// Admitted sources, sorted by path.
    pub files: Vec<DiscoveredFile>,
    pub build_files: BuildFiles,
    pub skipped: Vec<SkippedEntry>,
    /// Generated sources left out (empty when `include_generated` is set).
    pub generated_files: Vec<PathBuf>,
    /// Entries that could not be read, with the error.
    pub unreadable: Vec<(PathBuf, String)>,
    pub headers: HeaderLanguages,
}

impl Discovery {
    pub fn paths(&self) -> Vec<PathBuf> {
        self.files.iter().map(|f| f.path.clone()).collect()
    }

    pub fn files_of(&self, lang: LangId) -> impl Iterator<Item = &DiscoveredFile> {
        self.files.iter().filter(move |f| f.lang == lang)
    }

    pub fn languages(&self) -> Vec<LangId> {
        let mut langs: Vec<LangId> = self.files.iter().map(|f| f.lang).collect();
        langs.sort();
        langs.dedup();
        langs
    }
}

/// Upper bound on directories visited when probing a skipped build tree for
/// `compile_commands.json`.
const COMPILE_DB_PROBE_DIRECTORIES: usize = 256;
/// Depth below a skipped build directory probed for `compile_commands.json`.
const COMPILE_DB_PROBE_DEPTH: usize = 3;

fn skip_kind_for_directory(path: &Path, name: &str, options: &DiscoveryOptions) -> Option<SkipKind> {
    let lower = name.to_ascii_lowercase();
    if VCS_DIRECTORIES.contains(&lower.as_str()) {
        return Some(SkipKind::VersionControl);
    }
    if is_build_output_directory(&lower) {
        return Some(SkipKind::BuildOutput);
    }
    if options
        .extra_excluded_directories
        .iter()
        .any(|d| d.eq_ignore_ascii_case(name))
    {
        return Some(SkipKind::UserExcluded);
    }
    if lower.starts_with('.') {
        return Some(SkipKind::Hidden);
    }
    if is_nuget_packages_directory(path) {
        return Some(SkipKind::PackageCache);
    }
    None
}

/// Finds `compile_commands.json` files inside a skipped build tree (bounded walk).
fn probe_compile_databases(dir: &Path, found: &mut Vec<PathBuf>) {
    let mut queue = std::collections::VecDeque::from([(dir.to_path_buf(), 0usize)]);
    let mut visited = 0usize;
    while let Some((current, depth)) = queue.pop_front() {
        visited += 1;
        if visited > COMPILE_DB_PROBE_DIRECTORIES {
            break;
        }
        let candidate = current.join("compile_commands.json");
        if candidate.is_file() {
            found.push(candidate);
        }
        if depth >= COMPILE_DB_PROBE_DEPTH {
            continue;
        }
        let Ok(entries) = fs::read_dir(&current) else {
            continue;
        };
        let mut subdirs: Vec<PathBuf> = entries
            .flatten()
            .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| !n.eq_ignore_ascii_case("CMakeFiles") && !n.starts_with('.'))
            })
            .collect();
        subdirs.sort();
        for sub in subdirs {
            queue.push_back((sub, depth + 1));
        }
    }
}

fn admits(options: &DiscoveryOptions, lang: LangId) -> bool {
    options.languages.is_empty() || options.languages.contains(&lang)
}

/// Discovers C-family sources and build files under `root` (a directory or one file).
///
/// Symbolic links are never followed; build-output, version-control, hidden and NuGet
/// package directories are skipped (a skipped build tree is still probed for
/// `compile_commands.json`); generated files are set aside; `.h` headers are resolved to
/// C or C++ once the whole tree is known.
pub fn discover(root: &Path, options: &DiscoveryOptions) -> io::Result<Discovery> {
    let meta = fs::metadata(root)?;
    let mut discovery = Discovery {
        root: root.to_path_buf(),
        ..Discovery::default()
    };
    let mut candidates: Vec<(PathBuf, ExtensionClass)> = Vec::new();

    if meta.is_file() {
        discovery.build_files.record(root);
        if let Some(class) = classify_extension(root) {
            candidates.push((root.to_path_buf(), class));
        }
    } else {
        let mut stack: Vec<(PathBuf, usize)> = vec![(root.to_path_buf(), 0)];
        while let Some((dir, depth)) = stack.pop() {
            let entries = match fs::read_dir(&dir) {
                Ok(entries) => entries,
                Err(err) => {
                    discovery.unreadable.push((dir, err.to_string()));
                    continue;
                }
            };
            let mut children: Vec<PathBuf> = Vec::new();
            for entry in entries {
                match entry {
                    Ok(entry) => children.push(entry.path()),
                    Err(err) => discovery.unreadable.push((dir.clone(), err.to_string())),
                }
            }
            children.sort();
            let mut subdirs = Vec::new();
            for path in children {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let entry_meta = match fs::symlink_metadata(&path) {
                    Ok(m) => m,
                    Err(err) => {
                        discovery.unreadable.push((path, err.to_string()));
                        continue;
                    }
                };
                if entry_meta.file_type().is_symlink() {
                    discovery.skipped.push(SkippedEntry {
                        path,
                        kind: SkipKind::Symlink,
                    });
                    continue;
                }
                if entry_meta.is_dir() {
                    if let Some(kind) = skip_kind_for_directory(&path, &name, options) {
                        if kind == SkipKind::BuildOutput {
                            probe_compile_databases(&path, &mut discovery.build_files.compile_databases);
                        }
                        discovery.skipped.push(SkippedEntry { path, kind });
                        continue;
                    }
                    if depth + 1 > options.max_depth {
                        discovery.skipped.push(SkippedEntry {
                            path,
                            kind: SkipKind::DepthLimit,
                        });
                        continue;
                    }
                    subdirs.push(path);
                    continue;
                }
                if !entry_meta.is_file() {
                    continue;
                }
                discovery.build_files.record(&path);
                if name.starts_with('.') {
                    continue;
                }
                if let Some(class) = classify_extension(&path) {
                    candidates.push((path, class));
                }
            }
            for sub in subdirs.into_iter().rev() {
                stack.push((sub, depth + 1));
            }
        }
    }

    // Generated files are set aside before `.h` resolution so they cannot tip the vote.
    let mut admitted: Vec<(PathBuf, ExtensionClass)> = Vec::with_capacity(candidates.len());
    for (path, class) in candidates {
        if !options.include_generated && is_generated(&path, options, &mut discovery.unreadable) {
            discovery.generated_files.push(path);
            continue;
        }
        admitted.push((path, class));
    }

    let all_paths: Vec<PathBuf> = admitted.iter().map(|(p, _)| p.clone()).collect();
    discovery.headers = resolve_header_languages(&all_paths);
    for (path, _) in admitted {
        let Some((lang, role)) = detect_language(&path, &discovery.headers) else {
            continue;
        };
        if admits(options, lang) {
            discovery.files.push(DiscoveredFile { path, lang, role });
        }
    }
    discovery.files.sort_by(|a, b| a.path.cmp(&b.path));
    discovery.generated_files.sort();
    discovery.build_files.sort();
    Ok(discovery)
}

fn is_generated(path: &Path, options: &DiscoveryOptions, unreadable: &mut Vec<(PathBuf, String)>) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if is_generated_file_name(name) {
        return true;
    }
    if !options.inspect_generated_markers {
        return false;
    }
    match read_prefix(path, GENERATED_MARKER_SCAN_BYTES) {
        Ok(head) => has_generated_marker(&head),
        Err(err) => {
            unreadable.push((path.to_path_buf(), err.to_string()));
            false
        }
    }
}
