//! Factory Method pattern rule (§9.2.5).
//!
//! A creator declares a factory operation returning an abstract product, and
//! subclasses override it to allocate concrete products.
//!
//! Anchor: the Creator. Mandatory: (1) the creator is an abstract class or an
//! interface; (2) it declares a non-static operation f whose result is a user
//! product type P (abstract, or with subtypes); (3) a subtype overrides f and
//! the override allocates a proper subtype of P. Optional: another operation of
//! the creator invokes f on `this` (the factory method serves the creator's own
//! logic); the overrides allocate ≥2 concrete products or there are ≥2
//! concrete creators.

use std::collections::BTreeSet;

use crate::uma::patterns::chain::{Evidence, PatternDetectionRule, PatternMatch};
use crate::uma::patterns::query::PatternContext;
use crate::uma::types::*;

pub struct FactoryMethodRule;

impl PatternDetectionRule for FactoryMethodRule {
    fn name(&self) -> &'static str {
        "FactoryMethod"
    }

    fn pattern_kind(&self) -> u8 {
        PATTERN_FACTORY
    }

    fn evaluate(&self, c: u32, ctx: &PatternContext<'_>) -> Vec<PatternMatch> {
        let mut out = Vec::new();
        if !ctx.is_abstract_type(c) {
            return out;
        }
        let subtypes = ctx.descendants(c);
        if subtypes.is_empty() {
            return out;
        }
        for f in ctx.methods(c) {
            if ctx.is_static(f) {
                continue;
            }
            let p = ctx.return_type(f);
            if !ctx.is_user_type(p) || ctx.related(p, c) {
                continue;
            }
            if !ctx.is_abstract_type(p) && ctx.descendants(p).is_empty() {
                continue;
            }
            let mut creators = Vec::new();
            let mut products: BTreeSet<u32> = BTreeSet::new();
            for &sub in &subtypes {
                if let Some(ov) = ctx.index.same_signature_in(f, sub) {
                    let made: Vec<u32> = ctx
                        .allocated_types(ov)
                        .into_iter()
                        .filter(|&t| t != p && ctx.is_subtype_of(t, p))
                        .collect();
                    if !made.is_empty() {
                        creators.push(sub);
                        products.extend(made);
                    }
                }
            }
            let mut ev = Evidence::new();
            if !ev.require(EVIDENCE_ALLOCATES_SUBTYPE, !creators.is_empty()) {
                continue;
            }
            let f_name = ctx.name_id(f);
            let used_by_creator = ctx
                .concrete_methods(c)
                .into_iter()
                .any(|m| m != f && ctx.invokes_own(m, f_name));
            ev.optional(EVIDENCE_HOOK_CALL, used_by_creator);
            ev.optional(
                EVIDENCE_MULTIPLE_IMPLEMENTORS,
                products.len() >= 2 || creators.len() >= 2,
            );
            let products: Vec<u32> = products.into_iter().collect();
            out.push(
                PatternMatch::new(PATTERN_FACTORY, c, ev)
                    .with(ROLE_CREATOR, c)
                    .with_all(ROLE_CONCRETE_CREATOR, &creators)
                    .with(ROLE_PRODUCT, p)
                    .with_all(ROLE_CONCRETE_PRODUCT, &products),
            );
        }
        out
    }
}
