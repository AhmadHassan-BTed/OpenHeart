//! Pass 5 — Type Hierarchy Construction & UML Association Detection
//!
//! Supertypes are read from the grammar clauses of each type header — `extends`, `implements`
//! and `permits` for Java (parsed from the tokens after the type's name, because Phase 2 drops
//! `generic_type` and flattens `superclass`/`super_interfaces`), the instantiated type of an
//! anonymous class, and the delegation specifiers of Kotlin declarations. Type arguments of a
//! supertype (`extends BaseService<Order, OrderRepository>`) are kept in its TypeRef and never
//! become supertypes; type-parameter bounds are not supertypes either. The relation follows
//! the clause and both declaration kinds (JLS 8.1.4, 8.1.5, 9.1.3): a class has at most one
//! superclass, and an interface extends interfaces.
//!
//! Field associations follow the decision table in `AssociationDetector`. Cycles of the
//! hierarchy are reported as diagnostics; no edge is deleted.

use crate::ast::BPASTArtifact;
use crate::core::types::symbol::{SymbolKind, THRelation};
use crate::ingestion::TokenCorpusArtifact;
use crate::symbol::adapter::{is_primitive_type_id, SemanticAdapter};
use crate::symbol::ast_index::AstIndex;
use crate::symbol::builder::SymbolTableBuilder;
use crate::symbol::model::{DiagnosticCode, Dialect, EdgeFlags, SupertypeRecord};
use crate::symbol::passes::type_lowering::TypeLowering;
use crate::symbol::syntax::{parse_java_type_header, TokenView, TypeOptions, TypeParser};
use crate::symbol::uml_meta::AssociationDetector;

pub struct Pass5Hierarchy;

impl Pass5Hierarchy {
    pub fn run(
        bpa: &BPASTArtifact,
        tca: &TokenCorpusArtifact,
        ast: &AstIndex,
        adapter: &dyn SemanticAdapter,
        builder: &mut SymbolTableBuilder,
    ) {
        let tv = TokenView::new(tca);
        let file_ranges = tv.file_ranges();
        let file_end = |tok: u32| -> u32 {
            tv.file(tok)
                .and_then(|f| file_ranges.get(&f).map(|r| r.1))
                .unwrap_or(tv.len())
        };
        let mut lowering = TypeLowering::new(&tv, adapter);

        // 1. Java type headers.
        let types = builder.structural_types.clone();
        for (sym, _node, name_tok) in types {
            if !tv.valid(name_tok) {
                continue;
            }
            let kind = match builder.symbol_kind(sym) {
                Some(k) => k,
                None => continue,
            };
            let header = parse_java_type_header(&tv, name_tok, file_end(name_tok));
            if header.malformed {
                builder.add_diagnostic(
                    DiagnosticCode::MalformedDeclaration,
                    sym,
                    name_tok,
                    Vec::new(),
                    "type header could not be parsed completely; later clauses are ignored"
                        .to_string(),
                );
            }
            let scope = builder.owned_scope(sym);
            let mut clauses = Vec::new();
            match kind {
                SymbolKind::SK_CLASS => {
                    if header.extends.len() > 1 {
                        builder.add_diagnostic(
                            DiagnosticCode::MultipleSuperclasses,
                            sym,
                            name_tok,
                            Vec::new(),
                            format!(
                                "class declares {} superclasses; only the first is used",
                                header.extends.len()
                            ),
                        );
                    }
                    if let Some(sup) = header.extends.first() {
                        clauses.push((sup.clone(), THRelation::TH_EXTENDS));
                    }
                    clauses.extend(
                        header
                            .implements
                            .iter()
                            .map(|t| (t.clone(), THRelation::TH_IMPLEMENTS)),
                    );
                }
                SymbolKind::SK_INTERFACE => clauses.extend(
                    header
                        .extends
                        .iter()
                        .map(|t| (t.clone(), THRelation::TH_EXTENDS)),
                ),
                SymbolKind::SK_ENUM | SymbolKind::SK_RECORD => clauses.extend(
                    header
                        .implements
                        .iter()
                        .map(|t| (t.clone(), THRelation::TH_IMPLEMENTS)),
                ),
                _ => {}
            }
            for (ty, relation) in clauses {
                let tr = lowering.lower(builder, &ty, scope, true);
                let head = builder.type_ref(tr).map(|t| t.head).unwrap_or(u32::MAX);
                if head == u32::MAX || head == sym || is_primitive_type_id(head) {
                    continue;
                }
                let target_kind = builder.effective_type_kind(head);
                let target_is_interface = matches!(
                    target_kind,
                    Some(SymbolKind::SK_INTERFACE | SymbolKind::SK_ANNOTATION_TYPE)
                );
                let expects_interface =
                    relation == THRelation::TH_IMPLEMENTS || kind == SymbolKind::SK_INTERFACE;
                if target_kind.is_some() && target_is_interface != expects_interface {
                    builder.add_diagnostic(
                        DiagnosticCode::MalformedDeclaration,
                        sym,
                        ty.first,
                        vec![head],
                        format!(
                            "{:?} names a supertype of kind {:?} in a clause that requires {}",
                            kind,
                            target_kind,
                            if expects_interface {
                                "an interface"
                            } else {
                                "a class"
                            }
                        ),
                    );
                }
                builder.add_th_edge(sym, head, relation);
                builder.supertypes.push(SupertypeRecord {
                    sub: sym,
                    sup: head,
                    relation,
                    type_ref: tr,
                });
            }
        }

        // 2. Anonymous classes extend the instantiated class or implement the interface.
        let anonymous = builder.anonymous_classes.clone();
        for (anon, _node, new_tok) in anonymous {
            let (scope, parent) = match builder.symbol(anon) {
                Some(s) => (s.scope_id, s.parent_sym),
                None => continue,
            };
            if new_tok == u32::MAX {
                // Constant-specific class body: an anonymous subclass of the enum (JLS 8.9.1).
                let enum_sym = builder
                    .symbol(parent)
                    .map(|c| c.type_id)
                    .unwrap_or(u32::MAX);
                if builder.symbol_kind(enum_sym) == Some(SymbolKind::SK_ENUM) {
                    builder.add_th_edge(anon, enum_sym, THRelation::TH_EXTENDS);
                    let tr = builder.add_type_ref(crate::symbol::model::TypeRef {
                        head: enum_sym,
                        written_name_id: u32::MAX,
                        args: Vec::new(),
                        array_dims: 0,
                        wildcard: crate::symbol::model::WildcardKind::None as u8,
                        flags: 0,
                        status: crate::symbol::model::ResolutionStatus::Project as u8,
                        first_token: u32::MAX,
                        last_token: u32::MAX,
                    });
                    builder.supertypes.push(SupertypeRecord {
                        sub: anon,
                        sup: enum_sym,
                        relation: THRelation::TH_EXTENDS,
                        type_ref: tr,
                    });
                }
                continue;
            }
            let mut parser = TypeParser::new(&tv, file_end(new_tok), Dialect::Java);
            let ty = match parser.parse(new_tok + 1, TypeOptions::default()) {
                Some((ty, _)) => ty,
                None => continue,
            };
            let tr = lowering.lower(builder, &ty, scope, false);
            let head = builder.type_ref(tr).map(|t| t.head).unwrap_or(u32::MAX);
            if head == u32::MAX || head == anon {
                continue;
            }
            let (relation, flags) = Self::relation_by_target(builder, anon, head, false, ty.first);
            builder.add_th_edge_with_flags(anon, head, relation, flags);
            builder.supertypes.push(SupertypeRecord {
                sub: anon,
                sup: head,
                relation,
                type_ref: tr,
            });
        }

        // 3. Kotlin delegation specifiers.
        let lexical = builder.lexical_supertypes.clone();
        for ls in lexical {
            let scope = match builder.owned_scope(ls.sub) {
                u32::MAX => builder
                    .symbol(ls.sub)
                    .map(|s| s.scope_id)
                    .unwrap_or(u32::MAX),
                s => s,
            };
            let tr = lowering.lower_span(builder, scope, ls.first, ls.last, ls.dialect, true);
            let head = builder.type_ref(tr).map(|t| t.head).unwrap_or(u32::MAX);
            if head == u32::MAX || head == ls.sub {
                continue;
            }
            let (relation, flags) = if builder.symbol_kind(ls.sub) == Some(SymbolKind::SK_INTERFACE)
                || ls.invokes_constructor
            {
                (THRelation::TH_EXTENDS, 0)
            } else {
                Self::relation_by_target(builder, ls.sub, head, true, ls.first)
            };
            builder.add_th_edge_with_flags(ls.sub, head, relation, flags);
            builder.supertypes.push(SupertypeRecord {
                sub: ls.sub,
                sup: head,
                relation,
                type_ref: tr,
            });
        }

        // 4. Field associations and TH_USES (project types only).
        AssociationDetector::detect_all(bpa, &tv, ast, adapter, builder);

        // 5. Diagnose hierarchy cycles (edges are kept and flagged).
        builder.mark_hierarchy_cycles();
    }

    /// Relation of a supertype whose clause does not determine it (anonymous classes, Kotlin
    /// supertypes without a constructor call), chosen from the target's kind. When the kind is
    /// unknown the relation is assumed and the edge is flagged.
    fn relation_by_target(
        b: &mut SymbolTableBuilder,
        sub: u32,
        head: u32,
        default_implements: bool,
        token: u32,
    ) -> (THRelation, u8) {
        match b.effective_type_kind(head) {
            Some(SymbolKind::SK_INTERFACE | SymbolKind::SK_ANNOTATION_TYPE) => {
                (THRelation::TH_IMPLEMENTS, 0)
            }
            Some(_) => (THRelation::TH_EXTENDS, 0),
            None => {
                let relation = if default_implements {
                    THRelation::TH_IMPLEMENTS
                } else {
                    THRelation::TH_EXTENDS
                };
                b.add_diagnostic(
                    DiagnosticCode::AssumedRelation,
                    sub,
                    token,
                    vec![head],
                    format!(
                        "kind of supertype {} is unknown; relation {:?} assumed",
                        head, relation
                    ),
                );
                (relation, EdgeFlags::RELATION_ASSUMED)
            }
        }
    }
}
