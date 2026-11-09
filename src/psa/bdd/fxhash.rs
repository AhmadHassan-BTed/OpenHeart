//! Fast non-cryptographic hashing for the BDD hot tables (§8.2.1).
//!
//! The unique table and the computed tables are keyed by small tuples of node ids that the
//! library itself allocates sequentially, so the DoS resistance of SipHash buys nothing here
//! while its per-lookup cost dominates `apply()`. This is the multiply-rotate "Fx" hash used
//! by rustc for the same kind of integer-keyed tables.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

/// Multiplicative constant of the Fx hash (golden-ratio derived, odd).
const SEED: u64 = 0x51_7c_c1_b7_27_22_0a_95;

/// Fx hasher state.
#[derive(Default, Clone, Copy)]
pub struct FxHasher {
    hash: u64,
}

impl FxHasher {
    #[inline]
    fn add_to_hash(&mut self, word: u64) {
        self.hash = (self.hash.rotate_left(5) ^ word).wrapping_mul(SEED);
    }
}

impl Hasher for FxHasher {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        for chunk in bytes.chunks(8) {
            let mut buf = [0u8; 8];
            buf[..chunk.len()].copy_from_slice(chunk);
            self.add_to_hash(u64::from_le_bytes(buf));
        }
    }

    #[inline]
    fn write_u8(&mut self, i: u8) {
        self.add_to_hash(i as u64);
    }

    #[inline]
    fn write_u16(&mut self, i: u16) {
        self.add_to_hash(i as u64);
    }

    #[inline]
    fn write_u32(&mut self, i: u32) {
        self.add_to_hash(i as u64);
    }

    #[inline]
    fn write_u64(&mut self, i: u64) {
        self.add_to_hash(i);
    }

    #[inline]
    fn write_usize(&mut self, i: usize) {
        self.add_to_hash(i as u64);
    }

    #[inline]
    fn finish(&self) -> u64 {
        self.hash
    }
}

/// `BuildHasher` for [`FxHasher`].
pub type FxBuildHasher = BuildHasherDefault<FxHasher>;

/// `HashMap` using the Fx hash.
pub type FxHashMap<K, V> = HashMap<K, V, FxBuildHasher>;

/// Create an empty [`FxHashMap`] with room for `capacity` entries.
#[inline]
pub fn fx_map_with_capacity<K, V>(capacity: usize) -> FxHashMap<K, V> {
    HashMap::with_capacity_and_hasher(capacity, FxBuildHasher::default())
}
