use std::collections::HashMap;
use std::ffi::OsString;
use std::path::PathBuf;

pub use crate::core::types::source::{SourceManifest, TokenFilter};
use crate::core::types::token::LangId;

#[derive(Debug, Default)]
pub struct SourceManifestBuilder {
    file_paths: Vec<PathBuf>,
    language_overrides: HashMap<OsString, LangId>,
    filter: TokenFilter,
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

    pub fn build(mut self) -> SourceManifest {
        self.file_paths.sort_unstable();
        SourceManifest {
            file_paths: self.file_paths,
            language_overrides: self.language_overrides,
            filter: self.filter,
        }
    }
}
