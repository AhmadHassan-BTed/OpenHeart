//! EngineBuilder — fluent builder for OpenHeartEngine configuration (§10.7).

use std::fs;
use std::io::Result as IoResult;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::ast::BPASTArtifact;
use crate::cfg::serializer::CFGArtifact;
use crate::core::types::cg::CallGraphArtifact;
use crate::ingestion::TokenCorpusArtifact;
use crate::psa::types::PathSummaryArtifact;
use crate::scpg::api::OpenHeartEngine;
use crate::scpg::mmap::MemoryMappedSCPG;
use crate::scpg::query::cache::DEFAULT_CACHE_CAPACITY;
use crate::scpg::query::snapshot::ScpgSnapshot;
use crate::scpg::serializer::{language_count, SCPGSerializer};
use crate::ssa::serializer::SSAArtifact;
use crate::symbol::SymbolTableArtifact;
use crate::tra::types::TraceabilityArtifact;
use crate::uma::types::UMLMetadataArtifact;

/// How much of a `.scpg` file is verified when it is opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyPolicy {
    /// Header, directory and bounds at open; each section's CRC on first access.
    Structure,
    /// Additionally every section CRC and the SHA-256 content digest before use.
    Full,
}

/// Fluent configuration for opening or building an `OpenHeartEngine`.
pub struct EngineBuilder {
    /// Directory `build` writes the `.scpg` file into.
    pub output_dir: PathBuf,
    /// Maximum number of cached query results (0 disables caching).
    pub cache_capacity: usize,
    /// Memory-map `.scpg` files (`true`) or read them into memory (`false`).
    pub enable_mmap: bool,
    /// Verification performed when a file is opened.
    pub verify: VerifyPolicy,
}

impl Default for EngineBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl EngineBuilder {
    /// Defaults: `./scpg_out`, 512 cached results, memory mapping, structural verification.
    pub fn new() -> Self {
        Self {
            output_dir: PathBuf::from("./scpg_out"),
            cache_capacity: DEFAULT_CACHE_CAPACITY,
            enable_mmap: true,
            verify: VerifyPolicy::Structure,
        }
    }

    /// Sets the output directory used by `build`.
    pub fn with_output_dir(mut self, path: PathBuf) -> Self {
        self.output_dir = path;
        self
    }

    /// Sets the query cache capacity.
    pub fn with_cache_capacity(mut self, cap: usize) -> Self {
        self.cache_capacity = cap;
        self
    }

    /// `false` reads `.scpg` files into memory instead of memory-mapping them.
    pub fn with_mmap(mut self, enable: bool) -> Self {
        self.enable_mmap = enable;
        self
    }

    /// Sets the verification policy applied by `open` and `build`.
    pub fn with_verification(mut self, policy: VerifyPolicy) -> Self {
        self.verify = policy;
        self
    }

    fn load(&self, path: &Path) -> IoResult<MemoryMappedSCPG> {
        let file = if self.enable_mmap {
            MemoryMappedSCPG::open(path)?
        } else {
            MemoryMappedSCPG::open_in_memory(path)?
        };
        if self.verify == VerifyPolicy::Full {
            file.verify_all()?;
        }
        Ok(file)
    }

    /// Opens an existing `.scpg` file and serves queries from it without re-running any phase.
    pub fn open(&self, path: &Path) -> IoResult<OpenHeartEngine> {
        let file = self.load(path)?;
        let snapshot = file.snapshot()?;
        Ok(OpenHeartEngine::from_parts(
            Arc::new(snapshot),
            self.cache_capacity,
            Some(Arc::new(file)),
        ))
    }

    /// Builds the snapshot of the nine artifacts, writes `output_dir/file_name`, and returns
    /// an engine over it.
    #[allow(clippy::too_many_arguments)]
    pub fn build(
        &self,
        tca: &TokenCorpusArtifact,
        bpa: &BPASTArtifact,
        sta: &SymbolTableArtifact,
        cfa: &CFGArtifact,
        ssa: &SSAArtifact,
        cga: &CallGraphArtifact,
        tra: &TraceabilityArtifact,
        uma: &UMLMetadataArtifact,
        psa: &PathSummaryArtifact,
        file_name: &str,
    ) -> IoResult<OpenHeartEngine> {
        fs::create_dir_all(&self.output_dir)?;
        let path = self.output_dir.join(file_name);
        let (snapshot, encoded) = ScpgSnapshot::build(tca, bpa, sta, cfa, ssa, cga, tra, uma, psa);
        SCPGSerializer::write_encoded(&encoded, snapshot.source_hash, language_count(tca), &path)?;
        let file = if self.enable_mmap {
            Some(Arc::new(self.load(&path)?))
        } else {
            None
        };
        Ok(OpenHeartEngine::from_parts(Arc::new(snapshot), self.cache_capacity, file))
    }

    /// Wraps an in-memory snapshot (no file backing).
    pub fn from_snapshot(&self, snapshot: ScpgSnapshot) -> OpenHeartEngine {
        OpenHeartEngine::from_parts(Arc::new(snapshot), self.cache_capacity, None)
    }
}
