//! Singleton pattern rule (§9.2.5).
//!
//! A class that controls its own instantiation and exposes its unique instance
//! through a static accessor.
//!
//! Mandatory: (1) at least one constructor is declared and every constructor is
//! private (Java's implicit constructor is public); (2) a static attribute typed
//! by the class itself, directly or in a nested static holder class; (3) a
//! non-private static method returning the class. Optional: the accessor
//! returns or lazily assigns that attribute; the attribute is final.
//! Language singletons — a Kotlin `object` declaration, or an enum with exactly
//! one constant (Bloch) — match on the declaration alone.

use crate::core::types::symbol::SymbolKind;
use crate::core::types::token::LangId;
use crate::uma::patterns::chain::{Evidence, PatternDetectionRule, PatternMatch};
use crate::uma::patterns::query::PatternContext;
use crate::uma::types::*;

pub struct SingletonRule;

impl PatternDetectionRule for SingletonRule {
    fn name(&self) -> &'static str {
        "Singleton"
    }

    fn pattern_kind(&self) -> u8 {
        PATTERN_SINGLETON
    }

    fn evaluate(&self, sym_id: u32, ctx: &PatternContext<'_>) -> Vec<PatternMatch> {
        detect(sym_id, ctx).into_iter().collect()
    }
}

fn language_singleton(c: u32) -> Option<PatternMatch> {
    let mut ev = Evidence::new();
    ev.require(EVIDENCE_LANGUAGE_SINGLETON, true);
    Some(PatternMatch::new(PATTERN_SINGLETON, c, ev).with(ROLE_SINGLETON, c))
}

fn detect(c: u32, ctx: &PatternContext<'_>) -> Option<PatternMatch> {
    let index = ctx.index;
    let kind = index.kind(c)?;
    if kind == SymbolKind::SK_ENUM {
        let constants = index
            .children_of_kind(c, SymbolKind::SK_ENUM_CONSTANT)
            .count();
        return if constants == 1 {
            language_singleton(c)
        } else {
            None
        };
    }
    if kind != SymbolKind::SK_CLASS {
        return None;
    }
    if index.language_of_sym(c) == Some(LangId::Kotlin) {
        if let Some(sym) = index.symbol(c) {
            if index.scanner.keyword_before_name(sym) == Some(&b"object"[..]) {
                return language_singleton(c);
            }
        }
    }

    let mut ev = Evidence::new();
    let ctors = ctx.constructors(c);
    let private_ctors = !ctors.is_empty() && ctors.iter().all(|&k| ctx.is_private(k));
    if !ev.require(EVIDENCE_PRIVATE_CONSTRUCTORS, private_ctors) {
        return None;
    }

    let self_fields: Vec<(u32, bool)> = ctx
        .fields(c)
        .iter()
        .filter(|f| f.is_static && !f.is_collection && f.type_id == c)
        .map(|f| (f.name, f.is_final))
        .collect();
    let holder: Option<(u32, u32, bool)> = if self_fields.is_empty() {
        index
            .children(c)
            .iter()
            .copied()
            .filter(|&n| index.kind(n) == Some(SymbolKind::SK_CLASS) && ctx.is_static(n))
            .find_map(|n| {
                ctx.fields(n)
                    .iter()
                    .find(|f| f.is_static && !f.is_collection && f.type_id == c)
                    .map(|f| (n, f.name, f.is_final))
            })
    } else {
        None
    };
    if !ev.require(
        EVIDENCE_STATIC_SELF_FIELD,
        !self_fields.is_empty() || holder.is_some(),
    ) {
        return None;
    }
    if holder.is_some() {
        ev.note(EVIDENCE_HOLDER_IDIOM);
    }

    let accessors: Vec<u32> = ctx
        .methods(c)
        .into_iter()
        .filter(|&m| ctx.is_static(m) && ctx.return_type(m) == c && !ctx.is_private(m))
        .collect();
    if !ev.require(EVIDENCE_STATIC_SELF_ACCESSOR, !accessors.is_empty()) {
        return None;
    }

    let field_names: Vec<u32> = self_fields
        .iter()
        .map(|&(n, _)| n)
        .chain(holder.map(|h| h.1))
        .collect();
    let accessor_uses_field = accessors.iter().any(|&m| {
        ctx.body(m)
            .map(|b| field_names.iter().any(|&n| b.returns_name(n) || b.assigns(n)))
            .unwrap_or(false)
    });
    ev.optional(EVIDENCE_ACCESSOR_RETURNS_FIELD, accessor_uses_field);
    let is_final_or_lazy = self_fields.iter().any(|&(_, fin)| fin)
        || holder.map(|h| h.2).unwrap_or(false)
        || accessors
            .iter()
            .any(|&m| field_names.iter().any(|&n| ctx.assigns_new(m, n) == Some(c)));
    ev.optional(EVIDENCE_ENCAPSULATED, is_final_or_lazy);

    Some(PatternMatch::new(PATTERN_SINGLETON, c, ev).with(ROLE_SINGLETON, c))
}
