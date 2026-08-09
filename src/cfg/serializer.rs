//! CFGArtifact binary serializer and deserializer for `.cfa` files (§4.6).

use crate::cfg::builder::FunctionCFGData;
use crate::ingestion::serializer::crc64_ecma;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

pub const CFA_MAGIC: u64 = 0x43464130_00010000; // "CFA\x00\x00\x01\x00\x00"
pub const CFA_HEADER_SIZE: usize = 64;

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

impl CFGArtifact {
    pub fn new(sta_hash: u64, bpa_hash: u64) -> Self {
        Self {
            magic: CFA_MAGIC,
            format_version: 1,
            function_count: 0,
            total_blocks: 0,
            total_edges: 0,
            sta_hash,
            bpa_hash,
            functions: Vec::new(),
        }
    }

    pub fn add_function(&mut self, cfg: FunctionCFGData) {
        self.total_blocks += cfg.blocks.len() as u32;
        self.total_edges += cfg.edges.len() as u32;
        self.functions.push(cfg);
        self.function_count = self.functions.len() as u32;
    }

    pub fn serialize(&self) -> Vec<u8> {
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

        // ── 2. FUNCTION DIRECTORY (n_func * 20 bytes) ──
        let dir_offset = CFA_HEADER_SIZE;
        let func_data_start = dir_offset + (self.function_count as usize * 20);

        let mut curr_offset = func_data_start as u32;
        let mut dir_entries = Vec::new();

        for func in &self.functions {
            let func_len = 32
                + (func.succ_offsets.len() * 4)
                + (func.succ_adj.len() * 4)
                + (func.pred_offsets.len() * 4)
                + (func.pred_adj.len() * 4)
                + func.edge_types.len()
                + (func.idom.len() * 4)
                + (func.df_offsets.len() * 4)
                + (func.df_adj.len() * 4);

            dir_entries.push((
                func.sym_id,
                curr_offset,
                func.blocks.len() as u32,
                func.edges.len() as u32,
            ));
            curr_offset += func_len as u32;
        }

        for (sym_id, offset, blk_cnt, edge_cnt) in dir_entries {
            buf.extend_from_slice(&sym_id.to_le_bytes());
            buf.extend_from_slice(&offset.to_le_bytes());
            buf.extend_from_slice(&blk_cnt.to_le_bytes());
            buf.extend_from_slice(&edge_cnt.to_le_bytes());
            buf.extend_from_slice(&[0u8; 4]); // alignment pad
        }

        // ── 3. FUNCTION DATA SUBSECTIONS ──
        for func in &self.functions {
            // Function Sub-header (32 bytes)
            buf.extend_from_slice(&func.sym_id.to_le_bytes());
            buf.extend_from_slice(&(func.blocks.len() as u32).to_le_bytes());
            buf.extend_from_slice(&(func.edges.len() as u32).to_le_bytes());
            buf.extend_from_slice(&0u32.to_le_bytes()); // entry block
            buf.extend_from_slice(&((func.blocks.len().saturating_sub(1)) as u32).to_le_bytes()); // exit block
            buf.extend_from_slice(&func.cyclomatic.to_le_bytes());
            buf.extend_from_slice(&[func.loop_info.max_loop_depth, 0u8]); // max_loop_depth, _pad
            buf.extend_from_slice(&[0u8; 8]); // reserved

            // Succ CSR
            for &off in &func.succ_offsets {
                buf.extend_from_slice(&off.to_le_bytes());
            }
            for &adj in &func.succ_adj {
                buf.extend_from_slice(&adj.to_le_bytes());
            }

            // Pred CSR
            for &off in &func.pred_offsets {
                buf.extend_from_slice(&off.to_le_bytes());
            }
            for &adj in &func.pred_adj {
                buf.extend_from_slice(&adj.to_le_bytes());
            }

            // Edge Types
            buf.extend_from_slice(&func.edge_types);

            // Dominators idom[]
            for &idom in &func.idom {
                buf.extend_from_slice(&idom.to_le_bytes());
            }

            // Dominance Frontier DF CSR
            for &off in &func.df_offsets {
                buf.extend_from_slice(&off.to_le_bytes());
            }
            for &adj in &func.df_adj {
                buf.extend_from_slice(&adj.to_le_bytes());
            }
        }

        // ── 4. CHECKSUM (8 bytes) ──
        let checksum = crc64_ecma(&buf);
        buf.extend_from_slice(&checksum.to_le_bytes());

        buf
    }

    pub fn deserialize(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() < CFA_HEADER_SIZE + 8 {
            return Err("CFA file too small".into());
        }

        let payload_len = bytes.len() - 8;
        let expected_crc = u64::from_le_bytes(bytes[payload_len..].try_into().unwrap());
        let computed_crc = crc64_ecma(&bytes[..payload_len]);

        if expected_crc != computed_crc {
            return Err(format!(
                "CFA CRC mismatch: computed 0x{:016X}, expected 0x{:016X}",
                computed_crc, expected_crc
            ));
        }

        let magic = u64::from_le_bytes(bytes[0..8].try_into().unwrap());
        if magic != CFA_MAGIC {
            return Err(format!("Invalid CFA magic: 0x{:016X}", magic));
        }

        let format_version = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
        let function_count = u32::from_le_bytes(bytes[12..16].try_into().unwrap());
        let total_blocks = u32::from_le_bytes(bytes[16..20].try_into().unwrap());
        let total_edges = u32::from_le_bytes(bytes[20..24].try_into().unwrap());
        let sta_hash = u64::from_le_bytes(bytes[24..32].try_into().unwrap());
        let bpa_hash = u64::from_le_bytes(bytes[32..40].try_into().unwrap());

        Ok(Self {
            magic,
            format_version,
            function_count,
            total_blocks,
            total_edges,
            sta_hash,
            bpa_hash,
            functions: Vec::new(),
        })
    }
}

pub struct CFGSerializer;

impl CFGSerializer {
    pub fn write(artifact: &CFGArtifact, out_path: &Path) -> std::io::Result<()> {
        let bytes = artifact.serialize();
        let mut file = BufWriter::new(File::create(out_path)?);
        file.write_all(&bytes)?;
        file.flush()?;
        Ok(())
    }
}
