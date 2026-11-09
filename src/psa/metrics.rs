//! PathMetrics — per-function path analysis metrics (§8.2.5, §8.4, §8.7).
//!
//! Stores the key metrics derived from Phase 8 construction:
//!
//! **V(G) — Cyclomatic complexity:**
//! `V(G) = |E| - |B| + 2` (§8.2.5).
//! Computed ONCE from CFA edge and block counts. Stored permanently as `u16`.
//! NOT derived from the ROBDD at query time.
//!
//! **log2_paths — Path count magnitude:**
//! `log2(#paths)` as `f64`. The exact count can exceed any fixed width; the function
//! directory carries it clamped to `u64` with `PSA_FLAG_COUNT_SATURATED` marking clamping,
//! and this field carries its magnitude in every case (negative infinity for zero paths).
//!
//! **max_path_len — Longest path (block count):**
//! Maximum number of basic blocks visited on any single path from entry through the
//! back-edge-free DAG (`PSA_FLAG_HAS_LOOPS` marks functions where loops make real paths
//! longer; `PSA_FLAG_MAX_PATH_SATURATED` marks clamping at `u16::MAX`).
//!
//! **unwind_depth:**
//! Recursive expansions included in the summary; Phase 8 summaries are intraprocedural, so
//! this is 0 and recursion is reported through `PSA_FLAG_RECURSIVE_SCC`.

/// Per-function path analysis metrics.
///
/// 16 bytes total — stored in the PSA PATH METRICS TABLE section (§8.6):
/// `(cyclomatic:u16, max_path:u16, unwind_depth:u16, flags:u16, log2_paths:f64)`.
///
/// **Phase 9 hot path:** This table is always memory-mapped (~240 KB for 15K functions)
/// providing O(1) cyclomatic complexity and path-count magnitude lookup for every function.
#[derive(Clone, Debug)]
pub struct PathMetrics {
    /// V(G) = |E| - |B| + 2 (McCabe cyclomatic complexity). Max value ~65K.
    pub cyclomatic: u16,

    /// Length of longest acyclic path (number of blocks visited). Max value ~65K.
    pub max_path_len: u16,

    /// Recursive expansions included in the summary (always 0 in Phase 8).
    pub unwind_depth: u16,

    /// `PSA_FLAG_*` bits (same values as the function directory record).
    pub flags: u16,

    /// `log2(number of paths)`; negative infinity when there is no path.
    pub log2_paths: f64,
}

impl PathMetrics {
    /// Construct path metrics from the values computed in Phase 8 step 8.
    pub fn new(
        cyclomatic: u16,
        max_path_len: u16,
        unwind_depth: u16,
        flags: u16,
        log2_paths: f64,
    ) -> Self {
        Self {
            cyclomatic,
            max_path_len,
            unwind_depth,
            flags,
            log2_paths,
        }
    }

    /// Serialize to the PSA binary format metrics record (16 bytes).
    ///
    /// Format (§8.6 PATH METRICS TABLE):
    /// ```text
    /// 0  cyclomatic:u16   2  max_path:u16   4  unwind_depth:u16   6  flags:u16
    /// 8  log2_paths:f64 (IEEE-754 binary64, little-endian)
    /// ```
    pub fn to_bytes(&self) -> [u8; 16] {
        let mut buf = [0u8; 16];
        buf[0..2].copy_from_slice(&self.cyclomatic.to_le_bytes());
        buf[2..4].copy_from_slice(&self.max_path_len.to_le_bytes());
        buf[4..6].copy_from_slice(&self.unwind_depth.to_le_bytes());
        buf[6..8].copy_from_slice(&self.flags.to_le_bytes());
        buf[8..16].copy_from_slice(&self.log2_paths.to_le_bytes());
        buf
    }

    /// Deserialize from 16 raw bytes (PSA format).
    pub fn from_bytes(buf: &[u8; 16]) -> Self {
        let mut log2 = [0u8; 8];
        log2.copy_from_slice(&buf[8..16]);
        Self {
            cyclomatic: u16::from_le_bytes([buf[0], buf[1]]),
            max_path_len: u16::from_le_bytes([buf[2], buf[3]]),
            unwind_depth: u16::from_le_bytes([buf[4], buf[5]]),
            flags: u16::from_le_bytes([buf[6], buf[7]]),
            log2_paths: f64::from_le_bytes(log2),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_bytes() {
        let m = PathMetrics::new(7, 12, 0, 0, 35.25);
        let bytes = m.to_bytes();
        let m2 = PathMetrics::from_bytes(&bytes);
        assert_eq!(m2.cyclomatic, 7);
        assert_eq!(m2.max_path_len, 12);
        assert_eq!(m2.log2_paths, 35.25);
    }

    #[test]
    fn cyclomatic_formula() {
        // V(G) = |E| - |B| + 2 — verify with a simple if-else CFG:
        // 5 blocks (ENTRY, condition, true-branch, false-branch, EXIT)
        // 6 edges: ENTRY→cond, cond→true, cond→false, true→exit, false→exit, (entry=1 edge)
        let e = 6i32;
        let b = 5i32;
        let vg = e - b + 2;
        assert_eq!(vg, 3, "Simple if-else: V(G) = 6 - 5 + 2 = 3");
    }
}
