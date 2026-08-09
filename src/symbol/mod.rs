//! Phase 3: Symbol Table & Type Hierarchy Construction
//! Orchestrates the 5 resolution passes: Discovery, Imports, Types, Members, Hierarchy.

pub mod adapter;
pub mod builder;
pub mod passes;
pub mod qual_name_table;
pub mod scope_graph;
pub mod serializer;
pub mod std_library;
pub mod uml_meta;

pub use adapter::*;
pub use builder::*;
pub use passes::*;
pub use qual_name_table::*;
pub use scope_graph::*;
pub use serializer::*;
pub use std_library::*;
pub use uml_meta::*;

use crate::ast::BPASTArtifact;
use crate::core::logger::{log_debug, log_info, PhaseTimer};
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

        let adapter = JavaSemanticAdapter::new();
        let mut builder = SymbolTableBuilder::new();

        // Pass 1: Declaration Discovery
        log_info("Executing Pass 1: Declaration Discovery DFS over BP AST...");
        Pass1Discovery::run(bpa, tca, &adapter, &mut builder);
        log_debug(&format!(
            "Pass 1 Discovered: {} symbols across {} scope regions",
            builder.symbols.len(),
            builder.scope_graph.scope_count()
        ));

        // Pass 2: Import Resolution
        log_info("Executing Pass 2: Import Map & Scope Import Edge Construction...");
        Pass2Imports::run(bpa, tca, &mut builder);

        // Pass 3: Type Reference Resolution
        log_info("Executing Pass 3: Type Reference Resolution & Scope Graph BFS...");
        Pass3Types::run(bpa, tca, &adapter, &mut builder);
        log_debug(&format!(
            "Pass 3 Resolved: {} type reference resolutions created",
            builder.type_ref_resolutions.len()
        ));

        // Pass 4: Member Declaration Type Resolution
        log_info("Executing Pass 4: Member Declaration Type Assignment...");
        Pass4Members::run(bpa, &mut builder);

        // Pass 5: Type Hierarchy Construction
        log_info("Executing Pass 5: Type Hierarchy CSR & UML Association Detection...");
        Pass5Hierarchy::run(bpa, &mut builder);
        log_debug(&format!(
            "Pass 5 Derived: {} TH edges, {} UML association records",
            builder.th_edges.len(),
            builder.associations.len()
        ));

        // Enforce Invariants 1-5
        log_info("Asserting Phase 3 Invariants 1-5...");
        builder.verify_invariants(bpa, tca).map_err(|e| {
            Error::new(
                ErrorKind::InvalidData,
                format!("Phase 3 Invariant Violation: {}", e),
            )
        })?;

        // Build binary artifact
        let artifact = SymbolTableArtifact::build(&builder, bpa_bytes, tca_bytes);

        log_info(&format!(
            "Phase 3 Complete: Symbol Table constructed with {} symbols, {} scopes, {} TH edges.",
            artifact.symbol_count, artifact.scope_count, artifact.th_edge_count
        ));

        Ok(artifact)
    }
}
