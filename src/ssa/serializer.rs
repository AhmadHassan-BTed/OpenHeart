//! Binary Serializer & Deserializer for SSAArtifact (.ssa format) (§5.6).
//! Authored by Ahmad Hassan (B-Ted).

use crate::core::io::binary::BinaryWriter;
use crate::core::io::mmap::MemoryMappedFile;
use crate::core::types::ssa::{DefUseCSR, IFDSResults, PhiRecord, SSARecord, CDGCSR};
use crate::ingestion::serializer::crc64_ecma;
use std::fs;
use std::path::Path;

pub const SSA_MAGIC: [u8; 4] = *b"SSA1";
pub const SSA_VERSION: u32 = 1;
pub const SSA_HEADER_SIZE: usize = 64;

#[derive(Debug, Clone)]
pub struct FunctionSSAData {
    pub sym_id: u32,
    pub ssa_records: Vec<SSARecord>,
    pub phi_records: Vec<PhiRecord>,
    pub def_use: DefUseCSR,
    pub cdg: CDGCSR,
    pub ifds: IFDSResults,
}

#[derive(Debug, Clone)]
pub struct SSAArtifact {
    pub format_version: u32,
    pub cfa_hash: u64,
    pub function_count: u32,
    pub total_ssa_vars: u32,
    pub total_phi_funcs: u32,
    pub functions: Vec<FunctionSSAData>,
}

impl SSAArtifact {
    pub fn new(cfa_hash: u64) -> Self {
        Self {
            format_version: SSA_VERSION,
            cfa_hash,
            function_count: 0,
            total_ssa_vars: 0,
            total_phi_funcs: 0,
            functions: Vec::new(),
        }
    }

    pub fn add_function(&mut self, func: FunctionSSAData) {
        self.total_ssa_vars += func.ssa_records.len() as u32;
        self.total_phi_funcs += func.phi_records.len() as u32;
        self.function_count += 1;
        self.functions.push(func);
    }
}

pub struct SSASerializer;

impl SSASerializer {
    pub fn write(artifact: &SSAArtifact, out_path: &Path) -> Result<(), String> {
        let mut buffer = Vec::new();
        {
            let mut bw = BinaryWriter::new(&mut buffer);

            // 1. Header (64 bytes)
            let _ = bw.write_bytes(&SSA_MAGIC);
            let _ = bw.write_u32(artifact.format_version);
            let _ = bw.write_u64(artifact.cfa_hash);
            let _ = bw.write_u32(artifact.function_count);
            let _ = bw.write_u32(artifact.total_ssa_vars);
            let _ = bw.write_u32(artifact.total_phi_funcs);
            let _ = bw.write_bytes(&[0u8; 36]); // Reserved

            // 2. Function Directory Table (20 bytes per function)
            let mut data_offset = SSA_HEADER_SIZE + (artifact.function_count as usize * 20);
            for func in &artifact.functions {
                let _ = bw.write_u32(func.sym_id);
                let _ = bw.write_u32(data_offset as u32);
                let _ = bw.write_u32(func.ssa_records.len() as u32);
                let _ = bw.write_u32(func.phi_records.len() as u32);
                let _ = bw.write_u32(func.cdg.cd_adj.len() as u32);

                let phi_bytes: usize = func.phi_records.iter().map(|p| 12 + p.args.len() * 8).sum();
                let defuse_bytes =
                    (func.def_use.def_offsets.len() * 4) + (func.def_use.use_adj.len() * 4);
                let cdg_bytes = (func.cdg.cd_offsets.len() * 4)
                    + (func.cdg.cd_adj.len() * 4)
                    + func.cdg.cd_types.len();
                let ifds_bytes = (func.ifds.taint_sparse.len() * 6)
                    + (func.ifds.nullable_sparse.len() * 4)
                    + (func.ifds.type_state_sparse.len() * 6);

                data_offset += (func.ssa_records.len() * 16)
                    + phi_bytes
                    + defuse_bytes
                    + cdg_bytes
                    + ifds_bytes;
            }

            // 3. Write Per-Function Data Blocks
            for func in &artifact.functions {
                for ssa in &func.ssa_records {
                    let _ = bw.write_u32(ssa.ssa_id);
                    let _ = bw.write_u32(ssa.orig_sym_id);
                    let _ = bw.write_u32(ssa.def_stmt);
                    let _ = bw.write_u16(ssa.version);
                    let _ = bw.write_u8(ssa.flags);
                    let _ = bw.write_u8(ssa.def_block);
                }

                for phi in &func.phi_records {
                    let _ = bw.write_u32(phi.ssa_id);
                    let _ = bw.write_u32(phi.block_id);
                    let _ = bw.write_u16(phi.args.len() as u16);
                    let _ = bw.write_u16((phi.orig_sym_id & 0xFFFF) as u16);
                    for arg in &phi.args {
                        let _ = bw.write_u32(arg.pred_block_id);
                        let _ = bw.write_u32(arg.arg_ssa_id);
                    }
                }

                let _ = bw.write_u32(func.def_use.def_offsets.len() as u32);
                for &off in &func.def_use.def_offsets {
                    let _ = bw.write_u32(off);
                }
                let _ = bw.write_u32(func.def_use.use_adj.len() as u32);
                for &u in &func.def_use.use_adj {
                    let _ = bw.write_u32(u);
                }

                let _ = bw.write_u32(func.cdg.cd_offsets.len() as u32);
                for &off in &func.cdg.cd_offsets {
                    let _ = bw.write_u32(off);
                }
                let _ = bw.write_u32(func.cdg.cd_adj.len() as u32);
                for &a in &func.cdg.cd_adj {
                    let _ = bw.write_u32(a);
                }
                for &t in &func.cdg.cd_types {
                    let _ = bw.write_u8(t);
                }

                let _ = bw.write_u32(func.ifds.taint_sparse.len() as u32);
                for &(ssa, src) in &func.ifds.taint_sparse {
                    let _ = bw.write_u32(ssa);
                    let _ = bw.write_u16(src);
                }

                let _ = bw.write_u32(func.ifds.nullable_sparse.len() as u32);
                for &ssa in &func.ifds.nullable_sparse {
                    let _ = bw.write_u32(ssa);
                }

                let _ = bw.write_u32(func.ifds.type_state_sparse.len() as u32);
                for &(ssa, st) in &func.ifds.type_state_sparse {
                    let _ = bw.write_u32(ssa);
                    let _ = bw.write_u16(st);
                }
            }
        }

        let checksum = crc64_ecma(&buffer);
        buffer.extend_from_slice(&checksum.to_le_bytes());

        fs::write(out_path, &buffer).map_err(|e| format!("IO write error: {}", e))?;
        Ok(())
    }

    pub fn read(path: &Path) -> Result<SSAArtifact, String> {
        let mmap = MemoryMappedFile::open(path).map_err(|e| format!("mmap failed: {}", e))?;
        let bytes = mmap.as_slice();

        if bytes.len() < SSA_HEADER_SIZE + 8 {
            return Err("File too small for SSA header + checksum".to_string());
        }

        let magic = &bytes[0..4];
        if magic != SSA_MAGIC {
            return Err("Invalid SSA magic header".to_string());
        }

        let payload_len = bytes.len() - 8;
        let file_crc = u64::from_le_bytes(bytes[payload_len..].try_into().unwrap());
        let computed_crc = crc64_ecma(&bytes[..payload_len]);

        if file_crc != computed_crc {
            return Err(format!(
                "CRC-64 Checksum mismatch in SSA artifact: expected 0x{:016X}, got 0x{:016X}",
                file_crc, computed_crc
            ));
        }

        let format_version = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
        let cfa_hash = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
        let function_count = u32::from_le_bytes(bytes[16..20].try_into().unwrap());
        let total_ssa_vars = u32::from_le_bytes(bytes[20..24].try_into().unwrap());
        let total_phi_funcs = u32::from_le_bytes(bytes[24..28].try_into().unwrap());

        Ok(SSAArtifact {
            format_version,
            cfa_hash,
            function_count,
            total_ssa_vars,
            total_phi_funcs,
            functions: Vec::new(),
        })
    }
}
