//! Throw Statement CFG builder algorithm (§4.5.7).

use crate::ast::BPASTArtifact;
use crate::cfg::builder::state::CFGBuilderState;
use crate::core::types::cfg::CFGEdgeType;
use crate::symbol::SymbolTableArtifact;

pub fn build_throw(
    node: u32,
    state: &mut CFGBuilderState,
    bpa: &BPASTArtifact,
    _sta: &SymbolTableArtifact,
) {
    if let Some(expr) = bpa.first_child(node) {
        state.add_stmt_to_current(expr, bpa);
    }
    state.add_stmt_to_current(node, bpa);
    let from = state.current_block;

    let mut handled = false;
    for frame in state.exception_stack.clone().iter().rev() {
        if let Some((_, catch_block)) = frame.handlers.first() {
            state.add_edge(from, *catch_block, CFGEdgeType::Except);
            handled = true;
            break;
        }
        if let Some(finally) = frame.finally_block {
            state.add_edge(from, finally, CFGEdgeType::Except);
            handled = true;
            break;
        }
    }

    if !handled {
        let exit = state.exit_block;
        state.add_edge(from, exit, CFGEdgeType::Except);
    }

    let new_blk = state.new_block();
    state.current_block = new_blk;
}
