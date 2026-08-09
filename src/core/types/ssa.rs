//! Core Types for Phase 5 SSA Conversion and Data Flow Graph.
//! Authored by Ahmad Hassan (B-Ted).

use std::fmt;

/// Flags for SSARecord (1 byte bitfields)
pub const SSA_FLAG_IS_PHI: u8 = 1 << 0;
pub const SSA_FLAG_IS_PARAM_DEF: u8 = 1 << 1;
pub const SSA_FLAG_IS_FIELD_DEF: u8 = 1 << 2;
pub const SSA_FLAG_IS_RETURN_VAL: u8 = 1 << 3;
pub const SSA_FLAG_IS_CONST: u8 = 1 << 4;

/// CDG Edge Types
pub const CD_EDGE_TRUE: u8 = 0x01;
pub const CD_EDGE_FALSE: u8 = 0x02;

/// 16-Byte Cache-Aligned SSARecord (§5.2.4)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SSARecord {
    pub ssa_id: u32,
    pub orig_sym_id: u32,
    pub def_stmt: u32,
    pub version: u16,
    pub flags: u8,
    pub def_block: u8,
}

impl SSARecord {
    pub fn new(
        ssa_id: u32,
        orig_sym_id: u32,
        def_stmt: u32,
        version: u16,
        flags: u8,
        def_block: u32,
    ) -> Self {
        Self {
            ssa_id,
            orig_sym_id,
            def_stmt,
            version,
            flags,
            def_block: (def_block & 0xFF) as u8,
        }
    }

    pub fn is_phi(&self) -> bool {
        (self.flags & SSA_FLAG_IS_PHI) != 0
    }
}

/// PhiArg (8 bytes)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhiArg {
    pub pred_block_id: u32,
    pub arg_ssa_id: u32,
}

/// Variable-length PhiRecord (§5.4)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhiRecord {
    pub ssa_id: u32,
    pub block_id: u32,
    pub orig_sym_id: u32,
    pub args: Vec<PhiArg>,
}

impl PhiRecord {
    pub fn new(ssa_id: u32, block_id: u32, orig_sym_id: u32, args: Vec<PhiArg>) -> Self {
        Self {
            ssa_id,
            block_id,
            orig_sym_id,
            args,
        }
    }
}

/// Def-Use CSR representation per function
#[derive(Debug, Clone, Default)]
pub struct DefUseCSR {
    pub def_offsets: Vec<u32>,
    pub use_adj: Vec<u32>,
}

impl DefUseCSR {
    pub fn uses_of(&self, ssa_id: u32) -> &[u32] {
        let idx = ssa_id as usize;
        if idx + 1 < self.def_offsets.len() {
            let start = self.def_offsets[idx] as usize;
            let end = self.def_offsets[idx + 1] as usize;
            if start <= end && end <= self.use_adj.len() {
                return &self.use_adj[start..end];
            }
        }
        &[]
    }
}

/// CDG CSR representation per function
#[derive(Debug, Clone, Default)]
pub struct CDGCSR {
    pub cd_offsets: Vec<u32>,
    pub cd_adj: Vec<u32>,
    pub cd_types: Vec<u8>,
}

/// Sparse IFDS Analysis Results
#[derive(Debug, Clone, Default)]
pub struct IFDSResults {
    pub taint_sparse: Vec<(u32, u16)>,
    pub nullable_sparse: Vec<u32>,
    pub type_state_sparse: Vec<(u32, u16)>,
}

impl fmt::Display for SSARecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "v{}_{} (ssa_id={}) def_stmt={} block={}",
            self.orig_sym_id, self.version, self.ssa_id, self.def_stmt, self.def_block
        )
    }
}
