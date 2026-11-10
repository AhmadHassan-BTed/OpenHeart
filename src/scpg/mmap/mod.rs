//! MemoryMappedSCPG — validated, read-only memory-mapped view of a .scpg file (§10.4).
//!
//! `open` maps the file with memmap2 (no copy) and validates the whole structure before any
//! allocation that depends on file contents: magic, major version, section count bound,
//! header and directory CRC-64s, declared file length, known section types and codecs,
//! alignment, bounds (checked arithmetic), uniqueness and non-overlap. Section checksums are
//! verified lazily on first access and cached; `verify_all` additionally recomputes the
//! SHA-256 content digest. Every failure is an `Err`, never a panic.
//!
//! The writer replaces files by atomic rename, so a mapped file is never modified in place by
//! OpenHeart; external tools that truncate a mapped file in place are outside this contract
//! (`open_in_memory` reads an owned copy for that case).

use std::fmt;
use std::fs;
use std::io::Result as IoResult;
use std::path::Path;
use std::sync::atomic::{AtomicU8, Ordering};

use serde::Serialize;

use crate::cfg::serializer::CFGArtifact;
use crate::core::io::mmap::MemoryMappedFile;
use crate::scpg::query::snapshot::ScpgSnapshot;
use crate::scpg::serializer::integrity::{crc64_nvme, short_hash, ContentDigestBuilder};
use crate::scpg::types::*;

const CHECK_UNKNOWN: u8 = 0;
const CHECK_OK: u8 = 1;
const CHECK_FAILED: u8 = 2;

enum Backing {
    Mapped(MemoryMappedFile),
    Owned(Vec<u8>),
}

impl Backing {
    fn bytes(&self) -> &[u8] {
        match self {
            Backing::Mapped(m) => m.as_slice(),
            Backing::Owned(v) => v.as_slice(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SectionReport {
    pub section: &'static str,
    pub offset: u64,
    pub length: u64,
    pub checksum: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VerificationReport {
    pub format_version: u32,
    pub file_length: u64,
    pub scpg_hash: u32,
    pub content_digest: String,
    pub sections: Vec<SectionReport>,
}

pub struct MemoryMappedSCPG {
    pub header: SCPGHeader,
    pub directory: Vec<SectionDirectoryEntry>,
    backing: Backing,
    verified: Vec<AtomicU8>,
}

impl fmt::Debug for MemoryMappedSCPG {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MemoryMappedSCPG")
            .field("header", &self.header)
            .field("directory", &self.directory)
            .field("mapped", &matches!(self.backing, Backing::Mapped(_)))
            .finish()
    }
}

fn read_u32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

fn read_u64(bytes: &[u8], at: usize) -> u64 {
    let mut b = [0u8; 8];
    b.copy_from_slice(&bytes[at..at + 8]);
    u64::from_le_bytes(b)
}

/// Validates header and directory. All reads are bounds-checked against `bytes.len()` first.
fn parse(bytes: &[u8]) -> Result<(SCPGHeader, Vec<SectionDirectoryEntry>), ScpgFormatError> {
    let len = bytes.len() as u64;
    if bytes.len() < SCPG_HEADER_SIZE {
        return Err(ScpgFormatError::Truncated {
            len,
            required: SCPG_HEADER_SIZE as u64,
        });
    }
    let magic = read_u64(bytes, 0);
    if magic != SCPG_MAGIC {
        return Err(ScpgFormatError::BadMagic(magic));
    }
    let format_version = read_u32(bytes, 8);
    if format_version != SCPG_FORMAT_VERSION {
        return Err(ScpgFormatError::UnsupportedVersion {
            found: format_version,
            supported: SCPG_FORMAT_VERSION,
        });
    }
    let section_count = read_u32(bytes, 12);
    if section_count > SCPG_MAX_SECTION_COUNT {
        return Err(ScpgFormatError::TooManySections {
            found: section_count,
            max: SCPG_MAX_SECTION_COUNT,
        });
    }
    let stored_header_crc = read_u64(bytes, SCPG_HEADER_CRC_OFFSET);
    let computed_header_crc = crc64_nvme(&bytes[..SCPG_HEADER_CRC_OFFSET]);
    if stored_header_crc != computed_header_crc {
        return Err(ScpgFormatError::HeaderChecksum {
            stored: stored_header_crc,
            computed: computed_header_crc,
        });
    }
    let file_length = read_u64(bytes, 104);
    if file_length != len {
        return Err(ScpgFormatError::LengthMismatch {
            declared: file_length,
            actual: len,
        });
    }
    if read_u32(bytes, 96) != 0 || read_u32(bytes, 100) != 0 {
        return Err(ScpgFormatError::ReservedNonZero);
    }

    // section_count ≤ 64, so this cannot overflow.
    let dir_end = SCPG_HEADER_SIZE + section_count as usize * SCPG_DIR_ENTRY_SIZE;
    if bytes.len() < dir_end {
        return Err(ScpgFormatError::Truncated {
            len,
            required: dir_end as u64,
        });
    }
    let stored_dir_crc = read_u64(bytes, 120);
    let computed_dir_crc = crc64_nvme(&bytes[SCPG_HEADER_SIZE..dir_end]);
    if stored_dir_crc != computed_dir_crc {
        return Err(ScpgFormatError::DirectoryChecksum {
            stored: stored_dir_crc,
            computed: computed_dir_crc,
        });
    }

    let mut source_hash = [0u8; 32];
    source_hash.copy_from_slice(&bytes[16..48]);
    let mut content_digest = [0u8; 32];
    content_digest.copy_from_slice(&bytes[48..80]);
    let scpg_hash = read_u32(bytes, 88);
    if scpg_hash != short_hash(&content_digest) {
        return Err(ScpgFormatError::DigestMismatch);
    }
    let header = SCPGHeader {
        magic,
        format_version,
        section_count,
        source_hash,
        content_digest,
        creation_ts_ns: read_u64(bytes, 80),
        scpg_hash,
        language_count: read_u32(bytes, 92),
        flags: read_u32(bytes, 96),
        file_length,
        header_crc64: stored_header_crc,
        directory_crc64: stored_dir_crc,
    };

    let mut directory = Vec::with_capacity(section_count as usize);
    let mut seen: Vec<SCPGSectionType> = Vec::with_capacity(section_count as usize);
    for i in 0..section_count as usize {
        let at = SCPG_HEADER_SIZE + i * SCPG_DIR_ENTRY_SIZE;
        let raw_type = read_u32(bytes, at);
        let section =
            SCPGSectionType::from_u32(raw_type).ok_or(ScpgFormatError::UnknownSection(raw_type))?;
        if seen.contains(&section) {
            return Err(ScpgFormatError::DuplicateSection(section));
        }
        seen.push(section);
        let codec = read_u32(bytes, at + 4);
        if SectionCodec::from_u32(codec).is_none() {
            return Err(ScpgFormatError::UnknownCodec { section, codec });
        }
        let byte_offset = read_u64(bytes, at + 8);
        let byte_length = read_u64(bytes, at + 16);
        let checksum = read_u64(bytes, at + 24);
        if read_u64(bytes, at + 32) != 0 {
            return Err(ScpgFormatError::ReservedNonZero);
        }
        if !byte_offset.is_multiple_of(SCPG_SECTION_ALIGN) {
            return Err(ScpgFormatError::Misaligned {
                section,
                offset: byte_offset,
            });
        }
        let in_bounds = byte_offset
            .checked_add(byte_length)
            .map(|end| byte_offset >= dir_end as u64 && end <= len)
            .unwrap_or(false);
        if !in_bounds {
            return Err(ScpgFormatError::SectionOutOfBounds {
                section,
                offset: byte_offset,
                length: byte_length,
            });
        }
        directory.push(SectionDirectoryEntry {
            section_type: raw_type,
            codec,
            byte_offset,
            byte_length,
            checksum,
        });
    }

    let mut by_offset: Vec<&SectionDirectoryEntry> = directory.iter().collect();
    by_offset.sort_by_key(|e| e.byte_offset);
    for w in by_offset.windows(2) {
        if w[0].byte_offset + w[0].byte_length > w[1].byte_offset {
            return Err(ScpgFormatError::Overlap {
                first: SCPGSectionType::from_u32(w[0].section_type)
                    .unwrap_or(SCPGSectionType::TokenTable),
                second: SCPGSectionType::from_u32(w[1].section_type)
                    .unwrap_or(SCPGSectionType::TokenTable),
            });
        }
    }
    for ty in SCPGSectionType::ALL {
        if !seen.contains(&ty) {
            return Err(ScpgFormatError::MissingSection(ty));
        }
    }
    Ok((header, directory))
}

impl MemoryMappedSCPG {
    /// Memory-maps (without copying) and validates `path`.
    pub fn open(path: &Path) -> IoResult<Self> {
        // SAFETY: `.scpg` files are only ever replaced by `write_atomic` (write a temporary
        // sibling, then rename), never truncated or rewritten in place, so the mapped bytes
        // stay valid and immutable for the lifetime of the mapping. Callers that cannot rule
        // out in-place modification by foreign tools use `open_in_memory`.
        let mapped = unsafe { MemoryMappedFile::map(path)? };
        Ok(Self::from_backing(Backing::Mapped(mapped))?)
    }

    /// Reads `path` into memory instead of mapping it (for filesystems without mmap support).
    pub fn open_in_memory(path: &Path) -> IoResult<Self> {
        let bytes = fs::read(path)?;
        Ok(Self::from_backing(Backing::Owned(bytes))?)
    }

    /// Validates an in-memory `.scpg` image.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, ScpgFormatError> {
        Self::from_backing(Backing::Owned(bytes))
    }

    fn from_backing(backing: Backing) -> Result<Self, ScpgFormatError> {
        let (header, directory) = parse(backing.bytes())?;
        let verified = directory
            .iter()
            .map(|_| AtomicU8::new(CHECK_UNKNOWN))
            .collect();
        Ok(Self {
            header,
            directory,
            backing,
            verified,
        })
    }

    pub fn is_memory_mapped(&self) -> bool {
        matches!(&self.backing, Backing::Mapped(m) if m.is_mapped())
    }

    pub fn file_len(&self) -> u64 {
        self.backing.bytes().len() as u64
    }

    fn entry(
        &self,
        sec_type: SCPGSectionType,
    ) -> Result<(usize, &SectionDirectoryEntry), ScpgFormatError> {
        self.directory
            .iter()
            .enumerate()
            .find(|(_, e)| e.section_type == sec_type as u32)
            .ok_or(ScpgFormatError::MissingSection(sec_type))
    }

    /// Checksum-verified payload of a section (zero-copy into the mapping). The CRC is
    /// computed on first access and the outcome cached.
    pub fn get_section(&self, sec_type: SCPGSectionType) -> Result<&[u8], ScpgFormatError> {
        let (i, entry) = self.entry(sec_type)?;
        // Bounds were validated at open.
        let start = entry.byte_offset as usize;
        let payload = &self.backing.bytes()[start..start + entry.byte_length as usize];
        match self.verified[i].load(Ordering::Acquire) {
            CHECK_OK => Ok(payload),
            state => {
                let computed = crc64_nvme(payload);
                if computed == entry.checksum {
                    self.verified[i].store(CHECK_OK, Ordering::Release);
                    Ok(payload)
                } else {
                    if state == CHECK_UNKNOWN {
                        self.verified[i].store(CHECK_FAILED, Ordering::Release);
                    }
                    Err(ScpgFormatError::SectionChecksum {
                        section: sec_type,
                        stored: entry.checksum,
                        computed,
                    })
                }
            }
        }
    }

    /// Verifies every section checksum and the SHA-256 content digest.
    pub fn verify_all(&self) -> Result<VerificationReport, ScpgFormatError> {
        let mut digest = ContentDigestBuilder::new();
        let mut sections = Vec::with_capacity(self.directory.len());
        for entry in &self.directory {
            let ty = SCPGSectionType::from_u32(entry.section_type)
                .ok_or(ScpgFormatError::UnknownSection(entry.section_type))?;
            let payload = self.get_section(ty)?;
            digest.section(ty, payload);
            sections.push(SectionReport {
                section: ty.name(),
                offset: entry.byte_offset,
                length: entry.byte_length,
                checksum: entry.checksum,
            });
        }
        let computed = digest.finish();
        if computed != self.header.content_digest {
            return Err(ScpgFormatError::DigestMismatch);
        }
        Ok(VerificationReport {
            format_version: self.header.format_version,
            file_length: self.header.file_length,
            scpg_hash: self.header.scpg_hash,
            content_digest: crate::scpg::serializer::integrity::digest_hex(&computed),
            sections,
        })
    }

    /// Decodes the query snapshot stored in this file (no analysis phase is re-run).
    pub fn snapshot(&self) -> Result<ScpgSnapshot, ScpgFormatError> {
        ScpgSnapshot::decode(
            |ty| self.get_section(ty),
            self.header.content_digest,
            self.header.source_hash,
        )
    }

    /// Parses the canonical `.cfa` artifact embedded in the `Cfg` section.
    pub fn cfg_artifact(&self) -> Result<CFGArtifact, ScpgFormatError> {
        ScpgSnapshot::cfg_from_section(self.get_section(SCPGSectionType::Cfg)?)
    }
}
