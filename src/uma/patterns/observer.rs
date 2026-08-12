//! Observer/Listener pattern query (§9.2.5).

use crate::core::types::cg::CallGraphArtifact;
use crate::ingestion::TokenCorpusArtifact;
use crate::symbol::SymbolTableArtifact;

pub fn is_observer_subject(
    class_sym: u32,
    sta: &SymbolTableArtifact,
    tca: &TokenCorpusArtifact,
    _cga: &CallGraphArtifact,
) -> (bool, u16) {
    let sym = match sta.symbol(class_sym) {
        Some(s) => s,
        None => return (false, 0),
    };
    let mut has_add_listener = false;

    let mut child_id = sym.first_child;
    while child_id != u32::MAX && (child_id as usize) < sta.symbol_records.len() {
        let child = &sta.symbol_records[child_id as usize];
        if child.kind == 6 { // SK_METHOD
            let bytes = tca.interner.lookup_text(child.name_id);
            let name = std::str::from_utf8(bytes).unwrap_or("");
            if (name.starts_with("add") || name.starts_with("register") || name.starts_with("attach"))
                && name.to_lowercase().contains("listener")
            {
                has_add_listener = true;
            }
        }
        child_id = child.next_sibling;
    }

    if has_add_listener {
        (true, 85)
    } else {
        (false, 0)
    }
}
