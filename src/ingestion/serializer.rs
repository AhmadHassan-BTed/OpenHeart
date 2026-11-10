//! `.tca` Token Corpus Artifact: binary writer and validating reader.
//!
//! Layout (all integers little-endian):
//! ```text
//! HEADER (64 B)      magic u64 | version u32 | token_count u32 | file_count u16 |
//!                    string_count u32 | flags u16 | source_tree_hash [32] | created_ns u64
//! FILE RECORDS       file_count x 64 B (SourceFileRecord; path_str_offset = path text_id)
//! FILE PATHS         file_count x (len u32 | utf-8 bytes), then zero padding to 8 B
//! TOKEN TABLE        token_count x 16 B, sorted by sort_key (forward index)
//! ENTRY MAP          token_count x 16 B, only when header flag TCA_FLAG_SEPARATE_ENTRY_MAP
//!                    is set (it is omitted when identical to the token table)
//! STRING HEADERS     string_count x (fnv1a_64 u64 | storage offset u32), sorted by hash
//! STRING STORAGE     string_count x (len u32 | bytes), in text_id order
//! DIAGNOSTICS        "DIAG" u32 | entry_count u32 | entries (see `write_diagnostics`)
//! CHECKSUM           CRC-64/ECMA u64 over everything above
//! ```
//! Version 1 files (u16 path and string length prefixes, an always-present entry map,
//! path_str_offset as a byte offset, no diagnostics) are still accepted by the reader.
//! CRC-64 detects accidental corruption only; the reader additionally validates every
//! count and offset against the bytes present before it allocates or indexes.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::core::io::binary::{pad_to_alignment, ByteCursor};
use crate::core::io::fs::write_atomic;
use crate::core::types::artifact::Artifact;
use crate::core::types::source::{FileDiagnostics, SkipReason, SourceFileRecord};
use crate::core::types::token::{TokenEntry, TokenRecord};
use crate::ingestion::interner::{fnv1a_64, StringInterner};

pub const TCA_MAGIC: u64 = 0x544F4B434F525001; // "TOKCORP\x01"
pub const TCA_VERSION: u32 = 0x00000002;
/// Oldest format version the reader accepts.
pub const TCA_MIN_READ_VERSION: u32 = 0x00000001;
pub const TCA_HEADER_SIZE: usize = 64;
pub const TCA_FILE_RECORD_SIZE: usize = 64;
pub const TCA_TOKEN_RECORD_SIZE: usize = 16;

/// Header flag: the backward entry map is stored separately because it differs from the
/// forward token table. Bits 0..3 of the header flags hold the TokenFilter settings.
pub const TCA_FLAG_SEPARATE_ENTRY_MAP: u16 = 1 << 8;

const DIAG_SECTION_MAGIC: u32 = 0x4741_4944; // "DIAG"

const CRC64_ECMA_POLY: u64 = 0x42F0_E1EB_A9EA_3693;

const fn build_crc64_ecma_table() -> [u64; 256] {
    let mut table = [0u64; 256];
    let mut i = 0;
    while i < 256 {
        let mut crc = (i as u64) << 56;
        let mut bit = 0;
        while bit < 8 {
            crc = if (crc & 0x8000_0000_0000_0000) != 0 {
                (crc << 1) ^ CRC64_ECMA_POLY
            } else {
                crc << 1
            };
            bit += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
}

static CRC64_ECMA_TABLE: [u64; 256] = build_crc64_ecma_table();

/// CRC-64/ECMA-182 (MSB-first, init 0, no final xor), table-driven: one lookup per byte
/// instead of eight conditional shifts. Bit-identical to the original bitwise version.
pub fn crc64_ecma(bytes: &[u8]) -> u64 {
    let mut crc: u64 = 0;
    for &b in bytes {
        let idx = (((crc >> 56) as u8) ^ b) as usize;
        crc = (crc << 8) ^ CRC64_ECMA_TABLE[idx];
    }
    crc
}

#[derive(Debug, Clone)]
pub struct TokenCorpusArtifact {
    pub file_records: Vec<SourceFileRecord>,
    pub token_records: Vec<TokenRecord>,
    pub token_entries: Vec<TokenEntry>,
    pub interner: StringInterner,
}

impl TokenCorpusArtifact {
    /// Path of `file_id` as recorded in Phase 1 (`path_str_offset` is its text_id).
    pub fn file_path(&self, file_id: u16) -> Option<&str> {
        let rec = self.file_records.get(file_id as usize)?;
        let bytes = self.interner.lookup_text(rec.path_str_offset);
        if bytes.is_empty() {
            return None;
        }
        std::str::from_utf8(bytes).ok()
    }

    /// True when the backward entry map is element-wise identical to the forward table.
    pub fn entries_match_records(&self) -> bool {
        self.token_records.len() == self.token_entries.len()
            && self
                .token_records
                .iter()
                .zip(&self.token_entries)
                .all(|(r, e)| {
                    r.sort_key == e.sort_key
                        && r.text_id == e.text_id
                        && r.len == e.len
                        && r.token_type == e.token_type
                        && r._padding == e._padding
                })
    }
}

impl Artifact for TokenCorpusArtifact {
    fn format_version(&self) -> u32 {
        TCA_VERSION
    }

    fn token_count(&self) -> u32 {
        u32::try_from(self.token_records.len()).unwrap_or(u32::MAX)
    }

    fn file_count(&self) -> u16 {
        u16::try_from(self.file_records.len()).unwrap_or(u16::MAX)
    }
}

/// Everything a `.tca` file holds.
#[derive(Debug, Clone)]
pub struct TcaContents {
    pub version: u32,
    pub flags: u16,
    pub source_tree_hash: [u8; 32],
    pub created_ns: u64,
    pub artifact: TokenCorpusArtifact,
    /// File paths in file-id order, from the path section.
    pub file_paths: Vec<String>,
    /// Per-file diagnostics (empty for version 1 files, which did not record them).
    pub diagnostics: Vec<FileDiagnostics>,
}

pub struct TokenCorpusSerializer;

impl TokenCorpusSerializer {
    /// Encodes the corpus into the version-2 byte layout.
    pub fn to_bytes(
        artifact: &TokenCorpusArtifact,
        file_paths: &[PathBuf],
        flags: u16,
        source_tree_hash: [u8; 32],
        diagnostics: &[FileDiagnostics],
    ) -> Result<Vec<u8>, String> {
        if file_paths.len() != artifact.file_records.len() {
            return Err(format!(
                "{} file paths supplied for {} file records",
                file_paths.len(),
                artifact.file_records.len()
            ));
        }
        let token_count = u32::try_from(artifact.token_records.len()).map_err(|_| {
            format!(
                "{} tokens exceed the .tca limit of {}",
                artifact.token_records.len(),
                u32::MAX
            )
        })?;
        let file_count = u16::try_from(artifact.file_records.len()).map_err(|_| {
            format!(
                "{} files exceed the .tca limit of {}",
                artifact.file_records.len(),
                u16::MAX
            )
        })?;

        let separate_entries = !artifact.entries_match_records();
        let flags = if separate_entries {
            flags | TCA_FLAG_SEPARATE_ENTRY_MAP
        } else {
            flags & !TCA_FLAG_SEPARATE_ENTRY_MAP
        };

        let token_bytes = artifact.token_records.len() * TCA_TOKEN_RECORD_SIZE;
        let mut buf: Vec<u8> = Vec::with_capacity(
            TCA_HEADER_SIZE
                + artifact.file_records.len() * TCA_FILE_RECORD_SIZE
                + token_bytes * if separate_entries { 2 } else { 1 }
                + artifact.interner.get_storage_bytes().len()
                + 4096,
        );

        // ── Section 1: HEADER (64 bytes) ──
        buf.extend_from_slice(&TCA_MAGIC.to_le_bytes());
        buf.extend_from_slice(&TCA_VERSION.to_le_bytes());
        buf.extend_from_slice(&token_count.to_le_bytes());
        buf.extend_from_slice(&file_count.to_le_bytes());
        buf.extend_from_slice(&artifact.interner.count().to_le_bytes());
        buf.extend_from_slice(&flags.to_le_bytes());
        buf.extend_from_slice(&source_tree_hash);
        let created_ns = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX))
            .unwrap_or(0);
        buf.extend_from_slice(&created_ns.to_le_bytes());
        debug_assert_eq!(
            buf.len(),
            TCA_HEADER_SIZE,
            "Header must be exactly 64 bytes"
        );

        // ── Section 2: FILE REGISTRY (F * 64 B) ──
        for rec in &artifact.file_records {
            buf.extend_from_slice(&rec.file_id.to_le_bytes());
            buf.push(rec.language_id);
            buf.push(rec.flags);
            buf.extend_from_slice(&rec.content_sha256);
            buf.extend_from_slice(&rec.path_str_offset.to_le_bytes());
            buf.extend_from_slice(&rec.file_size_bytes.to_le_bytes());
            buf.extend_from_slice(&rec.mtime_ns.to_le_bytes());
            buf.extend_from_slice(&rec.first_token_id.to_le_bytes());
            buf.extend_from_slice(&rec.file_token_count.to_le_bytes());
        }

        // ── Section 3: FILE PATHS ──
        for path in file_paths {
            let path_str = path.to_string_lossy();
            let bytes = path_str.as_bytes();
            let len = u32::try_from(bytes.len())
                .map_err(|_| format!("path {} is longer than 4 GiB", path.display()))?;
            buf.extend_from_slice(&len.to_le_bytes());
            buf.extend_from_slice(bytes);
        }
        pad_to_alignment(&mut buf, 8);

        // ── Section 4: TOKEN TABLE / Forward Index (n_tok * 16 B) ──
        for rec in &artifact.token_records {
            buf.extend_from_slice(&rec.sort_key.to_le_bytes());
            buf.extend_from_slice(&rec.text_id.to_le_bytes());
            buf.extend_from_slice(&rec.len.to_le_bytes());
            buf.push(rec.token_type);
            buf.push(rec._padding);
        }

        // ── Section 5: ENTRY MAP / Backward Index, only when it differs ──
        if separate_entries {
            for entry in &artifact.token_entries {
                buf.extend_from_slice(&entry.sort_key.to_le_bytes());
                buf.extend_from_slice(&entry.text_id.to_le_bytes());
                buf.extend_from_slice(&entry.len.to_le_bytes());
                buf.push(entry.token_type);
                buf.push(entry._padding);
            }
        }

        // ── Section 6: STR HEADERS & Section 7: STR STORAGE ──
        let string_count = artifact.interner.count();
        let offsets = artifact.interner.get_offsets();
        let mut str_headers = Vec::with_capacity(string_count as usize);
        for text_id in 0..string_count {
            let text = artifact.interner.lookup_text(text_id);
            str_headers.push((fnv1a_64(text), offsets[text_id as usize]));
        }
        str_headers.sort_unstable();
        for (hash, offset) in str_headers {
            buf.extend_from_slice(&hash.to_le_bytes());
            buf.extend_from_slice(&offset.to_le_bytes());
        }
        buf.extend_from_slice(artifact.interner.get_storage_bytes());

        // ── Section 8: DIAGNOSTICS ──
        write_diagnostics(&mut buf, diagnostics)?;

        // ── Section 9: CHECKSUM (8 bytes) ──
        let checksum = crc64_ecma(&buf);
        buf.extend_from_slice(&checksum.to_le_bytes());
        Ok(buf)
    }

    /// Encodes the corpus and atomically replaces `out_path` with it. Returns the exact
    /// bytes written so callers can hash or reuse them without reading the file back.
    pub fn write(
        artifact: &TokenCorpusArtifact,
        file_paths: &[PathBuf],
        flags: u16,
        source_tree_hash: [u8; 32],
        diagnostics: &[FileDiagnostics],
        out_path: &Path,
    ) -> Result<Vec<u8>, String> {
        let bytes = Self::to_bytes(artifact, file_paths, flags, source_tree_hash, diagnostics)?;
        write_atomic(out_path, &bytes)
            .map_err(|e| format!("Failed to write {}: {}", out_path.display(), e))?;
        Ok(bytes)
    }

    pub fn read(in_path: &Path) -> Result<TokenCorpusArtifact, String> {
        let buffer = fs::read(in_path).map_err(|e| format!("{}: {}", in_path.display(), e))?;
        Self::from_bytes(&buffer)
    }

    /// Decodes and validates a `.tca` image.
    pub fn from_bytes(bytes: &[u8]) -> Result<TokenCorpusArtifact, String> {
        Self::decode(bytes).map(|c| c.artifact)
    }

    /// Verifies checksum, magic, version and every structural invariant, returning the
    /// full contents. Used by Phase 2 and by artifact inspection.
    pub fn decode(bytes: &[u8]) -> Result<TcaContents, String> {
        if bytes.len() < TCA_HEADER_SIZE + 8 {
            return Err(format!(
                "Invalid .tca file: {} bytes is smaller than the {}-byte header plus checksum",
                bytes.len(),
                TCA_HEADER_SIZE
            ));
        }

        let payload_len = bytes.len() - 8;
        let mut trailer = [0u8; 8];
        trailer.copy_from_slice(&bytes[payload_len..]);
        let stored_checksum = u64::from_le_bytes(trailer);
        let computed_checksum = crc64_ecma(&bytes[..payload_len]);
        if stored_checksum != computed_checksum {
            return Err(format!(
                "Checksum Mismatch: Stored 0x{:016X} != Computed 0x{:016X}",
                stored_checksum, computed_checksum
            ));
        }

        decode_payload(&bytes[..payload_len]).map_err(|e| format!("Malformed .tca: {}", e))
    }
}

/// DIAG section: `"DIAG" u32 | count u32 | entries`, each entry
/// `file_id u16 | skip_reason u8 | reserved u8 | error_count u32 | missing_count u32 |
///  range_count u16 | detail_len u16 | ranges (start u32, end u32) x range_count | detail`.
fn write_diagnostics(buf: &mut Vec<u8>, diagnostics: &[FileDiagnostics]) -> Result<(), String> {
    let count =
        u32::try_from(diagnostics.len()).map_err(|_| "too many diagnostics entries".to_string())?;
    buf.extend_from_slice(&DIAG_SECTION_MAGIC.to_le_bytes());
    buf.extend_from_slice(&count.to_le_bytes());
    for d in diagnostics {
        let range_count = u16::try_from(d.error_ranges.len())
            .map_err(|_| format!("file {}: too many error ranges", d.file_id))?;
        let detail = truncate_utf8(&d.detail, u16::MAX as usize);
        let detail_len = detail.len() as u16;
        buf.extend_from_slice(&d.file_id.to_le_bytes());
        buf.push(d.skip_reason as u8);
        buf.push(0);
        buf.extend_from_slice(&d.error_count.to_le_bytes());
        buf.extend_from_slice(&d.missing_count.to_le_bytes());
        buf.extend_from_slice(&range_count.to_le_bytes());
        buf.extend_from_slice(&detail_len.to_le_bytes());
        for &(start, end) in &d.error_ranges {
            buf.extend_from_slice(&start.to_le_bytes());
            buf.extend_from_slice(&end.to_le_bytes());
        }
        buf.extend_from_slice(detail.as_bytes());
    }
    Ok(())
}

/// Longest prefix of `s` that is at most `max` bytes and ends on a char boundary.
fn truncate_utf8(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

fn read_token_table(cur: &mut ByteCursor, count: usize) -> std::io::Result<Vec<TokenRecord>> {
    let n = cur.checked_count(count as u64, TCA_TOKEN_RECORD_SIZE)?;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let sort_key = cur.read_u64()?;
        let text_id = cur.read_u32()?;
        let len = cur.read_u16()?;
        let token_type = cur.read_u8()?;
        let padding = cur.read_u8()?;
        out.push(TokenRecord {
            sort_key,
            text_id,
            len,
            token_type,
            _padding: padding,
        });
    }
    Ok(out)
}

fn decode_payload(payload: &[u8]) -> Result<TcaContents, String> {
    let io = |e: std::io::Error| e.to_string();
    let mut cur = ByteCursor::new(payload);

    let magic = cur.read_u64().map_err(io)?;
    if magic != TCA_MAGIC {
        return Err(format!("Invalid Magic: 0x{:016X}", magic));
    }
    let version = cur.read_u32().map_err(io)?;
    if !(TCA_MIN_READ_VERSION..=TCA_VERSION).contains(&version) {
        return Err(format!(
            "Unsupported Version: {} (this build reads {}..={})",
            version, TCA_MIN_READ_VERSION, TCA_VERSION
        ));
    }
    let legacy = version == 1;

    let token_count = cur.read_u32().map_err(io)? as usize;
    let file_count = cur.read_u16().map_err(io)? as usize;
    let string_count = cur.read_u32().map_err(io)? as usize;
    let flags = cur.read_u16().map_err(io)?;
    let source_tree_hash: [u8; 32] = cur.read_array().map_err(io)?;
    let created_ns = cur.read_u64().map_err(io)?;

    // ── File records ──
    let n_files = cur
        .checked_count(file_count as u64, TCA_FILE_RECORD_SIZE)
        .map_err(io)?;
    let mut file_records = Vec::with_capacity(n_files);
    for idx in 0..n_files {
        let file_id = cur.read_u16().map_err(io)?;
        let language_id = cur.read_u8().map_err(io)?;
        let file_flags = cur.read_u8().map_err(io)?;
        let content_sha256: [u8; 32] = cur.read_array().map_err(io)?;
        let path_str_offset = cur.read_u32().map_err(io)?;
        let file_size_bytes = cur.read_u64().map_err(io)?;
        let mtime_ns = cur.read_u64().map_err(io)?;
        let first_token_id = cur.read_u32().map_err(io)?;
        let file_token_count = cur.read_u32().map_err(io)?;

        if file_id as usize != idx {
            return Err(format!("file record {} carries file_id {}", idx, file_id));
        }
        let end = (first_token_id as u64) + (file_token_count as u64);
        if file_token_count > 0 && end > token_count as u64 {
            return Err(format!(
                "file {} token span {}..{} exceeds token count {}",
                idx, first_token_id, end, token_count
            ));
        }
        file_records.push(SourceFileRecord {
            file_id,
            language_id,
            flags: file_flags,
            content_sha256,
            path_str_offset,
            file_size_bytes,
            mtime_ns,
            first_token_id,
            file_token_count,
        });
    }

    // ── File paths ──
    let paths_start = cur.position();
    let mut path_offsets = Vec::with_capacity(n_files);
    let mut file_paths = Vec::with_capacity(n_files);
    for idx in 0..n_files {
        path_offsets.push(cur.position() - paths_start);
        let len = if legacy {
            cur.read_u16().map_err(io)? as usize
        } else {
            cur.read_u32().map_err(io)? as usize
        };
        let bytes = cur.read_bytes(len).map_err(io)?;
        let path = std::str::from_utf8(bytes)
            .map_err(|_| format!("path of file {} is not valid UTF-8", idx))?;
        file_paths.push(path.to_string());
    }
    cur.align_to(8).map_err(io)?;

    // ── Token table and entry map ──
    let token_records = read_token_table(&mut cur, token_count).map_err(io)?;
    for (i, pair) in token_records.windows(2).enumerate() {
        if pair[1].sort_key <= pair[0].sort_key {
            return Err(format!(
                "token table is not strictly sorted at index {}",
                i + 1
            ));
        }
    }
    let separate_entries = legacy || (flags & TCA_FLAG_SEPARATE_ENTRY_MAP) != 0;
    let token_entries: Vec<TokenEntry> = if separate_entries {
        read_token_table(&mut cur, token_count)
            .map_err(io)?
            .into_iter()
            .map(|r| TokenEntry {
                sort_key: r.sort_key,
                text_id: r.text_id,
                len: r.len,
                token_type: r.token_type,
                _padding: r._padding,
            })
            .collect()
    } else {
        token_records
            .iter()
            .map(|r| TokenEntry {
                sort_key: r.sort_key,
                text_id: r.text_id,
                len: r.len,
                token_type: r.token_type,
                _padding: r._padding,
            })
            .collect()
    };

    // ── String headers (hash index; not needed to rebuild the interner) ──
    let n_strings = cur.checked_count(string_count as u64, 12).map_err(io)?;
    cur.skip(n_strings * 12).map_err(io)?;

    // ── String storage ──
    let prefix = if legacy { 2 } else { 4 };
    cur.checked_count(n_strings as u64, prefix).map_err(io)?;
    let mut interner = StringInterner::with_capacity(n_strings);
    for expected_id in 0..n_strings {
        let len = if legacy {
            cur.read_u16().map_err(io)? as usize
        } else {
            cur.read_u32().map_err(io)? as usize
        };
        let bytes = cur.read_bytes(len).map_err(io)?;
        let id = interner.intern(bytes);
        if id as usize != expected_id {
            return Err(format!(
                "string {} duplicates string {}; text ids would be inconsistent",
                expected_id, id
            ));
        }
    }

    for (i, rec) in token_records.iter().enumerate() {
        if rec.text_id as usize >= n_strings {
            return Err(format!(
                "token {} references text_id {} of {} strings",
                i, rec.text_id, n_strings
            ));
        }
    }

    // ── path_str_offset: one meaning (text_id) ──
    for (idx, rec) in file_records.iter_mut().enumerate() {
        if legacy {
            // Version 1 stored a byte offset into the path section.
            if rec.path_str_offset as usize != path_offsets[idx] {
                return Err(format!(
                    "file {} path offset {} does not match the path section",
                    idx, rec.path_str_offset
                ));
            }
            let id = interner.find_id(file_paths[idx].as_bytes());
            if id == u32::MAX {
                return Err(format!(
                    "path of file {} is missing from the string table",
                    idx
                ));
            }
            rec.path_str_offset = id;
        } else if rec.path_str_offset as usize >= n_strings {
            return Err(format!(
                "file {} path text_id {} out of range",
                idx, rec.path_str_offset
            ));
        }
    }

    // ── Diagnostics ──
    let diagnostics = if legacy {
        Vec::new()
    } else {
        read_diagnostics(&mut cur).map_err(io)?
    };

    if !cur.is_at_end() {
        return Err(format!(
            "{} unexpected trailing bytes before the checksum",
            cur.remaining()
        ));
    }

    Ok(TcaContents {
        version,
        flags,
        source_tree_hash,
        created_ns,
        artifact: TokenCorpusArtifact {
            file_records,
            token_records,
            token_entries,
            interner,
        },
        file_paths,
        diagnostics,
    })
}

fn read_diagnostics(cur: &mut ByteCursor) -> std::io::Result<Vec<FileDiagnostics>> {
    use crate::core::io::binary::invalid_data;

    let magic = cur.read_u32()?;
    if magic != DIAG_SECTION_MAGIC {
        return Err(invalid_data(format!(
            "expected DIAG section, found 0x{:08X}",
            magic
        )));
    }
    let count = cur.read_u32()?;
    // Each entry has a fixed 16-byte head.
    let n = cur.checked_count(count as u64, 16)?;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let file_id = cur.read_u16()?;
        let skip_reason = SkipReason::from(cur.read_u8()?);
        let _reserved = cur.read_u8()?;
        let error_count = cur.read_u32()?;
        let missing_count = cur.read_u32()?;
        let range_count = cur.read_u16()? as usize;
        let detail_len = cur.read_u16()? as usize;
        let n_ranges = cur.checked_count(range_count as u64, 8)?;
        let mut error_ranges = Vec::with_capacity(n_ranges);
        for _ in 0..n_ranges {
            let start = cur.read_u32()?;
            let end = cur.read_u32()?;
            error_ranges.push((start, end));
        }
        let detail_bytes = cur.read_bytes(detail_len)?;
        let detail = String::from_utf8_lossy(detail_bytes).into_owned();
        out.push(FileDiagnostics {
            file_id,
            skip_reason,
            error_count,
            missing_count,
            error_ranges,
            detail,
        });
    }
    Ok(out)
}
