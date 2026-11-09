//! Command pattern rule (§9.2.5).
//!
//! A command abstraction exposes a small operation set; concrete commands forward
//! the operation to a receiver attribute of an unrelated type; an invoker holds
//! commands and triggers the operation without knowing the receiver.
//!
//! Anchor: the Command abstraction. Mandatory: (1) the anchor is an abstract class
//! or an interface declaring 1–2 operations; (2) a concrete command's override of
//! a command operation invokes an operation through an attribute typed by a user
//! type outside the command hierarchy (the Receiver); (3) an invoker — a class
//! outside the command hierarchy holding the command (attribute or collection) —
//! invokes a command operation through that attribute. Optional: ≥2 concrete
//! commands delegate; ≥2 distinct receivers.

use std::collections::BTreeSet;

use crate::uma::patterns::chain::{Evidence, PatternDetectionRule, PatternMatch};
use crate::uma::patterns::query::PatternContext;
use crate::uma::types::*;

pub struct CommandRule;

impl PatternDetectionRule for CommandRule {
    fn name(&self) -> &'static str {
        "Command"
    }

    fn pattern_kind(&self) -> u8 {
        PATTERN_COMMAND
    }

    fn evaluate(&self, c: u32, ctx: &PatternContext<'_>) -> Vec<PatternMatch> {
        if !ctx.is_abstract_type(c) {
            return Vec::new();
        }
        let ops = ctx.methods(c);
        let mut ev = Evidence::new();
        if !ev.require(EVIDENCE_SUPERTYPE, !ops.is_empty() && ops.len() <= 2) {
            return Vec::new();
        }

        let mut commands = Vec::new();
        let mut receivers: BTreeSet<u32> = BTreeSet::new();
        for cc in ctx.concrete_descendants(c) {
            let overrides = ctx.overrides_of(cc, c);
            let mut delegated = false;
            for f in ctx.fields(cc) {
                let r = f.type_id;
                if f.is_static || f.is_collection || !ctx.is_user_type(r) || ctx.related(r, c) {
                    continue;
                }
                if overrides.iter().any(|&m| ctx.forwards_through(m, f.name, r, false)) {
                    receivers.insert(r);
                    delegated = true;
                }
            }
            if delegated {
                commands.push(cc);
            }
        }
        if !ev.require(EVIDENCE_FORWARDING_CALL, !commands.is_empty()) {
            return Vec::new();
        }

        let line = ctx.descendants(c);
        let mut invokers = Vec::new();
        for &holder in ctx.referrers(c) {
            if holder == c || line.contains(&holder) {
                continue;
            }
            let triggers = ctx.fields(holder).iter().any(|f| {
                if f.referenced_type() != c {
                    return false;
                }
                ctx.methods(holder).into_iter().any(|m| {
                    if f.is_collection {
                        ctx.loops_over_and_calls(m, f.name, c)
                    } else {
                        ctx.forwards_through(m, f.name, c, false)
                    }
                })
            });
            if triggers {
                invokers.push(holder);
            }
        }
        if !ev.require(EVIDENCE_INVOKER, !invokers.is_empty()) {
            return Vec::new();
        }
        ev.optional(EVIDENCE_MULTIPLE_IMPLEMENTORS, commands.len() >= 2);
        ev.optional(EVIDENCE_CONCRETE_PARTICIPANTS, receivers.len() >= 2);

        let receivers: Vec<u32> = receivers.into_iter().collect();
        vec![PatternMatch::new(PATTERN_COMMAND, c, ev)
            .with(ROLE_COMMAND, c)
            .with_all(ROLE_CONCRETE_COMMAND, &commands)
            .with_all(ROLE_RECEIVER, &receivers)
            .with_all(ROLE_INVOKER, &invokers)]
    }
}
