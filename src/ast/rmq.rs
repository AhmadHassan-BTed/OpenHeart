//! Range min-max tree over the BP excess sequence (Navarro & Sadakane, "Fully Functional
//! Static and Dynamic Succinct Trees", ACM TALG 2014), restricted to the minimum.
//!
//! The excess `E(i)` is `(#open - #close)` in `BP[0..=i]`, with `E(-1) = 0`. The structure
//! stores the minimum prefix excess of every 64-bit word (one i8, 12.5 % of the BP) and a
//! min segment tree over 512-bit superblocks (one i32 per superblock and internal node).
//! Absolute excess at any position comes from rank (`E(i) = 2 * rank1(i) - i - 1`), so no
//! per-bit array is kept. Together with `RankSelectIndex` this supports `find_close`,
//! `find_open`, `enclose`, range-minimum and LCA in O(log n) word operations, replacing the
//! former O(n log n)-word sparse table and the 32-bit-per-bit jump table.

use super::bp_encoder::BPEncoder;
use super::rank_select::{masked_word, valid_bits, RankSelectIndex, WORDS_PER_SUPERBLOCK};

const SUPERBLOCK_BITS: usize = 64 * WORDS_PER_SUPERBLOCK;

const fn build_byte_excess() -> [i8; 256] {
    let mut table = [0i8; 256];
    let mut b = 0;
    while b < 256 {
        table[b] = 2 * (b as u8).count_ones() as i8 - 8;
        b += 1;
    }
    table
}

const fn build_byte_min() -> [i8; 256] {
    let mut table = [0i8; 256];
    let mut b = 0;
    while b < 256 {
        let mut e: i8 = 0;
        let mut m: i8 = i8::MAX;
        let mut bit = 0;
        while bit < 8 {
            if (b >> (7 - bit)) & 1 == 1 {
                e += 1;
            } else {
                e -= 1;
            }
            if e < m {
                m = e;
            }
            bit += 1;
        }
        table[b] = m;
        b += 1;
    }
    table
}

/// Excess change across each byte value (MSB-first).
static BYTE_EXCESS: [i8; 256] = build_byte_excess();
/// Minimum prefix excess inside each byte value (MSB-first), over prefixes of length 1..=8.
static BYTE_MIN: [i8; 256] = build_byte_min();

#[inline]
fn bit_at(bp: &BPEncoder, p: usize) -> bool {
    bp.get_bit(p) == 1
}

#[inline]
fn step(bit_is_open: bool) -> i64 {
    if bit_is_open {
        1
    } else {
        -1
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RangeMinMaxTree {
    /// Minimum prefix excess inside each word, relative to the excess before the word.
    pub word_min: Vec<i8>,
    /// Min segment tree over superblocks; leaf `s` is `tree[leaf_base + s]`, padding leaves
    /// hold `i32::MAX`, the root is `tree[1]`.
    pub tree: Vec<i32>,
    pub leaf_base: usize,
    pub superblock_count: usize,
    pub n_bits: usize,
}

/// Former name of the LCA / range-minimum structure, kept for API compatibility.
pub type SparseTableRMQ = RangeMinMaxTree;

impl RangeMinMaxTree {
    pub fn build(bp: &BPEncoder, _rs: &RankSelectIndex) -> Self {
        let n_bits = bp.bit_count;
        let n_words = n_bits.div_ceil(64);
        let mut word_min = Vec::with_capacity(n_words);
        for w in 0..n_words {
            word_min.push(Self::word_min_prefix(
                masked_word(bp, w, n_bits),
                valid_bits(w, n_bits),
            ));
        }

        let superblock_count = n_words.div_ceil(WORDS_PER_SUPERBLOCK);
        let leaf_base = superblock_count.max(1).next_power_of_two();
        let mut tree = vec![i32::MAX; 2 * leaf_base];
        let mut before: i64 = 0;
        for sb in 0..superblock_count {
            let mut m = i64::MAX;
            let first = sb * WORDS_PER_SUPERBLOCK;
            let last = (first + WORDS_PER_SUPERBLOCK).min(n_words);
            for w in first..last {
                m = m.min(before + word_min[w] as i64);
                before += Self::word_delta(bp, w, n_bits);
            }
            tree[leaf_base + sb] = Self::clamp(m);
        }
        for i in (1..leaf_base).rev() {
            tree[i] = tree[2 * i].min(tree[2 * i + 1]);
        }

        Self {
            word_min,
            tree,
            leaf_base,
            superblock_count,
            n_bits,
        }
    }

    #[inline]
    fn clamp(v: i64) -> i32 {
        v.clamp(i32::MIN as i64, i32::MAX as i64) as i32
    }

    /// Minimum prefix excess of the first `valid` bits of `word` (MSB-first).
    fn word_min_prefix(word: u64, valid: usize) -> i8 {
        if valid == 0 {
            return 0;
        }
        let mut e: i32 = 0;
        let mut m: i32 = i32::MAX;
        if valid == 64 {
            for byte_idx in 0..8 {
                let byte = ((word >> (56 - 8 * byte_idx)) & 0xFF) as usize;
                m = m.min(e + BYTE_MIN[byte] as i32);
                e += BYTE_EXCESS[byte] as i32;
            }
        } else {
            for b in 0..valid {
                e += if (word >> (63 - b)) & 1 == 1 { 1 } else { -1 };
                m = m.min(e);
            }
        }
        m as i8
    }

    #[inline]
    fn word_delta(bp: &BPEncoder, w: usize, n_bits: usize) -> i64 {
        let valid = valid_bits(w, n_bits) as i64;
        let ones = masked_word(bp, w, n_bits).count_ones() as i64;
        2 * ones - valid
    }

    /// `E(p - 1)`: excess before position `p` (`E(-1) = 0`).
    #[inline]
    pub fn excess_before(bp: &BPEncoder, rs: &RankSelectIndex, p: usize) -> i64 {
        let p = p.min(rs.n_bits);
        2 * rs.rank1_exclusive(bp, p) as i64 - p as i64
    }

    /// `E(p)`: excess after position `p`.
    #[inline]
    pub fn excess_at(bp: &BPEncoder, rs: &RankSelectIndex, p: usize) -> i64 {
        Self::excess_before(bp, rs, p + 1)
    }

    /// Scans word `w` forward with `e` = excess before the word. Returns the first
    /// position whose excess is `<= target`, or advances `e` past the word.
    fn scan_word_fwd(&self, bp: &BPEncoder, w: usize, e: &mut i64, target: i64) -> Option<usize> {
        if *e + self.word_min[w] as i64 > target {
            *e += Self::word_delta(bp, w, self.n_bits);
            return None;
        }
        let word = masked_word(bp, w, self.n_bits);
        let valid = valid_bits(w, self.n_bits);
        if valid == 64 {
            for byte_idx in 0..8 {
                let byte = ((word >> (56 - 8 * byte_idx)) & 0xFF) as usize;
                if *e + (BYTE_MIN[byte] as i64) <= target {
                    for bit in 0..8 {
                        *e += step(byte & (0x80 >> bit) != 0);
                        if *e <= target {
                            return Some(w * 64 + byte_idx * 8 + bit);
                        }
                    }
                }
                *e += BYTE_EXCESS[byte] as i64;
            }
        } else {
            for b in 0..valid {
                *e += step((word >> (63 - b)) & 1 == 1);
                if *e <= target {
                    return Some(w * 64 + b);
                }
            }
        }
        None
    }

    /// Scans word `w` backward with `e` = excess at the word's last valid bit. Returns the
    /// last position whose excess is `<= target`, or leaves `e` = excess before the word.
    fn scan_word_bwd(&self, bp: &BPEncoder, w: usize, e: &mut i64, target: i64) -> Option<usize> {
        let delta = Self::word_delta(bp, w, self.n_bits);
        if *e - delta + self.word_min[w] as i64 > target {
            *e -= delta;
            return None;
        }
        let valid = valid_bits(w, self.n_bits);
        let mut b = valid;
        while b > 0 {
            b -= 1;
            let q = w * 64 + b;
            if *e <= target {
                return Some(q);
            }
            *e -= step(bit_at(bp, q));
        }
        None
    }

    /// First superblock `>= from` whose minimum excess is `<= target`.
    fn tree_next(&self, from: usize, target: i64) -> Option<usize> {
        if from >= self.superblock_count {
            return None;
        }
        let mut v = self.leaf_base + from;
        if (self.tree[v] as i64) <= target {
            return Some(from);
        }
        loop {
            if v <= 1 {
                return None;
            }
            if v % 2 == 0 && (self.tree[v + 1] as i64) <= target {
                v += 1;
                break;
            }
            v /= 2;
        }
        while v < self.leaf_base {
            let left = 2 * v;
            v = if (self.tree[left] as i64) <= target {
                left
            } else {
                left + 1
            };
        }
        let leaf = v - self.leaf_base;
        (leaf < self.superblock_count).then_some(leaf)
    }

    /// Last superblock `<= from` whose minimum excess is `<= target`.
    fn tree_prev(&self, from: usize, target: i64) -> Option<usize> {
        if from >= self.superblock_count {
            return None;
        }
        let mut v = self.leaf_base + from;
        if (self.tree[v] as i64) <= target {
            return Some(from);
        }
        loop {
            if v <= 1 {
                return None;
            }
            if v % 2 == 1 && (self.tree[v - 1] as i64) <= target {
                v -= 1;
                break;
            }
            v /= 2;
        }
        while v < self.leaf_base {
            let right = 2 * v + 1;
            v = if (self.tree[right] as i64) <= target {
                right
            } else {
                right - 1
            };
        }
        let leaf = v - self.leaf_base;
        (leaf < self.superblock_count).then_some(leaf)
    }

    /// Minimum over superblocks `l..=r`.
    fn tree_range_min(&self, l: usize, r: usize) -> i64 {
        let mut lo = l + self.leaf_base;
        let mut hi = r + self.leaf_base + 1;
        let mut m = i32::MAX;
        while lo < hi {
            if lo & 1 == 1 {
                m = m.min(self.tree[lo]);
                lo += 1;
            }
            if hi & 1 == 1 {
                hi -= 1;
                m = m.min(self.tree[hi]);
            }
            lo >>= 1;
            hi >>= 1;
        }
        m as i64
    }

    /// Smallest `q >= start` with `E(q) <= target`.
    pub fn fwd_search(
        &self,
        bp: &BPEncoder,
        rs: &RankSelectIndex,
        start: usize,
        target: i64,
    ) -> Option<usize> {
        if start >= self.n_bits {
            return None;
        }
        let n_words = self.word_min.len();
        let mut e = Self::excess_before(bp, rs, start);

        // Rest of the starting word.
        let w0 = start / 64;
        let valid = valid_bits(w0, self.n_bits);
        let word = masked_word(bp, w0, self.n_bits);
        for b in (start % 64)..valid {
            e += step((word >> (63 - b)) & 1 == 1);
            if e <= target {
                return Some(w0 * 64 + b);
            }
        }

        // Rest of the starting superblock.
        let sb0 = w0 / WORDS_PER_SUPERBLOCK;
        let sb_end = ((sb0 + 1) * WORDS_PER_SUPERBLOCK).min(n_words);
        for w in (w0 + 1)..sb_end {
            if let Some(q) = self.scan_word_fwd(bp, w, &mut e, target) {
                return Some(q);
            }
        }

        // Later superblocks through the tree.
        let sb = self.tree_next(sb0 + 1, target)?;
        let mut e = Self::excess_before(bp, rs, sb * SUPERBLOCK_BITS);
        let first = sb * WORDS_PER_SUPERBLOCK;
        let last = (first + WORDS_PER_SUPERBLOCK).min(n_words);
        for w in first..last {
            if let Some(q) = self.scan_word_fwd(bp, w, &mut e, target) {
                return Some(q);
            }
        }
        None
    }

    /// Largest `q < end` with `E(q) <= target`. `Some(-1)` denotes the virtual position
    /// before the sequence (`E(-1) = 0`), returned when no real position qualifies and
    /// `target >= 0`.
    pub fn bwd_search(
        &self,
        bp: &BPEncoder,
        rs: &RankSelectIndex,
        end: usize,
        target: i64,
    ) -> Option<isize> {
        let virtual_start = if target >= 0 { Some(-1isize) } else { None };
        let end = end.min(self.n_bits);
        if end == 0 {
            return virtual_start;
        }

        // Start word, from position end-1 backward.
        let mut q = end - 1;
        let w0 = q / 64;
        let mut e = Self::excess_at(bp, rs, q);
        loop {
            if e <= target {
                return Some(q as isize);
            }
            e -= step(bit_at(bp, q));
            if q % 64 == 0 {
                break;
            }
            q -= 1;
        }

        // Earlier words of the starting superblock.
        let sb0 = w0 / WORDS_PER_SUPERBLOCK;
        let mut w = w0;
        while w > sb0 * WORDS_PER_SUPERBLOCK {
            w -= 1;
            if let Some(p) = self.scan_word_bwd(bp, w, &mut e, target) {
                return Some(p as isize);
            }
        }
        if sb0 == 0 {
            return virtual_start;
        }

        // Earlier superblocks through the tree.
        let Some(sb) = self.tree_prev(sb0 - 1, target) else {
            return virtual_start;
        };
        let mut e = Self::excess_before(bp, rs, (sb + 1) * SUPERBLOCK_BITS);
        let first = sb * WORDS_PER_SUPERBLOCK;
        let mut w = (first + WORDS_PER_SUPERBLOCK).min(self.word_min.len());
        while w > first {
            w -= 1;
            if let Some(p) = self.scan_word_bwd(bp, w, &mut e, target) {
                return Some(p as isize);
            }
        }
        None
    }

    /// Minimum excess over positions `i..=j` (both `< n_bits`).
    fn min_excess(&self, bp: &BPEncoder, rs: &RankSelectIndex, i: usize, j: usize) -> i64 {
        let sbi = i / SUPERBLOCK_BITS;
        let sbj = j / SUPERBLOCK_BITS;
        if sbi == sbj {
            return self.scan_min(bp, rs, i, j);
        }
        let mut m = self.scan_min(bp, rs, i, (sbi + 1) * SUPERBLOCK_BITS - 1);
        if sbi + 1 < sbj {
            m = m.min(self.tree_range_min(sbi + 1, sbj - 1));
        }
        m.min(self.scan_min(bp, rs, sbj * SUPERBLOCK_BITS, j))
    }

    /// Minimum excess over `i..=j`, word-accelerated.
    fn scan_min(&self, bp: &BPEncoder, rs: &RankSelectIndex, i: usize, j: usize) -> i64 {
        let mut e = Self::excess_before(bp, rs, i);
        let mut m = i64::MAX;
        let mut p = i;
        while p <= j {
            if p % 64 == 0 && p + 63 <= j {
                let w = p / 64;
                m = m.min(e + self.word_min[w] as i64);
                e += Self::word_delta(bp, w, self.n_bits);
                p += 64;
            } else {
                e += step(bit_at(bp, p));
                m = m.min(e);
                p += 1;
            }
        }
        m
    }

    /// Range minimum query: leftmost position with minimum excess in `[l, r]`.
    pub fn rmq(&self, bp: &BPEncoder, rs: &RankSelectIndex, l: usize, r: usize) -> Option<usize> {
        let (l, r) = if l <= r { (l, r) } else { (r, l) };
        if r >= self.n_bits {
            return None;
        }
        let m = self.min_excess(bp, rs, l, r);
        self.fwd_search(bp, rs, l, m).filter(|&q| q <= r)
    }

    /// Range minimum query returning the position as `u32`; `u32::MAX` when the range
    /// lies outside the sequence.
    pub fn range_min(&self, bp: &BPEncoder, rs: &RankSelectIndex, l: usize, r: usize) -> u32 {
        self.rmq(bp, rs, l, r)
            .and_then(|p| u32::try_from(p).ok())
            .unwrap_or(u32::MAX)
    }

    /// Matching close parenthesis of the open parenthesis at `p`.
    pub fn find_close(&self, bp: &BPEncoder, rs: &RankSelectIndex, p: usize) -> Option<usize> {
        if p >= self.n_bits || !bit_at(bp, p) {
            return None;
        }
        let target = Self::excess_at(bp, rs, p) - 1;
        self.fwd_search(bp, rs, p + 1, target)
    }

    /// Matching open parenthesis of the close parenthesis at `q`.
    pub fn find_open(&self, bp: &BPEncoder, rs: &RankSelectIndex, q: usize) -> Option<usize> {
        if q >= self.n_bits || bit_at(bp, q) {
            return None;
        }
        let target = Self::excess_at(bp, rs, q);
        let r = self.bwd_search(bp, rs, q, target)?;
        usize::try_from(r + 1).ok()
    }

    /// Open parenthesis of the parent of the node opened at `p` (`None` for a root).
    pub fn enclose(&self, bp: &BPEncoder, rs: &RankSelectIndex, p: usize) -> Option<usize> {
        if p >= self.n_bits || !bit_at(bp, p) {
            return None;
        }
        let k = Self::excess_at(bp, rs, p);
        if k <= 1 {
            return None;
        }
        let r = self.bwd_search(bp, rs, p, k - 2)?;
        usize::try_from(r + 1).ok()
    }

    /// Lowest common ancestor of two nodes given by pre-order index (Sadakane & Navarro,
    /// SODA 2010): `u` if it encloses `v`, otherwise the parent of the node that opens just
    /// after the excess minimum between them. Returns `u32::MAX` when the nodes are in
    /// different trees of the forest or an index is out of range.
    pub fn lca(&self, bp: &BPEncoder, rs: &RankSelectIndex, u: u32, v: u32) -> u32 {
        if u == v {
            return if u < rs.ones { u } else { u32::MAX };
        }
        let (Some(pu), Some(pv)) = (
            rs.select1_checked(bp, u.saturating_add(1)),
            rs.select1_checked(bp, v.saturating_add(1)),
        ) else {
            return u32::MAX;
        };
        let (x, y, x_node) = if pu < pv { (pu, pv, u) } else { (pv, pu, v) };
        match self.find_close(bp, rs, x) {
            Some(close) if close > y => return x_node,
            Some(_) => {}
            None => return u32::MAX,
        }
        let Some(m) = self.rmq(bp, rs, x, y) else {
            return u32::MAX;
        };
        match self.enclose(bp, rs, m + 1) {
            Some(parent_open) => rs.rank1(bp, parent_open).saturating_sub(1),
            None => u32::MAX,
        }
    }

    /// Checks that the sequence is a well-formed forest: every prefix excess is
    /// non-negative and the total excess is zero.
    pub fn validate(&self, rs: &RankSelectIndex) -> Result<(), String> {
        if self.n_bits == 0 {
            return Ok(());
        }
        let total = 2 * rs.ones as i64 - self.n_bits as i64;
        if total != 0 {
            return Err(format!(
                "unbalanced parentheses: final excess {} (expected 0)",
                total
            ));
        }
        let min = self.tree.get(1).copied().unwrap_or(i32::MAX);
        if min < 0 {
            return Err(format!(
                "close parenthesis without a matching open (minimum excess {})",
                min
            ));
        }
        Ok(())
    }

    /// Heap bytes held by the structure.
    pub fn heap_bytes(&self) -> usize {
        self.word_min.len() + self.tree.len() * 4
    }
}
