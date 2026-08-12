//! LRUQueryCache — 512-entry LRU cache invalidated on scpg_hash change (§10.5).

use crate::scpg::types::{QueryKey, QueryResult};
use std::collections::HashMap;

pub struct LRUQueryCache {
    capacity: usize,
    current_hash: u32,
    entries: HashMap<QueryKey, QueryResult>,
}

impl LRUQueryCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            current_hash: 0,
            entries: HashMap::new(),
        }
    }

    pub fn get(&mut self, key: &QueryKey) -> Option<QueryResult> {
        if key.scpg_hash != self.current_hash {
            self.entries.clear();
            self.current_hash = key.scpg_hash;
            return None;
        }
        self.entries.get(key).cloned()
    }

    pub fn put(&mut self, key: QueryKey, result: QueryResult) {
        if self.entries.len() >= self.capacity {
            self.entries.clear(); // Simple bounded LRU eviction
        }
        self.current_hash = key.scpg_hash;
        self.entries.insert(key, result);
    }

    pub fn invalidate(&mut self, new_hash: u32) {
        self.entries.clear();
        self.current_hash = new_hash;
    }
}
