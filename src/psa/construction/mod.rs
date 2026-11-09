//! FunctionROBDDBuilder — the per-function Phase 8 pipeline (§8.5.2, §8.3).
//!
//! ```text
//! Step 1: PathGraph — edge-id CSR, DFS classification (back edges, loop headers, inactive edges)
//! Step 2: Exact metrics by O(V+E) dynamic programs: Ball-Larus path count (unbounded integer)
//!         and longest acyclic path — always available, independent of any BDD budget
//! Step 3: Variable ordering — locality-preserving topological seed, refined by FORCE with the
//!         span-based stopping rule
//! Step 4: Predicate levels for feasibility filtering (when canonical predicates are supplied)
//! Step 5: Exact path formula f_paths, built under a node budget and a deadline
//! Step 6: Cross-check #SAT(f_paths) against the exact path count
//! Step 7: Feasibility filtering and ∃-quantification of the predicate variables
//! Step 8: Mark-compact the root's sub-DAG and drop the predicate levels
//! Step 9: Rudell sifting (budgeted; kept only if it shrinks the ROBDD and preserves #SAT)
//! Step 10: Assemble metrics and flags
//! ```
//!
//! When steps 3–9 cannot complete (budget, deadline, more than 65535 variables) the
//! function is recorded with `PSA_FLAG_BDD_UNAVAILABLE` and the step 2 metrics.

pub mod constraints;
pub mod feasibility;
pub mod graph;
pub mod levels;
pub mod recursive;

use std::collections::HashMap;
use std::time::Instant;

use crate::cfg::builder::FunctionCFGData;
use crate::psa::bdd::node::ROBDDNode;
use crate::psa::bdd::{count_models, BDDLibrary, BddLimits, CompactBdd};
use crate::psa::config::Phase8Config;
use crate::psa::count::PathCount;
use crate::psa::ordering::force::FORCE_MAX_ITERATIONS;
use crate::psa::ordering::{
    build_constraint_hyperedges, force_ordering, locality_seed, SiftingConfig, SiftingOptimizer,
    VarKey, VariableOrdering, MAX_BDD_VARS,
};
use crate::psa::types::{
    BoolOp, FunctionROBDD, SummaryUnavailable, PSA_FLAG_BDD_UNAVAILABLE,
    PSA_FLAG_COUNT_SATURATED, PSA_FLAG_EMPTY_CFG, PSA_FLAG_FEASIBILITY_FILTERED,
    PSA_FLAG_HAS_LOOPS, PSA_FLAG_MAX_PATH_SATURATED, PSA_FLAG_SIFTED, PSA_NO_ROOT,
};

use self::constraints::build_path_formula;
use self::feasibility::{BranchPredicate, FeasibilityFilter, UsablePredicate};
use self::graph::PathGraph;
use self::levels::{BuildLevels, BuildVar};
use self::recursive::RecursiveHandler;

/// Builds the summary for a single function.
///
/// Called from `Phase8Stage::run()` (possibly on several worker threads; every call is
/// independent and owns its BDD library).
pub struct FunctionROBDDBuilder;

/// The ROBDD part of a summary.
struct BuiltSummary {
    bdd: CompactBdd,
    ordering: VariableOrdering,
    count: PathCount,
    filtered: bool,
    sifted: bool,
}

impl FunctionROBDDBuilder {
    /// Build the ROBDD and path metrics for one function.
    ///
    /// # Parameters
    /// - `sym_id`: function symbol ID from STA.
    /// - `cfg`: CFG data for this function from the CFA artifact.
    /// - `predicates`: canonical branch predicates of this function (may be empty).
    /// - `is_recursive`: true iff this function's SCC has `scc_class ≥ 1`.
    /// - `config`: budgets and tuning.
    pub fn build(
        sym_id: u32,
        cfg: &FunctionCFGData,
        predicates: &[BranchPredicate],
        is_recursive: bool,
        config: &Phase8Config,
    ) -> FunctionROBDD {
        // ── Step 1: edge-indexed, classified CFG ─────────────────────────────
        let graph = PathGraph::new(cfg);
        let (mut flags, unwind_depth) = RecursiveHandler::classify(is_recursive);
        if graph.has_loops() {
            flags |= PSA_FLAG_HAS_LOOPS;
        }

        // ── Step 2: exact metrics (no BDD needed) ────────────────────────────
        let structural_count = graph.path_count();
        let longest = graph.longest_path_blocks();
        let max_path_len = if longest > u16::MAX as u64 {
            flags |= PSA_FLAG_MAX_PATH_SATURATED;
            u16::MAX
        } else {
            longest as u16
        };
        // V(G) = |E| - |B| + 2, computed by Phase 4 from CFA metadata (§8.2.5).
        let cyclomatic = cfg.cyclomatic;

        // ── Steps 3–9: the ROBDD summary ─────────────────────────────────────
        let outcome = if graph.n_blocks() == 0 {
            flags |= PSA_FLAG_EMPTY_CFG;
            Err(SummaryUnavailable::EmptyCfg)
        } else {
            Self::build_summary(&graph, predicates, &structural_count, config)
        };

        // ── Step 10: assemble ────────────────────────────────────────────────
        let (ordering, nodes, root, path_count, unavailable) = match outcome {
            Ok(summary) => {
                if summary.filtered {
                    flags |= PSA_FLAG_FEASIBILITY_FILTERED;
                }
                if summary.sifted {
                    flags |= PSA_FLAG_SIFTED;
                }
                (
                    summary.ordering,
                    summary.bdd.nodes,
                    summary.bdd.root,
                    summary.count,
                    None,
                )
            }
            Err(reason) => {
                flags |= PSA_FLAG_BDD_UNAVAILABLE;
                (
                    VariableOrdering::empty(),
                    vec![ROBDDNode::false_terminal(), ROBDDNode::true_terminal()],
                    PSA_NO_ROOT,
                    structural_count,
                    Some(reason),
                )
            }
        };

        let sat_count = match path_count.to_u64() {
            Some(v) => v,
            None => {
                flags |= PSA_FLAG_COUNT_SATURATED;
                u64::MAX
            }
        };
        let log2_paths = path_count.log2();

        // ── Invariant 3 check (§8.8): canonicity and order of the stored array ──
        #[cfg(debug_assertions)]
        {
            if root != PSA_NO_ROOT {
                if let Err(msg) =
                    crate::psa::bdd::verify_node_array(&nodes, Some(ordering.n_vars()))
                {
                    panic!(
                        "Phase 8 Invariant 3 (Canonicity) violated for sym_id={}: {}",
                        sym_id, msg
                    );
                }
            }
        }

        FunctionROBDD {
            sym_id,
            ordering,
            nodes,
            root,
            path_count,
            sat_count,
            log2_paths,
            cyclomatic,
            max_path_len,
            unwind_depth,
            flags,
            unavailable,
        }
    }

    /// Steps 3–9.
    fn build_summary(
        graph: &PathGraph,
        predicates: &[BranchPredicate],
        dp_count: &PathCount,
        config: &Phase8Config,
    ) -> Result<BuiltSummary, SummaryUnavailable> {
        // ── Step 3: variable ordering ─────────────────────────────────────────
        let seed = locality_seed(graph);
        if seed.len() > MAX_BDD_VARS {
            return Err(SummaryUnavailable::TooManyVariables { needed: seed.len() });
        }
        let hyperedges = build_constraint_hyperedges(graph, &seed);
        let order = force_ordering(seed.len(), &hyperedges, FORCE_MAX_ITERATIONS);
        let path_order: Vec<VarKey> = order.iter().map(|&i| seed[i]).collect();

        // ── Step 4: predicate levels ──────────────────────────────────────────
        let usable = FeasibilityFilter::usable_predicates(graph, predicates);
        let build_vars = interleave_predicates(&path_order, &usable);
        if build_vars.len() > MAX_BDD_VARS {
            return Err(SummaryUnavailable::TooManyVariables {
                needed: build_vars.len(),
            });
        }
        let levels = BuildLevels::new(build_vars, graph.n_edges())
            .map_err(SummaryUnavailable::InvalidBdd)?;
        let n_levels = levels.n_levels() as u16;

        // ── Step 5: exact path formula under the node budget and deadline ─────
        let deadline = Instant::now() + config.bdd_time_budget;
        let mut bdd = BDDLibrary::with_limits(BddLimits::new(config.max_bdd_nodes, Some(deadline)));
        let f_paths =
            build_path_formula(graph, &levels, &mut bdd).map_err(SummaryUnavailable::Limit)?;

        // ── Step 6: #SAT(f_paths) must equal the exact path count ─────────────
        let structural = bdd
            .try_sat_count_fixed(f_paths, n_levels, &levels.predicates_fixed_false())
            .map_err(SummaryUnavailable::InvalidBdd)?;
        if &structural != dp_count {
            return Err(SummaryUnavailable::CountMismatch {
                bdd_count: structural.to_string(),
                dp_count: dp_count.to_string(),
            });
        }

        // ── Step 7: feasibility filtering + ∃ predicates ──────────────────────
        let (root, filtered) = if usable.is_empty() {
            (f_paths, false)
        } else {
            let constraint = FeasibilityFilter::constraint(&usable, &levels, &mut bdd)
                .map_err(SummaryUnavailable::Limit)?;
            let constrained = bdd
                .try_apply(BoolOp::And, f_paths, constraint)
                .map_err(SummaryUnavailable::Limit)?;
            let projected = bdd
                .try_exists(constrained, &levels.predicate_mask())
                .map_err(SummaryUnavailable::Limit)?;
            (projected, true)
        };

        // ── Step 8: compact the root's sub-DAG, drop predicate levels ─────────
        let (relabel, final_keys) = levels.final_relabel();
        let mut compact = bdd.compact(root, Some(&relabel));
        drop(bdd);
        let mut ordering =
            VariableOrdering::from_keys(&final_keys).map_err(SummaryUnavailable::InvalidBdd)?;
        let n_vars = ordering.n_vars();
        let count = if filtered {
            count_models(&compact.nodes, compact.root, n_vars, None)
                .map_err(SummaryUnavailable::InvalidBdd)?
        } else {
            dp_count.clone()
        };

        // ── Step 9: Rudell sifting ────────────────────────────────────────────
        let mut sifted = false;
        let internal_nodes = compact.nodes.len().saturating_sub(2);
        if config.sift && n_vars > 1 && internal_nodes >= config.min_sift_nodes {
            let sift_config = SiftingConfig {
                max_growth: config.sift_max_growth,
                max_swaps: config.max_sift_swaps,
                max_vars: config.max_sift_vars,
                deadline: Some(Instant::now() + config.sift_time_budget),
            };
            let result = SiftingOptimizer::sift(&compact, n_vars, &sift_config);
            if result.size_after < result.size_before {
                // Keep the reordered ROBDD only if it is consistent: a bijective level map and
                // an unchanged model count.
                let recount = count_models(&result.nodes, result.root, n_vars, None);
                let permuted = ordering.permuted(&result.level_to_old_level);
                if let (Ok(c), Ok(o)) = (recount, permuted) {
                    if c == count {
                        ordering = o;
                        compact = CompactBdd {
                            nodes: result.nodes,
                            root: result.root,
                        };
                        sifted = true;
                    }
                }
            }
        }

        Ok(BuiltSummary {
            bdd: compact,
            ordering,
            count,
            filtered,
            sifted,
        })
    }
}

/// Insert each predicate variable immediately above the first level of any of its branch
/// edges, so the BDD needs to remember a predicate's value only while it is relevant.
fn interleave_predicates(path_order: &[VarKey], usable: &[UsablePredicate]) -> Vec<BuildVar> {
    let mut out: Vec<BuildVar> = Vec::with_capacity(path_order.len() + usable.len());
    if usable.is_empty() {
        out.extend(path_order.iter().map(|&k| BuildVar::Path(k)));
        return out;
    }

    let mut edge_pos: HashMap<u32, usize> = HashMap::with_capacity(path_order.len());
    for (pos, key) in path_order.iter().enumerate() {
        if let VarKey::Edge(e) = *key {
            edge_pos.insert(e, pos);
        }
    }
    // First position at which each predicate is needed.
    let mut first_use: HashMap<u32, usize> = HashMap::new();
    for u in usable {
        let pos = [u.true_edge, u.false_edge]
            .iter()
            .filter_map(|e| edge_pos.get(e).copied())
            .min()
            .unwrap_or(0);
        let slot = first_use.entry(u.predicate_id).or_insert(pos);
        if pos < *slot {
            *slot = pos;
        }
    }
    let mut by_pos: HashMap<usize, Vec<u32>> = HashMap::new();
    for (&p, &pos) in &first_use {
        by_pos.entry(pos).or_default().push(p);
    }
    for list in by_pos.values_mut() {
        list.sort_unstable();
    }

    for (pos, &key) in path_order.iter().enumerate() {
        if let Some(list) = by_pos.get(&pos) {
            out.extend(list.iter().map(|&p| BuildVar::Predicate(p)));
        }
        out.push(BuildVar::Path(key));
    }
    out
}
