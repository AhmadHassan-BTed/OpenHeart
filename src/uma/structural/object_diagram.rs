use crate::core::types::symbol::SymbolKind;
use crate::ssa::serializer::SSAArtifact;
use crate::symbol::SymbolTableArtifact;
use crate::uma::types::ObjectRecord;

pub struct ObjectDiagramExtractor;

impl ObjectDiagramExtractor {
    pub fn extract(sta: &SymbolTableArtifact, ssa: &SSAArtifact) -> Vec<ObjectRecord> {
        let mut objects = Vec::new();

        for func_ssa in &ssa.functions {
            for ssa_rec in &func_ssa.ssa_records {
                // Check if this SSA variable represents an allocation site of a class-like symbol
                if !ssa_rec.is_phi() && ssa_rec.orig_sym_id != u32::MAX {
                    if let Some(target_sym) = sta.symbol(ssa_rec.orig_sym_id) {
                        let kind = SymbolKind::from(target_sym.kind);
                        if matches!(kind, SymbolKind::SK_CLASS | SymbolKind::SK_INTERFACE | SymbolKind::SK_ENUM | SymbolKind::SK_RECORD) {
                            objects.push(ObjectRecord {
                                alloc_ssa_id: ssa_rec.ssa_id,
                                type_sym_id: ssa_rec.orig_sym_id,
                                label_text_id: ssa_rec.ssa_id,
                                containing_method_sym: func_ssa.sym_id,
                            });
                        }
                    }
                }
            }
        }

        objects
    }
}
