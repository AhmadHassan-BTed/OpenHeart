//! Decorator Pattern Detector (§9.2.5).
//! A Decorator class implements or extends a Component interface/class while wrapping a Component field.

use crate::ingestion::TokenCorpusArtifact;
use crate::symbol::SymbolTableArtifact;
use crate::uma::patterns::PatternInspector;

pub fn is_decorator(
    sym_id: u32,
    sta: &SymbolTableArtifact,
    tca: &TokenCorpusArtifact,
) -> (bool, u16) {
    let is_name_match = PatternInspector::name_matches(sta, tca, sym_id, &["Decorator"]);
    let parent_types: Vec<u32> = sta
        .th_edges
        .iter()
        .filter(|e| e.from_sym == sym_id)
        .map(|e| e.to_sym)
        .collect();

    let has_wrapped_parent_field = PatternInspector::get_fields(sta, sym_id)
        .iter()
        .any(|f| parent_types.contains(&f.type_id));

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
