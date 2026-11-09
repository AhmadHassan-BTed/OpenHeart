//! AssociationDetector: UML association inference from declared field types (phase3.md
//! decision table, classification after Guéhéneuc & Albin-Amiot, OOPSLA 2004).
//!
//! * Primitives, library value types (boxed primitives, `String`, `java.time`, `BigDecimal`,
//!   `UUID`, ...), type variables and types outside the project are attributes, not
//!   associations.
//! * Arrays and library collections (`Collection`, `Iterable`, `List`, `Set`, ...) give a
//!   0..* end to their element type, maps to their value type; `Optional` gives 0..1.
//! * A single reference to a project type is 0..1 in Java (1 when annotated `@NonNull`,
//!   `@NotNull` or `@Nonnull`) and 1 for a non-nullable Kotlin type.
//! * Collection-valued ends are aggregations. A single-valued end is a composition only with
//!   lifecycle evidence: a private final field initialized with `new` of the target (or a
//!   subtype) whose owner declares no method returning the target type, so the part neither
//!   outlives nor escapes the whole as far as the visible code shows. Otherwise it is a plain
//!   association. Constructor bodies are dropped by Phase 2 and give no evidence.
//! * Static fields are class-scope attributes and are not associations.
//! * Two ends that reference each other's owner exactly once are paired as one bidirectional
//!   association.

use crate::ast::BPASTArtifact;
use crate::core::types::ast::ASTNodeType;
use crate::core::types::symbol::{
    AssocKind, SymbolKind, SymbolModifiers, SymbolVisibility, THRelation, UMLAssociationRecord,
};
use crate::symbol::adapter::SemanticAdapter;
use crate::symbol::ast_index::AstIndex;
use crate::symbol::builder::SymbolTableBuilder;
use crate::symbol::model::{Dialect, TypeRefFlags, WildcardKind};
use crate::symbol::std_library::LibraryCategory;
use crate::symbol::syntax::TokenView;
use std::collections::{HashMap, HashSet, VecDeque};

/// The far end of an association: target type and multiplicity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AssociationEnd {
    pub target: u32,
    pub mult_min: u16,
    pub mult_max: u16,
    pub many: bool,
}

pub struct AssociationDetector;

impl AssociationDetector {
    /// Far end of the association declared by `field`, or `None` when the field is an
    /// attribute.
    pub fn classify_field(
        adapter: &dyn SemanticAdapter,
        b: &SymbolTableBuilder,
        tv: &TokenView,
        field: u32,
    ) -> Option<AssociationEnd> {
        let tr = b.symbol_type_refs.get(field as usize).copied()?;
        let (target, min, max, many) = Self::end_of(adapter, b, tr, 0)?;
        if many {
            return Some(AssociationEnd {
                target,
                mult_min: min,
                mult_max: max,
                many,
            });
        }
        let nullable = b
            .type_ref(tr)
            .map(|t| t.flags & TypeRefFlags::NULLABLE != 0)
            .unwrap_or(false);
        let kotlin = b
            .type_spans
            .get(&field)
            .map(|s| s.dialect == Dialect::Kotlin)
            .unwrap_or(false);
        let mult_min = if max == 1 && min == 0 && (kotlin && !nullable || Self::has_non_null_annotation(b, tv, field))
        {
            1
        } else {
            min
        };
        Some(AssociationEnd {
            target,
            mult_min,
            mult_max: max,
            many,
        })
    }

    /// (target, min, max, many) of a TypeRef, following collections, maps, optionals and
    /// arrays to their element type.
    fn end_of(
        adapter: &dyn SemanticAdapter,
        b: &SymbolTableBuilder,
        tr: u32,
        depth: u8,
    ) -> Option<(u32, u16, u16, bool)> {
        if depth > 4 {
            return None;
        }
        let t = b.type_ref(tr)?;
        if t.flags
            & (TypeRefFlags::UNION
                | TypeRefFlags::FUNCTION
                | TypeRefFlags::UNKNOWN
                | TypeRefFlags::PRIMITIVE
                | TypeRefFlags::TYPE_VARIABLE)
            != 0
        {
            return None;
        }
        if t.is_wildcard() {
            return if t.wildcard == WildcardKind::Extends as u8 {
                Self::end_of(adapter, b, *t.args.first()?, depth + 1)
            } else {
                None
            };
        }
        if t.array_dims > 0 {
            return if b.is_project_type(t.head) {
                Some((t.head, 0, u16::MAX, true))
            } else {
                None
            };
        }
        if b.is_project_type(t.head) {
            return Some((t.head, 0, 1, false));
        }
        let category = match b.library_category_of(t.head) {
            Some(c) => c,
            // A library type outside the catalog: the language adapter decides whether its
            // qualified name denotes a collection.
            None => match b.qualified_name_of(t.head) {
                Some(q)
                    if b.symbol(t.head).is_some_and(|s| {
                        s.flags & crate::symbol::model::SymbolFlags::UNRESOLVED == 0
                    }) && adapter.is_collection_type(q) =>
                {
                    LibraryCategory::Collection
                }
                _ => return None,
            },
        };
        match category {
            LibraryCategory::Collection => {
                let (h, _, _, _) = Self::end_of(adapter, b, *t.args.first()?, depth + 1)?;
                Some((h, 0, u16::MAX, true))
            }
            LibraryCategory::Map => {
                let (h, _, _, _) = Self::end_of(adapter, b, *t.args.get(1)?, depth + 1)?;
                Some((h, 0, u16::MAX, true))
            }
            LibraryCategory::Optional => {
                let inner = Self::end_of(adapter, b, *t.args.first()?, depth + 1)?;
                if inner.3 {
                    Some(inner)
                } else {
                    Some((inner.0, 0, 1, false))
                }
            }
            LibraryCategory::Value | LibraryCategory::Other => None,
        }
    }

    /// `@NonNull`, `@NotNull` or `@Nonnull` written among the field's modifiers.
    fn has_non_null_annotation(b: &SymbolTableBuilder, tv: &TokenView, field: u32) -> bool {
        let first = match b.symbol(field) {
            Some(s) if tv.valid(s.first_token_id) => s.first_token_id,
            _ => return false,
        };
        let limit = b
            .type_spans
            .get(&field)
            .map(|s| s.first)
            .unwrap_or(first);
        (first..limit).any(|t| {
            tv.punct(t, "@")
                && tv.next(t, limit).is_some_and(|n| {
                    matches!(tv.text(n), b"NonNull" | b"NotNull" | b"Nonnull")
                })
        })
    }

    /// Lifecycle evidence for composition (see the module documentation).
    fn has_composition_evidence(
        bpa: &BPASTArtifact,
        ast: &AstIndex,
        b: &SymbolTableBuilder,
        supertypes: &HashMap<u32, Vec<u32>>,
        field: u32,
        owner: u32,
        target: u32,
    ) -> bool {
        let f = match b.symbol(field) {
            Some(f) => f,
            None => return false,
        };
        if SymbolVisibility::from(f.visibility) != SymbolVisibility::Private
            || f.modifiers & SymbolModifiers::FINAL == 0
            || f.decl_node == u32::MAX
        {
            return false;
        }
        let children = ast.children(f.decl_node);
        let init = match children {
            [first, second, ..] if bpa.node_type(*first) == ASTNodeType::NN_IDENTIFIER_EXPR => {
                *second
            }
            _ => return false,
        };
        if bpa.node_type(init) != ASTNodeType::NN_NEW_EXPR {
            return false;
        }
        let created = match ast
            .first_child_of_type(bpa, init, ASTNodeType::NN_TYPE_REF)
            .and_then(|n| b.get_type_ref_resolution(n))
        {
            Some(c) => c,
            None => return false,
        };
        if !Self::is_subtype(supertypes, created, target) {
            return false;
        }
        // A method of the owner whose declared type is the target can hand the part out.
        let escapes = b.symbols.iter().any(|m| {
            m.parent_sym == owner
                && SymbolKind::from(m.kind) == SymbolKind::SK_METHOD
                && m.type_id == target
        });
        !escapes
    }

    fn is_subtype(supertypes: &HashMap<u32, Vec<u32>>, sub: u32, sup: u32) -> bool {
        if sub == sup {
            return true;
        }
        let mut seen = HashSet::new();
        let mut queue = VecDeque::from([sub]);
        while let Some(t) = queue.pop_front() {
            if !seen.insert(t) {
                continue;
            }
            for &s in supertypes.get(&t).map(|v| v.as_slice()).unwrap_or(&[]) {
                if s == sup {
                    return true;
                }
                queue.push_back(s);
            }
        }
        false
    }

    /// Emits association records and TH_USES edges for every instance field of a project type,
    /// then pairs mutually referencing ends.
    pub fn detect_all(
        bpa: &BPASTArtifact,
        tv: &TokenView,
        ast: &AstIndex,
        adapter: &dyn SemanticAdapter,
        b: &mut SymbolTableBuilder,
    ) {
        let supertypes = b.supertype_index();
        let mut records = Vec::new();
        for field in 0..b.symbol_count() as u32 {
            let (owner, name_id, modifiers) = match b.symbol(field) {
                Some(s) if SymbolKind::from(s.kind) == SymbolKind::SK_FIELD => {
                    (s.parent_sym, s.name_id, s.modifiers)
                }
                _ => continue,
            };
            if modifiers & SymbolModifiers::STATIC != 0 || !b.is_project_type(owner) {
                continue;
            }
            let end = match Self::classify_field(adapter, b, tv, field) {
                Some(e) => e,
                None => continue,
            };
            let kind = if end.many {
                AssocKind::Aggregation
            } else if Self::has_composition_evidence(
                bpa,
                ast,
                b,
                &supertypes,
                field,
                owner,
                end.target,
            ) {
                AssocKind::Composition
            } else {
                AssocKind::Association
            };
            records.push(UMLAssociationRecord {
                from_symbol_id: owner,
                to_symbol_id: end.target,
                field_symbol_id: field,
                assoc_kind: kind as u8,
                mult_min: end.mult_min,
                mult_max: end.mult_max,
                is_navigable: 1,
                role_name_id: name_id,
                _reserved: 0,
                _padding: 0,
            });
        }

        let mut by_pair: HashMap<(u32, u32), Vec<u32>> = HashMap::new();
        for rec in records {
            b.add_th_edge(rec.from_symbol_id, rec.to_symbol_id, THRelation::TH_USES);
            let idx = b.add_association(rec);
            by_pair
                .entry((rec.from_symbol_id, rec.to_symbol_id))
                .or_default()
                .push(idx);
        }
        let mut pairs: Vec<(u32, u32)> = Vec::new();
        for (&(from, to), ends) in &by_pair {
            if from >= to || ends.len() != 1 {
                continue;
            }
            if let Some(back) = by_pair.get(&(to, from)) {
                if back.len() == 1 {
                    pairs.push((ends[0], back[0]));
                }
            }
        }
        pairs.sort_unstable();
        b.association_pairs.extend(pairs);
    }
}
