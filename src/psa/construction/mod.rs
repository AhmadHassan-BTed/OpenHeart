//! FunctionROBDDBuilder — the per-function Phase 8 pipeline (§8.5.2, §8.3).
//!
//! The input is a language-neutral CFG ([`PathCfg`]): a Phase 4 CFG through
//! [`FunctionROBDDBuilder::build`], or any frontend's CFG with its edge guards through
//! [`FunctionROBDDBuilder::build_neutral`]. Both run the same steps:
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
//! Step 10: Assemble metrics and flags; Invariant 2 (V(G) against the structural lower
//!          bound) and Invariant 3 (canonicity, order and ids of the stored array)
//! ```
//!
//! When steps 3–9 cannot complete (budget, deadline, more than 65535 variables) or the
//! stored array fails Invariant 3, the function is recorded with `PSA_FLAG_BDD_UNAVAILABLE`
//! and the step 2 metrics; nothing panics.

pub mod constraints;
pub mod feasibility;
pub mod graph;
pub mod levels;
pub mod recursive;

use std::collections::HashMap;
use std::time::Instant;

use crate::cfg::builder::state::CfgDiagnosticKind;
use crate::cfg::builder::FunctionCFGData;
use crate::psa::bdd::node::ROBDDNode;
use crate::psa::bdd::{count_models, verify_node_array, BDDLibrary, BddLimits, CompactBdd};
use crate::psa::config::Phase8Config;
use crate::psa::count::PathCount;
use crate::psa::ordering::force::FORCE_MAX_ITERATIONS;
use crate::psa::ordering::{
    build_constraint_hyperedges, force_ordering, locality_seed, SiftingConfig, SiftingOptimizer,
    VarKey, VariableOrdering, MAX_BDD_VARS,
};
use crate::psa::types::{
    BoolOp, FunctionROBDD, SummaryUnavailable, PSA_FLAG_BDD_UNAVAILABLE, PSA_FLAG_COUNT_SATURATED,
    PSA_FLAG_CYCLOMATIC_INCONSISTENT, PSA_FLAG_CYCLOMATIC_SATURATED, PSA_FLAG_EMPTY_CFG,
    PSA_FLAG_EXCEPTIONAL_EXCLUDED, PSA_FLAG_FEASIBILITY_FILTERED, PSA_FLAG_HAS_LOOPS,
    PSA_FLAG_MAX_PATH_SATURATED, PSA_FLAG_NEUTRAL_CFG, PSA_FLAG_SIFTED, PSA_NO_ROOT,
};

use self::constraints::build_path_formula;
use self::feasibility::{BranchPredicate, EdgeGuard, FeasibilityFilter};
use self::graph::{ExceptionalEdges, PathCfg, PathGraph};
use self::levels::{BuildLevels, BuildVar};
use self::recursive::RecursiveHandler;

/// One function in the language-neutral input form of Phase 8.
///
/// A frontend fills it from its own CFG: blocks, entry and typed edges ([`PathCfg`]), the
/// guards its canonical branch conditions put on edges ([`EdgeGuard`]), and its V(G) when it
/// computes one. Edge ids are positions in `cfg.edges`, for the guards and for the ordering
/// table of the resulting summary (recorded with `PSA_FLAG_NEUTRAL_CFG`).
#[derive(Clone, Debug)]
pub struct NeutralPathFunction {
    /// Symbol id the summary is recorded under.
    pub sym_id: u32,
    /// The function's CFG.
    pub cfg: PathCfg,
    /// What taking each edge implies about canonical branch predicates (may be empty).
    pub guards: Vec<EdgeGuard>,
    /// McCabe V(G) as defined by the producer, when it computes one. When `None`, Phase 8
    /// stores the structural value `decision points + 1` of `cfg`.
    pub cyclomatic: Option<u16>,
}

/// The producer's V(G), when there is one.
#[derive(Clone, Copy, Debug)]
struct ReportedCyclomatic {
    value: u16,
    /// The producer clamped the value at `u16::MAX`.
    saturated: bool,
}

/// What the builder needs besides the graph.
struct FunctionInput<'a> {
    sym_id: u32,
    cfg: &'a PathCfg,
    graph: &'a PathGraph,
    guards: &'a [EdgeGuard],
    cyclomatic: Option<ReportedCyclomatic>,
    neutral: bool,
}

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
    /// Build the ROBDD and path metrics for one Phase 4 function.
    ///
    /// # Parameters
    /// - `sym_id`: function symbol ID from STA.
    /// - `cfg`: CFG data for this function from the CFA artifact.
    /// - `predicates`: canonical branch predicates of this function (may be empty), keyed by
    ///   Phase 4 block id.
    /// - `is_recursive`: true iff this function's SCC has `scc_class ≥ 1`.
    /// - `config`: budgets and tuning.
    pub fn build(
        sym_id: u32,
        cfg: &FunctionCFGData,
        predicates: &[BranchPredicate],
        is_recursive: bool,
        config: &Phase8Config,
    ) -> FunctionROBDD {
        let path_cfg = PathCfg::from_cfa(cfg);
        let graph = PathGraph::new(&path_cfg, config.exceptional_edges);
        let guards = EdgeGuard::from_branch_predicates(&graph, predicates);
        let saturated = cfg
            .diagnostics
            .iter()
            .any(|d| d.kind == CfgDiagnosticKind::CyclomaticSaturated);
        Self::build_graph(
            FunctionInput {
                sym_id,
                cfg: &path_cfg,
                graph: &graph,
                guards: &guards,
                cyclomatic: Some(ReportedCyclomatic {
                    value: cfg.cyclomatic,
                    saturated,
                }),
                neutral: false,
            },
            is_recursive,
            config,
        )
    }

    /// Build the ROBDD and path metrics for one function supplied in neutral form.
    pub fn build_neutral(
        function: &NeutralPathFunction,
        is_recursive: bool,
        config: &Phase8Config,
    ) -> FunctionROBDD {
        let graph = PathGraph::new(&function.cfg, config.exceptional_edges);
        Self::build_graph(
            FunctionInput {
                sym_id: function.sym_id,
                cfg: &function.cfg,
                graph: &graph,
                guards: &function.guards,
                cyclomatic: function.cyclomatic.map(|value| ReportedCyclomatic {
                    value,
                    saturated: false,
                }),
                neutral: true,
            },
            is_recursive,
            config,
        )
    }

    /// Steps 1–10 over a built graph.
    fn build_graph(
        input: FunctionInput<'_>,
        is_recursive: bool,
        config: &Phase8Config,
    ) -> FunctionROBDD {
        let graph = input.graph;

        // ── Step 1: classification flags ─────────────────────────────────────
        let (mut flags, unwind_depth) = RecursiveHandler::classify(is_recursive);
        if graph.has_loops() {
            flags |= PSA_FLAG_HAS_LOOPS;
        }
        if input.neutral {
            flags |= PSA_FLAG_NEUTRAL_CFG;
        }
        if config.exceptional_edges == ExceptionalEdges::Exclude {
            flags |= PSA_FLAG_EXCEPTIONAL_EXCLUDED;
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
        let (cyclomatic, cyclomatic_flags) =
            cyclomatic_metric(input.cyclomatic, input.cfg.structural_cyclomatic());
        flags |= cyclomatic_flags;

        // ── Steps 3–9: the ROBDD summary ─────────────────────────────────────
        let outcome = if graph.n_blocks() == 0 {
            flags |= PSA_FLAG_EMPTY_CFG;
            Err(SummaryUnavailable::EmptyCfg)
        } else if !graph.entry_is_valid() {
            Err(SummaryUnavailable::InvalidCfg(format!(
                "entry block {} is not one of its {} blocks",
                graph.entry(),
                graph.n_blocks()
            )))
        } else {
            Self::build_summary(graph, input.guards, &structural_count, config)
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

        FunctionROBDD {
            sym_id: input.sym_id,
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
        guards: &[EdgeGuard],
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
        let usable = FeasibilityFilter::usable_guards(graph, guards);
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

        // ── Invariant 3 (§8.8): canonicity, order and ids of the stored array ─
        verify_node_array(&compact.nodes, Some(n_vars)).map_err(|msg| {
            SummaryUnavailable::InvalidBdd(format!("Invariant 3 (canonicity): {}", msg))
        })?;
        if compact.root as usize >= compact.nodes.len() {
            return Err(SummaryUnavailable::InvalidBdd(format!(
                "root {} is outside the {} stored nodes",
                compact.root,
                compact.nodes.len()
            )));
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

/// The V(G) value to store and the flags qualifying it (§8.2.5, Invariant 2 of §8.8).
///
/// `structural` is `decision points + 1` recomputed from the CFG's non-exceptional edges —
/// a lower bound of every producer's value (Phase 4 adds one decision per catch clause,
/// which the CFG's edges do not show). A producer value below it is inconsistent and
/// flagged; without a producer value the structural one is stored.
fn cyclomatic_metric(reported: Option<ReportedCyclomatic>, structural: u64) -> (u16, u16) {
    let mut flags = 0u16;
    let value = match reported {
        Some(r) => {
            if (r.value as u64) < structural.min(u16::MAX as u64) {
                flags |= PSA_FLAG_CYCLOMATIC_INCONSISTENT;
            }
            if r.saturated {
                flags |= PSA_FLAG_CYCLOMATIC_SATURATED;
            }
            r.value
        }
        None => u16::try_from(structural).unwrap_or(u16::MAX),
    };
    if structural > u16::MAX as u64 {
        flags |= PSA_FLAG_CYCLOMATIC_SATURATED;
    }
    (value, flags)
}

/// Insert each predicate variable immediately above the first level of any edge it guards,
/// so the BDD needs to remember a predicate's value only while it is relevant.
fn interleave_predicates(path_order: &[VarKey], usable: &[EdgeGuard]) -> Vec<BuildVar> {
    let mut out: Vec<BuildVar> = Vec::with_capacity(path_order.len() + usable.len());
    let predicate_ids = FeasibilityFilter::predicate_ids(usable);
    if predicate_ids.is_empty() {
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
    let mut first_use: HashMap<u32, usize> = HashMap::with_capacity(predicate_ids.len());
    for g in usable {
        if let EdgeGuard::Requires { edge, literals } = g {
            let pos = edge_pos.get(edge).copied().unwrap_or(0);
            for lit in literals {
                let slot = first_use.entry(lit.predicate_id).or_insert(pos);
                if pos < *slot {
                    *slot = pos;
                }
            }
        }
    }
    let mut by_pos: HashMap<usize, Vec<u32>> = HashMap::new();
    for &p in &predicate_ids {
        let pos = first_use.get(&p).copied().unwrap_or(0);
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
