//! Factory Method / Abstract Factory pattern query.

use crate::ingestion::TokenCorpusArtifact;
use crate::symbol::SymbolTableArtifact;

pub fn is_factory(class_sym: u32, sta: &SymbolTableArtifact, tca: &TokenCorpusArtifact) -> (bool, u16) {
    let sym = match sta.symbol(class_sym) {
        Some(s) => s,
        None => return (false, 0),
    };
    let bytes = tca.interner.lookup_text(sym.name_id);
    let name = std::str::from_utf8(bytes).unwrap_or("");
    if name.to_lowercase().contains("factory") {
        (true, 90)
    } else {
        (false, 0)
    }
}
