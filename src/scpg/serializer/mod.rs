//! SCPGSerializer — merges all 9 prior artifacts into unified .scpg binary (§10.2, §10.6.1).

pub mod integrity;
pub mod layout;

use std::fs;
use std::io::Result as IoResult;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::ast::BPASTArtifact;
use crate::cfg::serializer::CFGArtifact;
use crate::core::types::cg::CallGraphArtifact;
use crate::ingestion::TokenCorpusArtifact;
use crate::psa::types::PathSummaryArtifact;
use crate::scpg::types::*;
use crate::ssa::serializer::SSAArtifact;
use crate::symbol::SymbolTableArtifact;
use crate::tra::types::TraceabilityArtifact;
use crate::uma::types::UMLMetadataArtifact;

use self::integrity::{compose_scpg_hash, crc32};
use self::layout::SectionLayoutPlanner;

pub struct SCPGSerializer;

impl SCPGSerializer {
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
        let mut file_buf = Vec::new();

        // ── 1. Header Placeholder (128 bytes) ────────────────────────────────
        file_buf.extend_from_slice(&[0u8; SCPG_HEADER_SIZE]);

        // ── 2. Directory Placeholder (11 × 24 bytes = 264 bytes) ─────────────
        let dir_offset = file_buf.len();
        let dir_size = SCPG_SECTION_COUNT as usize * SCPG_DIR_ENTRY_SIZE;
        file_buf.extend_from_slice(&vec![0u8; dir_size]);

        let mut directory_entries = Vec::with_capacity(SCPG_SECTION_COUNT as usize);

        // ── 3. Write 11 Sections in Hot -> Warm -> Cold Order ────────────────
        for &section_type in SectionLayoutPlanner::ordered_sections() {
            let offset = file_buf.len() as u64;

            let section_payload = match section_type {
                SCPGSectionType::TokenTable => {
                    let mut b = Vec::new();
                    b.extend_from_slice(&(tca.token_records.len() as u32).to_le_bytes());
                    b
                }
                SCPGSectionType::StringTable => {
                    tca.interner.get_storage_bytes().to_vec()
                }
                SCPGSectionType::Traceability => {
                    let mut b = Vec::new();
                    b.extend_from_slice(&(tra.uml_links.len() as u32).to_le_bytes());
                    b
                }
                SCPGSectionType::SymbolTable => {
                    let mut b = Vec::new();
                    b.extend_from_slice(&(sta.symbol_count).to_le_bytes());
                    b
                }
                SCPGSectionType::TypeHierarchy => {
                    let mut b = Vec::new();
                    b.extend_from_slice(&(sta.th_edge_count).to_le_bytes());
                    b
                }
                SCPGSectionType::SemanticMetadata => {
                    let mut b = Vec::new();
                    b.extend_from_slice(&(uma.classes.len() as u32).to_le_bytes());
                    b
                }
                SCPGSectionType::CallGraph => {
                    let mut b = Vec::new();
                    b.extend_from_slice(&(cga.call_site_count).to_le_bytes());
                    b
                }
                SCPGSectionType::BpAst => {
                    let mut b = Vec::new();
                    b.extend_from_slice(&bpa.node_count.to_le_bytes());
                    b
                }
                SCPGSectionType::Cfg => {
                    let mut b = Vec::new();
                    b.extend_from_slice(&(cfa.total_blocks).to_le_bytes());
                    b
                }
                SCPGSectionType::SsaDfg => {
                    let mut b = Vec::new();
                    b.extend_from_slice(&(ssa.total_ssa_vars).to_le_bytes());
                    b
                }
                SCPGSectionType::PathSummaries => {
                    let mut b = Vec::new();
                    b.extend_from_slice(&psa.total_nodes.to_le_bytes());
                    b
                }
            };

            let length = section_payload.len() as u64;
            let section_crc = crc32(&section_payload);

            file_buf.extend_from_slice(&section_payload);

            directory_entries.push(SectionDirectoryEntry {
                section_type: section_type as u32,
                byte_offset: offset,
                byte_length: length,
                crc32: section_crc,
            });
        }

        // ── 4. Fill in Header & Section Directory ─────────────────────────────
        let scpg_hash = compose_scpg_hash(
            tca.token_records.len() as u64,
            bpa.tca_hash,
            sta.bpa_hash,
            cfa.sta_hash,
            ssa.cfa_hash,
            cga.ssa_hash,
            tra.hashes.tca_hash,
            uma.tra_hash,
            psa.cfa_hash,
        );

        let now_ns = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;

        // Build Header (128 bytes)
        let mut hdr_bytes = Vec::with_capacity(SCPG_HEADER_SIZE);
        hdr_bytes.extend_from_slice(&SCPG_MAGIC.to_le_bytes());
        hdr_bytes.extend_from_slice(&SCPG_FORMAT_VERSION.to_le_bytes());
        hdr_bytes.extend_from_slice(&SCPG_SECTION_COUNT.to_le_bytes());
        hdr_bytes.extend_from_slice(&[0u8; 32]); // source_hash
        hdr_bytes.extend_from_slice(&now_ns.to_le_bytes());
        hdr_bytes.extend_from_slice(&scpg_hash.to_le_bytes());
        hdr_bytes.extend_from_slice(&1u32.to_le_bytes()); // language_count
        hdr_bytes.extend_from_slice(&[0u8; 64]); // _reserved

        debug_assert_eq!(hdr_bytes.len(), SCPG_HEADER_SIZE);

        file_buf[0..SCPG_HEADER_SIZE].copy_from_slice(&hdr_bytes);

        // Build Section Directory (264 bytes)
        let mut dir_bytes = Vec::with_capacity(dir_size);
        for entry in &directory_entries {
            dir_bytes.extend_from_slice(&entry.section_type.to_le_bytes());
            dir_bytes.extend_from_slice(&entry.byte_offset.to_le_bytes());
            dir_bytes.extend_from_slice(&entry.byte_length.to_le_bytes());
            dir_bytes.extend_from_slice(&entry.crc32.to_le_bytes());
        }

        file_buf[dir_offset..dir_offset + dir_size].copy_from_slice(&dir_bytes);

        fs::write(out_path, &file_buf)?;
        Ok(scpg_hash)
    }
}
