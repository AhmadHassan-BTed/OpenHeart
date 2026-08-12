//! CommunicationDiagramExtractor — object collaboration graphs (§9.2.1).

use crate::symbol::SymbolTableArtifact;
use crate::uma::types::SequenceDiagramRecord;

pub struct CommunicationDiagramExtractor;

impl CommunicationDiagramExtractor {
    pub fn extract(
        _sta: &SymbolTableArtifact,
        _sequences: &[SequenceDiagramRecord],
    ) {
        // Shared data with SequenceDiagramRecord, rendered as collaboration graph
    }
}
