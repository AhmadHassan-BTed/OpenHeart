//! MSBuild reader: solutions (`.sln`, `.slnx`), projects (`.csproj`, `.vcxproj`,
//! `.fsproj`, `.vbproj`), the directory-level files MSBuild imports implicitly
//! (`Directory.Build.props`, `Directory.Build.targets`, `Directory.Packages.props`) and
//! legacy `packages.config`.
//!
//! Projects are evaluated the way MSBuild does, in passes: properties and imports in
//! document order (with `Directory.Build.props` at the SDK / `Microsoft.Common.props`
//! import point and `Directory.Build.targets` at the targets import point), then item
//! definitions and items against the final property values. Conditions are evaluated with
//! three-valued logic: a condition that depends on something unknown (an environment
//! variable, an SDK property, a host-dependent function) is `Unknown`, and the content it
//! guards is kept with `conditional = true` instead of being guessed away. Undefined
//! properties and unsupported property functions stay symbolic. Both SDK-style and legacy
//! projects are understood; Visual C++ projects additionally yield include directories,
//! preprocessor definitions, forced includes and language standards.
//!
//! The XML reader is hand-written (elements, attributes, text, CDATA, comments, processing
//! instructions, DOCTYPE, entity and character references) and reports errors with line
//! numbers.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Component, Path, PathBuf};

use thiserror::Error;

use crate::core::types::LangId;
use crate::lang::build::{
    expand_glob, has_wildcard, normalize_path, path_key, read_build_file, resolve_path,
    to_native_separators, BuildDiagnostic, GlobOptions, GlobPattern, IncludeKind, IncludePath,
    LanguageStandard, MacroDefinition,
};

// ── XML ──────────────────────────────────────────────────────────────────────

/// An XML element. `name` is the local name (namespace prefix removed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XmlElement {
    pub name: String,
    pub attributes: Vec<(String, String)>,
    pub children: Vec<XmlNode>,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum XmlNode {
    Element(XmlElement),
    Text(String),
}

impl XmlElement {
    /// Attribute value by name (case-insensitive, as MSBuild treats its attributes).
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn elements(&self) -> impl Iterator<Item = &XmlElement> {
        self.children.iter().filter_map(|c| match c {
            XmlNode::Element(e) => Some(e),
            XmlNode::Text(_) => None,
        })
    }

    /// First child element named `name` (case-insensitive).
    pub fn child(&self, name: &str) -> Option<&XmlElement> {
        self.elements().find(|e| e.name.eq_ignore_ascii_case(name))
    }

    /// Concatenated text content of the direct children, trimmed.
    pub fn text(&self) -> String {
        let mut out = String::new();
        for child in &self.children {
            if let XmlNode::Text(t) = child {
                out.push_str(t);
            }
        }
        out.trim().to_string()
    }

    fn is(&self, name: &str) -> bool {
        self.name.eq_ignore_ascii_case(name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("line {line}: {message}")]
pub struct XmlError {
    pub line: u32,
    pub message: String,
}

/// Parses an XML document and returns its root element.
pub fn parse_xml(text: &str) -> Result<XmlElement, XmlError> {
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    let mut parser = XmlParser {
        src: text,
        bytes: text.as_bytes(),
        pos: 0,
        line: 1,
    };
    parser.document()
}

struct XmlParser<'a> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
    line: u32,
}

impl<'a> XmlParser<'a> {
    fn error(&self, message: impl Into<String>) -> XmlError {
        XmlError {
            line: self.line,
            message: message.into(),
        }
    }

    fn starts_with(&self, s: &str) -> bool {
        self.bytes[self.pos..].starts_with(s.as_bytes())
    }

    /// Moves to `new_pos`, counting newlines.
    fn advance_to(&mut self, new_pos: usize) {
        let end = new_pos.min(self.bytes.len());
        self.line += self.bytes[self.pos..end].iter().filter(|&&b| b == b'\n').count() as u32;
        self.pos = end;
    }

    /// Index of `needle` at or after `self.pos`.
    fn find(&self, needle: &str) -> Option<usize> {
        self.src[self.pos..].find(needle).map(|i| self.pos + i)
    }

    fn skip_whitespace(&mut self) {
        while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_whitespace() {
            if self.bytes[self.pos] == b'\n' {
                self.line += 1;
            }
            self.pos += 1;
        }
    }

    fn document(&mut self) -> Result<XmlElement, XmlError> {
        let mut stack: Vec<XmlElement> = Vec::new();
        let mut root: Option<XmlElement> = None;
        while self.pos < self.bytes.len() {
            if self.starts_with("<?") {
                let end = self.find("?>").ok_or_else(|| self.error("unterminated processing instruction"))?;
                self.advance_to(end + 2);
            } else if self.starts_with("<!--") {
                let end = self.find("-->").ok_or_else(|| self.error("unterminated comment"))?;
                self.advance_to(end + 3);
            } else if self.starts_with("<![CDATA[") {
                let start = self.pos + 9;
                let end = self.src[start..]
                    .find("]]>")
                    .map(|i| start + i)
                    .ok_or_else(|| self.error("unterminated CDATA section"))?;
                let content = self.src[start..end].to_string();
                self.advance_to(end + 3);
                match stack.last_mut() {
                    Some(parent) => parent.children.push(XmlNode::Text(content)),
                    None => return Err(self.error("CDATA section outside the root element")),
                }
            } else if self.starts_with("<!") {
                self.skip_declaration()?;
            } else if self.starts_with("</") {
                let start = self.pos + 2;
                let end = self.find(">").ok_or_else(|| self.error("unterminated closing tag"))?;
                let name = local_name(self.src[start..end].trim());
                self.advance_to(end + 1);
                let element = stack
                    .pop()
                    .ok_or_else(|| self.error(format!("closing tag </{}> without an open element", name)))?;
                if !element.name.eq_ignore_ascii_case(&name) {
                    return Err(self.error(format!(
                        "closing tag </{}> does not match <{}> opened at line {}",
                        name, element.name, element.line
                    )));
                }
                self.attach(&mut stack, &mut root, element)?;
            } else if self.starts_with("<") {
                let (element, self_closing) = self.open_tag()?;
                if self_closing {
                    self.attach(&mut stack, &mut root, element)?;
                } else {
                    stack.push(element);
                }
            } else {
                let end = self.find("<").unwrap_or(self.bytes.len());
                let raw = &self.src[self.pos..end];
                let line = self.line;
                match stack.last_mut() {
                    Some(parent) => {
                        let text = unescape_xml(raw, line)?;
                        parent.children.push(XmlNode::Text(text));
                    }
                    None => {
                        if !raw.trim().is_empty() {
                            return Err(self.error("text outside the root element"));
                        }
                    }
                }
                self.advance_to(end);
            }
        }
        if let Some(open) = stack.last() {
            return Err(XmlError {
                line: open.line,
                message: format!("element <{}> is not closed", open.name),
            });
        }
        root.ok_or_else(|| self.error("the document has no root element"))
    }

    fn attach(&self, stack: &mut [XmlElement], root: &mut Option<XmlElement>, element: XmlElement) -> Result<(), XmlError> {
        match stack.last_mut() {
            Some(parent) => {
                parent.children.push(XmlNode::Element(element));
                Ok(())
            }
            None if root.is_none() => {
                *root = Some(element);
                Ok(())
            }
            None => Err(self.error("more than one root element")),
        }
    }

    /// Skips `<!DOCTYPE ...>` (with an internal subset) or another `<!...>` declaration.
    fn skip_declaration(&mut self) -> Result<(), XmlError> {
        let mut depth = 0i32;
        let mut quote: Option<u8> = None;
        let mut i = self.pos + 2;
        while i < self.bytes.len() {
            let b = self.bytes[i];
            match quote {
                Some(q) if b == q => quote = None,
                Some(_) => {}
                None => match b {
                    b'"' | b'\'' => quote = Some(b),
                    b'[' => depth += 1,
                    b']' => depth -= 1,
                    b'>' if depth <= 0 => {
                        self.advance_to(i + 1);
                        return Ok(());
                    }
                    _ => {}
                },
            }
            i += 1;
        }
        Err(self.error("unterminated declaration"))
    }

    fn read_name(&mut self) -> String {
        let start = self.pos;
        while self.pos < self.bytes.len() {
            let b = self.bytes[self.pos];
            if b.is_ascii_whitespace() || b == b'/' || b == b'>' || b == b'=' {
                break;
            }
            self.pos += 1;
        }
        self.src[start..self.pos].to_string()
    }

    fn open_tag(&mut self) -> Result<(XmlElement, bool), XmlError> {
        let line = self.line;
        self.pos += 1;
        let name = self.read_name();
        if name.is_empty() {
            return Err(self.error("element without a name"));
        }
        let mut element = XmlElement {
            name: local_name(&name),
            attributes: Vec::new(),
            children: Vec::new(),
            line,
        };
        loop {
            self.skip_whitespace();
            if self.pos >= self.bytes.len() {
                return Err(XmlError {
                    line,
                    message: format!("unterminated start tag <{}>", name),
                });
            }
            if self.starts_with("/>") {
                self.pos += 2;
                return Ok((element, true));
            }
            if self.starts_with(">") {
                self.pos += 1;
                return Ok((element, false));
            }
            let attr_name = self.read_name();
            if attr_name.is_empty() {
                return Err(self.error(format!("malformed attribute in <{}>", name)));
            }
            self.skip_whitespace();
            if !self.starts_with("=") {
                return Err(self.error(format!("attribute '{}' has no value", attr_name)));
            }
            self.pos += 1;
            self.skip_whitespace();
            let quote = match self.bytes.get(self.pos) {
                Some(&q) if q == b'"' || q == b'\'' => q,
                _ => return Err(self.error(format!("attribute '{}' value is not quoted", attr_name))),
            };
            let start = self.pos + 1;
            let end = self.bytes[start..]
                .iter()
                .position(|&b| b == quote)
                .map(|i| start + i)
                .ok_or_else(|| self.error(format!("unterminated value of attribute '{}'", attr_name)))?;
            let value_line = self.line;
            let raw = &self.src[start..end];
            // Attribute-value normalization: literal tabs and newlines become spaces.
            let normalized: String = raw
                .chars()
                .map(|c| if c == '\t' || c == '\n' || c == '\r' { ' ' } else { c })
                .collect();
            let value = unescape_xml(&normalized, value_line)?;
            self.advance_to(end + 1);
            element.attributes.push((local_name(&attr_name), value));
        }
    }
}

fn local_name(name: &str) -> String {
    match name.rfind(':') {
        Some(i) if !name.starts_with("xmlns") => name[i + 1..].to_string(),
        _ => name.to_string(),
    }
}

/// Replaces entity and character references. Unknown entities are kept literally.
fn unescape_xml(raw: &str, line: u32) -> Result<String, XmlError> {
    if !raw.contains('&') {
        return Ok(raw.to_string());
    }
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after = &rest[amp + 1..];
        let Some(semi) = after.find(';').filter(|&s| s <= 12) else {
            out.push('&');
            rest = after;
            continue;
        };
        let entity = &after[..semi];
        let decoded = match entity {
            "lt" => Some('<'),
            "gt" => Some('>'),
            "amp" => Some('&'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => {
                let code = if let Some(hex) = entity.strip_prefix("#x").or_else(|| entity.strip_prefix("#X")) {
                    u32::from_str_radix(hex, 16).ok()
                } else if let Some(dec) = entity.strip_prefix('#') {
                    dec.parse::<u32>().ok()
                } else {
                    None
                };
                match code {
                    Some(c) => Some(char::from_u32(c).ok_or_else(|| XmlError {
                        line,
                        message: format!("invalid character reference &{};", entity),
                    })?),
                    None => None,
                }
            }
        };
        match decoded {
            Some(c) => out.push(c),
            None => {
                out.push('&');
                out.push_str(entity);
                out.push(';');
            }
        }
        rest = &after[semi + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

// ── Errors ───────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum MsBuildError {
    #[error("failed to read '{path}': {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("'{path}': {source}")]
    Xml {
        path: PathBuf,
        #[source]
        source: XmlError,
    },
    #[error("'{path}': {message}")]
    Format { path: PathBuf, message: String },
}

fn load_xml(path: &Path) -> Result<XmlElement, MsBuildError> {
    let text = read_build_file(path).map_err(|source| MsBuildError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    parse_xml(&text).map_err(|source| MsBuildError::Xml {
        path: path.to_path_buf(),
        source,
    })
}

// ── Languages and project kinds ──────────────────────────────────────────────

/// Language of an MSBuild project.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProjectLanguage {
    CSharp,
    Cpp,
    VisualBasic,
    FSharp,
    Unknown,
}

impl ProjectLanguage {
    /// The analyzable language, when the C-family frontends cover it.
    pub fn lang_id(self) -> Option<LangId> {
        match self {
            ProjectLanguage::CSharp => Some(LangId::CSharp),
            ProjectLanguage::Cpp => Some(LangId::Cpp),
            _ => None,
        }
    }

    pub fn from_path(path: &Path) -> Self {
        match path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref()
        {
            Some("csproj") => ProjectLanguage::CSharp,
            Some("vcxproj") | Some("vcproj") => ProjectLanguage::Cpp,
            Some("vbproj") => ProjectLanguage::VisualBasic,
            Some("fsproj") => ProjectLanguage::FSharp,
            _ => ProjectLanguage::Unknown,
        }
    }

    /// Source extension of the SDK default `Compile` glob.
    fn default_compile_extension(self) -> Option<&'static str> {
        match self {
            ProjectLanguage::CSharp => Some("cs"),
            ProjectLanguage::VisualBasic => Some("vb"),
            _ => None,
        }
    }
}

const GUID_SOLUTION_FOLDER: &str = "2150E333-8FDC-42A3-9474-1A3956D46DE8";
const GUID_SHARED_PROJECT: &str = "D954291E-2A0B-460D-934E-DC6B0785DB48";

fn language_from_type_guid(guid: &str) -> Option<ProjectLanguage> {
    match guid.trim_matches(|c| c == '{' || c == '}').to_ascii_uppercase().as_str() {
        "FAE04EC0-301F-11D3-BF4B-00C04F79EFBC" | "9A19103F-16F7-4668-BE54-9A1E7A4F7556" => Some(ProjectLanguage::CSharp),
        "8BC9CEB8-8B4A-11D0-8D11-00A0C91BC942" => Some(ProjectLanguage::Cpp),
        "F184B08F-C81C-45F6-A57F-5ABD9991F28F" | "778DAE3C-4631-46EA-AA77-85C1314464D9" => Some(ProjectLanguage::VisualBasic),
        "F2A71F9B-5D33-465A-A702-920D77279786" | "6EC3EE1D-3C4E-46DD-8F32-0CC8E7565705" => Some(ProjectLanguage::FSharp),
        _ => None,
    }
}

// ── Solutions ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SolutionFormat {
    /// Classic text `.sln`.
    Sln,
    /// XML `.slnx`.
    Slnx,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SolutionItemKind {
    Project(ProjectLanguage),
    /// `.shproj` shared project (sources compiled into referencing projects).
    SharedProject,
    Folder,
}

/// One entry of a solution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SolutionProject {
    pub name: String,
    /// Path as written in the solution (`src\App\App.csproj`); the folder name for folders.
    pub relative_path: String,
    /// Absolute project path (`None` for solution folders).
    pub path: Option<PathBuf>,
    /// Project GUID (`.sln`), or the relative path for `.slnx` entries, which have none.
    pub id: String,
    pub type_guid: Option<String>,
    pub kind: SolutionItemKind,
    /// Ids of projects this one must build after (`ProjectDependencies` / `BuildDependency`).
    pub dependencies: Vec<String>,
    /// Solution-folder names from the outermost folder inwards.
    pub folder_path: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Solution {
    pub path: PathBuf,
    pub format: SolutionFormat,
    pub format_version: Option<String>,
    pub projects: Vec<SolutionProject>,
    /// `Configuration|Platform` pairs.
    pub configurations: Vec<String>,
    pub diagnostics: Vec<BuildDiagnostic>,
}

impl Solution {
    pub fn load(path: &Path) -> Result<Self, MsBuildError> {
        let is_slnx = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("slnx"));
        if is_slnx {
            Self::load_slnx(path)
        } else {
            let text = read_build_file(path).map_err(|source| MsBuildError::Io {
                path: path.to_path_buf(),
                source,
            })?;
            Self::parse_sln(&text, path)
        }
    }

    /// Projects (not folders), in solution order.
    pub fn project_entries(&self) -> impl Iterator<Item = &SolutionProject> {
        self.projects
            .iter()
            .filter(|p| !matches!(p.kind, SolutionItemKind::Folder))
    }

    /// Parses classic `.sln` text.
    pub fn parse_sln(text: &str, path: &Path) -> Result<Self, MsBuildError> {
        let dir = parent_dir(path);
        let mut solution = Solution {
            path: path.to_path_buf(),
            format: SolutionFormat::Sln,
            format_version: None,
            projects: Vec::new(),
            configurations: Vec::new(),
            diagnostics: Vec::new(),
        };
        let mut saw_header = false;
        let mut nested: Vec<(String, String)> = Vec::new();
        let mut section: Option<String> = None;
        let mut current: Option<usize> = None;

        for (index, raw_line) in text.lines().enumerate() {
            let line_no = index as u32 + 1;
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(rest) = line.strip_prefix("Microsoft Visual Studio Solution File, Format Version") {
                saw_header = true;
                solution.format_version = Some(rest.trim().to_string());
                continue;
            }
            if line.starts_with("Project(") {
                let quoted = quoted_strings(line);
                if quoted.len() < 4 {
                    solution.diagnostics.push(BuildDiagnostic::warning(path, line_no, "malformed Project line"));
                    continue;
                }
                let type_guid = quoted[0].clone();
                let name = quoted[1].clone();
                let relative = quoted[2].clone();
                let id = quoted[3].trim_matches(|c| c == '{' || c == '}').to_ascii_uppercase();
                let type_upper = type_guid.trim_matches(|c| c == '{' || c == '}').to_ascii_uppercase();
                let kind = if type_upper == GUID_SOLUTION_FOLDER {
                    SolutionItemKind::Folder
                } else if type_upper == GUID_SHARED_PROJECT {
                    SolutionItemKind::SharedProject
                } else {
                    let resolved = resolve_path(&dir, &relative);
                    SolutionItemKind::Project(
                        language_from_type_guid(&type_guid).unwrap_or_else(|| ProjectLanguage::from_path(&resolved)),
                    )
                };
                let project_path = match kind {
                    SolutionItemKind::Folder => None,
                    _ => Some(resolve_path(&dir, &relative)),
                };
                current = Some(solution.projects.len());
                solution.projects.push(SolutionProject {
                    name,
                    relative_path: relative,
                    path: project_path,
                    id,
                    type_guid: Some(type_upper),
                    kind,
                    dependencies: Vec::new(),
                    folder_path: Vec::new(),
                });
                continue;
            }
            if line == "EndProject" {
                current = None;
                continue;
            }
            if let Some(rest) = line.strip_prefix("ProjectSection(").or_else(|| line.strip_prefix("GlobalSection(")) {
                section = rest.split(')').next().map(|s| s.trim().to_string());
                continue;
            }
            if line == "EndProjectSection" || line == "EndGlobalSection" {
                section = None;
                continue;
            }
            let Some((left, right)) = line.split_once('=') else {
                continue;
            };
            let (left, right) = (left.trim(), right.trim());
            match section.as_deref() {
                Some("ProjectDependencies") => {
                    if let Some(index) = current {
                        let dep = left.trim_matches(|c| c == '{' || c == '}').to_ascii_uppercase();
                        solution.projects[index].dependencies.push(dep);
                    }
                }
                Some("SolutionConfigurationPlatforms") => {
                    if !solution.configurations.contains(&left.to_string()) {
                        solution.configurations.push(left.to_string());
                    }
                }
                Some("NestedProjects") => {
                    let child = left.trim_matches(|c| c == '{' || c == '}').to_ascii_uppercase();
                    let parent = right.trim_matches(|c| c == '{' || c == '}').to_ascii_uppercase();
                    nested.push((child, parent));
                }
                _ => {}
            }
        }
        if !saw_header {
            return Err(MsBuildError::Format {
                path: path.to_path_buf(),
                message: "missing the 'Microsoft Visual Studio Solution File' header".to_string(),
            });
        }

        // Folder paths from NestedProjects.
        let parent_of: HashMap<String, String> = nested.into_iter().collect();
        let name_of: HashMap<String, String> = solution
            .projects
            .iter()
            .map(|p| (p.id.clone(), p.name.clone()))
            .collect();
        for project in &mut solution.projects {
            let mut chain = Vec::new();
            let mut cursor = parent_of.get(&project.id);
            let mut guard = 0;
            while let Some(parent) = cursor {
                guard += 1;
                if guard > 64 {
                    break;
                }
                if let Some(name) = name_of.get(parent) {
                    chain.push(name.clone());
                }
                cursor = parent_of.get(parent);
            }
            chain.reverse();
            project.folder_path = chain;
        }
        for project in &solution.projects {
            if let Some(p) = &project.path {
                if !p.is_file() {
                    solution.diagnostics.push(BuildDiagnostic::warning(
                        path,
                        0,
                        format!("project '{}' listed in the solution does not exist at {}", project.name, p.display()),
                    ));
                }
            }
        }
        Ok(solution)
    }

    fn load_slnx(path: &Path) -> Result<Self, MsBuildError> {
        let root = load_xml(path)?;
        if !root.is("Solution") {
            return Err(MsBuildError::Format {
                path: path.to_path_buf(),
                message: format!("root element is <{}>, expected <Solution>", root.name),
            });
        }
        let dir = parent_dir(path);
        let mut solution = Solution {
            path: path.to_path_buf(),
            format: SolutionFormat::Slnx,
            format_version: None,
            projects: Vec::new(),
            configurations: Vec::new(),
            diagnostics: Vec::new(),
        };
        let mut build_types = Vec::new();
        let mut platforms = Vec::new();
        if let Some(configs) = root.child("Configurations") {
            for e in configs.elements() {
                if let Some(name) = e.attr("Name") {
                    if e.is("BuildType") {
                        build_types.push(name.to_string());
                    } else if e.is("Platform") {
                        platforms.push(name.to_string());
                    }
                }
            }
        }
        if build_types.is_empty() {
            build_types = vec!["Debug".to_string(), "Release".to_string()];
        }
        if platforms.is_empty() {
            platforms = vec!["Any CPU".to_string()];
        }
        for b in &build_types {
            for p in &platforms {
                solution.configurations.push(format!("{}|{}", b, p));
            }
        }
        Self::slnx_children(&root, &dir, &[], &mut solution);
        Ok(solution)
    }

    fn slnx_children(element: &XmlElement, dir: &Path, folders: &[String], solution: &mut Solution) {
        for e in element.elements() {
            if e.is("Folder") {
                let name = e.attr("Name").unwrap_or("");
                let parts: Vec<String> = name
                    .split('/')
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect();
                solution.projects.push(SolutionProject {
                    name: parts.last().cloned().unwrap_or_default(),
                    relative_path: name.to_string(),
                    path: None,
                    id: name.to_string(),
                    type_guid: None,
                    kind: SolutionItemKind::Folder,
                    dependencies: Vec::new(),
                    folder_path: parts[..parts.len().saturating_sub(1)].to_vec(),
                });
                Self::slnx_children(e, dir, &parts, solution);
            } else if e.is("Project") {
                let Some(rel) = e.attr("Path") else {
                    continue;
                };
                let resolved = resolve_path(dir, rel);
                let language = e
                    .attr("Type")
                    .and_then(language_from_type_guid)
                    .unwrap_or_else(|| ProjectLanguage::from_path(&resolved));
                let is_shared = resolved
                    .extension()
                    .and_then(|x| x.to_str())
                    .is_some_and(|x| x.eq_ignore_ascii_case("shproj"));
                let dependencies = e
                    .elements()
                    .filter(|d| d.is("BuildDependency"))
                    .filter_map(|d| d.attr("Project"))
                    .map(|p| path_key(&resolve_path(dir, p)))
                    .collect();
                let name = resolved
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| rel.to_string());
                if !resolved.is_file() {
                    solution.diagnostics.push(BuildDiagnostic::warning(
                        &solution.path,
                        e.line,
                        format!("project '{}' listed in the solution does not exist at {}", name, resolved.display()),
                    ));
                }
                solution.projects.push(SolutionProject {
                    name,
                    relative_path: rel.to_string(),
                    id: path_key(&resolved),
                    path: Some(resolved),
                    type_guid: e.attr("Type").map(str::to_string),
                    kind: if is_shared {
                        SolutionItemKind::SharedProject
                    } else {
                        SolutionItemKind::Project(language)
                    },
                    dependencies,
                    folder_path: folders.to_vec(),
                });
            }
        }
    }
}

/// The `"..."` segments of a line, in order.
fn quoted_strings(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = line;
    while let Some(start) = rest.find('"') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('"') else {
            break;
        };
        out.push(after[..end].to_string());
        rest = &after[end + 1..];
    }
    out
}

fn parent_dir(path: &Path) -> PathBuf {
    match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

// ── Project model ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectFormat {
    /// `<Project Sdk="...">` / `<Sdk Name="..."/>` / `<Import Sdk="..."/>`.
    SdkStyle { sdk: String },
    Legacy,
}

/// `OutputType` of a managed project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutputType {
    Exe,
    WinExe,
    Library,
    Module,
    AppContainerExe,
    WinMdObj,
    Other(String),
}

impl OutputType {
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "exe" => OutputType::Exe,
            "winexe" => OutputType::WinExe,
            "library" => OutputType::Library,
            "module" => OutputType::Module,
            "appcontainerexe" => OutputType::AppContainerExe,
            "winmdobj" => OutputType::WinMdObj,
            _ => OutputType::Other(value.trim().to_string()),
        }
    }

    pub fn is_executable(&self) -> bool {
        matches!(self, OutputType::Exe | OutputType::WinExe | OutputType::AppContainerExe)
    }
}

/// C# `Nullable` context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NullableContext {
    Enable,
    Disable,
    Warnings,
    Annotations,
}

impl NullableContext {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "enable" | "true" => Some(NullableContext::Enable),
            "disable" | "false" => Some(NullableContext::Disable),
            "warnings" | "safeonly" | "safeonlywarnings" => Some(NullableContext::Warnings),
            "annotations" => Some(NullableContext::Annotations),
            _ => None,
        }
    }
}

/// Visual C++ `ConfigurationType`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VcxConfigurationType {
    Application,
    DynamicLibrary,
    StaticLibrary,
    Utility,
    Makefile,
    Other(String),
}

impl VcxConfigurationType {
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "application" => VcxConfigurationType::Application,
            "dynamiclibrary" => VcxConfigurationType::DynamicLibrary,
            "staticlibrary" => VcxConfigurationType::StaticLibrary,
            "utility" => VcxConfigurationType::Utility,
            "makefile" => VcxConfigurationType::Makefile,
            _ => VcxConfigurationType::Other(value.trim().to_string()),
        }
    }
}

/// An evaluated property.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Property {
    /// Name as first written.
    pub name: String,
    pub value: String,
    /// The value contains unresolved references.
    pub symbolic: bool,
    /// Assigned under a condition that could not be decided.
    pub conditional: bool,
    /// Values of undecided conditional assignments that may replace `value`.
    pub alternatives: Vec<String>,
    /// File that assigned the value (the project or an imported file).
    pub defined_in: PathBuf,
}

/// A source file item (`Compile`, `ClCompile`, `ClInclude`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceItem {
    pub path: PathBuf,
    pub exists: bool,
    pub conditional: bool,
    /// Produced by the SDK default `**/*.cs` glob rather than an explicit item.
    pub from_default_glob: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectReference {
    pub include: String,
    pub path: Option<PathBuf>,
    /// `ReferenceOutputAssembly="false"`: a build-order-only reference.
    pub reference_output_assembly: bool,
    pub conditional: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionSource {
    /// `Version` on the reference.
    Explicit,
    /// `VersionOverride` on the reference (central package management).
    VersionOverride,
    /// `PackageVersion` in `Directory.Packages.props`.
    Central,
    /// Legacy `packages.config`.
    PackagesConfig,
    /// No version could be found.
    Missing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageReference {
    pub name: String,
    pub version: Option<String>,
    pub version_source: VersionSource,
    /// `PrivateAssets="all"` / `developmentDependency`: analyzers, build tools.
    pub development_only: bool,
    pub conditional: bool,
}

/// A legacy `<Reference>` to an assembly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssemblyReference {
    pub name: String,
    pub version: Option<String>,
    pub hint_path: Option<PathBuf>,
    /// No `HintPath`: resolved from the framework / GAC.
    pub framework: bool,
    pub conditional: bool,
}

/// A global using directive (`<Using>` item or an SDK implicit using).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsingDirective {
    pub namespace: String,
    pub alias: Option<String>,
    pub is_static: bool,
    /// Added by the SDK because `ImplicitUsings` is enabled.
    pub implicit: bool,
}

/// Visual C++ compilation settings for the evaluated configuration.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CppSettings {
    pub configuration_type: Option<VcxConfigurationType>,
    pub include_paths: Vec<IncludePath>,
    pub definitions: Vec<MacroDefinition>,
    /// Definitions the toolset adds implicitly (`UNICODE`, `_WINDLL`, `_DEBUG`, `_DLL`, ...).
    pub implicit_definitions: Vec<MacroDefinition>,
    pub forced_includes: Vec<PathBuf>,
    pub cpp_standard: Option<LanguageStandard>,
    pub c_standard: Option<LanguageStandard>,
    /// `Link/AdditionalDependencies` (`kernel32.lib`, `ws2_32.lib`, ...).
    pub additional_dependencies: Vec<String>,
    pub platform_toolset: Option<String>,
    pub character_set: Option<String>,
    /// Include directories or definitions that could not be resolved.
    pub unresolved: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportStatus {
    Imported,
    /// An SDK import (`Sdk="..."`); the SDK's own files are not read.
    Sdk,
    /// A toolset file (`$(MSBuildToolsPath)`, `$(VCTargetsPath)`); not read.
    Toolset,
    /// The path has unresolved references.
    Symbolic,
    NotFound,
    ConditionFalse,
    /// Already imported (MSBuild ignores repeated imports).
    Duplicate,
    /// Imports itself through a chain of imports.
    Cycle,
    /// The file could not be read or parsed.
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportRecord {
    pub spec: String,
    pub resolved: Option<PathBuf>,
    pub status: ImportStatus,
    pub conditional: bool,
    pub importing_file: PathBuf,
}

/// Configuration and platform an evaluation uses, plus optional global properties.
#[derive(Debug, Clone, Default)]
pub struct EvaluationContext {
    /// `None` = `Debug`.
    pub configuration: Option<String>,
    /// `None` = `AnyCPU` for managed projects, the first `ProjectConfiguration` with the
    /// chosen configuration for Visual C++ projects.
    pub platform: Option<String>,
    pub solution_dir: Option<PathBuf>,
    /// Global properties (`/p:Name=Value`); projects cannot override them.
    pub global_properties: BTreeMap<String, String>,
}

/// An evaluated MSBuild project.
#[derive(Debug, Clone)]
pub struct MsBuildProject {
    pub path: PathBuf,
    pub directory: PathBuf,
    /// `MSBuildProjectName` (the file stem).
    pub name: String,
    pub language: ProjectLanguage,
    pub format: ProjectFormat,
    pub configuration: String,
    pub platform: String,
    /// Every evaluated property, keyed by lower-case name.
    pub properties: BTreeMap<String, Property>,
    pub assembly_name: Option<String>,
    pub root_namespace: Option<String>,
    /// `TargetFrameworks` (or the single `TargetFramework`, or the legacy
    /// `TargetFrameworkVersion` as a short TFM such as `net472`).
    pub target_frameworks: Vec<String>,
    pub output_type: Option<OutputType>,
    pub implicit_usings: Option<bool>,
    pub nullable: Option<NullableContext>,
    pub lang_version: Option<LanguageStandard>,
    pub define_constants: Vec<String>,
    pub project_guid: Option<String>,
    /// `Compile` (managed) or `ClCompile` (Visual C++) items.
    pub compile_items: Vec<SourceItem>,
    /// `ClInclude` items.
    pub header_items: Vec<SourceItem>,
    /// Item specs that could not be resolved.
    pub unresolved_items: Vec<String>,
    pub project_references: Vec<ProjectReference>,
    pub package_references: Vec<PackageReference>,
    pub assembly_references: Vec<AssemblyReference>,
    pub framework_references: Vec<String>,
    /// Global usings in effect (implicit SDK usings and `<Using>` items, removals applied).
    pub usings: Vec<UsingDirective>,
    /// `ProjectConfiguration` items (`Debug|x64`).
    pub project_configurations: Vec<String>,
    pub cpp: Option<CppSettings>,
    pub imports: Vec<ImportRecord>,
    pub diagnostics: Vec<BuildDiagnostic>,
}

impl MsBuildProject {
    /// Loads and evaluates `path`.
    pub fn load(path: &Path, ctx: &EvaluationContext) -> Result<Self, MsBuildError> {
        evaluate_project(path, ctx)
    }

    /// Value of an evaluated property (case-insensitive name).
    pub fn property(&self, name: &str) -> Option<&str> {
        self.properties
            .get(&name.to_ascii_lowercase())
            .map(|p| p.value.as_str())
    }

    pub fn is_sdk_style(&self) -> bool {
        matches!(self.format, ProjectFormat::SdkStyle { .. })
    }

    pub fn sdk(&self) -> Option<&str> {
        match &self.format {
            ProjectFormat::SdkStyle { sdk } => Some(sdk),
            ProjectFormat::Legacy => None,
        }
    }

    /// `AssemblyName`, defaulting to the project name.
    pub fn effective_assembly_name(&self) -> String {
        self.assembly_name.clone().unwrap_or_else(|| self.name.clone())
    }

    /// `RootNamespace`, defaulting (SDK rule) to the project name with spaces as `_`.
    pub fn effective_root_namespace(&self) -> String {
        self.root_namespace
            .clone()
            .unwrap_or_else(|| self.name.replace(' ', "_"))
    }

    /// `OutputType`, defaulting to `Library`.
    pub fn effective_output_type(&self) -> OutputType {
        self.output_type.clone().unwrap_or(OutputType::Library)
    }

    pub fn effective_implicit_usings(&self) -> bool {
        self.implicit_usings.unwrap_or(false)
    }

    /// The C# language version: `LangVersion` when set, else the compiler default for the
    /// first target framework. `None` when neither decides it.
    pub fn effective_lang_version(&self) -> Option<LanguageStandard> {
        if let Some(v) = &self.lang_version {
            if v.version.is_some() {
                return Some(v.clone());
            }
        }
        let tfm = self.target_frameworks.first()?;
        let version = default_csharp_version(tfm)?;
        let spelling = format!("{}.{}", version / 10, version % 10);
        LanguageStandard::from_csharp_lang_version(&spelling)
    }

    /// Conditional-compilation symbols for `target_framework` (the first target framework
    /// when `None`): `DefineConstants`, plus, for SDK-style projects, the configuration
    /// symbol (`DEBUG`, `RELEASE`), `TRACE` and the framework symbols (`NET8_0`,
    /// `NET5_0_OR_GREATER`, `NETFRAMEWORK`, ...).
    pub fn preprocessor_symbols(&self, target_framework: Option<&str>) -> Vec<String> {
        let mut symbols: Vec<String> = Vec::new();
        let mut push = |s: String| {
            if !s.is_empty() && !symbols.contains(&s) {
                symbols.push(s);
            }
        };
        for s in &self.define_constants {
            push(s.clone());
        }
        if self.is_sdk_style() && self.language == ProjectLanguage::CSharp {
            push("TRACE".to_string());
            let disabled = self
                .property("DisableImplicitConfigurationDefines")
                .is_some_and(|v| v.eq_ignore_ascii_case("true"));
            if !disabled {
                push(
                    self.configuration
                        .to_uppercase()
                        .replace(['-', '.', ' '], "_"),
                );
            }
            let disabled_framework = self
                .property("DisableImplicitFrameworkDefines")
                .is_some_and(|v| v.eq_ignore_ascii_case("true"));
            if !disabled_framework {
                let tfm = target_framework.or_else(|| self.target_frameworks.first().map(String::as_str));
                if let Some(tfm) = tfm {
                    for s in framework_symbols(tfm) {
                        push(s);
                    }
                }
            }
        }
        symbols
    }

    /// File name of the primary build output: `{AssemblyName}.dll` / `.exe` for managed
    /// projects, `{TargetName}{TargetExt}` for Visual C++ projects. `None` when the project
    /// produces no binary (utility / makefile projects) or the name is symbolic.
    pub fn artifact_file_name(&self) -> Option<String> {
        if let Some(cpp) = &self.cpp {
            let name = self
                .property("TargetName")
                .filter(|n| !n.is_empty() && !n.contains("$("))
                .map(str::to_string)
                .unwrap_or_else(|| self.name.clone());
            if let Some(ext) = self.property("TargetExt").filter(|e| !e.is_empty() && !e.contains("$(")) {
                return Some(format!("{}{}", name, ext));
            }
            let ext = match cpp.configuration_type.as_ref().unwrap_or(&VcxConfigurationType::Application) {
                VcxConfigurationType::Application => ".exe",
                VcxConfigurationType::DynamicLibrary => ".dll",
                VcxConfigurationType::StaticLibrary => ".lib",
                _ => return None,
            };
            return Some(format!("{}{}", name, ext));
        }
        let name = self.effective_assembly_name();
        if name.contains("$(") {
            return None;
        }
        let netfx = self.target_frameworks.first().is_some_and(|t| is_netfx_tfm(t)) || !self.is_sdk_style();
        let ext = match self.effective_output_type() {
            OutputType::Exe | OutputType::WinExe | OutputType::AppContainerExe => {
                if netfx {
                    ".exe"
                } else {
                    ".dll"
                }
            }
            OutputType::Module => ".netmodule",
            OutputType::WinMdObj => ".winmdobj",
            OutputType::Library | OutputType::Other(_) => ".dll",
        };
        Some(format!("{}{}", name, ext))
    }
}

// ── Target frameworks ────────────────────────────────────────────────────────

/// .NET Framework versions in release order, as written in TFMs (`net472`).
const NETFX_VERSIONS: &[&str] = &["20", "35", "40", "45", "451", "452", "46", "461", "462", "47", "471", "472", "48", "481"];
/// .NET Core versions before .NET 5.
const NETCORE_VERSIONS: &[(u32, u32)] = &[(1, 0), (1, 1), (2, 0), (2, 1), (2, 2), (3, 0), (3, 1)];
/// .NET Standard versions.
const NETSTANDARD_VERSIONS: &[(u32, u32)] = &[(1, 0), (1, 1), (1, 2), (1, 3), (1, 4), (1, 5), (1, 6), (2, 0), (2, 1)];

/// A parsed target framework moniker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetFramework {
    /// `net5.0` and later (`net8.0-windows` → platform `windows`).
    Net { major: u32, minor: u32, platform: Option<String> },
    NetCoreApp { major: u32, minor: u32 },
    NetStandard { major: u32, minor: u32 },
    /// `net472`: the digits as written.
    NetFramework { digits: String },
}

impl TargetFramework {
    pub fn parse(tfm: &str) -> Option<Self> {
        let lower = tfm.trim().to_ascii_lowercase();
        let (base, platform) = match lower.split_once('-') {
            Some((b, p)) => (b.to_string(), Some(p.to_string())),
            None => (lower.clone(), None),
        };
        let dotted = |s: &str| -> Option<(u32, u32)> {
            let (a, b) = s.split_once('.')?;
            Some((a.parse().ok()?, b.parse().ok()?))
        };
        if let Some(v) = base.strip_prefix("netcoreapp") {
            let (major, minor) = dotted(v)?;
            return Some(TargetFramework::NetCoreApp { major, minor });
        }
        if let Some(v) = base.strip_prefix("netstandard") {
            let (major, minor) = dotted(v)?;
            return Some(TargetFramework::NetStandard { major, minor });
        }
        let v = base.strip_prefix("net")?;
        if v.contains('.') {
            let (major, minor) = dotted(v)?;
            if major >= 5 {
                let platform = platform.map(|p| p.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.').to_string());
                return Some(TargetFramework::Net { major, minor, platform });
            }
            return None;
        }
        if NETFX_VERSIONS.contains(&v) {
            return Some(TargetFramework::NetFramework { digits: v.to_string() });
        }
        None
    }

    /// `[MSBuild]::GetTargetFrameworkIdentifier` value.
    pub fn identifier(&self) -> &'static str {
        match self {
            TargetFramework::Net { .. } | TargetFramework::NetCoreApp { .. } => ".NETCoreApp",
            TargetFramework::NetStandard { .. } => ".NETStandard",
            TargetFramework::NetFramework { .. } => ".NETFramework",
        }
    }

    /// `[MSBuild]::GetTargetFrameworkVersion` value (`8.0`, `4.7.2`).
    pub fn version_text(&self) -> String {
        match self {
            TargetFramework::Net { major, minor, .. }
            | TargetFramework::NetCoreApp { major, minor }
            | TargetFramework::NetStandard { major, minor } => format!("{}.{}", major, minor),
            TargetFramework::NetFramework { digits } => digits
                .chars()
                .map(|c| c.to_string())
                .collect::<Vec<_>>()
                .join("."),
        }
    }
}

fn is_netfx_tfm(tfm: &str) -> bool {
    matches!(TargetFramework::parse(tfm), Some(TargetFramework::NetFramework { .. }))
}

/// Implicit framework symbols the .NET SDK defines for `tfm`.
pub fn framework_symbols(tfm: &str) -> Vec<String> {
    let Some(framework) = TargetFramework::parse(tfm) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    match framework {
        TargetFramework::Net { major, minor, platform } => {
            out.push("NET".to_string());
            out.push(format!("NET{}_{}", major, minor));
            out.push("NETCOREAPP".to_string());
            for m in 5..=major {
                out.push(format!("NET{}_0_OR_GREATER", m));
            }
            for (a, b) in NETCORE_VERSIONS {
                out.push(format!("NETCOREAPP{}_{}_OR_GREATER", a, b));
            }
            if let Some(p) = platform.filter(|p| !p.is_empty()) {
                out.push(p.to_ascii_uppercase());
            }
        }
        TargetFramework::NetCoreApp { major, minor } => {
            out.push("NETCOREAPP".to_string());
            out.push(format!("NETCOREAPP{}_{}", major, minor));
            for (a, b) in NETCORE_VERSIONS {
                if (*a, *b) <= (major, minor) {
                    out.push(format!("NETCOREAPP{}_{}_OR_GREATER", a, b));
                }
            }
        }
        TargetFramework::NetStandard { major, minor } => {
            out.push("NETSTANDARD".to_string());
            out.push(format!("NETSTANDARD{}_{}", major, minor));
            for (a, b) in NETSTANDARD_VERSIONS {
                if (*a, *b) <= (major, minor) {
                    out.push(format!("NETSTANDARD{}_{}_OR_GREATER", a, b));
                }
            }
        }
        TargetFramework::NetFramework { digits } => {
            out.push("NETFRAMEWORK".to_string());
            out.push(format!("NET{}", digits));
            if let Some(pos) = NETFX_VERSIONS.iter().position(|v| *v == digits) {
                for v in &NETFX_VERSIONS[..=pos] {
                    out.push(format!("NET{}_OR_GREATER", v));
                }
            }
        }
    }
    out
}

/// Default C# language version (`major * 10 + minor`) the compiler uses for `tfm`.
fn default_csharp_version(tfm: &str) -> Option<u16> {
    match TargetFramework::parse(tfm)? {
        TargetFramework::Net { major, .. } => match major {
            5 => Some(90),
            6 => Some(100),
            7 => Some(110),
            8 => Some(120),
            9 => Some(130),
            10 => Some(140),
            _ => None,
        },
        TargetFramework::NetCoreApp { major: 3, .. } => Some(80),
        TargetFramework::NetCoreApp { .. } => Some(73),
        TargetFramework::NetStandard { major: 2, minor: 1 } => Some(80),
        TargetFramework::NetStandard { .. } => Some(73),
        TargetFramework::NetFramework { .. } => Some(73),
    }
}

/// Global usings the SDK adds when `ImplicitUsings` is enabled.
fn implicit_usings_for(sdk: &str, use_windows_forms: bool) -> Vec<&'static str> {
    let mut usings = vec![
        "System",
        "System.Collections.Generic",
        "System.IO",
        "System.Linq",
        "System.Net.Http",
        "System.Threading",
        "System.Threading.Tasks",
    ];
    let lower = sdk.to_ascii_lowercase();
    if lower.contains("microsoft.net.sdk.web") {
        usings.extend([
            "System.Net.Http.Json",
            "Microsoft.AspNetCore.Builder",
            "Microsoft.AspNetCore.Hosting",
            "Microsoft.AspNetCore.Http",
            "Microsoft.AspNetCore.Routing",
            "Microsoft.Extensions.Configuration",
            "Microsoft.Extensions.DependencyInjection",
            "Microsoft.Extensions.Hosting",
            "Microsoft.Extensions.Logging",
        ]);
    } else if lower.contains("microsoft.net.sdk.worker") {
        usings.extend([
            "Microsoft.Extensions.Configuration",
            "Microsoft.Extensions.DependencyInjection",
            "Microsoft.Extensions.Hosting",
            "Microsoft.Extensions.Logging",
        ]);
    }
    if use_windows_forms {
        usings.extend(["System.Drawing", "System.Windows.Forms"]);
    }
    usings
}

// ── Expansion and conditions ─────────────────────────────────────────────────

/// Result of expanding property references in a string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expanded {
    pub text: String,
    pub symbolic: bool,
    pub conditional: bool,
}

impl Expanded {
    fn known(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            symbolic: false,
            conditional: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tri {
    True,
    False,
    Unknown,
}

impl Tri {
    fn from_bool(b: bool) -> Self {
        if b {
            Tri::True
        } else {
            Tri::False
        }
    }

    fn and(self, other: Tri) -> Tri {
        match (self, other) {
            (Tri::False, _) | (_, Tri::False) => Tri::False,
            (Tri::True, Tri::True) => Tri::True,
            _ => Tri::Unknown,
        }
    }

    fn or(self, other: Tri) -> Tri {
        match (self, other) {
            (Tri::True, _) | (_, Tri::True) => Tri::True,
            (Tri::False, Tri::False) => Tri::False,
            _ => Tri::Unknown,
        }
    }

    fn not(self) -> Tri {
        match self {
            Tri::True => Tri::False,
            Tri::False => Tri::True,
            Tri::Unknown => Tri::Unknown,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CondTok {
    Str(String),
    Word(String),
    LParen,
    RParen,
    Comma,
    Not,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    And,
    Or,
}

/// End index (exclusive) of a parenthesized group whose `(` is at `open`; single-quoted
/// strings inside are skipped.
fn matching_paren(chars: &[char], open: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut in_quote = false;
    let mut i = open;
    while i < chars.len() {
        let c = chars[i];
        if in_quote {
            if c == '\'' {
                in_quote = false;
            }
        } else if c == '\'' {
            in_quote = true;
        } else if c == '(' {
            depth += 1;
        } else if c == ')' {
            depth -= 1;
            if depth == 0 {
                return Some(i + 1);
            }
        }
        i += 1;
    }
    None
}

fn lex_condition(text: &str) -> Result<Vec<CondTok>, String> {
    let chars: Vec<char> = text.chars().collect();
    let mut toks = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            c if c.is_whitespace() => i += 1,
            '\'' => {
                let start = i + 1;
                let end = chars[start..]
                    .iter()
                    .position(|&q| q == '\'')
                    .map(|p| start + p)
                    .ok_or("unterminated quoted string")?;
                toks.push(CondTok::Str(chars[start..end].iter().collect()));
                i = end + 1;
            }
            '(' => {
                toks.push(CondTok::LParen);
                i += 1;
            }
            ')' => {
                toks.push(CondTok::RParen);
                i += 1;
            }
            ',' => {
                toks.push(CondTok::Comma);
                i += 1;
            }
            '!' if chars.get(i + 1) == Some(&'=') => {
                toks.push(CondTok::Ne);
                i += 2;
            }
            '!' => {
                toks.push(CondTok::Not);
                i += 1;
            }
            '=' if chars.get(i + 1) == Some(&'=') => {
                toks.push(CondTok::Eq);
                i += 2;
            }
            '=' => return Err("single '=' in condition".to_string()),
            '<' if chars.get(i + 1) == Some(&'=') => {
                toks.push(CondTok::Le);
                i += 2;
            }
            '<' => {
                toks.push(CondTok::Lt);
                i += 1;
            }
            '>' if chars.get(i + 1) == Some(&'=') => {
                toks.push(CondTok::Ge);
                i += 2;
            }
            '>' => {
                toks.push(CondTok::Gt);
                i += 1;
            }
            _ => {
                let start = i;
                while i < chars.len() {
                    let w = chars[i];
                    if (w == '$' || w == '@' || w == '%') && chars.get(i + 1) == Some(&'(') {
                        i = matching_paren(&chars, i + 1).ok_or("unterminated property reference")?;
                        continue;
                    }
                    if w.is_whitespace() || "()!=<>,'".contains(w) {
                        break;
                    }
                    i += 1;
                }
                let word: String = chars[start..i].iter().collect();
                if word.eq_ignore_ascii_case("and") {
                    toks.push(CondTok::And);
                } else if word.eq_ignore_ascii_case("or") {
                    toks.push(CondTok::Or);
                } else {
                    toks.push(CondTok::Word(word));
                }
            }
        }
    }
    Ok(toks)
}

enum CondVal {
    Text(Expanded),
    Bool(Tri),
}

fn as_tri(value: &CondVal) -> Tri {
    match value {
        CondVal::Bool(t) => *t,
        CondVal::Text(e) if e.symbolic => Tri::Unknown,
        CondVal::Text(e) => match e.text.trim().to_ascii_lowercase().as_str() {
            "true" | "on" | "yes" | "!false" | "!off" | "!no" => Tri::True,
            "false" | "off" | "no" | "!true" | "!on" | "!yes" => Tri::False,
            _ => Tri::Unknown,
        },
    }
}

/// Parses `1.2.3` / `42` / `0x1F` for ordered comparisons.
fn comparable(text: &str) -> Option<Vec<u64>> {
    let t = text.trim();
    if let Some(hex) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        return u64::from_str_radix(hex, 16).ok().map(|v| vec![v]);
    }
    let t = t.strip_prefix('v').unwrap_or(t);
    let parts: Option<Vec<u64>> = t.split('.').map(|p| p.parse().ok()).collect();
    parts.filter(|p| !p.is_empty())
}

fn compare_versions(a: &[u64], b: &[u64]) -> std::cmp::Ordering {
    let len = a.len().max(b.len());
    for i in 0..len {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        match x.cmp(&y) {
            std::cmp::Ordering::Equal => continue,
            other => return other,
        }
    }
    std::cmp::Ordering::Equal
}

// ── Evaluator ────────────────────────────────────────────────────────────────

/// Properties MSBuild reserves; projects cannot assign them.
const RESERVED_PROPERTIES: &[&str] = &[
    "MSBuildProjectDirectory",
    "MSBuildProjectDirectoryNoRoot",
    "MSBuildProjectFile",
    "MSBuildProjectExtension",
    "MSBuildProjectFullPath",
    "MSBuildProjectName",
    "MSBuildThisFile",
    "MSBuildThisFileDirectory",
    "MSBuildThisFileDirectoryNoRoot",
    "MSBuildThisFileExtension",
    "MSBuildThisFileFullPath",
    "MSBuildThisFileName",
];

/// Item attributes that are not metadata.
const ITEM_OPERATION_ATTRIBUTES: &[&str] = &[
    "Include",
    "Exclude",
    "Remove",
    "Update",
    "Condition",
    "KeepMetadata",
    "RemoveMetadata",
    "KeepDuplicates",
    "MatchOnMetadata",
    "MatchOnMetadataOptions",
    "Label",
];

/// Maximum import nesting.
const MAX_IMPORT_DEPTH: usize = 32;

struct DeferredGroup {
    element: XmlElement,
    conditional: bool,
    this_file: PathBuf,
}

/// A path matcher for `Exclude` / `Remove` specs.
enum SpecMatcher {
    Literal(String),
    Glob(GlobPattern),
}

impl SpecMatcher {
    fn matches(&self, path: &Path) -> bool {
        match self {
            SpecMatcher::Literal(key) => path_key(path) == *key,
            SpecMatcher::Glob(glob) => glob.is_match(Path::new(&normal_components(path))),
        }
    }
}

/// The normal components of a path joined with `/` (prefix and root dropped).
fn normal_components(path: &Path) -> String {
    path.components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            Component::ParentDir => Some("..".to_string()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// Undoes MSBuild `%XX` escaping in an item spec.
fn msbuild_unescape(text: &str) -> String {
    if !text.contains('%') {
        return text.to_string();
    }
    let bytes = text.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() && bytes[i + 1].is_ascii_hexdigit() && bytes[i + 2].is_ascii_hexdigit() {
            let digit = |b: u8| match b {
                b'0'..=b'9' => b - b'0',
                b'a'..=b'f' => b - b'a' + 10,
                _ => b - b'A' + 10,
            };
            out.push(digit(bytes[i + 1]) * 16 + digit(bytes[i + 2]));
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Splits an item spec list on `;`, trimming and dropping empty entries.
fn split_specs(text: &str) -> Vec<String> {
    text.split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

fn is_property_name(text: &str) -> bool {
    !text.is_empty()
        && text
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && text.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Splits property-function arguments on top-level commas.
fn split_function_args(text: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut depth = 0i32;
    let mut in_quote = false;
    for c in text.chars() {
        match c {
            '\'' => {
                in_quote = !in_quote;
                current.push(c);
            }
            '(' if !in_quote => {
                depth += 1;
                current.push(c);
            }
            ')' if !in_quote => {
                depth -= 1;
                current.push(c);
            }
            ',' if !in_quote && depth == 0 => args.push(std::mem::take(&mut current)),
            _ => current.push(c),
        }
    }
    if !current.trim().is_empty() || !args.is_empty() {
        args.push(current);
    }
    args.into_iter().map(|a| a.trim().to_string()).collect()
}

#[derive(Default)]
struct ItemState {
    compile: Vec<SourceItem>,
    headers: Vec<SourceItem>,
    unresolved: Vec<String>,
    project_references: Vec<ProjectReference>,
    package_references: Vec<PackageReference>,
    package_versions: HashMap<String, String>,
    references: Vec<AssemblyReference>,
    framework_references: Vec<String>,
    usings: Vec<UsingDirective>,
    removed_usings: Vec<String>,
    project_configurations: Vec<String>,
    /// Item definitions: item type (lower case) → metadata name (lower case) → value.
    definitions: HashMap<String, HashMap<String, Expanded>>,
}

struct Evaluator {
    project_path: PathBuf,
    project_dir: PathBuf,
    language: ProjectLanguage,
    sdk: Option<String>,
    props: HashMap<String, Property>,
    global: HashSet<String>,
    deferred: Vec<DeferredGroup>,
    imports: Vec<ImportRecord>,
    imported: HashSet<String>,
    chain: Vec<String>,
    diagnostics: Vec<BuildDiagnostic>,
    directory_props_done: bool,
    directory_targets_done: bool,
    this_file: PathBuf,
}

impl Evaluator {
    fn new(path: &Path, ctx: &EvaluationContext, language: ProjectLanguage, sdk: Option<String>) -> Self {
        let project_path = normalize_path(path);
        let project_dir = parent_dir(&project_path);
        let mut ev = Self {
            project_dir,
            language,
            sdk,
            props: HashMap::new(),
            global: HashSet::new(),
            deferred: Vec::new(),
            imports: Vec::new(),
            imported: HashSet::new(),
            chain: vec![path_key(&project_path)],
            diagnostics: Vec::new(),
            directory_props_done: false,
            directory_targets_done: false,
            this_file: project_path.clone(),
            project_path,
        };
        ev.imported.insert(path_key(&ev.project_path));
        let dir_text = ev.project_dir.to_string_lossy().into_owned();
        let file_name = ev
            .project_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let stem = ev
            .project_path
            .file_stem()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let ext = ev
            .project_path
            .extension()
            .map(|n| format!(".{}", n.to_string_lossy()))
            .unwrap_or_default();
        let full = ev.project_path.to_string_lossy().into_owned();
        ev.set_reserved("MSBuildProjectDirectory", &dir_text);
        ev.set_reserved("MSBuildProjectDirectoryNoRoot", &normal_components(&ev.project_dir.clone()));
        ev.set_reserved("MSBuildProjectFile", &file_name);
        ev.set_reserved("MSBuildProjectExtension", &ext);
        ev.set_reserved("MSBuildProjectFullPath", &full);
        ev.set_reserved("MSBuildProjectName", &stem);
        let this = ev.this_file.clone();
        ev.set_this_file(&this);
        if let Some(dir) = &ctx.solution_dir {
            let mut text = dir.to_string_lossy().into_owned();
            if !text.ends_with(['/', '\\']) {
                text.push(std::path::MAIN_SEPARATOR);
            }
            ev.set_plain("SolutionDir", &text);
        }
        for (name, value) in &ctx.global_properties {
            ev.set_plain(name, value);
            ev.global.insert(name.to_ascii_lowercase());
        }
        ev
    }

    fn set_plain(&mut self, name: &str, value: &str) {
        self.props.insert(
            name.to_ascii_lowercase(),
            Property {
                name: name.to_string(),
                value: value.to_string(),
                symbolic: false,
                conditional: false,
                alternatives: Vec::new(),
                defined_in: self.this_file.clone(),
            },
        );
    }

    fn set_reserved(&mut self, name: &str, value: &str) {
        self.set_plain(name, value);
    }

    fn set_this_file(&mut self, file: &Path) {
        self.this_file = file.to_path_buf();
        let dir = parent_dir(file);
        let mut dir_text = dir.to_string_lossy().into_owned();
        if !dir_text.ends_with(['/', '\\']) {
            dir_text.push(std::path::MAIN_SEPARATOR);
        }
        let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let stem = file.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let ext = file
            .extension()
            .map(|n| format!(".{}", n.to_string_lossy()))
            .unwrap_or_default();
        self.set_reserved("MSBuildThisFile", &name);
        self.set_reserved("MSBuildThisFileName", &stem);
        self.set_reserved("MSBuildThisFileExtension", &ext);
        self.set_reserved("MSBuildThisFileFullPath", &file.to_string_lossy());
        self.set_reserved("MSBuildThisFileDirectory", &dir_text);
        self.set_reserved("MSBuildThisFileDirectoryNoRoot", &normal_components(&dir));
    }

    fn known(&self, name: &str) -> Option<&str> {
        self.props
            .get(&name.to_ascii_lowercase())
            .filter(|p| !p.symbolic)
            .map(|p| p.value.as_str())
    }

    fn is_false(&self, name: &str) -> bool {
        self.known(name).is_some_and(|v| v.eq_ignore_ascii_case("false"))
    }

    fn is_true(&self, name: &str) -> bool {
        self.known(name).is_some_and(|v| v.eq_ignore_ascii_case("true"))
    }

    // ── Expansion ──

    fn expand(&self, raw: &str) -> Expanded {
        let chars: Vec<char> = raw.chars().collect();
        let mut out = String::with_capacity(raw.len());
        let mut symbolic = false;
        let mut conditional = false;
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            if (c == '$' || c == '@' || c == '%') && chars.get(i + 1) == Some(&'(') {
                if let Some(end) = matching_paren(&chars, i + 1) {
                    if c == '$' {
                        let inner: String = chars[i + 2..end - 1].iter().collect();
                        match self.property_reference(&inner) {
                            Some(e) => {
                                out.push_str(&e.text);
                                symbolic |= e.symbolic;
                                conditional |= e.conditional;
                            }
                            None => {
                                out.push_str("$(");
                                out.push_str(&inner);
                                out.push(')');
                                symbolic = true;
                            }
                        }
                    } else {
                        out.extend(chars[i..end].iter());
                        symbolic = true;
                    }
                    i = end;
                    continue;
                }
            }
            out.push(c);
            i += 1;
        }
        Expanded {
            text: out,
            symbolic,
            conditional,
        }
    }

    /// Evaluates the inside of `$(...)`. `None` keeps the reference symbolic.
    fn property_reference(&self, inner: &str) -> Option<Expanded> {
        let inner = inner.trim();
        if is_property_name(inner) {
            let prop = self.props.get(&inner.to_ascii_lowercase())?;
            return Some(Expanded {
                text: prop.value.clone(),
                symbolic: prop.symbolic,
                conditional: prop.conditional,
            });
        }
        if inner.starts_with('[') {
            return self.static_function(inner);
        }
        // Instance-method chain on a property: `Name.Method(args).Method(args)`.
        let name_end = inner.find('.')?;
        let base = &inner[..name_end];
        if !is_property_name(base) {
            return None;
        }
        let prop = self.props.get(&base.to_ascii_lowercase())?;
        if prop.symbolic {
            return None;
        }
        let mut value = prop.value.clone();
        let chars: Vec<char> = inner.chars().collect();
        let mut i = name_end;
        while i < chars.len() {
            if chars[i] != '.' {
                return None;
            }
            let start = i + 1;
            let mut j = start;
            while j < chars.len() && (chars[j].is_ascii_alphanumeric() || chars[j] == '_') {
                j += 1;
            }
            let method: String = chars[start..j].iter().collect();
            let args: Vec<String> = if chars.get(j) == Some(&'(') {
                let end = matching_paren(&chars, j)?;
                let raw_args: String = chars[j + 1..end - 1].iter().collect();
                i = end;
                self.function_args(&raw_args)?
            } else {
                i = j;
                Vec::new()
            };
            value = string_method(&value, &method, &args)?;
        }
        Some(Expanded {
            text: value,
            symbolic: false,
            conditional: prop.conditional,
        })
    }

    /// Expands property-function arguments; `None` if any argument stays symbolic.
    fn function_args(&self, raw: &str) -> Option<Vec<String>> {
        let mut out = Vec::new();
        for arg in split_function_args(raw) {
            let unquoted = arg
                .strip_prefix('\'')
                .and_then(|a| a.strip_suffix('\''))
                .unwrap_or(&arg);
            let expanded = self.expand(unquoted);
            if expanded.symbolic {
                return None;
            }
            out.push(expanded.text);
        }
        Some(out)
    }

    fn static_function(&self, inner: &str) -> Option<Expanded> {
        let close = inner.find("]::")?;
        let class = inner[1..close].to_ascii_lowercase();
        let rest = &inner[close + 3..];
        let open = rest.find('(')?;
        let method = rest[..open].trim().to_ascii_lowercase();
        let chars: Vec<char> = rest.chars().collect();
        let open_char = rest[..open].chars().count();
        let end = matching_paren(&chars, open_char)?;
        if end != chars.len() {
            return None;
        }
        let raw_args: String = chars[open_char + 1..end - 1].iter().collect();
        let args = self.function_args(&raw_args)?;
        let arg = |k: usize| args.get(k).cloned().unwrap_or_default();
        let bool_text = |b: bool| if b { "True".to_string() } else { "False".to_string() };
        let text = match (class.as_str(), method.as_str()) {
            ("msbuild", "getpathoffileabove") => {
                let start = if args.len() > 1 {
                    resolve_path(&self.project_dir, &arg(1))
                } else {
                    parent_dir(&self.this_file)
                };
                find_file_above(&start, &arg(0))
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default()
            }
            ("msbuild", "getdirectorynameoffileabove") => {
                let start = resolve_path(&self.project_dir, &arg(0));
                find_file_above(&start, &arg(1))
                    .and_then(|p| p.parent().map(|d| d.to_string_lossy().into_owned()))
                    .unwrap_or_default()
            }
            ("msbuild", "ensuretrailingslash") => {
                let mut v = arg(0);
                if !v.is_empty() && !v.ends_with(['/', '\\']) {
                    v.push(std::path::MAIN_SEPARATOR);
                }
                v
            }
            ("msbuild", "normalizedirectory") | ("msbuild", "normalizepath") => {
                let mut path = self.project_dir.clone();
                for a in &args {
                    path = resolve_path(&path, a);
                }
                let mut v = normalize_path(&path).to_string_lossy().into_owned();
                if method == "normalizedirectory" && !v.ends_with(['/', '\\']) {
                    v.push(std::path::MAIN_SEPARATOR);
                }
                v
            }
            ("msbuild", "valueordefault") => {
                if arg(0).is_empty() {
                    arg(1)
                } else {
                    arg(0)
                }
            }
            ("msbuild", "gettargetframeworkidentifier") => TargetFramework::parse(&arg(0))?.identifier().to_string(),
            ("msbuild", "gettargetframeworkversion") => TargetFramework::parse(&arg(0))?.version_text(),
            ("msbuild", m) if m.starts_with("version") => {
                let a = comparable(&arg(0))?;
                let b = comparable(&arg(1))?;
                let ord = compare_versions(&a, &b);
                let result = match m {
                    "versionequals" => ord.is_eq(),
                    "versionnotequals" => ord.is_ne(),
                    "versiongreaterthan" => ord.is_gt(),
                    "versiongreaterthanorequals" => ord.is_ge(),
                    "versionlessthan" => ord.is_lt(),
                    "versionlessthanorequals" => ord.is_le(),
                    _ => return None,
                };
                bool_text(result)
            }
            ("system.io.path", "combine") => {
                let mut path = PathBuf::new();
                for a in &args {
                    path.push(to_native_separators(a));
                }
                path.to_string_lossy().into_owned()
            }
            ("system.io.path", "getfilename") => Path::new(&to_native_separators(&arg(0)))
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            ("system.io.path", "getfilenamewithoutextension") => Path::new(&to_native_separators(&arg(0)))
                .file_stem()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            ("system.io.path", "getdirectoryname") => Path::new(&to_native_separators(&arg(0)))
                .parent()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            ("system.string", "isnullorempty") => bool_text(arg(0).is_empty()),
            ("system.string", "isnullorwhitespace") => bool_text(arg(0).trim().is_empty()),
            ("system.string", "copy") => arg(0),
            _ => return None,
        };
        Some(Expanded::known(text))
    }

    // ── Conditions ──

    fn condition(&self, element: &XmlElement) -> Tri {
        match element.attr("Condition") {
            None => Tri::True,
            Some(text) if text.trim().is_empty() => Tri::True,
            Some(text) => self.evaluate_condition(text, element.line),
        }
    }

    fn evaluate_condition(&self, text: &str, _line: u32) -> Tri {
        let Ok(toks) = lex_condition(text) else {
            return Tri::Unknown;
        };
        let mut parser = CondParser { toks, pos: 0, ev: self };
        match parser.parse_or() {
            Ok(value) if parser.pos == parser.toks.len() => as_tri(&value),
            _ => Tri::Unknown,
        }
    }

    // ── Pass 1: properties and imports ──

    fn process_project(&mut self, project: &XmlElement, conditional: bool) {
        for child in project.elements() {
            if child.is("PropertyGroup") {
                self.property_group(child, conditional);
            } else if child.is("ItemGroup") || child.is("ItemDefinitionGroup") {
                self.deferred.push(DeferredGroup {
                    element: child.clone(),
                    conditional,
                    this_file: self.this_file.clone(),
                });
            } else if child.is("Import") {
                self.import(child, conditional);
            } else if child.is("ImportGroup") {
                let cond = self.condition(child);
                if cond == Tri::False {
                    continue;
                }
                for import in child.elements().filter(|e| e.is("Import")) {
                    self.import(import, conditional || cond == Tri::Unknown);
                }
            } else if child.is("Choose") {
                self.choose(child, conditional);
            }
        }
    }

    fn property_group(&mut self, group: &XmlElement, conditional: bool) {
        let cond = self.condition(group);
        if cond == Tri::False {
            return;
        }
        let conditional = conditional || cond == Tri::Unknown;
        for prop in group.elements() {
            let c = self.condition(prop);
            if c == Tri::False {
                continue;
            }
            let value = self.expand(&prop.text());
            self.assign(&prop.name, value, conditional || c == Tri::Unknown, prop.line);
        }
    }

    fn assign(&mut self, name: &str, value: Expanded, conditional: bool, line: u32) {
        let key = name.to_ascii_lowercase();
        if self.global.contains(&key) {
            return;
        }
        if RESERVED_PROPERTIES.iter().any(|r| r.eq_ignore_ascii_case(name)) {
            let file = self.this_file.clone();
            self.diagnostics.push(BuildDiagnostic::warning(&file, line, format!("reserved property '{}' cannot be assigned", name)));
            return;
        }
        let defined_in = self.this_file.clone();
        match (conditional, self.props.get_mut(&key)) {
            (true, Some(existing)) => {
                existing.conditional = true;
                if existing.value != value.text && !existing.alternatives.contains(&value.text) {
                    existing.alternatives.push(value.text);
                }
            }
            (_, existing) => {
                let first_name = existing.map(|e| e.name.clone()).unwrap_or_else(|| name.to_string());
                self.props.insert(
                    key,
                    Property {
                        name: first_name,
                        value: value.text,
                        symbolic: value.symbolic,
                        conditional: conditional || value.conditional,
                        alternatives: Vec::new(),
                        defined_in,
                    },
                );
            }
        }
    }

    fn choose(&mut self, choose: &XmlElement, conditional: bool) {
        let mut any_unknown = false;
        for branch in choose.elements() {
            if branch.is("When") {
                match self.condition(branch) {
                    Tri::False => continue,
                    Tri::True => {
                        self.process_project(branch, conditional || any_unknown);
                        return;
                    }
                    Tri::Unknown => {
                        any_unknown = true;
                        self.process_project(branch, true);
                    }
                }
            } else if branch.is("Otherwise") {
                self.process_project(branch, conditional || any_unknown);
                return;
            }
        }
    }

    fn record_import(&mut self, spec: &str, resolved: Option<PathBuf>, status: ImportStatus, conditional: bool) {
        self.imports.push(ImportRecord {
            spec: spec.to_string(),
            resolved,
            status,
            conditional,
            importing_file: self.this_file.clone(),
        });
    }

    fn import(&mut self, element: &XmlElement, conditional: bool) {
        let spec = element.attr("Project").unwrap_or("").to_string();
        let cond = self.condition(element);
        if cond == Tri::False {
            self.record_import(&spec, None, ImportStatus::ConditionFalse, conditional);
            return;
        }
        let conditional = conditional || cond == Tri::Unknown;
        let lower = spec.to_ascii_lowercase().replace('\\', "/");
        if element.attr("Sdk").is_some() {
            self.record_import(&spec, None, ImportStatus::Sdk, conditional);
            if lower.ends_with("sdk.props") {
                self.sdk_props_point();
            } else if lower.ends_with("sdk.targets") {
                self.sdk_targets_point();
            }
            return;
        }
        const PROPS_ANCHORS: &[&str] = &["microsoft.common.props", "microsoft.cpp.default.props"];
        const TARGETS_ANCHORS: &[&str] = &[
            "microsoft.common.targets",
            "microsoft.csharp.targets",
            "microsoft.visualbasic.targets",
            "microsoft.fsharp.targets",
            "microsoft.cpp.targets",
        ];
        if PROPS_ANCHORS.iter().any(|a| lower.ends_with(a)) {
            self.record_import(&spec, None, ImportStatus::Toolset, conditional);
            self.import_directory_props(conditional);
            return;
        }
        if TARGETS_ANCHORS.iter().any(|a| lower.ends_with(a)) {
            self.record_import(&spec, None, ImportStatus::Toolset, conditional);
            self.import_directory_targets(conditional);
            return;
        }
        let expanded = self.expand(&spec);
        if expanded.symbolic {
            let toolset = ["$(msbuildextensionspath", "$(msbuildtoolspath", "$(vctargetspath", "$(msbuildbinpath", "$(msbuildsdkspath", "$(msbuildframeworktoolspath"]
                .iter()
                .any(|p| lower.starts_with(p));
            let status = if toolset {
                ImportStatus::Toolset
            } else {
                ImportStatus::Symbolic
            };
            self.record_import(&spec, None, status, conditional);
            return;
        }
        if expanded.text.trim().is_empty() {
            self.record_import(&spec, None, ImportStatus::NotFound, conditional);
            return;
        }
        let base = parent_dir(&self.this_file);
        for part in split_specs(&expanded.text) {
            if has_wildcard(&part) {
                let files = expand_glob(&base, &part, GlobOptions::default(), &|_| false).files;
                for file in files {
                    self.import_file(&spec, &file, conditional);
                }
                continue;
            }
            let path = resolve_path(&base, &part);
            if path.is_file() {
                self.import_file(&spec, &path, conditional);
            } else {
                self.record_import(&spec, Some(path), ImportStatus::NotFound, conditional);
            }
        }
    }

    fn import_file(&mut self, spec: &str, path: &Path, conditional: bool) {
        let key = path_key(path);
        if self.chain.contains(&key) {
            self.record_import(spec, Some(path.to_path_buf()), ImportStatus::Cycle, conditional);
            return;
        }
        if self.imported.contains(&key) {
            self.record_import(spec, Some(path.to_path_buf()), ImportStatus::Duplicate, conditional);
            return;
        }
        if self.chain.len() > MAX_IMPORT_DEPTH {
            let file = self.this_file.clone();
            self.diagnostics.push(BuildDiagnostic::warning(&file, 0, format!("import of '{}' exceeds the nesting limit of {}", path.display(), MAX_IMPORT_DEPTH)));
            self.record_import(spec, Some(path.to_path_buf()), ImportStatus::Failed, conditional);
            return;
        }
        let root = match load_xml(path) {
            Ok(root) => root,
            Err(err) => {
                let file = self.this_file.clone();
                self.diagnostics.push(BuildDiagnostic::warning(&file, 0, format!("import failed: {}", err)));
                self.record_import(spec, Some(path.to_path_buf()), ImportStatus::Failed, conditional);
                return;
            }
        };
        if !root.is("Project") {
            let file = self.this_file.clone();
            self.diagnostics.push(BuildDiagnostic::warning(&file, 0, format!("imported file '{}' has root <{}>, expected <Project>", path.display(), root.name)));
            self.record_import(spec, Some(path.to_path_buf()), ImportStatus::Failed, conditional);
            return;
        }
        self.imported.insert(key.clone());
        self.chain.push(key);
        self.record_import(spec, Some(path.to_path_buf()), ImportStatus::Imported, conditional);
        let saved = self.this_file.clone();
        self.set_this_file(path);
        self.process_project(&root, conditional);
        self.set_this_file(&saved);
        self.chain.pop();
    }

    /// Imports the nearest `Directory.Build.props` and `Directory.Packages.props`.
    fn import_directory_props(&mut self, conditional: bool) {
        if self.directory_props_done {
            return;
        }
        self.directory_props_done = true;
        if !self.is_false("ImportDirectoryBuildProps") {
            if let Some(path) = find_file_above(&self.project_dir, "Directory.Build.props") {
                self.import_file("Directory.Build.props", &path, conditional);
            }
        }
        if !self.is_false("ImportDirectoryPackagesProps") {
            if let Some(path) = find_file_above(&self.project_dir, "Directory.Packages.props") {
                self.import_file("Directory.Packages.props", &path, conditional);
            }
        }
    }

    fn import_directory_targets(&mut self, conditional: bool) {
        if self.directory_targets_done {
            return;
        }
        self.directory_targets_done = true;
        if !self.is_false("ImportDirectoryBuildTargets") {
            if let Some(path) = find_file_above(&self.project_dir, "Directory.Build.targets") {
                self.import_file("Directory.Build.targets", &path, conditional);
            }
        }
    }

    /// The SDK `Sdk.props` import point: directory props, then the SDK property defaults
    /// that projects commonly build on.
    fn sdk_props_point(&mut self) {
        self.import_directory_props(false);
        let sep = std::path::MAIN_SEPARATOR;
        let defaults = [
            ("BaseIntermediateOutputPath", format!("obj{}", sep)),
            ("BaseOutputPath", format!("bin{}", sep)),
        ];
        for (name, value) in defaults {
            if self.known(name).is_none() {
                self.set_plain(name, &value);
            }
        }
        if self.known("DefaultItemExcludes").is_none() {
            let bin = self.known("BaseOutputPath").unwrap_or("bin/").to_string();
            let obj = self.known("BaseIntermediateOutputPath").unwrap_or("obj/").to_string();
            let value = format!(
                "{}/**;{}/**;**/*.user;**/*.*proj;**/*.sln;**/*.vssscc",
                bin.trim_end_matches(['/', '\\']),
                obj.trim_end_matches(['/', '\\'])
            );
            self.set_plain("DefaultItemExcludes", &value);
        }
    }

    fn sdk_targets_point(&mut self) {
        self.import_directory_targets(false);
    }

    // ── Pass 2 / 3: item definitions and items ──

    fn evaluate_items(&mut self) -> ItemState {
        let mut state = ItemState::default();
        let groups = std::mem::take(&mut self.deferred);

        // The SDK default `Compile` glob precedes every project item.
        if self.sdk.is_some() && !self.is_false("EnableDefaultItems") && !self.is_false("EnableDefaultCompileItems") {
            if let Some(ext) = self.language.default_compile_extension() {
                let mut excludes = split_specs(self.known("DefaultItemExcludes").unwrap_or(""));
                excludes.push("**/.*/**".to_string());
                if let Some(extra) = self.known("DefaultItemExcludesInProjectFolder") {
                    excludes.extend(split_specs(extra));
                }
                let pattern = format!("**/*.{}", ext);
                self.include_sources(&pattern, &excludes, false, true, SourceKind::Compile, &mut state);
            }
        }

        let saved = self.this_file.clone();
        for group in &groups {
            self.set_this_file(&group.this_file);
            let cond = self.condition(&group.element);
            if cond == Tri::False {
                continue;
            }
            let conditional = group.conditional || cond == Tri::Unknown;
            if group.element.is("ItemDefinitionGroup") {
                for definition in group.element.elements() {
                    if self.condition(definition) == Tri::False {
                        continue;
                    }
                    let type_key = definition.name.to_ascii_lowercase();
                    for meta in definition.elements() {
                        if self.condition(meta) == Tri::False {
                            continue;
                        }
                        let meta_key = meta.name.to_ascii_lowercase();
                        let previous = state
                            .definitions
                            .get(&type_key)
                            .and_then(|m| m.get(&meta_key))
                            .cloned();
                        let raw = meta.text();
                        let self_ref = format!("%({})", meta.name);
                        let substituted = match &previous {
                            Some(prev) => replace_ignore_case(&raw, &self_ref, &prev.text),
                            None => remove_self_reference(&raw, &self_ref),
                        };
                        let mut value = self.expand(&substituted);
                        if let Some(prev) = previous {
                            value.symbolic |= prev.symbolic;
                        }
                        value.conditional |= conditional;
                        state.definitions.entry(type_key.clone()).or_default().insert(meta_key, value);
                    }
                }
                continue;
            }
            for item in group.element.elements() {
                self.item(item, conditional, &mut state);
            }
        }
        self.set_this_file(&saved);
        state
    }

    /// Metadata of an item: attributes other than the operation attributes, then child
    /// elements (whose conditions are honored).
    fn metadata(&self, item: &XmlElement) -> HashMap<String, Expanded> {
        let mut meta = HashMap::new();
        for (name, value) in &item.attributes {
            if ITEM_OPERATION_ATTRIBUTES.iter().any(|a| a.eq_ignore_ascii_case(name)) {
                continue;
            }
            meta.insert(name.to_ascii_lowercase(), self.expand(value));
        }
        for child in item.elements() {
            if self.condition(child) == Tri::False {
                continue;
            }
            meta.insert(child.name.to_ascii_lowercase(), self.expand(&child.text()));
        }
        meta
    }

    fn item(&mut self, item: &XmlElement, conditional: bool, state: &mut ItemState) {
        let cond = self.condition(item);
        if cond == Tri::False {
            return;
        }
        let conditional = conditional || cond == Tri::Unknown;
        let include = item.attr("Include").map(|s| self.expand(s));
        let exclude = item.attr("Exclude").map(|s| self.expand(s));
        let remove = item.attr("Remove").map(|s| self.expand(s));
        let update = item.attr("Update").map(|s| self.expand(s));
        let meta = self.metadata(item);
        let kind = item.name.to_ascii_lowercase();
        let source_kind = match kind.as_str() {
            "compile" | "clcompile" => Some(SourceKind::Compile),
            "clinclude" => Some(SourceKind::Header),
            _ => None,
        };
        if let Some(source_kind) = source_kind {
            if let Some(include) = &include {
                let excludes = exclude.as_ref().map(|e| split_specs(&e.text)).unwrap_or_default();
                if include.symbolic {
                    for spec in split_specs(&include.text) {
                        if spec.contains("$(") || spec.contains("@(") || spec.contains("%(") {
                            state.unresolved.push(spec);
                        } else {
                            self.include_sources(&spec, &excludes, conditional, false, source_kind, state);
                        }
                    }
                } else {
                    for spec in split_specs(&include.text) {
                        self.include_sources(&spec, &excludes, conditional, false, source_kind, state);
                    }
                }
            }
            if let Some(remove) = &remove {
                let matchers: Vec<SpecMatcher> = split_specs(&remove.text).iter().map(|s| self.matcher(s)).collect();
                let list = match source_kind {
                    SourceKind::Compile => &mut state.compile,
                    SourceKind::Header => &mut state.headers,
                };
                if conditional {
                    for entry in list.iter_mut().filter(|e| matchers.iter().any(|m| m.matches(&e.path))) {
                        entry.conditional = true;
                    }
                } else {
                    list.retain(|e| !matchers.iter().any(|m| m.matches(&e.path)));
                }
            }
            return;
        }
        let meta_text = |name: &str| meta.get(name).filter(|v| !v.symbolic).map(|v| v.text.clone());
        let meta_true = |name: &str| meta_text(name).is_some_and(|v| v.eq_ignore_ascii_case("true"));
        match kind.as_str() {
            "projectreference" => {
                if let Some(include) = &include {
                    for spec in split_specs(&include.text) {
                        let symbolic = spec.contains("$(");
                        let path = if symbolic {
                            None
                        } else {
                            Some(resolve_path(&self.project_dir, &msbuild_unescape(&spec)))
                        };
                        let reference_output_assembly = !meta_text("referenceoutputassembly").is_some_and(|v| v.eq_ignore_ascii_case("false"));
                        state.project_references.push(ProjectReference {
                            include: spec,
                            path,
                            reference_output_assembly,
                            conditional,
                        });
                    }
                }
                if let Some(remove) = &remove {
                    let matchers: Vec<SpecMatcher> = split_specs(&remove.text).iter().map(|s| self.matcher(s)).collect();
                    state
                        .project_references
                        .retain(|r| !r.path.as_ref().is_some_and(|p| matchers.iter().any(|m| m.matches(p))));
                }
            }
            "packagereference" => {
                let version = meta_text("version");
                let version_override = meta_text("versionoverride");
                let development_only = meta_text("privateassets").is_some_and(|v| v.eq_ignore_ascii_case("all"))
                    || meta_true("developmentdependency");
                if let Some(include) = &include {
                    for name in split_specs(&include.text) {
                        let (version, source) = match (&version_override, &version) {
                            (Some(v), _) => (Some(v.clone()), VersionSource::VersionOverride),
                            (None, Some(v)) => (Some(v.clone()), VersionSource::Explicit),
                            (None, None) => (None, VersionSource::Missing),
                        };
                        state.package_references.push(PackageReference {
                            name,
                            version,
                            version_source: source,
                            development_only,
                            conditional,
                        });
                    }
                }
                if let Some(update) = &update {
                    for name in split_specs(&update.text) {
                        for reference in state
                            .package_references
                            .iter_mut()
                            .filter(|r| r.name.eq_ignore_ascii_case(&name))
                        {
                            if let Some(v) = version_override.clone().or_else(|| version.clone()) {
                                reference.version = Some(v);
                                reference.version_source = if version_override.is_some() {
                                    VersionSource::VersionOverride
                                } else {
                                    VersionSource::Explicit
                                };
                            }
                            if development_only {
                                reference.development_only = true;
                            }
                        }
                    }
                }
                if let Some(remove) = &remove {
                    let names = split_specs(&remove.text);
                    state
                        .package_references
                        .retain(|r| !names.iter().any(|n| n.eq_ignore_ascii_case(&r.name)));
                }
            }
            "packageversion" => {
                if let (Some(include), Some(version)) = (&include, meta_text("version")) {
                    for name in split_specs(&include.text) {
                        state.package_versions.insert(name.to_ascii_lowercase(), version.clone());
                    }
                }
            }
            "reference" => {
                if let Some(include) = &include {
                    for spec in split_specs(&include.text) {
                        let mut parts = spec.split(',').map(str::trim);
                        let name = parts.next().unwrap_or("").to_string();
                        let version = parts
                            .find_map(|p| p.strip_prefix("Version=").or_else(|| p.strip_prefix("version=")))
                            .map(str::to_string);
                        let hint_path = meta_text("hintpath").map(|h| resolve_path(&self.project_dir, &h));
                        state.references.push(AssemblyReference {
                            framework: hint_path.is_none(),
                            name,
                            version,
                            hint_path,
                            conditional,
                        });
                    }
                }
            }
            "frameworkreference" => {
                if let Some(include) = &include {
                    for name in split_specs(&include.text) {
                        if !state.framework_references.contains(&name) {
                            state.framework_references.push(name);
                        }
                    }
                }
            }
            "using" => {
                if let Some(include) = &include {
                    for namespace in split_specs(&include.text) {
                        state.usings.push(UsingDirective {
                            namespace,
                            alias: meta_text("alias").filter(|a| !a.is_empty()),
                            is_static: meta_true("static"),
                            implicit: false,
                        });
                    }
                }
                if let Some(remove) = &remove {
                    state.removed_usings.extend(split_specs(&remove.text));
                }
            }
            "projectconfiguration" => {
                if let Some(include) = &include {
                    state.project_configurations.extend(split_specs(&include.text));
                }
            }
            _ => {}
        }
    }

    fn matcher(&self, spec: &str) -> SpecMatcher {
        let unescaped = msbuild_unescape(spec);
        let resolved = resolve_path(&self.project_dir, &unescaped);
        if has_wildcard(&unescaped) {
            SpecMatcher::Glob(GlobPattern::new(&normal_components(&resolved), GlobOptions::default()))
        } else {
            SpecMatcher::Literal(path_key(&resolved))
        }
    }

    fn include_sources(
        &mut self,
        spec: &str,
        excludes: &[String],
        conditional: bool,
        from_default_glob: bool,
        kind: SourceKind,
        state: &mut ItemState,
    ) {
        let spec = msbuild_unescape(spec);
        let matchers: Vec<SpecMatcher> = excludes
            .iter()
            .filter(|e| !e.contains("$(") && !e.contains("@("))
            .map(|e| self.matcher(e))
            .collect();
        let mut found: Vec<(PathBuf, bool)> = Vec::new();
        if has_wildcard(&spec) {
            let probe = "\u{1}probe";
            let prune = |dir: &Path| matchers.iter().any(|m| m.matches(&dir.join(probe)));
            let expansion = expand_glob(&self.project_dir, &spec, GlobOptions::default(), &prune);
            for dir in &expansion.unreadable {
                let file = self.this_file.clone();
                self.diagnostics.push(BuildDiagnostic::warning(&file, 0, format!("item glob '{}' could not read directory '{}'", spec, dir.display())));
            }
            for file in expansion.files {
                if !matchers.iter().any(|m| m.matches(&file)) {
                    found.push((file, true));
                }
            }
        } else {
            let path = resolve_path(&self.project_dir, &spec);
            if !matchers.iter().any(|m| m.matches(&path)) {
                let exists = path.is_file();
                if !exists {
                    let file = self.this_file.clone();
                    self.diagnostics.push(BuildDiagnostic::info(&file, 0, format!("item '{}' does not exist on disk", path.display())));
                }
                found.push((path, exists));
            }
        }
        let list = match kind {
            SourceKind::Compile => &mut state.compile,
            SourceKind::Header => &mut state.headers,
        };
        for (path, exists) in found {
            let key = path_key(&path);
            if let Some(existing) = list.iter_mut().find(|e| path_key(&e.path) == key) {
                existing.conditional &= conditional;
                continue;
            }
            list.push(SourceItem {
                path,
                exists,
                conditional,
                from_default_glob,
            });
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SourceKind {
    Compile,
    Header,
}

struct CondParser<'e> {
    toks: Vec<CondTok>,
    pos: usize,
    ev: &'e Evaluator,
}

impl<'e> CondParser<'e> {
    fn peek(&self) -> Option<&CondTok> {
        self.toks.get(self.pos)
    }

    fn eat(&mut self, tok: &CondTok) -> bool {
        if self.peek() == Some(tok) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn parse_or(&mut self) -> Result<CondVal, String> {
        let mut left = self.parse_and()?;
        while self.eat(&CondTok::Or) {
            let right = self.parse_and()?;
            left = CondVal::Bool(as_tri(&left).or(as_tri(&right)));
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<CondVal, String> {
        let mut left = self.parse_not()?;
        while self.eat(&CondTok::And) {
            let right = self.parse_not()?;
            left = CondVal::Bool(as_tri(&left).and(as_tri(&right)));
        }
        Ok(left)
    }

    fn parse_not(&mut self) -> Result<CondVal, String> {
        if self.eat(&CondTok::Not) {
            let inner = self.parse_not()?;
            return Ok(CondVal::Bool(as_tri(&inner).not()));
        }
        self.parse_comparison()
    }

    fn parse_comparison(&mut self) -> Result<CondVal, String> {
        let left = self.parse_primary()?;
        let op = match self.peek() {
            Some(CondTok::Eq) | Some(CondTok::Ne) | Some(CondTok::Lt) | Some(CondTok::Gt) | Some(CondTok::Le) | Some(CondTok::Ge) => {
                self.peek().cloned()
            }
            _ => None,
        };
        let Some(op) = op else {
            return Ok(left);
        };
        self.pos += 1;
        let right = self.parse_primary()?;
        Ok(CondVal::Bool(compare(&op, &left, &right)))
    }

    fn parse_primary(&mut self) -> Result<CondVal, String> {
        let tok = self.peek().cloned().ok_or("unexpected end of condition")?;
        self.pos += 1;
        match tok {
            CondTok::Str(s) => Ok(CondVal::Text(self.ev.expand(&s))),
            CondTok::Word(w) => {
                if self.peek() == Some(&CondTok::LParen) && is_property_name(&w) {
                    self.pos += 1;
                    let mut args = Vec::new();
                    if !self.eat(&CondTok::RParen) {
                        loop {
                            args.push(self.parse_primary()?);
                            if self.eat(&CondTok::Comma) {
                                continue;
                            }
                            if self.eat(&CondTok::RParen) {
                                break;
                            }
                            return Err("expected ',' or ')' in function call".to_string());
                        }
                    }
                    return Ok(CondVal::Bool(self.call(&w, &args)));
                }
                Ok(CondVal::Text(self.ev.expand(&w)))
            }
            CondTok::LParen => {
                let value = self.parse_or()?;
                if !self.eat(&CondTok::RParen) {
                    return Err("missing ')'".to_string());
                }
                Ok(value)
            }
            other => Err(format!("unexpected token {:?}", other)),
        }
    }

    fn call(&self, name: &str, args: &[CondVal]) -> Tri {
        let text = |k: usize| match args.get(k) {
            Some(CondVal::Text(e)) if !e.symbolic => Some(e.text.clone()),
            _ => None,
        };
        match name.to_ascii_lowercase().as_str() {
            "exists" => match text(0) {
                Some(t) if t.trim().is_empty() => Tri::False,
                Some(t) => {
                    let path = resolve_path(&self.ev.project_dir, &t);
                    Tri::from_bool(path.exists())
                }
                None => Tri::Unknown,
            },
            "hastrailingslash" => match text(0) {
                Some(t) => Tri::from_bool(t.ends_with(['/', '\\'])),
                None => Tri::Unknown,
            },
            _ => Tri::Unknown,
        }
    }
}

fn compare(op: &CondTok, left: &CondVal, right: &CondVal) -> Tri {
    let text_of = |v: &CondVal| -> Option<String> {
        match v {
            CondVal::Text(e) if e.symbolic => None,
            CondVal::Text(e) => Some(e.text.clone()),
            CondVal::Bool(Tri::True) => Some("true".to_string()),
            CondVal::Bool(Tri::False) => Some("false".to_string()),
            CondVal::Bool(Tri::Unknown) => None,
        }
    };
    let (Some(a), Some(b)) = (text_of(left), text_of(right)) else {
        return Tri::Unknown;
    };
    match op {
        CondTok::Eq => Tri::from_bool(a.trim().eq_ignore_ascii_case(b.trim())),
        CondTok::Ne => Tri::from_bool(!a.trim().eq_ignore_ascii_case(b.trim())),
        _ => {
            let (Some(x), Some(y)) = (comparable(&a), comparable(&b)) else {
                return Tri::Unknown;
            };
            let ord = compare_versions(&x, &y);
            Tri::from_bool(match op {
                CondTok::Lt => ord.is_lt(),
                CondTok::Gt => ord.is_gt(),
                CondTok::Le => ord.is_le(),
                CondTok::Ge => ord.is_ge(),
                _ => return Tri::Unknown,
            })
        }
    }
}

/// String instance methods supported in property functions.
fn string_method(value: &str, method: &str, args: &[String]) -> Option<String> {
    let bool_text = |b: bool| if b { "True".to_string() } else { "False".to_string() };
    let arg = |k: usize| args.get(k).map(String::as_str);
    Some(match method.to_ascii_lowercase().as_str() {
        "trim" if args.is_empty() => value.trim().to_string(),
        "trimend" => match arg(0) {
            None => value.trim_end().to_string(),
            Some(chars) => value.trim_end_matches(|c: char| chars.contains(c)).to_string(),
        },
        "trimstart" => match arg(0) {
            None => value.trim_start().to_string(),
            Some(chars) => value.trim_start_matches(|c: char| chars.contains(c)).to_string(),
        },
        "tolower" | "tolowerinvariant" => value.to_lowercase(),
        "toupper" | "toupperinvariant" => value.to_uppercase(),
        "replace" => value.replace(arg(0)?, arg(1).unwrap_or("")),
        "contains" => bool_text(value.contains(arg(0)?)),
        "startswith" => bool_text(value.starts_with(arg(0)?)),
        "endswith" => bool_text(value.ends_with(arg(0)?)),
        "length" if args.is_empty() => value.chars().count().to_string(),
        "substring" => {
            let start: usize = arg(0)?.trim().parse().ok()?;
            let chars: Vec<char> = value.chars().collect();
            if start > chars.len() {
                return None;
            }
            match arg(1) {
                Some(len) => {
                    let len: usize = len.trim().parse().ok()?;
                    if start + len > chars.len() {
                        return None;
                    }
                    chars[start..start + len].iter().collect()
                }
                None => chars[start..].iter().collect(),
            }
        }
        _ => return None,
    })
}

/// Case-insensitive replacement of every occurrence of `needle`.
fn replace_ignore_case(haystack: &str, needle: &str, replacement: &str) -> String {
    let lower_hay = haystack.to_ascii_lowercase();
    let lower_needle = needle.to_ascii_lowercase();
    let mut out = String::with_capacity(haystack.len());
    let mut last = 0;
    let mut search = 0;
    while let Some(pos) = lower_hay[search..].find(&lower_needle) {
        let start = search + pos;
        out.push_str(&haystack[last..start]);
        out.push_str(replacement);
        last = start + needle.len();
        search = last;
    }
    out.push_str(&haystack[last..]);
    out
}

/// Drops `%(Self)` and the separator next to it from a metadata value with no inherited
/// definition (`a;b;%(AdditionalIncludeDirectories)` → `a;b`).
fn remove_self_reference(value: &str, self_ref: &str) -> String {
    let replaced = replace_ignore_case(value, self_ref, "");
    split_specs(&replaced).join(";")
}

/// Searches `start` and its ancestors for `file_name`.
pub fn find_file_above(start: &Path, file_name: &str) -> Option<PathBuf> {
    if file_name.is_empty() {
        return None;
    }
    let mut dir = Some(normalize_path(start));
    while let Some(current) = dir {
        let candidate = current.join(file_name);
        if candidate.is_file() {
            return Some(candidate);
        }
        dir = current.parent().map(Path::to_path_buf);
    }
    None
}

/// Root-element facts read before evaluation: the SDK and the project configurations.
fn detect_sdk(root: &XmlElement) -> Option<String> {
    if let Some(sdk) = root.attr("Sdk") {
        return Some(sdk.split(';').next().unwrap_or(sdk).trim().to_string());
    }
    if let Some(sdk) = root.elements().find(|e| e.is("Sdk")).and_then(|e| e.attr("Name")) {
        return Some(sdk.to_string());
    }
    root.elements()
        .filter(|e| e.is("Import"))
        .find_map(|e| e.attr("Sdk"))
        .map(str::to_string)
}

fn first_project_configuration(root: &XmlElement, configuration: &str) -> Option<String> {
    let configs: Vec<&str> = root
        .elements()
        .filter(|e| e.is("ItemGroup"))
        .flat_map(|g| g.elements())
        .filter(|e| e.is("ProjectConfiguration"))
        .filter_map(|e| e.attr("Include"))
        .collect();
    configs
        .iter()
        .find(|c| c.split('|').next().is_some_and(|cfg| cfg.eq_ignore_ascii_case(configuration)))
        .or_else(|| configs.first())
        .and_then(|c| c.split('|').nth(1))
        .map(str::to_string)
}

/// Reads `packages.config` next to a legacy project.
fn read_packages_config(path: &Path, diagnostics: &mut Vec<BuildDiagnostic>) -> Vec<PackageReference> {
    let root = match load_xml(path) {
        Ok(root) => root,
        Err(err) => {
            diagnostics.push(BuildDiagnostic::warning(path, 0, err.to_string()));
            return Vec::new();
        }
    };
    root.elements()
        .filter(|e| e.is("package"))
        .filter_map(|e| {
            let name = e.attr("id")?.to_string();
            Some(PackageReference {
                name,
                version: e.attr("version").map(str::to_string),
                version_source: VersionSource::PackagesConfig,
                development_only: e.attr("developmentDependency").is_some_and(|v| v.eq_ignore_ascii_case("true")),
                conditional: false,
            })
        })
        .collect()
}

fn evaluate_project(path: &Path, ctx: &EvaluationContext) -> Result<MsBuildProject, MsBuildError> {
    let root = load_xml(path)?;
    if !root.is("Project") {
        return Err(MsBuildError::Format {
            path: path.to_path_buf(),
            message: format!("root element is <{}>, expected <Project>", root.name),
        });
    }
    let language = ProjectLanguage::from_path(path);
    let sdk = detect_sdk(&root);
    let configuration = ctx.configuration.clone().unwrap_or_else(|| "Debug".to_string());
    let platform = ctx.platform.clone().unwrap_or_else(|| {
        if language == ProjectLanguage::Cpp {
            first_project_configuration(&root, &configuration).unwrap_or_else(|| "Win32".to_string())
        } else {
            "AnyCPU".to_string()
        }
    });

    let mut ev = Evaluator::new(path, ctx, language, sdk.clone());
    if !ev.global.contains("configuration") {
        ev.set_plain("Configuration", &configuration);
    }
    if !ev.global.contains("platform") {
        ev.set_plain("Platform", &platform);
    }
    // Properties MSBuild treats as empty until a project appends to them.
    for name in ["DefineConstants", "NoWarn"] {
        if ev.known(name).is_none() {
            ev.set_plain(name, "");
        }
    }

    let implicit_sdk_imports = root.attr("Sdk").is_some() || root.elements().any(|e| e.is("Sdk"));
    if implicit_sdk_imports {
        ev.sdk_props_point();
    }
    ev.process_project(&root, false);
    if implicit_sdk_imports {
        ev.sdk_targets_point();
    }
    let mut state = ev.evaluate_items();

    let mut diagnostics = std::mem::take(&mut ev.diagnostics);
    let prop = |name: &str| {
        ev.props
            .get(&name.to_ascii_lowercase())
            .filter(|p| !p.symbolic && !p.value.trim().is_empty())
            .map(|p| p.value.trim().to_string())
    };

    let mut target_frameworks: Vec<String> = prop("TargetFrameworks")
        .map(|v| split_specs(&v))
        .unwrap_or_default();
    if target_frameworks.is_empty() {
        if let Some(tfm) = prop("TargetFramework") {
            target_frameworks.push(tfm);
        } else if let Some(version) = prop("TargetFrameworkVersion") {
            let digits: String = version.trim_start_matches(['v', 'V']).chars().filter(|c| *c != '.').collect();
            if !digits.is_empty() {
                target_frameworks.push(format!("net{}", digits));
            }
        }
    }

    // Central package management and packages.config versions.
    let central = prop("ManagePackageVersionsCentrally").is_some_and(|v| v.eq_ignore_ascii_case("true"));
    for reference in &mut state.package_references {
        if reference.version.is_none() {
            if let Some(v) = state.package_versions.get(&reference.name.to_ascii_lowercase()) {
                reference.version = Some(v.clone());
                reference.version_source = VersionSource::Central;
            } else if central {
                diagnostics.push(BuildDiagnostic::info(
                    path,
                    0,
                    format!("package '{}' has no PackageVersion in Directory.Packages.props", reference.name),
                ));
            }
        }
    }
    let project_dir = parent_dir(&normalize_path(path));
    let packages_config = project_dir.join("packages.config");
    if packages_config.is_file() {
        for package in read_packages_config(&packages_config, &mut diagnostics) {
            if !state.package_references.iter().any(|r| r.name.eq_ignore_ascii_case(&package.name)) {
                state.package_references.push(package);
            }
        }
    }

    // Global usings.
    let implicit_usings = prop("ImplicitUsings").map(|v| matches!(v.to_ascii_lowercase().as_str(), "enable" | "true"));
    let mut usings: Vec<UsingDirective> = Vec::new();
    if implicit_usings == Some(true) && language == ProjectLanguage::CSharp {
        let win_forms = prop("UseWindowsForms").is_some_and(|v| v.eq_ignore_ascii_case("true"));
        for ns in implicit_usings_for(sdk.as_deref().unwrap_or("Microsoft.NET.Sdk"), win_forms) {
            usings.push(UsingDirective {
                namespace: ns.to_string(),
                alias: None,
                is_static: false,
                implicit: true,
            });
        }
    }
    for using in std::mem::take(&mut state.usings) {
        if !usings.iter().any(|u| u.namespace == using.namespace && u.alias == using.alias && u.is_static == using.is_static) {
            usings.push(using);
        }
    }
    usings.retain(|u| !state.removed_usings.iter().any(|r| r == &u.namespace));

    let cpp = if language == ProjectLanguage::Cpp {
        Some(cpp_settings(&ev, &state, &project_dir))
    } else {
        None
    };

    let define_constants: Vec<String> = prop("DefineConstants")
        .map(|v| {
            v.split([';', ','])
                .map(str::trim)
                .filter(|s| !s.is_empty() && !s.contains("$("))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();

    let name = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let compile_items = std::mem::take(&mut state.compile);
    let header_items = std::mem::take(&mut state.headers);
    let assembly_name = prop("AssemblyName");
    let root_namespace = prop("RootNamespace");
    let output_type = prop("OutputType").map(|v| OutputType::parse(&v));
    let nullable = prop("Nullable").and_then(|v| NullableContext::parse(&v));
    let lang_version = prop("LangVersion").and_then(|v| LanguageStandard::from_csharp_lang_version(&v));
    let project_guid = prop("ProjectGuid").map(|g| g.trim_matches(|c| c == '{' || c == '}').to_ascii_uppercase());
    let properties: BTreeMap<String, Property> = ev.props.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    let imports = std::mem::take(&mut ev.imports);

    Ok(MsBuildProject {
        path: normalize_path(path),
        directory: project_dir,
        name,
        language,
        format: match sdk {
            Some(sdk) => ProjectFormat::SdkStyle { sdk },
            None => ProjectFormat::Legacy,
        },
        configuration,
        platform,
        properties,
        assembly_name,
        root_namespace,
        target_frameworks,
        output_type,
        implicit_usings,
        nullable,
        lang_version,
        define_constants,
        project_guid,
        compile_items,
        header_items,
        unresolved_items: std::mem::take(&mut state.unresolved),
        project_references: std::mem::take(&mut state.project_references),
        package_references: std::mem::take(&mut state.package_references),
        assembly_references: std::mem::take(&mut state.references),
        framework_references: std::mem::take(&mut state.framework_references),
        usings,
        project_configurations: std::mem::take(&mut state.project_configurations),
        cpp,
        imports,
        diagnostics,
    })
}

/// Visual C++ settings from the `ClCompile` / `Link` item definitions and the
/// configuration properties.
fn cpp_settings(ev: &Evaluator, state: &ItemState, project_dir: &Path) -> CppSettings {
    let mut settings = CppSettings::default();
    let prop = |name: &str| ev.known(name).map(str::trim).filter(|v| !v.is_empty()).map(str::to_string);
    let cl = state.definitions.get("clcompile");
    let link = state.definitions.get("link");
    let meta = |defs: Option<&HashMap<String, Expanded>>, name: &str| defs.and_then(|d| d.get(&name.to_ascii_lowercase())).cloned();

    settings.configuration_type = prop("ConfigurationType").map(|v| VcxConfigurationType::parse(&v));
    settings.platform_toolset = prop("PlatformToolset");
    settings.character_set = prop("CharacterSet");

    let push_dirs = |value: &Expanded, kind: IncludeKind, settings: &mut CppSettings| {
        for entry in split_specs(&value.text) {
            if entry.starts_with("%(") {
                continue;
            }
            if entry.contains("$(") {
                settings.unresolved.push(entry);
                continue;
            }
            let path = resolve_path(project_dir, &entry);
            if !settings.include_paths.iter().any(|p| p.path == path) {
                settings.include_paths.push(IncludePath::new(path, kind));
            }
        }
    };
    if let Some(dirs) = meta(cl, "AdditionalIncludeDirectories") {
        push_dirs(&dirs, IncludeKind::Angled, &mut settings);
    }
    if let Some(dirs) = meta(cl, "ExternalIncludeDirectories") {
        push_dirs(&dirs, IncludeKind::System, &mut settings);
    }
    for name in ["IncludePath", "ExternalIncludePath"] {
        if let Some(p) = ev.props.get(&name.to_ascii_lowercase()) {
            let value = Expanded {
                text: p.value.clone(),
                symbolic: p.symbolic,
                conditional: p.conditional,
            };
            push_dirs(&value, IncludeKind::System, &mut settings);
        }
    }

    if let Some(defs) = meta(cl, "PreprocessorDefinitions") {
        for entry in split_specs(&defs.text) {
            if entry.starts_with("%(") {
                continue;
            }
            if entry.contains("$(") {
                settings.unresolved.push(entry);
                continue;
            }
            match MacroDefinition::parse(&entry) {
                Some(def) => settings.definitions.push(def),
                None => settings.unresolved.push(entry),
            }
        }
    }
    if let Some(forced) = meta(cl, "ForcedIncludeFiles") {
        for entry in split_specs(&forced.text) {
            if entry.starts_with("%(") || entry.contains("$(") {
                continue;
            }
            settings.forced_includes.push(resolve_path(project_dir, &entry));
        }
    }
    settings.cpp_standard = meta(cl, "LanguageStandard")
        .filter(|v| !v.symbolic)
        .and_then(|v| LanguageStandard::from_vcxproj(&v.text))
        .or_else(|| Some(LanguageStandard::new(crate::lang::build::StandardFamily::Cpp, "stdcpp14", Some(2014), false)));
    settings.c_standard = meta(cl, "LanguageStandard_C")
        .filter(|v| !v.symbolic)
        .and_then(|v| LanguageStandard::from_vcxproj(&v.text));
    if let Some(deps) = meta(link, "AdditionalDependencies") {
        for entry in split_specs(&deps.text) {
            if !entry.starts_with("%(") && !entry.contains("$(") {
                settings.additional_dependencies.push(entry);
            }
        }
    }

    // Toolset-implied definitions.
    let mut implicit = Vec::new();
    match settings.character_set.as_deref().map(str::to_ascii_lowercase).as_deref() {
        Some("unicode") => {
            implicit.push(MacroDefinition::object("UNICODE", None));
            implicit.push(MacroDefinition::object("_UNICODE", None));
        }
        Some("multibyte") => implicit.push(MacroDefinition::object("_MBCS", None)),
        _ => {}
    }
    if settings.configuration_type == Some(VcxConfigurationType::DynamicLibrary) {
        implicit.push(MacroDefinition::object("_WINDLL", None));
    }
    if prop("UseOfMfc").is_some_and(|v| v.eq_ignore_ascii_case("Dynamic")) {
        implicit.push(MacroDefinition::object("_AFXDLL", None));
    }
    // The CRT selection (`/MD`, `/MDd`, `/MT`, `/MTd`) predefines `_MT`, `_DLL`, `_DEBUG`.
    let debug_libraries = prop("UseDebugLibraries").is_some_and(|v| v.eq_ignore_ascii_case("true"));
    let runtime = meta(cl, "RuntimeLibrary")
        .filter(|v| !v.symbolic)
        .map(|v| v.text.to_ascii_lowercase())
        .unwrap_or_else(|| {
            if debug_libraries {
                "multithreadeddebugdll".to_string()
            } else {
                "multithreadeddll".to_string()
            }
        });
    implicit.push(MacroDefinition::object("_MT", None));
    if runtime.ends_with("dll") {
        implicit.push(MacroDefinition::object("_DLL", None));
    }
    if runtime.contains("debug") {
        implicit.push(MacroDefinition::object("_DEBUG", None));
    }
    settings.implicit_definitions = implicit;
    settings
}

// ── Workspace ────────────────────────────────────────────────────────────────

/// Every solution and project of a source tree, evaluated.
#[derive(Debug, Clone, Default)]
pub struct MsBuildWorkspace {
    pub solutions: Vec<Solution>,
    pub projects: Vec<MsBuildProject>,
    pub diagnostics: Vec<BuildDiagnostic>,
}

impl MsBuildWorkspace {
    /// Loads `solutions` and their projects, then every project in `projects` not already
    /// loaded through a solution. Failures become diagnostics; nothing aborts the load.
    pub fn load(solutions: &[PathBuf], projects: &[PathBuf], ctx: &EvaluationContext) -> Self {
        let mut workspace = MsBuildWorkspace::default();
        let mut loaded: HashSet<String> = HashSet::new();
        for solution_path in solutions {
            let solution = match Solution::load(solution_path) {
                Ok(s) => s,
                Err(err) => {
                    workspace.diagnostics.push(BuildDiagnostic::error(solution_path, 0, err.to_string()));
                    continue;
                }
            };
            let mut solution_ctx = ctx.clone();
            if solution_ctx.solution_dir.is_none() {
                solution_ctx.solution_dir = Some(parent_dir(solution_path));
            }
            for entry in solution.project_entries() {
                let Some(path) = &entry.path else {
                    continue;
                };
                if matches!(entry.kind, SolutionItemKind::SharedProject) || !path.is_file() {
                    continue;
                }
                if loaded.insert(path_key(path)) {
                    workspace.load_one(path, &solution_ctx);
                }
            }
            workspace.solutions.push(solution);
        }
        for path in projects {
            if loaded.insert(path_key(path)) {
                workspace.load_one(path, ctx);
            }
        }
        workspace
    }

    fn load_one(&mut self, path: &Path, ctx: &EvaluationContext) {
        match MsBuildProject::load(path, ctx) {
            Ok(project) => self.projects.push(project),
            Err(err) => self.diagnostics.push(BuildDiagnostic::error(path, 0, err.to_string())),
        }
    }

    pub fn project_by_path(&self, path: &Path) -> Option<&MsBuildProject> {
        let key = path_key(path);
        self.projects.iter().find(|p| path_key(&p.path) == key)
    }

    /// The solution-folder path of a project, from the first solution listing it.
    pub fn folder_of(&self, project: &MsBuildProject) -> Vec<String> {
        let key = path_key(&project.path);
        self.solutions
            .iter()
            .flat_map(|s| s.projects.iter())
            .find(|p| p.path.as_ref().is_some_and(|pp| path_key(pp) == key))
            .map(|p| p.folder_path.clone())
            .unwrap_or_default()
    }
}
