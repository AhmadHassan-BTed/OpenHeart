//! Crash- and race-safe artifact file operations.
//!
//! Artifacts are never truncated in place: each write goes to a freshly created
//! (`create_new`, so an existing file or planted symlink is never opened) temporary file
//! in the destination directory, is flushed to disk, and is then atomically renamed over
//! the target. A reader that has the previous version open or memory-mapped keeps seeing
//! a complete file, and a crash mid-write leaves the old artifact intact.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static UNIQUE_COUNTER: AtomicU64 = AtomicU64::new(0);

fn unique_suffix() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let seq = UNIQUE_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{}-{:x}-{}", std::process::id(), nanos, seq)
}

fn write_and_sync(file: &mut fs::File, bytes: &[u8]) -> io::Result<()> {
    file.write_all(bytes)?;
    file.flush()?;
    file.sync_all()
}

/// Atomically replaces `path` with `bytes`.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let file_name = path.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("artifact path {} has no file name", path.display()),
        )
    })?;
    let dir = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    };

    const MAX_ATTEMPTS: usize = 16;
    for _ in 0..MAX_ATTEMPTS {
        let tmp_path = dir.join(format!(
            ".{}.{}.tmp",
            file_name.to_string_lossy(),
            unique_suffix()
        ));
        let mut file = match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp_path)
        {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        };

        let written = write_and_sync(&mut file, bytes);
        drop(file);
        if let Err(e) = written {
            let _ = fs::remove_file(&tmp_path);
            return Err(e);
        }

        if let Err(e) = fs::rename(&tmp_path, path) {
            let _ = fs::remove_file(&tmp_path);
            return Err(io::Error::new(
                e.kind(),
                format!(
                    "failed to move {} into place at {}: {}",
                    tmp_path.display(),
                    path.display(),
                    e
                ),
            ));
        }
        return Ok(());
    }

    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!(
            "could not create a unique temporary file next to {}",
            path.display()
        ),
    ))
}

/// Creates a new, empty directory named `<prefix>-<unique>` under `parent`.
/// `create_dir` (not `create_dir_all`) fails if the name already exists, so the returned
/// directory is never one that another process or user prepared in advance.
pub fn create_unique_dir(parent: &Path, prefix: &str) -> io::Result<PathBuf> {
    const MAX_ATTEMPTS: usize = 16;
    for _ in 0..MAX_ATTEMPTS {
        let candidate = parent.join(format!("{}-{}", prefix, unique_suffix()));
        match fs::create_dir(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!("could not create a unique directory under {}", parent.display()),
    ))
}
