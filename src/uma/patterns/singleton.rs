//! Singleton pattern query (§9.2.5).

use crate::core::types::symbol::SymbolKind;
use crate::ingestion::TokenCorpusArtifact;
use crate::symbol::SymbolTableArtifact;

pub fn is_singleton(
    class_sym: u32,
    sta: &SymbolTableArtifact,
    tca: &TokenCorpusArtifact,
) -> (bool, u16) {
    let sym = match sta.symbol(class_sym) {
        Some(s) => s,
        None => return (false, 0),
    };
    if SymbolKind::from(sym.kind) != SymbolKind::SK_CLASS {
        return (false, 0);
    }
    let class_name_bytes = tca.interner.lookup_text(sym.name_id);
    let class_name = std::str::from_utf8(class_name_bytes).unwrap_or("");

    let mut has_singleton_ref = false;

    let mut child_id = sym.first_child;
    while child_id != u32::MAX && (child_id as usize) < sta.symbol_records.len() {
        let child = &sta.symbol_records[child_id as usize];
        let child_name_bytes = tca.interner.lookup_text(child.name_id);
        let child_name = std::str::from_utf8(child_name_bytes).unwrap_or("");

        if child_name.to_lowercase() == "instance"
            || child_name.to_lowercase() == "getinstance"
            || (!class_name.is_empty() && child_name == class_name)
        {
            has_singleton_ref = true;
            break;
        }

        child_id = child.next_sibling;
    }

    if has_singleton_ref {
        (true, 100)
    } else {
        (false, 0)
    }
}
