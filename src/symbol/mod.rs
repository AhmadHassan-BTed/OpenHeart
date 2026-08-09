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
use crate::ingestion::TokenCorpusArtifact;
use std::io::{Error, ErrorKind, Result};

pub struct Phase3Stage;

impl Phase3Stage {
    /// Runs all 5 resolution passes in sequence:
    /// Pass 1: Declaration Discovery
    /// Pass 2: Import Resolution
    /// Pass 3: Type Reference Resolution
    /// Pass 4: Member Declaration Type Resolution
    /// Pass 5: Type Hierarchy Construction & Association Detection
    /// Verifies Invariants 1-5 and returns SymbolTableArtifact (.sta).
    pub fn run(
        tca: &TokenCorpusArtifact,
        bpa: &BPASTArtifact,
        tca_bytes: &[u8],
        bpa_bytes: &[u8],
    ) -> Result<SymbolTableArtifact> {
        let adapter = JavaSemanticAdapter::new();
        let mut builder = SymbolTableBuilder::new();

        // Pass 1: Declaration Discovery
        Pass1Discovery::run(bpa, tca, &adapter, &mut builder);

        // Pass 2: Import Resolution
        Pass2Imports::run(bpa, tca, &mut builder);

        // Pass 3: Type Reference Resolution
        Pass3Types::run(bpa, tca, &adapter, &mut builder);

        // Pass 4: Member Declaration Type Resolution
        Pass4Members::run(bpa, &mut builder);

        // Pass 5: Type Hierarchy Construction
        Pass5Hierarchy::run(bpa, &mut builder);

        // Enforce Invariants 1-5
        builder.verify_invariants(bpa, tca).map_err(|e| {
            Error::new(
                ErrorKind::InvalidData,
                format!("Phase 3 Invariant Violation: {}", e),
            )
        })?;

        // Build binary artifact
        Ok(SymbolTableArtifact::build(&builder, bpa_bytes, tca_bytes))
    }
}
