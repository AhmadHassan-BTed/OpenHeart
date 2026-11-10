//! Dockerfile facts: build stages (`FROM ... AS name`), `WORKDIR`, `COPY`/`ADD` (with
//! `--from`), `EXPOSE`, `ENTRYPOINT` and `CMD`, each with its line. Line continuations
//! (`\`) and comments are handled; other instructions are not interpreted.

use crate::project::paths;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DockerCopy {
    /// `COPY` or `ADD`.
    pub instruction: String,
    pub sources: Vec<String>,
    pub dest: String,
    /// Absolute destination inside the image (resolved against the stage's `WORKDIR`).
    pub dest_abs: String,
    /// `--from` stage name, stage index or image.
    pub from: Option<String>,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DockerStage {
    pub base: String,
    pub name: Option<String>,
    pub line: u32,
    pub copies: Vec<DockerCopy>,
    pub expose: Vec<(String, u32)>,
    pub entrypoint: Option<(String, u32)>,
    pub cmd: Option<(String, u32)>,
    pub workdir: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DockerImage {
    /// Project-relative path of the Dockerfile.
    pub file: String,
    /// Directory of the Dockerfile.
    pub dir: String,
    pub stages: Vec<DockerStage>,
}

impl DockerImage {
    /// The stage `docker build` produces without `--target`: the last one.
    pub fn final_stage(&self) -> Option<&DockerStage> {
        self.stages.last()
    }

    /// Stage named `reference` (by `AS` name or numeric index) that precedes stage `before`.
    pub fn stage(&self, reference: &str, before: usize) -> Option<(usize, &DockerStage)> {
        if let Ok(idx) = reference.parse::<usize>() {
            return self.stages.get(idx).filter(|_| idx < before).map(|s| (idx, s));
        }
        self.stages
            .iter()
            .enumerate()
            .take(before)
            .filter(|(_, s)| {
                s.name
                    .as_deref()
                    .map(|n| n.eq_ignore_ascii_case(reference))
                    .unwrap_or(false)
            })
            .last()
    }
}

/// True for the file names Docker reads as a Dockerfile.
pub fn is_dockerfile(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower == "dockerfile"
        || lower.starts_with("dockerfile.")
        || lower.ends_with(".dockerfile")
        || lower == "containerfile"
}

/// Splits the arguments of an instruction: JSON array form or whitespace-separated words.
fn arguments(text: &str) -> Vec<String> {
    let t = text.trim();
    if t.starts_with('[') {
        if let Ok(serde_json::Value::Array(items)) = serde_json::from_str::<serde_json::Value>(t) {
            return items
                .into_iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect();
        }
    }
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    for c in t.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => cur.push(c),
            None if c == '"' || c == '\'' => quote = Some(c),
            None if c.is_whitespace() => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            None => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Command text of `ENTRYPOINT`/`CMD` (JSON form joined with blanks).
fn command_text(text: &str) -> String {
    let t = text.trim();
    if t.starts_with('[') {
        let args = arguments(t);
        if !args.is_empty() {
            return args.join(" ");
        }
    }
    t.to_string()
}

pub fn parse(file: &str, text: &str) -> DockerImage {
    let mut image = DockerImage {
        file: paths::normalize(file),
        dir: paths::parent(file),
        stages: Vec::new(),
    };
    // Logical instructions: (first line, text) with continuations joined.
    let mut instructions: Vec<(u32, String)> = Vec::new();
    let mut pending: Option<(u32, String)> = None;
    for (idx, raw) in text.lines().enumerate() {
        let number = (idx + 1) as u32;
        let line = raw.trim_end_matches('\r');
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            continue;
        }
        let (content, continues) = match trimmed.strip_suffix('\\') {
            Some(c) => (c.trim_end(), true),
            None => (trimmed, false),
        };
        match pending.as_mut() {
            Some((_, acc)) => {
                if !content.is_empty() {
                    acc.push(' ');
                    acc.push_str(content);
                }
            }
            None => {
                if content.is_empty() && !continues {
                    continue;
                }
                pending = Some((number, content.to_string()));
            }
        }
        if !continues {
            if let Some(done) = pending.take() {
                instructions.push(done);
            }
        }
    }
    if let Some(done) = pending.take() {
        instructions.push(done);
    }

    for (line, instr) in instructions {
        let (keyword, rest) = match instr.find(char::is_whitespace) {
            Some(i) => (instr[..i].to_ascii_uppercase(), instr[i..].trim().to_string()),
            None => (instr.to_ascii_uppercase(), String::new()),
        };
        if keyword == "FROM" {
            let words: Vec<String> = arguments(&rest)
                .into_iter()
                .filter(|w| !w.starts_with("--"))
                .collect();
            let base = words.first().cloned().unwrap_or_default();
            let name = match (words.get(1), words.get(2)) {
                (Some(kw), Some(n)) if kw.eq_ignore_ascii_case("as") => Some(n.clone()),
                _ => None,
            };
            image.stages.push(DockerStage {
                base,
                name,
                line,
                copies: Vec::new(),
                expose: Vec::new(),
                entrypoint: None,
                cmd: None,
                workdir: "/".to_string(),
            });
            continue;
        }
        let stage = match image.stages.last_mut() {
            Some(s) => s,
            None => continue,
        };
        match keyword.as_str() {
            "WORKDIR" => {
                let dir = rest.trim().trim_matches('"').to_string();
                stage.workdir = if dir.starts_with('/') {
                    paths::normalize(&dir)
                } else {
                    paths::normalize(&format!("{}/{}", stage.workdir, dir))
                };
            }
            "COPY" | "ADD" => {
                let mut from = None;
                let mut words = Vec::new();
                for w in arguments(&rest) {
                    if let Some(v) = w.strip_prefix("--from=") {
                        from = Some(v.to_string());
                    } else if w.starts_with("--") {
                        continue;
                    } else {
                        words.push(w);
                    }
                }
                if words.len() < 2 {
                    continue;
                }
                let dest = words.pop().unwrap_or_default();
                let dest_abs = if dest.starts_with('/') {
                    let mut d = paths::normalize(&dest);
                    if dest.ends_with('/') {
                        d.push('/');
                    }
                    d
                } else {
                    let mut d = paths::normalize(&format!("{}/{}", stage.workdir, dest));
                    if dest.ends_with('/') || dest == "." {
                        d.push('/');
                    }
                    d
                };
                stage.copies.push(DockerCopy {
                    instruction: keyword.clone(),
                    sources: words,
                    dest,
                    dest_abs,
                    from,
                    line,
                });
            }
            "EXPOSE" => {
                for port in arguments(&rest) {
                    stage.expose.push((port, line));
                }
            }
            "ENTRYPOINT" => stage.entrypoint = Some((command_text(&rest), line)),
            "CMD" => stage.cmd = Some((command_text(&rest), line)),
            _ => {}
        }
    }
    image
}

/// Directory segments that build tools write their outputs to (Maven `target`, Gradle
/// `build`, npm bundlers `dist`/`build`, MSBuild `bin`/`publish`, CMake `out`).
const OUTPUT_DIRS: &[&str] = &["target", "build", "dist", "out", "bin", "publish", "release", "libs"];
/// File extensions of deployable build outputs.
const OUTPUT_EXTENSIONS: &[&str] = &["jar", "war", "ear", "dll", "exe", "so", "dylib", "a", "wasm", "nupkg"];

/// True when a `COPY` source is a build output: copied from another stage or image, or a
/// path inside a build output directory, or a file with a deployable extension.
pub fn is_build_output(copy: &DockerCopy, source: &str) -> bool {
    if copy.from.is_some() {
        return true;
    }
    let norm = paths::normalize(source);
    if norm.split('/').any(|seg| OUTPUT_DIRS.contains(&seg)) {
        return true;
    }
    match paths::file_name(&norm).rsplit_once('.') {
        Some((_, ext)) => OUTPUT_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()),
        None => false,
    }
}
