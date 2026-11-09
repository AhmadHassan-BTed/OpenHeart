use std::collections::hash_map::RandomState;
use std::hash::BuildHasher;

const FNV1A_PRIME: u64 = 0x0000_0100_0000_01B3;
const FNV1A_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;

/// FNV-1a 64-bit hash function.
///
/// Used only for the persisted string-header index of the `.tca` format, which must be
/// stable across processes. The in-memory interner table does not use it (see
/// `StringInterner`).
#[inline]
pub fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash = FNV1A_OFFSET;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(FNV1A_PRIME);
    }
    hash
}

/// Bytes of the length prefix in front of every stored string.
pub const STRING_LEN_PREFIX_BYTES: usize = 4;

/// Deduplicating string interner: insertion-ordered ids over an open-addressing table.
///
/// The table hashes with a per-process randomly keyed SipHash (`RandomState`), so a crafted
/// input cannot force all identifiers into one probe cluster (hash flooding). Ids are
/// assigned in insertion order and never depend on the hash, so the corpus stays
/// deterministic. Collisions are resolved by comparing the bytes.
///
/// Storage layout: for each string in id order, a little-endian u32 length followed by the
/// bytes. Strings are stored in full; nothing is truncated.
#[derive(Debug, Clone)]
pub struct StringInterner {
    table: Vec<(u64, u32)>,
    table_mask: usize,
    count: u32,
    load_limit: usize,
    storage: Vec<u8>,
    offsets: Vec<u32>,
    hasher: RandomState,
}

impl StringInterner {
    pub fn with_capacity(initial_capacity: usize) -> Self {
        let cap = initial_capacity.max(16).next_power_of_two();
        Self {
            table: vec![(0, u32::MAX); cap],
            table_mask: cap - 1,
            count: 0,
            load_limit: cap / 4 * 3,
            storage: Vec::new(),
            offsets: Vec::new(),
            hasher: RandomState::new(),
        }
    }

    pub fn new() -> Self {
        Self::with_capacity(1024)
    }

    pub fn count(&self) -> u32 {
        self.count
    }

    #[inline]
    fn slot_hash(&self, text: &[u8]) -> u64 {
        // 0 marks an empty slot.
        self.hasher.hash_one(text).max(1)
    }

    pub fn intern(&mut self, text: &[u8]) -> u32 {
        let hash = self.slot_hash(text);
        let mut slot = (hash as usize) & self.table_mask;
        loop {
            let (h, id) = self.table[slot];
            if h == 0 {
                let text_id = self.count;
                self.store_string(text);
                self.table[slot] = (hash, text_id);
                self.count += 1;
                if self.count as usize > self.load_limit {
                    self.resize();
                }
                return text_id;
            }
            if h == hash && self.lookup_text(id) == text {
                return id;
            }
            slot = (slot + 1) & self.table_mask;
        }
    }

    pub fn find_id(&self, text: &[u8]) -> u32 {
        if self.count == 0 {
            return u32::MAX;
        }
        let hash = self.slot_hash(text);
        let mut slot = (hash as usize) & self.table_mask;
        loop {
            let (h, id) = self.table[slot];
            if h == 0 {
                return u32::MAX;
            }
            if h == hash && self.lookup_text(id) == text {
                return id;
            }
            slot = (slot + 1) & self.table_mask;
        }
    }

    fn store_string(&mut self, text: &[u8]) {
        let offset = u32::try_from(self.storage.len())
            .expect("string storage exceeds the 4 GiB addressable by u32 offsets");
        let len = u32::try_from(text.len()).expect("a single string exceeds 4 GiB");
        self.offsets.push(offset);
        self.storage.extend_from_slice(&len.to_le_bytes());
        self.storage.extend_from_slice(text);
    }

    pub fn lookup_text(&self, text_id: u32) -> &[u8] {
        if text_id == u32::MAX || (text_id as usize) >= self.offsets.len() {
            return b"";
        }
        let offset = self.offsets[text_id as usize] as usize;
        let Some(prefix) = self.storage.get(offset..offset + STRING_LEN_PREFIX_BYTES) else {
            return b"";
        };
        let len = u32::from_le_bytes([prefix[0], prefix[1], prefix[2], prefix[3]]) as usize;
        let start = offset + STRING_LEN_PREFIX_BYTES;
        self.storage.get(start..start + len).unwrap_or(b"")
    }

    /// Raw storage (u32 length-prefixed strings in id order).
    pub fn get_storage_bytes(&self) -> &[u8] {
        &self.storage
    }

    /// Byte offset of each string's length prefix in `get_storage_bytes`, indexed by id.
    pub fn get_offsets(&self) -> &[u32] {
        &self.offsets
    }

    fn resize(&mut self) {
        let new_cap = self.table.len() * 2;
        let new_mask = new_cap - 1;
        let mut new_table = vec![(0, u32::MAX); new_cap];

        for &(h, id) in &self.table {
            if h != 0 {
                let mut slot = (h as usize) & new_mask;
                while new_table[slot].0 != 0 {
                    slot = (slot + 1) & new_mask;
                }
                new_table[slot] = (h, id);
            }
        }

        self.table = new_table;
        self.table_mask = new_mask;
        self.load_limit = new_cap / 4 * 3;
    }
}

impl Default for StringInterner {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_interner_deduplication() {
        let mut interner = StringInterner::new();
        let id1 = interner.intern(b"class");
        let id2 = interner.intern(b"OpenHeart");
        let id3 = interner.intern(b"class");

        assert_eq!(id1, id3);
        assert_ne!(id1, id2);
        assert_eq!(interner.count(), 2);
        assert_eq!(interner.lookup_text(id1), b"class");
        assert_eq!(interner.lookup_text(id2), b"OpenHeart");
    }
}
