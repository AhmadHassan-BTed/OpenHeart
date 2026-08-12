//! Builder (fluent interface) pattern query (§9.2.5).

use crate::ingestion::TokenCorpusArtifact;
use crate::symbol::SymbolTableArtifact;

pub fn is_builder(
    class_sym: u32,
    sta: &SymbolTableArtifact,
    tca: &TokenCorpusArtifact,
) -> (bool, u16) {
    let sym = match sta.symbol(class_sym) {
        Some(s) => s,
        None => return (false, 0),
    };
    let bytes = tca.interner.lookup_text(sym.name_id);
    let name = std::str::from_utf8(bytes).unwrap_or("");

    let mut total_methods = 0;
    let mut self_returning = 0;

    let mut child_id = sym.first_child;
    while child_id != u32::MAX && (child_id as usize) < sta.symbol_records.len() {
        let child = &sta.symbol_records[child_id as usize];
        if child.kind == 6 {
            // SK_METHOD
            total_methods += 1;
            if child.type_id == class_sym {
                self_returning += 1;
            }
        }
        child_id = child.next_sibling;
    }

    let is_name_builder = name.ends_with("Builder");
    let fluent_ratio = if total_methods > 0 {
        self_returning as f32 / total_methods as f32
    } else {
        0.0
    };

    if is_name_builder || fluent_ratio > 0.4 {
        let conf = if is_name_builder && fluent_ratio > 0.4 {
            95
        } else {
            75
        };
        (true, conf)
    } else {
        (false, 0)
    }
}
