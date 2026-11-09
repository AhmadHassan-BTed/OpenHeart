//! Flyweight pattern rule (§9.2.5).
//!
//! A flyweight factory keeps a keyed pool of shared flyweights and allocates a
//! new flyweight only when the pool lookup misses.
//!
//! Anchor: the FlyweightFactory. Mandatory: (1) a library collection attribute
//! (the pool); (2) an operation with ≥1 parameter whose result is a user type F,
//! that both looks the pool up (`get`, `containsKey`, `computeIfAbsent`,
//! `putIfAbsent`, `getOrPut`) and allocates F or a subtype. Optional: the pool's
//! element type resolves to F; the operation stores into the pool.

use crate::uma::patterns::chain::{Evidence, PatternDetectionRule, PatternMatch};
use crate::uma::patterns::query::{FieldFact, PatternContext};
use crate::uma::types::*;

/// JDK `Map` / Kotlin `MutableMap` lookup operations.
const POOL_LOOKUPS: &[&[u8]] = &[
    b"get",
    b"containsKey",
    b"computeIfAbsent",
    b"putIfAbsent",
    b"getOrDefault",
    b"getOrPut",
];
/// JDK `Map` / Kotlin `MutableMap` store operations.
const POOL_STORES: &[&[u8]] = &[b"put", b"computeIfAbsent", b"putIfAbsent", b"getOrPut"];

pub struct FlyweightRule;

impl PatternDetectionRule for FlyweightRule {
    fn name(&self) -> &'static str {
        "Flyweight"
    }

    fn pattern_kind(&self) -> u8 {
        PATTERN_FLYWEIGHT
    }

    fn evaluate(&self, ff: u32, ctx: &PatternContext<'_>) -> Vec<PatternMatch> {
        if !ctx.is_user_type(ff) || ctx.is_interface(ff) {
            return Vec::new();
        }
        let pools: Vec<&FieldFact> = ctx
            .fields(ff)
            .iter()
            .filter(|f| f.is_collection && ctx.index.is_collection_type(f.type_id))
            .collect();
        let mut ev = Evidence::new();
        if !ev.require(EVIDENCE_COLLECTION_FIELD, !pools.is_empty()) {
            return Vec::new();
        }
        for m in ctx.methods(ff) {
            if ctx.param_count(m) == 0 {
                continue;
            }
            let fw = ctx.return_type(m);
            if !ctx.is_user_type(fw) || fw == ff {
                continue;
            }
            let pool = pools
                .iter()
                .find(|p| ctx.invokes_on(m, p.name, POOL_LOOKUPS));
            let pool = match pool {
                Some(p) => *p,
                None => continue,
            };
            let made: Vec<u32> = ctx
                .allocated_types(m)
                .into_iter()
                .filter(|&t| ctx.is_subtype_of(t, fw))
                .collect();
            if made.is_empty() {
                continue;
            }
            let mut ev = ev.clone();
            ev.require(EVIDENCE_CACHE_LOOKUP, true);
            ev.optional(EVIDENCE_TYPED_COLLECTION, pool.element == fw);
            ev.optional(
                EVIDENCE_ADDS_TO_COLLECTION,
                ctx.invokes_on(m, pool.name, POOL_STORES),
            );
            return vec![PatternMatch::new(PATTERN_FLYWEIGHT, ff, ev)
                .with(ROLE_FLYWEIGHT_FACTORY, ff)
                .with(ROLE_FLYWEIGHT, fw)
                .with_all(ROLE_CONCRETE_FLYWEIGHT, &made)];
        }
        Vec::new()
    }
}
