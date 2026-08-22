//! Facade Pattern Detector (§9.2.5).
//! Detects Facade classes aggregating multiple subsystems into a simplified interface.

use crate::ingestion::TokenCorpusArtifact;
use crate::symbol::SymbolTableArtifact;

pub fn is_facade(sym_id: u32, sta: &SymbolTableArtifact, tca: &TokenCorpusArtifact) -> (bool, u16) {
    let sym = match sta.symbol(sym_id) {
        Some(s) => s,
        None => return (false, 0),
    };

    let name = std::str::from_utf8(tca.interner.lookup_text(sym.name_id)).unwrap_or("");
    let is_name_match = name.ends_with("Facade") || name.contains("Facade");

    // Count aggregated subsystem fields
    let mut subsystem_field_count = 0;
    let mut child_id = sym.first_child;
    while child_id != u32::MAX && (child_id as usize) < sta.symbol_records.len() {
        let child = &sta.symbol_records[child_id as usize];
        if child.kind == crate::core::types::symbol::SymbolKind::SK_FIELD as u8 {
            if child.type_id != u32::MAX && child.type_id != sym_id {
                subsystem_field_count += 1;
            }
        }
        child_id = child.next_sibling;
    }

    if is_name_match && subsystem_field_count >= 2 {
        (true, 95)
    } else if is_name_match {
        (true, 85)
    } else if subsystem_field_count >= 3 {
        (true, 70)
    } else {
        (false, 0)
    }
}
