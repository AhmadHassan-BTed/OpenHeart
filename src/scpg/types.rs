//! Core types for Phase 10: SCPG Unified Binary & Query Engine (§10.2, §10.5).

pub const SCPG_MAGIC: u64 = u64::from_le_bytes(*b"SCPG\x00\x01\x00\x00");
pub const SCPG_FORMAT_VERSION: u32 = 1;
pub const SCPG_HEADER_SIZE: usize = 128;
pub const SCPG_SECTION_COUNT: u32 = 11;
pub const SCPG_DIR_ENTRY_SIZE: usize = 24;

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SCPGSectionType {
    TokenTable = 0x01,       // HOT
    StringTable = 0x02,      // HOT
    BpAst = 0x03,            // COLD
    SymbolTable = 0x04,      // HOT
    Cfg = 0x05,              // COLD
    SsaDfg = 0x06,           // COLD
    CallGraph = 0x07,        // WARM
    TypeHierarchy = 0x08,    // HOT
    Traceability = 0x09,     // HOT
    SemanticMetadata = 0x0A, // WARM
    PathSummaries = 0x0B,    // COLD
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SCPGHeader {
    pub magic: u64,
    pub format_version: u32,
    pub section_count: u32,
    pub source_hash: [u8; 32],
    pub creation_ts_ns: u64,
    pub scpg_hash: u32,
    pub language_count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SectionDirectoryEntry {
    pub section_type: u32,
    pub byte_offset: u64,
    pub byte_length: u64,
    pub crc32: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct QueryKey {
    pub query_type: u8,
    pub params_crc: u64,
    pub scpg_hash: u32,
}

#[derive(Debug, Clone)]
pub enum QueryResult {
    Boolean(bool),
    Count(u64),
    StringList(Vec<String>),
    SymbolList(Vec<u32>),
    Slice(Vec<u32>),
    Json(String),
}
