//! Observer pattern rule (§9.2.5).
//!
//! A subject keeps a collection of observers typed by an abstraction, registers
//! them through an operation taking that abstraction, and notifies them by
//! invoking one of its operations in a loop over the collection.
//!
//! Anchor: the Subject. Mandatory: (1) a collection attribute that can hold the
//! observer type O (its element type is O or unresolved); (2) a registration
//! operation with a parameter typed by the abstract user type O; (3) an
//! operation that loops over that collection invoking an operation of O's
//! hierarchy. Optional: the element type is resolved to O; the registration
//! operation adds its parameter to the collection; O has concrete observers.

use std::collections::BTreeSet;

use crate::uma::patterns::chain::{Evidence, PatternDetectionRule, PatternMatch};
use crate::uma::patterns::query::{FieldFact, PatternContext};
use crate::uma::types::*;

/// Library collection insertion operations (JDK / Kotlin collections API).
const INSERT_OPERATIONS: &[&[u8]] = &[
    b"add",
    b"addAll",
    b"addElement",
    b"addFirst",
    b"addLast",
    b"push",
    b"offer",
    b"put",
    b"putIfAbsent",
];

pub struct ObserverRule;

impl PatternDetectionRule for ObserverRule {
    fn name(&self) -> &'static str {
        "Observer"
    }

    fn pattern_kind(&self) -> u8 {
        PATTERN_OBSERVER
    }

    fn evaluate(&self, s: u32, ctx: &PatternContext<'_>) -> Vec<PatternMatch> {
        let mut out = Vec::new();
        if !ctx.is_user_type(s) || ctx.is_interface(s) {
            return out;
        }
        let collections: Vec<&FieldFact> =
            ctx.fields(s).iter().filter(|f| f.is_collection).collect();
        if collections.is_empty() {
            return out;
        }
        let methods = ctx.methods(s);
        let mut observer_types: BTreeSet<u32> = BTreeSet::new();
        for &m in &methods {
            for p in ctx.param_types(m) {
                if ctx.is_abstract_type(p) && p != s && !ctx.related(s, p) {
                    observer_types.insert(p);
                }
            }
        }

        for o in observer_types {
            let holders: Vec<&FieldFact> = collections
                .iter()
                .copied()
                .filter(|f| f.element == o || f.element == UNKNOWN_SYM)
                .collect();
            let mut ev = Evidence::new();
            if !ev.require(EVIDENCE_COLLECTION_FIELD, !holders.is_empty()) {
                continue;
            }
            let registrations: Vec<u32> = methods
                .iter()
                .copied()
                .filter(|&m| ctx.param_types(m).contains(&o))
                .collect();
            if !ev.require(EVIDENCE_REGISTRATION_METHOD, !registrations.is_empty()) {
                continue;
            }
            let notifies = methods
                .iter()
                .any(|&m| holders.iter().any(|f| ctx.loops_over_and_calls(m, f.name, o)));
            if !ev.require(EVIDENCE_LOOP_DISPATCH, notifies) {
                continue;
            }
            ev.optional(
                EVIDENCE_TYPED_COLLECTION,
                holders.iter().any(|f| f.element == o),
            );
            let adds = registrations.iter().any(|&m| {
                holders
                    .iter()
                    .any(|f| ctx.invokes_on(m, f.name, INSERT_OPERATIONS))
            });
            ev.optional(EVIDENCE_ADDS_TO_COLLECTION, adds);
            let concrete = ctx.concrete_descendants(o);
            ev.optional(EVIDENCE_CONCRETE_PARTICIPANTS, !concrete.is_empty());

            out.push(
                PatternMatch::new(PATTERN_OBSERVER, s, ev)
                    .with(ROLE_SUBJECT, s)
                    .with(ROLE_OBSERVER, o)
                    .with_all(ROLE_CONCRETE_OBSERVER, &concrete),
            );
        }
        out
    }
}
