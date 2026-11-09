//! Clang JSON compilation database reader (`compile_commands.json`).
//!
//! Each entry (`directory`, `file`, `command` or `arguments`, optional `output`) becomes a
//! [`CompileCommand`] holding the facts later phases need: include search paths by kind,
//! `-D` / `-U` actions in order, forced includes, the language and its standard, and the
//! working directory every relative path was resolved against. GCC/Clang and MSVC/clang-cl
//! spellings are both understood; compiler launchers (`ccache`, `sccache`, ...) and
//! response files (`@file`) are handled. Entries that cannot be interpreted are reported
//! as diagnostics instead of aborting the whole database.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::Value;
use thiserror::Error;

use crate::core::types::LangId;
use crate::lang::build::{
    path_key, read_build_file, resolve_path, BuildDiagnostic, IncludeKind, IncludePath,
    LanguageStandard, MacroAction, MacroDefinition, StandardFamily,
};
use crate::lang::profile::{classify_extension, ExtensionClass};

/// File name of a compilation database.
pub const COMPILE_DB_FILE_NAME: &str = "compile_commands.json";

/// Maximum nesting of `@response` files.
const MAX_RESPONSE_FILE_DEPTH: usize = 8;

#[derive(Debug, Error)]
pub enum CompileDbError {
    #[error("failed to read '{path}': {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("'{path}' is not valid JSON: {message}")]
    Json { path: PathBuf, message: String },
    #[error("'{path}' must contain a JSON array of compile command objects")]
    NotAnArray { path: PathBuf },
}

/// Compiler family, which decides the option syntax.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CompilerDriver {
    Gcc,
    Clang,
    /// `clang-cl`: MSVC option syntax, Clang frontend.
    ClangCl,
    /// `cl.exe` (and Intel `icl`): MSVC option syntax.
    Msvc,
    Unknown,
}

impl CompilerDriver {
    /// Detects the driver from `argv[0]`.
    pub fn detect(program: &str) -> Self {
        let stem = program_stem(program);
        match stem.as_str() {
            "cl" | "icl" => CompilerDriver::Msvc,
            "clang-cl" => CompilerDriver::ClangCl,
            s if s.contains("clang") || s == "icx" || s == "icpx" || s == "emcc" || s == "em++" => {
                CompilerDriver::Clang
            }
            s if s.contains("gcc") || s.contains("g++") || s == "cc" || s == "c++" || s.ends_with("-cc") || s.ends_with("-c++") => {
                CompilerDriver::Gcc
            }
            _ => CompilerDriver::Unknown,
        }
    }

    /// True for drivers that take MSVC-style `/` options.
    pub fn uses_msvc_syntax(self) -> bool {
        matches!(self, CompilerDriver::Msvc | CompilerDriver::ClangCl)
    }
}

/// Lower-case file stem of a program path, without `.exe` and without a version suffix
/// (`gcc-12` → `gcc`, `clang++-17` → `clang++`).
fn program_stem(program: &str) -> String {
    let file = program.rsplit(['/', '\\']).next().unwrap_or(program);
    let lower = file.to_ascii_lowercase();
    let without_exe = lower.strip_suffix(".exe").unwrap_or(&lower);
    let trimmed = without_exe.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.');
    let trimmed = trimmed.strip_suffix('-').unwrap_or(trimmed);
    if trimmed.is_empty() {
        without_exe.to_string()
    } else {
        trimmed.to_string()
    }
}

/// Compiler launchers that wrap the real compiler (`ccache gcc ...`).
const COMPILER_LAUNCHERS: &[&str] = &["ccache", "sccache", "distcc", "icecc", "buildcache"];

/// How a compile command's standard was determined.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StandardSource {
    /// An explicit `-std=` / `/std:` flag.
    Flag,
    /// The documented default of the driver (MSVC compiles C++ as C++14 without `/std:`).
    DriverDefault,
}

/// One interpreted compilation database entry.
#[derive(Debug, Clone, PartialEq)]
pub struct CompileCommand {
    /// Source file, absolute and normalized.
    pub file: PathBuf,
    /// Working directory of the compiler invocation.
    pub directory: PathBuf,
    pub output: Option<PathBuf>,
    /// Full argument vector (launchers removed, response files spliced in).
    pub arguments: Vec<String>,
    /// `argv[0]` of the compiler (after any launcher).
    pub compiler: Option<String>,
    pub driver: CompilerDriver,
    /// `None` when neither a flag nor the extension decides the language.
    pub language: Option<LangId>,
    pub standard: Option<LanguageStandard>,
    pub standard_source: Option<StandardSource>,
    pub include_paths: Vec<IncludePath>,
    /// `-include` / `/FI` files, processed before the source.
    pub forced_includes: Vec<PathBuf>,
    /// `-imacros` files: only their macro definitions apply.
    pub macro_files: Vec<PathBuf>,
    pub macro_actions: Vec<MacroAction>,
    pub sysroot: Option<PathBuf>,
    /// `--target=` triple.
    pub target: Option<String>,
    /// `-nostdinc` / `/X`: the compiler's built-in include directories are not searched.
    pub ignores_standard_includes: bool,
    /// `/Zc:__cplusplus`: MSVC reports the real standard in `__cplusplus` (otherwise 199711L).
    pub msvc_conformant_cplusplus: bool,
    /// `/u`: MSVC removes every predefined Microsoft-specific macro.
    pub msvc_undefines_all: bool,
    /// Response files that could not be read.
    pub unresolved_response_files: Vec<PathBuf>,
    /// Positional inputs other than `file`.
    pub other_inputs: Vec<PathBuf>,
}

impl CompileCommand {
    /// Definitions in effect after every `-D` / `-U` is applied.
    pub fn defines(&self) -> Vec<MacroDefinition> {
        crate::lang::build::effective_macros(&self.macro_actions)
    }

    pub fn is_cpp(&self) -> bool {
        self.language == Some(LangId::Cpp)
    }

    /// Macros the driver predefines with values this command cannot know (compiler
    /// version macros), and macros it certainly leaves undefined.
    pub fn driver_assumptions(&self) -> (Vec<&'static str>, Vec<&'static str>) {
        match self.driver {
            CompilerDriver::Gcc => (vec!["__GNUC__", "__GNUC_MINOR__", "__GNUC_PATCHLEVEL__"], vec!["_MSC_VER", "__clang__"]),
            CompilerDriver::Clang => (vec!["__clang__", "__clang_major__", "__clang_minor__", "__GNUC__"], vec!["_MSC_VER"]),
            CompilerDriver::ClangCl => (vec!["__clang__", "_MSC_VER", "_WIN32"], vec!["__GNUC__"]),
            CompilerDriver::Msvc => (vec!["_MSC_VER", "_MSC_FULL_VER", "_WIN32"], vec!["__GNUC__", "__clang__"]),
            CompilerDriver::Unknown => (Vec::new(), Vec::new()),
        }
    }
}

/// How a compile command was chosen for a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchQuality {
    /// The database has an entry for the file itself.
    Exact,
    /// Borrowed from a translation unit with the same stem in the same directory
    /// (`foo.h` ← `foo.cpp`).
    SameStem,
    /// Borrowed from the translation unit sharing the longest directory prefix.
    SharedDirectory { shared_components: usize },
}

/// A loaded compilation database.
#[derive(Debug, Clone)]
pub struct CompileDatabase {
    pub path: PathBuf,
    pub commands: Vec<CompileCommand>,
    pub diagnostics: Vec<BuildDiagnostic>,
    by_file: HashMap<String, Vec<usize>>,
}

impl CompileDatabase {
    /// Reads and interprets `path`.
    pub fn load(path: &Path) -> Result<Self, CompileDbError> {
        let text = read_build_file(path).map_err(|source| CompileDbError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        Self::parse(&text, path)
    }

    /// Interprets database text; `db_path` anchors relative `directory` fields.
    pub fn parse(text: &str, db_path: &Path) -> Result<Self, CompileDbError> {
        let value: Value = serde_json::from_str(text).map_err(|e| CompileDbError::Json {
            path: db_path.to_path_buf(),
            message: e.to_string(),
        })?;
        let entries = value.as_array().ok_or_else(|| CompileDbError::NotAnArray {
            path: db_path.to_path_buf(),
        })?;
        let db_dir = match db_path.parent() {
            Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
            _ => PathBuf::from("."),
        };

        let mut db = CompileDatabase {
            path: db_path.to_path_buf(),
            commands: Vec::with_capacity(entries.len()),
            diagnostics: Vec::new(),
            by_file: HashMap::new(),
        };
        for (index, entry) in entries.iter().enumerate() {
            match parse_entry(entry, &db_dir) {
                Ok(command) => {
                    for missing in &command.unresolved_response_files {
                        db.diagnostics.push(BuildDiagnostic::warning(
                            db_path,
                            0,
                            format!(
                                "entry {} ({}): response file '{}' could not be read; its options are missing",
                                index,
                                command.file.display(),
                                missing.display()
                            ),
                        ));
                    }
                    if command.language.is_none() {
                        db.diagnostics.push(BuildDiagnostic::info(
                            db_path,
                            0,
                            format!(
                                "entry {} ({}): the source language could not be determined",
                                index,
                                command.file.display()
                            ),
                        ));
                    }
                    db.by_file
                        .entry(path_key(&command.file))
                        .or_default()
                        .push(db.commands.len());
                    db.commands.push(command);
                }
                Err(message) => db.diagnostics.push(BuildDiagnostic::warning(
                    db_path,
                    0,
                    format!("entry {} skipped: {}", index, message),
                )),
            }
        }
        Ok(db)
    }

    /// Candidate database locations under a project root, existing files only: the root
    /// itself, `build/`, `out/`, `cmake-build-*/`, `out/build/*/` and `build/*/`.
    pub fn locate(root: &Path) -> Vec<PathBuf> {
        let mut found = Vec::new();
        let mut push = |p: PathBuf| {
            if p.is_file() && !found.contains(&p) {
                found.push(p);
            }
        };
        push(root.join(COMPILE_DB_FILE_NAME));
        push(root.join("build").join(COMPILE_DB_FILE_NAME));
        push(root.join("out").join(COMPILE_DB_FILE_NAME));
        let nested = [root.to_path_buf(), root.join("out").join("build"), root.join("build")];
        for (i, dir) in nested.iter().enumerate() {
            let Ok(entries) = std::fs::read_dir(dir) else {
                continue;
            };
            let mut subdirs: Vec<PathBuf> = entries
                .flatten()
                .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
                .map(|e| e.path())
                .collect();
            subdirs.sort();
            for sub in subdirs {
                let name = sub
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.to_ascii_lowercase())
                    .unwrap_or_default();
                if i > 0 || name.starts_with("cmake-build-") {
                    push(sub.join(COMPILE_DB_FILE_NAME));
                }
            }
        }
        found
    }

    /// Every entry for `file` (a file can be compiled more than once, e.g. per configuration).
    pub fn commands_for(&self, file: &Path) -> Vec<&CompileCommand> {
        self.by_file
            .get(&path_key(file))
            .map(|ids| ids.iter().map(|&i| &self.commands[i]).collect())
            .unwrap_or_default()
    }

    /// The first entry for `file`.
    pub fn command_for(&self, file: &Path) -> Option<&CompileCommand> {
        self.by_file
            .get(&path_key(file))
            .and_then(|ids| ids.first())
            .map(|&i| &self.commands[i])
    }

    /// The entry to use for `file`: its own, or — for headers and other files the database
    /// does not list — the entry of the most closely related translation unit. `None` when
    /// the database is empty.
    pub fn best_match(&self, file: &Path) -> Option<(&CompileCommand, MatchQuality)> {
        if let Some(cmd) = self.command_for(file) {
            return Some((cmd, MatchQuality::Exact));
        }
        let target_key = path_key(file);
        let target_parts: Vec<&str> = target_key.split('/').collect();
        let target_dir = &target_parts[..target_parts.len().saturating_sub(1)];
        let target_stem = file
            .file_stem()
            .map(|s| s.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let wants_cpp = matches!(
            classify_extension(file),
            Some(ExtensionClass::Known {
                lang: LangId::Cpp,
                ..
            })
        );

        let mut best: Option<(usize, (bool, usize, bool))> = None;
        for (index, cmd) in self.commands.iter().enumerate() {
            let key = path_key(&cmd.file);
            let parts: Vec<&str> = key.split('/').collect();
            let dir = &parts[..parts.len().saturating_sub(1)];
            let shared = dir.iter().zip(target_dir.iter()).take_while(|(a, b)| a == b).count();
            let same_dir = dir.len() == target_dir.len() && shared == dir.len();
            let stem = cmd
                .file
                .file_stem()
                .map(|s| s.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            let same_stem = same_dir && stem == target_stem;
            let lang_fits = !wants_cpp || cmd.is_cpp();
            let score = (same_stem, shared, lang_fits);
            let better = match &best {
                None => true,
                Some((_, current)) => score > *current,
            };
            if better {
                best = Some((index, score));
            }
        }
        best.map(|(index, (same_stem, shared, _))| {
            let quality = if same_stem {
                MatchQuality::SameStem
            } else {
                MatchQuality::SharedDirectory {
                    shared_components: shared,
                }
            };
            (&self.commands[index], quality)
        })
    }

    /// Distinct translation units, in database order.
    pub fn translation_units(&self) -> Vec<&Path> {
        let mut seen = std::collections::HashSet::new();
        self.commands
            .iter()
            .filter(|c| seen.insert(path_key(&c.file)))
            .map(|c| c.file.as_path())
            .collect()
    }
}

fn parse_entry(entry: &Value, db_dir: &Path) -> Result<CompileCommand, String> {
    let obj = entry.as_object().ok_or("not a JSON object")?;
    let directory_text = obj
        .get("directory")
        .and_then(Value::as_str)
        .ok_or("missing string field 'directory'")?;
    let file_text = obj
        .get("file")
        .and_then(Value::as_str)
        .ok_or("missing string field 'file'")?;
    let directory = resolve_path(db_dir, directory_text);
    let file = resolve_path(&directory, file_text);
    let output = obj
        .get("output")
        .and_then(Value::as_str)
        .map(|o| resolve_path(&directory, o));

    let argv: Vec<String> = match (obj.get("arguments"), obj.get("command")) {
        (Some(args), _) => {
            let list = args.as_array().ok_or("'arguments' is not an array")?;
            list.iter()
                .map(|a| a.as_str().map(str::to_string).ok_or("'arguments' contains a non-string element"))
                .collect::<Result<Vec<_>, _>>()?
        }
        (None, Some(cmd)) => {
            let text = cmd.as_str().ok_or("'command' is not a string")?;
            split_command_line(text, command_line_style(text))
        }
        (None, None) => return Err("entry has neither 'arguments' nor 'command'".to_string()),
    };
    if argv.is_empty() {
        return Err("empty command line".to_string());
    }
    Ok(interpret(argv, directory, file, output))
}

// ── Command-line splitting ───────────────────────────────────────────────────

/// Quoting rules of a command string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandLineStyle {
    /// POSIX shell words: `'...'`, `"..."` with `\` escapes, `\` outside quotes.
    Posix,
    /// `CommandLineToArgvW`: `\` is literal except before `"`.
    Windows,
}

/// Chooses the quoting style for a `command` string: Windows rules on Windows hosts, for
/// MSVC-style drivers, and when the program is a drive-letter path; POSIX otherwise.
pub fn command_line_style(command: &str) -> CommandLineStyle {
    if cfg!(windows) {
        return CommandLineStyle::Windows;
    }
    let first = command.trim_start();
    let program: String = if let Some(rest) = first.strip_prefix('"') {
        rest.split('"').next().unwrap_or("").to_string()
    } else {
        first.split_whitespace().next().unwrap_or("").to_string()
    };
    let bytes = program.as_bytes();
    let drive_path = bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && (bytes[2] == b'\\' || bytes[2] == b'/');
    if drive_path || CompilerDriver::detect(&program).uses_msvc_syntax() {
        CommandLineStyle::Windows
    } else {
        CommandLineStyle::Posix
    }
}

/// Splits a command string into arguments.
pub fn split_command_line(command: &str, style: CommandLineStyle) -> Vec<String> {
    match style {
        CommandLineStyle::Posix => split_posix(command),
        CommandLineStyle::Windows => split_windows(command),
    }
}

fn split_posix(command: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_word = false;
    let mut chars = command.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            c if c.is_whitespace() => {
                if in_word {
                    args.push(std::mem::take(&mut current));
                    in_word = false;
                }
            }
            '\'' => {
                in_word = true;
                for q in chars.by_ref() {
                    if q == '\'' {
                        break;
                    }
                    current.push(q);
                }
            }
            '"' => {
                in_word = true;
                while let Some(q) = chars.next() {
                    match q {
                        '"' => break,
                        '\\' => match chars.peek() {
                            Some(&n) if matches!(n, '"' | '\\' | '$' | '`') => {
                                current.push(n);
                                chars.next();
                            }
                            Some('\n') => {
                                chars.next();
                            }
                            _ => current.push('\\'),
                        },
                        other => current.push(other),
                    }
                }
            }
            '\\' => {
                in_word = true;
                match chars.next() {
                    Some('\n') => {}
                    Some(n) => current.push(n),
                    None => current.push('\\'),
                }
            }
            other => {
                in_word = true;
                current.push(other);
            }
        }
    }
    if in_word {
        args.push(current);
    }
    args
}

fn split_windows(command: &str) -> Vec<String> {
    let chars: Vec<char> = command.chars().collect();
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_word = false;
    let mut in_quotes = false;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' {
            let mut backslashes = 0;
            while i < chars.len() && chars[i] == '\\' {
                backslashes += 1;
                i += 1;
            }
            in_word = true;
            if i < chars.len() && chars[i] == '"' {
                current.extend(std::iter::repeat_n('\\', backslashes / 2));
                if backslashes % 2 == 1 {
                    current.push('"');
                    i += 1;
                }
            } else {
                current.extend(std::iter::repeat_n('\\', backslashes));
            }
            continue;
        }
        if c == '"' {
            in_word = true;
            if in_quotes && chars.get(i + 1) == Some(&'"') {
                current.push('"');
                i += 2;
                continue;
            }
            in_quotes = !in_quotes;
            i += 1;
            continue;
        }
        if c.is_whitespace() && !in_quotes {
            if in_word {
                args.push(std::mem::take(&mut current));
                in_word = false;
            }
            i += 1;
            continue;
        }
        in_word = true;
        current.push(c);
        i += 1;
    }
    if in_word {
        args.push(current);
    }
    args
}

// ── Argument interpretation ──────────────────────────────────────────────────

/// MSVC options whose value may be the next argument and that this reader does not model.
const MSVC_SEPARATE_VALUE_OPTIONS: &[&str] = &[
    "sourceDependencies", "scanDependencies", "reference", "headerUnit", "headerUnit:quote",
    "headerUnit:angle", "ifcOutput", "ifcSearchDir", "Xclang", "o",
];

/// MSVC option prefixes with joined or no values that carry no include, macro, language or
/// standard information (`/W4`, `/O2`, `/EHsc`, `/MD`, `/Zi`, ...).
const MSVC_IGNORED_OPTION_PREFIXES: &[&str] = &[
    "F", "W", "O", "Z", "E", "G", "M", "Y", "nologo", "arch:", "permissive", "source-charset",
    "execution-charset", "utf-8", "bigobj", "sdl", "RTC", "diagnostics", "analyze", "JMC",
    "showIncludes", "experimental:", "translateInclude", "interface", "internalPartition",
    "exportHeader", "volatile:", "fp:", "openmp", "favor:", "Qpar", "Qspectre", "guard:",
    "await", "constexpr:", "errorReport:", "options:", "doc", "kernel", "hotpatch", "LD",
    "wd", "we", "wo", "w1", "w2", "w3", "w4", "showFilenames", "fsanitize", "clr", "Bt",
    "cgthreads", "d1", "d2", "Qvec", "Qimprecise_fwaits", "Qfast_transcendentals",
];

/// GCC/Clang options whose value is the next argument and that this reader does not model;
/// skipping them keeps their values from being taken as inputs.
const GCC_SEPARATE_VALUE_OPTIONS: &[&str] = &[
    "-MF", "-MT", "-MQ", "-Xclang", "-Xlinker", "-Xpreprocessor", "-Xassembler", "-Xarch_device",
    "-Xarch_host", "-Xcuda-fatbinary", "-Xcuda-ptxas", "-arch", "-include-pch", "-ivfsoverlay",
    "-iprefix", "-iwithprefix", "-iwithprefixbefore", "-L", "-l", "-T", "-u", "-z", "-e",
    "-aux-info", "-dumpbase", "-dumpdir", "-module-dependency-dir", "-main-file-name", "-mllvm",
    "-Xcompiler", "-Xptxas", "-Xnvlink", "-ccbin", "-B", "-G", "--param", "-specs", "-wrapper",
    "-serialize-diagnostics", "--serialize-diagnostics", "-fmodule-file",
    "-fmodule-output", "-ftemplate-depth", "-index-store-path",
];

/// Matches `flag` joined (`-Ifoo`) or as a separate argument (`-I foo`) and advances `i`
/// past the value. A trailing flag without a value yields an empty string.
fn take_flag_value(args: &[String], i: &mut usize, flag: &str, joined: bool, separate: bool) -> Option<String> {
    let arg = args[*i].as_str();
    if separate && arg == flag {
        let value = args.get(*i + 1).cloned().unwrap_or_default();
        *i += 2;
        return Some(value);
    }
    if joined && arg.len() > flag.len() && arg.starts_with(flag) {
        let value = arg[flag.len()..].to_string();
        *i += 1;
        return Some(value);
    }
    None
}

/// Language named by `-x` (`None` for languages outside the C family or unknown).
fn language_from_x(value: &str) -> Option<LangId> {
    match value {
        "c" | "c-header" | "cpp-output" => Some(LangId::C),
        "c++" | "c++-header" | "c++-cpp-output" | "c++-module" => Some(LangId::Cpp),
        _ => None,
    }
}

struct Interpretation {
    language_flag: Option<Option<LangId>>,
    standard: Option<LanguageStandard>,
    includes: Vec<IncludePath>,
    forced: Vec<PathBuf>,
    macro_files: Vec<PathBuf>,
    actions: Vec<MacroAction>,
    output: Option<PathBuf>,
    sysroot: Option<PathBuf>,
    target: Option<String>,
    nostdinc: bool,
    conformant_cplusplus: bool,
    undefine_all: bool,
    inputs: Vec<PathBuf>,
}

impl Interpretation {
    fn new() -> Self {
        Self {
            language_flag: None,
            standard: None,
            includes: Vec::new(),
            forced: Vec::new(),
            macro_files: Vec::new(),
            actions: Vec::new(),
            output: None,
            sysroot: None,
            target: None,
            nostdinc: false,
            conformant_cplusplus: false,
            undefine_all: false,
            inputs: Vec::new(),
        }
    }

    fn define(&mut self, spec: &str, msvc: bool) -> bool {
        let spec = if msvc && !spec.contains('=') {
            // MSVC accepts `/DNAME#VALUE` for `/DNAME=VALUE`.
            spec.replacen('#', "=", 1)
        } else {
            spec.to_string()
        };
        match MacroDefinition::parse(&spec) {
            Some(def) => {
                self.actions.push(MacroAction::Define(def));
                true
            }
            None => false,
        }
    }

    fn undefine(&mut self, name: &str) -> bool {
        let name = name.trim();
        if crate::lang::build::is_identifier(name) {
            self.actions.push(MacroAction::Undefine(name.to_string()));
            true
        } else {
            false
        }
    }
}

/// Expands `@file` response files in place (recursively, bounded).
fn expand_response_files(args: Vec<String>, directory: &Path, style: CommandLineStyle, depth: usize, unresolved: &mut Vec<PathBuf>) -> Vec<String> {
    let mut out = Vec::with_capacity(args.len());
    for arg in args {
        let Some(spec) = arg.strip_prefix('@') else {
            out.push(arg);
            continue;
        };
        let path = resolve_path(directory, spec);
        if depth >= MAX_RESPONSE_FILE_DEPTH {
            unresolved.push(path);
            continue;
        }
        match read_build_file(&path) {
            Ok(text) => {
                let inner = split_command_line(&text, style);
                out.extend(expand_response_files(inner, directory, style, depth + 1, unresolved));
            }
            Err(_) => unresolved.push(path),
        }
    }
    out
}

fn interpret(argv: Vec<String>, directory: PathBuf, file: PathBuf, output: Option<PathBuf>) -> CompileCommand {
    // Drop compiler launchers: `ccache g++ ...` → `g++ ...`.
    let mut start = 0;
    while start + 1 < argv.len() && COMPILER_LAUNCHERS.contains(&program_stem(&argv[start]).as_str()) {
        start += 1;
    }
    let compiler = argv.get(start).cloned();
    let driver = compiler
        .as_deref()
        .map(CompilerDriver::detect)
        .unwrap_or(CompilerDriver::Unknown);
    let style = if driver.uses_msvc_syntax() || cfg!(windows) {
        CommandLineStyle::Windows
    } else {
        CommandLineStyle::Posix
    };
    let mut unresolved = Vec::new();
    let rest: Vec<String> = argv.iter().skip(start + 1).cloned().collect();
    let rest = expand_response_files(rest, &directory, style, 0, &mut unresolved);

    let mut interp = Interpretation::new();
    if driver.uses_msvc_syntax() {
        interpret_msvc(&rest, &directory, &mut interp);
    } else {
        interpret_gcc(&rest, &directory, &mut interp);
    }

    let file_key = path_key(&file);
    let other_inputs: Vec<PathBuf> = interp
        .inputs
        .iter()
        .filter(|p| path_key(p) != file_key)
        .cloned()
        .collect();

    let cpp_driver = compiler
        .as_deref()
        .map(program_stem)
        .is_some_and(|s| s.contains("++") || s == "clang-cl" || s == "cl");
    let language = match interp.language_flag {
        Some(lang) => lang,
        None => match classify_extension(&file) {
            Some(ExtensionClass::Known { lang, .. }) => Some(lang),
            Some(ExtensionClass::AmbiguousHeader) => {
                let cpp_standard = interp
                    .standard
                    .as_ref()
                    .is_some_and(|s| s.family == StandardFamily::Cpp);
                if cpp_standard || (cpp_driver && !driver.uses_msvc_syntax()) {
                    Some(LangId::Cpp)
                } else {
                    Some(LangId::C)
                }
            }
            None => None,
        },
    };

    let (standard, standard_source) = match interp.standard.take() {
        Some(s) => (Some(s), Some(StandardSource::Flag)),
        None if driver.uses_msvc_syntax() && language == Some(LangId::Cpp) => (
            Some(LanguageStandard::new(StandardFamily::Cpp, "c++14", Some(2014), false)),
            Some(StandardSource::DriverDefault),
        ),
        None => (None, None),
    };

    let mut arguments = Vec::with_capacity(rest.len() + 1);
    if let Some(c) = &compiler {
        arguments.push(c.clone());
    }
    arguments.extend(rest);

    CompileCommand {
        file,
        directory,
        output: output.or(interp.output),
        arguments,
        compiler,
        driver,
        language,
        standard,
        standard_source,
        include_paths: interp.includes,
        forced_includes: interp.forced,
        macro_files: interp.macro_files,
        macro_actions: interp.actions,
        sysroot: interp.sysroot,
        target: interp.target,
        ignores_standard_includes: interp.nostdinc,
        msvc_conformant_cplusplus: interp.conformant_cplusplus,
        msvc_undefines_all: interp.undefine_all,
        unresolved_response_files: unresolved,
        other_inputs,
    }
}

fn interpret_gcc(args: &[String], dir: &Path, out: &mut Interpretation) {
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        if arg == "--" {
            for input in &args[i + 1..] {
                out.inputs.push(resolve_path(dir, input));
            }
            break;
        }
        if GCC_SEPARATE_VALUE_OPTIONS.contains(&arg) {
            i += 2;
            continue;
        }
        // Long forms first, so `-isystem-after` is not read as `-isystem` + `-after`.
        if let Some(v) = take_flag_value(args, &mut i, "-isystem-after", true, true) {
            out.includes.push(IncludePath::new(resolve_path(dir, &v), IncludeKind::After));
            continue;
        }
        if let Some(v) = take_flag_value(args, &mut i, "-idirafter", true, true) {
            out.includes.push(IncludePath::new(resolve_path(dir, &v), IncludeKind::After));
            continue;
        }
        if let Some(v) = take_flag_value(args, &mut i, "-iquote", true, true) {
            out.includes.push(IncludePath::new(resolve_path(dir, &v), IncludeKind::Quote));
            continue;
        }
        if let Some(v) = take_flag_value(args, &mut i, "-iwithsysroot", true, true) {
            let base = out.sysroot.clone().unwrap_or_else(|| PathBuf::from("/"));
            out.includes.push(IncludePath::new(resolve_path(&base, v.trim_start_matches('/')), IncludeKind::System));
            continue;
        }
        if let Some(v) = take_flag_value(args, &mut i, "-isysroot", true, true) {
            out.sysroot = Some(resolve_path(dir, &v));
            continue;
        }
        let mut system_matched = false;
        for flag in ["-cxx-isystem", "-isystem", "-imsvc"] {
            if let Some(v) = take_flag_value(args, &mut i, flag, true, true) {
                out.includes.push(IncludePath::new(resolve_path(dir, &v), IncludeKind::System));
                system_matched = true;
                break;
            }
        }
        if system_matched {
            continue;
        }
        if let Some(v) = take_flag_value(args, &mut i, "-imacros", false, true) {
            out.macro_files.push(resolve_path(dir, &v));
            continue;
        }
        if let Some(v) = take_flag_value(args, &mut i, "-include", false, true) {
            out.forced.push(resolve_path(dir, &v));
            continue;
        }
        if let Some(v) = take_flag_value(args, &mut i, "--include-directory=", true, false)
            .or_else(|| take_flag_value(args, &mut i, "--include-directory", false, true))
        {
            out.includes.push(IncludePath::new(resolve_path(dir, &v), IncludeKind::Angled));
            continue;
        }
        if arg == "-I-" {
            i += 1;
            continue;
        }
        if let Some(v) = take_flag_value(args, &mut i, "-I", true, true) {
            out.includes.push(IncludePath::new(resolve_path(dir, &v), IncludeKind::Angled));
            continue;
        }
        if let Some(v) = take_flag_value(args, &mut i, "--define-macro=", true, false)
            .or_else(|| take_flag_value(args, &mut i, "--define-macro", false, true))
            .or_else(|| take_flag_value(args, &mut i, "-D", true, true))
        {
            out.define(&v, false);
            continue;
        }
        if arg == "-undef" {
            // `-undef`: no system-specific predefined macros; not a `-U` of `ndef`.
            i += 1;
            continue;
        }
        if let Some(v) = take_flag_value(args, &mut i, "--undefine-macro=", true, false)
            .or_else(|| take_flag_value(args, &mut i, "--undefine-macro", false, true))
            .or_else(|| take_flag_value(args, &mut i, "-U", true, true))
        {
            out.undefine(&v);
            continue;
        }
        if let Some(v) = take_flag_value(args, &mut i, "-std=", true, false)
            .or_else(|| take_flag_value(args, &mut i, "--std=", true, false))
            .or_else(|| take_flag_value(args, &mut i, "--std", false, true))
        {
            if let Some(standard) = LanguageStandard::from_gcc_flag(&v) {
                out.standard = Some(standard);
            }
            continue;
        }
        if arg == "-ansi" {
            // `-ansi` is C90 in C mode and C++98 in C++ mode; the mode is decided later.
            if out.standard.is_none() {
                out.standard = Some(LanguageStandard::new(StandardFamily::C, "c90", Some(1990), false));
            }
            i += 1;
            continue;
        }
        if let Some(v) = take_flag_value(args, &mut i, "-x", true, true) {
            out.language_flag = if v == "none" { None } else { Some(language_from_x(&v)) };
            continue;
        }
        if let Some(v) = take_flag_value(args, &mut i, "-o", true, true) {
            out.output = Some(resolve_path(dir, &v));
            continue;
        }
        if let Some(v) = take_flag_value(args, &mut i, "--sysroot=", true, false)
            .or_else(|| take_flag_value(args, &mut i, "--sysroot", false, true))
        {
            out.sysroot = Some(resolve_path(dir, &v));
            continue;
        }
        if let Some(v) = take_flag_value(args, &mut i, "--target=", true, false)
            .or_else(|| take_flag_value(args, &mut i, "-target", false, true))
        {
            out.target = Some(v);
            continue;
        }
        if arg == "-nostdinc" || arg == "-nostdinc++" {
            out.nostdinc = true;
            i += 1;
            continue;
        }
        if !arg.starts_with('-') && !arg.is_empty() {
            out.inputs.push(resolve_path(dir, arg));
        }
        i += 1;
    }
}

fn interpret_msvc(args: &[String], dir: &Path, out: &mut Interpretation) {
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].clone();
        let Some(body) = arg.strip_prefix('/').or_else(|| arg.strip_prefix('-')) else {
            out.inputs.push(resolve_path(dir, &arg));
            i += 1;
            continue;
        };
        // clang-cl passes GCC-style options through `/clang:`.
        if let Some(inner) = body.strip_prefix("clang:") {
            let single = vec![inner.to_string()];
            interpret_gcc(&single, dir, out);
            i += 1;
            continue;
        }
        let as_value = |i: &mut usize, flag: &str| -> Option<String> {
            if body == flag {
                let v = args.get(*i + 1).cloned().unwrap_or_default();
                *i += 2;
                Some(v)
            } else if body.len() > flag.len() && body.starts_with(flag) {
                let v = body[flag.len()..].trim_start_matches(':').to_string();
                *i += 1;
                Some(v)
            } else {
                None
            }
        };
        if let Some(v) = as_value(&mut i, "external:I") {
            out.includes.push(IncludePath::new(resolve_path(dir, &v), IncludeKind::System));
            continue;
        }
        if let Some(v) = as_value(&mut i, "imsvc") {
            out.includes.push(IncludePath::new(resolve_path(dir, &v), IncludeKind::System));
            continue;
        }
        if body.starts_with("external:") {
            i += 1;
            continue;
        }
        if body == "X" {
            out.nostdinc = true;
            i += 1;
            continue;
        }
        if body == "u" {
            out.undefine_all = true;
            i += 1;
            continue;
        }
        if body == "TP" {
            out.language_flag = Some(Some(LangId::Cpp));
            i += 1;
            continue;
        }
        if body == "TC" {
            out.language_flag = Some(Some(LangId::C));
            i += 1;
            continue;
        }
        if let Some(v) = as_value(&mut i, "Tp") {
            out.language_flag = Some(Some(LangId::Cpp));
            out.inputs.push(resolve_path(dir, &v));
            continue;
        }
        if let Some(v) = as_value(&mut i, "Tc") {
            out.language_flag = Some(Some(LangId::C));
            out.inputs.push(resolve_path(dir, &v));
            continue;
        }
        if let Some(v) = as_value(&mut i, "std:") {
            if let Some(standard) = LanguageStandard::from_msvc_flag(&v) {
                out.standard = Some(standard);
            }
            continue;
        }
        if body == "Zc:__cplusplus" {
            out.conformant_cplusplus = true;
            i += 1;
            continue;
        }
        if body == "Zc:__cplusplus-" {
            out.conformant_cplusplus = false;
            i += 1;
            continue;
        }
        if let Some(v) = as_value(&mut i, "FI") {
            out.forced.push(resolve_path(dir, &v));
            continue;
        }
        if let Some(v) = as_value(&mut i, "Fo") {
            out.output = Some(resolve_path(dir, &v));
            continue;
        }
        if body == "link" {
            // Everything after `/link` is a linker option.
            break;
        }
        if MSVC_SEPARATE_VALUE_OPTIONS.contains(&body) {
            i += 2;
            continue;
        }
        if body.starts_with('F') && body.len() <= 3 && body.ends_with(':') {
            // `/Fe: name`, `/Fd: name`: the value is the next argument.
            i += 2;
            continue;
        }
        if MSVC_IGNORED_OPTION_PREFIXES.iter().any(|p| body.starts_with(p)) || body == "c" {
            i += 1;
            continue;
        }
        // `/I`, `/D` and `/U` come last: a Unix absolute input path such as `/Data/a.c`
        // must not be read as `/D`. Values that are not valid for the option fall back to
        // being inputs.
        let mut probe = i;
        if let Some(v) = as_value(&mut probe, "I") {
            out.includes.push(IncludePath::new(resolve_path(dir, &v), IncludeKind::Angled));
            i = probe;
            continue;
        }
        let mut probe = i;
        if let Some(v) = as_value(&mut probe, "D") {
            if out.define(&v, true) {
                i = probe;
                continue;
            }
        }
        let mut probe = i;
        if let Some(v) = as_value(&mut probe, "U") {
            if out.undefine(&v) {
                i = probe;
                continue;
            }
        }
        if arg.starts_with('/') && !cfg!(windows) && arg[1..].contains('/') {
            out.inputs.push(resolve_path(dir, &arg));
        }
        i += 1;
    }
}
