//! Decorator Pattern Detector (§9.2.5).
//! A Decorator class implements or extends a Component interface/class while wrapping a Component field.

use crate::ingestion::TokenCorpusArtifact;
use crate::symbol::SymbolTableArtifact;

pub fn is_decorator(
    sym_id: u32,
    sta: &SymbolTableArtifact,
    tca: &TokenCorpusArtifact,
) -> (bool, u16) {
    let sym = match sta.symbol(sym_id) {
        Some(s) => s,
        None => return (false, 0),
    };

    let name = std::str::from_utf8(tca.interner.lookup_text(sym.name_id)).unwrap_or("");
    let is_name_match = name.ends_with("Decorator") || name.contains("Decorator");

    // Check if class implements an interface/superclass AND has a field of that same type
    let mut parent_types = Vec::new();
    for edge in &sta.th_edges {
        if edge.from_sym == sym_id {
            parent_types.push(edge.to_sym);
        }
    }

    let mut has_wrapped_parent_field = false;
    let mut child_id = sym.first_child;
    while child_id != u32::MAX && (child_id as usize) < sta.symbol_records.len() {
        let child = &sta.symbol_records[child_id as usize];
        if child.kind == crate::core::types::symbol::SymbolKind::SK_FIELD as u8 {
            if parent_types.contains(&child.type_id) {
                has_wrapped_parent_field = true;
                break;
            }
        }
        child_id = child.next_sibling;
    }

    if has_wrapped_parent_field && is_name_match {
        (true, 95)
    } else if has_wrapped_parent_field {
        (true, 85)
    } else if is_name_match && !parent_types.is_empty() {
        (true, 75)
    } else {
        (false, 0)
    }
}
