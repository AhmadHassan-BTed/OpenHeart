//! Facade pattern rule (§9.2.5).
//!
//! A facade holds references to several subsystem classes and implements its
//! public operations by calling into them, hiding them from clients.
//!
//! Anchor: the Facade. Mandatory: (1) non-static, non-collection attributes typed
//! by ≥2 distinct user types outside the facade's own hierarchy (subsystems);
//! (2) the facade's public operations invoke operations through attributes of
//! ≥2 distinct subsystem types. Optional: some subsystem lives in another
//! package; ≥3 subsystems are used; no public operation returns a subsystem type
//! (the subsystem stays hidden); no subsystem calls back into the facade.
//! Primitive, library and unresolved attribute types never count as subsystems.

use std::collections::BTreeSet;

use crate::uma::patterns::chain::{Evidence, PatternDetectionRule, PatternMatch};
use crate::uma::patterns::query::{FieldFact, PatternContext};
use crate::uma::types::*;

pub struct FacadeRule;

impl PatternDetectionRule for FacadeRule {
    fn name(&self) -> &'static str {
        "Facade"
    }

    fn pattern_kind(&self) -> u8 {
        PATTERN_FACADE
    }

    fn evaluate(&self, fa: u32, ctx: &PatternContext<'_>) -> Vec<PatternMatch> {
        if !ctx.is_concrete_class(fa) {
            return Vec::new();
        }
        let subsystem_fields: Vec<&FieldFact> = ctx
            .fields(fa)
            .iter()
            .filter(|f| {
                !f.is_static
                    && !f.is_collection
                    && ctx.is_user_type(f.type_id)
                    && !ctx.related(f.type_id, fa)
            })
            .collect();
        let declared: BTreeSet<u32> = subsystem_fields.iter().map(|f| f.type_id).collect();
        let mut ev = Evidence::new();
        if !ev.require(EVIDENCE_WRAPPED_FIELD, declared.len() >= 2) {
            return Vec::new();
        }

        let public_ops: Vec<u32> = ctx
            .methods(fa)
            .into_iter()
            .filter(|&m| ctx.is_public(m) && !ctx.is_static(m))
            .collect();
        let mut used: BTreeSet<u32> = BTreeSet::new();
        for &m in &public_ops {
            for f in &subsystem_fields {
                if ctx.forwards_through(m, f.name, f.type_id, false) {
                    used.insert(f.type_id);
                }
            }
        }
        if !ev.require(EVIDENCE_SUBSYSTEM_CALLS, used.len() >= 2) {
            return Vec::new();
        }

        let own_pkg = ctx.index.package_name_of(fa);
        let foreign = used
            .iter()
            .any(|&t| ctx.index.package_name_of(t) != own_pkg);
        ev.optional(EVIDENCE_FOREIGN_PACKAGE, foreign);
        ev.optional(EVIDENCE_MULTIPLE_IMPLEMENTORS, used.len() >= 3);
        let hidden = public_ops
            .iter()
            .all(|&m| !used.contains(&ctx.return_type(m)));
        ev.optional(EVIDENCE_ENCAPSULATED, hidden);
        let no_callbacks = used.iter().all(|&t| {
            ctx.methods(t).into_iter().all(|m| {
                ctx.calls_of(m)
                    .iter()
                    .all(|c| ctx.index.enclosing_type(c.callee) != fa)
            })
        });
        ev.optional(EVIDENCE_BACK_REFERENCE, no_callbacks);

        let subsystems: Vec<u32> = used.into_iter().collect();
        vec![PatternMatch::new(PATTERN_FACADE, fa, ev)
            .with(ROLE_FACADE, fa)
            .with_all(ROLE_SUBSYSTEM, &subsystems)]
    }
}
