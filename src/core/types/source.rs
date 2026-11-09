use std::collections::HashMap;
use std::ffi::OsString;
use std::path::PathBuf;

use crate::core::types::token::LangId;

// ── SourceFileRecord.flags bits (per-file lexical diagnostics) ──

/// The file starts with a UTF-8 byte-order mark.
pub const FILE_FLAG_HAS_BOM: u8 = 1 << 0;
/// The file uses CRLF line terminators (at least one `\r\n`).
pub const FILE_FLAG_CRLF: u8 = 1 << 1;
/// The parser recovered from syntax errors (one or more ERROR nodes).
pub const FILE_FLAG_SYNTAX_ERRORS: u8 = 1 << 2;
/// The parser inserted MISSING nodes (zero-width tokens absent from the source).
pub const FILE_FLAG_MISSING_NODES: u8 = 1 << 3;
/// Tokens come from a lexer-only frontend; no syntax tree exists for later phases.
pub const FILE_FLAG_LEXICAL_ONLY: u8 = 1 << 4;
/// The file contributed no tokens; the reason is in the file's `FileDiagnostics`.
pub const FILE_FLAG_SKIPPED: u8 = 1 << 5;
/// Parsing was aborted (timeout or cancellation); the file contributed no tokens.
pub const FILE_FLAG_PARSE_ABORTED: u8 = 1 << 6;
/// At least one token is longer than `u16::MAX` bytes (its record `len` is saturated).
pub const FILE_FLAG_LONG_TOKENS: u8 = 1 << 7;

/// Source File Record (64 bytes exact binary layout).
///
/// `path_str_offset` is the interner `text_id` of the file path, in memory and on disk.
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

impl SourceFileRecord {
    #[inline]
    pub fn has_flag(&self, flag: u8) -> bool {
        (self.flags & flag) != 0
    }

    /// True when the file has tokens and a syntax tree that later phases can rebuild.
    #[inline]
    pub fn has_syntax_tree(&self) -> bool {
        !self.has_flag(FILE_FLAG_SKIPPED | FILE_FLAG_PARSE_ABORTED | FILE_FLAG_LEXICAL_ONLY)
    }
}

/// Why a file contributed no tokens to the corpus.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SkipReason {
    None = 0,
    /// No grammar or lexer is linked for the detected language.
    UnsupportedLanguage = 1,
    /// The file exceeds the manifest's `max_file_size_bytes`.
    TooLarge = 2,
    /// A token position cannot be encoded in a sort key (line > 2^24-1 or byte column > 65535).
    UnrepresentablePosition = 3,
    /// The parser gave up (timeout or cancellation).
    ParseAborted = 4,
}

impl From<u8> for SkipReason {
    fn from(val: u8) -> Self {
        match val {
            1 => SkipReason::UnsupportedLanguage,
            2 => SkipReason::TooLarge,
            3 => SkipReason::UnrepresentablePosition,
            4 => SkipReason::ParseAborted,
            _ => SkipReason::None,
        }
    }
}

/// Maximum number of syntax-error byte ranges retained per file.
pub const MAX_RECORDED_ERROR_RANGES: usize = 16;

/// Per-file diagnostics recorded during Phase 1 and persisted in the `.tca` DIAG section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDiagnostics {
    pub file_id: u16,
    pub skip_reason: SkipReason,
    /// Number of ERROR nodes in the concrete syntax tree.
    pub error_count: u32,
    /// Number of MISSING nodes the parser inserted.
    pub missing_count: u32,
    /// Byte ranges `[start, end)` of the first `MAX_RECORDED_ERROR_RANGES` ERROR / MISSING nodes.
    pub error_ranges: Vec<(u32, u32)>,
    /// Human-readable detail for skipped or aborted files (empty otherwise).
    pub detail: String,
}

impl FileDiagnostics {
    pub fn new(file_id: u16) -> Self {
        Self {
            file_id,
            skip_reason: SkipReason::None,
            error_count: 0,
            missing_count: 0,
            error_ranges: Vec::new(),
            detail: String::new(),
        }
    }

    /// Records one ERROR or MISSING region.
    pub fn record_error_range(&mut self, start: usize, end: usize) {
        if self.error_ranges.len() < MAX_RECORDED_ERROR_RANGES {
            let s = u32::try_from(start).unwrap_or(u32::MAX);
            let e = u32::try_from(end).unwrap_or(u32::MAX);
            self.error_ranges.push((s, e));
        }
    }

    pub fn is_clean(&self) -> bool {
        self.skip_reason == SkipReason::None && self.error_count == 0 && self.missing_count == 0
    }
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

/// Default cap on a single source file (bytes). Larger files are skipped with a diagnostic.
pub const DEFAULT_MAX_FILE_SIZE_BYTES: u64 = 16 * 1024 * 1024;
/// Default per-file parse budget (microseconds). 0 disables the timeout.
pub const DEFAULT_PARSE_TIMEOUT_MICROS: u64 = 30_000_000;

/// Environment variable overriding the worker-thread count (0 or unset = one per core).
pub const ENV_NUM_THREADS: &str = "OPENHEART_NUM_THREADS";

/// Reads `OPENHEART_NUM_THREADS`; `0`, unset or unparsable means "one worker per core".
pub fn worker_threads_from_env() -> usize {
    std::env::var(ENV_NUM_THREADS)
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(0)
}

/// Sole input manifest for Phase 1.
#[derive(Debug, Clone)]
pub struct SourceManifest {
    pub file_paths: Vec<PathBuf>,
    pub language_overrides: HashMap<OsString, LangId>,
    pub filter: TokenFilter,
    /// Files larger than this are skipped with `SkipReason::TooLarge`.
    pub max_file_size_bytes: u64,
    /// Per-file tree-sitter parse budget in microseconds (0 = unlimited).
    pub parse_timeout_micros: u64,
    /// Worker threads for parsing (0 = one per available core).
    pub worker_threads: usize,
}

impl SourceManifest {
    pub fn new(file_paths: Vec<PathBuf>) -> Self {
        Self {
            file_paths,
            language_overrides: HashMap::new(),
            filter: TokenFilter::default(),
            max_file_size_bytes: DEFAULT_MAX_FILE_SIZE_BYTES,
            parse_timeout_micros: DEFAULT_PARSE_TIMEOUT_MICROS,
            worker_threads: worker_threads_from_env(),
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
