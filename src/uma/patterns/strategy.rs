//! Strategy Pattern Detector (§9.2.5).
//! Detects strategy interfaces/abstract algorithms and their concrete strategy implementors.

use crate::ingestion::TokenCorpusArtifact;
use crate::symbol::SymbolTableArtifact;

pub fn is_strategy(
    sym_id: u32,
    sta: &SymbolTableArtifact,
    tca: &TokenCorpusArtifact,
) -> (bool, u16) {
    let sym = match sta.symbol(sym_id) {
        Some(s) => s,
        None => return (false, 0),
    };

    let name = std::str::from_utf8(tca.interner.lookup_text(sym.name_id)).unwrap_or("");
    let is_name_match = name.ends_with("Strategy") || name.contains("Strategy");

    // Check if symbol is an interface/abstract class or implements a Strategy interface
    let mut implements_strategy = false;
    for edge in &sta.th_edges {
        if edge.from_sym == sym_id {
            if let Some(target_sym) = sta.symbol(edge.to_sym) {
                let target_name =
                    std::str::from_utf8(tca.interner.lookup_text(target_sym.name_id)).unwrap_or("");
                if target_name.ends_with("Strategy") || target_name.contains("Strategy") {
                    implements_strategy = true;
                    break;
                }
            }
        }
    }

    if is_name_match {
        (true, 95)
    } else if implements_strategy {
        (true, 90)
    } else {
        (false, 0)
    }
}
