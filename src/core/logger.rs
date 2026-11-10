//! Dual Logging Engine for OpenHeart Engine.
//! Maintains both Persistent Logs (appended across runs, size-rotated) and Session Logs
//! (fresh per execution session).
//!
//! Console and file sinks have independent level thresholds. A message whose level is
//! enabled for neither sink returns before taking the logger lock or touching the file
//! system, and the `oh_trace!` / `oh_debug!` / ... macros skip even the `format!` of the
//! message, so disabled levels cost one atomic load.
//!
//! Environment:
//! - `OPENHEART_LOG` (or, when unset, `RUST_LOG`): console level
//!   (`off|error|warn|info|debug|trace` or `0..5`).
//! - `OPENHEART_FILE_LOG`: file-sink level (default `info`).
//! - `OPENHEART_LOG_MAX_BYTES`: rotation threshold for the persistent log
//!   (default 16 MiB, `0` disables rotation).

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    Off = 0,
    Error = 1,
    Warn = 2,
    Info = 3,
    Debug = 4,
    Trace = 5,
}

impl LogLevel {
    fn from_u8(val: u8) -> LogLevel {
        match val {
            0 => LogLevel::Off,
            1 => LogLevel::Error,
            2 => LogLevel::Warn,
            3 => LogLevel::Info,
            4 => LogLevel::Debug,
            5 => LogLevel::Trace,
            _ => LogLevel::Info,
        }
    }

    /// Parses a level name or digit (`warn`, `WARN`, `2`).
    pub fn parse(text: &str) -> Option<LogLevel> {
        match text.trim().to_ascii_lowercase().as_str() {
            "off" | "0" => Some(LogLevel::Off),
            "error" | "1" => Some(LogLevel::Error),
            "warn" | "warning" | "2" => Some(LogLevel::Warn),
            "info" | "3" => Some(LogLevel::Info),
            "debug" | "4" => Some(LogLevel::Debug),
            "trace" | "5" => Some(LogLevel::Trace),
            _ => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            LogLevel::Error => "ERROR",
            LogLevel::Warn => "WARN ",
            LogLevel::Info => "INFO ",
            LogLevel::Debug => "DEBUG",
            LogLevel::Trace => "TRACE",
            LogLevel::Off => "OFF  ",
        }
    }
}

/// Default rotation threshold of the persistent log.
pub const DEFAULT_PERSISTENT_LOG_MAX_BYTES: u64 = 16 * 1024 * 1024;

static CONSOLE_LOG_LEVEL: AtomicU8 = AtomicU8::new(LogLevel::Info as u8);
static FILE_LOG_LEVEL: AtomicU8 = AtomicU8::new(LogLevel::Info as u8);
static FILE_SINKS_ACTIVE: AtomicBool = AtomicBool::new(false);

struct LogSink {
    file: File,
    path: PathBuf,
    written: u64,
    /// Rotate when the file would grow beyond this many bytes (0 = never).
    max_bytes: u64,
}

impl LogSink {
    fn write_line(&mut self, line: &str) {
        if self.max_bytes > 0
            && self.written > 0
            && self.written.saturating_add(line.len() as u64) > self.max_bytes
        {
            self.rotate();
        }
        if self.file.write_all(line.as_bytes()).is_ok() {
            self.written = self.written.saturating_add(line.len() as u64);
        }
    }

    /// Moves the current file to `<name>.1` (replacing an older rotation) and reopens a
    /// fresh file. On failure the sink keeps appending to the current file.
    fn rotate(&mut self) {
        let mut rotated = self.path.clone().into_os_string();
        rotated.push(".1");
        let rotated = PathBuf::from(rotated);
        let _ = self.file.flush();
        if fs::rename(&self.path, &rotated).is_err() {
            return;
        }
        if let Ok(fresh) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            self.file = fresh;
            self.written = 0;
        }
    }
}

struct DualLoggerState {
    persistent: Option<LogSink>,
    session: Option<LogSink>,
    session_id: String,
}

static LOGGER_STATE: Mutex<Option<DualLoggerState>> = Mutex::new(None);
static PERSISTENT_MAX_BYTES: AtomicU64 = AtomicU64::new(DEFAULT_PERSISTENT_LOG_MAX_BYTES);

pub fn set_log_level(level: LogLevel) {
    CONSOLE_LOG_LEVEL.store(level as u8, Ordering::SeqCst);
}

pub fn get_log_level() -> LogLevel {
    LogLevel::from_u8(CONSOLE_LOG_LEVEL.load(Ordering::SeqCst))
}

/// Sets the threshold for the persistent and session log files.
pub fn set_file_log_level(level: LogLevel) {
    FILE_LOG_LEVEL.store(level as u8, Ordering::SeqCst);
}

pub fn get_file_log_level() -> LogLevel {
    LogLevel::from_u8(FILE_LOG_LEVEL.load(Ordering::SeqCst))
}

/// Sets the persistent-log rotation threshold in bytes (0 disables rotation). Applies to
/// sinks opened by later `init_dual_logger` calls.
pub fn set_persistent_log_max_bytes(max_bytes: u64) {
    PERSISTENT_MAX_BYTES.store(max_bytes, Ordering::SeqCst);
}

/// True when a message at `level` would reach at least one sink. Callers building
/// expensive messages should check this first (the `oh_*!` macros do).
#[inline]
pub fn log_enabled(level: LogLevel) -> bool {
    if level == LogLevel::Off {
        return false;
    }
    let lvl = level as u8;
    lvl <= CONSOLE_LOG_LEVEL.load(Ordering::Relaxed)
        || (FILE_SINKS_ACTIVE.load(Ordering::Relaxed) && lvl <= FILE_LOG_LEVEL.load(Ordering::Relaxed))
}

fn open_sink(path: &Path, append: bool, max_bytes: u64) -> Option<LogSink> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            let _ = fs::create_dir_all(parent);
        }
    }
    let file = if append {
        OpenOptions::new().create(true).append(true).open(path).ok()?
    } else {
        OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path)
            .ok()?
    };
    let written = if append {
        file.metadata().map(|m| m.len()).unwrap_or(0)
    } else {
        0
    };
    Some(LogSink {
        file,
        path: path.to_path_buf(),
        written,
        max_bytes,
    })
}

/// Opens the persistent and session log files. The WebAssembly build has no file system:
/// there this only starts a console session and the file sinks stay inactive.
pub fn init_dual_logger(persistent_path: Option<&Path>, session_path: Option<&Path>) {
    #[cfg(target_arch = "wasm32")]
    let (persistent_path, session_path): (Option<&Path>, Option<&Path>) = {
        let _ = (persistent_path, session_path);
        (None, None)
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let session_id = format!("session-{}", now);

    let persistent_max = PERSISTENT_MAX_BYTES.load(Ordering::SeqCst);
    let persistent = persistent_path.and_then(|p| open_sink(p, true, persistent_max));
    let session = session_path.and_then(|p| open_sink(p, false, 0));
    let active = persistent.is_some() || session.is_some();

    {
        let mut state_guard = LOGGER_STATE.lock().unwrap_or_else(|e| e.into_inner());
        *state_guard = Some(DualLoggerState {
            persistent,
            session,
            session_id: session_id.clone(),
        });
    }
    FILE_SINKS_ACTIVE.store(active, Ordering::SeqCst);

    log_event(
        LogLevel::Info,
        &format!("=== NEW LOGGING SESSION STARTED (ID: {}) ===", session_id),
    );
}

pub fn init_logger_from_env() {
    let console = std::env::var("OPENHEART_LOG")
        .ok()
        .and_then(|v| LogLevel::parse(&v))
        .or_else(|| {
            std::env::var("RUST_LOG")
                .ok()
                .and_then(|v| parse_rust_log_level(&v))
        });
    if let Some(level) = console {
        set_log_level(level);
    }

    if let Some(level) = std::env::var("OPENHEART_FILE_LOG")
        .ok()
        .and_then(|v| LogLevel::parse(&v))
    {
        set_file_log_level(level);
    }

    if let Some(max) = std::env::var("OPENHEART_LOG_MAX_BYTES")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
    {
        set_persistent_log_max_bytes(max);
    }

    let default_persistent = PathBuf::from("./openheart_persistent.log");
    let default_session = PathBuf::from("./openheart_session.log");
    init_dual_logger(Some(&default_persistent), Some(&default_session));
}

/// Extracts the level that applies to this crate from an `env_logger`-style `RUST_LOG`
/// value: an `openheart=<level>` directive wins over a bare global `<level>` directive.
fn parse_rust_log_level(value: &str) -> Option<LogLevel> {
    let mut global = None;
    let mut crate_level = None;
    for directive in value.split(',') {
        let directive = directive.trim();
        // Drop an optional `/filter` regex suffix.
        let directive = directive.split('/').next().unwrap_or("");
        match directive.split_once('=') {
            Some((target, level)) => {
                let target = target.trim();
                if target == "openheart" || target.starts_with("openheart::") {
                    crate_level = LogLevel::parse(level).or(crate_level);
                }
            }
            None => {
                if let Some(level) = LogLevel::parse(directive) {
                    global = Some(level);
                }
            }
        }
    }
    crate_level.or(global)
}

fn format_timestamp() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}.{:03}", now.as_secs(), now.subsec_millis())
}

fn log_event(level: LogLevel, msg: &str) {
    if !log_enabled(level) {
        return;
    }
    let lvl = level as u8;

    // 1. Console sink
    if lvl <= CONSOLE_LOG_LEVEL.load(Ordering::Relaxed) {
        if level == LogLevel::Error {
            eprintln!("[{}] {}", level.label(), msg);
        } else {
            println!("[{}] {}", level.label(), msg);
        }
    }

    // 2. Persistent and session file sinks (never opened on wasm32, so the WebAssembly
    //    build links no file-system calls from here)
    #[cfg(not(target_arch = "wasm32"))]
    {
        if FILE_SINKS_ACTIVE.load(Ordering::Relaxed) && lvl <= FILE_LOG_LEVEL.load(Ordering::Relaxed)
        {
            let ts = format_timestamp();
            let mut state_guard = LOGGER_STATE.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(ref mut state) = *state_guard {
                let line =
                    format!("[{}] [{}] [{}] {}\n", ts, state.session_id, level.label(), msg);
                if let Some(ref mut sink) = state.persistent {
                    sink.write_line(&line);
                }
                if let Some(ref mut sink) = state.session {
                    sink.write_line(&line);
                }
            }
        }
    }
}

/// Logs a preformatted message at `level`.
#[inline]
pub fn log_at(level: LogLevel, msg: &str) {
    log_event(level, msg);
}

#[inline]
pub fn log_error(msg: &str) {
    log_event(LogLevel::Error, msg);
}

#[inline]
pub fn log_warn(msg: &str) {
    log_event(LogLevel::Warn, msg);
}

#[inline]
pub fn log_info(msg: &str) {
    log_event(LogLevel::Info, msg);
}

#[inline]
pub fn log_debug(msg: &str) {
    log_event(LogLevel::Debug, msg);
}

#[inline]
pub fn log_trace(msg: &str) {
    log_event(LogLevel::Trace, msg);
}

/// Logs a `format!`-style message at an explicit level, formatting it only when the level
/// is enabled for some sink.
#[macro_export]
macro_rules! oh_log {
    ($level:expr, $($arg:tt)+) => {{
        let level = $level;
        if $crate::core::logger::log_enabled(level) {
            $crate::core::logger::log_at(level, &::std::format!($($arg)+));
        }
    }};
}

#[macro_export]
macro_rules! oh_error {
    ($($arg:tt)+) => { $crate::oh_log!($crate::core::logger::LogLevel::Error, $($arg)+) };
}

#[macro_export]
macro_rules! oh_warn {
    ($($arg:tt)+) => { $crate::oh_log!($crate::core::logger::LogLevel::Warn, $($arg)+) };
}

#[macro_export]
macro_rules! oh_info {
    ($($arg:tt)+) => { $crate::oh_log!($crate::core::logger::LogLevel::Info, $($arg)+) };
}

#[macro_export]
macro_rules! oh_debug {
    ($($arg:tt)+) => { $crate::oh_log!($crate::core::logger::LogLevel::Debug, $($arg)+) };
}

#[macro_export]
macro_rules! oh_trace {
    ($($arg:tt)+) => { $crate::oh_log!($crate::core::logger::LogLevel::Trace, $($arg)+) };
}

/// Stage timer. The completion line is logged by `finish` (with a summary) or, if the
/// timer is dropped without `finish` (early return, `?`, panic unwind), by `Drop`, so
/// every started stage reports its duration.
pub struct PhaseTimer {
    name: &'static str,
    start: Instant,
    finished: bool,
}

impl PhaseTimer {
    pub fn start(name: &'static str) -> Self {
        log_info(&format!("══► Starting Stage: {}...", name));
        Self {
            name,
            start: Instant::now(),
            finished: false,
        }
    }

    pub fn elapsed(&self) -> Duration {
        self.start.elapsed()
    }

    pub fn finish(mut self, summary: &str) {
        self.finished = true;
        log_info(&format!(
            "✓ Completed Stage: {} in {:.2?} | {}",
            self.name,
            self.start.elapsed(),
            summary
        ));
    }
}

impl Drop for PhaseTimer {
    fn drop(&mut self) {
        if !self.finished {
            let outcome = if std::thread::panicking() {
                "aborted by panic"
            } else {
                "ended without a summary"
            };
            log_info(&format!(
                "✓ Stage {} {} after {:.2?}",
                self.name,
                outcome,
                self.start.elapsed()
            ));
        }
    }
}
