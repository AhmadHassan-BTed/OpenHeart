//! Machine-readable `ohb-json` export of a snapshot (§10.7).
//!
//! The export is versioned (`OHB_JSON_SCHEMA_ID`) and described by `ohb_json_schema()`. It
//! carries what scorers previously had to regex out of PlantUML: symbols with stable ids,
//! FQNs, kinds and source spans; type edges; overrides; call edges and call sites with
//! positions; SCCs; points-to; path summaries; the UML model and pattern instances. When the
//! CFG / SSA artifacts are supplied (or the CFG is read back from a `.scpg`), per-method
//! CFGs with dominators and φ-functions are included too. Anything the inputs cannot
//! provide is emitted as `null` together with a `*_unavailable` reason, never invented.

use serde_json::{json, Value as Json};

use crate::cfg::serializer::CFGArtifact;
use crate::core::types::cfg::CFGEdgeType;
use crate::core::types::cg::CALL_SITE_FLAG_IS_POLYMORPHIC;
use crate::scpg::query::graph::call_type_name;
use crate::scpg::query::semantic::{pattern_name, stereotype_name};
use crate::scpg::query::snapshot::ScpgSnapshot;
use crate::scpg::query::symbols::*;
use crate::scpg::serializer::integrity::digest_hex;
use crate::ssa::serializer::SSAArtifact;

pub const OHB_JSON_SCHEMA_ID: &str = "https://openheart.dev/schemas/ohb-json/1";
pub const OHB_JSON_VERSION: u32 = 1;

/// Optional inputs that are not part of a snapshot.
#[derive(Default)]
pub struct ExportInputs<'a> {
    pub cfg: Option<&'a CFGArtifact>,
    pub ssa: Option<&'a SSAArtifact>,
    /// `(phase name, wall-clock milliseconds)` from the pipeline run, when known.
    pub phase_timings_ms: Option<&'a [(String, f64)]>,
    /// Include every SDG node and edge (can be large).
    pub include_sdg: bool,
}

#[allow(unreachable_patterns)]
fn edge_kind_name(kind: CFGEdgeType) -> &'static str {
    match kind {
        CFGEdgeType::Uncond => "uncond",
        CFGEdgeType::True => "true",
        CFGEdgeType::False => "false",
        CFGEdgeType::Except => "except",
        CFGEdgeType::Return => "return",
        CFGEdgeType::Switch => "switch",
        CFGEdgeType::LoopBack => "loop_back",
        _ => "other",
    }
}

fn location_json(s: &ScpgSnapshot, sym: u32) -> Json {
    match s.navigation.location(sym, &s.files, s.scpg_hash) {
        Some(l) => json!({
            "file_id": l.file_id,
            "path": l.path,
            "line_start": l.line_start,
            "col_start": l.col_start,
            "line_end": l.line_end,
            "col_end": l.col_end,
            "first_token": l.first_token,
            "last_token": l.last_token,
        }),
        None => Json::Null,
    }
}

pub fn export_ohb_json(s: &ScpgSnapshot, inputs: &ExportInputs<'_>) -> Json {
    let syms = &s.symbols;
    let n = syms.len() as u32;

    let files: Vec<Json> = s
        .files
        .file_ids()
        .iter()
        .map(|&f| {
            json!({
                "file_id": f,
                "path": s.files.path(f),
                "language_id": s.files.language(f),
                "size_bytes": s.files.size(f),
                "sha256": s.files.content_hash(f).map(|h| digest_hex(&h)),
            })
        })
        .collect();

    let symbols: Vec<Json> = (0..n)
        .map(|i| {
            let modifiers: Vec<&str> = MODIFIER_NAMES
                .iter()
                .filter(|(flag, _)| syms.has_modifier(i, *flag))
                .map(|(_, name)| *name)
                .collect();
            json!({
                "id": i,
                "stable_id": syms.stable_id(i),
                "fqn": syms.qualified_name(i).filter(|q| !q.is_empty()),
                "name": syms.name(i),
                "kind": kind_name(syms.kind(i).unwrap_or(0)),
                "parent": syms.parent(i),
                "visibility": visibility_name(syms.visibility(i).unwrap_or(0)),
                "modifiers": modifiers,
                "param_count": syms.kind(i).filter(|&k| is_callable_kind(k)).map(|_| syms.param_count(i)),
                "location": location_json(s, i),
            })
        })
        .collect();

    let type_edges: Vec<Json> = s
        .hierarchy
        .edges()
        .map(|(f, t, r)| json!({ "from": f, "to": t, "relation": relation_name(r) }))
        .collect();

    let mut overrides = Vec::new();
    for i in 0..n {
        for o in s.hierarchy.overridden_methods(syms, i) {
            overrides.push(json!({ "method": i, "overridden": o }));
        }
    }

    let call_edges: Vec<Json> = s
        .call_graph
        .edges()
        .map(|(u, v, k)| json!({ "caller": u, "callee": v, "kind": call_type_name(k) }))
        .collect();

    let call_sites: Vec<Json> = s
        .call_graph
        .call_sites()
        .iter()
        .enumerate()
        .map(|(row, site)| {
            let position = s.tokens.start(site.call_token).map(|p| {
                json!({ "file_id": p.file_id, "path": s.files.path(p.file_id), "line": p.line, "col": p.col })
            });
            json!({
                "site": site.site_id,
                "caller": site.caller,
                "kind": call_type_name(site.call_type),
                "arg_count": site.arg_count,
                "polymorphic": site.flags & CALL_SITE_FLAG_IS_POLYMORPHIC != 0,
                "ast_node": site.call_node,
                "position": position,
                "targets": s.call_graph.site_targets(row),
            })
        })
        .collect();

    let mut components: std::collections::BTreeMap<u32, Vec<u32>> =
        std::collections::BTreeMap::new();
    for m in 0..s.call_graph.node_count() as u32 {
        if s.call_graph.is_recursive(m) {
            if let Some(c) = s.call_graph.component_of(m) {
                components.entry(c).or_default().push(m);
            }
        }
    }
    let recursive_sccs: Vec<Json> = components
        .into_iter()
        .map(|(c, members)| json!({ "component": c, "members": members }))
        .collect();

    let points_to: Vec<Json> = s
        .call_graph
        .points_to()
        .iter()
        .map(|&(ssa_id, alloc)| json!({ "ssa_id": ssa_id, "alloc_type": alloc }))
        .collect();

    let path_summaries: Vec<Json> = s
        .paths
        .functions()
        .iter()
        .map(|f| serde_json::to_value(f).unwrap_or(Json::Null))
        .collect();

    let uml_classes: Vec<Json> = s
        .semantic
        .classes
        .iter()
        .map(|c| {
            json!({
                "class": c.sym_id,
                "stereotype": stereotype_name(c.stereotype),
                "extends": if c.extends_sym == u32::MAX { None } else { Some(c.extends_sym) },
                "field_count": c.field_count,
                "method_count": c.method_count,
                "has_state_machine": s.semantic.has_state_machine(c.sym_id),
            })
        })
        .collect();
    let patterns: Vec<Json> = s
        .semantic
        .patterns
        .iter()
        .map(|p| {
            json!({
                "instance": p.instance_id,
                "class": p.class_sym,
                "pattern": pattern_name(p.pattern_kind),
                "role": p.role,
                "confidence": p.confidence,
            })
        })
        .collect();

    let (cfg_json, cfg_note) = match inputs.cfg {
        Some(cfa) => {
            let methods: Vec<Json> = cfa
                .functions
                .iter()
                .map(|f| {
                    json!({
                        "method": f.sym_id,
                        "cyclomatic": f.cyclomatic,
                        "blocks": f.blocks.iter().map(|b| json!({
                            "id": b.id,
                            "stmts": b.stmts,
                            "first_token": b.first_token,
                            "last_token": b.last_token,
                            "entry": b.is_entry,
                            "exit": b.is_exit,
                        })).collect::<Vec<_>>(),
                        "edges": f.edges.iter().map(|&(a, b, k)| json!({
                            "from": a, "to": b, "kind": edge_kind_name(k)
                        })).collect::<Vec<_>>(),
                        "idom": f.idom.iter().map(|&d| if d == u32::MAX { None } else { Some(d) }).collect::<Vec<_>>(),
                    })
                })
                .collect();
            (Json::Array(methods), Json::Null)
        }
        None => (
            Json::Null,
            json!("no CFG artifact was supplied to the exporter"),
        ),
    };

    let (phi_json, phi_note) = match inputs.ssa {
        Some(ssa) => {
            let methods: Vec<Json> = ssa
                .functions
                .iter()
                .map(|f| {
                    json!({
                        "method": f.sym_id,
                        "phi": f.phi_records.iter().map(|p| json!({
                            "ssa_id": p.ssa_id,
                            "block": p.block_id,
                            "variable": p.orig_sym_id,
                            "args": p.args.iter().map(|a| json!({
                                "pred_block": a.pred_block_id,
                                "ssa_id": if a.arg_ssa_id == u32::MAX { None } else { Some(a.arg_ssa_id) },
                            })).collect::<Vec<_>>(),
                        })).collect::<Vec<_>>(),
                    })
                })
                .collect();
            (Json::Array(methods), Json::Null)
        }
        None => (
            Json::Null,
            json!("no SSA artifact was supplied to the exporter"),
        ),
    };

    let (timings, timings_note) = match inputs.phase_timings_ms {
        Some(t) => (
            Json::Array(
                t.iter()
                    .map(|(name, ms)| json!({ "phase": name, "ms": ms }))
                    .collect(),
            ),
            Json::Null,
        ),
        None => (
            Json::Null,
            json!("phase timings are not stored in the snapshot and were not supplied"),
        ),
    };

    let sdg = if inputs.include_sdg {
        let nodes: Vec<Json> = (0..s.sdg.node_count() as u32)
            .filter_map(|id| s.sdg.node(id).map(|nd| (id, nd)))
            .map(|(id, nd)| {
                json!({
                    "id": id,
                    "method": nd.method,
                    "kind": crate::scpg::query::sdg::node_kind_name(nd.kind),
                    "ast_node": if nd.ast_node == u32::MAX { None } else { Some(nd.ast_node) },
                })
            })
            .collect();
        let mut edges = Vec::new();
        for to in 0..s.sdg.node_count() as u32 {
            let (froms, kinds) = s.sdg.preds(to);
            for (&from, &k) in froms.iter().zip(kinds) {
                edges.push(json!({ "to": to, "from": from, "kind": crate::scpg::query::sdg::dependence_name(k) }));
            }
        }
        json!({ "nodes": nodes, "edges": edges })
    } else {
        json!({ "node_count": s.sdg.node_count(), "edge_count": s.sdg.edge_count() })
    };

    json!({
        "$schema": OHB_JSON_SCHEMA_ID,
        "version": OHB_JSON_VERSION,
        "scpg": {
            "digest": digest_hex(&s.digest),
            "scpg_hash": format!("0x{:08X}", s.scpg_hash),
            "source_hash": digest_hex(&s.source_hash),
            "stats": serde_json::to_value(s.stats()).unwrap_or(Json::Null),
        },
        "files": files,
        "symbols": symbols,
        "type_edges": type_edges,
        "overrides": overrides,
        "call_edges": call_edges,
        "call_sites": call_sites,
        "recursive_sccs": recursive_sccs,
        "points_to": points_to,
        "dependence_graph": sdg,
        "path_summaries": path_summaries,
        "uml": { "classes": uml_classes, "pattern_instances": patterns },
        "cfg": cfg_json,
        "cfg_unavailable": cfg_note,
        "phi": phi_json,
        "phi_unavailable": phi_note,
        "phase_timings": timings,
        "phase_timings_unavailable": timings_note,
    })
}

/// JSON Schema (draft 2020-12) of the `ohb-json` export.
pub fn ohb_json_schema() -> Json {
    let nullable = |t: &str| json!({ "type": [t, "null"] });
    let location = json!({
        "type": ["object", "null"],
        "required": ["file_id", "line_start", "col_start", "line_end", "col_end"],
        "properties": {
            "file_id": { "type": "integer" },
            "path": { "type": ["string", "null"] },
            "line_start": { "type": "integer", "minimum": 1 },
            "col_start": { "type": "integer", "minimum": 0 },
            "line_end": { "type": "integer", "minimum": 1 },
            "col_end": { "type": "integer", "minimum": 0 },
            "first_token": { "type": "integer" },
            "last_token": { "type": "integer" }
        }
    });
    let id_array = json!({ "type": "array", "items": { "type": "integer" } });
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": OHB_JSON_SCHEMA_ID,
        "title": "OpenHeart ohb-json export",
        "type": "object",
        "required": ["version", "scpg", "files", "symbols", "type_edges", "call_edges", "call_sites"],
        "properties": {
            "version": { "const": OHB_JSON_VERSION },
            "scpg": {
                "type": "object",
                "required": ["digest", "scpg_hash", "source_hash"],
                "properties": {
                    "digest": { "type": "string", "pattern": "^[0-9a-f]{64}$" },
                    "scpg_hash": { "type": "string", "pattern": "^0x[0-9A-F]{8}$" },
                    "source_hash": { "type": "string", "pattern": "^[0-9a-f]{64}$" },
                    "stats": { "type": ["object", "null"] }
                }
            },
            "files": { "type": "array", "items": {
                "type": "object",
                "required": ["file_id"],
                "properties": {
                    "file_id": { "type": "integer" },
                    "path": nullable("string"),
                    "language_id": nullable("integer"),
                    "size_bytes": nullable("integer"),
                    "sha256": nullable("string")
                }
            }},
            "symbols": { "type": "array", "items": {
                "type": "object",
                "required": ["id", "stable_id", "kind", "name"],
                "properties": {
                    "id": { "type": "integer" },
                    "stable_id": { "type": "string" },
                    "fqn": nullable("string"),
                    "name": { "type": "string" },
                    "kind": { "enum": KIND_NAME_LIST },
                    "parent": nullable("integer"),
                    "visibility": { "enum": ["public", "private", "protected", "package"] },
                    "modifiers": { "type": "array", "items": { "type": "string" } },
                    "param_count": nullable("integer"),
                    "location": location
                }
            }},
            "type_edges": { "type": "array", "items": {
                "type": "object",
                "required": ["from", "to", "relation"],
                "properties": {
                    "from": { "type": "integer" },
                    "to": { "type": "integer" },
                    "relation": { "enum": ["extends", "implements", "uses", "creates", "association"] }
                }
            }},
            "overrides": { "type": "array", "items": {
                "type": "object", "required": ["method", "overridden"],
                "properties": { "method": { "type": "integer" }, "overridden": { "type": "integer" } }
            }},
            "call_edges": { "type": "array", "items": {
                "type": "object", "required": ["caller", "callee", "kind"],
                "properties": { "caller": { "type": "integer" }, "callee": { "type": "integer" }, "kind": { "type": "string" } }
            }},
            "call_sites": { "type": "array", "items": {
                "type": "object", "required": ["site", "caller", "kind", "targets"],
                "properties": {
                    "site": { "type": "integer" },
                    "caller": { "type": "integer" },
                    "kind": { "type": "string" },
                    "arg_count": { "type": "integer" },
                    "polymorphic": { "type": "boolean" },
                    "ast_node": { "type": "integer" },
                    "position": { "type": ["object", "null"] },
                    "targets": id_array
                }
            }},
            "recursive_sccs": { "type": "array", "items": {
                "type": "object", "properties": { "component": { "type": "integer" }, "members": id_array }
            }},
            "points_to": { "type": "array" },
            "dependence_graph": { "type": "object" },
            "path_summaries": { "type": "array", "items": {
                "type": "object",
                "required": ["sym_id", "sat_count", "cyclomatic"],
                "properties": {
                    "sym_id": { "type": "integer" },
                    "n_vars": { "type": "integer" },
                    "n_nodes": { "type": "integer" },
                    "root": { "type": "integer" },
                    "sat_count": { "type": "integer" },
                    "cyclomatic": { "type": "integer" },
                    "max_path_len": { "type": "integer" },
                    "unwind_depth": { "type": "integer" }
                }
            }},
            "uml": { "type": "object", "properties": {
                "classes": { "type": "array" },
                "pattern_instances": { "type": "array" }
            }},
            "cfg": { "type": ["array", "null"] },
            "cfg_unavailable": nullable("string"),
            "phi": { "type": ["array", "null"] },
            "phi_unavailable": nullable("string"),
            "phase_timings": { "type": ["array", "null"] },
            "phase_timings_unavailable": nullable("string")
        }
    })
}

const KIND_NAME_LIST: [&str; 19] = [
    "package",
    "class",
    "interface",
    "enum",
    "record",
    "annotation",
    "method",
    "constructor",
    "field",
    "enum_constant",
    "param",
    "local",
    "type_param",
    "lambda",
    "anon_class",
    "static_init",
    "instance_init",
    "module",
    "external",
];
