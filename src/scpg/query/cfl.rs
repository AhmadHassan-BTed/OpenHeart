//! CFLReachability — cubic worklist tabulation algorithm for inter-procedural path reachability (§10.3).

use crate::core::types::cg::CallGraphArtifact;
use std::collections::{HashSet, VecDeque};

pub struct CFLReachability;

impl CFLReachability {
    /// Determines whether target method `t` is CFL-reachable from source method `s` in G* (§10.3).
    pub fn is_reachable(source: u32, target: u32, cga: &CallGraphArtifact) -> bool {
        if source == target {
            return true;
        }

        let mut summary: HashSet<(u32, u32)> = HashSet::new();
        let mut work: VecDeque<(u32, u32)> = VecDeque::new();

        summary.insert((source, source));
        work.push_back((source, source));

        while let Some((s, u)) = work.pop_front() {
            if u == target {
                return true;
            }

            // Iterate call site successors from u
            for &(clr, callee, _site_id) in &cga.site_to_edge_map {
                if clr == u && summary.insert((s, callee)) {
                    work.push_back((s, callee));
                }
            }
        }

        summary.contains(&(source, target))
    }
}
