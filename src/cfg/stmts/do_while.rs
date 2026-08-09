//! Do-While Loop CFG builder algorithm.

use crate::ast::BPASTArtifact;
use crate::cfg::builder::state::CFGBuilderState;
use crate::cfg::stmts::dispatch_stmt;
use crate::core::types::cfg::CFGEdgeType;
use crate::symbol::SymbolTableArtifact;

pub fn build_do_while(
    node: u32,
    state: &mut CFGBuilderState,
    bpa: &BPASTArtifact,
    sta: &SymbolTableArtifact,
) {
    let body_node = match bpa.first_child(node) {
        Some(b) => b,
        None => return,
    };
    let cond_node = match bpa.next_sibling(body_node) {
        Some(c) => c,
        None => return,
    };

    let body_entry = state.new_block();
    state.flush_pending(body_entry);

    let cond_block = state.new_block();
    let exit = state.new_block();

    state.push_break(exit, None);
    state.push_continue(cond_block, None);

    state.current_block = body_entry;
    dispatch_stmt(body_node, state, bpa, sta);

    state.flush_pending(cond_block);
    state.add_stmt_to_block(cond_block, cond_node, bpa);

    state.pop_continue();
    state.pop_break();

    state.add_edge(cond_block, body_entry, CFGEdgeType::LoopBack);
    state.add_edge(cond_block, exit, CFGEdgeType::False);

    state.current_block = exit;
}
