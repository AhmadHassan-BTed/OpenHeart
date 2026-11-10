//! Compose file facts (Compose Specification): services with `image`, `build`
//! (`context`, `dockerfile`), `depends_on`, `ports`, `expose`, `networks`, `network_mode`,
//! `command` and `entrypoint`, and the top-level `networks`.

use crate::project::paths;
use crate::project::yaml::{self, YamlNode};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposeBuild {
    /// Build context, resolved against the Compose file's directory.
    pub context: String,
    /// Dockerfile path, resolved against the context (`Dockerfile` when not stated).
    pub dockerfile: String,
    /// True when the Compose file states the Dockerfile explicitly.
    pub dockerfile_stated: bool,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposeService {
    pub name: String,
    pub line: u32,
    pub image: Option<String>,
    pub build: Option<ComposeBuild>,
    pub depends_on: Vec<(String, u32)>,
    pub ports: Vec<String>,
    pub expose: Vec<String>,
    /// Networks the service joins as written; empty when not stated.
    pub networks: Vec<(String, u32)>,
    pub network_mode: Option<String>,
    pub command: Option<String>,
    pub entrypoint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposeFile {
    pub file: String,
    pub dir: String,
    pub services: Vec<ComposeService>,
    /// Top-level network declarations (name, line).
    pub networks: Vec<(String, u32)>,
}

/// True for the file names Compose loads by default and their override variants.
pub fn is_compose_file(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    let yaml = lower.ends_with(".yml") || lower.ends_with(".yaml");
    yaml && (lower.starts_with("docker-compose") || lower.starts_with("compose."))
        || lower == "compose.yml"
        || lower == "compose.yaml"
}

/// Text of a scalar or of a sequence of scalars (joined with blanks).
fn text_of(node: &YamlNode) -> Option<String> {
    if let Some(s) = node.as_str() {
        return Some(s.to_string());
    }
    let items: Vec<&str> = node.as_seq().iter().filter_map(|i| i.as_str()).collect();
    if items.is_empty() {
        None
    } else {
        Some(items.join(" "))
    }
}

fn port_text(node: &YamlNode) -> Option<String> {
    if let Some(s) = node.as_str() {
        return Some(s.to_string());
    }
    if node.is_map() {
        let target = node.get("target").and_then(|t| t.as_str());
        let published = node.get("published").and_then(|t| t.as_str());
        let protocol = node.get("protocol").and_then(|t| t.as_str());
        let mut text = match (published, target) {
            (Some(p), Some(t)) => format!("{}:{}", p, t),
            (None, Some(t)) => t.to_string(),
            _ => return None,
        };
        if let Some(proto) = protocol {
            text.push('/');
            text.push_str(proto);
        }
        return Some(text);
    }
    None
}

pub fn parse(file: &str, text: &str) -> Option<ComposeFile> {
    let docs = yaml::parse_documents(text);
    let root = docs.into_iter().next()?;
    let services_node = root.get("services")?;
    let dir = paths::parent(file);
    let mut compose = ComposeFile {
        file: paths::normalize(file),
        dir: dir.clone(),
        services: Vec::new(),
        networks: root
            .get("networks")
            .map(|n| n.as_map().iter().map(|(k, v)| (k.clone(), v.line)).collect())
            .unwrap_or_default(),
    };
    for (name, svc) in services_node.as_map() {
        let build = svc.get("build").and_then(|b| {
            let (context, dockerfile) = if let Some(ctx) = b.as_str() {
                (ctx.to_string(), None)
            } else {
                (
                    b.get("context")
                        .and_then(|c| c.as_str())
                        .unwrap_or(".")
                        .to_string(),
                    b.get("dockerfile").and_then(|d| d.as_str()).map(str::to_string),
                )
            };
            // A remote context (Git URL) has no file in the project.
            if context.contains("://") || context.starts_with("git@") {
                return None;
            }
            let context = paths::join(&dir, &context);
            let stated = dockerfile.is_some();
            let dockerfile = paths::join(&context, dockerfile.as_deref().unwrap_or("Dockerfile"));
            Some(ComposeBuild {
                context,
                dockerfile,
                dockerfile_stated: stated,
                line: b.line,
            })
        });
        compose.services.push(ComposeService {
            name: name.clone(),
            line: svc.line,
            image: svc.get("image").and_then(|i| i.as_str()).map(str::to_string),
            build,
            depends_on: svc.get("depends_on").map(|d| d.names()).unwrap_or_default(),
            ports: svc
                .get("ports")
                .map(|p| p.as_seq().iter().filter_map(port_text).collect())
                .unwrap_or_default(),
            expose: svc
                .get("expose")
                .map(|p| p.as_seq().iter().filter_map(|e| e.as_str().map(str::to_string)).collect())
                .unwrap_or_default(),
            networks: svc.get("networks").map(|n| n.names()).unwrap_or_default(),
            network_mode: svc
                .get("network_mode")
                .and_then(|n| n.as_str())
                .map(str::to_string),
            command: svc.get("command").and_then(text_of),
            entrypoint: svc.get("entrypoint").and_then(text_of),
        });
    }
    Some(compose)
}
