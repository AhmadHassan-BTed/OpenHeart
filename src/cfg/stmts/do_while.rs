//! Do-While Loop CFG builder algorithm.

use crate::ast::BPASTArtifact;
use crate::cfg::ast_query::children;
use crate::cfg::builder::state::{CFGBuilderState, FrameKind};
use crate::cfg::stmts::dispatch_stmt;
use crate::cfg::stmts::expr::build_branch;
use crate::core::types::cfg::CFGEdgeType;
use crate::symbol::SymbolTableArtifact;

pub fn build_do_while(
    node: u32,
    state: &mut CFGBuilderState,
    bpa: &BPASTArtifact,
    sta: &SymbolTableArtifact,
) {
    let labels = state.take_labels();
    let kids = children(bpa, node);
    // The condition is always the last child; an empty `do ; while (c);` body is
    // dropped by Phase 2.
    let cond = kids.last().copied();
    let body = if kids.len() >= 2 { Some(kids[0]) } else { None };

    let body_entry = state.new_block();
    state.goto(body_entry, CFGEdgeType::Uncond);

    let cond_block = state.new_block();
    let exit = state.new_block();

    state.push_frame(
        FrameKind::Loop {
            break_target: exit,
            continue_target: cond_block,
            continue_edge: CFGEdgeType::Uncond,
        },
        labels,
    );

    if let Some(b) = body {
        dispatch_stmt(b, state, bpa, sta);
    }
    state.goto(cond_block, CFGEdgeType::Uncond);
    state.pop_frame();

    match cond {
        Some(c) => build_branch(c, body_entry, exit, CFGEdgeType::LoopBack, state, bpa, sta),
        None => state.add_edge(cond_block, body_entry, CFGEdgeType::LoopBack),
    }

    state.current_block = exit;
}
