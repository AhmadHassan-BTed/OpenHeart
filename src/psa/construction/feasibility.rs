//! FeasibilityFilter — removes paths that take contradictory outcomes of the same branch
//! predicate (§8.2.6).
//!
//! The structural formula `f_paths` encodes syntactically valid paths. A path is
//! semantically infeasible if, for example, it takes the true edge of `if (x > 0)` and later
//! the false edge of another `if (x > 0)` over the same SSA value of `x`. Such correlations
//! are captured with **predicate variables**:
//!
//! ```text
//! for every two-way branch block b with canonical predicate p and polarity s:
//!     x_true(b)  → (p = s)          x_false(b) → (p ≠ s)
//! f_feasible = ∃ p₁ … pₖ . f_paths ∧ ⋀ constraints
//! ```
//!
//! Predicate variables live on their own BDD levels — never an edge's level (the previous
//! filter reused SSA ids as variable indices, so `cond → x_true` constrained unrelated
//! edges and deleted feasible paths) — and are quantified out before counting, so the
//! result counts paths, not (path, valuation) pairs.
//!
//! **Soundness.** Two branches may share a predicate id only if their conditions are the
//! same side-effect-free expression over the same SSA values; within one acyclic path
//! segment every SSA value has a single dynamic instance, so such branches always agree on
//! a feasible path. Under that contract the filter removes only infeasible paths. Branches
//! without a canonical predicate are left unconstrained (sound over-approximation).
//!
//! **Where predicates come from.** Canonical predicates require the condition expression
//! (AST + SSA value numbering of its operands), which Phase 8's inputs (CFA, SSA, CGA) do
//! not carry: an SSA record identifies a *definition*, not a branch condition. They are
//! therefore supplied through a [`BranchPredicateTable`]; with no table, no filtering is
//! performed and the PSA record says so (no `PSA_FLAG_FEASIBILITY_FILTERED`).

use std::collections::{HashMap, HashSet};

use crate::core::types::cfg::CFGEdgeType;
use crate::psa::bdd::{BDDLibrary, BddLimit};
use crate::psa::construction::graph::PathGraph;
use crate::psa::construction::levels::BuildLevels;
use crate::psa::types::BoolOp;

/// A canonical predicate attached to a two-way branch block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BranchPredicate {
    /// The branch block (its out-edges are one `CFGEdgeType::True` and one `False` edge).
    pub block_id: u32,
    /// Canonical predicate id: equal ids ⇔ same pure condition over the same SSA values.
    pub predicate_id: u32,
    /// Polarity: `true` if the block's True edge is taken exactly when the predicate holds,
    /// `false` if it is taken when the predicate does not hold (normalized negation).
    pub true_edge_when: bool,
}

/// Canonical branch predicates per function, keyed by function `sym_id`.
#[derive(Clone, Debug, Default)]
pub struct BranchPredicateTable {
    by_function: HashMap<u32, Vec<BranchPredicate>>,
}

impl BranchPredicateTable {
    /// An empty table (no filtering).
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a predicate for function `sym_id`.
    pub fn insert(&mut self, sym_id: u32, predicate: BranchPredicate) {
        self.by_function.entry(sym_id).or_default().push(predicate);
    }

    /// The predicates of function `sym_id` (empty when none were recorded).
    pub fn predicates_for(&self, sym_id: u32) -> &[BranchPredicate] {
        self.by_function
            .get(&sym_id)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// True iff no predicate is recorded for any function.
    pub fn is_empty(&self) -> bool {
        self.by_function.values().all(|v| v.is_empty())
    }
}

/// A predicate whose branch block was validated against the CFG.
#[derive(Clone, Copy, Debug)]
pub struct UsablePredicate {
    pub predicate_id: u32,
    pub true_edge: u32,
    pub false_edge: u32,
    pub true_edge_when: bool,
}

/// Feasibility filter over predicate variables.
pub struct FeasibilityFilter;

impl FeasibilityFilter {
    /// Validate `predicates` against `graph` and keep the ones that can rule out a path:
    /// the block must be reachable with exactly two out-edges, one `True` and one `False`,
    /// and the predicate id must be shared by at least two such blocks — a predicate tested
    /// once constrains nothing. Only the first predicate recorded for a block is used.
    pub fn usable_predicates(graph: &PathGraph, predicates: &[BranchPredicate]) -> Vec<UsablePredicate> {
        let mut seen_blocks: HashSet<u32> = HashSet::new();
        let mut candidates: Vec<UsablePredicate> = Vec::new();
        for p in predicates {
            if !seen_blocks.insert(p.block_id) {
                continue;
            }
            if !graph.is_reachable(p.block_id) {
                continue;
            }
            let outs = graph.out_edges(p.block_id);
            if outs.len() != 2 {
                continue;
            }
            let mut true_edge = None;
            let mut false_edge = None;
            for &e in outs {
                match graph.edge(e).2 {
                    CFGEdgeType::True => true_edge = Some(e),
                    CFGEdgeType::False => false_edge = Some(e),
                    _ => {}
                }
            }
            if let (Some(t), Some(f)) = (true_edge, false_edge) {
                candidates.push(UsablePredicate {
                    predicate_id: p.predicate_id,
                    true_edge: t,
                    false_edge: f,
                    true_edge_when: p.true_edge_when,
                });
            }
        }

        let mut uses: HashMap<u32, usize> = HashMap::new();
        for c in &candidates {
            *uses.entry(c.predicate_id).or_insert(0) += 1;
        }
        candidates
            .into_iter()
            .filter(|c| uses.get(&c.predicate_id).copied().unwrap_or(0) >= 2)
            .collect()
    }

    /// The conjunction of all branch/predicate implications for `usable`.
    pub fn constraint(
        usable: &[UsablePredicate],
        levels: &BuildLevels,
        bdd: &mut BDDLibrary,
    ) -> Result<u32, BddLimit> {
        let mut acc = bdd.true_id();
        for u in usable {
            let (p_level, t_level, f_level) = match (
                levels.predicate(u.predicate_id),
                levels.edge(u.true_edge),
                levels.edge(u.false_edge),
            ) {
                (Some(p), Some(t), Some(f)) => (p, t, f),
                _ => unreachable!(
                    "predicate {} or its branch edges have no construction level",
                    u.predicate_id
                ),
            };
            let p = bdd.try_var(p_level)?;
            let not_p = bdd.try_not(p)?;
            let (holds_on_true, holds_on_false) = if u.true_edge_when {
                (p, not_p)
            } else {
                (not_p, p)
            };
            let x_true = bdd.try_var(t_level)?;
            let x_false = bdd.try_var(f_level)?;
            let on_true = bdd.try_implies(x_true, holds_on_true)?;
            let on_false = bdd.try_implies(x_false, holds_on_false)?;
            let both = bdd.try_apply(BoolOp::And, on_true, on_false)?;
            acc = bdd.try_apply(BoolOp::And, acc, both)?;
        }
        Ok(acc)
    }
}
