//! Throw Statement CFG builder algorithm (§4.5.7).

use crate::ast::BPASTArtifact;
use crate::cfg::ast_query::NewParts;
use crate::cfg::builder::state::{CFGBuilderState, Jump};
use crate::cfg::stmts::expr::lower_embedded;
use crate::core::types::ast::ASTNodeType;
use crate::symbol::SymbolTableArtifact;

pub fn build_throw(
    node: u32,
    state: &mut CFGBuilderState,
    bpa: &BPASTArtifact,
    sta: &SymbolTableArtifact,
) {
    lower_embedded(node, state, bpa, sta);

    // Evaluating the operand can itself raise an exception (e.g. `throw new E(f())`).
    // Constructing the thrown exception object is modelled as non-throwing, so
    // `throw new E("msg")` is routed with its exact type only.
    let operand_may_throw = match bpa.first_child(node) {
        Some(expr) if bpa.node_type(expr) == ASTNodeType::NN_NEW_EXPR => {
            let parts = NewParts::of(bpa, state.src.as_ref(), expr);
            parts.args.iter().any(|&a| state.may_throw(a, bpa))
        }
        Some(expr) => state.may_throw(expr, bpa),
        None => false,
    };
    if operand_may_throw {
        state.add_stmt(node, bpa);
    } else {
        state.add_stmt_no_implicit(node, bpa);
    }

    // Exceptional edges go to the innermost handlers whose type may match, then through
    // finally blocks and outer frames, finally to EXIT.
    let thrown = state.exceptions.thrown_type(bpa, node);
    let from = state.current_block;
    state.route_jump(from, Jump::Throw(thrown));

    state.terminate();
}
