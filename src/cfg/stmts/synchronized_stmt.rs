//! Synchronized Statement CFG builder algorithm (JLS 14.19).
//!
//! `synchronized (lock) { body }` evaluates the lock (monitor enter), then the body, and
//! releases the monitor on every exit: the release block acts as an implicit `finally`,
//! so returns, breaks and exceptions leaving the body pass through it before continuing
//! to their targets.

use crate::ast::BPASTArtifact;
use crate::cfg::ast_query::children;
use crate::cfg::builder::state::{CFGBuilderState, FrameKind};
use crate::cfg::stmts::dispatch_stmt;
use crate::cfg::stmts::expr::build_simple;
use crate::core::types::ast::ASTNodeType;
use crate::core::types::cfg::CFGEdgeType;
use crate::symbol::SymbolTableArtifact;

pub fn build_synchronized(
    node: u32,
    state: &mut CFGBuilderState,
    bpa: &BPASTArtifact,
    sta: &SymbolTableArtifact,
) {
    let kids = children(bpa, node);
    let body = kids
        .iter()
        .copied()
        .find(|&c| bpa.node_type(c) == ASTNodeType::NN_BLOCK);
    let lock = kids
        .iter()
        .copied()
        .find(|&c| bpa.node_type(c) != ASTNodeType::NN_BLOCK);

    // Monitor enter: evaluating the lock expression may throw (NullPointerException).
    if let Some(l) = lock {
        build_simple(l, state, bpa, sta);
    }

    let release = state.new_block();
    state.push_frame(
        FrameKind::Protected {
            handlers: Vec::new(),
            handlers_active: true,
            finally_entry: Some(release),
            continuations: Vec::new(),
        },
        Vec::new(),
    );

    if let Some(b) = body {
        dispatch_stmt(b, state, bpa, sta);
    }

    let frame = state.pop_frame();
    let body_end = state.current_block;
    let completes_normally = state.is_reachable(body_end);
    state.add_edge(body_end, release, CFGEdgeType::Uncond);

    // Monitor exit, then every abrupt completion continues to its target.
    if let Some(FrameKind::Protected { continuations, .. }) = frame.map(|f| f.kind) {
        for j in continuations {
            state.route_jump(release, j);
        }
    }

    let after = state.new_block();
    if completes_normally {
        state.add_edge(release, after, CFGEdgeType::Uncond);
    }
    state.current_block = after;
}
