//! Dual Logging Engine for OpenHeart Engine.
//! Maintains both Persistent Logs (appended across runs) and Session Logs (fresh per execution session).

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Mutex;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

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

static CONSOLE_LOG_LEVEL: AtomicU8 = AtomicU8::new(LogLevel::Info as u8);

struct DualLoggerState {
    persistent_file: Option<File>,
    session_file: Option<File>,
    session_id: String,
}

static LOGGER_STATE: Mutex<Option<DualLoggerState>> = Mutex::new(None);

pub fn set_log_level(level: LogLevel) {
    CONSOLE_LOG_LEVEL.store(level as u8, Ordering::SeqCst);
}

pub fn get_log_level() -> LogLevel {
    match CONSOLE_LOG_LEVEL.load(Ordering::SeqCst) {
        0 => LogLevel::Off,
        1 => LogLevel::Error,
        2 => LogLevel::Warn,
        3 => LogLevel::Info,
        4 => LogLevel::Debug,
        5 => LogLevel::Trace,
        _ => LogLevel::Info,
    }
}

pub fn init_dual_logger(persistent_path: Option<&Path>, session_path: Option<&Path>) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let session_id = format!("session-{}", now);

    let persistent_file = if let Some(p_path) = persistent_path {
        if let Some(parent) = p_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(p_path)
            .ok()
    } else {
        None
    };

    let session_file = if let Some(s_path) = session_path {
        if let Some(parent) = s_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(s_path)
            .ok()
    } else {
        None
    };

    let mut state_guard = LOGGER_STATE.lock().unwrap();
    *state_guard = Some(DualLoggerState {
        persistent_file,
        session_file,
        session_id,
    });

    drop(state_guard);

    log_event(
        LogLevel::Info,
        &format!("=== NEW LOGGING SESSION STARTED (ID: session-{}) ===", now),
    );
}

pub fn init_logger_from_env() {
    if let Ok(val) = std::env::var("OPENHEART_LOG") {
        match val.to_lowercase().as_str() {
            "off" | "0" => set_log_level(LogLevel::Off),
            "error" | "1" => set_log_level(LogLevel::Error),
            "warn" | "2" => set_log_level(LogLevel::Warn),
            "info" | "3" => set_log_level(LogLevel::Info),
            "debug" | "4" => set_log_level(LogLevel::Debug),
            "trace" | "5" => set_log_level(LogLevel::Trace),
            _ => {}
        }
    }

    let default_persistent = PathBuf::from("./openheart_persistent.log");
    let default_session = PathBuf::from("./openheart_session.log");
    init_dual_logger(Some(&default_persistent), Some(&default_session));
}

fn format_timestamp() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs();
    let millis = now.subsec_millis();
    format!("{}.{:03}", secs, millis)
}

fn log_event(level: LogLevel, msg: &str) {
    let ts = format_timestamp();
    let level_str = match level {
        LogLevel::Error => "ERROR",
        LogLevel::Warn => "WARN ",
        LogLevel::Info => "INFO ",
        LogLevel::Debug => "DEBUG",
        LogLevel::Trace => "TRACE",
        LogLevel::Off => "OFF  ",
    };

    // 1. Write to Console if console verbosity level allows
    if get_log_level() >= level && level != LogLevel::Off {
        if level == LogLevel::Error {
            eprintln!("[{}] {}", level_str, msg);
        } else {
            println!("[{}] {}", level_str, msg);
        }
    }

    // 2. Write to Persistent and Session log files
    let mut state_guard = LOGGER_STATE.lock().unwrap();
    if let Some(ref mut state) = *state_guard {
        let line = format!("[{}] [{}] [{}] {}\n", ts, state.session_id, level_str, msg);

        if let Some(ref mut p_file) = state.persistent_file {
            let _ = p_file.write_all(line.as_bytes());
            let _ = p_file.flush();
        }

        if let Some(ref mut s_file) = state.session_file {
            let _ = s_file.write_all(line.as_bytes());
            let _ = s_file.flush();
        }
    }
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

pub struct PhaseTimer {
    name: &'static str,
    start: Instant,
}

impl PhaseTimer {
    pub fn start(name: &'static str) -> Self {
        log_info(&format!("══► Starting Stage: {}...", name));
        Self {
            name,
            start: Instant::now(),
        }
    }

    pub fn finish(self, summary: &str) {
        log_info(&format!(
            "✓ Completed Stage: {} in {:.2?} | {}",
            self.name,
            self.start.elapsed(),
            summary
        ));
    }
}
