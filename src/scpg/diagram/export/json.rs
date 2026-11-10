//! JSONExporter — versioned Graph IR serialization of diagram view models (§10.4).
//!
//! The Graph IR is the studio's input. Node ids are the view-unique element keys (qualified
//! names, `fqn#sym` on collision, or structural ids), so ids never repeat. Source locations
//! come from the declaring token range (`file`/`lines`/`span`, `null` when unknown), the
//! metadata carries the SHA-256 input digest, a SHA-256 content hash of the graph and the
//! result of the structural checks behind `verified`, and every element is tagged with its
//! provenance tier. [`JSONExporter::json_schema`] publishes the schema of the current version.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::ingestion::TokenCorpusArtifact;
use crate::scpg::diagram::common::DiagramUtils;
use crate::scpg::diagram::context::{DiagramContext, SourceSpan};
use crate::scpg::diagram::model::*;
use crate::scpg::diagram::views;
use crate::symbol::SymbolTableArtifact;
use crate::uma::types::UMLMetadataArtifact;

/// Schema identifier of the Graph IR emitted by this version.
pub const GRAPH_IR_SCHEMA_VERSION: &str = "openheart.graph-ir/2";

/// Documents without `schema_version` predate versioning.
fn legacy_schema_version() -> String {
    "openheart.graph-ir/1".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphIR {
    #[serde(default = "legacy_schema_version")]
    pub schema_version: String,
    pub diagram_type: String,
    pub title: String,
    /// `derived`, `heuristic` or `unavailable`.
    #[serde(default)]
    pub provenance: String,
    pub nodes: Vec<GraphNodeIR>,
    pub edges: Vec<GraphEdgeIR>,
    /// Ordered sequence scenarios (sequence diagrams only).
    #[serde(default)]
    pub interactions: Vec<InteractionIR>,
    pub metadata: GraphMetadataIR,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphMetadataIR {
    pub total_nodes: usize,
    pub total_edges: usize,
    /// SHA-256 digest (hex) of the analyzed input snapshot. Same value as `input_digest`,
    /// kept under its historical name.
    pub compiler_hash: String,
    /// True when the view was derived and every structural check passed (unique ids,
    /// resolvable edge endpoints and parents, acyclic containment); see `checks`.
    pub verified: bool,
    #[serde(default)]
    pub input_digest: String,
    /// SHA-256 digest (hex) of the canonical serialization of nodes, edges and interactions.
    #[serde(default)]
    pub content_hash: String,
    /// Failed structural checks (empty when `verified`).
    #[serde(default)]
    pub checks: Vec<String>,
    #[serde(default)]
    pub notes: Vec<String>,
    /// Qualified names of elements left out by budgets or focus.
    #[serde(default)]
    pub omitted: Vec<String>,
    #[serde(default)]
    pub omitted_count: usize,
    /// Artifacts the view was derived from.
    #[serde(default)]
    pub sources: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphNodeIR {
    pub id: String,
    pub label: String,
    pub name: String,
    /// Studio node kind: "class", "interface", "abstract", "enum", "package", "state",
    /// "action", "component", "bb", "bdd_gate", "bdd_terminal", "participant", ...
    pub kind: String,
    pub stereotype: Option<String>,
    pub parent: Option<String>,
    pub nest_level: u32,
    pub is_package: bool,
    pub is_domain_tier: bool,
    /// Source file path, `None` when the element has no known source location.
    pub file: Option<String>,
    /// `[line_start]` or `[line_start, line_end]`, empty when unknown.
    pub lines: Vec<u32>,
    pub fields: Vec<MemberIR>,
    pub methods: Vec<MemberIR>,
    pub instructions: Vec<String>,
    /// Identifier used for this element by the PlantUML and Mermaid outputs.
    #[serde(default)]
    pub alias: String,
    /// Precise view-IR node kind.
    #[serde(default)]
    pub view_kind: String,
    #[serde(default)]
    pub qualified_name: Option<String>,
    #[serde(default)]
    pub stereotypes: Vec<String>,
    #[serde(default)]
    pub span: Option<SourceSpan>,
    #[serde(default)]
    pub provenance: String,
    #[serde(default)]
    pub sym_id: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParamIR {
    pub name: Option<String>,
    pub type_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberIR {
    pub visibility: String, // "+", "-", "#", "~"
    pub name: String,
    pub type_name: String,
    pub signature: String,
    pub is_static: bool,
    pub is_final: bool,
    /// "field", "literal", "method" or "constructor".
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub is_abstract: bool,
    #[serde(default)]
    pub multiplicity: Option<String>,
    /// Parameters in declaration order (empty when `params_known` is false).
    #[serde(default)]
    pub params: Vec<ParamIR>,
    #[serde(default)]
    pub params_known: bool,
    #[serde(default)]
    pub param_count: u16,
    #[serde(default)]
    pub span: Option<SourceSpan>,
    #[serde(default)]
    pub sym_id: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphEdgeIR {
    pub id: String,
    pub source: String,
    pub target: String,
    /// "generalization", "realization", "composition", "aggregation", "association",
    /// "dependency", "control_flow", "data_use", "message", "call", ...
    pub kind: String,
    pub label: Option<String>,
    pub arrow: String,
    #[serde(default)]
    pub source_multiplicity: Option<String>,
    #[serde(default)]
    pub target_multiplicity: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub order: Option<u32>,
    #[serde(default)]
    pub span: Option<SourceSpan>,
    #[serde(default)]
    pub provenance: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InteractionEventIR {
    /// "call", "reply", "deactivate", "fragment_start", "fragment_else", "fragment_end" or
    /// "note".
    pub event: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fragment: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_async: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activation: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<SourceSpan>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InteractionIR {
    pub id: String,
    pub title: String,
    pub lifelines: Vec<String>,
    pub events: Vec<InteractionEventIR>,
    pub omitted_messages: usize,
    pub truncated: bool,
}

pub struct JSONExporter;

impl JSONExporter {
    /// Builds the view of `kind` and converts it to Graph IR.
    pub fn export(kind: ViewKind, ctx: &DiagramContext) -> GraphIR {
        Self::from_model(&views::build(kind, ctx), &ctx.input_digest())
    }

    /// Graph IR of a diagram type name (historical aliases accepted); `None` for unknown types.
    pub fn export_type(diagram_type: &str, ctx: &DiagramContext) -> Option<GraphIR> {
        ViewKind::parse(diagram_type).map(|kind| Self::export(kind, ctx))
    }

    fn export_basic(
        kind: ViewKind,
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> GraphIR {
        Self::export(kind, &DiagramContext::new(uma, sta, tca))
    }

    /// Converts a finalized view model to Graph IR.
    pub fn from_model(model: &ViewModel, input_digest: &str) -> GraphIR {
        let checks = model.validate();
        let nodes: Vec<GraphNodeIR> = model
            .nodes
            .iter()
            .map(|n| Self::node_ir(model, n))
            .collect();
        let edges: Vec<GraphEdgeIR> = model.edges.iter().map(Self::edge_ir).collect();
        let interactions: Vec<InteractionIR> = model
            .interactions
            .iter()
            .map(Self::interaction_ir)
            .collect();
        let content_hash = Self::content_hash(&nodes, &edges, &interactions);
        GraphIR {
            schema_version: GRAPH_IR_SCHEMA_VERSION.to_string(),
            diagram_type: model.kind.json_type().to_string(),
            title: model.title.clone(),
            provenance: model.provenance.as_str().to_string(),
            metadata: GraphMetadataIR {
                total_nodes: nodes.len(),
                total_edges: edges.len(),
                compiler_hash: input_digest.to_string(),
                verified: checks.is_empty() && !model.is_unavailable(),
                input_digest: input_digest.to_string(),
                content_hash,
                checks,
                notes: model.notes.clone(),
                omitted: model.omitted.clone(),
                omitted_count: model.omitted_count(),
                sources: model.sources.clone(),
            },
            nodes,
            edges,
            interactions,
        }
    }

    fn content_hash(
        nodes: &[GraphNodeIR],
        edges: &[GraphEdgeIR],
        interactions: &[InteractionIR],
    ) -> String {
        let bytes = serde_json::to_vec(&(nodes, edges, interactions)).unwrap_or_default();
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        let digest: [u8; 32] = hasher.finalize().into();
        DiagramUtils::hex(&digest)
    }

    /// Node kind string understood by the studio's graph loader.
    fn studio_kind(view: ViewKind, kind: NodeKind) -> &'static str {
        match kind {
            NodeKind::Package | NodeKind::Group => "package",
            NodeKind::StructuredClass => "composite_classifier",
            NodeKind::Class | NodeKind::Record => "class",
            NodeKind::AbstractClass => "abstract",
            NodeKind::Interface | NodeKind::ProvidedInterface => "interface",
            NodeKind::Enum => "enum",
            NodeKind::Annotation => "annotation",
            NodeKind::Object => "object",
            NodeKind::Component => "component",
            NodeKind::Part => "part",
            NodeKind::Stereotype => "stereotype",
            NodeKind::Metaclass => "metaclass",
            NodeKind::Actor => "actor",
            NodeKind::Lifeline => "participant",
            NodeKind::State => "state",
            NodeKind::Initial | NodeKind::Final if view == ViewKind::StateMachine => "state",
            NodeKind::BasicBlock => "bb",
            NodeKind::SsaValue | NodeKind::SsaPhi => "data_node",
            NodeKind::BddVariable => "bdd_gate",
            NodeKind::BddTrue | NodeKind::BddFalse => "bdd_terminal",
            _ => "action",
        }
    }

    fn primary_stereotype(node: &ViewNode, studio_kind: &str) -> String {
        let implied = match node.kind {
            NodeKind::InteractionUse => Some("interaction_use"),
            NodeKind::Initial => Some("initial"),
            NodeKind::Final => Some("final"),
            NodeKind::Decision => Some("decision"),
            NodeKind::Merge => Some("merge"),
            NodeKind::Fork => Some("fork"),
            NodeKind::Join => Some("join"),
            NodeKind::ExceptionHandler => Some("exception"),
            NodeKind::Function => Some("function"),
            NodeKind::SsaPhi => Some("phi"),
            NodeKind::SsaValue => Some("value"),
            NodeKind::Statement => Some("statement"),
            NodeKind::Group => Some("group"),
            NodeKind::BddTrue | NodeKind::BddFalse => Some("bdd_terminal"),
            NodeKind::BddVariable => Some("bdd_gate"),
            _ => None,
        };
        let text = implied
            .map(str::to_string)
            .or_else(|| node.stereotypes.first().cloned())
            .unwrap_or_else(|| studio_kind.to_string());
        format!("<<{}>>", text)
    }

    fn member_ir(member: &MemberView) -> MemberIR {
        MemberIR {
            visibility: member.visibility.to_string(),
            name: member.name.clone(),
            type_name: member.type_name.clone().unwrap_or_default(),
            signature: format!("{}{}", member.visibility, member.signature()),
            is_static: member.is_static,
            is_final: member.is_final,
            kind: match member.kind {
                MemberKind::Field => "field",
                MemberKind::Literal => "literal",
                MemberKind::Method => "method",
                MemberKind::Constructor => "constructor",
            }
            .to_string(),
            is_abstract: member.is_abstract,
            multiplicity: member.multiplicity.clone(),
            params: member
                .params
                .as_ref()
                .map(|ps| {
                    ps.iter()
                        .map(|p| ParamIR {
                            name: p.name.clone(),
                            type_name: p.type_name.clone(),
                        })
                        .collect()
                })
                .unwrap_or_default(),
            params_known: member.params.is_some(),
            param_count: member.param_count,
            span: member.span.clone(),
            sym_id: member.sym_id,
        }
    }

    fn node_ir(model: &ViewModel, node: &ViewNode) -> GraphNodeIR {
        let kind = Self::studio_kind(model.kind, node.kind);
        let is_package = node.kind.is_container();
        let (fields, methods): (Vec<&MemberView>, Vec<&MemberView>) =
            node.members.iter().partition(|m| !m.is_operation());
        let lines = match &node.span {
            Some(s) if s.line_end > s.line_start => vec![s.line_start, s.line_end],
            Some(s) => vec![s.line_start],
            None => Vec::new(),
        };
        GraphNodeIR {
            id: node.key.clone(),
            label: node.label.clone(),
            name: node.label.clone(),
            kind: kind.to_string(),
            stereotype: Some(Self::primary_stereotype(node, kind)),
            parent: node.parent.clone(),
            nest_level: model.depth(&node.key) as u32,
            is_package,
            is_domain_tier: is_package && node.parent.is_none(),
            file: node.span.as_ref().and_then(|s| s.path.clone()),
            lines,
            fields: fields.into_iter().map(Self::member_ir).collect(),
            methods: methods.into_iter().map(Self::member_ir).collect(),
            instructions: node.details.clone(),
            alias: node.alias.clone(),
            view_kind: node.kind.as_str().to_string(),
            qualified_name: node.qualified_name.clone(),
            stereotypes: node.stereotypes.clone(),
            span: node.span.clone(),
            provenance: node.provenance.as_str().to_string(),
            sym_id: node.sym_id,
        }
    }

    fn arrow(kind: EdgeKind) -> &'static str {
        match kind {
            EdgeKind::Generalization | EdgeKind::Extension => "--|>",
            EdgeKind::Realization => "..|>",
            EdgeKind::Aggregation => "o--",
            EdgeKind::Composition => "*--",
            EdgeKind::Nesting => "+--",
            EdgeKind::Link | EdgeKind::Connector | EdgeKind::Provides => "--",
            kind if kind.is_dashed() => "..>",
            _ => "-->",
        }
    }

    fn edge_ir(edge: &ViewEdge) -> GraphEdgeIR {
        GraphEdgeIR {
            id: edge.key.clone(),
            source: edge.source.clone(),
            target: edge.target.clone(),
            kind: edge.kind.as_str().to_string(),
            label: edge.label.clone().or_else(|| edge.role.clone()),
            arrow: Self::arrow(edge.kind).to_string(),
            source_multiplicity: edge.source_multiplicity.clone(),
            target_multiplicity: edge.target_multiplicity.clone(),
            role: edge.role.clone(),
            order: edge.order,
            span: edge.span.clone(),
            provenance: edge.provenance.as_str().to_string(),
        }
    }

    fn event_ir(event: &InteractionEvent) -> InteractionEventIR {
        let mut ir = InteractionEventIR {
            event: String::new(),
            from: None,
            to: None,
            label: None,
            number: None,
            fragment: None,
            guard: None,
            is_async: None,
            activation: None,
            span: None,
        };
        match event {
            InteractionEvent::Call {
                from,
                to,
                label,
                number,
                is_async,
                activates,
                span,
                ..
            } => {
                ir.event = "call".to_string();
                ir.from = Some(from.clone());
                ir.to = Some(to.clone());
                ir.label = Some(label.clone());
                ir.number = Some(number.clone());
                ir.is_async = Some(*is_async);
                ir.activation = Some(*activates);
                ir.span = span.clone();
            }
            InteractionEvent::Reply {
                from,
                to,
                label,
                deactivates,
            } => {
                ir.event = "reply".to_string();
                ir.from = Some(from.clone());
                ir.to = Some(to.clone());
                ir.label = Some(label.clone());
                ir.activation = Some(*deactivates);
            }
            InteractionEvent::Deactivate { lifeline } => {
                ir.event = "deactivate".to_string();
                ir.from = Some(lifeline.clone());
            }
            InteractionEvent::FragmentStart { kind, guard } => {
                ir.event = "fragment_start".to_string();
                ir.fragment = Some(kind.as_str().to_string());
                ir.guard = guard.clone();
            }
            InteractionEvent::FragmentElse { guard } => {
                ir.event = "fragment_else".to_string();
                ir.guard = guard.clone();
            }
            InteractionEvent::FragmentEnd => {
                ir.event = "fragment_end".to_string();
            }
            InteractionEvent::Note { over, text } => {
                ir.event = "note".to_string();
                ir.from = Some(over.clone());
                ir.label = Some(text.clone());
            }
        }
        ir
    }

    fn interaction_ir(interaction: &Interaction) -> InteractionIR {
        InteractionIR {
            id: interaction.key.clone(),
            title: interaction.title.clone(),
            lifelines: interaction.lifelines.clone(),
            events: interaction.events.iter().map(Self::event_ir).collect(),
            omitted_messages: interaction.omitted_messages,
            truncated: interaction.truncated,
        }
    }

    /// JSON Schema (draft 2020-12) of the Graph IR emitted by this version.
    pub fn json_schema() -> serde_json::Value {
        let string_or_null = serde_json::json!({ "type": ["string", "null"] });
        let span = serde_json::json!({
            "type": ["object", "null"],
            "required": ["file_id", "line_start", "col_start", "line_end", "col_end"],
            "properties": {
                "file_id": { "type": "integer", "minimum": 0 },
                "path": { "type": ["string", "null"] },
                "line_start": { "type": "integer", "minimum": 1 },
                "col_start": { "type": "integer", "minimum": 0 },
                "line_end": { "type": "integer", "minimum": 1 },
                "col_end": { "type": "integer", "minimum": 0 },
                "first_token": { "type": "integer" },
                "last_token": { "type": "integer" }
            }
        });
        let member = serde_json::json!({
            "type": "object",
            "required": ["visibility", "name", "type_name", "signature", "is_static", "is_final"],
            "properties": {
                "visibility": { "enum": ["+", "-", "#", "~"] },
                "name": { "type": "string" },
                "type_name": { "type": "string" },
                "signature": { "type": "string" },
                "is_static": { "type": "boolean" },
                "is_final": { "type": "boolean" },
                "is_abstract": { "type": "boolean" },
                "kind": { "enum": ["field", "literal", "method", "constructor"] },
                "multiplicity": string_or_null,
                "params_known": { "type": "boolean" },
                "param_count": { "type": "integer", "minimum": 0 },
                "params": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": { "name": string_or_null, "type_name": string_or_null }
                    }
                },
                "span": span
            }
        });
        let provenance = serde_json::json!({ "enum": ["derived", "heuristic", "unavailable"] });
        let diagram_types: Vec<&str> = ViewKind::ALL.iter().map(|k| k.json_type()).collect();
        let schema_id = format!(
            "https://openheart.dev/schema/{}.json",
            GRAPH_IR_SCHEMA_VERSION
        );
        serde_json::json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$id": schema_id,
            "title": "OpenHeart Graph IR",
            "type": "object",
            "required": ["schema_version", "diagram_type", "title", "provenance", "nodes", "edges", "metadata"],
            "properties": {
                "schema_version": { "const": GRAPH_IR_SCHEMA_VERSION },
                "diagram_type": { "enum": diagram_types },
                "title": { "type": "string" },
                "provenance": provenance,
                "nodes": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "required": ["id", "label", "name", "kind", "parent", "nest_level", "is_package", "file", "lines", "fields", "methods", "instructions"],
                        "properties": {
                            "id": { "type": "string", "minLength": 1 },
                            "label": { "type": "string" },
                            "name": { "type": "string" },
                            "kind": { "type": "string" },
                            "view_kind": { "type": "string" },
                            "alias": { "type": "string", "pattern": "^[A-Za-z_][A-Za-z0-9_]*$" },
                            "stereotype": string_or_null,
                            "stereotypes": { "type": "array", "items": { "type": "string" } },
                            "parent": string_or_null,
                            "nest_level": { "type": "integer", "minimum": 0 },
                            "is_package": { "type": "boolean" },
                            "is_domain_tier": { "type": "boolean" },
                            "file": string_or_null,
                            "lines": { "type": "array", "items": { "type": "integer", "minimum": 1 }, "maxItems": 2 },
                            "qualified_name": string_or_null,
                            "span": span,
                            "provenance": provenance,
                            "sym_id": { "type": ["integer", "null"] },
                            "fields": { "type": "array", "items": member },
                            "methods": { "type": "array", "items": member },
                            "instructions": { "type": "array", "items": { "type": "string" } }
                        }
                    }
                },
                "edges": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "required": ["id", "source", "target", "kind", "arrow"],
                        "properties": {
                            "id": { "type": "string", "minLength": 1 },
                            "source": { "type": "string" },
                            "target": { "type": "string" },
                            "kind": { "type": "string" },
                            "label": string_or_null,
                            "arrow": { "type": "string" },
                            "source_multiplicity": string_or_null,
                            "target_multiplicity": string_or_null,
                            "role": string_or_null,
                            "order": { "type": ["integer", "null"] },
                            "span": span,
                            "provenance": provenance
                        }
                    }
                },
                "interactions": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "required": ["id", "title", "lifelines", "events"],
                        "properties": {
                            "id": { "type": "string" },
                            "title": { "type": "string" },
                            "lifelines": { "type": "array", "items": { "type": "string" } },
                            "events": {
                                "type": "array",
                                "items": {
                                    "type": "object",
                                    "required": ["event"],
                                    "properties": {
                                        "event": { "enum": ["call", "reply", "deactivate", "fragment_start", "fragment_else", "fragment_end", "note"] }
                                    }
                                }
                            },
                            "omitted_messages": { "type": "integer", "minimum": 0 },
                            "truncated": { "type": "boolean" }
                        }
                    }
                },
                "metadata": {
                    "type": "object",
                    "required": ["total_nodes", "total_edges", "compiler_hash", "verified", "input_digest", "content_hash", "checks"],
                    "properties": {
                        "total_nodes": { "type": "integer", "minimum": 0 },
                        "total_edges": { "type": "integer", "minimum": 0 },
                        "compiler_hash": { "type": "string", "pattern": "^[0-9a-f]{64}$" },
                        "input_digest": { "type": "string", "pattern": "^[0-9a-f]{64}$" },
                        "content_hash": { "type": "string", "pattern": "^[0-9a-f]{64}$" },
                        "verified": { "type": "boolean" },
                        "checks": { "type": "array", "items": { "type": "string" } },
                        "notes": { "type": "array", "items": { "type": "string" } },
                        "omitted": { "type": "array", "items": { "type": "string" } },
                        "omitted_count": { "type": "integer", "minimum": 0 },
                        "sources": { "type": "array", "items": { "type": "string" } }
                    }
                }
            }
        })
    }

    // ── Static entry points (mandatory artifacts only) ───────────────────────

    pub fn export_class_diagram(
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> GraphIR {
        Self::export_basic(ViewKind::Class, uma, sta, tca)
    }

    pub fn export_package_diagram(
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> GraphIR {
        Self::export_basic(ViewKind::Package, uma, sta, tca)
    }

    pub fn export_sequence_diagram(
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> GraphIR {
        Self::export_basic(ViewKind::Sequence, uma, sta, tca)
    }

    pub fn export_state_diagram(
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> GraphIR {
        Self::export_basic(ViewKind::StateMachine, uma, sta, tca)
    }

    pub fn export_activity_diagram(
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> GraphIR {
        Self::export_basic(ViewKind::Activity, uma, sta, tca)
    }

    pub fn export_component_diagram(
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> GraphIR {
        Self::export_basic(ViewKind::Component, uma, sta, tca)
    }

    pub fn export_deployment_diagram(
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> GraphIR {
        Self::export_basic(ViewKind::Deployment, uma, sta, tca)
    }

    pub fn export_usecase_diagram(
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> GraphIR {
        Self::export_basic(ViewKind::UseCase, uma, sta, tca)
    }

    pub fn export_object_diagram(
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> GraphIR {
        Self::export_basic(ViewKind::Object, uma, sta, tca)
    }

    pub fn export_composite_diagram(
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> GraphIR {
        Self::export_basic(ViewKind::CompositeStructure, uma, sta, tca)
    }

    pub fn export_profile_diagram(
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> GraphIR {
        Self::export_basic(ViewKind::Profile, uma, sta, tca)
    }

    pub fn export_timing_diagram(
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> GraphIR {
        Self::export_basic(ViewKind::Timing, uma, sta, tca)
    }

    pub fn export_communication_diagram(
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> GraphIR {
        Self::export_basic(ViewKind::Communication, uma, sta, tca)
    }

    pub fn export_interaction_diagram(
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> GraphIR {
        Self::export_basic(ViewKind::InteractionOverview, uma, sta, tca)
    }

    /// Unavailable unless exported through [`Self::export`] with a CFA in the context.
    pub fn export_cfg_diagram(
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> GraphIR {
        Self::export_basic(ViewKind::Cfg, uma, sta, tca)
    }

    /// Unavailable unless exported through [`Self::export`] with an SSA artifact in the context.
    pub fn export_dfg_diagram(
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> GraphIR {
        Self::export_basic(ViewKind::Dfg, uma, sta, tca)
    }

    /// Unavailable unless exported through [`Self::export`] with an SSA artifact in the context.
    pub fn export_cdg_diagram(
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> GraphIR {
        Self::export_basic(ViewKind::Cdg, uma, sta, tca)
    }

    /// Unavailable unless exported through [`Self::export`] with a CGA in the context.
    pub fn export_callgraph_diagram(
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> GraphIR {
        Self::export_basic(ViewKind::CallGraph, uma, sta, tca)
    }

    /// Unavailable unless exported through [`Self::export`] with a PSA in the context.
    pub fn export_robdd_diagram(
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> GraphIR {
        Self::export_basic(ViewKind::Robdd, uma, sta, tca)
    }
}
