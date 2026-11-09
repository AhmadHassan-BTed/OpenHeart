//! Delivery adapters around the analysis engine: the shared pipeline runner used by both
//! the CLI and the HTTP server, the HTTP server itself, and the GitHub repository fetcher.

mod env;
mod http;
pub mod runner;
mod security;
pub mod server;
mod session;
pub mod web_repo;

pub use runner::{
    discover_sources, run_isolated, run_pipeline, DiscoveryOptions, PipelineError,
    PipelineOptions, PipelineRun, SourceInventory, SourceLimits, WorkDir,
};
pub use server::{OpenHeartServer, ServerConfig};
