//! UMLLink Registry & Pre-computation (§7.2.4, §7.5.3, Invariant 2).

pub mod hash_chain;

pub use crate::tra::types::ScpgHashChain;

use crate::core::types::symbol::SymbolKind;
use crate::core::types::token::unpack_sort_key;
use crate::ingestion::TokenCorpusArtifact;
use crate::symbol::SymbolTableArtifact;
use crate::tra::types::{BISymEntry, UMLLinkRecord};

pub struct UMLLinkRegistry;

impl UMLLinkRegistry {
    pub fn build(
        bi_sym: &[BISymEntry],
        tca: &TokenCorpusArtifact,
        sta: &SymbolTableArtifact,
        scpg_hash: u32,
    ) -> Vec<UMLLinkRecord> {
        let uml_kinds: &[u8] = &[
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

        let mut records = Vec::new();

        for (sym_id, sym) in sta.symbol_records.iter().enumerate() {
            if uml_kinds.contains(&sym.kind) {
                let entry = &bi_sym[sym_id];
                let (file_id, line_start, col_start, line_end, col_end) = if entry.decl_first_tok
                    != u32::MAX
                    && (entry.decl_first_tok as usize) < tca.token_records.len()
                {
                    let start_tok = &tca.token_records[entry.decl_first_tok as usize];
                    let end_tok = &tca.token_records
                        [entry.decl_last_tok.min(tca.token_records.len() as u32 - 1) as usize];
                    let (fid, ls, cs) = unpack_sort_key(start_tok.sort_key);
                    let (_, le, ce) = unpack_sort_key(end_tok.sort_key);
                    (fid, ls, cs, le, ce + end_tok.len as u16)
                } else {
                    (0, 0, 0, 0, 0)
                };

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
        }

        // Sort by sym_id for O(1) array index access
        records.sort_unstable_by_key(|r| r.sym_id);
        records
    }
}
