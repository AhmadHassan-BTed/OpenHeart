//! Exact path formula `f_paths` over the Ball-Larus DAG (§8.2.3, §8.5.3).
//!
//! For a function with CFG (B, E) every CFA edge `e` has a Boolean variable `xₑ` and every
//! loop header `h` a loop-entry variable `yₕ` (the Ball-Larus dummy edge `ENTRY → h`). An
//! assignment satisfies `f_paths` **iff** it is the characteristic vector of exactly one
//! acyclic path segment (see `construction::graph`), so `#SAT(f_paths)` *is* the path count:
//!
//! ```text
//! starts:     s₀ = ¬(∨ₕ yₕ)                          the segment starts at ENTRY …
//!             AtMostOne{yₕ}                           … or at exactly one loop header
//! block b:    In_b  = {s₀ if b = ENTRY} ∪ {y_b if b is a header}
//!                   ∪ {xₑ : e = (p → b) active, not a back edge}
//!             Out_b = {xₑ : e = (b → s)}              (back edges end the segment)
//!             AtMostOne(In_b)
//!             Out_b = ∅ :  nothing more               (the segment may end here)
//!             Out_b ≠ ∅ :  AtMostOne(Out_b) ∧ (∨In_b ↔ ∨Out_b)
//! inactive e: ¬xₑ                                     (unreachable source / bad endpoint)
//! ```
//!
//! Flow is conserved with unit capacity at every block and exactly one unit enters at a
//! start; because back edges only *leave* blocks, the 1-edges form one path in a DAG and
//! every variable is determined by the path. The previous encoding (`reach(b) → out(b)`)
//! left the edges of unreached blocks free and had no exit condition, overstating counts
//! exponentially (one if/else counted 8 paths instead of 2).
//!
//! Each `AtMostOne`/`∨` is built with O(k) operations by the two-accumulator construction
//! (`none`, `exactly_one`) instead of O(k²) pairwise clauses, and blocks are conjoined in
//! topological order, matching the locality-preserving variable order.

use crate::psa::bdd::{BDDLibrary, BddLimit};
use crate::psa::construction::graph::{PathGraph, ENTRY_BLOCK};
use crate::psa::construction::levels::BuildLevels;
use crate::psa::types::BoolOp;

/// `(at_most_one, exactly_one, any)` of a list of BDDs.
pub struct Cardinality {
    pub at_most_one: u32,
    pub exactly_one: u32,
    pub any: u32,
}

/// Build the cardinality constraints of `xs` with O(|xs|) operations.
pub fn cardinality(bdd: &mut BDDLibrary, xs: &[u32]) -> Result<Cardinality, BddLimit> {
    let mut none = bdd.true_id();
    let mut one = bdd.false_id();
    for &x in xs {
        let not_x = bdd.try_not(x)?;
        let stay_one = bdd.try_apply(BoolOp::And, one, not_x)?;
        let become_one = bdd.try_apply(BoolOp::And, none, x)?;
        one = bdd.try_apply(BoolOp::Or, stay_one, become_one)?;
        none = bdd.try_apply(BoolOp::And, none, not_x)?;
    }
    let at_most_one = bdd.try_apply(BoolOp::Or, none, one)?;
    let any = bdd.try_not(none)?;
    Ok(Cardinality {
        at_most_one,
        exactly_one: one,
        any,
    })
}

/// Literal BDD of the variable at `level`.
#[inline]
fn literal(bdd: &mut BDDLibrary, level: u16) -> Result<u32, BddLimit> {
    bdd.try_var(level)
}

/// Level of edge `e`; every CFA edge has a construction level by construction of
/// [`BuildLevels`].
#[inline]
fn edge_level(levels: &BuildLevels, e: u32) -> u16 {
    match levels.edge(e) {
        Some(l) => l,
        None => unreachable!("CFA edge {} has no construction level", e),
    }
}

/// Build `f_paths` for `graph` under `levels`.
pub fn build_path_formula(
    graph: &PathGraph,
    levels: &BuildLevels,
    bdd: &mut BDDLibrary,
) -> Result<u32, BddLimit> {
    // ── Segment starts ────────────────────────────────────────────────────────
    let mut loop_entries: Vec<u32> = Vec::with_capacity(graph.headers().len());
    for &h in graph.headers() {
        let level = match levels.loop_entry(h) {
            Some(l) => l,
            None => unreachable!("loop header {} has no loop-entry level", h),
        };
        loop_entries.push(literal(bdd, level)?);
    }
    let starts = cardinality(bdd, &loop_entries)?;
    let function_start = bdd.try_not(starts.any)?;
    let mut f_paths = starts.at_most_one;

    // ── Flow conservation, block by block in topological order ────────────────
    for &b in graph.topo_order() {
        let phi_b = build_block_constraint(b, graph, levels, function_start, bdd)?;
        f_paths = bdd.try_apply(BoolOp::And, f_paths, phi_b)?;
    }

    // ── Edges that can lie on no path ─────────────────────────────────────────
    for e in 0..graph.n_edges() as u32 {
        if graph.is_inactive(e) {
            let x = literal(bdd, edge_level(levels, e))?;
            let not_x = bdd.try_not(x)?;
            f_paths = bdd.try_apply(BoolOp::And, f_paths, not_x)?;
        }
    }

    Ok(f_paths)
}

/// The flow constraint Φ_b of one reachable block.
fn build_block_constraint(
    block: u32,
    graph: &PathGraph,
    levels: &BuildLevels,
    function_start: u32,
    bdd: &mut BDDLibrary,
) -> Result<u32, BddLimit> {
    let mut ins: Vec<u32> = Vec::new();
    if block == ENTRY_BLOCK {
        ins.push(function_start);
    }
    if graph.is_header(block) {
        let level = match levels.loop_entry(block) {
            Some(l) => l,
            None => unreachable!("loop header {} has no loop-entry level", block),
        };
        ins.push(literal(bdd, level)?);
    }
    for &e in graph.in_edges(block) {
        if graph.is_forward(e) {
            ins.push(literal(bdd, edge_level(levels, e))?);
        }
    }

    let mut outs: Vec<u32> = Vec::with_capacity(graph.out_edges(block).len());
    for &e in graph.out_edges(block) {
        outs.push(literal(bdd, edge_level(levels, e))?);
    }

    let input = cardinality(bdd, &ins)?;
    if outs.is_empty() {
        // A block without successors ends the segment.
        return Ok(input.at_most_one);
    }
    let output = cardinality(bdd, &outs)?;
    let conserve = bdd.try_iff(input.any, output.any)?;
    let both_unique = bdd.try_apply(BoolOp::And, input.at_most_one, output.at_most_one)?;
    bdd.try_apply(BoolOp::And, both_unique, conserve)
}
