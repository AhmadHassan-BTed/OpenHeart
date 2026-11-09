//! Activity diagrams from the IR control-flow graph of one function.
//!
//! * **Actions** are the instructions with an effect: calls (call actions), allocations
//!   of classifier instances (create actions), stores (structural-feature and variable
//!   writes), releases at scope exit, effectful intrinsics and raises. Pure computations
//!   are data flow, not actions; they appear inside guards and action labels.
//! * **Control nodes.** A block with two or more predecessors starts with a merge, a
//!   branch or switch ends with a decision whose outgoing flows carry the reconstructed
//!   guards, returns flow to the activity final node, raises that leave the function flow
//!   to an exceptional final node, unreachable ends are flow finals. Blocks without
//!   effects and with a single successor are passed through.
//! * **Exception flows** go from a call (or raise) to the first node of its landing block.
//! * **Loops** are natural loops, reported as loop regions covering their nodes and
//!   guarded by the loop's exit condition.
//! * **Partitions** (swimlanes) are the owning types of the behaviour each action executes:
//!   the callee's type for calls, the function's own type otherwise.

use std::collections::{HashMap, HashSet};

use crate::derive::compiler_views::{edge_guard, view_functions, FlowGraph, FlowLabel};
use crate::derive::model::{
    Derivation, DiagramEdge, DiagramKind, DiagramModel, DiagramNode, EdgeKind, ElementId, Fragment,
    FragmentKind, FragmentOperand, IdBuilder, NodeKind,
};
use crate::derive::provenance::{callee_text, func_span, inst_span, term_span, Fact, ValueRenderer};
use crate::derive::{cap, DeriveContext};
use crate::ir::func::FuncIr;
use crate::ir::inst::{Callee, Inst, IntrinsicKind, Region, Terminator};
use crate::ir::types::{FuncId, Span};
use crate::memory::location::Context;

struct ActivityBuilder<'c, 'a> {
    ctx: &'c DeriveContext<'a>,
    func: &'a FuncIr,
    fg: FlowGraph,
    r: ValueRenderer<'a>,
    key: String,
    model: DiagramModel,
    partitions: HashMap<u32, ElementId>,
    own_owner: Option<u32>,
    /// First and last node of each block's chain (empty chains are passed through).
    chains: Vec<Vec<ElementId>>,
    /// Exception flows: (source node, landing block, guard).
    exceptional: Vec<(ElementId, usize, Option<String>, Option<Span>)>,
    final_node: Option<ElementId>,
    raise_final: Option<ElementId>,
    flow_final: Option<ElementId>,
    entry_cache: HashMap<usize, ElementId>,
}

impl<'c, 'a> ActivityBuilder<'c, 'a> {
    fn partition(&mut self, owner: Option<u32>) -> Option<ElementId> {
        let owner = owner?;
        if let Some(id) = self.partitions.get(&owner) {
            return Some(id.clone());
        }
        let ctx = self.ctx;
        let id = IdBuilder::new("act_part")
            .text(&self.key)
            .text(&ctx.qualified_name(owner))
            .finish();
        self.model.add_node(
            DiagramNode::new(id.clone(), NodeKind::Partition, ctx.symbol_name(owner))
                .with_qualified_name(ctx.qualified_name(owner))
                .with_symbol(owner)
                .with_fact(Fact::Symbol(owner)),
        );
        self.partitions.insert(owner, id.clone());
        Some(id)
    }

    fn node_id(&self, prefix: &'static str, b: usize, k: usize) -> ElementId {
        IdBuilder::new(prefix)
            .text(&self.key)
            .num(b as u64)
            .num(k as u64)
            .finish()
    }

    fn add(&mut self, node: DiagramNode, owner: Option<u32>) -> ElementId {
        let parent = self.partition(owner);
        self.model.add_node(node.with_parent(parent))
    }

    fn final_node(&mut self) -> ElementId {
        if let Some(id) = &self.final_node {
            return id.clone();
        }
        let id = IdBuilder::new("act_final").text(&self.key).finish();
        let owner = self.own_owner;
        self.add(DiagramNode::new(id.clone(), NodeKind::ActivityFinal, ""), owner);
        self.final_node = Some(id.clone());
        id
    }

    fn raise_final(&mut self) -> ElementId {
        if let Some(id) = &self.raise_final {
            return id.clone();
        }
        let id = IdBuilder::new("act_raise_final").text(&self.key).finish();
        let owner = self.own_owner;
        self.add(
            DiagramNode::new(id.clone(), NodeKind::ActivityFinal, "exception")
                .with_stereotype("exception"),
            owner,
        );
        self.raise_final = Some(id.clone());
        id
    }

    fn flow_final(&mut self) -> ElementId {
        if let Some(id) = &self.flow_final {
            return id.clone();
        }
        let id = IdBuilder::new("act_flow_final").text(&self.key).finish();
        let owner = self.own_owner;
        self.add(DiagramNode::new(id.clone(), NodeKind::FlowFinal, ""), owner);
        self.flow_final = Some(id.clone());
        id
    }

    /// Owner type of the behaviour a call executes.
    fn call_owner(&self, site: crate::ir::types::CallSiteId, callee: &Callee) -> (Option<u32>, Derivation) {
        let ctx = self.ctx;
        let targets = ctx.callees(site);
        let mut owners: Vec<u32> = targets
            .iter()
            .filter_map(|&t| ctx.func(t).and_then(|f| ctx.owner_type(f)))
            .collect();
        owners.sort_unstable();
        owners.dedup();
        let exact = targets.iter().all(|&t| ctx.edge_is_exact(site, t));
        if owners.len() == 1 {
            let d = if exact { Derivation::Exact } else { Derivation::OverApproximate };
            return (owners.first().copied(), d);
        }
        if owners.len() > 1 {
            return (self.own_owner, Derivation::OverApproximate);
        }
        if let Callee::Static(f) = callee {
            return (ctx.func(*f).and_then(|tf| ctx.owner_type(tf)), Derivation::Exact);
        }
        match callee.method_symbol() {
            Some(m) => {
                let owner = ctx
                    .symbol(m)
                    .and_then(|s| s.parent)
                    .and_then(|p| ctx.classifier_of(p));
                let external = ctx.symbol(m).map_or(false, |s| s.external);
                let d = if external {
                    Derivation::Exact
                } else {
                    Derivation::partial("a called declaration has no lowered body")
                };
                (owner, d)
            }
            None => (
                self.own_owner,
                Derivation::partial("a call target is not derivable"),
            ),
        }
    }

    /// Builds the node chain of every reachable block.
    fn build_chains(&mut self) {
        let ctx = self.ctx;
        let func = self.func;
        let fid = func.id.raw();
        for b in 0..self.fg.len {
            if !self.fg.is_reachable(b) {
                continue;
            }
            let block = &func.blocks[b];
            let mut chain: Vec<ElementId> = Vec::new();
            let preds = self.fg.pred_blocks(b).len();
            let is_entry = b == self.fg.entry;
            if preds >= 2 || (is_entry && preds >= 1) {
                let id = self.node_id("act_merge", b, 0);
                let owner = self.own_owner;
                let mut node = DiagramNode::new(id.clone(), NodeKind::Merge, "")
                    .with_fact(Fact::Terminator { func: fid, block: b as u32 });
                if self.fg.loop_of_header[b].is_some() {
                    node = node.with_stereotype("loop");
                }
                chain.push(self.add(node, owner));
            }
            for (k, inst) in block.insts.iter().enumerate() {
                let span = inst_span(block, k);
                let fact = Fact::Inst {
                    func: fid,
                    block: b as u32,
                    index: k as u32,
                };
                let (label, stereotype, owner, derivation) = match inst {
                    Inst::Call {
                        dst,
                        site,
                        callee,
                        args,
                        unwind,
                    } => {
                        let (owner, derivation) = self.call_owner(*site, callee);
                        let call = format!(
                            "{}({})",
                            callee_text(ctx, &self.r, callee),
                            self.r.args(ctx, callee, args)
                        );
                        let label = match dst.and_then(|d| ctx.value_name(func, d)) {
                            Some(n) => format!("{} := {}", n, call),
                            None => call,
                        };
                        let id = self.node_id("act_action", b, k);
                        if let Some(h) = unwind {
                            let guard = edge_guard(
                                ctx,
                                &self.r,
                                func,
                                &crate::derive::compiler_views::FlowEdge {
                                    from: b,
                                    to: h.index(),
                                    label: FlowLabel::Unwind(k),
                                },
                            );
                            self.exceptional.push((id.clone(), h.index(), guard, span));
                        }
                        (label, "call", owner, derivation)
                    }
                    Inst::Alloc { ty, region, .. } => {
                        if *region == Region::Static
                            || !ctx.named_symbol(*ty).map_or(false, |s| ctx.is_classifier(s))
                        {
                            continue;
                        }
                        (format!("create {}", ctx.render_type(*ty)), "create", self.own_owner, Derivation::Exact)
                    }
                    Inst::Store { place, value } => (
                        format!("{} := {}", self.r.place(ctx, place), self.r.expr(ctx, value)),
                        "write",
                        self.own_owner,
                        Derivation::Exact,
                    ),
                    Inst::Release { place } => {
                        let destroys = match (place.base(), ctx.heap) {
                            (Some(v), Some(heap)) => {
                                let objs = ctx.objects_of(func.id, Context::Empty, v);
                                let loc = heap.locations();
                                !objs.is_empty()
                                    && objs.iter().all(|&o| {
                                        let info = loc.object(o);
                                        info.region == Some(Region::Stack) && info.func == Some(func.id)
                                    })
                            }
                            _ => false,
                        };
                        (
                            format!("release {}", self.r.place(ctx, place)),
                            if destroys { "destroy" } else { "release" },
                            self.own_owner,
                            Derivation::Exact,
                        )
                    }
                    Inst::Intrinsic { dst, name, args } => {
                        let kind = ctx.module.intrinsic_kind(*name);
                        if matches!(
                            kind,
                            Some(IntrinsicKind::TypeLiteral)
                                | Some(IntrinsicKind::OpaqueLiteral)
                                | Some(IntrinsicKind::ArrayLength)
                        ) {
                            continue;
                        }
                        let call = format!(
                            "{}({})",
                            ctx.text(*name),
                            args.iter()
                                .map(|a| self.r.operand(ctx, a))
                                .collect::<Vec<_>>()
                                .join(", ")
                        );
                        let label = match dst.and_then(|d| ctx.value_name(func, d)) {
                            Some(n) => format!("{} := {}", n, call),
                            None => call,
                        };
                        let derivation = if kind.map_or(false, |k| k.is_not_derivable()) {
                            Derivation::partial("the lowering marks a construct it cannot derive")
                        } else {
                            Derivation::Exact
                        };
                        (label, "intrinsic", self.own_owner, derivation)
                    }
                    _ => continue,
                };
                let id = self.node_id("act_action", b, k);
                let node = DiagramNode::new(id, NodeKind::Action, label)
                    .with_stereotype(stereotype)
                    .with_span(span)
                    .with_fact(fact)
                    .with_derivation(derivation);
                chain.push(self.add(node, owner));
            }
            let tspan = term_span(block);
            let tfact = Fact::Terminator {
                func: fid,
                block: b as u32,
            };
            match &block.term {
                Terminator::Branch { then_b, else_b, .. } if then_b != else_b => {
                    let id = self.node_id("act_decision", b, usize::MAX);
                    let owner = self.own_owner;
                    chain.push(self.add(
                        DiagramNode::new(id, NodeKind::Decision, "")
                            .with_span(tspan)
                            .with_fact(tfact),
                        owner,
                    ));
                }
                Terminator::Switch { .. } => {
                    let id = self.node_id("act_decision", b, usize::MAX);
                    let owner = self.own_owner;
                    chain.push(self.add(
                        DiagramNode::new(id, NodeKind::Decision, "")
                            .with_span(tspan)
                            .with_fact(tfact),
                        owner,
                    ));
                }
                Terminator::Throw(v) => {
                    let id = self.node_id("act_raise", b, usize::MAX);
                    let owner = self.own_owner;
                    let raise = self.add(
                        DiagramNode::new(id, NodeKind::Action, format!("raise {}", self.r.operand(ctx, v)))
                            .with_stereotype("raise")
                            .with_span(tspan)
                            .with_fact(tfact),
                        owner,
                    );
                    if let Some(h) = block.unwind {
                        self.exceptional.push((raise.clone(), h.index(), None, tspan));
                    }
                    chain.push(raise);
                }
                _ => {}
            }
            self.chains[b] = chain;
        }
    }

    /// First node of block `b`, passing through blocks without nodes.
    fn entry(&mut self, b: usize) -> ElementId {
        if let Some(id) = self.entry_cache.get(&b) {
            return id.clone();
        }
        let func = self.func;
        let mut seen: HashSet<usize> = HashSet::new();
        let mut cur = b;
        loop {
            if let Some(first) = self.chains.get(cur).and_then(|c| c.first()).cloned() {
                self.entry_cache.insert(b, first.clone());
                return first;
            }
            if !seen.insert(cur) {
                // A cycle of blocks without nodes: anchor it on a merge.
                let id = self.node_id("act_merge", cur, 1);
                let owner = self.own_owner;
                let merge = self.add(DiagramNode::new(id, NodeKind::Merge, ""), owner);
                self.chains[cur] = vec![merge.clone()];
                self.entry_cache.insert(b, merge.clone());
                return merge;
            }
            let term = &func.blocks[cur].term;
            let next = match term {
                Terminator::Jump(t) => Some(t.index()),
                Terminator::Branch { then_b, else_b, .. } if then_b == else_b => Some(then_b.index()),
                _ => None,
            };
            match next {
                Some(t) if t < self.fg.len => cur = t,
                _ => {
                    let id = match term {
                        Terminator::Return(_) => self.final_node(),
                        Terminator::Throw(_) => self.raise_final(),
                        _ => self.flow_final(),
                    };
                    self.entry_cache.insert(b, id.clone());
                    return id;
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn flow(&mut self, from: ElementId, to: ElementId, discriminator: u64, guard: Option<String>, label: Option<String>, span: Option<Span>, exceptional: bool) {
        let mut edge = DiagramEdge::new(
            IdBuilder::new("act_flow")
                .text(&self.key)
                .text(from.as_str())
                .text(to.as_str())
                .num(discriminator)
                .finish(),
            if exceptional {
                EdgeKind::ExceptionFlow
            } else {
                EdgeKind::ControlFlow
            },
            from,
            to,
        )
        .with_guard(guard)
        .with_span(span);
        if let Some(l) = label {
            edge = edge.with_label(l);
        }
        self.model.add_edge(edge);
    }

    fn connect(&mut self) {
        let ctx = self.ctx;
        let func = self.func;
        let initial = IdBuilder::new("act_initial").text(&self.key).finish();
        let owner = self.own_owner;
        self.add(
            DiagramNode::new(initial.clone(), NodeKind::Initial, "").with_span(func_span(func)),
            owner,
        );
        let first = self.entry(self.fg.entry);
        self.flow(initial, first, 0, None, None, None, false);

        for b in 0..self.fg.len {
            if !self.fg.is_reachable(b) {
                continue;
            }
            let chain = self.chains[b].clone();
            for w in chain.windows(2) {
                self.flow(w[0].clone(), w[1].clone(), 0, None, None, None, false);
            }
            let Some(last) = chain.last().cloned() else {
                continue;
            };
            let block = &func.blocks[b];
            let tspan = term_span(block);
            match &block.term {
                Terminator::Jump(t) => {
                    let to = self.entry(t.index());
                    self.flow(last, to, 0, None, None, tspan, false);
                }
                Terminator::Branch { then_b, else_b, .. } if then_b == else_b => {
                    let to = self.entry(then_b.index());
                    self.flow(last, to, 0, None, None, tspan, false);
                }
                Terminator::Branch { .. } | Terminator::Switch { .. } => {
                    let succs = self.fg.succs[b].clone();
                    for e in succs {
                        let edge = self.fg.edges[e];
                        if edge.label.is_exceptional() {
                            continue;
                        }
                        let guard = edge_guard(ctx, &self.r, func, &edge);
                        let to = self.entry(edge.to);
                        self.flow(last.clone(), to, e as u64, guard, None, tspan, false);
                    }
                }
                Terminator::Return(v) => {
                    let to = self.final_node();
                    let label = v.as_ref().map(|op| format!("return {}", self.r.operand(ctx, op)));
                    self.flow(last, to, 0, None, label, tspan, false);
                }
                Terminator::Throw(_) => {
                    if block.unwind.is_none() {
                        let to = self.raise_final();
                        self.flow(last, to, 0, None, None, tspan, true);
                    }
                }
                Terminator::Unreachable => {
                    let to = self.flow_final();
                    self.flow(last, to, 0, None, None, tspan, false);
                }
            }
        }
        let exceptional = std::mem::take(&mut self.exceptional);
        for (i, (from, landing, guard, span)) in exceptional.into_iter().enumerate() {
            if landing >= self.fg.len {
                continue;
            }
            let to = self.entry(landing);
            self.flow(from, to, 1_000_000 + i as u64, guard, None, span, true);
        }
    }

    fn loops(&mut self) {
        let ctx = self.ctx;
        let func = self.func;
        for lp in self.fg.loops.clone().iter() {
            let mut members: Vec<ElementId> = Vec::new();
            for &b in &lp.body {
                members.extend(self.chains.get(b).cloned().unwrap_or_default());
            }
            members.sort();
            members.dedup();
            if members.is_empty() {
                continue;
            }
            let guard = lp
                .exits
                .iter()
                .map(|&e| self.fg.edges[e])
                .find(|e| e.from == lp.header && !e.label.is_exceptional())
                .and_then(|exit| {
                    // The guard of staying in the loop is the other arm of the header's branch.
                    self.fg.succs[lp.header]
                        .iter()
                        .map(|&e| self.fg.edges[e])
                        .find(|e| e.to != exit.to && lp.contains(e.to) && !e.label.is_exceptional())
                })
                .and_then(|stay| edge_guard(ctx, &self.r, func, &stay));
            let id = IdBuilder::new("act_loop")
                .text(&self.key)
                .num(lp.header as u64)
                .finish();
            self.model.add_fragment(Fragment {
                id,
                kind: FragmentKind::Loop,
                parent: lp.parent.map(|p| {
                    IdBuilder::new("act_loop")
                        .text(&self.key)
                        .num(self.fg.loops[p].header as u64)
                        .finish()
                }),
                covered: members.clone(),
                operands: vec![FragmentOperand { guard, members }],
                spans: term_span(&func.blocks[lp.header]).into_iter().collect(),
                facts: vec![Fact::Loop {
                    func: func.id.raw(),
                    header: lp.header as u32,
                }],
                derivation: Derivation::Exact,
            });
        }
        if self.fg.irreducible {
            self.model.weaken(&Derivation::partial(
                "the function has irreducible control flow: some cycles are not natural loops and have no loop region",
            ));
        }
    }
}

/// The activity diagram of one function.
pub fn derive_function(ctx: &DeriveContext, func_id: FuncId) -> DiagramModel {
    let kind = DiagramKind::Activity;
    let Some(func) = ctx.func(func_id) else {
        return DiagramModel::not_derivable(kind, kind.title(), "the function is not part of the module");
    };
    let qn = ctx.func_qualified_name(func);
    if func.blocks.is_empty() {
        return DiagramModel::not_derivable(
            kind,
            format!("act {}", qn),
            "the function has no lowered body",
        )
        .with_scope(qn);
    }
    let fg = FlowGraph::build(func);
    let len = fg.len;
    let unreachable = (0..len).filter(|&i| !fg.is_reachable(i)).count();
    let mut b = ActivityBuilder {
        ctx,
        func,
        fg,
        r: ValueRenderer::new(func),
        key: format!("{}#{}", qn, func.id.raw()),
        model: DiagramModel::new(kind, format!("act {}", qn)).with_scope(qn),
        partitions: HashMap::new(),
        own_owner: ctx.owner_type(func),
        chains: vec![Vec::new(); len],
        exceptional: Vec::new(),
        final_node: None,
        raise_final: None,
        flow_final: None,
        entry_cache: HashMap::new(),
    };
    b.build_chains();
    b.connect();
    b.loops();
    let mut model = b.model;
    if unreachable > 0 {
        model.note(format!(
            "{} block(s) unreachable from the entry are not shown",
            unreachable
        ));
    }
    model.finalize(ctx.limits.max_spans);
    model
}

/// Activity diagrams of the functions the views cover.
pub fn derive(ctx: &DeriveContext) -> Vec<DiagramModel> {
    let mut funcs = view_functions(ctx);
    if funcs.is_empty() {
        return vec![DiagramModel::not_derivable(
            DiagramKind::Activity,
            DiagramKind::Activity.title(),
            "the module has no function with a lowered body",
        )];
    }
    let cut = cap(&mut funcs, ctx.limits.max_diagrams);
    let mut out: Vec<DiagramModel> = funcs.into_iter().map(|f| derive_function(ctx, f)).collect();
    if cut > 0 {
        out.push(DiagramModel::omission(
            DiagramKind::Activity,
            cut,
            ctx.limits.max_diagrams,
        ));
    }
    out
}
