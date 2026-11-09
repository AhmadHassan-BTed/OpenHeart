//! Cytron's Phi-Placement Algorithm (Phase A) with Pruned SSA (§5.2.2 & §5.5.1).
//! Authored by Ahmad Hassan (B-Ted).

use crate::cfg::builder::FunctionCFGData;
use crate::ssa::liveness::{BlockEvents, LivenessResult};
use crate::ssa::names::VarEvent;

/// A φ-function to be created at the head of `block_id` for variable `var`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingPhi {
    pub block_id: u32,
    pub var: u32,
}

/// Returns, per block, the variables that need a φ-function (sorted by variable id).
///
/// A φ for `v` is placed at every block of the iterated dominance frontier of `v`'s
/// definition sites where `v` is live on entry (pruned SSA).
pub fn place_phi_functions(
    cfg: &FunctionCFGData,
    events: &BlockEvents,
    var_count: usize,
    entry_defs: &[u32],
    liveness: &LivenessResult,
) -> Vec<Vec<u32>> {
    let n = cfg.blocks.len();
    let mut phis: Vec<Vec<u32>> = vec![Vec::new(); n];
    if n == 0 || var_count == 0 {
        return phis;
    }

    // 1. Definition sites per variable (block 0 for entry definitions).
    let mut defsites: Vec<Vec<u32>> = vec![Vec::new(); var_count];
    for &v in entry_defs {
        if (v as usize) < var_count {
            defsites[v as usize].push(0);
        }
    }
    for (b, stmts) in events.stmts.iter().enumerate().take(n) {
        for &(_, s, e) in stmts {
            for ev in events.of_stmt(s, e) {
                if let VarEvent::Def { var, .. } = *ev {
                    let list = &mut defsites[var as usize];
                    if list.last() != Some(&(b as u32)) {
                        list.push(b as u32);
                    }
                }
            }
        }
    }

    // 2. Cytron's DF worklist per variable (stamps avoid per-variable clearing).
    let mut has_phi = vec![u32::MAX; n];
    let mut queued = vec![u32::MAX; n];
    let mut worklist: Vec<u32> = Vec::new();
    let mut placed = 0usize;

    for (var, sites) in defsites.iter().enumerate() {
        if sites.is_empty() {
            continue;
        }
        let stamp = var as u32;
        worklist.clear();
        for &b in sites {
            if queued[b as usize] != stamp {
                queued[b as usize] = stamp;
                worklist.push(b);
            }
        }
        while let Some(b) = worklist.pop() {
            let bi = b as usize;
            if bi + 1 >= cfg.df_offsets.len() {
                continue;
            }
            let (s, e) = (cfg.df_offsets[bi] as usize, cfg.df_offsets[bi + 1] as usize);
            for &y in &cfg.df_adj[s..e.min(cfg.df_adj.len())] {
                let yi = y as usize;
                if yi >= n || has_phi[yi] == stamp || !liveness.is_live_in(y, var as u32) {
                    continue;
                }
                has_phi[yi] = stamp;
                phis[yi].push(var as u32);
                placed += 1;
                if queued[yi] != stamp {
                    queued[yi] = stamp;
                    worklist.push(y);
                }
            }
        }
    }

    for list in &mut phis {
        list.sort_unstable();
    }

    if crate::core::logger::get_log_level() >= crate::core::logger::LogLevel::Trace {
        crate::core::logger::log_trace(&format!(
            "Cytron's Phi-Placement complete: placed {} φ-functions across {} variables",
            placed, var_count
        ));
    }

    phis
}
