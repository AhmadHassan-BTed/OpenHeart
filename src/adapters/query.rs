//! The engine query protocol (`docs/engine-query-protocol.md`) shared by
//! `openheart query <analysis-dir>`, `POST /api/sessions/<id>/query` and the WebAssembly
//! export `oh_query`.
//!
//! Every transport parses a [`QueryRequest`] with [`parse_request`] and answers it with
//! [`execute_session`] over one finished [`AnalysisSession`], so all of them answer every
//! query identically. The call-graph, dependency and focused-view operations (`functions`,
//! `callers`, `callees`, `dependencies`, `dependents`, `view`) come from [`crate::facts`]
//! and the focused diagram builders; the SCPG operations (`stats` through `datalog`) run on
//! the session's [`OpenHeartEngine`] through [`execute`]. A failure is a [`QueryFailure`],
//! sent as `{ "error": { "status": <status>, "message": "<reason>" } }`
//! ([`QueryFailure::to_json`]). Symbols are named by numeric id, stable id, qualified name
//! or simple name; a name matching several symbols is an error that lists nothing it
//! cannot back with the snapshot.

#![forbid(unsafe_code)]

use std::io;
use std::path::Path;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::facts::{self, FactsError};
use crate::scpg::api::VerifyPolicy;
use crate::scpg::diagram::views::focused;
use crate::scpg::query::{QueryEngine, QueryError, RELATIONS, STARTER_QUERIES};
use crate::scpg::{EngineBuilder, OpenHeartEngine};
use crate::session::AnalysisSession;

/// Largest Datalog program accepted.
pub const MAX_PROGRAM_BYTES: usize = 64 * 1024;

/// Functions listed by `functions` when the request gives no `limit` (protocol section 2.2).
pub const DEFAULT_FUNCTION_LIMIT: usize = 200;

/// Call-graph depth of `callers` and `callees` when the request gives no `depth`.
pub const DEFAULT_CALL_DEPTH: u32 = 1;

/// Smallest `depth` accepted by `callers` and `callees` (protocol section 2.2).
pub const MIN_CALL_DEPTH: u32 = 1;

/// Largest `depth` accepted by `callers` and `callees` (protocol section 2.2).
pub const MAX_CALL_DEPTH: u32 = 6;

/// Focus depth of `view` when the request names a symbol but gives no `depth`.
pub const DEFAULT_VIEW_DEPTH: u32 = 2;

/// Direction of a program slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SliceDirectionArg {
    /// Statements the criterion depends on.
    #[default]
    Backward,
    /// Statements that depend on the criterion.
    Forward,
}

/// One query against an SCPG snapshot.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "query", rename_all = "snake_case")]
pub enum QueryRequest {
    /// Snapshot statistics, hash and digest.
    Stats,
    /// Full verification of every section checksum and the content digest.
    Verify,
    /// The Datalog relation catalog and starter queries.
    Catalog,
    /// Resolves a symbol specification to its id.
    Resolve {
        /// Symbol id, stable id, qualified name or simple name.
        symbol: String,
    },
    /// Whether `to` is reachable from `from` through zero or more calls.
    Reachable {
        /// Calling method.
        from: String,
        /// Called method.
        to: String,
    },
    /// One call path from `from` to `to`.
    CallPath {
        /// Calling method.
        from: String,
        /// Called method.
        to: String,
    },
    /// Program slice of a method.
    Slice {
        /// Slicing criterion (a method).
        symbol: String,
        /// Slice direction.
        #[serde(default)]
        direction: SliceDirectionArg,
        /// Maximum dependence depth (`None`: unbounded).
        #[serde(default)]
        depth: Option<u32>,
    },
    /// Entities impacted by a change to a symbol.
    Impact {
        /// The changed symbol.
        symbol: String,
    },
    /// Source location of a symbol.
    Source {
        /// The symbol.
        symbol: String,
    },
    /// Entities whose source span contains a position.
    At {
        /// Corpus file id.
        file_id: u16,
        /// Line (1-based).
        line: u32,
        /// Column (0-based).
        column: u16,
    },
    /// Path summary and exact path count of a function.
    PathSummary {
        /// The function.
        symbol: String,
    },
    /// Evaluates an OpenHeart Datalog program.
    Datalog {
        /// Program text.
        program: String,
    },
    /// Lists or searches functions and methods.
    Functions {
        /// Case-insensitive substring a function's name must contain (`None`: every
        /// function).
        #[serde(default)]
        filter: Option<String>,
        /// Largest number of functions returned (`None`: [`DEFAULT_FUNCTION_LIMIT`]).
        #[serde(default)]
        limit: Option<usize>,
    },
    /// The functions that call a function, transitively up to `depth`.
    Callers {
        /// The called function.
        symbol: String,
        /// Call-graph depth, [`MIN_CALL_DEPTH`] to [`MAX_CALL_DEPTH`] (`None`:
        /// [`DEFAULT_CALL_DEPTH`]).
        #[serde(default)]
        depth: Option<u32>,
    },
    /// The functions a function calls, transitively up to `depth`.
    Callees {
        /// The calling function.
        symbol: String,
        /// Call-graph depth, [`MIN_CALL_DEPTH`] to [`MAX_CALL_DEPTH`] (`None`:
        /// [`DEFAULT_CALL_DEPTH`]).
        #[serde(default)]
        depth: Option<u32>,
    },
    /// Everything a function or type depends on: calls, field reads and writes, types and
    /// packages.
    Dependencies {
        /// The function or type.
        symbol: String,
    },
    /// Everything that depends on a function, field or type.
    Dependents {
        /// The function, field or type.
        symbol: String,
    },
    /// A diagram as Graph IR: the whole program, or focused on one element.
    View {
        /// Diagram id.
        view: String,
        /// Element to focus on (`None`: the whole-program diagram).
        #[serde(default)]
        symbol: Option<String>,
        /// Focus depth (`None`: [`DEFAULT_VIEW_DEPTH`]).
        #[serde(default)]
        depth: Option<u32>,
    },
}

impl QueryRequest {
    /// The operation name, as sent in the request's `query` field.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Stats => "stats",
            Self::Verify => "verify",
            Self::Catalog => "catalog",
            Self::Resolve { .. } => "resolve",
            Self::Reachable { .. } => "reachable",
            Self::CallPath { .. } => "call_path",
            Self::Slice { .. } => "slice",
            Self::Impact { .. } => "impact",
            Self::Source { .. } => "source",
            Self::At { .. } => "at",
            Self::PathSummary { .. } => "path_summary",
            Self::Datalog { .. } => "datalog",
            Self::Functions { .. } => "functions",
            Self::Callers { .. } => "callers",
            Self::Callees { .. } => "callees",
            Self::Dependencies { .. } => "dependencies",
            Self::Dependents { .. } => "dependents",
            Self::View { .. } => "view",
        }
    }
}

/// Parses a request document (protocol section 2). A body that is not a request object
/// with a known `query` operation is a 400 failure carrying the parser's reason.
pub fn parse_request(bytes: &[u8]) -> Result<QueryRequest, QueryFailure> {
    serde_json::from_slice(bytes)
        .map_err(|err| QueryFailure::new(400, format!("invalid query request: {}", err)))
}

/// A query that could not be answered, with the HTTP status that describes it.
#[derive(Debug, Clone)]
pub struct QueryFailure {
    /// 400 malformed query, 404 unknown symbol, location or analysis, 409 ambiguous name,
    /// 422 not derivable for this element or snapshot, 500 internal failure.
    pub status: u16,
    /// Human-readable reason.
    pub message: String,
}

impl QueryFailure {
    /// A failure with `status` and the reason `message`.
    pub fn new(status: u16, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    /// The protocol's failure document: `{ "error": { "status": .., "message": .. } }`.
    pub fn to_json(&self) -> Value {
        json!({ "error": { "status": self.status, "message": self.message } })
    }
}

impl From<FactsError> for QueryFailure {
    fn from(err: FactsError) -> Self {
        Self {
            status: err.status,
            message: err.message,
        }
    }
}

/// Status of an analysis directory that could not be opened for queries: 404 when it, or
/// a file it must hold, does not exist; 422 when its contents are not a readable analysis;
/// 500 for any other I/O failure.
pub fn status_for_io(err: &io::Error) -> u16 {
    match err.kind() {
        io::ErrorKind::NotFound => 404,
        io::ErrorKind::InvalidData | io::ErrorKind::UnexpectedEof => 422,
        _ => 500,
    }
}

impl From<QueryError> for QueryFailure {
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

/// Opens `path` for queries. The file is read into memory rather than mapped, so the
/// directory holding it can be removed while the engine is alive; `verify_fully` checks
/// every section checksum and the content digest before the first query.
pub fn open_engine(path: &Path, verify_fully: bool) -> io::Result<OpenHeartEngine> {
    let policy = if verify_fully {
        VerifyPolicy::Full
    } else {
        VerifyPolicy::Structure
    };
    EngineBuilder::new()
        .with_mmap(false)
        .with_verification(policy)
        .open(path)
}

fn resolve(queries: &QueryEngine, spec: &str) -> Result<u32, QueryFailure> {
    queries
        .resolve_symbol(symbol_spec(spec)?)
        .map_err(QueryFailure::from)
}

fn to_json<T: serde::Serialize>(value: &T) -> Result<Value, QueryFailure> {
    serde_json::to_value(value)
        .map_err(|err| QueryFailure::new(500, format!("result serialization failed: {}", err)))
}

fn symbol_spec(spec: &str) -> Result<&str, QueryFailure> {
    let spec = spec.trim();
    if spec.is_empty() {
        return Err(QueryFailure::new(400, "the symbol specification is empty"));
    }
    Ok(spec)
}

fn call_depth(depth: Option<u32>) -> Result<u32, QueryFailure> {
    let depth = depth.unwrap_or(DEFAULT_CALL_DEPTH);
    if (MIN_CALL_DEPTH..=MAX_CALL_DEPTH).contains(&depth) {
        Ok(depth)
    } else {
        Err(QueryFailure::new(
            400,
            format!(
                "'depth' must be between {} and {}, got {}",
                MIN_CALL_DEPTH, MAX_CALL_DEPTH, depth
            ),
        ))
    }
}

/// Answers `request` from one finished analysis as a JSON value (protocol section 2).
///
/// `functions`, `callers`, `callees`, `dependencies` and `dependents` come from
/// [`crate::facts`], `view` from the focused diagram builders, and every other operation
/// from the session's engine through [`execute`]. `verify` needs an engine opened from the
/// `.scpg` file; a session whose engine the pipeline built in memory answers it with 422.
pub fn execute_session(s: &AnalysisSession, request: &QueryRequest) -> Result<Value, QueryFailure> {
    let answer = match request {
        QueryRequest::Functions { filter, limit } => {
            let limit = limit.unwrap_or(DEFAULT_FUNCTION_LIMIT);
            if limit == 0 {
                return Err(QueryFailure::new(400, "'limit' must be at least 1"));
            }
            facts::functions(s, filter.as_deref(), limit)?
        }
        QueryRequest::Callers { symbol, depth } => {
            facts::callers(s, symbol_spec(symbol)?, call_depth(*depth)?)?
        }
        QueryRequest::Callees { symbol, depth } => {
            facts::callees(s, symbol_spec(symbol)?, call_depth(*depth)?)?
        }
        QueryRequest::Dependencies { symbol } => facts::dependencies(s, symbol_spec(symbol)?)?,
        QueryRequest::Dependents { symbol } => facts::dependents(s, symbol_spec(symbol)?)?,
        QueryRequest::View {
            view,
            symbol,
            depth,
        } => {
            let view = view.trim();
            if view.is_empty() {
                return Err(QueryFailure::new(400, "the 'view' diagram id is empty"));
            }
            let symbol = symbol.as_deref().map(symbol_spec).transpose()?;
            focused::view(s, view, symbol, depth.unwrap_or(DEFAULT_VIEW_DEPTH))?
        }
        _ => return execute(&s.artifacts.engine, request),
    };
    Ok(answer)
}

/// Answers `request` from `engine` as a JSON object. The session operations (`functions`,
/// `callers`, `callees`, `dependencies`, `dependents`, `view`) need a whole analysis and
/// fail here with 422; [`execute_session`] answers them.
pub fn execute(engine: &OpenHeartEngine, request: &QueryRequest) -> Result<Value, QueryFailure> {
    let queries = engine.queries();
    let answer = match request {
        QueryRequest::Stats => json!({
            "query": "stats",
            "scpg_hash": format!("0x{:08X}", engine.scpg_hash()),
            "scpg_digest": engine.digest_hex(),
            "stats": to_json(&engine.stats())?,
        }),
        QueryRequest::Verify => match engine.verify() {
            None => {
                return Err(QueryFailure::new(
                    422,
                    "this engine was built in memory and is not backed by an .scpg file, so \
                     there is nothing to verify; verification needs an engine opened from the \
                     analysis' unified.scpg",
                ))
            }
            Some(Err(err)) => {
                return Err(QueryFailure::new(
                    422,
                    format!("the .scpg file failed verification: {}", err),
                ))
            }
            Some(Ok(report)) => json!({
                "query": "verify",
                "verified": true,
                "report": to_json(&report)?,
            }),
        },
        QueryRequest::Catalog => {
            let relations: Vec<Value> = RELATIONS
                .iter()
                .map(|relation| {
                    json!({
                        "name": relation.name,
                        "columns": relation.columns,
                        "description": relation.description,
                    })
                })
                .collect();
            let starters: Vec<Value> = STARTER_QUERIES
                .iter()
                .map(|(name, description, program)| {
                    json!({ "name": name, "description": description, "program": program })
                })
                .collect();
            json!({ "query": "catalog", "relations": relations, "starter_queries": starters })
        }
        QueryRequest::Resolve { symbol } => {
            let sym_id = resolve(&queries, symbol)?;
            json!({ "query": "resolve", "symbol": symbol, "sym_id": sym_id })
        }
        QueryRequest::Reachable { from, to } => {
            let source = resolve(&queries, from)?;
            let target = resolve(&queries, to)?;
            let reachable = queries.is_reachable(source, target)?;
            json!({
                "query": "reachable",
                "from": source,
                "to": target,
                "reachable": reachable,
            })
        }
        QueryRequest::CallPath { from, to } => {
            let source = resolve(&queries, from)?;
            let target = resolve(&queries, to)?;
            let path = queries.call_path(source, target)?;
            json!({ "query": "call_path", "from": source, "to": target, "path": path })
        }
        QueryRequest::Slice {
            symbol,
            direction,
            depth,
        } => {
            let method = resolve(&queries, symbol)?;
            let slice = match direction {
                SliceDirectionArg::Backward => queries.backward_slice(method, *depth)?,
                SliceDirectionArg::Forward => queries.forward_slice(method, *depth)?,
            };
            json!({ "query": "slice", "sym_id": method, "slice": to_json(slice.as_ref())? })
        }
        QueryRequest::Impact { symbol } => {
            let sym_id = resolve(&queries, symbol)?;
            let impacted = engine.impact(sym_id)?;
            json!({
                "query": "impact",
                "sym_id": sym_id,
                "impacted": to_json(impacted.as_ref())?,
            })
        }
        QueryRequest::Source { symbol } => {
            let sym_id = resolve(&queries, symbol)?;
            match engine.to_source(sym_id) {
                Some(location) => json!({
                    "query": "source",
                    "sym_id": sym_id,
                    "location": to_json(&location)?,
                }),
                None => {
                    return Err(QueryFailure::new(
                        404,
                        format!("symbol {} has no recorded source location", sym_id),
                    ))
                }
            }
        }
        QueryRequest::At {
            file_id,
            line,
            column,
        } => {
            let entities = engine.entities_at(*file_id, *line, *column);
            json!({
                "query": "at",
                "file_id": file_id,
                "line": line,
                "column": column,
                "entities": to_json(&entities)?,
            })
        }
        QueryRequest::PathSummary { symbol } => {
            let sym_id = resolve(&queries, symbol)?;
            let Some(summary) = queries.path_summary(sym_id) else {
                return Err(QueryFailure::new(
                    404,
                    format!("symbol {} has no path summary (it has no CFG body)", sym_id),
                ));
            };
            let paths = queries.count_paths(sym_id, &[])?;
            json!({
                "query": "path_summary",
                "sym_id": sym_id,
                "summary": to_json(&summary)?,
                // An exact count as a decimal string (it may exceed 2^53); a saturated count
                // is a lower bound.
                "paths": paths.value.to_string(),
                "paths_saturated": paths.saturated,
                "paths_log2": if paths.value == 0 { 0.0 } else { (paths.value as f64).log2() },
            })
        }
        QueryRequest::Datalog { program } => {
            if program.trim().is_empty() {
                return Err(QueryFailure::new(400, "the Datalog program is empty"));
            }
            if program.len() > MAX_PROGRAM_BYTES {
                return Err(QueryFailure::new(
                    400,
                    format!(
                        "the Datalog program exceeds the limit of {} bytes",
                        MAX_PROGRAM_BYTES
                    ),
                ));
            }
            let output = engine.run_query(program)?;
            json!({ "query": "datalog", "output": to_json(output.as_ref())? })
        }
        QueryRequest::Functions { .. }
        | QueryRequest::Callers { .. }
        | QueryRequest::Callees { .. }
        | QueryRequest::Dependencies { .. }
        | QueryRequest::Dependents { .. }
        | QueryRequest::View { .. } => {
            return Err(QueryFailure::new(
                422,
                format!(
                    "the '{}' query is answered from a whole analysis session (its artifacts, \
                     sources and project files); an .scpg engine alone does not hold what it \
                     needs",
                    request.name()
                ),
            ))
        }
    };
    Ok(answer)
}
