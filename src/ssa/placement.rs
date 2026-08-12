//! Cytron's Phi-Placement Algorithm (Phase A) with Pruned SSA (§5.2.2 & §5.5.1).
//! Authored by Ahmad Hassan (B-Ted).

use crate::ast::BPASTArtifact;
use crate::cfg::builder::FunctionCFGData;
use crate::ssa::liveness::{extract_stmt_vars, LivenessResult};
use std::collections::{HashMap, HashSet, VecDeque};

#[derive(Debug, Clone)]
pub struct PendingPhi {
    pub block_id: u32,
    pub orig_sym_id: u32,
    pub target_ssa: u32,
}

pub fn place_phi_functions(
    cfg: &FunctionCFGData,
    bpa: &BPASTArtifact,
    liveness: &LivenessResult,
) -> (Vec<PendingPhi>, HashMap<u32, Vec<u32>>) {
    let mut pending_phis = Vec::new();
    let mut block_phi_map: HashMap<u32, Vec<u32>> = HashMap::new(); // block_id -> Vec<orig_sym_id>

    // 1. Gather all variables defined in this function
    let mut var_defsites: HashMap<u32, HashSet<u32>> = HashMap::new();

    for block in &cfg.blocks {
        for &stmt_node in &block.stmts {
            let (_, defs) = extract_stmt_vars(stmt_node, bpa);
            for d in defs {
                var_defsites.entry(d).or_default().insert(block.id);
            }
        }
    }

    // 2. Cytron's DF Worklist Algorithm per variable
    for (&var_sym, defsites) in &var_defsites {
        let mut phi_placed: HashSet<u32> = HashSet::new();
        let mut worklist: VecDeque<u32> = defsites.iter().copied().collect();

        while let Some(b) = worklist.pop_front() {
            let df_list = get_dominance_frontier(cfg, b);
            for &y in &df_list {
                // Pruned SSA check: place phi only if var_sym is live at entry of y
                if !phi_placed.contains(&y) && liveness.is_live_in(y, var_sym) {
                    phi_placed.insert(y);
                    let list = block_phi_map.entry(y).or_default();
                    if !list.contains(&var_sym) {
                        list.push(var_sym);
                    }

                    pending_phis.push(PendingPhi {
                        block_id: y,
                        orig_sym_id: var_sym,
                        target_ssa: u32::MAX,
                    });

                    if !defsites.contains(&y) {
                        worklist.push_back(y);
                    }
                }
            }
        }
    }

    crate::core::logger::log_trace(&format!(
        "Cytron's Phi-Placement complete: placed {} φ-functions across {} variables",
        pending_phis.len(),
        var_defsites.len()
    ));

    (pending_phis, block_phi_map)
}

fn get_dominance_frontier(cfg: &FunctionCFGData, block_id: u32) -> Vec<u32> {
    let idx = block_id as usize;
    if idx + 1 < cfg.df_offsets.len() {
        let start = cfg.df_offsets[idx] as usize;
        let end = cfg.df_offsets[idx + 1] as usize;
        if start <= end && end <= cfg.df_adj.len() {
            return cfg.df_adj[start..end].to_vec();
        }
    }
    Vec::new()
}
