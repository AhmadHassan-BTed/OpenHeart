//! QueryEngine — central query coordinator with LRU caching (§10.4, §10.5).

pub mod cache;
pub mod cfl;
pub mod impact;
pub mod navigation;
pub mod robdd;
pub mod slice;

pub use cache::LRUQueryCache;
pub use cfl::CFLReachability;
pub use impact::ImpactAnalyzer;
pub use navigation::NavigationEngine;
pub use robdd::ROBDDQueryEngine;
pub use slice::SliceEngine;

use crate::core::types::cg::CallGraphArtifact;
use crate::scpg::types::*;
use crate::ssa::serializer::SSAArtifact;
use crate::symbol::SymbolTableArtifact;

pub struct QueryEngine {
    pub cache: LRUQueryCache,
}

impl QueryEngine {
    pub fn new() -> Self {
        Self {
            cache: LRUQueryCache::new(512),
        }
    }

    pub fn is_reachable(&mut self, source: u32, target: u32, cga: &CallGraphArtifact, scpg_hash: u32) -> bool {
        let key = QueryKey {
            query_type: 0, // CFL
            params_crc: (source as u64) << 32 | (target as u64),
            scpg_hash,
        };

        if let Some(QueryResult::Boolean(res)) = self.cache.get(&key) {
            return res;
        }

        let res = CFLReachability::is_reachable(source, target, cga);
        self.cache.put(key, QueryResult::Boolean(res));
        res
    }

    pub fn backward_slice(
        &mut self,
        root_sym: u32,
        sta: &SymbolTableArtifact,
        ssa: &SSAArtifact,
        cga: &CallGraphArtifact,
        depth: u32,
        scpg_hash: u32,
    ) -> Vec<u32> {
        let key = QueryKey {
            query_type: 2, // SLICE_BWD
            params_crc: (root_sym as u64) << 32 | (depth as u64),
            scpg_hash,
        };

        if let Some(QueryResult::Slice(res)) = self.cache.get(&key) {
            return res;
        }

        let res = SliceEngine::backward_slice(root_sym, sta, ssa, cga, depth);
        self.cache.put(key, QueryResult::Slice(res.clone()));
        res
    }
}
