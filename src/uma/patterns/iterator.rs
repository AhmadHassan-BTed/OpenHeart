//! Iterator pattern rule (§9.2.5).
//!
//! An aggregate creates a concrete iterator that traverses the aggregate's
//! elements through an iteration protocol.
//!
//! Anchor: the ConcreteIterator. Mandatory: (1) the class realizes an iteration
//! protocol — a supertype from the JDK/Kotlin iteration API (`Iterator`,
//! `ListIterator`, `Enumeration`, `Spliterator`), or a user abstraction declaring
//! a parameterless boolean operation and a parameterless value-returning
//! operation; (2) a member of another user class (the Aggregate) allocates it.
//! Optional: the iterator holds an attribute typed by the aggregate, or receives
//! the aggregate through a constructor parameter.

use crate::uma::index::UMAIndex;
use crate::uma::patterns::chain::{Evidence, PatternDetectionRule, PatternMatch};
use crate::uma::patterns::query::PatternContext;
use crate::uma::types::*;

/// Iteration protocol types of the JDK and Kotlin standard libraries.
const LIBRARY_ITERATION_PROTOCOLS: &[&[u8]] = &[
    b"Iterator",
    b"ListIterator",
    b"Enumeration",
    b"Spliterator",
    b"MutableIterator",
    b"PrimitiveIterator",
];

pub struct IteratorRule;

impl PatternDetectionRule for IteratorRule {
    fn name(&self) -> &'static str {
        "Iterator"
    }

    fn pattern_kind(&self) -> u8 {
        PATTERN_ITERATOR
    }

    fn evaluate(&self, it: u32, ctx: &PatternContext<'_>) -> Vec<PatternMatch> {
        if !ctx.is_concrete_class(it) {
            return Vec::new();
        }
        let library_protocol = ctx.index.ancestors(it).into_iter().any(|a| {
            ctx.index.is_library_type(a)
                && LIBRARY_ITERATION_PROTOCOLS.contains(&ctx.index.name_bytes(a))
        });
        let user_protocol = ctx
            .user_ancestors(it)
            .into_iter()
            .find(|&a| ctx.is_abstract_type(a) && declares_iteration_protocol(a, ctx));
        let mut ev = Evidence::new();
        if !ev.require(
            EVIDENCE_ITERATION_PROTOCOL,
            library_protocol || user_protocol.is_some(),
        ) {
            return Vec::new();
        }

        let aggregates: Vec<u32> = {
            let mut v: Vec<u32> = ctx
                .allocators(it)
                .iter()
                .map(|&m| ctx.index.enclosing_type(m))
                .filter(|&owner| owner != UNKNOWN_SYM && owner != it && !ctx.related(owner, it))
                .collect();
            v.sort_unstable();
            v.dedup();
            v
        };
        if !ev.require(EVIDENCE_ALLOCATES_SUBTYPE, !aggregates.is_empty()) {
            return Vec::new();
        }
        let back_reference = aggregates.iter().any(|&ag| {
            ctx.fields(it).iter().any(|f| f.type_id == ag)
                || ctx
                    .constructors(it)
                    .into_iter()
                    .any(|k| ctx.param_types(k).contains(&ag))
        });
        ev.optional(EVIDENCE_BACK_REFERENCE, back_reference);

        let mut pm = PatternMatch::new(PATTERN_ITERATOR, it, ev)
            .with(ROLE_CONCRETE_ITERATOR, it)
            .with_all(ROLE_AGGREGATE, &aggregates);
        if let Some(p) = user_protocol {
            pm.bind(ROLE_ITERATOR, p);
        }
        vec![pm]
    }
}

fn declares_iteration_protocol(t: u32, ctx: &PatternContext<'_>) -> bool {
    let methods = ctx.methods(t);
    let has_test = methods
        .iter()
        .any(|&m| ctx.param_count(m) == 0 && UMAIndex::is_boolean(ctx.return_type(m)));
    let has_next = methods.iter().any(|&m| {
        let r = ctx.return_type(m);
        ctx.param_count(m) == 0
            && r != UNKNOWN_SYM
            && !UMAIndex::is_void(r)
            && !UMAIndex::is_boolean(r)
    });
    has_test && has_next
}
