//! Labeled Statement CFG builder algorithm (JLS 14.7).
//!
//! The label is attached to the frame of the statement it labels: loops and switches
//! claim pending labels when they push their frame, so `break l` / `continue l` resolve
//! to them. Any other statement gets a `Labeled` frame whose break target is the block
//! after the statement (`l: { ... break l; ... }`).

use crate::ast::BPASTArtifact;
use crate::cfg::ast_query::children;
use crate::cfg::builder::state::{CFGBuilderState, FrameKind};
use crate::cfg::stmts::dispatch_stmt;
use crate::core::types::ast::ASTNodeType;
use crate::core::types::cfg::CFGEdgeType;
use crate::symbol::SymbolTableArtifact;

pub fn build_labeled(
    node: u32,
    state: &mut CFGBuilderState,
    bpa: &BPASTArtifact,
    sta: &SymbolTableArtifact,
) {
    let kids = children(bpa, node);
    let label_node = match kids.first() {
        Some(&l) if bpa.node_type(l) == ASTNodeType::NN_IDENTIFIER_EXPR => l,
        _ => {
            // Unexpected shape: build the children as plain statements.
            for c in kids {
                dispatch_stmt(c, state, bpa, sta);
            }
            return;
        }
    };
    let inner = match kids.get(1) {
        Some(&s) => s,
        // `l: ;` — the empty statement was dropped by Phase 2.
        None => return,
    };

    let label = state.label_ref(bpa, label_node);
    state.pending_labels.push(label);

    let claims_labels = matches!(
        bpa.node_type(inner),
        ASTNodeType::NN_WHILE_STMT
            | ASTNodeType::NN_FOR_STMT
            | ASTNodeType::NN_ENHANCED_FOR
            | ASTNodeType::NN_DO_WHILE_STMT
            | ASTNodeType::NN_SWITCH_STMT
            | ASTNodeType::NN_SWITCH_EXPR
            | ASTNodeType::NN_JAVA_LABELED_STMT
    );
    if claims_labels {
        dispatch_stmt(inner, state, bpa, sta);
        return;
    }

    let labels = state.take_labels();
    let after = state.new_block();
    state.push_frame(
        FrameKind::Labeled {
            break_target: after,
        },
        labels,
    );
    dispatch_stmt(inner, state, bpa, sta);
    state.pop_frame();
    state.goto(after, CFGEdgeType::Uncond);
}
