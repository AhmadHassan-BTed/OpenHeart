//! ObjectDiagramExtractor — static object diagrams from allocation sites (§9.2.1;
//! Tonella & Potrich, Reverse Engineering of Object Oriented Code, 2005).
//!
//! - Objects: every `new T(...)` of a user type T inside a member body or a
//!   field initializer of an extracted class. The label is `name : T` when the
//!   instance is stored into a variable or attribute, `: T` otherwise.
//! - Links: an instance stored into attribute `f` of class C is linked from
//!   every allocation site of C (or of a subtype of C) via `f` — the static
//!   approximation of the field points-to relation.

use std::collections::HashMap;

use crate::core::types::symbol::SymbolKind;
use crate::uma::bodies::MemberBodies;
use crate::uma::index::UMAIndex;
use crate::uma::label_extraction::LabelTable;
use crate::uma::source::AssignedValue;
use crate::uma::types::*;

pub struct ObjectDiagramExtractor;

impl ObjectDiagramExtractor {
    pub fn extract(
        index: &UMAIndex<'_>,
        bodies: &MemberBodies,
        labels: &mut LabelTable,
        class_syms: &[u32],
    ) -> (Vec<ObjectRecord>, Vec<ObjectLinkRecord>) {
        let mut objects: Vec<ObjectRecord> = Vec::new();

        for &c in class_syms {
            let field_by_name: HashMap<u32, u32> = index
                .fields(c)
                .into_iter()
                .map(|f| (index.name_id(f), f))
                .collect();

            // Field initializers: `private final Engine engine = new Engine();`
            for (&name, &f) in Self::sorted(&field_by_name) {
                let sym = match index.symbol(f) {
                    Some(s) => s,
                    None => continue,
                };
                let (a, b) = match index.scanner.initializer_range(sym) {
                    Some(r) => r,
                    None => continue,
                };
                let file = match index.scanner.file_of(a) {
                    Some(fid) => fid,
                    None => continue,
                };
                let mut t = a;
                while t <= b {
                    if index.scanner.is(t, b"new") {
                        if let Some(n) = index.scanner.next_code(t, file) {
                            let ty = index.resolve_type_text(index.scanner.text_id(n), c);
                            if index.is_user_type(ty) {
                                objects.push(ObjectRecord {
                                    alloc_token: t,
                                    type_sym_id: ty,
                                    label_text_id: labels.intern(&format!(
                                        "{} : {}",
                                        String::from_utf8_lossy(
                                            index.tca.interner.lookup_text(name)
                                        ),
                                        index.name(ty)
                                    )),
                                    containing_method_sym: UNKNOWN_SYM,
                                    assigned_field_sym: f,
                                });
                            }
                        }
                    }
                    t = match index.scanner.next_code(t, file) {
                        Some(n) => n,
                        None => break,
                    };
                }
            }

            for &member in index.children(c) {
                if !matches!(
                    index.kind(member),
                    Some(SymbolKind::SK_METHOD)
                        | Some(SymbolKind::SK_CONSTRUCTOR)
                        | Some(SymbolKind::SK_STATIC_INIT)
                        | Some(SymbolKind::SK_INSTANCE_INIT)
                ) {
                    continue;
                }
                let body = match bodies.get(member) {
                    Some(b) => b,
                    None => continue,
                };
                for alloc in &body.allocations {
                    let ty = index.resolve_type_text(alloc.type_text, member);
                    if !index.is_user_type(ty) {
                        continue;
                    }
                    let target = body.assignments.iter().find(|asg| {
                        matches!(asg.value, AssignedValue::New { new_token, .. } if new_token == alloc.new_token)
                    });
                    let (var_name, assigned_field_sym) = match target {
                        Some(asg) => {
                            let field = if !asg.is_declaration && !asg.via_other_receiver {
                                field_by_name
                                    .get(&asg.lhs_text)
                                    .copied()
                                    .unwrap_or(UNKNOWN_SYM)
                            } else {
                                UNKNOWN_SYM
                            };
                            (Some(asg.lhs_text), field)
                        }
                        None => (None, UNKNOWN_SYM),
                    };
                    let label = match var_name {
                        Some(v) => format!(
                            "{} : {}",
                            String::from_utf8_lossy(index.tca.interner.lookup_text(v)),
                            index.name(ty)
                        ),
                        None => format!(": {}", index.name(ty)),
                    };
                    objects.push(ObjectRecord {
                        alloc_token: alloc.new_token,
                        type_sym_id: ty,
                        label_text_id: labels.intern(&label),
                        containing_method_sym: member,
                        assigned_field_sym,
                    });
                }
            }
        }

        objects.sort_by_key(|o| (o.alloc_token, o.type_sym_id));
        objects.dedup_by_key(|o| (o.alloc_token, o.type_sym_id));

        let mut by_type: HashMap<u32, Vec<usize>> = HashMap::new();
        for (i, o) in objects.iter().enumerate() {
            by_type.entry(o.type_sym_id).or_default().push(i);
        }
        let mut links = Vec::new();
        for (to_idx, obj) in objects.iter().enumerate() {
            if obj.assigned_field_sym == UNKNOWN_SYM {
                continue;
            }
            let owner = index.enclosing_type(obj.assigned_field_sym);
            if owner == UNKNOWN_SYM {
                continue;
            }
            let mut holder_types = vec![owner];
            holder_types.extend(index.descendants(owner));
            let mut holders: Vec<usize> = holder_types
                .iter()
                .filter_map(|t| by_type.get(t))
                .flatten()
                .copied()
                .filter(|&i| i != to_idx)
                .collect();
            holders.sort_unstable();
            for from_idx in holders {
                if let (Ok(from_object), Ok(to_object)) =
                    (u32::try_from(from_idx), u32::try_from(to_idx))
                {
                    links.push(ObjectLinkRecord {
                        from_object,
                        to_object,
                        via_field_sym: obj.assigned_field_sym,
                    });
                }
            }
        }
        (objects, links)
    }

    fn sorted(map: &HashMap<u32, u32>) -> Vec<(&u32, &u32)> {
        let mut v: Vec<(&u32, &u32)> = map.iter().collect();
        v.sort_by_key(|&(_, &f)| f);
        v
    }
}
