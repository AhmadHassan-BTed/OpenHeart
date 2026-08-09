//! Phase 2: CST Reduction & Balanced Parentheses AST Encoding Module.

pub mod adapter;
pub mod bp_encoder;
pub mod builder;
pub mod jump_table;
pub mod preorder;
pub mod rank_select;
pub mod reducer;
pub mod rmq;
pub mod serializer;

use crate::ingestion::serializer::crc64_ecma;
use std::fs;
use std::path::Path;

use crate::core::io::mmap::MemoryMappedFile;
use crate::core::logger::{log_debug, log_info, log_trace, PhaseTimer};
use crate::core::types::token::TokenRecord;
use crate::ingestion::adapter::registry::AdapterRegistry as Phase1Registry;
use crate::ingestion::parser::tree_sitter::TreeSitterParser;
use crate::ingestion::parser::CSTParser;
use adapter::registry::ASTAdapterRegistry;
pub use builder::{BPASTArtifact, BPASTBuilder};
use reducer::reduce_and_encode;
use serializer::BPASTSerializer;

pub const TCA_HEADER_SIZE: usize = 64;

pub struct ASTStageInput {
    pub tca: MemoryMappedFile,
}

pub struct ASTStage;

impl ASTStage {
    pub fn run(input: &ASTStageInput, out_path: &Path) -> std::io::Result<BPASTArtifact> {
        let _timer = PhaseTimer::start("Phase 2: CST Reduction & BP AST Encoding");

        let tca_bytes = input.tca.as_slice();
        if tca_bytes.len() < TCA_HEADER_SIZE {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "TCA file too small",
            ));
        }

        let tca_hash = crc64_ecma(tca_bytes);
        log_info(&format!(
            "TCA binary hash link computed: 0x{:016X}",
            tca_hash
        ));

        let _phase1_registry = Phase1Registry::new();
        let ast_registry = ASTAdapterRegistry::new();
        let mut parser = TreeSitterParser::new()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

        let token_count = u32::from_le_bytes(tca_bytes[12..16].try_into().unwrap()) as usize;
        let file_count = u16::from_le_bytes(tca_bytes[16..18].try_into().unwrap()) as usize;
        let mut offset = TCA_HEADER_SIZE;

        log_debug(&format!(
            "TCA Header: token_count={}, file_count={}",
            token_count, file_count
        ));

        let mut file_recs = Vec::with_capacity(file_count);
        for _ in 0..file_count {
            if offset + 64 > tca_bytes.len() {
                break;
            }
            let file_id = u16::from_le_bytes(tca_bytes[offset..offset + 2].try_into().unwrap());
            let lang_id = tca_bytes[offset + 2].into();
            let path_str_offset =
                u32::from_le_bytes(tca_bytes[offset + 36..offset + 40].try_into().unwrap())
                    as usize;
            file_recs.push((file_id, lang_id, path_str_offset));
            offset += 64;
        }

        let paths_start = offset;
        let mut source_paths = Vec::with_capacity(file_count);
        let mut paths_end = paths_start;

        for (file_id, lang_id, path_off) in file_recs {
            let p_offset = paths_start + path_off;
            if p_offset + 2 <= tca_bytes.len() {
                let len = u16::from_le_bytes(tca_bytes[p_offset..p_offset + 2].try_into().unwrap())
                    as usize;
                if p_offset + 2 + len <= tca_bytes.len() {
                    if let Ok(path_str) =
                        std::str::from_utf8(&tca_bytes[p_offset + 2..p_offset + 2 + len])
                    {
                        source_paths.push((file_id, lang_id, path_str.to_string()));
                        paths_end = paths_end.max(p_offset + 2 + len);
                    }
                }
            }
        }

        let remainder = paths_end % 8;
        if remainder != 0 {
            paths_end += 8 - remainder;
        }
        offset = paths_end;

        let mut tok_table = Vec::with_capacity(token_count);
        for _ in 0..token_count {
            if offset + 16 <= tca_bytes.len() {
                let sort_key =
                    u64::from_le_bytes(tca_bytes[offset..offset + 8].try_into().unwrap());
                let text_id =
                    u32::from_le_bytes(tca_bytes[offset + 8..offset + 12].try_into().unwrap());
                let len =
                    u16::from_le_bytes(tca_bytes[offset + 12..offset + 14].try_into().unwrap());
                let token_type = tca_bytes[offset + 14];

                tok_table.push(TokenRecord {
                    sort_key,
                    text_id,
                    len,
                    token_type,
                    _padding: 0,
                });
                offset += 16;
            }
        }

        log_info(&format!(
            "Extracted {} sorted token records from TCA table for AST token-range mapping.",
            tok_table.len()
        ));

        let mut builder = BPASTBuilder::new(token_count * 2, tca_hash);

        for (file_id, lang_id, path_str) in source_paths {
            log_trace(&format!(
                "Reducing file_id={} ({}) with AST adapter...",
                file_id, path_str
            ));
            if let Some(ast_adapter) = ast_registry.get(lang_id) {
                if let Ok(source) = fs::read(&path_str) {
                    if let Ok(tree) = parser.parse(&source, ast_adapter.ts_language()) {
                        reduce_and_encode(
                            tree.root_node(),
                            &source,
                            file_id,
                            ast_adapter.as_ref(),
                            &tok_table,
                            &mut builder,
                        );
                    }
                }
            }
        }

        log_info("Building auxiliary succinct structures (JumpTable, RankSelectIndex, SparseTableRMQ)...");
        let artifact = builder.finalize();

        log_debug(&format!(
            "Succinct Bitstring: bit_count={}, node_count={}",
            artifact.bp_encoder.bit_count, artifact.node_count
        ));

        BPASTSerializer::write(&artifact, out_path)?;
        log_info(&format!(
            "Phase 2 Complete: Encoded {} BP AST nodes to {}.",
            artifact.node_count,
            out_path.display()
        ));

        Ok(artifact)
    }
}
