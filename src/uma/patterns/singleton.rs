//! Singleton pattern query (§9.2.5).

use crate::core::types::symbol::SymbolKind;
use crate::ingestion::TokenCorpusArtifact;
use crate::symbol::SymbolTableArtifact;
use crate::uma::patterns::PatternInspector;

pub fn is_singleton(
    class_sym: u32,
    sta: &SymbolTableArtifact,
    tca: &TokenCorpusArtifact,
) -> (bool, u16) {
    match sta.symbol(class_sym) {
        Some(s) if s.kind == SymbolKind::SK_CLASS as u8 => {}
        _ => return (false, 0),
    };

    let class_name = PatternInspector::get_name(sta, tca, class_sym);

    let has_singleton_ref = PatternInspector::get_fields(sta, class_sym)
        .iter()
        .any(|f| {
            let f_name = PatternInspector::get_name(sta, tca, f.symbol_id);
            f_name.eq_ignore_ascii_case("instance")
                || f_name.eq_ignore_ascii_case("getinstance")
                || f.type_id == class_sym
                || (!class_name.is_empty() && f_name == class_name)
        });

    if has_singleton_ref {
        (true, 100)
    } else {
        (false, 0)
    }
}
