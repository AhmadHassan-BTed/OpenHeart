//! ROBDDQueryEngine — path counts, feasibility, witnesses and metrics over PSA ROBDDs (§10.5).
//!
//! Every function's ROBDD encodes `f_paths` over one Boolean variable per CFG edge
//! (`ordering[var] = edge_id`). Queries restrict variables (edge taken / not taken) and
//! evaluate the stored node arrays directly in O(|ROBDD|):
//! - `count_paths`: #SAT under the restriction, with free variables in skipped levels counted;
//! - `is_feasible`: whether any path satisfies the restriction;
//! - `witness`: one satisfying assignment (`any_sat`) along a single root-to-TRUE path;
//! - `paths_through_edge`: #SAT with one edge forced taken.

use serde::Serialize;

use crate::psa::bdd::node::{FALSE_ID, TRUE_ID};
use crate::psa::types::PathSummaryArtifact;
use crate::scpg::serializer::codec::{SectionDecoder, SectionEncoder};
use crate::scpg::types::ScpgFormatError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct FunctionPathSummary {
    pub sym_id: u32,
    pub n_vars: u32,
    pub n_nodes: u32,
    pub root: u32,
    pub sat_count: u64,
    pub cyclomatic: u16,
    pub max_path_len: u16,
    pub unwind_depth: u16,
    #[serde(skip)]
    node_offset: u64,
    #[serde(skip)]
    order_offset: u64,
    #[serde(skip)]
    order_len: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub struct EdgeConstraint {
    pub edge_id: u32,
    pub taken: bool,
}

/// A path count. `saturated` is set when the exact value exceeds `u128::MAX`, in which case
/// `value` is `u128::MAX` and only a lower bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct PathCount {
    pub value: u128,
    pub saturated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BddQueryError {
    #[error("no path summary for symbol {0}")]
    UnknownFunction(u32),
    #[error("the ROBDD node array of function {0} was not stored")]
    NodesUnavailable(u32),
    #[error("edge {edge_id} is not a variable of the ROBDD of function {sym_id}")]
    UnknownEdge { sym_id: u32, edge_id: u32 },
    #[error("contradictory constraints on edge {0}")]
    Contradiction(u32),
    #[error("ROBDD of function {sym_id} is malformed: {reason}")]
    Malformed { sym_id: u32, reason: String },
}

#[derive(Debug, Clone, Default)]
pub struct PathSummaryIndex {
    functions: Vec<FunctionPathSummary>,
    node_vars: Vec<u16>,
    node_lo: Vec<u32>,
    node_hi: Vec<u32>,
    ordering: Vec<u32>,
}

fn mul_pow2(v: u128, k: u64, saturated: &mut bool) -> u128 {
    if v == 0 {
        0
    } else if k >= 128 || (v.leading_zeros() as u64) < k {
        *saturated = true;
        u128::MAX
    } else {
        v << k
    }
}

fn add_sat(a: u128, b: u128, saturated: &mut bool) -> u128 {
    a.checked_add(b).unwrap_or_else(|| {
        *saturated = true;
        u128::MAX
    })
}

struct Evaluation {
    /// Count of satisfying assignments of the sub-ROBDD below each node, over the variables
    /// strictly below that node's level (restricted variables excluded).
    memo: Vec<Option<u128>>,
    total: PathCount,
}

impl PathSummaryIndex {
    pub fn from_psa(psa: &PathSummaryArtifact) -> Self {
        let mut index = Self::default();
        let mut order: Vec<usize> = (0..psa.function_dir.len()).collect();
        order.sort_by_key(|&i| psa.function_dir[i].sym_id);
        for i in order {
            let h = &psa.function_dir[i];
            if index.functions.last().map(|f| f.sym_id == h.sym_id).unwrap_or(false) {
                continue;
            }
            let nodes = psa.node_arrays.get(i).map(|v| v.as_slice()).unwrap_or(&[]);
            let ordering = psa.ordering_tables.get(i).map(|v| v.as_slice()).unwrap_or(&[]);
            index.functions.push(FunctionPathSummary {
                sym_id: h.sym_id,
                n_vars: h.n_vars,
                n_nodes: nodes.len() as u32,
                root: h.root_node,
                sat_count: h.sat_count,
                cyclomatic: h.cyclomatic,
                max_path_len: h.max_path_len,
                unwind_depth: h.unwind_depth,
                node_offset: index.node_vars.len() as u64,
                order_offset: index.ordering.len() as u64,
                order_len: ordering.len() as u32,
            });
            for n in nodes {
                index.node_vars.push(n.var);
                index.node_lo.push(n.lo);
                index.node_hi.push(n.hi);
            }
            index.ordering.extend_from_slice(ordering);
        }
        index
    }

    pub fn functions(&self) -> &[FunctionPathSummary] {
        &self.functions
    }

    pub fn summary(&self, sym_id: u32) -> Option<&FunctionPathSummary> {
        self.functions
            .binary_search_by_key(&sym_id, |f| f.sym_id)
            .ok()
            .map(|i| &self.functions[i])
    }

    fn function(&self, sym_id: u32) -> Result<&FunctionPathSummary, BddQueryError> {
        let f = self.summary(sym_id).ok_or(BddQueryError::UnknownFunction(sym_id))?;
        if f.n_nodes < 2 {
            return Err(BddQueryError::NodesUnavailable(sym_id));
        }
        Ok(f)
    }

    fn ordering_of(&self, f: &FunctionPathSummary) -> &[u32] {
        let start = f.order_offset as usize;
        &self.ordering[start..start + f.order_len as usize]
    }

    /// Per-variable restriction from edge constraints.
    fn restriction(&self, f: &FunctionPathSummary, constraints: &[EdgeConstraint]) -> Result<Vec<Option<bool>>, BddQueryError> {
        let ordering = self.ordering_of(f);
        let n_vars = f.n_vars as usize;
        let mut restricted = vec![None; n_vars];
        for c in constraints {
            let var = ordering
                .iter()
                .position(|&e| e == c.edge_id)
                .filter(|&v| v < n_vars)
                .ok_or(BddQueryError::UnknownEdge {
                    sym_id: f.sym_id,
                    edge_id: c.edge_id,
                })?;
            match restricted[var] {
                Some(prev) if prev != c.taken => return Err(BddQueryError::Contradiction(c.edge_id)),
                _ => restricted[var] = Some(c.taken),
            }
        }
        Ok(restricted)
    }

    fn evaluate(&self, f: &FunctionPathSummary, restricted: &[Option<bool>]) -> Result<Evaluation, BddQueryError> {
        let n_vars = f.n_vars as usize;
        let n_nodes = f.n_nodes as usize;
        let base = f.node_offset as usize;
        let vars = &self.node_vars[base..base + n_nodes];
        let lo = &self.node_lo[base..base + n_nodes];
        let hi = &self.node_hi[base..base + n_nodes];
        let malformed = |reason: String| BddQueryError::Malformed {
            sym_id: f.sym_id,
            reason,
        };
        let is_terminal = |id: u32| id == FALSE_ID || id == TRUE_ID;
        let level = |id: u32| -> usize {
            if is_terminal(id) {
                n_vars
            } else {
                vars[id as usize] as usize
            }
        };

        let mut prefix = vec![0u64; n_vars + 1];
        for v in 0..n_vars {
            prefix[v + 1] = prefix[v] + restricted[v].is_some() as u64;
        }
        // Free (unrestricted) variables strictly between levels a < b.
        let free_between = |a: usize, b: usize| -> u64 { (b - a - 1) as u64 - (prefix[b] - prefix[a + 1]) };

        if (f.root as usize) >= n_nodes {
            return Err(malformed(format!("root {} outside {} nodes", f.root, n_nodes)));
        }
        let mut memo: Vec<Option<u128>> = vec![None; n_nodes];
        let mut saturated = false;
        let mut stack = vec![f.root];
        while let Some(&id) = stack.last() {
            if is_terminal(id) || memo[id as usize].is_some() {
                stack.pop();
                continue;
            }
            let x = vars[id as usize] as usize;
            if x >= n_vars {
                return Err(malformed(format!("node {} has variable {} ≥ {}", id, x, n_vars)));
            }
            let mut pending = false;
            for (child, value) in [(lo[id as usize], false), (hi[id as usize], true)] {
                if restricted[x] == Some(!value) {
                    continue;
                }
                if (child as usize) >= n_nodes {
                    return Err(malformed(format!("node {} has child {} outside the array", id, child)));
                }
                if level(child) <= x {
                    return Err(malformed(format!("node {} violates the variable ordering", id)));
                }
                if !is_terminal(child) && memo[child as usize].is_none() {
                    stack.push(child);
                    pending = true;
                }
            }
            if pending {
                continue;
            }
            let mut total = 0u128;
            for (child, value) in [(lo[id as usize], false), (hi[id as usize], true)] {
                if restricted[x] == Some(!value) {
                    continue;
                }
                let child_count = match child {
                    TRUE_ID => 1,
                    FALSE_ID => 0,
                    c => memo[c as usize].unwrap_or(0),
                };
                let contribution = mul_pow2(child_count, free_between(x, level(child)), &mut saturated);
                total = add_sat(total, contribution, &mut saturated);
            }
            memo[id as usize] = Some(total);
            stack.pop();
        }

        let root_count = match f.root {
            TRUE_ID => 1,
            FALSE_ID => 0,
            r => memo[r as usize].unwrap_or(0),
        };
        let root_level = level(f.root);
        let free_above = root_level as u64 - prefix[root_level];
        let value = mul_pow2(root_count, free_above, &mut saturated);
        Ok(Evaluation {
            memo,
            total: PathCount { value, saturated },
        })
    }

    /// Number of satisfying edge assignments (feasible paths) under `constraints`.
    pub fn count_paths(&self, sym_id: u32, constraints: &[EdgeConstraint]) -> Result<PathCount, BddQueryError> {
        let f = self.function(sym_id)?;
        let restricted = self.restriction(f, constraints)?;
        Ok(self.evaluate(f, &restricted)?.total)
    }

    pub fn is_feasible(&self, sym_id: u32, constraints: &[EdgeConstraint]) -> Result<bool, BddQueryError> {
        Ok(self.count_paths(sym_id, constraints)?.value > 0)
    }

    pub fn paths_through_edge(&self, sym_id: u32, edge_id: u32) -> Result<PathCount, BddQueryError> {
        self.count_paths(sym_id, &[EdgeConstraint { edge_id, taken: true }])
    }

    /// One satisfying assignment under `constraints` (`any_sat`), or `None` if infeasible.
    /// Edges not listed are don't-care for the returned path.
    pub fn witness(&self, sym_id: u32, constraints: &[EdgeConstraint]) -> Result<Option<Vec<EdgeConstraint>>, BddQueryError> {
        let f = self.function(sym_id)?;
        let restricted = self.restriction(f, constraints)?;
        let eval = self.evaluate(f, &restricted)?;
        if eval.total.value == 0 {
            return Ok(None);
        }
        let ordering = self.ordering_of(f);
        let base = f.node_offset as usize;
        let count_of = |id: u32| -> u128 {
            match id {
                TRUE_ID => 1,
                FALSE_ID => 0,
                c => eval.memo[c as usize].unwrap_or(0),
            }
        };
        let mut assignment: Vec<EdgeConstraint> = constraints.to_vec();
        let mut id = f.root;
        while id != TRUE_ID && id != FALSE_ID {
            let x = self.node_vars[base + id as usize] as usize;
            let (lo, hi) = (self.node_lo[base + id as usize], self.node_hi[base + id as usize]);
            let value = match restricted[x] {
                Some(v) => v,
                None => count_of(lo) == 0,
            };
            if let Some(&edge_id) = ordering.get(x) {
                assignment.push(EdgeConstraint { edge_id, taken: value });
            }
            id = if value { hi } else { lo };
        }
        assignment.sort_by_key(|c| (c.edge_id, c.taken));
        assignment.dedup_by_key(|c| c.edge_id);
        Ok(Some(assignment))
    }

    pub fn encode(&self, enc: &mut SectionEncoder) {
        let f = &self.functions;
        enc.put_u32s(&f.iter().map(|x| x.sym_id).collect::<Vec<_>>());
        enc.put_u32s(&f.iter().map(|x| x.n_vars).collect::<Vec<_>>());
        enc.put_u32s(&f.iter().map(|x| x.n_nodes).collect::<Vec<_>>());
        enc.put_u32s(&f.iter().map(|x| x.root).collect::<Vec<_>>());
        enc.put_u64s(&f.iter().map(|x| x.sat_count).collect::<Vec<_>>());
        enc.put_u16s(&f.iter().map(|x| x.cyclomatic).collect::<Vec<_>>());
        enc.put_u16s(&f.iter().map(|x| x.max_path_len).collect::<Vec<_>>());
        enc.put_u16s(&f.iter().map(|x| x.unwind_depth).collect::<Vec<_>>());
        enc.put_u32s(&f.iter().map(|x| x.order_len).collect::<Vec<_>>());
        enc.put_u16s(&self.node_vars);
        enc.put_u32s(&self.node_lo);
        enc.put_u32s(&self.node_hi);
        enc.put_u32s(&self.ordering);
    }

    pub fn decode(dec: &mut SectionDecoder<'_>) -> Result<Self, ScpgFormatError> {
        let sym_ids = dec.u32s()?;
        let n_vars = dec.u32s()?;
        let n_nodes = dec.u32s()?;
        let roots = dec.u32s()?;
        let sat_counts = dec.u64s()?;
        let cyclomatic = dec.u16s()?;
        let max_path_len = dec.u16s()?;
        let unwind_depth = dec.u16s()?;
        let order_lens = dec.u32s()?;
        let node_vars = dec.u16s()?;
        let node_lo = dec.u32s()?;
        let node_hi = dec.u32s()?;
        let ordering = dec.u32s()?;
        let n = sym_ids.len();
        for (name, len) in [
            ("n_vars", n_vars.len()),
            ("n_nodes", n_nodes.len()),
            ("roots", roots.len()),
            ("sat_counts", sat_counts.len()),
            ("cyclomatic", cyclomatic.len()),
            ("max_path_len", max_path_len.len()),
            ("unwind_depth", unwind_depth.len()),
            ("order_lens", order_lens.len()),
        ] {
            dec.expect_len(name, len, n)?;
        }
        dec.expect_len("node_lo", node_lo.len(), node_vars.len())?;
        dec.expect_len("node_hi", node_hi.len(), node_vars.len())?;
        if sym_ids.windows(2).any(|w| w[0] >= w[1]) {
            return Err(dec.malformed("path summaries are not sorted by sym_id"));
        }
        let mut functions = Vec::with_capacity(n);
        let (mut node_offset, mut order_offset) = (0u64, 0u64);
        for (i, &sym_id) in sym_ids.iter().enumerate() {
            functions.push(FunctionPathSummary {
                sym_id,
                n_vars: n_vars[i],
                n_nodes: n_nodes[i],
                root: roots[i],
                sat_count: sat_counts[i],
                cyclomatic: cyclomatic[i],
                max_path_len: max_path_len[i],
                unwind_depth: unwind_depth[i],
                node_offset,
                order_offset,
                order_len: order_lens[i],
            });
            node_offset += n_nodes[i] as u64;
            order_offset += order_lens[i] as u64;
        }
        if node_offset != node_vars.len() as u64 || order_offset != ordering.len() as u64 {
            return Err(dec.malformed("per-function node or ordering lengths do not add up"));
        }
        Ok(Self {
            functions,
            node_vars,
            node_lo,
            node_hi,
            ordering,
        })
    }
}

/// Artifact-level metric lookups for callers holding a `PathSummaryArtifact`.
pub struct ROBDDQueryEngine;

impl ROBDDQueryEngine {
    /// O(log n) #SAT lookup of a function's path count; `None` for symbols without a summary.
    pub fn path_count(sym_id: u32, psa: &PathSummaryArtifact) -> Option<u64> {
        psa.function_header(sym_id).map(|h| h.sat_count)
    }

    /// O(log n) cyclomatic complexity lookup; `None` for symbols without a summary.
    pub fn cyclomatic(sym_id: u32, psa: &PathSummaryArtifact) -> Option<u16> {
        psa.function_header(sym_id).map(|h| h.cyclomatic)
    }
}
