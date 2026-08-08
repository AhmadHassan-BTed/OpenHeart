use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::core::io::binary::{BinaryReader, BinaryWriter};
use crate::core::types::artifact::Artifact;
use crate::core::types::source::SourceFileRecord;
use crate::core::types::token::{TokenEntry, TokenRecord};
use crate::phase1::interner::{fnv1a_64, StringInterner};

pub const TCA_MAGIC: u64 = 0x544F4B434F525001; // "TOKCORP\x01"
pub const TCA_VERSION: u32 = 0x00000001;

/// CRC-64/ECMA calculation function.
pub fn crc64_ecma(bytes: &[u8]) -> u64 {
    let mut crc: u64 = 0;
    for &b in bytes {
        crc ^= (b as u64) << 56;
        for _ in 0..8 {
            if (crc & 0x8000_0000_0000_0000) != 0 {
                crc = (crc << 1) ^ 0x42F0_E1EB_A9EA_3693;
            } else {
                crc <<= 1;
            }
        }
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

impl Artifact for TokenCorpusArtifact {
    fn format_version(&self) -> u32 {
        TCA_VERSION
    }

    fn token_count(&self) -> u32 {
        self.token_records.len() as u32
    }

    fn file_count(&self) -> u16 {
        self.file_records.len() as u16
    }
}

pub struct TokenCorpusSerializer;

impl TokenCorpusSerializer {
    pub fn write(
        artifact: &TokenCorpusArtifact,
        file_paths: &[PathBuf],
        flags: u16,
        source_tree_hash: [u8; 32],
        out_path: &Path,
    ) -> Result<(), String> {
        let mut buffer = Vec::new();
        {
            let mut bw = BinaryWriter::new(&mut buffer);

            // ── Section 1: HEADER (64 bytes) ──
            bw.write_u64(TCA_MAGIC).map_err(|e| e.to_string())?;
            bw.write_u32(TCA_VERSION).map_err(|e| e.to_string())?;
            bw.write_u32(artifact.token_count()).map_err(|e| e.to_string())?;
            bw.write_u16(artifact.file_count()).map_err(|e| e.to_string())?;
            bw.write_u32(artifact.interner.count()).map_err(|e| e.to_string())?;
            bw.write_u16(flags).map_err(|e| e.to_string())?;
            bw.write_bytes(&source_tree_hash).map_err(|e| e.to_string())?;

            let ts_ns = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0);
            bw.write_u64(ts_ns).map_err(|e| e.to_string())?;

            assert_eq!(bw.written_bytes(), 64, "Header must be exactly 64 bytes");

            // ── Section 2: FILE REGISTRY & Section 3: FILE PATHS ──
            // Build path string section
            let mut path_bytes_sec = Vec::new();
            let mut file_recs_updated = artifact.file_records.clone();

            for (idx, path) in file_paths.iter().enumerate() {
                let path_str = path.to_string_lossy();
                let path_bytes = path_str.as_bytes();
                let offset = path_bytes_sec.len() as u32;
                file_recs_updated[idx].path_str_offset = offset;

                let len = (path_bytes.len().min(u16::MAX as usize)) as u16;
                path_bytes_sec.extend_from_slice(&len.to_le_bytes());
                path_bytes_sec.extend_from_slice(&path_bytes[..len as usize]);
            }

            // Write File Records (F * 64 B)
            for rec in &file_recs_updated {
                bw.write_u16(rec.file_id).map_err(|e| e.to_string())?;
                bw.write_u8(rec.language_id).map_err(|e| e.to_string())?;
                bw.write_u8(rec.flags).map_err(|e| e.to_string())?;
                bw.write_bytes(&rec.content_sha256).map_err(|e| e.to_string())?;
                bw.write_u32(rec.path_str_offset).map_err(|e| e.to_string())?;
                bw.write_u64(rec.file_size_bytes).map_err(|e| e.to_string())?;
                bw.write_u64(rec.mtime_ns).map_err(|e| e.to_string())?;
                bw.write_u32(rec.first_token_id).map_err(|e| e.to_string())?;
                bw.write_u32(rec.file_token_count).map_err(|e| e.to_string())?;
            }

            // Write File Paths
            bw.write_bytes(&path_bytes_sec).map_err(|e| e.to_string())?;
            bw.align_to(8).map_err(|e| e.to_string())?;

            // ── Section 4: TOKEN TABLE / Forward Index (n_tok * 16 B) ──
            for rec in &artifact.token_records {
                bw.write_u64(rec.sort_key).map_err(|e| e.to_string())?;
                bw.write_u32(rec.text_id).map_err(|e| e.to_string())?;
                bw.write_u16(rec.len).map_err(|e| e.to_string())?;
                bw.write_u8(rec.token_type).map_err(|e| e.to_string())?;
                bw.write_u8(rec._padding).map_err(|e| e.to_string())?;
            }

            // ── Section 5: ENTRY MAP / Backward Index (n_tok * 16 B) ──
            for entry in &artifact.token_entries {
                bw.write_u64(entry.sort_key).map_err(|e| e.to_string())?;
                bw.write_u32(entry.text_id).map_err(|e| e.to_string())?;
                bw.write_u16(entry.len).map_err(|e| e.to_string())?;
                bw.write_u8(entry.token_type).map_err(|e| e.to_string())?;
                bw.write_u8(entry._padding).map_err(|e| e.to_string())?;
            }

            // ── Section 6: STR HEADERS & Section 7: STR STORAGE ──
            let mut str_headers = Vec::new();
            let string_count = artifact.interner.count();
            for text_id in 0..string_count {
                let text = artifact.interner.lookup_text(text_id);
                let hash = fnv1a_64(text);
                let offset = artifact.interner.get_offsets()[text_id as usize];
                str_headers.push((hash, offset));
            }
            str_headers.sort_unstable_by_key(|&(h, _)| h);

            for (hash, offset) in str_headers {
                bw.write_u64(hash).map_err(|e| e.to_string())?;
                bw.write_u32(offset).map_err(|e| e.to_string())?;
            }

            bw.write_bytes(artifact.interner.get_storage_bytes())
                .map_err(|e| e.to_string())?;
        }

        // ── Section 8: CHECKSUM (8 bytes) ──
        let checksum = crc64_ecma(&buffer);
        buffer.extend_from_slice(&checksum.to_le_bytes());

        let mut file = BufWriter::new(File::create(out_path).map_err(|e| e.to_string())?);
        file.write_all(&buffer).map_err(|e| e.to_string())?;
        file.flush().map_err(|e| e.to_string())?;

        Ok(())
    }

    pub fn read(in_path: &Path) -> Result<TokenCorpusArtifact, String> {
        let file = File::open(in_path).map_err(|e| e.to_string())?;
        let file_len = file.metadata().map_err(|e| e.to_string())?.len();
        if file_len < 72 {
            return Err("Invalid .tca file: smaller than 72 bytes".to_string());
        }

        let mut reader = BufReader::new(file);
        let mut buffer = Vec::new();
        reader.read_to_end(&mut buffer).map_err(|e| e.to_string())?;

        // Verify Checksum
        let payload_len = buffer.len() - 8;
        let stored_checksum = u64::from_le_bytes(
            buffer[payload_len..]
                .try_into()
                .map_err(|_| "Failed to read checksum")?,
        );
        let computed_checksum = crc64_ecma(&buffer[..payload_len]);
        if stored_checksum != computed_checksum {
            return Err(format!(
                "Checksum Mismatch: Stored 0x{:016X} != Computed 0x{:016X}",
                stored_checksum, computed_checksum
            ));
        }

        let mut br = BinaryReader::new(&buffer[..payload_len]);
        let magic = br.read_u64().map_err(|e| e.to_string())?;
        if magic != TCA_MAGIC {
            return Err(format!("Invalid Magic: 0x{:016X}", magic));
        }

        let version = br.read_u32().map_err(|e| e.to_string())?;
        if version != TCA_VERSION {
            return Err(format!("Unsupported Version: {}", version));
        }

        let token_count = br.read_u32().map_err(|e| e.to_string())? as usize;
        let file_count = br.read_u16().map_err(|e| e.to_string())? as usize;
        let string_count = br.read_u32().map_err(|e| e.to_string())?;
        let _flags = br.read_u16().map_err(|e| e.to_string())?;
        let _hash = br.read_exact_bytes(32).map_err(|e| e.to_string())?;
        let _mtime = br.read_u64().map_err(|e| e.to_string())?;

        let mut file_records = Vec::with_capacity(file_count);
        for _ in 0..file_count {
            let file_id = br.read_u16().map_err(|e| e.to_string())?;
            let language_id = br.read_u8().map_err(|e| e.to_string())?;
            let flags = br.read_u8().map_err(|e| e.to_string())?;
            let sha_bytes = br.read_exact_bytes(32).map_err(|e| e.to_string())?;
            let mut content_sha256 = [0u8; 32];
            content_sha256.copy_from_slice(&sha_bytes);

            let path_str_offset = br.read_u32().map_err(|e| e.to_string())?;
            let file_size_bytes = br.read_u64().map_err(|e| e.to_string())?;
            let mtime_ns = br.read_u64().map_err(|e| e.to_string())?;
            let first_token_id = br.read_u32().map_err(|e| e.to_string())?;
            let file_token_count = br.read_u32().map_err(|e| e.to_string())?;

            file_records.push(SourceFileRecord {
                file_id,
                language_id,
                flags,
                content_sha256,
                path_str_offset,
                file_size_bytes,
                mtime_ns,
                first_token_id,
                file_token_count,
            });
        }

        // Read path bytes and skip 8-byte alignment
        for _ in 0..file_count {
            let len = br.read_u16().map_err(|e| e.to_string())? as usize;
            br.read_exact_bytes(len).map_err(|e| e.to_string())?;
        }
        br.skip_alignment(8).map_err(|e| e.to_string())?;

        let mut token_records = Vec::with_capacity(token_count);
        for _ in 0..token_count {
            let sort_key = br.read_u64().map_err(|e| e.to_string())?;
            let text_id = br.read_u32().map_err(|e| e.to_string())?;
            let len = br.read_u16().map_err(|e| e.to_string())?;
            let token_type = br.read_u8().map_err(|e| e.to_string())?;
            let _padding = br.read_u8().map_err(|e| e.to_string())?;

            token_records.push(TokenRecord {
                sort_key,
                text_id,
                len,
                token_type,
                _padding,
            });
        }

        let mut token_entries = Vec::with_capacity(token_count);
        for _ in 0..token_count {
            let sort_key = br.read_u64().map_err(|e| e.to_string())?;
            let text_id = br.read_u32().map_err(|e| e.to_string())?;
            let len = br.read_u16().map_err(|e| e.to_string())?;
            let token_type = br.read_u8().map_err(|e| e.to_string())?;
            let _padding = br.read_u8().map_err(|e| e.to_string())?;

            token_entries.push(TokenEntry {
                sort_key,
                text_id,
                len,
                token_type,
                _padding,
            });
        }

        // String headers & storage
        for _ in 0..string_count {
            br.read_u64().map_err(|e| e.to_string())?;
            br.read_u32().map_err(|e| e.to_string())?;
        }

        let mut interner = StringInterner::with_capacity(string_count as usize);
        for _ in 0..string_count {
            let len = br.read_u16().map_err(|e| e.to_string())? as usize;
            let bytes = br.read_exact_bytes(len).map_err(|e| e.to_string())?;
            interner.intern(&bytes);
        }

        Ok(TokenCorpusArtifact {
            file_records,
            token_records,
            token_entries,
            interner,
        })
    }
}
