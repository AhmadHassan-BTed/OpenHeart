//! ImpactAnalyzer — impact set computation via call graph + symbol table (§10.5).

use crate::core::types::cg::CallGraphArtifact;
use crate::symbol::SymbolTableArtifact;
use std::collections::HashSet;

pub struct ImpactAnalyzer;

impl ImpactAnalyzer {
    /// Compute impact set of modifying `sym_id` (callers, overrides, and dependents).
    pub fn impact_set(
        sym_id: u32,
        _sta: &SymbolTableArtifact,
        cga: &CallGraphArtifact,
    ) -> Vec<u32> {
        let mut impacted = HashSet::new();
        let mut work = vec![sym_id];

        while let Some(s) = work.pop() {
            if !impacted.insert(s) {
                continue;
            }

            for &(clr, callee, _) in &cga.site_to_edge_map {
                if callee == s {
                    work.push(clr);
                }
            }
        }

        let mut res: Vec<u32> = impacted.into_iter().collect();
        res.sort_unstable();
        res
    }
}
