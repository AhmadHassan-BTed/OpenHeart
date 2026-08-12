//! Singleton pattern query (§9.2.5).

use crate::core::types::symbol::SymbolKind;
use crate::symbol::SymbolTableArtifact;

pub fn is_singleton(class_sym: u32, sta: &SymbolTableArtifact) -> (bool, u16) {
    let sym = match sta.symbol(class_sym) {
        Some(s) => s,
        None => return (false, 0),
    };
    if SymbolKind::from(sym.kind) != SymbolKind::SK_CLASS {
        return (false, 0);
    }

    let mut has_ctor = false;
    let mut has_field = false;
    let mut has_factory = false;

    let mut child_id = sym.first_child;
    while child_id != u32::MAX && (child_id as usize) < sta.symbol_records.len() {
        let child = &sta.symbol_records[child_id as usize];
        let child_kind = SymbolKind::from(child.kind);

        if child_kind == SymbolKind::SK_CONSTRUCTOR {
            has_ctor = true;
        } else if child_kind == SymbolKind::SK_FIELD {
            has_field = true;
        } else if child_kind == SymbolKind::SK_METHOD {
            has_factory = true;
        }

        child_id = child.next_sibling;
    }

    if has_ctor && (has_field || has_factory) {
        let confidence = if has_ctor && has_field && has_factory { 100 } else { 80 };
        (true, confidence)
    } else {
        (false, 0)
    }
}
