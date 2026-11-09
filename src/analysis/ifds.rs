//! Interprocedural, finite, distributive, subset problems (Reps, Horwitz & Sagiv 1995),
//! solved by the tabulation algorithm with the worklist refinements of Naeem, Lhoták &
//! Rodriguez (2010).
//!
//! **Supergraph.** Nodes are program points of the IR: `(function, block, i)` is the point
//! before instruction `i`; `i == insts.len()` is the terminator. Every function also has one
//! virtual exceptional exit. Edges:
//!
//! ```text
//! normal          (f, b, i) → (f, b, i+1)              non-call instruction
//!                 (f, b, len) → (f, s, 0)              terminator successor (Jump, Branch, Switch)
//!                 (f, b, len) → (f, EXC, 0)            Throw
//! call            (f, b, i) → start(g)                 Call whose site has callee g in the call graph
//! return          exit(g) → return site                Return → (f, b, i+1);
//!                                                      exceptional exit → (f, h, 0) for the unwind
//!                                                      handler h, or (f, EXC, 0) without one
//! call-to-return  (f, b, i) → each return site         facts the call does not touch
//! ```
//!
//! The call edges come from `crate::memory::callgraph::CallGraph`; a call site the call graph
//! does not resolve goes through [`IfdsProblem::unresolved_call_flow`].
//!
//! **Tabulation.** A path edge `⟨s_p, d₁⟩ → ⟨n, d₂⟩` states that `d₂` holds at `n` along a
//! realizable path from the start of `n`'s function entered with `d₁`. End summaries record
//! `⟨s_p, d₁⟩ → ⟨exit, d₂⟩`; incoming sets record, per callee start fact, the caller path
//! edges that reached it. A new end summary is applied to every recorded caller and a new
//! caller to every recorded end summary, which yields the summary edges
//! `⟨call, d⟩ → ⟨return site, d'⟩` without recomputing the callee.
//!
//! **Zero fact.** The solver propagates `Λ` along every edge itself; flow functions only add
//! the facts they generate from `Λ` and transform the others.

use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::hash::Hash;

use crate::analysis::solver::{CfgEdge, EdgeKind, IrCfg};
use crate::ir::{CallSiteId, FuncId, FuncIr, Inst, ModuleIr, Terminator};
use crate::memory::callgraph::CallGraph;

// ── Supergraph ───────────────────────────────────────────────────────────────

/// Block value of the virtual exceptional exit node of a function.
pub const EXCEPTIONAL_EXIT: u32 = u32::MAX;

/// A supergraph node.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Node {
    /// Index of the function in `ModuleIr::funcs`.
    pub func: u32,
    /// Block position, or [`EXCEPTIONAL_EXIT`].
    pub block: u32,
    /// Instruction index; the block's instruction count denotes its terminator.
    pub inst: u32,
}

impl Node {
    pub fn is_exceptional_exit(&self) -> bool {
        self.block == EXCEPTIONAL_EXIT
    }
}

/// How control returns from a callee.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ReturnKind {
    Normal,
    Exceptional,
}

/// A return site of a call node.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ReturnSite {
    pub node: Node,
    pub kind: ReturnKind,
}

/// The interprocedural control-flow graph of a module.
pub struct Supergraph<'m> {
    pub module: &'m ModuleIr,
    /// Per function index.
    pub cfgs: Vec<IrCfg>,
    /// Whether exceptional returns and `Throw` edges take part.
    pub exceptional_flow: bool,
    func_index: HashMap<FuncId, u32>,
    callees: HashMap<CallSiteId, Vec<u32>>,
    call_nodes: HashMap<CallSiteId, Node>,
    callers: Vec<Vec<Node>>,
    reachable: Vec<bool>,
}

impl<'m> Supergraph<'m> {
    pub fn build(module: &'m ModuleIr, callgraph: &CallGraph, exceptional_flow: bool) -> Supergraph<'m> {
        let func_index: HashMap<FuncId, u32> = module
            .funcs
            .iter()
            .enumerate()
            .map(|(i, f)| (f.id, i as u32))
            .collect();
        let cfgs: Vec<IrCfg> = module.funcs.iter().map(IrCfg::build).collect();

        let mut call_nodes: HashMap<CallSiteId, Node> = HashMap::new();
        for (fi, func) in module.funcs.iter().enumerate() {
            for (b, block) in func.blocks.iter().enumerate() {
                for (i, inst) in block.insts.iter().enumerate() {
                    if let Inst::Call { site, .. } = inst {
                        call_nodes.insert(
                            *site,
                            Node {
                                func: fi as u32,
                                block: b as u32,
                                inst: i as u32,
                            },
                        );
                    }
                }
            }
        }

        let mut callees: HashMap<CallSiteId, Vec<u32>> = HashMap::new();
        let mut callers: Vec<Vec<Node>> = vec![Vec::new(); module.funcs.len()];
        for (site, target) in &callgraph.edges {
            let Some(&ti) = func_index.get(target) else {
                continue;
            };
            let list = callees.entry(*site).or_default();
            if !list.contains(&ti) {
                list.push(ti);
                if let Some(&node) = call_nodes.get(site) {
                    callers[ti as usize].push(node);
                }
            }
        }
        let mut reachable = vec![false; module.funcs.len()];
        for f in &callgraph.reachable {
            if let Some(&i) = func_index.get(f) {
                reachable[i as usize] = true;
            }
        }

        Supergraph {
            module,
            cfgs,
            exceptional_flow,
            func_index,
            callees,
            call_nodes,
            callers,
            reachable,
        }
    }

    pub fn func(&self, f: u32) -> &'m FuncIr {
        &self.module.funcs[f as usize]
    }

    pub fn func_index(&self, id: FuncId) -> Option<u32> {
        self.func_index.get(&id).copied()
    }

    /// True when the call graph marks the function reachable.
    pub fn is_reachable(&self, f: u32) -> bool {
        self.reachable.get(f as usize).copied().unwrap_or(false)
    }

    /// The start node of a function with a body.
    pub fn start(&self, f: u32) -> Option<Node> {
        let cfg = self.cfgs.get(f as usize)?;
        if cfg.is_empty() {
            return None;
        }
        Some(Node {
            func: f,
            block: cfg.entry,
            inst: 0,
        })
    }

    /// The instruction at `n`, if `n` is not a terminator or the exceptional exit.
    pub fn inst_at(&self, n: Node) -> Option<&'m Inst> {
        if n.is_exceptional_exit() {
            return None;
        }
        self.func(n.func)
            .blocks
            .get(n.block as usize)?
            .insts
            .get(n.inst as usize)
    }

    /// The terminator at `n`, if `n` is a terminator node.
    pub fn terminator_at(&self, n: Node) -> Option<&'m Terminator> {
        if n.is_exceptional_exit() {
            return None;
        }
        let block = self.func(n.func).blocks.get(n.block as usize)?;
        if n.inst as usize == block.insts.len() {
            Some(&block.term)
        } else {
            None
        }
    }

    /// The call site of a call node.
    pub fn call_site_at(&self, n: Node) -> Option<CallSiteId> {
        match self.inst_at(n)? {
            Inst::Call { site, .. } => Some(*site),
            _ => None,
        }
    }

    /// The node of a call site.
    pub fn call_node(&self, site: CallSiteId) -> Option<Node> {
        self.call_nodes.get(&site).copied()
    }

    /// Function indices the call graph resolves `site` to.
    pub fn callees_of(&self, site: CallSiteId) -> &[u32] {
        self.callees.get(&site).map_or(&[], |v| v.as_slice())
    }

    /// Call nodes that may invoke function `f`.
    pub fn callers_of(&self, f: u32) -> &[Node] {
        self.callers.get(f as usize).map_or(&[], |v| v.as_slice())
    }

    /// How control leaves the function at exit node `n`, if it is one.
    pub fn exit_kind(&self, n: Node) -> Option<ReturnKind> {
        if n.is_exceptional_exit() {
            return Some(ReturnKind::Exceptional);
        }
        match self.terminator_at(n)? {
            Terminator::Return(_) => Some(ReturnKind::Normal),
            _ => None,
        }
    }

    /// The return sites of call node `call`.
    pub fn return_sites(&self, call: Node) -> Vec<ReturnSite> {
        let mut out = vec![ReturnSite {
            node: Node {
                inst: call.inst + 1,
                ..call
            },
            kind: ReturnKind::Normal,
        }];
        if !self.exceptional_flow {
            return out;
        }
        let cfg = &self.cfgs[call.func as usize];
        let exceptional = match cfg.unwind_edge(call.block, call.inst) {
            Some(e) => Node {
                func: call.func,
                block: cfg.edge(e).to,
                inst: 0,
            },
            None => Node {
                func: call.func,
                block: EXCEPTIONAL_EXIT,
                inst: 0,
            },
        };
        out.push(ReturnSite {
            node: exceptional,
            kind: ReturnKind::Exceptional,
        });
        out
    }

    /// Intraprocedural successors of a non-call node, with the CFG edge for terminator
    /// successors.
    pub fn successors(&self, n: Node) -> Vec<(Node, Option<u32>)> {
        if n.is_exceptional_exit() {
            return Vec::new();
        }
        let func = self.func(n.func);
        let Some(block) = func.blocks.get(n.block as usize) else {
            return Vec::new();
        };
        if (n.inst as usize) < block.insts.len() {
            return vec![(
                Node {
                    inst: n.inst + 1,
                    ..n
                },
                None,
            )];
        }
        let cfg = &self.cfgs[n.func as usize];
        let mut out = Vec::new();
        for &e in &cfg.succ_edges[n.block as usize] {
            let edge = cfg.edge(e);
            if edge.kind.is_unwind() {
                continue;
            }
            out.push((
                Node {
                    func: n.func,
                    block: edge.to,
                    inst: 0,
                },
                Some(e),
            ));
        }
        if self.exceptional_flow && matches!(block.term, Terminator::Throw(_)) {
            out.push((
                Node {
                    func: n.func,
                    block: EXCEPTIONAL_EXIT,
                    inst: 0,
                },
                None,
            ));
        }
        out
    }

    /// The CFG edge `e` of function `f`.
    pub fn edge(&self, f: u32, e: u32) -> &CfgEdge {
        self.cfgs[f as usize].edge(e)
    }

    /// True when `edge` is the `True` or `False` edge of a branch (convenience for
    /// branch-sensitive flow functions).
    pub fn is_branch_edge(edge: &CfgEdge) -> bool {
        matches!(edge.kind, EdgeKind::True | EdgeKind::False)
    }
}

// ── Problem interface ────────────────────────────────────────────────────────

/// An IFDS problem: flow functions over a finite fact domain with a zero fact `Λ`.
pub trait IfdsProblem {
    type Fact: Clone + Eq + Hash + fmt::Debug;

    /// The zero fact `Λ`.
    fn zero(&self) -> Self::Fact;

    /// Initial `(node, fact)` pairs; each node should be a function start node.
    fn seeds(&self, sg: &Supergraph<'_>) -> Vec<(Node, Self::Fact)>;

    /// Facts at `to` generated from `fact` at `from` along an intraprocedural edge.
    fn normal_flow(
        &self,
        sg: &Supergraph<'_>,
        from: Node,
        to: Node,
        edge: Option<&CfgEdge>,
        fact: &Self::Fact,
    ) -> Vec<Self::Fact>;

    /// Facts at the start of `callee` generated from `fact` at `call`.
    fn call_flow(&self, sg: &Supergraph<'_>, call: Node, callee: u32, fact: &Self::Fact) -> Vec<Self::Fact>;

    /// Facts at return site `ret` generated from `fact` at exit node `exit` of `callee`.
    fn return_flow(
        &self,
        sg: &Supergraph<'_>,
        call: Node,
        callee: u32,
        exit: Node,
        ret: ReturnSite,
        fact: &Self::Fact,
    ) -> Vec<Self::Fact>;

    /// Facts at return site `ret` generated from `fact` at `call`, bypassing the callee.
    fn call_to_return_flow(
        &self,
        sg: &Supergraph<'_>,
        call: Node,
        ret: ReturnSite,
        fact: &Self::Fact,
    ) -> Vec<Self::Fact>;

    /// Flow across a call the call graph does not resolve. The default treats it like the
    /// call-to-return flow; a problem that must be sound for unknown code overrides it.
    fn unresolved_call_flow(
        &self,
        sg: &Supergraph<'_>,
        call: Node,
        ret: ReturnSite,
        fact: &Self::Fact,
    ) -> Vec<Self::Fact> {
        self.call_to_return_flow(sg, call, ret, fact)
    }
}

/// Solver configuration.
#[derive(Clone, Copy, Debug)]
pub struct IfdsConfig {
    /// Stop after this many path edges (the result is then incomplete).
    pub max_path_edges: usize,
    /// When an exit is reached in a function entered only through a seed, continue into
    /// every caller the call graph knows (unbalanced returns).
    pub follow_returns_past_seeds: bool,
}

impl Default for IfdsConfig {
    fn default() -> Self {
        Self {
            max_path_edges: 50_000_000,
            follow_returns_past_seeds: false,
        }
    }
}

/// Interning table for facts; id `0` is always `Λ`.
pub struct FactTable<F> {
    facts: Vec<F>,
    index: HashMap<F, u32>,
}

impl<F: Clone + Eq + Hash> FactTable<F> {
    pub fn new(zero: F) -> Self {
        let mut index = HashMap::new();
        index.insert(zero.clone(), 0);
        Self {
            facts: vec![zero],
            index,
        }
    }

    pub fn intern(&mut self, f: F) -> u32 {
        if let Some(&id) = self.index.get(&f) {
            return id;
        }
        let id = self.facts.len() as u32;
        self.facts.push(f.clone());
        self.index.insert(f, id);
        id
    }

    pub fn get(&self, id: u32) -> &F {
        &self.facts[id as usize]
    }

    pub fn into_vec(self) -> Vec<F> {
        self.facts
    }
}

const ZERO: u32 = 0;

/// Results of an IFDS solve.
#[derive(Clone, Debug)]
pub struct IfdsResults<F> {
    /// All facts by id (`0` is `Λ`).
    pub facts: Vec<F>,
    /// Fact ids holding at each node (over all calling contexts), sorted; `Λ` excluded.
    pub at_node: HashMap<Node, Vec<u32>>,
    /// Summary edges `(call, fact at call, return site, fact at return site)`.
    pub summaries: Vec<(Node, u32, Node, u32)>,
    pub path_edges: usize,
    /// False when the path-edge budget was exhausted.
    pub complete: bool,
}

impl<F> IfdsResults<F> {
    /// The facts holding at `n`.
    pub fn facts_at(&self, n: Node) -> impl Iterator<Item = &F> {
        self.at_node
            .get(&n)
            .into_iter()
            .flat_map(move |ids| ids.iter().map(move |&i| &self.facts[i as usize]))
    }
}

impl<F: PartialEq> IfdsResults<F> {
    pub fn holds(&self, n: Node, fact: &F) -> bool {
        self.facts_at(n).any(|f| f == fact)
    }
}

// ── Solver ───────────────────────────────────────────────────────────────────

/// The tabulation solver.
pub struct IfdsSolver<'s, 'm, P: IfdsProblem> {
    sg: &'s Supergraph<'m>,
    problem: &'s P,
    config: IfdsConfig,
    facts: FactTable<P::Fact>,
    path_edges: HashSet<(u32, Node, u32)>,
    worklist: VecDeque<(u32, Node, u32)>,
    /// `(func, d₁) → {(exit, d₂)}`.
    end_summary: HashMap<(u32, u32), HashSet<(Node, u32)>>,
    /// `(callee, d₃) → {(call, d₂ at call, d₁ of the caller's path edge)}`.
    incoming: HashMap<(u32, u32), HashSet<(Node, u32, u32)>>,
    summaries: HashSet<(Node, u32, Node, u32)>,
    /// Start facts that came from seeds rather than calls.
    seeded: HashSet<(u32, u32)>,
    complete: bool,
}

impl<'s, 'm, P: IfdsProblem> IfdsSolver<'s, 'm, P> {
    pub fn new(sg: &'s Supergraph<'m>, problem: &'s P, config: IfdsConfig) -> Self {
        Self {
            sg,
            problem,
            config,
            facts: FactTable::new(problem.zero()),
            path_edges: HashSet::new(),
            worklist: VecDeque::new(),
            end_summary: HashMap::new(),
            incoming: HashMap::new(),
            summaries: HashSet::new(),
            seeded: HashSet::new(),
            complete: true,
        }
    }

    fn propagate(&mut self, d1: u32, n: Node, d2: u32) {
        if !self.complete {
            return;
        }
        if self.path_edges.len() >= self.config.max_path_edges {
            self.complete = false;
            return;
        }
        if self.path_edges.insert((d1, n, d2)) {
            self.worklist.push_back((d1, n, d2));
        }
    }

    /// Interns the flow function's output and adds `Λ` when the input is `Λ`.
    fn outputs(&mut self, input: u32, produced: Vec<P::Fact>) -> Vec<u32> {
        let mut out: Vec<u32> = produced.into_iter().map(|f| self.facts.intern(f)).collect();
        if input == ZERO && !out.contains(&ZERO) {
            out.push(ZERO);
        }
        out
    }

    /// Runs the tabulation to completion (or to the path-edge budget).
    pub fn solve(mut self) -> IfdsResults<P::Fact> {
        let sg = self.sg;
        for (node, fact) in self.problem.seeds(sg) {
            let d = self.facts.intern(fact);
            self.seeded.insert((node.func, d));
            self.propagate(d, node, d);
            if d != ZERO {
                self.seeded.insert((node.func, ZERO));
                self.propagate(ZERO, node, ZERO);
            }
        }

        while let Some((d1, n, d2)) = self.worklist.pop_front() {
            if !self.complete {
                break;
            }
            if sg.call_site_at(n).is_some() {
                self.process_call(d1, n, d2);
            } else if sg.exit_kind(n).is_some() {
                self.process_exit(d1, n, d2);
            } else {
                self.process_normal(d1, n, d2);
            }
        }
        self.finish()
    }

    fn process_normal(&mut self, d1: u32, n: Node, d2: u32) {
        let sg = self.sg;
        for (m, edge) in sg.successors(n) {
            let edge_ref = edge.map(|e| sg.edge(n.func, e));
            let fact = self.facts.get(d2).clone();
            let produced = self.problem.normal_flow(sg, n, m, edge_ref, &fact);
            for d3 in self.outputs(d2, produced) {
                self.propagate(d1, m, d3);
            }
        }
    }

    fn process_call(&mut self, d1: u32, n: Node, d2: u32) {
        let sg = self.sg;
        let Some(site) = sg.call_site_at(n) else {
            return;
        };
        let returns = sg.return_sites(n);
        let fact = self.facts.get(d2).clone();
        let callees: Vec<u32> = sg
            .callees_of(site)
            .iter()
            .copied()
            .filter(|&q| sg.start(q).is_some())
            .collect();

        if callees.is_empty() {
            for r in &returns {
                let produced = self.problem.unresolved_call_flow(sg, n, *r, &fact);
                for d3 in self.outputs(d2, produced) {
                    self.propagate(d1, r.node, d3);
                }
            }
            return;
        }

        for &q in &callees {
            let Some(start) = sg.start(q) else {
                continue;
            };
            let produced = self.problem.call_flow(sg, n, q, &fact);
            for d3 in self.outputs(d2, produced) {
                self.propagate(d3, start, d3);
                let fresh = self.incoming.entry((q, d3)).or_default().insert((n, d2, d1));
                if fresh {
                    let ends: Vec<(Node, u32)> = self
                        .end_summary
                        .get(&(q, d3))
                        .map(|s| s.iter().copied().collect())
                        .unwrap_or_default();
                    for (exit, d4) in ends {
                        self.apply_return(n, d2, d1, q, exit, d4);
                    }
                }
            }
        }
        for r in &returns {
            let produced = self.problem.call_to_return_flow(sg, n, *r, &fact);
            for d3 in self.outputs(d2, produced) {
                self.propagate(d1, r.node, d3);
            }
        }
    }

    /// Applies the callee exit fact `d4` (at `exit` of `q`) to the caller path edge
    /// `(d1, call, d2)`.
    fn apply_return(&mut self, call: Node, d2: u32, d1: u32, q: u32, exit: Node, d4: u32) {
        let sg = self.sg;
        let Some(kind) = sg.exit_kind(exit) else {
            return;
        };
        let fact = self.facts.get(d4).clone();
        for r in sg.return_sites(call) {
            if r.kind != kind {
                continue;
            }
            let produced = self.problem.return_flow(sg, call, q, exit, r, &fact);
            for d5 in self.outputs(d4, produced) {
                self.summaries.insert((call, d2, r.node, d5));
                self.propagate(d1, r.node, d5);
            }
        }
    }

    fn process_exit(&mut self, d1: u32, n: Node, d2: u32) {
        let sg = self.sg;
        let p = n.func;
        if !self.end_summary.entry((p, d1)).or_default().insert((n, d2)) {
            return;
        }
        let callers: Vec<(Node, u32, u32)> = self
            .incoming
            .get(&(p, d1))
            .map(|s| s.iter().copied().collect())
            .unwrap_or_default();
        for (call, d_call, d1_caller) in callers {
            self.apply_return(call, d_call, d1_caller, p, n, d2);
        }
        if self.config.follow_returns_past_seeds && self.seeded.contains(&(p, d1)) {
            let Some(kind) = sg.exit_kind(n) else {
                return;
            };
            let fact = self.facts.get(d2).clone();
            for &call in sg.callers_of(p) {
                for r in sg.return_sites(call) {
                    if r.kind != kind {
                        continue;
                    }
                    let produced = self.problem.return_flow(sg, call, p, n, r, &fact);
                    for d5 in self.outputs(d2, produced) {
                        self.seeded.insert((call.func, ZERO));
                        self.propagate(ZERO, r.node, d5);
                        if d5 != ZERO {
                            self.propagate(ZERO, r.node, ZERO);
                        }
                    }
                }
            }
        }
    }

    fn finish(self) -> IfdsResults<P::Fact> {
        let mut at_node: HashMap<Node, Vec<u32>> = HashMap::new();
        for &(_, n, d2) in &self.path_edges {
            if d2 != ZERO {
                at_node.entry(n).or_default().push(d2);
            }
        }
        for ids in at_node.values_mut() {
            ids.sort_unstable();
            ids.dedup();
        }
        let mut summaries: Vec<(Node, u32, Node, u32)> = self.summaries.into_iter().collect();
        summaries.sort_unstable();
        IfdsResults {
            path_edges: self.path_edges.len(),
            facts: self.facts.into_vec(),
            at_node,
            summaries,
            complete: self.complete,
        }
    }
}
