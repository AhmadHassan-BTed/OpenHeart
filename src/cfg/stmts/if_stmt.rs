//! If Statement CFG builder algorithm (§4.5.3).

use crate::ast::BPASTArtifact;
use crate::cfg::ast_query::children;
use crate::cfg::builder::state::CFGBuilderState;
use crate::cfg::stmts::dispatch_stmt;
use crate::cfg::stmts::expr::build_branch;
use crate::core::types::cfg::CFGEdgeType;
use crate::symbol::SymbolTableArtifact;

pub fn build_if(
    node: u32,
    state: &mut CFGBuilderState,
    bpa: &BPASTArtifact,
    sta: &SymbolTableArtifact,
) {
    let kids = children(bpa, node);
    let cond_node = match kids.first() {
        Some(&c) => c,
        None => return,
    };

    // Phase 2 drops an empty `;` branch, so two children are either `if (c) S` or
    // `if (c) ; else S`; the token before the second child tells them apart.
    let (then_node, else_node) = match kids.len() {
        1 => (None, None),
        2 => {
            let second = kids[1];
            let empty_then = state
                .src
                .as_ref()
                .is_some_and(|s| s.text_before(bpa, second) == Some(&b"else"[..]));
            if empty_then {
                (None, Some(second))
            } else {
                (Some(second), None)
            }
        }
        _ => (Some(kids[1]), Some(kids[2])),
    };

    let then_entry = state.new_block();
    let else_entry = else_node.map(|_| state.new_block());
    let join = state.new_block();

    build_branch(
        cond_node,
        then_entry,
        else_entry.unwrap_or(join),
        CFGEdgeType::True,
        state,
        bpa,
        sta,
    );

    // ── THEN branch ──
    state.current_block = then_entry;
    if let Some(t) = then_node {
        dispatch_stmt(t, state, bpa, sta);
    }
    state.goto(join, CFGEdgeType::Uncond);

    // ── ELSE branch ──
    if let (Some(e), Some(entry)) = (else_node, else_entry) {
        state.current_block = entry;
        dispatch_stmt(e, state, bpa, sta);
        state.goto(join, CFGEdgeType::Uncond);
    }

    // ── JOIN block ──
    state.current_block = join;
}
