//! PathCount — exact, arbitrary-precision path counts (§8.2.5).
//!
//! The number of paths through a method grows exponentially with its sequential decisions:
//! 64 independent `if` statements already exceed `u64`. Phase 8 therefore counts with an
//! unbounded unsigned integer and derives the persisted fixed-width forms from it
//! explicitly — a saturated `u64` together with an exactness flag, and `log2` as `f64` —
//! instead of silently clamping inside the counting algorithms.
//!
//! Only the operations the counting algorithms need are provided: addition, multiplication
//! by a power of two (variable gaps in #SAT), comparison and lossless/lossy conversions.

use std::cmp::Ordering;
use std::fmt;

/// Unbounded unsigned integer stored as little-endian 64-bit limbs.
///
/// Invariant: `limbs` never ends in a zero limb, so zero is the empty vector and equal
/// values have identical representations (derived `Eq`/`Hash` are therefore correct).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Default)]
pub struct PathCount {
    limbs: Vec<u64>,
}

impl PathCount {
    /// The value 0.
    #[inline]
    pub fn zero() -> Self {
        Self { limbs: Vec::new() }
    }

    /// The value 1.
    #[inline]
    pub fn one() -> Self {
        Self { limbs: vec![1] }
    }

    /// Lossless conversion from `u64`.
    #[inline]
    pub fn from_u64(value: u64) -> Self {
        if value == 0 {
            Self::zero()
        } else {
            Self { limbs: vec![value] }
        }
    }

    /// The value `2^bits`.
    pub fn pow2(bits: u32) -> Self {
        let mut c = Self::one();
        c.shl_assign(bits);
        c
    }

    /// True iff the value is 0.
    #[inline]
    pub fn is_zero(&self) -> bool {
        self.limbs.is_empty()
    }

    /// `self += other`.
    pub fn add_assign(&mut self, other: &PathCount) {
        if other.limbs.len() > self.limbs.len() {
            self.limbs.resize(other.limbs.len(), 0);
        }
        let mut carry = 0u64;
        for i in 0..self.limbs.len() {
            if i >= other.limbs.len() && carry == 0 {
                break;
            }
            let rhs = other.limbs.get(i).copied().unwrap_or(0);
            let (s1, c1) = self.limbs[i].overflowing_add(rhs);
            let (s2, c2) = s1.overflowing_add(carry);
            self.limbs[i] = s2;
            carry = (c1 as u64) + (c2 as u64);
        }
        if carry != 0 {
            self.limbs.push(carry);
        }
    }

    /// `self + other`.
    pub fn plus(&self, other: &PathCount) -> PathCount {
        let mut r = self.clone();
        r.add_assign(other);
        r
    }

    /// `self *= 2^bits`.
    pub fn shl_assign(&mut self, bits: u32) {
        if self.is_zero() || bits == 0 {
            return;
        }
        let limb_shift = (bits / 64) as usize;
        let bit_shift = bits % 64;
        if bit_shift != 0 {
            let mut carry = 0u64;
            for limb in self.limbs.iter_mut() {
                let next_carry = *limb >> (64 - bit_shift);
                *limb = (*limb << bit_shift) | carry;
                carry = next_carry;
            }
            if carry != 0 {
                self.limbs.push(carry);
            }
        }
        if limb_shift > 0 {
            let mut shifted = vec![0u64; limb_shift];
            shifted.extend_from_slice(&self.limbs);
            self.limbs = shifted;
        }
    }

    /// `self * 2^bits`.
    pub fn shifted(&self, bits: u32) -> PathCount {
        let mut r = self.clone();
        r.shl_assign(bits);
        r
    }

    /// The exact value if it fits in `u64`, otherwise `None`.
    #[inline]
    pub fn to_u64(&self) -> Option<u64> {
        match self.limbs.len() {
            0 => Some(0),
            1 => Some(self.limbs[0]),
            _ => None,
        }
    }

    /// The value clamped to `u64::MAX`. Callers must record that the value was clamped
    /// (see [`PathCount::fits_u64`]); this method never hides it on its own.
    #[inline]
    pub fn saturating_u64(&self) -> u64 {
        self.to_u64().unwrap_or(u64::MAX)
    }

    /// True iff the value is representable as `u64` without loss.
    #[inline]
    pub fn fits_u64(&self) -> bool {
        self.limbs.len() <= 1
    }

    /// Number of significant bits (0 for the value 0).
    pub fn bit_len(&self) -> u64 {
        match self.limbs.last() {
            None => 0,
            Some(&top) => 64 * (self.limbs.len() as u64 - 1) + (64 - top.leading_zeros() as u64),
        }
    }

    /// `log2(self)` as `f64`; negative infinity for 0. Exact for powers of two and accurate
    /// to `f64` precision otherwise (the top 64 significant bits are used).
    pub fn log2(&self) -> f64 {
        let n = self.bit_len();
        if n == 0 {
            return f64::NEG_INFINITY;
        }
        if n <= 64 {
            return (self.limbs[0] as f64).log2();
        }
        let shift = n - 64;
        let idx = (shift / 64) as usize;
        let off = (shift % 64) as u32;
        let mut top = self.limbs[idx] >> off;
        if off > 0 && idx + 1 < self.limbs.len() {
            top |= self.limbs[idx + 1] << (64 - off);
        }
        (top as f64).log2() + shift as f64
    }
}

impl PartialOrd for PathCount {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PathCount {
    fn cmp(&self, other: &Self) -> Ordering {
        self.limbs
            .len()
            .cmp(&other.limbs.len())
            .then_with(|| self.limbs.iter().rev().cmp(other.limbs.iter().rev()))
    }
}

impl fmt::Display for PathCount {
    /// Exact decimal rendering.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_zero() {
            return f.write_str("0");
        }
        // Repeated division by 10^19 (the largest power of ten below 2^64).
        const CHUNK: u128 = 10_000_000_000_000_000_000;
        let mut work = self.limbs.clone();
        let mut chunks: Vec<u64> = Vec::new();
        while !work.is_empty() {
            let mut rem: u128 = 0;
            for limb in work.iter_mut().rev() {
                let cur = (rem << 64) | (*limb as u128);
                *limb = (cur / CHUNK) as u64;
                rem = cur % CHUNK;
            }
            chunks.push(rem as u64);
            while work.last() == Some(&0) {
                work.pop();
            }
        }
        let mut iter = chunks.iter().rev();
        if let Some(most_significant) = iter.next() {
            write!(f, "{}", most_significant)?;
        }
        for chunk in iter {
            write!(f, "{:019}", chunk)?;
        }
        Ok(())
    }
}
