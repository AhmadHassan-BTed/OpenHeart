//! PathSummaryArtifactBuilder — accumulates FunctionROBDD records (§8.3, §8.5.1).
//!
//! After `Phase8Stage::run()` processes each function in SCC order, the resulting
//! `FunctionROBDD` is added to this builder. When all functions are processed,
//! `PathSummaryArtifactBuilder::finalize()` sorts the function directory by sym_id and
//! assembles the complete `PathSummaryArtifact`.
//!
//! **Invariant 1 check (§8.8):** Every function with a body must have at least one path.
//! A zero path count after Phase 8 signals a provably unreachable function body (or, with
//! feasibility filtering, contradictory branch predicates on every path). This is reported
//! as a diagnostic (not a failure) during `finalize()`.

use crate::core::logger::log_info;
use crate::psa::metrics::PathMetrics;
use crate::psa::types::{
    FunctionPSAHeader, FunctionROBDD, PathSummaryArtifact, PSA_FLAG_EMPTY_CFG,
    PSA_FORMAT_VERSION,
};

/// Builder for the PathSummaryArtifact.
///
/// Accumulates FunctionROBDD records as they are produced, then finalizes by sorting and
/// assembling the complete artifact.
pub struct PathSummaryArtifactBuilder {
    /// Accumulated function ROBDDs (in processing order).
    functions: Vec<FunctionROBDD>,
}

impl PathSummaryArtifactBuilder {
    /// Create an empty builder.
    pub fn new() -> Self {
        Self {
            functions: Vec::new(),
        }
    }

    /// Add a completed FunctionROBDD to the artifact.
    pub fn add_function(&mut self, robdd: FunctionROBDD) {
        self.functions.push(robdd);
    }

    /// Finalize the artifact: sort function directory by sym_id, run invariant checks,
    /// and assemble the PathSummaryArtifact.
    ///
    /// # Invariant checks (§8.8)
    /// - **Invariant 1 (Path Coverage):** at least one path for every function with a body.
    ///   Functions without paths are logged as diagnostics (not failures).
    /// - **Invariant 2 (Cyclomatic Consistency):** cyclomatic == |E| - |B| + 2 — the value
    ///   is copied from the CFA record during construction.
    pub fn finalize(mut self, cfa_hash: u64, ssa_hash: u64) -> PathSummaryArtifact {
        // Sort functions by sym_id for O(log n) binary search in Phase 9.
        self.functions.sort_unstable_by_key(|f| f.sym_id);

        let n = self.functions.len();
        let mut function_dir: Vec<FunctionPSAHeader> = Vec::with_capacity(n);
        let mut ordering_tables: Vec<Vec<u32>> = Vec::with_capacity(n);
        let mut node_arrays = Vec::with_capacity(n);
        let mut metrics: Vec<PathMetrics> = Vec::with_capacity(n);
        let mut total_nodes: u64 = 0;

        for robdd in self.functions {
            // ── Invariant 1 check (§8.8) ────────────────────────────────────
            if robdd.path_count.is_zero() {
                let reason = if robdd.flags & PSA_FLAG_EMPTY_CFG != 0 {
                    "empty CFG"
                } else {
                    "no path reaches a terminating block"
                };
                log_info(&format!(
                    "Phase 8 DIAGNOSTIC: sym_id={} has no path ({}) — Invariant 1 check.",
                    robdd.sym_id, reason
                ));
            }

            let header = FunctionPSAHeader::from_robdd(&robdd);
            total_nodes += robdd.nodes.len() as u64;

            let m = PathMetrics::new(
                robdd.cyclomatic,
                robdd.max_path_len,
                robdd.unwind_depth,
                robdd.flags,
                robdd.log2_paths,
            );

            function_dir.push(header);
            ordering_tables.push(robdd.ordering.codes().to_vec());
            node_arrays.push(robdd.nodes);
            metrics.push(m);
        }

        PathSummaryArtifact {
            format_version: PSA_FORMAT_VERSION,
            cfa_hash,
            ssa_hash,
            function_dir,
            ordering_tables,
            node_arrays,
            metrics,
            total_nodes,
        }
    }
}

impl Default for PathSummaryArtifactBuilder {
    fn default() -> Self {
        Self::new()
    }
}
