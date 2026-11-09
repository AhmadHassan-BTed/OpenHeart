//! Phase8Config — resource limits and tuning of Phase 8 (§8.5, §8.7).
//!
//! ROBDD size depends on the variable order and can be exponential in the worst case, and
//! Phase 8 also runs inside `/api/analyze` on repositories supplied by remote users. Every
//! function is therefore built under a node budget and a wall-clock budget; exceeding either
//! degrades that function to a "summary unavailable" record that still carries the exact
//! path count, cyclomatic complexity and longest-path metrics (computed by O(V + E) dynamic
//! programs that need no BDD).

use std::time::Duration;

/// Phase 8 configuration.
#[derive(Clone, Debug)]
pub struct Phase8Config {
    /// Maximum ROBDD nodes (terminals included) per function during construction.
    pub max_bdd_nodes: usize,
    /// Wall-clock budget for building one function's ROBDD.
    pub bdd_time_budget: Duration,
    /// Run Rudell sifting on each stored ROBDD.
    pub sift: bool,
    /// Wall-clock budget for sifting one function's ROBDD.
    pub sift_time_budget: Duration,
    /// Adjacent-level swaps allowed per function.
    pub max_sift_swaps: usize,
    /// Abandon a sifting direction once the size exceeds this factor of the best size.
    pub sift_max_growth: f64,
    /// Variables sifted per function (largest first).
    pub max_sift_vars: usize,
    /// ROBDDs with fewer internal nodes than this are not sifted.
    pub min_sift_nodes: usize,
    /// Worker threads for per-function construction; 0 = available parallelism (max 8).
    pub worker_threads: usize,
}

impl Default for Phase8Config {
    fn default() -> Self {
        Self {
            max_bdd_nodes: 1_000_000,
            bdd_time_budget: Duration::from_secs(2),
            sift: true,
            sift_time_budget: Duration::from_millis(500),
            max_sift_swaps: 200_000,
            sift_max_growth: 1.2,
            max_sift_vars: 1000,
            min_sift_nodes: 32,
            worker_threads: 0,
        }
    }
}

impl Phase8Config {
    /// Number of worker threads to use for `jobs` functions.
    pub fn resolved_threads(&self, jobs: usize) -> usize {
        let wanted = if self.worker_threads == 0 {
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1)
                .min(8)
        } else {
            self.worker_threads
        };
        wanted.min(jobs).max(1)
    }
}
