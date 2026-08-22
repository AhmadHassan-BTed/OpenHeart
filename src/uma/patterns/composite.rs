//! Composite Pattern Detector (§9.2.5).
//! Detects Composite classes that implement a component interface and contain a collection of components.

use crate::ingestion::TokenCorpusArtifact;
use crate::symbol::SymbolTableArtifact;

pub fn is_composite(
    sym_id: u32,
    sta: &SymbolTableArtifact,
    tca: &TokenCorpusArtifact,
) -> (bool, u16) {
    let sym = match sta.symbol(sym_id) {
        Some(s) => s,
        None => return (false, 0),
    };

    let name = std::str::from_utf8(tca.interner.lookup_text(sym.name_id)).unwrap_or("");
    let is_name_match = name.ends_with("Composite") || name.contains("Composite");

    let mut parent_types = Vec::new();
    for edge in &sta.th_edges {
        if edge.from_sym == sym_id {
            parent_types.push(edge.to_sym);
        }
    }

    let mut has_child_collection = false;
    let mut child_id = sym.first_child;
    while child_id != u32::MAX && (child_id as usize) < sta.symbol_records.len() {
        let child = &sta.symbol_records[child_id as usize];
        if child.kind == crate::core::types::symbol::SymbolKind::SK_FIELD as u8 {
            if parent_types.contains(&child.type_id) {
                has_child_collection = true;
                break;
            }
        }
        child_id = child.next_sibling;
    }

    if is_name_match && has_child_collection {
        (true, 95)
    } else if is_name_match {
        (true, 80)
    } else if has_child_collection {
        (true, 75)
    } else {
        (false, 0)
    }
}
