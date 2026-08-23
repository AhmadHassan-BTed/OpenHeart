//! Composite Pattern Detector (§9.2.5).
//! Detects Composite classes that implement a component interface and contain a collection of components.

use crate::ingestion::TokenCorpusArtifact;
use crate::symbol::SymbolTableArtifact;
use crate::uma::patterns::PatternInspector;

pub fn is_composite(
    sym_id: u32,
    sta: &SymbolTableArtifact,
    tca: &TokenCorpusArtifact,
) -> (bool, u16) {
    let is_name_match = PatternInspector::name_matches(sta, tca, sym_id, &["Composite"]);

    let parent_types: Vec<u32> = sta
        .th_edges
        .iter()
        .filter(|e| e.from_sym == sym_id)
        .map(|e| e.to_sym)
        .collect();

    let has_child_collection = PatternInspector::get_fields(sta, sym_id)
        .iter()
        .any(|f| parent_types.contains(&f.type_id));

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
