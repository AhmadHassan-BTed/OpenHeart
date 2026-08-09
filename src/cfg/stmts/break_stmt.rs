//! Break Statement CFG builder algorithm (§4.5.3).

use crate::ast::BPASTArtifact;
use crate::cfg::builder::state::CFGBuilderState;
use crate::core::types::cfg::CFGEdgeType;
use crate::symbol::SymbolTableArtifact;

pub fn build_break(
    node: u32,
    state: &mut CFGBuilderState,
    bpa: &BPASTArtifact,
    _sta: &SymbolTableArtifact,
) {
    state.add_stmt_to_current(node, bpa);
    let from = state.current_block;

    let target_label = bpa.first_child(node);

    let target_frame = if let Some(label_node) = target_label {
        state
            .break_stack
            .iter()
            .rev()
            .find(|f| f.label == Some(label_node))
            .or_else(|| state.break_stack.last())
            .copied()
    } else {
        state.break_stack.last().copied()
    };

    if let Some(frame) = target_frame {
        state.add_edge(from, frame.target, CFGEdgeType::Uncond);
    }

    let new_blk = state.new_block();
    state.current_block = new_blk;
}
