//! Phase 1 Lexical Ingestion Module.

pub mod adapter;
pub mod allocator;
pub mod builder;
pub mod interner;
pub mod manifest;
pub mod parser;
pub mod serializer;
pub mod walker;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::UNIX_EPOCH;

use sha2::{Digest, Sha256};

use crate::core::logger::{log_debug, log_info, log_trace, PhaseTimer};
use crate::core::types::source::SourceFileRecord;
use crate::core::types::token::{build_sort_key, TokenRecord};
use crate::ingestion::adapter::registry::AdapterRegistry;
use crate::ingestion::allocator::TokenIdAllocator;
use crate::ingestion::builder::TokenCorpusBuilder;
use crate::ingestion::interner::StringInterner;
use crate::ingestion::manifest::SourceManifest;
use crate::ingestion::parser::tree_sitter::TreeSitterParser;
use crate::ingestion::parser::CSTParser;
pub use crate::ingestion::serializer::{TokenCorpusArtifact, TokenCorpusSerializer};
use crate::ingestion::walker::walk_cst;

pub struct IngestionStage;

impl IngestionStage {
    pub fn run(manifest: SourceManifest, out_path: &Path) -> Result<TokenCorpusArtifact, String> {
        let _timer = PhaseTimer::start("Phase 1: Lexical Ingestion");

        let mut files: Vec<PathBuf> = manifest.file_paths.clone();
        files.sort_unstable();

        log_info(&format!(
            "Ingesting {} source files from manifest...",
            files.len()
        ));

        let mut file_records: Vec<SourceFileRecord> = Vec::with_capacity(files.len());
        let adapter_reg = AdapterRegistry::new();
        let mut source_hashes: Vec<[u8; 32]> = Vec::with_capacity(files.len());

        for (file_id, path) in files.iter().enumerate() {
            let meta = fs::metadata(path)
                .map_err(|e| format!("Failed to read metadata for {:?}: {}", path, e))?;
            let content = fs::read(path)
                .map_err(|e| format!("Failed to read source file {:?}: {}", path, e))?;

            let mut hasher = Sha256::new();
            hasher.update(&content);
            let sha256_result: [u8; 32] = hasher.finalize().into();
            source_hashes.push(sha256_result);

            let lang_id = AdapterRegistry::detect(&manifest.language_overrides, path);
            let mtime_ns = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0);

            file_records.push(SourceFileRecord {
                file_id: file_id as u16,
                language_id: lang_id as u8,
                flags: 0,
                content_sha256: sha256_result,
                path_str_offset: 0,
                file_size_bytes: meta.len(),
                mtime_ns,
                first_token_id: 0,
                file_token_count: 0,
            });

            log_trace(&format!(
                "File record {}: path={}, size={} bytes, lang_id={:?}, SHA256={:02x}{:02x}...",
                file_id,
                path.display(),
                meta.len(),
                lang_id,
                sha256_result[0],
                sha256_result[1]
            ));
        }

        // Compute overall source tree hash
        let mut tree_hasher = Sha256::new();
        for hash in &source_hashes {
            tree_hasher.update(hash);
        }
        let source_tree_hash: [u8; 32] = tree_hasher.finalize().into();

        log_debug(&format!(
            "Computed overall source tree hash: {:02x}{:02x}{:02x}{:02x}...",
            source_tree_hash[0], source_tree_hash[1], source_tree_hash[2], source_tree_hash[3]
        ));

        let allocator = TokenIdAllocator::new();
        let interner = Mutex::new(StringInterner::with_capacity(65536));
        let mut builder = TokenCorpusBuilder::new();

        let mut parser = TreeSitterParser::new()?;

        for (file_id, (path, record)) in files.iter().zip(&mut file_records).enumerate() {
            let source = fs::read(path).map_err(|e| format!("Read error: {}", e))?;
            let lang_id = crate::core::types::token::LangId::from(record.language_id);
            let adapter = adapter_reg
                .get(lang_id)
                .ok_or_else(|| format!("No adapter registered for language ID {:?}", lang_id))?;

            let tree = parser.parse(&source, adapter.ts_language())?;
            let first_id = allocator.current();

            let raw_tokens = walk_cst(
                tree.root_node(),
                &source,
                file_id as u16,
                adapter.as_ref(),
                &manifest.filter,
            );

            log_debug(&format!(
                "  File {} ({}): Walked CST, extracted {} tokens",
                file_id,
                path.display(),
                raw_tokens.len()
            ));

            {
                let mut intern = interner.lock().unwrap();
                for rt in &raw_tokens {
                    let token_id = allocator.next_id();
                    let text = &source[rt.text_start..rt.text_start + rt.text_len];
                    let text_id = intern.intern(text);
                    let sort_key = build_sort_key(rt.file_id, rt.line, rt.col);

                    builder.push(
                        token_id,
                        TokenRecord {
                            sort_key,
                            text_id,
                            len: rt.len,
                            token_type: rt.token_type.as_u8(),
                            _padding: 0,
                        },
                    );
                }
            }

            record.first_token_id = first_id;
            record.file_token_count = raw_tokens.len() as u32;
        }

        log_info("Finalizing TokenCorpusBuilder and asserting Invariants 1-4...");
        let artifact = builder.finalize(file_records, interner.into_inner().unwrap())?;

        let flags: u16 = (manifest.filter.include_whitespace as u16)
            | ((manifest.filter.include_line_comments as u16) << 1)
            | ((manifest.filter.include_block_comments as u16) << 2)
            | ((manifest.filter.include_doc_comments as u16) << 3);

        log_info(&format!(
            "Serializing .tca binary artifact to {}...",
            out_path.display()
        ));
        TokenCorpusSerializer::write(&artifact, &files, flags, source_tree_hash, out_path)?;

        log_info(&format!(
            "Phase 1 Ingestion Complete: Ingested {} tokens across {} source files.",
            artifact.token_records.len(),
            artifact.file_records.len()
        ));

        Ok(artifact)
    }
}
