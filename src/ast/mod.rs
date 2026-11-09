//! Phase 2: CST Reduction & Balanced Parentheses AST Encoding Module.
//!
//! Phase 2 consumes the Phase 1 corpus either in memory (`ASTStage::run_in_memory`, no
//! file round trip) or from a `.tca` image (`ASTStage::run`), which is fully validated
//! first: checksum, magic, version, and every count and offset.
//!
//! Each file with a syntax tree is re-read and its SHA-256 compared with the digest
//! Phase 1 recorded; a changed or unreadable file fails the stage instead of yielding an
//! AST that disagrees with the token corpus. Files are re-parsed in parallel and reduced
//! sequentially in file order, so pre-order ids are deterministic. Files that Phase 1
//! skipped, lexical-only files (no grammar), and languages without a Phase 2 adapter are
//! reported, never parsed with another language's grammar.

#![forbid(unsafe_code)]

pub mod adapter;
pub mod bp_encoder;
pub mod builder;
pub mod jump_table;
pub mod preorder;
pub mod rank_select;
pub mod reducer;
pub mod rmq;
pub mod serializer;

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::core::io::mmap::MemoryMappedFile;
use crate::core::logger::{log_info, log_warn, PhaseTimer};
use crate::core::parallel::{parallel_map_with, resolve_worker_count};
use crate::core::types::source::{
    worker_threads_from_env, DEFAULT_PARSE_TIMEOUT_MICROS, FILE_FLAG_LEXICAL_ONLY,
    FILE_FLAG_PARSE_ABORTED, FILE_FLAG_SKIPPED,
};
use crate::core::types::token::LangId;
use crate::ingestion::parser::tree_sitter::TreeSitterParser;
use crate::ingestion::serializer::{crc64_ecma, TokenCorpusArtifact, TokenCorpusSerializer};
use adapter::registry::ASTAdapterRegistry;
use adapter::ASTReductionAdapter;
pub use builder::{BPASTArtifact, BPASTBuilder};
use reducer::{reduce_file, ReductionStats};
use serializer::BPASTSerializer;

pub const TCA_HEADER_SIZE: usize = crate::ingestion::serializer::TCA_HEADER_SIZE;

pub struct ASTStageInput {
    pub tca: MemoryMappedFile,
}

/// Phase 2 tuning.
#[derive(Debug, Clone, Copy)]
pub struct ASTStageOptions {
    /// Worker threads for re-parsing (0 = one per available core).
    pub worker_threads: usize,
    /// Per-file parse budget in microseconds (0 = unlimited).
    pub parse_timeout_micros: u64,
}

impl Default for ASTStageOptions {
    fn default() -> Self {
        Self {
            worker_threads: worker_threads_from_env(),
            parse_timeout_micros: DEFAULT_PARSE_TIMEOUT_MICROS,
        }
    }
}

pub struct ASTStage;

struct ReduceJob {
    file_id: u16,
    lang: LangId,
    path: PathBuf,
    sha256: [u8; 32],
    adapter: std::sync::Arc<dyn ASTReductionAdapter>,
}

fn invalid_data<M: Into<String>>(msg: M) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

impl ASTStage {
    /// Runs Phase 2 on a `.tca` image. The image is decoded and validated in full before
    /// use; its CRC-64 becomes the artifact's hash link.
    pub fn run(input: &ASTStageInput, out_path: &Path) -> io::Result<BPASTArtifact> {
        let tca_bytes = input.tca.as_slice();
        let tca = TokenCorpusSerializer::from_bytes(tca_bytes)
            .map_err(|e| invalid_data(format!("Phase 2 input is not a valid .tca: {}", e)))?;
        Self::run_in_memory(&tca, crc64_ecma(tca_bytes), out_path)
    }

    /// Runs Phase 2 on the in-memory Phase 1 artifact. `tca_hash` is the CRC-64 of the
    /// `.tca` bytes Phase 1 wrote (`IngestionReport::tca_hash`).
    pub fn run_in_memory(
        tca: &TokenCorpusArtifact,
        tca_hash: u64,
        out_path: &Path,
    ) -> io::Result<BPASTArtifact> {
        Self::run_with_options(tca, tca_hash, out_path, ASTStageOptions::default())
    }

    pub fn run_with_options(
        tca: &TokenCorpusArtifact,
        tca_hash: u64,
        out_path: &Path,
        options: ASTStageOptions,
    ) -> io::Result<BPASTArtifact> {
        Self::run_with_registry(tca, tca_hash, out_path, options, &ASTAdapterRegistry::new())
    }

    /// Runs Phase 2 with a caller-supplied reduction-adapter registry (for example one
    /// with an adapter for an additional linked grammar).
    pub fn run_with_registry(
        tca: &TokenCorpusArtifact,
        tca_hash: u64,
        out_path: &Path,
        options: ASTStageOptions,
        ast_registry: &ASTAdapterRegistry,
    ) -> io::Result<BPASTArtifact> {
        let timer = PhaseTimer::start("Phase 2: CST Reduction & BP AST Encoding");
        log_info(&format!(
            "TCA binary hash link computed: 0x{:016X}",
            tca_hash
        ));

        let tok_table = &tca.token_records;

        let mut jobs = Vec::with_capacity(tca.file_records.len());
        let mut skipped_by_phase1 = 0usize;
        let mut lexical_only: BTreeMap<LangId, usize> = BTreeMap::new();
        let mut no_adapter: BTreeMap<LangId, usize> = BTreeMap::new();

        for rec in &tca.file_records {
            let lang = LangId::from(rec.language_id);
            if rec.has_flag(FILE_FLAG_SKIPPED | FILE_FLAG_PARSE_ABORTED) {
                skipped_by_phase1 += 1;
                continue;
            }
            if rec.has_flag(FILE_FLAG_LEXICAL_ONLY) {
                *lexical_only.entry(lang).or_insert(0) += 1;
                continue;
            }
            let Some(adapter) = ast_registry.get(lang) else {
                *no_adapter.entry(lang).or_insert(0) += 1;
                continue;
            };
            let path = tca.file_path(rec.file_id).ok_or_else(|| {
                invalid_data(format!(
                    "file {} has no recorded path in the token corpus",
                    rec.file_id
                ))
            })?;
            jobs.push(ReduceJob {
                file_id: rec.file_id,
                lang,
                path: PathBuf::from(path),
                sha256: rec.content_sha256,
                adapter,
            });
        }

        if skipped_by_phase1 > 0 {
            log_info(&format!(
                "{} file(s) were skipped by Phase 1 and have no AST (see the .tca diagnostics)",
                skipped_by_phase1
            ));
        }
        for (lang, count) in &lexical_only {
            log_warn(&format!(
                "{} {:?} file(s) are lexical-only: no {:?} grammar is linked, so they contribute \
                 tokens but no AST",
                count, lang, lang
            ));
        }
        for (lang, count) in &no_adapter {
            log_warn(&format!(
                "{} {:?} file(s) have no Phase 2 reduction adapter and get no AST",
                count, lang
            ));
        }

        let workers = resolve_worker_count(options.worker_threads, jobs.len());
        let mut builder = BPASTBuilder::new(tok_table.len(), tca_hash);
        let mut totals = ReductionStats::default();

        // Parse in parallel (bounded batches), reduce sequentially in file order.
        let batch_len = workers.max(1) * 8;
        for batch in jobs.chunks(batch_len) {
            let parsed = parallel_map_with(
                batch,
                workers,
                || TreeSitterParser::with_timeout(options.parse_timeout_micros),
                |parser, _, job| Self::load_and_parse(job, parser),
            );
            for (job, result) in batch.iter().zip(parsed) {
                let (source, tree) = result?;
                crate::oh_trace!(
                    "Reducing file_id={} ({}) with the {:?} adapter...",
                    job.file_id,
                    job.path.display(),
                    job.lang
                );
                let reduction = reduce_file(
                    tree.root_node(),
                    &source,
                    job.file_id,
                    job.adapter.as_ref(),
                    tok_table,
                    &mut builder,
                );
                if reduction.stats.error_regions > 0 || reduction.stats.missing > 0 {
                    crate::oh_debug!(
                        "  {}: {} ERROR region(s) eliminated into their parents, {} MISSING node(s) skipped",
                        job.path.display(),
                        reduction.stats.error_regions,
                        reduction.stats.missing
                    );
                }
                totals.kept += reduction.stats.kept;
                totals.eliminated += reduction.stats.eliminated;
                totals.dropped += reduction.stats.dropped;
                totals.error_regions += reduction.stats.error_regions;
                totals.missing += reduction.stats.missing;
            }
        }

        log_info("Building succinct navigation indexes (rank/select, range min-max tree)...");
        let artifact = builder
            .try_finalize()
            .map_err(|e| io::Error::other(format!("Phase 2 finalization failed: {}", e)))?;

        let usage = artifact.memory_usage();
        crate::oh_debug!(
            "Succinct Bitstring: bit_count={}, node_count={}, in-memory bytes={} (topology {} bits/node)",
            artifact.bp_encoder.bit_count,
            artifact.node_count,
            usage.total(),
            usage
                .topology_bits_per_node(artifact.node_count)
                .map(|b| format!("{:.2}", b))
                .unwrap_or_else(|| "n/a".to_string())
        );

        BPASTSerializer::write(&artifact, out_path)?;
        let summary = format!(
            "Encoded {} BP AST nodes from {} file(s) ({} CST nodes eliminated, {} trivia subtrees dropped, {} ERROR regions) to {}.",
            artifact.node_count,
            jobs.len(),
            totals.eliminated,
            totals.dropped,
            totals.error_regions,
            out_path.display()
        );
        log_info(&format!("Phase 2 Complete: {}", summary));
        timer.finish(&summary);

        Ok(artifact)
    }

    /// Re-reads one file, verifies it against the Phase 1 digest and parses it.
    fn load_and_parse(
        job: &ReduceJob,
        parser: &mut Result<TreeSitterParser, String>,
    ) -> io::Result<(Vec<u8>, tree_sitter::Tree)> {
        let source = fs::read(&job.path).map_err(|e| {
            io::Error::new(
                e.kind(),
                format!(
                    "Phase 2 cannot read {} (ingested by Phase 1): {}",
                    job.path.display(),
                    e
                ),
            )
        })?;
        let digest: [u8; 32] = Sha256::digest(&source).into();
        if digest != job.sha256 {
            return Err(invalid_data(format!(
                "{} changed after Phase 1 (SHA-256 mismatch); its AST would disagree with the token corpus. Re-run the analysis.",
                job.path.display()
            )));
        }
        let parser = parser.as_mut().map_err(|e| io::Error::other(e.clone()))?;
        let tree = parser
            .parse_incremental(&source, job.adapter.ts_language(), None)
            .map_err(|e| {
                io::Error::other(format!(
                    "Phase 2 could not parse {}: {}",
                    job.path.display(),
                    e
                ))
            })?;
        Ok((source, tree))
    }
}
