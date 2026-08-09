//! High-Performance Zero-Dependency Logger for OpenHeart Pipeline.
//! Supports INFO, DEBUG, TRACE levels with elapsed timing and structured output.

use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Instant;

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

static CURRENT_LOG_LEVEL: AtomicU8 = AtomicU8::new(LogLevel::Info as u8);

pub fn set_log_level(level: LogLevel) {
    CURRENT_LOG_LEVEL.store(level as u8, Ordering::SeqCst);
}

pub fn get_log_level() -> LogLevel {
    match CURRENT_LOG_LEVEL.load(Ordering::SeqCst) {
        0 => LogLevel::Off,
        1 => LogLevel::Error,
        2 => LogLevel::Warn,
        3 => LogLevel::Info,
        4 => LogLevel::Debug,
        5 => LogLevel::Trace,
        _ => LogLevel::Info,
    }
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
}

#[inline]
pub fn log_error(msg: &str) {
    if get_log_level() >= LogLevel::Error {
        eprintln!("[ERROR] {}", msg);
    }
}

#[inline]
pub fn log_warn(msg: &str) {
    if get_log_level() >= LogLevel::Warn {
        eprintln!("[WARN]  {}", msg);
    }
}

#[inline]
pub fn log_info(msg: &str) {
    if get_log_level() >= LogLevel::Info {
        println!("[INFO]  {}", msg);
    }
}

#[inline]
pub fn log_debug(msg: &str) {
    if get_log_level() >= LogLevel::Debug {
        println!("[DEBUG] {}", msg);
    }
}

#[inline]
pub fn log_trace(msg: &str) {
    if get_log_level() >= LogLevel::Trace {
        println!("[TRACE] {}", msg);
    }
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
