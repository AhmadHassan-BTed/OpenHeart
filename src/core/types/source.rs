use std::collections::HashMap;
use std::ffi::OsString;
use std::path::PathBuf;

use crate::core::types::token::LangId;

/// Source File Record (64 bytes exact binary layout).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct SourceFileRecord {
    pub file_id: u16,
    pub language_id: u8,
    pub flags: u8,
    pub content_sha256: [u8; 32],
    pub path_str_offset: u32,
    pub file_size_bytes: u64,
    pub mtime_ns: u64,
    pub first_token_id: u32,
    pub file_token_count: u32,
}

/// Token filtering rules for lexical ingestion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenFilter {
    pub include_whitespace: bool,
    pub include_line_comments: bool,
    pub include_block_comments: bool,
    pub include_doc_comments: bool,
}

impl Default for TokenFilter {
    fn default() -> Self {
        Self {
            include_whitespace: false,
            include_line_comments: false,
            include_block_comments: false,
            include_doc_comments: true,
        }
    }
}

/// Sole input manifest for Phase 1.
#[derive(Debug, Clone)]
pub struct SourceManifest {
    pub file_paths: Vec<PathBuf>,
    pub language_overrides: HashMap<OsString, LangId>,
    pub filter: TokenFilter,
}

impl SourceManifest {
    pub fn new(file_paths: Vec<PathBuf>) -> Self {
        Self {
            file_paths,
            language_overrides: HashMap::new(),
            filter: TokenFilter::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_source_file_record_layout() {
        assert_eq!(std::mem::size_of::<SourceFileRecord>(), 64);
    }
}
