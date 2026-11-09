//! Control dependence on the IR CFG (Ferrante, Ottenstein & Warren 1987), computed from the
//! post-dominator tree in the runner formulation of Cytron et al. (1991).
//!
//! Block `y` is control dependent on edge `x → s` iff `y` post-dominates `s` but does not
//! strictly post-dominate `x`: taking that edge commits execution to `y`, taking another
//! out-edge of `x` may avoid it. Equivalently, `x` is in the post-dominance frontier of `y`.
//!
//! ```text
//! for every edge e = (x → s):
//!     runner := s
//!     while runner ≠ ipdom(x) and runner ≠ EXIT:
//!         CD(runner) += (x, e)
//!         runner := ipdom(runner)
//! ```
//!
//! Dependences are recorded per *edge*, not per branch block, so the several cases of a
//! `Switch` that reach the same block stay distinguishable and each dependence can be turned
//! into a guard literal by [`crate::analysis::conditions`].
//!
//! The post-dominator tree is rooted at a virtual exit fed by every block without successors;
//! regions that cannot reach an exit (infinite loops) are attached through one representative
//! each (see `crate::cfg::analysis::postdom`). Exceptional edges are included on request: with
//! them every block after a call that may throw depends on the call completing normally.

use crate::analysis::solver::IrCfg;
use crate::cfg::analysis::dominators::UNDEFINED_IDOM;
use crate::cfg::analysis::postdom::compute_post_idom;

/// A dependence of a block on one out-edge of a branching block.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ControlDep {
    /// Position of the branching block.
    pub branch: u32,
    /// Index of the edge in `IrCfg::edges`.
    pub edge: u32,
}

/// Control dependence of one function.
#[derive(Clone, Debug)]
pub struct ControlDependence {
    /// Immediate post-dominator per position, `virtual_exit` for blocks whose immediate
    /// post-dominator is the virtual exit. Index `virtual_exit` holds the exit itself.
    pub ipdom: Vec<u32>,
    pub virtual_exit: u32,
    /// Blocks attached to the virtual exit because their region cannot reach an exit.
    pub infinite_loop_roots: Vec<u32>,
    /// `deps[y]`: the edges `y` is control dependent on, sorted.
    pub deps: Vec<Vec<ControlDep>>,
    /// `controlled[x]`: the blocks control dependent on some out-edge of `x`, sorted.
    pub controlled: Vec<Vec<u32>>,
    /// Whether unwind edges took part.
    pub include_exceptional: bool,
}

impl ControlDependence {
    pub fn compute(cfg: &IrCfg, include_exceptional: bool) -> ControlDependence {
        let n = cfg.len();
        let mut succs: Vec<Vec<u32>> = vec![Vec::new(); n];
        for e in &cfg.edges {
            if !include_exceptional && e.kind.is_unwind() {
                continue;
            }
            let list = &mut succs[e.from as usize];
            if !list.contains(&e.to) {
                list.push(e.to);
            }
        }
        let pd = compute_post_idom(n, &succs, u32::MAX);
        let virtual_exit = pd.virtual_exit;
        let ipdom = pd.ipdom;

        let mut deps: Vec<Vec<ControlDep>> = vec![Vec::new(); n];
        let mut controlled: Vec<Vec<u32>> = vec![Vec::new(); n];
        for (ei, e) in cfg.edges.iter().enumerate() {
            if !include_exceptional && e.kind.is_unwind() {
                continue;
            }
            if !cfg.is_reachable(e.from) {
                continue;
            }
            let stop = ipdom.get(e.from as usize).copied().unwrap_or(UNDEFINED_IDOM);
            let mut runner = e.to;
            let mut steps = 0usize;
            while runner != stop && runner != virtual_exit && runner != UNDEFINED_IDOM {
                let dep = ControlDep {
                    branch: e.from,
                    edge: ei as u32,
                };
                let list = &mut deps[runner as usize];
                if !list.contains(&dep) {
                    list.push(dep);
                }
                let ctl = &mut controlled[e.from as usize];
                if !ctl.contains(&runner) {
                    ctl.push(runner);
                }
                let next = ipdom.get(runner as usize).copied().unwrap_or(UNDEFINED_IDOM);
                steps += 1;
                if next == runner || steps > n {
                    break;
                }
                runner = next;
            }
        }
        for list in &mut deps {
            list.sort_unstable();
        }
        for list in &mut controlled {
            list.sort_unstable();
        }

        ControlDependence {
            ipdom,
            virtual_exit,
            infinite_loop_roots: pd.infinite_loop_roots,
            deps,
            controlled,
            include_exceptional,
        }
    }

    /// The edges `block` is control dependent on.
    pub fn deps_of(&self, block: u32) -> &[ControlDep] {
        self.deps.get(block as usize).map_or(&[], |d| d.as_slice())
    }

    /// The post-dominance frontier of `block`: the distinct branching blocks it depends on.
    pub fn frontier(&self, block: u32) -> Vec<u32> {
        let mut out: Vec<u32> = self.deps_of(block).iter().map(|d| d.branch).collect();
        out.dedup();
        out
    }

    /// True when `block` executes on every path from entry to an exit (it depends on no
    /// branch).
    pub fn is_unconditional(&self, block: u32) -> bool {
        self.deps_of(block).is_empty()
    }

    /// True when `a` post-dominates `b` (reflexive).
    pub fn post_dominates(&self, a: u32, b: u32) -> bool {
        let mut runner = b;
        let limit = self.ipdom.len();
        for _ in 0..=limit {
            if runner == a {
                return true;
            }
            if runner == self.virtual_exit || runner == UNDEFINED_IDOM {
                return false;
            }
            let next = self.ipdom.get(runner as usize).copied().unwrap_or(UNDEFINED_IDOM);
            if next == runner {
                return false;
            }
            runner = next;
        }
        false
    }

    /// Immediate post-dominator of `block`; `None` when it is the virtual exit.
    pub fn immediate_post_dominator(&self, block: u32) -> Option<u32> {
        match self.ipdom.get(block as usize).copied() {
            Some(p) if p != self.virtual_exit && p != UNDEFINED_IDOM && p != block => Some(p),
            _ => None,
        }
    }

    /// Every dependence reachable from `block` through the transitive control-dependence
    /// relation (the backward slice of control over `block`), sorted.
    pub fn transitive_deps(&self, block: u32) -> Vec<ControlDep> {
        let mut seen_blocks = vec![false; self.deps.len()];
        let mut out: Vec<ControlDep> = Vec::new();
        let mut stack = vec![block];
        if let Some(s) = seen_blocks.get_mut(block as usize) {
            *s = true;
        }
        while let Some(b) = stack.pop() {
            for &d in self.deps_of(b) {
                if !out.contains(&d) {
                    out.push(d);
                }
                if let Some(seen) = seen_blocks.get_mut(d.branch as usize) {
                    if !*seen {
                        *seen = true;
                        stack.push(d.branch);
                    }
                }
            }
        }
        out.sort_unstable();
        out
    }

    /// Regions: blocks grouped by identical dependence sets (Ferrante et al.'s region nodes
    /// of the program dependence graph), as `(dependences, blocks)` sorted by first block.
    pub fn regions(&self, cfg: &IrCfg) -> Vec<(Vec<ControlDep>, Vec<u32>)> {
        let mut regions: Vec<(Vec<ControlDep>, Vec<u32>)> = Vec::new();
        for &b in &cfg.rpo {
            let key = self.deps_of(b);
            match regions.iter_mut().find(|(k, _)| k.as_slice() == key) {
                Some((_, blocks)) => blocks.push(b),
                None => regions.push((key.to_vec(), vec![b])),
            }
        }
        for (_, blocks) in &mut regions {
            blocks.sort_unstable();
        }
        regions.sort_by_key(|(_, blocks)| blocks.first().copied().unwrap_or(u32::MAX));
        regions
    }
}
