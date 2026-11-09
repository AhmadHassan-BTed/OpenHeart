//! Rank/Select index over BP bitstrings (rank9 layout with sampled select).
//!
//! Per 512-bit superblock the index stores one u32 cumulative count and one u64 holding
//! seven 9-bit in-superblock counts (Vigna, "Broadword Implementation of Rank/Select
//! Queries", WEA 2008), i.e. 96 bits per 512 bits (18.75 %). `select1` binary-searches the
//! superblock counts inside a window narrowed by one sample per `SELECT_SAMPLE_RATE` ones,
//! then finishes with the packed counts and an in-word select.
//!
//! Bits are stored MSB-first in each u64 word (see `BPEncoder`). Bits at or beyond
//! `bit_count` are treated as zero even if the backing words contain garbage.

use super::bp_encoder::BPEncoder;

pub const SUPERBLOCK_BITS: usize = 512;
pub const WORDS_PER_SUPERBLOCK: usize = SUPERBLOCK_BITS / 64;
/// One select sample per this many 1-bits.
pub const SELECT_SAMPLE_RATE: u32 = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RankSelectIndex {
    /// Number of 1-bits before each superblock, followed by the total.
    pub superblocks: Vec<u32>,
    /// For each superblock, bits `9(k-1)..9k` hold the 1-bits in its words `0..k` (k = 1..=7).
    pub sub_counts: Vec<u64>,
    /// Superblock containing the `(i * SELECT_SAMPLE_RATE + 1)`-th 1-bit.
    pub select_samples: Vec<u32>,
    /// Total number of 1-bits.
    pub ones: u32,
    pub n_bits: usize,
}

/// Number of valid bits in word `w` of an `n_bits`-bit string.
#[inline]
pub(crate) fn valid_bits(w: usize, n_bits: usize) -> usize {
    let start = w * 64;
    if start >= n_bits {
        0
    } else {
        (n_bits - start).min(64)
    }
}

/// Word `w` with every bit at or beyond `n_bits` cleared.
#[inline]
pub(crate) fn masked_word(bp: &BPEncoder, w: usize, n_bits: usize) -> u64 {
    let word = bp.words.get(w).copied().unwrap_or(0);
    match valid_bits(w, n_bits) {
        0 => 0,
        64 => word,
        valid => word & (!0u64 << (64 - valid)),
    }
}

/// Position (0..64, MSB-first) of the `r`-th (1-based) set bit of `word`.
#[inline]
pub(crate) fn select_in_word(word: u64, r: u32) -> Option<u32> {
    if r == 0 || r > word.count_ones() {
        return None;
    }
    let mut remaining = r;
    for byte_idx in 0..8u32 {
        let byte = ((word >> (56 - 8 * byte_idx)) & 0xFF) as u8;
        let count = byte.count_ones();
        if remaining <= count {
            for bit in 0..8u32 {
                if byte & (0x80u8 >> bit) != 0 {
                    remaining -= 1;
                    if remaining == 0 {
                        return Some(byte_idx * 8 + bit);
                    }
                }
            }
        }
        remaining -= count;
    }
    None
}

impl RankSelectIndex {
    pub fn build(bp: &BPEncoder) -> Self {
        let n_bits = bp.bit_count;
        let n_words = n_bits.div_ceil(64);
        let n_sb = n_words.div_ceil(WORDS_PER_SUPERBLOCK);

        let mut superblocks = Vec::with_capacity(n_sb + 1);
        let mut sub_counts = Vec::with_capacity(n_sb);
        let mut select_samples = Vec::new();
        let mut cumulative: u64 = 0;
        let mut next_sample: u64 = 1;

        for sb in 0..n_sb {
            superblocks.push(u32::try_from(cumulative).unwrap_or(u32::MAX));
            let mut within: u64 = 0;
            let mut packed: u64 = 0;
            for k in 0..WORDS_PER_SUPERBLOCK {
                if k > 0 {
                    packed |= within << (9 * (k - 1));
                }
                let w = sb * WORDS_PER_SUPERBLOCK + k;
                if w < n_words {
                    within += masked_word(bp, w, n_bits).count_ones() as u64;
                }
            }
            sub_counts.push(packed);
            let end = cumulative + within;
            while next_sample <= end {
                select_samples.push(sb as u32);
                next_sample += SELECT_SAMPLE_RATE as u64;
            }
            cumulative = end;
        }
        superblocks.push(u32::try_from(cumulative).unwrap_or(u32::MAX));

        RankSelectIndex {
            superblocks,
            sub_counts,
            select_samples,
            ones: u32::try_from(cumulative).unwrap_or(u32::MAX),
            n_bits,
        }
    }

    /// Number of 1-bits in `BP[0..p)` (exclusive), for `p <= n_bits`.
    pub fn rank1_exclusive(&self, bp: &BPEncoder, p: usize) -> u32 {
        let p = p.min(self.n_bits);
        let w = p / 64;
        let b = p % 64;
        let sb = w / WORDS_PER_SUPERBLOCK;
        let k = w % WORDS_PER_SUPERBLOCK;
        if sb >= self.sub_counts.len() {
            return self.ones;
        }
        let mut r = self.superblocks[sb] as u64;
        if k > 0 {
            r += (self.sub_counts[sb] >> (9 * (k - 1))) & 0x1FF;
        }
        if b > 0 {
            let word = masked_word(bp, w, self.n_bits);
            r += (word >> (64 - b)).count_ones() as u64;
        }
        u32::try_from(r).unwrap_or(u32::MAX)
    }

    /// O(1) rank1 query: returns number of 1-bits in BP[0..=i] (i is clamped to the last bit).
    pub fn rank1(&self, bp: &BPEncoder, i: usize) -> u32 {
        if self.n_bits == 0 {
            return 0;
        }
        let pos = i.min(self.n_bits - 1);
        self.rank1_exclusive(bp, pos + 1)
    }

    /// Position of the `target_rank`-th (1-based) 1-bit, or `None` when `target_rank` is 0
    /// or exceeds the number of 1-bits.
    pub fn select1_checked(&self, bp: &BPEncoder, target_rank: u32) -> Option<usize> {
        if target_rank == 0 || target_rank > self.ones || self.sub_counts.is_empty() {
            return None;
        }
        let sample = ((target_rank - 1) / SELECT_SAMPLE_RATE) as usize;
        let lo = self.select_samples.get(sample).copied().unwrap_or(0) as usize;
        let hi = self
            .select_samples
            .get(sample + 1)
            .map(|&s| s as usize + 1)
            .unwrap_or(self.sub_counts.len())
            .min(self.sub_counts.len());
        if lo >= hi {
            return None;
        }

        // Last superblock in [lo, hi) whose cumulative count is below the target.
        let rel = self.superblocks[lo..hi].partition_point(|&c| c < target_rank);
        let sb = lo + rel.checked_sub(1)?;

        let mut remaining = target_rank - self.superblocks[sb];
        let packed = self.sub_counts[sb];
        let mut k = 0usize;
        let mut before = 0u32;
        for kk in 1..WORDS_PER_SUPERBLOCK {
            let c = ((packed >> (9 * (kk - 1))) & 0x1FF) as u32;
            if c < remaining {
                k = kk;
                before = c;
            } else {
                break;
            }
        }
        remaining -= before;

        let word_idx = sb * WORDS_PER_SUPERBLOCK + k;
        let bit = select_in_word(masked_word(bp, word_idx, self.n_bits), remaining)?;
        Some(word_idx * 64 + bit as usize)
    }

    /// Position of the `target_rank`-th (1-based) 1-bit. Absent ranks (0 or more than the
    /// number of 1-bits) return `bp.bit_count`, a one-past-the-end sentinel that is never a
    /// valid position; use `select1_checked` to receive `None` instead.
    pub fn select1(&self, bp: &BPEncoder, target_rank: u32) -> usize {
        self.select1_checked(bp, target_rank)
            .unwrap_or(bp.bit_count)
    }

    /// Heap bytes held by the index.
    pub fn heap_bytes(&self) -> usize {
        self.superblocks.len() * 4 + self.sub_counts.len() * 8 + self.select_samples.len() * 4
    }
}
