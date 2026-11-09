//! BehavioralExtractor — coordinates behavioral UML diagram extractions (§9.3).
//!
//! Timing diagrams are not extracted: no prior phase models threads or time,
//! so there is no evidence to build them from.

pub mod activity_diagram;
pub mod communication_diagram;
pub mod interaction_overview;
pub mod sequence_diagram;
pub mod state_machine;

pub use activity_diagram::ActivityDiagramExtractor;
pub use communication_diagram::CommunicationDiagramExtractor;
pub use interaction_overview::InteractionOverviewExtractor;
pub use sequence_diagram::{SequenceDiagramExtractor, SequenceOptions};
pub use state_machine::StateMachineExtractor;

use crate::ast::BPASTArtifact;
use crate::cfg::serializer::CFGArtifact;
use crate::ingestion::TokenCorpusArtifact;
use crate::psa::types::PathSummaryArtifact;
use crate::uma::bodies::MemberBodies;
use crate::uma::index::UMAIndex;
use crate::uma::label_extraction::LabelTable;
use crate::uma::types::*;

/// All behavioral records of one Phase 9 run.
pub struct BehavioralRecords {
    pub activities: Vec<ActivityRecord>,
    pub state_machines: Vec<StateMachineRecord>,
    pub sequences: Vec<SequenceDiagramRecord>,
    pub communications: Vec<CommunicationDiagramRecord>,
    pub interaction_overviews: Vec<InteractionOverviewRecord>,
}

pub struct BehavioralExtractor;

impl BehavioralExtractor {
    #[allow(clippy::too_many_arguments)]
    pub fn extract_all(
        index: &UMAIndex<'_>,
        bodies: &MemberBodies,
        cfa: &CFGArtifact,
        bpa: &BPASTArtifact,
        tca: &TokenCorpusArtifact,
        psa: &PathSummaryArtifact,
        labels: &mut LabelTable,
        class_syms: &[u32],
        include_tests: bool,
        sequence_options: SequenceOptions,
    ) -> BehavioralRecords {
        let activities = ActivityDiagramExtractor::extract_all(cfa, bpa, tca, psa, labels);
        let state_machines = StateMachineExtractor::extract_all(index, bodies, labels, class_syms);
        let sequences = SequenceDiagramExtractor::extract_all(
            index,
            bodies,
            labels,
            include_tests,
            sequence_options,
        );
        let communications = CommunicationDiagramExtractor::extract(&sequences);
        let interaction_overviews =
            InteractionOverviewExtractor::extract(index, &activities, &sequences);

        BehavioralRecords {
            activities,
            state_machines,
            sequences,
            communications,
            interaction_overviews,
        }
    }
}
