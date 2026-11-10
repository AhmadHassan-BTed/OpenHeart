//! SliceEngine — statement-level, context-sensitive program slicing over the SDG (§10.6.3).
//!
//! Slices are computed with the two-phase traversal of Horwitz, Reps and Binkley (TOPLAS
//! 1990) over the `SystemDependenceGraph`, which is built from the CFG, the CDG CSR and the
//! def-use CSR of every function plus the resolved call edges:
//! - backward phase 1 ascends into callers (follows `Call`, not `ParamOut`); phase 2 descends
//!   into callees from everything phase 1 found (follows `ParamOut`, not `Call`);
//! - forward slicing is the dual (phase 1 follows `ParamOut`, phase 2 follows `Call`).
//!
//! `SliceMode::Thin` gives thin slices (Sridharan, Fink, Bodík; PLDI 2007): only producer
//! (data and parameter/return) dependences are followed. Control dependences are dropped
//! except the ones on a method entry, which is the channel incoming parameters flow through.
//!
//! The depth limit counts dependence edges. Traversal keeps each node's best (smallest)
//! depth — breadth-first in phase 1 and Dijkstra-ordered in phase 2 — so a node reachable
//! within the limit is never lost because a longer path reached it first.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, VecDeque};

use serde::Serialize;

use crate::cfg::serializer::CFGArtifact;
use crate::core::types::cg::CallGraphArtifact;
use crate::scpg::query::graph::CallGraphIndex;
use crate::scpg::query::sdg::*;
use crate::scpg::query::source::AstIndex;
use crate::ssa::serializer::SSAArtifact;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum SliceDirection {
    Backward,
    Forward,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum SliceMode {
    Full,
    Thin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SliceCriterion {
    /// A single statement (BP-AST pre-order node) of a method.
    Statement { method: u32, ast_node: u32 },
    /// Every statement of a method.
    Method(u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SliceOptions {
    pub direction: SliceDirection,
    pub mode: SliceMode,
    /// Maximum number of dependence edges from the criterion; `None` = unbounded.
    pub depth_limit: Option<u32>,
}

impl Default for SliceOptions {
    fn default() -> Self {
        Self {
            direction: SliceDirection::Backward,
            mode: SliceMode::Full,
            depth_limit: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SliceStatement {
    pub method: u32,
    pub ast_node: u32,
    /// Smallest number of dependence edges between the criterion and this statement.
    pub depth: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProgramSlice {
    pub direction: SliceDirection,
    pub mode: SliceMode,
    pub depth_limit: Option<u32>,
    /// Statements in the slice, sorted by (method, ast_node).
    pub statements: Vec<SliceStatement>,
    /// Methods whose entry (parameter / control region) is in the slice.
    pub entries: Vec<u32>,
    /// Every method contributing at least one node, sorted.
    pub methods: Vec<u32>,
}

pub(crate) struct Traversal {
    pub dist: Vec<u32>,
    pub reached_target: bool,
}

fn allowed(
    kind: u8,
    controller_is_entry: bool,
    phase: u8,
    direction: SliceDirection,
    mode: SliceMode,
) -> bool {
    if mode == SliceMode::Thin && kind == DEP_CONTROL && !controller_is_entry {
        return false;
    }
    match (direction, phase) {
        (SliceDirection::Backward, 1) => kind != DEP_PARAM_OUT,
        (SliceDirection::Backward, _) => kind != DEP_CALL,
        (SliceDirection::Forward, 1) => kind != DEP_CALL,
        (SliceDirection::Forward, _) => kind != DEP_PARAM_OUT,
    }
}

pub struct SliceEngine;

impl SliceEngine {
    /// SDG nodes of a criterion, or `None` if the criterion is not in the graph.
    pub fn criterion_nodes(
        sdg: &SystemDependenceGraph,
        criterion: SliceCriterion,
    ) -> Option<Vec<u32>> {
        let nodes: Vec<u32> = match criterion {
            SliceCriterion::Statement { method, ast_node } => {
                let mut v = Vec::new();
                if let Some(s) = sdg.statement(method, ast_node) {
                    v.push(s);
                }
                if let Some(a) = sdg.find(method, NODE_ACTUAL_IN, ast_node) {
                    v.push(a);
                }
                v
            }
            SliceCriterion::Method(method) => sdg.method_nodes(method).collect(),
        };
        if nodes.is_empty() {
            None
        } else {
            Some(nodes)
        }
    }

    pub(crate) fn two_phase(
        sdg: &SystemDependenceGraph,
        seeds: &[u32],
        direction: SliceDirection,
        mode: SliceMode,
        depth_limit: Option<u32>,
        target: Option<u32>,
    ) -> Traversal {
        let n = sdg.node_count();
        let mut dist = vec![u32::MAX; n];
        let at_limit = |d: u32| depth_limit.map(|l| d >= l).unwrap_or(false);
        let neighbors = |u: u32| match direction {
            SliceDirection::Backward => sdg.preds(u),
            SliceDirection::Forward => sdg.succs(u),
        };
        let is_entry = |id: u32| {
            sdg.node(id)
                .map(|nd| nd.kind == NODE_ENTRY)
                .unwrap_or(false)
        };
        let controller_is_entry = |u: u32, v: u32| match direction {
            SliceDirection::Backward => is_entry(v),
            SliceDirection::Forward => is_entry(u),
        };

        // ── Phase 1: breadth-first, so first discovery is the shortest depth ──
        let mut queue = VecDeque::new();
        for &s in seeds {
            if (s as usize) < n && dist[s as usize] != 0 {
                dist[s as usize] = 0;
                queue.push_back(s);
                if Some(s) == target {
                    return Traversal {
                        dist,
                        reached_target: true,
                    };
                }
            }
        }
        while let Some(u) = queue.pop_front() {
            let d = dist[u as usize];
            if at_limit(d) {
                continue;
            }
            let (adj, kinds) = neighbors(u);
            for (&v, &k) in adj.iter().zip(kinds) {
                if !allowed(k, controller_is_entry(u, v), 1, direction, mode)
                    || dist[v as usize] != u32::MAX
                {
                    continue;
                }
                dist[v as usize] = d + 1;
                if Some(v) == target {
                    return Traversal {
                        dist,
                        reached_target: true,
                    };
                }
                queue.push_back(v);
            }
        }

        // ── Phase 2: seeded with every phase-1 node at its depth; Dijkstra order ──
        let mut heap: BinaryHeap<Reverse<(u32, u32)>> = dist
            .iter()
            .enumerate()
            .filter(|&(_, &d)| d != u32::MAX)
            .map(|(v, &d)| Reverse((d, v as u32)))
            .collect();
        while let Some(Reverse((d, u))) = heap.pop() {
            if d > dist[u as usize] || at_limit(d) {
                continue;
            }
            let (adj, kinds) = neighbors(u);
            for (&v, &k) in adj.iter().zip(kinds) {
                if !allowed(k, controller_is_entry(u, v), 2, direction, mode)
                    || d + 1 >= dist[v as usize]
                {
                    continue;
                }
                dist[v as usize] = d + 1;
                if Some(v) == target {
                    return Traversal {
                        dist,
                        reached_target: true,
                    };
                }
                heap.push(Reverse((d + 1, v)));
            }
        }
        Traversal {
            dist,
            reached_target: false,
        }
    }

    /// Computes a slice; `None` if the criterion has no SDG node (unknown method/statement).
    pub fn slice(
        sdg: &SystemDependenceGraph,
        criterion: SliceCriterion,
        options: SliceOptions,
    ) -> Option<ProgramSlice> {
        let seeds = Self::criterion_nodes(sdg, criterion)?;
        let traversal = Self::two_phase(
            sdg,
            &seeds,
            options.direction,
            options.mode,
            options.depth_limit,
            None,
        );

        let mut statements: Vec<SliceStatement> = Vec::new();
        let mut entries = Vec::new();
        let mut methods = Vec::new();
        for (id, &d) in traversal.dist.iter().enumerate() {
            if d == u32::MAX {
                continue;
            }
            let Some(node) = sdg.node(id as u32) else {
                continue;
            };
            methods.push(node.method);
            if node.kind == NODE_ENTRY {
                entries.push(node.method);
            } else {
                statements.push(SliceStatement {
                    method: node.method,
                    ast_node: node.ast_node,
                    depth: d,
                });
            }
        }
        // Statement and ActualIn nodes of one call map to the same statement: keep the
        // smallest depth.
        statements.sort_by_key(|s| (s.method, s.ast_node, s.depth));
        statements.dedup_by_key(|s| (s.method, s.ast_node));
        entries.sort_unstable();
        entries.dedup();
        methods.sort_unstable();
        methods.dedup();

        Some(ProgramSlice {
            direction: options.direction,
            mode: options.mode,
            depth_limit: options.depth_limit,
            statements,
            entries,
            methods,
        })
    }

    fn method_level(
        root_sym: u32,
        cfa: &CFGArtifact,
        ssa: &SSAArtifact,
        cga: &CallGraphArtifact,
        depth_limit: u32,
        direction: SliceDirection,
    ) -> Vec<u32> {
        let cg = CallGraphIndex::build(cga);
        let sdg = SystemDependenceGraph::build(cfa, ssa, &cg, &AstIndex::default());
        let options = SliceOptions {
            direction,
            mode: SliceMode::Full,
            depth_limit: Some(depth_limit),
        };
        Self::slice(&sdg, SliceCriterion::Method(root_sym), options)
            .map(|s| s.methods)
            .unwrap_or_default()
    }

    /// Methods containing at least one statement of the backward slice of every statement of
    /// `root_sym`, within `depth_limit` dependence edges. Empty when `root_sym` has no CFG.
    ///
    /// This one-shot form builds the call graph index and the SDG on every call; engines keep
    /// both in their snapshot and use `slice` directly.
    pub fn backward_slice(
        root_sym: u32,
        cfa: &CFGArtifact,
        ssa: &SSAArtifact,
        cga: &CallGraphArtifact,
        depth_limit: u32,
    ) -> Vec<u32> {
        Self::method_level(
            root_sym,
            cfa,
            ssa,
            cga,
            depth_limit,
            SliceDirection::Backward,
        )
    }

    /// Forward counterpart of `backward_slice`.
    pub fn forward_slice(
        root_sym: u32,
        cfa: &CFGArtifact,
        ssa: &SSAArtifact,
        cga: &CallGraphArtifact,
        depth_limit: u32,
    ) -> Vec<u32> {
        Self::method_level(
            root_sym,
            cfa,
            ssa,
            cga,
            depth_limit,
            SliceDirection::Forward,
        )
    }
}
