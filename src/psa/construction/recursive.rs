//! RecursiveHandler — classification of functions in recursive SCCs (§8.2.6).
//!
//! Phase 8 summarizes each function's *intraprocedural* CFG. Recursion does not change the
//! set of paths through a single body, so no unwinding is folded into the summary: the
//! record is flagged with `PSA_FLAG_RECURSIVE_SCC` and its `unwind_depth` is 0, meaning
//! "no recursive expansion is included". Consumers that compose summaries across call sites
//! (Phase 10) must bound recursion themselves; [`RECOMMENDED_UNWIND_DEPTH`] is the bound the
//! specification suggests for that composition.
//!
//! Earlier revisions stamped `unwind_depth = 3` on recursive functions although nothing was
//! unwound, which advertised a bounded-unwinding summary that did not exist.

use crate::psa::types::PSA_FLAG_RECURSIVE_SCC;

/// Unwinding bound suggested for interprocedural composition of recursive SCCs (§8.2.6).
pub const RECOMMENDED_UNWIND_DEPTH: u16 = 3;

/// Handler for recursive SCC functions.
pub struct RecursiveHandler;

impl RecursiveHandler {
    /// `(flags, unwind_depth)` contribution for a function whose SCC is recursive iff
    /// `is_recursive`. The summary itself is never modified.
    pub fn classify(is_recursive: bool) -> (u16, u16) {
        if is_recursive {
            (PSA_FLAG_RECURSIVE_SCC, 0)
        } else {
            (0, 0)
        }
    }
}
