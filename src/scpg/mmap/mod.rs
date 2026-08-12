//! MemoryMappedSCPG — read-only memory mapped interface for .scpg files (§10.4).

use std::fs;
use std::io::{self, Result as IoResult};
use std::path::Path;

use crate::scpg::types::*;

#[derive(Debug)]
pub struct MemoryMappedSCPG {
    pub header: SCPGHeader,
    pub directory: Vec<SectionDirectoryEntry>,
    bytes: Vec<u8>,
}

impl MemoryMappedSCPG {
    pub fn open(path: &Path) -> IoResult<Self> {
        let bytes = fs::read(path)?;
        if bytes.len() < SCPG_HEADER_SIZE + (SCPG_SECTION_COUNT as usize * SCPG_DIR_ENTRY_SIZE) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "SCPG file size smaller than header + directory",
            ));
        }

        let magic = u64::from_le_bytes(bytes[0..8].try_into().unwrap());
        if magic != SCPG_MAGIC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("SCPG magic mismatch: got 0x{:016X}", magic),
            ));
        }

        let format_version = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
        let section_count = u32::from_le_bytes(bytes[12..16].try_into().unwrap());
        let mut source_hash = [0u8; 32];
        source_hash.copy_from_slice(&bytes[16..48]);
        let creation_ts_ns = u64::from_le_bytes(bytes[48..56].try_into().unwrap());
        let scpg_hash = u32::from_le_bytes(bytes[56..60].try_into().unwrap());
        let language_count = u32::from_le_bytes(bytes[60..64].try_into().unwrap());

        let header = SCPGHeader {
            magic,
            format_version,
            section_count,
            source_hash,
            creation_ts_ns,
            scpg_hash,
            language_count,
        };

        let mut directory = Vec::with_capacity(section_count as usize);
        let mut dir_offset = SCPG_HEADER_SIZE;
        for _ in 0..section_count {
            let section_type =
                u32::from_le_bytes(bytes[dir_offset..dir_offset + 4].try_into().unwrap());
            let byte_offset =
                u64::from_le_bytes(bytes[dir_offset + 4..dir_offset + 12].try_into().unwrap());
            let byte_length =
                u64::from_le_bytes(bytes[dir_offset + 12..dir_offset + 20].try_into().unwrap());
            let crc =
                u32::from_le_bytes(bytes[dir_offset + 20..dir_offset + 24].try_into().unwrap());

            directory.push(SectionDirectoryEntry {
                section_type,
                byte_offset,
                byte_length,
                crc32: crc,
            });

            dir_offset += SCPG_DIR_ENTRY_SIZE;
        }

        Ok(Self {
            header,
            directory,
            bytes,
        })
    }

    pub fn get_section(&self, sec_type: SCPGSectionType) -> Option<&[u8]> {
        let entry = self
            .directory
            .iter()
            .find(|e| e.section_type == sec_type as u32)?;
        let start = entry.byte_offset as usize;
        let end = start + entry.byte_length as usize;
        if end <= self.bytes.len() {
            Some(&self.bytes[start..end])
        } else {
            None
        }
    }
}
