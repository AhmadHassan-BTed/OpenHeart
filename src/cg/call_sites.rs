//! Call Site Extraction & Classification for Phase 6.
//! Authored solely by Ahmad Hassan (B-Ted).

use crate::ast::BPASTArtifact;
use crate::cfg::serializer::CFGArtifact;
use crate::cg::resolution::resolve_method_target;
use crate::core::logger::log_trace;
use crate::core::types::symbol::*;
use crate::core::types::ASTNodeType::*;
use crate::core::types::*;
use crate::ssa::SSAArtifact;
use crate::symbol::SymbolTableArtifact;

pub const MOD_STATIC: u16 = 1 << 3;
pub const VIS_PRIVATE: u8 = SymbolVisibility::Private as u8;

/// Extract all call sites from the BP AST (§6.5.1)
pub fn extract_call_sites(
    bpa: &BPASTArtifact,
    sta: &SymbolTableArtifact,
    cfa: &CFGArtifact,
    ssa: &SSAArtifact,
) -> Vec<CallSite> {
    let mut sites = Vec::new();
    let mut id_counter = 0u32;

    for pre_idx in 0..bpa.node_count {
        let ntype = bpa.node_type(pre_idx);
        if !matches!(
            ntype,
            NN_CALL_EXPR | NN_NEW_EXPR | NN_METHOD_REF | NN_LAMBDA_EXPR
        ) {
            continue;
        }

        let caller_sym = find_enclosing_method(pre_idx, bpa, sta);
        if caller_sym == u32::MAX {
            continue;
        }

        let call_type = classify_call(pre_idx, ntype, bpa, sta);
        let receiver_ssa = extract_receiver_ssa(pre_idx, bpa, ssa);
        let call_block = find_block_for_stmt(pre_idx, caller_sym, cfa);
        let call_token = find_method_name_token(pre_idx, bpa);
        let arg_count = count_arguments(pre_idx, bpa) as u16;

        let flags = 0u8;

        let site = CallSite::new(
            id_counter,
            caller_sym,
            pre_idx,
            receiver_ssa,
            call_block,
            call_token,
            call_type,
            flags,
            arg_count,
        );

        log_trace(&format!(
            "Extracted CallSite #{}: caller_sym={} call_type={:#04x} node={}",
            id_counter, caller_sym, call_type, pre_idx
        ));

        sites.push(site);
        id_counter += 1;
    }

    sites.sort_unstable_by_key(|s| s.caller_sym);
    sites
}

fn find_enclosing_method(node: u32, bpa: &BPASTArtifact, sta: &SymbolTableArtifact) -> u32 {
    let mut curr = node;
    loop {
        for sym in &sta.symbol_records {
            if (sym.kind == SymbolKind::SK_METHOD as u8
                || sym.kind == SymbolKind::SK_CONSTRUCTOR as u8
                || sym.kind == SymbolKind::SK_STATIC_INIT as u8
                || sym.kind == SymbolKind::SK_LAMBDA as u8)
                && (sym.def_node == curr || sym.decl_node == curr)
            {
                return sym.symbol_id;
            }
        }
        let p = bpa.parent(curr);
        if p != u32::MAX && p != curr {
            curr = p;
        } else {
            break;
        }
    }
    u32::MAX
}

fn classify_call(
    call_node: u32,
    ntype: ASTNodeType,
    bpa: &BPASTArtifact,
    sta: &SymbolTableArtifact,
) -> u8 {
    match ntype {
        NN_NEW_EXPR => CG_EDGE_CONSTRUCTOR,
        NN_METHOD_REF | NN_LAMBDA_EXPR => CG_EDGE_DYNAMIC,
        NN_CALL_EXPR => {
            let receiver = bpa.first_child(call_node);
            if receiver.is_none() {
                return CG_EDGE_SPECIAL;
            }
            if let Some(target_sym) = resolve_method_target(call_node, bpa, sta) {
                if let Some(sym) = sta.symbol(target_sym) {
                    if (sym.modifiers & MOD_STATIC) != 0 {
                        CG_EDGE_DIRECT
                    } else if sym.visibility == VIS_PRIVATE {
                        CG_EDGE_SPECIAL
                    } else if sym.kind == SymbolKind::SK_INTERFACE as u8 {
                        CG_EDGE_INTERFACE
                    } else {
                        CG_EDGE_VIRTUAL
                    }
                } else {
                    CG_EDGE_VIRTUAL
                }
            } else {
                CG_EDGE_VIRTUAL
            }
        }
        _ => CG_EDGE_DIRECT,
    }
}

fn extract_receiver_ssa(call_node: u32, bpa: &BPASTArtifact, ssa: &SSAArtifact) -> u32 {
    if let Some(rcv_node) = bpa.first_child(call_node) {
        for func in &ssa.functions {
            for ssa_rec in &func.ssa_records {
                if ssa_rec.def_stmt == rcv_node {
                    return ssa_rec.ssa_id;
                }
            }
        }
    }
    u32::MAX
}

fn find_block_for_stmt(stmt_node: u32, caller_sym: u32, cfa: &CFGArtifact) -> u32 {
    if let Some(fn_cfg) = cfa.functions.iter().find(|f| f.sym_id == caller_sym) {
        for b in &fn_cfg.blocks {
            if b.stmts.contains(&stmt_node) {
                return b.id;
            }
        }
    }
    0
}

fn find_method_name_token(call_node: u32, bpa: &BPASTArtifact) -> u32 {
    bpa.token_range(call_node).0
}

fn count_arguments(call_node: u32, bpa: &BPASTArtifact) -> usize {
    let mut count: usize = 0;
    let mut child = bpa.first_child(call_node);
    while let Some(c) = child {
        count += 1;
        child = bpa.next_sibling(c);
    }
    count.saturating_sub(1)
}
