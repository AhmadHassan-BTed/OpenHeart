//! Compiler views straight from the analysis facts: the control-flow graph with its
//! dominator tree, the SSA def-use data-flow graph, the control dependence graph, the call
//! graph with its strongly connected components and the ROBDD path summaries.
//!
//! `FlowGraph` is the per-function control-flow structure every behavioural view shares:
//! typed edges (branch arms, switch cases, unwind and raise edges), reverse post-order,
//! dominators (Cooper, Harvey & Kennedy 2001), post-dominators over a virtual exit,
//! natural loops with their exits and follow blocks, irreducibility, and control
//! dependence (Ferrante, Ottenstein & Warren 1987). It is computed from the IR alone.

use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};

use crate::derive::model::{
    CompartmentEntry, CompartmentKind, Derivation, DiagramEdge, DiagramKind, DiagramModel,
    DiagramNode, EdgeKind, ElementId, IdBuilder, NodeKind,
};
use crate::derive::provenance::{
    block_span, call_site_span, const_text, func_span, inst_span, term_span, Fact, ValueRenderer,
};
use crate::derive::{cap, DeriveContext};
use crate::ir::func::{DefSite, FuncIr};
use crate::ir::inst::{Inst, Terminator};
use crate::ir::types::{BlockId, FuncId, Span};
use crate::ir::value::Operand;
use crate::memory::callgraph::{CallbackOrigin, EdgePrecision, ExternalTarget};

// ── Flow graph ───────────────────────────────────────────────────────────────

/// Kind of a control-flow edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FlowLabel {
    /// `Jump`.
    Always,
    /// `Branch` taken when the condition holds.
    True,
    /// `Branch` taken when the condition does not hold.
    False,
    /// `Switch` case `k` (index into `cases`).
    Case(usize),
    /// `Switch` default.
    Default,
    /// Landing block of the call at instruction `k`.
    Unwind(usize),
    /// Landing block of a `Throw`.
    Raise,
}

impl FlowLabel {
    #[inline]
    pub fn is_exceptional(self) -> bool {
        matches!(self, FlowLabel::Unwind(_) | FlowLabel::Raise)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlowEdge {
    pub from: usize,
    pub to: usize,
    pub label: FlowLabel,
}

/// How a block leaves the function.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitKind {
    Return,
    /// `Throw` without a landing block.
    Raise,
    Unreachable,
}

/// A natural loop: a header and the blocks of its back edges' cycles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NaturalLoop {
    pub header: usize,
    /// Body blocks, header included, sorted.
    pub body: Vec<usize>,
    /// Sources of the back edges.
    pub latches: Vec<usize>,
    /// Edges leaving the body: (edge index).
    pub exits: Vec<usize>,
    /// Block where control continues after the loop (see `FlowGraph::build`).
    pub follow: Option<usize>,
    /// Innermost enclosing loop.
    pub parent: Option<usize>,
    pub depth: usize,
}

impl NaturalLoop {
    #[inline]
    pub fn contains(&self, block: usize) -> bool {
        self.body.binary_search(&block).is_ok()
    }
}

/// Control-flow structure of one function. Blocks are addressed by dense index
/// (`BlockId::index`); index `len` is the virtual exit of the post-dominator tree.
#[derive(Debug, Clone)]
pub struct FlowGraph {
    pub func: FuncId,
    pub len: usize,
    pub entry: usize,
    pub edges: Vec<FlowEdge>,
    /// Outgoing edge indices per block (terminator edges first, then unwind edges).
    pub succs: Vec<Vec<usize>>,
    /// Incoming edge indices per block.
    pub preds: Vec<Vec<usize>>,
    pub exit_kind: Vec<Option<ExitKind>>,
    pub rpo: Vec<usize>,
    /// Position in `rpo`; `usize::MAX` for unreachable blocks.
    pub rpo_pos: Vec<usize>,
    pub idom: Vec<Option<usize>>,
    /// Immediate post-dominator per block; `Some(len)` is the virtual exit. Index `len`
    /// holds the exit itself.
    pub ipdom: Vec<Option<usize>>,
    pub loops: Vec<NaturalLoop>,
    pub loop_of_header: Vec<Option<usize>>,
    pub innermost_loop: Vec<Option<usize>>,
    /// A cycle exists whose entry does not dominate it (no natural loop describes it).
    pub irreducible: bool,
    /// Reachable blocks with no path to a function exit; they are given a virtual edge to
    /// the exit so post-dominance is defined for them.
    pub exitless: Vec<usize>,
}

impl FlowGraph {
    pub fn build(func: &FuncIr) -> FlowGraph {
        let n = func.blocks.len();
        let mut edges: Vec<FlowEdge> = Vec::new();
        let mut succs: Vec<Vec<usize>> = vec![Vec::new(); n];
        let mut preds: Vec<Vec<usize>> = vec![Vec::new(); n];
        let mut exit_kind: Vec<Option<ExitKind>> = vec![None; n];

        let mut push = |edges: &mut Vec<FlowEdge>, from: usize, to: BlockId, label: FlowLabel| {
            let to = to.index();
            if to < n {
                let idx = edges.len();
                edges.push(FlowEdge { from, to, label });
                succs[from].push(idx);
                preds[to].push(idx);
            }
        };
        for (b, block) in func.blocks.iter().enumerate() {
            match &block.term {
                Terminator::Jump(t) => push(&mut edges, b, *t, FlowLabel::Always),
                Terminator::Branch { then_b, else_b, .. } => {
                    push(&mut edges, b, *then_b, FlowLabel::True);
                    push(&mut edges, b, *else_b, FlowLabel::False);
                }
                Terminator::Switch { cases, default, .. } => {
                    for (k, (_, t)) in cases.iter().enumerate() {
                        push(&mut edges, b, *t, FlowLabel::Case(k));
                    }
                    push(&mut edges, b, *default, FlowLabel::Default);
                }
                Terminator::Throw(_) => match block.unwind {
                    Some(u) => push(&mut edges, b, u, FlowLabel::Raise),
                    None => exit_kind[b] = Some(ExitKind::Raise),
                },
                Terminator::Return(_) => exit_kind[b] = Some(ExitKind::Return),
                Terminator::Unreachable => exit_kind[b] = Some(ExitKind::Unreachable),
            }
            for (k, inst) in block.insts.iter().enumerate() {
                if let Inst::Call {
                    unwind: Some(h), ..
                } = inst
                {
                    push(&mut edges, b, *h, FlowLabel::Unwind(k));
                }
            }
        }

        let entry = if func.entry.index() < n { func.entry.index() } else { 0 };
        let adj: Vec<Vec<usize>> = (0..n)
            .map(|b| succs[b].iter().map(|&e| edges[e].to).collect())
            .collect();
        let radj: Vec<Vec<usize>> = (0..n)
            .map(|b| preds[b].iter().map(|&e| edges[e].from).collect())
            .collect();

        let rpo = if n == 0 { Vec::new() } else { reverse_postorder(&adj, entry) };
        let mut rpo_pos = vec![usize::MAX; n];
        for (i, &b) in rpo.iter().enumerate() {
            rpo_pos[b] = i;
        }
        let idom = if n == 0 {
            Vec::new()
        } else {
            immediate_dominators(&adj, &radj, entry)
        };

        // Post-dominators over the reversed graph rooted at the virtual exit `n`.
        let mut can_exit = vec![false; n];
        let mut queue: VecDeque<usize> = VecDeque::new();
        for b in 0..n {
            if exit_kind[b].is_some() {
                can_exit[b] = true;
                queue.push_back(b);
            }
        }
        while let Some(b) = queue.pop_front() {
            for &p in &radj[b] {
                if !can_exit[p] {
                    can_exit[p] = true;
                    queue.push_back(p);
                }
            }
        }
        let exitless: Vec<usize> = (0..n)
            .filter(|&b| rpo_pos[b] != usize::MAX && !can_exit[b])
            .collect();
        let mut rev_adj: Vec<Vec<usize>> = radj.clone();
        let mut rev_radj: Vec<Vec<usize>> = adj.clone();
        rev_adj.push(Vec::new());
        rev_radj.push(Vec::new());
        for b in 0..n {
            if exit_kind[b].is_some() || exitless.contains(&b) {
                rev_adj[n].push(b);
                rev_radj[b].push(n);
            }
        }
        let ipdom = immediate_dominators(&rev_adj, &rev_radj, n);

        let mut graph = FlowGraph {
            func: func.id,
            len: n,
            entry,
            edges,
            succs,
            preds,
            exit_kind,
            rpo,
            rpo_pos,
            idom,
            ipdom,
            loops: Vec::new(),
            loop_of_header: vec![None; n],
            innermost_loop: vec![None; n],
            irreducible: false,
            exitless,
        };
        graph.find_loops(&radj);
        graph
    }

    fn find_loops(&mut self, radj: &[Vec<usize>]) {
        let mut irreducible = false;
        let mut by_header: BTreeMap<usize, (BTreeSet<usize>, BTreeSet<usize>)> = BTreeMap::new();
        for e in &self.edges {
            let (u, h) = (e.from, e.to);
            if !self.is_reachable(u) || !self.is_reachable(h) {
                continue;
            }
            if self.rpo_pos[h] <= self.rpo_pos[u] {
                if self.dominates(h, u) {
                    let entry = by_header.entry(h).or_default();
                    entry.1.insert(u);
                    entry.0.insert(h);
                    let mut stack = vec![u];
                    while let Some(x) = stack.pop() {
                        if entry.0.insert(x) {
                            for &p in &radj[x] {
                                if self.is_reachable(p) && !entry.0.contains(&p) {
                                    stack.push(p);
                                }
                            }
                        }
                    }
                } else {
                    irreducible = true;
                }
            }
        }
        self.irreducible = irreducible;
        let mut loops: Vec<NaturalLoop> = by_header
            .into_iter()
            .map(|(header, (body, latches))| NaturalLoop {
                header,
                body: body.into_iter().collect(),
                latches: latches.into_iter().collect(),
                exits: Vec::new(),
                follow: None,
                parent: None,
                depth: 1,
            })
            .collect();
        for l in loops.iter_mut() {
            l.exits = l
                .body
                .iter()
                .flat_map(|&b| self.succs[b].iter().copied())
                .filter(|&e| l.body.binary_search(&self.edges[e].to).is_err())
                .collect();
            l.exits.sort_unstable();
        }
        let follows: Vec<Option<usize>> = loops.iter().map(|l| self.choose_follow(l)).collect();
        for (l, f) in loops.iter_mut().zip(follows) {
            l.follow = f;
        }
        // Nesting: the parent is the smallest other loop whose body holds this header.
        let count = loops.len();
        for i in 0..count {
            let mut best: Option<usize> = None;
            for j in 0..count {
                if i == j || loops[j].body.len() <= loops[i].body.len() {
                    continue;
                }
                if loops[j].contains(loops[i].header)
                    && best.map_or(true, |b| loops[j].body.len() < loops[b].body.len())
                {
                    best = Some(j);
                }
            }
            loops[i].parent = best;
        }
        for i in 0..count {
            let mut depth = 1;
            let mut cur = loops[i].parent;
            while let Some(p) = cur {
                depth += 1;
                if depth > count + 1 {
                    break;
                }
                cur = loops[p].parent;
            }
            loops[i].depth = depth;
        }
        for (i, l) in loops.iter().enumerate() {
            self.loop_of_header[l.header] = Some(i);
            for &b in &l.body {
                let better = match self.innermost_loop[b] {
                    Some(cur) => l.body.len() < loops[cur].body.len(),
                    None => true,
                };
                if better {
                    self.innermost_loop[b] = Some(i);
                }
            }
        }
        self.loops = loops;
    }

    /// Follow block of a loop: the target of the header's exit (the loop condition of a
    /// pre-tested loop), else of a latch's exit (post-tested loop), else the single exit
    /// target, else the nearest common post-dominator of the exit targets, else the exit
    /// target first in reverse post-order. The choice places the code after the loop; the
    /// other exits are walked as `break` operands.
    fn choose_follow(&self, l: &NaturalLoop) -> Option<usize> {
        let targets_from = |pred: &dyn Fn(usize) -> bool| -> BTreeSet<usize> {
            l.exits
                .iter()
                .filter(|&&e| pred(self.edges[e].from))
                .map(|&e| self.edges[e].to)
                .collect()
        };
        let from_header = targets_from(&|b: usize| b == l.header);
        if from_header.len() == 1 {
            return from_header.into_iter().next();
        }
        let from_latch = targets_from(&|b: usize| l.latches.contains(&b));
        if from_latch.len() == 1 {
            return from_latch.into_iter().next();
        }
        let all = targets_from(&|_: usize| true);
        if all.len() == 1 {
            return all.into_iter().next();
        }
        let mut iter = all.iter().copied();
        if let Some(first) = iter.next() {
            let mut acc = Some(first);
            for t in iter {
                acc = acc.and_then(|a| self.common_postdominator(a, t));
            }
            if let Some(p) = acc {
                if p < self.len {
                    return Some(p);
                }
            }
        }
        all.into_iter()
            .filter(|&b| self.is_reachable(b))
            .min_by_key(|&b| self.rpo_pos[b])
    }

    fn common_postdominator(&self, a: usize, b: usize) -> Option<usize> {
        let mut chain = HashSet::new();
        let mut cur = a;
        for _ in 0..=self.len + 1 {
            chain.insert(cur);
            match self.ipdom.get(cur).copied().flatten() {
                Some(p) if p != cur => cur = p,
                _ => break,
            }
        }
        let mut cur = b;
        for _ in 0..=self.len + 1 {
            if chain.contains(&cur) {
                return Some(cur);
            }
            match self.ipdom.get(cur).copied().flatten() {
                Some(p) if p != cur => cur = p,
                _ => break,
            }
        }
        None
    }

    #[inline]
    pub fn exit(&self) -> usize {
        self.len
    }

    #[inline]
    pub fn is_reachable(&self, b: usize) -> bool {
        b < self.len && self.rpo_pos[b] != usize::MAX
    }

    /// Successor blocks of `b`, in edge order.
    pub fn succ_blocks(&self, b: usize) -> Vec<usize> {
        self.succs[b].iter().map(|&e| self.edges[e].to).collect()
    }

    /// Distinct predecessor blocks of `b` (normal and exceptional), reachable ones only.
    pub fn pred_blocks(&self, b: usize) -> Vec<usize> {
        let mut out: Vec<usize> = self.preds[b]
            .iter()
            .map(|&e| self.edges[e].from)
            .filter(|&p| self.is_reachable(p))
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Immediate post-dominator block of `b`; `None` when it is the virtual exit.
    pub fn ipdom_block(&self, b: usize) -> Option<usize> {
        match self.ipdom.get(b).copied().flatten() {
            Some(p) if p < self.len => Some(p),
            _ => None,
        }
    }

    pub fn dominates(&self, a: usize, b: usize) -> bool {
        let mut cur = b;
        for _ in 0..=self.len {
            if cur == a {
                return true;
            }
            match self.idom.get(cur).copied().flatten() {
                Some(p) if p != cur => cur = p,
                _ => return false,
            }
        }
        false
    }

    /// True when edge `e` closes a natural loop.
    pub fn is_back_edge(&self, e: usize) -> bool {
        let edge = self.edges[e];
        match self.loop_of_header[edge.to] {
            Some(l) => self.loops[l].latches.contains(&edge.from) && self.dominates(edge.to, edge.from),
            None => false,
        }
    }

    /// Control dependences `(controller, dependent, edge)`: `dependent` executes only when
    /// `controller` takes edge `edge`.
    pub fn control_dependences(&self) -> Vec<(usize, usize, usize)> {
        let mut out = Vec::new();
        for (ei, e) in self.edges.iter().enumerate() {
            if !self.is_reachable(e.from) {
                continue;
            }
            let stop = self.ipdom.get(e.from).copied().flatten();
            let mut runner = e.to;
            for _ in 0..=self.len {
                if Some(runner) == stop || runner >= self.len {
                    break;
                }
                out.push((e.from, runner, ei));
                runner = match self.ipdom.get(runner).copied().flatten() {
                    Some(p) if p != runner => p,
                    _ => break,
                };
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    #[inline]
    pub fn block_id(&self, b: usize) -> BlockId {
        BlockId::new(b)
    }
}

/// Reverse post-order of the blocks reachable from `root` (iterative DFS).
pub fn reverse_postorder(adj: &[Vec<usize>], root: usize) -> Vec<usize> {
    let n = adj.len();
    let mut post = Vec::with_capacity(n);
    if root >= n {
        return post;
    }
    let mut visited = vec![false; n];
    let mut stack: Vec<(usize, usize)> = vec![(root, 0)];
    visited[root] = true;
    while let Some(top) = stack.last_mut() {
        let v = top.0;
        if top.1 < adj[v].len() {
            let w = adj[v][top.1];
            top.1 += 1;
            if w < n && !visited[w] {
                visited[w] = true;
                stack.push((w, 0));
            }
        } else {
            post.push(v);
            stack.pop();
        }
    }
    post.reverse();
    post
}

/// Immediate dominators (Cooper, Harvey & Kennedy): `Some(root)` for the root, `None` for
/// nodes unreachable from it.
pub fn immediate_dominators(adj: &[Vec<usize>], radj: &[Vec<usize>], root: usize) -> Vec<Option<usize>> {
    let n = adj.len();
    let mut idom: Vec<Option<usize>> = vec![None; n];
    let order = reverse_postorder(adj, root);
    if order.is_empty() {
        return idom;
    }
    let mut pos = vec![usize::MAX; n];
    for (i, &b) in order.iter().enumerate() {
        pos[b] = i;
    }
    idom[root] = Some(root);
    let mut changed = true;
    while changed {
        changed = false;
        for &b in order.iter().skip(1) {
            let mut new_idom: Option<usize> = None;
            for &p in &radj[b] {
                if p >= n || idom[p].is_none() {
                    continue;
                }
                new_idom = Some(match new_idom {
                    None => p,
                    Some(cur) => intersect(p, cur, &idom, &pos),
                });
            }
            if new_idom.is_some() && idom[b] != new_idom {
                idom[b] = new_idom;
                changed = true;
            }
        }
    }
    idom
}

fn intersect(mut a: usize, mut b: usize, idom: &[Option<usize>], pos: &[usize]) -> usize {
    let limit = idom.len() * 2 + 2;
    let mut steps = 0;
    while a != b {
        while pos[a] > pos[b] {
            match idom[a] {
                Some(x) if x != a => a = x,
                _ => return b,
            }
            steps += 1;
            if steps > limit {
                return b;
            }
        }
        while pos[b] > pos[a] {
            match idom[b] {
                Some(x) if x != b => b = x,
                _ => return a,
            }
            steps += 1;
            if steps > limit {
                return a;
            }
        }
    }
    a
}

/// Strongly connected components (iterative Tarjan), each sorted, in completion order
/// (callees before callers for a call graph).
pub fn strongly_connected(adj: &[Vec<usize>]) -> Vec<Vec<usize>> {
    let n = adj.len();
    let mut index = vec![usize::MAX; n];
    let mut low = vec![0usize; n];
    let mut on_stack = vec![false; n];
    let mut stack: Vec<usize> = Vec::new();
    let mut out: Vec<Vec<usize>> = Vec::new();
    let mut counter = 0usize;
    let mut call: Vec<(usize, usize)> = Vec::new();
    for root in 0..n {
        if index[root] != usize::MAX {
            continue;
        }
        index[root] = counter;
        low[root] = counter;
        counter += 1;
        stack.push(root);
        on_stack[root] = true;
        call.push((root, 0));
        while let Some(&(v, i)) = call.last() {
            if i < adj[v].len() {
                let top = call.len() - 1;
                call[top].1 += 1;
                let w = adj[v][i];
                if w >= n {
                    continue;
                }
                if index[w] == usize::MAX {
                    index[w] = counter;
                    low[w] = counter;
                    counter += 1;
                    stack.push(w);
                    on_stack[w] = true;
                    call.push((w, 0));
                } else if on_stack[w] {
                    low[v] = low[v].min(index[w]);
                }
            } else {
                call.pop();
                if let Some(&(u, _)) = call.last() {
                    low[u] = low[u].min(low[v]);
                }
                if low[v] == index[v] {
                    let mut comp = Vec::new();
                    while let Some(w) = stack.pop() {
                        on_stack[w] = false;
                        comp.push(w);
                        if w == v {
                            break;
                        }
                    }
                    comp.sort_unstable();
                    out.push(comp);
                }
            }
        }
    }
    out
}

// ── Edge descriptions shared by the behavioural views ────────────────────────

/// Short label of a control-flow edge (`true`, `false`, a case value, `default`, ...).
pub fn edge_label(ctx: &DeriveContext, func: &FuncIr, edge: &FlowEdge) -> String {
    match edge.label {
        FlowLabel::Always => String::new(),
        FlowLabel::True => "true".to_string(),
        FlowLabel::False => "false".to_string(),
        FlowLabel::Case(k) => match func.blocks.get(edge.from).map(|b| &b.term) {
            Some(Terminator::Switch { cases, .. }) => cases
                .get(k)
                .map(|(c, _)| const_text(ctx, c))
                .unwrap_or_default(),
            _ => String::new(),
        },
        FlowLabel::Default => "default".to_string(),
        FlowLabel::Unwind(_) => "unwind".to_string(),
        FlowLabel::Raise => "raise".to_string(),
    }
}

/// Guard of a control-flow edge, reconstructed from the branch operand's definition.
pub fn edge_guard(
    ctx: &DeriveContext,
    renderer: &ValueRenderer,
    func: &FuncIr,
    edge: &FlowEdge,
) -> Option<String> {
    let term = &func.blocks.get(edge.from)?.term;
    match (edge.label, term) {
        (FlowLabel::True, Terminator::Branch { cond, .. }) => {
            Some(renderer.condition(ctx, cond, true))
        }
        (FlowLabel::False, Terminator::Branch { cond, .. }) => {
            Some(renderer.condition(ctx, cond, false))
        }
        (FlowLabel::Case(k), Terminator::Switch { scrutinee, cases, .. }) => cases
            .get(k)
            .map(|(c, _)| format!("{} = {}", renderer.expr(ctx, scrutinee), const_text(ctx, c))),
        (FlowLabel::Default, Terminator::Switch { .. }) => Some("else".to_string()),
        (FlowLabel::Unwind(k), _) => {
            let block = func.blocks.get(edge.from)?;
            match block.insts.get(k) {
                Some(Inst::Call { callee, .. }) => Some(format!(
                    "exception from {}",
                    crate::derive::provenance::callee_text(ctx, renderer, callee)
                )),
                _ => Some("exception".to_string()),
            }
        }
        (FlowLabel::Raise, Terminator::Throw(v)) => {
            Some(format!("raised {}", renderer.operand(ctx, v)))
        }
        _ => None,
    }
}

// ── CFG view ─────────────────────────────────────────────────────────────────

fn func_key(ctx: &DeriveContext, func: &FuncIr) -> String {
    format!("{}#{}", ctx.func_qualified_name(func), func.id.raw())
}

/// Control-flow graph of one function with its dominator tree.
pub fn cfg(ctx: &DeriveContext, func_id: FuncId) -> DiagramModel {
    let Some(func) = ctx.func(func_id) else {
        return DiagramModel::not_derivable(
            DiagramKind::Cfg,
            format!("CFG of {}", func_id),
            "the function is not part of the module",
        );
    };
    let qn = ctx.func_qualified_name(func);
    if func.blocks.is_empty() {
        return DiagramModel::not_derivable(
            DiagramKind::Cfg,
            format!("CFG of {}", qn),
            "the function has no lowered body",
        )
        .with_scope(qn);
    }
    let fg = FlowGraph::build(func);
    let r = ValueRenderer::new(func);
    let key = func_key(ctx, func);
    let mut model = DiagramModel::new(DiagramKind::Cfg, format!("CFG of {}", qn)).with_scope(qn);
    let fid = func.id.raw();

    let entry_id = IdBuilder::new("cfg_entry").text(&key).finish();
    let exit_id = IdBuilder::new("cfg_exit").text(&key).finish();
    model.add_node(DiagramNode::new(entry_id.clone(), NodeKind::Entry, "entry").with_span(func_span(func)));
    model.add_node(DiagramNode::new(exit_id.clone(), NodeKind::Exit, "exit"));

    let block_ids: Vec<ElementId> = (0..fg.len)
        .map(|b| IdBuilder::new("cfg_block").text(&key).num(b as u64).finish())
        .collect();
    for (b, block) in func.blocks.iter().enumerate() {
        let mut entries: Vec<CompartmentEntry> = Vec::with_capacity(block.insts.len() + 1);
        for (k, inst) in block.insts.iter().enumerate() {
            let mut e = CompartmentEntry::named(r.inst(ctx, inst));
            if let Some(s) = inst_span(block, k) {
                e.spans.push(s);
            }
            entries.push(e);
        }
        let mut t = CompartmentEntry::named(r.terminator(ctx, block));
        if let Some(s) = term_span(block) {
            t.spans.push(s);
        }
        entries.push(t);
        let mut node = DiagramNode::new(block_ids[b].clone(), NodeKind::BasicBlock, format!("{}", block.id))
            .with_compartment(CompartmentKind::Statements, entries)
            .with_span(block_span(block))
            .with_fact(Fact::Terminator {
                func: fid,
                block: b as u32,
            });
        if let Some(l) = fg.loop_of_header[b] {
            node = node
                .with_stereotype("loop header")
                .with_property("loop depth", fg.loops[l].depth.to_string())
                .with_fact(Fact::Loop {
                    func: fid,
                    header: b as u32,
                });
        }
        if !fg.is_reachable(b) {
            node = node.with_property("reachable", "false");
        }
        if fg.exitless.contains(&b) {
            node = node.with_property("reaches exit", "false");
        }
        model.add_node(node);
    }

    model.add_edge(DiagramEdge::new(
        IdBuilder::new("cfg_edge").text(&key).text("entry").finish(),
        EdgeKind::ControlFlow,
        entry_id,
        block_ids[fg.entry].clone(),
    ));
    for (ei, e) in fg.edges.iter().enumerate() {
        let kind = if e.label.is_exceptional() {
            EdgeKind::ExceptionFlow
        } else {
            EdgeKind::ControlFlow
        };
        let mut edge = DiagramEdge::new(
            IdBuilder::new("cfg_edge").text(&key).num(ei as u64).finish(),
            kind,
            block_ids[e.from].clone(),
            block_ids[e.to].clone(),
        )
        .with_label(edge_label(ctx, func, e))
        .with_span(term_span(&func.blocks[e.from]))
        .with_fact(Fact::Terminator {
            func: fid,
            block: e.from as u32,
        });
        if fg.is_back_edge(ei) {
            edge = edge.with_property("back edge", "true");
        }
        model.add_edge(edge);
    }
    for b in 0..fg.len {
        if let Some(kind) = fg.exit_kind[b] {
            let label = match kind {
                ExitKind::Return => "return",
                ExitKind::Raise => "raise",
                ExitKind::Unreachable => "unreachable",
            };
            let edge_kind = if kind == ExitKind::Raise {
                EdgeKind::ExceptionFlow
            } else {
                EdgeKind::ControlFlow
            };
            model.add_edge(
                DiagramEdge::new(
                    IdBuilder::new("cfg_exit_edge").text(&key).num(b as u64).finish(),
                    edge_kind,
                    block_ids[b].clone(),
                    exit_id.clone(),
                )
                .with_label(label)
                .with_span(term_span(&func.blocks[b])),
            );
        }
    }
    for b in 0..fg.len {
        if let Some(p) = fg.idom[b] {
            if p != b {
                model.add_edge(
                    DiagramEdge::new(
                        IdBuilder::new("cfg_dom").text(&key).num(b as u64).finish(),
                        EdgeKind::Dominates,
                        block_ids[p].clone(),
                        block_ids[b].clone(),
                    )
                    .with_fact(Fact::Dominance {
                        func: fid,
                        block: b as u32,
                    }),
                );
            }
        }
    }
    if fg.irreducible {
        model.note("the function has irreducible control flow: some cycles have no dominating header and no natural loop describes them");
    }
    if !fg.exitless.is_empty() {
        model.note(format!(
            "{} block(s) cannot reach a function exit; post-dominance treats them as exiting",
            fg.exitless.len()
        ));
    }
    model.finalize(ctx.limits.max_spans);
    model
}

// ── DFG view ─────────────────────────────────────────────────────────────────

/// SSA def-use data-flow graph of one function.
pub fn dfg(ctx: &DeriveContext, func_id: FuncId) -> DiagramModel {
    let Some(func) = ctx.func(func_id) else {
        return DiagramModel::not_derivable(
            DiagramKind::Dfg,
            format!("DFG of {}", func_id),
            "the function is not part of the module",
        );
    };
    let qn = ctx.func_qualified_name(func);
    if func.blocks.is_empty() {
        return DiagramModel::not_derivable(
            DiagramKind::Dfg,
            format!("DFG of {}", qn),
            "the function has no lowered body",
        )
        .with_scope(qn);
    }
    let r = ValueRenderer::new(func);
    let key = func_key(ctx, func);
    let fid = func.id.raw();
    let mut model = DiagramModel::new(DiagramKind::Dfg, format!("DFG of {}", qn)).with_scope(qn);
    let defs = func.def_sites();
    let value_node = |v: usize| IdBuilder::new("dfg_value").text(&key).num(v as u64).finish();

    for (i, &p) in func.params.iter().enumerate() {
        model.add_node(
            DiagramNode::new(
                value_node(p.index()),
                NodeKind::Value,
                format!("param {}: {}", i, r.name(ctx, p)),
            )
            .with_property("type", ctx.render_type(func.value_type(p)))
            .with_fact(Fact::DefUse {
                func: fid,
                value: p.raw(),
            }),
        );
    }
    for (b, block) in func.blocks.iter().enumerate() {
        for (k, inst) in block.insts.iter().enumerate() {
            let span = inst_span(block, k);
            let user = match inst.dst() {
                Some(d) => {
                    let kind = if inst.is_phi() { NodeKind::Phi } else { NodeKind::Value };
                    let id = value_node(d.index());
                    model.add_node(
                        DiagramNode::new(id.clone(), kind, r.inst(ctx, inst))
                            .with_property("type", ctx.render_type(func.value_type(d)))
                            .with_span(span)
                            .with_fact(Fact::DefUse {
                                func: fid,
                                value: d.raw(),
                            }),
                    );
                    id
                }
                None => {
                    let id = IdBuilder::new("dfg_effect")
                        .text(&key)
                        .num(b as u64)
                        .num(k as u64)
                        .finish();
                    model.add_node(
                        DiagramNode::new(id.clone(), NodeKind::Effect, r.inst(ctx, inst))
                            .with_span(span)
                            .with_fact(Fact::Inst {
                                func: fid,
                                block: b as u32,
                                index: k as u32,
                            }),
                    );
                    id
                }
            };
            if let Inst::Phi { incoming, .. } = inst {
                for (pred, op) in incoming {
                    if let Operand::Value(v) = op {
                        let src = ensure_def_node(ctx, &mut model, &r, &defs, &key, *v);
                        model.add_edge(
                            DiagramEdge::new(
                                IdBuilder::new("dfg_phi")
                                    .text(&key)
                                    .text(user.as_str())
                                    .num(pred.raw() as u64)
                                    .num(v.raw() as u64)
                                    .finish(),
                                EdgeKind::PhiArgument,
                                src,
                                user.clone(),
                            )
                            .with_label(format!("{}", pred)),
                        );
                    }
                }
                continue;
            }
            let mut uses = inst.uses();
            uses.sort_unstable();
            uses.dedup();
            for v in uses {
                let src = ensure_def_node(ctx, &mut model, &r, &defs, &key, v);
                model.add_edge(
                    DiagramEdge::new(
                        IdBuilder::new("dfg_use")
                            .text(&key)
                            .text(user.as_str())
                            .num(v.raw() as u64)
                            .finish(),
                        EdgeKind::DataFlow,
                        src,
                        user.clone(),
                    )
                    .with_label(r.name(ctx, v)),
                );
            }
        }
        let mut uses = block.term.uses();
        if uses.is_empty() {
            continue;
        }
        uses.sort_unstable();
        uses.dedup();
        let term_id = IdBuilder::new("dfg_term").text(&key).num(b as u64).finish();
        model.add_node(
            DiagramNode::new(term_id.clone(), NodeKind::Effect, r.terminator(ctx, block))
                .with_span(term_span(block))
                .with_fact(Fact::Terminator {
                    func: fid,
                    block: b as u32,
                }),
        );
        for v in uses {
            let src = ensure_def_node(ctx, &mut model, &r, &defs, &key, v);
            model.add_edge(
                DiagramEdge::new(
                    IdBuilder::new("dfg_use")
                        .text(&key)
                        .text(term_id.as_str())
                        .num(v.raw() as u64)
                        .finish(),
                    EdgeKind::DataFlow,
                    src,
                    term_id.clone(),
                )
                .with_label(r.name(ctx, v)),
            );
        }
    }
    model.finalize(ctx.limits.max_spans);
    model
}

/// Node of a value's definition; a value without one gets an explicit `Partial` node.
fn ensure_def_node(
    ctx: &DeriveContext,
    model: &mut DiagramModel,
    r: &ValueRenderer,
    defs: &[DefSite],
    key: &str,
    v: crate::ir::types::ValueId,
) -> ElementId {
    let id = IdBuilder::new("dfg_value").text(key).num(v.index() as u64).finish();
    if model.has_node(&id) {
        return id;
    }
    let defined = matches!(defs.get(v.index()), Some(DefSite::Param(_)) | Some(DefSite::Inst { .. }));
    let mut node = DiagramNode::new(id.clone(), NodeKind::Value, r.name(ctx, v));
    if !defined {
        node = node.with_derivation(Derivation::partial(
            "a value is used but has no definition in the function",
        ));
    }
    model.add_node(node)
}

// ── CDG view ─────────────────────────────────────────────────────────────────

/// Control dependence graph of one function, from post-dominance.
pub fn cdg(ctx: &DeriveContext, func_id: FuncId) -> DiagramModel {
    let Some(func) = ctx.func(func_id) else {
        return DiagramModel::not_derivable(
            DiagramKind::Cdg,
            format!("CDG of {}", func_id),
            "the function is not part of the module",
        );
    };
    let qn = ctx.func_qualified_name(func);
    if func.blocks.is_empty() {
        return DiagramModel::not_derivable(
            DiagramKind::Cdg,
            format!("CDG of {}", qn),
            "the function has no lowered body",
        )
        .with_scope(qn);
    }
    let fg = FlowGraph::build(func);
    let r = ValueRenderer::new(func);
    let key = func_key(ctx, func);
    let fid = func.id.raw();
    let mut model = DiagramModel::new(DiagramKind::Cdg, format!("CDG of {}", qn)).with_scope(qn);
    let entry_id = IdBuilder::new("cdg_entry").text(&key).finish();
    model.add_node(DiagramNode::new(entry_id.clone(), NodeKind::Entry, "entry").with_span(func_span(func)));
    let block_ids: Vec<ElementId> = (0..fg.len)
        .map(|b| IdBuilder::new("cdg_block").text(&key).num(b as u64).finish())
        .collect();
    for b in 0..fg.len {
        if !fg.is_reachable(b) {
            continue;
        }
        let block = &func.blocks[b];
        model.add_node(
            DiagramNode::new(block_ids[b].clone(), NodeKind::BasicBlock, format!("{}", block.id))
                .with_property("terminator", r.terminator(ctx, block))
                .with_span(block_span(block)),
        );
    }
    let deps = fg.control_dependences();
    let mut controlled = vec![false; fg.len];
    for &(a, b, ei) in &deps {
        controlled[b] = true;
        let e = fg.edges[ei];
        model.add_edge(
            DiagramEdge::new(
                IdBuilder::new("cdg_edge")
                    .text(&key)
                    .num(a as u64)
                    .num(b as u64)
                    .num(ei as u64)
                    .finish(),
                EdgeKind::ControlDependence,
                block_ids[a].clone(),
                block_ids[b].clone(),
            )
            .with_label(edge_label(ctx, func, &e))
            .with_guard(edge_guard(ctx, &r, func, &e))
            .with_span(term_span(&func.blocks[a]))
            .with_fact(Fact::ControlDependence {
                func: fid,
                controller: a as u32,
                dependent: b as u32,
            }),
        );
    }
    for b in 0..fg.len {
        if fg.is_reachable(b) && !controlled[b] {
            model.add_edge(DiagramEdge::new(
                IdBuilder::new("cdg_entry_edge").text(&key).num(b as u64).finish(),
                EdgeKind::ControlDependence,
                entry_id.clone(),
                block_ids[b].clone(),
            ));
        }
    }
    if !fg.exitless.is_empty() {
        model.note(format!(
            "{} block(s) cannot reach a function exit; post-dominance treats them as exiting",
            fg.exitless.len()
        ));
    }
    model.finalize(ctx.limits.max_spans);
    model
}

// ── Call graph view ──────────────────────────────────────────────────────────

/// Call graph of the module with recursive strongly connected components as groups,
/// external targets, unresolved sites and callbacks from external code.
pub fn call_graph(ctx: &DeriveContext) -> DiagramModel {
    let cg = ctx.call_graph;
    let mut model = DiagramModel::new(DiagramKind::CallGraph, DiagramKind::CallGraph.title());
    if ctx.funcs().is_empty() {
        return DiagramModel::not_derivable(
            DiagramKind::CallGraph,
            DiagramKind::CallGraph.title(),
            "the module has no functions",
        );
    }
    let func_node = |f: FuncId| -> ElementId {
        match ctx.func(f) {
            Some(fi) => IdBuilder::new("cg_func").text(&func_key(ctx, fi)).finish(),
            None => IdBuilder::new("cg_func").num(f.raw() as u64).finish(),
        }
    };
    let entries: BTreeMap<FuncId, String> = ctx
        .entry_points()
        .into_iter()
        .map(|e| (e.func, e.kind.mnemonic().to_string()))
        .collect();

    // Recursive SCCs over the reachable functions.
    let reach: Vec<FuncId> = cg.reachable.clone();
    let pos_of = |f: FuncId| reach.binary_search(&f).ok();
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); reach.len()];
    for e in &cg.edge_info {
        if let (Some(a), Some(b)) = (pos_of(e.caller), pos_of(e.callee)) {
            adj[a].push(b);
        }
    }
    for list in &mut adj {
        list.sort_unstable();
        list.dedup();
    }
    let mut group_of: BTreeMap<FuncId, ElementId> = BTreeMap::new();
    for comp in strongly_connected(&adj) {
        let recursive = comp.len() > 1 || adj[comp[0]].contains(&comp[0]);
        if !recursive {
            continue;
        }
        let members: Vec<FuncId> = comp.iter().map(|&i| reach[i]).collect();
        let mut builder = IdBuilder::new("cg_scc");
        for m in &members {
            builder = builder.text(&func_node(*m).to_string());
        }
        let gid = builder.finish();
        let label = if members.len() == 1 {
            "self-recursive".to_string()
        } else {
            format!("recursive SCC ({} functions)", members.len())
        };
        model.add_node(DiagramNode::new(gid.clone(), NodeKind::Group, label).with_stereotype("scc"));
        for m in members {
            group_of.insert(m, gid.clone());
        }
    }

    for func in ctx.funcs() {
        let reachable = ctx.is_reachable(func.id);
        let has_edges = !ctx.callers(func.id).is_empty();
        if !reachable && !has_edges && !cg.reachable.is_empty() {
            continue;
        }
        let mut node = DiagramNode::new(func_node(func.id), NodeKind::Function, ctx.func_qualified_name(func))
            .with_parent(group_of.get(&func.id).cloned())
            .with_property("kind", func.kind.mnemonic())
            .with_span(func_span(func));
        if let Some(sym) = func.symbol {
            node = node.with_symbol(sym).with_fact(Fact::Symbol(sym));
        }
        if let Some(kind) = entries.get(&func.id) {
            node = node.with_stereotype("entry").with_property("entry kind", kind.clone());
        }
        if !reachable {
            node = node.with_property("reachable", "false");
        }
        model.add_node(node);
    }

    // Lowered edges, aggregated per (caller, callee).
    let mut pairs: BTreeMap<(FuncId, FuncId), (Vec<Span>, Vec<Fact>, bool)> = BTreeMap::new();
    for e in &cg.edge_info {
        let entry = pairs
            .entry((e.caller, e.callee))
            .or_insert_with(|| (Vec::new(), Vec::new(), true));
        if let Some(s) = call_site_span(ctx, e.site) {
            entry.0.push(s);
        }
        entry.1.push(Fact::CallEdge {
            site: e.site.raw(),
            callee: e.callee.raw(),
        });
        if e.precision != EdgePrecision::Exact {
            entry.2 = false;
        }
    }
    for ((caller, callee), (spans, facts, exact)) in pairs {
        let sites = facts.len();
        let mut edge = DiagramEdge::new(
            IdBuilder::new("cg_edge")
                .text(func_node(caller).as_str())
                .text(func_node(callee).as_str())
                .finish(),
            EdgeKind::Call,
            func_node(caller),
            func_node(callee),
        )
        .with_spans(spans)
        .with_facts(facts)
        .with_derivation(if exact {
            Derivation::Exact
        } else {
            Derivation::OverApproximate
        });
        if sites > 1 {
            edge = edge.with_label(format!("{} sites", sites));
        }
        model.add_edge(edge);
    }

    // External targets and unresolved sites.
    let mut unresolved_by_caller: BTreeMap<FuncId, usize> = BTreeMap::new();
    for site in &cg.sites {
        for ext in &site.external {
            let (id, label, derivation, stereo) = match ext {
                ExternalTarget::Symbol(s) => (
                    IdBuilder::new("cg_ext").text(&ctx.qualified_name(*s)).num(*s as u64).finish(),
                    ctx.qualified_name(*s),
                    Derivation::Exact,
                    "external",
                ),
                ExternalTarget::Name(n) => (
                    IdBuilder::new("cg_unresolved").text(&ctx.text(*n)).finish(),
                    ctx.text(*n),
                    Derivation::partial("a callee name matches no declaration"),
                    "unresolved",
                ),
                ExternalTarget::FunctionValue => (
                    IdBuilder::new("cg_ext_fv").finish(),
                    "function value from external code".to_string(),
                    Derivation::partial("the called function value was created outside the module"),
                    "external",
                ),
                ExternalTarget::MissingFunction(f) => (
                    IdBuilder::new("cg_missing").num(f.raw() as u64).finish(),
                    format!("{}", f),
                    Derivation::partial("a call targets a function absent from the module"),
                    "missing",
                ),
            };
            model.add_node(
                DiagramNode::new(id.clone(), NodeKind::Function, label)
                    .with_stereotype(stereo)
                    .with_derivation(derivation),
            );
            model.add_edge(
                DiagramEdge::new(
                    IdBuilder::new("cg_ext_edge")
                        .text(func_node(site.caller).as_str())
                        .text(id.as_str())
                        .finish(),
                    EdgeKind::Call,
                    func_node(site.caller),
                    id,
                )
                .with_span(site.span)
                .with_fact(Fact::CallSite(site.site.raw())),
            );
        }
        if !site.unresolved.is_empty() {
            *unresolved_by_caller.entry(site.caller).or_insert(0) += 1;
        }
    }
    for (caller, count) in unresolved_by_caller {
        let id = func_node(caller);
        if let Some(node) = model.node_mut(&id) {
            node.properties
                .push(("unresolved call sites".to_string(), count.to_string()));
            node.derivation.weaken(&Derivation::partial(
                "some call sites resolved to no target (empty receiver set, missing hierarchy facts or solver budget)",
            ));
        }
    }

    // Callbacks: invocations of module code from external code.
    if !cg.callbacks.is_empty() {
        let gate = IdBuilder::new("cg_external_caller").finish();
        model.add_node(DiagramNode::new(gate.clone(), NodeKind::Gate, "external code"));
        for (i, cb) in cg.callbacks.iter().enumerate() {
            let label = match cb.origin {
                CallbackOrigin::ExternalOverride { declaration } => {
                    format!("overrides {}", ctx.qualified_name(declaration))
                }
                CallbackOrigin::ExternalFunctionValue => "function value".to_string(),
                CallbackOrigin::Intrinsic { name, .. } => ctx.text(name),
            };
            model.add_edge(
                DiagramEdge::new(
                    IdBuilder::new("cg_callback")
                        .text(func_node(cb.func).as_str())
                        .num(i as u64)
                        .finish(),
                    EdgeKind::Call,
                    gate.clone(),
                    func_node(cb.func),
                )
                .with_label(label)
                .with_span(cb.span)
                .with_fact(Fact::Callback(cb.func.raw()))
                .with_derivation(Derivation::OverApproximate),
            );
        }
    }
    if cg.reachable.is_empty() {
        model.weaken(&Derivation::partial(
            "the call graph reaches no function: the points-to solve had no entry points",
        ));
    }
    model.finalize(ctx.limits.max_spans);
    model
}

// ── ROBDD view ───────────────────────────────────────────────────────────────

/// A node of a path-summary ROBDD. Node 0 is FALSE and node 1 is TRUE; internal nodes
/// test variable `var` (a level of `BddSummary::variables`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BddNodeFact {
    pub var: u16,
    pub lo: u32,
    pub hi: u32,
}

/// Meaning of one ROBDD variable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BddVariable {
    /// The path takes the CFG edge `from -> to`.
    Edge { from: BlockId, to: BlockId },
    /// The path enters the loop headed by `header`.
    LoopEntry { header: BlockId },
    /// A branch condition atom, as the condition analysis renders it.
    Condition(String),
}

/// The path summary of one function, as the path-summary phase computed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BddSummary {
    pub func: FuncId,
    pub nodes: Vec<BddNodeFact>,
    pub root: Option<u32>,
    /// Meaning of each variable, indexed by level.
    pub variables: Vec<BddVariable>,
    /// Exact number of encoded paths, as text (it may exceed 64 bits).
    pub path_count: Option<String>,
    /// Why no ROBDD is stored, when `root` is `None`.
    pub unavailable: Option<String>,
}

/// One ROBDD path summary.
pub fn robdd(ctx: &DeriveContext, summary: &BddSummary) -> DiagramModel {
    let name = ctx
        .func(summary.func)
        .map(|f| ctx.func_qualified_name(f))
        .unwrap_or_else(|| format!("{}", summary.func));
    let title = format!("ROBDD of {}", name);
    let Some(root) = summary.root else {
        let reason = summary
            .unavailable
            .clone()
            .unwrap_or_else(|| "the path-summary phase stored no ROBDD for the function".to_string());
        return DiagramModel::not_derivable(DiagramKind::Robdd, title, reason).with_scope(name);
    };
    let key = format!("{}#{}", name, summary.func.raw());
    let fid = summary.func.raw();
    let mut model = DiagramModel::new(DiagramKind::Robdd, title).with_scope(name);
    let node_id = |n: u32| IdBuilder::new("bdd").text(&key).num(n as u64).finish();
    let mut seen: BTreeSet<u32> = BTreeSet::new();
    let mut queue: VecDeque<u32> = VecDeque::new();
    queue.push_back(root);
    let mut invalid = 0usize;
    while let Some(n) = queue.pop_front() {
        if !seen.insert(n) {
            continue;
        }
        if n <= 1 {
            model.add_node(
                DiagramNode::new(node_id(n), NodeKind::BddTerminal, if n == 1 { "1" } else { "0" })
                    .with_fact(Fact::BddNode { func: fid, node: n }),
            );
            continue;
        }
        let Some(fact) = summary.nodes.get(n as usize) else {
            invalid += 1;
            continue;
        };
        let label = match summary.variables.get(fact.var as usize) {
            Some(BddVariable::Edge { from, to }) => format!("{} -> {}", from, to),
            Some(BddVariable::LoopEntry { header }) => format!("enter loop {}", header),
            Some(BddVariable::Condition(text)) => text.clone(),
            None => format!("x{}", fact.var),
        };
        model.add_node(
            DiagramNode::new(node_id(n), NodeKind::BddNode, label)
                .with_property("level", fact.var.to_string())
                .with_fact(Fact::BddNode { func: fid, node: n }),
        );
        model.add_edge(DiagramEdge::new(
            IdBuilder::new("bdd_lo").text(&key).num(n as u64).finish(),
            EdgeKind::BddLow,
            node_id(n),
            node_id(fact.lo),
        ));
        model.add_edge(DiagramEdge::new(
            IdBuilder::new("bdd_hi").text(&key).num(n as u64).finish(),
            EdgeKind::BddHigh,
            node_id(n),
            node_id(fact.hi),
        ));
        queue.push_back(fact.lo);
        queue.push_back(fact.hi);
    }
    if let Some(count) = &summary.path_count {
        model.note(format!("{} path(s) encoded", count));
    }
    if invalid > 0 {
        model.weaken(&Derivation::partial(format!(
            "{} ROBDD child reference(s) point outside the stored node array",
            invalid
        )));
    }
    model.finalize(ctx.limits.max_spans);
    model
}

// ── Dispatch ─────────────────────────────────────────────────────────────────

/// Functions the per-function views cover: the reachable functions with bodies, or every
/// function with a body when the call graph reaches none.
pub(crate) fn view_functions(ctx: &DeriveContext) -> Vec<FuncId> {
    let mut funcs: Vec<FuncId> = ctx
        .call_graph
        .reachable
        .iter()
        .copied()
        .filter(|&f| ctx.func(f).map_or(false, |fi| !fi.blocks.is_empty()))
        .collect();
    if funcs.is_empty() {
        funcs = ctx
            .funcs()
            .iter()
            .filter(|f| !f.blocks.is_empty())
            .map(|f| f.id)
            .collect();
    }
    funcs.sort_unstable();
    funcs.dedup();
    funcs
}

pub fn derive(ctx: &DeriveContext, kind: DiagramKind) -> Vec<DiagramModel> {
    match kind {
        DiagramKind::CallGraph => vec![call_graph(ctx)],
        DiagramKind::Robdd => {
            if ctx.bdds.is_empty() {
                return vec![DiagramModel::not_derivable(
                    DiagramKind::Robdd,
                    DiagramKind::Robdd.title(),
                    "no path summaries were computed for this module",
                )];
            }
            let mut summaries: Vec<&BddSummary> = ctx.bdds.iter().collect();
            summaries.sort_by_key(|s| s.func);
            let cut = cap(&mut summaries, ctx.limits.max_diagrams);
            let mut out: Vec<DiagramModel> = summaries.into_iter().map(|s| robdd(ctx, s)).collect();
            if cut > 0 {
                out.push(DiagramModel::omission(kind, cut, ctx.limits.max_diagrams));
            }
            out
        }
        DiagramKind::Cfg | DiagramKind::Dfg | DiagramKind::Cdg => {
            let mut funcs = view_functions(ctx);
            if funcs.is_empty() {
                return vec![DiagramModel::not_derivable(
                    kind,
                    kind.title(),
                    "the module has no function with a lowered body",
                )];
            }
            let cut = cap(&mut funcs, ctx.limits.max_diagrams);
            let mut out: Vec<DiagramModel> = funcs
                .into_iter()
                .map(|f| match kind {
                    DiagramKind::Cfg => cfg(ctx, f),
                    DiagramKind::Dfg => dfg(ctx, f),
                    _ => cdg(ctx, f),
                })
                .collect();
            if cut > 0 {
                out.push(DiagramModel::omission(kind, cut, ctx.limits.max_diagrams));
            }
            out
        }
        other => vec![DiagramModel::not_derivable(
            other,
            other.title(),
            "not a compiler view",
        )],
    }
}
