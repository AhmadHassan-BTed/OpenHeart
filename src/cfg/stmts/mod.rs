pub mod assert_stmt;
pub mod break_stmt;
pub mod continue_stmt;
pub mod do_while;
pub mod expr;
pub mod for_stmt;
pub mod if_stmt;
pub mod labeled_stmt;
pub mod return_stmt;
pub mod switch_stmt;
pub mod synchronized_stmt;
pub mod throw_stmt;
pub mod try_stmt;
pub mod while_stmt;
pub mod yield_stmt;

use crate::ast::BPASTArtifact;
use crate::cfg::ast_query::is_type_decl;
use crate::cfg::builder::state::CFGBuilderState;
use crate::core::types::ast::ASTNodeType;
use crate::symbol::SymbolTableArtifact;

pub use assert_stmt::build_assert;
pub use break_stmt::build_break;
pub use continue_stmt::build_continue;
pub use do_while::build_do_while;
pub use expr::{build_branch, build_simple, lower_embedded, lower_value};
pub use for_stmt::{build_enhanced_for, build_for};
pub use if_stmt::build_if;
pub use labeled_stmt::build_labeled;
pub use return_stmt::build_return;
pub use switch_stmt::{build_switch, build_switch_value};
pub use synchronized_stmt::build_synchronized;
pub use throw_stmt::build_throw;
pub use try_stmt::build_try;
pub use while_stmt::build_while;
pub use yield_stmt::build_yield;

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
        // tree-sitter-java emits statement switches as `switch_expression`; in statement
        // position both kinds have statement semantics (`break` allowed, no value).
        ASTNodeType::NN_SWITCH_STMT | ASTNodeType::NN_SWITCH_EXPR => {
            build_switch(node, state, bpa, sta)
        }
        ASTNodeType::NN_TRY_STMT => build_try(node, state, bpa, sta),
        ASTNodeType::NN_RETURN_STMT => build_return(node, state, bpa, sta),
        ASTNodeType::NN_THROW_STMT => build_throw(node, state, bpa, sta),
        ASTNodeType::NN_BREAK_STMT => build_break(node, state, bpa, sta),
        ASTNodeType::NN_CONTINUE_STMT => build_continue(node, state, bpa, sta),
        ASTNodeType::NN_YIELD_STMT => build_yield(node, state, bpa, sta),
        ASTNodeType::NN_JAVA_LABELED_STMT => build_labeled(node, state, bpa, sta),
        ASTNodeType::NN_JAVA_SYNCHRONIZED => build_synchronized(node, state, bpa, sta),
        ASTNodeType::NN_JAVA_ASSERT_STMT => build_assert(node, state, bpa, sta),
        ASTNodeType::NN_BLOCK => {
            let mut child = bpa.first_child(node);
            while let Some(c) = child {
                dispatch_stmt(c, state, bpa, sta);
                child = bpa.next_sibling(c);
            }
        }
        // Local class / record / enum / interface declarations: their members are
        // separate functions with their own CFGs.
        t if is_type_decl(t) => {}
        _ => build_simple(node, state, bpa, sta),
    }
}
