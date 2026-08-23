//! Phase 9: UML Semantic Metadata Extraction (§9.1, §9.3).
//!
//! **Phase Mandate:** Translates all eight prior SCPG artifacts (`.tca`, `.bpa`, `.sta`,
//! `.cfa`, `.ssa`, `.cga`, `.tra`, `.psa`) into semantic metadata records driving
//! all 14 UML diagram types.
//!
//! **Output:** `UMLMetadataArtifact (.uma)`.

pub mod actor_identification;
pub mod behavioral;
pub mod builder;
pub mod label_extraction;
pub mod patterns;
pub mod serializer;
pub mod structural;
pub mod types;
pub mod visitor;

pub use builder::UMABuilder;
pub use serializer::UMASerializer;
pub use types::{UMLMetadataArtifact, UMA_MAGIC};
pub use visitor::{ArchitecturalMetricsVisitor, AttackSurfaceVisitor, UMAVisitor, VisitableUMA};

use std::path::Path;

use crate::ast::BPASTArtifact;
use crate::cfg::serializer::CFGArtifact;
use crate::core::logger::log_info;
use crate::core::types::cg::CallGraphArtifact;
use crate::ingestion::serializer::crc64_ecma;
use crate::ingestion::TokenCorpusArtifact;
use crate::psa::types::PathSummaryArtifact;
use crate::ssa::serializer::SSAArtifact;
use crate::symbol::SymbolTableArtifact;
use crate::tra::types::TraceabilityArtifact;

use self::behavioral::BehavioralExtractor;
use self::patterns::PatternDetector;
use self::structural::StructuralExtractor;

pub struct Phase9Stage;

impl Phase9Stage {
    /// Execute Phase 9 UML Semantic Metadata Extraction across all 14 diagram types.
    pub fn run(
        tca: &TokenCorpusArtifact,
        bpa: &BPASTArtifact,
        sta: &SymbolTableArtifact,
        cfa: &CFGArtifact,
        ssa: &SSAArtifact,
        cga: &CallGraphArtifact,
        tra: &TraceabilityArtifact,
        psa: &PathSummaryArtifact,
        tra_bytes: &[u8],
        out: &Path,
    ) -> UMLMetadataArtifact {
        log_info("══► Starting Stage: Phase 9: UML Semantic Metadata Extraction...");

        let tra_hash = crc64_ecma(tra_bytes);

        // ── Step 1: Extract Structural Diagram Records ──────────────────────
        let (mut classes, objects, packages, components) =
            StructuralExtractor::extract_all(sta, tca, psa, tra, ssa, cga);

        // ── Step 2: Extract Behavioral Diagram Records ──────────────────────
        let (activities, state_machines, sequences) =
            BehavioralExtractor::extract_all(sta, cfa, bpa, tca, psa, ssa, cga, tra);

        // ── Step 3: Run Design Pattern Detection Queries ────────────────────
        let patterns = PatternDetector::detect_all(sta, tca, cga, &mut classes);

        log_info(&format!(
            "  Phase 9: Extracted {} classes, {} activities, {} sequence scenarios, {} state machines, {} packages, {} components, {} design patterns.",
            classes.len(),
            activities.len(),
            sequences.len(),
            state_machines.len(),
            packages.len(),
            components.len(),
            patterns.len(),
        ));

        // ── Step 4: Aggregate into UMABuilder & Finalize ────────────────────
        let mut builder = UMABuilder::new(tra_hash);
        builder.set_classes(classes);
        builder.set_objects(objects);
        builder.set_packages(packages);
        builder.set_components(components);
        builder.set_activities(activities);
        builder.set_state_machines(state_machines);
        builder.set_sequences(sequences);
        builder.set_patterns(patterns);

        let artifact = builder.finalize();

        // ── Step 5: Write .uma to Disk ──────────────────────────────────────
        if let Err(e) = UMASerializer::write(&artifact, out) {
            panic!("Phase 9: Failed to write UMLMetadataArtifact (.uma): {}", e);
        }

        log_info(&format!(
            "Phase 9 Complete: Built UMLMetadataArtifact (.uma) with {} total class records.",
            artifact.classes.len()
        ));

        artifact
    }
}
