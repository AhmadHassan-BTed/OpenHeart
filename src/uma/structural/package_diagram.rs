//! PackageDiagramExtractor — extracts PackageRecord[] from STA packages (§9.2.1).

use crate::core::types::symbol::SymbolKind;
use crate::symbol::SymbolTableArtifact;
use crate::uma::types::PackageRecord;

pub struct PackageDiagramExtractor;

impl PackageDiagramExtractor {
    pub fn extract(sta: &SymbolTableArtifact) -> Vec<PackageRecord> {
        let mut packages = Vec::new();

        for sym_id in 0..sta.symbol_count as u32 {
            let sym = match sta.symbol(sym_id) {
                Some(s) => s,
                None => continue,
            };
            if SymbolKind::from(sym.kind) == SymbolKind::SK_PACKAGE {
                if !sta.custom_package_names.contains_key(&sym_id)
                    && sym.first_child == u32::MAX
                    && sym.name_id == u32::MAX
                {
                    continue;
                }

                let mut class_count = 0u16;
                let mut subpackage_count = 0u16;

                let mut child_id = sym.first_child;
                while child_id != u32::MAX && (child_id as usize) < sta.symbol_records.len() {
                    let child = &sta.symbol_records[child_id as usize];
                    match SymbolKind::from(child.kind) {
                        SymbolKind::SK_CLASS
                        | SymbolKind::SK_INTERFACE
                        | SymbolKind::SK_ENUM
                        | SymbolKind::SK_RECORD => {
                            class_count += 1;
                        }
                        SymbolKind::SK_PACKAGE => {
                            subpackage_count += 1;
                        }
                        _ => {}
                    }
                    child_id = child.next_sibling;
                }

                packages.push(PackageRecord {
                    package_sym_id: sym_id,
                    name_id: sym.name_id,
                    parent_package_sym: sym.parent_sym,
                    class_count,
                    subpackage_count,
                });
            }
        }

        packages
    }
}
