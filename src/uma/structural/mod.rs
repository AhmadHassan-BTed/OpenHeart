//! StructuralExtractor — coordinates structural UML diagram extractions (§9.3).

pub mod class_diagram;
pub mod component_diagram;
pub mod composite_diagram;
pub mod object_diagram;
pub mod package_diagram;

pub use class_diagram::ClassDiagramExtractor;
pub use component_diagram::ComponentDiagramExtractor;
pub use composite_diagram::CompositeDiagramExtractor;
pub use object_diagram::ObjectDiagramExtractor;
pub use package_diagram::PackageDiagramExtractor;

use crate::core::types::cg::CallGraphArtifact;
use crate::ingestion::TokenCorpusArtifact;
use crate::psa::types::PathSummaryArtifact;
use crate::ssa::serializer::SSAArtifact;
use crate::symbol::SymbolTableArtifact;
use crate::tra::types::TraceabilityArtifact;
use crate::uma::types::*;

pub struct StructuralExtractor;

impl StructuralExtractor {
    pub fn extract_all(
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
        psa: &PathSummaryArtifact,
        tra: &TraceabilityArtifact,
        ssa: &SSAArtifact,
        cga: &CallGraphArtifact,
    ) -> (
        Vec<ClassRecord>,
        Vec<ObjectRecord>,
        Vec<PackageRecord>,
        Vec<ComponentRecord>,
    ) {
        let classes = ClassDiagramExtractor::extract(sta, tca, psa, tra);
        let objects = ObjectDiagramExtractor::extract(sta, ssa);
        let packages = PackageDiagramExtractor::extract(sta);
        let components = ComponentDiagramExtractor::extract(sta, cga);

        CompositeDiagramExtractor::extract(sta, &classes);

        (classes, objects, packages, components)
    }
}
