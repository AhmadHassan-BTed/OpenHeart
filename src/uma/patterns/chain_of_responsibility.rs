//! Chain of Responsibility pattern rule (§9.2.5).
//!
//! A handler abstraction holds a successor of its own type; handlers forward a
//! request to the successor when they do not handle it themselves.
//!
//! Anchor: the Handler (the type declaring the successor attribute). Mandatory:
//! (1) a non-static, non-collection attribute typed by the anchor itself; (2) an
//! operation of the anchor or of a subtype invokes the same operation through
//! that attribute; (3) a subtype of the anchor overrides that operation (several
//! handler kinds exist). A self-referencing data node without handler subtypes
//! (e.g. a linked-list `Node next`) fails (3). Optional: ≥2 concrete handlers;
//! the successor is assigned from a parameter.

use crate::uma::patterns::chain::{Evidence, PatternDetectionRule, PatternMatch};
use crate::uma::patterns::query::PatternContext;
use crate::uma::types::*;

pub struct ChainOfResponsibilityRule;

impl PatternDetectionRule for ChainOfResponsibilityRule {
    fn name(&self) -> &'static str {
        "ChainOfResponsibility"
    }

    fn pattern_kind(&self) -> u8 {
        PATTERN_CHAIN_OF_RESPONSIBILITY
    }

    fn evaluate(&self, h: u32, ctx: &PatternContext<'_>) -> Vec<PatternMatch> {
        if !ctx.is_user_type(h) || ctx.is_interface(h) {
            return Vec::new();
        }
        let successor = match ctx
            .fields(h)
            .iter()
            .find(|f| !f.is_static && !f.is_collection && f.type_id == h)
        {
            Some(f) => f,
            None => return Vec::new(),
        };
        let mut ev = Evidence::new();
        ev.require(EVIDENCE_SUCCESSOR_FIELD, true);

        let subtypes = ctx.descendants(h);
        let mut handled_ops: Vec<u32> = Vec::new();
        let mut owners = vec![h];
        owners.extend(subtypes.iter().copied());
        for &owner in &owners {
            for m in ctx.methods(owner) {
                if ctx.forwards_through(m, successor.name, h, true) {
                    let base = if owner == h {
                        Some(m)
                    } else {
                        ctx.index.same_signature_in(m, h)
                    };
                    if let Some(b) = base {
                        if !handled_ops.contains(&b) {
                            handled_ops.push(b);
                        }
                    }
                }
            }
        }
        if !ev.require(EVIDENCE_FORWARDING_CALL, !handled_ops.is_empty()) {
            return Vec::new();
        }
        let handlers: Vec<u32> = subtypes
            .iter()
            .copied()
            .filter(|&s| {
                handled_ops
                    .iter()
                    .any(|&op| ctx.index.same_signature_in(op, s).is_some())
            })
            .collect();
        if !ev.require(EVIDENCE_OVERRIDE, !handlers.is_empty()) {
            return Vec::new();
        }
        let concrete: Vec<u32> = handlers
            .iter()
            .copied()
            .filter(|&s| ctx.is_concrete_class(s))
            .collect();
        ev.optional(EVIDENCE_MULTIPLE_IMPLEMENTORS, concrete.len() >= 2);
        let linked = ctx
            .methods(h)
            .into_iter()
            .chain(ctx.constructors(h))
            .any(|m| ctx.param_types(m).contains(&h) && ctx.assigns_from_name(m, successor.name));
        ev.optional(EVIDENCE_INJECTED_DELEGATE, linked);

        vec![PatternMatch::new(PATTERN_CHAIN_OF_RESPONSIBILITY, h, ev)
            .with(ROLE_HANDLER, h)
            .with_all(ROLE_CONCRETE_HANDLER, &handlers)]
    }
}
