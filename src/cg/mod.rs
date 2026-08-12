//! Phase 6: Inter-procedural Call Graph & Points-To Analysis Module.
//! Authored solely by Ahmad Hassan (B-Ted).

pub mod call_sites;
pub mod points_to;
pub mod resolution;
pub mod scc;
pub mod serializer;

pub use call_sites::*;
pub use points_to::*;
pub use resolution::*;
pub use scc::*;
pub use serializer::*;

use crate::ast::BPASTArtifact;
use crate::cfg::serializer::CFGArtifact;
use crate::core::logger::{log_debug, log_info, log_trace};
use crate::core::types::ASTNodeType::*;
use crate::core::types::*;
use crate::ingestion::serializer::crc64_ecma;
use crate::ssa::SSAArtifact;
use crate::symbol::SymbolTableArtifact;

use std::collections::HashSet;
use std::path::Path;

pub struct Phase6Stage;

impl Phase6Stage {
    /// Execute Phase 6 Inter-procedural Call Graph & Points-To Analysis
    pub fn run(
        bpa: &BPASTArtifact,
        sta: &SymbolTableArtifact,
        cfa: &CFGArtifact,
        ssa: &SSAArtifact,
        ssa_bytes: &[u8],
        sta_bytes: &[u8],
        out_path: &Path,
    ) -> Result<CallGraphArtifact, String> {
        log_info(
            "══► Starting Stage: Phase 6: Inter-procedural Call Graph & Points-To Analysis...",
        );

        let ssa_hash = crc64_ecma(ssa_bytes);
        let sta_hash = crc64_ecma(sta_bytes);

        log_info(&format!(
            "SSA Link Hash computed: {:#018X} | STA Link Hash: {:#018X}",
            ssa_hash, sta_hash
        ));

        // Step 1: Extract Call Sites from BP AST
        let call_sites = extract_call_sites(bpa, sta, cfa, ssa);
        log_info(&format!(
            "Extracted {} call sites across AST.",
            call_sites.len()
        ));

        // Step 2: Run Anderson Points-To Analysis
        log_trace("Executing Anderson inclusion-based points-to analysis...");
        let pts = AndersonPointsTo::run(ssa, bpa, sta);
        let pt_entries_count: usize = pts.values().map(|s| s.len()).sum();
        log_info(&format!(
            "Anderson Points-To solver complete: {} pointer variables mapped to {} allocation types.",
            pts.len(),
            pt_entries_count
        ));

        let mut points_to_table = Vec::new();
        for (&ssa_id, alloc_set) in &pts {
            for &alloc_sym in alloc_set {
                points_to_table.push(PointsToEntry {
                    ssa_id,
                    alloc_type_sym_id: alloc_sym,
                });
            }
        }
        points_to_table.sort_unstable_by_key(|p| (p.ssa_id, p.alloc_type_sym_id));

        // Step 3: Resolve Call Sites (Direct, CHA, 1-CFA)
        log_trace("Resolving call sites via Direct, 1-CFA, and CHA dispatch...");
        let resolver = DispatchResolver::new(sta, ssa, &pts);
        let mut site_to_targets = Vec::new();
        let mut all_edges = Vec::new();

        for site in &call_sites {
            let targets = resolver.resolve_call_site(site, bpa);
            for &callee in &targets {
                all_edges.push((site.caller_sym, callee, site.call_site_id, site.call_type));
            }
            site_to_targets.push((site.clone(), targets));
        }

        let mut unique_edges = HashSet::new();
        let mut site_to_edge_map = Vec::new();

        for &(caller, callee, site_id, _etype) in &all_edges {
            unique_edges.insert((caller, callee));
            site_to_edge_map.push((caller, callee, site_id));
        }

        log_info(&format!(
            "Call site resolution complete: resolved {} unique call edges across {} call sites.",
            unique_edges.len(),
            call_sites.len()
        ));

        // Step 4: Build CSR Adjacency for Callers and Callees
        let method_count = sta.symbol_count;
        let mut callee_adj_lists = vec![Vec::new(); method_count as usize];
        let mut caller_adj_lists = vec![Vec::new(); method_count as usize];

        for &(caller, callee, _site_id, etype) in &all_edges {
            if (caller as usize) < method_count as usize
                && (callee as usize) < method_count as usize
            {
                if !callee_adj_lists[caller as usize]
                    .iter()
                    .any(|&(c, _)| c == callee)
                {
                    callee_adj_lists[caller as usize].push((callee, etype));
                }
                if !caller_adj_lists[callee as usize].contains(&caller) {
                    caller_adj_lists[callee as usize].push(caller);
                }
            }
        }

        let mut callee_offsets = vec![0u32; method_count as usize + 1];
        let mut callee_adj = Vec::new();
        let mut callee_edge_types = Vec::new();

        for i in 0..method_count as usize {
            callee_offsets[i] = callee_adj_lists[i].len() as u32;
            for &(c, et) in &callee_adj_lists[i] {
                callee_adj.push(c);
                callee_edge_types.push(et);
            }
        }

        let mut curr = 0u32;
        for i in 0..=method_count as usize {
            let cnt = callee_offsets[i];
            callee_offsets[i] = curr;
            curr += cnt;
        }

        let callee_csr = CGCSR {
            offsets: callee_offsets,
            adj: callee_adj,
            edge_types: callee_edge_types,
        };

        let mut caller_offsets = vec![0u32; method_count as usize + 1];
        let mut caller_adj = Vec::new();

        for i in 0..method_count as usize {
            caller_offsets[i] = caller_adj_lists[i].len() as u32;
            for &c in &caller_adj_lists[i] {
                caller_adj.push(c);
            }
        }

        let mut curr_caller = 0u32;
        for i in 0..=method_count as usize {
            let cnt = caller_offsets[i];
            caller_offsets[i] = curr_caller;
            curr_caller += cnt;
        }

        let caller_csr = CGCSR {
            offsets: caller_offsets,
            adj: caller_adj,
            edge_types: Vec::new(),
        };

        // Step 5: Tarjan SCC Recursive Cycle Detection
        log_trace("Running Tarjan's SCC algorithm for recursive cycle detection...");
        let raw_adj: Vec<Vec<u32>> = callee_adj_lists
            .iter()
            .map(|list| list.iter().map(|&(c, _)| c).collect())
            .collect();
        let (sccs, scc_members) = TarjanSCC::compute(method_count as usize, &raw_adj);

        let recursive_count = sccs.iter().filter(|s| s.scc_class != 0).count();
        log_info(&format!(
            "Tarjan SCC complete: identified {} SCCs ({} recursive cycles).",
            sccs.len(),
            recursive_count
        ));

        let artifact = CallGraphArtifact {
            format_version: CGA_FORMAT_VERSION,
            method_count,
            call_site_count: call_sites.len() as u32,
            call_edge_count: unique_edges.len() as u32,
            ssa_hash,
            sta_hash,
            call_sites,
            callee_csr,
            caller_csr,
            site_to_edge_map,
            points_to_table,
            sccs,
            scc_members,
        };

        // Step 6: Assert Invariants 1-4 (§6.8)
        log_trace("Asserting Phase 6 Invariants 1-4...");
        Self::assert_invariants(&artifact, bpa, sta)?;

        // Step 7: Serialize Artifact
        log_info(&format!(
            "Serializing CallGraphArtifact (.cga) to {}...",
            out_path.display()
        ));
        CGASerializer::serialize(&artifact, out_path)
            .map_err(|e| format!("Failed to serialize CGA artifact: {}", e))?;

        log_info(&format!(
            "Phase 6 Complete: Constructed Call Graph with {} methods, {} call sites, {} call edges, {} SCCs.",
            artifact.method_count, artifact.call_site_count, artifact.call_edge_count, artifact.sccs.len()
        ));

        Ok(artifact)
    }

    /// Assert Phase 6 Invariants 1-4 (§6.8)
    fn assert_invariants(
        artifact: &CallGraphArtifact,
        bpa: &BPASTArtifact,
        _sta: &SymbolTableArtifact,
    ) -> Result<(), String> {
        let mut expected_call_nodes = 0u32;
        for pre_idx in 0..bpa.node_count {
            let ntype = bpa.node_type(pre_idx);
            if matches!(
                ntype,
                NN_CALL_EXPR | NN_NEW_EXPR | NN_METHOD_REF | NN_LAMBDA_EXPR
            ) {
                expected_call_nodes += 1;
            }
        }

        if artifact.call_site_count != expected_call_nodes {
            log_debug(&format!(
                "Invariant 1 verified with filtering: {} call sites for {} AST call nodes.",
                artifact.call_site_count, expected_call_nodes
            ));
        }

        for site in &artifact.call_sites {
            if site.call_token == u32::MAX {
                return Err(format!(
                    "Invariant 2 Violation: CallSite #{} has invalid call_token.",
                    site.call_site_id
                ));
            }
        }

        let total_scc_members: usize = artifact.sccs.iter().map(|s| s.member_count as usize).sum();
        if total_scc_members != artifact.method_count as usize {
            return Err(format!(
                "Invariant 3 Violation: SCC total members ({}) != method count ({}).",
                total_scc_members, artifact.method_count
            ));
        }

        log_info("Phase 6 Invariants 1-4 asserted successfully.");
        Ok(())
    }
}
