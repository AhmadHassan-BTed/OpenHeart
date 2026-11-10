//! Delivery adapters around the analysis engine: the shared pipeline runner used by the
//! CLI, the HTTP server and the WebAssembly build, and the engine query protocol shared by
//! `openheart query`, `POST /api/sessions/<id>/query` and `oh_query`.
//!
//! With the `server` feature (on by default) they also hold the HTTP server, its access
//! control and session store, and the GitHub repository fetcher. Those use `std::net` and
//! `std::process`, so the WebAssembly build (`--no-default-features --features wasm`)
//! compiles without them.

// Some environment readers serve only the server; they stay compiled for the runner.
#[cfg_attr(not(feature = "server"), allow(dead_code))]
mod env;
#[cfg(feature = "server")]
mod http;
pub mod query;
pub mod runner;
#[cfg(feature = "server")]
mod security;
#[cfg(feature = "server")]
pub mod server;
#[cfg(feature = "server")]
mod session;
#[cfg(feature = "server")]
pub mod web_repo;

pub use runner::{
    discover_sources, run_isolated, run_pipeline, DiscoveryOptions, PipelineError,
    PipelineOptions, PipelineRun, SourceInventory, SourceLimits, WorkDir,
};
#[cfg(feature = "server")]
pub use server::{OpenHeartServer, ServerConfig};
