//! One finished analysis, kept for queries (docs/engine-query-protocol.md, section 5).
//!
//! An [`AnalysisSession`] holds every phase artifact of a run, the root-relative path of each
//! corpus file, the build and deployment files found next to the sources, and the source
//! texts. The query protocol ([`crate::adapters::query::execute_session`]) answers every
//! request from it. A session comes either straight from a finished run
//! ([`AnalysisSession::from_run`]) or from the directory a disk run wrote
//! ([`AnalysisSession::open`]).

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::adapters::runner::{
    AnalysisArtifacts, ArtifactPaths, PipelineRun, PROJECT_FILES_FILE, RUN_REPORT_FILE,
};
use crate::ast::serializer::BPASTSerializer;
use crate::cfg::serializer::CFGArtifact;
use crate::cg::serializer::CGASerializer;
use crate::ingestion::serializer::TokenCorpusSerializer;
use crate::psa::serializer::PathSummarySerializer;
use crate::scpg::api::builder::EngineBuilder;
use crate::scpg::diagram::context::DiagramContext;
use crate::ssa::serializer::SSASerializer;
use crate::symbol::SymbolTableArtifact;
use crate::tra::serializer::TraceabilitySerializer;
use crate::uma::serializer::UMASerializer;

/// A build or deployment file (Dockerfile, compose file, Kubernetes manifest, `pom.xml`,
/// Gradle script, `package.json`, `.csproj`, `.sln`, `CMakeLists.txt`) found next to the
/// analysed sources, with its root-relative path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectFile {
    pub path: String,
    pub text: String,
}

/// One finished analysis.
pub struct AnalysisSession {
    /// Every phase artifact, including the Phase 10 query engine.
    pub artifacts: AnalysisArtifacts,
    /// Root-relative path of each corpus file, indexed by file id (`None` when unmapped).
    pub relative_paths: Vec<Option<String>>,
    /// Build and deployment files found next to the sources.
    pub project_files: Vec<ProjectFile>,
    /// Text of every analysed source file, keyed by its root-relative path. Empty for a
    /// session reopened from a directory, which does not keep the sources.
    pub sources: BTreeMap<String, String>,
}

impl AnalysisSession {
    /// Keeps a finished run for queries.
    pub fn from_run(
        run: PipelineRun,
        project_files: Vec<ProjectFile>,
        sources: BTreeMap<String, String>,
    ) -> Self {
        Self {
            artifacts: run.artifacts,
            relative_paths: run.relative_paths,
            project_files,
            sources,
        }
    }

    /// Reloads the analysis a disk run wrote to `dir`: every phase artifact, the relative
    /// paths from `run.json`, and the project files from `project-files.json` when present.
    pub fn open(dir: &Path) -> io::Result<Self> {
        let paths = ArtifactPaths::in_dir(dir);
        let tca = TokenCorpusSerializer::read(&paths.tca).map_err(other("corpus.tca"))?;
        let bpa = BPASTSerializer::read(&paths.bpa)?;
        let sta = SymbolTableArtifact::deserialize(&fs::read(&paths.sta)?)?;
        let cfa =
            CFGArtifact::deserialize(&fs::read(&paths.cfa)?).map_err(other("the CFG artifact"))?;
        let ssa = SSASerializer::read(&paths.ssa).map_err(other("the SSA artifact"))?;
        let cga = CGASerializer::deserialize(&paths.cga)?;
        let tra = TraceabilitySerializer::read(&paths.tra)?;
        let psa = PathSummarySerializer::read(&paths.psa)?;
        let uma = UMASerializer::read(&paths.uma)?;
        let engine = EngineBuilder::new().with_mmap(false).open(&paths.scpg)?;

        let relative_paths = read_relative_paths(&dir.join(RUN_REPORT_FILE))?;
        let project_files = match fs::read(dir.join(PROJECT_FILES_FILE)) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|err| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("{}: {}", PROJECT_FILES_FILE, err),
                )
            })?,
            Err(err) if err.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(err) => return Err(err),
        };

        Ok(Self {
            artifacts: AnalysisArtifacts {
                tca,
                bpa,
                sta,
                cfa,
                ssa,
                cga,
                tra,
                psa,
                uma,
                engine,
            },
            relative_paths,
            project_files,
            sources: BTreeMap::new(),
        })
    }

    /// The diagram context over every artifact of the session.
    pub fn diagram_context(&self) -> DiagramContext<'_> {
        let a = &self.artifacts;
        DiagramContext::new(&a.uma, &a.sta, &a.tca)
            .with_bpa(&a.bpa)
            .with_cfa(&a.cfa)
            .with_ssa(&a.ssa)
            .with_cga(&a.cga)
            .with_psa(&a.psa)
            .with_tra(&a.tra)
    }

    /// Root-relative path of a corpus file, when known.
    pub fn file_path(&self, file_id: u16) -> Option<&str> {
        self.relative_paths
            .get(file_id as usize)
            .and_then(|p| p.as_deref())
    }
}

fn other(what: &'static str) -> impl Fn(String) -> io::Error {
    move |message| io::Error::new(io::ErrorKind::InvalidData, format!("{}: {}", what, message))
}

/// `relative_paths` of a `run.json` report (empty when the report predates the field).
fn read_relative_paths(report: &Path) -> io::Result<Vec<Option<String>>> {
    let bytes = match fs::read(report) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err),
    };
    let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|err| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{}: {}", RUN_REPORT_FILE, err),
        )
    })?;
    Ok(value
        .get("relative_paths")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .map(|item| item.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default())
}
