//! Control Dependence Graph (CDG) Construction (§5.2.5).
//! Authored by Ahmad Hassan (B-Ted).
//!
//! Ferrante–Ottenstein–Warren (1987): `y` is control dependent on `x` through edge
//! `x -> s` iff `y` lies on the post-dominator-tree path from `s` up to, but excluding,
//! `ipdom(x)`. Post-dominators are computed on the CFG augmented with a virtual exit
//! (fed by EXIT, by every sink and by one block per infinite-loop region) and with the
//! FOW edge `ENTRY -> virtual exit`, so that blocks executed unconditionally are control
//! dependent on ENTRY and every block has a post-dominator.

use crate::cfg::analysis::compute_post_idom_augmented;
use crate::cfg::analysis::dominators::UNDEFINED_IDOM;
use crate::cfg::builder::FunctionCFGData;
use crate::core::types::cfg::CFGEdgeType;
use crate::core::types::ssa::{CDGCSR, CD_EDGE_FALSE, CD_EDGE_TRUE};

/// Dependence through a switch selector edge.
pub const CD_EDGE_SWITCH: u8 = 0x03;
/// Dependence through an exceptional edge (the guarded statement may throw).
pub const CD_EDGE_EXCEPT: u8 = 0x04;
/// Dependence on ENTRY (the block executes whenever the function runs).
pub const CD_EDGE_ENTRY: u8 = 0x05;
/// Dependence through an unconditional edge out of a multi-successor block.
pub const CD_EDGE_UNCOND: u8 = 0x06;

fn label_of(edge_type: u8, from_entry: bool) -> u8 {
    if from_entry {
        return CD_EDGE_ENTRY;
    }
    match CFGEdgeType::from(edge_type) {
        // A do-while back edge is the true branch of its condition.
        CFGEdgeType::True | CFGEdgeType::LoopBack => CD_EDGE_TRUE,
        CFGEdgeType::False => CD_EDGE_FALSE,
        CFGEdgeType::Switch => CD_EDGE_SWITCH,
        CFGEdgeType::Except => CD_EDGE_EXCEPT,
        CFGEdgeType::Uncond | CFGEdgeType::Return => CD_EDGE_UNCOND,
    }
}

pub struct CDGBuilder;

impl CDGBuilder {
    /// Returns `(ipdom, cdg)`. `ipdom[b] == u32::MAX` means `b` is post-dominated only by
    /// the virtual exit.
    pub fn build(cfg: &FunctionCFGData) -> (Vec<u32>, CDGCSR) {
        let n = cfg.blocks.len();
        if n == 0 {
            return (Vec::new(), CDGCSR::default());
        }

        let succs: Vec<Vec<u32>> = (0..n as u32).map(|b| cfg.succs_of(b).to_vec()).collect();
        let pdom = compute_post_idom_augmented(n, &succs, cfg.exit_block, Some(0));
        let ipdom = &pdom.ipdom;
        let virtual_exit = pdom.virtual_exit;

        let mut cdg_adj: Vec<Vec<(u32, u8)>> = vec![Vec::new(); n];
        // stamp[y] == x + 1 once y has been recorded as dependent on x.
        let mut stamp = vec![0u32; n];

        for x in 0..n as u32 {
            let stop = ipdom[x as usize];
            for k in cfg.succ_edge_range(x) {
                let s = cfg.succ_adj[k];
                let label = label_of(cfg.edge_types[k], x == 0);
                let mut runner = s;
                while runner != stop && runner != virtual_exit && runner != UNDEFINED_IDOM {
                    let ri = runner as usize;
                    if ri >= n {
                        break;
                    }
                    if stamp[ri] != x + 1 {
                        stamp[ri] = x + 1;
                        cdg_adj[x as usize].push((runner, label));
                    }
                    let next = ipdom[ri];
                    if next == runner {
                        break;
                    }
                    runner = next;
                }
            }
        }

        let mut cd_offsets = Vec::with_capacity(n + 1);
        let mut cd_adj = Vec::new();
        let mut cd_types = Vec::new();

        cd_offsets.push(0);
        for list in &cdg_adj {
            for &(y, t) in list {
                cd_adj.push(y);
                cd_types.push(t);
            }
            cd_offsets.push(cd_adj.len() as u32);
        }

        if crate::core::logger::get_log_level() >= crate::core::logger::LogLevel::Trace {
            crate::core::logger::log_trace(&format!(
                "CDG Construction complete: constructed {} control dependence edges across {} basic blocks ({} infinite-loop roots)",
                cd_adj.len(),
                n,
                pdom.infinite_loop_roots.len()
            ));
        }

        let real_ipdom: Vec<u32> = ipdom[..n]
            .iter()
            .map(|&p| if p == virtual_exit { u32::MAX } else { p })
            .collect();

        (
            real_ipdom,
            CDGCSR {
                cd_offsets,
                cd_adj,
                cd_types,
            },
        )
    }
}
