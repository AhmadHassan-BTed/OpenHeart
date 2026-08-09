//! Continue Statement CFG builder algorithm.

use crate::ast::BPASTArtifact;
use crate::cfg::builder::state::CFGBuilderState;
use crate::core::types::cfg::CFGEdgeType;
use crate::symbol::SymbolTableArtifact;

pub fn build_continue(
    node: u32,
    state: &mut CFGBuilderState,
    bpa: &BPASTArtifact,
    _sta: &SymbolTableArtifact,
) {
    state.add_stmt_to_current(node, bpa);
    let from = state.current_block;

    if let Some(frame) = state.continue_stack.last().copied() {
        state.add_edge(from, frame.target, CFGEdgeType::LoopBack);
    }

    let new_blk = state.new_block();
    state.current_block = new_blk;
}
