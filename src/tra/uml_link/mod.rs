//! UMLLink Registry & Pre-computation (§7.2.4, §7.5.3, Invariant 2).

pub mod digest;
pub mod hash_chain;

pub use crate::tra::types::ScpgHashChain;

use crate::core::types::symbol::SymbolKind;
use crate::ingestion::TokenCorpusArtifact;
use crate::symbol::SymbolTableArtifact;
use crate::tra::position::{clamp_span, column_u16, token_end, token_start};
use crate::tra::types::{
    BISymEntry, UMLLinkRecord, NO_SLOT, UML_LINK_NO_COL, UML_LINK_NO_FILE, UML_LINK_NO_LINE,
};

/// Symbol kinds that appear as UML elements and therefore get a UMLLink — the single
/// definition used by the registry and by Invariant 2.
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
        let mut records = Vec::new();

        for (sym_id, sym) in sta.symbol_records.iter().enumerate() {
            if !UML_SYMBOL_KINDS.contains(&sym.kind) {
                continue;
            }
            // The declaration span, clamped to the declaring file; its end is the exact end
            // of the last token (multi-line tokens end on a later line). Columns beyond the
            // 16-bit field saturate at `u16::MAX`.
            let located = bi_sym.get(sym_id).and_then(|entry| {
                let (first, last) = clamp_span(tca, entry.decl_first_tok, entry.decl_last_tok)?;
                let (fid, ls, cs) = token_start(tca, first);
                let (le, ce) = token_end(tca, last);
                Some((fid, ls, cs, le, column_u16(ce)))
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
