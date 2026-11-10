use std::collections::HashMap;
use std::ffi::OsString;
use std::path::PathBuf;

use crate::core::types::source::{
    worker_threads_from_env, DEFAULT_MAX_FILE_SIZE_BYTES, DEFAULT_PARSE_TIMEOUT_MICROS,
};
pub use crate::core::types::source::{SourceManifest, TokenFilter};
use crate::core::types::token::LangId;

#[derive(Debug)]
pub struct SourceManifestBuilder {
    file_paths: Vec<PathBuf>,
    language_overrides: HashMap<OsString, LangId>,
    filter: TokenFilter,
    max_file_size_bytes: u64,
    parse_timeout_micros: u64,
    worker_threads: usize,
}

impl Default for SourceManifestBuilder {
    fn default() -> Self {
        Self {
            file_paths: Vec::new(),
            language_overrides: HashMap::new(),
            filter: TokenFilter::default(),
            max_file_size_bytes: DEFAULT_MAX_FILE_SIZE_BYTES,
            parse_timeout_micros: DEFAULT_PARSE_TIMEOUT_MICROS,
            worker_threads: worker_threads_from_env(),
        }
    }
}

impl SourceManifestBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_file<P: Into<PathBuf>>(mut self, path: P) -> Self {
        self.file_paths.push(path.into());
        self
    }

    pub fn add_files<I, P>(mut self, paths: I) -> Self
    where
        I: IntoIterator<Item = P>,
        P: Into<PathBuf>,
    {
        for p in paths {
            self.file_paths.push(p.into());
        }
        self
    }

    pub fn with_override<S: Into<OsString>>(mut self, extension: S, lang_id: LangId) -> Self {
        self.language_overrides.insert(extension.into(), lang_id);
        self
    }

    pub fn with_filter(mut self, filter: TokenFilter) -> Self {
        self.filter = filter;
        self
    }

    /// Files larger than `bytes` are skipped with a diagnostic instead of being read.
    pub fn with_max_file_size(mut self, bytes: u64) -> Self {
        self.max_file_size_bytes = bytes;
        self
    }

    /// Per-file parse budget in microseconds (0 = unlimited).
    pub fn with_parse_timeout_micros(mut self, micros: u64) -> Self {
        self.parse_timeout_micros = micros;
        self
    }

    /// Worker threads for parsing (0 = one per available core).
    pub fn with_worker_threads(mut self, threads: usize) -> Self {
        self.worker_threads = threads;
        self
    }

    pub fn build(mut self) -> SourceManifest {
        self.file_paths.sort_unstable();
        self.file_paths.dedup();
        SourceManifest {
            file_paths: self.file_paths,
            language_overrides: self.language_overrides,
            filter: self.filter,
            max_file_size_bytes: self.max_file_size_bytes,
            parse_timeout_micros: self.parse_timeout_micros,
            worker_threads: self.worker_threads,
        }
    }
}
