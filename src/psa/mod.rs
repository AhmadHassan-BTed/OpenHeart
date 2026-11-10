//! Phase 8: ROBDD Path Summary Computation — Module Root (§8.1, §8.3, §8.5.1).
//!
//! **Phase Mandate (§8.1):** Phase 8 converts each function's CFG — a graph of basic blocks
//! and edges — into a compact Boolean formula whose satisfying assignments are exactly its
//! acyclic execution paths (Ball-Larus path segments when the CFG has loops). This formula
//! is stored as a Reduced Ordered Binary Decision Diagram (ROBDD): a canonical DAG that is
//! linear in the CFG size for structured code under the locality-preserving variable order.
//!
//! **Inputs:**  `CFGArtifact (.cfa)`, `SSAArtifact (.ssa)`, `CallGraphArtifact (.cga)` —
//!              or, language-neutrally, one [`NeutralPathFunction`] per function (a CFG with
//!              typed edges plus the edge guards of its canonical branch conditions), which
//!              any frontend's IR can supply (`Phase8Stage::try_run_neutral`).
//! **Output:** `PathSummaryArtifact (.psa)` — per-function ROBDD node arrays, variable
//!              ordering tables, and path metrics.
//!
//! **Capabilities of the PSA artifact:**
//! 1. Exact path counts — from an O(V+E) dynamic program, available for every function;
//!    counts above 2^64 are flagged and their magnitude is stored as `log2`.
//! 2. Path queries in O(|ROBDD|): paths through an edge, feasibility of required/forbidden
//!    edge sets, witness extraction (`psa::query`).
//! 3. Cyclomatic complexity in O(1) from stored metadata.
//! 4. Feasibility filtering when canonical branch predicates or edge guards are supplied;
//!    otherwise counts are structural and the record says so.
//!
//! **Phase 8 does NOT:** generate UML diagrams (Phase 9), serialize SCPG (Phase 10),
//! perform inter-procedural SSA analysis (Phase 5), or compose summaries across call sites —
//! every summary is intraprocedural, and recursive functions are flagged, not unwound.
//!
//! **Order and parallelism (§8.5.1):** functions are scheduled in the SCC order of the call
//! graph — Tarjan's algorithm emits SCCs callee-first (reverse topological order of the
//! condensation DAG), which is the bottom-up order interprocedural composition needs — and
//! built on worker threads. Every function is independent; results are collected and
//! logged in schedule order, so the artifact and the log are deterministic.
//!
//! **Errors.** The `try_run*` entry points return `io::Result`: a failed write, or two
//! summaries for one sym_id, is an error, never a panic. Per-function problems (budgets,
//! malformed CFGs, failed invariants) degrade that function's record and are reported as
//! diagnostics. `run` is the infallible wrapper kept for callers that treat a failed write
//! as fatal.
//!
//! This module tree contains no `unsafe` code.

#![forbid(unsafe_code)]

pub mod bdd;
pub mod builder;
pub mod config;
pub mod construction;
pub mod count;
pub mod metrics;
pub mod ordering;
pub mod query;
pub mod serializer;
pub mod types;

pub use builder::PathSummaryArtifactBuilder;
pub use config::Phase8Config;
pub use construction::feasibility::{
    BranchPredicate, BranchPredicateTable, EdgeGuard, PredicateLiteral,
};
pub use construction::graph::{ExceptionalEdges, PathCfg, PathEdgeKind};
pub use construction::NeutralPathFunction;
pub use count::PathCount;
pub use serializer::PathSummarySerializer;
pub use types::{FunctionROBDD, PathSummaryArtifact, PSA_MAGIC};

use std::collections::{HashMap, HashSet};
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::cfg::serializer::CFGArtifact;
use crate::core::logger::{log_debug, log_enabled, log_info, log_warn, LogLevel};
use crate::core::types::cg::CallGraphArtifact;
use crate::ingestion::serializer::crc64_ecma;
use crate::ssa::serializer::SSAArtifact;
use crate::tra::uml_link::digest::{digest_prefix_u64, digest_ssa};

use self::construction::FunctionROBDDBuilder;

/// Degraded-function diagnostics logged individually per run; the rest are counted.
const MAX_LOGGED_DEGRADED: usize = 32;

/// 64-bit digests identifying the inputs of a PSA built from neutral CFGs; stored in the
/// header fields `cfa_hash` and `ssa_hash`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PsaInputDigests {
    /// Digest of the artifact the CFGs were derived from.
    pub cfg_hash: u64,
    /// Digest of the SSA form the edge guards were derived from.
    pub ssa_hash: u64,
}

/// One scheduled function.
struct WorkItem {
    sym_id: u32,
    /// Position of the function in the stage's input list.
    index: usize,
    is_recursive: bool,
}

/// Per-run tallies for the completion log line.
#[derive(Default)]
struct RunStats {
    functions: usize,
    with_loops: usize,
    filtered: usize,
    recursive: usize,
    saturated: usize,
    degraded: usize,
}

/// Phase 8 stage — orchestrates ROBDD construction for all functions in the repository.
pub struct Phase8Stage;

impl Phase8Stage {
    /// Run Phase 8 with the default configuration and no branch predicates.
    ///
    /// Panics if the `.psa` file cannot be written; use [`Phase8Stage::try_run`] to handle
    /// the error.
    pub fn run(
        cfa: &CFGArtifact,
        ssa: &SSAArtifact,
        cga: &CallGraphArtifact,
        cfa_bytes: &[u8],
        out: &Path,
    ) -> PathSummaryArtifact {
        match Self::try_run(cfa, ssa, cga, cfa_bytes, out) {
            Ok(artifact) => artifact,
            Err(e) => panic!("Phase 8: Failed to write PathSummaryArtifact (.psa): {}", e),
        }
    }

    /// Fallible form of [`Phase8Stage::run`].
    ///
    /// # Arguments
    /// - `cfa`: CFGArtifact from Phase 4 — provides function CFG data.
    /// - `ssa`: SSAArtifact from Phase 5 — its canonical digest is recorded in the PSA.
    /// - `cga`: CallGraphArtifact from Phase 6 — provides the SCC table for scheduling.
    /// - `cfa_bytes`: raw CFA bytes for the CRC-64 hash (§8.8 Invariant 4: PSA → CFA chain).
    /// - `out`: output path for the .psa file.
    pub fn try_run(
        cfa: &CFGArtifact,
        ssa: &SSAArtifact,
        cga: &CallGraphArtifact,
        cfa_bytes: &[u8],
        out: &Path,
    ) -> io::Result<PathSummaryArtifact> {
        Self::try_run_with(cfa, ssa, cga, cfa_bytes, None, &Phase8Config::default(), out)
    }

    /// Run Phase 8 over Phase 4 CFGs with explicit configuration and optional canonical
    /// branch predicates (keyed by Phase 4 block id) for feasibility filtering.
    pub fn try_run_with(
        cfa: &CFGArtifact,
        ssa: &SSAArtifact,
        cga: &CallGraphArtifact,
        cfa_bytes: &[u8],
        predicates: Option<&BranchPredicateTable>,
        config: &Phase8Config,
        out: &Path,
    ) -> io::Result<PathSummaryArtifact> {
        log_info("══► Starting Stage: Phase 8: ROBDD Path Summary Computation...");

        let cfa_hash = crc64_ecma(cfa_bytes);
        // Chain: the PSA references the SSA artifact by its canonical content digest.
        let ssa_hash = digest_prefix_u64(&digest_ssa(ssa));

        let sym_ids: Vec<u32> = cfa.functions.iter().map(|f| f.sym_id).collect();
        let work = Self::schedule(&sym_ids, cga);
        let results = Self::build_all(&work, config, |item: &WorkItem| {
            let cfg = &cfa.functions[item.index];
            let preds = predicates
                .map(|t| t.predicates_for(item.sym_id))
                .unwrap_or(&[]);
            FunctionROBDDBuilder::build(item.sym_id, cfg, preds, item.is_recursive, config)
        });
        Self::finish(results, cfa_hash, ssa_hash, out)
    }

    /// Run Phase 8 over language-neutral CFGs (for example, CFGs of a frontend IR with the
    /// edge guards of its canonical branch conditions). Every record carries
    /// `PSA_FLAG_NEUTRAL_CFG`: its edge ids are positions in the supplied CFG's edge list.
    ///
    /// `cga` provides the SCC table for scheduling and recursion flags; functions the call
    /// graph does not cover are scheduled after it, in input order.
    pub fn try_run_neutral(
        functions: &[NeutralPathFunction],
        cga: &CallGraphArtifact,
        digests: PsaInputDigests,
        config: &Phase8Config,
        out: &Path,
    ) -> io::Result<PathSummaryArtifact> {
        log_info("══► Starting Stage: Phase 8: ROBDD Path Summary Computation (neutral CFGs)...");

        let sym_ids: Vec<u32> = functions.iter().map(|f| f.sym_id).collect();
        let work = Self::schedule(&sym_ids, cga);
        let results = Self::build_all(&work, config, |item: &WorkItem| {
            FunctionROBDDBuilder::build_neutral(&functions[item.index], item.is_recursive, config)
        });
        Self::finish(results, digests.cfg_hash, digests.ssa_hash, out)
    }

    /// Assemble, validate and write the artifact.
    fn finish(
        results: Vec<FunctionROBDD>,
        cfa_hash: u64,
        ssa_hash: u64,
        out: &Path,
    ) -> io::Result<PathSummaryArtifact> {
        let mut stats = RunStats::default();
        let mut artifact_builder = PathSummaryArtifactBuilder::new();
        for robdd in results {
            Self::log_function(&robdd, &mut stats);
            artifact_builder.add_function(robdd);
        }
        if stats.degraded > MAX_LOGGED_DEGRADED {
            log_warn(&format!(
                "Phase 8 DIAGNOSTIC: {} functions in total were stored without an ROBDD.",
                stats.degraded
            ));
        }

        // Finalize: sort function directory by sym_id (Invariants 1 and 2 reported).
        let artifact = artifact_builder
            .finalize(cfa_hash, ssa_hash)
            .map_err(|msg| io::Error::new(io::ErrorKind::InvalidData, msg))?;

        // Write .psa to disk (includes CRC-64 checksum for §8.8 Invariant 4).
        PathSummarySerializer::write(&artifact, out).map_err(|e| {
            io::Error::new(e.kind(), format!("cannot write {}: {}", out.display(), e))
        })?;

        log_info(&format!(
            "Phase 8 Complete: PathSummaryArtifact (.psa) — {} functions, {} total ROBDD nodes \
             ({} with loops, {} feasibility-filtered, {} in recursive SCCs, {} with counts above \
             2^64, {} without ROBDD).",
            artifact.function_count(),
            artifact.total_nodes,
            stats.with_loops,
            stats.filtered,
            stats.recursive,
            stats.saturated,
            stats.degraded,
        ));
        debug_assert_eq!(stats.functions, artifact.function_count());

        Ok(artifact)
    }

    /// Schedule every function exactly once: first in Tarjan SCC order (callee-first), then
    /// any function the call graph does not cover, in input order. `index` is the position
    /// of the function's first record in `sym_ids`; repeated records of a sym_id are not
    /// scheduled and are reported.
    fn schedule(sym_ids: &[u32], cga: &CallGraphArtifact) -> Vec<WorkItem> {
        let mut first_index: HashMap<u32, usize> = HashMap::with_capacity(sym_ids.len());
        let mut repeated = 0usize;
        for (i, &sym) in sym_ids.iter().enumerate() {
            if first_index.insert(sym, i).is_some() {
                repeated += 1;
            }
        }
        // `insert` kept the last position; restore the first one.
        if repeated > 0 {
            first_index.clear();
            for (i, &sym) in sym_ids.iter().enumerate() {
                first_index.entry(sym).or_insert(i);
            }
            log_warn(&format!(
                "Phase 8 DIAGNOSTIC: {} function records repeat an already listed sym_id; only \
                 the first record of each sym_id is summarized.",
                repeated
            ));
        }

        let mut scheduled: HashSet<u32> = HashSet::with_capacity(sym_ids.len());
        let mut work: Vec<WorkItem> = Vec::with_capacity(first_index.len());

        for scc in &cga.sccs {
            // scc_class >= 1 means this SCC is recursive (§6.4, §8.2.6).
            let is_recursive = scc.scc_class >= 1;
            let start = scc.member_offset as usize;
            let count = scc.member_count as usize;
            let members: &[u32] = match start.checked_add(count) {
                Some(end) if end <= cga.scc_members.len() => &cga.scc_members[start..end],
                _ => {
                    log_warn(&format!(
                        "Phase 8 DIAGNOSTIC: SCC {} member range {}+{} exceeds the {} recorded \
                         members; its functions are scheduled as non-recursive.",
                        scc.scc_id,
                        start,
                        count,
                        cga.scc_members.len()
                    ));
                    &[]
                }
            };
            for &sym_id in members {
                if let Some(&index) = first_index.get(&sym_id) {
                    if scheduled.insert(sym_id) {
                        work.push(WorkItem {
                            sym_id,
                            index,
                            is_recursive,
                        });
                    }
                }
            }
        }

        for (index, &sym_id) in sym_ids.iter().enumerate() {
            if first_index.get(&sym_id) == Some(&index) && scheduled.insert(sym_id) {
                work.push(WorkItem {
                    sym_id,
                    index,
                    is_recursive: false,
                });
            }
        }
        work
    }

    /// Build every scheduled function, in parallel when more than one worker is configured.
    /// The result vector is in schedule order.
    fn build_all<F>(work: &[WorkItem], config: &Phase8Config, build_one: F) -> Vec<FunctionROBDD>
    where
        F: Fn(&WorkItem) -> FunctionROBDD + Sync,
    {
        // The WebAssembly build has no threads: `scope.spawn` would panic there.
        let threads = if cfg!(target_arch = "wasm32") {
            1
        } else {
            config.resolved_threads(work.len())
        };
        if threads <= 1 {
            return work.iter().map(&build_one).collect();
        }

        let build_ref = &build_one;
        let next = AtomicUsize::new(0);
        let mut slots: Vec<Option<FunctionROBDD>> = (0..work.len()).map(|_| None).collect();
        std::thread::scope(|scope| {
            let handles: Vec<_> = (0..threads)
                .map(|_| {
                    scope.spawn(|| {
                        let mut local: Vec<(usize, FunctionROBDD)> = Vec::new();
                        loop {
                            let i = next.fetch_add(1, Ordering::Relaxed);
                            if i >= work.len() {
                                break;
                            }
                            local.push((i, build_ref(&work[i])));
                        }
                        local
                    })
                })
                .collect();
            for handle in handles {
                match handle.join() {
                    Ok(local) => {
                        for (i, robdd) in local {
                            slots[i] = Some(robdd);
                        }
                    }
                    Err(payload) => std::panic::resume_unwind(payload),
                }
            }
        });

        slots
            .into_iter()
            .map(|slot| match slot {
                Some(robdd) => robdd,
                None => unreachable!("every scheduled function is built by exactly one worker"),
            })
            .collect()
    }

    /// Tally one record; per-function detail goes to the debug log, degraded records to the
    /// diagnostics (warn) log.
    fn log_function(robdd: &FunctionROBDD, stats: &mut RunStats) {
        stats.functions += 1;
        if robdd.flags & types::PSA_FLAG_HAS_LOOPS != 0 {
            stats.with_loops += 1;
        }
        if robdd.flags & types::PSA_FLAG_FEASIBILITY_FILTERED != 0 {
            stats.filtered += 1;
        }
        if robdd.flags & types::PSA_FLAG_RECURSIVE_SCC != 0 {
            stats.recursive += 1;
        }
        if robdd.flags & types::PSA_FLAG_COUNT_SATURATED != 0 {
            stats.saturated += 1;
        }

        if log_enabled(LogLevel::Debug) {
            let mut line = format!(
                "  Phase 8: sym_id={} | {} nodes | {} vars | paths={} | V(G)={}",
                robdd.sym_id,
                robdd.nodes.len(),
                robdd.ordering.n_vars(),
                robdd.path_count,
                robdd.cyclomatic,
            );
            if robdd.flags & types::PSA_FLAG_HAS_LOOPS != 0 {
                line.push_str(" | acyclic segments (loops)");
            }
            if robdd.flags & types::PSA_FLAG_FEASIBILITY_FILTERED != 0 {
                line.push_str(" | feasibility-filtered");
            }
            if robdd.flags & types::PSA_FLAG_RECURSIVE_SCC != 0 {
                line.push_str(" | recursive SCC");
            }
            if robdd.flags & types::PSA_FLAG_EXCEPTIONAL_EXCLUDED != 0 {
                line.push_str(" | normal paths only");
            }
            log_debug(&line);
        }

        if let Some(reason) = &robdd.unavailable {
            stats.degraded += 1;
            if stats.degraded <= MAX_LOGGED_DEGRADED {
                log_warn(&format!(
                    "Phase 8 DIAGNOSTIC: sym_id={} stored without ROBDD ({}); exact path metrics kept.",
                    robdd.sym_id, reason
                ));
            }
        }
    }
}
