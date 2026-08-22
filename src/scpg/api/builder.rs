//! EngineBuilder — fluent builder for OpenHeartEngine configuration (§10.7).

use std::path::PathBuf;

pub struct EngineBuilder {
    pub output_dir: PathBuf,
    pub cache_capacity: usize,
    pub enable_mmap: bool,
}

impl Default for EngineBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl EngineBuilder {
    pub fn new() -> Self {
        Self {
            output_dir: PathBuf::from("./scpg_out"),
            cache_capacity: 512,
            enable_mmap: true,
        }
    }

    pub fn with_output_dir(mut self, path: PathBuf) -> Self {
        self.output_dir = path;
        self
    }

    pub fn with_cache_capacity(mut self, cap: usize) -> Self {
        self.cache_capacity = cap;
        self
    }
}
