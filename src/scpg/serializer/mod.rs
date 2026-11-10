//! SCPGSerializer — writes all 9 prior artifacts as one unified .scpg binary (§10.2, §10.6.1).
//!
//! Each of the 11 sections holds the full columnar encoding of one snapshot index (see
//! `query::snapshot`), aligned to 64 B (4 KiB once it spans a page) and protected by a
//! CRC-64/NVME checksum. The header carries the SHA-256 content digest, a header CRC and a
//! directory CRC. Files are written through `core::io::fs::write_atomic` (temporary sibling,
//! fsync, rename), so readers never observe a partially written file and existing memory
//! maps keep the previous contents.

pub mod codec;
pub mod integrity;
pub mod layout;

use std::collections::BTreeSet;
use std::io::{self, Result as IoResult};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::ast::BPASTArtifact;
use crate::cfg::serializer::CFGArtifact;
use crate::core::io::fs::write_atomic;
use crate::core::types::cg::CallGraphArtifact;
use crate::ingestion::TokenCorpusArtifact;
use crate::psa::types::PathSummaryArtifact;
use crate::scpg::query::snapshot::{EncodedSections, ScpgSnapshot};
use crate::scpg::types::*;
use crate::ssa::serializer::SSAArtifact;
use crate::symbol::SymbolTableArtifact;
use crate::tra::types::TraceabilityArtifact;
use crate::uma::types::UMLMetadataArtifact;

use self::integrity::{crc64_nvme, short_hash};
use self::layout::{align_up, SectionLayoutPlanner};

/// Outcome of writing one `.scpg` file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WriteReport {
    pub digest: ScpgDigest,
    pub scpg_hash: u32,
    pub file_length: u64,
    pub section_count: u32,
}

pub struct SCPGSerializer;

impl SCPGSerializer {
    /// Builds the snapshot of the nine artifacts and writes it to `out_path`.
    /// Returns the snapshot's short hash (first four bytes of the content digest).
    #[allow(clippy::too_many_arguments)]
    pub fn write(
        tca: &TokenCorpusArtifact,
        bpa: &BPASTArtifact,
        sta: &SymbolTableArtifact,
        cfa: &CFGArtifact,
        ssa: &SSAArtifact,
        cga: &CallGraphArtifact,
        tra: &TraceabilityArtifact,
        uma: &UMLMetadataArtifact,
        psa: &PathSummaryArtifact,
        out_path: &Path,
    ) -> IoResult<u32> {
        let (snapshot, encoded) = ScpgSnapshot::build(tca, bpa, sta, cfa, ssa, cga, tra, uma, psa);
        let report = Self::write_encoded(&encoded, snapshot.source_hash, language_count(tca), out_path)?;
        Ok(report.scpg_hash)
    }

    /// The complete `.scpg` image of pre-encoded sections, stamped with
    /// [`creation_timestamp_ns`]: exactly the bytes [`SCPGSerializer::write_encoded`]
    /// writes, for callers that keep the image in memory.
    pub fn encode(
        encoded: &EncodedSections,
        source_hash: [u8; 32],
        language_count: u32,
    ) -> IoResult<Vec<u8>> {
        Self::assemble(encoded, source_hash, language_count, creation_timestamp_ns())
    }

    /// Writes pre-encoded sections atomically to `out_path`.
    pub fn write_encoded(
        encoded: &EncodedSections,
        source_hash: [u8; 32],
        language_count: u32,
        out_path: &Path,
    ) -> IoResult<WriteReport> {
        let bytes = Self::encode(encoded, source_hash, language_count)?;
        write_atomic(out_path, &bytes)?;
        Ok(WriteReport {
            digest: encoded.digest,
            scpg_hash: short_hash(&encoded.digest),
            file_length: bytes.len() as u64,
            section_count: encoded.sections.len() as u32,
        })
    }

    /// Lays out header, directory and aligned sections into one buffer.
    pub fn assemble(
        encoded: &EncodedSections,
        source_hash: [u8; 32],
        language_count: u32,
        creation_ts_ns: u64,
    ) -> IoResult<Vec<u8>> {
        let count = encoded.sections.len();
        if count as u64 > SCPG_MAX_SECTION_COUNT as u64 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{} sections exceed the format maximum {}", count, SCPG_MAX_SECTION_COUNT),
            ));
        }
        let dir_end = SCPG_HEADER_SIZE + count * SCPG_DIR_ENTRY_SIZE;
        let overflow = || io::Error::new(io::ErrorKind::InvalidInput, "section layout overflows u64");

        let mut entries = Vec::with_capacity(count);
        let mut offset = dir_end as u64;
        for (ty, payload) in &encoded.sections {
            let len = payload.len() as u64;
            let start = align_up(offset, SectionLayoutPlanner::alignment_for(len)).ok_or_else(overflow)?;
            entries.push(SectionDirectoryEntry {
                section_type: *ty as u32,
                codec: SectionCodec::Raw as u32,
                byte_offset: start,
                byte_length: len,
                checksum: crc64_nvme(payload),
            });
            offset = start.checked_add(len).ok_or_else(overflow)?;
        }
        let file_len = usize::try_from(offset).map_err(|_| overflow())?;

        let mut buf = vec![0u8; file_len];
        for ((_, payload), entry) in encoded.sections.iter().zip(&entries) {
            let start = entry.byte_offset as usize;
            buf[start..start + payload.len()].copy_from_slice(payload);
        }
        for (i, entry) in entries.iter().enumerate() {
            let at = SCPG_HEADER_SIZE + i * SCPG_DIR_ENTRY_SIZE;
            buf[at..at + 4].copy_from_slice(&entry.section_type.to_le_bytes());
            buf[at + 4..at + 8].copy_from_slice(&entry.codec.to_le_bytes());
            buf[at + 8..at + 16].copy_from_slice(&entry.byte_offset.to_le_bytes());
            buf[at + 16..at + 24].copy_from_slice(&entry.byte_length.to_le_bytes());
            buf[at + 24..at + 32].copy_from_slice(&entry.checksum.to_le_bytes());
            // bytes [at + 32, at + 40) stay zero (reserved)
        }
        let directory_crc = crc64_nvme(&buf[SCPG_HEADER_SIZE..dir_end]);

        let mut hdr = [0u8; SCPG_HEADER_SIZE];
        hdr[0..8].copy_from_slice(&SCPG_MAGIC.to_le_bytes());
        hdr[8..12].copy_from_slice(&SCPG_FORMAT_VERSION.to_le_bytes());
        hdr[12..16].copy_from_slice(&(count as u32).to_le_bytes());
        hdr[16..48].copy_from_slice(&source_hash);
        hdr[48..80].copy_from_slice(&encoded.digest);
        hdr[80..88].copy_from_slice(&creation_ts_ns.to_le_bytes());
        hdr[88..92].copy_from_slice(&short_hash(&encoded.digest).to_le_bytes());
        hdr[92..96].copy_from_slice(&language_count.to_le_bytes());
        // [96, 104): flags + reserved, zero
        hdr[104..112].copy_from_slice(&offset.to_le_bytes());
        let header_crc = crc64_nvme(&hdr[..SCPG_HEADER_CRC_OFFSET]);
        hdr[112..120].copy_from_slice(&header_crc.to_le_bytes());
        hdr[120..128].copy_from_slice(&directory_crc.to_le_bytes());
        buf[..SCPG_HEADER_SIZE].copy_from_slice(&hdr);
        Ok(buf)
    }
}

/// Distinct source languages in the corpus.
pub fn language_count(tca: &TokenCorpusArtifact) -> u32 {
    tca.file_records
        .iter()
        .map(|f| f.language_id)
        .collect::<BTreeSet<_>>()
        .len() as u32
}

/// Creation time for the header: `SOURCE_DATE_EPOCH` (seconds) when set, for reproducible
/// builds, otherwise the current time. The timestamp is never part of the content digest.
pub fn creation_timestamp_ns() -> u64 {
    if let Ok(epoch) = std::env::var("SOURCE_DATE_EPOCH") {
        if let Ok(secs) = epoch.trim().parse::<u64>() {
            return secs.saturating_mul(1_000_000_000);
        }
    }
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos().min(u64::MAX as u128) as u64)
        .unwrap_or(0)
}
