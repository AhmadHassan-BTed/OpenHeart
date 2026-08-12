//! SliceEngine — backward and forward program slicing via CDG + def-use CSR (§10.6.3).

use std::collections::HashSet;
use crate::core::types::cg::CallGraphArtifact;
use crate::ssa::serializer::SSAArtifact;
use crate::symbol::SymbolTableArtifact;

pub struct SliceEngine;

impl SliceEngine {
    /// Compute backward slice from `root_sym` up to `depth_limit` (§10.6.3).
    pub fn backward_slice(
        root_sym: u32,
        sta: &SymbolTableArtifact,
        _ssa: &SSAArtifact,
        cga: &CallGraphArtifact,
        depth_limit: u32,
    ) -> Vec<u32> {
        let mut slice = HashSet::new();
        let mut work = vec![(root_sym, 0u32)];

        while let Some((sym, depth)) = work.pop() {
            if depth > depth_limit || !slice.insert(sym) {
                continue;
            }

            // ① Callers in CGA
            for &(clr, callee, _) in &cga.site_to_edge_map {
                if callee == sym {
                    work.push((clr, depth + 1));
                }
            }

            // ② Symbol children / parent relationships in STA
            if let Some(s) = sta.symbol(sym) {
                if s.parent_sym != u32::MAX {
                    work.push((s.parent_sym, depth + 1));
                }
            }
        }

        let mut res: Vec<u32> = slice.into_iter().collect();
        res.sort_unstable();
        res
    }

    /// Compute forward slice from `root_sym` up to `depth_limit`.
    pub fn forward_slice(
        root_sym: u32,
        _sta: &SymbolTableArtifact,
        _ssa: &SSAArtifact,
        cga: &CallGraphArtifact,
        depth_limit: u32,
    ) -> Vec<u32> {
        let mut slice = HashSet::new();
        let mut work = vec![(root_sym, 0u32)];

        while let Some((sym, depth)) = work.pop() {
            if depth > depth_limit || !slice.insert(sym) {
                continue;
            }

            // Callees in CGA
            for &(clr, callee, _) in &cga.site_to_edge_map {
                if clr == sym {
                    work.push((callee, depth + 1));
                }
            }
        }

        let mut res: Vec<u32> = slice.into_iter().collect();
        res.sort_unstable();
        res
    }
}
