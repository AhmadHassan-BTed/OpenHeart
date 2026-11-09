//! PathSummaryArtifact binary I/O (.psa format) (§8.6).
//!
//! **PSA File Format v2 (all integers: little-endian):**
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
//! ║ VARIABLE ORDERING   │ variable     │ Per-function: code[] of    ║
//! ║ TABLES              │              │ length n_vars. Maps level  ║
//! ║                     │              │ → edge id / loop entry.    ║
//! ╠═════════════════════╪══════════════╪════════════════════════════╣
//! ║ ROBDD NODE ARRAYS   │ variable     │ Per-function: ROBDDNode[]  ║
//! ║                     │              │ (12 bytes each), compacted,║
//! ║                     │              │ children before parents.   ║
//! ╠═════════════════════╪══════════════╪════════════════════════════╣
//! ║ PATH METRICS TABLE  │ n_f × 16 B   │ (cyclomatic:u16,           ║
//! ║                     │              │  max_path:u16, unwind:u16, ║
//! ║                     │              │  flags:u16, log2:f64)      ║
//! ║                     │              │ Hot path — always mmap'd.  ║
//! ╠═════════════════════╪══════════════╪════════════════════════════╣
//! ║ CHECKSUM            │ 8 B          │ CRC-64/ECMA-182            ║
//! ╚═════════════════════╩══════════════╩════════════════════════════╝
//! ```
//!
//! **Header layout (64 bytes):**
//! ```text
//!  0- 7: magic     : u64   = PSA_MAGIC (b"OPENHPSA")
//!  8-11: version   : u32   = 2
//! 12-15: n_funcs   : u32   number of functions
//! 16-23: cfa_hash  : u64   CRC-64 of the CFA artifact (§8.8 Invariant 4)
//! 24-31: ssa_hash  : u64   first 64 bits of the SSA canonical SHA-256 digest
//! 32-39: total_nodes: u64  total ROBDD nodes across all functions
//! 40-47: dir_offset : u64  byte offset of FUNCTION DIR section
//! 48-55: metrics_offset: u64  byte offset of PATH METRICS TABLE
//! 56-63: _reserved : u64   (must be zero)
//! ```
//!
//! The checksum is the CRC-64/ECMA-182 implementation shared by every OpenHeart artifact
//! (`ingestion::serializer::crc64_ecma`). The reader treats the file as untrusted: every read
//! is bounds-checked, every declared count is validated against the bytes that remain before
//! anything is allocated, and every node array is validated (ids, order, canonicity) so the
//! query API can traverse it safely. Malformed input yields `InvalidData`, never a panic.

use std::fs;
use std::io;
use std::path::Path;

use crate::ingestion::serializer::crc64_ecma;
use crate::psa::bdd::node::ROBDDNode;
use crate::psa::bdd::verify_node_array;
use crate::psa::metrics::PathMetrics;
use crate::psa::ordering::VariableOrdering;
use crate::psa::types::{
    FunctionPSAHeader, PathSummaryArtifact, PSA_FORMAT_VERSION, PSA_MAGIC, PSA_NO_ROOT,
};

/// Header size in bytes.
const HEADER_SIZE: usize = 64;
/// Function directory entry size in bytes.
const DIR_ENTRY_SIZE: usize = 32;
/// ROBDD node size in bytes.
const NODE_SIZE: usize = 12;
/// Path metrics entry size in bytes.
const METRICS_SIZE: usize = 16;
/// Ordering table entry size in bytes.
const ORDERING_ENTRY_SIZE: usize = 4;
/// CRC-64 checksum size in bytes.
const CHECKSUM_SIZE: usize = 8;

/// Serializer and deserializer for the PathSummaryArtifact binary format (.psa).
pub struct PathSummarySerializer;

impl PathSummarySerializer {
    /// Serialize the PathSummaryArtifact to the .psa file at `path`.
    pub fn write(artifact: &PathSummaryArtifact, path: &Path) -> io::Result<()> {
        let buf = Self::to_bytes(artifact)?;
        fs::write(path, &buf)
    }

    /// Serialize the PathSummaryArtifact to bytes.
    ///
    /// Writes all sections in order: HEADER, FUNCTION DIR, VARIABLE ORDERING TABLES,
    /// ROBDD NODE ARRAYS, PATH METRICS TABLE, CHECKSUM (CRC-64/ECMA of all preceding bytes).
    pub fn to_bytes(artifact: &PathSummaryArtifact) -> io::Result<Vec<u8>> {
        let n = artifact.function_count();
        if artifact.ordering_tables.len() != n
            || artifact.node_arrays.len() != n
            || artifact.metrics.len() != n
        {
            return Err(invalid(format!(
                "PSA sections disagree: {} directory entries, {} ordering tables, \
                 {} node arrays, {} metrics records",
                n,
                artifact.ordering_tables.len(),
                artifact.node_arrays.len(),
                artifact.metrics.len()
            )));
        }
        let n_u32 = u32::try_from(n).map_err(|_| invalid("too many functions for PSA"))?;

        // ── Pre-compute section sizes and offsets ─────────────────────────────
        let dir_size = n * DIR_ENTRY_SIZE;
        let ordering_size: usize = artifact
            .ordering_tables
            .iter()
            .map(|t| t.len() * ORDERING_ENTRY_SIZE)
            .sum();
        let node_array_size: usize = artifact
            .node_arrays
            .iter()
            .map(|a| a.len() * NODE_SIZE)
            .sum();
        let metrics_size = n * METRICS_SIZE;

        let dir_offset = HEADER_SIZE as u64;
        let ordering_offset = dir_offset + dir_size as u64;
        let node_array_offset = ordering_offset + ordering_size as u64;
        let metrics_offset = node_array_offset + node_array_size as u64;
        let checksum_offset = metrics_offset + metrics_size as u64;
        let total_size = checksum_offset as usize + CHECKSUM_SIZE;

        let mut buf: Vec<u8> = Vec::with_capacity(total_size);

        // ── HEADER (64 bytes) ─────────────────────────────────────────────────
        buf.extend_from_slice(&PSA_MAGIC.to_le_bytes());
        buf.extend_from_slice(&artifact.format_version.to_le_bytes());
        buf.extend_from_slice(&n_u32.to_le_bytes());
        buf.extend_from_slice(&artifact.cfa_hash.to_le_bytes());
        buf.extend_from_slice(&artifact.ssa_hash.to_le_bytes());
        buf.extend_from_slice(&artifact.total_nodes.to_le_bytes());
        buf.extend_from_slice(&dir_offset.to_le_bytes());
        buf.extend_from_slice(&metrics_offset.to_le_bytes());
        buf.extend_from_slice(&0u64.to_le_bytes());
        debug_assert_eq!(buf.len(), HEADER_SIZE);

        // ── FUNCTION DIRECTORY (n × 32 bytes) ────────────────────────────────
        for (i, header) in artifact.function_dir.iter().enumerate() {
            if header.n_vars as usize != artifact.ordering_tables[i].len()
                || header.n_nodes as usize != artifact.node_arrays[i].len()
            {
                return Err(invalid(format!(
                    "PSA directory entry for sym_id={} disagrees with its tables",
                    header.sym_id
                )));
            }
            buf.extend_from_slice(&header.to_bytes());
        }

        // ── VARIABLE ORDERING TABLES (per-function code[n_vars]) ─────────────
        for ordering_table in &artifact.ordering_tables {
            for &code in ordering_table {
                buf.extend_from_slice(&code.to_le_bytes());
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
        debug_assert_eq!(buf.len() as u64, checksum_offset);

        // ── CHECKSUM (CRC-64/ECMA of all preceding bytes) ─────────────────────
        let crc = crc64_ecma(&buf);
        buf.extend_from_slice(&crc.to_le_bytes());
        Ok(buf)
    }

    /// Deserialize a PathSummaryArtifact from a .psa file at `path`.
    pub fn read(path: &Path) -> io::Result<PathSummaryArtifact> {
        let bytes = fs::read(path)?;
        Self::from_bytes(&bytes)
    }

    /// Deserialize a PathSummaryArtifact from bytes.
    ///
    /// Validates:
    /// 1. Length, CRC-64/ECMA checksum and PSA_MAGIC.
    /// 2. Format version, section offsets and every declared count against the remaining bytes.
    /// 3. Every ordering table (decodable, no duplicate variable) and node array (terminals,
    ///    ids, variable order, canonicity) and every root id.
    pub fn from_bytes(bytes: &[u8]) -> io::Result<PathSummaryArtifact> {
        if bytes.len() < HEADER_SIZE + CHECKSUM_SIZE {
            return Err(invalid("PSA file too small"));
        }

        // ── Validate CRC-64 ───────────────────────────────────────────────────
        let data_end = bytes.len() - CHECKSUM_SIZE;
        let mut tail = Cursor::new(&bytes[data_end..]);
        let stored_crc = tail.u64()?;
        let computed_crc = crc64_ecma(&bytes[..data_end]);
        if stored_crc != computed_crc {
            return Err(invalid(format!(
                "PSA CRC-64 mismatch: stored=0x{:016X}, computed=0x{:016X}",
                stored_crc, computed_crc
            )));
        }
        let data = &bytes[..data_end];

        // ── Parse HEADER ──────────────────────────────────────────────────────
        let mut cur = Cursor::new(data);
        let magic = cur.u64()?;
        if magic != PSA_MAGIC {
            return Err(invalid(format!("PSA magic mismatch: got 0x{:016X}", magic)));
        }
        let format_version = cur.u32()?;
        if format_version != PSA_FORMAT_VERSION {
            return Err(invalid(format!(
                "unsupported PSA format version {} (expected {})",
                format_version, PSA_FORMAT_VERSION
            )));
        }
        let n_funcs = cur.u32()? as usize;
        let cfa_hash = cur.u64()?;
        let ssa_hash = cur.u64()?;
        let total_nodes = cur.u64()?;
        let dir_offset = cur.u64()?;
        let metrics_offset = cur.u64()?;
        let _reserved = cur.u64()?;
        if dir_offset != HEADER_SIZE as u64 {
            return Err(invalid(format!(
                "PSA directory offset {} (expected {})",
                dir_offset, HEADER_SIZE
            )));
        }

        // ── Parse FUNCTION DIRECTORY ──────────────────────────────────────────
        cur.require(n_funcs, DIR_ENTRY_SIZE, "function directory")?;
        let mut function_dir: Vec<FunctionPSAHeader> = Vec::with_capacity(n_funcs);
        for _ in 0..n_funcs {
            let entry = cur.array::<DIR_ENTRY_SIZE>()?;
            function_dir.push(FunctionPSAHeader::from_bytes(&entry));
        }
        if function_dir.windows(2).any(|w| w[0].sym_id >= w[1].sym_id) {
            return Err(invalid("PSA function directory is not strictly sorted by sym_id"));
        }

        // ── Parse VARIABLE ORDERING TABLES ───────────────────────────────────
        let mut ordering_tables: Vec<Vec<u32>> = Vec::with_capacity(n_funcs);
        for header in &function_dir {
            let n_vars = header.n_vars as usize;
            cur.require(n_vars, ORDERING_ENTRY_SIZE, "variable ordering table")?;
            let mut table: Vec<u32> = Vec::with_capacity(n_vars);
            for _ in 0..n_vars {
                table.push(cur.u32()?);
            }
            VariableOrdering::from_codes(&table).map_err(|msg| {
                invalid(format!(
                    "PSA ordering table of sym_id={}: {}",
                    header.sym_id, msg
                ))
            })?;
            ordering_tables.push(table);
        }

        // ── Parse ROBDD NODE ARRAYS ───────────────────────────────────────────
        let mut node_arrays: Vec<Vec<ROBDDNode>> = Vec::with_capacity(n_funcs);
        let mut counted_nodes: u64 = 0;
        for header in &function_dir {
            let n_nodes = header.n_nodes as usize;
            cur.require(n_nodes, NODE_SIZE, "ROBDD node array")?;
            let mut arr: Vec<ROBDDNode> = Vec::with_capacity(n_nodes);
            for _ in 0..n_nodes {
                let var = cur.u16()?;
                let flags = cur.u16()?;
                let lo = cur.u32()?;
                let hi = cur.u32()?;
                arr.push(ROBDDNode { var, flags, lo, hi });
            }
            verify_node_array(&arr, Some(header.n_vars.min(u16::MAX as u32) as u16)).map_err(
                |msg| invalid(format!("PSA node array of sym_id={}: {}", header.sym_id, msg)),
            )?;
            if header.root_node != PSA_NO_ROOT && header.root_node as usize >= arr.len() {
                return Err(invalid(format!(
                    "PSA root {} of sym_id={} is outside its {} nodes",
                    header.root_node,
                    header.sym_id,
                    arr.len()
                )));
            }
            counted_nodes += n_nodes as u64;
            node_arrays.push(arr);
        }
        if counted_nodes != total_nodes {
            return Err(invalid(format!(
                "PSA header declares {} nodes but the arrays hold {}",
                total_nodes, counted_nodes
            )));
        }

        // ── Parse PATH METRICS TABLE ──────────────────────────────────────────
        if metrics_offset != cur.position() as u64 {
            return Err(invalid(format!(
                "PSA metrics offset {} does not follow the node arrays (at {})",
                metrics_offset,
                cur.position()
            )));
        }
        cur.require(n_funcs, METRICS_SIZE, "path metrics table")?;
        let mut metrics: Vec<PathMetrics> = Vec::with_capacity(n_funcs);
        for _ in 0..n_funcs {
            let entry = cur.array::<METRICS_SIZE>()?;
            metrics.push(PathMetrics::from_bytes(&entry));
        }
        if cur.remaining() != 0 {
            return Err(invalid(format!(
                "{} unexpected trailing bytes before the PSA checksum",
                cur.remaining()
            )));
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

/// `InvalidData` error with a message.
fn invalid(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

/// Bounds-checked little-endian reader over an untrusted byte slice.
struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    #[inline]
    fn position(&self) -> usize {
        self.pos
    }

    #[inline]
    fn remaining(&self) -> usize {
        self.bytes.len() - self.pos
    }

    /// Fail unless `count` records of `size` bytes remain (checked arithmetic), so that
    /// allocations sized by `count` are bounded by the input length.
    fn require(&self, count: usize, size: usize, what: &str) -> io::Result<()> {
        match count.checked_mul(size) {
            Some(needed) if needed <= self.remaining() => Ok(()),
            _ => Err(invalid(format!(
                "PSA {} declares {} records of {} bytes but only {} bytes remain",
                what,
                count,
                size,
                self.remaining()
            ))),
        }
    }

    fn take(&mut self, n: usize) -> io::Result<&'a [u8]> {
        if n > self.remaining() {
            return Err(invalid(format!(
                "PSA truncated: needed {} bytes at offset {}, {} remain",
                n,
                self.pos,
                self.remaining()
            )));
        }
        let s = &self.bytes[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    fn array<const N: usize>(&mut self) -> io::Result<[u8; N]> {
        let s = self.take(N)?;
        let mut out = [0u8; N];
        out.copy_from_slice(s);
        Ok(out)
    }

    fn u16(&mut self) -> io::Result<u16> {
        Ok(u16::from_le_bytes(self.array::<2>()?))
    }

    fn u32(&mut self) -> io::Result<u32> {
        Ok(u32::from_le_bytes(self.array::<4>()?))
    }

    fn u64(&mut self) -> io::Result<u64> {
        Ok(u64::from_le_bytes(self.array::<8>()?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    fn make_minimal_artifact() -> PathSummaryArtifact {
        use crate::psa::bdd::node::ROBDDNode;
        PathSummaryArtifact {
            format_version: PSA_FORMAT_VERSION,
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
                flags: 0,
            }],
            ordering_tables: vec![vec![10, 20]],
            node_arrays: vec![vec![
                ROBDDNode::false_terminal(),
                ROBDDNode::true_terminal(),
                ROBDDNode::internal(0, 0, 1),
            ]],
            metrics: vec![PathMetrics::new(3, 5, 0, 0, 1.0)],
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

        assert_eq!(loaded.format_version, PSA_FORMAT_VERSION);
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
