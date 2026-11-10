//! Visitor pattern rule (§9.2.5).
//!
//! A visitor abstraction declares one visit operation per element type; each
//! element accepts a visitor and calls back the visit operation for itself
//! (double dispatch).
//!
//! Anchor: the Visitor abstraction. Mandatory: (1) the anchor is an abstract
//! class or an interface; (2) it declares ≥2 operations that each take exactly one
//! parameter, typed by ≥2 distinct user element types; (3) an element type (or one
//! of its subtypes) declares an operation with a parameter typed by the visitor
//! whose body invokes an operation of the visitor's hierarchy through that
//! parameter. Optional: every element type accepts; the visitor has ≥1 concrete
//! visitor.

use std::collections::BTreeSet;

use crate::uma::patterns::chain::{Evidence, PatternDetectionRule, PatternMatch};
use crate::uma::patterns::query::PatternContext;
use crate::uma::types::*;

pub struct VisitorRule;

impl PatternDetectionRule for VisitorRule {
    fn name(&self) -> &'static str {
        "Visitor"
    }

    fn pattern_kind(&self) -> u8 {
        PATTERN_VISITOR
    }

    fn evaluate(&self, v: u32, ctx: &PatternContext<'_>) -> Vec<PatternMatch> {
        if !ctx.is_abstract_type(v) {
            return Vec::new();
        }
        let mut elements: BTreeSet<u32> = BTreeSet::new();
        for m in ctx.methods(v) {
            let params = ctx.param_types(m);
            if params.len() == 1 && ctx.is_user_type(params[0]) && !ctx.related(params[0], v) {
                elements.insert(params[0]);
            }
        }
        let mut ev = Evidence::new();
        if !ev.require(EVIDENCE_SUPERTYPE, elements.len() >= 2) {
            return Vec::new();
        }

        let mut accepting: Vec<u32> = Vec::new();
        let mut accept_declarers: BTreeSet<u32> = BTreeSet::new();
        for &e in &elements {
            let mut candidates = vec![e];
            candidates.extend(ctx.user_ancestors(e));
            let accepts = candidates.iter().any(|&owner| {
                let mut line = vec![owner];
                line.extend(ctx.descendants(owner));
                line.iter().any(|&cls| {
                    ctx.methods(cls)
                        .into_iter()
                        .any(|m| match ctx.param_named_by_type(m, v) {
                            Some(pname) => {
                                let dispatches = ctx
                                    .calls_through(m, pname)
                                    .iter()
                                    .any(|c| ctx.targets_hierarchy(c.method_text, &c.callees, v));
                                if dispatches {
                                    accept_declarers.insert(owner);
                                }
                                dispatches
                            }
                            None => false,
                        })
                })
            });
            if accepts {
                accepting.push(e);
            }
        }
        if !ev.require(EVIDENCE_DOUBLE_DISPATCH, !accepting.is_empty()) {
            return Vec::new();
        }
        ev.optional(EVIDENCE_TYPED_COLLECTION, accepting.len() == elements.len());
        let concrete_visitors = ctx.concrete_descendants(v);
        ev.optional(
            EVIDENCE_CONCRETE_PARTICIPANTS,
            !concrete_visitors.is_empty(),
        );

        let element_list: Vec<u32> = elements.into_iter().collect();
        let abstract_elements: Vec<u32> = accept_declarers
            .into_iter()
            .filter(|&a| ctx.is_abstract_type(a) && !element_list.contains(&a))
            .collect();
        vec![PatternMatch::new(PATTERN_VISITOR, v, ev)
            .with(ROLE_VISITOR, v)
            .with_all(ROLE_CONCRETE_VISITOR, &concrete_visitors)
            .with_all(ROLE_ELEMENT, &abstract_elements)
            .with_all(ROLE_CONCRETE_ELEMENT, &element_list)]
    }
}
