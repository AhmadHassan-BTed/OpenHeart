//! Builder pattern rule (§9.2.5).
//!
//! A builder accumulates construction state through operations that return the
//! builder itself and assembles a separate product in a build step.
//!
//! Anchor: the Builder. Mandatory: (1) ≥2 non-static operations whose result type
//! is the builder and whose bodies return `this` (fluent steps); (2) an operation
//! whose result is a distinct user type P and whose body allocates P (the build
//! step). Optional: fluent steps assign builder attributes; the builder is
//! nested in P or P declares a constructor taking the builder.

use crate::uma::patterns::chain::{Evidence, PatternDetectionRule, PatternMatch};
use crate::uma::patterns::query::PatternContext;
use crate::uma::types::*;

pub struct BuilderRule;

impl PatternDetectionRule for BuilderRule {
    fn name(&self) -> &'static str {
        "Builder"
    }

    fn pattern_kind(&self) -> u8 {
        PATTERN_BUILDER
    }

    fn evaluate(&self, b: u32, ctx: &PatternContext<'_>) -> Vec<PatternMatch> {
        if !ctx.is_user_type(b) || ctx.is_interface(b) {
            return Vec::new();
        }
        let methods = ctx.methods(b);
        let fluent: Vec<u32> = methods
            .iter()
            .copied()
            .filter(|&m| {
                !ctx.is_static(m)
                    && ctx.return_type(m) == b
                    && ctx.body(m).map(|body| body.returns_this()).unwrap_or(false)
            })
            .collect();
        let mut ev = Evidence::new();
        if !ev.require(EVIDENCE_FLUENT_RETURNS, fluent.len() >= 2) {
            return Vec::new();
        }

        let build_step = methods.iter().copied().find_map(|m| {
            let p = ctx.return_type(m);
            if p == b || !ctx.is_user_type(p) || ctx.related(p, b) {
                return None;
            }
            let made = ctx.allocated_types(m);
            if made.iter().any(|&t| t == p || ctx.is_subtype_of(t, p)) {
                Some(p)
            } else {
                None
            }
        });
        let product = match build_step {
            Some(p) => p,
            None => {
                ev.require(EVIDENCE_BUILD_METHOD, false);
                return Vec::new();
            }
        };
        ev.require(EVIDENCE_BUILD_METHOD, true);

        let own_fields: Vec<u32> = ctx.fields(b).iter().map(|f| f.name).collect();
        let assigns_state = fluent.iter().any(|&m| {
            ctx.body(m)
                .map(|body| own_fields.iter().any(|&n| body.assigns(n)))
                .unwrap_or(false)
        });
        ev.optional(EVIDENCE_ENCAPSULATED, assigns_state);
        let coupled = ctx.index.enclosing_type(b) == product
            || ctx
                .constructors(product)
                .into_iter()
                .any(|k| ctx.param_types(k).contains(&b));
        ev.optional(EVIDENCE_BACK_REFERENCE, coupled);

        vec![PatternMatch::new(PATTERN_BUILDER, b, ev)
            .with(ROLE_BUILDER, b)
            .with(ROLE_PRODUCT, product)]
    }
}
