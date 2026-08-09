//! Return Statement CFG builder algorithm.

use crate::ast::BPASTArtifact;
use crate::cfg::builder::state::CFGBuilderState;
use crate::core::types::cfg::CFGEdgeType;
use crate::symbol::SymbolTableArtifact;

pub fn build_return(
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
    let exit = state.exit_block;
    state.add_edge(from, exit, CFGEdgeType::Return);

    // Unreachable code after return: open a fresh isolated block
    let new_blk = state.new_block();
    state.current_block = new_blk;
}
