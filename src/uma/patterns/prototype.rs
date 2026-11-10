//! Prototype pattern rule (§9.2.5).
//!
//! Concrete prototypes implement a polymorphic copy operation by allocating a
//! copy of themselves or delegating to `super.clone()`.
//!
//! Anchor: the ConcretePrototype. Mandatory: (1) a non-static operation without
//! parameters whose result type is the class or one of its supertypes; (2) its
//! body allocates the class or invokes `super.clone()`; (3) the copy operation is
//! polymorphic — it overrides an operation declared in a supertype, or the class
//! realizes the JDK `Cloneable` protocol. Optional: the overridden declaration
//! sits in an abstract user type (the Prototype role).

use crate::uma::patterns::chain::{Evidence, PatternDetectionRule, PatternMatch};
use crate::uma::patterns::query::PatternContext;
use crate::uma::source::Receiver;
use crate::uma::types::*;

pub struct PrototypeRule;

impl PatternDetectionRule for PrototypeRule {
    fn name(&self) -> &'static str {
        "Prototype"
    }

    fn pattern_kind(&self) -> u8 {
        PATTERN_PROTOTYPE
    }

    fn evaluate(&self, c: u32, ctx: &PatternContext<'_>) -> Vec<PatternMatch> {
        if !ctx.is_concrete_class(c) {
            return Vec::new();
        }
        let ancestors = ctx.index.ancestors(c);
        let cloneable = ctx
            .ancestor_names(c)
            .iter()
            .any(|n| *n == &b"Cloneable"[..]);
        let scanner = &ctx.index.scanner;

        for m in ctx.methods(c) {
            if ctx.is_static(m) || ctx.param_count(m) != 0 {
                continue;
            }
            let ret = ctx.return_type(m);
            let mut ev = Evidence::new();
            if !ev.require(EVIDENCE_COPY_METHOD, ret == c || ancestors.contains(&ret)) {
                continue;
            }
            let body = match ctx.body(m) {
                Some(b) => b,
                None => continue,
            };
            let copies_self = ctx.allocated_types(m).contains(&c)
                || body.invocations.iter().any(|inv| {
                    inv.receiver == Receiver::Super && scanner.text(inv.name_token) == b"clone"
                });
            if !ev.require(EVIDENCE_ALLOCATES_SUBTYPE, copies_self) {
                continue;
            }
            let declared_in = ctx
                .user_ancestors(c)
                .into_iter()
                .find(|&a| ctx.index.same_signature_in(m, a).is_some());
            if !ev.require(EVIDENCE_OVERRIDE, declared_in.is_some() || cloneable) {
                continue;
            }
            ev.optional(
                EVIDENCE_SUPERTYPE,
                declared_in
                    .map(|a| ctx.is_abstract_type(a))
                    .unwrap_or(false),
            );
            let mut pm =
                PatternMatch::new(PATTERN_PROTOTYPE, c, ev).with(ROLE_CONCRETE_PROTOTYPE, c);
            if let Some(a) = declared_in {
                pm.bind(ROLE_PROTOTYPE, a);
            }
            return vec![pm];
        }
        Vec::new()
    }
}
