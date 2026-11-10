//! Delta Applicator & Incremental Invalidation (§7.5.4).
//!
//! Compares two traceability artifacts entity by entity. Symbols are matched by their
//! **stable key** (kind, qualified name path, parameter signature, ordinal — see
//! `tra::entity`), never by positional `sym_id` or token id, so inserting a declaration
//! early in a file does not invalidate everything after it. For every matched pair the
//! **content digest** decides whether the entity changed, and the declaration location
//! whether it merely moved. Deletions are reported explicitly.
//!
//! [`DeltaApplicator::propagate`] then extends the invalidation through dependencies that
//! affect generated views: the enclosing declarations of changed members (class diagrams)
//! and, transitively, the callers of changed callables (sequence diagrams).
//!
//! Scope: this is the per-entity diff between two complete builds. Re-running Phases 1–6
//! only for touched functions (incremental reparse, dependency-tracked recomputation) is the
//! job of the pipeline driver; the delta tells it which entities' derived records changed.

pub mod invalidation;

pub use invalidation::StaleDetector;

use std::collections::{HashMap, HashSet, VecDeque};

use crate::core::types::cg::CallGraphArtifact;
use crate::symbol::SymbolTableArtifact;
use crate::tra::types::{EntityRecord, TraceabilityArtifact};

/// Entity-level difference between two traceability artifacts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceabilityDelta {
    pub new_scpg_hash: u32,
    pub old_scpg_hash: u32,
    /// New sym_ids with no counterpart in the old build.
    pub added: Vec<u32>,
    /// Old sym_ids with no counterpart in the new build.
    pub removed: Vec<u32>,
    /// `(old sym_id, new sym_id)` pairs whose content changed.
    pub modified: Vec<(u32, u32)>,
    /// `(old sym_id, new sym_id)` pairs with unchanged content at a new location.
    pub relocated: Vec<(u32, u32)>,
    /// New sym_ids whose derived records must be regenerated (added ∪ modified ∪ relocated,
    /// plus whatever [`DeltaApplicator::propagate`] adds), sorted and deduplicated.
    pub invalidated_syms: Vec<u32>,
}

impl TraceabilityDelta {
    /// True iff nothing changed between the two builds.
    pub fn is_empty(&self) -> bool {
        self.added.is_empty()
            && self.removed.is_empty()
            && self.modified.is_empty()
            && self.relocated.is_empty()
    }
}

pub struct DeltaApplicator;

impl DeltaApplicator {
    /// Diff `old` against `new` by stable keys.
    pub fn compute_delta(
        old: &TraceabilityArtifact,
        new: &TraceabilityArtifact,
    ) -> TraceabilityDelta {
        let old_by_key = key_map(&old.entities);
        let mut matched_old: HashSet<u32> = HashSet::with_capacity(old.entities.len());

        let mut added = Vec::new();
        let mut modified = Vec::new();
        let mut relocated = Vec::new();

        for (new_id, entity) in new.entities.iter().enumerate() {
            let new_id = new_id as u32;
            match old_by_key.get(&entity.stable_key) {
                None => added.push(new_id),
                Some(&old_id) => {
                    matched_old.insert(old_id);
                    let before = &old.entities[old_id as usize];
                    if before.content_digest != entity.content_digest {
                        modified.push((old_id, new_id));
                    } else if !same_location(before, entity) {
                        relocated.push((old_id, new_id));
                    }
                }
            }
        }

        let removed: Vec<u32> = (0..old.entities.len() as u32)
            .filter(|id| !matched_old.contains(id))
            .collect();

        let mut invalidated: Vec<u32> = added
            .iter()
            .copied()
            .chain(modified.iter().map(|&(_, n)| n))
            .chain(relocated.iter().map(|&(_, n)| n))
            .collect();
        invalidated.sort_unstable();
        invalidated.dedup();

        TraceabilityDelta {
            new_scpg_hash: new.hashes.scpg_hash,
            old_scpg_hash: old.hashes.scpg_hash,
            added,
            removed,
            modified,
            relocated,
            invalidated_syms: invalidated,
        }
    }

    /// Extend `delta.invalidated_syms` (new-build sym_ids) with the enclosing declarations
    /// of every invalidated or newly added symbol and with the transitive callers of every
    /// modified or added callable, using the new build's symbol table and call graph.
    pub fn propagate(
        delta: &mut TraceabilityDelta,
        new_sta: &SymbolTableArtifact,
        new_cga: &CallGraphArtifact,
    ) {
        let n = new_sta.symbol_records.len();
        let mut invalid: HashSet<u32> = delta.invalidated_syms.iter().copied().collect();

        // Callers of changed callables, transitively (reverse call edges).
        let mut callers_of: HashMap<u32, Vec<u32>> = HashMap::new();
        for &(caller, callee, _) in &new_cga.site_to_edge_map {
            callers_of.entry(callee).or_default().push(caller);
        }
        let mut queue: VecDeque<u32> = delta
            .modified
            .iter()
            .map(|&(_, new_id)| new_id)
            .chain(delta.added.iter().copied())
            .collect();
        let mut visited: HashSet<u32> = queue.iter().copied().collect();
        while let Some(callee) = queue.pop_front() {
            if let Some(callers) = callers_of.get(&callee) {
                for &caller in callers {
                    if visited.insert(caller) {
                        invalid.insert(caller);
                        queue.push_back(caller);
                    }
                }
            }
        }

        // Enclosing declarations of everything invalidated so far.
        let seeds: Vec<u32> = invalid.iter().copied().collect();
        for sym in seeds {
            let mut cur = sym;
            let mut steps = 0usize;
            while (cur as usize) < n && steps <= n {
                let parent = new_sta.symbol_records[cur as usize].parent_sym;
                if parent == u32::MAX || parent == cur || (parent as usize) >= n {
                    break;
                }
                if !invalid.insert(parent) {
                    break; // already invalidated, and so are its ancestors
                }
                cur = parent;
                steps += 1;
            }
        }

        let mut all: Vec<u32> = invalid.into_iter().collect();
        all.sort_unstable();
        delta.invalidated_syms = all;
    }
}

/// `stable_key → sym_id` (first occurrence wins; keys are unique by construction).
fn key_map(entities: &[EntityRecord]) -> HashMap<[u8; 16], u32> {
    let mut map = HashMap::with_capacity(entities.len());
    for (id, e) in entities.iter().enumerate() {
        map.entry(e.stable_key).or_insert(id as u32);
    }
    map
}

fn same_location(a: &EntityRecord, b: &EntityRecord) -> bool {
    a.file_id == b.file_id
        && a.line_start == b.line_start
        && a.col_start == b.col_start
        && a.line_end == b.line_end
}
