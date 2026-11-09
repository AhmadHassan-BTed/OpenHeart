//! Try-Catch-Finally and try-with-resources CFG builder algorithms (§4.5.6, JLS 14.20).
//!
//! Expected AST shape: `[resource*, NN_BLOCK body, NN_CATCH_CLAUSE*, NN_FINALLY_CLAUSE?]`.
//! Resources (`NN_LOCAL_VAR_DECL` declarators or variable references) precede the body.
//!
//! Lowering:
//! * the statement's catch clauses and finally form one protected frame around the
//!   resources and the body;
//! * try-with-resources is desugared per JLS 14.20.3: each resource is initialised in
//!   order inside its own protected scope whose "finally" is the implicit `close()`
//!   block; the scopes are closed innermost-first on normal and abrupt completion, and
//!   `close()` itself may throw;
//! * catch bodies run with the statement's own handlers inactive;
//! * the finally body is built once; the abrupt jumps that entered it (return, break,
//!   continue, yield, uncaught exceptions) continue from its end, and the normal exit is
//!   wired only when the protected part or a catch body can complete normally.

use crate::ast::BPASTArtifact;
use crate::cfg::ast_query::{children, first_child_of_type};
use crate::cfg::builder::exceptions::ThrownType;
use crate::cfg::builder::state::{CFGBuilderState, FrameKind, Handler, Jump};
use crate::cfg::stmts::dispatch_stmt;
use crate::cfg::stmts::expr::build_simple;
use crate::core::types::ast::ASTNodeType;
use crate::core::types::cfg::CFGEdgeType;
use crate::symbol::SymbolTableArtifact;

pub fn build_try(
    node: u32,
    state: &mut CFGBuilderState,
    bpa: &BPASTArtifact,
    sta: &SymbolTableArtifact,
) {
    let kids = children(bpa, node);
    let body_idx = match kids
        .iter()
        .position(|&c| bpa.node_type(c) == ASTNodeType::NN_BLOCK)
    {
        Some(i) => i,
        None => {
            // No body survived Phase 2: keep the statement as an opaque statement.
            build_simple(node, state, bpa, sta);
            return;
        }
    };
    let resources: Vec<u32> = kids[..body_idx].to_vec();
    let body = kids[body_idx];
    let catches: Vec<u32> = kids[body_idx + 1..]
        .iter()
        .copied()
        .filter(|&c| bpa.node_type(c) == ASTNodeType::NN_CATCH_CLAUSE)
        .collect();
    let finally = kids[body_idx + 1..]
        .iter()
        .copied()
        .find(|&c| bpa.node_type(c) == ASTNodeType::NN_FINALLY_CLAUSE);

    let post_try = state.new_block();
    let handlers: Vec<Handler> = catches
        .iter()
        .map(|&c| Handler {
            block: state.new_block(),
            types: state.exceptions.catch_types(bpa, c),
        })
        .collect();
    let finally_entry = finally.map(|_| state.new_block());
    state.catch_clauses = state.catch_clauses.saturating_add(catches.len() as u32);

    let outer = state.push_frame(
        FrameKind::Protected {
            handlers: handlers.clone(),
            handlers_active: true,
            finally_entry,
            continuations: Vec::new(),
        },
        Vec::new(),
    );

    // ── Resources (JLS 14.20.3.1) ──
    let mut close_blocks = Vec::with_capacity(resources.len());
    for &r in &resources {
        build_simple(r, state, bpa, sta);
        let close = state.new_block();
        state.push_frame(
            FrameKind::Protected {
                handlers: Vec::new(),
                handlers_active: true,
                finally_entry: Some(close),
                continuations: Vec::new(),
            },
            Vec::new(),
        );
        close_blocks.push(close);
    }

    // ── Body ──
    dispatch_stmt(body, state, bpa, sta);

    // ── Implicit close() of each resource, innermost first ──
    for close in close_blocks.into_iter().rev() {
        let frame = state.pop_frame();
        let body_end = state.current_block;
        let completes_normally = state.is_reachable(body_end);
        state.add_edge(body_end, close, CFGEdgeType::Uncond);
        // close() may throw; the exception propagates to the enclosing scopes.
        state.route_jump(close, Jump::Throw(ThrownType::Unknown));
        if let Some(FrameKind::Protected { continuations, .. }) = frame.map(|f| f.kind) {
            for j in continuations {
                state.route_jump(close, j);
            }
        }
        let next = state.new_block();
        if completes_normally {
            state.add_edge(close, next, CFGEdgeType::Uncond);
        }
        state.current_block = next;
    }

    // ── Normal completion of the protected part ──
    let normal_target = finally_entry.unwrap_or(post_try);
    let body_end = state.current_block;
    let mut normal_into_finally = state.is_reachable(body_end);
    state.add_edge(body_end, normal_target, CFGEdgeType::Uncond);

    // ── Catch clauses: the statement's own handlers are inactive in their bodies ──
    if let FrameKind::Protected {
        handlers_active, ..
    } = &mut state.control[outer].kind
    {
        *handlers_active = false;
    }
    for (&catch_node, handler) in catches.iter().zip(handlers.iter()) {
        state.current_block = handler.block;
        for c in children(bpa, catch_node) {
            match bpa.node_type(c) {
                ASTNodeType::NN_BLOCK => {}
                // The exception parameter (kept as a parameter declaration, or its bare
                // name when Phase 2 eliminated the wrapper) is defined on entry.
                ASTNodeType::NN_PARAM_DECL | ASTNodeType::NN_IDENTIFIER_EXPR => {
                    state.add_stmt_no_implicit(c, bpa);
                }
                _ => {}
            }
        }
        if let Some(catch_body) = first_child_of_type(bpa, catch_node, ASTNodeType::NN_BLOCK) {
            dispatch_stmt(catch_body, state, bpa, sta);
        }
        let catch_end = state.current_block;
        normal_into_finally |= state.is_reachable(catch_end);
        state.add_edge(catch_end, normal_target, CFGEdgeType::Uncond);
    }

    let frame = state.pop_frame();

    // ── Finally ──
    if let (Some(fin_node), Some(fin_entry)) = (finally, finally_entry) {
        state.current_block = fin_entry;
        if let Some(fin_body) = first_child_of_type(bpa, fin_node, ASTNodeType::NN_BLOCK) {
            dispatch_stmt(fin_body, state, bpa, sta);
        }
        let fin_end = state.current_block;
        if normal_into_finally {
            state.add_edge(fin_end, post_try, CFGEdgeType::Uncond);
        }
        if let Some(FrameKind::Protected { continuations, .. }) = frame.map(|f| f.kind) {
            for j in continuations {
                state.route_jump(fin_end, j);
            }
        }
    }

    state.current_block = post_try;
}
