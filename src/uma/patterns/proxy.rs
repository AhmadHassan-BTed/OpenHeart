//! Proxy pattern rule (§9.2.5).
//!
//! A proxy realizes the subject abstraction and controls access to a concrete
//! real subject of the same abstraction, forwarding the same operations.
//!
//! Anchor: the Proxy. Mandatory: (1) a user abstract supertype S (the Subject);
//! (2) a non-static, non-collection attribute typed by a concrete user class R
//! (R ≠ proxy) that is a subtype of S; (3) an operation overriding an operation
//! of S invokes the same operation through that attribute. Optional: the proxy
//! allocates R itself (controlled or lazy creation); R is not injected through a
//! constructor parameter.

use crate::uma::patterns::chain::{Evidence, PatternDetectionRule, PatternMatch};
use crate::uma::patterns::query::PatternContext;
use crate::uma::types::*;

pub struct ProxyRule;

impl PatternDetectionRule for ProxyRule {
    fn name(&self) -> &'static str {
        "Proxy"
    }

    fn pattern_kind(&self) -> u8 {
        PATTERN_PROXY
    }

    fn evaluate(&self, p: u32, ctx: &PatternContext<'_>) -> Vec<PatternMatch> {
        if !ctx.is_user_type(p) || ctx.is_interface(p) {
            return Vec::new();
        }
        for s in ctx.user_ancestors(p) {
            if !ctx.is_abstract_type(s) {
                continue;
            }
            for f in ctx.fields(p) {
                let r = f.type_id;
                if f.is_static
                    || f.is_collection
                    || r == p
                    || !ctx.is_concrete_class(r)
                    || !ctx.is_subtype_of(r, s)
                    || ctx.related(r, p)
                {
                    continue;
                }
                let mut ev = Evidence::new();
                ev.require(EVIDENCE_SUPERTYPE, true);
                ev.require(EVIDENCE_WRAPPED_FIELD, true);
                let forwards = ctx
                    .overrides_of(p, s)
                    .into_iter()
                    .any(|m| ctx.forwards_through(m, f.name, s, true));
                if !ev.require(EVIDENCE_FORWARDING_CALL, forwards) {
                    continue;
                }
                let creates = ctx
                    .methods(p)
                    .into_iter()
                    .chain(ctx.constructors(p))
                    .any(|m| ctx.assigns_new(m, f.name).map(|t| ctx.is_subtype_of(t, r)).unwrap_or(false))
                    || ctx.initializer_allocates(f.sym).map(|t| ctx.is_subtype_of(t, r)).unwrap_or(false);
                ev.optional(EVIDENCE_ALLOCATES_SUBTYPE, creates);
                let injected = ctx.constructors(p).into_iter().any(|k| {
                    ctx.param_types(k).contains(&r) && ctx.assigns_from_name(k, f.name)
                });
                ev.optional(EVIDENCE_ENCAPSULATED, !injected);
                return vec![PatternMatch::new(PATTERN_PROXY, p, ev)
                    .with(ROLE_PROXY, p)
                    .with(ROLE_SUBJECT, s)
                    .with(ROLE_REAL_SUBJECT, r)];
            }
        }
        Vec::new()
    }
}
