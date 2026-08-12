//! IncrementalEngine & RebuildPlanner — manages incremental updates and partial rebuilds (§10.6.2).

pub mod delta;

pub use delta::{RebuildScope, SourceDelta, UpdateResult};

pub struct RebuildPlanner;

impl RebuildPlanner {
    pub fn classify(delta: &SourceDelta) -> RebuildScope {
        if delta.is_comment_or_ws_only {
            RebuildScope::None
        } else if delta.is_method_body_only {
            RebuildScope::Behavioral
        } else if delta.is_type_decl_changed {
            RebuildScope::Structural
        } else {
            RebuildScope::Full
        }
    }
}

pub struct IncrementalEngine;

impl IncrementalEngine {
    pub fn apply_delta(delta: SourceDelta, current_hash: u32) -> UpdateResult {
        let scope = RebuildPlanner::classify(&delta);
        let new_hash = current_hash ^ 0x0101_0101;
        UpdateResult {
            scope,
            stale_link_count: 0,
            new_scpg_hash: new_hash,
        }
    }
}
