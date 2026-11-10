//! PatternDetector — runs the GoF pattern pipeline over all extracted classes
//! (§9.2.5, §9.3).
//!
//! One chain (`chain::PatternDetectionChain`) evaluates every rule per class.
//! Matches are normalized, de-duplicated, put through conflict resolution, and
//! emitted as `PatternInstanceRecord`s (pattern, roles, evidence, score) plus one
//! `DesignPatternRecord` row per participating type.

pub mod abstract_factory;
pub mod adapter;
pub mod builder;
pub mod chain;
pub mod chain_of_responsibility;
pub mod command;
pub mod composite;
pub mod decorator;
pub mod facade;
pub mod factory;
pub mod flyweight;
pub mod interpreter;
pub mod iterator;
pub mod mediator;
pub mod memento;
pub mod observer;
pub mod prototype;
pub mod proxy;
pub mod query;
pub mod singleton;
pub mod strategy;
pub mod template_method;
pub mod visitor;

pub use chain::{Evidence, PatternDetectionChain, PatternDetectionRule, PatternMatch};
pub use query::{FieldFact, PatternContext};

use std::collections::{BTreeMap, HashSet};

use crate::uma::types::*;

pub struct PatternDetector;

impl PatternDetector {
    /// Detect pattern instances anchored at the extracted classes, set each
    /// class's display stereotype, and return (participant rows, instances).
    pub fn detect_all(
        ctx: &PatternContext<'_>,
        classes: &mut [ClassRecord],
    ) -> (Vec<DesignPatternRecord>, Vec<PatternInstanceRecord>) {
        let chain = PatternDetectionChain::new();
        let mut matches: Vec<PatternMatch> = Vec::new();
        for class_rec in classes.iter() {
            matches.extend(chain.evaluate(class_rec.sym_id, ctx));
        }

        let matches = Self::deduplicate(matches);
        let mut matches = Self::resolve_conflicts(matches);
        matches.sort_by(|a, b| {
            (a.kind, a.primary, &a.roles_key()).cmp(&(b.kind, b.primary, &b.roles_key()))
        });

        let mut instances = Vec::with_capacity(matches.len());
        let mut rows = Vec::new();
        for (i, m) in matches.iter().enumerate() {
            let instance_id = match u32::try_from(i) {
                Ok(id) => id,
                Err(_) => break,
            };
            instances.push(PatternInstanceRecord {
                instance_id,
                pattern_kind: m.kind,
                score: m.score(),
                primary_sym: m.primary,
                evidence_mask: m.evidence.mask,
                roles: m.roles.clone(),
            });
            for b in &m.roles {
                if !ctx.is_user_type(b.sym_id) {
                    // Method-level roles (template method, primitive operations)
                    // live only in the instance record.
                    continue;
                }
                rows.push(DesignPatternRecord {
                    class_sym: b.sym_id,
                    pattern_kind: u16::from(m.kind),
                    confidence: u16::from(m.score()),
                    instance_id,
                    role: b.role,
                });
            }
        }

        // Display stereotype: the strongest instance a class participates in,
        // preferring instances it anchors; ties break on kind, then instance id.
        let mut best: BTreeMap<u32, (u8, bool, std::cmp::Reverse<u8>, std::cmp::Reverse<u32>)> =
            BTreeMap::new();
        for inst in &instances {
            for b in &inst.roles {
                let key = (
                    inst.score,
                    inst.primary_sym == b.sym_id,
                    std::cmp::Reverse(inst.pattern_kind),
                    std::cmp::Reverse(inst.instance_id),
                );
                let entry = best.entry(b.sym_id).or_insert(key);
                if key > *entry {
                    *entry = key;
                }
            }
        }
        for class_rec in classes.iter_mut() {
            class_rec.design_pattern = match best.get(&class_rec.sym_id) {
                Some(&(_, _, std::cmp::Reverse(kind), _)) => kind,
                None => PATTERN_NONE,
            };
        }

        (rows, instances)
    }

    /// Merge matches with identical (kind, role bindings), keeping the best score.
    fn deduplicate(matches: Vec<PatternMatch>) -> Vec<PatternMatch> {
        let mut by_key: BTreeMap<(u8, Vec<(u8, u32)>), PatternMatch> = BTreeMap::new();
        for mut m in matches {
            m.normalize();
            let key = (m.kind, m.roles_key());
            match by_key.get(&key) {
                Some(existing) if existing.score() >= m.score() => {}
                _ => {
                    by_key.insert(key, m);
                }
            }
        }
        by_key.into_values().collect()
    }

    /// Mutual-exclusion rules between structurally similar patterns:
    /// - a ConcreteCommand forwarding to its receiver is not also an Adapter;
    /// - a nonterminal expression is not also a Decorator or Composite;
    /// - an AbstractFactory's creation operations are not also Factory Methods.
    fn resolve_conflicts(matches: Vec<PatternMatch>) -> Vec<PatternMatch> {
        let mut concrete_commands: HashSet<u32> = HashSet::new();
        let mut nonterminals: HashSet<u32> = HashSet::new();
        let mut abstract_factories: HashSet<u32> = HashSet::new();
        for m in &matches {
            match m.kind {
                PATTERN_COMMAND => concrete_commands.extend(m.bound(ROLE_CONCRETE_COMMAND)),
                PATTERN_INTERPRETER => nonterminals.extend(m.bound(ROLE_NONTERMINAL_EXPRESSION)),
                PATTERN_ABSTRACT_FACTORY => {
                    abstract_factories.extend(m.bound(ROLE_ABSTRACT_FACTORY))
                }
                _ => {}
            }
        }
        matches
            .into_iter()
            .filter(|m| match m.kind {
                PATTERN_ADAPTER => !m
                    .bound(ROLE_ADAPTER)
                    .any(|s| concrete_commands.contains(&s)),
                PATTERN_DECORATOR => !m.bound(ROLE_DECORATOR).any(|s| nonterminals.contains(&s)),
                PATTERN_COMPOSITE => !m.bound(ROLE_COMPOSITE).any(|s| nonterminals.contains(&s)),
                PATTERN_FACTORY => !m
                    .bound(ROLE_CREATOR)
                    .any(|s| abstract_factories.contains(&s)),
                _ => true,
            })
            .collect()
    }
}

impl PatternMatch {
    fn roles_key(&self) -> Vec<(u8, u32)> {
        self.roles.iter().map(|b| (b.role, b.sym_id)).collect()
    }
}
