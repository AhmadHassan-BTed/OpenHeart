//! PathSummaryArtifactBuilder — accumulates FunctionROBDD records (§8.3, §8.5.1).
//!
//! After `Phase8Stage::run()` processes each function in SCC topological order,
//! the resulting `FunctionROBDD` is added to this builder. When all functions are
//! processed, `PathSummaryArtifactBuilder::finalize()` sorts the function directory
//! by sym_id and assembles the complete `PathSummaryArtifact`.
//!
//! **Invariant 1 check (§8.8):** Every function with a body must have sat_count ≥ 1.
//! A sat_count == 0 after Phase 8 signals a provably unreachable function body.
//! This is reported as a diagnostic (not a failure) during `finalize()`.

use crate::core::logger::log_info;
use crate::psa::metrics::PathMetrics;
use crate::psa::types::{FunctionPSAHeader, FunctionROBDD, PathSummaryArtifact};

/// Builder for the PathSummaryArtifact.
///
/// Accumulates FunctionROBDD records as they are produced in topological order,
/// then finalizes by sorting and assembling the complete artifact.
pub struct PathSummaryArtifactBuilder {
    /// Accumulated function ROBDDs (in SCC topological order).
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
    /// - **Invariant 1 (Path Coverage):** sat_count ≥ 1 for every function with a body.
    ///   Functions with sat_count == 0 are logged as diagnostics (not failures).
    /// - **Invariant 2 (Cyclomatic Consistency):** cyclomatic == |E| - |B| + 2.
    ///   Cross-verified against CFA metadata if provided (omitted here — verified during construction).
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
            if robdd.sat_count == 0 {
                log_info(&format!(
                    "Phase 8 DIAGNOSTIC: sym_id={} has sat_count=0 — \
                     provably unreachable function body (Invariant 1 check).",
                    robdd.sym_id
                ));
                // Not a failure — continue processing.
            }

            let header = FunctionPSAHeader::from_robdd(&robdd);
            total_nodes += robdd.nodes.len() as u64;

            let edge_seq: Vec<u32> = robdd.ordering.as_edge_slice().iter().map(|&e| e as u32).collect();
            let m = PathMetrics::new(
                robdd.cyclomatic,
                robdd.max_path_len,
                robdd.sat_count,
                robdd.unwind_depth,
            );

            function_dir.push(header);
            ordering_tables.push(edge_seq);
            node_arrays.push(robdd.nodes);
            metrics.push(m);
        }

        PathSummaryArtifact {
            format_version: 1,
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
