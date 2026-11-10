//! Template Method & Builder Design Patterns for SCPG Compilation Pipeline (§10.1).
//!
//! `SCPGPipelineBuilder::execute` drives all ten phases with one error type
//! (`PipelineError`), per-stage timing, and in-memory hand-over of the Phase 1 corpus to
//! Phase 2 (no `.tca` read-back or memory mapping). Phases 1 and 2 run through the
//! `PipelineStageTemplate` lifecycle, including invariant verification.
//!
//! Artifacts are written atomically (temporary file + rename). Without an explicit output
//! directory the builder uses `OPENHEART_ARTIFACT_DIR`, or else a freshly created,
//! uniquely named directory under the system temp directory, so concurrent runs never
//! share, truncate or follow planted files at fixed paths.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Instant;

use thiserror::Error;

use crate::ast::{ASTStage, BPASTArtifact};
use crate::cfg::serializer::CFGArtifact;
use crate::cfg::Phase4Stage;
use crate::cg::Phase6Stage;
use crate::core::io::fs::{create_unique_dir, write_atomic};
use crate::core::logger::{log_info, log_warn};
use crate::core::types::cg::CallGraphArtifact;
use crate::core::types::source::SkipReason;
use crate::ingestion::manifest::SourceManifest;
use crate::ingestion::{IngestionReport, IngestionStage, TokenCorpusArtifact};
use crate::psa::types::PathSummaryArtifact;
use crate::psa::Phase8Stage;
use crate::scpg::diagram::factory::{DiagramFormat, UniversalDiagramEngine};
use crate::scpg::{OpenHeartEngine, Phase10Stage};
use crate::ssa::serializer::SSAArtifact;
use crate::ssa::Phase5Stage;
use crate::symbol::{Phase3Stage, SymbolTableArtifact};
use crate::tra::types::TraceabilityArtifact;
use crate::tra::Phase7Stage;
use crate::uma::types::UMLMetadataArtifact;
use crate::uma::Phase9Stage;

/// Environment variable naming the default artifact directory.
pub const ENV_ARTIFACT_DIR: &str = "OPENHEART_ARTIFACT_DIR";

/// Typed pipeline failure.
#[derive(Debug, Error)]
pub enum PipelineError {
    #[error("cannot execute the SCPG pipeline with zero source files")]
    EmptyManifest,
    #[error("{context}: {source}")]
    Io {
        context: String,
        #[source]
        source: io::Error,
    },
    #[error("Phase {phase} ({stage}) failed: {message}")]
    Stage {
        phase: u8,
        stage: &'static str,
        message: String,
    },
    #[error("Phase {phase} ({stage}) violated an invariant: {message}")]
    Invariant {
        phase: u8,
        stage: &'static str,
        message: String,
    },
}

impl PipelineError {
    fn io(context: impl Into<String>, source: io::Error) -> Self {
        PipelineError::Io {
            context: context.into(),
            source,
        }
    }
}

// ── Template Method Pattern: Stage Execution Template ────────────────────────

/// Template Method Trait for SCPG Compilation Stages.
pub trait PipelineStageTemplate<Input, Output> {
    /// Stage identifier for telemetry.
    fn stage_name(&self) -> &'static str;

    /// Phase number (1 through 10).
    fn phase_number(&self) -> u8;

    /// Step 1: Pre-execution telemetry and initialization hook.
    fn before_stage(&self, _input: &Input) {
        log_info(&format!(
            "══► Starting Stage {}: {}...",
            self.phase_number(),
            self.stage_name()
        ));
    }

    /// Step 2: Core compilation transformation (primitive operation).
    fn run_stage(&self, input: &Input, output_path: &Path) -> Result<Output, String>;

    /// Step 3: Mathematical and structural invariant verification.
    fn verify_invariants(&self, _output: &Output) -> Result<(), String> {
        Ok(())
    }

    /// Step 4: Post-execution telemetry and digest logging hook.
    fn after_stage(&self, _output: &Output, duration: std::time::Duration) {
        log_info(&format!(
            "✓ Stage {} [{}] Complete in {:.2}ms",
            self.phase_number(),
            self.stage_name(),
            duration.as_secs_f64() * 1000.0
        ));
    }

    /// Template Method: Enforces the invariant execution lifecycle.
    fn execute(&self, input: &Input, output_path: &Path) -> Result<Output, String> {
        self.before_stage(input);
        let start = Instant::now();
        let output = self.run_stage(input, output_path)?;
        self.verify_invariants(&output)?;
        let elapsed = start.elapsed();
        self.after_stage(&output, elapsed);
        Ok(output)
    }
}

impl PipelineStageTemplate<SourceManifest, (TokenCorpusArtifact, IngestionReport)>
    for IngestionStage
{
    fn stage_name(&self) -> &'static str {
        "Lexical Ingestion"
    }

    fn phase_number(&self) -> u8 {
        1
    }

    fn run_stage(
        &self,
        input: &SourceManifest,
        output_path: &Path,
    ) -> Result<(TokenCorpusArtifact, IngestionReport), String> {
        IngestionStage::run_with_report(input.clone(), output_path)
    }

    fn verify_invariants(
        &self,
        output: &(TokenCorpusArtifact, IngestionReport),
    ) -> Result<(), String> {
        let (tca, report) = output;
        if report.diagnostics.len() != tca.file_records.len() {
            return Err(format!(
                "{} diagnostics entries for {} files",
                report.diagnostics.len(),
                tca.file_records.len()
            ));
        }
        if tca.token_records.len() != tca.token_entries.len() {
            return Err(format!(
                "forward index has {} tokens, backward index {}",
                tca.token_records.len(),
                tca.token_entries.len()
            ));
        }
        Ok(())
    }
}

/// Phase 2 input: the in-memory corpus and the CRC-64 of its `.tca` bytes.
pub struct AstStageRequest<'a> {
    pub tca: &'a TokenCorpusArtifact,
    pub tca_hash: u64,
}

impl<'a> PipelineStageTemplate<AstStageRequest<'a>, BPASTArtifact> for ASTStage {
    fn stage_name(&self) -> &'static str {
        "CST Reduction & BP AST Encoding"
    }

    fn phase_number(&self) -> u8 {
        2
    }

    fn run_stage(
        &self,
        input: &AstStageRequest<'a>,
        output_path: &Path,
    ) -> Result<BPASTArtifact, String> {
        ASTStage::run_in_memory(input.tca, input.tca_hash, output_path).map_err(|e| e.to_string())
    }

    fn verify_invariants(&self, output: &BPASTArtifact) -> Result<(), String> {
        output.validate()
    }
}

/// Runs one phase body with timing telemetry and typed error mapping.
fn timed<T>(
    phase: u8,
    stage: &'static str,
    body: impl FnOnce() -> Result<T, String>,
) -> Result<T, PipelineError> {
    log_info(&format!("══► Starting Stage {}: {}...", phase, stage));
    let start = Instant::now();
    let out = body().map_err(|message| PipelineError::Stage {
        phase,
        stage,
        message,
    })?;
    log_info(&format!(
        "✓ Stage {} [{}] Complete in {:.2}ms",
        phase,
        stage,
        start.elapsed().as_secs_f64() * 1000.0
    ));
    Ok(out)
}

fn read_artifact(path: &Path) -> Result<Vec<u8>, PipelineError> {
    fs::read(path).map_err(|e| PipelineError::io(format!("Failed to read {}", path.display()), e))
}

/// Diagram types exported for every requested format.
pub const DIAGRAM_TYPES: &[&str] = &[
    "class",
    "object",
    "component",
    "deployment",
    "package",
    "composite",
    "profile",
    "usecase",
    "activity",
    "statemachine",
    "sequence",
    "communication",
    "interaction",
    "timing",
    "cfg",
    "robdd",
    "dfg",
    "cdg",
    "callgraph",
];

// ── Builder Pattern: Fluent Pipeline Construction ─────────────────────────────

/// Complete set of compiled binary artifacts generated by the 10-phase pipeline.
pub struct PipelineArtifacts {
    pub tca: TokenCorpusArtifact,
    pub bpa: BPASTArtifact,
    pub sta: SymbolTableArtifact,
    pub cfa: CFGArtifact,
    pub ssa: SSAArtifact,
    pub cga: CallGraphArtifact,
    pub tra: TraceabilityArtifact,
    pub psa: PathSummaryArtifact,
    pub uma: UMLMetadataArtifact,
    pub engine: OpenHeartEngine,
    /// Phase 1 diagnostics (skipped files, syntax errors) and the written `.tca` bytes.
    pub ingestion_report: IngestionReport,
    /// Directory holding the binary artifacts.
    pub output_dir: PathBuf,
    /// Diagram files written for the requested formats.
    pub diagram_files: Vec<PathBuf>,
}

/// Fluent Builder for configuring and executing the 10-Phase SCPG Compilation Pipeline.
pub struct SCPGPipelineBuilder {
    source_files: Vec<PathBuf>,
    output_dir: Option<PathBuf>,
    diagram_formats: Vec<DiagramFormat>,
}

impl Default for SCPGPipelineBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl SCPGPipelineBuilder {
    pub fn new() -> Self {
        Self {
            source_files: Vec::new(),
            output_dir: None,
            diagram_formats: vec![DiagramFormat::PlantUML, DiagramFormat::Mermaid],
        }
    }

    /// Adds a source file to the pipeline manifest.
    pub fn with_source_file<P: Into<PathBuf>>(mut self, path: P) -> Self {
        self.source_files.push(path.into());
        self
    }

    /// Adds multiple source files to the pipeline manifest.
    pub fn with_source_files<I, P>(mut self, paths: I) -> Self
    where
        I: IntoIterator<Item = P>,
        P: Into<PathBuf>,
    {
        for p in paths {
            self.source_files.push(p.into());
        }
        self
    }

    /// Sets the target directory for serialized binary artifacts (.tca, .bpa, etc.).
    pub fn with_output_dir<P: Into<PathBuf>>(mut self, dir: P) -> Self {
        self.output_dir = Some(dir.into());
        self
    }

    /// Adds a diagram export format.
    pub fn with_diagram_format(mut self, format: DiagramFormat) -> Self {
        if !self.diagram_formats.contains(&format) {
            self.diagram_formats.push(format);
        }
        self
    }

    /// Exports no diagrams (binary artifacts only).
    pub fn without_diagrams(mut self) -> Self {
        self.diagram_formats.clear();
        self
    }

    fn resolve_output_dir(&self) -> Result<PathBuf, PipelineError> {
        if let Some(dir) = &self.output_dir {
            fs::create_dir_all(dir).map_err(|e| {
                PipelineError::io(
                    format!("Failed to create output directory {}", dir.display()),
                    e,
                )
            })?;
            return Ok(dir.clone());
        }
        if let Some(dir) = std::env::var_os(ENV_ARTIFACT_DIR).filter(|v| !v.is_empty()) {
            let dir = PathBuf::from(dir);
            fs::create_dir_all(&dir).map_err(|e| {
                PipelineError::io(
                    format!(
                        "Failed to create {} directory {}",
                        ENV_ARTIFACT_DIR,
                        dir.display()
                    ),
                    e,
                )
            })?;
            return Ok(dir);
        }
        // WASI has no temporary directory (`std::env::temp_dir` panics there); the
        // WebAssembly build runs `adapters::runner::run_pipeline_in_memory` instead.
        if cfg!(target_os = "wasi") {
            return Err(PipelineError::io(
                "No output directory was given and this target has no temporary directory"
                    .to_string(),
                std::io::Error::new(
                    std::io::ErrorKind::Unsupported,
                    "std::env::temp_dir is unavailable on WASI",
                ),
            ));
        }
        let temp = std::env::temp_dir();
        create_unique_dir(&temp, "openheart-scpg").map_err(|e| {
            PipelineError::io(
                format!(
                    "Failed to create a private output directory under {}",
                    temp.display()
                ),
                e,
            )
        })
    }

    /// Executes all 10 compilation phases sequentially with full invariant guarantees.
    pub fn execute(self) -> Result<PipelineArtifacts, PipelineError> {
        if self.source_files.is_empty() {
            return Err(PipelineError::EmptyManifest);
        }
        let output_dir = self.resolve_output_dir()?;

        let tca_path = output_dir.join("corpus.tca");
        let bpa_path = output_dir.join("ast.bpa");
        let sta_path = output_dir.join("symbols.sta");
        let cfa_path = output_dir.join("cfg.cfa");
        let ssa_path = output_dir.join("ssa.ssa");
        let cga_path = output_dir.join("callgraph.cga");
        let tra_path = output_dir.join("traceability.tra");
        let psa_path = output_dir.join("paths.psa");
        let uma_path = output_dir.join("metadata.uma");
        let scpg_path = output_dir.join("graph.scpg");

        // Phase 1: Ingestion
        let manifest = SourceManifest::new(self.source_files.clone());
        let (tca, report) = PipelineStageTemplate::execute(&IngestionStage, &manifest, &tca_path)
            .map_err(|message| PipelineError::Stage {
            phase: 1,
            stage: "Lexical Ingestion",
            message,
        })?;
        let skipped = report
            .diagnostics
            .iter()
            .filter(|d| d.skip_reason != SkipReason::None)
            .count();
        if skipped > 0 {
            log_warn(&format!(
                "{} of {} source files were skipped by Phase 1; see the .tca diagnostics",
                skipped,
                report.diagnostics.len()
            ));
        }

        // Phase 2: AST, from the in-memory corpus.
        let request = AstStageRequest {
            tca: &tca,
            tca_hash: report.tca_hash(),
        };
        let bpa =
            PipelineStageTemplate::execute(&ASTStage, &request, &bpa_path).map_err(|message| {
                PipelineError::Stage {
                    phase: 2,
                    stage: "CST Reduction & BP AST Encoding",
                    message,
                }
            })?;
        let bpa_bytes = read_artifact(&bpa_path)?;

        // Phase 3: Symbols & Scope Graph
        let sta = timed(3, "Symbol Table & Type Hierarchy", || {
            Phase3Stage::run(&tca, &bpa, &report.tca_bytes, &bpa_bytes).map_err(|e| e.to_string())
        })?;
        let sta_bytes = sta.serialize();
        write_atomic(&sta_path, &sta_bytes)
            .map_err(|e| PipelineError::io(format!("Failed to write {}", sta_path.display()), e))?;

        // Phase 4: CFG
        let cfa = timed(4, "Control Flow Graph", || {
            Phase4Stage::run_with_source(&bpa, &sta, Some(&tca), &sta_bytes, &bpa_bytes, &cfa_path)
        })?;
        let cfa_bytes = read_artifact(&cfa_path)?;

        // Phase 5: SSA
        let ssa = timed(5, "SSA Conversion", || {
            Phase5Stage::run(&bpa, &sta, &cfa, &cfa_bytes, &ssa_path)
        })?;
        let ssa_bytes = read_artifact(&ssa_path)?;

        // Phase 6: Call Graph
        let cga = timed(6, "Call Graph & Points-To", || {
            Phase6Stage::run_with_source(
                &bpa,
                &sta,
                Some(&tca),
                &cfa,
                &ssa,
                &ssa_bytes,
                &sta_bytes,
                &cga_path,
            )
        })?;

        // Phase 7: Traceability
        let tra = timed(7, "Traceability Index", || {
            Ok(Phase7Stage::run(
                &tca, &bpa, &sta, &cfa, &ssa, &cga, &tra_path,
            ))
        })?;
        let tra_bytes = read_artifact(&tra_path)?;

        // Phase 8: PSA
        let psa = timed(8, "ROBDD Path Summaries", || {
            Ok(Phase8Stage::run(&cfa, &ssa, &cga, &cfa_bytes, &psa_path))
        })?;

        // Phase 9: UMA
        let uma = timed(9, "UML Semantic Metadata", || {
            Ok(Phase9Stage::run(
                &tca, &bpa, &sta, &cfa, &ssa, &cga, &tra, &psa, &tra_bytes, &uma_path,
            ))
        })?;

        // Phase 10: SCPG Binary & Engine Bootstrap
        let engine = timed(10, "SCPG Unified Binary", || {
            Phase10Stage::try_run(
                &tca, &bpa, &sta, &cfa, &ssa, &cga, &tra, &uma, &psa, &scpg_path,
            )
            .map_err(|e| e.to_string())
        })?;

        // Diagram export for every requested format.
        let diagram_files = Self::export_diagrams(
            &self.diagram_formats,
            &output_dir.join("diagrams"),
            &uma,
            &sta,
            &tca,
        )?;

        Ok(PipelineArtifacts {
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
            ingestion_report: report,
            output_dir,
            diagram_files,
        })
    }

    fn export_diagrams(
        formats: &[DiagramFormat],
        dir: &Path,
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> Result<Vec<PathBuf>, PipelineError> {
        if formats.is_empty() {
            return Ok(Vec::new());
        }
        fs::create_dir_all(dir).map_err(|e| {
            PipelineError::io(
                format!("Failed to create diagram directory {}", dir.display()),
                e,
            )
        })?;
        let engine = UniversalDiagramEngine::new();
        let mut written = Vec::new();
        for &format in formats {
            for &diagram_type in DIAGRAM_TYPES {
                let Some(content) = engine.export_diagram(format, diagram_type, uma, sta, tca)
                else {
                    continue;
                };
                let path = dir.join(format!("{}.{}", diagram_type, format.file_extension()));
                write_atomic(&path, content.as_bytes()).map_err(|e| {
                    PipelineError::io(format!("Failed to write {}", path.display()), e)
                })?;
                written.push(path);
            }
        }
        log_info(&format!(
            "Exported {} diagram file(s) to {}",
            written.len(),
            dir.display()
        ));
        Ok(written)
    }
}
