//! QueryEngine — central query coordinator over an immutable snapshot, with LRU caching (§10.4, §10.5).
//!
//! The engine owns an `Arc<ScpgSnapshot>`; every query reads the snapshot directly, so
//! callers never pass artifacts and the results always belong to the snapshot whose digest
//! keys the cache. All methods take `&self`: concurrent queries only contend on the cache
//! shard they touch, never on a global lock.

pub mod cache;
pub mod cfl;
pub mod csr;
pub mod graph;
pub mod impact;
pub mod lang;
pub mod navigation;
pub mod robdd;
pub mod sdg;
pub mod semantic;
pub mod slice;
pub mod snapshot;
pub mod source;
pub mod symbols;

pub use cache::LRUQueryCache;
pub use cfl::CFLReachability;
pub use graph::CallGraphIndex;
pub use impact::{ImpactAnalyzer, ImpactEntry, ImpactOptions, ImpactReason};
pub use lang::{EvalLimits, QueryLangError, QueryOutput, Value, RELATIONS, STARTER_QUERIES};
pub use navigation::{NavigationEngine, NavigationIndex, SourceLocation};
pub use robdd::{BddQueryError, EdgeConstraint, FunctionPathSummary, PathCount, PathSummaryIndex, ROBDDQueryEngine};
pub use sdg::SystemDependenceGraph;
pub use slice::{ProgramSlice, SliceCriterion, SliceDirection, SliceEngine, SliceMode, SliceOptions};
pub use snapshot::{ScpgSnapshot, SnapshotStats};
pub use symbols::{SymbolIndex, TypeHierarchyIndex};

use std::sync::Arc;

use crate::scpg::types::*;

/// Failure of a query against a snapshot.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum QueryError {
    /// The id names no symbol or method of the snapshot.
    #[error("unknown symbol {0}")]
    UnknownSymbol(u32),
    /// No symbol matches the reference.
    #[error("no symbol matches '{0}'")]
    UnresolvedSymbol(String),
    /// Several symbols match the reference.
    #[error("'{spec}' is ambiguous: {count} symbols match")]
    AmbiguousSymbol { spec: String, count: usize },
    /// The symbol exists but has no CFG body, so no dependence graph.
    #[error("symbol {0} has no dependence graph (it has no CFG body)")]
    NoBody(u32),
    /// The AST node is not a statement of the method.
    #[error("AST node {ast_node} is not a statement of method {method}")]
    UnknownStatement { method: u32, ast_node: u32 },
    /// The Datalog program failed to parse, check or evaluate.
    #[error(transparent)]
    Language(#[from] QueryLangError),
    /// A path-summary (ROBDD) query failed.
    #[error(transparent)]
    Bdd(#[from] BddQueryError),
}

fn depth_param(depth: Option<u32>) -> u64 {
    depth.map(|d| d as u64).unwrap_or(u64::MAX)
}

/// Query coordinator over one immutable snapshot with a sharded LRU cache.
pub struct QueryEngine {
    snapshot: Arc<ScpgSnapshot>,
    cache: LRUQueryCache,
    limits: EvalLimits,
}

impl QueryEngine {
    /// Engine with the default cache capacity.
    pub fn new(snapshot: Arc<ScpgSnapshot>) -> Self {
        Self::with_cache_capacity(snapshot, cache::DEFAULT_CACHE_CAPACITY)
    }

    /// Engine caching at most about `capacity` results.
    pub fn with_cache_capacity(snapshot: Arc<ScpgSnapshot>, capacity: usize) -> Self {
        Self {
            snapshot,
            cache: LRUQueryCache::new(capacity),
            limits: EvalLimits::default(),
        }
    }

    /// Overrides the Datalog evaluation limits.
    pub fn with_limits(mut self, limits: EvalLimits) -> Self {
        self.limits = limits;
        self
    }

    /// The snapshot queries run against.
    pub fn snapshot(&self) -> &Arc<ScpgSnapshot> {
        &self.snapshot
    }

    /// The result cache.
    pub fn cache(&self) -> &LRUQueryCache {
        &self.cache
    }

    fn key(&self, kind: QueryKind, params: QueryParams) -> QueryKey {
        QueryKey {
            kind,
            params,
            digest: self.snapshot.digest,
        }
    }

    fn known_symbol(&self, sym_id: u32) -> Result<(), QueryError> {
        if self.snapshot.symbols.contains(sym_id) || self.snapshot.call_graph.contains(sym_id) {
            Ok(())
        } else {
            Err(QueryError::UnknownSymbol(sym_id))
        }
    }

    /// Resolves a numeric id, stable id, qualified name or simple name to exactly one symbol.
    pub fn resolve_symbol(&self, spec: &str) -> Result<u32, QueryError> {
        let matches = self.snapshot.symbols.resolve(spec);
        match matches.len() {
            0 => Err(QueryError::UnresolvedSymbol(spec.to_string())),
            1 => Ok(matches[0]),
            count => Err(QueryError::AmbiguousSymbol {
                spec: spec.to_string(),
                count,
            }),
        }
    }

    /// Method reachability through zero or more calls (index lookup, not cached).
    pub fn is_reachable(&self, source: u32, target: u32) -> Result<bool, QueryError> {
        self.known_symbol(source)?;
        self.known_symbol(target)?;
        Ok(self.snapshot.call_graph.is_reachable(source, target).unwrap_or(false))
    }

    /// A shortest call chain from `source` to `target`, if any.
    pub fn call_path(&self, source: u32, target: u32) -> Result<Option<Vec<u32>>, QueryError> {
        self.known_symbol(source)?;
        self.known_symbol(target)?;
        Ok(self.snapshot.call_graph.call_path(source, target))
    }

    /// Statement-level two-phase slice (cached).
    pub fn slice(&self, criterion: SliceCriterion, options: SliceOptions) -> Result<Arc<ProgramSlice>, QueryError> {
        let (method, ast) = match criterion {
            SliceCriterion::Statement { method, ast_node } => (method, ast_node as u64),
            SliceCriterion::Method(method) => (method, u64::MAX),
        };
        let params = QueryParams::Ids(vec![
            method as u64,
            ast,
            options.direction as u64,
            options.mode as u64,
            depth_param(options.depth_limit),
        ]);
        let key = self.key(QueryKind::Slice, params);
        if let Some(hit) = self.cache.get(&key) {
            if let QueryResult::Slice(s) = hit.as_ref() {
                return Ok(Arc::clone(s));
            }
        }
        let result = match SliceEngine::slice(&self.snapshot.sdg, criterion, options) {
            Some(s) => Arc::new(s),
            None => {
                return Err(match criterion {
                    SliceCriterion::Statement { method, ast_node } if self.snapshot.sdg.has_method(method) => {
                        QueryError::UnknownStatement { method, ast_node }
                    }
                    _ => {
                        self.known_symbol(method)?;
                        QueryError::NoBody(method)
                    }
                })
            }
        };
        self.cache.put(key, Arc::new(QueryResult::Slice(Arc::clone(&result))));
        Ok(result)
    }

    /// Backward slice of every statement of `method`.
    pub fn backward_slice(&self, method: u32, depth_limit: Option<u32>) -> Result<Arc<ProgramSlice>, QueryError> {
        self.slice(
            SliceCriterion::Method(method),
            SliceOptions {
                direction: SliceDirection::Backward,
                mode: SliceMode::Full,
                depth_limit,
            },
        )
    }

    /// Forward slice of every statement of `method`.
    pub fn forward_slice(&self, method: u32, depth_limit: Option<u32>) -> Result<Arc<ProgramSlice>, QueryError> {
        self.slice(
            SliceCriterion::Method(method),
            SliceOptions {
                direction: SliceDirection::Forward,
                mode: SliceMode::Full,
                depth_limit,
            },
        )
    }

    /// Realizable-path dependence reachability between two statements.
    pub fn flows_to(&self, from: (u32, u32), to: (u32, u32), mode: SliceMode) -> Result<bool, QueryError> {
        for (method, ast_node) in [from, to] {
            if self.snapshot.sdg.statement(method, ast_node).is_none() {
                return Err(QueryError::UnknownStatement { method, ast_node });
            }
        }
        Ok(CFLReachability::flows_to(&self.snapshot.sdg, from, to, mode).unwrap_or(false))
    }

    /// Change-impact set of `sym_id` (cached).
    pub fn impact(&self, sym_id: u32, options: ImpactOptions) -> Result<Arc<Vec<ImpactEntry>>, QueryError> {
        let params = QueryParams::Ids(vec![
            sym_id as u64,
            options.include_overrides as u64,
            options.include_type_dependents as u64,
            depth_param(options.depth_limit),
        ]);
        let key = self.key(QueryKind::Impact, params);
        if let Some(hit) = self.cache.get(&key) {
            if let QueryResult::Impact(v) = hit.as_ref() {
                return Ok(Arc::clone(v));
            }
        }
        let s = &self.snapshot;
        let entries = ImpactAnalyzer::analyze(&s.symbols, &s.hierarchy, &s.call_graph, sym_id, options)
            .ok_or(QueryError::UnknownSymbol(sym_id))?;
        let result = Arc::new(entries);
        self.cache.put(key, Arc::new(QueryResult::Impact(Arc::clone(&result))));
        Ok(result)
    }

    /// O(1) entity → source location.
    pub fn to_source(&self, sym_id: u32) -> Option<SourceLocation> {
        let s = &self.snapshot;
        s.navigation.location(sym_id, &s.files, s.scpg_hash)
    }

    /// Entities containing a source position, innermost first, in O(log n + k).
    pub fn entities_at(&self, file_id: u16, line: u32, col: u16) -> Vec<SourceLocation> {
        let s = &self.snapshot;
        s.navigation
            .entities_at(file_id, line, col)
            .into_iter()
            .filter_map(|span| s.navigation.location(span.sym_id, &s.files, s.scpg_hash))
            .collect()
    }

    /// ROBDD path summary of a function.
    pub fn path_summary(&self, sym_id: u32) -> Option<FunctionPathSummary> {
        self.snapshot.paths.summary(sym_id).copied()
    }

    /// Feasible paths of a function under edge constraints.
    pub fn count_paths(&self, sym_id: u32, constraints: &[EdgeConstraint]) -> Result<PathCount, QueryError> {
        Ok(self.snapshot.paths.count_paths(sym_id, constraints)?)
    }

    /// One feasible path of a function under edge constraints.
    pub fn path_witness(&self, sym_id: u32, constraints: &[EdgeConstraint]) -> Result<Option<Vec<EdgeConstraint>>, QueryError> {
        Ok(self.snapshot.paths.witness(sym_id, constraints)?)
    }

    /// Evaluates an OpenHeart Datalog program (see `lang`).
    pub fn run(&self, program: &str) -> Result<Arc<QueryOutput>, QueryError> {
        let key = self.key(QueryKind::Program, QueryParams::Text(program.to_string()));
        if let Some(hit) = self.cache.get(&key) {
            if let QueryResult::Table(t) = hit.as_ref() {
                return Ok(Arc::clone(t));
            }
        }
        let output = Arc::new(lang::run_program(&self.snapshot, program, self.limits)?);
        self.cache.put(key, Arc::new(QueryResult::Table(Arc::clone(&output))));
        Ok(output)
    }
}
