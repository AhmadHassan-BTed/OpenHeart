//! Phase 8: ROBDD Path Summary Computation — Module Root (§8.1, §8.3, §8.5.1).
//!
//! **Phase Mandate (§8.1):** Phase 8 converts each function's CFG — a graph of basic blocks
//! and edges — into a compact Boolean formula that encodes exactly the set of all
//! structurally feasible execution paths through that function. This formula is stored
//! as a Reduced Ordered Binary Decision Diagram (ROBDD): a canonical, maximally-compressed
//! directed acyclic graph whose size is exponentially smaller than explicit path enumeration
//! for the vast majority of real-world code.
//!
//! **Inputs:**  `CFGArtifact (.cfa)`, `SSAArtifact (.ssa)`, `CallGraphArtifact (.cga)`.
//! **Output:** `PathSummaryArtifact (.psa)` — per-function ROBDD node arrays, variable
//!              ordering tables, and path metrics.
//!
//! **Four capabilities enabled by the PSA artifact (§8.1):**
//! 1. Counting feasible paths in O(|ROBDD|).
//! 2. Checking path feasibility in O(|ROBDD|).
//! 3. Computing cyclomatic complexity in O(1) from stored metadata.
//! 4. Enabling Phase 10 CFL-reachability to compose inter-procedural path queries.
//!
//! **Phase 8 does NOT:** generate UML diagrams (Phase 9), serialize SCPG (Phase 10),
//! or perform inter-procedural SSA analysis (Phase 5).
//!
//! **Top-level algorithm (§8.5.1):** Process functions in SCC condensation DAG topological
//! order (leaves/callees first) so callee summaries are available before callers need them.

pub mod bdd;
pub mod builder;
pub mod construction;
pub mod metrics;
pub mod ordering;
pub mod serializer;
pub mod types;

pub use builder::PathSummaryArtifactBuilder;
pub use serializer::PathSummarySerializer;
pub use types::{FunctionROBDD, PathSummaryArtifact, PSA_MAGIC};

use std::collections::HashMap;
use std::path::Path;

use crate::cfg::serializer::CFGArtifact;
use crate::core::logger::log_info;
use crate::core::types::cg::CallGraphArtifact;
use crate::ingestion::serializer::crc64_ecma;
use crate::ssa::serializer::SSAArtifact;

use self::construction::FunctionROBDDBuilder;

/// Phase 8 stage — orchestrates ROBDD construction for all functions in the repository.
pub struct Phase8Stage;

impl Phase8Stage {
    /// Run Phase 8: produce a `PathSummaryArtifact` from the CFA, SSA, and CGA artifacts.
    ///
    /// Functions are processed in **SCC condensation DAG topological order** (callees first)
    /// so that callee summaries are available when callers are processed — required for
    /// recursive SCC bounded unwinding (§8.2.6).
    ///
    /// The SCC topological order is the reverse of Tarjan's SCC output order (Tarjan produces
    /// SCCs in reverse topological order of the condensation DAG).
    ///
    /// # Arguments
    /// - `cfa`: CFGArtifact from Phase 4 — provides function CFG data.
    /// - `ssa`: SSAArtifact from Phase 5 — provides branch condition data for feasibility filtering.
    /// - `cga`: CallGraphArtifact from Phase 6 — provides SCC table for topological ordering.
    /// - `cfa_bytes`: raw CFA bytes for CRC-64 hash (§8.8 Invariant 4: PSA → CFA hash chain).
    /// - `out`: output path for the .psa file.
    pub fn run(
        cfa: &CFGArtifact,
        ssa: &SSAArtifact,
        cga: &CallGraphArtifact,
        cfa_bytes: &[u8],
        out: &Path,
    ) -> PathSummaryArtifact {
        log_info("══► Starting Stage: Phase 8: ROBDD Path Summary Computation...");

        let cfa_hash = crc64_ecma(cfa_bytes);
        let ssa_hash = cfa.sta_hash; // chain: PSA references CFA hash, CFA references STA hash

        let mut artifact_builder = PathSummaryArtifactBuilder::new();

        // Build sym_id → SSA function index for O(1) lookup.
        let ssa_index: HashMap<u32, usize> = ssa
            .functions
            .iter()
            .enumerate()
            .map(|(i, f)| (f.sym_id, i))
            .collect();

        // Build sym_id → CFA function index for O(1) lookup.
        let cfa_index: HashMap<u32, usize> = cfa
            .functions
            .iter()
            .enumerate()
            .map(|(i, f)| (f.sym_id, i))
            .collect();

        // Tarjan's algorithm produces SCCs in REVERSE topological order of the condensation DAG.
        // Reversing gives callee-first (bottom-up) order — exactly what §8.5.1 requires.
        // Iterate sccs in reverse order: the last SCC in cga.sccs is topologically first (a source
        // node in the condensation DAG = a leaf callee with no outgoing call edges).
        let scc_topo_iter: Vec<_> = cga.sccs.iter().rev().collect();

        for scc in scc_topo_iter {
            // scc_class >= 1 means this SCC is recursive (§6.4, §8.2.6).
            let is_recursive = scc.scc_class >= 1;

            // Extract member sym_ids from the flat scc_members array.
            let start = scc.member_offset as usize;
            let count = scc.member_count as usize;
            let members: &[u32] = if start + count <= cga.scc_members.len() {
                &cga.scc_members[start..start + count]
            } else {
                &[]
            };

            for &sym_id in members {
                // Retrieve function CFG data.
                let cfa_idx = match cfa_index.get(&sym_id) {
                    Some(&idx) => idx,
                    None => continue, // abstract method — no body
                };
                let cfg = &cfa.functions[cfa_idx];

                // Retrieve SSA data (None if the function had no SSA processing).
                let ssa_data = ssa_index.get(&sym_id).map(|&idx| &ssa.functions[idx]);

                // Build the ROBDD for this function (the full 8-step pipeline, §8.5.2).
                let robdd = FunctionROBDDBuilder::build(sym_id, cfg, ssa_data, is_recursive);

                log_info(&format!(
                    "  Phase 8: sym_id={} | {} nodes | {} vars | sat_count={} | V(G)={}{}",
                    sym_id,
                    robdd.nodes.len(),
                    robdd.ordering.n_vars(),
                    robdd.sat_count,
                    robdd.cyclomatic,
                    if robdd.unwind_depth > 0 {
                        format!(" | unwind_depth={}", robdd.unwind_depth)
                    } else {
                        String::new()
                    }
                ));

                artifact_builder.add_function(robdd);
            }
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
}
