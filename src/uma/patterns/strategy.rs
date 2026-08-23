//! Strategy Pattern Detector (§9.2.5).
//! Detects strategy interfaces/abstract algorithms and their concrete strategy implementors.

use crate::ingestion::TokenCorpusArtifact;
use crate::symbol::SymbolTableArtifact;
use crate::uma::patterns::PatternInspector;

pub fn is_strategy(
    sym_id: u32,
    sta: &SymbolTableArtifact,
    tca: &TokenCorpusArtifact,
) -> (bool, u16) {
    let is_name_match = PatternInspector::name_matches(sta, tca, sym_id, &["Strategy"]);

    let implements_strategy = sta.th_edges.iter().any(|edge| {
        edge.from_sym == sym_id
            && PatternInspector::name_matches(sta, tca, edge.to_sym, &["Strategy"])
    });

    if is_name_match {
        (true, 95)
    } else if implements_strategy {
        (true, 90)
    } else {
        (false, 0)
    }
}
