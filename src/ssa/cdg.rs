//! Control Dependence Graph (CDG) Construction (§5.2.5).
//! Authored by Ahmad Hassan (B-Ted).

use crate::cfg::analysis::dominators::compute_idom_cooper;
use crate::cfg::builder::FunctionCFGData;
use crate::core::types::ssa::{CDGCSR, CD_EDGE_FALSE, CD_EDGE_TRUE};

pub struct CDGBuilder;

impl CDGBuilder {
    pub fn build(cfg: &FunctionCFGData) -> (Vec<u32>, CDGCSR) {
        let n = cfg.blocks.len();
        if n == 0 {
            return (Vec::new(), CDGCSR::default());
        }

        let mut _exit_block = 0;
        for b in &cfg.blocks {
            if b.is_exit {
                _exit_block = b.id;
                break;
            }
        }

        let mut rev_preds = vec![Vec::new(); n]; // reversed predecessors = forward successors
        let mut rev_succs = vec![Vec::new(); n]; // reversed successors = forward predecessors

        for &(u, v, _) in &cfg.edges {
            if (u as usize) < n && (v as usize) < n {
                rev_succs[v as usize].push(u);
                rev_preds[u as usize].push(v);
            }
        }

        let rev_rpo = (0..n as u32).collect::<Vec<u32>>();
        let ipdom = compute_idom_cooper(n, &rev_preds, &rev_rpo);

        let mut cdg_adj: Vec<Vec<(u32, u8)>> = vec![Vec::new(); n];

        for &(x, s, etype) in &cfg.edges {
            let cd_type = if etype == crate::core::types::cfg::CFGEdgeType::False {
                CD_EDGE_FALSE
            } else {
                CD_EDGE_TRUE
            };

            let ipdom_x = ipdom.get(x as usize).copied().unwrap_or(u32::MAX);
            let mut runner = s;

            while runner != u32::MAX && runner != ipdom_x {
                let list = &mut cdg_adj[x as usize];
                if !list.iter().any(|&(y, _)| y == runner) {
                    list.push((runner, cd_type));
                }
                runner = ipdom.get(runner as usize).copied().unwrap_or(u32::MAX);
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

        (
            ipdom,
            CDGCSR {
                cd_offsets,
                cd_adj,
                cd_types,
            },
        )
    }
}
