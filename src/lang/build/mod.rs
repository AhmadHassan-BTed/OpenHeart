//! Build-system awareness for the C family: Clang compilation databases, CMake project
//! files and MSBuild solutions / projects, reduced to the shared vocabulary of this module
//! (include search paths, macro definitions, language standards, diagnostics, path and
//! glob helpers).
//!
//! Every reader keeps what it cannot evaluate explicit: an unresolved variable, a generator
//! expression or an MSBuild property function stays symbolic and is reported, never
//! replaced with a guess.

pub mod compile_db;
pub mod cmake;
pub mod msbuild;

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

// ── Include search paths ─────────────────────────────────────────────────────

/// How an include directory takes part in `#include` lookup (GCC / Clang / MSVC order).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum IncludeKind {
    /// `-iquote`: searched for `#include "..."` only, before every `Angled` directory.
    Quote,
    /// `-I`, `/I`, CMake `target_include_directories`, MSBuild `AdditionalIncludeDirectories`.
    Angled,
    /// `-isystem`, `/external:I`, `-imsvc`, CMake `SYSTEM`: searched after `Angled`.
    System,
    /// `-idirafter`: searched last.
    After,
}

/// One include search directory.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct IncludePath {
    pub path: PathBuf,
    pub kind: IncludeKind,
}

impl IncludePath {
    pub fn new(path: impl Into<PathBuf>, kind: IncludeKind) -> Self {
        Self {
            path: path.into(),
            kind,
        }
    }
}

/// Orders include paths into lookup order (stable within one kind) and drops repeats of a
/// directory, keeping its first, highest-priority occurrence.
pub fn search_order(paths: &[IncludePath]) -> Vec<IncludePath> {
    let mut ordered: Vec<IncludePath> = paths.to_vec();
    ordered.sort_by_key(|p| p.kind);
    let mut seen = HashSet::new();
    ordered.retain(|p| seen.insert(path_key(&p.path)));
    ordered
}

// ── Macro definitions ────────────────────────────────────────────────────────

/// A macro defined on the command line or by a build file (`-DNAME=VALUE`,
/// `target_compile_definitions`, `PreprocessorDefinitions`, `DefineConstants`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MacroDefinition {
    pub name: String,
    /// Parameter names of a function-like macro, `None` for an object-like one. A variadic
    /// parameter is kept verbatim (`...` or `args...`).
    pub params: Option<Vec<String>>,
    /// Replacement text. `None` when spelled without `=` (`-DNAME`), which defines `1`.
    pub value: Option<String>,
}

impl MacroDefinition {
    pub fn object(name: impl Into<String>, value: Option<String>) -> Self {
        Self {
            name: name.into(),
            params: None,
            value,
        }
    }

    /// Parses a command-line definition: `NAME`, `NAME=VALUE`, `NAME=` (empty) or
    /// `NAME(a,b)=BODY`. Returns `None` when the name is not an identifier.
    pub fn parse(spec: &str) -> Option<Self> {
        let spec = spec.trim_start();
        let (head, value) = match spec.find('=') {
            Some(eq) => (&spec[..eq], Some(spec[eq + 1..].to_string())),
            None => (spec.trim_end(), None),
        };
        let head = head.trim_end();
        let (name, params) = match head.find('(') {
            Some(open) => {
                let close = head.rfind(')')?;
                if close < open || close + 1 != head.len() {
                    return None;
                }
                let list = head[open + 1..close].trim();
                let params = if list.is_empty() {
                    Vec::new()
                } else {
                    list.split(',').map(|p| p.trim().to_string()).collect()
                };
                (&head[..open], Some(params))
            }
            None => (head, None),
        };
        if !is_identifier(name) {
            return None;
        }
        Some(Self {
            name: name.to_string(),
            params,
            value,
        })
    }

    /// The replacement list the definition produces (`1` for a bare `-DNAME`).
    pub fn replacement(&self) -> &str {
        self.value.as_deref().unwrap_or("1")
    }

    pub fn is_function_like(&self) -> bool {
        self.params.is_some()
    }
}

/// One `-D` or `-U` in command-line order; later actions override earlier ones.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MacroAction {
    Define(MacroDefinition),
    Undefine(String),
}

/// Applies `actions` in order and returns the definitions left standing, in the order the
/// surviving names were first defined.
pub fn effective_macros(actions: &[MacroAction]) -> Vec<MacroDefinition> {
    let mut order: Vec<String> = Vec::new();
    let mut current: HashMap<String, MacroDefinition> = HashMap::new();
    for action in actions {
        match action {
            MacroAction::Define(def) => {
                if !current.contains_key(&def.name) && !order.contains(&def.name) {
                    order.push(def.name.clone());
                }
                current.insert(def.name.clone(), def.clone());
            }
            MacroAction::Undefine(name) => {
                current.remove(name);
            }
        }
    }
    order
        .into_iter()
        .filter_map(|name| current.remove(&name))
        .collect()
}

/// True for a C identifier (ASCII letters, digits and `_`, plus non-ASCII letters, which
/// GCC, Clang and MSVC accept as extended identifier characters).
pub fn is_identifier(text: &str) -> bool {
    let mut chars = text.chars();
    match chars.next() {
        Some(c) if c == '_' || c.is_ascii_alphabetic() || (!c.is_ascii() && c.is_alphabetic()) => {}
        _ => return false,
    }
    chars.all(|c| c == '_' || c.is_ascii_alphanumeric() || (!c.is_ascii() && c.is_alphanumeric()))
}

// ── Language standards ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StandardFamily {
    C,
    Cpp,
    CSharp,
}

/// A language standard selected by a build file or compiler flag.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LanguageStandard {
    pub family: StandardFamily,
    /// The spelling found in the build input (`c++17`, `gnu11`, `stdcpp20`, `latest`, `7.3`).
    pub spelling: String,
    /// For C and C++ the ISO publication year (2017); for C# `major * 10 + minor` (73 for
    /// 7.3). `None` when the spelling names a moving target (`latest`, `preview`) or a
    /// draft whose final value is not fixed.
    pub version: Option<u16>,
    /// GNU dialect (`gnu++17`, `gnu11`, `CXX_EXTENSIONS ON`).
    pub gnu_extensions: bool,
}

impl LanguageStandard {
    pub fn new(family: StandardFamily, spelling: &str, version: Option<u16>, gnu: bool) -> Self {
        Self {
            family,
            spelling: spelling.to_string(),
            version,
            gnu_extensions: gnu,
        }
    }

    /// Parses the value of GCC / Clang `-std=` (`c++17`, `gnu++2a`, `c11`, `iso9899:1999`).
    pub fn from_gcc_flag(value: &str) -> Option<Self> {
        let v = value.trim().to_ascii_lowercase();
        if let Some(rest) = v.strip_prefix("gnu++") {
            return cxx_version(rest).map(|y| Self::new(StandardFamily::Cpp, &v, y, true));
        }
        if let Some(rest) = v.strip_prefix("c++") {
            return cxx_version(rest).map(|y| Self::new(StandardFamily::Cpp, &v, y, false));
        }
        if let Some(rest) = v.strip_prefix("iso9899:") {
            let year = match rest {
                "1990" => Some(1990),
                "199409" => Some(1994),
                "1999" | "199x" => Some(1999),
                "2011" => Some(2011),
                "2017" | "2018" => Some(2017),
                "2024" => Some(2023),
                _ => return None,
            };
            return Some(Self::new(StandardFamily::C, &v, year, false));
        }
        if let Some(rest) = v.strip_prefix("gnu") {
            return c_version(rest).map(|y| Self::new(StandardFamily::C, &v, y, true));
        }
        if let Some(rest) = v.strip_prefix('c') {
            return c_version(rest).map(|y| Self::new(StandardFamily::C, &v, y, false));
        }
        None
    }

    /// Parses the value of MSVC / clang-cl `/std:` (`c++17`, `c++latest`, `c11`, `clatest`).
    pub fn from_msvc_flag(value: &str) -> Option<Self> {
        let v = value.trim().to_ascii_lowercase();
        if let Some(rest) = v.strip_prefix("c++") {
            let rest = rest.strip_suffix("preview").unwrap_or(rest);
            return cxx_version(rest).map(|y| Self::new(StandardFamily::Cpp, &v, y, false));
        }
        if let Some(rest) = v.strip_prefix('c') {
            return c_version(rest).map(|y| Self::new(StandardFamily::C, &v, y, false));
        }
        None
    }

    /// Parses `CMAKE_CXX_STANDARD` / `CXX_STANDARD` / `cxx_std_NN` values (`17`, `20`).
    pub fn from_cmake(family: StandardFamily, value: &str, extensions: bool) -> Option<Self> {
        let v = value.trim();
        let version = match family {
            StandardFamily::Cpp => cxx_version(v)?,
            StandardFamily::C => c_version(v)?,
            StandardFamily::CSharp => return None,
        };
        let prefix = match (family, extensions) {
            (StandardFamily::Cpp, true) => "gnu++",
            (StandardFamily::Cpp, false) => "c++",
            (_, true) => "gnu",
            (_, false) => "c",
        };
        Some(Self::new(family, &format!("{}{}", prefix, v), version, extensions))
    }

    /// Parses a CMake compile feature naming a standard (`cxx_std_17`, `c_std_11`).
    pub fn from_cmake_feature(feature: &str) -> Option<Self> {
        let f = feature.trim().to_ascii_lowercase();
        if let Some(rest) = f.strip_prefix("cxx_std_") {
            return Self::from_cmake(StandardFamily::Cpp, rest, false);
        }
        if let Some(rest) = f.strip_prefix("c_std_") {
            return Self::from_cmake(StandardFamily::C, rest, false);
        }
        None
    }

    /// Parses a Visual C++ project `LanguageStandard` / `LanguageStandard_C` value
    /// (`stdcpp17`, `stdcpplatest`, `stdc11`). `Default` yields `None` (the toolset default).
    pub fn from_vcxproj(value: &str) -> Option<Self> {
        let v = value.trim().to_ascii_lowercase();
        if let Some(rest) = v.strip_prefix("stdcpp") {
            return cxx_version(rest).map(|y| Self::new(StandardFamily::Cpp, &v, y, false));
        }
        if let Some(rest) = v.strip_prefix("stdc") {
            return c_version(rest).map(|y| Self::new(StandardFamily::C, &v, y, false));
        }
        None
    }

    /// Parses a C# `LangVersion` value (`7.3`, `10.0`, `12`, `latest`, `preview`, `ISO-2`).
    pub fn from_csharp_lang_version(value: &str) -> Option<Self> {
        let v = value.trim();
        let lower = v.to_ascii_lowercase();
        let version = match lower.as_str() {
            "latest" | "latestmajor" | "preview" | "default" => None,
            "iso-1" => Some(10),
            "iso-2" => Some(20),
            _ => {
                let mut parts = lower.split('.');
                let major: u16 = parts.next()?.parse().ok()?;
                let minor: u16 = match parts.next() {
                    Some(m) => m.parse().ok()?,
                    None => 0,
                };
                if parts.next().is_some() || minor > 9 {
                    return None;
                }
                Some(major.checked_mul(10)?.checked_add(minor)?)
            }
        };
        Some(Self::new(StandardFamily::CSharp, v, version, false))
    }

    /// Value of `__cplusplus` this standard defines, when the standard is final.
    pub fn cplusplus_value(&self) -> Option<i64> {
        if self.family != StandardFamily::Cpp {
            return None;
        }
        match self.version? {
            1998 | 2003 => Some(199711),
            2011 => Some(201103),
            2014 => Some(201402),
            2017 => Some(201703),
            2020 => Some(202002),
            2023 => Some(202302),
            _ => None,
        }
    }

    /// Value of `__STDC_VERSION__` this standard defines. `None` when the value is not
    /// fixed; C89/C90 do not define the macro at all (see [`Self::defines_stdc_version`]).
    pub fn stdc_version_value(&self) -> Option<i64> {
        if self.family != StandardFamily::C {
            return None;
        }
        match self.version? {
            1994 => Some(199409),
            1999 => Some(199901),
            2011 => Some(201112),
            2017 => Some(201710),
            2023 => Some(202311),
            _ => None,
        }
    }

    /// `Some(false)` for C89/C90, which predate `__STDC_VERSION__`; `Some(true)` for every
    /// later C standard; `None` for non-C standards or an unknown version.
    pub fn defines_stdc_version(&self) -> Option<bool> {
        if self.family != StandardFamily::C {
            return None;
        }
        match self.version {
            Some(1989) | Some(1990) => Some(false),
            Some(_) => Some(true),
            None => None,
        }
    }
}

/// C++ standard suffix (`17`, `1z`, `latest`) to `Some(year)`, `Some(None)` for a
/// recognized moving target, `None` when unrecognized.
fn cxx_version(suffix: &str) -> Option<Option<u16>> {
    let year = match suffix {
        "98" => 1998,
        "03" => 2003,
        "0x" | "11" => 2011,
        "1y" | "14" => 2014,
        "1z" | "17" => 2017,
        "2a" | "20" => 2020,
        "2b" | "23" => 2023,
        "2c" | "26" => 2026,
        "latest" => return Some(None),
        _ => return None,
    };
    Some(Some(year))
}

/// C standard suffix (`11`, `1x`, `2x`) to `Some(year)`, `Some(None)` for a recognized
/// draft or moving target, `None` when unrecognized.
fn c_version(suffix: &str) -> Option<Option<u16>> {
    let year = match suffix {
        "89" => 1989,
        "90" => 1990,
        "99" | "9x" => 1999,
        "1x" | "11" => 2011,
        "17" | "18" => 2017,
        "2x" | "23" => 2023,
        "2y" | "latest" => return Some(None),
        _ => return None,
    };
    Some(Some(year))
}

// ── Diagnostics ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Severity {
    Info,
    Warning,
    Error,
}

/// A problem found while reading a build file. `line` is 1-based, 0 for the whole file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildDiagnostic {
    pub file: PathBuf,
    pub line: u32,
    pub severity: Severity,
    pub message: String,
}

impl BuildDiagnostic {
    pub fn new(file: &Path, line: u32, severity: Severity, message: impl Into<String>) -> Self {
        Self {
            file: file.to_path_buf(),
            line,
            severity,
            message: message.into(),
        }
    }

    pub fn info(file: &Path, line: u32, message: impl Into<String>) -> Self {
        Self::new(file, line, Severity::Info, message)
    }

    pub fn warning(file: &Path, line: u32, message: impl Into<String>) -> Self {
        Self::new(file, line, Severity::Warning, message)
    }

    pub fn error(file: &Path, line: u32, message: impl Into<String>) -> Self {
        Self::new(file, line, Severity::Error, message)
    }
}

impl fmt::Display for BuildDiagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let level = match self.severity {
            Severity::Info => "info",
            Severity::Warning => "warning",
            Severity::Error => "error",
        };
        if self.line > 0 {
            write!(f, "{}:{}: {}: {}", self.file.display(), self.line, level, self.message)
        } else {
            write!(f, "{}: {}: {}", self.file.display(), level, self.message)
        }
    }
}

// ── Text decoding ────────────────────────────────────────────────────────────

/// Decodes a build file: UTF-8 (with or without BOM) or BOM-marked UTF-16 (older Visual
/// Studio versions save `.sln` / `.vcxproj` files that way). Invalid sequences become
/// U+FFFD rather than failing the whole file.
pub fn decode_text(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(rest).into_owned();
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return decode_utf16(rest, u16::from_le_bytes);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return decode_utf16(rest, u16::from_be_bytes);
    }
    String::from_utf8_lossy(bytes).into_owned()
}

fn decode_utf16(bytes: &[u8], unit: fn([u8; 2]) -> u16) -> String {
    let units: Vec<u16> = bytes.chunks_exact(2).map(|c| unit([c[0], c[1]])).collect();
    String::from_utf16_lossy(&units)
}

/// Largest build file any reader accepts (bytes).
pub const MAX_BUILD_FILE_BYTES: u64 = 64 * 1024 * 1024;

/// Reads and decodes a build file, refusing files above [`MAX_BUILD_FILE_BYTES`].
pub fn read_build_file(path: &Path) -> io::Result<String> {
    let len = fs::metadata(path)?.len();
    if len > MAX_BUILD_FILE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "{} is {} bytes, above the {} byte limit for build files",
                path.display(),
                len,
                MAX_BUILD_FILE_BYTES
            ),
        ));
    }
    Ok(decode_text(&fs::read(path)?))
}

// ── Paths ────────────────────────────────────────────────────────────────────

/// Lexically normalizes `path`: drops `.` components and folds `..` into the preceding
/// normal component. A leading `..` of a relative path is kept; `..` at the root is dropped.
/// The file system is never consulted, so symbolic links are not resolved.
pub fn normalize_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => match out.components().next_back() {
                Some(Component::Normal(_)) => {
                    out.pop();
                }
                Some(Component::RootDir) | Some(Component::Prefix(_)) => {}
                _ => out.push(".."),
            },
            other => out.push(other.as_os_str()),
        }
    }
    if out.as_os_str().is_empty() {
        out.push(".");
    }
    out
}

/// Converts the path separators of a build-file path to the host's: MSBuild and Windows
/// compile databases write `\`, which is an ordinary character on Unix.
pub fn to_native_separators(spec: &str) -> String {
    if cfg!(windows) {
        spec.to_string()
    } else {
        spec.replace('\\', "/")
    }
}

/// Resolves a path written in a build file against `base` and normalizes it.
pub fn resolve_path(base: &Path, spec: &str) -> PathBuf {
    let native = to_native_separators(spec.trim());
    let candidate = Path::new(&native);
    if candidate.is_absolute() {
        normalize_path(candidate)
    } else {
        normalize_path(&base.join(candidate))
    }
}

/// Comparison key for a path: normalized, `/`-separated, and case-folded on Windows,
/// whose file systems are case-insensitive.
pub fn path_key(path: &Path) -> String {
    let text = normalize_path(path).to_string_lossy().replace('\\', "/");
    if cfg!(windows) {
        text.to_lowercase()
    } else {
        text
    }
}

/// Renders `path` with `/` separators (CMake's spelling of paths).
pub fn to_slash_string(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

// ── Globs ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
enum GlobToken {
    Char(char),
    /// `*`: any run of characters within one path component.
    Star,
    /// `?`: exactly one character.
    Question,
    /// `[abc]`, `[a-z]`, `[!a]` / `[^a]` (CMake only).
    Class { negated: bool, ranges: Vec<(char, char)> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum GlobSegment {
    /// `**`: zero or more whole path components.
    Recursive,
    Pattern(Vec<GlobToken>),
}

/// Options shared by MSBuild item globs and CMake `file(GLOB)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GlobOptions {
    /// Compare characters case-insensitively (MSBuild on Windows).
    pub case_insensitive: bool,
    /// Interpret `[...]` as a character class (CMake); otherwise brackets are literal.
    pub char_classes: bool,
}

impl Default for GlobOptions {
    fn default() -> Self {
        Self {
            case_insensitive: cfg!(windows),
            char_classes: false,
        }
    }
}

/// A path glob over `/`- or `\`-separated components with `*`, `?`, `**` and, optionally,
/// character classes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobPattern {
    segments: Vec<GlobSegment>,
    options: GlobOptions,
}

impl GlobPattern {
    pub fn new(pattern: &str, options: GlobOptions) -> Self {
        let mut segments = Vec::new();
        for part in pattern.split(['/', '\\']) {
            if part.is_empty() || part == "." {
                continue;
            }
            if part == "**" {
                if segments.last() != Some(&GlobSegment::Recursive) {
                    segments.push(GlobSegment::Recursive);
                }
                continue;
            }
            segments.push(GlobSegment::Pattern(parse_glob_component(part, options.char_classes)));
        }
        Self { segments, options }
    }

    /// True when the pattern contains any wildcard.
    pub fn has_wildcards(&self) -> bool {
        self.segments.iter().any(|s| match s {
            GlobSegment::Recursive => true,
            GlobSegment::Pattern(tokens) => tokens.iter().any(|t| !matches!(t, GlobToken::Char(_))),
        })
    }

    /// Matches a path relative to the glob's base directory.
    pub fn is_match(&self, relative: &Path) -> bool {
        let comps: Vec<Vec<char>> = relative
            .components()
            .filter_map(|c| match c {
                Component::Normal(s) => Some(s.to_string_lossy().chars().collect()),
                Component::ParentDir => Some(vec!['.', '.']),
                _ => None,
            })
            .collect();
        match_segments(&self.segments, &comps, self.options.case_insensitive)
    }

    /// Splits off the leading wildcard-free components: `(literal prefix, remainder)`.
    fn split_literal_prefix(&self) -> (PathBuf, Vec<GlobSegment>) {
        let mut prefix = PathBuf::new();
        let mut rest_start = self.segments.len();
        for (i, segment) in self.segments.iter().enumerate() {
            match segment {
                GlobSegment::Pattern(tokens) if tokens.iter().all(|t| matches!(t, GlobToken::Char(_))) => {
                    let text: String = tokens
                        .iter()
                        .map(|t| match t {
                            GlobToken::Char(c) => *c,
                            _ => '?',
                        })
                        .collect();
                    prefix.push(text);
                }
                _ => {
                    rest_start = i;
                    break;
                }
            }
        }
        (prefix, self.segments[rest_start..].to_vec())
    }
}

fn parse_glob_component(part: &str, char_classes: bool) -> Vec<GlobToken> {
    let chars: Vec<char> = part.chars().collect();
    let mut tokens = Vec::with_capacity(chars.len());
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '*' => {
                if tokens.last() != Some(&GlobToken::Star) {
                    tokens.push(GlobToken::Star);
                }
                i += 1;
            }
            '?' => {
                tokens.push(GlobToken::Question);
                i += 1;
            }
            '[' if char_classes => match parse_char_class(&chars, i) {
                Some((token, next)) => {
                    tokens.push(token);
                    i = next;
                }
                None => {
                    tokens.push(GlobToken::Char('['));
                    i += 1;
                }
            },
            c => {
                tokens.push(GlobToken::Char(c));
                i += 1;
            }
        }
    }
    tokens
}

/// Parses `[...]` starting at `open`; returns the class and the index after `]`.
fn parse_char_class(chars: &[char], open: usize) -> Option<(GlobToken, usize)> {
    let mut i = open + 1;
    let negated = matches!(chars.get(i), Some('!') | Some('^'));
    if negated {
        i += 1;
    }
    let mut ranges = Vec::new();
    let mut first = true;
    while i < chars.len() {
        let c = chars[i];
        if c == ']' && !first {
            return Some((GlobToken::Class { negated, ranges }, i + 1));
        }
        first = false;
        if i + 2 < chars.len() && chars[i + 1] == '-' && chars[i + 2] != ']' {
            ranges.push((c, chars[i + 2]));
            i += 3;
        } else {
            ranges.push((c, c));
            i += 1;
        }
    }
    None
}

fn chars_equal(a: char, b: char, case_insensitive: bool) -> bool {
    a == b || (case_insensitive && a.to_lowercase().eq(b.to_lowercase()))
}

fn token_matches(token: &GlobToken, c: char, case_insensitive: bool) -> bool {
    match token {
        GlobToken::Char(p) => chars_equal(*p, c, case_insensitive),
        GlobToken::Question => true,
        GlobToken::Star => false,
        GlobToken::Class { negated, ranges } => {
            let lower: Vec<char> = if case_insensitive {
                c.to_lowercase().chain(c.to_uppercase()).collect()
            } else {
                vec![c]
            };
            let hit = ranges
                .iter()
                .any(|(lo, hi)| lower.iter().any(|ch| *lo <= *ch && *ch <= *hi));
            hit != *negated
        }
    }
}

/// Wildcard match of one component, with single-star backtracking.
fn match_component(tokens: &[GlobToken], text: &[char], case_insensitive: bool) -> bool {
    let (mut t, mut s) = (0usize, 0usize);
    let mut backtrack: Option<(usize, usize)> = None;
    while s < text.len() {
        if t < tokens.len() {
            if tokens[t] == GlobToken::Star {
                backtrack = Some((t, s));
                t += 1;
                continue;
            }
            if token_matches(&tokens[t], text[s], case_insensitive) {
                t += 1;
                s += 1;
                continue;
            }
        }
        match backtrack {
            Some((star_t, star_s)) => {
                t = star_t + 1;
                s = star_s + 1;
                backtrack = Some((star_t, star_s + 1));
            }
            None => return false,
        }
    }
    while t < tokens.len() && tokens[t] == GlobToken::Star {
        t += 1;
    }
    t == tokens.len()
}

fn match_segments(segments: &[GlobSegment], comps: &[Vec<char>], case_insensitive: bool) -> bool {
    match segments.split_first() {
        None => comps.is_empty(),
        Some((GlobSegment::Recursive, rest)) => {
            (0..=comps.len()).any(|skip| match_segments(rest, &comps[skip..], case_insensitive))
        }
        Some((GlobSegment::Pattern(tokens), rest)) => match comps.split_first() {
            Some((first, tail)) => {
                match_component(tokens, first, case_insensitive)
                    && match_segments(rest, tail, case_insensitive)
            }
            None => false,
        },
    }
}

/// Files a glob matched, plus the directories that could not be read during the walk.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GlobExpansion {
    pub files: Vec<PathBuf>,
    pub unreadable: Vec<PathBuf>,
}

/// Expands `pattern` (relative to `base`, or absolute) against the file system.
///
/// Only regular files are returned, sorted. Directory symbolic links are never descended
/// (no cycles); `prune` is asked about every directory before it is entered. A
/// wildcard-free pattern yields the one file if it exists.
pub fn expand_glob(
    base: &Path,
    pattern: &str,
    options: GlobOptions,
    prune: &dyn Fn(&Path) -> bool,
) -> GlobExpansion {
    let native = to_native_separators(pattern.trim());
    let anchored = if Path::new(&native).is_absolute() {
        PathBuf::from(&native)
    } else {
        base.join(&native)
    };
    // The root / drive prefix is kept aside; the glob covers the remaining components.
    let mut root = PathBuf::new();
    let mut parts: Vec<String> = Vec::new();
    for component in anchored.components() {
        match component {
            Component::Prefix(_) | Component::RootDir => root.push(component.as_os_str()),
            Component::CurDir => {}
            other => parts.push(other.as_os_str().to_string_lossy().into_owned()),
        }
    }
    let glob = GlobPattern::new(&parts.join("/"), options);
    let (prefix, rest) = glob.split_literal_prefix();
    let start = normalize_path(&root.join(prefix));

    let mut expansion = GlobExpansion::default();
    if rest.is_empty() {
        if start.is_file() {
            expansion.files.push(start);
        }
        return expansion;
    }

    let unbounded = rest.contains(&GlobSegment::Recursive);
    let max_depth = rest.len();
    let mut stack: Vec<(PathBuf, Vec<Vec<char>>)> = vec![(start, Vec::new())];
    while let Some((dir, rel)) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) => {
                expansion.unreadable.push(dir);
                continue;
            }
        };
        let mut children: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
        children.sort();
        for child in children {
            let Some(name) = child.file_name().map(|n| n.to_string_lossy().into_owned()) else {
                continue;
            };
            let meta = match fs::symlink_metadata(&child) {
                Ok(meta) => meta,
                Err(_) => continue,
            };
            let mut child_rel = rel.clone();
            child_rel.push(name.chars().collect());
            if meta.is_dir() {
                if (unbounded || child_rel.len() < max_depth) && !prune(&child) {
                    stack.push((child, child_rel));
                }
                continue;
            }
            let is_file = meta.is_file()
                || (meta.file_type().is_symlink() && fs::metadata(&child).map(|m| m.is_file()).unwrap_or(false));
            if is_file && match_segments(&rest, &child_rel, options.case_insensitive) {
                expansion.files.push(normalize_path(&child));
            }
        }
    }
    expansion.files.sort();
    expansion.files.dedup();
    expansion
}

/// True when `text` contains a glob wildcard (`*` or `?`).
pub fn has_wildcard(text: &str) -> bool {
    text.contains('*') || text.contains('?')
}
