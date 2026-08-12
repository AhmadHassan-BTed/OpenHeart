//! Symbol Backward Index (§7.3, §7.4).

use crate::ast::BPASTArtifact;
use crate::symbol::SymbolTableArtifact;
use crate::tra::types::BISymEntry;

pub struct SymbolBackwardIndex;

impl SymbolBackwardIndex {
    pub fn build(sta: &SymbolTableArtifact, bpa: &BPASTArtifact) -> Vec<BISymEntry> {
        let count = sta.symbol_records.len();
        let mut bi_sym = Vec::with_capacity(count);

        for sym in &sta.symbol_records {
            let (decl_ft, decl_lt) = if sym.decl_node != u32::MAX
                && (sym.decl_node as usize) < bpa.node_count as usize
            {
                bpa.token_range(sym.decl_node)
            } else {
                (sym.first_token_id, sym.last_token_id)
            };

            let (def_ft, def_lt) =
                if sym.def_node != u32::MAX && (sym.def_node as usize) < bpa.node_count as usize {
                    bpa.token_range(sym.def_node)
                } else {
                    (u32::MAX, u32::MAX)
                };

            bi_sym.push(BISymEntry {
                decl_first_tok: decl_ft,
                decl_last_tok: decl_lt,
                def_first_tok: def_ft,
                def_last_tok: def_lt,
            });
        }

        bi_sym
    }
}
