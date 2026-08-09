//! Core Types for Phase 6 Inter-procedural Call Graph & Points-To Analysis.
//! Authored solely by Ahmad Hassan (B-Ted).

use std::fmt;

/// Call Edge Types (Σ_CG) (§6.2.1)
pub const CG_EDGE_DIRECT: u8 = 0x00;
pub const CG_EDGE_SPECIAL: u8 = 0x01;
pub const CG_EDGE_VIRTUAL: u8 = 0x02;
pub const CG_EDGE_INTERFACE: u8 = 0x03;
pub const CG_EDGE_CONSTRUCTOR: u8 = 0x04;
pub const CG_EDGE_DYNAMIC: u8 = 0x05;
pub const CG_EDGE_REFLECTION: u8 = 0x06;

/// Call Site Flags (§6.4)
pub const CALL_SITE_FLAG_IS_POLYMORPHIC: u8 = 1 << 0;
pub const CALL_SITE_FLAG_HAS_NULL_RECEIVER: u8 = 1 << 1;
pub const CALL_SITE_FLAG_IS_TAIL_CALL: u8 = 1 << 2;

/// SCC Classification Class (§6.4)
pub const SCC_CLASS_NON_RECURSIVE: u8 = 0;
pub const SCC_CLASS_SELF_RECURSIVE: u8 = 1;
pub const SCC_CLASS_MUTUAL_RECURSIVE: u8 = 2;

/// Fixed-size 28-byte CallSite (§6.4)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CallSite {
    pub call_site_id: u32,
    pub caller_sym: u32,
    pub call_node: u32,
    pub receiver_ssa: u32,
    pub call_block: u32,
    pub call_token: u32,
    pub call_type: u8,
    pub flags: u8,
    pub arg_count: u16,
}

impl CallSite {
    pub fn new(
        call_site_id: u32,
        caller_sym: u32,
        call_node: u32,
        receiver_ssa: u32,
        call_block: u32,
        call_token: u32,
        call_type: u8,
        flags: u8,
        arg_count: u16,
    ) -> Self {
        Self {
            call_site_id,
            caller_sym,
            call_node,
            receiver_ssa,
            call_block,
            call_token,
            call_type,
            flags,
            arg_count,
        }
    }

    pub fn is_polymorphic(&self) -> bool {
        (self.flags & CALL_SITE_FLAG_IS_POLYMORPHIC) != 0
    }
}

/// Fixed-size 16-byte CallEdge (§6.4)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CallEdge {
    pub callee_sym: u32,
    pub call_site_id: u32,
    pub edge_type: u8,
    pub _padding: [u8; 7],
}

impl CallEdge {
    pub fn new(callee_sym: u32, call_site_id: u32, edge_type: u8) -> Self {
        Self {
            callee_sym,
            call_site_id,
            edge_type,
            _padding: [0; 7],
        }
    }
}

/// Fixed-size 12-byte SCCRecord (§6.4)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SCCRecord {
    pub scc_id: u32,
    pub member_offset: u32,
    pub member_count: u16,
    pub scc_class: u8,
    pub _padding: u8,
}

impl SCCRecord {
    pub fn new(scc_id: u32, member_offset: u32, member_count: u16, scc_class: u8) -> Self {
        Self {
            scc_id,
            member_offset,
            member_count,
            scc_class,
            _padding: 0,
        }
    }
}

/// Points-To Entry from Anderson analysis (§6.2.5)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PointsToEntry {
    pub ssa_id: u32,
    pub alloc_type_sym_id: u32,
}

/// CSR representation for Call Graph edges
#[derive(Debug, Clone, Default)]
pub struct CGCSR {
    pub offsets: Vec<u32>,
    pub adj: Vec<u32>,
    pub edge_types: Vec<u8>,
}

impl CGCSR {
    pub fn edges_of(&self, method_id: u32) -> (&[u32], &[u8]) {
        let idx = method_id as usize;
        if idx + 1 < self.offsets.len() {
            let start = self.offsets[idx] as usize;
            let end = self.offsets[idx + 1] as usize;
            if start <= end && end <= self.adj.len() {
                return (&self.adj[start..end], &self.edge_types[start..end]);
            }
        }
        (&[], &[])
    }
}

/// In-memory CallGraphArtifact (.cga) Persisted Representation (§6.6)
#[derive(Debug, Clone)]
pub struct CallGraphArtifact {
    pub format_version: u32,
    pub method_count: u32,
    pub call_site_count: u32,
    pub call_edge_count: u32,
    pub ssa_hash: u64,
    pub sta_hash: u64,

    pub call_sites: Vec<CallSite>,
    pub callee_csr: CGCSR,
    pub caller_csr: CGCSR,
    pub site_to_edge_map: Vec<(u32, u32, u32)>, // (caller_sym, callee_sym, call_site_id)
    pub points_to_table: Vec<PointsToEntry>,
    pub sccs: Vec<SCCRecord>,
    pub scc_members: Vec<u32>,
}

impl fmt::Display for CallSite {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "CallSite #{} [caller={} node={} block={} type={:#04x} args={}]",
            self.call_site_id,
            self.caller_sym,
            self.call_node,
            self.call_block,
            self.call_type,
            self.arg_count
        )
    }
}
