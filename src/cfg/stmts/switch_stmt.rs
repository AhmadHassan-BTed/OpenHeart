//! Switch Statement CFG builder algorithm.

use crate::ast::BPASTArtifact;
use crate::cfg::builder::state::CFGBuilderState;
use crate::cfg::stmts::dispatch_stmt;
use crate::core::types::ast::ASTNodeType;
use crate::core::types::cfg::CFGEdgeType;
use crate::symbol::SymbolTableArtifact;

pub fn build_switch(
    node: u32,
    state: &mut CFGBuilderState,
    bpa: &BPASTArtifact,
    sta: &SymbolTableArtifact,
) {
    let selector_node = match bpa.first_child(node) {
        Some(s) => s,
        None => return,
    };

    state.add_stmt_to_current(selector_node, bpa);
    let selector_block = state.current_block;

    let exit = state.new_block();
    state.push_break(exit, None);

    let mut case_groups = Vec::new();
    let mut cur = bpa.next_sibling(selector_node);
    while let Some(c) = cur {
        if bpa.node_type(c) == ASTNodeType::NN_SWITCH_CASE {
            case_groups.push(c);
        }
        cur = bpa.next_sibling(c);
    }

    let mut prev_case_exit = None;
    let mut has_default = false;

    for case_node in case_groups {
        let case_entry = state.new_block();
        state.add_edge(selector_block, case_entry, CFGEdgeType::Switch);

        // Handle fall-through from previous case
        if let Some(prev) = prev_case_exit {
            state.add_edge(prev, case_entry, CFGEdgeType::Uncond);
        }

        let mut is_label = false;
        let first_child = bpa.first_child(case_node);
        if let Some(fc) = first_child {
            let ntype = bpa.node_type(fc);
            if ntype == ASTNodeType::NN_LITERAL || ntype == ASTNodeType::NN_IDENTIFIER_EXPR {
                // Label
                state.add_stmt_to_block(case_entry, fc, bpa);
                is_label = true;
            } else {
                has_default = true;
            }
        }

        state.current_block = case_entry;

        // Process statements inside case group (skip label node if present)
        let mut stmt = if is_label && first_child.is_some() {
            bpa.next_sibling(first_child.unwrap())
        } else {
            first_child
        };
        while let Some(s) = stmt {
            dispatch_stmt(s, state, bpa, sta);
            stmt = bpa.next_sibling(s);
        }

        prev_case_exit = Some(state.current_block);
    }

    state.pop_break();

    // If fall-through off end of last case
    if let Some(prev) = prev_case_exit {
        state.add_edge(prev, exit, CFGEdgeType::Uncond);
    }

    // If no default case: selector jumps directly to exit on unhandled value
    if !has_default {
        state.add_edge(selector_block, exit, CFGEdgeType::False);
    }

    state.flush_pending(exit);
    state.current_block = exit;
}
