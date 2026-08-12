//! InteractionOverviewExtractor — hybrid activity diagram embedding sequence diagram references (§9.2.1).

use crate::symbol::SymbolTableArtifact;
use crate::uma::types::{ActivityRecord, SequenceDiagramRecord};

pub struct InteractionOverviewExtractor;

impl InteractionOverviewExtractor {
    pub fn extract(
        _sta: &SymbolTableArtifact,
        _activities: &[ActivityRecord],
        _sequences: &[SequenceDiagramRecord],
    ) {
        // Hybrid activity embedding sequence records
    }
}
