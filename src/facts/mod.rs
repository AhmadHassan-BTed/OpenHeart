//! Per-function and per-symbol facts for the query protocol
//! (docs/engine-query-protocol.md, section 2.2): `functions`, `callers`, `callees`,
//! `dependencies` and `dependents`, answered from one [`AnalysisSession`].
//!
//! Everything here is read from the analysis, never guessed:
//! - functions, names, kinds and source locations come from the Phase 10 snapshot's symbol
//!   and navigation indexes;
//! - call edges come from the Phase 6 call graph, with each site's location from the token
//!   corpus and its dispatch and resolution kinds from the call graph's edge-kind table;
//! - dependents come from the engine's impact analysis (callers, overriding and overridden
//!   methods, subtypes, type references) at depth 1.
//!
//! Field reads and writes and per-function type uses are not indexed per function yet. Those
//! lists are returned empty, with a note saying so, rather than approximated.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;

use serde_json::{json, Value};

use crate::core::types::cg::{
    dispatch_name, dispatch_of_edge_type, resolution_name, unresolved_reason_text,
};
use crate::core::types::token::unpack_sort_key;
use crate::scpg::query::symbols::{is_callable_kind, kind_name};
use crate::scpg::query::{ImpactReason, QueryError};
use crate::session::AnalysisSession;

/// Deepest call-graph neighbourhood a `callers` / `callees` request may ask for.
pub const MAX_CALL_DEPTH: u32 = 6;

/// A request the facts could not answer, with the protocol status that describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactsError {
    pub status: u16,
    pub message: String,
}

impl FactsError {
    pub fn new(status: u16, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
}

impl fmt::Display for FactsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.message, self.status)
    }
}

impl std::error::Error for FactsError {}

impl From<QueryError> for FactsError {
    fn from(err: QueryError) -> Self {
        let status = match &err {
            QueryError::UnknownSymbol(_) | QueryError::UnresolvedSymbol(_) => 404,
            QueryError::AmbiguousSymbol { .. } => 409,
            QueryError::Language(_) => 400,
            _ => 422,
        };
        Self::new(status, err.to_string())
    }
}

/// Resolves a symbol spec (numeric id, stable id, qualified name, then simple name).
pub fn resolve_symbol(s: &AnalysisSession, spec: &str) -> Result<u32, FactsError> {
    let spec = spec.trim();
    if spec.is_empty() {
        return Err(FactsError::new(400, "the symbol is empty"));
    }
    Ok(s.artifacts.engine.queries().resolve_symbol(spec)?)
}

/// `FunctionRef` of the protocol for one symbol.
fn function_ref(s: &AnalysisSession, sym: u32) -> Value {
    let snapshot = s.artifacts.engine.snapshot();
    let symbols = &snapshot.symbols;
    let graph = &snapshot.call_graph;
    let owner = symbols
        .parent(sym)
        .and_then(|p| symbols.qualified_name(p).or_else(|| symbols.name(p)))
        .map(str::to_string);
    let location = s.artifacts.engine.to_source(sym);
    let kind = symbols.kind(sym).map(kind_name).unwrap_or("unknown");
    json!({
        "symbol": symbol_spec_of(s, sym),
        "sym_id": sym,
        "name": symbols.name(sym),
        "qualified_name": symbols.qualified_name(sym),
        "kind": kind,
        "owner": owner,
        "file": location.as_ref().and_then(|l| l.path.clone()),
        "line": location.as_ref().map(|l| l.line_start),
        "callers": if graph.contains(sym) { graph.callers(sym).len() } else { 0 },
        "callees": if graph.contains(sym) { graph.callees(sym).len() } else { 0 },
    })
}

/// The stable id of a symbol (the protocol's symbol spec), or its numeric id as text.
fn symbol_spec_of(s: &AnalysisSession, sym: u32) -> String {
    s.artifacts
        .engine
        .snapshot()
        .symbols
        .stable_id(sym)
        .map(str::to_string)
        .unwrap_or_else(|| sym.to_string())
}

/// Functions and methods, optionally filtered by a case-insensitive substring of the
/// qualified or simple name, in symbol-id order.
pub fn functions(
    s: &AnalysisSession,
    filter: Option<&str>,
    limit: usize,
) -> Result<Value, FactsError> {
    let symbols = &s.artifacts.engine.snapshot().symbols;
    let needle = filter
        .map(|f| f.trim().to_lowercase())
        .filter(|f| !f.is_empty());
    let mut out = Vec::new();
    let mut truncated = false;
    for sym in 0..symbols.len() as u32 {
        let Some(kind) = symbols.kind(sym) else {
            continue;
        };
        if !is_callable_kind(kind) {
            continue;
        }
        if let Some(needle) = &needle {
            let name = symbols.name(sym).unwrap_or("").to_lowercase();
            let qualified = symbols.qualified_name(sym).unwrap_or("").to_lowercase();
            if !name.contains(needle.as_str()) && !qualified.contains(needle.as_str()) {
                continue;
            }
        }
        if out.len() >= limit {
            truncated = true;
            break;
        }
        out.push(function_ref(s, sym));
    }
    Ok(json!({ "functions": out, "truncated": truncated }))
}

/// Callers of a function up to `depth` call levels.
pub fn callers(s: &AnalysisSession, symbol: &str, depth: u32) -> Result<Value, FactsError> {
    neighbourhood(s, symbol, depth, false)
}

/// Callees of a function up to `depth` call levels.
pub fn callees(s: &AnalysisSession, symbol: &str, depth: u32) -> Result<Value, FactsError> {
    neighbourhood(s, symbol, depth, true)
}

/// Breadth-first call-graph neighbourhood, deterministic (symbol-id order at each level).
fn neighbourhood(
    s: &AnalysisSession,
    symbol: &str,
    depth: u32,
    forward: bool,
) -> Result<Value, FactsError> {
    if depth == 0 || depth > MAX_CALL_DEPTH {
        return Err(FactsError::new(
            400,
            format!("'depth' must be between 1 and {}", MAX_CALL_DEPTH),
        ));
    }
    let root = resolve_symbol(s, symbol)?;
    let graph = &s.artifacts.engine.snapshot().call_graph;
    let sites = call_sites_by_pair(s);

    let mut seen: BTreeSet<u32> = BTreeSet::from([root]);
    let mut frontier = VecDeque::from([(root, 0u32)]);
    let mut edges = Vec::new();
    while let Some((node, level)) = frontier.pop_front() {
        if level >= depth || !graph.contains(node) {
            continue;
        }
        let mut next: Vec<u32> = if forward {
            graph.callees(node).to_vec()
        } else {
            graph.callers(node).to_vec()
        };
        next.sort_unstable();
        next.dedup();
        for other in next {
            let (caller, callee) = if forward {
                (node, other)
            } else {
                (other, node)
            };
            let pair_sites = sites.get(&(caller, callee)).cloned().unwrap_or_default();
            let (dispatch, resolution) = pair_sites
                .first()
                .map(|site| (site.dispatch, site.resolution))
                .unwrap_or((None, None));
            edges.push(json!({
                "caller": symbol_spec_of(s, caller),
                "callee": symbol_spec_of(s, callee),
                "sites": pair_sites.iter().map(|site| site.location.clone()).collect::<Vec<_>>(),
                "dispatch": dispatch,
                "resolution": resolution,
            }));
            if seen.insert(other) {
                frontier.push_back((other, level + 1));
            }
        }
    }

    let functions: BTreeMap<String, Value> = seen
        .iter()
        .map(|&sym| (symbol_spec_of(s, sym), function_ref(s, sym)))
        .collect();
    let unresolved = if forward {
        unresolved_sites_of(s, root)
    } else {
        Vec::new()
    };
    let mut notes = Vec::new();
    if !unresolved.is_empty() {
        notes.push(format!(
            "{} call site(s) in this function have no resolved target",
            unresolved.len()
        ));
    }
    Ok(json!({
        "root": symbol_spec_of(s, root),
        "functions": functions,
        "edges": edges,
        "unresolved_sites": unresolved,
        "notes": notes,
    }))
}

/// One call site of a (caller, callee) pair, with its location and kinds.
#[derive(Clone)]
struct PairSite {
    location: Value,
    dispatch: Option<&'static str>,
    resolution: Option<&'static str>,
}

/// Call sites per (caller, callee) pair, from the call graph's site table.
fn call_sites_by_pair(s: &AnalysisSession) -> BTreeMap<(u32, u32), Vec<PairSite>> {
    let cga = &s.artifacts.cga;
    let mut out: BTreeMap<(u32, u32), Vec<PairSite>> = BTreeMap::new();
    for (i, &(caller, callee, site_id)) in cga.site_to_edge_map.iter().enumerate() {
        let kind = cga.edge_kinds.get(i);
        let dispatch = match kind {
            Some(k) => dispatch_name(k.dispatch),
            None => cga
                .site(site_id)
                .and_then(|site| dispatch_of_edge_type(site.call_type))
                .and_then(dispatch_name),
        };
        let resolution = kind.and_then(|k| resolution_name(k.resolution));
        out.entry((caller, callee)).or_default().push(PairSite {
            location: site_location(s, site_id),
            dispatch,
            resolution,
        });
    }
    out
}

/// `{ file, line, column }` of a call site's token.
fn site_location(s: &AnalysisSession, site_id: u32) -> Value {
    let Some(site) = s.artifacts.cga.site(site_id) else {
        return Value::Null;
    };
    let Some(token) = s.artifacts.tca.token_records.get(site.call_token as usize) else {
        return Value::Null;
    };
    let (file_id, line, col) = unpack_sort_key(token.sort_key);
    json!({ "file": s.file_path(file_id), "line": line, "column": col })
}

/// Unresolved call sites inside one function, with their reasons.
fn unresolved_sites_of(s: &AnalysisSession, function: u32) -> Vec<Value> {
    let cga = &s.artifacts.cga;
    cga.unresolved_sites
        .iter()
        .filter(|u| cga.site(u.call_site_id).map(|site| site.caller_sym) == Some(function))
        .map(|u| {
            let mut location = site_location(s, u.call_site_id);
            if let Value::Object(map) = &mut location {
                map.insert("reason".into(), json!(unresolved_reason_text(u.reason)));
            }
            location
        })
        .collect()
}

/// What a function (or type) depends on.
pub fn dependencies(s: &AnalysisSession, symbol: &str) -> Result<Value, FactsError> {
    let root = resolve_symbol(s, symbol)?;
    let graph = &s.artifacts.engine.snapshot().call_graph;
    let symbols = &s.artifacts.engine.snapshot().symbols;
    let mut counts: BTreeMap<u32, usize> = BTreeMap::new();
    if graph.contains(root) {
        for &callee in graph.callees(root) {
            *counts.entry(callee).or_default() += 1;
        }
    }
    let calls: Vec<Value> = counts
        .iter()
        .map(|(&sym, &count)| json!({ "symbol": symbol_spec_of(s, sym), "qualified_name": symbols.qualified_name(sym), "count": count }))
        .collect();
    let mut packages: BTreeSet<String> = BTreeSet::new();
    for &sym in counts.keys() {
        if let Some(owner) = symbols
            .parent(sym)
            .and_then(|p| symbols.parent(p))
            .and_then(|pkg| symbols.qualified_name(pkg))
        {
            packages.insert(owner.to_string());
        }
    }
    Ok(json!({
        "root": symbol_spec_of(s, root),
        "calls": calls,
        "reads": [],
        "writes": [],
        "types": [],
        "packages": packages,
        "notes": ["field reads and writes and per-function type uses are not indexed per function yet, so 'reads', 'writes' and 'types' are not derived"],
    }))
}

/// Who depends on a function, field or type (impact analysis at depth 1).
pub fn dependents(s: &AnalysisSession, symbol: &str) -> Result<Value, FactsError> {
    let root = resolve_symbol(s, symbol)?;
    let impacted = s.artifacts.engine.impact(root)?;
    let symbols = &s.artifacts.engine.snapshot().symbols;
    let mut callers = Vec::new();
    let mut type_users = Vec::new();
    let mut subtypes = Vec::new();
    let mut overrides = Vec::new();
    let mut entries: Vec<_> = impacted.iter().filter(|e| e.depth == 1).collect();
    entries.sort_by_key(|e| e.sym_id);
    for entry in entries {
        let item = json!({ "symbol": symbol_spec_of(s, entry.sym_id), "qualified_name": symbols.qualified_name(entry.sym_id) });
        match entry.reason {
            ImpactReason::Caller => callers.push(item),
            ImpactReason::TypeReference => type_users.push(item),
            ImpactReason::Subtype => subtypes.push(item),
            ImpactReason::OverriddenMethod | ImpactReason::OverridingMethod => overrides.push(item),
            _ => {}
        }
    }
    Ok(json!({
        "root": symbol_spec_of(s, root),
        "callers": callers,
        "readers": [],
        "writers": [],
        "type_users": type_users,
        "subtypes": subtypes,
        "overrides": overrides,
        "notes": ["field readers and writers are not indexed per field yet, so 'readers' and 'writers' are not derived"],
    }))
}
