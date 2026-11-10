//! Kubernetes manifest facts: workloads (Deployment, StatefulSet, DaemonSet, ReplicaSet,
//! Job, CronJob, Pod) with their pod-template labels and containers, and Services with
//! their selectors and ports. Documents containing template directives (`{{ ... }}`, Helm)
//! are not evaluated and are reported as skipped.

use crate::project::paths;
use crate::project::yaml::{self, YamlNode};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Container {
    pub name: String,
    pub image: Option<String>,
    pub ports: Vec<String>,
    pub init: bool,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workload {
    pub file: String,
    pub line: u32,
    pub kind: String,
    pub name: String,
    pub namespace: Option<String>,
    pub replicas: Option<String>,
    /// Labels of the pods the workload creates (pod template labels; a Pod's own labels).
    pub pod_labels: Vec<(String, String)>,
    pub containers: Vec<Container>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Service {
    pub file: String,
    pub line: u32,
    pub name: String,
    pub namespace: Option<String>,
    pub service_type: Option<String>,
    pub selector: Vec<(String, String)>,
    pub ports: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Manifests {
    pub workloads: Vec<Workload>,
    pub services: Vec<Service>,
    /// Documents that were not read (templated).
    pub skipped: usize,
}

const WORKLOAD_KINDS: &[&str] = &[
    "Deployment",
    "StatefulSet",
    "DaemonSet",
    "ReplicaSet",
    "Job",
    "CronJob",
    "Pod",
];

fn string_map(node: Option<&YamlNode>) -> Vec<(String, String)> {
    node.map(|n| {
        n.as_map()
            .iter()
            .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
            .collect()
    })
    .unwrap_or_default()
}

fn containers(spec: &YamlNode, key: &str, init: bool, out: &mut Vec<Container>) {
    if let Some(list) = spec.get(key) {
        for c in list.as_seq() {
            let ports = c
                .get("ports")
                .map(|p| {
                    p.as_seq()
                        .iter()
                        .filter_map(|port| {
                            let number = port.get("containerPort").and_then(|v| v.as_str())?;
                            Some(match port.get("protocol").and_then(|v| v.as_str()) {
                                Some(proto) => format!("{}/{}", number, proto),
                                None => number.to_string(),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            out.push(Container {
                name: c
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("")
                    .to_string(),
                image: c.get("image").and_then(|i| i.as_str()).map(str::to_string),
                ports,
                init,
                line: c.line,
            });
        }
    }
}

/// True when a YAML text holds at least one document with `apiVersion` and `kind`.
pub fn looks_like_manifest(text: &str) -> bool {
    text.contains("apiVersion") && text.contains("kind")
}

pub fn parse(file: &str, text: &str, into: &mut Manifests) {
    let file = paths::normalize(file);
    // Split templated documents out before parsing: their structure depends on values
    // that only the template engine knows.
    let mut chunks: Vec<(usize, String)> = Vec::new();
    let mut cur = String::new();
    let mut start_line = 0usize;
    for (idx, line) in text.lines().enumerate() {
        if line.trim_end() == "---" || line.starts_with("--- ") {
            chunks.push((start_line, std::mem::take(&mut cur)));
            start_line = idx + 1;
            continue;
        }
        cur.push_str(line);
        cur.push('\n');
    }
    chunks.push((start_line, cur));
    for (offset, chunk) in chunks {
        if chunk.trim().is_empty() {
            continue;
        }
        if chunk.contains("{{") {
            into.skipped += 1;
            continue;
        }
        for doc in yaml::parse_documents(&chunk) {
            let line_shift = offset as u32;
            let kind = match doc.get("kind").and_then(|k| k.as_str()) {
                Some(k) => k.to_string(),
                None => continue,
            };
            let name = doc
                .path(&["metadata", "name"])
                .and_then(|n| n.as_str())
                .unwrap_or("")
                .to_string();
            let namespace = doc
                .path(&["metadata", "namespace"])
                .and_then(|n| n.as_str())
                .map(str::to_string);
            if WORKLOAD_KINDS.contains(&kind.as_str()) {
                let template = match kind.as_str() {
                    "Pod" => Some(&doc),
                    "CronJob" => doc.path(&["spec", "jobTemplate", "spec", "template"]),
                    _ => doc.path(&["spec", "template"]),
                };
                let mut list = Vec::new();
                let mut pod_labels = Vec::new();
                if let Some(t) = template {
                    pod_labels = string_map(t.path(&["metadata", "labels"]));
                    if let Some(spec) = t.get("spec") {
                        containers(spec, "initContainers", true, &mut list);
                        containers(spec, "containers", false, &mut list);
                    }
                }
                for c in &mut list {
                    c.line += line_shift;
                }
                into.workloads.push(Workload {
                    file: file.clone(),
                    line: doc.line + line_shift,
                    kind,
                    name,
                    namespace,
                    replicas: doc
                        .path(&["spec", "replicas"])
                        .and_then(|r| r.as_str())
                        .map(str::to_string),
                    pod_labels,
                    containers: list,
                });
            } else if kind == "Service" {
                let ports = doc
                    .path(&["spec", "ports"])
                    .map(|p| {
                        p.as_seq()
                            .iter()
                            .filter_map(|port| {
                                let number = port.get("port").and_then(|v| v.as_str())?;
                                Some(match port.get("targetPort").and_then(|v| v.as_str()) {
                                    Some(target) if target != number => format!("{}→{}", number, target),
                                    _ => number.to_string(),
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                into.services.push(Service {
                    file: file.clone(),
                    line: doc.line + line_shift,
                    name,
                    namespace,
                    service_type: doc
                        .path(&["spec", "type"])
                        .and_then(|t| t.as_str())
                        .map(str::to_string),
                    selector: string_map(doc.path(&["spec", "selector"])),
                    ports,
                });
            }
        }
    }
}

/// True when a Service selects the pods of a workload: every selector entry is one of the
/// pod labels and both live in the same namespace (Kubernetes label-selector semantics).
pub fn selects(service: &Service, workload: &Workload) -> bool {
    if service.selector.is_empty() || service.namespace != workload.namespace {
        return false;
    }
    service
        .selector
        .iter()
        .all(|(k, v)| workload.pod_labels.iter().any(|(lk, lv)| lk == k && lv == v))
}
