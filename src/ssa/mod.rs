//! Phase 5: SSA Conversion & Data Flow Graph Module (§5.1 - §5.8).
//! Authored by Ahmad Hassan (B-Ted).
//!
//! Per function: variable resolution (locals, parameters, captured variables keyed by
//! symbol id), bitset liveness, pruned Cytron φ-placement, dominator-tree renaming, the
//! control dependence graph and sparse intraprocedural data-flow facts. Functions are
//! independent and are converted in parallel on large-stack workers; results are merged
//! in CFA order.

pub mod cdg;
pub mod ifds;
pub mod liveness;
pub mod names;
pub mod placement;
pub mod renaming;
pub mod serializer;
pub mod version_stack;

use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::ast::BPASTArtifact;
use crate::cfg::analysis::DominatorTree;
use crate::cfg::builder::FunctionCFGData;
use crate::cfg::parallel::par_map;
use crate::cfg::serializer::CFGArtifact;
use crate::cfg::source::SourceText;
use crate::core::logger::{get_log_level, log_debug, log_info, log_warn, LogLevel, PhaseTimer};
use crate::core::types::ssa::DefUseCSR;
use crate::ingestion::serializer::{crc64_ecma, TokenCorpusArtifact};
use crate::ssa::ifds::{DataflowInput, TaintSpec};
use crate::ssa::liveness::BlockEvents;
use crate::ssa::names::{local_symbol_index, FunctionVars};
use crate::ssa::renaming::PHI_USE;
use crate::ssa::serializer::{FunctionSSAData, OperandCSR};
use crate::symbol::SymbolTableArtifact;

pub use cdg::CDGBuilder;
pub use ifds::SparseDataflowAnalyzer;
pub use liveness::LivenessAnalysis;
pub use placement::place_phi_functions;
pub use renaming::rename_function;
pub use serializer::{SSAArtifact, SSASerializer};

pub struct Phase5Stage;

impl Phase5Stage {
    /// Runs Phase 5 without token text: declarations still produce definitions, but
    /// identifier uses cannot be matched to variables and are all reported as
    /// unresolved. Prefer [`Phase5Stage::run_with_source`].
    pub fn run(
        bpa: &BPASTArtifact,
        sta: &SymbolTableArtifact,
        cfa: &CFGArtifact,
        cfa_bytes: &[u8],
        out_path: &Path,
    ) -> Result<SSAArtifact, String> {
        Self::run_with_source(bpa, sta, None, cfa, cfa_bytes, out_path)
    }

    pub fn run_with_source(
        bpa: &BPASTArtifact,
        sta: &SymbolTableArtifact,
        tca: Option<&TokenCorpusArtifact>,
        cfa: &CFGArtifact,
        cfa_bytes: &[u8],
        out_path: &Path,
    ) -> Result<SSAArtifact, String> {
        let timer = PhaseTimer::start("Phase 5: SSA Conversion & Data Flow Graph Construction");

        let cfa_hash = crc64_ecma(cfa_bytes);
        log_info(&format!("CFA Link Hash computed: 0x{:016X}", cfa_hash));

        let src = tca.map(SourceText::new);
        if src.is_none() {
            log_warn(
                "Phase 5 running without token text: identifier uses cannot be resolved to variables.",
            );
        }
        let sym_by_decl = local_symbol_index(sta);
        let spec = TaintSpec::java_default();

        let results = par_map(&cfa.functions, |func_cfg: &FunctionCFGData| {
            Self::convert_function(func_cfg, bpa, src.as_ref(), &sym_by_decl, &spec)
        });

        let mut artifact = SSAArtifact::new(cfa_hash);
        let mut unresolved_total = 0u64;
        for (func_cfg, result) in cfa.functions.iter().zip(results) {
            let func_data = result.map_err(|e| {
                format!("SSA conversion failed for function sym {}: {}", func_cfg.sym_id, e)
            })?;
            unresolved_total += u64::from(func_data.unresolved_names);

            if get_log_level() >= LogLevel::Debug {
                log_debug(&format!(
                    "  Function sym_id={}: {} SSA vars, {} φ-funcs, {} CDG edges, {} taint facts",
                    func_data.sym_id,
                    func_data.ssa_records.len(),
                    func_data.phi_records.len(),
                    func_data.cdg.cd_adj.len(),
                    func_data.ifds.taint_sparse.len()
                ));
            }
            artifact.add_function(func_data)?;
        }

        log_info(&format!(
            "Serializing SSAArtifact (.ssa) to {}...",
            out_path.display()
        ));
        SSASerializer::write(&artifact, out_path)?;

        let summary = format!(
            "Converted {} functions to SSA ({} total SSA vars, {} total φ-funcs, {} unresolved non-local names).",
            artifact.function_count, artifact.total_ssa_vars, artifact.total_phi_funcs, unresolved_total
        );
        log_info(&format!("Phase 5 Complete: {}", summary));
        timer.finish(&summary);

        Ok(artifact)
    }

    fn convert_function(
        func_cfg: &FunctionCFGData,
        bpa: &BPASTArtifact,
        src: Option<&SourceText<'_>>,
        sym_by_decl: &HashMap<u32, u32>,
        spec: &TaintSpec,
    ) -> Result<FunctionSSAData, String> {
        // 0. Resolve variables and extract per-statement def/use events.
        let vars = FunctionVars::build(bpa, src, sym_by_decl, func_cfg.decl_node);
        let n = func_cfg.blocks.len();
        let mut events = BlockEvents {
            stmts: vec![Vec::new(); n],
            events: Vec::new(),
        };
        for blk in &func_cfg.blocks {
            let bi = blk.id as usize;
            for &stmt in &blk.stmts {
                let s = events.events.len();
                vars.stmt_events(stmt, bpa, func_cfg, src, &mut events.events);
                let e = events.events.len();
                if bi < n {
                    events.stmts[bi].push((stmt, s, e));
                }
            }
        }
        let entry_vars: Vec<u32> = vars.entry_defs().iter().map(|&(v, _, _)| v).collect();

        // 1. Backward Liveness Analysis (Pruned SSA)
        let liveness = LivenessAnalysis::compute(func_cfg, &events, vars.var_count(), &entry_vars);

        // 2. Cytron's Phi Placement (Phase A)
        let phis = place_phi_functions(func_cfg, &events, vars.var_count(), &entry_vars, &liveness);

        // 3. Dominator Tree Variable Renaming (Phase B)
        let ren = rename_function(func_cfg, bpa, &vars, &events, &phis)?;

        // 4. Control Dependence Graph (CDG) Construction
        let (_ipdom, cdg) = CDGBuilder::build(func_cfg);

        // 5. Sparse intraprocedural data-flow facts (taint, nullness, resource state)
        let ifds = SparseDataflowAnalyzer::analyze(
            &DataflowInput {
                bpa,
                src,
                cfg: func_cfg,
                vars: &vars,
                ssa_records: &ren.ssa_records,
                phi_records: &ren.phi_records,
                value_nodes: &ren.value_nodes,
                operand_offsets: &ren.operand_offsets,
                operand_adj: &ren.operand_adj,
                def_offsets: &ren.def_offsets,
                use_adj: &ren.use_adj,
            },
            spec,
        );

        let func_data = FunctionSSAData {
            sym_id: func_cfg.sym_id,
            ssa_records: ren.ssa_records,
            def_blocks: ren.def_blocks,
            phi_records: ren.phi_records,
            def_use: DefUseCSR {
                def_offsets: ren.def_offsets,
                use_adj: ren.use_adj,
            },
            use_blocks: ren.use_blocks,
            value_nodes: ren.value_nodes,
            operands: OperandCSR {
                offsets: ren.operand_offsets,
                adj: ren.operand_adj,
            },
            cdg,
            ifds,
            var_count: u32::try_from(vars.var_count())
                .map_err(|_| "variable count exceeds the u32 range".to_string())?,
            unresolved_names: u32::try_from(vars.unresolved.len())
                .map_err(|_| "unresolved name count exceeds the u32 range".to_string())?,
        };

        Self::verify_function_invariants(&func_data, func_cfg, bpa)?;
        Ok(func_data)
    }

    fn verify_function_invariants(
        ssa_data: &FunctionSSAData,
        cfg_data: &FunctionCFGData,
        bpa: &BPASTArtifact,
    ) -> Result<(), String> {
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
            let pred_count = cfg_data.preds_of(phi.block_id).len();
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
        if offsets.len() != ssa_data.ssa_records.len() + 1
            || offsets.last().copied().unwrap_or(0) as usize != adj.len()
            || ssa_data.use_blocks.len() != adj.len()
        {
            return Err(format!(
                "Invariant 3 Violation (Def-Use Completeness): sym_id={} has {} defs, {} offsets, {} uses, {} use blocks",
                ssa_data.sym_id,
                ssa_data.ssa_records.len(),
                offsets.len(),
                adj.len(),
                ssa_data.use_blocks.len()
            ));
        }

        // ── Invariant 4: SSA->TCA Traceability Seed ──
        for ssa in &ssa_data.ssa_records {
            if !ssa.is_phi() && ssa.def_stmt != u32::MAX && ssa.def_stmt >= bpa.node_count {
                return Err(format!(
                    "Invariant 4 Violation (Traceability Seed): SSA v{} def_stmt {} is out of BPA node range {}",
                    ssa.ssa_id, ssa.def_stmt, bpa.node_count
                ));
            }
        }

        // ── Invariant 5: Dominance (every use is dominated by its definition) ──
        let dom = DominatorTree::new(&cfg_data.idom, 0);
        for (d, rec) in ssa_data.ssa_records.iter().enumerate() {
            let def_block = ssa_data.def_blocks.get(d).copied().unwrap_or(u32::MAX);
            let (s, e) = (offsets[d] as usize, offsets[d + 1] as usize);
            for k in s..e {
                let use_block = ssa_data.use_blocks[k];
                if !dom.dominates(def_block, use_block) {
                    return Err(format!(
                        "Invariant 5 Violation (Dominance): SSA v{} defined in block {} does not dominate its {} in block {} (sym_id={})",
                        rec.ssa_id,
                        def_block,
                        if adj[k] == PHI_USE { "φ argument edge" } else { "use" },
                        use_block,
                        ssa_data.sym_id
                    ));
                }
            }
        }

        Ok(())
    }
}
