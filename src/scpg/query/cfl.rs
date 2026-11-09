//! CFLReachability — method reachability and realizable-path dependence reachability (§10.3).
//!
//! Two distinct questions are answered here:
//! - **Method reachability** (`is_reachable`): can `target` execute during a call of `source`?
//!   Starting from a method entry, every call chain consists of unmatched call edges only, so
//!   every call-graph path is CFL-realizable; the question reduces to plain reachability and is
//!   answered by `CallGraphIndex` (SCC condensation + reachability index, CSR adjacency).
//! - **Dependence reachability** (`flows_to`): can the value or execution of one statement
//!   influence another along a *realizable* path of the SDG, i.e. one whose call and return
//!   edges form a matched-parenthesis (Dyck) string, possibly with unmatched prefixes? This is
//!   the CFL-reachability problem of Reps (1998); it is solved by the two-phase
//!   Horwitz-Reps-Binkley tabulation in `SliceEngine::two_phase` with early exit.

use crate::core::types::cg::CallGraphArtifact;
use crate::scpg::query::graph::CallGraphIndex;
use crate::scpg::query::sdg::SystemDependenceGraph;
use crate::scpg::query::slice::{SliceDirection, SliceEngine, SliceMode};

pub struct CFLReachability;

impl CFLReachability {
    /// Determines whether method `target` is reachable from method `source` through zero or
    /// more calls. Unknown method ids are unreachable (except `source == target`, which is
    /// trivially reachable only when the id is part of the call graph).
    ///
    /// This one-shot form indexes `cga` on every call (O(V + E)); engines keep a
    /// `CallGraphIndex` and query it directly.
    pub fn is_reachable(source: u32, target: u32, cga: &CallGraphArtifact) -> bool {
        CallGraphIndex::build(cga)
            .is_reachable(source, target)
            .unwrap_or(false)
    }

    /// Whether statement `from` can influence statement `to` along a realizable SDG path.
    /// `None` when either statement is not in the SDG.
    pub fn flows_to(
        sdg: &SystemDependenceGraph,
        from: (u32, u32),
        to: (u32, u32),
        mode: SliceMode,
    ) -> Option<bool> {
        let source = sdg.statement(from.0, from.1)?;
        let target = sdg.statement(to.0, to.1)?;
        let mut seeds = vec![source];
        if let Some(actual_in) = sdg.find(from.0, crate::scpg::query::sdg::NODE_ACTUAL_IN, from.1) {
            seeds.push(actual_in);
        }
        let traversal = SliceEngine::two_phase(sdg, &seeds, SliceDirection::Forward, mode, None, Some(target));
        Some(traversal.reached_target)
    }
}
