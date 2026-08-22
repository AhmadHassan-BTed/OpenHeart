//! Adapter Pattern Detector (§9.2.5).
//! Detects Adapter classes implementing a target interface and wrapping an adaptee reference.

use crate::ingestion::TokenCorpusArtifact;
use crate::symbol::SymbolTableArtifact;

pub fn is_adapter(
    sym_id: u32,
    sta: &SymbolTableArtifact,
    tca: &TokenCorpusArtifact,
) -> (bool, u16) {
    let sym = match sta.symbol(sym_id) {
        Some(s) => s,
        None => return (false, 0),
    };

    let name = std::str::from_utf8(tca.interner.lookup_text(sym.name_id)).unwrap_or("");
    let is_name_match = name.ends_with("Adapter") || name.contains("Adapter");

    let mut implements_any = false;
    for edge in &sta.th_edges {
        if edge.from_sym == sym_id {
            implements_any = true;
            break;
        }
    }

    let mut has_adaptee_field = false;
    let mut child_id = sym.first_child;
    while child_id != u32::MAX && (child_id as usize) < sta.symbol_records.len() {
        let child = &sta.symbol_records[child_id as usize];
        if child.kind == crate::core::types::symbol::SymbolKind::SK_FIELD as u8 {
            if child.type_id != u32::MAX && child.type_id != sym_id {
                has_adaptee_field = true;
                break;
            }
        }
        child_id = child.next_sibling;
    }

    if is_name_match && implements_any && has_adaptee_field {
        (true, 95)
    } else if is_name_match {
        (true, 80)
    } else {
        (false, 0)
    }
}
