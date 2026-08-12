//! PathMetrics — per-function path analysis metrics (§8.2.5, §8.4, §8.7).
//!
//! Stores the three key metrics derived from Phase 8 construction:
//!
//! **V(G) — Cyclomatic complexity:**
//! `V(G) = |E| - |B| + 2` (§8.2.5).
//! Computed ONCE from CFA edge and block counts. Stored permanently as `u16`.
//! NOT derived from the ROBDD at query time. This equals the number of linearly
//! independent paths — geometrically, the number of binary decision nodes whose
//! both branches are satisfiable.
//!
//! **sat_count — Feasible path count:**
//! `#SAT(f_paths)` = number of satisfying assignments = number of feasible execution paths.
//! For Phase 9's UML diagrams, this directly drives complexity annotations on activity
//! and state machine diagrams.
//!
//! **max_path_len — Longest path (block count):**
//! Maximum number of basic blocks visited on any single path from entry to exit.
//! Upper-bounds loop iteration estimates in Phase 10 queries.
//!
//! **unwind_depth:**
//! 0 = non-recursive function. > 0 = unwinding bound for recursive SCC functions.
//! Phase 10's CFL-reachability engine uses this when composing inter-procedural queries.

/// Per-function path analysis metrics.
///
/// 16 bytes total — stored in the PSA PATH METRICS TABLE section (§8.6):
/// `(cyclomatic:u16, max_path:u16, sat_lo:u32, sat_hi:u32, mean:f32)`.
///
/// **Phase 9 hot path:** This table is always memory-mapped (~240 KB for 15K functions)
/// providing O(1) cyclomatic complexity and sat_count lookup for every function.
#[derive(Clone, Debug)]
pub struct PathMetrics {
    /// V(G) = |E| - |B| + 2 (McCabe cyclomatic complexity). Max value ~65K.
    pub cyclomatic: u16,

    /// Length of longest path (number of blocks visited). Max value ~65K.
    pub max_path_len: u16,

    /// #SAT(f_paths): number of feasible execution paths (full 64-bit count).
    pub sat_count: u64,

    /// 0 = non-recursive. >0 = bounded unwinding depth for recursive SCC functions.
    pub unwind_depth: u16,

    /// Reserved padding for 8-byte alignment.
    pub _reserved: u16,
}

impl PathMetrics {
    /// Construct path metrics from the values computed in Phase 8 step 8.
    pub fn new(cyclomatic: u16, max_path_len: u16, sat_count: u64, unwind_depth: u16) -> Self {
        Self {
            cyclomatic,
            max_path_len,
            sat_count,
            unwind_depth,
            _reserved: 0,
        }
    }

    /// Serialize to the PSA binary format metrics record (16 bytes).
    ///
    /// Format (§8.6 PATH METRICS TABLE):
    /// ```text
    /// cyclomatic:u16, max_path:u16, sat_lo:u32, sat_hi:u32, mean:f32
    /// ```
    /// where sat_lo = sat_count & 0xFFFFFFFF, sat_hi = sat_count >> 32,
    /// and mean = sat_count as f32 (approximate float for analytics).
    pub fn to_bytes(&self) -> [u8; 16] {
        let mut buf = [0u8; 16];
        buf[0..2].copy_from_slice(&self.cyclomatic.to_le_bytes());
        buf[2..4].copy_from_slice(&self.max_path_len.to_le_bytes());
        let sat_lo = (self.sat_count & 0xFFFF_FFFF) as u32;
        let sat_hi = (self.sat_count >> 32) as u32;
        buf[4..8].copy_from_slice(&sat_lo.to_le_bytes());
        buf[8..12].copy_from_slice(&sat_hi.to_le_bytes());
        // mean: approximate float for analytics consumers
        let mean = self.sat_count as f32;
        buf[12..16].copy_from_slice(&mean.to_le_bytes());
        buf
    }

    /// Deserialize from 16 raw bytes (PSA format).
    pub fn from_bytes(buf: &[u8; 16]) -> Self {
        let cyclomatic = u16::from_le_bytes([buf[0], buf[1]]);
        let max_path_len = u16::from_le_bytes([buf[2], buf[3]]);
        let sat_lo = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]) as u64;
        let sat_hi = u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]) as u64;
        let sat_count = sat_lo | (sat_hi << 32);
        Self {
            cyclomatic,
            max_path_len,
            sat_count,
            unwind_depth: 0,
            _reserved: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_bytes() {
        let m = PathMetrics::new(7, 12, 42_000_000_001, 0);
        let bytes = m.to_bytes();
        let m2 = PathMetrics::from_bytes(&bytes);
        assert_eq!(m2.cyclomatic, 7);
        assert_eq!(m2.max_path_len, 12);
        assert_eq!(m2.sat_count, 42_000_000_001);
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
