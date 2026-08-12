//! OpenHeartEngine — production entry point for all SCPG operations (§10.7).

pub mod builder;

pub use builder::EngineBuilder;

use std::io::Result as IoResult;
use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::core::types::cg::CallGraphArtifact;
use crate::scpg::diagram::export::json::JSONExporter;
use crate::scpg::diagram::export::plantuml::PlantUMLExporter;
use crate::scpg::diagram::export::xmi::XMIExporter;
use crate::scpg::diagram::DiagramGenerator;
use crate::scpg::mmap::MemoryMappedSCPG;
use crate::scpg::query::QueryEngine;
use crate::ssa::serializer::SSAArtifact;
use crate::symbol::SymbolTableArtifact;
use crate::uma::types::*;

pub struct OpenHeartEngine {
    pub scpg_hash: u32,
    pub query: Arc<Mutex<QueryEngine>>,
    pub mmap: Option<Arc<MemoryMappedSCPG>>,
}

impl OpenHeartEngine {
    pub fn new(scpg_hash: u32) -> Self {
        Self {
            scpg_hash,
            query: Arc::new(Mutex::new(QueryEngine::new())),
            mmap: None,
        }
    }

    pub fn open(scpg_path: &Path) -> IoResult<Self> {
        let mmap = MemoryMappedSCPG::open(scpg_path)?;
        let scpg_hash = mmap.header.scpg_hash;
        Ok(Self {
            scpg_hash,
            query: Arc::new(Mutex::new(QueryEngine::new())),
            mmap: Some(Arc::new(mmap)),
        })
    }

    pub fn scpg_hash(&self) -> u32 {
        self.scpg_hash
    }

    pub fn is_reachable(&self, source: u32, target: u32, cga: &CallGraphArtifact) -> bool {
        if let Ok(mut engine) = self.query.lock() {
            engine.is_reachable(source, target, cga, self.scpg_hash)
        } else {
            false
        }
    }

    pub fn backward_slice(
        &self,
        root_sym: u32,
        sta: &SymbolTableArtifact,
        ssa: &SSAArtifact,
        cga: &CallGraphArtifact,
        depth: u32,
    ) -> Vec<u32> {
        if let Ok(mut engine) = self.query.lock() {
            engine.backward_slice(root_sym, sta, ssa, cga, depth, self.scpg_hash)
        } else {
            Vec::new()
        }
    }

    pub fn export_mermaid(&self, uma: &UMLMetadataArtifact, sta: &crate::symbol::SymbolTableArtifact, tca: &crate::ingestion::TokenCorpusArtifact) -> String {
        crate::scpg::diagram::export::mermaid::MermaidExporter::export_class_diagram(uma, sta, tca)
    }

    pub fn export_xmi(&self, classes: &[ClassRecord]) -> String {
        XMIExporter::export_class_diagram(classes)
    }

    pub fn export_plantuml(&self, classes: &[ClassRecord]) -> String {
        PlantUMLExporter::export_class_diagram(classes)
    }

    pub fn export_json(&self, classes: &[ClassRecord]) -> String {
        JSONExporter::export_class_diagram(classes)
    }

    pub fn summary(&self, uma: &UMLMetadataArtifact) -> String {
        DiagramGenerator::generate_summary(uma)
    }
}
