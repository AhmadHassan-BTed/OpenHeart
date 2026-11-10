//! For Loop & Enhanced For Loop CFG builder algorithms (§4.5.5).
//!
//! Phase 2 drops tree-sitter field names and the `;` / `:` punctuation, so the roles of
//! the header children are recovered from the token text: the two depth-1 semicolons of
//! the `for (...)` header delimit init, condition and update, and the closing `)` marks
//! the body. Without token text the roles are inferred from child positions (the body
//! is always the last child) and the approximation is recorded as a diagnostic.

use crate::ast::BPASTArtifact;
use crate::cfg::ast_query::{children, ForEachParts};
use crate::cfg::builder::state::{CFGBuilderState, CfgDiagnosticKind, FrameKind};
use crate::cfg::stmts::dispatch_stmt;
use crate::cfg::stmts::expr::build_simple;
use crate::cfg::stmts::while_stmt::build_pretest_loop;
use crate::core::types::ast::ASTNodeType;
use crate::core::types::cfg::CFGEdgeType;
use crate::symbol::SymbolTableArtifact;

/// Roles of the children of a basic `for` statement.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ForParts {
    pub init: Vec<u32>,
    pub cond: Option<u32>,
    pub update: Vec<u32>,
    pub body: Option<u32>,
    pub exact: bool,
}

pub fn for_parts(node: u32, state: &CFGBuilderState, bpa: &BPASTArtifact) -> ForParts {
    let kids = children(bpa, node);
    let mut parts = ForParts::default();

    let header = state.src.as_ref().and_then(|s| {
        let (ft, _) = bpa.token_range(node);
        if ft == u32::MAX {
            None
        } else {
            s.paren_header(ft, Some(b";".as_slice()))
        }
    });

    if let Some(h) = header.filter(|h| h.separators.len() >= 2) {
        let (s1, s2) = (h.separators[0], h.separators[1]);
        let mut all_positioned = true;
        for &c in &kids {
            let (cft, _) = bpa.token_range(c);
            if cft == u32::MAX {
                all_positioned = false;
                break;
            }
            if cft > h.close {
                parts.body = Some(c);
            } else if cft < s1 {
                parts.init.push(c);
            } else if cft < s2 {
                if parts.cond.is_none() {
                    parts.cond = Some(c);
                } else {
                    // A condition is a single expression; extra nodes cannot occur in a
                    // well-formed header and are evaluated with the update part.
                    parts.update.push(c);
                }
            } else {
                parts.update.push(c);
            }
        }
        if all_positioned {
            parts.exact = true;
            return parts;
        }
        parts = ForParts::default();
    }

    // Positional fallback: the body is the last child; header children are taken as
    // init, condition, update... in order.
    if let Some((&last, head)) = kids.split_last() {
        parts.body = Some(last);
        match head.len() {
            0 => {}
            1 => {
                if bpa.node_type(head[0]) == ASTNodeType::NN_LOCAL_VAR_DECL {
                    parts.init.push(head[0]);
                } else {
                    parts.cond = Some(head[0]);
                }
            }
            _ => {
                parts.init.push(head[0]);
                parts.cond = Some(head[1]);
                parts.update.extend_from_slice(&head[2..]);
            }
        }
    }
    parts.exact = false;
    parts
}

pub fn build_for(
    node: u32,
    state: &mut CFGBuilderState,
    bpa: &BPASTArtifact,
    sta: &SymbolTableArtifact,
) {
    let labels = state.take_labels();
    let parts = for_parts(node, state, bpa);
    if !parts.exact {
        state.diag(CfgDiagnosticKind::ForHeaderApproximated, node);
    }

    for &init in &parts.init {
        build_simple(init, state, bpa, sta);
    }

    build_pretest_loop(
        parts.cond,
        parts.body,
        &parts.update,
        labels,
        state,
        bpa,
        sta,
    );
}

pub fn build_enhanced_for(
    node: u32,
    state: &mut CFGBuilderState,
    bpa: &BPASTArtifact,
    sta: &SymbolTableArtifact,
) {
    let labels = state.take_labels();
    let parts = ForEachParts::of(bpa, state.src.as_ref(), node);
    if !parts.exact {
        state.diag(CfgDiagnosticKind::ForEachHeaderApproximated, node);
    }

    // The iterable is evaluated once, before the loop (iterator() / array load).
    if let Some(it) = parts.iterable {
        build_simple(it, state, bpa, sta);
    }

    // header: hasNext() / index < length
    let header = state.new_block();
    state.goto(header, CFGEdgeType::Uncond);
    let body_entry = state.new_block();
    let exit = state.new_block();
    state.add_edge(header, body_entry, CFGEdgeType::True);
    state.add_edge(header, exit, CFGEdgeType::False);

    state.push_frame(
        FrameKind::Loop {
            break_target: exit,
            continue_target: header,
            continue_edge: CFGEdgeType::LoopBack,
        },
        labels,
    );

    // Each iteration starts by assigning the next element to the loop variable; the
    // variable's identifier is the statement that defines it.
    state.current_block = body_entry;
    if let Some(v) = parts.var {
        state.add_stmt_no_implicit(v, bpa);
    }
    if let Some(b) = parts.body {
        dispatch_stmt(b, state, bpa, sta);
    }
    let tail = state.current_block;
    state.add_edge(tail, header, CFGEdgeType::LoopBack);

    state.pop_frame();
    state.current_block = exit;
}
