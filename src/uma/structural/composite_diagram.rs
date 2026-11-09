//! CompositeDiagramExtractor — composite structure of classes (§9.2.1).
//!
//! Parts are the attributes the class owns by composition (the class allocates
//! the part and never hands it out — see `ClassDiagramExtractor`). A connector
//! links part A to part B when A's type holds a reference to B's type and one of
//! A's operations calls an operation declared in B's type. Ports are not
//! modelled: the sources declare no port concept. Classes without composed
//! parts get no composite structure.

use crate::uma::index::UMAIndex;
use crate::uma::types::*;

pub struct CompositeDiagramExtractor;

impl CompositeDiagramExtractor {
    pub fn extract(index: &UMAIndex<'_>, classes: &[ClassRecord]) -> Vec<CompositeStructureRecord> {
        let mut out = Vec::new();
        for class_rec in classes {
            let parts: Vec<CompositePartRecord> = class_rec
                .fields
                .iter()
                .filter(|f| f.assoc_kind == ASSOC_COMPOSITION && index.is_user_type(f.type_sym_id))
                .map(|f| CompositePartRecord {
                    part_sym: f.field_sym_id,
                    type_sym_id: f.type_sym_id,
                    mult_min: f.mult_min,
                    mult_max: f.mult_max,
                    is_nested_type: u8::from(class_rec.inner_classes.contains(&f.type_sym_id)),
                })
                .collect();
            if parts.is_empty() {
                continue;
            }

            let mut connectors = Vec::new();
            for a in &parts {
                for b in &parts {
                    if a.part_sym == b.part_sym {
                        continue;
                    }
                    let refers = index.fields(a.type_sym_id).into_iter().any(|f| {
                        let t = index.type_of(f);
                        t != UNKNOWN_SYM && index.is_subtype_of(b.type_sym_id, t)
                    });
                    if !refers {
                        continue;
                    }
                    let via = index.methods(a.type_sym_id).into_iter().find(|&m| {
                        index.calls_of_member(m).iter().any(|c| {
                            let owner = index.enclosing_type(c.callee);
                            owner != UNKNOWN_SYM && index.related(owner, b.type_sym_id)
                        })
                    });
                    if let Some(via_method_sym) = via {
                        connectors.push(CompositeConnectorRecord {
                            from_part: a.part_sym,
                            to_part: b.part_sym,
                            via_method_sym,
                        });
                    }
                }
            }
            out.push(CompositeStructureRecord {
                class_sym_id: class_rec.sym_id,
                parts,
                connectors,
            });
        }
        out
    }
}
