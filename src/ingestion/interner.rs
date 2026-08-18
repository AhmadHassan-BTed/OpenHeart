const FNV1A_PRIME: u64 = 0x0000_0100_0000_01B3;
const FNV1A_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;

/// FNV-1a 64-bit hash function.
#[inline]
pub fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash = FNV1A_OFFSET;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(FNV1A_PRIME);
    }
    hash
}

/// Deduplicating String Interner using FNV-1a hash table with open addressing.
#[derive(Debug, Clone)]
pub struct StringInterner {
    table: Vec<(u64, u32)>,
    table_mask: usize,
    count: u32,
    load_limit: usize,
    storage: Vec<u8>,
    offsets: Vec<u32>,
}

impl StringInterner {
    pub fn with_capacity(initial_capacity: usize) -> Self {
        let cap = initial_capacity.next_power_of_two().max(16);
        Self {
            table: vec![(0, u32::MAX); cap],
            table_mask: cap - 1,
            count: 0,
            load_limit: (cap as f64 * 0.75) as usize,
            storage: Vec::new(),
            offsets: Vec::new(),
        }
    }

    pub fn new() -> Self {
        Self::with_capacity(1024)
    }

    pub fn count(&self) -> u32 {
        self.count
    }

    pub fn intern(&mut self, text: &[u8]) -> u32 {
        let mut hash = fnv1a_64(text);
        if hash == 0 {
            hash = 1;
        }

        let mut slot = (hash as usize) & self.table_mask;
        loop {
            let (h, id) = self.table[slot];
            if h == 0 {
                // Empty slot: insert new string
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
        let mut hash = fnv1a_64(text);
        if hash == 0 {
            hash = 1;
        }
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
        let offset = self.storage.len() as u32;
        self.offsets.push(offset);

        let len = (text.len().min(u16::MAX as usize)) as u16;
        self.storage.extend_from_slice(&len.to_le_bytes());
        self.storage.extend_from_slice(&text[..len as usize]);
    }

    pub fn lookup_text(&self, text_id: u32) -> &[u8] {
        if text_id == u32::MAX || (text_id as usize) >= self.offsets.len() {
            return b"";
        }
        let offset = self.offsets[text_id as usize] as usize;
        if offset + 2 > self.storage.len() {
            return b"";
        }
        let len = u16::from_le_bytes([self.storage[offset], self.storage[offset + 1]]) as usize;
        if offset + 2 + len > self.storage.len() {
            return b"";
        }
        &self.storage[offset + 2..offset + 2 + len]
    }

    pub fn get_storage_bytes(&self) -> &[u8] {
        &self.storage
    }

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
        self.load_limit = (new_cap as f64 * 0.75) as usize;
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
