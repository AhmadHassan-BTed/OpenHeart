//! Where Phases 1 and 2 read source text from.
//!
//! The command line and the server analyze files on disk; the WebAssembly build receives
//! every file's text in memory and must never touch a file system. Both phases take a
//! [`SourceProvider`] so the same code reads, hashes and parses either kind of input. A file
//! is identified by the path recorded in the token corpus; in memory that is the path the
//! caller supplied, exactly as given (no canonicalization, no file-system lookups), and the
//! SHA-256 that links Phase 2 back to Phase 1 is computed from the in-memory bytes.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;
use std::time::UNIX_EPOCH;

/// Source texts held in memory, keyed by the path recorded in the token corpus.
#[derive(Debug, Clone, Default)]
pub struct MemorySources {
    files: BTreeMap<String, Vec<u8>>,
}

impl MemorySources {
    /// An empty set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds (or replaces) the text of `path`; returns the previous text, if any.
    pub fn insert(&mut self, path: impl Into<String>, text: impl Into<Vec<u8>>) -> Option<Vec<u8>> {
        self.files.insert(path.into(), text.into())
    }

    /// The bytes of `path`.
    pub fn get(&self, path: &str) -> Option<&[u8]> {
        self.files.get(path).map(Vec::as_slice)
    }

    /// Whether `path` is held.
    pub fn contains(&self, path: &str) -> bool {
        self.files.contains_key(path)
    }

    /// Every held path, in sorted order.
    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.files.keys().map(String::as_str)
    }

    /// Number of held files.
    pub fn len(&self) -> usize {
        self.files.len()
    }

    /// Whether no file is held.
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// Combined size of every held file.
    pub fn total_bytes(&self) -> u64 {
        self.files.values().map(|text| text.len() as u64).sum()
    }
}

/// Size and modification time of one source, as recorded in the token corpus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceStat {
    /// Size in bytes.
    pub size: u64,
    /// Modification time in nanoseconds since the Unix epoch (0 when unknown, which is
    /// always the case for in-memory sources).
    pub mtime_ns: u64,
}

/// The origin of every source file a run reads.
#[derive(Debug, Clone, Copy)]
pub enum SourceProvider<'a> {
    /// Files are read from the file system.
    Disk,
    /// Files are taken from memory; the file system is never touched.
    Memory(&'a MemorySources),
}

impl<'a> SourceProvider<'a> {
    /// Whether this provider reads from memory.
    pub fn is_memory(&self) -> bool {
        matches!(self, SourceProvider::Memory(_))
    }

    /// The path as recorded in the corpus (lossless for in-memory paths, which are UTF-8).
    fn key(path: &Path) -> std::borrow::Cow<'_, str> {
        path.to_string_lossy()
    }

    /// Size and modification time of `path`.
    pub fn stat(&self, path: &Path) -> io::Result<SourceStat> {
        match self {
            SourceProvider::Disk => {
                let meta = fs::metadata(path)?;
                let mtime_ns = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map(|d| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX))
                    .unwrap_or(0);
                Ok(SourceStat {
                    size: meta.len(),
                    mtime_ns,
                })
            }
            SourceProvider::Memory(sources) => sources
                .get(&Self::key(path))
                .map(|text| SourceStat {
                    size: text.len() as u64,
                    mtime_ns: 0,
                })
                .ok_or_else(|| not_held(path)),
        }
    }

    /// The full contents of `path`.
    pub fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        match self {
            SourceProvider::Disk => fs::read(path),
            SourceProvider::Memory(sources) => sources
                .get(&Self::key(path))
                .map(<[u8]>::to_vec)
                .ok_or_else(|| not_held(path)),
        }
    }
}

fn not_held(path: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::NotFound,
        format!("{} is not among the in-memory sources", path.display()),
    )
}
