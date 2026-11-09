//! Continue Statement CFG builder algorithm (§4.5.3).

use crate::ast::BPASTArtifact;
use crate::cfg::builder::state::{CFGBuilderState, Jump};
use crate::core::types::ast::ASTNodeType;
use crate::symbol::SymbolTableArtifact;

pub fn build_continue(
    node: u32,
    state: &mut CFGBuilderState,
    bpa: &BPASTArtifact,
    _sta: &SymbolTableArtifact,
) {
    state.add_stmt_no_implicit(node, bpa);

    let label = bpa
        .first_child(node)
        .filter(|&l| bpa.node_type(l) == ASTNodeType::NN_IDENTIFIER_EXPR)
        .map(|l| state.label_ref(bpa, l));

    // See `build_break` for the handling of unresolvable targets.
    if let Some(frame) = state.find_continue(label, node) {
        let from = state.current_block;
        state.route_jump(from, Jump::Continue(frame));
        state.terminate();
    }
}
