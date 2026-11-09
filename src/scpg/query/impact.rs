//! ImpactAnalyzer — change-impact sets over calls, overrides, subtypes and type references (§10.5).
//!
//! The impact of changing an entity is the reverse reachability over every dependence the
//! SCPG records:
//! - methods: callers (reverse call edges), overridden super-methods (their callers may
//!   dispatch to the changed override) and overriding sub-methods (signature contract);
//! - types: subtypes and every type that references them (uses, creates, field associations);
//! - fields and enum constants: the methods of the declaring type and, unless private, the
//!   types that reference the declaring type. Phase 6 records no field read/write edges, so
//!   this is the narrowest sound approximation the artifacts allow.

use std::collections::{HashMap, VecDeque};

use serde::Serialize;

use crate::core::types::cg::CallGraphArtifact;
use crate::core::types::symbol::SymbolKind;
use crate::ingestion::TokenCorpusArtifact;
use crate::scpg::query::graph::CallGraphIndex;
use crate::scpg::query::symbols::{is_callable_kind, is_type_kind, SymbolIndex, TypeHierarchyIndex};
use crate::symbol::SymbolTableArtifact;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum ImpactReason {
    /// The entity being changed.
    Changed,
    /// Calls an impacted method.
    Caller,
    /// Overridden by an impacted method; its callers may dispatch to the override.
    OverriddenMethod,
    /// Overrides an impacted method.
    OverridingMethod,
    /// Extends or implements an impacted type.
    Subtype,
    /// Uses, creates or holds a field of an impacted type.
    TypeReference,
    /// Method of the type declaring an impacted field.
    DeclaringTypeMember,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ImpactEntry {
    pub sym_id: u32,
    pub depth: u32,
    pub reason: ImpactReason,
    /// The impacted entity that caused this one to be included (`None` for the seed).
    pub via: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ImpactOptions {
    pub include_overrides: bool,
    pub include_type_dependents: bool,
    pub depth_limit: Option<u32>,
}

impl Default for ImpactOptions {
    fn default() -> Self {
        Self {
            include_overrides: true,
            include_type_dependents: true,
            depth_limit: None,
        }
    }
}

pub struct ImpactAnalyzer;

impl ImpactAnalyzer {
    /// Impact set of changing `sym_id`, in BFS order of discovery depth. `None` when the
    /// symbol is unknown.
    pub fn analyze(
        symbols: &SymbolIndex,
        hierarchy: &TypeHierarchyIndex,
        cg: &CallGraphIndex,
        sym_id: u32,
        options: ImpactOptions,
    ) -> Option<Vec<ImpactEntry>> {
        if !symbols.contains(sym_id) {
            return None;
        }
        let mut found: HashMap<u32, ImpactEntry> = HashMap::new();
        // (entity, depth, callers_only)
        let mut queue: VecDeque<(u32, u32, bool)> = VecDeque::new();
        found.insert(
            sym_id,
            ImpactEntry {
                sym_id,
                depth: 0,
                reason: ImpactReason::Changed,
                via: None,
            },
        );
        queue.push_back((sym_id, 0, false));

        while let Some((e, d, callers_only)) = queue.pop_front() {
            if options.depth_limit.map(|l| d >= l).unwrap_or(false) {
                continue;
            }
            let mut next: Vec<(u32, ImpactReason, bool)> = Vec::new();
            let kind = symbols.kind(e).unwrap_or(SymbolKind::SK_EXTERNAL as u8);

            if is_callable_kind(kind) {
                next.extend(cg.callers(e).iter().map(|&c| (c, ImpactReason::Caller, false)));
                if options.include_overrides && !callers_only && kind == SymbolKind::SK_METHOD as u8 {
                    next.extend(
                        hierarchy
                            .overridden_methods(symbols, e)
                            .into_iter()
                            .map(|s| (s, ImpactReason::OverriddenMethod, true)),
                    );
                    next.extend(
                        hierarchy
                            .overriding_methods(symbols, e)
                            .into_iter()
                            .map(|o| (o, ImpactReason::OverridingMethod, false)),
                    );
                }
            } else if is_type_kind(kind) {
                next.extend(hierarchy.subtypes(e).iter().map(|&t| (t, ImpactReason::Subtype, false)));
                if options.include_type_dependents {
                    next.extend(
                        hierarchy
                            .dependents(e)
                            .iter()
                            .map(|&t| (t, ImpactReason::TypeReference, false)),
                    );
                }
            } else if kind == SymbolKind::SK_FIELD as u8 || kind == SymbolKind::SK_ENUM_CONSTANT as u8 {
                if let Some(owner) = symbols.parent(e) {
                    next.extend(
                        symbols
                            .children(owner)
                            .iter()
                            .filter(|&&m| symbols.kind(m).map(is_callable_kind).unwrap_or(false))
                            .map(|&m| (m, ImpactReason::DeclaringTypeMember, false)),
                    );
                    if options.include_type_dependents && !symbols.is_private(e) {
                        next.extend(
                            hierarchy
                                .dependents(owner)
                                .iter()
                                .map(|&t| (t, ImpactReason::TypeReference, false)),
                        );
                    }
                }
            }

            for (s, reason, only_callers) in next {
                if found.contains_key(&s) {
                    continue;
                }
                found.insert(
                    s,
                    ImpactEntry {
                        sym_id: s,
                        depth: d + 1,
                        reason,
                        via: Some(e),
                    },
                );
                queue.push_back((s, d + 1, only_callers));
            }
        }

        let mut out: Vec<ImpactEntry> = found.into_values().collect();
        out.sort_by_key(|e| (e.depth, e.sym_id));
        Some(out)
    }

    /// Sorted impact set of modifying `sym_id` (callers, overrides, and dependents).
    ///
    /// This one-shot form indexes the artifacts on every call; engines keep the indexes in
    /// their snapshot and use `analyze` directly.
    pub fn impact_set(
        sym_id: u32,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
        cga: &CallGraphArtifact,
    ) -> Vec<u32> {
        let symbols = SymbolIndex::from_artifacts(sta, tca);
        let hierarchy = TypeHierarchyIndex::from_artifacts(sta);
        let cg = CallGraphIndex::build(cga);
        let mut out: Vec<u32> = Self::analyze(&symbols, &hierarchy, &cg, sym_id, ImpactOptions::default())
            .map(|entries| entries.into_iter().map(|e| e.sym_id).collect())
            .unwrap_or_default();
        out.sort_unstable();
        out
    }
}
