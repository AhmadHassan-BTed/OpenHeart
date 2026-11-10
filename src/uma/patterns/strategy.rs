//! Strategy / State / Bridge pattern rule (§9.2.5).
//!
//! All three describe a class that delegates to an abstraction with concrete
//! implementations; they differ in who drives the delegate. The rule therefore
//! classifies each (context, attribute) pair into exactly one of them, so a class
//! can never be reported as both State and Strategy.
//!
//! Anchor: the Context. Mandatory: (1) a non-static, non-collection attribute
//! typed by a user abstract type T outside the context's own hierarchy; (2) T has
//! ≥1 concrete implementation; (3) an operation of the context invokes an
//! operation of T's hierarchy through the attribute.
//! Classification:
//! - **State** when transitions are driven internally: a non-constructor
//!   operation of the context reassigns the attribute to a new concrete T, or a
//!   concrete T calls back into the context or allocates a sibling concrete T.
//! - **Bridge** when the context is itself abstract with refined subclasses, so
//!   abstraction and implementation hierarchies vary independently.
//! - **Strategy** otherwise (the client chooses the delegate).
//! Optional: ≥2 concrete implementations; the delegate is injected through a
//! constructor or setter parameter typed T (Strategy/Bridge).

use crate::uma::patterns::chain::{Evidence, PatternDetectionRule, PatternMatch};
use crate::uma::patterns::query::PatternContext;
use crate::uma::types::*;

pub struct DelegationFamilyRule;

impl PatternDetectionRule for DelegationFamilyRule {
    fn name(&self) -> &'static str {
        "Strategy/State/Bridge"
    }

    fn pattern_kind(&self) -> u8 {
        PATTERN_STRATEGY
    }

    fn evaluate(&self, x: u32, ctx: &PatternContext<'_>) -> Vec<PatternMatch> {
        let mut out = Vec::new();
        if !ctx.is_user_type(x) || ctx.is_interface(x) {
            return out;
        }
        let methods = ctx.methods(x);
        let ctors = ctx.constructors(x);
        for f in ctx.fields(x) {
            let t = f.type_id;
            if f.is_static || f.is_collection || !ctx.is_abstract_type(t) || ctx.related(t, x) {
                continue;
            }
            let impls: Vec<u32> = ctx
                .concrete_descendants(t)
                .into_iter()
                .filter(|&i| i != x)
                .collect();
            let mut ev = Evidence::new();
            ev.require(EVIDENCE_WRAPPED_FIELD, true);
            if !ev.require(EVIDENCE_CONCRETE_PARTICIPANTS, !impls.is_empty()) {
                continue;
            }
            let delegates = methods
                .iter()
                .any(|&m| ctx.forwards_through(m, f.name, t, false));
            if !ev.require(EVIDENCE_POLYMORPHIC_DELEGATE, delegates) {
                continue;
            }
            ev.optional(EVIDENCE_MULTIPLE_IMPLEMENTORS, impls.len() >= 2);

            let reassigned = methods.iter().any(|&m| {
                ctx.assigns_new(m, f.name)
                    .map(|n| ctx.is_subtype_of(n, t))
                    .unwrap_or(false)
            });
            let callbacks = impls.iter().any(|&s| {
                ctx.methods(s).into_iter().any(|m| {
                    ctx.calls_of(m)
                        .iter()
                        .any(|c| ctx.index.enclosing_type(c.callee) == x)
                })
            });
            let sibling_transitions = impls.iter().any(|&s| {
                ctx.methods(s).into_iter().any(|m| {
                    ctx.allocated_types(m)
                        .into_iter()
                        .any(|a| a != s && impls.contains(&a))
                })
            });
            let injected = ctors
                .iter()
                .chain(methods.iter())
                .any(|&m| ctx.param_types(m).contains(&t) && ctx.assigns_from_name(m, f.name));

            let pm = if reassigned || callbacks || sibling_transitions {
                ev.note(EVIDENCE_INTERNAL_REASSIGNMENT);
                ev.optional(EVIDENCE_BACK_REFERENCE, callbacks);
                PatternMatch::new(PATTERN_STATE, x, ev)
                    .with(ROLE_CONTEXT, x)
                    .with(ROLE_STATE, t)
                    .with_all(ROLE_CONCRETE_STATE, &impls)
            } else if ctx.is_abstract_type(x) && !ctx.descendants(x).is_empty() {
                ev.note(EVIDENCE_REFINED_ABSTRACTION);
                ev.optional(EVIDENCE_INJECTED_DELEGATE, injected);
                let refined = ctx.descendants(x);
                PatternMatch::new(PATTERN_BRIDGE, x, ev)
                    .with(ROLE_ABSTRACTION, x)
                    .with_all(ROLE_REFINED_ABSTRACTION, &refined)
                    .with(ROLE_IMPLEMENTOR, t)
                    .with_all(ROLE_CONCRETE_IMPLEMENTOR, &impls)
            } else {
                ev.optional(EVIDENCE_INJECTED_DELEGATE, injected);
                PatternMatch::new(PATTERN_STRATEGY, x, ev)
                    .with(ROLE_CONTEXT, x)
                    .with(ROLE_STRATEGY, t)
                    .with_all(ROLE_CONCRETE_STRATEGY, &impls)
            };
            out.push(pm);
        }
        out
    }
}
