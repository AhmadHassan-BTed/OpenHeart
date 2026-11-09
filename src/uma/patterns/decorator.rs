//! Decorator pattern rule (§9.2.5).
//!
//! A decorator realizes the component abstraction, wraps one component typed by
//! that abstraction, and forwards component operations to it.
//!
//! Anchor: the Decorator (the class declaring the wrapped attribute).
//! Mandatory: (1) a user abstract supertype C (the Component); (2) a non-static,
//! non-collection attribute typed exactly C; (3) an operation overriding an
//! operation of C invokes the same operation through that attribute.
//! Optional: the component is injected through a constructor parameter typed C;
//! C has a concrete component outside the decorator's line; the decorator has
//! decorator subclasses. A wrapped attribute typed by a concrete subject is a
//! Proxy instead (see `proxy.rs`), so the two rules never overlap.

use crate::uma::patterns::chain::{Evidence, PatternDetectionRule, PatternMatch};
use crate::uma::patterns::query::PatternContext;
use crate::uma::types::*;

pub struct DecoratorRule;

impl PatternDetectionRule for DecoratorRule {
    fn name(&self) -> &'static str {
        "Decorator"
    }

    fn pattern_kind(&self) -> u8 {
        PATTERN_DECORATOR
    }

    fn evaluate(&self, d: u32, ctx: &PatternContext<'_>) -> Vec<PatternMatch> {
        let mut out = Vec::new();
        if !ctx.is_user_type(d) || ctx.is_interface(d) {
            return out;
        }
        for c in ctx.user_ancestors(d) {
            if !ctx.is_abstract_type(c) {
                continue;
            }
            for f in ctx.fields(d) {
                if f.is_static || f.is_collection || f.type_id != c {
                    continue;
                }
                let mut ev = Evidence::new();
                ev.require(EVIDENCE_SUPERTYPE, true);
                ev.require(EVIDENCE_WRAPPED_FIELD, true);
                let forwards = ctx
                    .overrides_of(d, c)
                    .into_iter()
                    .any(|m| ctx.forwards_through(m, f.name, c, true));
                if !ev.require(EVIDENCE_FORWARDING_CALL, forwards) {
                    continue;
                }
                let injected = ctx.constructors(d).into_iter().any(|k| {
                    ctx.param_types(k).contains(&c) && ctx.assigns_from_name(k, f.name)
                });
                ev.optional(EVIDENCE_INJECTED_DELEGATE, injected);
                let line = ctx.descendants(d);
                let components: Vec<u32> = ctx
                    .concrete_descendants(c)
                    .into_iter()
                    .filter(|&x| x != d && !line.contains(&x))
                    .collect();
                ev.optional(EVIDENCE_CONCRETE_PARTICIPANTS, !components.is_empty());
                ev.optional(EVIDENCE_REFINED_ABSTRACTION, !line.is_empty());
                out.push(
                    PatternMatch::new(PATTERN_DECORATOR, d, ev)
                        .with(ROLE_DECORATOR, d)
                        .with_all(ROLE_DECORATOR, &line)
                        .with(ROLE_COMPONENT, c)
                        .with_all(ROLE_CONCRETE_COMPONENT, &components),
                );
                return out;
            }
        }
        out
    }
}
