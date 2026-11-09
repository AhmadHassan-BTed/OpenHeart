//! Phase 3: Symbol Table & Type Hierarchy Construction
//! Orchestrates the resolution passes: Discovery, Imports, Types, Members, Hierarchy and
//! References, then checks the Phase 3 invariants and builds the `.sta` artifact.

pub mod adapter;
pub mod ast_index;
pub mod builder;
pub mod model;
pub mod passes;
pub mod qual_name_table;
pub mod scope_graph;
pub mod serializer;
pub mod std_library;
pub mod syntax;
pub mod uml_meta;

pub use adapter::*;
pub use builder::*;
pub use model::*;
pub use passes::*;
pub use qual_name_table::*;
pub use scope_graph::*;
pub use serializer::*;
pub use std_library::*;
pub use uml_meta::*;

use crate::ast::BPASTArtifact;
use crate::core::logger::{log_debug, log_info, PhaseTimer};
use crate::ingestion::serializer::crc64_ecma;
use crate::ingestion::TokenCorpusArtifact;
use std::io::{Error, ErrorKind, Result};

pub struct Phase3Stage;

impl Phase3Stage {
    pub fn run(
        tca: &TokenCorpusArtifact,
        bpa: &BPASTArtifact,
        tca_bytes: &[u8],
        bpa_bytes: &[u8],
    ) -> Result<SymbolTableArtifact> {
        let _timer = PhaseTimer::start("Phase 3: Symbol Table & Type Hierarchy Construction");

        let invariant_error = |e: String| {
            Error::new(
                ErrorKind::InvalidData,
                format!("Phase 3 Invariant Violation: {}", e),
            )
        };

        // Hash both inputs once: the TCA hash checks the BPA link (Invariant 4) and both are
        // embedded in the artifact.
        let tca_hash = crc64_ecma(tca_bytes);
        let bpa_hash = crc64_ecma(bpa_bytes);

        let adapter = JavaSemanticAdapter::new();
        let mut builder = SymbolTableBuilder::new();
        builder
            .verify_artifact_link(bpa, tca_hash)
            .map_err(invariant_error)?;

        let ast = ast_index::AstIndex::build(bpa, tca);

        // Pass 1: Declaration Discovery
        log_info("Executing Pass 1: Declaration Discovery over the BP AST and token corpus...");
        Pass1Discovery::run_with_index(bpa, tca, &ast, &adapter, &mut builder);
        log_debug(&format!(
            "Pass 1 Discovered: {} symbols across {} scope regions in {} compilation units",
            builder.symbols.len(),
            builder.scope_graph.scope_count(),
            builder.units.len()
        ));

        // Pass 2: Import Resolution
        log_info("Executing Pass 2: Import declarations per compilation unit...");
        Pass2Imports::run(bpa, tca, &mut builder);

        // Pass 3: Type Reference Resolution
        log_info("Executing Pass 3: Type reference resolution (JLS 6.4.1 / 7.5 order)...");
        Pass3Types::run(bpa, tca, &adapter, &mut builder);

        // Pass 4: Member Declaration Type Resolution
        log_info("Executing Pass 4: Declared types into the TypeRef table...");
        Pass4Members::run(bpa, tca, &ast, &adapter, &mut builder);

        // Pass 5: Type Hierarchy Construction
        log_info("Executing Pass 5: Type hierarchy and UML association inference...");
        Pass5Hierarchy::run(bpa, tca, &ast, &adapter, &mut builder);

        // Pass 6: Reference Resolution
        log_info("Executing Pass 6: Expression names, field accesses, calls and creations...");
        Pass6References::run(bpa, tca, &ast, &adapter, &mut builder);
        log_debug(&format!(
            "Passes 3-6: {} type refs, {} TH edges, {} associations, {} occurrences, {} diagnostics",
            builder.type_refs.len(),
            builder.th_edges.len(),
            builder.associations.len(),
            builder.occurrences.len(),
            builder.diagnostics.len()
        ));

        // Finalization: containment chains, stable descriptors, import offsets, occurrences.
        builder.rebuild_child_chains();
        builder.assign_descriptors(tca);
        builder.scope_graph.assign_import_offsets();
        builder.finalize_occurrences();

        log_info("Asserting Phase 3 invariants...");
        builder
            .verify_invariants(bpa, tca)
            .map_err(invariant_error)?;

        let artifact = SymbolTableArtifact::from_builder(&builder, bpa_hash, tca_hash);

        log_info(&format!(
            "Phase 3 Complete: {} symbols, {} scopes, {} TH edges, {} occurrences, {} diagnostics.",
            artifact.symbol_count,
            artifact.scope_count,
            artifact.th_edge_count,
            artifact.occurrences.len(),
            artifact.diagnostics.len()
        ));

        Ok(artifact)
    }
}
