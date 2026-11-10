//! OpenHeartEngine — production entry point for all SCPG operations (§10.7).
//!
//! An engine wraps one immutable `ScpgSnapshot` (built in memory by Phase 10 or decoded from
//! a `.scpg` file) and a `QueryEngine` over it. Queries never take artifacts as arguments
//! and run concurrently (`&self`, no global lock). Incremental updates plan against the
//! current snapshot and commit a rebuilt one, which atomically switches the query engine.

pub mod builder;
pub mod export;

pub use builder::{EngineBuilder, VerifyPolicy};
pub use export::{export_ohb_json, ohb_json_schema, ExportInputs, OHB_JSON_SCHEMA_ID};

use std::io::Result as IoResult;
use std::path::Path;
use std::sync::Arc;

use crate::cfg::serializer::CFGArtifact;
use crate::scpg::diagram::export::json::JSONExporter;
use crate::scpg::diagram::export::plantuml::PlantUMLExporter;
use crate::scpg::diagram::export::xmi::XMIExporter;
use crate::scpg::diagram::DiagramGenerator;
use crate::scpg::incremental::{IncrementalEngine, RebuildPlan, SourceDelta, UpdateResult};
use crate::scpg::mmap::{MemoryMappedSCPG, VerificationReport};
use crate::scpg::query::cache::DEFAULT_CACHE_CAPACITY;
use crate::scpg::query::impact::{ImpactEntry, ImpactOptions};
use crate::scpg::query::lang::QueryOutput;
use crate::scpg::query::navigation::SourceLocation;
use crate::scpg::query::slice::ProgramSlice;
use crate::scpg::query::snapshot::{ScpgSnapshot, SnapshotStats};
use crate::scpg::query::{QueryEngine, QueryError};
use crate::scpg::serializer::integrity::digest_hex;
use crate::scpg::types::{ScpgDigest, ScpgFormatError};
use crate::uma::types::*;

/// Production query engine over one immutable SCPG snapshot.
pub struct OpenHeartEngine {
    /// Short hash of the snapshot (first four bytes of its SHA-256 content digest).
    pub scpg_hash: u32,
    query: Arc<QueryEngine>,
    incremental: IncrementalEngine,
    /// Backing `.scpg` file when the engine was opened from disk.
    pub mmap: Option<Arc<MemoryMappedSCPG>>,
    cache_capacity: usize,
}

impl OpenHeartEngine {
    /// An in-memory engine over `snapshot` with the default cache size.
    pub fn new(snapshot: ScpgSnapshot) -> Self {
        Self::from_parts(Arc::new(snapshot), DEFAULT_CACHE_CAPACITY, None)
    }

    /// Assembles an engine from a snapshot, a cache size and an optional file backing.
    pub fn from_parts(
        snapshot: Arc<ScpgSnapshot>,
        cache_capacity: usize,
        mmap: Option<Arc<MemoryMappedSCPG>>,
    ) -> Self {
        Self {
            scpg_hash: snapshot.scpg_hash,
            query: Arc::new(QueryEngine::with_cache_capacity(
                Arc::clone(&snapshot),
                cache_capacity,
            )),
            incremental: IncrementalEngine::new(snapshot),
            mmap,
            cache_capacity,
        }
    }

    /// Opens a `.scpg` file (memory-mapped, structurally verified) and decodes its snapshot.
    pub fn open(scpg_path: &Path) -> IoResult<Self> {
        EngineBuilder::new().open(scpg_path)
    }

    /// Short hash of the current snapshot.
    pub fn scpg_hash(&self) -> u32 {
        self.scpg_hash
    }

    /// Full SHA-256 content digest of the current snapshot.
    pub fn digest(&self) -> ScpgDigest {
        self.query.snapshot().digest
    }

    /// Content digest as lowercase hex.
    pub fn digest_hex(&self) -> String {
        digest_hex(&self.query.snapshot().digest)
    }

    /// The current snapshot.
    pub fn snapshot(&self) -> &Arc<ScpgSnapshot> {
        self.query.snapshot()
    }

    /// Shared handle to the query engine (e.g. for server worker threads).
    pub fn queries(&self) -> Arc<QueryEngine> {
        Arc::clone(&self.query)
    }

    /// Element counts of the current snapshot.
    pub fn stats(&self) -> SnapshotStats {
        self.snapshot().stats()
    }

    /// Full verification of the backing file (every section CRC and the content digest).
    /// `None` for engines that are not backed by a file.
    pub fn verify(&self) -> Option<Result<VerificationReport, ScpgFormatError>> {
        self.mmap.as_ref().map(|m| m.verify_all())
    }

    /// The canonical CFG artifact stored in the backing file, if any.
    pub fn cfg_artifact(&self) -> Option<Result<CFGArtifact, ScpgFormatError>> {
        self.mmap.as_ref().map(|m| m.cfg_artifact())
    }

    /// Whether method `target` is reachable from `source` through calls.
    pub fn is_reachable(&self, source: u32, target: u32) -> Result<bool, QueryError> {
        self.query.is_reachable(source, target)
    }

    /// Statement-level backward slice of every statement of `root_sym`.
    pub fn backward_slice(
        &self,
        root_sym: u32,
        depth: Option<u32>,
    ) -> Result<Arc<ProgramSlice>, QueryError> {
        self.query.backward_slice(root_sym, depth)
    }

    /// Statement-level forward slice of every statement of `root_sym`.
    pub fn forward_slice(
        &self,
        root_sym: u32,
        depth: Option<u32>,
    ) -> Result<Arc<ProgramSlice>, QueryError> {
        self.query.forward_slice(root_sym, depth)
    }

    /// Change-impact set of `sym_id` (callers, overrides, subtypes, type references).
    pub fn impact(&self, sym_id: u32) -> Result<Arc<Vec<ImpactEntry>>, QueryError> {
        self.query.impact(sym_id, ImpactOptions::default())
    }

    /// Source location of `sym_id` in O(1).
    pub fn to_source(&self, sym_id: u32) -> Option<SourceLocation> {
        self.query.to_source(sym_id)
    }

    /// Entities containing a source position, innermost first.
    pub fn entities_at(&self, file_id: u16, line: u32, col: u16) -> Vec<SourceLocation> {
        self.query.entities_at(file_id, line, col)
    }

    /// Evaluates an OpenHeart Datalog program.
    pub fn run_query(&self, program: &str) -> Result<Arc<QueryOutput>, QueryError> {
        self.query.run(program)
    }

    /// Evaluates a program and renders `{ "columns": [...], "rows": [[...]], ... }`.
    pub fn run_query_json(&self, program: &str) -> Result<String, QueryError> {
        let output = self.query.run(program)?;
        Ok(serde_json::to_string(output.as_ref()).unwrap_or_else(|e| {
            serde_json::json!({ "error": format!("serialization failed: {}", e) }).to_string()
        }))
    }

    /// Versioned machine-readable export (`ohb-json`). The CFG is read back from the backing
    /// file when `inputs.cfg` is not supplied.
    pub fn export_ohb_json(&self, inputs: &ExportInputs<'_>) -> serde_json::Value {
        if inputs.cfg.is_none() {
            if let Some(Ok(cfa)) = self.cfg_artifact() {
                let with_cfg = ExportInputs {
                    cfg: Some(&cfa),
                    ssa: inputs.ssa,
                    phase_timings_ms: inputs.phase_timings_ms,
                    include_sdg: inputs.include_sdg,
                };
                return export_ohb_json(self.snapshot(), &with_cfg);
            }
        }
        export_ohb_json(self.snapshot(), inputs)
    }

    /// Plans the incremental rebuild of a file change against the current snapshot.
    pub fn plan_update(&self, delta: &SourceDelta) -> RebuildPlan {
        self.incremental.plan(delta)
    }

    /// Switches to a rebuilt snapshot and reports what changed. The new query engine starts
    /// with an empty cache keyed by the new content digest; the file backing is dropped
    /// because it describes the previous snapshot.
    pub fn commit_snapshot(&mut self, snapshot: ScpgSnapshot) -> UpdateResult {
        let snapshot = Arc::new(snapshot);
        let result = self.incremental.commit(Arc::clone(&snapshot));
        self.query.cache().clear();
        self.query = Arc::new(QueryEngine::with_cache_capacity(
            Arc::clone(&snapshot),
            self.cache_capacity,
        ));
        self.scpg_hash = snapshot.scpg_hash;
        self.mmap = None;
        result
    }

    /// Incremental revision counter (1 for a fresh engine).
    pub fn revision(&self) -> u64 {
        self.incremental.revision()
    }

    /// Mermaid class diagram of the UML metadata.
    pub fn export_mermaid(
        &self,
        uma: &UMLMetadataArtifact,
        sta: &crate::symbol::SymbolTableArtifact,
        tca: &crate::ingestion::TokenCorpusArtifact,
    ) -> String {
        crate::scpg::diagram::export::mermaid::MermaidExporter::export_class_diagram(uma, sta, tca)
    }

    /// XMI class diagram of the given classes.
    pub fn export_xmi(&self, classes: &[ClassRecord]) -> String {
        XMIExporter::export_class_diagram(classes)
    }

    /// PlantUML class diagram of the UML metadata.
    pub fn export_plantuml(
        &self,
        uma: &UMLMetadataArtifact,
        sta: &crate::symbol::SymbolTableArtifact,
        tca: &crate::ingestion::TokenCorpusArtifact,
    ) -> String {
        PlantUMLExporter::export_class_diagram(uma, sta, tca)
    }

    /// Default diagram export (PlantUML class diagram).
    pub fn export_diagram_default(
        &self,
        uma: &UMLMetadataArtifact,
        sta: &crate::symbol::SymbolTableArtifact,
        tca: &crate::ingestion::TokenCorpusArtifact,
    ) -> String {
        self.export_plantuml(uma, sta, tca)
    }

    /// JSON graph IR of the class diagram.
    pub fn export_json(
        &self,
        uma: &UMLMetadataArtifact,
        sta: &crate::symbol::SymbolTableArtifact,
        tca: &crate::ingestion::TokenCorpusArtifact,
    ) -> String {
        let graph_ir = JSONExporter::export_class_diagram(uma, sta, tca);
        serde_json::to_string_pretty(&graph_ir).unwrap_or_else(|_| "{}".to_string())
    }

    /// Human-readable summary of the UML metadata.
    pub fn summary(&self, uma: &UMLMetadataArtifact) -> String {
        DiagramGenerator::generate_summary(uma)
    }
}
