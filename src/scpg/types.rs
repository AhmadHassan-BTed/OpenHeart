//! Core types for Phase 10: SCPG Unified Binary & Query Engine (§10.2, §10.5).
//!
//! ## On-disk layout (format version 2)
//!
//! ```text
//! Offset  Size  Field
//!   0      8    magic              : u64   b"SCPG\0\x01\0\0"
//!   8      4    format_version     : u32   major version (readers reject any other value)
//!  12      4    section_count      : u32   ≤ SCPG_MAX_SECTION_COUNT
//!  16     32    source_hash        : [u8;32] SHA-256 over the per-file content hashes
//!  48     32    content_digest     : [u8;32] SHA-256 over every section payload (canonical)
//!  80      8    creation_ts_ns     : u64   SOURCE_DATE_EPOCH-aware, excluded from the digest
//!  88      4    scpg_hash          : u32   first 4 bytes (LE) of content_digest
//!  92      4    language_count     : u32
//!  96      4    flags              : u32   reserved, must be zero
//! 100      4    _reserved          : u32   must be zero
//! 104      8    file_length        : u64   total file size in bytes
//! 112      8    header_crc64       : u64   CRC-64/NVME of bytes [0, 112)
//! 120      8    directory_crc64    : u64   CRC-64/NVME of the section directory
//! 128   n×40    section directory  : SectionDirectoryEntry[section_count]
//! ...           sections, each aligned to 64 B (4 KiB once the payload spans a page)
//! ```

use std::fmt;
use std::sync::Arc;

use crate::scpg::query::impact::ImpactEntry;
use crate::scpg::query::lang::QueryOutput;
use crate::scpg::query::slice::ProgramSlice;

pub const SCPG_MAGIC: u64 = u64::from_le_bytes(*b"SCPG\x00\x01\x00\x00");
/// Major on-disk format version. Version 1 files (count-only sections) are rejected.
pub const SCPG_FORMAT_VERSION: u32 = 2;
pub const SCPG_HEADER_SIZE: usize = 128;
pub const SCPG_SECTION_COUNT: u32 = 11;
/// Hard upper bound on the directory size accepted by the loader (validated before allocation).
pub const SCPG_MAX_SECTION_COUNT: u32 = 64;
pub const SCPG_DIR_ENTRY_SIZE: usize = 40;
/// Minimum alignment of every section start (cache line).
pub const SCPG_SECTION_ALIGN: u64 = 64;
/// Alignment used for sections whose payload spans at least one OS page.
pub const SCPG_PAGE_ALIGN: u64 = 4096;

/// Byte offset of the header CRC field; the CRC covers `[0, SCPG_HEADER_CRC_OFFSET)`.
pub const SCPG_HEADER_CRC_OFFSET: usize = 112;

/// Canonical SHA-256 content digest of a `.scpg` snapshot.
pub type ScpgDigest = [u8; 32];

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
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

impl SCPGSectionType {
    pub const ALL: [SCPGSectionType; 11] = [
        SCPGSectionType::TokenTable,
        SCPGSectionType::StringTable,
        SCPGSectionType::BpAst,
        SCPGSectionType::SymbolTable,
        SCPGSectionType::Cfg,
        SCPGSectionType::SsaDfg,
        SCPGSectionType::CallGraph,
        SCPGSectionType::TypeHierarchy,
        SCPGSectionType::Traceability,
        SCPGSectionType::SemanticMetadata,
        SCPGSectionType::PathSummaries,
    ];

    pub fn from_u32(value: u32) -> Option<Self> {
        Self::ALL.iter().copied().find(|t| *t as u32 == value)
    }

    pub fn name(&self) -> &'static str {
        match self {
            SCPGSectionType::TokenTable => "TokenTable",
            SCPGSectionType::StringTable => "StringTable",
            SCPGSectionType::BpAst => "BpAst",
            SCPGSectionType::SymbolTable => "SymbolTable",
            SCPGSectionType::Cfg => "Cfg",
            SCPGSectionType::SsaDfg => "SsaDfg",
            SCPGSectionType::CallGraph => "CallGraph",
            SCPGSectionType::TypeHierarchy => "TypeHierarchy",
            SCPGSectionType::Traceability => "Traceability",
            SCPGSectionType::SemanticMetadata => "SemanticMetadata",
            SCPGSectionType::PathSummaries => "PathSummaries",
        }
    }
}

impl fmt::Display for SCPGSectionType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Storage codec of one section payload. Only uncompressed storage is defined in format v2,
/// which keeps every section directly addressable inside the memory map.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SectionCodec {
    Raw = 0,
}

impl SectionCodec {
    pub fn from_u32(value: u32) -> Option<Self> {
        match value {
            0 => Some(SectionCodec::Raw),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SCPGHeader {
    pub magic: u64,
    pub format_version: u32,
    pub section_count: u32,
    pub source_hash: [u8; 32],
    pub content_digest: ScpgDigest,
    pub creation_ts_ns: u64,
    pub scpg_hash: u32,
    pub language_count: u32,
    pub flags: u32,
    pub file_length: u64,
    pub header_crc64: u64,
    pub directory_crc64: u64,
}

/// One 40-byte section directory record.
///
/// ```text
///  0  4  section_type : u32
///  4  4  codec        : u32   (SectionCodec)
///  8  8  byte_offset  : u64   aligned to SCPG_SECTION_ALIGN
/// 16  8  byte_length  : u64
/// 24  8  checksum     : u64   CRC-64/NVME of the stored bytes
/// 32  8  _reserved    : u64   must be zero
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SectionDirectoryEntry {
    pub section_type: u32,
    pub codec: u32,
    pub byte_offset: u64,
    pub byte_length: u64,
    pub checksum: u64,
}

/// Every way a `.scpg` file can fail validation. All loader paths return one of these instead
/// of panicking or allocating from untrusted sizes.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ScpgFormatError {
    #[error("SCPG file is {len} bytes, at least {required} bytes are required")]
    Truncated { len: u64, required: u64 },
    #[error("SCPG magic mismatch: got 0x{0:016X}")]
    BadMagic(u64),
    #[error("unsupported SCPG format version {found} (this build reads version {supported})")]
    UnsupportedVersion { found: u32, supported: u32 },
    #[error("SCPG section count {found} exceeds the maximum of {max}")]
    TooManySections { found: u32, max: u32 },
    #[error("SCPG header checksum mismatch (stored 0x{stored:016X}, computed 0x{computed:016X})")]
    HeaderChecksum { stored: u64, computed: u64 },
    #[error(
        "SCPG directory checksum mismatch (stored 0x{stored:016X}, computed 0x{computed:016X})"
    )]
    DirectoryChecksum { stored: u64, computed: u64 },
    #[error("SCPG header declares {declared} bytes but the file has {actual} bytes")]
    LengthMismatch { declared: u64, actual: u64 },
    #[error("SCPG reserved header field is non-zero")]
    ReservedNonZero,
    #[error("unknown SCPG section type 0x{0:X}")]
    UnknownSection(u32),
    #[error("duplicate SCPG section {0}")]
    DuplicateSection(SCPGSectionType),
    #[error("SCPG section {0} is missing")]
    MissingSection(SCPGSectionType),
    #[error("unknown codec {codec} on SCPG section {section}")]
    UnknownCodec {
        section: SCPGSectionType,
        codec: u32,
    },
    #[error("SCPG section {section} range [{offset}, +{length}) lies outside the file")]
    SectionOutOfBounds {
        section: SCPGSectionType,
        offset: u64,
        length: u64,
    },
    #[error("SCPG section {section} starts at misaligned offset {offset}")]
    Misaligned {
        section: SCPGSectionType,
        offset: u64,
    },
    #[error("SCPG sections {first} and {second} overlap")]
    Overlap {
        first: SCPGSectionType,
        second: SCPGSectionType,
    },
    #[error("SCPG section {section} checksum mismatch (stored 0x{stored:016X}, computed 0x{computed:016X})")]
    SectionChecksum {
        section: SCPGSectionType,
        stored: u64,
        computed: u64,
    },
    #[error("SCPG content digest mismatch")]
    DigestMismatch,
    #[error("SCPG section {section} is malformed: {reason}")]
    Malformed {
        section: SCPGSectionType,
        reason: String,
    },
}

impl From<ScpgFormatError> for std::io::Error {
    fn from(err: ScpgFormatError) -> Self {
        std::io::Error::new(std::io::ErrorKind::InvalidData, err)
    }
}

/// Kind of a cached query. Each kind owns its parameter encoding in `QueryParams`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QueryKind {
    Slice,
    Impact,
    Program,
}

/// Structured, collision-free query parameters (no CRC folding).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum QueryParams {
    Ids(Vec<u64>),
    Text(String),
}

/// Cache key: query kind + exact parameters + the full SHA-256 digest of the snapshot that
/// answered it. A different snapshot can never produce a colliding key.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct QueryKey {
    pub kind: QueryKind,
    pub params: QueryParams,
    pub digest: ScpgDigest,
}

#[derive(Debug, Clone)]
pub enum QueryResult {
    Boolean(bool),
    Count(u64),
    SymbolList(Vec<u32>),
    Slice(Arc<ProgramSlice>),
    Impact(Arc<Vec<ImpactEntry>>),
    Table(Arc<QueryOutput>),
}
