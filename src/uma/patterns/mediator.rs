//! Mediator pattern rule (§9.2.5).
//!
//! Colleagues hold a reference to a mediator abstraction and notify it; a
//! concrete mediator references the colleagues and coordinates them.
//!
//! Anchor: the Mediator abstraction. Mandatory: (1) the anchor is an abstract
//! class or an interface; (2) ≥2 distinct colleague classes outside its hierarchy
//! hold an attribute typed by it and invoke one of its operations through that
//! attribute; (3) a concrete mediator (subtype of the anchor) holds attributes
//! referencing ≥2 colleague types (or their supertypes). Optional: the concrete
//! mediator invokes operations on the colleagues.

use crate::uma::patterns::chain::{Evidence, PatternDetectionRule, PatternMatch};
use crate::uma::patterns::query::PatternContext;
use crate::uma::types::*;

pub struct MediatorRule;

impl PatternDetectionRule for MediatorRule {
    fn name(&self) -> &'static str {
        "Mediator"
    }

    fn pattern_kind(&self) -> u8 {
        PATTERN_MEDIATOR
    }

    fn evaluate(&self, md: u32, ctx: &PatternContext<'_>) -> Vec<PatternMatch> {
        if !ctx.is_abstract_type(md) {
            return Vec::new();
        }
        let colleagues: Vec<u32> = ctx
            .referrers(md)
            .iter()
            .copied()
            .filter(|&col| !ctx.related(col, md))
            .filter(|&col| {
                ctx.fields(col).iter().any(|f| {
                    !f.is_collection
                        && f.type_id == md
                        && ctx
                            .methods(col)
                            .into_iter()
                            .any(|m| ctx.forwards_through(m, f.name, md, false))
                })
            })
            .collect();
        let mut ev = Evidence::new();
        if !ev.require(EVIDENCE_COLLEAGUE_CALLS, colleagues.len() >= 2) {
            return Vec::new();
        }

        let mut mediators = Vec::new();
        let mut coordinates = false;
        for cm in ctx.concrete_descendants(md) {
            let held: Vec<(u32, u32)> = ctx
                .fields(cm)
                .iter()
                .filter_map(|f| {
                    let r = f.referenced_type();
                    let hit = colleagues.iter().copied().find(|&col| ctx.is_subtype_of(col, r));
                    hit.map(|col| (f.name, col))
                })
                .collect();
            let mut distinct: Vec<u32> = held.iter().map(|&(_, col)| col).collect();
            distinct.sort_unstable();
            distinct.dedup();
            if distinct.len() >= 2 || held.iter().any(|&(name, _)| {
                ctx.fields(cm).iter().any(|f| f.name == name && f.is_collection)
            }) {
                mediators.push(cm);
                coordinates |= ctx.methods(cm).into_iter().any(|m| {
                    held.iter()
                        .any(|&(name, col)| ctx.forwards_through(m, name, col, false))
                });
            }
        }
        if !ev.require(EVIDENCE_BACK_REFERENCE, !mediators.is_empty()) {
            return Vec::new();
        }
        ev.optional(EVIDENCE_FORWARDING_CALL, coordinates);

        vec![PatternMatch::new(PATTERN_MEDIATOR, md, ev)
            .with(ROLE_MEDIATOR, md)
            .with_all(ROLE_CONCRETE_MEDIATOR, &mediators)
            .with_all(ROLE_COLLEAGUE, &colleagues)]
    }
}
