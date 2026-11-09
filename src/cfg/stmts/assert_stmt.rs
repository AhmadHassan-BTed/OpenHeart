//! Assert Statement CFG builder algorithm (JLS 14.10).
//!
//! Modelled with assertions enabled: the condition is evaluated and, when false, the
//! optional detail expression is evaluated and an `AssertionError` is thrown.

use crate::ast::BPASTArtifact;
use crate::cfg::ast_query::children;
use crate::cfg::builder::state::{CFGBuilderState, Jump};
use crate::cfg::stmts::expr::{build_branch, build_simple};
use crate::core::types::cfg::CFGEdgeType;
use crate::symbol::SymbolTableArtifact;

pub fn build_assert(
    node: u32,
    state: &mut CFGBuilderState,
    bpa: &BPASTArtifact,
    sta: &SymbolTableArtifact,
) {
    let kids = children(bpa, node);
    let cond = match kids.first() {
        Some(&c) => c,
        None => return,
    };

    let ok = state.new_block();
    let fail = state.new_block();
    build_branch(cond, ok, fail, CFGEdgeType::True, state, bpa, sta);

    state.current_block = fail;
    if let Some(&detail) = kids.get(1) {
        build_simple(detail, state, bpa, sta);
    }
    let thrown = state.exceptions.named(b"AssertionError");
    let from = state.current_block;
    state.route_jump(from, Jump::Throw(thrown));

    state.current_block = ok;
}
