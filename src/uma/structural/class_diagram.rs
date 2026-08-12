//! ClassDiagramExtractor — translates STA symbols & TH into ClassRecord[] (§9.2.1).

use crate::core::types::symbol::{SymbolKind, SymbolModifiers};
use crate::psa::types::PathSummaryArtifact;
use crate::symbol::SymbolTableArtifact;
use crate::tra::types::{TraceabilityArtifact, UMLLinkRecord};
use crate::uma::types::*;

pub struct ClassDiagramExtractor;

impl ClassDiagramExtractor {
    pub fn extract(
        sta: &SymbolTableArtifact,
        psa: &PathSummaryArtifact,
        tra: &TraceabilityArtifact,
    ) -> Vec<ClassRecord> {
        let mut classes = Vec::new();

        for sym_id in 0..sta.symbol_count as u32 {
            let sym = match sta.symbol(sym_id) {
                Some(s) => s,
                None => continue,
            };
            let kind = SymbolKind::from(sym.kind);

            let stereotype = match kind {
                SymbolKind::SK_INTERFACE => STEREOTYPE_INTERFACE,
                SymbolKind::SK_ENUM => STEREOTYPE_ENUM,
                SymbolKind::SK_RECORD => STEREOTYPE_RECORD,
                SymbolKind::SK_ANNOTATION_TYPE => STEREOTYPE_ANNOTATION,
                SymbolKind::SK_CLASS => {
                    if (sym.modifiers & SymbolModifiers::ABSTRACT) != 0 {
                        STEREOTYPE_ABSTRACT
                    } else {
                        STEREOTYPE_NONE
                    }
                }
                _ => continue, // Only extract class-like type declarations
            };

            let mut fields = Vec::new();
            let mut methods = Vec::new();
            let mut inner_classes = Vec::new();

            // Collect all member symbols whose parent_sym == sym_id
            for child_sym_id in 0..sta.symbol_count as u32 {
                let child = match sta.symbol(child_sym_id) {
                    Some(c) => c,
                    None => continue,
                };
                if child.parent_sym != sym_id {
                    continue;
                }
                let child_kind = SymbolKind::from(child.kind);

                match child_kind {
                    SymbolKind::SK_FIELD | SymbolKind::SK_ENUM_CONSTANT => {
                        fields.push(FieldRecord {
                            field_sym_id: child.symbol_id,
                            type_sym_id: child.type_id,
                            visibility: child.visibility,
                            modifiers: child.modifiers as u8,
                            is_collection: 0,
                            _pad: 0,
                            uml_link_node: child.decl_node,
                            _reserved: 0,
                        });
                    }
                    SymbolKind::SK_METHOD | SymbolKind::SK_CONSTRUCTOR => {
                        let (cyc, sat) = if let Some(hdr) = psa.function_header(child.symbol_id) {
                            (hdr.cyclomatic, hdr.sat_count)
                        } else {
                            (1, 1)
                        };

                        methods.push(MethodRecord {
                            method_sym_id: child.symbol_id,
                            return_type_sym_id: child.type_id,
                            visibility: child.visibility,
                            modifiers: child.modifiers as u8,
                            param_count: child.param_count,
                            cyclomatic: cyc,
                            sat_count: sat,
                        });
                    }
                    SymbolKind::SK_CLASS
                    | SymbolKind::SK_INTERFACE
                    | SymbolKind::SK_ENUM
                    | SymbolKind::SK_RECORD => {
                        inner_classes.push(child.symbol_id);
                    }
                    _ => {}
                }
            }

            let mut association_syms = Vec::new();
            let mut implements_syms = Vec::new();

            for field in &fields {
                if field.type_sym_id != u32::MAX && field.type_sym_id != sym_id {
                    if let Some(target_sym) = sta.symbol(field.type_sym_id) {
                        let target_kind = SymbolKind::from(target_sym.kind);
                        if matches!(
                            target_kind,
                            SymbolKind::SK_CLASS | SymbolKind::SK_INTERFACE | SymbolKind::SK_ENUM
                        ) {
                            if !association_syms.contains(&field.type_sym_id) {
                                association_syms.push(field.type_sym_id);
                            }
                        }
                    }
                }
            }

            for method in &methods {
                if method.return_type_sym_id != u32::MAX && method.return_type_sym_id != sym_id {
                    if let Some(target_sym) = sta.symbol(method.return_type_sym_id) {
                        let target_kind = SymbolKind::from(target_sym.kind);
                        if matches!(
                            target_kind,
                            SymbolKind::SK_CLASS | SymbolKind::SK_INTERFACE | SymbolKind::SK_ENUM
                        ) {
                            if !association_syms.contains(&method.return_type_sym_id) {
                                association_syms.push(method.return_type_sym_id);
                            }
                        }
                    }
                }
            }

            if sym.parent_sym != u32::MAX {
                if let Some(parent_sym) = sta.symbol(sym.parent_sym) {
                    if SymbolKind::from(parent_sym.kind) == SymbolKind::SK_INTERFACE {
                        implements_syms.push(sym.parent_sym);
                    }
                }
            }

            // Find UMLLink for this class from TRA
            let uml_link = tra
                .uml_links
                .iter()
                .find(|link| link.sym_id == sym_id)
                .cloned()
                .unwrap_or(UMLLinkRecord {
                    sym_id,
                    file_id: 0,
                    line_start: 1,
                    col_start: 1,
                    line_end: 1,
                    col_end: 1,
                    scpg_hash: tra.hashes.scpg_hash,
                    sym_kind: sym.kind,
                    _reserved: [0; 3],
                });

            classes.push(ClassRecord {
                sym_id,
                stereotype,
                visibility: sym.visibility,
                modifiers: sym.modifiers,
                extends_sym: u32::MAX,
                field_count: fields.len() as u16,
                method_count: methods.len() as u16,
                inner_count: inner_classes.len() as u16,
                design_pattern: PATTERN_NONE,
                _reserved: 0,
                type_param_count: sym.type_param_count,
                _pad: 0,
                uml_link,
                fields,
                methods,
                inner_classes,
                implements_syms,
                association_syms,
            });
        }

        classes
    }
}
