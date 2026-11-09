//! SystemDependenceGraph — statement-level SDG over CFG, CDG, def-use and call edges (§10.6.3).
//!
//! Nodes:
//! - `Entry(m)` — method entry; stands for the incoming parameters and the control region of
//!   statements that no predicate controls (Ferrante-Ottenstein-Warren PDG entry).
//! - `Statement(m, n)` — a CFG statement (BP-AST pre-order node `n`). For a call statement it
//!   also plays the *actual-out* role (the call's result).
//! - `ActualIn(m, n)` — the *actual-in* side of the call statement `n`: it carries the
//!   statement's intra-procedural dependences but not its return value.
//!
//! Edges point from a node to the node it depends on (`to` depends on `from`):
//! - `Data`: SSA def-use, with φ-functions resolved to their reaching definitions;
//! - `Control`: CDG control dependence on the controlling block's predicate (or `Entry`);
//! - `Call`: callee `Entry` ← caller `ActualIn` (parameter-in / call edge);
//! - `ParamOut`: caller call `Statement` ← callee return statements.
//!
//! Actual parameters of one call are collapsed into its `ActualIn` node, so the summary edges
//! of Horwitz-Reps-Binkley are subsumed by the call statement's own data dependences. This
//! over-approximates per-argument precision but keeps the two-phase traversal context
//! sensitive. Heap dependences are not modelled: Phase 6 points-to facts are keyed by
//! function-local SSA ids without the field accesses needed to link writes to reads.

use std::collections::{HashMap, HashSet};

use crate::cfg::builder::FunctionCFGData;
use crate::cfg::serializer::CFGArtifact;
use crate::core::types::cfg::CFGEdgeType;
use crate::scpg::query::csr::{csr_from_triples, row};
use crate::scpg::query::graph::CallGraphIndex;
use crate::scpg::query::source::AstIndex;
use crate::scpg::serializer::codec::{SectionDecoder, SectionEncoder};
use crate::scpg::types::ScpgFormatError;
use crate::ssa::serializer::{FunctionSSAData, SSAArtifact};

pub const NODE_ENTRY: u8 = 0;
pub const NODE_STATEMENT: u8 = 1;
pub const NODE_ACTUAL_IN: u8 = 2;

pub const DEP_DATA: u8 = 0;
pub const DEP_CONTROL: u8 = 1;
pub const DEP_CALL: u8 = 2;
pub const DEP_PARAM_OUT: u8 = 3;

/// `ast_node` value of entry nodes.
pub const ENTRY_AST: u32 = u32::MAX;

pub fn dependence_name(kind: u8) -> &'static str {
    match kind {
        DEP_DATA => "data",
        DEP_CONTROL => "control",
        DEP_CALL => "call",
        _ => "param_out",
    }
}

pub fn node_kind_name(kind: u8) -> &'static str {
    match kind {
        NODE_ENTRY => "entry",
        NODE_STATEMENT => "statement",
        _ => "actual_in",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SdgNode {
    pub method: u32,
    pub kind: u8,
    pub ast_node: u32,
}

impl SdgNode {
    fn entry(method: u32) -> Self {
        Self {
            method,
            kind: NODE_ENTRY,
            ast_node: ENTRY_AST,
        }
    }

    fn statement(method: u32, ast_node: u32) -> Self {
        Self {
            method,
            kind: NODE_STATEMENT,
            ast_node,
        }
    }

    fn actual_in(method: u32, ast_node: u32) -> Self {
        Self {
            method,
            kind: NODE_ACTUAL_IN,
            ast_node,
        }
    }
}

#[derive(Default)]
struct Builder {
    ids: HashMap<SdgNode, u32>,
    nodes: Vec<SdgNode>,
    edges: Vec<(u32, u32, u8)>,
}

impl Builder {
    fn node(&mut self, n: SdgNode) -> u32 {
        if let Some(&id) = self.ids.get(&n) {
            return id;
        }
        let id = self.nodes.len() as u32;
        self.nodes.push(n);
        self.ids.insert(n, id);
        id
    }

    fn lookup(&self, n: SdgNode) -> Option<u32> {
        self.ids.get(&n).copied()
    }

    fn edge(&mut self, to: u32, from: u32, kind: u8) {
        if to != from {
            self.edges.push((to, from, kind));
        }
    }
}

/// Predicate nodes that control block `x`: its last statement, or — for an empty block —
/// the predicates of the blocks controlling it, or the method entry.
#[allow(clippy::too_many_arguments)]
fn block_predicates(
    x: usize,
    cfg: &FunctionCFGData,
    method: u32,
    controllers: &[Vec<u32>],
    builder: &Builder,
    entry: u32,
    memo: &mut [Option<Vec<u32>>],
    in_progress: &mut [bool],
) -> Vec<u32> {
    if let Some(done) = &memo[x] {
        return done.clone();
    }
    if in_progress[x] {
        return vec![entry];
    }
    in_progress[x] = true;
    let result = if let Some(&last) = cfg.blocks[x].stmts.last() {
        builder
            .lookup(SdgNode::statement(method, last))
            .map(|id| vec![id])
            .unwrap_or_else(|| vec![entry])
    } else if controllers[x].is_empty() {
        vec![entry]
    } else {
        let mut acc = Vec::new();
        for &c in &controllers[x] {
            if c as usize == x {
                continue;
            }
            acc.extend(block_predicates(
                c as usize,
                cfg,
                method,
                controllers,
                builder,
                entry,
                memo,
                in_progress,
            ));
        }
        acc.sort_unstable();
        acc.dedup();
        if acc.is_empty() {
            vec![entry]
        } else {
            acc
        }
    };
    in_progress[x] = false;
    memo[x] = Some(result.clone());
    result
}

/// Reaching non-φ definition statements of `ssa_id`, looking through φ-functions.
fn reaching_definitions(
    ssa_id: u32,
    func: &FunctionSSAData,
    phi_args: &HashMap<u32, Vec<u32>>,
    memo: &mut HashMap<u32, Vec<u32>>,
) -> Vec<u32> {
    if let Some(done) = memo.get(&ssa_id) {
        return done.clone();
    }
    let mut defs = Vec::new();
    let mut seen = HashSet::new();
    let mut stack = vec![ssa_id];
    while let Some(v) = stack.pop() {
        if !seen.insert(v) {
            continue;
        }
        let Some(rec) = func.ssa_records.get(v as usize) else {
            continue;
        };
        if rec.is_phi() {
            if let Some(args) = phi_args.get(&v) {
                stack.extend(args.iter().copied().filter(|&a| a != u32::MAX));
            }
        } else if rec.def_stmt != u32::MAX {
            defs.push(rec.def_stmt);
        }
    }
    defs.sort_unstable();
    defs.dedup();
    memo.insert(ssa_id, defs.clone());
    defs
}

/// Statement of `cfg` enclosing the call expression `call_node`: the last statement in
/// pre-order that starts at or before the call and (when token ranges are known) contains it.
fn enclosing_statement(cfg: &FunctionCFGData, call_block: u32, call_node: u32, ast: &AstIndex) -> Option<u32> {
    let contains = |stmt: u32| -> bool {
        match (ast.token_range(stmt), ast.token_range(call_node)) {
            (Some((sf, sl)), Some((cf, cl))) => sf <= cf && cl <= sl,
            _ => true,
        }
    };
    let pick = |stmts: &mut dyn Iterator<Item = u32>| -> Option<u32> {
        stmts.filter(|&s| s <= call_node && contains(s)).max()
    };
    if let Some(block) = cfg.blocks.get(call_block as usize) {
        if let Some(found) = pick(&mut block.stmts.iter().copied()) {
            return Some(found);
        }
    }
    pick(&mut cfg.blocks.iter().flat_map(|b| b.stmts.iter().copied()))
}

fn return_statements(cfg: &FunctionCFGData) -> Vec<u32> {
    let exit = if (cfg.exit_block as usize) < cfg.blocks.len() {
        Some(cfg.exit_block)
    } else {
        cfg.blocks.iter().find(|b| b.is_exit).map(|b| b.id)
    };
    let Some(exit) = exit else {
        return Vec::new();
    };
    let mut returns: Vec<u32> = cfg
        .edges
        .iter()
        .filter(|&&(_, to, kind)| to == exit && kind == CFGEdgeType::Return)
        .filter_map(|&(from, _, _)| cfg.blocks.get(from as usize).and_then(|b| b.stmts.last().copied()))
        .collect();
    if returns.is_empty() {
        returns = cfg
            .edges
            .iter()
            .filter(|&&(_, to, _)| to == exit)
            .filter_map(|&(from, _, _)| cfg.blocks.get(from as usize).and_then(|b| b.stmts.last().copied()))
            .collect();
    }
    returns.sort_unstable();
    returns.dedup();
    returns
}

#[derive(Debug, Clone, Default)]
pub struct SystemDependenceGraph {
    node_methods: Vec<u32>,
    node_kinds: Vec<u8>,
    node_ast: Vec<u32>,
    pred_offsets: Vec<u32>,
    pred_adj: Vec<u32>,
    pred_kinds: Vec<u8>,
    succ_offsets: Vec<u32>,
    succ_adj: Vec<u32>,
    succ_kinds: Vec<u8>,
}

impl SystemDependenceGraph {
    pub fn build(cfa: &CFGArtifact, ssa: &SSAArtifact, cg: &CallGraphIndex, ast: &AstIndex) -> Self {
        let ssa_by_method: HashMap<u32, &FunctionSSAData> =
            ssa.functions.iter().map(|f| (f.sym_id, f)).collect();

        let mut sites_by_caller: HashMap<u32, Vec<usize>> = HashMap::new();
        for (row_idx, site) in cg.call_sites().iter().enumerate() {
            sites_by_caller.entry(site.caller).or_default().push(row_idx);
        }

        let mut b = Builder::default();
        let mut entries: HashMap<u32, u32> = HashMap::new();
        let mut returns_of: HashMap<u32, Vec<u32>> = HashMap::new();
        // (actual_in node, call statement node, call-site row)
        let mut pending_calls: Vec<(u32, u32, usize)> = Vec::new();
        let mut actual_in_of: HashMap<u32, u32> = HashMap::new();

        for cfg in &cfa.functions {
            let m = cfg.sym_id;
            if entries.contains_key(&m) {
                continue;
            }
            let entry = b.node(SdgNode::entry(m));
            entries.insert(m, entry);

            let nblocks = cfg.blocks.len();
            for block in &cfg.blocks {
                for &s in &block.stmts {
                    b.node(SdgNode::statement(m, s));
                }
            }

            // ── Control dependences ──
            let func = ssa_by_method.get(&m).copied();
            let mut controllers: Vec<Vec<u32>> = vec![Vec::new(); nblocks];
            if let Some(f) = func {
                for x in 0..nblocks {
                    for &y in row(&f.cdg.cd_offsets, &f.cdg.cd_adj, x as u32) {
                        if (y as usize) < nblocks {
                            controllers[y as usize].push(x as u32);
                        }
                    }
                }
            }
            let mut memo: Vec<Option<Vec<u32>>> = vec![None; nblocks];
            let mut in_progress = vec![false; nblocks];
            for (y, block) in cfg.blocks.iter().enumerate() {
                let deps: Vec<u32> = if controllers[y].is_empty() {
                    vec![entry]
                } else {
                    let mut acc = Vec::new();
                    for &x in &controllers[y] {
                        acc.extend(block_predicates(
                            x as usize,
                            cfg,
                            m,
                            &controllers,
                            &b,
                            entry,
                            &mut memo,
                            &mut in_progress,
                        ));
                    }
                    acc
                };
                for &s in &block.stmts {
                    let Some(stmt) = b.lookup(SdgNode::statement(m, s)) else {
                        continue;
                    };
                    for &p in &deps {
                        b.edge(stmt, p, DEP_CONTROL);
                    }
                }
            }

            // ── Data dependences ──
            if let Some(f) = func {
                let phi_args: HashMap<u32, Vec<u32>> = f
                    .phi_records
                    .iter()
                    .map(|p| (p.ssa_id, p.args.iter().map(|a| a.arg_ssa_id).collect()))
                    .collect();
                let mut def_memo: HashMap<u32, Vec<u32>> = HashMap::new();
                for rec in &f.ssa_records {
                    let uses = f.def_use.uses_of(rec.ssa_id);
                    if uses.is_empty() {
                        continue;
                    }
                    let defs = reaching_definitions(rec.ssa_id, f, &phi_args, &mut def_memo);
                    let def_nodes: Vec<u32> = defs
                        .iter()
                        .filter_map(|&d| b.lookup(SdgNode::statement(m, d)))
                        .collect();
                    for &u in uses {
                        if u == u32::MAX {
                            continue;
                        }
                        let Some(use_node) = b.lookup(SdgNode::statement(m, u)) else {
                            continue;
                        };
                        for &d in &def_nodes {
                            b.edge(use_node, d, DEP_DATA);
                        }
                    }
                }
            }

            returns_of.insert(
                m,
                return_statements(cfg)
                    .into_iter()
                    .filter_map(|r| b.lookup(SdgNode::statement(m, r)))
                    .collect(),
            );

            // ── Call sites of this method ──
            for &site_row in sites_by_caller.get(&m).map(|v| v.as_slice()).unwrap_or(&[]) {
                let site = &cg.call_sites()[site_row];
                let (stmt_node, anchor_ast) =
                    match enclosing_statement(cfg, site.call_block, site.call_node, ast) {
                        Some(s) => match b.lookup(SdgNode::statement(m, s)) {
                            Some(id) => (id, s),
                            None => continue,
                        },
                        None => {
                            // The call is not inside any CFG statement (e.g. a field
                            // initializer): model the call expression as its own statement,
                            // controlled by its block's predicates or the entry.
                            let id = b.node(SdgNode::statement(m, site.call_node));
                            let block = site.call_block as usize;
                            let deps = if block < nblocks && !controllers[block].is_empty() {
                                let mut acc = Vec::new();
                                for &x in &controllers[block] {
                                    acc.extend(block_predicates(
                                        x as usize,
                                        cfg,
                                        m,
                                        &controllers,
                                        &b,
                                        entry,
                                        &mut memo,
                                        &mut in_progress,
                                    ));
                                }
                                acc
                            } else {
                                vec![entry]
                            };
                            for p in deps {
                                b.edge(id, p, DEP_CONTROL);
                            }
                            (id, site.call_node)
                        }
                    };
                let actual_in = b.node(SdgNode::actual_in(m, anchor_ast));
                actual_in_of.insert(stmt_node, actual_in);
                pending_calls.push((actual_in, stmt_node, site_row));
            }
        }

        // ── Inter-procedural edges ──
        for &(actual_in, stmt_node, site_row) in &pending_calls {
            for &callee in cg.site_targets(site_row) {
                if let Some(&callee_entry) = entries.get(&callee) {
                    b.edge(callee_entry, actual_in, DEP_CALL);
                }
                if let Some(rets) = returns_of.get(&callee) {
                    for &r in rets {
                        b.edge(stmt_node, r, DEP_PARAM_OUT);
                    }
                }
            }
        }

        // ActualIn nodes inherit the intra-procedural dependences of their call statement.
        let inherited: Vec<(u32, u32, u8)> = b
            .edges
            .iter()
            .filter(|&&(_, _, k)| k == DEP_DATA || k == DEP_CONTROL)
            .filter_map(|&(to, from, k)| actual_in_of.get(&to).map(|&ai| (ai, from, k)))
            .collect();
        for (to, from, k) in inherited {
            b.edge(to, from, k);
        }

        Self::finalize(b)
    }

    fn finalize(b: Builder) -> Self {
        let n = b.nodes.len();
        let mut order: Vec<u32> = (0..n as u32).collect();
        order.sort_by_key(|&i| b.nodes[i as usize]);
        let mut new_id = vec![0u32; n];
        for (new, &old) in order.iter().enumerate() {
            new_id[old as usize] = new as u32;
        }
        let node_methods = order.iter().map(|&i| b.nodes[i as usize].method).collect();
        let node_kinds = order.iter().map(|&i| b.nodes[i as usize].kind).collect();
        let node_ast = order.iter().map(|&i| b.nodes[i as usize].ast_node).collect();

        let pred_triples: Vec<(u32, u32, u8)> = b
            .edges
            .iter()
            .map(|&(to, from, k)| (new_id[to as usize], new_id[from as usize], k))
            .collect();
        Self::from_parts(node_methods, node_kinds, node_ast, pred_triples)
    }

    fn from_parts(
        node_methods: Vec<u32>,
        node_kinds: Vec<u8>,
        node_ast: Vec<u32>,
        pred_triples: Vec<(u32, u32, u8)>,
    ) -> Self {
        let n = node_methods.len();
        let succ_triples: Vec<(u32, u32, u8)> = pred_triples.iter().map(|&(t, f, k)| (f, t, k)).collect();
        let (pred_offsets, pred_adj, pred_kinds) = csr_from_triples(n, pred_triples, false);
        let (succ_offsets, succ_adj, succ_kinds) = csr_from_triples(n, succ_triples, false);
        Self {
            node_methods,
            node_kinds,
            node_ast,
            pred_offsets,
            pred_adj,
            pred_kinds,
            succ_offsets,
            succ_adj,
            succ_kinds,
        }
    }

    pub fn node_count(&self) -> usize {
        self.node_methods.len()
    }

    pub fn edge_count(&self) -> usize {
        self.pred_adj.len()
    }

    pub fn node(&self, id: u32) -> Option<SdgNode> {
        let i = id as usize;
        Some(SdgNode {
            method: *self.node_methods.get(i)?,
            kind: self.node_kinds[i],
            ast_node: self.node_ast[i],
        })
    }

    fn key(&self, i: usize) -> (u32, u8, u32) {
        (self.node_methods[i], self.node_kinds[i], self.node_ast[i])
    }

    pub fn find(&self, method: u32, kind: u8, ast_node: u32) -> Option<u32> {
        let probe = (method, kind, ast_node);
        let n = self.node_count();
        let pos = partition_point_by(n, |i| self.key(i) < probe);
        if pos < n && self.key(pos) == probe {
            Some(pos as u32)
        } else {
            None
        }
    }

    pub fn entry_of(&self, method: u32) -> Option<u32> {
        self.find(method, NODE_ENTRY, ENTRY_AST)
    }

    pub fn statement(&self, method: u32, ast_node: u32) -> Option<u32> {
        self.find(method, NODE_STATEMENT, ast_node)
    }

    /// All node ids belonging to `method` (contiguous because nodes are sorted by method).
    pub fn method_nodes(&self, method: u32) -> std::ops::Range<u32> {
        let n = self.node_count();
        let lo = partition_point_by(n, |i| self.node_methods[i] < method);
        let hi = partition_point_by(n, |i| self.node_methods[i] <= method);
        lo as u32..hi.max(lo) as u32
    }

    pub fn statements_of(&self, method: u32) -> Vec<u32> {
        self.method_nodes(method)
            .filter(|&i| self.node_kinds[i as usize] == NODE_STATEMENT)
            .collect()
    }

    pub fn has_method(&self, method: u32) -> bool {
        self.entry_of(method).is_some()
    }

    /// Methods containing a statement for BP-AST node `ast_node`.
    pub fn methods_with_statement(&self, ast_node: u32) -> Vec<u32> {
        let mut out: Vec<u32> = (0..self.node_count())
            .filter(|&i| self.node_kinds[i] == NODE_STATEMENT && self.node_ast[i] == ast_node)
            .map(|i| self.node_methods[i])
            .collect();
        out.dedup();
        out
    }

    /// Nodes `id` depends on, with dependence kinds.
    pub fn preds(&self, id: u32) -> (&[u32], &[u8]) {
        (
            row(&self.pred_offsets, &self.pred_adj, id),
            row(&self.pred_offsets, &self.pred_kinds, id),
        )
    }

    /// Nodes that depend on `id`, with dependence kinds.
    pub fn succs(&self, id: u32) -> (&[u32], &[u8]) {
        (
            row(&self.succ_offsets, &self.succ_adj, id),
            row(&self.succ_offsets, &self.succ_kinds, id),
        )
    }

    pub fn encode(&self, enc: &mut SectionEncoder) {
        enc.put_u32s(&self.node_methods);
        enc.put_u8s(&self.node_kinds);
        enc.put_u32s(&self.node_ast);
        enc.put_u32s(&self.pred_offsets);
        enc.put_u32s(&self.pred_adj);
        enc.put_u8s(&self.pred_kinds);
    }

    pub fn decode(dec: &mut SectionDecoder<'_>) -> Result<Self, ScpgFormatError> {
        let node_methods = dec.u32s()?;
        let node_kinds = dec.u8s()?.to_vec();
        let node_ast = dec.u32s()?;
        let n = node_methods.len();
        dec.expect_len("node_kinds", node_kinds.len(), n)?;
        dec.expect_len("node_ast", node_ast.len(), n)?;
        if node_kinds.iter().any(|&k| k > NODE_ACTUAL_IN) {
            return Err(dec.malformed("unknown SDG node kind"));
        }
        for i in 1..n {
            let prev = (node_methods[i - 1], node_kinds[i - 1], node_ast[i - 1]);
            let cur = (node_methods[i], node_kinds[i], node_ast[i]);
            if prev >= cur {
                return Err(dec.malformed("SDG nodes are not strictly sorted"));
            }
        }
        let pred_offsets = dec.u32s()?;
        let pred_adj = dec.u32s()?;
        let pred_kinds = dec.u8s()?.to_vec();
        dec.check_csr("sdg_preds", &pred_offsets, &pred_adj, n, Some(n))?;
        dec.expect_len("sdg_pred_kinds", pred_kinds.len(), pred_adj.len())?;
        if pred_kinds.iter().any(|&k| k > DEP_PARAM_OUT) {
            return Err(dec.malformed("unknown SDG dependence kind"));
        }
        let mut triples = Vec::with_capacity(pred_adj.len());
        for to in 0..n as u32 {
            for (&from, &k) in row(&pred_offsets, &pred_adj, to)
                .iter()
                .zip(row(&pred_offsets, &pred_kinds, to))
            {
                triples.push((to, from, k));
            }
        }
        Ok(Self::from_parts(node_methods, node_kinds, node_ast, triples))
    }
}

/// `partition_point` over the index range `0..n`.
fn partition_point_by(n: usize, pred: impl Fn(usize) -> bool) -> usize {
    let (mut lo, mut hi) = (0usize, n);
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if pred(mid) {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    lo
}
