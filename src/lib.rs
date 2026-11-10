//! OpenHeart: a static program-analysis engine that compiles Java/Kotlin sources through a
//! 10-phase pipeline (lexical ingestion, BP-encoded AST, symbol table, CFG, SSA, call
//! graph, traceability, ROBDD path summaries, UML metadata, unified SCPG binary) and
//! exports UML and program-analysis diagrams.
//!
//! [`adapters::runner`] is the single orchestration of that pipeline shared by the
//! `openheart` CLI and the HTTP server in [`adapters::server`].

#![warn(missing_docs)]
// The JSON exporters build large documents with serde_json::json!, which nests deeper than the default limit.
#![recursion_limit = "512"]

pub mod adapters;
pub mod ast;
pub mod cfg;
pub mod cg;
pub mod core;
pub mod facts;
pub mod ingestion;
pub mod psa;
pub mod scpg;
pub mod session;
pub mod ssa;
pub mod symbol;
pub mod tra;
pub mod uma;
