//! ObjectDiagramExtractor — extracts ObjectRecord[] from SSA allocation sites (§9.2.1).

use crate::ssa::serializer::SSAArtifact;
use crate::symbol::SymbolTableArtifact;
use crate::uma::types::ObjectRecord;

pub struct ObjectDiagramExtractor;

impl ObjectDiagramExtractor {
    pub fn extract(
        _sta: &SymbolTableArtifact,
        ssa: &SSAArtifact,
    ) -> Vec<ObjectRecord> {
        let mut objects = Vec::new();

        for func_ssa in &ssa.functions {
            for ssa_rec in &func_ssa.ssa_records {
                // Check if this SSA variable represents an allocation site (e.g. new instance)
                if !ssa_rec.is_phi() && ssa_rec.orig_sym_id != u32::MAX {
                    objects.push(ObjectRecord {
                        alloc_ssa_id: ssa_rec.ssa_id,
                        type_sym_id: ssa_rec.orig_sym_id,
                        label_text_id: ssa_rec.ssa_id,
                        containing_method_sym: func_ssa.sym_id,
                    });
                }
            }
        }

        objects
    }
}
