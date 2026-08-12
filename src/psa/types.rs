//! Core types for Phase 8: ROBDD Path Summary Computation (§8.4, §8.2).
//!
//! All Phase 8-specific data structures:
//! - `BoolOp` — the three Boolean operations for `apply()`.
//! - `FunctionROBDD` — the complete per-function ROBDD output.
//! - `FunctionPSAHeader` — the 32-byte binary record in the PSA function directory (§8.4).
//! - `PathSummaryArtifact` — the full PSA collection.
//! - `PSA_MAGIC` — 8-byte magic for the PSA file format (§8.6).
//!
//! Phase 8 consumes `cfg::builder::FunctionCFGData` directly (from the CFA artifact).
//! It does NOT redeclare a separate CFG type — it works with the existing one.

use crate::psa::bdd::node::ROBDDNode;
use crate::psa::metrics::PathMetrics;
use crate::psa::ordering::VariableOrdering;

// ── PSA file format magic number (§8.6) ──────────────────────────────────────
/// Magic bytes for the PSA binary format: `b"OPENHPSA"` as little-endian u64.
pub const PSA_MAGIC: u64 = u64::from_le_bytes(*b"OPENHPSA");

// ── Boolean operations for apply() (§8.2.2) ──────────────────────────────────

/// The three Boolean operations supported by the `apply()` algorithm.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoolOp {
    /// Logical AND (conjunction): f ∧ g.
    And,
    /// Logical OR (disjunction): f ∨ g.
    Or,
    /// Logical XOR (exclusive-or): f ⊕ g. Used for binary branch constraints.
    Xor,
}

// ── Per-function ROBDD output ─────────────────────────────────────────────────

/// The complete ROBDD for one function, produced by `FunctionROBDDBuilder::build()`.
pub struct FunctionROBDD {
    /// Function symbol ID from STA (tags the PSA record).
    pub sym_id: u32,

    /// The variable ordering: var_idx ↔ edge_id bijection.
    pub ordering: VariableOrdering,

    /// All ROBDD nodes for this function (including FALSE at id=0 and TRUE at id=1).
    pub nodes: Vec<ROBDDNode>,

    /// Root node_id of the ROBDD encoding f_paths.
    pub root: u32,

    /// #SAT(f_paths): number of feasible execution paths (§8.2.5).
    pub sat_count: u64,

    /// V(G) = |E| - |B| + 2 (§8.2.5). Stored as u16, NOT re-derived from ROBDD at query time.
    pub cyclomatic: u16,

    /// Length of longest path (number of blocks visited on any path from entry to exit).
    pub max_path_len: u16,

    /// 0 = non-recursive. UNWIND_DEPTH (=3) for recursive SCC functions (§8.2.6).
    pub unwind_depth: u16,
}

// ── FunctionPSAHeader (32 bytes per §8.4) ────────────────────────────────────

/// The 32-byte binary record stored in the PSA FUNCTION DIRECTORY section (§8.4).
///
/// All integers are little-endian. The directory is sorted by `sym_id` enabling
/// O(log n) binary search for any function lookup.
///
/// Binary layout:
/// ```text
/// Offset  Size  Field
///  0       4    sym_id           : u32   function symbol_id from STA
///  4       4    n_vars           : u32   number of Boolean variables (= CFA edge count)
///  8       4    n_nodes          : u32   number of ROBDD nodes (including shared terminals)
/// 12       4    root_node        : u32   root node index within this function's node array
/// 16       8    sat_count        : u64   #SAT(f_paths) = total feasible paths
/// 24       2    cyclomatic       : u16   V(G) = |E| - |B| + 2
/// 26       2    max_path_len     : u16   length of longest path (blocks visited)
/// 28       2    unwind_depth     : u16   0 = non-recursive, >0 = unwinding bound
/// 30       2    _reserved        : u16   (must be zero)
/// Total: 32 bytes
/// ```
#[derive(Clone, Debug)]
pub struct FunctionPSAHeader {
    pub sym_id: u32,
    pub n_vars: u32,
    pub n_nodes: u32,
    pub root_node: u32,
    pub sat_count: u64,
    pub cyclomatic: u16,
    pub max_path_len: u16,
    pub unwind_depth: u16,
    pub _reserved: u16,
}

impl FunctionPSAHeader {
    /// Serialize to 32 bytes (little-endian).
    pub fn to_bytes(&self) -> [u8; 32] {
        let mut buf = [0u8; 32];
        buf[0..4].copy_from_slice(&self.sym_id.to_le_bytes());
        buf[4..8].copy_from_slice(&self.n_vars.to_le_bytes());
        buf[8..12].copy_from_slice(&self.n_nodes.to_le_bytes());
        buf[12..16].copy_from_slice(&self.root_node.to_le_bytes());
        buf[16..24].copy_from_slice(&self.sat_count.to_le_bytes());
        buf[24..26].copy_from_slice(&self.cyclomatic.to_le_bytes());
        buf[26..28].copy_from_slice(&self.max_path_len.to_le_bytes());
        buf[28..30].copy_from_slice(&self.unwind_depth.to_le_bytes());
        buf[30..32].copy_from_slice(&self._reserved.to_le_bytes());
        buf
    }

    /// Deserialize from 32 raw bytes.
    pub fn from_bytes(buf: &[u8; 32]) -> Self {
        Self {
            sym_id:       u32::from_le_bytes([buf[0],  buf[1],  buf[2],  buf[3]]),
            n_vars:       u32::from_le_bytes([buf[4],  buf[5],  buf[6],  buf[7]]),
            n_nodes:      u32::from_le_bytes([buf[8],  buf[9],  buf[10], buf[11]]),
            root_node:    u32::from_le_bytes([buf[12], buf[13], buf[14], buf[15]]),
            sat_count:    u64::from_le_bytes([
                buf[16], buf[17], buf[18], buf[19],
                buf[20], buf[21], buf[22], buf[23],
            ]),
            cyclomatic:   u16::from_le_bytes([buf[24], buf[25]]),
            max_path_len: u16::from_le_bytes([buf[26], buf[27]]),
            unwind_depth: u16::from_le_bytes([buf[28], buf[29]]),
            _reserved:    0,
        }
    }

    /// Build a `FunctionPSAHeader` from a completed `FunctionROBDD`.
    pub fn from_robdd(robdd: &FunctionROBDD) -> Self {
        Self {
            sym_id:       robdd.sym_id,
            n_vars:       robdd.ordering.n_vars() as u32,
            n_nodes:      robdd.nodes.len() as u32,
            root_node:    robdd.root,
            sat_count:    robdd.sat_count,
            cyclomatic:   robdd.cyclomatic,
            max_path_len: robdd.max_path_len,
            unwind_depth: robdd.unwind_depth,
            _reserved:    0,
        }
    }
}

// ── Full PSA artifact ─────────────────────────────────────────────────────────

/// The complete PathSummaryArtifact produced by Phase 8 (§8.6).
///
/// Contains all five binary sections:
/// - HEADER (64 B)
/// - FUNCTION DIRECTORY (n × 32 B, sorted by sym_id)
/// - VARIABLE ORDERING TABLES (per-function edge_id[n_vars])
/// - ROBDD NODE ARRAYS (per-function ROBDDNode[], lazy-loaded)
/// - PATH METRICS TABLE (n × 16 B, always mmap'd in Phase 9)
pub struct PathSummaryArtifact {
    /// File format version. Current: 1.
    pub format_version: u32,

    /// CRC-64 of the CFA artifact this PSA was built from (§8.8 Invariant 4).
    pub cfa_hash: u64,

    /// CRC-64 of the SSA artifact (hash chain integrity).
    pub ssa_hash: u64,

    /// Function directory sorted by sym_id (binary search for O(log n) lookup).
    pub function_dir: Vec<FunctionPSAHeader>,

    /// Per-function variable ordering tables. `ordering_tables[i]` = edge_id[] for function i.
    pub ordering_tables: Vec<Vec<u32>>,

    /// Per-function ROBDD node arrays. `node_arrays[i]` = ROBDDNode[] for function i.
    /// Lazy-loaded: only touched on first path query for a given function.
    pub node_arrays: Vec<Vec<ROBDDNode>>,

    /// Path metrics table — always memory-mapped in Phase 9 for O(1) access per function.
    pub metrics: Vec<PathMetrics>,

    /// Total ROBDD nodes across all functions (§8.7 size estimate).
    pub total_nodes: u64,
}

impl PathSummaryArtifact {
    /// Look up the PSA header for a function by sym_id using binary search (O(log n)).
    pub fn function_header(&self, sym_id: u32) -> Option<&FunctionPSAHeader> {
        let idx = self
            .function_dir
            .binary_search_by_key(&sym_id, |h| h.sym_id)
            .ok()?;
        Some(&self.function_dir[idx])
    }

    /// Look up path metrics for a function by its directory index (O(1)).
    pub fn path_metrics(&self, dir_idx: usize) -> Option<&PathMetrics> {
        self.metrics.get(dir_idx)
    }

    /// Total number of functions in this PSA.
    pub fn function_count(&self) -> usize {
        self.function_dir.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn function_psa_header_serializes_to_32_bytes() {
        let h = FunctionPSAHeader {
            sym_id: 42, n_vars: 35, n_nodes: 500, root_node: 100,
            sat_count: 1024, cyclomatic: 7, max_path_len: 15,
            unwind_depth: 0, _reserved: 0,
        };
        assert_eq!(h.to_bytes().len(), 32);
    }

    #[test]
    fn function_psa_header_round_trip() {
        let h = FunctionPSAHeader {
            sym_id: 1234, n_vars: 50, n_nodes: 800, root_node: 200,
            sat_count: 9_999_999_999, cyclomatic: 11, max_path_len: 30,
            unwind_depth: 3, _reserved: 0,
        };
        let h2 = FunctionPSAHeader::from_bytes(&h.to_bytes());
        assert_eq!(h2.sym_id, 1234);
        assert_eq!(h2.sat_count, 9_999_999_999);
        assert_eq!(h2.unwind_depth, 3);
    }

    #[test]
    fn psa_magic_reads_as_openhpsa() {
        assert_eq!(&PSA_MAGIC.to_le_bytes(), b"OPENHPSA");
    }
}
