//! Adapter (object adapter) pattern rule (§9.2.5).
//!
//! An adapter realizes a target abstraction and implements its operations by
//! calling a wrapped adaptee of an unrelated type.
//!
//! Anchor: the Adapter. Mandatory: (1) a user supertype T (the Target); (2) a
//! non-static, non-collection attribute typed by a user type E (the Adaptee)
//! outside both T's and the adapter's hierarchies; (3) an operation overriding an
//! operation of T invokes an operation of E's hierarchy through that attribute.
//! Optional: the forwarded operation has a different name (translation); the
//! adaptee is injected through a constructor parameter.

use crate::uma::patterns::chain::{Evidence, PatternDetectionRule, PatternMatch};
use crate::uma::patterns::query::PatternContext;
use crate::uma::types::*;

pub struct AdapterRule;

impl PatternDetectionRule for AdapterRule {
    fn name(&self) -> &'static str {
        "Adapter"
    }

    fn pattern_kind(&self) -> u8 {
        PATTERN_ADAPTER
    }

    fn evaluate(&self, a: u32, ctx: &PatternContext<'_>) -> Vec<PatternMatch> {
        let mut out = Vec::new();
        if !ctx.is_user_type(a) || ctx.is_interface(a) {
            return out;
        }
        for t in ctx.user_supertypes(a) {
            for f in ctx.fields(a) {
                let e = f.type_id;
                if f.is_static || f.is_collection || !ctx.is_user_type(e) {
                    continue;
                }
                if ctx.related(e, t) || ctx.related(e, a) {
                    continue;
                }
                let mut ev = Evidence::new();
                ev.require(EVIDENCE_SUPERTYPE, true);
                ev.require(EVIDENCE_WRAPPED_FIELD, true);
                let overrides = ctx.overrides_of(a, t);
                let forwarding: Vec<u32> = overrides
                    .iter()
                    .copied()
                    .filter(|&m| ctx.forwards_through(m, f.name, e, false))
                    .collect();
                if !ev.require(EVIDENCE_FORWARDING_CALL, !forwarding.is_empty()) {
                    continue;
                }
                let translated = forwarding.iter().any(|&m| {
                    let own = ctx.name_id(m);
                    ctx.calls_through(m, f.name)
                        .iter()
                        .any(|c| c.method_text != own)
                });
                ev.optional(EVIDENCE_TRANSLATED_CALL, translated);
                let injected = ctx
                    .constructors(a)
                    .into_iter()
                    .any(|k| ctx.param_types(k).contains(&e) && ctx.assigns_from_name(k, f.name));
                ev.optional(EVIDENCE_INJECTED_DELEGATE, injected);
                out.push(
                    PatternMatch::new(PATTERN_ADAPTER, a, ev)
                        .with(ROLE_ADAPTER, a)
                        .with(ROLE_TARGET, t)
                        .with(ROLE_ADAPTEE, e),
                );
            }
        }
        out
    }
}
