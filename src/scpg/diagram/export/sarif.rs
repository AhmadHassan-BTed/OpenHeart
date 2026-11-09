//! SarifExporter — SARIF 2.1.0 log of the Phase 9 design-pattern detections (§10.4).
//!
//! Each detected pattern instance becomes one `note`-level result located at the anchoring
//! class's declaration, with the participating classes as related locations and the
//! structural-evidence score and evidence bits as properties. Results are tagged as
//! heuristic: the scores are uncalibrated. Only line numbers are reported, because the
//! token corpus records byte columns, which match neither SARIF column kind.

use std::collections::BTreeSet;

use serde_json::{json, Value};

use crate::scpg::diagram::common::DiagramUtils;
use crate::scpg::diagram::context::{DiagramContext, SourceSpan};
use crate::uma::types::*;

pub const SARIF_SCHEMA: &str = "https://json.schemastore.org/sarif-2.1.0.json";

/// Display name of a pattern participant role.
pub fn role_name(role: u8) -> &'static str {
    match role {
        ROLE_SINGLETON => "Singleton",
        ROLE_SUBJECT => "Subject",
        ROLE_OBSERVER => "Observer",
        ROLE_CONCRETE_OBSERVER => "ConcreteObserver",
        ROLE_CREATOR => "Creator",
        ROLE_CONCRETE_CREATOR => "ConcreteCreator",
        ROLE_PRODUCT => "Product",
        ROLE_CONCRETE_PRODUCT => "ConcreteProduct",
        ROLE_BUILDER => "Builder",
        ROLE_CONTEXT => "Context",
        ROLE_STATE => "State",
        ROLE_CONCRETE_STATE => "ConcreteState",
        ROLE_STRATEGY => "Strategy",
        ROLE_CONCRETE_STRATEGY => "ConcreteStrategy",
        ROLE_ABSTRACT_CLASS => "AbstractClass",
        ROLE_CONCRETE_CLASS => "ConcreteClass",
        ROLE_TEMPLATE_METHOD => "TemplateMethod",
        ROLE_PRIMITIVE_OPERATION => "PrimitiveOperation",
        ROLE_COMPONENT => "Component",
        ROLE_CONCRETE_COMPONENT => "ConcreteComponent",
        ROLE_DECORATOR => "Decorator",
        ROLE_TARGET => "Target",
        ROLE_ADAPTER => "Adapter",
        ROLE_ADAPTEE => "Adaptee",
        ROLE_FACADE => "Facade",
        ROLE_SUBSYSTEM => "Subsystem",
        ROLE_COMPOSITE => "Composite",
        ROLE_LEAF => "Leaf",
        ROLE_PROXY => "Proxy",
        ROLE_REAL_SUBJECT => "RealSubject",
        ROLE_VISITOR => "Visitor",
        ROLE_CONCRETE_VISITOR => "ConcreteVisitor",
        ROLE_ELEMENT => "Element",
        ROLE_CONCRETE_ELEMENT => "ConcreteElement",
        ROLE_COMMAND => "Command",
        ROLE_CONCRETE_COMMAND => "ConcreteCommand",
        ROLE_INVOKER => "Invoker",
        ROLE_RECEIVER => "Receiver",
        ROLE_HANDLER => "Handler",
        ROLE_CONCRETE_HANDLER => "ConcreteHandler",
        ROLE_ABSTRACT_FACTORY => "AbstractFactory",
        ROLE_CONCRETE_FACTORY => "ConcreteFactory",
        ROLE_PROTOTYPE => "Prototype",
        ROLE_CONCRETE_PROTOTYPE => "ConcretePrototype",
        ROLE_ITERATOR => "Iterator",
        ROLE_CONCRETE_ITERATOR => "ConcreteIterator",
        ROLE_AGGREGATE => "Aggregate",
        ROLE_MEDIATOR => "Mediator",
        ROLE_CONCRETE_MEDIATOR => "ConcreteMediator",
        ROLE_COLLEAGUE => "Colleague",
        ROLE_ORIGINATOR => "Originator",
        ROLE_MEMENTO => "Memento",
        ROLE_CARETAKER => "Caretaker",
        ROLE_FLYWEIGHT_FACTORY => "FlyweightFactory",
        ROLE_FLYWEIGHT => "Flyweight",
        ROLE_ABSTRACTION => "Abstraction",
        ROLE_REFINED_ABSTRACTION => "RefinedAbstraction",
        ROLE_IMPLEMENTOR => "Implementor",
        ROLE_CONCRETE_IMPLEMENTOR => "ConcreteImplementor",
        ROLE_ABSTRACT_EXPRESSION => "AbstractExpression",
        ROLE_TERMINAL_EXPRESSION => "TerminalExpression",
        ROLE_NONTERMINAL_EXPRESSION => "NonterminalExpression",
        ROLE_CLIENT => "Client",
        _ => "Participant",
    }
}

/// `file:` URI (or relative URI reference) of an ingested path.
fn artifact_uri(path: &str) -> String {
    let mut encoded = String::with_capacity(path.len());
    for c in path.chars() {
        match c {
            ' ' => encoded.push_str("%20"),
            '%' => encoded.push_str("%25"),
            '#' => encoded.push_str("%23"),
            '?' => encoded.push_str("%3F"),
            _ => encoded.push(c),
        }
    }
    let bytes = encoded.as_bytes();
    if encoded.starts_with('/') {
        format!("file://{}", encoded)
    } else if bytes.len() > 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        format!("file:///{}", encoded)
    } else {
        encoded
    }
}

fn location(span: &SourceSpan, fqn: &str) -> Value {
    let mut physical = json!({
        "region": {
            "startLine": span.line_start,
            "endLine": span.line_end.max(span.line_start)
        }
    });
    if let Some(path) = &span.path {
        physical["artifactLocation"] = json!({ "uri": artifact_uri(path) });
    }
    json!({
        "physicalLocation": physical,
        "logicalLocations": [{ "fullyQualifiedName": fqn, "kind": "type" }]
    })
}

pub struct SarifExporter;

impl SarifExporter {
    fn class_span(ctx: &DiagramContext, sym: u32) -> Option<SourceSpan> {
        ctx.class_record(sym)
            .and_then(|c| ctx.span_of_link(&c.uml_link))
            .or_else(|| ctx.symbol_span(sym))
    }

    fn class_fqn(ctx: &DiagramContext, sym: u32) -> String {
        ctx.type_fqn(sym)
            .unwrap_or_else(|| format!("<type #{}>", sym))
    }

    /// SARIF 2.1.0 log (pretty-printed JSON) of every detected pattern instance.
    pub fn export_patterns(ctx: &DiagramContext) -> String {
        let kinds: BTreeSet<u8> = ctx
            .uma
            .pattern_instances
            .iter()
            .map(|inst| inst.pattern_kind)
            .filter(|&kind| !pattern_kind_name(kind).is_empty())
            .collect();
        let rule_index = |kind: u8| -> usize { kinds.iter().position(|&k| k == kind).unwrap_or(0) };
        let rules: Vec<Value> = kinds
            .iter()
            .map(|&kind| {
                let name = pattern_kind_name(kind);
                json!({
                    "id": format!("openheart.pattern.{}", name.to_ascii_lowercase()),
                    "name": name,
                    "shortDescription": { "text": format!("{} design pattern instance", name) },
                    "fullDescription": {
                        "text": format!("Classes whose structure and call relations match the {} pattern constraints checked by OpenHeart Phase 9.", name)
                    },
                    "defaultConfiguration": { "level": "note" },
                    "properties": { "provenance": "heuristic", "tags": ["design-pattern"] }
                })
            })
            .collect();

        let mut results: Vec<Value> = Vec::new();
        for inst in &ctx.uma.pattern_instances {
            let name = pattern_kind_name(inst.pattern_kind);
            if name.is_empty() {
                continue;
            }
            let primary_fqn = Self::class_fqn(ctx, inst.primary_sym);
            let roles: Vec<Value> = inst
                .roles
                .iter()
                .map(|r| {
                    json!({
                        "role": role_name(r.role),
                        "type": Self::class_fqn(ctx, r.sym_id)
                    })
                })
                .collect();
            let role_text: Vec<String> = inst
                .roles
                .iter()
                .map(|r| format!("{} = {}", role_name(r.role), Self::class_fqn(ctx, r.sym_id)))
                .collect();
            let mut result = json!({
                "ruleId": format!("openheart.pattern.{}", name.to_ascii_lowercase()),
                "ruleIndex": rule_index(inst.pattern_kind),
                "level": "note",
                "message": {
                    "text": format!(
                        "{} anchors a {} instance (structural-evidence score {}%, uncalibrated): {}.",
                        primary_fqn,
                        name,
                        inst.score.min(100),
                        role_text.join("; ")
                    )
                },
                "partialFingerprints": {
                    "openheartPattern/v1": DiagramUtils::short_hash(&[name, &primary_fqn])
                },
                "properties": {
                    "provenance": "heuristic",
                    "score": inst.score,
                    "evidenceMask": format!("0x{:016x}", inst.evidence_mask),
                    "roles": roles
                }
            });
            if let Some(span) = Self::class_span(ctx, inst.primary_sym) {
                result["locations"] = json!([location(&span, &primary_fqn)]);
            }
            let related: Vec<Value> = inst
                .roles
                .iter()
                .filter(|r| r.sym_id != inst.primary_sym)
                .enumerate()
                .filter_map(|(i, r)| {
                    let span = Self::class_span(ctx, r.sym_id)?;
                    let fqn = Self::class_fqn(ctx, r.sym_id);
                    let mut loc = location(&span, &fqn);
                    loc["id"] = json!(i);
                    loc["message"] = json!({ "text": role_name(r.role) });
                    Some(loc)
                })
                .collect();
            if !related.is_empty() {
                result["relatedLocations"] = json!(related);
            }
            results.push(result);
        }

        let log = json!({
            "$schema": SARIF_SCHEMA,
            "version": "2.1.0",
            "runs": [{
                "tool": {
                    "driver": {
                        "name": "OpenHeart",
                        "version": env!("CARGO_PKG_VERSION"),
                        "rules": rules
                    }
                },
                "properties": { "inputDigest": ctx.input_digest() },
                "results": results
            }]
        });
        serde_json::to_string_pretty(&log).unwrap_or_else(|_| "{}".to_string())
    }
}
