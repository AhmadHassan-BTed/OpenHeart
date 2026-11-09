//! Composite pattern rule (§9.2.5).
//!
//! A composite realizes the component abstraction, holds a collection of
//! components and implements component operations by iterating its children.
//!
//! Anchor: the Composite. Mandatory: (1) a user abstract supertype C (the
//! Component); (2) a collection attribute whose element type is C, or whose
//! element type is unresolved while an operation takes a C parameter; (3) an
//! operation overriding an operation of C loops over that collection invoking an
//! operation of C's hierarchy. Optional: the element type is resolved to C; an
//! operation adds a C parameter to the collection; C has a leaf (a concrete
//! subtype outside the composite's line that holds no such collection).

use crate::uma::patterns::chain::{Evidence, PatternDetectionRule, PatternMatch};
use crate::uma::patterns::query::{FieldFact, PatternContext};
use crate::uma::types::*;

const INSERT_OPERATIONS: &[&[u8]] = &[b"add", b"addAll", b"addFirst", b"addLast", b"push", b"offer", b"put"];

pub struct CompositeRule;

impl PatternDetectionRule for CompositeRule {
    fn name(&self) -> &'static str {
        "Composite"
    }

    fn pattern_kind(&self) -> u8 {
        PATTERN_COMPOSITE
    }

    fn evaluate(&self, k: u32, ctx: &PatternContext<'_>) -> Vec<PatternMatch> {
        let mut out = Vec::new();
        if !ctx.is_user_type(k) || ctx.is_interface(k) {
            return out;
        }
        let methods = ctx.methods(k);
        for c in ctx.user_ancestors(k) {
            if !ctx.is_abstract_type(c) {
                continue;
            }
            let takes_component = methods.iter().any(|&m| ctx.param_types(m).contains(&c));
            let children: Vec<&FieldFact> = ctx
                .fields(k)
                .iter()
                .filter(|f| {
                    f.is_collection && (f.element == c || (f.element == UNKNOWN_SYM && takes_component))
                })
                .collect();
            let mut ev = Evidence::new();
            ev.require(EVIDENCE_SUPERTYPE, true);
            if !ev.require(EVIDENCE_COLLECTION_FIELD, !children.is_empty()) {
                continue;
            }
            let dispatching = ctx.overrides_of(k, c).into_iter().any(|m| {
                children
                    .iter()
                    .any(|f| ctx.loops_over_and_calls(m, f.name, c))
            });
            if !ev.require(EVIDENCE_LOOP_DISPATCH, dispatching) {
                continue;
            }
            ev.optional(
                EVIDENCE_TYPED_COLLECTION,
                children.iter().any(|f| f.element == c),
            );
            let adds = methods.iter().any(|&m| {
                ctx.param_types(m).contains(&c)
                    && children
                        .iter()
                        .any(|f| ctx.invokes_on(m, f.name, INSERT_OPERATIONS))
            });
            ev.optional(EVIDENCE_ADDS_TO_COLLECTION, adds);
            let leaves: Vec<u32> = ctx
                .concrete_descendants(c)
                .into_iter()
                .filter(|&l| {
                    !ctx.related(l, k)
                        && !ctx
                            .fields(l)
                            .iter()
                            .any(|f| f.is_collection && (f.element == c || f.element == UNKNOWN_SYM))
                })
                .collect();
            ev.optional(EVIDENCE_CONCRETE_PARTICIPANTS, !leaves.is_empty());
            out.push(
                PatternMatch::new(PATTERN_COMPOSITE, k, ev)
                    .with(ROLE_COMPOSITE, k)
                    .with(ROLE_COMPONENT, c)
                    .with_all(ROLE_LEAF, &leaves),
            );
            // The nearest qualifying component abstraction defines the instance.
            break;
        }
        out
    }
}
