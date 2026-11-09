//! Switch Statement and Switch Expression CFG builder algorithms (JLS 14.11, 15.28).
//!
//! Expected AST shape: the switch node's first child is the selector, followed by one
//! `NN_SWITCH_CASE` per `switch_block_statement_group` / `switch_rule` (directly, or
//! inside an `NN_BLOCK` standing for the switch block). A case node holds its label
//! expressions / patterns first and its statements after them; labels are always
//! expressions, so the first statement-kind child starts the body.
//!
//! * colon groups fall through to the next group; arrow rules never do;
//! * a statement switch without `default` gets a selector edge to the exit;
//! * a switch expression without `default` is exhaustive (JLS 15.28.1): an unmatched
//!   value raises an exception instead of completing normally;
//! * non-literal labels (`Color.RED`, qualified constants, patterns) are ordinary labels,
//!   never a default.

use crate::ast::BPASTArtifact;
use crate::cfg::ast_query::{children, is_statement_kind};
use crate::cfg::builder::exceptions::ThrownType;
use crate::cfg::builder::state::{CFGBuilderState, CfgDiagnosticKind, FrameKind, Jump};
use crate::cfg::stmts::dispatch_stmt;
use crate::cfg::stmts::expr::{build_simple, lower_embedded};
use crate::core::types::ast::ASTNodeType;
use crate::core::types::cfg::CFGEdgeType;
use crate::symbol::SymbolTableArtifact;

/// Decomposition of one case group / rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaseInfo {
    pub node: u32,
    pub labels: Vec<u32>,
    pub body: Vec<u32>,
    pub is_default: bool,
    /// `Some(true)` for an arrow rule, `Some(false)` for a colon group, `None` when the
    /// token text needed to tell them apart is unavailable.
    pub arrow: Option<bool>,
}

/// Case nodes of a switch, in source order.
pub fn switch_cases(bpa: &BPASTArtifact, node: u32) -> Vec<u32> {
    let mut out = Vec::new();
    for c in children(bpa, node).into_iter().skip(1) {
        match bpa.node_type(c) {
            ASTNodeType::NN_SWITCH_CASE => out.push(c),
            ASTNodeType::NN_BLOCK => {
                for g in children(bpa, c) {
                    if bpa.node_type(g) == ASTNodeType::NN_SWITCH_CASE {
                        out.push(g);
                    }
                }
            }
            _ => {}
        }
    }
    out
}

pub fn case_info(case: u32, state: &CFGBuilderState, bpa: &BPASTArtifact) -> CaseInfo {
    let kids = children(bpa, case);
    let split = kids
        .iter()
        .position(|&c| is_statement_kind(bpa.node_type(c)))
        .unwrap_or(kids.len());
    let labels = kids[..split].to_vec();
    let body = kids[split..].to_vec();

    let (case_ft, case_lt) = bpa.token_range(case);
    let label_end = match body.first() {
        Some(&b) => {
            let (bft, _) = bpa.token_range(b);
            if bft == u32::MAX || bft == 0 {
                case_lt
            } else {
                bft - 1
            }
        }
        None => case_lt,
    };

    match state.src.as_ref() {
        Some(s) if case_ft != u32::MAX => CaseInfo {
            node: case,
            is_default: s.range_contains(case_ft, label_end, b"default"),
            arrow: Some(s.range_contains(case_ft, label_end, b"->")),
            labels,
            body,
        },
        _ => CaseInfo {
            node: case,
            // A group always has a label; with `default` dropped by Phase 2 a group
            // without label expressions can only be the default group.
            is_default: labels.is_empty(),
            arrow: None,
            labels,
            body,
        },
    }
}

/// Switch in statement position (`NN_SWITCH_STMT`, or `NN_SWITCH_EXPR` used as a
/// statement): `break` exits the switch.
pub fn build_switch(
    node: u32,
    state: &mut CFGBuilderState,
    bpa: &BPASTArtifact,
    sta: &SymbolTableArtifact,
) {
    let labels = state.take_labels();
    let kids = children(bpa, node);
    if let Some(&selector) = kids.first() {
        build_simple(selector, state, bpa, sta);
    }
    let selector_block = state.current_block;

    let exit = state.new_block();
    state.push_frame(FrameKind::Switch { break_target: exit }, labels);

    let mut fallthrough: Option<u32> = None;
    let mut has_default = false;
    let mut kind_unknown = false;

    for case in switch_cases(bpa, node) {
        let info = case_info(case, state, bpa);
        kind_unknown |= info.arrow.is_none();
        has_default |= info.is_default;

        let entry = state.new_block();
        state.add_edge(selector_block, entry, CFGEdgeType::Switch);
        if let Some(prev) = fallthrough {
            state.add_edge(prev, entry, CFGEdgeType::Uncond);
        }
        state.current_block = entry;
        for &l in &info.labels {
            state.add_stmt_no_implicit(l, bpa);
        }
        for &s in &info.body {
            dispatch_stmt(s, state, bpa, sta);
        }

        if info.arrow == Some(true) {
            state.goto(exit, CFGEdgeType::Uncond);
            fallthrough = None;
        } else {
            fallthrough = Some(state.current_block);
        }
    }

    if let Some(prev) = fallthrough {
        state.add_edge(prev, exit, CFGEdgeType::Uncond);
    }
    if !has_default {
        state.add_edge(selector_block, exit, CFGEdgeType::Switch);
    }
    if kind_unknown {
        state.diag(CfgDiagnosticKind::SwitchRuleKindUnknown, node);
    }

    state.pop_frame();
    state.current_block = exit;
}

/// Switch expression in a value context: `yield` (or the expression of an arrow rule)
/// delivers the value to the join block, which is current on return.
pub fn build_switch_value(
    node: u32,
    state: &mut CFGBuilderState,
    bpa: &BPASTArtifact,
    sta: &SymbolTableArtifact,
) {
    let kids = children(bpa, node);
    if let Some(&selector) = kids.first() {
        lower_embedded(selector, state, bpa, sta);
        state.add_stmt(selector, bpa);
    }
    let selector_block = state.current_block;

    let join = state.new_block();
    state.push_frame(FrameKind::SwitchExpr { yield_target: join }, Vec::new());

    let mut fallthrough: Option<u32> = None;
    let mut has_default = false;
    let mut kind_unknown = false;

    for case in switch_cases(bpa, node) {
        let info = case_info(case, state, bpa);
        kind_unknown |= info.arrow.is_none();
        has_default |= info.is_default;

        let entry = state.new_block();
        state.add_edge(selector_block, entry, CFGEdgeType::Switch);
        if let Some(prev) = fallthrough {
            state.add_edge(prev, entry, CFGEdgeType::Uncond);
        }
        state.current_block = entry;
        for &l in &info.labels {
            state.add_stmt_no_implicit(l, bpa);
        }

        let is_value_arm = info.arrow == Some(true)
            && info.body.len() == 1
            && bpa.node_type(info.body[0]) == ASTNodeType::NN_EXPR_STMT;
        if is_value_arm {
            build_simple(info.body[0], state, bpa, sta);
        } else {
            for &s in &info.body {
                dispatch_stmt(s, state, bpa, sta);
            }
        }

        if info.arrow == Some(true) {
            // Valid code reaches the end of a block / throw arm only unreachably.
            state.goto(join, CFGEdgeType::Uncond);
            fallthrough = None;
        } else {
            fallthrough = Some(state.current_block);
        }
    }

    if let Some(prev) = fallthrough {
        state.add_edge(prev, join, CFGEdgeType::Uncond);
    }
    if !has_default {
        // Exhaustive switch expression: an unmatched value throws (MatchException /
        // IncompatibleClassChangeError).
        state.route_jump(selector_block, Jump::Throw(ThrownType::Unknown));
    }
    if kind_unknown {
        state.diag(CfgDiagnosticKind::SwitchRuleKindUnknown, node);
    }

    state.pop_frame();
    state.current_block = join;
}
