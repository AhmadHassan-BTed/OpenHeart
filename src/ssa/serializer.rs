//! Binary Serializer & Deserializer for SSAArtifact (.ssa format) (§5.6).
//! Authored by Ahmad Hassan (B-Ted).
//!
//! Format version 2. Directory offsets are measured from the bytes actually written,
//! `def_block` and φ `orig_sym_id` are stored as full u32 values, and
//! [`SSASerializer::read`] reconstructs every `FunctionSSAData` field.
//!
//! Layout (little endian):
//! * header (64 B): magic "SSA1", version u32, cfa_hash u64, function_count u32,
//!   total_ssa_vars u32, total_phi_funcs u32, reserved[36]
//! * directory (function_count × 20 B): sym_id, offset, ssa_count, phi_count, cd_edges
//! * per function: sym_id, var_count, unresolved_names; SSA records (20 B each:
//!   ssa_id, orig_sym_id, def_stmt, version u16, flags u8, pad u8, def_block u32);
//!   value nodes; φ records (ssa_id, block_id, orig_sym_id, argc, args); def-use CSR
//!   with use blocks; operand CSR; CDG CSR; data-flow facts
//! * CRC-64 of everything before it (8 B)

use crate::cfg::serializer::{put_u32, put_u32s, Cursor};
use crate::core::io::mmap::MemoryMappedFile;
use crate::core::types::ssa::{DefUseCSR, IFDSResults, PhiArg, PhiRecord, SSARecord, CDGCSR};
use crate::ingestion::serializer::crc64_ecma;
use std::fs;
use std::path::Path;

pub const SSA_MAGIC: [u8; 4] = *b"SSA1";
pub const SSA_VERSION: u32 = 2;
pub const SSA_HEADER_SIZE: usize = 64;
const DIR_ENTRY_SIZE: usize = 20;

/// Operand (use-def) CSR: for SSA id `d`, `adj[offsets[d]..offsets[d+1]]` are the SSA
/// ids read to compute `d` (the arguments for a φ).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OperandCSR {
    pub offsets: Vec<u32>,
    pub adj: Vec<u32>,
}

impl OperandCSR {
    pub fn operands_of(&self, ssa_id: u32) -> &[u32] {
        let i = ssa_id as usize;
        if i + 1 < self.offsets.len() {
            let (s, e) = (self.offsets[i] as usize, self.offsets[i + 1] as usize);
            if s <= e && e <= self.adj.len() {
                return &self.adj[s..e];
            }
        }
        &[]
    }
}

#[derive(Debug, Clone)]
pub struct FunctionSSAData {
    pub sym_id: u32,
    /// SSA ids are local to the function (0-based); use
    /// [`SSAArtifact::global_ssa_base`] to number them program-wide.
    pub ssa_records: Vec<SSARecord>,
    /// Untruncated defining block per SSA id (`SSARecord::def_block` keeps 8 bits).
    pub def_blocks: Vec<u32>,
    pub phi_records: Vec<PhiRecord>,
    /// Def-use chains: using statements per SSA id (`u32::MAX` for a φ argument).
    pub def_use: DefUseCSR,
    /// Block of each `def_use.use_adj` entry (the predecessor block for φ arguments).
    pub use_blocks: Vec<u32>,
    /// Value expression assigned by each definition (`u32::MAX` when none).
    pub value_nodes: Vec<u32>,
    pub operands: OperandCSR,
    pub cdg: CDGCSR,
    pub ifds: IFDSResults,
    /// Number of distinct variables (locals, parameters, captured) of the function.
    pub var_count: u32,
    /// Identifier occurrences that did not resolve to a local variable (fields, types,
    /// statics; all uses when token text was unavailable).
    pub unresolved_names: u32,
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

fn checked_u32(v: usize, what: &str) -> Result<u32, String> {
    u32::try_from(v).map_err(|_| format!("SSA {} {} exceeds the u32 range", what, v))
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

    pub fn add_function(&mut self, func: FunctionSSAData) -> Result<(), String> {
        self.total_ssa_vars = self
            .total_ssa_vars
            .checked_add(checked_u32(func.ssa_records.len(), "SSA count")?)
            .ok_or("SSA total variable count exceeds the u32 range")?;
        self.total_phi_funcs = self
            .total_phi_funcs
            .checked_add(checked_u32(func.phi_records.len(), "φ count")?)
            .ok_or("SSA total φ count exceeds the u32 range")?;
        self.functions.push(func);
        self.function_count = checked_u32(self.functions.len(), "function count")?;
        Ok(())
    }

    /// Program-wide SSA numbering: the global id of local SSA id `i` of function `f` is
    /// `global_ssa_base()[f] + i`.
    pub fn global_ssa_base(&self) -> Vec<u32> {
        let mut base = Vec::with_capacity(self.functions.len());
        let mut acc = 0u32;
        for f in &self.functions {
            base.push(acc);
            acc = acc.saturating_add(f.ssa_records.len() as u32);
        }
        base
    }

    fn write_function(func: &FunctionSSAData, buf: &mut Vec<u8>) -> Result<(), String> {
        put_u32(buf, func.sym_id);
        put_u32(buf, func.var_count);
        put_u32(buf, func.unresolved_names);

        let n = func.ssa_records.len();
        put_u32(buf, checked_u32(n, "SSA count")?);
        for (i, ssa) in func.ssa_records.iter().enumerate() {
            put_u32(buf, ssa.ssa_id);
            put_u32(buf, ssa.orig_sym_id);
            put_u32(buf, ssa.def_stmt);
            buf.extend_from_slice(&ssa.version.to_le_bytes());
            buf.push(ssa.flags);
            buf.push(0);
            put_u32(
                buf,
                func.def_blocks
                    .get(i)
                    .copied()
                    .unwrap_or(ssa.def_block as u32),
            );
        }
        for i in 0..n {
            put_u32(buf, func.value_nodes.get(i).copied().unwrap_or(u32::MAX));
        }

        put_u32(buf, checked_u32(func.phi_records.len(), "φ count")?);
        for phi in &func.phi_records {
            put_u32(buf, phi.ssa_id);
            put_u32(buf, phi.block_id);
            put_u32(buf, phi.orig_sym_id);
            put_u32(buf, checked_u32(phi.args.len(), "φ arity")?);
            for arg in &phi.args {
                put_u32(buf, arg.pred_block_id);
                put_u32(buf, arg.arg_ssa_id);
            }
        }

        put_u32(
            buf,
            checked_u32(func.def_use.def_offsets.len(), "def-use offsets")?,
        );
        put_u32s(buf, &func.def_use.def_offsets);
        put_u32(
            buf,
            checked_u32(func.def_use.use_adj.len(), "def-use size")?,
        );
        put_u32s(buf, &func.def_use.use_adj);
        put_u32(buf, checked_u32(func.use_blocks.len(), "use blocks")?);
        put_u32s(buf, &func.use_blocks);

        put_u32(
            buf,
            checked_u32(func.operands.offsets.len(), "operand offsets")?,
        );
        put_u32s(buf, &func.operands.offsets);
        put_u32(buf, checked_u32(func.operands.adj.len(), "operand size")?);
        put_u32s(buf, &func.operands.adj);

        put_u32(buf, checked_u32(func.cdg.cd_offsets.len(), "CDG offsets")?);
        put_u32s(buf, &func.cdg.cd_offsets);
        put_u32(buf, checked_u32(func.cdg.cd_adj.len(), "CDG size")?);
        put_u32s(buf, &func.cdg.cd_adj);
        if func.cdg.cd_types.len() != func.cdg.cd_adj.len() {
            return Err(format!(
                "SSA function sym {} has {} CDG edges but {} edge labels",
                func.sym_id,
                func.cdg.cd_adj.len(),
                func.cdg.cd_types.len()
            ));
        }
        buf.extend_from_slice(&func.cdg.cd_types);

        put_u32(
            buf,
            checked_u32(func.ifds.taint_sparse.len(), "taint facts")?,
        );
        for &(ssa, src) in &func.ifds.taint_sparse {
            put_u32(buf, ssa);
            buf.extend_from_slice(&src.to_le_bytes());
        }
        put_u32(
            buf,
            checked_u32(func.ifds.nullable_sparse.len(), "nullness facts")?,
        );
        put_u32s(buf, &func.ifds.nullable_sparse);
        put_u32(
            buf,
            checked_u32(func.ifds.type_state_sparse.len(), "type-state facts")?,
        );
        for &(ssa, st) in &func.ifds.type_state_sparse {
            put_u32(buf, ssa);
            buf.extend_from_slice(&st.to_le_bytes());
        }
        Ok(())
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, String> {
        let mut buf = Vec::new();

        // 1. Header (64 bytes)
        buf.extend_from_slice(&SSA_MAGIC);
        buf.extend_from_slice(&self.format_version.to_le_bytes());
        buf.extend_from_slice(&self.cfa_hash.to_le_bytes());
        buf.extend_from_slice(&self.function_count.to_le_bytes());
        buf.extend_from_slice(&self.total_ssa_vars.to_le_bytes());
        buf.extend_from_slice(&self.total_phi_funcs.to_le_bytes());
        buf.extend_from_slice(&[0u8; 36]); // Reserved
        debug_assert_eq!(buf.len(), SSA_HEADER_SIZE);

        // 2. Per-function data, with offsets taken from the bytes actually written
        let data_start = SSA_HEADER_SIZE + self.functions.len() * DIR_ENTRY_SIZE;
        let mut data = Vec::new();
        let mut dir = Vec::with_capacity(self.functions.len());
        for func in &self.functions {
            let offset = checked_u32(data_start + data.len(), "function offset")?;
            Self::write_function(func, &mut data)?;
            dir.push((
                func.sym_id,
                offset,
                checked_u32(func.ssa_records.len(), "SSA count")?,
                checked_u32(func.phi_records.len(), "φ count")?,
                checked_u32(func.cdg.cd_adj.len(), "CDG size")?,
            ));
        }

        // 3. Function Directory Table (20 bytes per function)
        for (sym_id, offset, n_ssa, n_phi, n_cd) in dir {
            put_u32(&mut buf, sym_id);
            put_u32(&mut buf, offset);
            put_u32(&mut buf, n_ssa);
            put_u32(&mut buf, n_phi);
            put_u32(&mut buf, n_cd);
        }
        buf.extend_from_slice(&data);

        let checksum = crc64_ecma(&buf);
        buf.extend_from_slice(&checksum.to_le_bytes());
        Ok(buf)
    }

    fn read_function(c: &mut Cursor<'_>) -> Result<FunctionSSAData, String> {
        let sym_id = c.u32()?;
        let var_count = c.u32()?;
        let unresolved_names = c.u32()?;

        let n = c.u32()? as usize;
        let mut ssa_records = Vec::with_capacity(n.min(1 << 20));
        let mut def_blocks = Vec::with_capacity(n.min(1 << 20));
        for _ in 0..n {
            let ssa_id = c.u32()?;
            let orig = c.u32()?;
            let def_stmt = c.u32()?;
            let version = c.u16()?;
            let flags = c.u8()?;
            let _pad = c.u8()?;
            let def_block = c.u32()?;
            ssa_records.push(SSARecord::new(
                ssa_id, orig, def_stmt, version, flags, def_block,
            ));
            def_blocks.push(def_block);
        }
        let value_nodes = c.u32s(n)?;

        let n_phi = c.u32()? as usize;
        let mut phi_records = Vec::with_capacity(n_phi.min(1 << 20));
        for _ in 0..n_phi {
            let ssa_id = c.u32()?;
            let block_id = c.u32()?;
            let orig = c.u32()?;
            let argc = c.u32()? as usize;
            let mut args = Vec::with_capacity(argc.min(1 << 16));
            for _ in 0..argc {
                args.push(PhiArg {
                    pred_block_id: c.u32()?,
                    arg_ssa_id: c.u32()?,
                });
            }
            phi_records.push(PhiRecord::new(ssa_id, block_id, orig, args));
        }

        let def_offsets = c.u32_vec()?;
        let use_adj = c.u32_vec()?;
        let use_blocks = c.u32_vec()?;
        let op_offsets = c.u32_vec()?;
        let op_adj = c.u32_vec()?;
        let cd_offsets = c.u32_vec()?;
        let cd_adj = c.u32_vec()?;
        let cd_types = c.take(cd_adj.len())?.to_vec();

        let n_taint = c.u32()? as usize;
        let mut taint_sparse = Vec::with_capacity(n_taint.min(1 << 20));
        for _ in 0..n_taint {
            taint_sparse.push((c.u32()?, c.u16()?));
        }
        let nullable_sparse = c.u32_vec()?;
        let n_ts = c.u32()? as usize;
        let mut type_state_sparse = Vec::with_capacity(n_ts.min(1 << 20));
        for _ in 0..n_ts {
            type_state_sparse.push((c.u32()?, c.u16()?));
        }

        Ok(FunctionSSAData {
            sym_id,
            ssa_records,
            def_blocks,
            phi_records,
            def_use: DefUseCSR {
                def_offsets,
                use_adj,
            },
            use_blocks,
            value_nodes,
            operands: OperandCSR {
                offsets: op_offsets,
                adj: op_adj,
            },
            cdg: CDGCSR {
                cd_offsets,
                cd_adj,
                cd_types,
            },
            ifds: IFDSResults {
                taint_sparse,
                nullable_sparse,
                type_state_sparse,
            },
            var_count,
            unresolved_names,
        })
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() < SSA_HEADER_SIZE + 8 {
            return Err("File too small for SSA header + checksum".to_string());
        }
        if bytes[0..4] != SSA_MAGIC {
            return Err("Invalid SSA magic header".to_string());
        }

        let payload_len = bytes.len() - 8;
        let mut crc = [0u8; 8];
        crc.copy_from_slice(&bytes[payload_len..]);
        let file_crc = u64::from_le_bytes(crc);
        let computed_crc = crc64_ecma(&bytes[..payload_len]);
        if file_crc != computed_crc {
            return Err(format!(
                "CRC-64 Checksum mismatch in SSA artifact: expected 0x{:016X}, got 0x{:016X}",
                file_crc, computed_crc
            ));
        }

        let payload = &bytes[..payload_len];
        let mut c = Cursor::new(payload, 4);
        let format_version = c.u32()?;
        if format_version != SSA_VERSION {
            return Err(format!(
                "Unsupported SSA format version {} (expected {})",
                format_version, SSA_VERSION
            ));
        }
        let cfa_hash = c.u64()?;
        let function_count = c.u32()?;
        let total_ssa_vars = c.u32()?;
        let total_phi_funcs = c.u32()?;

        let mut dc = Cursor::new(payload, SSA_HEADER_SIZE);
        let mut dir =
            Vec::with_capacity((function_count as usize).min(payload.len() / DIR_ENTRY_SIZE));
        for _ in 0..function_count {
            let sym_id = dc.u32()?;
            let offset = dc.u32()? as usize;
            let _n_ssa = dc.u32()?;
            let _n_phi = dc.u32()?;
            let _n_cd = dc.u32()?;
            dir.push((sym_id, offset));
        }

        let mut functions = Vec::with_capacity(dir.len());
        for (sym_id, offset) in dir {
            let mut fc = Cursor::new(payload, offset);
            let func = Self::read_function(&mut fc)?;
            if func.sym_id != sym_id {
                return Err(format!(
                    "SSA directory entry for sym {} points at function sym {}",
                    sym_id, func.sym_id
                ));
            }
            functions.push(func);
        }

        Ok(SSAArtifact {
            format_version,
            cfa_hash,
            function_count,
            total_ssa_vars,
            total_phi_funcs,
            functions,
        })
    }
}

pub struct SSASerializer;

impl SSASerializer {
    pub fn write(artifact: &SSAArtifact, out_path: &Path) -> Result<(), String> {
        let buffer = artifact.to_bytes()?;
        fs::write(out_path, &buffer).map_err(|e| format!("IO write error: {}", e))?;
        Ok(())
    }

    pub fn read(path: &Path) -> Result<SSAArtifact, String> {
        let mmap = MemoryMappedFile::open(path).map_err(|e| format!("mmap failed: {}", e))?;
        SSAArtifact::from_bytes(mmap.as_slice())
    }
}
