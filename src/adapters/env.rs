//! Typed readers for the `OPENHEART_*` environment variables shared by the CLI and server.
//!
//! Every reader distinguishes "unset" (`Ok(None)`) from "set but invalid" (`Err`), so a
//! mistyped value fails loudly instead of silently falling back to a default.

#![forbid(unsafe_code)]

use std::env;
use std::path::PathBuf;
use std::str::FromStr;

/// Returns the trimmed value of `name`, or `None` when it is unset or blank.
pub(crate) fn string(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// Returns the first variable in `names` that is set and non-blank.
pub(crate) fn first_string(names: &[&str]) -> Option<(String, String)> {
    names
        .iter()
        .find_map(|name| string(name).map(|value| (name.to_string(), value)))
}

/// Parses `name` as a number, reporting the variable name on malformed input.
pub(crate) fn number<T: FromStr>(name: &str) -> Result<Option<T>, String> {
    match string(name) {
        None => Ok(None),
        Some(raw) => raw
            .parse::<T>()
            .map(Some)
            .map_err(|_| format!("environment variable {} has invalid value '{}'", name, raw)),
    }
}

/// Parses `name` as a boolean (`1/0`, `true/false`, `yes/no`, `on/off`).
pub(crate) fn flag(name: &str) -> Result<Option<bool>, String> {
    match string(name) {
        None => Ok(None),
        Some(raw) => parse_flag(&raw).map(Some).ok_or_else(|| {
            format!(
                "environment variable {} must be true or false, got '{}'",
                name, raw
            )
        }),
    }
}

/// Parses a boolean spelled `1/0`, `true/false`, `yes/no` or `on/off` (case-insensitive).
pub(crate) fn parse_flag(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

/// Splits a comma-separated variable into trimmed, non-empty entries.
pub(crate) fn list(name: &str) -> Vec<String> {
    string(name)
        .map(|raw| {
            raw.split(',')
                .map(|item| item.trim().to_string())
                .filter(|item| !item.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// Splits a variable using the platform's `PATH` separator (`:` on Unix, `;` on Windows).
pub(crate) fn paths(name: &str) -> Option<Vec<PathBuf>> {
    env::var_os(name).map(|raw| {
        env::split_paths(&raw)
            .filter(|path| !path.as_os_str().is_empty())
            .collect()
    })
}
