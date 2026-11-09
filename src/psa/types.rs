//! Core types for Phase 8: ROBDD Path Summary Computation (§8.4, §8.2).
//!
//! All Phase 8-specific data structures:
//! - `BoolOp` — the three Boolean operations for `apply()`.
//! - `FunctionROBDD` — the complete per-function output.
//! - `FunctionPSAHeader` — the 32-byte binary record in the PSA function directory (§8.4).
//! - `PathSummaryArtifact` — the full PSA collection.
//! - `PSA_MAGIC` — 8-byte magic for the PSA file format (§8.6).
//! - `PSA_FLAG_*` — per-function flags that make every approximation explicit.
//!
//! Phase 8 consumes `cfg::builder::FunctionCFGData` directly (from the CFA artifact).
//! It does NOT redeclare a separate CFG type — it works with the existing one.

use std::fmt;

use crate::psa::bdd::node::ROBDDNode;
use crate::psa::bdd::BddLimit;
use crate::psa::count::PathCount;
use crate::psa::metrics::PathMetrics;
use crate::psa::ordering::VariableOrdering;

// ── PSA file format magic number (§8.6) ──────────────────────────────────────
/// Magic bytes for the PSA binary format: `b"OPENHPSA"` as little-endian u64.
pub const PSA_MAGIC: u64 = u64::from_le_bytes(*b"OPENHPSA");

/// Current PSA format version. Version 2: exact flow encoding, flags, compacted node arrays,
/// `log2` path counts in the metrics table, CRC-64/ECMA-182 shared with all artifacts.
pub const PSA_FORMAT_VERSION: u32 = 2;

// ── Per-function flags (FunctionPSAHeader.flags / PathMetrics.flags) ─────────

/// The CFG has a reachable cycle. `sat_count` counts Ball-Larus acyclic path segments
/// (ENTRY or a loop header to a sink or a back edge) and `max_path_len` is measured on the
/// back-edge-free DAG; neither bounds the number of loop iterations.
pub const PSA_FLAG_HAS_LOOPS: u16 = 1 << 0;
/// The exact path count exceeds 64 bits: `sat_count` holds `u64::MAX` and the magnitude is
/// in `PathMetrics::log2_paths`.
pub const PSA_FLAG_COUNT_SATURATED: u16 = 1 << 1;
/// No ROBDD is stored (node or time budget exhausted, too many variables, or an internal
/// consistency check failed). `root_node` is [`PSA_NO_ROOT`]; counts come from the exact
/// dynamic program over the CFG and remain valid.
pub const PSA_FLAG_BDD_UNAVAILABLE: u16 = 1 << 2;
/// Branch-predicate feasibility filtering was applied: `sat_count` counts feasible paths.
/// Without this flag the count is structural (every CFG path is assumed feasible).
pub const PSA_FLAG_FEASIBILITY_FILTERED: u16 = 1 << 3;
/// The function belongs to a recursive SCC; the summary is intraprocedural (no unwinding).
pub const PSA_FLAG_RECURSIVE_SCC: u16 = 1 << 4;
/// The longest path has more than `u16::MAX` blocks; `max_path_len` holds `u16::MAX`.
pub const PSA_FLAG_MAX_PATH_SATURATED: u16 = 1 << 5;
/// The stored ROBDD was reordered by sifting (the ordering table reflects the new order).
pub const PSA_FLAG_SIFTED: u16 = 1 << 6;
/// The CFG has no blocks; there is no path and no ROBDD.
pub const PSA_FLAG_EMPTY_CFG: u16 = 1 << 7;

/// `root_node` value of functions without a stored ROBDD.
pub const PSA_NO_ROOT: u32 = u32::MAX;

// ── Boolean operations for apply() (§8.2.2) ──────────────────────────────────

/// The three Boolean operations supported by the `apply()` algorithm.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoolOp {
    /// Logical AND (conjunction): f ∧ g.
    And,
    /// Logical OR (disjunction): f ∨ g.
    Or,
    /// Logical XOR (exclusive-or): f ⊕ g.
    Xor,
}

// ── Why a function has no stored ROBDD ───────────────────────────────────────

/// Reason a function's ROBDD summary is unavailable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SummaryUnavailable {
    /// The CFG has no blocks.
    EmptyCfg,
    /// The path formula needs more variables than a 16-bit node label can address.
    TooManyVariables { needed: usize },
    /// A budgeted BDD operation hit the node budget or the deadline.
    Limit(BddLimit),
    /// The structural ROBDD's model count disagreed with the exact path-count dynamic
    /// program (an internal error); the ROBDD was discarded rather than published.
    CountMismatch { bdd_count: String, dp_count: String },
    /// The constructed ROBDD failed validation (an internal error); it was discarded.
    InvalidBdd(String),
}

impl fmt::Display for SummaryUnavailable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SummaryUnavailable::EmptyCfg => f.write_str("empty CFG"),
            SummaryUnavailable::TooManyVariables { needed } => write!(
                f,
                "{} path variables exceed the 65535-variable ROBDD limit",
                needed
            ),
            SummaryUnavailable::Limit(limit) => write!(f, "{}", limit),
            SummaryUnavailable::CountMismatch {
                bdd_count,
                dp_count,
            } => write!(
                f,
                "ROBDD model count {} disagrees with path count {}",
                bdd_count, dp_count
            ),
            SummaryUnavailable::InvalidBdd(msg) => write!(f, "invalid ROBDD: {}", msg),
        }
    }
}

// ── Per-function ROBDD output ─────────────────────────────────────────────────

/// The complete summary for one function, produced by `FunctionROBDDBuilder::build()`.
pub struct FunctionROBDD {
    /// Function symbol ID from STA (tags the PSA record).
    pub sym_id: u32,

    /// The variable ordering: BDD level ↔ edge / loop-entry bijection.
    pub ordering: VariableOrdering,

    /// The root's sub-DAG only, compacted (FALSE at 0, TRUE at 1, children before parents).
    /// Just the two terminals when the summary is unavailable.
    pub nodes: Vec<ROBDDNode>,

    /// Root node_id of the ROBDD encoding f_paths, or [`PSA_NO_ROOT`].
    pub root: u32,

    /// Exact number of paths (feasible paths when `PSA_FLAG_FEASIBILITY_FILTERED`).
    pub path_count: PathCount,

    /// `path_count` clamped to `u64` (`PSA_FLAG_COUNT_SATURATED` set when clamped).
    pub sat_count: u64,

    /// `log2(path_count)`; negative infinity when there is no path.
    pub log2_paths: f64,

    /// V(G) = |E| - |B| + 2 (§8.2.5), taken from Phase 4. Stored as u16.
    pub cyclomatic: u16,

    /// Number of blocks on the longest path through the back-edge-free DAG.
    pub max_path_len: u16,

    /// Recursive expansions included in the summary — always 0 (see `RecursiveHandler`).
    pub unwind_depth: u16,

    /// `PSA_FLAG_*` bits.
    pub flags: u16,

    /// Why no ROBDD is stored, when `PSA_FLAG_BDD_UNAVAILABLE` is set.
    pub unavailable: Option<SummaryUnavailable>,
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
///  4       4    n_vars           : u32   number of Boolean variables (edges + loop entries)
///  8       4    n_nodes          : u32   number of ROBDD nodes (including shared terminals)
/// 12       4    root_node        : u32   root node index, or PSA_NO_ROOT
/// 16       8    sat_count        : u64   path count, clamped (see PSA_FLAG_COUNT_SATURATED)
/// 24       2    cyclomatic       : u16   V(G) = |E| - |B| + 2
/// 26       2    max_path_len     : u16   blocks on the longest acyclic path
/// 28       2    unwind_depth     : u16   recursive expansions included (always 0)
/// 30       2    flags            : u16   PSA_FLAG_* bits
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
    pub flags: u16,
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
        buf[30..32].copy_from_slice(&self.flags.to_le_bytes());
        buf
    }

    /// Deserialize from 32 raw bytes.
    pub fn from_bytes(buf: &[u8; 32]) -> Self {
        Self {
            sym_id: u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]),
            n_vars: u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]),
            n_nodes: u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]),
            root_node: u32::from_le_bytes([buf[12], buf[13], buf[14], buf[15]]),
            sat_count: u64::from_le_bytes([
                buf[16], buf[17], buf[18], buf[19], buf[20], buf[21], buf[22], buf[23],
            ]),
            cyclomatic: u16::from_le_bytes([buf[24], buf[25]]),
            max_path_len: u16::from_le_bytes([buf[26], buf[27]]),
            unwind_depth: u16::from_le_bytes([buf[28], buf[29]]),
            flags: u16::from_le_bytes([buf[30], buf[31]]),
        }
    }

    /// Build a `FunctionPSAHeader` from a completed `FunctionROBDD`.
    pub fn from_robdd(robdd: &FunctionROBDD) -> Self {
        Self {
            sym_id: robdd.sym_id,
            n_vars: robdd.ordering.n_vars() as u32,
            n_nodes: robdd.nodes.len() as u32,
            root_node: robdd.root,
            sat_count: robdd.sat_count,
            cyclomatic: robdd.cyclomatic,
            max_path_len: robdd.max_path_len,
            unwind_depth: robdd.unwind_depth,
            flags: robdd.flags,
        }
    }

    /// True iff `flag` (a `PSA_FLAG_*` bit) is set.
    #[inline]
    pub fn has_flag(&self, flag: u16) -> bool {
        self.flags & flag != 0
    }

    /// True iff `sat_count` is the exact path count (not clamped).
    #[inline]
    pub fn sat_count_is_exact(&self) -> bool {
        !self.has_flag(PSA_FLAG_COUNT_SATURATED)
    }

    /// True iff an ROBDD is stored for this function.
    #[inline]
    pub fn has_bdd(&self) -> bool {
        !self.has_flag(PSA_FLAG_BDD_UNAVAILABLE) && self.root_node != PSA_NO_ROOT
    }
}

// ── Full PSA artifact ─────────────────────────────────────────────────────────

/// The complete PathSummaryArtifact produced by Phase 8 (§8.6).
///
/// Contains all five binary sections:
/// - HEADER (64 B)
/// - FUNCTION DIRECTORY (n × 32 B, sorted by sym_id)
/// - VARIABLE ORDERING TABLES (per-function variable code[n_vars])
/// - ROBDD NODE ARRAYS (per-function ROBDDNode[], compacted)
/// - PATH METRICS TABLE (n × 16 B, always mmap'd in Phase 9)
pub struct PathSummaryArtifact {
    /// File format version ([`PSA_FORMAT_VERSION`]).
    pub format_version: u32,

    /// CRC-64/ECMA-182 of the CFA artifact bytes this PSA was built from (§8.8 Invariant 4).
    pub cfa_hash: u64,

    /// First 64 bits (little-endian) of the SHA-256 canonical content digest of the SSA
    /// artifact — the same digest the Phase 7 hash chain records for the SSA artifact.
    pub ssa_hash: u64,

    /// Function directory sorted by sym_id (binary search for O(log n) lookup).
    pub function_dir: Vec<FunctionPSAHeader>,

    /// Per-function variable ordering tables: `ordering_tables[i][level]` = variable code
    /// (see `psa::ordering`): a CFA edge id, or `LOOP_ENTRY_FLAG | header_block`.
    pub ordering_tables: Vec<Vec<u32>>,

    /// Per-function ROBDD node arrays. `node_arrays[i]` = ROBDDNode[] for function i.
    pub node_arrays: Vec<Vec<ROBDDNode>>,

    /// Path metrics table — always memory-mapped in Phase 9 for O(1) access per function.
    pub metrics: Vec<PathMetrics>,

    /// Total stored ROBDD nodes across all functions (§8.7 size estimate).
    pub total_nodes: u64,
}

impl PathSummaryArtifact {
    /// Look up the directory index of a function by sym_id using binary search (O(log n)).
    pub fn function_index(&self, sym_id: u32) -> Option<usize> {
        self.function_dir
            .binary_search_by_key(&sym_id, |h| h.sym_id)
            .ok()
    }

    /// Look up the PSA header for a function by sym_id using binary search (O(log n)).
    pub fn function_header(&self, sym_id: u32) -> Option<&FunctionPSAHeader> {
        let idx = self.function_index(sym_id)?;
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
            sym_id: 42,
            n_vars: 35,
            n_nodes: 500,
            root_node: 100,
            sat_count: 1024,
            cyclomatic: 7,
            max_path_len: 15,
            unwind_depth: 0,
            flags: 0,
        };
        assert_eq!(h.to_bytes().len(), 32);
    }

    #[test]
    fn function_psa_header_round_trip() {
        let h = FunctionPSAHeader {
            sym_id: 1234,
            n_vars: 50,
            n_nodes: 800,
            root_node: 200,
            sat_count: 9_999_999_999,
            cyclomatic: 11,
            max_path_len: 30,
            unwind_depth: 3,
            flags: PSA_FLAG_HAS_LOOPS,
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
