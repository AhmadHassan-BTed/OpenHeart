//! Abstract interpretation and data-flow analysis over the IR (`crate::ir`).
//!
//! Every fact this module produces is computed from the lowered program — SSA values,
//! explicit memory operations (`Load`, `Store`, `Alloc`, `Release`), calls with their
//! exceptional successors, and the call graph built by `crate::memory`. Nothing is keyed
//! on names, source text or language: language knowledge reaches the analyses only through
//! the oracle traits below, which the frontends (`crate::lang`) and the memory model
//! (`crate::memory`) implement. When an oracle has no answer, the analyses fall back to the
//! least precise sound value and report which results depend on the missing fact.
//!
//! ```text
//! lattice     Lattice trait (bottom, join, meet, leq, widen, narrow) and reusable lattices
//! solver      IR CFG view, monotone-framework worklist solvers (forward / backward, RPO,
//!             widening at loop heads, narrowing pass), the IR binding layer and oracles
//! reaching    SSA def-use chains; reaching definitions of memory locations, load chains
//! liveness    SSA liveness with phi-aware edge semantics
//! sccp        sparse conditional constant propagation (Wegman–Zadeck 1991)
//! interval    integer interval analysis with branch refinement, widening and narrowing
//! nullness    reference nullness with dereference verdicts and decided null tests
//! typestate   abstract values of state-carrying locations, transitions and guards
//! controldep  control dependence from post-dominance frontiers (Ferrante et al. 1987)
//! conditions  canonical branch-condition atoms (SSA value identity) and block guards
//! ifds        Reps–Horwitz–Sagiv tabulation over the interprocedural supergraph
//! ide         Sagiv–Reps–Horwitz IDE: jump functions over environment transformers
//! context     k-limited call-string, object- and type-sensitive contexts
//! ```
//!
//! Oracles:
//! - [`solver::TypeFacts`] — scalar kind and width of IR types and values.
//! - [`reaching::AliasOracle`] — may/must alias of memory locations and call side effects.
//! - [`nullness::NullabilityFacts`] — declared nullability of parameters, loads and calls.
//! - [`typestate::SymbolFacts`] — which symbolic constants denote distinct values.

pub mod lattice;
pub mod solver;
pub mod reaching;
pub mod liveness;
pub mod sccp;
pub mod interval;
pub mod nullness;
pub mod typestate;
pub mod controldep;
pub mod ifds;
pub mod ide;
pub mod context;
pub mod conditions;

pub use lattice::Lattice;
pub use solver::{
    solve_backward, solve_forward, BackwardTransfer, DataflowResult, ForwardTransfer, IrCfg,
    SolverOptions,
};
