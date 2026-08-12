//! PathSummaryArtifact binary I/O (.psa format) (§8.6).
//!
//! **PSA File Format v1.0 (all integers: little-endian):**
//!
//! ```text
//! ╔══════════════════════════════════════════════════════════════════╗
//! ║  Section            │ Size         │ Description                ║
//! ╠═════════════════════╪══════════════╪════════════════════════════╣
//! ║ HEADER              │ 64 B         │ Magic, counts, cfa_hash    ║
//! ╠═════════════════════╪══════════════╪════════════════════════════╣
//! ║ FUNCTION DIR.       │ n_f × 32 B   │ FunctionPSAHeader[] sorted ║
//! ║                     │              │ by sym_id. Binary search.  ║
//! ╠═════════════════════╪══════════════╪════════════════════════════╣
//! ║ VARIABLE ORDERING   │ variable     │ Per-function: edge_id[] of ║
//! ║ TABLES              │              │ length n_vars. Maps        ║
//! ║                     │              │ var_idx → CFA edge_id.     ║
//! ╠═════════════════════╪══════════════╪════════════════════════════╣
//! ║ ROBDD NODE ARRAYS   │ variable     │ Per-function: ROBDDNode[]  ║
//! ║                     │              │ (12 bytes each). Lazy-     ║
//! ║                     │              │ loaded on first path query.║
//! ╠═════════════════════╪══════════════╪════════════════════════════╣
//! ║ PATH METRICS TABLE  │ n_f × 16 B   │ (cyclomatic:u16,           ║
//! ║                     │              │  max_path:u16, sat_lo:u32, ║
//! ║                     │              │  sat_hi:u32, mean:f32)     ║
//! ║                     │              │ Hot path — always mmap'd.  ║
//! ╠═════════════════════╪══════════════╪════════════════════════════╣
//! ║ CHECKSUM            │ 8 B          │ CRC-64/ECMA                ║
//! ╚═════════════════════╩══════════════╩════════════════════════════╝
//! ```
//!
//! **Header layout (64 bytes):**
//! ```text
//!  0- 7: magic     : u64   = PSA_MAGIC (b"OPENHPSA")
//!  8-11: version   : u32   = 1
//! 12-15: n_funcs   : u32   number of functions
//! 16-23: cfa_hash  : u64   CRC-64 of the CFA artifact (§8.8 Invariant 4)
//! 24-31: ssa_hash  : u64   CRC-64 of the SSA artifact
//! 32-39: total_nodes: u64  total ROBDD nodes across all functions
//! 40-47: dir_offset : u64  byte offset of FUNCTION DIR section
//! 48-55: metrics_offset: u64  byte offset of PATH METRICS TABLE
//! 56-63: _reserved : u64   (must be zero)
//! ```

use std::io;
use std::path::Path;
use std::fs;

use crate::psa::bdd::node::ROBDDNode;
use crate::psa::metrics::PathMetrics;
use crate::psa::types::{FunctionPSAHeader, PathSummaryArtifact, PSA_MAGIC};

/// Header size in bytes.
const HEADER_SIZE: usize = 64;
/// Function directory entry size in bytes.
const DIR_ENTRY_SIZE: usize = 32;
/// ROBDD node size in bytes.
const NODE_SIZE: usize = 12;
/// Path metrics entry size in bytes.
const METRICS_SIZE: usize = 16;
/// CRC-64 checksum size in bytes.
const CHECKSUM_SIZE: usize = 8;

/// Serializer and deserializer for the PathSummaryArtifact binary format (.psa).
pub struct PathSummarySerializer;

impl PathSummarySerializer {
    /// Serialize the PathSummaryArtifact to the .psa file at `path`.
    ///
    /// Writes all sections in order: HEADER, FUNCTION DIR, VARIABLE ORDERING TABLES,
    /// ROBDD NODE ARRAYS, PATH METRICS TABLE, CHECKSUM.
    ///
    /// The CHECKSUM is CRC-64/ECMA of all preceding bytes.
    pub fn write(artifact: &PathSummaryArtifact, path: &Path) -> io::Result<()> {
        let n = artifact.function_count();

        // ── Pre-compute section sizes and offsets ─────────────────────────────
        let dir_size = n * DIR_ENTRY_SIZE;

        // Variable ordering: each function's ordering is n_vars × 4 bytes.
        let ordering_size: usize = artifact
            .ordering_tables
            .iter()
            .map(|t| t.len() * 4)
            .sum();

        // ROBDD node arrays: each node is 12 bytes.
        let node_array_size: usize = artifact
            .node_arrays
            .iter()
            .map(|a| a.len() * NODE_SIZE)
            .sum();

        let metrics_size = n * METRICS_SIZE;

        // Section byte offsets.
        let dir_offset = HEADER_SIZE as u64;
        let ordering_offset = dir_offset + dir_size as u64;
        let node_array_offset = ordering_offset + ordering_size as u64;
        let metrics_offset = node_array_offset + node_array_size as u64;
        let checksum_offset = metrics_offset + metrics_size as u64;
        let total_size = checksum_offset as usize + CHECKSUM_SIZE;

        // ── Build the complete file in memory ─────────────────────────────────
        let mut buf: Vec<u8> = Vec::with_capacity(total_size);

        // ── HEADER (64 bytes) ─────────────────────────────────────────────────
        buf.extend_from_slice(&PSA_MAGIC.to_le_bytes());
        buf.extend_from_slice(&artifact.format_version.to_le_bytes());
        buf.extend_from_slice(&(n as u32).to_le_bytes());
        buf.extend_from_slice(&artifact.cfa_hash.to_le_bytes());
        buf.extend_from_slice(&artifact.ssa_hash.to_le_bytes());
        buf.extend_from_slice(&artifact.total_nodes.to_le_bytes());
        buf.extend_from_slice(&dir_offset.to_le_bytes());
        buf.extend_from_slice(&metrics_offset.to_le_bytes());
        // _reserved (8 bytes)
        buf.extend_from_slice(&0u64.to_le_bytes());
        debug_assert_eq!(buf.len(), HEADER_SIZE);

        // ── FUNCTION DIRECTORY (n × 32 bytes) ────────────────────────────────
        for header in &artifact.function_dir {
            buf.extend_from_slice(&header.to_bytes());
        }
        debug_assert_eq!(buf.len(), HEADER_SIZE + dir_size);

        // ── VARIABLE ORDERING TABLES (per-function edge_id[n_vars]) ──────────
        for ordering_table in &artifact.ordering_tables {
            for &edge_id in ordering_table {
                buf.extend_from_slice(&edge_id.to_le_bytes());
            }
        }

        // ── ROBDD NODE ARRAYS (per-function ROBDDNode[n_nodes]) ───────────────
        for node_array in &artifact.node_arrays {
            for node in node_array {
                // ROBDDNode: var(2) + flags(2) + lo(4) + hi(4) = 12 bytes.
                buf.extend_from_slice(&node.var.to_le_bytes());
                buf.extend_from_slice(&node.flags.to_le_bytes());
                buf.extend_from_slice(&node.lo.to_le_bytes());
                buf.extend_from_slice(&node.hi.to_le_bytes());
            }
        }

        // ── PATH METRICS TABLE (n × 16 bytes, always mmap'd in Phase 9) ──────
        for metric in &artifact.metrics {
            buf.extend_from_slice(&metric.to_bytes());
        }

        // ── CHECKSUM (CRC-64/ECMA of all preceding bytes) ─────────────────────
        let crc = crc64_ecma(&buf);
        buf.extend_from_slice(&crc.to_le_bytes());

        // ── Write to disk ─────────────────────────────────────────────────────
        fs::write(path, &buf)?;
        Ok(())
    }

    /// Deserialize a PathSummaryArtifact from a .psa file at `path`.
    ///
    /// Validates:
    /// 1. PSA_MAGIC matches.
    /// 2. CRC-64/ECMA checksum matches (verifying file integrity).
    pub fn read(path: &Path) -> io::Result<PathSummaryArtifact> {
        let bytes = fs::read(path)?;

        if bytes.len() < HEADER_SIZE + CHECKSUM_SIZE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "PSA file too small",
            ));
        }

        // ── Validate CRC-64 ───────────────────────────────────────────────────
        let data_end = bytes.len() - CHECKSUM_SIZE;
        let stored_crc = u64::from_le_bytes(
            bytes[data_end..data_end + 8].try_into().unwrap(),
        );
        let computed_crc = crc64_ecma(&bytes[..data_end]);
        if stored_crc != computed_crc {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "PSA CRC-64 mismatch: stored=0x{:016X}, computed=0x{:016X}",
                    stored_crc, computed_crc
                ),
            ));
        }

        // ── Parse HEADER ──────────────────────────────────────────────────────
        let magic = u64::from_le_bytes(bytes[0..8].try_into().unwrap());
        if magic != PSA_MAGIC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("PSA magic mismatch: got 0x{:016X}", magic),
            ));
        }

        let format_version = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
        let n_funcs = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
        let cfa_hash = u64::from_le_bytes(bytes[16..24].try_into().unwrap());
        let ssa_hash = u64::from_le_bytes(bytes[24..32].try_into().unwrap());
        let total_nodes = u64::from_le_bytes(bytes[32..40].try_into().unwrap());
        // dir_offset (40..48) — always HEADER_SIZE in v1.
        let metrics_offset = u64::from_le_bytes(bytes[48..56].try_into().unwrap()) as usize;

        // ── Parse FUNCTION DIRECTORY ──────────────────────────────────────────
        let mut pos = HEADER_SIZE;
        let mut function_dir: Vec<FunctionPSAHeader> = Vec::with_capacity(n_funcs);
        for _ in 0..n_funcs {
            let entry_bytes: &[u8; 32] = bytes[pos..pos + DIR_ENTRY_SIZE].try_into().unwrap();
            function_dir.push(FunctionPSAHeader::from_bytes(entry_bytes));
            pos += DIR_ENTRY_SIZE;
        }

        // ── Parse VARIABLE ORDERING TABLES ───────────────────────────────────
        let mut ordering_tables: Vec<Vec<u32>> = Vec::with_capacity(n_funcs);
        for i in 0..n_funcs {
            let n_vars = function_dir[i].n_vars as usize;
            let mut table: Vec<u32> = Vec::with_capacity(n_vars);
            for _ in 0..n_vars {
                let eid = u32::from_le_bytes(bytes[pos..pos + 4].try_into().unwrap());
                table.push(eid);
                pos += 4;
            }
            ordering_tables.push(table);
        }

        // ── Parse ROBDD NODE ARRAYS ───────────────────────────────────────────
        let mut node_arrays: Vec<Vec<ROBDDNode>> = Vec::with_capacity(n_funcs);
        for i in 0..n_funcs {
            let n_nodes = function_dir[i].n_nodes as usize;
            let mut arr: Vec<ROBDDNode> = Vec::with_capacity(n_nodes);
            for _ in 0..n_nodes {
                let var = u16::from_le_bytes(bytes[pos..pos + 2].try_into().unwrap());
                let flags = u16::from_le_bytes(bytes[pos + 2..pos + 4].try_into().unwrap());
                let lo = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().unwrap());
                let hi = u32::from_le_bytes(bytes[pos + 8..pos + 12].try_into().unwrap());
                arr.push(ROBDDNode { var, flags, lo, hi });
                pos += NODE_SIZE;
            }
            node_arrays.push(arr);
        }

        // ── Parse PATH METRICS TABLE ──────────────────────────────────────────
        pos = metrics_offset;
        let mut metrics: Vec<PathMetrics> = Vec::with_capacity(n_funcs);
        for _ in 0..n_funcs {
            let entry: &[u8; 16] = bytes[pos..pos + METRICS_SIZE].try_into().unwrap();
            metrics.push(PathMetrics::from_bytes(entry));
            pos += METRICS_SIZE;
        }

        Ok(PathSummaryArtifact {
            format_version,
            cfa_hash,
            ssa_hash,
            function_dir,
            ordering_tables,
            node_arrays,
            metrics,
            total_nodes,
        })
    }
}

/// CRC-64/ECMA polynomial computation (§8.6, §8.8 Invariant 4).
///
/// Implements CRC-64/ECMA-182 with polynomial 0x42F0E1EBA9EA3693.
/// This matches the checksum algorithm used by all other OpenHeart artifact serializers.
fn crc64_ecma(data: &[u8]) -> u64 {
    const POLY: u64 = 0x42F0E1EBA9EA3693;
    let mut crc: u64 = 0xFFFF_FFFF_FFFF_FFFF;
    for &byte in data {
        let b = byte as u64;
        for bit in (0..8).rev() {
            let bit_val = (b >> bit) & 1;
            let top_bit = (crc >> 63) & 1;
            crc <<= 1;
            crc ^= bit_val;
            if top_bit != 0 {
                crc ^= POLY;
            }
        }
    }
    crc ^ 0xFFFF_FFFF_FFFF_FFFF
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    fn make_minimal_artifact() -> PathSummaryArtifact {
        use crate::psa::bdd::node::ROBDDNode;
        PathSummaryArtifact {
            format_version: 1,
            cfa_hash: 0xDEADBEEF_CAFEBABE,
            ssa_hash: 0xABCD_1234_5678_9DEF,
            function_dir: vec![FunctionPSAHeader {
                sym_id: 1,
                n_vars: 2,
                n_nodes: 3,
                root_node: 2,
                sat_count: 2,
                cyclomatic: 3,
                max_path_len: 5,
                unwind_depth: 0,
                _reserved: 0,
            }],
            ordering_tables: vec![vec![10, 20]],
            node_arrays: vec![vec![
                ROBDDNode::false_terminal(),
                ROBDDNode::true_terminal(),
                ROBDDNode::internal(0, 0, 1),
            ]],
            metrics: vec![PathMetrics::new(3, 5, 2, 0)],
            total_nodes: 3,
        }
    }

    #[test]
    fn write_and_read_round_trip() {
        let artifact = make_minimal_artifact();
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path();

        PathSummarySerializer::write(&artifact, path).unwrap();
        let loaded = PathSummarySerializer::read(path).unwrap();

        assert_eq!(loaded.format_version, 1);
        assert_eq!(loaded.cfa_hash, 0xDEADBEEF_CAFEBABE);
        assert_eq!(loaded.function_count(), 1);
        assert_eq!(loaded.function_dir[0].sym_id, 1);
        assert_eq!(loaded.function_dir[0].sat_count, 2);
        assert_eq!(loaded.function_dir[0].cyclomatic, 3);
        assert_eq!(loaded.ordering_tables[0], vec![10, 20]);
        assert_eq!(loaded.node_arrays[0].len(), 3);
        assert_eq!(loaded.metrics[0].cyclomatic, 3);
    }

    #[test]
    fn crc64_deterministic() {
        let data = b"OpenHeart Phase 8";
        let c1 = crc64_ecma(data);
        let c2 = crc64_ecma(data);
        assert_eq!(c1, c2);
    }

    #[test]
    fn binary_search_function_header() {
        let artifact = make_minimal_artifact();
        let h = artifact.function_header(1).unwrap();
        assert_eq!(h.sym_id, 1);
        assert!(artifact.function_header(999).is_none());
    }
}
