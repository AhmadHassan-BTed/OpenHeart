//! Phase 10: SCPG Unified Binary Serialization & Production Engine Bootstrap (§10.1, §10.6.1).
//!
//! **Phase Mandate:** Merges all 9 prior SCPG artifacts (`.tca`, `.bpa`, `.sta`,
//! `.cfa`, `.ssa`, `.cga`, `.tra`, `.uma`, `.psa`) into one query-ready snapshot and writes
//! it as a single memory-mappable `.scpg` binary ordered hot -> warm -> cold for OS page
//! cache locality. Every section carries the full columnar data of one snapshot index, so a
//! `.scpg` file can be reopened and queried without re-running phases 1-9. Bootstraps the
//! query engine and exposes the production public API.

pub mod api;
pub mod diagram;
pub mod incremental;
pub mod mmap;
pub mod query;
pub mod serializer;
pub mod types;

pub use api::{EngineBuilder, OpenHeartEngine};
pub use mmap::MemoryMappedSCPG;
pub use query::{QueryEngine, ScpgSnapshot};
pub use serializer::SCPGSerializer;
pub use types::{SCPGHeader, SCPGSectionType, ScpgFormatError, SCPG_MAGIC};

use std::io::Result as IoResult;
use std::path::Path;
use std::sync::Arc;

use crate::ast::BPASTArtifact;
use crate::cfg::serializer::CFGArtifact;
use crate::core::logger::log_info;
use crate::core::types::cg::CallGraphArtifact;
use crate::ingestion::TokenCorpusArtifact;
use crate::psa::types::PathSummaryArtifact;
use crate::scpg::query::cache::DEFAULT_CACHE_CAPACITY;
use crate::scpg::serializer::integrity::digest_hex;
use crate::scpg::serializer::language_count;
use crate::ssa::serializer::SSAArtifact;
use crate::symbol::SymbolTableArtifact;
use crate::tra::types::TraceabilityArtifact;
use crate::uma::types::UMLMetadataArtifact;

/// Phase 10 driver: snapshot construction, `.scpg` serialization, engine bootstrap.
pub struct Phase10Stage;

impl Phase10Stage {
    /// Execute Phase 10: SCPG binary serialization & OpenHeartEngine bootstrap.
    ///
    /// Panics if the `.scpg` file cannot be written; use `try_run` to handle the error.
    #[allow(clippy::too_many_arguments)]
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
        Self::try_run(tca, bpa, sta, cfa, ssa, cga, tra, uma, psa, out_path)
            .expect("Phase 10: Failed to write unified .scpg binary file")
    }

    /// Fallible form of `run`.
    #[allow(clippy::too_many_arguments)]
    pub fn try_run(
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
    ) -> IoResult<OpenHeartEngine> {
        log_info("══► Starting Stage: Phase 10: SCPG Unified Binary & Engine Bootstrap...");

        let (snapshot, encoded) = ScpgSnapshot::build(tca, bpa, sta, cfa, ssa, cga, tra, uma, psa);
        let report = SCPGSerializer::write_encoded(
            &encoded,
            snapshot.source_hash,
            language_count(tca),
            out_path,
        )?;

        let stats = snapshot.stats();
        log_info(&format!(
            "  Phase 10: Wrote {} sections ({} bytes) to {} (scpg_hash: 0x{:08X}, digest: {}).",
            report.section_count,
            report.file_length,
            out_path.display(),
            report.scpg_hash,
            digest_hex(&report.digest)
        ));
        log_info(&format!(
            "  Phase 10: Snapshot holds {} symbols, {} call edges, {} SDG nodes / {} edges, {} navigable entities, {} path summaries.",
            stats.symbols, stats.call_edges, stats.sdg_nodes, stats.sdg_edges, stats.navigable_entities, stats.path_summaries
        ));

        // Queries are served from the in-memory snapshot. The written file is deliberately not
        // kept mapped: on Windows an open mapping would block the next atomic replacement of
        // the same output path (e.g. a re-analysis by a long-running server). Reopen it with
        // `OpenHeartEngine::open` to verify it or to serve queries from disk.
        let engine = OpenHeartEngine::from_parts(Arc::new(snapshot), DEFAULT_CACHE_CAPACITY, None);

        log_info("Phase 10 Complete: OpenHeartEngine bootstrapped over the unified snapshot.");

        Ok(engine)
    }

    /// Phase 10 without a file system (the WebAssembly build and other in-memory runs).
    ///
    /// The snapshot is built exactly as by [`Phase10Stage::try_run`] and the `.scpg` image
    /// is assembled in memory instead of being written. The engine is backed by that owned
    /// image (never a memory mapping), so whole-image verification and the embedded CFG
    /// section behave as for an engine reopened from disk.
    #[allow(clippy::too_many_arguments)]
    pub fn try_run_in_memory(
        tca: &TokenCorpusArtifact,
        bpa: &BPASTArtifact,
        sta: &SymbolTableArtifact,
        cfa: &CFGArtifact,
        ssa: &SSAArtifact,
        cga: &CallGraphArtifact,
        tra: &TraceabilityArtifact,
        uma: &UMLMetadataArtifact,
        psa: &PathSummaryArtifact,
    ) -> IoResult<OpenHeartEngine> {
        log_info(
            "══► Starting Stage: Phase 10: SCPG Unified Binary & Engine Bootstrap (in memory)...",
        );

        let (snapshot, encoded) = ScpgSnapshot::build(tca, bpa, sta, cfa, ssa, cga, tra, uma, psa);
        let bytes = SCPGSerializer::encode(&encoded, snapshot.source_hash, language_count(tca))?;
        drop(encoded);
        let image_length = bytes.len();
        let image = MemoryMappedSCPG::from_bytes(bytes)?;

        let stats = snapshot.stats();
        log_info(&format!(
            "  Phase 10: Assembled {} sections ({} bytes) in memory (scpg_hash: 0x{:08X}, digest: {}).",
            image.directory.len(),
            image_length,
            image.header.scpg_hash,
            digest_hex(&image.header.content_digest)
        ));
        log_info(&format!(
            "  Phase 10: Snapshot holds {} symbols, {} call edges, {} SDG nodes / {} edges, {} navigable entities, {} path summaries.",
            stats.symbols, stats.call_edges, stats.sdg_nodes, stats.sdg_edges, stats.navigable_entities, stats.path_summaries
        ));

        let engine = OpenHeartEngine::from_parts(
            Arc::new(snapshot),
            DEFAULT_CACHE_CAPACITY,
            Some(Arc::new(image)),
        );

        log_info("Phase 10 Complete: OpenHeartEngine bootstrapped over the in-memory snapshot.");

        Ok(engine)
    }
}
