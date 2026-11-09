//! Return Statement CFG builder algorithm.

use crate::ast::BPASTArtifact;
use crate::cfg::builder::state::{CFGBuilderState, Jump};
use crate::cfg::stmts::expr::lower_embedded;
use crate::symbol::SymbolTableArtifact;

pub fn build_return(
    node: u32,
    state: &mut CFGBuilderState,
    bpa: &BPASTArtifact,
    sta: &SymbolTableArtifact,
) {
    // The return statement node covers its operand, so it is added once; evaluating the
    // operand may raise an exception before the method returns.
    lower_embedded(node, state, bpa, sta);
    state.add_stmt(node, bpa);

    // Routed through every enclosing finally / synchronized release to EXIT.
    let from = state.current_block;
    state.route_jump(from, Jump::Return);

    // Unreachable code after return: open a fresh isolated block
    state.terminate();
}
