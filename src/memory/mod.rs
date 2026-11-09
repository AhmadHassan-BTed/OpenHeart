//! Abstract Memory Model and Whole-Program Pointer Analysis over the IR (§M).
//!
//! **Mandate:** derive, statically and without per-language templates, what every pointer
//! may refer to, which functions every call may reach, how far every object travels and who
//! owns it. Everything here is computed from the IR (`crate::ir`) the frontends lower to;
//! the only language knowledge that enters is explicit facts handed in by `crate::lang`
//! frontends (type hierarchy and override relation, intrinsic semantics, thread-spawn
//! points, field storage classes). A fact that cannot be derived is reported as such —
//! never filled in by a guess.
//!
//! **Pipeline:**
//! 1. `location` — the abstract memory: allocation-site objects (optionally qualified by a
//!    k = 1 context), object slots (fields, a summarized array element, the pointee content,
//!    closure captures), globals and statics, entry-point parameter objects and one explicit
//!    `Unknown` object standing for everything outside the program.
//! 2. `points_to` — field-sensitive inclusion-based (Andersen) analysis with difference
//!    propagation, lazy cycle detection (Hardekopf & Lin 2007) and sparse bit sets, under a
//!    configurable context policy (insensitive, 1-call-site, 1-object).
//! 3. `callgraph` — built on the fly by the solver: dispatch through the dynamic types of
//!    the receiver's points-to set, class-hierarchy fallback for unknown receivers (marked
//!    over-approximate), closures and function values, external calls as explicit nodes.
//! 4. `escape` — no escape / argument escape / global escape / thread escape per object.
//! 5. `ownership` — who allocates, stores and releases every part; composition versus
//!    aggregation versus association from those facts.
//! 6. `modref` — per-function mod/ref summaries and the alias query API.
//!
//! This module tree contains no `unsafe` code.

#![forbid(unsafe_code)]

pub mod location;
pub mod points_to;
pub mod callgraph;
pub mod escape;
pub mod ownership;
pub mod modref;

pub use callgraph::{CallGraph, Dispatch, MethodFacts, TypeHierarchy};
pub use escape::{EscapeConfig, EscapeResult, EscapeState};
pub use location::{AbstractObject, Cell, CellId, Context, HeapBase, LocationTable, ObjId, Slot, SparseBitSet};
pub use modref::{AliasOracle, AliasResult, ModRefInfo, ModRefSummaries};
pub use ownership::{OwnershipHooks, OwnershipKind, OwnershipResult};
pub use points_to::{solve, ContextPolicy, EntryPoints, ExternalPolicy, IntrinsicEffect, MemoryConfig, PointsToResult};
