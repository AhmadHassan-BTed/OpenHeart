//! Variable Renaming Algorithm (Phase B) (§5.2.3 & §5.5.2).
//! Authored by Ahmad Hassan (B-Ted).

use crate::ast::BPASTArtifact;
use crate::cfg::builder::FunctionCFGData;
use crate::core::types::ssa::{PhiArg, PhiRecord, SSARecord, SSA_FLAG_IS_PHI};
use crate::ssa::liveness::extract_stmt_vars;
use crate::ssa::placement::PendingPhi;
use crate::ssa::version_stack::VersionStack;
use std::collections::HashMap;

pub struct RenamingResult {
    pub ssa_records: Vec<SSARecord>,
    pub phi_records: Vec<PhiRecord>,
    pub def_offsets: Vec<u32>,
    pub use_adj: Vec<u32>,
}

pub fn rename_function(
    cfg: &FunctionCFGData,
    bpa: &BPASTArtifact,
    pending_phis: Vec<PendingPhi>,
    _block_phi_map: HashMap<u32, Vec<u32>>,
) -> RenamingResult {
    let mut ssa_records = Vec::new();
    let mut phi_records = Vec::new();
    let mut version_counts: HashMap<u32, u16> = HashMap::new();
    let mut ssa_uses_map: HashMap<u32, Vec<u32>> = HashMap::new();

    let mut phi_by_block: HashMap<u32, Vec<usize>> = HashMap::new();
    for (idx, phi) in pending_phis.iter().enumerate() {
        phi_by_block.entry(phi.block_id).or_default().push(idx);
    }

    // Build Dominator Tree Children mapping from idom[]
    let mut dom_children: HashMap<u32, Vec<u32>> = HashMap::new();
    for (node, &parent) in cfg.idom.iter().enumerate() {
        if node > 0 && parent != u32::MAX {
            dom_children.entry(parent).or_default().push(node as u32);
        }
    }

    let mut vstack = VersionStack::new();
    let mut phi_targets: HashMap<usize, u32> = HashMap::new();
    let mut phi_args_map: HashMap<usize, Vec<PhiArg>> = HashMap::new();

    // ── Execute Dominator Tree DFS Renaming starting from ENTRY (block 0) ──
    rename_dfs(
        0,
        cfg,
        bpa,
        &dom_children,
        &phi_by_block,
        &pending_phis,
        &mut vstack,
        &mut version_counts,
        &mut ssa_records,
        &mut phi_targets,
        &mut phi_args_map,
        &mut ssa_uses_map,
    );

    // Build PhiRecord instances
    for (idx, pphi) in pending_phis.into_iter().enumerate() {
        let ssa_id = phi_targets.get(&idx).copied().unwrap_or(u32::MAX);
        let args = phi_args_map.remove(&idx).unwrap_or_default();
        phi_records.push(PhiRecord::new(
            ssa_id,
            pphi.block_id,
            pphi.orig_sym_id,
            args,
        ));
    }

    // Build DefUseCSR
    let total_ssa = ssa_records.len();
    let mut def_offsets = Vec::with_capacity(total_ssa + 1);
    let mut use_adj = Vec::new();
    def_offsets.push(0);

    for ssa_id in 0..total_ssa as u32 {
        if let Some(uses) = ssa_uses_map.get(&ssa_id) {
            use_adj.extend_from_slice(uses);
        }
        def_offsets.push(use_adj.len() as u32);
    }

    crate::core::logger::log_trace(&format!(
        "Variable Renaming complete: generated {} SSA records, {} Phi records, {} total uses in DefUseCSR",
        ssa_records.len(),
        phi_records.len(),
        use_adj.len()
    ));

    RenamingResult {
        ssa_records,
        phi_records,
        def_offsets,
        use_adj,
    }
}

fn rename_dfs(
    b: u32,
    cfg: &FunctionCFGData,
    bpa: &BPASTArtifact,
    dom_children: &HashMap<u32, Vec<u32>>,
    phi_by_block: &HashMap<u32, Vec<usize>>,
    pending_phis: &[PendingPhi],
    vstack: &mut VersionStack,
    version_counts: &mut HashMap<u32, u16>,
    ssa_records: &mut Vec<SSARecord>,
    phi_targets: &mut HashMap<usize, u32>,
    phi_args_map: &mut HashMap<usize, Vec<PhiArg>>,
    ssa_uses_map: &mut HashMap<u32, Vec<u32>>,
) {
    let saved_depths = vstack.save_depths();

    // ── Part 1: Process phi-functions at head of block b ──
    if let Some(phi_indices) = phi_by_block.get(&b) {
        for &phi_idx in phi_indices {
            let var = pending_phis[phi_idx].orig_sym_id;
            let ver = version_counts.entry(var).or_insert(0);
            let ssa_id = ssa_records.len() as u32;

            let rec = SSARecord::new(
                ssa_id,
                var,
                u32::MAX, // def_stmt for phi is MAX
                *ver,
                SSA_FLAG_IS_PHI,
                b,
            );
            ssa_records.push(rec);
            *ver += 1;

            vstack.push(var, ssa_id);
            phi_targets.insert(phi_idx, ssa_id);
        }
    }

    // ── Part 2: Process ordinary statements in b ──
    let b_idx = b as usize;
    if b_idx < cfg.blocks.len() {
        for &stmt_node in &cfg.blocks[b_idx].stmts {
            let (uses, defs) = extract_stmt_vars(stmt_node, bpa);

            // Rename USES first (RHS semantics)
            for u in uses {
                let current_ver = vstack.top(u);
                if current_ver != u32::MAX {
                    ssa_uses_map.entry(current_ver).or_default().push(stmt_node);
                }
            }

            // Rename DEFS (LHS)
            for d in defs {
                let ver = version_counts.entry(d).or_insert(0);
                let ssa_id = ssa_records.len() as u32;

                let rec = SSARecord::new(ssa_id, d, stmt_node, *ver, 0, b);
                ssa_records.push(rec);
                *ver += 1;

                vstack.push(d, ssa_id);
            }
        }
    }

    // ── Part 3: Fill phi-function arguments in CFG successors ──
    let succs = get_succs(cfg, b);
    for succ in succs {
        if let Some(phi_indices) = phi_by_block.get(&succ) {
            for &phi_idx in phi_indices {
                let var = pending_phis[phi_idx].orig_sym_id;
                let current_ver = vstack.top(var);
                if current_ver != u32::MAX {
                    ssa_uses_map.entry(current_ver).or_default().push(u32::MAX);
                    // synthetic phi use
                }
                phi_args_map.entry(phi_idx).or_default().push(PhiArg {
                    pred_block_id: b,
                    arg_ssa_id: current_ver,
                });
            }
        }
    }

    // ── Part 4: Recurse over dominator-tree children ──
    if let Some(children) = dom_children.get(&b) {
        for &c in children {
            rename_dfs(
                c,
                cfg,
                bpa,
                dom_children,
                phi_by_block,
                pending_phis,
                vstack,
                version_counts,
                ssa_records,
                phi_targets,
                phi_args_map,
                ssa_uses_map,
            );
        }
    }

    // ── Part 5: Restore version stack ──
    vstack.restore_to(&saved_depths);
}

fn get_succs(cfg: &FunctionCFGData, block_id: u32) -> Vec<u32> {
    let idx = block_id as usize;
    if idx + 1 < cfg.succ_offsets.len() {
        let start = cfg.succ_offsets[idx] as usize;
        let end = cfg.succ_offsets[idx + 1] as usize;
        if start <= end && end <= cfg.succ_adj.len() {
            return cfg.succ_adj[start..end].to_vec();
        }
    }
    Vec::new()
}
