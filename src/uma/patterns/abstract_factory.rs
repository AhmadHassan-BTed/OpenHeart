//! Abstract Factory pattern rule (§9.2.5).
//!
//! An abstract factory declares creation operations for a family of distinct
//! abstract products; concrete factories override them to allocate concrete
//! products of one family.
//!
//! Anchor: the AbstractFactory. Mandatory: (1) the anchor is an abstract class or
//! an interface; (2) it declares ≥2 non-static operations whose results are
//! distinct user product types; (3) a concrete subtype overrides ≥2 of them, each
//! override allocating a proper subtype of its product. Optional: ≥2 concrete
//! factories.

use std::collections::BTreeSet;

use crate::uma::patterns::chain::{Evidence, PatternDetectionRule, PatternMatch};
use crate::uma::patterns::query::PatternContext;
use crate::uma::types::*;

pub struct AbstractFactoryRule;

impl PatternDetectionRule for AbstractFactoryRule {
    fn name(&self) -> &'static str {
        "AbstractFactory"
    }

    fn pattern_kind(&self) -> u8 {
        PATTERN_ABSTRACT_FACTORY
    }

    fn evaluate(&self, af: u32, ctx: &PatternContext<'_>) -> Vec<PatternMatch> {
        if !ctx.is_abstract_type(af) {
            return Vec::new();
        }
        let creation_ops: Vec<(u32, u32)> = ctx
            .methods(af)
            .into_iter()
            .filter(|&m| !ctx.is_static(m))
            .map(|m| (m, ctx.return_type(m)))
            .filter(|&(_, p)| {
                ctx.is_user_type(p)
                    && !ctx.related(p, af)
                    && (ctx.is_abstract_type(p) || !ctx.descendants(p).is_empty())
            })
            .collect();
        let distinct: BTreeSet<u32> = creation_ops.iter().map(|&(_, p)| p).collect();
        let mut ev = Evidence::new();
        if !ev.require(EVIDENCE_ALLOCATES_SUBTYPE, distinct.len() >= 2) {
            return Vec::new();
        }

        let mut factories = Vec::new();
        let mut concrete_products: BTreeSet<u32> = BTreeSet::new();
        for sub in ctx.concrete_descendants(af) {
            let mut families = 0usize;
            let mut made_here = Vec::new();
            for &(op, p) in &creation_ops {
                if let Some(ov) = ctx.index.same_signature_in(op, sub) {
                    let made: Vec<u32> = ctx
                        .allocated_types(ov)
                        .into_iter()
                        .filter(|&t| t != p && ctx.is_subtype_of(t, p))
                        .collect();
                    if !made.is_empty() {
                        families += 1;
                        made_here.extend(made);
                    }
                }
            }
            if families >= 2 {
                factories.push(sub);
                concrete_products.extend(made_here);
            }
        }
        if !ev.require(EVIDENCE_OVERRIDE, !factories.is_empty()) {
            return Vec::new();
        }
        ev.optional(EVIDENCE_MULTIPLE_IMPLEMENTORS, factories.len() >= 2);

        let products: Vec<u32> = distinct.into_iter().collect();
        let concrete: Vec<u32> = concrete_products.into_iter().collect();
        vec![PatternMatch::new(PATTERN_ABSTRACT_FACTORY, af, ev)
            .with(ROLE_ABSTRACT_FACTORY, af)
            .with_all(ROLE_CONCRETE_FACTORY, &factories)
            .with_all(ROLE_PRODUCT, &products)
            .with_all(ROLE_CONCRETE_PRODUCT, &concrete)]
    }
}
