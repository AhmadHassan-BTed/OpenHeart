//! Yield Statement CFG builder algorithm (JLS 14.21).

use crate::ast::BPASTArtifact;
use crate::cfg::builder::state::{CFGBuilderState, CfgDiagnosticKind, Jump};
use crate::cfg::stmts::expr::lower_embedded;
use crate::symbol::SymbolTableArtifact;

pub fn build_yield(
    node: u32,
    state: &mut CFGBuilderState,
    bpa: &BPASTArtifact,
    sta: &SymbolTableArtifact,
) {
    lower_embedded(node, state, bpa, sta);
    state.add_stmt(node, bpa);

    match state.find_yield() {
        Some(frame) => {
            let from = state.current_block;
            state.route_jump(from, Jump::Yield(frame));
            state.terminate();
        }
        None => state.diag(CfgDiagnosticKind::YieldOutsideSwitchExpression, node),
    }
}
