//! LRUQueryCache — sharded, bounded LRU cache keyed by full snapshot digest (§10.5).
//!
//! - Keys carry the query kind, the exact parameters and the 32-byte SHA-256 digest of the
//!   snapshot that answered them, so results of different snapshots can never collide.
//! - Each shard is a true LRU (O(1) get/put via an index-linked list); a full shard evicts its
//!   least recently used entry only.
//! - Shards are independent mutexes held only for the lookup or insert itself; queries run
//!   outside any lock against the immutable `Arc` snapshot, so they execute concurrently.
//! - A poisoned shard (a panic while it was held) is cleared and reused: the cache only ever
//!   holds recomputable data, so recovery costs a recomputation, never a wrong answer.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, MutexGuard};

use crate::scpg::types::{QueryKey, QueryResult};

const NIL: usize = usize::MAX;
pub const DEFAULT_CACHE_CAPACITY: usize = 512;
const SHARD_COUNT: usize = 16;

struct Entry {
    key: QueryKey,
    value: Arc<QueryResult>,
    prev: usize,
    next: usize,
}

struct LruShard {
    capacity: usize,
    map: HashMap<QueryKey, usize>,
    slots: Vec<Entry>,
    free: Vec<usize>,
    head: usize,
    tail: usize,
}

impl LruShard {
    fn new(capacity: usize) -> Self {
        Self {
            capacity,
            map: HashMap::new(),
            slots: Vec::new(),
            free: Vec::new(),
            head: NIL,
            tail: NIL,
        }
    }

    fn clear(&mut self) {
        self.map.clear();
        self.slots.clear();
        self.free.clear();
        self.head = NIL;
        self.tail = NIL;
    }

    fn unlink(&mut self, i: usize) {
        let (prev, next) = (self.slots[i].prev, self.slots[i].next);
        if prev != NIL {
            self.slots[prev].next = next;
        } else {
            self.head = next;
        }
        if next != NIL {
            self.slots[next].prev = prev;
        } else {
            self.tail = prev;
        }
        self.slots[i].prev = NIL;
        self.slots[i].next = NIL;
    }

    fn push_front(&mut self, i: usize) {
        self.slots[i].prev = NIL;
        self.slots[i].next = self.head;
        if self.head != NIL {
            self.slots[self.head].prev = i;
        }
        self.head = i;
        if self.tail == NIL {
            self.tail = i;
        }
    }

    fn get(&mut self, key: &QueryKey) -> Option<Arc<QueryResult>> {
        let i = *self.map.get(key)?;
        self.unlink(i);
        self.push_front(i);
        Some(Arc::clone(&self.slots[i].value))
    }

    fn put(&mut self, key: QueryKey, value: Arc<QueryResult>) {
        if self.capacity == 0 {
            return;
        }
        if let Some(&i) = self.map.get(&key) {
            self.slots[i].value = value;
            self.unlink(i);
            self.push_front(i);
            return;
        }
        if self.map.len() >= self.capacity && self.tail != NIL {
            let victim = self.tail;
            self.unlink(victim);
            let old_key = self.slots[victim].key.clone();
            self.map.remove(&old_key);
            self.free.push(victim);
        }
        let entry = Entry {
            key: key.clone(),
            value,
            prev: NIL,
            next: NIL,
        };
        let i = match self.free.pop() {
            Some(i) => {
                self.slots[i] = entry;
                i
            }
            None => {
                self.slots.push(entry);
                self.slots.len() - 1
            }
        };
        self.map.insert(key, i);
        self.push_front(i);
    }

    fn len(&self) -> usize {
        self.map.len()
    }
}

pub struct LRUQueryCache {
    shards: Vec<Mutex<LruShard>>,
}

impl Default for LRUQueryCache {
    fn default() -> Self {
        Self::new(DEFAULT_CACHE_CAPACITY)
    }
}

impl LRUQueryCache {
    /// A cache holding at most about `capacity` results (rounded up to a multiple of the
    /// shard count). `capacity == 0` disables caching.
    pub fn new(capacity: usize) -> Self {
        let per_shard = if capacity == 0 {
            0
        } else {
            capacity.div_ceil(SHARD_COUNT)
        };
        Self {
            shards: (0..SHARD_COUNT)
                .map(|_| Mutex::new(LruShard::new(per_shard)))
                .collect(),
        }
    }

    fn shard(&self, key: &QueryKey) -> MutexGuard<'_, LruShard> {
        let mut h = DefaultHasher::new();
        key.hash(&mut h);
        let mutex = &self.shards[(h.finish() as usize) % SHARD_COUNT];
        match mutex.lock() {
            Ok(guard) => guard,
            Err(poisoned) => {
                let mut guard = poisoned.into_inner();
                guard.clear();
                mutex.clear_poison();
                guard
            }
        }
    }

    pub fn get(&self, key: &QueryKey) -> Option<Arc<QueryResult>> {
        self.shard(key).get(key)
    }

    pub fn put(&self, key: QueryKey, result: Arc<QueryResult>) {
        let mut shard = self.shard(&key);
        shard.put(key, result);
    }

    /// Drops every cached result (e.g. after the engine switched to a new snapshot; old keys
    /// could never hit again, this only releases their memory).
    pub fn clear(&self) {
        for mutex in &self.shards {
            let mut guard = match mutex.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            guard.clear();
            drop(guard);
            mutex.clear_poison();
        }
    }

    pub fn len(&self) -> usize {
        self.shards
            .iter()
            .map(|m| match m.lock() {
                Ok(g) => g.len(),
                Err(p) => p.into_inner().len(),
            })
            .sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
