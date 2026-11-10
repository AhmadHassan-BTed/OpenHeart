//! Read-only views of artifact files.
//!
//! `MemoryMappedFile::open` is the safe constructor: it reads the file into an owned
//! buffer, so the bytes can never change underneath the program. True memory mapping is
//! available through the `unsafe` constructor `MemoryMappedFile::map`, whose contract the
//! caller must uphold (see its `# Safety` section). OpenHeart's own artifact writers never
//! modify a file in place (they write a temporary file and rename it, see
//! `core::io::fs::write_atomic`), which is what makes that contract achievable for
//! artifacts this process produced.
//!
//! `wasm32` has no memory mapping (and the WebAssembly build does not link `memmap2`):
//! there `map` reads an owned copy, exactly like `open`.

use std::fs;
use std::io;
use std::ops::Deref;
use std::path::Path;

#[cfg(not(target_arch = "wasm32"))]
use memmap2::Mmap;

enum Backing {
    Owned(Vec<u8>),
    #[cfg(not(target_arch = "wasm32"))]
    Mapped(Mmap),
}

pub struct MemoryMappedFile {
    backing: Backing,
}

impl MemoryMappedFile {
    /// Opens `path` as an immutable, owned byte buffer. Always safe: the contents are
    /// copied once and cannot be altered by other processes afterwards.
    pub fn open<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let bytes = fs::read(path)?;
        Ok(Self {
            backing: Backing::Owned(bytes),
        })
    }

    /// Wraps bytes that are already in memory.
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        Self {
            backing: Backing::Owned(bytes),
        }
    }

    /// Memory-maps `path` read-only without copying it.
    ///
    /// # Safety
    ///
    /// The caller must guarantee that, for the whole lifetime of the returned value, no
    /// process (including this one) truncates, resizes or writes the file in place. A
    /// truncated mapping faults on access (SIGBUS on Unix) and an in-place write changes
    /// bytes that safe code observes through `&[u8]`, which is undefined behaviour.
    /// Replacing the file by renaming a new file over it is permitted: the mapping keeps
    /// referring to the old file contents.
    #[cfg(not(target_arch = "wasm32"))]
    pub unsafe fn map<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let file = fs::File::open(path)?;
        // SAFETY: the caller upholds the contract documented above: the mapped file is
        // not truncated or modified in place while this mapping is alive, so the slice
        // handed out by `as_slice` stays valid and immutable.
        let mmap = unsafe { Mmap::map(&file)? };
        Ok(Self {
            backing: Backing::Mapped(mmap),
        })
    }

    /// `wasm32` cannot map files: reads an owned copy, as [`MemoryMappedFile::open`] does.
    ///
    /// # Safety
    ///
    /// Always safe on this target; the signature matches the mapping constructor of the
    /// other targets.
    #[cfg(target_arch = "wasm32")]
    pub unsafe fn map<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        Self::open(path)
    }

    /// True when the bytes are a live memory mapping rather than an owned copy.
    pub fn is_mapped(&self) -> bool {
        match &self.backing {
            Backing::Owned(_) => false,
            #[cfg(not(target_arch = "wasm32"))]
            Backing::Mapped(_) => true,
        }
    }

    pub fn as_slice(&self) -> &[u8] {
        match &self.backing {
            Backing::Owned(bytes) => bytes.as_slice(),
            #[cfg(not(target_arch = "wasm32"))]
            Backing::Mapped(mmap) => &mmap[..],
        }
    }
}

impl Deref for MemoryMappedFile {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.as_slice()
    }
}
