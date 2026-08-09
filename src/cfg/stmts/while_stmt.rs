//! While Loop CFG builder algorithm (§4.5.4).

use crate::ast::BPASTArtifact;
use crate::cfg::builder::state::CFGBuilderState;
use crate::cfg::stmts::dispatch_stmt;
use crate::core::types::cfg::CFGEdgeType;
use crate::symbol::SymbolTableArtifact;

pub fn build_while(
    node: u32,
    state: &mut CFGBuilderState,
    bpa: &BPASTArtifact,
    sta: &SymbolTableArtifact,
) {
    let cond_node = match bpa.first_child(node) {
        Some(c) => c,
        None => return,
    };
    let body_node = match bpa.next_sibling(cond_node) {
        Some(b) => b,
        None => return,
    };

    let header = state.new_block();
    state.flush_pending(header);
    state.add_stmt_to_block(header, cond_node, bpa);

    let exit = state.new_block();

    state.push_break(exit, None);
    state.push_continue(header, None);

    let body_entry = state.new_block();
    state.add_edge(header, body_entry, CFGEdgeType::True);
    state.current_block = body_entry;
    dispatch_stmt(body_node, state, bpa, sta);

    state.flush_pending_with_type(header, CFGEdgeType::LoopBack);

    state.pop_continue();
    state.pop_break();

    state.add_edge(header, exit, CFGEdgeType::False);
    state.current_block = exit;
}
