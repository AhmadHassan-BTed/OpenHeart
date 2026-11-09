//! SCPG queries shared by `openheart query` and `POST /api/sessions/<id>/query`.
//!
//! Both entry points parse a [`QueryRequest`], open the unified `.scpg` binary through
//! [`open_engine`], and render the result with [`execute`] as JSON, so the CLI and the API
//! answer every query identically. Symbols are named by numeric id, stable id, qualified
//! name or simple name; a name matching several symbols is an error that lists nothing it
//! cannot back with the snapshot.

#![forbid(unsafe_code)]

use std::io;
use std::path::Path;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::scpg::query::{QueryEngine, QueryError, RELATIONS, STARTER_QUERIES};
use crate::scpg::{EngineBuilder, OpenHeartEngine};
use crate::scpg::api::VerifyPolicy;

/// Largest Datalog program accepted.
pub const MAX_PROGRAM_BYTES: usize = 64 * 1024;

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
}

/// A query that could not be answered, with the HTTP status that describes it.
#[derive(Debug, Clone)]
pub struct QueryFailure {
    /// 400 malformed query, 404 unknown symbol or location, 409 ambiguous name or no
    /// backing file, 422 unanswerable for this snapshot, 500 internal failure.
    pub status: u16,
    /// Human-readable reason.
    pub message: String,
}

impl QueryFailure {
    fn new(status: u16, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
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
    let spec = spec.trim();
    if spec.is_empty() {
        return Err(QueryFailure::new(400, "the symbol specification is empty"));
    }
    queries.resolve_symbol(spec).map_err(QueryFailure::from)
}

fn to_json<T: serde::Serialize>(value: &T) -> Result<Value, QueryFailure> {
    serde_json::to_value(value)
        .map_err(|err| QueryFailure::new(500, format!("result serialization failed: {}", err)))
}

/// Answers `request` from `engine` as a JSON object.
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
                    409,
                    "this engine is not backed by an .scpg file, so there is nothing to verify",
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
                "paths": paths.to_string(),
                "paths_log2": paths.log2(),
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
    };
    Ok(answer)
}
