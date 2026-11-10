//! StructuralExtractor — coordinates structural UML diagram extractions (§9.3).
//!
//! Deployment diagrams are not extracted: the token corpus carries no
//! deployment descriptors (Dockerfile, compose, Kubernetes/Helm manifests), so
//! deployment views have no evidence and must be reported as unavailable.

pub mod class_diagram;
pub mod component_diagram;
pub mod composite_diagram;
pub mod object_diagram;
pub mod package_diagram;

pub use class_diagram::{ClassDiagramExtractor, ClassExtractionOptions};
pub use component_diagram::ComponentDiagramExtractor;
pub use composite_diagram::CompositeDiagramExtractor;
pub use object_diagram::ObjectDiagramExtractor;
pub use package_diagram::{PackageDiagramExtractor, PackageStructure};

use crate::cfg::serializer::CFGArtifact;
use crate::psa::types::PathSummaryArtifact;
use crate::uma::bodies::MemberBodies;
use crate::uma::index::UMAIndex;
use crate::uma::label_extraction::LabelTable;
use crate::uma::types::*;

/// All structural records of one Phase 9 run.
pub struct StructuralRecords {
    pub classes: Vec<ClassRecord>,
    pub objects: Vec<ObjectRecord>,
    pub object_links: Vec<ObjectLinkRecord>,
    pub packages: Vec<PackageRecord>,
    pub package_dependencies: Vec<PackageDependencyRecord>,
    pub package_metrics: Vec<PackageMetricsRecord>,
    pub components: Vec<ComponentRecord>,
    pub composite_structures: Vec<CompositeStructureRecord>,
}

pub struct StructuralExtractor;

impl StructuralExtractor {
    pub fn extract_all(
        index: &UMAIndex<'_>,
        bodies: &MemberBodies,
        psa: &PathSummaryArtifact,
        cfa: &CFGArtifact,
        labels: &mut LabelTable,
        options: ClassExtractionOptions,
    ) -> StructuralRecords {
        let classes = ClassDiagramExtractor::extract(index, psa, cfa, bodies, labels, options);
        let class_syms: Vec<u32> = classes.iter().map(|c| c.sym_id).collect();
        let (objects, object_links) =
            ObjectDiagramExtractor::extract(index, bodies, labels, &class_syms);
        let PackageStructure {
            packages,
            dependencies,
            metrics,
        } = PackageDiagramExtractor::extract(index, bodies, &class_syms);
        let components = ComponentDiagramExtractor::extract(index, bodies, &packages, &class_syms);
        let composite_structures = CompositeDiagramExtractor::extract(index, &classes);

        StructuralRecords {
            classes,
            objects,
            object_links,
            packages,
            package_dependencies: dependencies,
            package_metrics: metrics,
            components,
            composite_structures,
        }
    }
}
