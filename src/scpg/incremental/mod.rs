//! IncrementalEngine & RebuildPlanner — incremental update planning and reconciliation (§10.6.2).
//!
//! The protocol has three steps:
//! 1. **Plan** (`RebuildPlanner::plan`): a `SourceDelta` derived from real file contents is
//!    classified against the current snapshot (tokens, entity spans, callable bodies). The
//!    plan lists the scope, the phases to re-run, the dirty callables/declarations and the
//!    inter-procedural dependents whose summaries must be refreshed. The edits convert to
//!    `tree_sitter::InputEdit` for `Tree::edit` + incremental reparse in Phase 1.
//! 2. **Rebuild**: the pipeline recomputes the planned phases and produces a new snapshot.
//! 3. **Commit** (`IncrementalEngine::commit`): the new snapshot is diffed against the old
//!    one entity by entity, keyed by *stable ids* (not positional ids), using
//!    position-independent signature and body fingerprints. Entities whose fingerprints are
//!    unchanged keep their `changed_at` revision (backdating / early cutoff, as in salsa's
//!    red-green algorithm); only changed entities and moved spans make UMLLinks stale. The
//!    new snapshot hash is the SHA-256 content digest of the rebuilt snapshot, so equal
//!    content always yields equal hashes and different content never reuses a cache key.

pub mod classify;
pub mod delta;

pub use classify::DeltaClassifier;
pub use delta::{
    EditClass, EditClassification, RebuildPlan, RebuildScope, SourceDelta, TextEdit, TextPoint,
    UpdateResult,
};

use std::collections::HashMap;
use std::sync::Arc;

use serde::Serialize;

use crate::scpg::query::impact::{ImpactAnalyzer, ImpactOptions};
use crate::scpg::query::snapshot::ScpgSnapshot;
use crate::scpg::serializer::integrity::digest_hex;

pub struct RebuildPlanner;

impl RebuildPlanner {
    /// Plans the rebuild of one file delta against `snapshot`.
    pub fn plan(snapshot: &ScpgSnapshot, delta: &SourceDelta) -> RebuildPlan {
        let edits = DeltaClassifier::new(snapshot).classify(delta);
        let scope = edits
            .iter()
            .map(|e| e.scope)
            .max()
            .unwrap_or(RebuildScope::None);

        let mut dirty_methods = Vec::new();
        let mut dirty_declarations = Vec::new();
        for e in &edits {
            match &e.class {
                EditClass::Body { methods } => dirty_methods.extend(methods.iter().copied()),
                EditClass::Declaration { entity } => dirty_declarations.push(*entity),
                EditClass::Trivia | EditClass::OutsideDeclarations => {}
            }
        }
        dirty_methods.sort_unstable();
        dirty_methods.dedup();
        dirty_declarations.sort_unstable();
        dirty_declarations.dedup();

        let mut affected = Vec::new();
        for &m in &dirty_methods {
            affected.extend(
                snapshot
                    .call_graph
                    .transitive_callers(m, None)
                    .into_iter()
                    .map(|(c, _)| c),
            );
        }
        for &d in &dirty_declarations {
            if let Some(entries) = ImpactAnalyzer::analyze(
                &snapshot.symbols,
                &snapshot.hierarchy,
                &snapshot.call_graph,
                d,
                ImpactOptions::default(),
            ) {
                affected.extend(entries.into_iter().map(|e| e.sym_id).filter(|&s| s != d));
            }
        }
        affected.sort_unstable();
        affected.dedup();
        affected.retain(|s| {
            dirty_methods.binary_search(s).is_err() && dirty_declarations.binary_search(s).is_err()
        });

        RebuildPlan {
            file_id: delta.file_id,
            scope,
            phases: scope.phases().to_vec(),
            edits,
            dirty_methods,
            dirty_declarations,
            affected,
        }
    }
}

/// Entity identity across builds: stable id plus occurrence index among equal stable ids.
type EntityKey = (String, u32);

fn entity_keys(snapshot: &ScpgSnapshot) -> HashMap<EntityKey, u32> {
    let mut seen: HashMap<String, u32> = HashMap::new();
    let mut keys = HashMap::with_capacity(snapshot.symbols.len());
    for sym in 0..snapshot.symbols.len() as u32 {
        let stable = snapshot.symbols.stable_id(sym).unwrap_or("").to_string();
        let occurrence = seen.entry(stable.clone()).or_insert(0);
        keys.insert((stable, *occurrence), sym);
        *occurrence += 1;
    }
    keys
}

/// Entity-level difference between two snapshots.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SnapshotDiff {
    pub added: Vec<(String, u32)>,
    pub removed: Vec<(String, u32)>,
    pub signature_changed: Vec<(String, u32)>,
    pub body_changed: Vec<(String, u32)>,
    pub moved: Vec<(String, u32)>,
    pub unchanged: Vec<(String, u32)>,
}

impl SnapshotDiff {
    /// Diffs `old` and `new` by stable id. Tuples are `(stable_id, sym_id)`, with the
    /// `sym_id` taken from the old snapshot for `removed` and from the new one otherwise.
    pub fn compute(old: &ScpgSnapshot, new: &ScpgSnapshot) -> Self {
        let old_keys = entity_keys(old);
        let new_keys = entity_keys(new);
        let mut diff = SnapshotDiff::default();

        let mut new_sorted: Vec<(&EntityKey, &u32)> = new_keys.iter().collect();
        new_sorted.sort_by_key(|&(_, &id)| id);
        for (key, &new_id) in new_sorted {
            let entry = (key.0.clone(), new_id);
            let Some(&old_id) = old_keys.get(key) else {
                diff.added.push(entry);
                continue;
            };
            let sig_changed = old.symbols.signature_fingerprint(old_id)
                != new.symbols.signature_fingerprint(new_id);
            let body_changed =
                old.symbols.body_fingerprint(old_id) != new.symbols.body_fingerprint(new_id);
            if sig_changed {
                diff.signature_changed.push(entry);
            } else if body_changed {
                diff.body_changed.push(entry);
            } else {
                let old_span = old.navigation.to_source(old_id).map(|s| s.range());
                let new_span = new.navigation.to_source(new_id).map(|s| s.range());
                if old_span != new_span {
                    diff.moved.push(entry);
                } else {
                    diff.unchanged.push(entry);
                }
            }
        }
        let mut removed: Vec<(String, u32)> = old_keys
            .iter()
            .filter(|(key, _)| !new_keys.contains_key(*key))
            .map(|(key, &id)| (key.0.clone(), id))
            .collect();
        removed.sort_by_key(|(_, id)| *id);
        diff.removed = removed;
        diff
    }

    pub fn scope(&self) -> RebuildScope {
        if !self.added.is_empty() || !self.removed.is_empty() || !self.signature_changed.is_empty()
        {
            RebuildScope::Structural
        } else if !self.body_changed.is_empty() {
            RebuildScope::Behavioral
        } else if !self.moved.is_empty() {
            RebuildScope::Positional
        } else {
            RebuildScope::None
        }
    }
}

/// Revision bookkeeping of one entity (salsa-style).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct EntityRevision {
    /// Last revision in which the entity's fingerprints actually changed.
    pub changed_at: u64,
    /// Last revision in which the entity was re-validated.
    pub verified_at: u64,
}

pub struct IncrementalEngine {
    revision: u64,
    snapshot: Arc<ScpgSnapshot>,
    revisions: HashMap<EntityKey, EntityRevision>,
}

impl IncrementalEngine {
    pub fn new(snapshot: Arc<ScpgSnapshot>) -> Self {
        let revision = 1;
        let revisions = entity_keys(&snapshot)
            .into_keys()
            .map(|k| {
                (
                    k,
                    EntityRevision {
                        changed_at: revision,
                        verified_at: revision,
                    },
                )
            })
            .collect();
        Self {
            revision,
            snapshot,
            revisions,
        }
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn snapshot(&self) -> &Arc<ScpgSnapshot> {
        &self.snapshot
    }

    pub fn plan(&self, delta: &SourceDelta) -> RebuildPlan {
        RebuildPlanner::plan(&self.snapshot, delta)
    }

    /// Revision record of the first entity with `stable_id`.
    pub fn entity_revision(&self, stable_id: &str) -> Option<EntityRevision> {
        self.revisions.get(&(stable_id.to_string(), 0)).copied()
    }

    /// Early-cutoff validity check for a derived result computed at revision `computed_at`
    /// from the entities `dependencies` (stable ids): valid iff none of them changed since.
    pub fn is_result_valid(&self, computed_at: u64, dependencies: &[&str]) -> bool {
        dependencies.iter().all(|d| {
            self.entity_revision(d)
                .map(|r| r.changed_at <= computed_at)
                .unwrap_or(false)
        })
    }

    /// Installs the rebuilt snapshot, diffing it against the current one.
    pub fn commit(&mut self, new_snapshot: Arc<ScpgSnapshot>) -> UpdateResult {
        let diff = SnapshotDiff::compute(&self.snapshot, &new_snapshot);
        let scope = if new_snapshot.digest == self.snapshot.digest {
            RebuildScope::None
        } else {
            // The digest also covers layers without per-entity fingerprints (e.g. SDG edges).
            diff.scope().max(RebuildScope::Positional)
        };
        self.revision += 1;
        let rev = self.revision;

        let new_keys = entity_keys(&new_snapshot);
        let changed: std::collections::HashSet<&str> = diff
            .added
            .iter()
            .chain(&diff.signature_changed)
            .chain(&diff.body_changed)
            .map(|(s, _)| s.as_str())
            .collect();
        let mut revisions = HashMap::with_capacity(new_keys.len());
        for key in new_keys.into_keys() {
            let previous = self.revisions.get(&key).copied();
            let record = match previous {
                Some(r) if !changed.contains(key.0.as_str()) => EntityRevision {
                    changed_at: r.changed_at,
                    verified_at: rev,
                },
                _ => EntityRevision {
                    changed_at: rev,
                    verified_at: rev,
                },
            };
            revisions.insert(key, record);
        }
        self.revisions = revisions;

        let mut stale_links: Vec<u32> = diff
            .added
            .iter()
            .chain(&diff.signature_changed)
            .chain(&diff.body_changed)
            .chain(&diff.moved)
            .map(|&(_, id)| id)
            .filter(|&id| new_snapshot.navigation.to_source(id).is_some())
            .collect();
        stale_links.sort_unstable();
        stale_links.dedup();
        let removed_links = diff
            .removed
            .iter()
            .filter(|&&(_, id)| self.snapshot.navigation.to_source(id).is_some())
            .count();

        let total = new_snapshot.symbols.len().max(1);
        let changed_count =
            diff.added.len() + diff.signature_changed.len() + diff.body_changed.len();
        fn names(v: &[(String, u32)]) -> Vec<String> {
            v.iter().map(|(s, _)| s.clone()).collect()
        }
        let result = UpdateResult {
            revision: rev,
            scope,
            added: names(&diff.added),
            removed: names(&diff.removed),
            signature_changed: names(&diff.signature_changed),
            body_changed: names(&diff.body_changed),
            moved: names(&diff.moved),
            unchanged: diff.unchanged.len(),
            stale_link_count: stale_links.len() + removed_links,
            stale_links,
            changed_fraction: changed_count as f64 / total as f64,
            old_scpg_hash: self.snapshot.scpg_hash,
            new_scpg_hash: new_snapshot.scpg_hash,
            new_digest: digest_hex(&new_snapshot.digest),
        };
        self.snapshot = new_snapshot;
        result
    }
}
