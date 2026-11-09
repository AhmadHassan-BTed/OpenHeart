//! Liveness Analysis Backward Fixpoint for Pruned SSA Form (§5.2.2).
//! Authored by Ahmad Hassan (B-Ted).
//!
//! Variables are the dense ids of [`FunctionVars`](crate::ssa::names::FunctionVars);
//! live sets are bitsets and the fixpoint is a worklist over predecessors, so each block
//! is revisited only when a successor's live-in set grew.

use crate::cfg::builder::FunctionCFGData;
use crate::ssa::names::VarEvent;
use std::collections::VecDeque;

/// Uses and definitions of every block statement, in evaluation order.
#[derive(Debug, Clone, Default)]
pub struct BlockEvents {
    /// Per block: `(statement node, start, end)` ranges into `events`.
    pub stmts: Vec<Vec<(u32, usize, usize)>>,
    pub events: Vec<VarEvent>,
}

impl BlockEvents {
    pub fn of_stmt(&self, start: usize, end: usize) -> &[VarEvent] {
        &self.events[start..end]
    }
}

pub struct LivenessResult {
    words: usize,
    live_in: Vec<u64>,
    live_out: Vec<u64>,
}

impl LivenessResult {
    #[inline]
    fn test(set: &[u64], words: usize, block: u32, var: u32) -> bool {
        let w = block as usize * words + (var as usize >> 6);
        set.get(w).is_some_and(|&bits| bits & (1u64 << (var & 63)) != 0)
    }

    pub fn is_live_in(&self, block_id: u32, var: u32) -> bool {
        Self::test(&self.live_in, self.words, block_id, var)
    }

    pub fn is_live_out(&self, block_id: u32, var: u32) -> bool {
        Self::test(&self.live_out, self.words, block_id, var)
    }

    /// Total number of (block, variable) live-in facts.
    pub fn live_in_count(&self) -> usize {
        self.live_in.iter().map(|w| w.count_ones() as usize).sum()
    }
}

pub struct LivenessAnalysis;

impl LivenessAnalysis {
    /// `entry_defs` are the variables defined on entry to block 0 (parameters, captured).
    pub fn compute(
        cfg: &FunctionCFGData,
        events: &BlockEvents,
        var_count: usize,
        entry_defs: &[u32],
    ) -> LivenessResult {
        let n = cfg.blocks.len();
        let words = var_count.div_ceil(64).max(1);
        let mut ue = vec![0u64; n * words];
        let mut kill = vec![0u64; n * words];

        let set = |v: &mut Vec<u64>, b: usize, var: u32| {
            v[b * words + (var as usize >> 6)] |= 1u64 << (var & 63);
        };
        let get = |v: &Vec<u64>, b: usize, var: u32| -> bool {
            v[b * words + (var as usize >> 6)] & (1u64 << (var & 63)) != 0
        };

        // ── 1. UEVar (used before any definition in the block) and VarKill ──
        if n > 0 {
            for &v in entry_defs {
                set(&mut kill, 0, v);
            }
        }
        for (b, stmts) in events.stmts.iter().enumerate().take(n) {
            for &(_, s, e) in stmts {
                for ev in events.of_stmt(s, e) {
                    match *ev {
                        VarEvent::Use { var, .. } => {
                            if !get(&kill, b, var) {
                                set(&mut ue, b, var);
                            }
                        }
                        VarEvent::Def { var, .. } => set(&mut kill, b, var),
                    }
                }
            }
        }

        // ── 2. Backward worklist fixpoint ──
        let mut live_in = ue.clone();
        let mut live_out = vec![0u64; n * words];
        let mut queued = vec![true; n];
        let mut worklist: VecDeque<u32> = (0..n as u32).rev().collect();
        let mut iterations = 0usize;

        while let Some(b) = worklist.pop_front() {
            let bi = b as usize;
            queued[bi] = false;
            iterations += 1;

            for w in 0..words {
                let mut out = 0u64;
                for &s in cfg.succs_of(b) {
                    out |= live_in[s as usize * words + w];
                }
                live_out[bi * words + w] = out;
            }

            let mut changed = false;
            for w in 0..words {
                let idx = bi * words + w;
                let new_in = ue[idx] | (live_out[idx] & !kill[idx]);
                if new_in != live_in[idx] {
                    live_in[idx] = new_in;
                    changed = true;
                }
            }

            if changed {
                for &p in cfg.preds_of(b) {
                    if !queued[p as usize] {
                        queued[p as usize] = true;
                        worklist.push_back(p);
                    }
                }
            }
        }

        let result = LivenessResult {
            words,
            live_in,
            live_out,
        };
        if crate::core::logger::get_log_level() >= crate::core::logger::LogLevel::Trace {
            crate::core::logger::log_trace(&format!(
                "Liveness Analysis converged after {} block visits ({} live-in entries across {} blocks)",
                iterations,
                result.live_in_count(),
                n
            ));
        }
        result
    }
}
