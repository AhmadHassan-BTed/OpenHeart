//! While Loop CFG builder algorithm (§4.5.4), and the pre-test loop lowering shared with
//! `for` loops.

use crate::ast::BPASTArtifact;
use crate::cfg::ast_query::children;
use crate::cfg::builder::state::{CFGBuilderState, FrameKind, LabelRef};
use crate::cfg::stmts::dispatch_stmt;
use crate::cfg::stmts::expr::{build_branch, build_simple};
use crate::core::types::cfg::CFGEdgeType;
use crate::symbol::SymbolTableArtifact;

pub fn build_while(
    node: u32,
    state: &mut CFGBuilderState,
    bpa: &BPASTArtifact,
    sta: &SymbolTableArtifact,
) {
    let labels = state.take_labels();
    let kids = children(bpa, node);
    let cond = kids.first().copied();
    // `while (c);` has no body child: Phase 2 drops the empty statement.
    let body = kids.get(1).copied();
    build_pretest_loop(cond, body, &[], labels, state, bpa, sta);
}

/// Lowers `header: if (!cond) goto exit; body; update; goto header`.
///
/// * `cond == None` (`for (;;)`) and constant-true conditions get no exit edge;
/// * `continue` targets the update block when there are update expressions, otherwise
///   the header (a back edge);
/// * the loop frame carries the labels of enclosing labeled statements.
pub(crate) fn build_pretest_loop(
    cond: Option<u32>,
    body: Option<u32>,
    update: &[u32],
    labels: Vec<LabelRef>,
    state: &mut CFGBuilderState,
    bpa: &BPASTArtifact,
    sta: &SymbolTableArtifact,
) {
    let header = state.new_block();
    state.goto(header, CFGEdgeType::Uncond);

    let body_entry = state.new_block();
    let exit = state.new_block();
    let update_block = if update.is_empty() {
        None
    } else {
        Some(state.new_block())
    };
    let (continue_target, continue_edge) = match update_block {
        Some(u) => (u, CFGEdgeType::Uncond),
        None => (header, CFGEdgeType::LoopBack),
    };

    state.push_frame(
        FrameKind::Loop {
            break_target: exit,
            continue_target,
            continue_edge,
        },
        labels,
    );

    state.current_block = header;
    match cond {
        Some(c) => build_branch(c, body_entry, exit, CFGEdgeType::True, state, bpa, sta),
        None => state.add_edge(header, body_entry, CFGEdgeType::Uncond),
    }

    state.current_block = body_entry;
    if let Some(b) = body {
        dispatch_stmt(b, state, bpa, sta);
    }

    match update_block {
        Some(u) => {
            state.goto(u, CFGEdgeType::Uncond);
            state.pop_frame();
            for &upd in update {
                build_simple(upd, state, bpa, sta);
            }
            let tail = state.current_block;
            state.add_edge(tail, header, CFGEdgeType::LoopBack);
        }
        None => {
            let tail = state.current_block;
            state.add_edge(tail, header, CFGEdgeType::LoopBack);
            state.pop_frame();
        }
    }

    state.current_block = exit;
}
