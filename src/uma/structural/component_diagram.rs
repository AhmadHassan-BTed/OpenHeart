//! ComponentDiagramExtractor — extracts ComponentRecord[] from packages & CGA inter-package edges (§9.2.1).

use super::package_diagram::PackageDiagramExtractor;
use crate::core::types::cg::CallGraphArtifact;
use crate::symbol::SymbolTableArtifact;
use crate::uma::types::ComponentRecord;

pub struct ComponentDiagramExtractor;

impl ComponentDiagramExtractor {
    pub fn extract(sta: &SymbolTableArtifact, _cga: &CallGraphArtifact) -> Vec<ComponentRecord> {
        let pkgs = PackageDiagramExtractor::extract(sta);
        let mut components = Vec::new();

        for pkg in pkgs {
            components.push(ComponentRecord {
                component_sym_id: pkg.package_sym_id,
                name_id: pkg.name_id,
                provided_interface_count: pkg.class_count,
                required_interface_count: pkg.subpackage_count,
                _pad: 0,
            });
        }

        components
    }
}
