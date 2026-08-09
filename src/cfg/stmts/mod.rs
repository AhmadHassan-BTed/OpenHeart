pub mod break_stmt;
pub mod continue_stmt;
pub mod do_while;
pub mod for_stmt;
pub mod if_stmt;
pub mod return_stmt;
pub mod switch_stmt;
pub mod throw_stmt;
pub mod try_stmt;
pub mod while_stmt;

use crate::ast::BPASTArtifact;
use crate::cfg::builder::state::CFGBuilderState;
use crate::core::types::ast::ASTNodeType;
use crate::symbol::SymbolTableArtifact;

pub use break_stmt::build_break;
pub use continue_stmt::build_continue;
pub use do_while::build_do_while;
pub use for_stmt::{build_enhanced_for, build_for};
pub use if_stmt::build_if;
pub use return_stmt::build_return;
pub use switch_stmt::build_switch;
pub use throw_stmt::build_throw;
pub use try_stmt::build_try;
pub use while_stmt::build_while;

pub fn dispatch_stmt(
    node: u32,
    state: &mut CFGBuilderState,
    bpa: &BPASTArtifact,
    sta: &SymbolTableArtifact,
) {
    let ntype = bpa.node_type(node);
    match ntype {
        ASTNodeType::NN_IF_STMT => build_if(node, state, bpa, sta),
        ASTNodeType::NN_WHILE_STMT => build_while(node, state, bpa, sta),
        ASTNodeType::NN_FOR_STMT => build_for(node, state, bpa, sta),
        ASTNodeType::NN_ENHANCED_FOR => build_enhanced_for(node, state, bpa, sta),
        ASTNodeType::NN_DO_WHILE_STMT => build_do_while(node, state, bpa, sta),
        ASTNodeType::NN_SWITCH_STMT => build_switch(node, state, bpa, sta),
        ASTNodeType::NN_TRY_STMT => build_try(node, state, bpa, sta),
        ASTNodeType::NN_RETURN_STMT => build_return(node, state, bpa, sta),
        ASTNodeType::NN_THROW_STMT => build_throw(node, state, bpa, sta),
        ASTNodeType::NN_BREAK_STMT => build_break(node, state, bpa, sta),
        ASTNodeType::NN_CONTINUE_STMT => build_continue(node, state, bpa, sta),
        ASTNodeType::NN_JAVA_LABELED_STMT => {
            let label_node = bpa.first_child(node);
            let inner_stmt = label_node.and_then(|l| bpa.next_sibling(l));
            if let Some(inner) = inner_stmt {
                dispatch_stmt(inner, state, bpa, sta);
            }
        }
        ASTNodeType::NN_BLOCK => {
            let mut child = bpa.first_child(node);
            while let Some(c) = child {
                dispatch_stmt(c, state, bpa, sta);
                child = bpa.next_sibling(c);
            }
        }
        _ => {
            state.add_stmt_to_current(node, bpa);
        }
    }
}
