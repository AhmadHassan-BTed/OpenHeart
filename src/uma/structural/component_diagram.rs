use super::package_diagram::PackageDiagramExtractor;
use crate::core::types::cg::CallGraphArtifact;
use crate::core::types::symbol::SymbolKind;
use crate::symbol::SymbolTableArtifact;
use crate::uma::types::ComponentRecord;

pub struct ComponentDiagramExtractor;

impl ComponentDiagramExtractor {
    pub fn extract(sta: &SymbolTableArtifact, _cga: &CallGraphArtifact) -> Vec<ComponentRecord> {
        let pkgs = PackageDiagramExtractor::extract(sta);
        let mut components = Vec::new();

        for pkg in pkgs {
            let mut provided_count = 0u16;
            let mut required_count = 0u16;

            if let Some(pkg_sym) = sta.symbol(pkg.package_sym_id) {
                let mut child_id = pkg_sym.first_child;
                while child_id != u32::MAX && (child_id as usize) < sta.symbol_records.len() {
                    let child = &sta.symbol_records[child_id as usize];
                    if SymbolKind::from(child.kind) == SymbolKind::SK_INTERFACE {
                        provided_count += 1;
                    }
                    child_id = child.next_sibling;
                }
            }

            // Count external interface dependencies from TH_USES / associations
            for edge in &sta.th_edges {
                if let Some(src_sym) = sta.symbol(edge.from_sym) {
                    if src_sym.parent_sym == pkg.package_sym_id {
                        if let Some(dst_sym) = sta.symbol(edge.to_sym) {
                            if dst_sym.parent_sym != pkg.package_sym_id
                                && SymbolKind::from(dst_sym.kind) == SymbolKind::SK_INTERFACE
                            {
                                required_count += 1;
                            }
                        }
                    }
                }
            }

            components.push(ComponentRecord {
                component_sym_id: pkg.package_sym_id,
                name_id: pkg.name_id,
                provided_interface_count: provided_count,
                required_interface_count: required_count,
                _pad: 0,
            });
        }

        components
    }
}
