//! PathSummaryArtifactBuilder — accumulates FunctionROBDD records (§8.3, §8.5.1).
//!
//! After `Phase8Stage` processes each function in SCC order, the resulting `FunctionROBDD`
//! is added to this builder. When all functions are processed,
//! `PathSummaryArtifactBuilder::finalize()` sorts the function directory by sym_id and
//! assembles the complete `PathSummaryArtifact`.
//!
//! **Invariant 1 check (§8.8):** Every function with a body must have at least one path.
//! A zero path count after Phase 8 signals a provably unreachable function body (or, with
//! feasibility filtering, contradictory branch predicates on every path). This is reported
//! as a diagnostic (not a failure) during `finalize()`.
//!
//! **Invariant 2 check (§8.8):** each function's V(G) is compared with the structural lower
//! bound recomputed from its CFG during construction; records flagged
//! `PSA_FLAG_CYCLOMATIC_INCONSISTENT` are reported here.

use crate::core::logger::log_warn;
use crate::psa::metrics::PathMetrics;
use crate::psa::types::{
    FunctionPSAHeader, FunctionROBDD, PathSummaryArtifact, PSA_FLAG_CYCLOMATIC_INCONSISTENT,
    PSA_FLAG_EMPTY_CFG, PSA_FORMAT_VERSION,
};

/// Diagnostics of one `finalize` call are logged individually up to this many per kind; the
/// rest are summarized in one line.
const MAX_LOGGED_DIAGNOSTICS: usize = 32;

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
    /// Fails when two records share a sym_id: the directory is a sorted map keyed by sym_id,
    /// and silently dropping either record would publish a summary for the wrong body.
    ///
    /// # Invariant checks (§8.8)
    /// - **Invariant 1 (Path Coverage):** at least one path for every function with a body.
    ///   Functions without paths are logged as diagnostics (not failures).
    /// - **Invariant 2 (Cyclomatic Consistency):** V(G) is at least the structural value
    ///   `decision points + 1` of the CFG; violations (flagged during construction) are
    ///   logged as diagnostics.
    pub fn finalize(mut self, cfa_hash: u64, ssa_hash: u64) -> Result<PathSummaryArtifact, String> {
        // Sort functions by sym_id for O(log n) binary search in Phase 9 (stable, so equal
        // keys keep processing order in the error message below).
        self.functions.sort_by_key(|f| f.sym_id);
        if let Some(w) = self
            .functions
            .windows(2)
            .find(|w| w[0].sym_id == w[1].sym_id)
        {
            return Err(format!(
                "two path summaries were produced for sym_id={}",
                w[0].sym_id
            ));
        }

        let n = self.functions.len();
        let mut function_dir: Vec<FunctionPSAHeader> = Vec::with_capacity(n);
        let mut ordering_tables: Vec<Vec<u32>> = Vec::with_capacity(n);
        let mut node_arrays = Vec::with_capacity(n);
        let mut metrics: Vec<PathMetrics> = Vec::with_capacity(n);
        let mut total_nodes: u64 = 0;
        let mut pathless = 0usize;
        let mut inconsistent = 0usize;

        for robdd in self.functions {
            // ── Invariant 1 check (§8.8) ────────────────────────────────────
            if robdd.path_count.is_zero() {
                pathless += 1;
                if pathless <= MAX_LOGGED_DIAGNOSTICS {
                    let reason = if robdd.flags & PSA_FLAG_EMPTY_CFG != 0 {
                        "empty CFG"
                    } else {
                        "no path reaches a terminating block"
                    };
                    log_warn(&format!(
                        "Phase 8 DIAGNOSTIC: sym_id={} has no path ({}) — Invariant 1 check.",
                        robdd.sym_id, reason
                    ));
                }
            }

            // ── Invariant 2 check (§8.8) ────────────────────────────────────
            if robdd.flags & PSA_FLAG_CYCLOMATIC_INCONSISTENT != 0 {
                inconsistent += 1;
                if inconsistent <= MAX_LOGGED_DIAGNOSTICS {
                    log_warn(&format!(
                        "Phase 8 DIAGNOSTIC: sym_id={} reports V(G)={}, below the structural \
                         value of its CFG — Invariant 2 check.",
                        robdd.sym_id, robdd.cyclomatic
                    ));
                }
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

        if pathless > MAX_LOGGED_DIAGNOSTICS {
            log_warn(&format!(
                "Phase 8 DIAGNOSTIC: {} functions in total have no path (Invariant 1).",
                pathless
            ));
        }
        if inconsistent > MAX_LOGGED_DIAGNOSTICS {
            log_warn(&format!(
                "Phase 8 DIAGNOSTIC: {} functions in total fail Invariant 2.",
                inconsistent
            ));
        }

        Ok(PathSummaryArtifact {
            format_version: PSA_FORMAT_VERSION,
            cfa_hash,
            ssa_hash,
            function_dir,
            ordering_tables,
            node_arrays,
            metrics,
            total_nodes,
        })
    }
}

impl Default for PathSummaryArtifactBuilder {
    fn default() -> Self {
        Self::new()
    }
}
