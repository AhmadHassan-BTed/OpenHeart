//! CFGArtifact binary serializer and deserializer for `.cfa` files (§4.6).
//!
//! Format version 2. Every directory offset is taken from the bytes actually written,
//! the EXIT block id is stored explicitly (blocks are numbered in reverse post-order, so
//! EXIT is not the last block), and the block statements, token ranges, loop forest,
//! lowered nodes and diagnostics are persisted so that [`CFGArtifact::deserialize`]
//! reconstructs every `FunctionCFGData` field.
//!
//! Layout (little endian):
//! * header (64 B): magic u64, version u32, function_count u32, total_blocks u32,
//!   total_edges u32, sta_hash u64, bpa_hash u64, reserved[24]
//! * directory (function_count × 20 B): sym_id, offset, block_count, edge_count, pad
//! * per function: sub-header (32 B: sym_id, block_count, edge_count, entry, exit,
//!   cyclomatic u16, max_loop_depth u8, pad u8, decl_node, body_node), succ CSR,
//!   pred CSR, pred edge ids, edge types, idom, DF CSR, blocks, loop info, lowered nodes,
//!   diagnostics
//! * CRC-64 of everything before it (8 B)

use crate::cfg::analysis::LoopInfo;
use crate::cfg::builder::state::{CfgDiagnostic, CfgDiagnosticKind};
use crate::cfg::builder::FunctionCFGData;
use crate::core::types::cfg::{BasicBlock, CFGEdgeType};
use crate::ingestion::serializer::crc64_ecma;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

pub const CFA_MAGIC: u64 = 0x43464130_00010000; // "CFA\x00\x00\x01\x00\x00"
pub const CFA_HEADER_SIZE: usize = 64;
pub const CFA_FORMAT_VERSION: u32 = 2;
const DIR_ENTRY_SIZE: usize = 20;

#[derive(Debug, Clone)]
pub struct CFGArtifact {
    pub magic: u64,
    pub format_version: u32,
    pub function_count: u32,
    pub total_blocks: u32,
    pub total_edges: u32,
    pub sta_hash: u64,
    pub bpa_hash: u64,
    pub functions: Vec<FunctionCFGData>,
}

fn checked_u32(v: usize, what: &str) -> Result<u32, String> {
    u32::try_from(v).map_err(|_| format!("CFA {} {} exceeds the u32 range", what, v))
}

pub(crate) fn put_u32(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_le_bytes());
}

pub(crate) fn put_u32s(buf: &mut Vec<u8>, vs: &[u32]) {
    for &v in vs {
        put_u32(buf, v);
    }
}

fn diag_from_u8(v: u8) -> Option<CfgDiagnosticKind> {
    Some(match v {
        1 => CfgDiagnosticKind::ForHeaderApproximated,
        2 => CfgDiagnosticKind::ForEachHeaderApproximated,
        3 => CfgDiagnosticKind::SwitchRuleKindUnknown,
        4 => CfgDiagnosticKind::UnresolvedJumpTarget,
        5 => CfgDiagnosticKind::AmbiguousLabel,
        6 => CfgDiagnosticKind::YieldOutsideSwitchExpression,
        7 => CfgDiagnosticKind::CyclomaticSaturated,
        _ => return None,
    })
}

/// Bounds-checked little-endian reader over a byte slice (shared by the Phase 4-6
/// artifact readers).
pub(crate) struct Cursor<'b> {
    bytes: &'b [u8],
    pos: usize,
}

impl<'b> Cursor<'b> {
    pub(crate) fn new(bytes: &'b [u8], pos: usize) -> Self {
        Self { bytes, pos }
    }

    pub(crate) fn take(&mut self, n: usize) -> Result<&'b [u8], String> {
        let end = self
            .pos
            .checked_add(n)
            .filter(|&e| e <= self.bytes.len())
            .ok_or_else(|| format!("artifact truncated at offset {} (need {} bytes)", self.pos, n))?;
        let s = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(s)
    }

    pub(crate) fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }

    pub(crate) fn u16(&mut self) -> Result<u16, String> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    pub(crate) fn u32(&mut self) -> Result<u32, String> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub(crate) fn u64(&mut self) -> Result<u64, String> {
        let b = self.take(8)?;
        let mut a = [0u8; 8];
        a.copy_from_slice(b);
        Ok(u64::from_le_bytes(a))
    }

    /// Reads `n` u32 values; rejects counts the remaining bytes cannot hold before
    /// allocating.
    pub(crate) fn u32s(&mut self, n: usize) -> Result<Vec<u32>, String> {
        if n > (self.bytes.len() - self.pos.min(self.bytes.len())) / 4 {
            return Err(format!("artifact array length {} exceeds the remaining bytes", n));
        }
        (0..n).map(|_| self.u32()).collect()
    }

    /// Reads a u32 length prefix followed by that many u32 values.
    pub(crate) fn u32_vec(&mut self) -> Result<Vec<u32>, String> {
        let n = self.u32()? as usize;
        self.u32s(n)
    }
}

impl CFGArtifact {
    pub fn new(sta_hash: u64, bpa_hash: u64) -> Self {
        Self {
            magic: CFA_MAGIC,
            format_version: CFA_FORMAT_VERSION,
            function_count: 0,
            total_blocks: 0,
            total_edges: 0,
            sta_hash,
            bpa_hash,
            functions: Vec::new(),
        }
    }

    pub fn add_function(&mut self, cfg: FunctionCFGData) -> Result<(), String> {
        let blocks = checked_u32(cfg.blocks.len(), "block count")?;
        let edges = checked_u32(cfg.edges.len(), "edge count")?;
        self.total_blocks = self
            .total_blocks
            .checked_add(blocks)
            .ok_or("CFA total block count exceeds the u32 range")?;
        self.total_edges = self
            .total_edges
            .checked_add(edges)
            .ok_or("CFA total edge count exceeds the u32 range")?;
        self.functions.push(cfg);
        self.function_count = checked_u32(self.functions.len(), "function count")?;
        Ok(())
    }

    fn serialize_function(func: &FunctionCFGData, buf: &mut Vec<u8>) -> Result<(), String> {
        let n = func.blocks.len();

        // Sub-header (32 bytes)
        put_u32(buf, func.sym_id);
        put_u32(buf, checked_u32(n, "block count")?);
        put_u32(buf, checked_u32(func.edges.len(), "edge count")?);
        put_u32(buf, 0); // entry block
        put_u32(buf, func.exit_block);
        buf.extend_from_slice(&func.cyclomatic.to_le_bytes());
        buf.push(func.loop_info.max_loop_depth);
        buf.push(0);
        put_u32(buf, func.decl_node);
        put_u32(buf, func.body_node);

        // Succ CSR, Pred CSR (+ edge ids), edge types
        put_u32s(buf, &func.succ_offsets);
        put_u32s(buf, &func.succ_adj);
        put_u32s(buf, &func.pred_offsets);
        put_u32s(buf, &func.pred_adj);
        put_u32s(buf, &func.pred_edge_idx);
        buf.extend_from_slice(&func.edge_types);

        // Dominators idom[], Dominance Frontier DF CSR
        put_u32s(buf, &func.idom);
        put_u32s(buf, &func.df_offsets);
        put_u32s(buf, &func.df_adj);

        // Blocks: token range, flags, statements
        for b in &func.blocks {
            put_u32(buf, b.first_token);
            put_u32(buf, b.last_token);
            let flags = u8::from(b.is_entry) | (u8::from(b.is_exit) << 1);
            buf.extend_from_slice(&[flags, 0, 0, 0]);
            put_u32(buf, checked_u32(b.stmts.len(), "statement count")?);
            put_u32s(buf, &b.stmts);
        }

        // Loop forest
        buf.extend_from_slice(&func.loop_info.loop_depth);
        put_u32(buf, checked_u32(func.loop_info.back_edges.len(), "back edge count")?);
        for &(t, h) in &func.loop_info.back_edges {
            put_u32(buf, t);
            put_u32(buf, h);
        }
        put_u32(buf, checked_u32(func.loop_info.headers.len(), "loop count")?);
        for (i, &h) in func.loop_info.headers.iter().enumerate() {
            put_u32(buf, h);
            put_u32(buf, func.loop_info.loop_parent.get(i).copied().unwrap_or(u32::MAX));
        }
        put_u32(
            buf,
            checked_u32(func.loop_info.irreducible_edges.len(), "irreducible edge count")?,
        );
        for &(a, b) in &func.loop_info.irreducible_edges {
            put_u32(buf, a);
            put_u32(buf, b);
        }

        // Lowered nodes, diagnostics
        put_u32(buf, checked_u32(func.lowered_nodes.len(), "lowered node count")?);
        put_u32s(buf, &func.lowered_nodes);
        put_u32(buf, checked_u32(func.diagnostics.len(), "diagnostic count")?);
        for d in &func.diagnostics {
            buf.extend_from_slice(&[d.kind as u8, 0, 0, 0]);
            put_u32(buf, d.node);
        }
        Ok(())
    }

    pub fn try_serialize(&self) -> Result<Vec<u8>, String> {
        let mut buf = Vec::new();

        // ── 1. HEADER (64 bytes) ──
        buf.extend_from_slice(&self.magic.to_le_bytes());
        buf.extend_from_slice(&self.format_version.to_le_bytes());
        buf.extend_from_slice(&self.function_count.to_le_bytes());
        buf.extend_from_slice(&self.total_blocks.to_le_bytes());
        buf.extend_from_slice(&self.total_edges.to_le_bytes());
        buf.extend_from_slice(&self.sta_hash.to_le_bytes());
        buf.extend_from_slice(&self.bpa_hash.to_le_bytes());
        buf.extend_from_slice(&[0u8; 24]); // _reserved
        debug_assert_eq!(buf.len(), CFA_HEADER_SIZE);

        // ── 2. FUNCTION SECTIONS (offsets measured from the bytes actually written) ──
        let data_start = CFA_HEADER_SIZE + self.functions.len() * DIR_ENTRY_SIZE;
        let mut data = Vec::new();
        let mut dir = Vec::with_capacity(self.functions.len());
        for func in &self.functions {
            let offset = checked_u32(data_start + data.len(), "function offset")?;
            Self::serialize_function(func, &mut data)?;
            dir.push((
                func.sym_id,
                offset,
                checked_u32(func.blocks.len(), "block count")?,
                checked_u32(func.edges.len(), "edge count")?,
            ));
        }

        // ── 3. FUNCTION DIRECTORY (n_func * 20 bytes) ──
        for (sym_id, offset, blk_cnt, edge_cnt) in dir {
            put_u32(&mut buf, sym_id);
            put_u32(&mut buf, offset);
            put_u32(&mut buf, blk_cnt);
            put_u32(&mut buf, edge_cnt);
            buf.extend_from_slice(&[0u8; 4]); // alignment pad
        }
        buf.extend_from_slice(&data);

        // ── 4. CHECKSUM (8 bytes) ──
        let checksum = crc64_ecma(&buf);
        buf.extend_from_slice(&checksum.to_le_bytes());

        Ok(buf)
    }

    /// Serializes the artifact. Panics only if a count exceeds the u32 range, which
    /// `add_function` already rejects; use [`CFGArtifact::try_serialize`] to get an error.
    pub fn serialize(&self) -> Vec<u8> {
        match self.try_serialize() {
            Ok(b) => b,
            Err(e) => panic!("{}", e),
        }
    }

    fn deserialize_function(c: &mut Cursor<'_>) -> Result<FunctionCFGData, String> {
        let sym_id = c.u32()?;
        let n = c.u32()? as usize;
        let e = c.u32()? as usize;
        let _entry = c.u32()?;
        let exit_block = c.u32()?;
        let cyclomatic = c.u16()?;
        let max_loop_depth = c.u8()?;
        let _pad = c.u8()?;
        let decl_node = c.u32()?;
        let body_node = c.u32()?;

        let succ_offsets = c.u32s(n + 1)?;
        let succ_adj = c.u32s(e)?;
        let pred_offsets = c.u32s(n + 1)?;
        let pred_adj = c.u32s(e)?;
        let pred_edge_idx = c.u32s(e)?;
        let edge_types = c.take(e)?.to_vec();
        let idom = c.u32s(n)?;
        let df_offsets = c.u32s(n + 1)?;
        let df_len = df_offsets.last().copied().unwrap_or(0) as usize;
        let df_adj = c.u32s(df_len)?;

        let mut blocks = Vec::with_capacity(n);
        for id in 0..n {
            let first_token = c.u32()?;
            let last_token = c.u32()?;
            let flags = c.take(4)?[0];
            let stmt_count = c.u32()? as usize;
            let stmts = c.u32s(stmt_count)?;
            blocks.push(BasicBlock {
                id: checked_u32(id, "block id")?,
                stmts,
                first_token,
                last_token,
                is_entry: flags & 1 != 0,
                is_exit: flags & 2 != 0,
            });
        }

        let loop_depth = c.take(n)?.to_vec();
        let back_count = c.u32()? as usize;
        let mut back_edges = Vec::with_capacity(back_count.min(e));
        for _ in 0..back_count {
            back_edges.push((c.u32()?, c.u32()?));
        }
        let loop_count = c.u32()? as usize;
        let mut headers = Vec::with_capacity(loop_count.min(n));
        let mut loop_parent = Vec::with_capacity(loop_count.min(n));
        for _ in 0..loop_count {
            headers.push(c.u32()?);
            loop_parent.push(c.u32()?);
        }
        let irr_count = c.u32()? as usize;
        let mut irreducible_edges = Vec::with_capacity(irr_count.min(e));
        for _ in 0..irr_count {
            irreducible_edges.push((c.u32()?, c.u32()?));
        }

        let lowered_count = c.u32()? as usize;
        let lowered_nodes = c.u32s(lowered_count)?;
        let diag_count = c.u32()? as usize;
        let mut diagnostics = Vec::with_capacity(diag_count.min(n + e + 1));
        for _ in 0..diag_count {
            let kind = c.take(4)?[0];
            let node = c.u32()?;
            let kind = diag_from_u8(kind)
                .ok_or_else(|| format!("CFA unknown diagnostic kind {}", kind))?;
            diagnostics.push(CfgDiagnostic { kind, node });
        }

        // Edges in canonical (CSR) order.
        let mut edges = Vec::with_capacity(e);
        for b in 0..n {
            let (s, t) = (succ_offsets[b] as usize, succ_offsets[b + 1] as usize);
            if s > t || t > e {
                return Err(format!("CFA corrupt successor offsets in function sym {}", sym_id));
            }
            for k in s..t {
                edges.push((checked_u32(b, "block id")?, succ_adj[k], CFGEdgeType::from(edge_types[k])));
            }
        }
        if edges.len() != e {
            return Err(format!("CFA edge count mismatch in function sym {}", sym_id));
        }

        Ok(FunctionCFGData {
            sym_id,
            decl_node,
            body_node,
            blocks,
            edges,
            succ_offsets,
            succ_adj,
            pred_offsets,
            pred_adj,
            pred_edge_idx,
            edge_types,
            idom,
            df_offsets,
            df_adj,
            loop_info: LoopInfo {
                back_edges,
                loop_depth,
                max_loop_depth,
                headers,
                loop_parent,
                irreducible_edges,
            },
            cyclomatic,
            exit_block,
            lowered_nodes,
            diagnostics,
        })
    }

    pub fn deserialize(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() < CFA_HEADER_SIZE + 8 {
            return Err("CFA file too small".into());
        }

        let payload_len = bytes.len() - 8;
        let mut crc = [0u8; 8];
        crc.copy_from_slice(&bytes[payload_len..]);
        let expected_crc = u64::from_le_bytes(crc);
        let computed_crc = crc64_ecma(&bytes[..payload_len]);

        if expected_crc != computed_crc {
            return Err(format!(
                "CFA CRC mismatch: computed 0x{:016X}, expected 0x{:016X}",
                computed_crc, expected_crc
            ));
        }

        let payload = &bytes[..payload_len];
        let mut c = Cursor::new(payload, 0);
        let magic = c.u64()?;
        if magic != CFA_MAGIC {
            return Err(format!("Invalid CFA magic: 0x{:016X}", magic));
        }
        let format_version = c.u32()?;
        if format_version != CFA_FORMAT_VERSION {
            return Err(format!(
                "Unsupported CFA format version {} (expected {})",
                format_version, CFA_FORMAT_VERSION
            ));
        }
        let function_count = c.u32()?;
        let total_blocks = c.u32()?;
        let total_edges = c.u32()?;
        let sta_hash = c.u64()?;
        let bpa_hash = c.u64()?;

        let mut dir = Vec::with_capacity((function_count as usize).min(payload.len() / DIR_ENTRY_SIZE));
        let mut dc = Cursor::new(payload, CFA_HEADER_SIZE);
        for _ in 0..function_count {
            let sym_id = dc.u32()?;
            let offset = dc.u32()?;
            let _blocks = dc.u32()?;
            let _edges = dc.u32()?;
            let _pad = dc.u32()?;
            dir.push((sym_id, offset as usize));
        }

        let mut functions = Vec::with_capacity(dir.len());
        for (sym_id, offset) in dir {
            let mut fc = Cursor::new(payload, offset);
            let func = Self::deserialize_function(&mut fc)?;
            if func.sym_id != sym_id {
                return Err(format!(
                    "CFA directory entry for sym {} points at function sym {}",
                    sym_id, func.sym_id
                ));
            }
            functions.push(func);
        }

        Ok(Self {
            magic,
            format_version,
            function_count,
            total_blocks,
            total_edges,
            sta_hash,
            bpa_hash,
            functions,
        })
    }
}

pub struct CFGSerializer;

impl CFGSerializer {
    pub fn write(artifact: &CFGArtifact, out_path: &Path) -> std::io::Result<()> {
        let bytes = artifact
            .try_serialize()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let mut file = BufWriter::new(File::create(out_path)?);
        file.write_all(&bytes)?;
        file.flush()?;
        Ok(())
    }
}
