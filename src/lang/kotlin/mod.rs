//! Kotlin frontend: tree-sitter-kotlin (fwcd grammar, 0.3.x) adapters for Phases 1–3.
//!
//! `ingest` classifies every CST leaf into the token alphabet (Phase 1), `reduce` maps the
//! concrete syntax tree onto the language-neutral BP AST node set (Phase 2), `semantics`
//! implements the Kotlin declaration, scoping, import, receiver and Java-interop rules
//! (Phase 3), and `stdlib` is the built-in model of the Kotlin standard library and the
//! `kotlinx.coroutines` basics that Phase 3 binds library references against.

pub mod ingest;
pub mod reduce;
pub mod semantics;
pub mod stdlib;
