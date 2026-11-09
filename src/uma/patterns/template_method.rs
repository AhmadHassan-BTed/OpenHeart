//! Template Method pattern rule (§9.2.5).
//!
//! An abstract class defines a concrete operation (the template) whose body
//! invokes abstract primitive operations that subclasses implement.
//!
//! Anchor: the AbstractClass. Mandatory: (1) the anchor is an abstract class or an
//! interface; (2) a bodied, non-abstract operation t invokes on `this` ≥1 abstract
//! operation h declared by the anchor; (3) a concrete subtype overrides h.
//! Optional: t is final; t invokes ≥2 primitive operations; ≥2 concrete subtypes
//! override the primitives. Visibility of the hooks is irrelevant.

use crate::uma::patterns::chain::{Evidence, PatternDetectionRule, PatternMatch};
use crate::uma::patterns::query::PatternContext;
use crate::uma::types::*;

pub struct TemplateMethodRule;

impl PatternDetectionRule for TemplateMethodRule {
    fn name(&self) -> &'static str {
        "TemplateMethod"
    }

    fn pattern_kind(&self) -> u8 {
        PATTERN_TEMPLATE_METHOD
    }

    fn evaluate(&self, a: u32, ctx: &PatternContext<'_>) -> Vec<PatternMatch> {
        if !ctx.is_abstract_type(a) {
            return Vec::new();
        }
        let hooks = ctx.abstract_methods(a);
        if hooks.is_empty() {
            return Vec::new();
        }
        let subclasses = ctx.concrete_descendants(a);
        let mut best: Option<PatternMatch> = None;

        for t in ctx.concrete_methods(a) {
            if ctx.is_static(t) || hooks.contains(&t) {
                continue;
            }
            let called: Vec<u32> = hooks
                .iter()
                .copied()
                .filter(|&h| ctx.invokes_own(t, ctx.name_id(h)))
                .collect();
            let mut ev = Evidence::new();
            if !ev.require(EVIDENCE_HOOK_CALL, !called.is_empty()) {
                continue;
            }
            let implementors: Vec<u32> = subclasses
                .iter()
                .copied()
                .filter(|&s| {
                    called
                        .iter()
                        .any(|&h| ctx.index.same_signature_in(h, s).is_some())
                })
                .collect();
            if !ev.require(EVIDENCE_OVERRIDE, !implementors.is_empty()) {
                continue;
            }
            ev.optional(EVIDENCE_FINAL_TEMPLATE, ctx.is_final(t));
            ev.optional(EVIDENCE_MULTIPLE_IMPLEMENTORS, called.len() >= 2);
            ev.optional(EVIDENCE_CONCRETE_PARTICIPANTS, implementors.len() >= 2);
            let pm = PatternMatch::new(PATTERN_TEMPLATE_METHOD, a, ev)
                .with(ROLE_ABSTRACT_CLASS, a)
                .with_all(ROLE_CONCRETE_CLASS, &implementors)
                .with(ROLE_TEMPLATE_METHOD, t)
                .with_all(ROLE_PRIMITIVE_OPERATION, &called);
            let better = match &best {
                Some(b) => pm.score() > b.score(),
                None => true,
            };
            if better {
                best = Some(pm);
            }
        }
        best.into_iter().collect()
    }
}
