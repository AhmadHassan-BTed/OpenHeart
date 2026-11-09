pub mod tree_sitter;

use std::fmt;

use ::tree_sitter::{Language, Tree};

pub trait CSTParser {
    fn parse(&mut self, source: &[u8], language: Language) -> Result<Tree, String>;
}

/// Why a parse produced no tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    /// The grammar could not be loaded (ABI mismatch between grammar and runtime).
    Language(String),
    /// The per-file parse budget was exhausted.
    TimedOut { timeout_micros: u64 },
    /// The parser stopped without a timeout (cancellation).
    Aborted,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::Language(e) => write!(f, "Failed to set tree-sitter language: {}", e),
            ParseError::TimedOut { timeout_micros } => write!(
                f,
                "Tree-sitter parse exceeded the {} µs budget",
                timeout_micros
            ),
            ParseError::Aborted => write!(f, "Tree-sitter parse was cancelled"),
        }
    }
}

impl std::error::Error for ParseError {}
