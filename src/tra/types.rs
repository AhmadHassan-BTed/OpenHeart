//! Data structure specifications for Phase 7: Traceability Index Construction (§7.4).

pub const TRA_MAGIC: u64 = 0x5452413100010000; // "TRA1\x00\x01\x00\x00"

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LayerKind {
    LTok = 0,
    LAst = 1,
    LSym = 2,
    LBlk = 3,
    LSsa = 4,
    LCs = 5,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EntityRef {
    pub layer: LayerKind,
    pub entity_id: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceRange {
    pub file_id: u16,
    pub line_start: u32,
    pub col_start: u16,
    pub line_end: u32,
    pub col_end: u16,
}

/// AST Backward Index entry (8 bytes, indexed by preorder_idx).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BIAstEntry {
    pub first_token_id: u32,
    pub last_token_id: u32,
}

/// Symbol Backward Index entry (16 bytes, indexed by sym_id).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BISymEntry {
    pub decl_first_tok: u32,
    pub decl_last_tok: u32,
    pub def_first_tok: u32,
    pub def_last_tok: u32,
}

/// Basic Block Backward Index entry (8 bytes, indexed by global_block_id).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BIBlkEntry {
    pub first_token_id: u32,
    pub last_token_id: u32,
}

/// SSA Variable Backward Index entry (12 bytes, indexed by ssa_id).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BISsaEntry {
    pub def_stmt: u32,
    pub first_token_id: u32,
    pub last_token_id: u32,
}

/// Call Site Backward Index entry (4 bytes, indexed by call_site_id).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BICsEntry {
    pub call_token: u32,
}

/// Symbol Span Record for Forward Queries (20 bytes).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SymbolSpanRecord {
    pub first_token_id: u32,
    pub last_token_id: u32,
    pub sym_id: u32,
    pub file_id: u16,
    pub line_start: u16,
    pub col_start: u16,
    pub line_end: u16,
}

/// Call Site Span Record for Forward Queries (12 bytes).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CallSiteSpanRecord {
    pub first_token_id: u32,
    pub call_site_id: u32,
    pub file_id: u16,
    pub line_start: u16,
}

/// Pre-computed UMLLink Record for Phase 9 (24 bytes).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UMLLinkRecord {
    pub sym_id: u32,
    pub file_id: u16,
    pub line_start: u32,
    pub col_start: u16,
    pub line_end: u32,
    pub col_end: u16,
    pub scpg_hash: u32,
    pub sym_kind: u8,
    pub _reserved: [u8; 3],
}

/// Fingerprint Hash Chain across all 6 SCPG build artifacts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScpgHashChain {
    pub tca_hash: u64,
    pub bpa_hash: u64,
    pub sta_hash: u64,
    pub cfa_hash: u64,
    pub ssa_hash: u64,
    pub cga_hash: u64,
    pub scpg_hash: u32,
}

/// Unified Traceability Artifact (.tra).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceabilityArtifact {
    pub format_version: u32,
    pub hashes: ScpgHashChain,
    pub bi_ast: Vec<BIAstEntry>,
    pub bi_sym: Vec<BISymEntry>,
    pub bi_blk: Vec<BIBlkEntry>,
    pub bi_ssa: Vec<BISsaEntry>,
    pub bi_cs: Vec<BICsEntry>,
    pub sym_span: Vec<SymbolSpanRecord>,
    pub cs_span: Vec<CallSiteSpanRecord>,
    pub uml_links: Vec<UMLLinkRecord>,
}
