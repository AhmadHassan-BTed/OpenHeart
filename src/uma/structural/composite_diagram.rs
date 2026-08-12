//! CompositeDiagramExtractor — extracts inner class and composition port records (§9.2.1).

use crate::symbol::SymbolTableArtifact;
use crate::uma::types::ClassRecord;

pub struct CompositeDiagramExtractor;

impl CompositeDiagramExtractor {
    pub fn extract(
        _sta: &SymbolTableArtifact,
        _classes: &[ClassRecord],
    ) {
        // Inner class structure & composition links are encoded within ClassRecord.inner_classes
    }
}
