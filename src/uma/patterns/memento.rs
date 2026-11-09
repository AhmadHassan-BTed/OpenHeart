//! Memento pattern rule (§9.2.5).
//!
//! An originator captures its state in a memento object and later restores it;
//! a caretaker keeps mementos without inspecting them.
//!
//! Anchor: the Originator. Mandatory: (1) an operation whose result is a user type
//! K outside the originator's hierarchy, whose body allocates K, where K holds ≥1
//! attribute (the save step); (2) an operation taking a parameter typed K (the
//! restore step). Optional: the restore step assigns originator attributes; a
//! caretaker — another class — holds K (attribute or collection).

use crate::uma::patterns::chain::{Evidence, PatternDetectionRule, PatternMatch};
use crate::uma::patterns::query::PatternContext;
use crate::uma::types::*;

pub struct MementoRule;

impl PatternDetectionRule for MementoRule {
    fn name(&self) -> &'static str {
        "Memento"
    }

    fn pattern_kind(&self) -> u8 {
        PATTERN_MEMENTO
    }

    fn evaluate(&self, o: u32, ctx: &PatternContext<'_>) -> Vec<PatternMatch> {
        if !ctx.is_concrete_class(o) {
            return Vec::new();
        }
        let methods = ctx.methods(o);
        let own_fields: Vec<u32> = ctx.fields(o).iter().map(|f| f.name).collect();
        for &save in &methods {
            let k = ctx.return_type(save);
            if !ctx.is_user_type(k) || ctx.related(k, o) || ctx.fields(k).is_empty() {
                continue;
            }
            if !ctx.allocated_types(save).contains(&k) {
                continue;
            }
            let restores: Vec<u32> = methods
                .iter()
                .copied()
                .filter(|&m| m != save && ctx.param_types(m).contains(&k))
                .collect();
            let mut ev = Evidence::new();
            ev.require(EVIDENCE_ALLOCATES_SUBTYPE, true);
            if !ev.require(EVIDENCE_STATE_SAVE_RESTORE, !restores.is_empty()) {
                continue;
            }
            let restores_state = restores.iter().any(|&m| {
                ctx.body(m)
                    .map(|b| own_fields.iter().any(|&n| b.assigns(n)))
                    .unwrap_or(false)
            });
            ev.optional(EVIDENCE_ENCAPSULATED, restores_state);
            let caretakers: Vec<u32> = ctx
                .referrers(k)
                .iter()
                .copied()
                .filter(|&c| c != o && c != k)
                .collect();
            ev.optional(EVIDENCE_BACK_REFERENCE, !caretakers.is_empty());
            return vec![PatternMatch::new(PATTERN_MEMENTO, o, ev)
                .with(ROLE_ORIGINATOR, o)
                .with(ROLE_MEMENTO, k)
                .with_all(ROLE_CARETAKER, &caretakers)];
        }
        Vec::new()
    }
}
