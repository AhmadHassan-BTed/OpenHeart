//! Lattices for abstract interpretation (Cousot & Cousot 1977).
//!
//! An analysis domain is a join-semilattice with a least element `⊥` meaning "no execution
//! reaches this point / no value yet". The solvers only ever *join* states and, at loop
//! heads, *widen* them; the optional descending pass *narrows*. `meet` and `leq` are used by
//! branch refinement and by the solvers' stabilisation checks.
//!
//! Reusable domains:
//!
//! | lattice            | carrier                                   | height      |
//! |--------------------|-------------------------------------------|-------------|
//! | [`BitSet`]         | powerset of dense indices                 | finite      |
//! | [`MapLattice`]     | `K → V`, pointwise, missing key = `V::⊥`  | per `V`     |
//! | [`Flat`]           | `⊥ < c₁, c₂, … < ⊤` (constant lattice)    | 3           |
//! | [`Interval`]       | `[lo, hi]` over `ℤ ∪ {±∞}`                | infinite    |
//! | [`Product`]        | componentwise pair                        | sum         |
//! | [`MustSet`]        | dual powerset (join = intersection)       | finite      |
//! | [`BoundedSet`]     | sets of at most `K` elements, then `⊤`    | `K + 2`     |
//!
//! Every mutating operation returns `true` exactly when the receiver changed, which is what
//! the worklist solvers test to decide whether successors must be revisited.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

// ── The lattice interface ────────────────────────────────────────────────────

/// A join-semilattice with a least element, a meet, an order, and widening / narrowing.
pub trait Lattice: Clone + PartialEq + fmt::Debug {
    /// The least element: no information, no execution reaches the point.
    fn bottom() -> Self;

    /// True iff `self` is the least element.
    fn is_bottom(&self) -> bool;

    /// `self := self ⊔ other`; returns true when `self` changed.
    fn join(&mut self, other: &Self) -> bool;

    /// `self := self ⊓ other`; returns true when `self` changed.
    fn meet(&mut self, other: &Self) -> bool;

    /// `self ⊑ other`.
    fn leq(&self, other: &Self) -> bool;

    /// `self := self ∇ newer`. The result is an upper bound of both operands and every chain
    /// `x₀, x₁ = x₀ ∇ y₀, x₂ = x₁ ∇ y₁, …` stabilises after finitely many steps. The default
    /// is the join, which is a widening for lattices without infinite ascending chains.
    fn widen(&mut self, newer: &Self) -> bool {
        self.join(newer)
    }

    /// `self := self Δ newer` for `newer ⊑ self`. The result lies between `newer` and `self`
    /// and every decreasing chain built this way stabilises. The default replaces `self` by
    /// `newer` when `newer ⊑ self`, which is a narrowing for lattices without infinite
    /// descending chains; an incomparable `newer` leaves `self` unchanged.
    fn narrow(&mut self, newer: &Self) -> bool {
        if newer != self && newer.leq(self) {
            *self = newer.clone();
            true
        } else {
            false
        }
    }
}

/// The two-point lattice `false ⊑ true` (reachability).
impl Lattice for bool {
    fn bottom() -> Self {
        false
    }

    fn is_bottom(&self) -> bool {
        !*self
    }

    fn join(&mut self, other: &Self) -> bool {
        let changed = !*self && *other;
        *self |= *other;
        changed
    }

    fn meet(&mut self, other: &Self) -> bool {
        let changed = *self && !*other;
        *self &= *other;
        changed
    }

    fn leq(&self, other: &Self) -> bool {
        !*self || *other
    }
}

// ── Powerset over dense indices ──────────────────────────────────────────────

/// A growable bit set: the powerset lattice over `0..∞`, ordered by inclusion.
///
/// Invariant: the last word is never zero, so structural equality is set equality and the
/// empty set has no words. The universe needs no declared size, which lets [`Lattice::bottom`]
/// be the empty set for every analysis.
#[derive(Clone, Default, PartialEq, Eq, Hash)]
pub struct BitSet {
    words: Vec<u64>,
}

impl BitSet {
    /// The empty set.
    pub fn new() -> Self {
        Self { words: Vec::new() }
    }

    /// The empty set with room for indices below `bits` without reallocation.
    pub fn with_capacity(bits: usize) -> Self {
        Self {
            words: Vec::with_capacity(bits.div_ceil(64)),
        }
    }

    /// `{i}`.
    pub fn singleton(i: usize) -> Self {
        let mut s = Self::new();
        s.insert(i);
        s
    }

    /// `{0, 1, …, n - 1}`.
    pub fn full(n: usize) -> Self {
        let mut words = vec![u64::MAX; n / 64];
        let rest = n % 64;
        if rest != 0 {
            words.push((1u64 << rest) - 1);
        }
        let mut s = Self { words };
        s.trim();
        s
    }

    #[inline]
    pub fn contains(&self, i: usize) -> bool {
        let w = i / 64;
        w < self.words.len() && (self.words[w] >> (i % 64)) & 1 == 1
    }

    /// Adds `i`; returns true when it was absent.
    #[inline]
    pub fn insert(&mut self, i: usize) -> bool {
        let w = i / 64;
        if w >= self.words.len() {
            self.words.resize(w + 1, 0);
        }
        let mask = 1u64 << (i % 64);
        let absent = self.words[w] & mask == 0;
        self.words[w] |= mask;
        absent
    }

    /// Removes `i`; returns true when it was present.
    #[inline]
    pub fn remove(&mut self, i: usize) -> bool {
        let w = i / 64;
        if w >= self.words.len() {
            return false;
        }
        let mask = 1u64 << (i % 64);
        let present = self.words[w] & mask != 0;
        self.words[w] &= !mask;
        self.trim();
        present
    }

    /// Number of elements.
    pub fn len(&self) -> usize {
        self.words.iter().map(|w| w.count_ones() as usize).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }

    pub fn clear(&mut self) {
        self.words.clear();
    }

    /// Elements in increasing order.
    pub fn iter(&self) -> BitIter<'_> {
        BitIter {
            words: &self.words,
            word: 0,
            current: self.words.first().copied().unwrap_or(0),
        }
    }

    /// `self ∪= other`; returns true when `self` grew.
    pub fn union_with(&mut self, other: &BitSet) -> bool {
        if other.words.len() > self.words.len() {
            self.words.resize(other.words.len(), 0);
        }
        let mut changed = false;
        for (mine, theirs) in self.words.iter_mut().zip(other.words.iter()) {
            let next = *mine | *theirs;
            changed |= next != *mine;
            *mine = next;
        }
        changed
    }

    /// `self ∩= other`; returns true when `self` shrank.
    pub fn intersect_with(&mut self, other: &BitSet) -> bool {
        let mut changed = false;
        if self.words.len() > other.words.len() {
            changed = self.words[other.words.len()..].iter().any(|&w| w != 0);
            self.words.truncate(other.words.len());
        }
        for (mine, theirs) in self.words.iter_mut().zip(other.words.iter()) {
            let next = *mine & *theirs;
            changed |= next != *mine;
            *mine = next;
        }
        self.trim();
        changed
    }

    /// `self −= other`; returns true when `self` shrank.
    pub fn subtract(&mut self, other: &BitSet) -> bool {
        let mut changed = false;
        for (mine, theirs) in self.words.iter_mut().zip(other.words.iter()) {
            let next = *mine & !*theirs;
            changed |= next != *mine;
            *mine = next;
        }
        self.trim();
        changed
    }

    /// `self ⊆ other`.
    pub fn is_subset(&self, other: &BitSet) -> bool {
        if self.words.len() > other.words.len() {
            return false;
        }
        self.words
            .iter()
            .zip(other.words.iter())
            .all(|(mine, theirs)| mine & !theirs == 0)
    }

    /// `self ∩ other ≠ ∅`.
    pub fn intersects(&self, other: &BitSet) -> bool {
        self.words
            .iter()
            .zip(other.words.iter())
            .any(|(mine, theirs)| mine & theirs != 0)
    }

    fn trim(&mut self) {
        while self.words.last() == Some(&0) {
            self.words.pop();
        }
    }
}

/// Iterator over the elements of a [`BitSet`] in increasing order.
pub struct BitIter<'a> {
    words: &'a [u64],
    word: usize,
    current: u64,
}

impl Iterator for BitIter<'_> {
    type Item = usize;

    fn next(&mut self) -> Option<usize> {
        loop {
            if self.current != 0 {
                let bit = self.current.trailing_zeros() as usize;
                self.current &= self.current - 1;
                return Some(self.word * 64 + bit);
            }
            self.word += 1;
            if self.word >= self.words.len() {
                return None;
            }
            self.current = self.words[self.word];
        }
    }
}

impl FromIterator<usize> for BitSet {
    fn from_iter<I: IntoIterator<Item = usize>>(iter: I) -> Self {
        let mut s = BitSet::new();
        for i in iter {
            s.insert(i);
        }
        s
    }
}

impl Extend<usize> for BitSet {
    fn extend<I: IntoIterator<Item = usize>>(&mut self, iter: I) {
        for i in iter {
            self.insert(i);
        }
    }
}

impl fmt::Debug for BitSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_set().entries(self.iter()).finish()
    }
}

impl Lattice for BitSet {
    fn bottom() -> Self {
        BitSet::new()
    }

    fn is_bottom(&self) -> bool {
        self.is_empty()
    }

    fn join(&mut self, other: &Self) -> bool {
        self.union_with(other)
    }

    fn meet(&mut self, other: &Self) -> bool {
        self.intersect_with(other)
    }

    fn leq(&self, other: &Self) -> bool {
        self.is_subset(other)
    }
}

// ── Map lattice ──────────────────────────────────────────────────────────────

/// Pointwise lattice of finite maps `K → V`; an absent key maps to `V::bottom()`.
///
/// Invariant: no stored value is bottom, so structural equality is lattice equality.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct MapLattice<K: Ord, V> {
    map: BTreeMap<K, V>,
}

impl<K: Ord, V> Default for MapLattice<K, V> {
    fn default() -> Self {
        Self {
            map: BTreeMap::new(),
        }
    }
}

impl<K: Ord + Clone + fmt::Debug, V: Lattice> MapLattice<K, V> {
    pub fn new() -> Self {
        Self::default()
    }

    /// The stored value of `key` (`None` means bottom).
    pub fn get(&self, key: &K) -> Option<&V> {
        self.map.get(key)
    }

    /// The value of `key`, bottom when absent.
    pub fn value(&self, key: &K) -> V {
        self.map.get(key).cloned().unwrap_or_else(V::bottom)
    }

    /// Strong update: `key ↦ value` (a bottom value removes the key).
    pub fn set(&mut self, key: K, value: V) {
        if value.is_bottom() {
            self.map.remove(&key);
        } else {
            self.map.insert(key, value);
        }
    }

    /// Weak update: `key ↦ old ⊔ value`; returns true when the entry changed.
    pub fn join_at(&mut self, key: K, value: &V) -> bool {
        if value.is_bottom() {
            return false;
        }
        match self.map.get_mut(&key) {
            Some(mine) => mine.join(value),
            None => {
                self.map.insert(key, value.clone());
                true
            }
        }
    }

    /// Refinement: `key ↦ old ⊓ value`; returns true when the entry changed.
    pub fn meet_at(&mut self, key: &K, value: &V) -> bool {
        let changed = match self.map.get_mut(key) {
            Some(mine) => mine.meet(value),
            None => false,
        };
        if self.map.get(key).is_some_and(|v| v.is_bottom()) {
            self.map.remove(key);
        }
        changed
    }

    pub fn remove(&mut self, key: &K) -> Option<V> {
        self.map.remove(key)
    }

    pub fn contains_key(&self, key: &K) -> bool {
        self.map.contains_key(key)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&K, &V)> {
        self.map.iter()
    }

    pub fn keys(&self) -> impl Iterator<Item = &K> {
        self.map.keys()
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Keeps only the entries for which `keep` holds.
    pub fn retain(&mut self, mut keep: impl FnMut(&K, &V) -> bool) {
        self.map.retain(|k, v| keep(k, v));
    }
}

impl<K: Ord + fmt::Debug, V: fmt::Debug> fmt::Debug for MapLattice<K, V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map().entries(self.map.iter()).finish()
    }
}

impl<K: Ord + Clone + fmt::Debug, V: Lattice> Lattice for MapLattice<K, V> {
    fn bottom() -> Self {
        Self::default()
    }

    fn is_bottom(&self) -> bool {
        self.map.is_empty()
    }

    fn join(&mut self, other: &Self) -> bool {
        let mut changed = false;
        for (k, v) in &other.map {
            match self.map.get_mut(k) {
                Some(mine) => changed |= mine.join(v),
                None => {
                    self.map.insert(k.clone(), v.clone());
                    changed = true;
                }
            }
        }
        changed
    }

    fn meet(&mut self, other: &Self) -> bool {
        let mut changed = false;
        self.map.retain(|k, v| match other.map.get(k) {
            Some(theirs) => {
                changed |= v.meet(theirs);
                !v.is_bottom()
            }
            None => {
                changed = true;
                false
            }
        });
        changed
    }

    fn leq(&self, other: &Self) -> bool {
        self.map.iter().all(|(k, v)| match other.map.get(k) {
            Some(theirs) => v.leq(theirs),
            None => v.is_bottom(),
        })
    }

    fn widen(&mut self, newer: &Self) -> bool {
        let mut changed = false;
        for (k, v) in &newer.map {
            match self.map.get_mut(k) {
                Some(mine) => changed |= mine.widen(v),
                None => {
                    self.map.insert(k.clone(), v.clone());
                    changed = true;
                }
            }
        }
        changed
    }

    fn narrow(&mut self, newer: &Self) -> bool {
        let bottom = V::bottom();
        let mut changed = false;
        self.map.retain(|k, v| {
            let target = newer.map.get(k).unwrap_or(&bottom);
            changed |= v.narrow(target);
            !v.is_bottom()
        });
        changed
    }
}

// ── Flat constant lattice ────────────────────────────────────────────────────

/// The flat lattice `⊥ < Value(c) < ⊤` over any carrier with equality: the constant-propagation
/// lattice when `T` is a constant, `⊤` meaning "not a single constant".
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Flat<T> {
    Bottom,
    Value(T),
    Top,
}

impl<T> Flat<T> {
    pub fn value(&self) -> Option<&T> {
        match self {
            Flat::Value(v) => Some(v),
            _ => None,
        }
    }

    pub fn is_top(&self) -> bool {
        matches!(self, Flat::Top)
    }

    pub fn is_value(&self) -> bool {
        matches!(self, Flat::Value(_))
    }
}

impl<T: Clone + PartialEq + fmt::Debug> Lattice for Flat<T> {
    fn bottom() -> Self {
        Flat::Bottom
    }

    fn is_bottom(&self) -> bool {
        matches!(self, Flat::Bottom)
    }

    fn join(&mut self, other: &Self) -> bool {
        match (&*self, other) {
            (_, Flat::Bottom) | (Flat::Top, _) => false,
            (Flat::Bottom, theirs) => {
                *self = theirs.clone();
                true
            }
            (Flat::Value(a), Flat::Value(b)) if a == b => false,
            _ => {
                *self = Flat::Top;
                true
            }
        }
    }

    fn meet(&mut self, other: &Self) -> bool {
        match (&*self, other) {
            (_, Flat::Top) | (Flat::Bottom, _) => false,
            (Flat::Top, theirs) => {
                *self = theirs.clone();
                true
            }
            (Flat::Value(a), Flat::Value(b)) if a == b => false,
            _ => {
                *self = Flat::Bottom;
                true
            }
        }
    }

    fn leq(&self, other: &Self) -> bool {
        match (self, other) {
            (Flat::Bottom, _) | (_, Flat::Top) => true,
            (Flat::Value(a), Flat::Value(b)) => a == b,
            _ => false,
        }
    }
}

// ── Intervals ────────────────────────────────────────────────────────────────

/// A bound of an interval over the extended integers. The derived order is
/// `NegInf < Finite(_) < PosInf` with finite bounds ordered by value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Bound {
    NegInf,
    Finite(i128),
    PosInf,
}

impl Bound {
    #[inline]
    pub fn finite(self) -> Option<i128> {
        match self {
            Bound::Finite(v) => Some(v),
            _ => None,
        }
    }

    #[inline]
    fn signum(self) -> i8 {
        match self {
            Bound::NegInf => -1,
            Bound::PosInf => 1,
            Bound::Finite(v) => v.signum() as i8,
        }
    }

    /// `self + 1`, saturating at the infinities.
    #[inline]
    fn succ(self) -> Bound {
        match self {
            Bound::Finite(v) => v.checked_add(1).map_or(Bound::PosInf, Bound::Finite),
            other => other,
        }
    }

    /// `self − 1`, saturating at the infinities.
    #[inline]
    fn pred(self) -> Bound {
        match self {
            Bound::Finite(v) => v.checked_sub(1).map_or(Bound::NegInf, Bound::Finite),
            other => other,
        }
    }

    #[inline]
    fn neg(self) -> Bound {
        match self {
            Bound::NegInf => Bound::PosInf,
            Bound::PosInf => Bound::NegInf,
            Bound::Finite(v) => v.checked_neg().map_or(Bound::PosInf, Bound::Finite),
        }
    }

    /// Absolute value as a bound (`|i128::MIN|` saturates to `+∞`).
    #[inline]
    fn abs(self) -> Bound {
        match self {
            Bound::NegInf | Bound::PosInf => Bound::PosInf,
            Bound::Finite(v) => v.checked_abs().map_or(Bound::PosInf, Bound::Finite),
        }
    }
}

/// Lower-bound addition: any `−∞` operand yields `−∞`.
#[inline]
fn add_lo(a: Bound, b: Bound) -> Bound {
    match (a, b) {
        (Bound::NegInf, _) | (_, Bound::NegInf) => Bound::NegInf,
        (Bound::PosInf, _) | (_, Bound::PosInf) => Bound::PosInf,
        (Bound::Finite(x), Bound::Finite(y)) => checked_or_saturate(x.checked_add(y), x),
    }
}

/// Upper-bound addition: any `+∞` operand yields `+∞`.
#[inline]
fn add_hi(a: Bound, b: Bound) -> Bound {
    match (a, b) {
        (Bound::PosInf, _) | (_, Bound::PosInf) => Bound::PosInf,
        (Bound::NegInf, _) | (_, Bound::NegInf) => Bound::NegInf,
        (Bound::Finite(x), Bound::Finite(y)) => checked_or_saturate(x.checked_add(y), x),
    }
}

/// Overflow of `x + y` happens only when both have the sign of `x`.
#[inline]
fn checked_or_saturate(result: Option<i128>, x: i128) -> Bound {
    match result {
        Some(v) => Bound::Finite(v),
        None if x > 0 => Bound::PosInf,
        None => Bound::NegInf,
    }
}

#[inline]
fn mul_bound(a: Bound, b: Bound) -> Bound {
    match (a, b) {
        (Bound::Finite(0), _) | (_, Bound::Finite(0)) => Bound::Finite(0),
        (Bound::Finite(x), Bound::Finite(y)) => match x.checked_mul(y) {
            Some(v) => Bound::Finite(v),
            None if (x < 0) == (y < 0) => Bound::PosInf,
            None => Bound::NegInf,
        },
        _ => {
            if a.signum() * b.signum() < 0 {
                Bound::NegInf
            } else {
                Bound::PosInf
            }
        }
    }
}

/// Truncating division of bounds; `b` is never `Finite(0)`.
#[inline]
fn div_bound(a: Bound, b: Bound) -> Bound {
    match (a, b) {
        (Bound::Finite(x), Bound::Finite(y)) => x.checked_div(y).map_or(Bound::PosInf, Bound::Finite),
        (Bound::Finite(_), _) => Bound::Finite(0),
        _ => {
            if a.signum() * b.signum() < 0 {
                Bound::NegInf
            } else {
                Bound::PosInf
            }
        }
    }
}

/// An integer interval `[lo, hi]` over `ℤ ∪ {−∞, +∞}`, or the empty interval.
///
/// Invariant of `Range`: `lo ≤ hi`, `lo ≠ +∞`, `hi ≠ −∞`. Arithmetic is exact on the
/// mathematical integers; fixed-width wrap-around is applied by [`Interval::wrap_to`].
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Interval {
    Bottom,
    Range { lo: Bound, hi: Bound },
}

impl fmt::Debug for Interval {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Interval::Bottom => f.write_str("⊥"),
            Interval::Range { lo, hi } => {
                let show = |b: &Bound| match b {
                    Bound::NegInf => "-∞".to_string(),
                    Bound::PosInf => "+∞".to_string(),
                    Bound::Finite(v) => v.to_string(),
                };
                write!(f, "[{}, {}]", show(lo), show(hi))
            }
        }
    }
}

impl Interval {
    /// `[lo, hi]`, empty when `lo > hi`.
    pub fn new(lo: Bound, hi: Bound) -> Interval {
        if lo == Bound::PosInf || hi == Bound::NegInf || lo > hi {
            Interval::Bottom
        } else {
            Interval::Range { lo, hi }
        }
    }

    /// `[−∞, +∞]`.
    pub fn top() -> Interval {
        Interval::Range {
            lo: Bound::NegInf,
            hi: Bound::PosInf,
        }
    }

    /// `[c, c]`.
    pub fn constant(c: i128) -> Interval {
        Interval::Range {
            lo: Bound::Finite(c),
            hi: Bound::Finite(c),
        }
    }

    /// `[lo, hi]` with finite bounds.
    pub fn range(lo: i128, hi: i128) -> Interval {
        Interval::new(Bound::Finite(lo), Bound::Finite(hi))
    }

    /// The range of a fixed-width integer type.
    pub fn type_range(bits: u16, signed: bool) -> Interval {
        if bits == 0 {
            return Interval::constant(0);
        }
        if signed {
            if bits >= 128 {
                return Interval::range(i128::MIN, i128::MAX);
            }
            let half = 1i128 << (bits - 1);
            Interval::range(-half, half - 1)
        } else if bits >= 128 {
            Interval::new(Bound::Finite(0), Bound::PosInf)
        } else if bits == 127 {
            Interval::range(0, i128::MAX)
        } else {
            Interval::range(0, (1i128 << bits) - 1)
        }
    }

    pub fn lo(&self) -> Option<Bound> {
        match self {
            Interval::Range { lo, .. } => Some(*lo),
            Interval::Bottom => None,
        }
    }

    pub fn hi(&self) -> Option<Bound> {
        match self {
            Interval::Range { hi, .. } => Some(*hi),
            Interval::Bottom => None,
        }
    }

    /// The single value of a one-element interval.
    pub fn as_constant(&self) -> Option<i128> {
        match self {
            Interval::Range {
                lo: Bound::Finite(a),
                hi: Bound::Finite(b),
            } if a == b => Some(*a),
            _ => None,
        }
    }

    pub fn is_top(&self) -> bool {
        matches!(
            self,
            Interval::Range {
                lo: Bound::NegInf,
                hi: Bound::PosInf
            }
        )
    }

    pub fn contains(&self, v: i128) -> bool {
        match self {
            Interval::Bottom => false,
            Interval::Range { lo, hi } => *lo <= Bound::Finite(v) && Bound::Finite(v) <= *hi,
        }
    }

    /// True iff every element is `≥ 0`.
    pub fn is_non_negative(&self) -> bool {
        matches!(self, Interval::Range { lo, .. } if *lo >= Bound::Finite(0))
    }

    /// Convex hull `self ⊔ other`.
    pub fn hull(&self, other: &Interval) -> Interval {
        match (self, other) {
            (Interval::Bottom, x) | (x, Interval::Bottom) => *x,
            (Interval::Range { lo: a, hi: b }, Interval::Range { lo: c, hi: d }) => {
                Interval::new((*a).min(*c), (*b).max(*d))
            }
        }
    }

    /// Intersection `self ⊓ other`.
    pub fn intersect(&self, other: &Interval) -> Interval {
        match (self, other) {
            (Interval::Bottom, _) | (_, Interval::Bottom) => Interval::Bottom,
            (Interval::Range { lo: a, hi: b }, Interval::Range { lo: c, hi: d }) => {
                Interval::new((*a).max(*c), (*b).min(*d))
            }
        }
    }

    /// The interval itself if it fits the fixed-width type, otherwise the whole type range
    /// (two's-complement wrap-around may produce any value of the type).
    pub fn wrap_to(&self, bits: u16, signed: bool) -> Interval {
        let range = Interval::type_range(bits, signed);
        match self {
            Interval::Bottom => Interval::Bottom,
            _ if self.leq(&range) => *self,
            _ => range,
        }
    }

    pub fn add(&self, other: &Interval) -> Interval {
        match (self, other) {
            (Interval::Range { lo: a, hi: b }, Interval::Range { lo: c, hi: d }) => {
                Interval::new(add_lo(*a, *c), add_hi(*b, *d))
            }
            _ => Interval::Bottom,
        }
    }

    pub fn neg(&self) -> Interval {
        match self {
            Interval::Range { lo, hi } => Interval::new(hi.neg(), lo.neg()),
            Interval::Bottom => Interval::Bottom,
        }
    }

    pub fn sub(&self, other: &Interval) -> Interval {
        self.add(&other.neg())
    }

    pub fn mul(&self, other: &Interval) -> Interval {
        match (self, other) {
            (Interval::Range { lo: a, hi: b }, Interval::Range { lo: c, hi: d }) => {
                let corners = [
                    mul_bound(*a, *c),
                    mul_bound(*a, *d),
                    mul_bound(*b, *c),
                    mul_bound(*b, *d),
                ];
                let lo = corners.iter().copied().min().unwrap_or(Bound::NegInf);
                let hi = corners.iter().copied().max().unwrap_or(Bound::PosInf);
                Interval::new(lo, hi)
            }
            _ => Interval::Bottom,
        }
    }

    /// Truncating division (rounding toward zero). Divisors equal to zero contribute no
    /// value: the operation traps or is undefined there, so no result flows on.
    pub fn div(&self, other: &Interval) -> Interval {
        let (Interval::Range { lo: a, hi: b }, Interval::Range { .. }) = (self, other) else {
            return Interval::Bottom;
        };
        let negative = other.intersect(&Interval::new(Bound::NegInf, Bound::Finite(-1)));
        let positive = other.intersect(&Interval::new(Bound::Finite(1), Bound::PosInf));
        let mut result = Interval::Bottom;
        for part in [negative, positive] {
            if let Interval::Range { lo: c, hi: d } = part {
                let corners = [
                    div_bound(*a, c),
                    div_bound(*a, d),
                    div_bound(*b, c),
                    div_bound(*b, d),
                ];
                let lo = corners.iter().copied().min().unwrap_or(Bound::NegInf);
                let hi = corners.iter().copied().max().unwrap_or(Bound::PosInf);
                result = result.hull(&Interval::new(lo, hi));
            }
        }
        result
    }

    /// Truncated remainder: the sign follows the dividend, `|r| < |divisor|` and
    /// `|r| ≤ |dividend|`.
    pub fn rem(&self, other: &Interval) -> Interval {
        let (Interval::Range { lo: a, hi: b }, Interval::Range { lo: c, hi: d }) = (self, other)
        else {
            return Interval::Bottom;
        };
        if let (Some(x), Some(y)) = (self.as_constant(), other.as_constant()) {
            return match x.checked_rem(y) {
                Some(r) => Interval::constant(r),
                None if y == 0 => Interval::Bottom,
                None => Interval::constant(0),
            };
        }
        if other.as_constant() == Some(0) {
            return Interval::Bottom;
        }
        let magnitude = c.abs().max(d.abs()).pred();
        let lo = if *a >= Bound::Finite(0) {
            Bound::Finite(0)
        } else {
            (*a).max(magnitude.neg())
        };
        let hi = if *b <= Bound::Finite(0) {
            Bound::Finite(0)
        } else {
            (*b).min(magnitude)
        };
        Interval::new(lo, hi)
    }

    /// Bitwise AND: exact on constants, `[0, min]`-bounded when an operand is non-negative.
    pub fn bit_and(&self, other: &Interval) -> Interval {
        if let (Some(x), Some(y)) = (self.as_constant(), other.as_constant()) {
            return Interval::constant(x & y);
        }
        match (self, other) {
            (Interval::Bottom, _) | (_, Interval::Bottom) => Interval::Bottom,
            _ => match (self.is_non_negative(), other.is_non_negative()) {
                (true, true) => Interval::new(
                    Bound::Finite(0),
                    self.hi().unwrap_or(Bound::PosInf).min(other.hi().unwrap_or(Bound::PosInf)),
                ),
                (true, false) => Interval::new(Bound::Finite(0), self.hi().unwrap_or(Bound::PosInf)),
                (false, true) => Interval::new(Bound::Finite(0), other.hi().unwrap_or(Bound::PosInf)),
                (false, false) => Interval::top(),
            },
        }
    }

    /// Bitwise OR / XOR of non-negative operands: `[0, 2^k − 1]` where `2^k` exceeds both
    /// upper bounds; exact on constants; otherwise unbounded.
    pub fn bit_or_xor(&self, other: &Interval, xor: bool) -> Interval {
        if let (Some(x), Some(y)) = (self.as_constant(), other.as_constant()) {
            return Interval::constant(if xor { x ^ y } else { x | y });
        }
        match (self, other) {
            (Interval::Bottom, _) | (_, Interval::Bottom) => Interval::Bottom,
            _ if self.is_non_negative() && other.is_non_negative() => {
                let hi = self.hi().unwrap_or(Bound::PosInf).max(other.hi().unwrap_or(Bound::PosInf));
                match hi {
                    Bound::Finite(h) => {
                        let bits = 128 - h.leading_zeros();
                        if bits >= 127 {
                            Interval::new(Bound::Finite(0), Bound::PosInf)
                        } else {
                            let lo = if xor {
                                0
                            } else {
                                self.lo().and_then(Bound::finite).unwrap_or(0).max(
                                    other.lo().and_then(Bound::finite).unwrap_or(0),
                                )
                            };
                            Interval::range(lo, (1i128 << bits) - 1)
                        }
                    }
                    _ => Interval::new(Bound::Finite(0), Bound::PosInf),
                }
            }
            _ => Interval::top(),
        }
    }

    /// Left shift by a constant distance below 127; any other distance is unbounded.
    pub fn shl(&self, distance: &Interval) -> Interval {
        match (self, distance.as_constant()) {
            (Interval::Bottom, _) => Interval::Bottom,
            (_, Some(k)) if (0..127).contains(&k) => self.mul(&Interval::constant(1i128 << k)),
            _ => Interval::top(),
        }
    }

    /// Arithmetic right shift (floor division by `2^k`) by a constant distance; a
    /// non-negative operand shifted by an unknown distance stays within `[0, hi]`.
    pub fn shr(&self, distance: &Interval) -> Interval {
        match (self, distance.as_constant()) {
            (Interval::Bottom, _) => Interval::Bottom,
            (Interval::Range { lo, hi }, Some(k)) if (0..128).contains(&k) => {
                let shift = |b: Bound| match b {
                    Bound::Finite(v) => Bound::Finite(v >> k),
                    other => other,
                };
                Interval::new(shift(*lo), shift(*hi))
            }
            (Interval::Range { hi, .. }, _) if self.is_non_negative() => {
                Interval::new(Bound::Finite(0), *hi)
            }
            _ => Interval::top(),
        }
    }

    /// `Some(true)` when `x < y` for every pair of elements, `Some(false)` when for none.
    pub fn decide_lt(&self, other: &Interval) -> Option<bool> {
        match (self, other) {
            (Interval::Range { lo: a, hi: b }, Interval::Range { lo: c, hi: d }) => {
                if *b < *c {
                    Some(true)
                } else if *a >= *d {
                    Some(false)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// `Some(true)` when `x ≤ y` for every pair, `Some(false)` when for none.
    pub fn decide_le(&self, other: &Interval) -> Option<bool> {
        match (self, other) {
            (Interval::Range { lo: a, hi: b }, Interval::Range { lo: c, hi: d }) => {
                if *b <= *c {
                    Some(true)
                } else if *a > *d {
                    Some(false)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// `Some(true)` when both are the same single value, `Some(false)` when disjoint.
    pub fn decide_eq(&self, other: &Interval) -> Option<bool> {
        match (self.as_constant(), other.as_constant()) {
            (Some(a), Some(b)) => Some(a == b),
            _ if self.intersect(other) == Interval::Bottom => Some(false),
            _ => None,
        }
    }

    /// Refines `(x, y)` under the assumption `x < y`.
    pub fn refine_lt(x: &Interval, y: &Interval) -> (Interval, Interval) {
        match (x, y) {
            (Interval::Range { lo: a, .. }, Interval::Range { hi: d, .. }) => (
                x.intersect(&Interval::new(Bound::NegInf, d.pred())),
                y.intersect(&Interval::new(a.succ(), Bound::PosInf)),
            ),
            _ => (Interval::Bottom, Interval::Bottom),
        }
    }

    /// Refines `(x, y)` under the assumption `x ≤ y`.
    pub fn refine_le(x: &Interval, y: &Interval) -> (Interval, Interval) {
        match (x, y) {
            (Interval::Range { lo: a, .. }, Interval::Range { hi: d, .. }) => (
                x.intersect(&Interval::new(Bound::NegInf, *d)),
                y.intersect(&Interval::new(*a, Bound::PosInf)),
            ),
            _ => (Interval::Bottom, Interval::Bottom),
        }
    }

    /// Refines `(x, y)` under the assumption `x = y`.
    pub fn refine_eq(x: &Interval, y: &Interval) -> (Interval, Interval) {
        let both = x.intersect(y);
        (both, both)
    }

    /// Refines `(x, y)` under the assumption `x ≠ y`: a constant on one side removes itself
    /// from the other side's ends.
    pub fn refine_ne(x: &Interval, y: &Interval) -> (Interval, Interval) {
        let trim = |v: &Interval, c: Option<i128>| -> Interval {
            match (v, c) {
                (Interval::Range { lo, hi }, Some(c)) => {
                    let mut lo = *lo;
                    let mut hi = *hi;
                    if lo == Bound::Finite(c) {
                        lo = lo.succ();
                    }
                    if hi == Bound::Finite(c) {
                        hi = hi.pred();
                    }
                    Interval::new(lo, hi)
                }
                _ => *v,
            }
        };
        (trim(x, y.as_constant()), trim(y, x.as_constant()))
    }
}

impl Lattice for Interval {
    fn bottom() -> Self {
        Interval::Bottom
    }

    fn is_bottom(&self) -> bool {
        matches!(self, Interval::Bottom)
    }

    fn join(&mut self, other: &Self) -> bool {
        let next = self.hull(other);
        let changed = next != *self;
        *self = next;
        changed
    }

    fn meet(&mut self, other: &Self) -> bool {
        let next = self.intersect(other);
        let changed = next != *self;
        *self = next;
        changed
    }

    fn leq(&self, other: &Self) -> bool {
        match (self, other) {
            (Interval::Bottom, _) => true,
            (_, Interval::Bottom) => false,
            (Interval::Range { lo: a, hi: b }, Interval::Range { lo: c, hi: d }) => {
                *c <= *a && *b <= *d
            }
        }
    }

    /// Standard interval widening: an unstable bound jumps to its infinity.
    fn widen(&mut self, newer: &Self) -> bool {
        let next = match (&*self, newer) {
            (Interval::Bottom, x) => *x,
            (x, Interval::Bottom) => *x,
            (Interval::Range { lo: a, hi: b }, Interval::Range { lo: c, hi: d }) => Interval::new(
                if *c < *a { Bound::NegInf } else { *a },
                if *d > *b { Bound::PosInf } else { *b },
            ),
        };
        let changed = next != *self;
        *self = next;
        changed
    }

    /// Standard interval narrowing: only infinite bounds are replaced by the newer bounds.
    fn narrow(&mut self, newer: &Self) -> bool {
        let next = match (&*self, newer) {
            (_, Interval::Bottom) => Interval::Bottom,
            (Interval::Bottom, _) => Interval::Bottom,
            (Interval::Range { lo: a, hi: b }, Interval::Range { lo: c, hi: d }) => Interval::new(
                if *a == Bound::NegInf { *c } else { *a },
                if *b == Bound::PosInf { *d } else { *b },
            ),
        };
        let changed = next != *self;
        *self = next;
        changed
    }
}

// ── Product ──────────────────────────────────────────────────────────────────

/// The componentwise product `A × B`.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Default)]
pub struct Product<A, B>(pub A, pub B);

impl<A: Lattice, B: Lattice> Lattice for Product<A, B> {
    fn bottom() -> Self {
        Product(A::bottom(), B::bottom())
    }

    fn is_bottom(&self) -> bool {
        self.0.is_bottom() && self.1.is_bottom()
    }

    fn join(&mut self, other: &Self) -> bool {
        let a = self.0.join(&other.0);
        let b = self.1.join(&other.1);
        a || b
    }

    fn meet(&mut self, other: &Self) -> bool {
        let a = self.0.meet(&other.0);
        let b = self.1.meet(&other.1);
        a || b
    }

    fn leq(&self, other: &Self) -> bool {
        self.0.leq(&other.0) && self.1.leq(&other.1)
    }

    fn widen(&mut self, newer: &Self) -> bool {
        let a = self.0.widen(&newer.0);
        let b = self.1.widen(&newer.1);
        a || b
    }

    fn narrow(&mut self, newer: &Self) -> bool {
        let a = self.0.narrow(&newer.0);
        let b = self.1.narrow(&newer.1);
        a || b
    }
}

// ── Must-sets ────────────────────────────────────────────────────────────────

/// The dual powerset for *must* facts (a fact holds only if it holds on every path):
/// ordered by reverse inclusion, so the join is the intersection and the least element is
/// the universal set `All` ("no path seen yet"). Reachable states are always `Set`, because
/// the boundary state of an analysis is a concrete set.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum MustSet<T: Ord> {
    All,
    Set(BTreeSet<T>),
}

impl<T: Ord + Clone + fmt::Debug> MustSet<T> {
    pub fn empty() -> Self {
        MustSet::Set(BTreeSet::new())
    }

    pub fn contains(&self, v: &T) -> bool {
        match self {
            MustSet::All => true,
            MustSet::Set(s) => s.contains(v),
        }
    }

    /// Adds `v` (no effect on `All`).
    pub fn insert(&mut self, v: T) {
        if let MustSet::Set(s) = self {
            s.insert(v);
        }
    }

    /// Removes every element for which `drop` holds (no effect on `All`, which stands for an
    /// unreached point).
    pub fn remove_where(&mut self, mut drop: impl FnMut(&T) -> bool) {
        if let MustSet::Set(s) = self {
            s.retain(|v| !drop(v));
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &T> {
        let set = match self {
            MustSet::All => None,
            MustSet::Set(s) => Some(s),
        };
        set.into_iter().flat_map(|s| s.iter())
    }
}

impl<T: Ord + Clone + fmt::Debug> Lattice for MustSet<T> {
    fn bottom() -> Self {
        MustSet::All
    }

    fn is_bottom(&self) -> bool {
        matches!(self, MustSet::All)
    }

    fn join(&mut self, other: &Self) -> bool {
        let theirs = match other {
            MustSet::All => return false,
            MustSet::Set(t) => t,
        };
        match self {
            MustSet::All => {
                *self = other.clone();
                true
            }
            MustSet::Set(mine) => {
                let before = mine.len();
                mine.retain(|v| theirs.contains(v));
                mine.len() != before
            }
        }
    }

    fn meet(&mut self, other: &Self) -> bool {
        let theirs = match other {
            MustSet::All => {
                let changed = !self.is_bottom();
                *self = MustSet::All;
                return changed;
            }
            MustSet::Set(t) => t,
        };
        match self {
            MustSet::All => false,
            MustSet::Set(mine) => {
                let before = mine.len();
                mine.extend(theirs.iter().cloned());
                mine.len() != before
            }
        }
    }

    fn leq(&self, other: &Self) -> bool {
        match (self, other) {
            (MustSet::All, _) => true,
            (_, MustSet::All) => false,
            (MustSet::Set(a), MustSet::Set(b)) => b.is_subset(a),
        }
    }
}

// ── Bounded sets ─────────────────────────────────────────────────────────────

/// Sets of at most `K` elements; a set that would grow beyond `K` becomes `Top` ("any
/// value"). The finite-height domain for value sets such as the states of a state field.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum BoundedSet<T: Ord, const K: usize> {
    Elems(BTreeSet<T>),
    Top,
}

impl<T: Ord + Clone + fmt::Debug, const K: usize> BoundedSet<T, K> {
    pub fn empty() -> Self {
        BoundedSet::Elems(BTreeSet::new())
    }

    pub fn singleton(v: T) -> Self {
        let mut s = BTreeSet::new();
        s.insert(v);
        BoundedSet::Elems(s)
    }

    /// The elements, or `None` for `Top`.
    pub fn elements(&self) -> Option<&BTreeSet<T>> {
        match self {
            BoundedSet::Elems(s) => Some(s),
            BoundedSet::Top => None,
        }
    }

    pub fn is_top(&self) -> bool {
        matches!(self, BoundedSet::Top)
    }

    /// True when `v` may be a member (always true for `Top`).
    pub fn may_contain(&self, v: &T) -> bool {
        match self {
            BoundedSet::Elems(s) => s.contains(v),
            BoundedSet::Top => true,
        }
    }

    /// Adds `v`; returns true when the set changed.
    pub fn insert(&mut self, v: T) -> bool {
        let (added, overflow) = match self {
            BoundedSet::Top => return false,
            BoundedSet::Elems(s) => {
                let added = s.insert(v);
                (added, s.len() > K)
            }
        };
        if overflow {
            *self = BoundedSet::Top;
        }
        added
    }
}

impl<T: Ord + Clone + fmt::Debug, const K: usize> FromIterator<T> for BoundedSet<T, K> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        let mut s = Self::empty();
        for v in iter {
            s.insert(v);
        }
        s
    }
}

impl<T: Ord + Clone + fmt::Debug, const K: usize> Lattice for BoundedSet<T, K> {
    fn bottom() -> Self {
        Self::empty()
    }

    fn is_bottom(&self) -> bool {
        matches!(self, BoundedSet::Elems(s) if s.is_empty())
    }

    fn join(&mut self, other: &Self) -> bool {
        let theirs = match other {
            BoundedSet::Top => {
                let changed = !self.is_top();
                *self = BoundedSet::Top;
                return changed;
            }
            BoundedSet::Elems(theirs) => theirs,
        };
        let (grew, overflow) = match self {
            BoundedSet::Top => return false,
            BoundedSet::Elems(mine) => {
                let before = mine.len();
                mine.extend(theirs.iter().cloned());
                (mine.len() != before, mine.len() > K)
            }
        };
        if overflow {
            *self = BoundedSet::Top;
        }
        grew
    }

    fn meet(&mut self, other: &Self) -> bool {
        let theirs = match other {
            BoundedSet::Top => return false,
            BoundedSet::Elems(theirs) => theirs,
        };
        match self {
            BoundedSet::Top => {
                *self = other.clone();
                true
            }
            BoundedSet::Elems(mine) => {
                let before = mine.len();
                mine.retain(|v| theirs.contains(v));
                mine.len() != before
            }
        }
    }

    fn leq(&self, other: &Self) -> bool {
        match (self, other) {
            (_, BoundedSet::Top) => true,
            (BoundedSet::Top, _) => false,
            (BoundedSet::Elems(a), BoundedSet::Elems(b)) => a.is_subset(b),
        }
    }
}
