//! BehavioralExtractor — coordinates behavioral UML diagram extractions (§9.3).

pub mod activity_diagram;
pub mod communication_diagram;
pub mod interaction_overview;
pub mod sequence_diagram;
pub mod state_machine;
pub mod timing_diagram;

pub use activity_diagram::ActivityDiagramExtractor;
pub use communication_diagram::CommunicationDiagramExtractor;
pub use interaction_overview::InteractionOverviewExtractor;
pub use sequence_diagram::SequenceDiagramExtractor;
pub use state_machine::StateMachineExtractor;
pub use timing_diagram::TimingDiagramExtractor;

use crate::ast::BPASTArtifact;
use crate::cfg::serializer::CFGArtifact;
use crate::core::types::cg::CallGraphArtifact;
use crate::ingestion::TokenCorpusArtifact;
use crate::psa::types::PathSummaryArtifact;
use crate::ssa::serializer::SSAArtifact;
use crate::symbol::SymbolTableArtifact;
use crate::tra::types::TraceabilityArtifact;
use crate::uma::types::*;

pub struct BehavioralExtractor;

impl BehavioralExtractor {
    pub fn extract_all(
        sta: &SymbolTableArtifact,
        cfa: &CFGArtifact,
        bpa: &BPASTArtifact,
        tca: &TokenCorpusArtifact,
        psa: &PathSummaryArtifact,
        ssa: &SSAArtifact,
        cga: &CallGraphArtifact,
        tra: &TraceabilityArtifact,
    ) -> (
        Vec<ActivityRecord>,
        Vec<StateMachineRecord>,
        Vec<SequenceDiagramRecord>,
    ) {
        let activities = ActivityDiagramExtractor::extract_all(cfa, bpa, tca, sta, psa);
        let state_machines = StateMachineExtractor::extract_all(sta, ssa);
        let sequences = SequenceDiagramExtractor::extract_all(sta, cfa, cga, tra);

        CommunicationDiagramExtractor::extract(sta, &sequences);
        InteractionOverviewExtractor::extract(sta, &activities, &sequences);
        TimingDiagramExtractor::extract(sta);

        (activities, state_machines, sequences)
    }
}
