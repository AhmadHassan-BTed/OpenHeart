//! Stabbing index over token intervals (§7.2.2, §7.5.2).
//!
//! The AST, block and SSA layers map entities to token ranges that may nest arbitrarily
//! (an AST node contains its descendants; synthetic blocks carry their whole function's
//! range). [`IntervalIndex`] answers "which entities contain token t" by binary search plus
//! a backward scan bounded by a prefix maximum of interval ends:
//!
//! ```text
//! entries sorted by (first, last desc, id);  reach[i] = max(last[0..=i])
//! stab(t): i = last entry with first ≤ t                    O(log N)
//!          scan i, i−1, … while reach[j] ≥ t, report last[j] ≥ t
//! ```
//!
//! The scan stops at the first position before which no interval reaches `t`, so its cost
//! is the number of intervals that start between the earliest interval containing `t` and
//! `t` itself — the reported intervals plus the ones nested inside them that end before
//! `t`. For well-nested layers that is proportional to the output.

/// A token interval owned by entity `id`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Interval {
    first: u32,
    last: u32,
    id: u32,
}

/// Static stabbing index over `(first, last, id)` token intervals.
#[derive(Clone, Debug, Default)]
pub struct IntervalIndex {
    entries: Vec<Interval>,
    reach: Vec<u32>,
}

impl IntervalIndex {
    /// Build from `(first, last, id)` triples; empty or inverted intervals and intervals
    /// starting at `skip_first` (the "no source" sentinel) are ignored.
    pub fn build(intervals: impl IntoIterator<Item = (u32, u32, u32)>, skip_first: u32) -> Self {
        let mut entries: Vec<Interval> = intervals
            .into_iter()
            .filter(|&(first, last, _)| first != skip_first && last >= first)
            .map(|(first, last, id)| Interval { first, last, id })
            .collect();
        // Equal starts: longer intervals first, so the backward scan meets the inner one
        // first.
        entries.sort_unstable_by(|a, b| {
            a.first
                .cmp(&b.first)
                .then(b.last.cmp(&a.last))
                .then(a.id.cmp(&b.id))
        });
        let mut reach = Vec::with_capacity(entries.len());
        let mut max_last = 0u32;
        for e in &entries {
            max_last = max_last.max(e.last);
            reach.push(max_last);
        }
        Self { entries, reach }
    }

    /// Number of indexed intervals.
    #[inline]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True iff nothing is indexed.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Ids of every interval containing `token`, innermost (latest start) first; equal
    /// starts are reported shorter interval first.
    pub fn stab(&self, token: u32) -> Vec<u32> {
        let upper = self.entries.partition_point(|e| e.first <= token);
        let mut out = Vec::new();
        let mut j = upper;
        while j > 0 {
            j -= 1;
            if self.reach[j] < token {
                break;
            }
            let e = self.entries[j];
            if e.last >= token {
                out.push(e.id);
            }
        }
        out
    }
}
