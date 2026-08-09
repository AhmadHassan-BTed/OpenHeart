//! Phase 5: SSA Conversion & Data Flow Graph Module (§5.1 - §5.8).
//! Authored by Ahmad Hassan (B-Ted).

pub mod cdg;
pub mod ifds;
pub mod liveness;
pub mod placement;
pub mod renaming;
pub mod serializer;
pub mod version_stack;

use std::collections::HashSet;
use std::path::Path;

use crate::ast::BPASTArtifact;
use crate::cfg::serializer::CFGArtifact;
use crate::core::logger::{log_debug, log_info, log_trace, PhaseTimer};
use crate::ingestion::serializer::crc64_ecma;
use crate::ssa::serializer::FunctionSSAData;
use crate::symbol::SymbolTableArtifact;

pub use cdg::CDGBuilder;
pub use ifds::IFDSAnalyzer;
pub use liveness::LivenessAnalysis;
pub use placement::place_phi_functions;
pub use renaming::rename_function;
pub use serializer::{SSAArtifact, SSASerializer};

pub struct Phase5Stage;

impl Phase5Stage {
    pub fn run(
        bpa: &BPASTArtifact,
        _sta: &SymbolTableArtifact,
        cfa: &CFGArtifact,
        cfa_bytes: &[u8],
        out_path: &Path,
    ) -> Result<SSAArtifact, String> {
        let _timer = PhaseTimer::start("Phase 5: SSA Conversion & Data Flow Graph Construction");

        let cfa_hash = crc64_ecma(cfa_bytes);
        log_info(&format!("CFA Link Hash computed: 0x{:016X}", cfa_hash));

        let mut artifact = SSAArtifact::new(cfa_hash);

        for func_cfg in &cfa.functions {
            log_trace(&format!(
                "Processing SSA conversion for function sym_id={} ({} blocks)...",
                func_cfg.sym_id,
                func_cfg.blocks.len()
            ));

            // 1. Backward Liveness Analysis (Pruned SSA)
            let liveness = LivenessAnalysis::compute(func_cfg, bpa);

            // 2. Cytron's Phi Placement (Phase A)
            let (pending_phis, block_phi_map) = place_phi_functions(func_cfg, bpa, &liveness);

            // 3. Dominator Tree DFS Variable Renaming (Phase B)
            let renaming_res = rename_function(func_cfg, bpa, pending_phis, block_phi_map);

            // 4. Control Dependence Graph (CDG) Construction
            let (_ipdom, cdg) = CDGBuilder::build(func_cfg);

            // 5. IFDS Distributive Data-Flow Analyses (Taint, Nullable, Type-State)
            let ifds = IFDSAnalyzer::analyze(&renaming_res.ssa_records, bpa);

            let func_data = FunctionSSAData {
                sym_id: func_cfg.sym_id,
                ssa_records: renaming_res.ssa_records,
                phi_records: renaming_res.phi_records,
                def_use: crate::core::types::ssa::DefUseCSR {
                    def_offsets: renaming_res.def_offsets,
                    use_adj: renaming_res.use_adj,
                },
                cdg,
                ifds,
            };

            // Assert Invariants 1-4
            Self::verify_function_invariants(&func_data, func_cfg, bpa)?;

            log_debug(&format!(
                "  Function sym_id={}: {} SSA vars, {} φ-funcs, {} CDG edges, {} taint facts",
                func_data.sym_id,
                func_data.ssa_records.len(),
                func_data.phi_records.len(),
                func_data.cdg.cd_adj.len(),
                func_data.ifds.taint_sparse.len()
            ));

            artifact.add_function(func_data);
        }

        log_info(&format!(
            "Serializing SSAArtifact (.ssa) to {}...",
            out_path.display()
        ));
        SSASerializer::write(&artifact, out_path)?;

        log_info(&format!(
            "Phase 5 Complete: Converted {} functions to SSA ({} total SSA vars, {} total φ-funcs).",
            artifact.function_count, artifact.total_ssa_vars, artifact.total_phi_funcs
        ));

        Ok(artifact)
    }

    fn verify_function_invariants(
        ssa_data: &FunctionSSAData,
        cfg_data: &crate::cfg::builder::FunctionCFGData,
        bpa: &BPASTArtifact,
    ) -> Result<(), String> {
        log_trace(&format!(
            "  Asserting Phase 5 Invariants 1-4 for function sym_id={}...",
            ssa_data.sym_id
        ));

        // ── Invariant 1: Single Assignment ──
        let mut seen_defs = HashSet::new();
        for ssa in &ssa_data.ssa_records {
            if !seen_defs.insert(ssa.ssa_id) {
                return Err(format!(
                    "Invariant 1 Violation (Single Assignment): SSA variable v{} has multiple definition sites in sym_id={}",
                    ssa.ssa_id, ssa_data.sym_id
                ));
            }
        }

        // ── Invariant 2: φ-argument Count ──
        for phi in &ssa_data.phi_records {
            let block_id = phi.block_id as usize;
            let pred_count = if block_id + 1 < cfg_data.pred_offsets.len() {
                (cfg_data.pred_offsets[block_id + 1] - cfg_data.pred_offsets[block_id]) as usize
            } else {
                0
            };

            if phi.args.len() != pred_count {
                return Err(format!(
                    "Invariant 2 Violation (φ-argument Count): φ-function for ssa_id {} in block {} has {} args, expected pred_count {}",
                    phi.ssa_id, phi.block_id, phi.args.len(), pred_count
                ));
            }
        }

        // ── Invariant 3: Def-Use Completeness ──
        let offsets = &ssa_data.def_use.def_offsets;
        let adj = &ssa_data.def_use.use_adj;
        if !offsets.is_empty() && offsets.len() == ssa_data.ssa_records.len() + 1 {
            let total_uses_in_csr = *offsets.last().unwrap() as usize;
            if total_uses_in_csr != adj.len() {
                return Err(format!(
                    "Invariant 3 Violation (Def-Use Completeness): Def-Use CSR total uses offset {} != adj len {}",
                    total_uses_in_csr,
                    adj.len()
                ));
            }
        }

        // ── Invariant 4: SSA->TCA Traceability Seed ──
        for ssa in &ssa_data.ssa_records {
            if !ssa.is_phi() && ssa.def_stmt != u32::MAX {
                if ssa.def_stmt >= bpa.node_count as u32 {
                    return Err(format!(
                        "Invariant 4 Violation (Traceability Seed): SSA v{} def_stmt {} is out of BPA node range {}",
                        ssa.ssa_id, ssa.def_stmt, bpa.node_count
                    ));
                }
            }
        }

        Ok(())
    }
}
