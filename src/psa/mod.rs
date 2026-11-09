//! Phase 8: ROBDD Path Summary Computation — Module Root (§8.1, §8.3, §8.5.1).
//!
//! **Phase Mandate (§8.1):** Phase 8 converts each function's CFG — a graph of basic blocks
//! and edges — into a compact Boolean formula whose satisfying assignments are exactly its
//! acyclic execution paths (Ball-Larus path segments when the CFG has loops). This formula
//! is stored as a Reduced Ordered Binary Decision Diagram (ROBDD): a canonical DAG that is
//! linear in the CFG size for structured code under the locality-preserving variable order.
//!
//! **Inputs:**  `CFGArtifact (.cfa)`, `SSAArtifact (.ssa)`, `CallGraphArtifact (.cga)`.
//! **Output:** `PathSummaryArtifact (.psa)` — per-function ROBDD node arrays, variable
//!              ordering tables, and path metrics.
//!
//! **Capabilities of the PSA artifact:**
//! 1. Exact path counts — from an O(V+E) dynamic program, available for every function;
//!    counts above 2^64 are flagged and their magnitude is stored as `log2`.
//! 2. Path queries in O(|ROBDD|): paths through an edge, feasibility of required/forbidden
//!    edge sets, witness extraction (`psa::query`).
//! 3. Cyclomatic complexity in O(1) from stored metadata.
//! 4. Branch-correlation feasibility filtering when canonical branch predicates are supplied
//!    (`run_with`); otherwise counts are structural and say so.
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
pub use construction::feasibility::{BranchPredicate, BranchPredicateTable};
pub use count::PathCount;
pub use serializer::PathSummarySerializer;
pub use types::{FunctionROBDD, PathSummaryArtifact, PSA_MAGIC};

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::cfg::serializer::CFGArtifact;
use crate::core::logger::{log_info, log_warn};
use crate::core::types::cg::CallGraphArtifact;
use crate::ingestion::serializer::crc64_ecma;
use crate::ssa::serializer::SSAArtifact;
use crate::tra::uml_link::digest::{digest_prefix_u64, digest_ssa};

use self::construction::FunctionROBDDBuilder;

/// One scheduled function.
struct WorkItem {
    sym_id: u32,
    cfa_index: usize,
    is_recursive: bool,
}

/// Phase 8 stage — orchestrates ROBDD construction for all functions in the repository.
pub struct Phase8Stage;

impl Phase8Stage {
    /// Run Phase 8 with the default configuration and no branch predicates.
    ///
    /// # Arguments
    /// - `cfa`: CFGArtifact from Phase 4 — provides function CFG data.
    /// - `ssa`: SSAArtifact from Phase 5 — its canonical digest is recorded in the PSA.
    /// - `cga`: CallGraphArtifact from Phase 6 — provides the SCC table for scheduling.
    /// - `cfa_bytes`: raw CFA bytes for the CRC-64 hash (§8.8 Invariant 4: PSA → CFA chain).
    /// - `out`: output path for the .psa file.
    pub fn run(
        cfa: &CFGArtifact,
        ssa: &SSAArtifact,
        cga: &CallGraphArtifact,
        cfa_bytes: &[u8],
        out: &Path,
    ) -> PathSummaryArtifact {
        Self::run_with(cfa, ssa, cga, cfa_bytes, None, &Phase8Config::default(), out)
    }

    /// Run Phase 8 with explicit configuration and optional canonical branch predicates for
    /// feasibility filtering (see `construction::feasibility`).
    pub fn run_with(
        cfa: &CFGArtifact,
        ssa: &SSAArtifact,
        cga: &CallGraphArtifact,
        cfa_bytes: &[u8],
        predicates: Option<&BranchPredicateTable>,
        config: &Phase8Config,
        out: &Path,
    ) -> PathSummaryArtifact {
        log_info("══► Starting Stage: Phase 8: ROBDD Path Summary Computation...");

        let cfa_hash = crc64_ecma(cfa_bytes);
        // Chain: the PSA references the SSA artifact by its canonical content digest.
        let ssa_hash = digest_prefix_u64(&digest_ssa(ssa));

        let work = Self::schedule(cfa, cga);
        let results = Self::build_all(&work, cfa, predicates, config);

        let mut artifact_builder = PathSummaryArtifactBuilder::new();
        for robdd in results {
            Self::log_function(&robdd);
            artifact_builder.add_function(robdd);
        }

        // Finalize: sort function directory by sym_id (Invariant 1 check included).
        let artifact = artifact_builder.finalize(cfa_hash, ssa_hash);

        // Write .psa to disk (includes CRC-64 checksum for §8.8 Invariant 4).
        if let Err(e) = PathSummarySerializer::write(&artifact, out) {
            panic!("Phase 8: Failed to write PathSummaryArtifact (.psa): {}", e);
        }

        log_info(&format!(
            "Phase 8 Complete: PathSummaryArtifact (.psa) — {} functions, {} total ROBDD nodes.",
            artifact.function_count(),
            artifact.total_nodes,
        ));

        artifact
    }

    /// Schedule every function with a CFG exactly once: first in Tarjan SCC order
    /// (callee-first), then any CFG function the call graph does not cover, in CFA order.
    fn schedule(cfa: &CFGArtifact, cga: &CallGraphArtifact) -> Vec<WorkItem> {
        let cfa_index: HashMap<u32, usize> = cfa
            .functions
            .iter()
            .enumerate()
            .map(|(i, f)| (f.sym_id, i))
            .collect();
        let mut scheduled: HashSet<u32> = HashSet::with_capacity(cfa.functions.len());
        let mut work: Vec<WorkItem> = Vec::with_capacity(cfa.functions.len());

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
                if let Some(&cfa_idx) = cfa_index.get(&sym_id) {
                    if scheduled.insert(sym_id) {
                        work.push(WorkItem {
                            sym_id,
                            cfa_index: cfa_idx,
                            is_recursive,
                        });
                    }
                }
            }
        }

        for (i, f) in cfa.functions.iter().enumerate() {
            if scheduled.insert(f.sym_id) {
                work.push(WorkItem {
                    sym_id: f.sym_id,
                    cfa_index: i,
                    is_recursive: false,
                });
            }
        }
        work
    }

    /// Build every scheduled function, in parallel when more than one worker is configured.
    /// The result vector is in schedule order.
    fn build_all(
        work: &[WorkItem],
        cfa: &CFGArtifact,
        predicates: Option<&BranchPredicateTable>,
        config: &Phase8Config,
    ) -> Vec<FunctionROBDD> {
        let build_one = |item: &WorkItem| -> FunctionROBDD {
            let cfg = &cfa.functions[item.cfa_index];
            let preds = predicates
                .map(|t| t.predicates_for(item.sym_id))
                .unwrap_or(&[]);
            FunctionROBDDBuilder::build(item.sym_id, cfg, preds, item.is_recursive, config)
        };

        let threads = config.resolved_threads(work.len());
        if threads <= 1 {
            return work.iter().map(build_one).collect();
        }

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
                            local.push((i, build_one(&work[i])));
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

    fn log_function(robdd: &FunctionROBDD) {
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
        log_info(&line);
        if let Some(reason) = &robdd.unavailable {
            log_warn(&format!(
                "Phase 8 DIAGNOSTIC: sym_id={} stored without ROBDD ({}); exact path metrics kept.",
                robdd.sym_id, reason
            ));
        }
    }
}
