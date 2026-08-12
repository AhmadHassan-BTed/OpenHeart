//! RecursiveHandler — bounded unwinding for functions in recursive SCCs (§8.2.6).
//!
//! For functions with `scc_class ≥ 1` (self-recursive or mutually-recursive):
//! - Apply bounded unwinding: record `unwind_depth = 3`.
//! - The PSA record is marked with this depth.
//! - Phase 10's CFL-reachability engine uses the bound when composing inter-procedural queries.
//!
//! The structural ROBDD represents paths within the single function body.
//! Recursion is bounded: the `unwind_depth` signals to Phase 10 that paths through
//! this function are counted at depth ≤ 3, not infinitely.

/// The fixed unwinding depth for recursive SCC functions (§8.2.6).
pub const UNWIND_DEPTH: u16 = 3;

/// Handler for recursive SCC functions.
pub struct RecursiveHandler;

impl RecursiveHandler {
    /// Record the bounded unwinding depth for a recursive function.
    ///
    /// The structural ROBDD already faithfully encodes the single-invocation paths.
    /// The `unwind_depth` metadata communicates to Phase 10 that path composition
    /// is bounded at `UNWIND_DEPTH` recursive expansions.
    ///
    /// Returns `(f_paths, unwind_depth)` — f_paths unchanged, depth set to 3.
    pub fn apply(f_paths: u32) -> (u32, u16) {
        (f_paths, UNWIND_DEPTH)
    }
}
