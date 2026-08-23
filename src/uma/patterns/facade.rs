//! Facade Pattern Detector (§9.2.5).
//! Detects Facade classes aggregating multiple subsystems into a simplified interface.

use crate::ingestion::TokenCorpusArtifact;
use crate::symbol::SymbolTableArtifact;
use crate::uma::patterns::PatternInspector;

pub fn is_facade(sym_id: u32, sta: &SymbolTableArtifact, tca: &TokenCorpusArtifact) -> (bool, u16) {
    let is_name_match = PatternInspector::name_matches(sta, tca, sym_id, &["Facade"]);

    let subsystem_field_count = PatternInspector::get_fields(sta, sym_id)
        .iter()
        .filter(|f| f.type_id != u32::MAX && f.type_id != sym_id)
        .count();

    if is_name_match && subsystem_field_count >= 2 {
        (true, 95)
    } else if is_name_match {
        (true, 85)
    } else if subsystem_field_count >= 3 {
        (true, 70)
    } else {
        (false, 0)
    }
}
