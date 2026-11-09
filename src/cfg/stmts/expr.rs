//! Lowering of control flow inside expressions (§4.5.3).
//!
//! Short-circuit operators, conditional expressions and switch expressions are lowered
//! into branch blocks, as javac and Jimple do. In a branch condition each operand of
//! `&&` / `||` / `!` / `?:` gets its own block and edges; in a value context the
//! control-flow sub-expressions of a statement are evaluated first, in source order, and
//! the residual statement is placed in the join block. Every lowered node is recorded in
//! `CFGBuilderState::lowered`: its evaluation is represented by the blocks that hold its
//! operands, so consumers walking a statement subtree must not descend into it again.

use crate::ast::BPASTArtifact;
use crate::cfg::ast_query::{
    children, is_logical_and, is_logical_not, is_logical_or, is_nested_body, literal_text,
    subtree_end,
};
use crate::cfg::builder::state::CFGBuilderState;
use crate::cfg::stmts::switch_stmt::build_switch_value;
use crate::core::types::ast::ASTNodeType;
use crate::core::types::cfg::CFGEdgeType;
use crate::symbol::SymbolTableArtifact;

/// Builds the evaluation of condition `cond`, starting in the current block, with edges
/// to `t` when it is true (typed `true_edge`) and to `f` when it is false. The current
/// block afterwards is unspecified; callers set it.
pub fn build_branch(
    cond: u32,
    t: u32,
    f: u32,
    true_edge: CFGEdgeType,
    state: &mut CFGBuilderState,
    bpa: &BPASTArtifact,
    sta: &SymbolTableArtifact,
) {
    let ntype = bpa.node_type(cond);
    let kids = children(bpa, cond);

    if is_logical_and(bpa, cond) && kids.len() == 2 {
        let mid = state.new_block();
        build_branch(kids[0], mid, f, CFGEdgeType::True, state, bpa, sta);
        state.current_block = mid;
        build_branch(kids[1], t, f, true_edge, state, bpa, sta);
        state.mark_lowered(cond);
        return;
    }
    if is_logical_or(bpa, cond) && kids.len() == 2 {
        let mid = state.new_block();
        build_branch(kids[0], t, mid, true_edge, state, bpa, sta);
        state.current_block = mid;
        build_branch(kids[1], t, f, true_edge, state, bpa, sta);
        state.mark_lowered(cond);
        return;
    }
    if ntype == ASTNodeType::NN_TERNARY_EXPR && kids.len() == 3 {
        let then_b = state.new_block();
        let else_b = state.new_block();
        build_branch(kids[0], then_b, else_b, CFGEdgeType::True, state, bpa, sta);
        state.current_block = then_b;
        build_branch(kids[1], t, f, true_edge, state, bpa, sta);
        state.current_block = else_b;
        build_branch(kids[2], t, f, true_edge, state, bpa, sta);
        state.mark_lowered(cond);
        return;
    }
    let negated = state
        .src
        .as_ref()
        .is_some_and(|s| is_logical_not(bpa, s, cond));
    if negated && kids.len() == 1 {
        // Evaluate the operand into fresh blocks, then connect them with swapped polarity
        // so the edge into `t` keeps its type (a negated loop condition still yields a
        // back edge) and the edge into `f` stays a false edge.
        let op_true = state.new_block();
        let op_false = state.new_block();
        build_branch(kids[0], op_true, op_false, CFGEdgeType::True, state, bpa, sta);
        state.add_edge(op_true, f, CFGEdgeType::False);
        state.add_edge(op_false, t, true_edge);
        state.mark_lowered(cond);
        return;
    }
    if ntype == ASTNodeType::NN_SWITCH_EXPR {
        lower_value(cond, state, bpa, sta);
        let c = state.current_block;
        state.add_edge(c, t, true_edge);
        state.add_edge(c, f, CFGEdgeType::False);
        return;
    }

    // Constant conditions get only the edge that can be taken.
    let constant = state
        .src
        .as_ref()
        .and_then(|s| literal_text(bpa, s, cond))
        .and_then(|txt| match txt {
            b"true" => Some(true),
            b"false" => Some(false),
            _ => None,
        });

    lower_embedded(cond, state, bpa, sta);
    state.add_stmt(cond, bpa);
    let c = state.current_block;
    match constant {
        Some(true) => state.add_edge(c, t, true_edge),
        Some(false) => state.add_edge(c, f, CFGEdgeType::False),
        None => {
            state.add_edge(c, t, true_edge);
            state.add_edge(c, f, CFGEdgeType::False);
        }
    }
}

/// Ternaries, short-circuit operators and switch expressions.
pub fn is_control_expression(node: u32, bpa: &BPASTArtifact) -> bool {
    let t = bpa.node_type(node);
    t == ASTNodeType::NN_TERNARY_EXPR
        || t == ASTNodeType::NN_SWITCH_EXPR
        || is_logical_and(bpa, node)
        || is_logical_or(bpa, node)
}

/// Lowers the control-flow sub-expressions of `root` (excluding `root` itself) in source
/// order. Nested lambda / class bodies are separate functions and are not entered.
pub fn lower_embedded(
    root: u32,
    state: &mut CFGBuilderState,
    bpa: &BPASTArtifact,
    sta: &SymbolTableArtifact,
) {
    let end = subtree_end(bpa, root);
    let mut k = root.saturating_add(1);
    while k < end {
        let t = bpa.node_type(k);
        if is_nested_body(t) || state.lowered.contains(&k) {
            k = subtree_end(bpa, k);
            continue;
        }
        if is_control_expression(k, bpa) {
            lower_value(k, state, bpa, sta);
            k = subtree_end(bpa, k);
            continue;
        }
        k += 1;
    }
}

/// Lowers one control-flow expression used as a value; on return the current block is
/// the join block where the value is available.
pub fn lower_value(
    node: u32,
    state: &mut CFGBuilderState,
    bpa: &BPASTArtifact,
    sta: &SymbolTableArtifact,
) {
    let kids = children(bpa, node);
    let t = bpa.node_type(node);

    if t == ASTNodeType::NN_SWITCH_EXPR {
        build_switch_value(node, state, bpa, sta);
        state.mark_lowered(node);
        return;
    }

    if t == ASTNodeType::NN_TERNARY_EXPR && kids.len() == 3 {
        let then_b = state.new_block();
        let else_b = state.new_block();
        let join = state.new_block();
        build_branch(kids[0], then_b, else_b, CFGEdgeType::True, state, bpa, sta);
        for (entry, operand) in [(then_b, kids[1]), (else_b, kids[2])] {
            state.current_block = entry;
            lower_operand(operand, state, bpa, sta);
            state.goto(join, CFGEdgeType::Uncond);
        }
        state.current_block = join;
        state.mark_lowered(node);
        return;
    }

    if (is_logical_and(bpa, node) || is_logical_or(bpa, node)) && kids.len() == 2 {
        let rhs = state.new_block();
        let join = state.new_block();
        if is_logical_and(bpa, node) {
            build_branch(kids[0], rhs, join, CFGEdgeType::True, state, bpa, sta);
        } else {
            build_branch(kids[0], join, rhs, CFGEdgeType::True, state, bpa, sta);
        }
        state.current_block = rhs;
        lower_operand(kids[1], state, bpa, sta);
        state.goto(join, CFGEdgeType::Uncond);
        state.current_block = join;
        state.mark_lowered(node);
        return;
    }

    // Unexpected shape (wrong arity): keep it as an ordinary expression.
    lower_embedded(node, state, bpa, sta);
}

/// Evaluates one operand of a lowered expression into the current block.
fn lower_operand(
    operand: u32,
    state: &mut CFGBuilderState,
    bpa: &BPASTArtifact,
    sta: &SymbolTableArtifact,
) {
    if is_control_expression(operand, bpa) {
        lower_value(operand, state, bpa, sta);
    } else {
        lower_embedded(operand, state, bpa, sta);
        state.add_stmt(operand, bpa);
    }
}

/// An expression statement or any statement without control flow of its own.
pub fn build_simple(
    node: u32,
    state: &mut CFGBuilderState,
    bpa: &BPASTArtifact,
    sta: &SymbolTableArtifact,
) {
    lower_embedded(node, state, bpa, sta);
    state.add_stmt(node, bpa);
}
