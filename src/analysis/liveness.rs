//! Liveness of SSA values (backward may-analysis) with phi-aware edge semantics.
//!
//! ```text
//! out(b) = ⋃_{s ∈ succ(b)} ( in(s) ∪ { x : φ in s reads x from b } )
//! in(b)  = transfer of b's instructions applied backwards to out(b)
//!          (an instruction kills its definition and generates its uses;
//!           a φ kills its definition and generates nothing — its operands are live
//!           at the end of the matching predecessor, not at the start of the φ's block)
//! ```
//!
//! Exceptional successors are joined at the call that names them, so a value read only by an
//! exception handler is live up to the call and not to the end of the block.
//!
//! The results drive dead-definition detection, last-use detection (where an owned value may
//! be moved or released) and the register-pressure figure of the compiler views.

use crate::analysis::lattice::BitSet;
use crate::analysis::solver::{
    for_each_inst_use, for_each_term_use, inst_def, phi_operands_from, replay_backward,
    solve_backward, BackwardTransfer, CfgEdge, DataflowResult, InstPoint, IrCfg, OperandView,
    SolverOptions,
};
use crate::ir::{FuncIr, Inst, Terminator, ValueId};

/// Liveness transfer functions over bit sets indexed by `ValueId.0`.
pub struct LivenessTransfer;

impl BackwardTransfer<BitSet> for LivenessTransfer {
    fn exit_state(&self, _func: &FuncIr, _cfg: &IrCfg, _block: u32, _term: &Terminator) -> BitSet {
        BitSet::new()
    }

    fn transfer_inst(&self, _at: InstPoint<'_>, inst: &Inst, state: &mut BitSet) {
        if let Some(d) = inst_def(inst) {
            state.remove(d.0 as usize);
        }
        if !matches!(inst, Inst::Phi { .. }) {
            for_each_inst_use(inst, |v| {
                state.insert(v.0 as usize);
            });
        }
    }

    fn transfer_terminator(&self, _func: &FuncIr, _block: u32, term: &Terminator, state: &mut BitSet) {
        for_each_term_use(term, |v| {
            state.insert(v.0 as usize);
        });
    }

    fn transfer_edge(&self, func: &FuncIr, cfg: &IrCfg, edge: &CfgEdge, state: &mut BitSet) -> bool {
        let pred = cfg.block_id(edge.from);
        for inst in &func.blocks[edge.to as usize].insts {
            if let Inst::Phi { incoming, .. } = inst {
                for op in phi_operands_from(incoming, pred) {
                    if let Some(v) = op.value() {
                        state.insert(v.0 as usize);
                    }
                }
            }
        }
        true
    }
}

/// A definition or use at an instruction; `index == insts.len()` denotes the terminator.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ValueSite {
    pub block: u32,
    pub index: u32,
    pub value: ValueId,
}

/// Liveness of one function.
#[derive(Clone, Debug)]
pub struct Liveness {
    pub result: DataflowResult<BitSet>,
}

impl Liveness {
    pub fn compute(func: &FuncIr, cfg: &IrCfg, options: &SolverOptions) -> Liveness {
        Liveness {
            result: solve_backward(func, cfg, &LivenessTransfer, options),
        }
    }

    /// Values live on entry to `block`.
    pub fn live_in(&self, block: u32) -> &BitSet {
        self.result.state_in(block)
    }

    /// Values live at the end of `block` (exceptional successors are not included: they are
    /// joined at their call).
    pub fn live_out(&self, block: u32) -> &BitSet {
        self.result.state_out(block)
    }

    pub fn is_live_in(&self, block: u32, v: ValueId) -> bool {
        self.live_in(block).contains(v.0 as usize)
    }

    pub fn is_live_out(&self, block: u32, v: ValueId) -> bool {
        self.live_out(block).contains(v.0 as usize)
    }

    /// The live set after each instruction of `block`, in instruction order.
    pub fn live_after_each(&self, func: &FuncIr, cfg: &IrCfg, block: u32) -> Vec<BitSet> {
        let n = func.blocks[block as usize].insts.len();
        let mut out = vec![BitSet::new(); n];
        replay_backward(func, cfg, &LivenessTransfer, &self.result, block, |at, _, state| {
            out[at.index as usize] = state.clone();
        });
        out
    }

    /// Definitions whose value is never read on any path (dead unless the instruction has a
    /// side effect of its own, which the caller decides from the instruction kind).
    pub fn dead_defs(&self, func: &FuncIr, cfg: &IrCfg) -> Vec<ValueSite> {
        let mut dead = Vec::new();
        for &b in &cfg.rpo {
            replay_backward(func, cfg, &LivenessTransfer, &self.result, b, |at, inst, state| {
                if let Some(d) = inst_def(inst) {
                    if !state.contains(d.0 as usize) {
                        dead.push(ValueSite {
                            block: at.block,
                            index: at.index,
                            value: d,
                        });
                    }
                }
            });
        }
        dead.sort_unstable();
        dead
    }

    /// Reads after which the value is dead on every path: where an owned value may be moved
    /// or released. A phi operand's last use is attributed to the terminator of the
    /// predecessor it flows from.
    pub fn last_uses(&self, func: &FuncIr, cfg: &IrCfg) -> Vec<ValueSite> {
        let mut last = Vec::new();
        for &b in &cfg.rpo {
            replay_backward(func, cfg, &LivenessTransfer, &self.result, b, |at, inst, state| {
                if matches!(inst, Inst::Phi { .. }) {
                    return;
                }
                for_each_inst_use(inst, |v| {
                    let site = ValueSite {
                        block: at.block,
                        index: at.index,
                        value: v,
                    };
                    if !state.contains(v.0 as usize) && !last.contains(&site) {
                        last.push(site);
                    }
                });
            });
            let body = &func.blocks[b as usize];
            let term_index = body.insts.len() as u32;
            let mut after_term = BitSet::new();
            for &e in &cfg.succ_edges[b as usize] {
                let edge = &cfg.edges[e as usize];
                if edge.kind.is_unwind() || !self.result.reached[edge.to as usize] {
                    continue;
                }
                after_term.union_with(self.live_in(edge.to));
            }
            let mut phi_reads = BitSet::new();
            for &e in &cfg.succ_edges[b as usize] {
                let edge = &cfg.edges[e as usize];
                if edge.kind.is_unwind() {
                    continue;
                }
                LivenessTransfer.transfer_edge(func, cfg, edge, &mut phi_reads);
            }
            let mut push = |v: usize| {
                let site = ValueSite {
                    block: b,
                    index: term_index,
                    value: ValueId(v as u32),
                };
                if !last.contains(&site) {
                    last.push(site);
                }
            };
            for_each_term_use(&body.term, |v| {
                if !after_term.contains(v.0 as usize) {
                    push(v.0 as usize);
                }
            });
            for v in phi_reads.iter() {
                if !after_term.contains(v) {
                    push(v);
                }
            }
        }
        last.sort_unstable();
        last
    }

    /// The largest number of simultaneously live values and a block where it occurs.
    pub fn max_pressure(&self, func: &FuncIr, cfg: &IrCfg) -> (usize, Option<u32>) {
        let mut best = (0usize, None);
        for &b in &cfg.rpo {
            let mut local = self.live_in(b).len().max(self.live_out(b).len());
            replay_backward(func, cfg, &LivenessTransfer, &self.result, b, |_, _, state| {
                local = local.max(state.len());
            });
            if local > best.0 {
                best = (local, Some(b));
            }
        }
        best
    }
}
