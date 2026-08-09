//! If Statement CFG builder algorithm (§4.5.3).

use crate::ast::BPASTArtifact;
use crate::cfg::builder::state::CFGBuilderState;
use crate::cfg::stmts::dispatch_stmt;
use crate::core::types::cfg::{CFGEdgeType, PendingEdge};
use crate::symbol::SymbolTableArtifact;

pub fn build_if(
    node: u32,
    state: &mut CFGBuilderState,
    bpa: &BPASTArtifact,
    sta: &SymbolTableArtifact,
) {
    let cond_node = match bpa.first_child(node) {
        Some(c) => c,
        None => return,
    };

    let then_node = match bpa.next_sibling(cond_node) {
        Some(t) => t,
        None => return,
    };

    let else_node = bpa.next_sibling(then_node);

    state.add_stmt_to_current(cond_node, bpa);
    let cond_block = state.current_block;

    // ── THEN branch ──
    let then_entry = state.new_block();
    state.add_edge(cond_block, then_entry, CFGEdgeType::True);
    state.current_block = then_entry;
    dispatch_stmt(then_node, state, bpa, sta);
    let then_pending = state.drain_pending();

    // ── ELSE branch ──
    let else_pending = if let Some(else_n) = else_node {
        let else_entry = state.new_block();
        state.add_edge(cond_block, else_entry, CFGEdgeType::False);
        state.current_block = else_entry;
        dispatch_stmt(else_n, state, bpa, sta);
        state.drain_pending()
    } else {
        vec![PendingEdge {
            from: cond_block,
            edge_type: CFGEdgeType::False,
        }]
    };

    // ── JOIN block ──
    let join = state.new_block();
    for pe in then_pending.into_iter().chain(else_pending) {
        state.add_edge(pe.from, join, pe.edge_type);
    }
    state.current_block = join;
}
