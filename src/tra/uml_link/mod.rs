//! UMLLink Registry & Pre-computation (§7.2.4, §7.5.3, Invariant 2).

pub mod digest;
pub mod hash_chain;

pub use crate::tra::types::ScpgHashChain;

use crate::core::types::symbol::SymbolKind;
use crate::core::types::token::unpack_sort_key;
use crate::ingestion::TokenCorpusArtifact;
use crate::symbol::SymbolTableArtifact;
use crate::tra::types::{
    BISymEntry, UMLLinkRecord, NO_SLOT, UML_LINK_NO_COL, UML_LINK_NO_FILE, UML_LINK_NO_LINE,
};

/// Symbol kinds that appear as UML elements and therefore get a UMLLink.
pub const UML_SYMBOL_KINDS: [u8; 9] = [
    SymbolKind::SK_CLASS as u8,
    SymbolKind::SK_INTERFACE as u8,
    SymbolKind::SK_ENUM as u8,
    SymbolKind::SK_RECORD as u8,
    SymbolKind::SK_ANNOTATION_TYPE as u8,
    SymbolKind::SK_METHOD as u8,
    SymbolKind::SK_CONSTRUCTOR as u8,
    SymbolKind::SK_FIELD as u8,
    SymbolKind::SK_ENUM_CONSTANT as u8,
];

pub struct UMLLinkRegistry;

impl UMLLinkRegistry {
    pub fn build(
        bi_sym: &[BISymEntry],
        tca: &TokenCorpusArtifact,
        sta: &SymbolTableArtifact,
        scpg_hash: u32,
    ) -> Vec<UMLLinkRecord> {
        let n_tokens = tca.token_records.len();
        let mut records = Vec::new();

        for (sym_id, sym) in sta.symbol_records.iter().enumerate() {
            if !UML_SYMBOL_KINDS.contains(&sym.kind) {
                continue;
            }
            let located = bi_sym.get(sym_id).and_then(|entry| {
                if entry.decl_first_tok == u32::MAX || (entry.decl_first_tok as usize) >= n_tokens {
                    return None;
                }
                let last = entry.decl_last_tok.min(n_tokens as u32 - 1).max(entry.decl_first_tok);
                let start_tok = &tca.token_records[entry.decl_first_tok as usize];
                let end_tok = &tca.token_records[last as usize];
                let (fid, ls, cs) = unpack_sort_key(start_tok.sort_key);
                let (_, le, ce) = unpack_sort_key(end_tok.sort_key);
                Some((fid, ls, cs, le, ce.saturating_add(end_tok.len)))
            });
            // No source location: explicit sentinels, never (file 0, line 0).
            let (file_id, line_start, col_start, line_end, col_end) = located.unwrap_or((
                UML_LINK_NO_FILE,
                UML_LINK_NO_LINE,
                UML_LINK_NO_COL,
                UML_LINK_NO_LINE,
                UML_LINK_NO_COL,
            ));

            records.push(UMLLinkRecord {
                sym_id: sym_id as u32,
                file_id,
                line_start,
                col_start,
                line_end,
                col_end,
                scpg_hash,
                sym_kind: sym.kind,
                _reserved: [0; 3],
            });
        }

        // Sorted by sym_id; `slots` gives the O(1) sym_id → record index map.
        records.sort_unstable_by_key(|r| r.sym_id);
        records
    }

    /// Dense `sym_id → index into links` table ([`NO_SLOT`] for symbols without a link).
    pub fn slots(links: &[UMLLinkRecord], symbol_count: usize) -> Vec<u32> {
        let mut slots = vec![NO_SLOT; symbol_count];
        for (idx, link) in links.iter().enumerate() {
            if let Some(slot) = slots.get_mut(link.sym_id as usize) {
                *slot = idx as u32;
            }
        }
        slots
    }
}
