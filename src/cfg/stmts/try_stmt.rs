//! Try-Catch-Finally CFG builder algorithm (§4.5.6).

use crate::ast::BPASTArtifact;
use crate::cfg::builder::state::CFGBuilderState;
use crate::cfg::stmts::dispatch_stmt;
use crate::core::types::ast::ASTNodeType;
use crate::core::types::cfg::ExceptionFrame;
use crate::symbol::SymbolTableArtifact;

pub fn build_try(
    node: u32,
    state: &mut CFGBuilderState,
    bpa: &BPASTArtifact,
    sta: &SymbolTableArtifact,
) {
    let try_body = match bpa.first_child(node) {
        Some(b) => b,
        None => return,
    };

    let mut catches = Vec::new();
    let mut finally = None;

    let mut cur = bpa.next_sibling(try_body);
    while let Some(c) = cur {
        let ntype = bpa.node_type(c);
        if ntype == ASTNodeType::NN_CATCH_CLAUSE {
            catches.push(c);
        } else if ntype == ASTNodeType::NN_FINALLY_CLAUSE {
            finally = Some(c);
        }
        cur = bpa.next_sibling(c);
    }

    let mut handler_map = Vec::new();
    for &catch in &catches {
        let catch_block = state.new_block();
        let ex_type_id = resolve_catch_type(catch, bpa, sta);
        handler_map.push((ex_type_id, catch_block));
    }

    let finally_block = finally.map(|_| state.new_block());
    let post_try = state.new_block();

    state.push_exception(ExceptionFrame {
        handlers: handler_map.clone(),
        finally_block,
    });

    let try_entry = state.new_block();
    state.flush_pending(try_entry);
    state.current_block = try_entry;
    dispatch_stmt(try_body, state, bpa, sta);
    state.pop_exception();

    let target_after_try = finally_block.unwrap_or(post_try);
    state.flush_pending(target_after_try);

    for (i, &catch_node) in catches.iter().enumerate() {
        let (_, catch_block_id) = handler_map[i];
        state.current_block = catch_block_id;

        if let Some(param) = bpa.first_child(catch_node) {
            state.add_stmt_to_current(param, bpa);
            if let Some(body) = bpa.next_sibling(param) {
                dispatch_stmt(body, state, bpa, sta);
            }
        }
        state.flush_pending(target_after_try);
    }

    if let Some(fin_node) = finally {
        let fin_block = finally_block.unwrap();
        state.current_block = fin_block;
        if let Some(fin_body) = bpa.first_child(fin_node) {
            dispatch_stmt(fin_body, state, bpa, sta);
        }
        state.flush_pending(post_try);
    }

    state.current_block = post_try;
}

fn resolve_catch_type(catch_node: u32, bpa: &BPASTArtifact, sta: &SymbolTableArtifact) -> u32 {
    if let Some(param) = bpa.first_child(catch_node) {
        if let Some(type_ref) = bpa.first_child(param) {
            if bpa.node_type(type_ref) == ASTNodeType::NN_TYPE_REF {
                for sym in &sta.symbol_records {
                    if sym.decl_node == type_ref || sym.def_node == type_ref {
                        return sym.symbol_id;
                    }
                }
                return type_ref;
            }
        }
    }
    u32::MAX
}
