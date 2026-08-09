//! For Loop & Enhanced For Loop CFG builder algorithms (§4.5.5).

use crate::ast::BPASTArtifact;
use crate::cfg::builder::state::CFGBuilderState;
use crate::cfg::stmts::dispatch_stmt;
use crate::core::types::cfg::CFGEdgeType;
use crate::symbol::SymbolTableArtifact;

pub fn build_for(
    node: u32,
    state: &mut CFGBuilderState,
    bpa: &BPASTArtifact,
    sta: &SymbolTableArtifact,
) {
    let mut children = Vec::new();
    let mut cur = bpa.first_child(node);
    while let Some(c) = cur {
        children.push(c);
        cur = bpa.next_sibling(c);
    }

    if children.is_empty() {
        return;
    }

    let (init_stmts, cond_expr, update_expr, body_node) = match children.len() {
        1 => (None, None, None, children[0]),
        2 => (Some(children[0]), None, None, children[1]),
        3 => (Some(children[0]), Some(children[1]), None, children[2]),
        _ => (
            Some(children[0]),
            Some(children[1]),
            Some(children[2]),
            children[3],
        ),
    };

    if let Some(init) = init_stmts {
        state.add_stmt_to_current(init, bpa);
    }

    let header = state.new_block();
    state.flush_pending(header);
    if state.current_block != header {
        state.add_edge(state.current_block, header, CFGEdgeType::Uncond);
    }

    if let Some(cond) = cond_expr {
        state.add_stmt_to_block(header, cond, bpa);
    }

    let exit = state.new_block();
    let update_block = state.new_block();

    state.push_break(exit, None);
    state.push_continue(update_block, None);

    let body_entry = state.new_block();
    state.add_edge(header, body_entry, CFGEdgeType::True);
    state.current_block = body_entry;
    dispatch_stmt(body_node, state, bpa, sta);

    if state.current_block != body_entry || !state.blocks[body_entry as usize].stmts.is_empty() {
        state.add_edge(state.current_block, update_block, CFGEdgeType::Uncond);
    }
    state.flush_pending(update_block);

    if let Some(upd) = update_expr {
        state.add_stmt_to_block(update_block, upd, bpa);
    }
    state.add_edge(update_block, header, CFGEdgeType::LoopBack);

    state.pop_continue();
    state.pop_break();

    if cond_expr.is_some() {
        state.add_edge(header, exit, CFGEdgeType::False);
    }
    state.current_block = exit;
}

pub fn build_enhanced_for(
    node: u32,
    state: &mut CFGBuilderState,
    bpa: &BPASTArtifact,
    sta: &SymbolTableArtifact,
) {
    let var_node = match bpa.first_child(node) {
        Some(v) => v,
        None => return,
    };

    let iter_node = match bpa.next_sibling(var_node) {
        Some(i) => i,
        None => return,
    };

    let body_node = match bpa.next_sibling(iter_node) {
        Some(b) => b,
        None => return,
    };

    state.add_stmt_to_current(var_node, bpa);
    state.add_stmt_to_current(iter_node, bpa);

    let header = state.new_block();
    state.flush_pending(header);
    if state.current_block != header {
        state.add_edge(state.current_block, header, CFGEdgeType::Uncond);
    }

    let exit = state.new_block();

    state.push_break(exit, None);
    state.push_continue(header, None);

    let body_entry = state.new_block();
    state.add_edge(header, body_entry, CFGEdgeType::True);
    state.current_block = body_entry;
    dispatch_stmt(body_node, state, bpa, sta);

    if state.current_block != body_entry || !state.blocks[body_entry as usize].stmts.is_empty() {
        state.add_edge(state.current_block, header, CFGEdgeType::LoopBack);
    }
    state.flush_pending_with_type(header, CFGEdgeType::LoopBack);

    state.pop_continue();
    state.pop_break();

    state.add_edge(header, exit, CFGEdgeType::False);
    state.current_block = exit;
}
