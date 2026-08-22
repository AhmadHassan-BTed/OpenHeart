//! Pass 5 — Type Hierarchy Construction & UML Association Detection
//! Builds E^TH CSR graph (TH_EXTENDS, TH_IMPLEMENTS, TH_USES) and detects UML field associations.

use crate::ast::BPASTArtifact;
use crate::core::types::ast::ASTNodeType;
use crate::core::types::symbol::{AssocKind, SymbolKind, THRelation, UMLAssociationRecord};
use crate::symbol::builder::SymbolTableBuilder;
use crate::symbol::uml_meta::AssociationDetector;

pub struct Pass5Hierarchy;

impl Pass5Hierarchy {
    pub fn run(bpa: &BPASTArtifact, builder: &mut SymbolTableBuilder) {
        // 1. Explicit inheritance (TH_EXTENDS & TH_IMPLEMENTS)
        for sym_id in 0..builder.symbol_count() as u32 {
            let (kind, decl_node) = match builder.symbol(sym_id) {
                Some(s) => (s.kind, s.decl_node),
                None => continue,
            };

            if decl_node == u32::MAX {
                continue;
            }

            if kind == SymbolKind::SK_CLASS as u8
                || kind == SymbolKind::SK_INTERFACE as u8
                || kind == SymbolKind::SK_ENUM as u8
                || kind == SymbolKind::SK_RECORD as u8
            {
                for type_ref_node in Self::find_all_type_refs(decl_node, bpa) {
                    if let Some(target_sym_id) = builder.get_type_ref_resolution(type_ref_node) {
                        if target_sym_id != sym_id {
                            let target_kind = builder.symbol(target_sym_id).map(|s| s.kind);
                            let relation = if kind == SymbolKind::SK_CLASS as u8
                                && target_kind == Some(SymbolKind::SK_INTERFACE as u8)
                            {
                                THRelation::TH_IMPLEMENTS
                            } else if kind == SymbolKind::SK_INTERFACE as u8 {
                                THRelation::TH_EXTENDS
                            } else {
                                THRelation::TH_EXTENDS
                            };
                            builder.add_th_edge(sym_id, target_sym_id, relation);
                        }
                    }
                }
            }
        }

        // 2. Field-based dependencies (TH_USES & UML associations)
        for sym_id in 0..builder.symbol_count() as u32 {
            let (kind, type_id, owner_sym_id, name_id) = match builder.symbol(sym_id) {
                Some(s) => (s.kind, s.type_id, s.parent_sym, s.name_id),
                None => continue,
            };

            if kind != SymbolKind::SK_FIELD as u8 || type_id == u32::MAX || owner_sym_id == u32::MAX
            {
                continue;
            }

            let field_type_kind = builder
                .symbol(type_id)
                .map(|s| s.kind)
                .unwrap_or(SymbolKind::SK_EXTERNAL as u8);

            if field_type_kind == SymbolKind::SK_CLASS as u8
                || field_type_kind == SymbolKind::SK_INTERFACE as u8
                || field_type_kind == SymbolKind::SK_ENUM as u8
                || field_type_kind == SymbolKind::SK_RECORD as u8
                || field_type_kind == SymbolKind::SK_EXTERNAL as u8
            {
                builder.add_th_edge(owner_sym_id, type_id, THRelation::TH_USES);

                let assoc_kind = AssociationDetector::detect_association(
                    sym_id,
                    owner_sym_id,
                    type_id,
                    builder,
                    bpa,
                );
                if assoc_kind != AssocKind::None {
                    let record = UMLAssociationRecord {
                        from_symbol_id: owner_sym_id,
                        to_symbol_id: type_id,
                        field_symbol_id: sym_id,
                        assoc_kind: assoc_kind as u8,
                        mult_min: 0,
                        mult_max: if assoc_kind == AssocKind::Aggregation {
                            u16::MAX
                        } else {
                            1
                        },
                        is_navigable: 1,
                        role_name_id: name_id,
                        _reserved: 0,
                        _padding: 0,
                    };
                    builder.add_association(record);
                }
            }
        }
    }

    fn find_all_type_refs(decl_node: u32, bpa: &BPASTArtifact) -> Vec<u32> {
        let mut refs = Vec::new();
        let mut cur = bpa.first_child(decl_node);
        while let Some(c) = cur {
            if bpa.node_type(c) == ASTNodeType::NN_TYPE_REF {
                refs.push(c);
            }
            cur = bpa.next_sibling(c);
        }
        refs
    }
}
