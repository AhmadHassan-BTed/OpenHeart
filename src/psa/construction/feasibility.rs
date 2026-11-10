//! FeasibilityFilter — removes paths whose branch outcomes contradict each other (§8.2.6).
//!
//! The structural formula `f_paths` encodes syntactically valid paths. A path is
//! semantically infeasible if, for example, it takes the true edge of `if (x > 0)` and later
//! the false edge of another `if (x > 0)` over the same SSA value of `x`, or if it takes an
//! edge whose condition is constantly false. Such facts are supplied as **edge guards**
//! ([`EdgeGuard`]) over canonical predicates and captured with **predicate variables**:
//!
//! ```text
//! Requires { e, l₁ … lₘ }:   xₑ → (l₁ ∧ … ∧ lₘ)        lᵢ = (pᵢ = vᵢ)
//! Never { e }:               ¬xₑ
//! f_feasible = ∃ p₁ … pₖ . f_paths ∧ ⋀ guards
//! ```
//!
//! Two-way branches give one guard per outcome edge, multi-way branches one per case edge
//! (and the default edge requires every case predicate to be false), and an edge whose
//! condition folds to `false` gets a `Never` guard. Predicate variables live on their own
//! BDD levels — never an edge's level — and are quantified out before counting, so the
//! result counts paths, not (path, valuation) pairs.
//!
//! **Soundness.** Two edges may share a predicate id only if their conditions are the same
//! side-effect-free predicate over the same SSA values; within one acyclic path segment
//! every SSA value has a single dynamic instance, so such edges always agree on a feasible
//! path. Under that contract the filter removes only infeasible paths. Edges without a guard
//! are left unconstrained (sound over-approximation). A predicate that only guards edges
//! leaving one block cannot rule out a path (a path takes at most one edge of a block per
//! segment), so it is dropped before any BDD work.
//!
//! **Where guards come from.** Canonical predicates need the condition expression and SSA
//! value numbering of its operands, which the CFA does not carry. They are supplied either
//! as block-level [`BranchPredicate`]s for Phase 4 CFGs (through a [`BranchPredicateTable`])
//! or directly as edge guards of a neutral CFG (`construction::NeutralPathFunction`, which a
//! frontend IR fills from its canonical branch-condition atoms). With neither, no filtering
//! is performed and the PSA record says so (no `PSA_FLAG_FEASIBILITY_FILTERED`).

use std::collections::{HashMap, HashSet};

use crate::psa::bdd::{BDDLibrary, BddLimit};
use crate::psa::construction::graph::{PathEdgeKind, PathGraph};
use crate::psa::construction::levels::BuildLevels;
use crate::psa::types::BoolOp;

/// A canonical predicate attached to a two-way branch block of a Phase 4 CFG.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BranchPredicate {
    /// The branch block (its out-edges include exactly one `True` and one `False` edge).
    pub block_id: u32,
    /// Canonical predicate id: equal ids ⇔ same pure condition over the same SSA values.
    pub predicate_id: u32,
    /// Polarity: `true` if the block's True edge is taken exactly when the predicate holds,
    /// `false` if it is taken when the predicate does not hold (normalized negation).
    pub true_edge_when: bool,
}

/// Canonical branch predicates per function, keyed by function `sym_id`.
///
/// Block ids are those of the CFG the table is applied to (Phase 4 block ids for
/// `Phase8Stage::try_run_with`). Predicates computed over another CFG (an IR CFG) must be
/// supplied as edge guards of that CFG instead.
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

/// Canonical predicate `predicate_id` has truth value `value`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PredicateLiteral {
    pub predicate_id: u32,
    pub value: bool,
}

/// What taking one CFG edge implies.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EdgeGuard {
    /// Taking `edge` implies every literal (a conjunction; empty means no constraint).
    Requires {
        edge: u32,
        literals: Vec<PredicateLiteral>,
    },
    /// `edge` can never be taken: its condition is constantly false.
    Never { edge: u32 },
}

impl EdgeGuard {
    /// The guarded edge.
    #[inline]
    pub fn edge(&self) -> u32 {
        match self {
            EdgeGuard::Requires { edge, .. } | EdgeGuard::Never { edge } => *edge,
        }
    }

    /// Guards of the True/False edges of the branch blocks annotated by `predicates`.
    ///
    /// Only the first predicate recorded for a block is used, and only for reachable blocks
    /// with exactly one active `True` and exactly one active `False` out-edge (other edge
    /// kinds, such as exceptional edges, may also leave the block; they are not guarded).
    pub fn from_branch_predicates(
        graph: &PathGraph,
        predicates: &[BranchPredicate],
    ) -> Vec<EdgeGuard> {
        let mut seen_blocks: HashSet<u32> = HashSet::new();
        let mut guards: Vec<EdgeGuard> = Vec::new();
        for p in predicates {
            if !seen_blocks.insert(p.block_id) || !graph.is_reachable(p.block_id) {
                continue;
            }
            let mut true_edges = Vec::new();
            let mut false_edges = Vec::new();
            for &e in graph.out_edges(p.block_id) {
                match graph.edge(e).2 {
                    PathEdgeKind::True => true_edges.push(e),
                    PathEdgeKind::False => false_edges.push(e),
                    _ => {}
                }
            }
            if let ([t], [f]) = (true_edges.as_slice(), false_edges.as_slice()) {
                guards.push(EdgeGuard::Requires {
                    edge: *t,
                    literals: vec![PredicateLiteral {
                        predicate_id: p.predicate_id,
                        value: p.true_edge_when,
                    }],
                });
                guards.push(EdgeGuard::Requires {
                    edge: *f,
                    literals: vec![PredicateLiteral {
                        predicate_id: p.predicate_id,
                        value: !p.true_edge_when,
                    }],
                });
            }
        }
        guards
    }
}

/// Feasibility filter over predicate variables.
pub struct FeasibilityFilter;

impl FeasibilityFilter {
    /// Validate `guards` against `graph` and keep only what can rule out a path:
    ///
    /// - guards of edges that are out of range or inactive are dropped (such an edge is
    ///   already constrained to 0);
    /// - `Never` guards of active edges are kept;
    /// - a `Requires` literal is kept only if its predicate guards active edges leaving at
    ///   least two distinct blocks; a guard left without literals is dropped.
    ///
    /// The result is sorted by edge id, so construction is deterministic.
    pub fn usable_guards(graph: &PathGraph, guards: &[EdgeGuard]) -> Vec<EdgeGuard> {
        let n_edges = graph.n_edges() as u32;
        let active = |e: u32| e < n_edges && !graph.is_inactive(e);

        let mut blocks_of: HashMap<u32, HashSet<u32>> = HashMap::new();
        for g in guards {
            if let EdgeGuard::Requires { edge, literals } = g {
                if !active(*edge) {
                    continue;
                }
                let source = graph.edge(*edge).0;
                for lit in literals {
                    blocks_of
                        .entry(lit.predicate_id)
                        .or_default()
                        .insert(source);
                }
            }
        }
        let shared = |p: u32| blocks_of.get(&p).map(|b| b.len() >= 2).unwrap_or(false);

        let mut out: Vec<EdgeGuard> = Vec::new();
        for g in guards {
            match g {
                EdgeGuard::Never { edge } => {
                    if active(*edge) {
                        out.push(g.clone());
                    }
                }
                EdgeGuard::Requires { edge, literals } => {
                    if !active(*edge) {
                        continue;
                    }
                    let mut kept: Vec<PredicateLiteral> = literals
                        .iter()
                        .copied()
                        .filter(|l| shared(l.predicate_id))
                        .collect();
                    kept.sort_unstable();
                    kept.dedup();
                    if !kept.is_empty() {
                        out.push(EdgeGuard::Requires {
                            edge: *edge,
                            literals: kept,
                        });
                    }
                }
            }
        }
        out.sort_by(|a, b| {
            a.edge()
                .cmp(&b.edge())
                .then_with(|| guard_rank(a).cmp(&guard_rank(b)))
        });
        out
    }

    /// The distinct predicate ids mentioned by `guards`, ascending.
    pub fn predicate_ids(guards: &[EdgeGuard]) -> Vec<u32> {
        let mut ids: Vec<u32> = guards
            .iter()
            .flat_map(|g| match g {
                EdgeGuard::Requires { literals, .. } => literals
                    .iter()
                    .map(|l| l.predicate_id)
                    .collect::<Vec<u32>>(),
                EdgeGuard::Never { .. } => Vec::new(),
            })
            .collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    }

    /// The conjunction of every guard's constraint.
    pub fn constraint(
        guards: &[EdgeGuard],
        levels: &BuildLevels,
        bdd: &mut BDDLibrary,
    ) -> Result<u32, BddLimit> {
        let mut acc = bdd.true_id();
        for g in guards {
            let edge_level = match levels.edge(g.edge()) {
                Some(l) => l,
                None => unreachable!("guarded edge {} has no construction level", g.edge()),
            };
            let x = bdd.try_var(edge_level)?;
            let part = match g {
                EdgeGuard::Never { .. } => bdd.try_not(x)?,
                EdgeGuard::Requires { literals, .. } => {
                    let mut conj = bdd.true_id();
                    for lit in literals {
                        let p_level = match levels.predicate(lit.predicate_id) {
                            Some(l) => l,
                            None => unreachable!(
                                "predicate {} has no construction level",
                                lit.predicate_id
                            ),
                        };
                        let p = bdd.try_var(p_level)?;
                        let l = if lit.value { p } else { bdd.try_not(p)? };
                        conj = bdd.try_apply(BoolOp::And, conj, l)?;
                    }
                    bdd.try_implies(x, conj)?
                }
            };
            acc = bdd.try_apply(BoolOp::And, acc, part)?;
        }
        Ok(acc)
    }
}

/// Tie-break of guards on the same edge (`Never` first).
fn guard_rank(g: &EdgeGuard) -> (u8, Vec<PredicateLiteral>) {
    match g {
        EdgeGuard::Never { .. } => (0, Vec::new()),
        EdgeGuard::Requires { literals, .. } => (1, literals.clone()),
    }
}
