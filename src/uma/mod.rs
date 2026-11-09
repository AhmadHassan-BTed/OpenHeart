//! Phase 9: UML Semantic Metadata Extraction (§9.1, §9.3).
//!
//! **Phase Mandate:** Translates the prior SCPG artifacts (`.tca`, `.bpa`, `.sta`,
//! `.cfa`, `.cga`, `.tra`, `.psa`) into semantic metadata records for the UML
//! views that the artifacts carry evidence for: class, object, package,
//! component, composite structure, activity, state machine, sequence,
//! communication and interaction overview. Timing and deployment views have no
//! source evidence and are not produced.
//!
//! **Determinism:** every record list is built in symbol-id, token or sorted-set
//! order, so identical inputs yield identical `.uma` bytes.
//!
//! **Output:** `UMLMetadataArtifact (.uma)`.

pub mod actor_identification;
pub mod behavioral;
pub mod bodies;
pub mod builder;
pub mod index;
pub mod label_extraction;
pub mod patterns;
pub mod serializer;
pub mod source;
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

use self::behavioral::{BehavioralExtractor, SequenceOptions};
use self::bodies::MemberBodies;
use self::index::UMAIndex;
use self::label_extraction::LabelTable;
use self::patterns::{PatternContext, PatternDetector};
use self::structural::{ClassExtractionOptions, StructuralExtractor};

/// Phase 9 configuration.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Phase9Options {
    /// Include types and entry points under test source roots.
    pub include_tests: bool,
    /// Bounds for sequence scenario construction.
    pub sequences: SequenceOptions,
}

pub struct Phase9Stage;

impl Phase9Stage {
    /// Execute Phase 9 with default options (test sources excluded).
    #[allow(clippy::too_many_arguments)]
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
        Self::run_with_options(
            tca,
            bpa,
            sta,
            cfa,
            ssa,
            cga,
            tra,
            psa,
            tra_bytes,
            out,
            &Phase9Options::default(),
        )
    }

    /// Execute Phase 9 UML Semantic Metadata Extraction.
    ///
    /// `_ssa` is accepted for pipeline compatibility; no Phase 9 extractor reads it,
    /// because SSA variables are keyed by AST nodes rather than symbols.
    #[allow(clippy::too_many_arguments)]
    pub fn run_with_options(
        tca: &TokenCorpusArtifact,
        bpa: &BPASTArtifact,
        sta: &SymbolTableArtifact,
        cfa: &CFGArtifact,
        _ssa: &SSAArtifact,
        cga: &CallGraphArtifact,
        tra: &TraceabilityArtifact,
        psa: &PathSummaryArtifact,
        tra_bytes: &[u8],
        out: &Path,
        options: &Phase9Options,
    ) -> UMLMetadataArtifact {
        log_info("══► Starting Stage: Phase 9: UML Semantic Metadata Extraction...");

        let tra_hash = crc64_ecma(tra_bytes);

        // ── Step 0: Shared indexes (built once) ─────────────────────────────
        let index = UMAIndex::build(sta, tca, cga, Some(tra));
        let bodies = MemberBodies::build(&index);
        let mut labels = LabelTable::new();
        if index.dropped_call_edges > 0 {
            log_info(&format!(
                "  Phase 9: ignored {} call edges whose callee name does not match the invoked identifier.",
                index.dropped_call_edges
            ));
        }

        // ── Step 1: Structural Diagram Records ──────────────────────────────
        let class_options = ClassExtractionOptions {
            include_tests: options.include_tests,
        };
        let mut structural =
            StructuralExtractor::extract_all(&index, &bodies, psa, cfa, &mut labels, class_options);
        let class_syms: Vec<u32> = structural.classes.iter().map(|c| c.sym_id).collect();

        // ── Step 2: Behavioral Diagram Records ──────────────────────────────
        let behavioral = BehavioralExtractor::extract_all(
            &index,
            &bodies,
            cfa,
            bpa,
            tca,
            psa,
            &mut labels,
            &class_syms,
            options.include_tests,
            options.sequences,
        );

        // ── Step 3: Design Pattern Detection ────────────────────────────────
        let ctx = PatternContext::new(&index, &bodies);
        let (patterns, instances) = PatternDetector::detect_all(&ctx, &mut structural.classes);

        log_info(&format!(
            "  Phase 9: Extracted {} classes, {} objects, {} activities, {} sequence scenarios, {} state machines, {} packages ({} dependencies), {} components, {} composite structures, {} pattern instances ({} participant rows).",
            structural.classes.len(),
            structural.objects.len(),
            behavioral.activities.len(),
            behavioral.sequences.len(),
            behavioral.state_machines.len(),
            structural.packages.len(),
            structural.package_dependencies.len(),
            structural.components.len(),
            structural.composite_structures.len(),
            instances.len(),
            patterns.len(),
        ));

        // ── Step 4: Aggregate into UMABuilder & Finalize ────────────────────
        let mut builder = UMABuilder::new(tra_hash);
        builder.set_structural(structural);
        builder.set_behavioral(behavioral);
        builder.set_patterns(patterns, instances);
        builder.set_labels(labels.into_map());

        let artifact = match builder.finalize() {
            Ok(a) => a,
            Err(e) => panic!("Phase 9: UML metadata invariant violated: {}", e),
        };

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
