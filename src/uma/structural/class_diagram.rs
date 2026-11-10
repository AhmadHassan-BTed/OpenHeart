//! ClassDiagramExtractor — translates STA symbols & TH into ClassRecord[] (§9.2.1).
//!
//! Types are selected by symbol kind and origin only: no name lists. A type is
//! excluded when it is synthetic, when it was fabricated by the token sweep in a
//! language that has a real AST front end (no declaration node), or when it lives
//! in a test source root and tests are not requested.
//!
//! Relationships follow Guéhéneuc & Albin-Amiot (OOPSLA 2004):
//! - attributes typed by user types are associations; library collections and
//!   arrays give multiplicity `0..*` towards their element type;
//! - an attribute whose value the owner allocates and never hands out is a
//!   composition; a collection attribute is an aggregation;
//! - return, parameter and instantiated types are «use»/«create» dependencies.

use std::collections::{BTreeSet, HashMap};

use crate::cfg::serializer::CFGArtifact;
use crate::core::logger::{log_debug, log_info};
use crate::core::types::symbol::{SymbolKind, SymbolModifiers, SymbolRecord};
use crate::core::types::token::LangId;
use crate::psa::types::PathSummaryArtifact;
use crate::tra::types::UMLLinkRecord;
use crate::uma::bodies::MemberBodies;
use crate::uma::index::UMAIndex;
use crate::uma::label_extraction::LabelTable;
use crate::uma::source::{is_identifier_text, AssignedValue};
use crate::uma::types::*;

/// Selection options for the class extractor.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ClassExtractionOptions {
    /// Include types declared under test source roots (`src/test`, `__tests__`, ...).
    pub include_tests: bool,
}

pub struct ClassDiagramExtractor;

#[derive(Default)]
struct ExclusionStats {
    synthetic: usize,
    sweep_fabricated: usize,
    unnamed: usize,
    tests: usize,
}

impl ClassDiagramExtractor {
    pub fn extract(
        index: &UMAIndex<'_>,
        psa: &PathSummaryArtifact,
        cfa: &CFGArtifact,
        bodies: &MemberBodies,
        labels: &mut LabelTable,
        options: ClassExtractionOptions,
    ) -> Vec<ClassRecord> {
        let mut classes = Vec::new();
        let mut stats = ExclusionStats::default();
        let cfg_cyclomatic: HashMap<u32, u16> = cfa
            .functions
            .iter()
            .map(|f| (f.sym_id, f.cyclomatic))
            .collect();
        let scpg_hash = index.tra.map(|t| t.hashes.scpg_hash).unwrap_or(0);

        for sym_id in 0..index.symbol_count() {
            let id = sym_id as u32;
            let sym = match index.symbol(id) {
                Some(s) => s,
                None => continue,
            };
            let kind = SymbolKind::from(sym.kind);
            if !UMAIndex::is_type_kind(kind) {
                continue;
            }
            if !Self::is_selected(index, id, sym, options, &mut stats) {
                continue;
            }
            classes.push(Self::build_record(
                index,
                id,
                sym,
                kind,
                psa,
                &cfg_cyclomatic,
                bodies,
                labels,
                scpg_hash,
            ));
        }

        log_info(&format!(
            "  Phase 9 class extraction: {} types; excluded {} synthetic, {} sweep-fabricated, {} unnamed, {} test-root types; {} supertype edges dropped as type arguments/bounds, {} recovered from headers.",
            classes.len(),
            stats.synthetic,
            stats.sweep_fabricated,
            stats.unnamed,
            stats.tests,
            index.dropped_supertype_edges,
            index.recovered_supertype_edges,
        ));
        classes
    }

    /// Selection by symbol kind and origin only.
    fn is_selected(
        index: &UMAIndex<'_>,
        id: u32,
        sym: &SymbolRecord,
        options: ClassExtractionOptions,
        stats: &mut ExclusionStats,
    ) -> bool {
        if (sym.modifiers & SymbolModifiers::SYNTHETIC) != 0 {
            stats.synthetic += 1;
            return false;
        }
        let lang = index.language_of_sym(id);
        if sym.decl_node == u32::MAX && lang == Some(LangId::Java) {
            // Java has an AST front end: a type without a declaration node was
            // produced by the secondary token sweep, not by a declaration.
            stats.sweep_fabricated += 1;
            log_debug(&format!(
                "[UMA] Skipping sweep-fabricated type sym_id={} name={}",
                id,
                index.name(id)
            ));
            return false;
        }
        if !is_identifier_text(index.name_bytes(id)) {
            stats.unnamed += 1;
            return false;
        }
        if !options.include_tests {
            if let Some(fid) = index.file_of_sym(id) {
                if index.is_test_file(fid) {
                    stats.tests += 1;
                    return false;
                }
            }
        }
        true
    }

    #[allow(clippy::too_many_arguments)]
    fn build_record(
        index: &UMAIndex<'_>,
        id: u32,
        sym: &SymbolRecord,
        kind: SymbolKind,
        psa: &PathSummaryArtifact,
        cfg_cyclomatic: &HashMap<u32, u16>,
        bodies: &MemberBodies,
        labels: &mut LabelTable,
        scpg_hash: u32,
    ) -> ClassRecord {
        let stereotype = match kind {
            SymbolKind::SK_INTERFACE => STEREOTYPE_INTERFACE,
            SymbolKind::SK_ENUM => STEREOTYPE_ENUM,
            SymbolKind::SK_RECORD => STEREOTYPE_RECORD,
            SymbolKind::SK_ANNOTATION_TYPE => STEREOTYPE_ANNOTATION,
            _ => {
                if (sym.modifiers & SymbolModifiers::ABSTRACT) != 0 {
                    STEREOTYPE_ABSTRACT
                } else {
                    STEREOTYPE_NONE
                }
            }
        };

        let mut fields = Vec::new();
        let mut methods = Vec::new();
        let mut inner_classes = Vec::new();
        for &child in index.children(id) {
            match index.kind(child) {
                Some(SymbolKind::SK_FIELD) => {
                    fields.push(Self::field_record(index, id, child, bodies));
                }
                Some(SymbolKind::SK_ENUM_CONSTANT) => {
                    fields.push(Self::enum_literal_record(index, id, child));
                }
                Some(SymbolKind::SK_METHOD) | Some(SymbolKind::SK_CONSTRUCTOR) => {
                    methods.push(Self::method_record(index, child, psa, cfg_cyclomatic));
                }
                Some(k) if UMAIndex::is_type_kind(k) => inner_classes.push(child),
                _ => {}
            }
        }

        // ── Generalization / realization (validated supertypes) ──
        let generalization_syms = index.direct_supertypes(id);
        let (extends_sym, implements_syms) = if kind == SymbolKind::SK_INTERFACE {
            let ext = index.extends_of(id);
            (
                if ext.len() == 1 { ext[0] } else { UNKNOWN_SYM },
                Vec::new(),
            )
        } else {
            let mut imp: Vec<u32> = index.implements_of(id).to_vec();
            imp.sort_unstable();
            imp.dedup();
            (index.superclass(id), imp)
        };

        // ── Associations from attributes ──
        let mut association_set: BTreeSet<u32> = BTreeSet::new();
        for f in &fields {
            if f.field_kind != FIELD_KIND_ATTRIBUTE || f.assoc_kind == ASSOC_NONE {
                continue;
            }
            let target = if f.is_collection != 0 {
                f.element_type_sym_id
            } else {
                f.type_sym_id
            };
            if index.is_user_type(target) {
                association_set.insert(target);
            }
        }

        // ── Dependencies: return, parameter and instantiated types ──
        let mut dependency_set: BTreeSet<u32> = BTreeSet::new();
        for m in &methods {
            if index.is_user_type(m.return_type_sym_id) {
                dependency_set.insert(m.return_type_sym_id);
            }
            for &p in &m.param_type_syms {
                if index.is_user_type(p) {
                    dependency_set.insert(p);
                }
            }
            if let Some(body) = bodies.get(m.method_sym_id) {
                for a in &body.allocations {
                    let t = index.resolve_type_text(a.type_text, m.method_sym_id);
                    if index.is_user_type(t) {
                        dependency_set.insert(t);
                    }
                }
            }
        }
        dependency_set.remove(&id);
        for t in association_set.iter().chain(generalization_syms.iter()) {
            dependency_set.remove(t);
        }

        let qualified = index.qualified_name(id);
        let qualified_name_id = if qualified != index.name(id) {
            labels.intern(&qualified)
        } else {
            NO_LABEL
        };

        ClassRecord {
            sym_id: id,
            stereotype,
            visibility: sym.visibility,
            modifiers: sym.modifiers,
            extends_sym,
            field_count: saturating_u16(fields.len()),
            method_count: saturating_u16(methods.len()),
            inner_count: saturating_u16(inner_classes.len()),
            design_pattern: PATTERN_NONE,
            _reserved: 0,
            type_param_count: sym.type_param_count,
            _pad: 0,
            uml_link: Self::uml_link(index, id, sym, scpg_hash),
            fields,
            methods,
            inner_classes,
            implements_syms,
            association_syms: association_set.into_iter().collect(),
            generalization_syms,
            dependency_syms: dependency_set.into_iter().collect(),
            qualified_name_id,
        }
    }

    fn field_record(
        index: &UMAIndex<'_>,
        owner: u32,
        field: u32,
        bodies: &MemberBodies,
    ) -> FieldRecord {
        let fsym = match index.symbol(field) {
            Some(s) => *s,
            None => {
                return FieldRecord {
                    field_sym_id: field,
                    type_sym_id: UNKNOWN_SYM,
                    visibility: 0,
                    modifiers: 0,
                    is_collection: 0,
                    field_kind: FIELD_KIND_ATTRIBUTE,
                    assoc_kind: ASSOC_NONE,
                    mult_min: MULTIPLICITY_UNKNOWN,
                    mult_max: MULTIPLICITY_UNKNOWN,
                    element_type_sym_id: UNKNOWN_SYM,
                    uml_link_node: u32::MAX,
                }
            }
        };
        let type_id = index.type_of(field);
        let shape = if fsym.first_token_id != u32::MAX {
            index.scanner.type_shape(&fsym)
        } else {
            Default::default()
        };
        let library_collection = index.is_collection_type(type_id);
        let is_collection = shape.is_array || library_collection;

        let element_type = if is_collection {
            let from_args = shape
                .type_argument_names
                .iter()
                .rev()
                .map(|&t| index.resolve_type_text(t, owner))
                .find(|&s| s != UNKNOWN_SYM);
            match from_args {
                Some(s) => s,
                None if shape.is_array && index.is_user_type(type_id) => type_id,
                None => UNKNOWN_SYM,
            }
        } else {
            UNKNOWN_SYM
        };

        let target = if is_collection { element_type } else { type_id };
        let assoc_kind = if !index.is_user_type(target) {
            ASSOC_NONE
        } else if is_collection {
            ASSOC_AGGREGATION
        } else if Self::is_owned_part(index, owner, field, &fsym, target, bodies) {
            ASSOC_COMPOSITION
        } else {
            ASSOC_ASSOCIATION
        };

        let is_kotlin = index.language_of_sym(field) == Some(LangId::Kotlin);
        let (mult_min, mult_max) = if type_id == UNKNOWN_SYM {
            (MULTIPLICITY_UNKNOWN, MULTIPLICITY_UNKNOWN)
        } else if is_collection {
            (0, MULTIPLICITY_MANY)
        } else if UMAIndex::is_primitive(type_id) {
            (1, 1)
        } else if assoc_kind == ASSOC_COMPOSITION {
            (1, 1)
        } else if is_kotlin && !shape.is_nullable {
            (1, 1)
        } else {
            // A Java reference attribute may hold null.
            (0, 1)
        };

        FieldRecord {
            field_sym_id: field,
            type_sym_id: type_id,
            visibility: fsym.visibility,
            modifiers: fsym.modifiers,
            is_collection: u8::from(is_collection),
            field_kind: FIELD_KIND_ATTRIBUTE,
            assoc_kind,
            mult_min,
            mult_max,
            element_type_sym_id: element_type,
            uml_link_node: fsym.decl_node,
        }
    }

    fn enum_literal_record(index: &UMAIndex<'_>, owner: u32, literal: u32) -> FieldRecord {
        let sym = index.symbol(literal);
        FieldRecord {
            field_sym_id: literal,
            type_sym_id: owner,
            visibility: sym.map(|s| s.visibility).unwrap_or(0),
            modifiers: sym.map(|s| s.modifiers).unwrap_or(0)
                | SymbolModifiers::STATIC
                | SymbolModifiers::FINAL,
            is_collection: 0,
            field_kind: FIELD_KIND_ENUM_LITERAL,
            assoc_kind: ASSOC_NONE,
            mult_min: 1,
            mult_max: 1,
            element_type_sym_id: UNKNOWN_SYM,
            uml_link_node: sym.map(|s| s.decl_node).unwrap_or(u32::MAX),
        }
    }

    /// Composition: the owner allocates the part itself (initializer or
    /// constructor/initializer body) and no method returns it to callers.
    fn is_owned_part(
        index: &UMAIndex<'_>,
        owner: u32,
        field: u32,
        fsym: &SymbolRecord,
        target: u32,
        bodies: &MemberBodies,
    ) -> bool {
        let name = fsym.name_id;
        let allocates_target = |type_text: u32, ctx: u32| {
            let t = index.resolve_type_text(type_text, ctx);
            t != UNKNOWN_SYM && index.is_subtype_of(t, target)
        };

        let mut allocated = false;
        if let Some((a, b)) = index.scanner.initializer_range(fsym) {
            if let Some(file) = index.scanner.file_of(a) {
                let mut t = a;
                while t <= b {
                    if index.scanner.is(t, b"new") {
                        if let Some(n) = index.scanner.next_code(t, file) {
                            if allocates_target(index.scanner.text_id(n), field) {
                                allocated = true;
                                break;
                            }
                        }
                    }
                    t = match index.scanner.next_code(t, file) {
                        Some(n) => n,
                        None => break,
                    };
                }
            }
        }
        let initializers = index.children(owner).iter().copied().filter(|&c| {
            matches!(
                index.kind(c),
                Some(SymbolKind::SK_CONSTRUCTOR)
                    | Some(SymbolKind::SK_INSTANCE_INIT)
                    | Some(SymbolKind::SK_STATIC_INIT)
            )
        });
        for ctor in initializers {
            if let Some(body) = bodies.get(ctor) {
                for asg in &body.assignments {
                    if asg.lhs_text != name || asg.via_other_receiver || asg.is_declaration {
                        continue;
                    }
                    match asg.value {
                        AssignedValue::New { type_text, .. }
                            if allocates_target(type_text, ctor) =>
                        {
                            allocated = true;
                        }
                        // Assigned from a parameter or another reference: shared, not owned.
                        AssignedValue::Name { .. } => return false,
                        _ => {}
                    }
                }
            }
        }
        if !allocated {
            return false;
        }
        // The part must not escape through a method result.
        for m in index.methods(owner) {
            if let Some(body) = bodies.get(m) {
                if body.returns_name(name) {
                    return false;
                }
            }
        }
        true
    }

    fn method_record(
        index: &UMAIndex<'_>,
        method: u32,
        psa: &PathSummaryArtifact,
        cfg_cyclomatic: &HashMap<u32, u16>,
    ) -> MethodRecord {
        let sym = index.symbol(method);
        let (cyclomatic, sat_count, metrics_known) = match psa.function_header(method) {
            Some(hdr) => (
                hdr.cyclomatic,
                hdr.sat_count,
                METRIC_CYCLOMATIC_KNOWN | METRIC_SAT_COUNT_KNOWN,
            ),
            None => match cfg_cyclomatic.get(&method) {
                Some(&c) => (c, 0, METRIC_CYCLOMATIC_KNOWN),
                None => (0, 0, 0),
            },
        };
        let param_syms = index.params(method);
        let param_type_syms = index.param_types(method);
        MethodRecord {
            method_sym_id: method,
            return_type_sym_id: index.type_of(method),
            visibility: sym.map(|s| s.visibility).unwrap_or(0),
            modifiers: sym.map(|s| s.modifiers).unwrap_or(0),
            param_count: sym
                .map(|s| s.param_count)
                .unwrap_or_else(|| saturating_u16(param_syms.len())),
            cyclomatic,
            sat_count,
            is_constructor: u8::from(index.kind(method) == Some(SymbolKind::SK_CONSTRUCTOR)),
            metrics_known,
            param_syms,
            param_type_syms,
        }
    }

    /// TRA link when recorded; otherwise the declaring token range; otherwise an
    /// explicit unknown location (file `UML_LINK_UNKNOWN_FILE`, line 0).
    fn uml_link(
        index: &UMAIndex<'_>,
        id: u32,
        sym: &SymbolRecord,
        scpg_hash: u32,
    ) -> UMLLinkRecord {
        if let Some(link) = index.uml_link(id) {
            if uml_link_is_known(&link) {
                return link;
            }
        }
        let start = index.scanner.position(sym.first_token_id);
        let end_tok = if sym.last_token_id != u32::MAX && sym.last_token_id >= sym.first_token_id {
            sym.last_token_id
        } else {
            sym.first_token_id
        };
        let end = index.scanner.position(end_tok);
        match (start, end) {
            (Some((fid, ls, cs)), Some((efid, le, ce))) if fid == efid => UMLLinkRecord {
                sym_id: id,
                file_id: fid,
                line_start: ls,
                col_start: cs,
                line_end: le,
                col_end: ce.saturating_add(index.scanner.token_len(end_tok)),
                scpg_hash,
                sym_kind: sym.kind,
                _reserved: [0; 3],
            },
            (Some((fid, ls, cs)), _) => UMLLinkRecord {
                sym_id: id,
                file_id: fid,
                line_start: ls,
                col_start: cs,
                line_end: ls,
                col_end: cs.saturating_add(index.scanner.token_len(sym.first_token_id)),
                scpg_hash,
                sym_kind: sym.kind,
                _reserved: [0; 3],
            },
            _ => UMLLinkRecord {
                sym_id: id,
                file_id: UML_LINK_UNKNOWN_FILE,
                line_start: 0,
                col_start: 0,
                line_end: 0,
                col_end: 0,
                scpg_hash,
                sym_kind: sym.kind,
                _reserved: [0; 3],
            },
        }
    }
}

pub(crate) fn saturating_u16(n: usize) -> u16 {
    u16::try_from(n).unwrap_or(u16::MAX)
}
