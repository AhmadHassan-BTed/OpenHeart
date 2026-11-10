//! Phase 1 Lexical Ingestion Module.
//!
//! Each manifest file is read exactly once: the same buffer is hashed (SHA-256), checked
//! for BOM / CRLF, and tokenised by the language's frontend (a tree-sitter grammar or a
//! dedicated lexer). Files are parsed in parallel on scoped worker threads; tokens are then
//! committed sequentially in file order, so token ids, text ids and the resulting `.tca`
//! bytes do not depend on the thread count.
//!
//! Nothing is dropped silently: unsupported languages, oversized files, parse timeouts and
//! token positions that the sort-key layout cannot encode each mark the file as skipped
//! with a reason in its `FileDiagnostics` and a warning in the log.
//!
//! Sources come from a [`SourceProvider`]: the file system (CLI, server) or memory
//! ([`IngestionStage::run_in_memory`], the WebAssembly build). In memory, paths are taken
//! exactly as supplied, nothing is canonicalized, and the corpus is encoded into bytes
//! without being written anywhere.

#![forbid(unsafe_code)]

pub mod adapter;
pub mod allocator;
pub mod builder;
pub mod interner;
pub mod manifest;
pub mod parser;
pub mod serializer;
pub mod sources;
pub mod walker;

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::core::logger::{log_info, log_warn, PhaseTimer};
use crate::core::parallel::{parallel_map_with, resolve_worker_count};
use crate::core::types::source::{
    FileDiagnostics, SkipReason, SourceFileRecord, FILE_FLAG_CRLF, FILE_FLAG_HAS_BOM,
    FILE_FLAG_LEXICAL_ONLY, FILE_FLAG_LONG_TOKENS, FILE_FLAG_MISSING_NODES,
    FILE_FLAG_PARSE_ABORTED, FILE_FLAG_SKIPPED, FILE_FLAG_SYNTAX_ERRORS,
};
use crate::core::types::token::{
    build_sort_key, try_build_sort_key, LangId, TokenRecord, TOKEN_FLAG_LEN_SATURATED,
};
use crate::ingestion::adapter::registry::{AdapterRegistry, Frontend};
use crate::ingestion::allocator::TokenIdAllocator;
use crate::ingestion::builder::TokenCorpusBuilder;
use crate::ingestion::interner::StringInterner;
use crate::ingestion::manifest::SourceManifest;
use crate::ingestion::parser::tree_sitter::TreeSitterParser;
use crate::ingestion::parser::ParseError;
pub use crate::ingestion::serializer::{crc64_ecma, TokenCorpusArtifact, TokenCorpusSerializer};
pub use crate::ingestion::sources::{MemorySources, SourceProvider};
use crate::ingestion::walker::{walk_cst_with_diagnostics, RawToken};

/// Maximum number of files a corpus can hold: file ids are 16-bit in the sort-key layout.
pub const MAX_CORPUS_FILES: usize = u16::MAX as usize;

/// Upper bound on source bytes held in memory between parsing and committing a batch.
const BATCH_BYTE_BUDGET: u64 = 256 * 1024 * 1024;

pub struct IngestionStage;

/// Results of Phase 1 beyond the token corpus itself.
#[derive(Debug, Clone)]
pub struct IngestionReport {
    /// One entry per file, in file-id order.
    pub diagnostics: Vec<FileDiagnostics>,
    /// Ingested paths in file-id order (sorted, duplicates removed).
    pub file_paths: Vec<PathBuf>,
    /// Manifest entries dropped because they resolve to an already-listed file.
    pub duplicate_paths_removed: usize,
    /// The exact `.tca` bytes (written to the output path when there is one).
    pub tca_bytes: Vec<u8>,
}

impl IngestionReport {
    /// CRC-64 of the written `.tca`, the hash link Phase 2 records.
    pub fn tca_hash(&self) -> u64 {
        crc64_ecma(&self.tca_bytes)
    }

    pub fn skipped_files(&self) -> usize {
        self.diagnostics
            .iter()
            .filter(|d| d.skip_reason != SkipReason::None)
            .count()
    }
}

struct FileJob {
    file_id: u16,
    path: PathBuf,
    lang: LangId,
    size: u64,
    mtime_ns: u64,
}

struct FileOutcome {
    sha256: [u8; 32],
    flags: u8,
    diagnostics: FileDiagnostics,
    /// Source bytes and the tokens referencing them (empty for skipped files).
    source: Vec<u8>,
    tokens: Vec<RawToken>,
}

impl IngestionStage {
    pub fn run(manifest: SourceManifest, out_path: &Path) -> Result<TokenCorpusArtifact, String> {
        Self::run_with_report(manifest, out_path).map(|(artifact, _)| artifact)
    }

    /// Runs Phase 1 with the built-in frontends and returns the corpus together with
    /// per-file diagnostics and the written `.tca` bytes.
    pub fn run_with_report(
        manifest: SourceManifest,
        out_path: &Path,
    ) -> Result<(TokenCorpusArtifact, IngestionReport), String> {
        Self::run_with_registry(manifest, out_path, &AdapterRegistry::new())
    }

    /// Runs Phase 1 with a caller-supplied frontend registry (for example one with an
    /// additional grammar registered through `AdapterRegistry::register`).
    pub fn run_with_registry(
        manifest: SourceManifest,
        out_path: &Path,
        adapter_reg: &AdapterRegistry,
    ) -> Result<(TokenCorpusArtifact, IngestionReport), String> {
        Self::run_with_provider(manifest, Some(out_path), adapter_reg, SourceProvider::Disk)
    }

    /// Runs Phase 1 over in-memory sources without touching the file system: the manifest
    /// lists the paths held by `sources`, and the `.tca` bytes are returned in the report
    /// instead of being written.
    pub fn run_in_memory(
        manifest: SourceManifest,
        sources: &MemorySources,
    ) -> Result<(TokenCorpusArtifact, IngestionReport), String> {
        Self::run_with_provider(
            manifest,
            None,
            &AdapterRegistry::new(),
            SourceProvider::Memory(sources),
        )
    }

    /// Runs Phase 1 reading every source through `provider`. The encoded corpus is
    /// written to `out_path` when one is given and is always returned in the report.
    pub fn run_with_provider(
        manifest: SourceManifest,
        out_path: Option<&Path>,
        adapter_reg: &AdapterRegistry,
        provider: SourceProvider<'_>,
    ) -> Result<(TokenCorpusArtifact, IngestionReport), String> {
        let timer = PhaseTimer::start("Phase 1: Lexical Ingestion");

        let (files, duplicate_paths_removed) = Self::resolve_files(&manifest.file_paths, provider)?;
        if files.len() > MAX_CORPUS_FILES {
            return Err(format!(
                "Manifest lists {} files; a .tca corpus holds at most {} (16-bit file ids)",
                files.len(),
                MAX_CORPUS_FILES
            ));
        }

        log_info(&format!(
            "Ingesting {} source files from manifest...",
            files.len()
        ));

        let mut jobs = Vec::with_capacity(files.len());
        for (idx, path) in files.iter().enumerate() {
            let stat = provider
                .stat(path)
                .map_err(|e| format!("Failed to read metadata for {:?}: {}", path, e))?;
            let mut lang = AdapterRegistry::detect(&manifest.language_overrides, path);
            if lang == LangId::Unknown {
                // Extensions claimed by frontends registered at runtime.
                if let Some(registered) = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .and_then(|e| adapter_reg.language_for_extension(e))
                {
                    lang = registered;
                }
            }
            jobs.push(FileJob {
                file_id: idx as u16,
                path: path.clone(),
                lang,
                size: stat.size,
                mtime_ns: stat.mtime_ns,
            });
        }

        let workers = resolve_worker_count(manifest.worker_threads, jobs.len());
        crate::oh_debug!("Phase 1 using {} worker thread(s)", workers);

        let allocator = TokenIdAllocator::new();
        let mut interner = StringInterner::with_capacity(65536);
        let mut builder = TokenCorpusBuilder::new();
        let mut file_records: Vec<SourceFileRecord> = Vec::with_capacity(jobs.len());
        let mut diagnostics: Vec<FileDiagnostics> = Vec::with_capacity(jobs.len());
        let mut source_hashes: Vec<[u8; 32]> = Vec::with_capacity(jobs.len());
        let mut unsupported: BTreeMap<LangId, usize> = BTreeMap::new();

        for batch in Self::batches(&jobs, workers) {
            let outcomes = parallel_map_with(
                batch,
                workers,
                || TreeSitterParser::with_timeout(manifest.parse_timeout_micros),
                |parser, _, job| Self::process_file(job, adapter_reg, parser, &manifest, provider),
            );

            for (job, outcome) in batch.iter().zip(outcomes) {
                let outcome = outcome?;
                if outcome.diagnostics.skip_reason == SkipReason::UnsupportedLanguage {
                    *unsupported.entry(job.lang).or_insert(0) += 1;
                } else if outcome.diagnostics.skip_reason != SkipReason::None {
                    log_warn(&format!(
                        "Skipped {}: {}",
                        job.path.display(),
                        outcome.diagnostics.detail
                    ));
                } else if outcome.diagnostics.error_count > 0
                    || outcome.diagnostics.missing_count > 0
                {
                    log_warn(&format!(
                        "{}: {} syntax error region(s), {} missing token(s); recovered tokens kept",
                        job.path.display(),
                        outcome.diagnostics.error_count,
                        outcome.diagnostics.missing_count
                    ));
                }

                let record = Self::commit_file(
                    job,
                    outcome.sha256,
                    outcome.flags,
                    &outcome.source,
                    &outcome.tokens,
                    &allocator,
                    &mut interner,
                    &mut builder,
                )?;
                crate::oh_debug!(
                    "  File {} ({}): {} tokens",
                    job.file_id,
                    job.path.display(),
                    record.file_token_count
                );
                source_hashes.push(outcome.sha256);
                file_records.push(record);
                diagnostics.push(outcome.diagnostics);
            }
        }

        for (lang, count) in &unsupported {
            log_warn(&format!(
                "Skipped {} {:?} file(s): no grammar or lexer is linked for {:?}; \
                 supported languages are {:?}",
                count,
                lang,
                lang,
                adapter_reg.supported_languages()
            ));
        }

        // Overall source tree hash
        let mut tree_hasher = Sha256::new();
        for hash in &source_hashes {
            tree_hasher.update(hash);
        }
        let source_tree_hash: [u8; 32] = tree_hasher.finalize().into();
        crate::oh_debug!(
            "Computed overall source tree hash: {:02x}{:02x}{:02x}{:02x}...",
            source_tree_hash[0],
            source_tree_hash[1],
            source_tree_hash[2],
            source_tree_hash[3]
        );

        log_info("Finalizing TokenCorpusBuilder and asserting Invariants 1-4...");
        let artifact = builder.finalize(file_records, interner)?;

        let flags: u16 = (manifest.filter.include_whitespace as u16)
            | ((manifest.filter.include_line_comments as u16) << 1)
            | ((manifest.filter.include_block_comments as u16) << 2)
            | ((manifest.filter.include_doc_comments as u16) << 3);

        let tca_bytes = match out_path {
            Some(out_path) => {
                log_info(&format!(
                    "Serializing .tca binary artifact to {}...",
                    out_path.display()
                ));
                TokenCorpusSerializer::write(
                    &artifact,
                    &files,
                    flags,
                    source_tree_hash,
                    &diagnostics,
                    out_path,
                )?
            }
            None => {
                log_info("Encoding the .tca binary artifact in memory...");
                TokenCorpusSerializer::to_bytes(
                    &artifact,
                    &files,
                    flags,
                    source_tree_hash,
                    &diagnostics,
                )?
            }
        };

        let report = IngestionReport {
            diagnostics,
            file_paths: files,
            duplicate_paths_removed,
            tca_bytes,
        };

        let summary = format!(
            "Ingested {} tokens across {} source files ({} skipped).",
            artifact.token_records.len(),
            artifact.file_records.len(),
            report.skipped_files()
        );
        log_info(&format!("Phase 1 Ingestion Complete: {}", summary));
        timer.finish(&summary);

        Ok((artifact, report))
    }

    /// Sorts the manifest and removes entries that resolve to the same file (through
    /// `..`, symlinks or case-insensitive file systems), keeping the first spelling.
    /// In-memory sources have no file system to resolve against: they are identified by
    /// the exact path supplied, and every manifest entry must be held by the provider.
    fn resolve_files(
        paths: &[PathBuf],
        provider: SourceProvider<'_>,
    ) -> Result<(Vec<PathBuf>, usize), String> {
        let mut sorted: Vec<PathBuf> = paths.to_vec();
        sorted.sort_unstable();
        let mut seen: HashSet<PathBuf> = HashSet::with_capacity(sorted.len());
        let mut files = Vec::with_capacity(sorted.len());
        let mut duplicates = 0usize;
        for path in sorted {
            let canonical = match provider {
                SourceProvider::Disk => fs::canonicalize(&path)
                    .map_err(|e| format!("Failed to resolve source file {:?}: {}", path, e))?,
                SourceProvider::Memory(sources) => {
                    if !sources.contains(&path.to_string_lossy()) {
                        return Err(format!(
                            "Source file {:?} is not among the in-memory sources",
                            path
                        ));
                    }
                    path.clone()
                }
            };
            if seen.insert(canonical) {
                files.push(path);
            } else {
                duplicates += 1;
            }
        }
        if duplicates > 0 {
            log_warn(&format!(
                "Removed {} duplicate manifest entr{} that resolve to an already-listed file",
                duplicates,
                if duplicates == 1 { "y" } else { "ies" }
            ));
        }
        Ok((files, duplicates))
    }

    /// Splits jobs into batches bounded by count and by total file size, so the sources of
    /// a batch fit in memory while it is parsed and committed.
    fn batches(jobs: &[FileJob], workers: usize) -> Vec<&[FileJob]> {
        let max_count = workers.max(1) * 8;
        let mut out = Vec::new();
        let mut start = 0;
        let mut bytes: u64 = 0;
        for (i, job) in jobs.iter().enumerate() {
            let count = i - start;
            if count > 0
                && (count >= max_count || bytes.saturating_add(job.size) > BATCH_BYTE_BUDGET)
            {
                out.push(&jobs[start..i]);
                start = i;
                bytes = 0;
            }
            bytes = bytes.saturating_add(job.size);
        }
        if start < jobs.len() {
            out.push(&jobs[start..]);
        }
        out
    }

    fn skipped(
        sha256: [u8; 32],
        flags: u8,
        reason: SkipReason,
        mut diagnostics: FileDiagnostics,
        detail: String,
    ) -> FileOutcome {
        diagnostics.skip_reason = reason;
        diagnostics.detail = detail;
        let extra = if reason == SkipReason::ParseAborted {
            FILE_FLAG_PARSE_ABORTED
        } else {
            0
        };
        FileOutcome {
            sha256,
            flags: flags | FILE_FLAG_SKIPPED | extra,
            diagnostics,
            source: Vec::new(),
            tokens: Vec::new(),
        }
    }

    /// Reads, hashes and tokenises one file. Runs on a worker thread.
    /// `Err` is reserved for failures that must abort the run (unreadable files, a grammar
    /// that cannot be loaded); everything else is reported through the outcome.
    fn process_file(
        job: &FileJob,
        registry: &AdapterRegistry,
        parser: &mut Result<TreeSitterParser, String>,
        manifest: &SourceManifest,
        provider: SourceProvider<'_>,
    ) -> Result<FileOutcome, String> {
        let diagnostics = FileDiagnostics::new(job.file_id);

        let Some(frontend) = registry.frontend(job.lang) else {
            return Ok(Self::skipped(
                [0u8; 32],
                0,
                SkipReason::UnsupportedLanguage,
                diagnostics,
                format!("no grammar or lexer is linked for {:?}", job.lang),
            ));
        };

        if job.size > manifest.max_file_size_bytes {
            return Ok(Self::skipped(
                [0u8; 32],
                0,
                SkipReason::TooLarge,
                diagnostics,
                format!(
                    "{} bytes exceeds the {}-byte limit (max_file_size_bytes)",
                    job.size, manifest.max_file_size_bytes
                ),
            ));
        }

        let source = provider
            .read(&job.path)
            .map_err(|e| format!("Failed to read source file {:?}: {}", job.path, e))?;
        if source.len() as u64 > manifest.max_file_size_bytes {
            // The file grew between metadata and read.
            return Ok(Self::skipped(
                [0u8; 32],
                0,
                SkipReason::TooLarge,
                diagnostics,
                format!(
                    "{} bytes exceeds the {}-byte limit (max_file_size_bytes)",
                    source.len(),
                    manifest.max_file_size_bytes
                ),
            ));
        }

        let sha256: [u8; 32] = Sha256::digest(&source).into();
        let mut flags = 0u8;
        if source.starts_with(b"\xEF\xBB\xBF") {
            flags |= FILE_FLAG_HAS_BOM;
        }
        if source.windows(2).any(|w| w == b"\r\n") {
            flags |= FILE_FLAG_CRLF;
        }

        let mut diagnostics = diagnostics;
        let tokens = match frontend {
            Frontend::Grammar(adapter) => {
                let parser = parser.as_mut().map_err(|e| e.clone())?;
                match parser.parse_incremental(&source, adapter.ts_language(), None) {
                    Ok(tree) => walk_cst_with_diagnostics(
                        tree.root_node(),
                        &source,
                        job.file_id,
                        adapter.as_ref(),
                        &manifest.filter,
                        &mut diagnostics,
                    ),
                    Err(ParseError::Language(e)) => {
                        return Err(format!(
                            "Cannot load the {:?} grammar for {:?}: {}",
                            job.lang, job.path, e
                        ))
                    }
                    Err(e) => {
                        return Ok(Self::skipped(
                            sha256,
                            flags,
                            SkipReason::ParseAborted,
                            diagnostics,
                            e.to_string(),
                        ))
                    }
                }
            }
            Frontend::Lexer(lexer) => {
                flags |= FILE_FLAG_LEXICAL_ONLY;
                lexer.lex(&source, job.file_id, &manifest.filter, &mut diagnostics)
            }
        };

        if diagnostics.error_count > 0 {
            flags |= FILE_FLAG_SYNTAX_ERRORS;
        }
        if diagnostics.missing_count > 0 {
            flags |= FILE_FLAG_MISSING_NODES;
        }

        // Every token position must fit the packed sort key; a wrapped key would collide
        // with another token and corrupt every token-id lookup.
        for tok in &tokens {
            if let Err(e) = try_build_sort_key(job.file_id as u32, tok.line, tok.col) {
                let detail = format!(
                    "token at line {} byte column {} cannot be encoded: {}",
                    tok.line, tok.col, e
                );
                return Ok(Self::skipped(
                    sha256,
                    flags,
                    SkipReason::UnrepresentablePosition,
                    diagnostics,
                    detail,
                ));
            }
            if tok.text_len > u16::MAX as usize {
                flags |= FILE_FLAG_LONG_TOKENS;
            }
        }

        Ok(FileOutcome {
            sha256,
            flags,
            diagnostics,
            source,
            tokens,
        })
    }

    /// Commits one file's tokens to the corpus in file order. Runs on the calling thread.
    #[allow(clippy::too_many_arguments)]
    fn commit_file(
        job: &FileJob,
        sha256: [u8; 32],
        flags: u8,
        source: &[u8],
        tokens: &[RawToken],
        allocator: &TokenIdAllocator,
        interner: &mut StringInterner,
        builder: &mut TokenCorpusBuilder,
    ) -> Result<SourceFileRecord, String> {
        let path_str = job.path.to_string_lossy();
        let path_text_id = interner.intern(path_str.as_bytes());

        let first_id = allocator.current();
        let token_count = u32::try_from(tokens.len())
            .ok()
            .filter(|n| first_id.checked_add(*n).is_some())
            .ok_or_else(|| {
                format!(
                    "Token corpus exceeds {} tokens while ingesting {:?}",
                    u32::MAX,
                    job.path
                )
            })?;

        for rt in tokens {
            let token_id = allocator.next_id();
            let text = &source[rt.text_start..rt.text_start + rt.text_len];
            let text_id = interner.intern(text);
            // Positions were validated by `process_file`.
            let sort_key = build_sort_key(rt.file_id, rt.line, rt.col as u16);
            let (len, token_flags) = match u16::try_from(rt.text_len) {
                Ok(len) => (len, 0u8),
                Err(_) => (u16::MAX, TOKEN_FLAG_LEN_SATURATED),
            };
            builder.push(
                token_id,
                TokenRecord {
                    sort_key,
                    text_id,
                    len,
                    token_type: rt.token_type.as_u8(),
                    _padding: token_flags,
                },
            );
        }

        Ok(SourceFileRecord {
            file_id: job.file_id,
            language_id: job.lang as u8,
            flags,
            content_sha256: sha256,
            path_str_offset: path_text_id,
            file_size_bytes: job.size,
            mtime_ns: job.mtime_ns,
            first_token_id: first_id,
            file_token_count: token_count,
        })
    }
}
