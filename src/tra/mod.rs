//! Phase 7: Traceability Index Construction (§7.1, §7.5.1).

pub mod backward;
pub mod builder;
pub mod delta;
pub mod forward;
pub mod serializer;
pub mod types;
pub mod uml_link;

pub use builder::TraceabilityArtifactBuilder;
pub use delta::{DeltaApplicator, StaleDetector, TraceabilityDelta};
pub use serializer::TraceabilitySerializer;
pub use types::*;

use std::path::Path;

use crate::ast::BPASTArtifact;
use crate::cfg::serializer::CFGArtifact;
use crate::core::logger::log_info;
use crate::core::types::cg::CallGraphArtifact;
use crate::ingestion::TokenCorpusArtifact;
use crate::ssa::SSAArtifact;
use crate::symbol::SymbolTableArtifact;

pub struct Phase7Stage;

impl Phase7Stage {
    pub fn run(
        tca: &TokenCorpusArtifact,
        bpa: &BPASTArtifact,
        sta: &SymbolTableArtifact,
        cfa: &CFGArtifact,
        ssa: &SSAArtifact,
        cga: &CallGraphArtifact,
        out: &Path,
    ) -> TraceabilityArtifact {
        log_info("══► Starting Stage: Phase 7: Traceability Index Construction...");

        let artifact = TraceabilityArtifactBuilder::build(tca, bpa, sta, cfa, ssa, cga);

        if let Err(e) = TraceabilitySerializer::write(&artifact, out) {
            panic!("Failed to write TraceabilityArtifact (.tra): {}", e);
        }

        log_info(&format!(
            "Phase 7 Complete: Built Traceability Index (.tra) with {} UMLLinks, {} Symbol Spans, composite hash 0x{:08X}.",
            artifact.uml_links.len(),
            artifact.sym_span.len(),
            artifact.hashes.scpg_hash
        ));

        artifact
    }
}
