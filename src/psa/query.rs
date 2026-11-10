//! Path queries over stored ROBDD summaries (§8.1 capabilities 1 and 2).
//!
//! A [`FunctionPathView`] reads one function's ordering table and node array from a
//! [`PathSummaryArtifact`] (built in memory or read from a `.psa` file — the reader validates
//! node arrays, and every traversal here re-checks ids and order) and answers, without
//! allocating BDD nodes:
//!
//! - how many paths take a set of required edges and avoid a set of forbidden edges,
//! - whether any such path exists, and
//! - a witness: the variables set on one such path (its edges and, for a segment that starts
//!   after a back edge, its loop entry).
//!
//! Edge ids are the ids of the CFG the summary was built from: Phase 4 CFA edge ids, or —
//! for records flagged `PSA_FLAG_NEUTRAL_CFG` — positions in the edge list of the neutral
//! CFG supplied to Phase 8 ([`FunctionPathView::is_neutral_cfg`]).
//!
//! Each query is O(|ROBDD|) big-integer additions: the required/forbidden edges are *fixed
//! variables* of the model count (see `bdd::sat_count`). Functions stored without an ROBDD
//! (`PSA_FLAG_BDD_UNAVAILABLE`) report [`PathQueryError::SummaryUnavailable`] instead of a
//! guess; their total path count is still available from the directory and metrics.

use std::fmt;

use crate::psa::bdd::node::ROBDDNode;
use crate::psa::bdd::{count_models, find_model};
use crate::psa::count::PathCount;
use crate::psa::ordering::{VarKey, VariableOrdering};
use crate::psa::types::{FunctionPSAHeader, PathSummaryArtifact, PSA_FLAG_NEUTRAL_CFG};

/// Why a path query could not be answered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PathQueryError {
    /// The PSA has no record for this function.
    UnknownFunction(u32),
    /// The function's record has no stored ROBDD (budget exceeded, empty CFG, …).
    SummaryUnavailable(u32),
    /// The edge id has no variable in this function (not one of its CFA edges).
    UnknownEdge(u32),
    /// The same edge is both required and forbidden.
    Conflicting(u32),
    /// The stored data is inconsistent.
    Malformed(String),
}

impl fmt::Display for PathQueryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PathQueryError::UnknownFunction(s) => write!(f, "no PSA record for sym_id={}", s),
            PathQueryError::SummaryUnavailable(s) => {
                write!(f, "sym_id={} has no stored ROBDD summary", s)
            }
            PathQueryError::UnknownEdge(e) => write!(f, "edge {} has no path variable", e),
            PathQueryError::Conflicting(e) => {
                write!(f, "edge {} is both required and forbidden", e)
            }
            PathQueryError::Malformed(m) => write!(f, "malformed PSA record: {}", m),
        }
    }
}

/// One path found by [`FunctionPathView::witness`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathWitness {
    /// CFA edge ids on the path, in BDD level order.
    pub edges: Vec<u32>,
    /// The loop header the segment starts at, or `None` when it starts at ENTRY.
    pub loop_entry: Option<u32>,
}

/// Read-only view of one function's stored summary.
pub struct FunctionPathView<'a> {
    header: &'a FunctionPSAHeader,
    ordering: VariableOrdering,
    nodes: &'a [ROBDDNode],
}

impl PathSummaryArtifact {
    /// Open the stored summary of `sym_id` for path queries.
    pub fn path_view(&self, sym_id: u32) -> Result<FunctionPathView<'_>, PathQueryError> {
        let idx = self
            .function_index(sym_id)
            .ok_or(PathQueryError::UnknownFunction(sym_id))?;
        let header = &self.function_dir[idx];
        if !header.has_bdd() {
            return Err(PathQueryError::SummaryUnavailable(sym_id));
        }
        let codes = self
            .ordering_tables
            .get(idx)
            .ok_or_else(|| PathQueryError::Malformed("missing ordering table".to_string()))?;
        let nodes = self
            .node_arrays
            .get(idx)
            .ok_or_else(|| PathQueryError::Malformed("missing node array".to_string()))?;
        let ordering = VariableOrdering::from_codes(codes).map_err(PathQueryError::Malformed)?;
        if header.root_node as usize >= nodes.len() {
            return Err(PathQueryError::Malformed(format!(
                "root {} outside {} nodes",
                header.root_node,
                nodes.len()
            )));
        }
        Ok(FunctionPathView {
            header,
            ordering,
            nodes: nodes.as_slice(),
        })
    }
}

impl<'a> FunctionPathView<'a> {
    /// The function's directory record.
    pub fn header(&self) -> &FunctionPSAHeader {
        self.header
    }

    /// The function's variable ordering.
    pub fn ordering(&self) -> &VariableOrdering {
        &self.ordering
    }

    /// True iff edge ids index a neutral CFG's edge list rather than the CFA's.
    pub fn is_neutral_cfg(&self) -> bool {
        self.header.has_flag(PSA_FLAG_NEUTRAL_CFG)
    }

    /// Fixed-variable table for `required` (= taken) and `forbidden` (= not taken) edges.
    fn fixed_table(
        &self,
        required: &[u32],
        forbidden: &[u32],
    ) -> Result<Vec<Option<bool>>, PathQueryError> {
        let mut fixed: Vec<Option<bool>> = vec![None; self.ordering.n_vars() as usize];
        for (edges, value) in [(required, true), (forbidden, false)] {
            for &e in edges {
                let level = self
                    .ordering
                    .level_of_edge(e)
                    .ok_or(PathQueryError::UnknownEdge(e))?;
                let slot = &mut fixed[level as usize];
                match *slot {
                    Some(existing) if existing != value => {
                        return Err(PathQueryError::Conflicting(e))
                    }
                    _ => *slot = Some(value),
                }
            }
        }
        Ok(fixed)
    }

    /// Exact number of paths that take every `required` edge and no `forbidden` edge.
    pub fn count_paths(
        &self,
        required: &[u32],
        forbidden: &[u32],
    ) -> Result<PathCount, PathQueryError> {
        let fixed = self.fixed_table(required, forbidden)?;
        count_models(
            self.nodes,
            self.header.root_node,
            self.ordering.n_vars(),
            Some(&fixed),
        )
        .map_err(PathQueryError::Malformed)
    }

    /// Exact number of paths through CFA edge `edge_id`.
    pub fn paths_through_edge(&self, edge_id: u32) -> Result<PathCount, PathQueryError> {
        self.count_paths(&[edge_id], &[])
    }

    /// Whether some path takes every `required` edge and no `forbidden` edge.
    pub fn is_feasible(&self, required: &[u32], forbidden: &[u32]) -> Result<bool, PathQueryError> {
        Ok(self.witness(required, forbidden)?.is_some())
    }

    /// One path that takes every `required` edge and no `forbidden` edge, if any.
    pub fn witness(
        &self,
        required: &[u32],
        forbidden: &[u32],
    ) -> Result<Option<PathWitness>, PathQueryError> {
        let fixed = self.fixed_table(required, forbidden)?;
        let model = find_model(
            self.nodes,
            self.header.root_node,
            self.ordering.n_vars(),
            Some(&fixed),
        )
        .map_err(PathQueryError::Malformed)?;
        let assignment = match model {
            Some(a) => a,
            None => return Ok(None),
        };
        let mut edges = Vec::new();
        let mut loop_entry = None;
        for (level, &taken) in assignment.iter().enumerate() {
            if !taken {
                continue;
            }
            match self.ordering.key_at(level as u16) {
                Some(VarKey::Edge(e)) => edges.push(e),
                Some(VarKey::LoopEntry(h)) => loop_entry = Some(h),
                None => {
                    return Err(PathQueryError::Malformed(format!(
                        "assignment level {} has no variable",
                        level
                    )))
                }
            }
        }
        Ok(Some(PathWitness { edges, loop_entry }))
    }
}
