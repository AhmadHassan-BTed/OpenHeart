//! Adapter Pattern Detector (§9.2.5).
//! Detects Adapter classes implementing a target interface and wrapping an adaptee reference.

use crate::ingestion::TokenCorpusArtifact;
use crate::symbol::SymbolTableArtifact;
use crate::uma::patterns::PatternInspector;

pub fn is_adapter(
    sym_id: u32,
    sta: &SymbolTableArtifact,
    tca: &TokenCorpusArtifact,
) -> (bool, u16) {
    let is_name_match = PatternInspector::name_matches(sta, tca, sym_id, &["Adapter"]);
    let implements_any = PatternInspector::has_type_hierarchy_edge(sta, sym_id);
    let has_adaptee_field = PatternInspector::get_fields(sta, sym_id)
        .iter()
        .any(|f| f.type_id != u32::MAX && f.type_id != sym_id);

    if is_name_match && implements_any && has_adaptee_field {
        (true, 95)
    } else if is_name_match {
        (true, 80)
    } else {
        (false, 0)
    }
}
