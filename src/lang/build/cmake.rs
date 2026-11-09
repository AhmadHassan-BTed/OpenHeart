//! CMake project reader: a tokenizer for the CMake language and a scoped interpreter for
//! the commands that describe targets.
//!
//! The tokenizer follows the CMake grammar: command invocations spanning several lines,
//! quoted arguments with escapes and line continuations, bracket arguments `[=[...]=]`,
//! line and bracket comments, nested parentheses and legacy unquoted forms.
//!
//! The interpreter walks `add_subdirectory` / `include` with CMake's directory-scope rules
//! and records targets (`add_executable`, `add_library`, `add_custom_target`), their
//! sources, include directories, compile definitions / options / features, link
//! dependencies and install destinations, plus `find_package` / `FetchContent_Declare` /
//! `pkg_check_modules` packages. Variables set by `set` / `list` / `option` /
//! `file(GLOB)` are expanded; anything else — unset variables, `$ENV{}`, generator
//! expressions other than `BUILD_INTERFACE` / `INSTALL_INTERFACE`, results of commands
//! that are not evaluated — stays symbolic and is flagged. `if` / `foreach` / `while` are
//! not evaluated: everything inside them is recorded with `conditional = true`, and a
//! variable assigned in a conditional block keeps the union of its possible values.
//! Bodies of `function` / `macro` are not executed; calls to user functions that create
//! targets are reported.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::lang::build::{
    expand_glob, normalize_path, path_key, read_build_file, resolve_path, to_slash_string,
    BuildDiagnostic, GlobOptions, IncludeKind, IncludePath, LanguageStandard, MacroDefinition,
    StandardFamily,
};
use crate::lang::profile::{
    is_build_output_directory, CPP_HEADER_EXTENSIONS, CPP_SOURCE_EXTENSIONS, C_SOURCE_EXTENSIONS,
};

/// File name of a CMake directory listing.
pub const CMAKE_LISTS_FILE_NAME: &str = "CMakeLists.txt";

/// Maximum `add_subdirectory` nesting.
const MAX_DIRECTORY_DEPTH: usize = 64;
/// Maximum `include()` nesting within one directory.
const MAX_INCLUDE_DEPTH: usize = 32;

/// Extensions CMake appends when a listed source has none (`add_executable(app main)`).
const CMAKE_SOURCE_EXTENSION_PROBES: &[&str] = &[
    "c", "C", "c++", "cc", "cpp", "cxx", "cu", "m", "M", "mm", "h", "hh", "h++", "hm", "hpp",
    "hxx", "in", "txx",
];

// ── Tokenizer ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArgumentKind {
    Unquoted,
    Quoted,
    /// `[[...]]` / `[=[...]=]`: taken literally, never expanded.
    Bracket,
}

/// One argument as written. `raw` keeps escapes and variable references for evaluation;
/// quoted-argument line continuations are already removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CMakeArgument {
    pub raw: String,
    pub kind: ArgumentKind,
    pub line: u32,
}

/// One command invocation. `name` is lower case (CMake command names are case-insensitive).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CMakeCommand {
    pub name: String,
    pub args: Vec<CMakeArgument>,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("line {line}: {message}")]
pub struct CMakeSyntaxError {
    pub line: u32,
    pub message: String,
}

/// Splits CMake source into command invocations.
pub fn tokenize(source: &str) -> Result<Vec<CMakeCommand>, CMakeSyntaxError> {
    let chars: Vec<char> = source.chars().collect();
    let mut lexer = Lexer {
        chars: &chars,
        pos: 0,
        line: 1,
    };
    if lexer.peek() == Some('\u{FEFF}') {
        lexer.pos += 1;
    }
    lexer.file()
}

struct Lexer<'a> {
    chars: &'a [char],
    pos: usize,
    line: u32,
}

impl<'a> Lexer<'a> {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn peek_at(&self, k: usize) -> Option<char> {
        self.chars.get(self.pos + k).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += 1;
        if c == '\n' {
            self.line += 1;
        }
        Some(c)
    }

    fn error(line: u32, message: impl Into<String>) -> CMakeSyntaxError {
        CMakeSyntaxError {
            line,
            message: message.into(),
        }
    }

    fn file(&mut self) -> Result<Vec<CMakeCommand>, CMakeSyntaxError> {
        let mut commands = Vec::new();
        loop {
            self.skip_space_and_comments()?;
            let Some(c) = self.peek() else {
                break;
            };
            if c == '_' || c.is_ascii_alphabetic() {
                commands.push(self.command()?);
            } else {
                return Err(Self::error(
                    self.line,
                    format!("unexpected character '{}' outside a command invocation", c),
                ));
            }
        }
        Ok(commands)
    }

    /// Number of `=` in a bracket opening `[=*[` at `at`, or `None`.
    fn bracket_open_len(&self, at: usize) -> Option<usize> {
        if self.chars.get(at) != Some(&'[') {
            return None;
        }
        let mut k = at + 1;
        while self.chars.get(k) == Some(&'=') {
            k += 1;
        }
        if self.chars.get(k) == Some(&'[') {
            Some(k - at - 1)
        } else {
            None
        }
    }

    /// Consumes a bracket argument / comment starting at `[` and returns its content.
    fn bracket_content(&mut self) -> Result<String, CMakeSyntaxError> {
        let start_line = self.line;
        let eqs = self
            .bracket_open_len(self.pos)
            .ok_or_else(|| Self::error(start_line, "expected a bracket opening"))?;
        self.pos += eqs + 2;
        // A newline right after the opening bracket is not part of the content.
        if self.peek() == Some('\r') && self.peek_at(1) == Some('\n') {
            self.bump();
            self.bump();
        } else if self.peek() == Some('\n') {
            self.bump();
        }
        let mut content = String::new();
        loop {
            match self.peek() {
                None => {
                    return Err(Self::error(start_line, "unterminated bracket argument or comment"));
                }
                Some(']') => {
                    let mut k = self.pos + 1;
                    let mut count = 0;
                    while self.chars.get(k) == Some(&'=') {
                        k += 1;
                        count += 1;
                    }
                    if count == eqs && self.chars.get(k) == Some(&']') {
                        self.pos = k + 1;
                        return Ok(content);
                    }
                    content.push(']');
                    self.pos += 1;
                }
                Some(_) => {
                    if let Some(c) = self.bump() {
                        content.push(c);
                    }
                }
            }
        }
    }

    fn skip_space_and_comments(&mut self) -> Result<(), CMakeSyntaxError> {
        loop {
            match self.peek() {
                Some(c) if c.is_whitespace() => {
                    self.bump();
                }
                Some('#') => self.comment()?,
                _ => return Ok(()),
            }
        }
    }

    fn comment(&mut self) -> Result<(), CMakeSyntaxError> {
        self.pos += 1;
        if self.bracket_open_len(self.pos).is_some() {
            self.bracket_content()?;
            return Ok(());
        }
        while let Some(c) = self.peek() {
            if c == '\n' {
                break;
            }
            self.pos += 1;
        }
        Ok(())
    }

    fn command(&mut self) -> Result<CMakeCommand, CMakeSyntaxError> {
        let line = self.line;
        let start = self.pos;
        while let Some(c) = self.peek() {
            if c == '_' || c.is_ascii_alphanumeric() {
                self.pos += 1;
            } else {
                break;
            }
        }
        let name: String = self.chars[start..self.pos].iter().collect::<String>().to_ascii_lowercase();
        while matches!(self.peek(), Some(' ') | Some('\t')) {
            self.pos += 1;
        }
        if self.peek() != Some('(') {
            return Err(Self::error(line, format!("expected '(' after command name `{}`", name)));
        }
        self.pos += 1;
        let mut args = Vec::new();
        let mut depth = 0usize;
        loop {
            match self.peek() {
                None => {
                    return Err(Self::error(line, format!("unterminated argument list of `{}`", name)));
                }
                Some(c) if c.is_whitespace() => {
                    self.bump();
                }
                Some('#') => self.comment()?,
                Some('(') => {
                    self.pos += 1;
                    depth += 1;
                    args.push(CMakeArgument {
                        raw: "(".to_string(),
                        kind: ArgumentKind::Unquoted,
                        line: self.line,
                    });
                }
                Some(')') => {
                    self.pos += 1;
                    if depth == 0 {
                        break;
                    }
                    depth -= 1;
                    args.push(CMakeArgument {
                        raw: ")".to_string(),
                        kind: ArgumentKind::Unquoted,
                        line: self.line,
                    });
                }
                Some('"') => args.push(self.quoted()?),
                Some('[') if self.bracket_open_len(self.pos).is_some() => {
                    let arg_line = self.line;
                    let raw = self.bracket_content()?;
                    args.push(CMakeArgument {
                        raw,
                        kind: ArgumentKind::Bracket,
                        line: arg_line,
                    });
                }
                Some(_) => args.push(self.unquoted()),
            }
        }
        Ok(CMakeCommand { name, args, line })
    }

    fn quoted(&mut self) -> Result<CMakeArgument, CMakeSyntaxError> {
        let line = self.line;
        self.pos += 1;
        let mut raw = String::new();
        loop {
            match self.bump() {
                None => return Err(Self::error(line, "unterminated quoted argument")),
                Some('"') => break,
                Some('\\') => match self.peek() {
                    // Line continuation: backslash-newline is removed.
                    Some('\n') => {
                        self.bump();
                    }
                    Some('\r') if self.peek_at(1) == Some('\n') => {
                        self.bump();
                        self.bump();
                    }
                    Some(c) => {
                        self.bump();
                        raw.push('\\');
                        raw.push(c);
                    }
                    None => return Err(Self::error(line, "unterminated quoted argument")),
                },
                Some(c) => raw.push(c),
            }
        }
        Ok(CMakeArgument {
            raw,
            kind: ArgumentKind::Quoted,
            line,
        })
    }

    fn unquoted(&mut self) -> CMakeArgument {
        let line = self.line;
        let mut raw = String::new();
        while let Some(c) = self.peek() {
            match c {
                c if c.is_whitespace() => break,
                '(' | ')' | '#' => break,
                '\\' => {
                    self.pos += 1;
                    raw.push('\\');
                    if let Some(n) = self.bump() {
                        raw.push(n);
                    }
                }
                '"' => {
                    // Legacy form `-Da="b c"`: the quoted part stays in the argument.
                    raw.push('"');
                    self.pos += 1;
                    while let Some(q) = self.bump() {
                        raw.push(q);
                        if q == '\\' {
                            if let Some(n) = self.bump() {
                                raw.push(n);
                            }
                            continue;
                        }
                        if q == '"' {
                            break;
                        }
                    }
                }
                '$' if self.peek_at(1) == Some('(') => {
                    // Legacy make-style reference `$(VAR)`.
                    while let Some(q) = self.bump() {
                        raw.push(q);
                        if q == ')' {
                            break;
                        }
                    }
                }
                _ => {
                    raw.push(c);
                    self.pos += 1;
                }
            }
        }
        CMakeArgument {
            raw,
            kind: ArgumentKind::Unquoted,
            line,
        }
    }
}

// ── Project model ────────────────────────────────────────────────────────────

/// Usage-requirement scope of a target item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ItemScope {
    Private,
    Public,
    Interface,
    /// Directory-level settings and the keyword-less `target_link_libraries` signature.
    Unspecified,
}

impl ItemScope {
    /// The item applies when building the target itself.
    pub fn applies_to_self(self) -> bool {
        self != ItemScope::Interface
    }

    /// The item propagates to targets that link this one.
    pub fn propagates(self) -> bool {
        matches!(self, ItemScope::Public | ItemScope::Interface)
    }
}

/// A source, include directory, definition, option or feature of a target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CMakeItem {
    /// Expanded text; unresolved references are kept verbatim (`${GEN_DIR}/x.c`).
    pub text: String,
    /// Resolved path, for sources and include directories without unresolved references.
    pub path: Option<PathBuf>,
    /// `path` exists on disk.
    pub exists: bool,
    pub scope: ItemScope,
    /// `text` contains an unresolved variable reference or generator expression.
    pub symbolic: bool,
    /// Declared inside `if` / `foreach` / `while` or uses a conditionally set variable.
    pub conditional: bool,
    /// `SYSTEM` include directory.
    pub system: bool,
    /// Inherited from `include_directories` / `add_definitions` / `add_compile_options`.
    pub from_directory: bool,
    pub line: u32,
}

/// Configuration filter of a `target_link_libraries` item (`debug` / `optimized`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LinkConfiguration {
    All,
    Debug,
    Optimized,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CMakeLinkItem {
    pub name: String,
    pub scope: ItemScope,
    pub configuration: LinkConfiguration,
    pub symbolic: bool,
    pub conditional: bool,
    pub from_directory: bool,
    pub line: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CMakeTargetKind {
    Executable,
    StaticLibrary,
    SharedLibrary,
    /// `MODULE`: a plugin loaded at run time, never linked.
    ModuleLibrary,
    ObjectLibrary,
    /// Header-only / usage-requirements-only library.
    InterfaceLibrary,
    /// `add_library` without a type while `BUILD_SHARED_LIBS` is not known.
    StaticOrSharedLibrary,
    /// `IMPORTED UNKNOWN` library.
    UnknownLibrary,
    /// `add_custom_target`.
    Custom,
}

/// One `install(TARGETS ...)` destination.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallRule {
    /// Artifact kind keyword (`RUNTIME`, `LIBRARY`, `ARCHIVE`, ...), `None` for all kinds.
    pub kind: Option<String>,
    pub destination: String,
    pub symbolic: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CMakeTarget {
    pub name: String,
    pub kind: CMakeTargetKind,
    pub imported: bool,
    /// `ALIAS` target: the aliased target's name.
    pub alias_of: Option<String>,
    /// `CMAKE_CURRENT_SOURCE_DIR` of the declaration.
    pub directory: PathBuf,
    pub list_file: PathBuf,
    pub line: u32,
    pub conditional: bool,
    pub win32_executable: bool,
    pub macos_bundle: bool,
    pub exclude_from_all: bool,
    pub sources: Vec<CMakeItem>,
    pub include_directories: Vec<CMakeItem>,
    pub compile_definitions: Vec<CMakeItem>,
    pub compile_options: Vec<CMakeItem>,
    pub compile_features: Vec<CMakeItem>,
    pub link_libraries: Vec<CMakeLinkItem>,
    /// `add_dependencies`: build-order dependencies.
    pub build_dependencies: Vec<String>,
    /// Target properties set by `set_target_properties` / `set_property(TARGET ...)`.
    pub properties: BTreeMap<String, String>,
    pub output_name: Option<String>,
    pub c_standard: Option<LanguageStandard>,
    pub cxx_standard: Option<LanguageStandard>,
    pub install_rules: Vec<InstallRule>,
}

impl CMakeTarget {
    fn new(name: String, kind: CMakeTargetKind, scope: &DirScope, line: u32, conditional: bool) -> Self {
        Self {
            name,
            kind,
            imported: false,
            alias_of: None,
            directory: scope.source_dir.clone(),
            list_file: scope.list_file.clone(),
            line,
            conditional,
            win32_executable: false,
            macos_bundle: false,
            exclude_from_all: false,
            sources: Vec::new(),
            include_directories: Vec::new(),
            compile_definitions: Vec::new(),
            compile_options: Vec::new(),
            compile_features: Vec::new(),
            link_libraries: Vec::new(),
            build_dependencies: Vec::new(),
            properties: BTreeMap::new(),
            output_name: None,
            c_standard: None,
            cxx_standard: None,
            install_rules: Vec::new(),
        }
    }

    /// True for targets that produce a linkable or loadable binary.
    pub fn produces_binary(&self) -> bool {
        !self.imported
            && self.alias_of.is_none()
            && !matches!(
                self.kind,
                CMakeTargetKind::InterfaceLibrary | CMakeTargetKind::Custom | CMakeTargetKind::ObjectLibrary
            )
    }

    /// Resolved source paths (symbolic sources excluded).
    pub fn source_paths(&self) -> Vec<PathBuf> {
        self.sources.iter().filter_map(|s| s.path.clone()).collect()
    }

    /// Compile definitions applying to the target itself, parsed.
    pub fn own_definitions(&self) -> Vec<MacroDefinition> {
        self.compile_definitions
            .iter()
            .filter(|d| d.scope.applies_to_self() && !d.symbolic)
            .filter_map(|d| MacroDefinition::parse(&d.text))
            .collect()
    }
}

/// Where a package comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackageOrigin {
    FindPackage { mode: Option<String> },
    FetchContent { repository: Option<String>, tag: Option<String> },
    PkgConfig { modules: Vec<String> },
}

/// An external package the project uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CMakePackage {
    pub name: String,
    pub version: Option<String>,
    pub required: bool,
    pub components: Vec<String>,
    pub origin: PackageOrigin,
    pub conditional: bool,
    pub list_file: PathBuf,
    pub line: u32,
}

/// An `option()` declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CMakeOption {
    pub name: String,
    pub description: String,
    pub default: Option<String>,
    pub list_file: PathBuf,
    pub line: u32,
}

/// A `project()` declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CMakeProjectDecl {
    pub name: String,
    pub version: Option<String>,
    pub languages: Vec<String>,
    pub directory: PathBuf,
}

/// How a `target_link_libraries` item resolves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkTarget {
    /// A target of this project (index into `CMakeProject::targets`, aliases followed).
    Target(usize),
    /// An imported target of a package (`Qt6::Widgets` → `Qt6`, component `Widgets`).
    Package { package: String, component: Option<String> },
    /// A library found by the linker (`pthread`, `-lz`, `ws2_32`).
    SystemLibrary(String),
    /// A library given by path.
    File(PathBuf),
    /// A linker flag (`-Wl,--as-needed`, `-pthread`).
    LinkerFlag(String),
    /// Contains unresolved references.
    Symbolic(String),
}

/// Include paths and definitions in effect when compiling a target, including the
/// `PUBLIC` / `INTERFACE` requirements of every target it links (transitively).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UsageRequirements {
    pub include_paths: Vec<IncludePath>,
    pub definitions: Vec<MacroDefinition>,
    /// Items that could not be resolved (symbolic text).
    pub unresolved: Vec<String>,
    /// Some contributing items are conditional.
    pub conditional: bool,
}

#[derive(Debug, Error)]
pub enum CMakeError {
    #[error("failed to read '{path}': {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("'{path}': {source}")]
    Syntax {
        path: PathBuf,
        #[source]
        source: CMakeSyntaxError,
    },
}

/// A CMake project: every target reachable from the top-level `CMakeLists.txt`.
#[derive(Debug, Clone, Default)]
pub struct CMakeProject {
    pub root_dir: PathBuf,
    /// Name of the first `project()`.
    pub name: Option<String>,
    pub version: Option<String>,
    pub minimum_version: Option<String>,
    pub languages: Vec<String>,
    pub declarations: Vec<CMakeProjectDecl>,
    /// Every `CMakeLists.txt` and included `.cmake` file processed.
    pub list_files: Vec<PathBuf>,
    pub targets: Vec<CMakeTarget>,
    pub packages: Vec<CMakePackage>,
    pub options: Vec<CMakeOption>,
    /// `include(<module>)` names that are CMake modules rather than project files.
    pub included_modules: Vec<String>,
    pub diagnostics: Vec<BuildDiagnostic>,
}

impl CMakeProject {
    /// Reads the project whose top-level `CMakeLists.txt` is in `root` (a directory, or the
    /// `CMakeLists.txt` path itself).
    pub fn load(root: &Path) -> Result<Self, CMakeError> {
        let root_dir = if root.is_file() {
            root.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."))
        } else {
            root.to_path_buf()
        };
        let root_dir = normalize_path(&root_dir);
        let mut reader = Reader::new(root_dir.clone());
        let mut vars = HashMap::new();
        let root_text = to_slash_string(&root_dir);
        for name in ["CMAKE_SOURCE_DIR", "CMAKE_HOME_DIRECTORY"] {
            vars.insert(name.to_string(), VarValue::known(&root_text));
        }
        reader.process_directory(root_dir, vars, DirectorySettings::default(), false, 0)?;
        reader.finish();
        Ok(reader.project)
    }

    /// Index of the target named `name`, following `ALIAS` targets.
    pub fn target_index(&self, name: &str) -> Option<usize> {
        let mut current = name.to_string();
        for _ in 0..8 {
            let index = self.targets.iter().position(|t| t.name == current)?;
            match &self.targets[index].alias_of {
                Some(aliased) => current = aliased.clone(),
                None => return Some(index),
            }
        }
        None
    }

    pub fn target(&self, name: &str) -> Option<&CMakeTarget> {
        self.target_index(name).map(|i| &self.targets[i])
    }

    /// Classifies a link item against this project's targets and packages.
    pub fn classify_link(&self, item: &CMakeLinkItem) -> LinkTarget {
        let name = item.name.trim();
        if item.symbolic {
            return LinkTarget::Symbolic(name.to_string());
        }
        if let Some(index) = self.target_index(name) {
            return LinkTarget::Target(index);
        }
        if let Some(lib) = name.strip_prefix("-l") {
            return LinkTarget::SystemLibrary(lib.to_string());
        }
        if name.starts_with('-') {
            return LinkTarget::LinkerFlag(name.to_string());
        }
        if let Some((package, component)) = name.split_once("::") {
            return LinkTarget::Package {
                package: package.to_string(),
                component: Some(component.to_string()).filter(|c| !c.is_empty()),
            };
        }
        if self
            .packages
            .iter()
            .any(|p| matches!(p.origin, PackageOrigin::FetchContent { .. }) && p.name.eq_ignore_ascii_case(name))
        {
            return LinkTarget::Package {
                package: name.to_string(),
                component: None,
            };
        }
        let lower = name.to_ascii_lowercase();
        let library_file = [".a", ".lib", ".so", ".dylib", ".dll", ".tbd"]
            .iter()
            .any(|ext| lower.ends_with(ext))
            || lower.contains(".so.")
            || lower.ends_with(".framework");
        if name.contains('/') || name.contains('\\') || library_file {
            return LinkTarget::File(PathBuf::from(name));
        }
        LinkTarget::SystemLibrary(name.to_string())
    }

    /// Include paths and definitions for compiling `target` (CMake usage requirements).
    pub fn usage_requirements(&self, target: &str) -> Option<UsageRequirements> {
        let start = self.target_index(target)?;
        let mut req = UsageRequirements::default();
        let mut seen_dirs = HashSet::new();
        let mut seen_defs = HashSet::new();
        self.collect_requirements(start, true, &mut req, &mut seen_dirs, &mut seen_defs);

        let mut visited = HashSet::from([start]);
        let mut queue: Vec<usize> = self.linked_targets(start, false);
        while let Some(dep) = queue.pop() {
            if !visited.insert(dep) {
                continue;
            }
            self.collect_requirements(dep, false, &mut req, &mut seen_dirs, &mut seen_defs);
            queue.extend(self.linked_targets(dep, true));
        }
        Some(req)
    }

    /// Internal targets linked by `index`; with `propagating_only`, only through
    /// `PUBLIC` / `INTERFACE` / keyword-less links.
    fn linked_targets(&self, index: usize, propagating_only: bool) -> Vec<usize> {
        self.targets[index]
            .link_libraries
            .iter()
            .filter(|l| !propagating_only || l.scope.propagates() || l.scope == ItemScope::Unspecified)
            .filter_map(|l| match self.classify_link(l) {
                LinkTarget::Target(t) => Some(t),
                _ => None,
            })
            .collect()
    }

    fn collect_requirements(
        &self,
        index: usize,
        own: bool,
        req: &mut UsageRequirements,
        seen_dirs: &mut HashSet<String>,
        seen_defs: &mut HashSet<String>,
    ) {
        let target = &self.targets[index];
        let wanted = |scope: ItemScope| if own { scope.applies_to_self() } else { scope.propagates() };
        for dir in target.include_directories.iter().filter(|d| wanted(d.scope)) {
            req.conditional |= dir.conditional;
            match (&dir.path, dir.symbolic) {
                (Some(path), false) => {
                    if seen_dirs.insert(path_key(path)) {
                        let kind = if dir.system {
                            IncludeKind::System
                        } else {
                            IncludeKind::Angled
                        };
                        req.include_paths.push(IncludePath::new(path.clone(), kind));
                    }
                }
                _ => req.unresolved.push(dir.text.clone()),
            }
        }
        for def in target.compile_definitions.iter().filter(|d| wanted(d.scope)) {
            req.conditional |= def.conditional;
            if def.symbolic {
                req.unresolved.push(def.text.clone());
                continue;
            }
            match MacroDefinition::parse(&def.text) {
                Some(parsed) => {
                    if seen_defs.insert(parsed.name.clone()) {
                        req.definitions.push(parsed);
                    }
                }
                None => req.unresolved.push(def.text.clone()),
            }
        }
    }

    /// The standard a target compiles with for `family`: the target property, else the
    /// highest `cxx_std_NN` / `c_std_NN` compile feature applying to it.
    pub fn standard_for(&self, target: &CMakeTarget, family: StandardFamily) -> Option<LanguageStandard> {
        let explicit = match family {
            StandardFamily::Cpp => target.cxx_standard.clone(),
            StandardFamily::C => target.c_standard.clone(),
            StandardFamily::CSharp => return None,
        };
        let from_features = target
            .compile_features
            .iter()
            .filter(|f| f.scope.applies_to_self() && !f.symbolic)
            .filter_map(|f| LanguageStandard::from_cmake_feature(&f.text))
            .filter(|s| s.family == family)
            .max_by_key(|s| s.version.unwrap_or(u16::MAX));
        match (explicit, from_features) {
            (Some(e), Some(f)) => {
                if f.version.unwrap_or(u16::MAX) > e.version.unwrap_or(u16::MAX) {
                    Some(f)
                } else {
                    Some(e)
                }
            }
            (Some(e), None) => Some(e),
            (None, f) => f,
        }
    }
}

// ── Interpreter ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
enum VarValue {
    Known { value: String, conditional: bool },
    /// Assigned by a command this reader does not evaluate.
    Unknown,
}

impl VarValue {
    fn known(value: &str) -> Self {
        VarValue::Known {
            value: value.to_string(),
            conditional: false,
        }
    }
}

#[derive(Debug, Clone, Default)]
struct DirectorySettings {
    include_dirs: Vec<CMakeItem>,
    definitions: Vec<CMakeItem>,
    compile_options: Vec<CMakeItem>,
    link_libraries: Vec<CMakeLinkItem>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Block {
    If,
    Foreach { var: Option<String>, saved: Option<VarValue> },
    While,
    Plain,
}

impl Block {
    fn is_conditional(&self) -> bool {
        !matches!(self, Block::Plain)
    }
}

struct DirScope {
    vars: HashMap<String, VarValue>,
    source_dir: PathBuf,
    list_file: PathBuf,
    settings: DirectorySettings,
    blocks: Vec<Block>,
    /// Inherited from a conditional `add_subdirectory`.
    base_conditional: bool,
    targets_in_dir: Vec<usize>,
    parent_sets: Vec<(String, Option<VarValue>)>,
    include_depth: usize,
}

impl DirScope {
    fn conditional(&self) -> bool {
        self.base_conditional || self.blocks.iter().any(Block::is_conditional)
    }

    fn known(&self, name: &str) -> Option<String> {
        match self.vars.get(name) {
            Some(VarValue::Known { value, conditional: false }) => Some(value.clone()),
            _ => None,
        }
    }
}

/// An expanded argument item.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Item {
    /// Expanded text; `\;` escapes are kept until the item is used as a final value.
    text: String,
    symbolic: bool,
    conditional: bool,
    line: u32,
}

impl Item {
    fn value(&self) -> String {
        self.text.replace("\\;", ";")
    }
}

struct FunctionInfo {
    creates_targets: bool,
}

#[derive(Debug, Clone)]
enum TargetOp {
    Sources(Vec<CMakeItem>),
    IncludeDirs(Vec<CMakeItem>),
    Definitions(Vec<CMakeItem>),
    Options(Vec<CMakeItem>),
    Features(Vec<CMakeItem>),
    Links(Vec<CMakeLinkItem>),
    Dependencies(Vec<String>),
    Property { name: String, value: String, append: bool, scope: Box<DirScopeSnapshot> },
    Install(InstallRule),
}

/// The parts of a scope a deferred property operation needs.
#[derive(Debug, Clone)]
struct DirScopeSnapshot {
    source_dir: PathBuf,
    conditional: bool,
    line: u32,
}

struct PendingOp {
    target: String,
    op: TargetOp,
    list_file: PathBuf,
    line: u32,
    command: String,
}

struct Reader {
    project: CMakeProject,
    functions: HashMap<String, FunctionInfo>,
    visited_dirs: HashSet<String>,
    pending: Vec<PendingOp>,
}

/// CMake truthiness of a constant (`ON`, `1`, `TRUE`, `NO`, `x-NOTFOUND`, ...). `None` for
/// strings that are neither true nor false constants.
pub fn cmake_truth(value: &str) -> Option<bool> {
    let v = value.trim();
    let upper = v.to_ascii_uppercase();
    match upper.as_str() {
        "1" | "ON" | "YES" | "TRUE" | "Y" => Some(true),
        "0" | "OFF" | "NO" | "FALSE" | "N" | "IGNORE" | "NOTFOUND" | "" => Some(false),
        _ if upper.ends_with("-NOTFOUND") => Some(false),
        _ => v.parse::<f64>().ok().map(|n| n != 0.0),
    }
}

/// Splits a CMake list on unescaped `;`, dropping empty elements.
fn split_list(text: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut current = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' && chars.peek() == Some(&';') {
            current.push_str("\\;");
            chars.next();
            continue;
        }
        if c == ';' {
            if !current.is_empty() {
                items.push(std::mem::take(&mut current));
            }
            continue;
        }
        current.push(c);
    }
    if !current.is_empty() {
        items.push(current);
    }
    items
}

fn has_unresolved(text: &str) -> bool {
    text.contains("${") || text.contains("$<") || text.contains("$ENV{")
}

/// End index (exclusive) of a `${...}` / `$X{...}` reference whose `{` is at `open`.
fn matching_brace(chars: &[char], open: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut i = open;
    while i < chars.len() {
        match chars[i] {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i + 1);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// End index (exclusive) of a generator expression starting with `$<` at `start`.
fn matching_genex(chars: &[char], start: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut i = start;
    while i < chars.len() {
        if chars[i] == '$' && chars.get(i + 1) == Some(&'<') {
            depth += 1;
            i += 2;
            continue;
        }
        if chars[i] == '>' {
            depth -= 1;
            if depth == 0 {
                return Some(i + 1);
            }
        }
        i += 1;
    }
    None
}

const INSTALL_KEYWORDS: &[&str] = &[
    "EXPORT", "ARCHIVE", "LIBRARY", "RUNTIME", "OBJECTS", "FRAMEWORK", "BUNDLE", "PRIVATE_HEADER",
    "PUBLIC_HEADER", "RESOURCE", "FILE_SET", "CXX_MODULES_BMI", "DESTINATION", "INCLUDES",
    "PERMISSIONS", "CONFIGURATIONS", "COMPONENT", "NAMELINK_COMPONENT", "OPTIONAL",
    "EXCLUDE_FROM_ALL", "NAMELINK_ONLY", "NAMELINK_SKIP", "RUNTIME_DEPENDENCIES",
    "RUNTIME_DEPENDENCY_SET",
];

const INSTALL_ARTIFACT_KINDS: &[&str] = &[
    "ARCHIVE", "LIBRARY", "RUNTIME", "OBJECTS", "FRAMEWORK", "BUNDLE", "PRIVATE_HEADER",
    "PUBLIC_HEADER", "RESOURCE", "FILE_SET", "CXX_MODULES_BMI",
];

impl Reader {
    fn new(root_dir: PathBuf) -> Self {
        Self {
            project: CMakeProject {
                root_dir,
                ..CMakeProject::default()
            },
            functions: HashMap::new(),
            visited_dirs: HashSet::new(),
            pending: Vec::new(),
        }
    }

    fn diag_warning(&mut self, file: &Path, line: u32, message: impl Into<String>) {
        self.project
            .diagnostics
            .push(BuildDiagnostic::warning(file, line, message));
    }

    fn diag_info(&mut self, file: &Path, line: u32, message: impl Into<String>) {
        self.project
            .diagnostics
            .push(BuildDiagnostic::info(file, line, message));
    }

    fn read_commands(path: &Path) -> Result<Vec<CMakeCommand>, CMakeError> {
        let text = read_build_file(path).map_err(|source| CMakeError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        tokenize(&text).map_err(|source| CMakeError::Syntax {
            path: path.to_path_buf(),
            source,
        })
    }

    fn process_directory(
        &mut self,
        source_dir: PathBuf,
        mut vars: HashMap<String, VarValue>,
        settings: DirectorySettings,
        base_conditional: bool,
        depth: usize,
    ) -> Result<Vec<(String, Option<VarValue>)>, CMakeError> {
        let list_file = source_dir.join(CMAKE_LISTS_FILE_NAME);
        let commands = Self::read_commands(&list_file)?;
        self.visited_dirs.insert(path_key(&source_dir));
        self.project.list_files.push(list_file.clone());

        let dir_text = to_slash_string(&source_dir);
        vars.insert("CMAKE_CURRENT_SOURCE_DIR".to_string(), VarValue::known(&dir_text));
        vars.insert("CMAKE_CURRENT_LIST_DIR".to_string(), VarValue::known(&dir_text));
        vars.insert(
            "CMAKE_CURRENT_LIST_FILE".to_string(),
            VarValue::known(&to_slash_string(&list_file)),
        );
        let mut scope = DirScope {
            vars,
            source_dir,
            list_file,
            settings,
            blocks: Vec::new(),
            base_conditional,
            targets_in_dir: Vec::new(),
            parent_sets: Vec::new(),
            include_depth: 0,
        };
        self.run(&commands, &mut scope, depth);
        if !scope.blocks.is_empty() {
            let file = scope.list_file.clone();
            self.diag_warning(&file, 0, format!("{} block(s) are not closed at the end of the file", scope.blocks.len()));
        }

        // Directory-level settings apply to every target of this directory.
        for &index in &scope.targets_in_dir {
            let target = &mut self.project.targets[index];
            prepend(&mut target.include_directories, &scope.settings.include_dirs);
            prepend(&mut target.compile_definitions, &scope.settings.definitions);
            prepend(&mut target.compile_options, &scope.settings.compile_options);
            let mut links = scope.settings.link_libraries.clone();
            links.append(&mut target.link_libraries);
            target.link_libraries = links;
        }
        Ok(scope.parent_sets)
    }

    fn run(&mut self, commands: &[CMakeCommand], scope: &mut DirScope, depth: usize) {
        let mut idx = 0;
        while idx < commands.len() {
            let cmd = &commands[idx];
            idx += 1;
            match cmd.name.as_str() {
                "function" | "macro" => {
                    let end_name = if cmd.name == "function" { "endfunction" } else { "endmacro" };
                    let mut nesting = 0usize;
                    let mut creates_targets = false;
                    let mut closed = false;
                    while idx < commands.len() {
                        let inner = &commands[idx];
                        idx += 1;
                        if inner.name == cmd.name {
                            nesting += 1;
                        } else if inner.name == end_name {
                            if nesting == 0 {
                                closed = true;
                                break;
                            }
                            nesting -= 1;
                        } else if matches!(inner.name.as_str(), "add_library" | "add_executable" | "add_custom_target") {
                            creates_targets = true;
                        } else if let Some(info) = self.functions.get(&inner.name) {
                            creates_targets |= info.creates_targets;
                        }
                    }
                    if !closed {
                        let file = scope.list_file.clone();
                        self.diag_warning(&file, cmd.line, format!("`{}` without matching `{}`", cmd.name, end_name));
                    }
                    if let Some(first) = cmd.args.first() {
                        let name = self.expand_arg(scope, first).first().map(|i| i.text.to_ascii_lowercase());
                        if let Some(name) = name {
                            self.functions.insert(name, FunctionInfo { creates_targets });
                        }
                    }
                }
                "if" => scope.blocks.push(Block::If),
                "elseif" | "else" => {
                    if scope.blocks.last() != Some(&Block::If) {
                        let file = scope.list_file.clone();
                        self.diag_warning(&file, cmd.line, format!("`{}` outside an `if` block", cmd.name));
                    }
                }
                "endif" => self.close_block(scope, cmd, |b| matches!(b, Block::If)),
                "foreach" => {
                    let var = cmd
                        .args
                        .first()
                        .and_then(|a| self.expand_arg(scope, a).into_iter().next())
                        .map(|i| i.text);
                    let saved = var.as_ref().and_then(|v| scope.vars.get(v).cloned());
                    if let Some(v) = &var {
                        scope.vars.insert(v.clone(), VarValue::Unknown);
                    }
                    scope.blocks.push(Block::Foreach { var, saved });
                }
                "endforeach" => {
                    if let Some(Block::Foreach { var, saved }) = scope.blocks.last().cloned() {
                        scope.blocks.pop();
                        if let Some(v) = var {
                            match saved {
                                Some(value) => {
                                    scope.vars.insert(v, value);
                                }
                                None => {
                                    scope.vars.remove(&v);
                                }
                            }
                        }
                    } else {
                        let file = scope.list_file.clone();
                        self.diag_warning(&file, cmd.line, "`endforeach` without matching `foreach`");
                    }
                }
                "while" => scope.blocks.push(Block::While),
                "endwhile" => self.close_block(scope, cmd, |b| matches!(b, Block::While)),
                "block" => scope.blocks.push(Block::Plain),
                "endblock" => self.close_block(scope, cmd, |b| matches!(b, Block::Plain)),
                "return" => {
                    if !scope.conditional() {
                        break;
                    }
                }
                _ => self.execute(cmd, scope, depth),
            }
        }
    }

    fn close_block(&mut self, scope: &mut DirScope, cmd: &CMakeCommand, fits: fn(&Block) -> bool) {
        match scope.blocks.last() {
            Some(b) if fits(b) => {
                scope.blocks.pop();
            }
            _ => {
                let file = scope.list_file.clone();
                self.diag_warning(&file, cmd.line, format!("`{}` without a matching opening command", cmd.name));
            }
        }
    }

    // ── Expansion ──

    /// Expands escapes, variable references and generator expressions in `raw`.
    /// Returns `(text, symbolic, conditional)`.
    fn expand_text(&self, scope: &DirScope, raw: &str) -> (String, bool, bool) {
        let chars: Vec<char> = raw.chars().collect();
        let mut out = String::with_capacity(raw.len());
        let mut symbolic = false;
        let mut conditional = false;
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            if c == '\\' && i + 1 < chars.len() {
                let next = chars[i + 1];
                match next {
                    't' => out.push('\t'),
                    'n' => out.push('\n'),
                    'r' => out.push('\r'),
                    ';' => out.push_str("\\;"),
                    n if n.is_ascii_alphanumeric() => {
                        out.push('\\');
                        out.push(n);
                    }
                    n => out.push(n),
                }
                i += 2;
                continue;
            }
            if c == '$' {
                let rest: String = chars[i + 1..chars.len().min(i + 8)].iter().collect();
                if rest.starts_with('{') {
                    if let Some(end) = matching_brace(&chars, i + 1) {
                        let inner: String = chars[i + 2..end - 1].iter().collect();
                        let (name, inner_symbolic, inner_conditional) = self.expand_text(scope, &inner);
                        conditional |= inner_conditional;
                        match (inner_symbolic, scope.vars.get(&name)) {
                            (false, Some(VarValue::Known { value, conditional: c })) => {
                                out.push_str(value);
                                conditional |= *c;
                                symbolic |= has_unresolved(value);
                            }
                            _ => {
                                out.push_str("${");
                                out.push_str(&name);
                                out.push('}');
                                symbolic = true;
                            }
                        }
                        i = end;
                        continue;
                    }
                } else if rest.starts_with("CACHE{") {
                    if let Some(end) = matching_brace(&chars, i + 6) {
                        let inner: String = chars[i + 7..end - 1].iter().collect();
                        match scope.vars.get(&inner) {
                            Some(VarValue::Known { value, conditional: c }) => {
                                out.push_str(value);
                                conditional |= *c;
                            }
                            _ => {
                                out.push_str("${");
                                out.push_str(&inner);
                                out.push('}');
                                symbolic = true;
                            }
                        }
                        i = end;
                        continue;
                    }
                } else if rest.starts_with("ENV{") {
                    if let Some(end) = matching_brace(&chars, i + 4) {
                        let text: String = chars[i..end].iter().collect();
                        out.push_str(&text);
                        symbolic = true;
                        i = end;
                        continue;
                    }
                } else if rest.starts_with('<') {
                    if let Some(end) = matching_genex(&chars, i) {
                        let inner: String = chars[i + 2..end - 1].iter().collect();
                        let (expanded, inner_symbolic, inner_conditional) = self.expand_text(scope, &inner);
                        conditional |= inner_conditional;
                        if let Some(body) = expanded
                            .strip_prefix("BUILD_INTERFACE:")
                            .or_else(|| expanded.strip_prefix("BUILD_LOCAL_INTERFACE:"))
                        {
                            out.push_str(body);
                            symbolic |= inner_symbolic;
                        } else if expanded.starts_with("INSTALL_INTERFACE:") {
                            // Install-tree-only value: not part of the build.
                        } else {
                            out.push_str("$<");
                            out.push_str(&expanded);
                            out.push('>');
                            symbolic = true;
                        }
                        i = end;
                        continue;
                    }
                }
            }
            out.push(c);
            i += 1;
        }
        (out, symbolic, conditional)
    }

    /// Expands one argument into items (unquoted arguments split into list elements).
    fn expand_arg(&self, scope: &DirScope, arg: &CMakeArgument) -> Vec<Item> {
        let base_conditional = scope.conditional();
        match arg.kind {
            ArgumentKind::Bracket => vec![Item {
                text: arg.raw.clone(),
                symbolic: false,
                conditional: base_conditional,
                line: arg.line,
            }],
            ArgumentKind::Quoted => {
                let (text, symbolic, conditional) = self.expand_text(scope, &arg.raw);
                vec![Item {
                    text,
                    symbolic,
                    conditional: conditional || base_conditional,
                    line: arg.line,
                }]
            }
            ArgumentKind::Unquoted => {
                let (text, symbolic, conditional) = self.expand_text(scope, &arg.raw);
                split_list(&text)
                    .into_iter()
                    .map(|t| Item {
                        symbolic: symbolic && has_unresolved(&t),
                        text: t,
                        conditional: conditional || base_conditional,
                        line: arg.line,
                    })
                    .collect()
            }
        }
    }

    fn expand_args(&self, scope: &DirScope, args: &[CMakeArgument]) -> Vec<Item> {
        args.iter().flat_map(|a| self.expand_arg(scope, a)).collect()
    }

    // ── Variables ──

    fn set_var(&mut self, scope: &mut DirScope, name: &str, values: &[Item], parent_scope: bool) {
        let conditional_ctx = scope.conditional();
        let item_conditional = values.iter().any(|v| v.conditional && !conditional_ctx);
        let new_value = values.iter().map(|v| v.text.as_str()).collect::<Vec<_>>().join(";");
        let target_map_value = |existing: Option<&VarValue>| -> Option<VarValue> {
            if values.is_empty() && !conditional_ctx {
                return None;
            }
            if conditional_ctx {
                // Union of the possible values: the variable may keep its old value.
                let mut merged: Vec<String> = match existing {
                    Some(VarValue::Known { value, .. }) => split_list(value),
                    Some(VarValue::Unknown) => return Some(VarValue::Unknown),
                    None => Vec::new(),
                };
                for item in split_list(&new_value) {
                    if !merged.contains(&item) {
                        merged.push(item);
                    }
                }
                return Some(VarValue::Known {
                    value: merged.join(";"),
                    conditional: true,
                });
            }
            Some(VarValue::Known {
                value: new_value.clone(),
                conditional: item_conditional,
            })
        };
        if parent_scope {
            let value = target_map_value(None);
            scope.parent_sets.push((name.to_string(), value));
            return;
        }
        let value = target_map_value(scope.vars.get(name));
        match value {
            Some(v) => {
                scope.vars.insert(name.to_string(), v);
            }
            None => {
                scope.vars.remove(name);
            }
        }
    }

    fn mark_unknown(scope: &mut DirScope, names: &[String]) {
        for name in names {
            if !name.is_empty() && !has_unresolved(name) {
                scope.vars.insert(name.clone(), VarValue::Unknown);
            }
        }
    }

    // ── Command dispatch ──

    fn execute(&mut self, cmd: &CMakeCommand, scope: &mut DirScope, depth: usize) {
        let items = self.expand_args(scope, &cmd.args);
        let conditional = scope.conditional();
        match cmd.name.as_str() {
            "cmake_minimum_required" => {
                if let Some(pos) = items.iter().position(|i| i.text == "VERSION") {
                    if self.project.minimum_version.is_none() {
                        self.project.minimum_version = items.get(pos + 1).map(|i| i.text.clone());
                    }
                }
            }
            "project" => self.cmd_project(scope, &items),
            "enable_language" => {
                for item in items.iter().filter(|i| i.text != "OPTIONAL") {
                    if !self.project.languages.contains(&item.text) {
                        self.project.languages.push(item.text.clone());
                    }
                }
            }
            "set" => self.cmd_set(scope, &items),
            "unset" => {
                if let Some(name) = items.first() {
                    if items.iter().any(|i| i.text == "PARENT_SCOPE") {
                        scope.parent_sets.push((name.text.clone(), None));
                    } else if conditional {
                        if let Some(VarValue::Known { value, .. }) = scope.vars.get(&name.text).cloned() {
                            scope.vars.insert(name.text.clone(), VarValue::Known { value, conditional: true });
                        }
                    } else {
                        scope.vars.remove(&name.text);
                    }
                }
            }
            "option" => self.cmd_option(scope, cmd, &items),
            "list" => self.cmd_list(scope, &items),
            "file" => self.cmd_file(scope, cmd, &items),
            "aux_source_directory" => self.cmd_aux_source_directory(scope, cmd, &items),
            "add_executable" => self.cmd_add_executable(scope, cmd, &items),
            "add_library" => self.cmd_add_library(scope, cmd, &items),
            "add_custom_target" => {
                if let Some(name) = items.first().filter(|n| !n.symbolic) {
                    let target = CMakeTarget::new(name.text.clone(), CMakeTargetKind::Custom, scope, cmd.line, conditional);
                    self.add_target(scope, target);
                }
            }
            "target_sources" => self.cmd_target_items(scope, cmd, &items, TargetItemKind::Sources),
            "target_include_directories" => self.cmd_target_items(scope, cmd, &items, TargetItemKind::IncludeDirs),
            "target_compile_definitions" => self.cmd_target_items(scope, cmd, &items, TargetItemKind::Definitions),
            "target_compile_options" => self.cmd_target_items(scope, cmd, &items, TargetItemKind::Options),
            "target_compile_features" => self.cmd_target_items(scope, cmd, &items, TargetItemKind::Features),
            "target_link_libraries" => self.cmd_target_link_libraries(scope, cmd, &items),
            "add_dependencies" => {
                if let Some((target, deps)) = items.split_first() {
                    let deps: Vec<String> = deps.iter().map(|d| d.text.clone()).collect();
                    self.target_op(scope, cmd, &target.text, TargetOp::Dependencies(deps));
                }
            }
            "set_target_properties" => self.cmd_set_target_properties(scope, cmd, &items),
            "set_property" => self.cmd_set_property(scope, cmd, &items),
            "include_directories" => {
                let mut system = false;
                for item in &items {
                    match item.text.as_str() {
                        "AFTER" | "BEFORE" => {}
                        "SYSTEM" => system = true,
                        _ => {
                            let mut entry = self.path_item(scope, item, ItemScope::Unspecified);
                            entry.system = system;
                            entry.from_directory = true;
                            scope.settings.include_dirs.push(entry);
                        }
                    }
                }
            }
            "add_definitions" => {
                for item in &items {
                    let text = item.value();
                    let def = text.strip_prefix("-D").or_else(|| text.strip_prefix("/D"));
                    let mut entry = self.text_item(item, ItemScope::Unspecified);
                    entry.from_directory = true;
                    match def {
                        Some(d) => {
                            entry.text = d.to_string();
                            scope.settings.definitions.push(entry);
                        }
                        None => scope.settings.compile_options.push(entry),
                    }
                }
            }
            "add_compile_definitions" => {
                for item in &items {
                    let mut entry = self.text_item(item, ItemScope::Unspecified);
                    entry.text = strip_define_prefix(&entry.text);
                    entry.from_directory = true;
                    if !entry.text.is_empty() {
                        scope.settings.definitions.push(entry);
                    }
                }
            }
            "add_compile_options" => {
                for item in &items {
                    let mut entry = self.text_item(item, ItemScope::Unspecified);
                    entry.from_directory = true;
                    scope.settings.compile_options.push(entry);
                }
            }
            "link_libraries" => {
                let links = parse_link_items(&items, ItemScope::Unspecified);
                for mut link in links {
                    link.from_directory = true;
                    scope.settings.link_libraries.push(link);
                }
            }
            "add_subdirectory" => self.cmd_add_subdirectory(scope, cmd, &items, depth),
            "include" => self.cmd_include(scope, cmd, &items, depth),
            "find_package" => self.cmd_find_package(scope, cmd, &items),
            "fetchcontent_declare" => self.cmd_fetchcontent_declare(scope, cmd, &items),
            "pkg_check_modules" | "pkg_search_module" => self.cmd_pkg_check_modules(scope, cmd, &items),
            "install" => self.cmd_install(scope, cmd, &items),
            "string" | "math" | "get_filename_component" | "get_target_property" | "get_property"
            | "get_directory_property" | "get_source_file_property" | "get_cmake_property"
            | "find_library" | "find_path" | "find_program" | "find_file" | "try_compile" | "try_run"
            | "separate_arguments" | "site_name" | "cmake_path" | "execute_process"
            | "cmake_host_system_information" | "check_include_file" | "check_include_files"
            | "check_include_file_cxx" | "check_function_exists" | "check_type_size"
            | "check_c_source_compiles" | "check_cxx_source_compiles" | "check_c_compiler_flag"
            | "check_cxx_compiler_flag" | "check_c_source_runs" | "check_cxx_source_runs"
            | "check_compiler_flag" | "check_linker_flag" | "check_symbol_exists"
            | "check_cxx_symbol_exists" | "check_library_exists" | "check_struct_has_member" => {
                let outputs = output_variables(&cmd.name, &items);
                Self::mark_unknown(scope, &outputs);
            }
            name => {
                let creates = self.functions.get(name).map(|f| f.creates_targets).unwrap_or(false);
                if creates {
                    let file = scope.list_file.clone();
                    self.diag_info(
                        &file,
                        cmd.line,
                        format!("`{}` is a project function that creates targets; its targets are not modeled", name),
                    );
                }
            }
        }
    }

    fn cmd_project(&mut self, scope: &mut DirScope, items: &[Item]) {
        let Some(name) = items.first().map(|i| i.text.clone()) else {
            return;
        };
        let mut version = None;
        let mut languages = Vec::new();
        let mut saw_keyword = false;
        let mut i = 1;
        while i < items.len() {
            match items[i].text.as_str() {
                "VERSION" => {
                    saw_keyword = true;
                    version = items.get(i + 1).map(|v| v.text.clone());
                    i += 2;
                }
                "DESCRIPTION" | "HOMEPAGE_URL" => {
                    saw_keyword = true;
                    i += 2;
                }
                "LANGUAGES" => {
                    saw_keyword = true;
                    i += 1;
                    while i < items.len() && !matches!(items[i].text.as_str(), "VERSION" | "DESCRIPTION" | "HOMEPAGE_URL") {
                        languages.push(items[i].text.clone());
                        i += 1;
                    }
                }
                other => {
                    if !saw_keyword {
                        languages.push(other.to_string());
                    }
                    i += 1;
                }
            }
        }
        if languages.is_empty() {
            languages = vec!["C".to_string(), "CXX".to_string()];
        }
        languages.retain(|l| l != "NONE");

        let dir_text = to_slash_string(&scope.source_dir);
        let set = set_known;
        set(scope, "PROJECT_NAME".to_string(), &name);
        set(scope, "PROJECT_SOURCE_DIR".to_string(), &dir_text);
        set(scope, format!("{}_SOURCE_DIR", name), &dir_text);
        if self.project.name.is_none() {
            set(scope, "CMAKE_PROJECT_NAME".to_string(), &name);
            self.project.name = Some(name.clone());
            self.project.version = version.clone();
        }
        if let Some(v) = &version {
            set(scope, "PROJECT_VERSION".to_string(), v);
            set(scope, format!("{}_VERSION", name), v);
            let parts: Vec<&str> = v.split('.').collect();
            for (suffix, part) in ["MAJOR", "MINOR", "PATCH", "TWEAK"].iter().zip(parts.iter()) {
                set(scope, format!("PROJECT_VERSION_{}", suffix), part);
                set(scope, format!("{}_VERSION_{}", name, suffix), part);
            }
        }
        for lang in &languages {
            if !self.project.languages.contains(lang) {
                self.project.languages.push(lang.clone());
            }
        }
        self.project.declarations.push(CMakeProjectDecl {
            name,
            version,
            languages,
            directory: scope.source_dir.clone(),
        });
    }

    fn cmd_set(&mut self, scope: &mut DirScope, items: &[Item]) {
        let Some((name_item, rest)) = items.split_first() else {
            return;
        };
        if name_item.symbolic || name_item.text.starts_with("ENV{") {
            return;
        }
        let name = name_item.text.clone();
        let mut values: Vec<Item> = rest.to_vec();
        let mut parent_scope = false;
        if values.last().is_some_and(|v| v.text == "PARENT_SCOPE") {
            values.pop();
            parent_scope = true;
        }
        if let Some(cache_pos) = values.iter().position(|v| v.text == "CACHE") {
            let force = values[cache_pos..].iter().any(|v| v.text == "FORCE");
            values.truncate(cache_pos);
            if !force && scope.vars.contains_key(&name) {
                return;
            }
        }
        self.set_var(scope, &name, &values, parent_scope);
    }

    fn cmd_option(&mut self, scope: &mut DirScope, cmd: &CMakeCommand, items: &[Item]) {
        let Some(name) = items.first().map(|i| i.text.clone()) else {
            return;
        };
        let description = items.get(1).map(|i| i.text.clone()).unwrap_or_default();
        let default = items.get(2).map(|i| i.text.clone());
        if !scope.vars.contains_key(&name) {
            let value = Item {
                text: default.clone().unwrap_or_else(|| "OFF".to_string()),
                symbolic: false,
                conditional: false,
                line: cmd.line,
            };
            self.set_var(scope, &name, &[value], false);
        }
        self.project.options.push(CMakeOption {
            name,
            description,
            default,
            list_file: scope.list_file.clone(),
            line: cmd.line,
        });
    }

    fn cmd_list(&mut self, scope: &mut DirScope, items: &[Item]) {
        let (Some(sub), Some(var)) = (items.first(), items.get(1)) else {
            return;
        };
        let var = var.text.clone();
        let conditional = scope.conditional();
        let args = &items[2..];
        let current = scope.vars.get(&var).cloned();
        match sub.text.as_str() {
            "APPEND" | "PREPEND" => {
                let added: Vec<String> = args.iter().map(|a| a.text.clone()).collect();
                let added_conditional = args.iter().any(|a| a.conditional);
                let new_value = match current {
                    Some(VarValue::Unknown) => VarValue::Unknown,
                    Some(VarValue::Known { value, conditional: c }) => {
                        let mut list = split_list(&value);
                        if sub.text == "APPEND" {
                            list.extend(added);
                        } else {
                            let mut front = added;
                            front.extend(list);
                            list = front;
                        }
                        VarValue::Known {
                            value: list.join(";"),
                            conditional: c || conditional || added_conditional,
                        }
                    }
                    None => VarValue::Known {
                        value: added.join(";"),
                        conditional: conditional || added_conditional,
                    },
                };
                scope.vars.insert(var, new_value);
            }
            "REMOVE_ITEM" => {
                if let Some(VarValue::Known { value, conditional: c }) = current {
                    if conditional {
                        // The removal may not happen: keep every element.
                        scope.vars.insert(var, VarValue::Known { value, conditional: true });
                    } else {
                        let remove: Vec<String> = args.iter().map(|a| a.text.clone()).collect();
                        let kept: Vec<String> = split_list(&value).into_iter().filter(|e| !remove.contains(e)).collect();
                        scope.vars.insert(var, VarValue::Known { value: kept.join(";"), conditional: c });
                    }
                }
            }
            "REMOVE_DUPLICATES" => {
                if let Some(VarValue::Known { value, conditional: c }) = current {
                    let mut seen = HashSet::new();
                    let kept: Vec<String> = split_list(&value).into_iter().filter(|e| seen.insert(e.clone())).collect();
                    scope.vars.insert(var, VarValue::Known { value: kept.join(";"), conditional: c });
                }
            }
            "LENGTH" | "GET" | "JOIN" | "SUBLIST" | "FIND" => {
                if let Some(out) = items.last() {
                    Self::mark_unknown(scope, &[out.text.clone()]);
                }
            }
            _ => {
                let mut outputs = vec![var];
                if let Some(pos) = items.iter().position(|i| i.text == "OUTPUT_VARIABLE") {
                    if let Some(out) = items.get(pos + 1) {
                        outputs.push(out.text.clone());
                    }
                }
                if matches!(sub.text.as_str(), "POP_BACK" | "POP_FRONT") {
                    outputs.extend(args.iter().map(|a| a.text.clone()));
                }
                Self::mark_unknown(scope, &outputs);
            }
        }
    }

    fn cmd_file(&mut self, scope: &mut DirScope, cmd: &CMakeCommand, items: &[Item]) {
        let Some(sub) = items.first().map(|i| i.text.clone()) else {
            return;
        };
        if sub != "GLOB" && sub != "GLOB_RECURSE" {
            let outputs = output_variables("file", items);
            Self::mark_unknown(scope, &outputs);
            return;
        }
        let Some(var) = items.get(1).map(|i| i.text.clone()) else {
            return;
        };
        let recursive = sub == "GLOB_RECURSE";
        let mut relative: Option<PathBuf> = None;
        let mut patterns: Vec<&Item> = Vec::new();
        let mut i = 2;
        while i < items.len() {
            match items[i].text.as_str() {
                "LIST_DIRECTORIES" => i += 2,
                "RELATIVE" => {
                    relative = items.get(i + 1).map(|r| resolve_path(&scope.source_dir, &r.text));
                    i += 2;
                }
                "CONFIGURE_DEPENDS" | "FOLLOW_SYMLINKS" => i += 1,
                _ => {
                    patterns.push(&items[i]);
                    i += 1;
                }
            }
        }
        if let Some(symbolic) = patterns.iter().find(|p| p.symbolic) {
            let file = scope.list_file.clone();
            let text = symbolic.text.clone();
            self.diag_info(&file, cmd.line, format!("file({}) pattern '{}' has unresolved references; `{}` is unknown", sub, text, var));
            Self::mark_unknown(scope, &[var]);
            return;
        }
        let options = GlobOptions {
            char_classes: true,
            ..GlobOptions::default()
        };
        let prune = |dir: &Path| {
            dir.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| is_build_output_directory(n) || n.starts_with('.'))
        };
        let mut found: Vec<PathBuf> = Vec::new();
        let mut any_conditional = false;
        for pattern in &patterns {
            any_conditional |= pattern.conditional;
            let text = pattern.value();
            let text = if recursive {
                // GLOB_RECURSE matches the last component in every subdirectory.
                match text.rfind(['/', '\\']) {
                    Some(pos) => format!("{}/**/{}", &text[..pos], &text[pos + 1..]),
                    None => format!("**/{}", text),
                }
            } else {
                text
            };
            let expansion = expand_glob(&scope.source_dir, &text, options, &prune);
            for dir in &expansion.unreadable {
                let file = scope.list_file.clone();
                self.diag_warning(&file, cmd.line, format!("file({}) could not read directory '{}'", sub, dir.display()));
            }
            found.extend(expansion.files);
        }
        found.sort();
        found.dedup();
        let values: Vec<Item> = found
            .iter()
            .map(|p| {
                let text = match &relative {
                    Some(base) => p
                        .strip_prefix(base)
                        .map(to_slash_string)
                        .unwrap_or_else(|_| to_slash_string(p)),
                    None => to_slash_string(p),
                };
                Item {
                    text,
                    symbolic: false,
                    conditional: any_conditional,
                    line: cmd.line,
                }
            })
            .collect();
        if values.is_empty() {
            let file = scope.list_file.clone();
            self.diag_info(&file, cmd.line, format!("file({}) for `{}` matched no files", sub, var));
        }
        // An empty match still defines the variable (as the empty list).
        let conditional = scope.conditional();
        if values.is_empty() && !conditional {
            scope.vars.insert(var, VarValue::known(""));
        } else {
            self.set_var(scope, &var, &values, false);
        }
    }

    fn cmd_aux_source_directory(&mut self, scope: &mut DirScope, cmd: &CMakeCommand, items: &[Item]) {
        let (Some(dir), Some(var)) = (items.first(), items.get(1)) else {
            return;
        };
        if dir.symbolic {
            Self::mark_unknown(scope, &[var.text.clone()]);
            return;
        }
        let dir_path = resolve_path(&scope.source_dir, &dir.value());
        let mut files: Vec<PathBuf> = match fs::read_dir(&dir_path) {
            Ok(entries) => entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_file())
                .filter(|p| {
                    p.extension()
                        .and_then(|e| e.to_str())
                        .map(|e| e.to_ascii_lowercase())
                        .is_some_and(|e| C_SOURCE_EXTENSIONS.contains(&e.as_str()) || CPP_SOURCE_EXTENSIONS.contains(&e.as_str()))
                })
                .collect(),
            Err(err) => {
                let file = scope.list_file.clone();
                self.diag_warning(&file, cmd.line, format!("aux_source_directory: cannot read '{}': {}", dir_path.display(), err));
                Self::mark_unknown(scope, &[var.text.clone()]);
                return;
            }
        };
        files.sort();
        let values: Vec<Item> = files
            .iter()
            .map(|p| Item {
                text: to_slash_string(p),
                symbolic: false,
                conditional: dir.conditional,
                line: cmd.line,
            })
            .collect();
        let var = var.text.clone();
        self.set_var(scope, &var, &values, false);
    }

    // ── Targets ──

    fn add_target(&mut self, scope: &mut DirScope, target: CMakeTarget) -> Option<usize> {
        if self.project.targets.iter().any(|t| t.name == target.name) {
            let file = scope.list_file.clone();
            let message = format!("target `{}` is declared more than once; the first declaration is kept", target.name);
            self.diag_warning(&file, target.line, message);
            return None;
        }
        let index = self.project.targets.len();
        self.project.targets.push(target);
        scope.targets_in_dir.push(index);
        Some(index)
    }

    fn standard_from_vars(scope: &DirScope, family: StandardFamily) -> Option<LanguageStandard> {
        let (std_var, ext_var) = match family {
            StandardFamily::Cpp => ("CMAKE_CXX_STANDARD", "CMAKE_CXX_EXTENSIONS"),
            StandardFamily::C => ("CMAKE_C_STANDARD", "CMAKE_C_EXTENSIONS"),
            StandardFamily::CSharp => return None,
        };
        let value = scope.known(std_var)?;
        // CMake's default for `<LANG>_EXTENSIONS` is ON.
        let extensions = scope.known(ext_var).and_then(|v| cmake_truth(&v)).unwrap_or(true);
        LanguageStandard::from_cmake(family, &value, extensions)
    }

    fn new_binary_target(&self, scope: &DirScope, name: &Item, kind: CMakeTargetKind, cmd: &CMakeCommand) -> CMakeTarget {
        let mut target = CMakeTarget::new(name.text.clone(), kind, scope, cmd.line, scope.conditional());
        target.cxx_standard = Self::standard_from_vars(scope, StandardFamily::Cpp);
        target.c_standard = Self::standard_from_vars(scope, StandardFamily::C);
        target
    }

    fn cmd_add_executable(&mut self, scope: &mut DirScope, cmd: &CMakeCommand, items: &[Item]) {
        let Some((name, rest)) = items.split_first() else {
            return;
        };
        if name.symbolic {
            let file = scope.list_file.clone();
            self.diag_warning(&file, cmd.line, format!("add_executable: target name '{}' is not resolvable", name.text));
            return;
        }
        let mut target = self.new_binary_target(scope, name, CMakeTargetKind::Executable, cmd);
        match rest.first().map(|r| r.text.as_str()) {
            Some("IMPORTED") => {
                target.imported = true;
                self.add_target(scope, target);
                return;
            }
            Some("ALIAS") => {
                target.alias_of = rest.get(1).map(|a| a.text.clone());
                self.add_target(scope, target);
                return;
            }
            _ => {}
        }
        let mut sources = Vec::new();
        for item in rest {
            match item.text.as_str() {
                "WIN32" => target.win32_executable = true,
                "MACOSX_BUNDLE" => target.macos_bundle = true,
                "EXCLUDE_FROM_ALL" => target.exclude_from_all = true,
                _ => sources.push(self.source_item(scope, item, ItemScope::Private)),
            }
        }
        target.sources = sources;
        self.add_target(scope, target);
    }

    fn cmd_add_library(&mut self, scope: &mut DirScope, cmd: &CMakeCommand, items: &[Item]) {
        let Some((name, rest)) = items.split_first() else {
            return;
        };
        if name.symbolic {
            let file = scope.list_file.clone();
            self.diag_warning(&file, cmd.line, format!("add_library: target name '{}' is not resolvable", name.text));
            return;
        }
        let mut kind: Option<CMakeTargetKind> = None;
        let mut imported = false;
        let mut exclude = false;
        let mut sources_start = 0;
        for (k, item) in rest.iter().enumerate() {
            let explicit = match item.text.as_str() {
                "STATIC" => Some(CMakeTargetKind::StaticLibrary),
                "SHARED" => Some(CMakeTargetKind::SharedLibrary),
                "MODULE" => Some(CMakeTargetKind::ModuleLibrary),
                "OBJECT" => Some(CMakeTargetKind::ObjectLibrary),
                "INTERFACE" => Some(CMakeTargetKind::InterfaceLibrary),
                "UNKNOWN" => Some(CMakeTargetKind::UnknownLibrary),
                "IMPORTED" => {
                    imported = true;
                    None
                }
                "GLOBAL" => None,
                "EXCLUDE_FROM_ALL" => {
                    exclude = true;
                    None
                }
                "ALIAS" => {
                    let mut target = self.new_binary_target(scope, name, CMakeTargetKind::StaticOrSharedLibrary, cmd);
                    target.alias_of = rest.get(k + 1).map(|a| a.text.clone());
                    self.add_target(scope, target);
                    return;
                }
                _ => {
                    sources_start = k;
                    break;
                }
            };
            if let Some(explicit) = explicit {
                kind = Some(explicit);
            }
            sources_start = k + 1;
        }
        let kind = kind.unwrap_or_else(|| match scope.vars.get("BUILD_SHARED_LIBS") {
            None => CMakeTargetKind::StaticLibrary,
            Some(VarValue::Known { value, conditional: false }) => match cmake_truth(value) {
                Some(true) => CMakeTargetKind::SharedLibrary,
                Some(false) => CMakeTargetKind::StaticLibrary,
                None => CMakeTargetKind::StaticOrSharedLibrary,
            },
            Some(_) => CMakeTargetKind::StaticOrSharedLibrary,
        });
        let mut target = self.new_binary_target(scope, name, kind, cmd);
        target.imported = imported;
        target.exclude_from_all = exclude;
        if !imported {
            let scope_kind = if kind == CMakeTargetKind::InterfaceLibrary {
                ItemScope::Interface
            } else {
                ItemScope::Private
            };
            target.sources = rest[sources_start.min(rest.len())..]
                .iter()
                .map(|item| self.source_item(scope, item, scope_kind))
                .collect();
        }
        self.add_target(scope, target);
    }

    /// A source item resolved against the current source directory; a missing file
    /// without extension is retried with CMake's source extensions.
    fn source_item(&self, scope: &DirScope, item: &Item, item_scope: ItemScope) -> CMakeItem {
        let mut entry = self.path_item(scope, item, item_scope);
        if let Some(path) = &entry.path {
            if !entry.exists && path.extension().is_none() {
                for ext in CMAKE_SOURCE_EXTENSION_PROBES {
                    let candidate = path.with_extension(ext);
                    if candidate.is_file() {
                        entry.path = Some(candidate);
                        entry.exists = true;
                        break;
                    }
                }
            }
        }
        entry
    }

    fn path_item(&self, scope: &DirScope, item: &Item, item_scope: ItemScope) -> CMakeItem {
        let text = item.value();
        let path = if item.symbolic || text.is_empty() {
            None
        } else {
            Some(resolve_path(&scope.source_dir, &text))
        };
        let exists = path.as_ref().is_some_and(|p| p.exists());
        CMakeItem {
            text,
            path,
            exists,
            scope: item_scope,
            symbolic: item.symbolic,
            conditional: item.conditional,
            system: false,
            from_directory: false,
            line: item.line,
        }
    }

    fn text_item(&self, item: &Item, item_scope: ItemScope) -> CMakeItem {
        CMakeItem {
            text: item.value(),
            path: None,
            exists: false,
            scope: item_scope,
            symbolic: item.symbolic,
            conditional: item.conditional,
            system: false,
            from_directory: false,
            line: item.line,
        }
    }

    fn cmd_target_items(&mut self, scope: &mut DirScope, cmd: &CMakeCommand, items: &[Item], kind: TargetItemKind) {
        let Some((target, rest)) = items.split_first() else {
            return;
        };
        let mut current_scope = ItemScope::Unspecified;
        let mut system = false;
        let mut collected: Vec<CMakeItem> = Vec::new();
        let mut i = 0;
        while i < rest.len() {
            let item = &rest[i];
            let text = item.text.as_str();
            if let Some(s) = scope_keyword(text) {
                current_scope = s;
                i += 1;
                continue;
            }
            match (kind, text) {
                (TargetItemKind::IncludeDirs, "SYSTEM") => {
                    system = true;
                    i += 1;
                    continue;
                }
                (TargetItemKind::IncludeDirs, "AFTER" | "BEFORE") | (TargetItemKind::Options, "BEFORE") => {
                    i += 1;
                    continue;
                }
                (TargetItemKind::Sources, "FILE_SET") => {
                    // FILE_SET <name> [TYPE <type>] [BASE_DIRS <dirs>...] [FILES <files>...]
                    i += 2;
                    let mut in_files = false;
                    while i < rest.len() {
                        let t = rest[i].text.as_str();
                        if scope_keyword(t).is_some() || t == "FILE_SET" {
                            break;
                        }
                        match t {
                            "TYPE" => {
                                in_files = false;
                                i += 2;
                                continue;
                            }
                            "BASE_DIRS" => in_files = false,
                            "FILES" => in_files = true,
                            _ if in_files => collected.push(self.source_item(scope, &rest[i], current_scope)),
                            _ => {}
                        }
                        i += 1;
                    }
                    continue;
                }
                _ => {}
            }
            let entry = match kind {
                TargetItemKind::Sources => self.source_item(scope, item, current_scope),
                TargetItemKind::IncludeDirs => {
                    let mut e = self.path_item(scope, item, current_scope);
                    e.system = system;
                    e
                }
                TargetItemKind::Definitions => {
                    let mut e = self.text_item(item, current_scope);
                    e.text = strip_define_prefix(&e.text);
                    e
                }
                TargetItemKind::Options | TargetItemKind::Features => self.text_item(item, current_scope),
            };
            if !entry.text.is_empty() {
                collected.push(entry);
            }
            i += 1;
        }
        let op = match kind {
            TargetItemKind::Sources => TargetOp::Sources(collected),
            TargetItemKind::IncludeDirs => TargetOp::IncludeDirs(collected),
            TargetItemKind::Definitions => TargetOp::Definitions(collected),
            TargetItemKind::Options => TargetOp::Options(collected),
            TargetItemKind::Features => TargetOp::Features(collected),
        };
        let target = target.text.clone();
        self.target_op(scope, cmd, &target, op);
    }

    fn cmd_target_link_libraries(&mut self, scope: &mut DirScope, cmd: &CMakeCommand, items: &[Item]) {
        let Some((target, rest)) = items.split_first() else {
            return;
        };
        let links = parse_link_items(rest, ItemScope::Unspecified);
        let target = target.text.clone();
        self.target_op(scope, cmd, &target, TargetOp::Links(links));
    }

    fn cmd_set_target_properties(&mut self, scope: &mut DirScope, cmd: &CMakeCommand, items: &[Item]) {
        let Some(props_pos) = items.iter().position(|i| i.text == "PROPERTIES") else {
            return;
        };
        let targets: Vec<String> = items[..props_pos].iter().map(|i| i.text.clone()).collect();
        let pairs = &items[props_pos + 1..];
        for pair in pairs.chunks(2) {
            let name = pair[0].text.clone();
            let value = pair.get(1).map(|v| v.text.clone()).unwrap_or_default();
            for target in &targets {
                let snapshot = Box::new(DirScopeSnapshot {
                    source_dir: scope.source_dir.clone(),
                    conditional: scope.conditional() || pair.iter().any(|p| p.conditional),
                    line: cmd.line,
                });
                let op = TargetOp::Property {
                    name: name.clone(),
                    value: value.clone(),
                    append: false,
                    scope: snapshot,
                };
                self.target_op(scope, cmd, target, op);
            }
        }
    }

    fn cmd_set_property(&mut self, scope: &mut DirScope, cmd: &CMakeCommand, items: &[Item]) {
        if items.first().map(|i| i.text.as_str()) != Some("TARGET") {
            return;
        }
        let Some(prop_pos) = items.iter().position(|i| i.text == "PROPERTY") else {
            return;
        };
        let mut targets = Vec::new();
        let mut append = false;
        for item in &items[1..prop_pos] {
            match item.text.as_str() {
                "APPEND" | "APPEND_STRING" => append = true,
                _ => targets.push(item.text.clone()),
            }
        }
        let Some(name) = items.get(prop_pos + 1).map(|i| i.text.clone()) else {
            return;
        };
        let value = items[(prop_pos + 2).min(items.len())..]
            .iter()
            .map(|i| i.text.as_str())
            .collect::<Vec<_>>()
            .join(";");
        for target in &targets {
            let snapshot = Box::new(DirScopeSnapshot {
                source_dir: scope.source_dir.clone(),
                conditional: scope.conditional(),
                line: cmd.line,
            });
            let op = TargetOp::Property {
                name: name.clone(),
                value: value.clone(),
                append,
                scope: snapshot,
            };
            self.target_op(scope, cmd, target, op);
        }
    }

    fn cmd_add_subdirectory(&mut self, scope: &mut DirScope, cmd: &CMakeCommand, items: &[Item], depth: usize) {
        let Some(dir) = items.first() else {
            return;
        };
        let file = scope.list_file.clone();
        if dir.symbolic {
            self.diag_warning(&file, cmd.line, format!("add_subdirectory: '{}' is not resolvable", dir.text));
            return;
        }
        let sub = resolve_path(&scope.source_dir, &dir.value());
        if depth + 1 > MAX_DIRECTORY_DEPTH {
            self.diag_warning(&file, cmd.line, format!("add_subdirectory: '{}' exceeds the nesting limit of {}", sub.display(), MAX_DIRECTORY_DEPTH));
            return;
        }
        if self.visited_dirs.contains(&path_key(&sub)) {
            self.diag_warning(&file, cmd.line, format!("add_subdirectory: '{}' was already added", sub.display()));
            return;
        }
        if !sub.join(CMAKE_LISTS_FILE_NAME).is_file() {
            self.diag_warning(&file, cmd.line, format!("add_subdirectory: '{}' has no {}", sub.display(), CMAKE_LISTS_FILE_NAME));
            return;
        }
        let mut child_vars = scope.vars.clone();
        for name in ["CMAKE_CURRENT_BINARY_DIR", "CMAKE_CURRENT_LIST_LINE"] {
            child_vars.remove(name);
        }
        let conditional = scope.conditional() || dir.conditional;
        match self.process_directory(sub, child_vars, scope.settings.clone(), conditional, depth + 1) {
            Ok(parent_sets) => {
                for (name, value) in parent_sets {
                    match value {
                        Some(v) => {
                            scope.vars.insert(name, v);
                        }
                        None => {
                            scope.vars.remove(&name);
                        }
                    }
                }
            }
            Err(err) => self.diag_warning(&file, cmd.line, format!("add_subdirectory failed: {}", err)),
        }
    }

    fn cmd_include(&mut self, scope: &mut DirScope, cmd: &CMakeCommand, items: &[Item], depth: usize) {
        let Some(spec) = items.first() else {
            return;
        };
        if let Some(pos) = items.iter().position(|i| i.text == "RESULT_VARIABLE") {
            if let Some(out) = items.get(pos + 1) {
                Self::mark_unknown(scope, &[out.text.clone()]);
            }
        }
        let optional = items.iter().any(|i| i.text == "OPTIONAL");
        let file = scope.list_file.clone();
        if spec.symbolic {
            if !optional {
                self.diag_info(&file, cmd.line, format!("include: '{}' is not resolvable", spec.text));
            }
            return;
        }
        let text = spec.value();
        let looks_like_file = text.ends_with(".cmake") || text.contains('/') || text.contains('\\');
        let mut candidate = None;
        if looks_like_file {
            let path = resolve_path(&scope.source_dir, &text);
            if path.is_file() {
                candidate = Some(path);
            }
        } else if let Some(module_path) = scope.known("CMAKE_MODULE_PATH") {
            for dir in split_list(&module_path) {
                let path = resolve_path(&scope.source_dir, &format!("{}/{}.cmake", dir, text));
                if path.is_file() {
                    candidate = Some(path);
                    break;
                }
            }
        }
        let Some(path) = candidate else {
            if looks_like_file {
                if !optional {
                    self.diag_warning(&file, cmd.line, format!("include: file '{}' not found", text));
                }
            } else if !self.project.included_modules.contains(&text) {
                self.project.included_modules.push(text);
            }
            return;
        };
        if scope.include_depth >= MAX_INCLUDE_DEPTH {
            self.diag_warning(&file, cmd.line, format!("include: '{}' exceeds the nesting limit of {}", path.display(), MAX_INCLUDE_DEPTH));
            return;
        }
        let commands = match Self::read_commands(&path) {
            Ok(c) => c,
            Err(err) => {
                self.diag_warning(&file, cmd.line, format!("include failed: {}", err));
                return;
            }
        };
        self.project.list_files.push(path.clone());
        let saved_dir = scope.vars.get("CMAKE_CURRENT_LIST_DIR").cloned();
        let saved_file = scope.vars.get("CMAKE_CURRENT_LIST_FILE").cloned();
        let list_dir = path.parent().map(to_slash_string).unwrap_or_default();
        scope.vars.insert("CMAKE_CURRENT_LIST_DIR".to_string(), VarValue::known(&list_dir));
        scope.vars.insert("CMAKE_CURRENT_LIST_FILE".to_string(), VarValue::known(&to_slash_string(&path)));
        let saved_list_file = std::mem::replace(&mut scope.list_file, path.clone());
        let saved_blocks = scope.blocks.len();
        scope.include_depth += 1;
        self.run(&commands, scope, depth);
        scope.include_depth -= 1;
        if scope.blocks.len() != saved_blocks {
            self.diag_warning(&path, 0, "blocks opened in this file are not closed");
            scope.blocks.truncate(saved_blocks);
        }
        scope.list_file = saved_list_file;
        for (name, saved) in [("CMAKE_CURRENT_LIST_DIR", saved_dir), ("CMAKE_CURRENT_LIST_FILE", saved_file)] {
            match saved {
                Some(v) => {
                    scope.vars.insert(name.to_string(), v);
                }
                None => {
                    scope.vars.remove(name);
                }
            }
        }
    }

    fn cmd_find_package(&mut self, scope: &mut DirScope, cmd: &CMakeCommand, items: &[Item]) {
        let Some(name) = items.first().map(|i| i.text.clone()) else {
            return;
        };
        let version = items
            .get(1)
            .filter(|v| v.text.chars().next().is_some_and(|c| c.is_ascii_digit()))
            .map(|v| v.text.clone());
        let mut required = false;
        let mut components = Vec::new();
        let mut mode = None;
        let mut in_components = false;
        for item in items.iter().skip(1) {
            match item.text.as_str() {
                "REQUIRED" => {
                    required = true;
                    in_components = true;
                }
                "COMPONENTS" | "OPTIONAL_COMPONENTS" => in_components = true,
                "MODULE" | "CONFIG" | "NO_MODULE" => {
                    mode = Some(item.text.clone());
                    in_components = false;
                }
                "EXACT" | "QUIET" | "GLOBAL" | "NO_POLICY_SCOPE" | "BYPASS_PROVIDER" => in_components = false,
                "NAMES" | "CONFIGS" | "HINTS" | "PATHS" | "PATH_SUFFIXES" | "REGISTRY_VIEW" => in_components = false,
                text if in_components && !text.starts_with("NO_") && !text.starts_with("CMAKE_") => {
                    components.push(text.to_string());
                }
                _ => {}
            }
        }
        for var in [format!("{}_FOUND", name), format!("{}_VERSION", name), format!("{}_INCLUDE_DIRS", name), format!("{}_LIBRARIES", name)] {
            scope.vars.insert(var, VarValue::Unknown);
        }
        self.project.packages.push(CMakePackage {
            name,
            version,
            required,
            components,
            origin: PackageOrigin::FindPackage { mode },
            conditional: scope.conditional(),
            list_file: scope.list_file.clone(),
            line: cmd.line,
        });
    }

    fn cmd_fetchcontent_declare(&mut self, scope: &mut DirScope, cmd: &CMakeCommand, items: &[Item]) {
        let Some(name) = items.first().map(|i| i.text.clone()) else {
            return;
        };
        let value_after = |key: &str| {
            items
                .iter()
                .position(|i| i.text == key)
                .and_then(|p| items.get(p + 1))
                .map(|v| v.text.clone())
        };
        let repository = value_after("GIT_REPOSITORY").or_else(|| value_after("URL"));
        let tag = value_after("GIT_TAG").or_else(|| value_after("URL_HASH"));
        self.project.packages.push(CMakePackage {
            name,
            version: None,
            required: true,
            components: Vec::new(),
            origin: PackageOrigin::FetchContent { repository, tag },
            conditional: scope.conditional(),
            list_file: scope.list_file.clone(),
            line: cmd.line,
        });
    }

    fn cmd_pkg_check_modules(&mut self, scope: &mut DirScope, cmd: &CMakeCommand, items: &[Item]) {
        let Some(prefix) = items.first().map(|i| i.text.clone()) else {
            return;
        };
        let mut required = false;
        let mut modules = Vec::new();
        for item in items.iter().skip(1) {
            match item.text.as_str() {
                "REQUIRED" => required = true,
                "QUIET" | "NO_CMAKE_PATH" | "NO_CMAKE_ENVIRONMENT_PATH" | "IMPORTED_TARGET" | "GLOBAL" => {}
                text => modules.push(text.to_string()),
            }
        }
        for suffix in ["FOUND", "INCLUDE_DIRS", "LIBRARIES", "LDFLAGS", "CFLAGS", "VERSION"] {
            scope.vars.insert(format!("{}_{}", prefix, suffix), VarValue::Unknown);
        }
        self.project.packages.push(CMakePackage {
            name: prefix,
            version: None,
            required,
            components: Vec::new(),
            origin: PackageOrigin::PkgConfig { modules },
            conditional: scope.conditional(),
            list_file: scope.list_file.clone(),
            line: cmd.line,
        });
    }

    fn cmd_install(&mut self, scope: &mut DirScope, cmd: &CMakeCommand, items: &[Item]) {
        if items.first().map(|i| i.text.as_str()) != Some("TARGETS") {
            return;
        }
        let mut targets = Vec::new();
        let mut i = 1;
        while i < items.len() && !INSTALL_KEYWORDS.contains(&items[i].text.as_str()) {
            targets.push(items[i].text.clone());
            i += 1;
        }
        let mut rules = Vec::new();
        let mut kind: Option<String> = None;
        while i < items.len() {
            let text = items[i].text.as_str();
            if INSTALL_ARTIFACT_KINDS.contains(&text) {
                kind = Some(text.to_string());
                if text == "FILE_SET" {
                    i += 1;
                }
            } else if text == "DESTINATION" {
                if let Some(dest) = items.get(i + 1) {
                    rules.push(InstallRule {
                        kind: kind.clone(),
                        destination: dest.text.clone(),
                        symbolic: dest.symbolic,
                    });
                }
                i += 1;
            } else if text == "EXPORT" || text == "COMPONENT" || text == "NAMELINK_COMPONENT" || text == "RUNTIME_DEPENDENCY_SET" {
                i += 1;
            }
            i += 1;
        }
        for target in targets {
            for rule in &rules {
                self.target_op(scope, cmd, &target, TargetOp::Install(rule.clone()));
            }
        }
    }

    // ── Target operations ──

    fn target_op(&mut self, scope: &mut DirScope, cmd: &CMakeCommand, target: &str, op: TargetOp) {
        match self.project.target_index(target) {
            Some(index) => Self::apply_op(&mut self.project.targets[index], op),
            None => self.pending.push(PendingOp {
                target: target.to_string(),
                op,
                list_file: scope.list_file.clone(),
                line: cmd.line,
                command: cmd.name.clone(),
            }),
        }
    }

    fn apply_op(target: &mut CMakeTarget, op: TargetOp) {
        match op {
            TargetOp::Sources(items) => target.sources.extend(items),
            TargetOp::IncludeDirs(items) => target.include_directories.extend(items),
            TargetOp::Definitions(items) => target.compile_definitions.extend(items),
            TargetOp::Options(items) => target.compile_options.extend(items),
            TargetOp::Features(items) => target.compile_features.extend(items),
            TargetOp::Links(items) => target.link_libraries.extend(items),
            TargetOp::Dependencies(deps) => {
                for dep in deps {
                    if !target.build_dependencies.contains(&dep) {
                        target.build_dependencies.push(dep);
                    }
                }
            }
            TargetOp::Install(rule) => target.install_rules.push(rule),
            TargetOp::Property { name, value, append, scope } => Self::apply_property(target, &name, &value, append, &scope),
        }
    }

    fn apply_property(target: &mut CMakeTarget, name: &str, value: &str, append: bool, scope: &DirScopeSnapshot) {
        let stored = match (append, target.properties.get(name)) {
            (true, Some(old)) if !old.is_empty() => format!("{};{}", old, value),
            _ => value.to_string(),
        };
        target.properties.insert(name.to_string(), stored);
        let symbolic = has_unresolved(value);
        let item = |text: &str, item_scope: ItemScope, as_path: bool| CMakeItem {
            text: text.to_string(),
            path: if as_path && !has_unresolved(text) {
                Some(resolve_path(&scope.source_dir, text))
            } else {
                None
            },
            exists: as_path && !has_unresolved(text) && resolve_path(&scope.source_dir, text).exists(),
            scope: item_scope,
            symbolic: has_unresolved(text),
            conditional: scope.conditional,
            system: false,
            from_directory: false,
            line: scope.line,
        };
        match name {
            "OUTPUT_NAME" if !symbolic => target.output_name = Some(value.to_string()),
            "CXX_STANDARD" | "C_STANDARD" => {
                let family = if name == "CXX_STANDARD" {
                    StandardFamily::Cpp
                } else {
                    StandardFamily::C
                };
                let ext_name = if name == "CXX_STANDARD" { "CXX_EXTENSIONS" } else { "C_EXTENSIONS" };
                let extensions = target
                    .properties
                    .get(ext_name)
                    .and_then(|v| cmake_truth(v))
                    .unwrap_or(true);
                let standard = LanguageStandard::from_cmake(family, value, extensions);
                if family == StandardFamily::Cpp {
                    target.cxx_standard = standard;
                } else {
                    target.c_standard = standard;
                }
            }
            "INCLUDE_DIRECTORIES" | "INTERFACE_INCLUDE_DIRECTORIES" => {
                let item_scope = if name.starts_with("INTERFACE_") {
                    ItemScope::Interface
                } else {
                    ItemScope::Private
                };
                if !append {
                    target.include_directories.retain(|d| d.scope != item_scope);
                }
                for part in split_list(value) {
                    target.include_directories.push(item(&part, item_scope, true));
                }
            }
            "COMPILE_DEFINITIONS" | "INTERFACE_COMPILE_DEFINITIONS" => {
                let item_scope = if name.starts_with("INTERFACE_") {
                    ItemScope::Interface
                } else {
                    ItemScope::Private
                };
                if !append {
                    target.compile_definitions.retain(|d| d.scope != item_scope);
                }
                for part in split_list(value) {
                    target.compile_definitions.push(item(&strip_define_prefix(&part), item_scope, false));
                }
            }
            "LINK_LIBRARIES" | "INTERFACE_LINK_LIBRARIES" => {
                let item_scope = if name.starts_with("INTERFACE_") {
                    ItemScope::Interface
                } else {
                    ItemScope::Private
                };
                if !append {
                    target.link_libraries.retain(|l| l.scope != item_scope);
                }
                for part in split_list(value) {
                    target.link_libraries.push(CMakeLinkItem {
                        symbolic: has_unresolved(&part),
                        name: part,
                        scope: item_scope,
                        configuration: LinkConfiguration::All,
                        conditional: scope.conditional,
                        from_directory: false,
                        line: scope.line,
                    });
                }
            }
            _ => {}
        }
    }

    /// Applies operations on targets declared after them and reports the rest.
    fn finish(&mut self) {
        let pending = std::mem::take(&mut self.pending);
        for op in pending {
            match self.project.target_index(&op.target) {
                Some(index) => Self::apply_op(&mut self.project.targets[index], op.op),
                None => {
                    let message = format!("{}: target `{}` is not declared in this project", op.command, op.target);
                    self.diag_warning(&op.list_file, op.line, message);
                }
            }
        }
        for target in &mut self.project.targets {
            dedup_items(&mut target.include_directories);
            dedup_items(&mut target.compile_definitions);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TargetItemKind {
    Sources,
    IncludeDirs,
    Definitions,
    Options,
    Features,
}

fn scope_keyword(text: &str) -> Option<ItemScope> {
    match text {
        "PRIVATE" => Some(ItemScope::Private),
        "PUBLIC" => Some(ItemScope::Public),
        "INTERFACE" => Some(ItemScope::Interface),
        _ => None,
    }
}

/// Removes a leading `-D` (CMake strips it from `target_compile_definitions` items).
fn strip_define_prefix(text: &str) -> String {
    text.strip_prefix("-D").unwrap_or(text).to_string()
}

fn prepend(list: &mut Vec<CMakeItem>, front: &[CMakeItem]) {
    if front.is_empty() {
        return;
    }
    let mut merged = front.to_vec();
    merged.append(list);
    *list = merged;
}

/// Drops exact repeats of an item (same text, scope and path), keeping the first.
fn dedup_items(list: &mut Vec<CMakeItem>) {
    let mut seen = HashSet::new();
    list.retain(|i| seen.insert((i.text.clone(), i.scope, i.path.clone())));
}

/// Parses `target_link_libraries` / `link_libraries` items with scope keywords and the
/// `debug` / `optimized` / `general` qualifiers.
fn parse_link_items(items: &[Item], default_scope: ItemScope) -> Vec<CMakeLinkItem> {
    let mut links = Vec::new();
    let mut current_scope = default_scope;
    let mut next_config = LinkConfiguration::All;
    for item in items {
        match item.text.as_str() {
            "PRIVATE" | "LINK_PRIVATE" => current_scope = ItemScope::Private,
            "PUBLIC" | "LINK_PUBLIC" => current_scope = ItemScope::Public,
            "INTERFACE" | "LINK_INTERFACE_LIBRARIES" => current_scope = ItemScope::Interface,
            "debug" => next_config = LinkConfiguration::Debug,
            "optimized" => next_config = LinkConfiguration::Optimized,
            "general" => next_config = LinkConfiguration::All,
            _ => {
                let name = item.value();
                if name.is_empty() {
                    continue;
                }
                links.push(CMakeLinkItem {
                    name,
                    scope: current_scope,
                    configuration: next_config,
                    symbolic: item.symbolic,
                    conditional: item.conditional,
                    from_directory: false,
                    line: item.line,
                });
                next_config = LinkConfiguration::All;
            }
        }
    }
    links
}

/// Variables a command assigns without this reader evaluating it.
fn output_variables(name: &str, items: &[Item]) -> Vec<String> {
    let at = |k: usize| items.get(k).map(|i| i.text.clone());
    let last = || items.last().map(|i| i.text.clone());
    let sub = at(0).unwrap_or_default().to_ascii_uppercase();
    let mut out: Vec<String> = Vec::new();
    match name {
        "get_filename_component" | "get_target_property" | "get_property" | "get_directory_property"
        | "get_source_file_property" | "get_cmake_property" | "find_library" | "find_path"
        | "find_program" | "find_file" | "try_compile" | "try_run" | "separate_arguments" | "site_name" => {
            out.extend(at(0));
        }
        "math" => out.extend(at(1)),
        "string" => {
            let index = match sub.as_str() {
                "REGEX" | "REPLACE" | "FIND" | "REPEAT" => Some(3),
                "CONCAT" | "MD5" | "SHA1" | "SHA224" | "SHA256" | "SHA384" | "SHA512" | "SHA3_224"
                | "SHA3_256" | "SHA3_384" | "SHA3_512" | "TIMESTAMP" | "UUID" | "APPEND" | "PREPEND"
                | "JSON" => Some(1),
                "JOIN" | "TOLOWER" | "TOUPPER" | "LENGTH" | "STRIP" | "MAKE_C_IDENTIFIER"
                | "GENEX_STRIP" | "CONFIGURE" | "HEX" => Some(2),
                "SUBSTRING" | "COMPARE" => Some(4),
                _ => None,
            };
            match index {
                Some(k) => out.extend(at(k)),
                None if sub == "ASCII" || sub == "RANDOM" => out.extend(last()),
                None => {}
            }
        }
        "file" => match sub.as_str() {
            "READ" | "STRINGS" | "SIZE" | "TIMESTAMP" | "READ_SYMLINK" | "TO_CMAKE_PATH"
            | "TO_NATIVE_PATH" | "REAL_PATH" | "MD5" | "SHA1" | "SHA224" | "SHA256" | "SHA384"
            | "SHA512" => out.extend(at(2)),
            "RELATIVE_PATH" => out.extend(at(1)),
            _ => {}
        },
        "cmake_path" => {
            match sub.as_str() {
                "GET" | "HAS_ROOT_NAME" | "HAS_ROOT_DIRECTORY" | "HAS_ROOT_PATH" | "HAS_FILENAME"
                | "HAS_EXTENSION" | "HAS_STEM" | "HAS_RELATIVE_PART" | "HAS_PARENT_PATH"
                | "IS_ABSOLUTE" | "IS_RELATIVE" | "IS_PREFIX" | "COMPARE" | "HASH" => out.extend(last()),
                "CONVERT" => out.extend(at(3)),
                _ => out.extend(at(1)),
            }
            out.extend(values_after_keywords(items, &["OUTPUT_VARIABLE"]));
        }
        "execute_process" => out.extend(values_after_keywords(
            items,
            &["OUTPUT_VARIABLE", "ERROR_VARIABLE", "RESULT_VARIABLE", "RESULTS_VARIABLE"],
        )),
        "cmake_host_system_information" => out.extend(values_after_keywords(items, &["RESULT"])),
        "check_include_file" | "check_include_files" | "check_include_file_cxx" | "check_function_exists"
        | "check_type_size" | "check_c_source_compiles" | "check_cxx_source_compiles"
        | "check_c_compiler_flag" | "check_cxx_compiler_flag" | "check_c_source_runs"
        | "check_cxx_source_runs" | "check_compiler_flag" | "check_linker_flag" => out.extend(at(1)),
        "check_symbol_exists" | "check_cxx_symbol_exists" => out.extend(at(2)),
        "check_library_exists" | "check_struct_has_member" => out.extend(at(3)),
        _ => {}
    }
    out
}

/// The item following each occurrence of any of `keywords`.
fn values_after_keywords(items: &[Item], keywords: &[&str]) -> Vec<String> {
    items
        .iter()
        .enumerate()
        .filter(|(_, i)| keywords.contains(&i.text.as_str()))
        .filter_map(|(k, _)| items.get(k + 1).map(|v| v.text.clone()))
        .collect()
}

fn set_known(scope: &mut DirScope, key: String, value: &str) {
    scope.vars.insert(key, VarValue::known(value));
}

/// Header extensions a CMake target may list among its sources.
pub fn is_header_source(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .is_some_and(|e| e == "h" || CPP_HEADER_EXTENSIONS.contains(&e.as_str()))
}
