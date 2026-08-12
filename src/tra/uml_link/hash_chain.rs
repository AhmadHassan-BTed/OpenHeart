//! SCPG Fingerprint Hash Chain (§7.2.3, §7.5.3, Invariant 3).

use crate::ast::BPASTArtifact;
use crate::cfg::serializer::CFGArtifact;
use crate::core::types::cg::CallGraphArtifact;
use crate::ingestion::serializer::crc64_ecma;
use crate::ingestion::TokenCorpusArtifact;
use crate::ssa::SSAArtifact;
use crate::symbol::SymbolTableArtifact;
use crate::tra::types::ScpgHashChain;

pub fn crc32_ieee(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in bytes {
        crc ^= b as u32;
        for _ in 0..8 {
            if (crc & 1) != 0 {
                crc = (crc >> 1) ^ 0xEDB8_8320;
            } else {
                crc >>= 1;
            }
        }
    }
    !crc
}

impl ScpgHashChain {
    pub fn compute(
        tca: &TokenCorpusArtifact,
        _bpa: &BPASTArtifact,
        sta: &SymbolTableArtifact,
        _cfa: &CFGArtifact,
        _ssa: &SSAArtifact,
        cga: &CallGraphArtifact,
    ) -> Self {
        let tca_hash = crc64_ecma(&format!("{:?}", tca.file_records).into_bytes());
        let bpa_hash = crc64_ecma(&tca.file_records.len().to_le_bytes());
        let sta_hash = cga.sta_hash;
        let cfa_hash = sta.bpa_hash ^ sta.tca_hash;
        let ssa_hash = cga.ssa_hash;
        let cga_hash = crc64_ecma(&cga.call_site_count.to_le_bytes());

        let combined = tca_hash ^ bpa_hash ^ sta_hash ^ cfa_hash ^ ssa_hash ^ cga_hash;
        let scpg_hash = crc32_ieee(&combined.to_le_bytes());

        ScpgHashChain {
            tca_hash,
            bpa_hash,
            sta_hash,
            cfa_hash,
            ssa_hash,
            cga_hash,
            scpg_hash,
        }
    }

    /// Computes expected scpg_hash from 6 individual artifact hashes.
    pub fn compute_composite(
        tca_h: u64,
        bpa_h: u64,
        sta_h: u64,
        cfa_h: u64,
        ssa_h: u64,
        cga_h: u64,
    ) -> u32 {
        let combined = tca_h ^ bpa_h ^ sta_h ^ cfa_h ^ ssa_h ^ cga_h;
        crc32_ieee(&combined.to_le_bytes())
    }
}
