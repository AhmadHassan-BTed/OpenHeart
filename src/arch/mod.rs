//! Architecture recovery from derived program facts, independent of the source language.
//!
//! `graph` builds typed, weighted dependency graphs with source provenance at type, file,
//! package/namespace and project level from the symbol table, the IR and the points-to call
//! graph; `dsm` partitions them into strongly connected components and orders them as a
//! dependency structure matrix; `layers` infers layers by longest-path levels over the
//! condensation and checks declared layers; `cluster` recovers subsystems by Louvain
//! modularity clustering; `metrics` computes Martin's package metrics, fan-in/fan-out, LCOM4
//! and cycle counts; `boundaries` finds entry points, public surfaces, external dependencies
//! and the boundary crossings between subsystems.
//!
//! Every result is computed from recorded facts. Language knowledge (what a `main` is, which
//! annotation marks a request handler, which build file declares a project) reaches this
//! module only as data supplied by the `lang` frontends; what no supplied fact establishes is
//! reported as not derivable instead of being filled in.

pub mod graph;
pub mod dsm;
pub mod layers;
pub mod cluster;
pub mod metrics;
pub mod boundaries;
