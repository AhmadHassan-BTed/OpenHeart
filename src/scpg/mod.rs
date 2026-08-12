//! Phase 10: SCPG Unified Binary Serialization & Production Engine Bootstrap (§10.1, §10.6.1).
//!
//! **Phase Mandate:** Merges all 9 prior SCPG artifacts (`.tca`, `.bpa`, `.sta`,
//! `.cfa`, `.ssa`, `.cga`, `.tra`, `.uma`, `.psa`) into a single, unified memory-mapped
//! `.scpg` binary file ordered hot -> warm -> cold for OS page cache optimization.
//! Bootstraps LRU query engine and exposes production public API.

pub mod api;
pub mod diagram;
pub mod incremental;
pub mod mmap;
pub mod query;
pub mod serializer;
pub mod types;

pub use api::{EngineBuilder, OpenHeartEngine};
pub use mmap::MemoryMappedSCPG;
pub use query::QueryEngine;
pub use serializer::SCPGSerializer;
pub use types::{SCPGHeader, SCPGSectionType, SCPG_MAGIC};

use std::path::Path;

use crate::ast::BPASTArtifact;
use crate::cfg::serializer::CFGArtifact;
use crate::core::logger::log_info;
use crate::core::types::cg::CallGraphArtifact;
use crate::ingestion::TokenCorpusArtifact;
use crate::psa::types::PathSummaryArtifact;
use crate::ssa::serializer::SSAArtifact;
use crate::symbol::SymbolTableArtifact;
use crate::tra::types::TraceabilityArtifact;
use crate::uma::types::UMLMetadataArtifact;

pub struct Phase10Stage;

impl Phase10Stage {
    /// Execute Phase 10: SCPG binary serialization & OpenHeartEngine bootstrap.
    pub fn run(
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
    ) -> OpenHeartEngine {
        log_info("══► Starting Stage: Phase 10: SCPG Unified Binary & Engine Bootstrap...");

        let scpg_hash =
            SCPGSerializer::write(tca, bpa, sta, cfa, ssa, cga, tra, uma, psa, out_path)
                .expect("Phase 10: Failed to write unified .scpg binary file");

        log_info(&format!(
            "  Phase 10: Merged all 9 artifacts into unified .scpg file (scpg_hash: 0x{:08X}).",
            scpg_hash
        ));

        let engine = OpenHeartEngine::new(scpg_hash);

        log_info("Phase 10 Complete: OpenHeartEngine production engine bootstrapped & ready.");

        engine
    }
}
