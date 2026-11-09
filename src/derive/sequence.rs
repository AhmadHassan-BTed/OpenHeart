//! Sequence diagrams: an interprocedural walk over the IR from each entry point, in
//! control-flow order, through the call graph.
//!
//! * **Lifelines** are the receivers of messages: the abstract objects the receiver's
//!   points-to set holds in the caller's context, the owning type of a call without a
//!   receiver, or the namespace of a free function.
//! * **Messages** are calls (asynchronous for spawned tasks), object creations
//!   (allocations of classifier instances), releases at scope exit (destruction when the
//!   released object lives on the releasing function's stack), returns and raises.
//! * **Fragments.** `alt`/`opt` come from branches and switches, closed at the branch's
//!   immediate post-dominator; `loop` from natural loops, guarded by the loop's exit
//!   condition; `break` from loop exits and from exception landing blocks; `par` from spawn
//!   idioms, closed at a matching join in the spawner's straight-line continuation;
//!   `critical` from monitor-guarded regions and lock idioms. A call with several dispatch
//!   targets is an `alt` with one operand per target, guarded by the receiver objects'
//!   dynamic types.
//! * **Cut-offs.** Recursion is folded (a call to a function already on the walk stack is
//!   drawn, not entered). Depth, message and step limits mark the interaction `Partial`.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::rc::Rc;

use crate::derive::compiler_views::FlowGraph;
use crate::derive::model::{
    DiagramEdge, DiagramKind, DiagramModel, DiagramNode, EdgeKind, ElementId, Fragment,
    FragmentKind, FragmentOperand, IdBuilder, MessageSort, NodeKind,
};
use crate::derive::model::Derivation;
use crate::derive::object::{object_id, object_label, object_type};
use crate::derive::provenance::{
    call_site_span, callee_text, const_text, func_span, inst_span, object_span, symbol_span,
    term_span, Fact, ValueRenderer,
};
use crate::derive::{
    cap, ConcurrencyRole, DeriveContext, EntryPointFact, HandleSource, Invoker, TaskSource,
};
use crate::ir::func::FuncIr;
use crate::ir::inst::{Callee, Inst, IntrinsicKind, Region, Terminator};
use crate::ir::types::{AllocSiteId, BlockId, CallSiteId, FuncId, Span, TypeId, ValueId};
use crate::ir::value::{ConstVal, Operand, Place};
use crate::memory::callgraph::Dispatch;
use crate::memory::location::{Context, HeapBase, ObjId};

// ── Trace ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LifelineKind {
    /// The invoker of the entry point.
    Actor,
    /// No invoker is known: the source of found and the target of lost messages.
    Gate,
    /// Abstract objects of the heap.
    Objects,
    /// A type (static members, or a receiver without points-to facts).
    Classifier,
    Namespace,
    File,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct Lifeline {
    /// Identity text (hashed into the element id).
    pub key: String,
    pub label: String,
    pub kind: LifelineKind,
    pub symbol: Option<u32>,
    pub objects: Vec<ObjId>,
    pub spans: Vec<Span>,
    pub facts: Vec<Fact>,
    pub derivation: Derivation,
}

#[derive(Debug, Clone)]
pub struct TraceMessage {
    pub order: u32,
    pub from: usize,
    pub to: usize,
    pub sort: MessageSort,
    pub label: String,
    /// Decimal sequence number (`1.2.1`); a reply carries the number of its call.
    pub number: String,
    pub caller: FuncId,
    pub block: usize,
    pub site: Option<CallSiteId>,
    pub callee: Option<FuncId>,
    /// Block of the entry function whose step (directly or through calls) sent the message.
    pub root_block: Option<usize>,
    pub depth: usize,
    /// Sent inside a loop (an iterated message of a communication diagram).
    pub in_loop: bool,
    /// Guard of the innermost enclosing conditional operand.
    pub guard: Option<String>,
    pub recursive: bool,
    pub spans: Vec<Span>,
    pub facts: Vec<Fact>,
    pub derivation: Derivation,
}

/// An operand covering the messages `start..end` (by order).
#[derive(Debug, Clone)]
pub struct TraceOperand {
    pub guard: Option<String>,
    pub start: u32,
    pub end: u32,
}

#[derive(Debug, Clone)]
pub struct TraceFragment {
    pub kind: FragmentKind,
    pub parent: Option<usize>,
    /// Lifeline of the function the fragment's control structure belongs to.
    pub owner: usize,
    pub operands: Vec<TraceOperand>,
    pub func: FuncId,
    pub block: usize,
    pub spans: Vec<Span>,
    pub facts: Vec<Fact>,
    pub derivation: Derivation,
}

#[derive(Debug, Clone)]
pub struct InteractionTrace {
    pub entry: EntryPointFact,
    /// Stable identity of the interaction (entry function and id).
    pub key: String,
    pub title: String,
    pub lifelines: Vec<Lifeline>,
    pub messages: Vec<TraceMessage>,
    pub fragments: Vec<TraceFragment>,
    pub notes: Vec<String>,
    pub derivation: Derivation,
}

/// The interactions of every entry point.
#[derive(Debug, Clone, Default)]
pub struct Traces {
    pub traces: Vec<InteractionTrace>,
    /// Entry points left out by the per-kind diagram limit.
    pub omitted: usize,
}

// ── Walker ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct ActiveLoop {
    index: usize,
    /// Stop set of the region the loop was entered from.
    outer_stop: Vec<usize>,
}

#[derive(Debug, Clone)]
struct Frame {
    func: FuncId,
    lifeline: usize,
    context: Context,
    number: String,
    children: u32,
    active_loops: Vec<ActiveLoop>,
    caller_lifeline: Option<usize>,
    asynchronous: bool,
}

#[derive(Debug, Clone)]
struct OpenOperand {
    fragment: usize,
    kind: FragmentKind,
    guard: Option<String>,
}

#[derive(Debug, Clone)]
struct CriticalRegion {
    lock: Option<ValueId>,
    lock_objects: Vec<ObjId>,
    /// `open.len()` right after the critical operand was opened.
    depth: usize,
    frame: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RegionEnd {
    /// Control reached a block of the stop set (or an active loop header).
    Reached(usize),
    /// Every path returned, raised or became unreachable.
    Terminated,
    /// The walk arrived at the requested end position.
    Joined,
    /// A budget ran out.
    Halted,
}

enum Flow {
    Continue(usize),
    End(RegionEnd),
}

enum Step {
    Next,
    Consumed(RegionEnd),
}

struct Walker<'c, 'a> {
    ctx: &'c DeriveContext<'a>,
    trace: InteractionTrace,
    lifeline_of: HashMap<String, usize>,
    flows: HashMap<FuncId, Rc<FlowGraph>>,
    renderers: HashMap<FuncId, Rc<ValueRenderer<'a>>>,
    frames: Vec<Frame>,
    open: Vec<OpenOperand>,
    criticals: Vec<CriticalRegion>,
    steps: usize,
    halted: bool,
    root_block: Option<usize>,
    top_children: u32,
}

impl<'c, 'a> Walker<'c, 'a> {
    fn new(ctx: &'c DeriveContext<'a>, entry: &EntryPointFact, key: String, title: String) -> Self {
        Self {
            ctx,
            trace: InteractionTrace {
                entry: entry.clone(),
                key,
                title,
                lifelines: Vec::new(),
                messages: Vec::new(),
                fragments: Vec::new(),
                notes: Vec::new(),
                derivation: Derivation::Exact,
            },
            lifeline_of: HashMap::new(),
            flows: HashMap::new(),
            renderers: HashMap::new(),
            frames: Vec::new(),
            open: Vec::new(),
            criticals: Vec::new(),
            steps: 0,
            halted: false,
            root_block: None,
            top_children: 0,
        }
    }

    // ── bookkeeping ──

    fn partial(&mut self, reason: impl Into<String>) {
        let reason = reason.into();
        self.trace.derivation.weaken(&Derivation::partial(reason.clone()));
        self.note(reason);
    }

    fn note(&mut self, text: String) {
        if !self.trace.notes.contains(&text) {
            self.trace.notes.push(text);
        }
    }

    fn halt(&mut self, reason: String) {
        if !self.halted {
            self.halted = true;
            self.partial(reason);
        }
    }

    fn flow(&mut self, func: &FuncIr) -> Rc<FlowGraph> {
        self.flows
            .entry(func.id)
            .or_insert_with(|| Rc::new(FlowGraph::build(func)))
            .clone()
    }

    fn renderer(&mut self, func: &'a FuncIr) -> Rc<ValueRenderer<'a>> {
        self.renderers
            .entry(func.id)
            .or_insert_with(|| Rc::new(ValueRenderer::new(func)))
            .clone()
    }

    fn current_lifeline(&self) -> usize {
        self.frames.last().map(|f| f.lifeline).unwrap_or(0)
    }

    fn current_context(&self) -> Context {
        self.frames.last().map(|f| f.context).unwrap_or(Context::Empty)
    }

    fn is_active_header(&self, fg: &FlowGraph, b: usize) -> bool {
        self.frames.last().map_or(false, |f| {
            f.active_loops.iter().any(|a| fg.loops[a.index].header == b)
        })
    }

    // ── lifelines ──

    fn lifeline(&mut self, l: Lifeline) -> usize {
        if let Some(&i) = self.lifeline_of.get(&l.key) {
            let existing = &mut self.trace.lifelines[i];
            existing.derivation.weaken(&l.derivation);
            return i;
        }
        let i = self.trace.lifelines.len();
        self.lifeline_of.insert(l.key.clone(), i);
        self.trace.lifelines.push(l);
        i
    }

    fn actor_lifeline(&mut self, entry: &EntryPointFact) -> (usize, bool) {
        let ctx = self.ctx;
        match &entry.invoker {
            Some(Invoker::Symbol(s)) => {
                let qn = ctx.qualified_name(*s);
                let l = Lifeline {
                    key: format!("actor:symbol:{}", qn),
                    label: ctx.symbol_name(*s),
                    kind: LifelineKind::Actor,
                    symbol: Some(*s),
                    objects: Vec::new(),
                    spans: symbol_span(ctx, *s).into_iter().collect(),
                    facts: vec![Fact::Symbol(*s)],
                    derivation: Derivation::Exact,
                };
                (self.lifeline(l), true)
            }
            Some(Invoker::Role(r)) => {
                let l = Lifeline {
                    key: format!("actor:role:{}", r),
                    label: r.clone(),
                    kind: LifelineKind::Actor,
                    symbol: None,
                    objects: Vec::new(),
                    spans: Vec::new(),
                    facts: vec![Fact::EntryPoint(entry.func.raw())],
                    derivation: Derivation::Exact,
                };
                (self.lifeline(l), true)
            }
            None => (self.gate(), false),
        }
    }

    fn gate(&mut self) -> usize {
        self.lifeline(Lifeline {
            key: "gate".to_string(),
            label: String::new(),
            kind: LifelineKind::Gate,
            symbol: None,
            objects: Vec::new(),
            spans: Vec::new(),
            facts: Vec::new(),
            derivation: Derivation::Exact,
        })
    }

    fn object_lifeline(&mut self, objs: &[ObjId]) -> usize {
        let ctx = self.ctx;
        let Some(heap) = ctx.heap else {
            return self.gate();
        };
        let loc = heap.locations();
        let ids: Vec<String> = objs
            .iter()
            .map(|&o| object_id(ctx, loc, o).to_string())
            .collect();
        let key = format!("objects:{}", ids.join(","));
        if let Some(&i) = self.lifeline_of.get(&key) {
            return i;
        }
        let (label, derivation) = if objs.len() == 1 {
            (object_label(ctx, loc, objs[0]), Derivation::Exact)
        } else {
            let mut types: Vec<String> = objs
                .iter()
                .map(|&o| object_type(ctx, loc, o).unwrap_or_else(|| "?".to_string()))
                .collect();
            types.sort();
            types.dedup();
            (format!(": {}", types.join(" | ")), Derivation::OverApproximate)
        };
        let symbol = if objs.len() == 1 {
            loc.object(objs[0]).dynamic_type
        } else {
            None
        };
        let opaque = objs.iter().any(|&o| loc.object(o).object.base.is_opaque());
        let derivation = if opaque {
            Derivation::partial("a receiver object comes from outside the analysed code")
        } else {
            derivation
        };
        self.lifeline(Lifeline {
            key,
            label,
            kind: LifelineKind::Objects,
            symbol,
            objects: objs.to_vec(),
            spans: objs.iter().filter_map(|&o| object_span(ctx, o)).collect(),
            facts: objs.iter().map(|o| Fact::Object(o.0)).collect(),
            derivation,
        })
    }

    fn type_lifeline(&mut self, sym: u32, derivation: Derivation) -> usize {
        let ctx = self.ctx;
        self.lifeline(Lifeline {
            key: format!("type:{}", ctx.qualified_name(sym)),
            label: ctx.symbol_name(sym),
            kind: LifelineKind::Classifier,
            symbol: Some(sym),
            objects: Vec::new(),
            spans: symbol_span(ctx, sym).into_iter().collect(),
            facts: vec![Fact::Symbol(sym)],
            derivation,
        })
    }

    fn namespace_lifeline(&mut self, sym: u32) -> usize {
        let ctx = self.ctx;
        self.lifeline(Lifeline {
            key: format!("namespace:{}", ctx.qualified_name(sym)),
            label: ctx.symbol_name(sym),
            kind: LifelineKind::Namespace,
            symbol: Some(sym),
            objects: Vec::new(),
            spans: symbol_span(ctx, sym).into_iter().collect(),
            facts: vec![Fact::Symbol(sym)],
            derivation: Derivation::Exact,
        })
    }

    fn file_lifeline(&mut self, file_id: u32) -> usize {
        let label = self.ctx.file_label(file_id);
        self.lifeline(Lifeline {
            key: format!("file:{}", label),
            label,
            kind: LifelineKind::File,
            symbol: None,
            objects: Vec::new(),
            spans: Vec::new(),
            facts: Vec::new(),
            derivation: Derivation::Exact,
        })
    }

    /// Participant that receives a call to `target` (or to the declared `symbol`).
    fn participant(
        &mut self,
        objs: &[ObjId],
        owner: Option<u32>,
        has_receiver: bool,
        target: Option<&FuncIr>,
        symbol: Option<u32>,
    ) -> usize {
        let ctx = self.ctx;
        if !objs.is_empty() {
            return self.object_lifeline(objs);
        }
        let derivation = if has_receiver {
            Derivation::partial(if ctx.heap.is_some() {
                "a receiver's points-to set is empty in its context: its lifeline stands for the declared type"
            } else {
                "points-to facts are unavailable: receiver lifelines stand for declared types"
            })
        } else {
            Derivation::Exact
        };
        if let Some(o) = owner {
            return self.type_lifeline(o, derivation);
        }
        let sym = target.and_then(|f| f.symbol).or(symbol);
        if let Some(ns) = sym.and_then(|s| ctx.namespace_of(s)) {
            return self.namespace_lifeline(ns);
        }
        if let Some(f) = target {
            if !f.span.is_none() {
                return self.file_lifeline(f.span.file_id);
            }
        }
        let name = target
            .map(|f| ctx.func_name(f))
            .or_else(|| sym.map(|s| ctx.symbol_name(s)))
            .unwrap_or_default();
        self.lifeline(Lifeline {
            key: format!("unknown:{}", name),
            label: name,
            kind: LifelineKind::Unknown,
            symbol: sym,
            objects: Vec::new(),
            spans: Vec::new(),
            facts: Vec::new(),
            derivation: Derivation::partial("a participant has no owner, namespace or source file"),
        })
    }

    // ── messages ──

    #[allow(clippy::too_many_arguments)]
    fn emit(
        &mut self,
        from: usize,
        to: usize,
        sort: MessageSort,
        label: String,
        number: Option<String>,
        caller: FuncId,
        block: usize,
        site: Option<CallSiteId>,
        callee: Option<FuncId>,
        spans: Vec<Span>,
        facts: Vec<Fact>,
        derivation: Derivation,
    ) -> Option<usize> {
        if self.trace.messages.len() >= self.ctx.limits.max_messages {
            self.halt(format!(
                "the interaction is cut off at {} messages",
                self.ctx.limits.max_messages
            ));
            return None;
        }
        let number = match number {
            Some(n) => n,
            None => match self.frames.last_mut() {
                Some(f) => {
                    f.children += 1;
                    if f.number.is_empty() {
                        f.children.to_string()
                    } else {
                        format!("{}.{}", f.number, f.children)
                    }
                }
                None => {
                    self.top_children += 1;
                    self.top_children.to_string()
                }
            },
        };
        let in_loop = self.open.iter().any(|o| o.kind == FragmentKind::Loop);
        let guard = self.open.iter().rev().find_map(|o| match o.kind {
            FragmentKind::Alt | FragmentKind::Opt | FragmentKind::Break => o.guard.clone(),
            _ => None,
        });
        let order = self.trace.messages.len() as u32;
        self.trace.messages.push(TraceMessage {
            order,
            from,
            to,
            sort,
            label,
            number,
            caller,
            block,
            site,
            callee,
            root_block: self.root_block,
            depth: self.frames.len(),
            in_loop,
            guard,
            recursive: false,
            spans,
            facts,
            derivation,
        });
        Some(order as usize)
    }

    // ── fragments ──

    fn open_fragment(&mut self, kind: FragmentKind, func: FuncId, block: usize, span: Option<Span>, fact: Option<Fact>) -> usize {
        let parent = self.open.last().map(|o| o.fragment);
        let idx = self.trace.fragments.len();
        self.trace.fragments.push(TraceFragment {
            kind,
            parent,
            owner: self.current_lifeline(),
            operands: Vec::new(),
            func,
            block,
            spans: span.into_iter().collect(),
            facts: fact.into_iter().collect(),
            derivation: Derivation::Exact,
        });
        idx
    }

    fn open_operand(&mut self, fragment: usize, guard: Option<String>) {
        let start = self.trace.messages.len() as u32;
        let kind = self.trace.fragments[fragment].kind;
        self.trace.fragments[fragment].operands.push(TraceOperand {
            guard: guard.clone(),
            start,
            end: start,
        });
        self.open.push(OpenOperand {
            fragment,
            kind,
            guard,
        });
    }

    fn close_operand(&mut self) {
        if let Some(o) = self.open.pop() {
            let end = self.trace.messages.len() as u32;
            if let Some(op) = self.trace.fragments[o.fragment].operands.last_mut() {
                op.end = end;
            }
        }
    }

    // ── walk ──

    fn run(&mut self) {
        let ctx = self.ctx;
        let entry = self.trace.entry.clone();
        let Some(func) = ctx.func(entry.func) else {
            self.partial("the entry point has no lowered function");
            return;
        };
        let (actor, known_invoker) = self.actor_lifeline(&entry);
        let receiver = func.receiver();
        let objs = receiver
            .map(|r| ctx.objects_of(func.id, Context::Empty, r))
            .unwrap_or_default();
        let target = self.participant(&objs, ctx.owner_type(func), receiver.is_some(), Some(func), None);
        let r = self.renderer(func);
        let params: Vec<String> = func
            .declared_params()
            .iter()
            .map(|&p| r.name(ctx, p))
            .collect();
        let label = format!("{}({})", ctx.func_name(func), params.join(", "));
        let sort = if known_invoker {
            MessageSort::SynchCall
        } else {
            MessageSort::Found
        };
        let mut spans: Vec<Span> = entry.span.into_iter().collect();
        spans.extend(func_span(func));
        let Some(mi) = self.emit(
            actor,
            target,
            sort,
            label,
            None,
            func.id,
            func.entry.index(),
            None,
            Some(func.id),
            spans,
            vec![Fact::EntryPoint(func.id.raw())],
            Derivation::Exact,
        ) else {
            return;
        };
        let number = self.trace.messages[mi].number.clone();
        self.frames.push(Frame {
            func: func.id,
            lifeline: target,
            context: Context::Empty,
            number,
            children: 0,
            active_loops: Vec::new(),
            caller_lifeline: Some(actor),
            asynchronous: false,
        });
        self.walk_function(func);
        self.frames.pop();
        while !self.open.is_empty() {
            self.close_operand();
        }
    }

    fn walk_function(&mut self, func: &'a FuncIr) -> RegionEnd {
        if func.blocks.is_empty() {
            return RegionEnd::Terminated;
        }
        let fg = self.flow(func);
        if fg.irreducible {
            self.partial(format!(
                "{} has irreducible control flow: cycles without a dominating header are walked once",
                self.ctx.func_qualified_name(func)
            ));
        }
        self.walk_region(&fg, func, fg.entry, 0, &[], None)
    }

    fn walk_region(
        &mut self,
        fg: &FlowGraph,
        func: &'a FuncIr,
        start: usize,
        start_inst: usize,
        stop: &[usize],
        end_at: Option<(usize, usize)>,
    ) -> RegionEnd {
        let mut b = start;
        let mut i = start_inst;
        let mut first = true;
        let mut visited: HashSet<usize> = HashSet::new();
        let critical_depth = self.criticals.len();
        let result = loop {
            if self.halted {
                break RegionEnd::Halted;
            }
            if b >= fg.len {
                break RegionEnd::Terminated;
            }
            if i == 0 {
                if !first && (stop.contains(&b) || self.is_active_header(fg, b)) {
                    break RegionEnd::Reached(b);
                }
                if let Some(l) = fg.loop_of_header[b] {
                    let active = self
                        .frames
                        .last()
                        .map_or(false, |f| f.active_loops.iter().any(|a| a.index == l));
                    if !active {
                        match self.walk_loop(fg, func, l, stop) {
                            Flow::Continue(next) => {
                                b = next;
                                i = 0;
                                first = false;
                                continue;
                            }
                            Flow::End(end) => break end,
                        }
                    }
                }
                if !visited.insert(b) {
                    self.partial(
                        "control flow re-enters a block outside any natural loop; the walk stops at the re-entry",
                    );
                    break RegionEnd::Terminated;
                }
                self.steps += 1;
                if self.steps > self.ctx.limits.max_walk_steps {
                    self.halt(format!(
                        "the walk is cut off after {} block visits",
                        self.ctx.limits.max_walk_steps
                    ));
                    break RegionEnd::Halted;
                }
                if self.frames.len() == 1 {
                    self.root_block = Some(b);
                }
            }
            first = false;
            let block = &func.blocks[b];
            let mut consumed: Option<RegionEnd> = None;
            let mut k = i;
            while k < block.insts.len() {
                if end_at == Some((b, k)) {
                    consumed = Some(RegionEnd::Joined);
                    break;
                }
                match self.step_inst(fg, func, b, k, stop) {
                    Step::Next => k += 1,
                    Step::Consumed(end) => {
                        consumed = Some(end);
                        break;
                    }
                }
                if self.halted {
                    consumed = Some(RegionEnd::Halted);
                    break;
                }
            }
            if let Some(end) = consumed {
                break end;
            }
            match &block.term {
                Terminator::Jump(t) => {
                    b = t.index();
                    i = 0;
                }
                Terminator::Branch {
                    cond,
                    then_b,
                    else_b,
                } => match self.branch(fg, func, b, cond, then_b.index(), else_b.index(), stop) {
                    Flow::Continue(next) => {
                        b = next;
                        i = 0;
                    }
                    Flow::End(end) => break end,
                },
                Terminator::Switch {
                    scrutinee,
                    cases,
                    default,
                } => match self.switch(fg, func, b, scrutinee, cases, default.index(), stop) {
                    Flow::Continue(next) => {
                        b = next;
                        i = 0;
                    }
                    Flow::End(end) => break end,
                },
                Terminator::Return(v) => {
                    self.emit_return(func, b, v.as_ref(), false);
                    break RegionEnd::Terminated;
                }
                Terminator::Throw(v) => match block.unwind {
                    Some(h) => {
                        b = h.index();
                        i = 0;
                    }
                    None => {
                        self.emit_return(func, b, Some(v), true);
                        break RegionEnd::Terminated;
                    }
                },
                Terminator::Unreachable => break RegionEnd::Terminated,
            }
        };
        while self.criticals.len() > critical_depth {
            self.close_critical();
        }
        result
    }

    fn walk_arm(&mut self, fg: &FlowGraph, func: &'a FuncIr, arm: usize, stop: &[usize]) -> RegionEnd {
        if stop.contains(&arm) || self.is_active_header(fg, arm) {
            RegionEnd::Reached(arm)
        } else {
            self.walk_region(fg, func, arm, 0, stop, None)
        }
    }

    fn loop_guard(&mut self, fg: &FlowGraph, func: &'a FuncIr, l: usize) -> Option<String> {
        let ctx = self.ctx;
        let r = self.renderer(func);
        let lp = &fg.loops[l];
        let branch_guard = |b: usize, stay_on: &dyn Fn(usize) -> bool| -> Option<String> {
            match &func.blocks[b].term {
                Terminator::Branch {
                    cond,
                    then_b,
                    else_b,
                } => {
                    let (t, e) = (then_b.index(), else_b.index());
                    match (stay_on(t), stay_on(e)) {
                        (true, false) => Some(r.condition(ctx, cond, true)),
                        (false, true) => Some(r.condition(ctx, cond, false)),
                        _ => None,
                    }
                }
                _ => None,
            }
        };
        if let Some(g) = branch_guard(lp.header, &|x: usize| lp.contains(x)) {
            return Some(g);
        }
        for &latch in &lp.latches {
            if let Some(g) = branch_guard(latch, &|x: usize| x == lp.header) {
                return Some(g);
            }
        }
        None
    }

    fn walk_loop(&mut self, fg: &FlowGraph, func: &'a FuncIr, l: usize, stop: &[usize]) -> Flow {
        let lp = &fg.loops[l];
        let header = lp.header;
        let guard = self.loop_guard(fg, func, l);
        let frag = self.open_fragment(
            FragmentKind::Loop,
            func.id,
            header,
            term_span(&func.blocks[header]),
            Some(Fact::Loop {
                func: func.id.raw(),
                header: header as u32,
            }),
        );
        self.open_operand(frag, guard);
        if let Some(f) = self.frames.last_mut() {
            f.active_loops.push(ActiveLoop {
                index: l,
                outer_stop: stop.to_vec(),
            });
        }
        let mut inner_stop: Vec<usize> = stop.to_vec();
        inner_stop.extend(lp.follow);
        inner_stop.extend(lp.exits.iter().map(|&e| fg.edges[e].to));
        inner_stop.sort_unstable();
        inner_stop.dedup();
        let end = self.walk_region(fg, func, header, 0, &inner_stop, None);
        if let Some(f) = self.frames.last_mut() {
            f.active_loops.pop();
        }
        self.close_operand();
        if end == RegionEnd::Halted {
            return Flow::End(RegionEnd::Halted);
        }
        match lp.follow {
            Some(f) if stop.contains(&f) || self.is_active_header(fg, f) => {
                Flow::End(RegionEnd::Reached(f))
            }
            Some(f) => Flow::Continue(f),
            None => Flow::End(RegionEnd::Terminated),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn branch(
        &mut self,
        fg: &FlowGraph,
        func: &'a FuncIr,
        b: usize,
        cond: &Operand,
        t: usize,
        e: usize,
        stop: &[usize],
    ) -> Flow {
        let ctx = self.ctx;
        if t == e {
            return Flow::Continue(t);
        }
        if let Operand::Const(ConstVal::Bool(v)) = cond {
            return Flow::Continue(if *v { t } else { e });
        }
        let r = self.renderer(func);
        let active = self.frames.last().and_then(|f| f.active_loops.last().cloned());
        if let Some(active) = active {
            let lp = &fg.loops[active.index];
            let (in_t, in_e) = (lp.contains(t), lp.contains(e));
            if in_t != in_e {
                let (stay, leave, leave_holds) = if in_t { (t, e, false) } else { (e, t, true) };
                if b == lp.header {
                    return Flow::Continue(stay);
                }
                let frag = self.open_fragment(
                    FragmentKind::Break,
                    func.id,
                    b,
                    term_span(&func.blocks[b]),
                    Some(Fact::Terminator {
                        func: func.id.raw(),
                        block: b as u32,
                    }),
                );
                self.open_operand(frag, Some(r.condition(ctx, cond, leave_holds)));
                let mut break_stop = active.outer_stop.clone();
                break_stop.extend(lp.follow);
                let end = self.walk_arm(fg, func, leave, &break_stop);
                self.close_operand();
                if end == RegionEnd::Halted {
                    return Flow::End(RegionEnd::Halted);
                }
                return Flow::Continue(stay);
            }
        }
        let merge = fg.ipdom_block(b);
        let mut arm_stop = stop.to_vec();
        arm_stop.extend(merge);
        let kind = if merge == Some(t) || merge == Some(e) {
            FragmentKind::Opt
        } else {
            FragmentKind::Alt
        };
        let frag = self.open_fragment(
            kind,
            func.id,
            b,
            term_span(&func.blocks[b]),
            Some(Fact::Terminator {
                func: func.id.raw(),
                block: b as u32,
            }),
        );
        let arms: Vec<(usize, bool)> = if kind == FragmentKind::Opt {
            if merge == Some(e) {
                vec![(t, true)]
            } else {
                vec![(e, false)]
            }
        } else {
            vec![(t, true), (e, false)]
        };
        for (arm, holds) in arms {
            self.open_operand(frag, Some(r.condition(ctx, cond, holds)));
            let end = self.walk_arm(fg, func, arm, &arm_stop);
            self.close_operand();
            if end == RegionEnd::Halted {
                return Flow::End(RegionEnd::Halted);
            }
        }
        match merge {
            Some(m) => Flow::Continue(m),
            None => Flow::End(RegionEnd::Terminated),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn switch(
        &mut self,
        fg: &FlowGraph,
        func: &'a FuncIr,
        b: usize,
        scrutinee: &Operand,
        cases: &[(ConstVal, BlockId)],
        default: usize,
        stop: &[usize],
    ) -> Flow {
        let ctx = self.ctx;
        let r = self.renderer(func);
        let subject = r.expr(ctx, scrutinee);
        let mut arms: Vec<(usize, Vec<String>)> = Vec::new();
        for (c, target) in cases {
            let t = target.index();
            let g = format!("{} = {}", subject, const_text(ctx, c));
            match arms.iter_mut().find(|a| a.0 == t) {
                Some(a) => a.1.push(g),
                None => arms.push((t, vec![g])),
            }
        }
        match arms.iter_mut().find(|a| a.0 == default) {
            Some(a) => a.1 = vec!["else".to_string()],
            None => arms.push((default, vec!["else".to_string()])),
        }
        if arms.len() == 1 {
            return Flow::Continue(arms[0].0);
        }
        let merge = fg.ipdom_block(b);
        let mut arm_stop = stop.to_vec();
        arm_stop.extend(merge);
        let frag = self.open_fragment(
            FragmentKind::Alt,
            func.id,
            b,
            term_span(&func.blocks[b]),
            Some(Fact::Terminator {
                func: func.id.raw(),
                block: b as u32,
            }),
        );
        let else_last: Vec<(usize, Vec<String>)> = {
            let (mut normal, elses): (Vec<_>, Vec<_>) =
                arms.into_iter().partition(|a| a.1 != ["else".to_string()]);
            normal.extend(elses);
            normal
        };
        for (target, guards) in else_last {
            self.open_operand(frag, Some(guards.join(" or ")));
            let end = if Some(target) == merge {
                RegionEnd::Reached(target)
            } else {
                self.walk_arm(fg, func, target, &arm_stop)
            };
            self.close_operand();
            if end == RegionEnd::Halted {
                return Flow::End(RegionEnd::Halted);
            }
        }
        match merge {
            Some(m) => Flow::Continue(m),
            None => Flow::End(RegionEnd::Terminated),
        }
    }

    fn step_inst(&mut self, fg: &FlowGraph, func: &'a FuncIr, b: usize, k: usize, stop: &[usize]) -> Step {
        let ctx = self.ctx;
        let inst = &func.blocks[b].insts[k];
        match inst {
            Inst::Call {
                dst,
                site,
                callee,
                args,
                unwind,
            } => {
                match ctx.program.concurrency(*site) {
                    Some(ConcurrencyRole::Spawn { task, handle }) => {
                        return Step::Consumed(self.spawn(fg, func, b, k, *dst, *site, callee, args, task, handle, stop));
                    }
                    Some(ConcurrencyRole::Lock { lock }) => {
                        self.call(func, b, k, *dst, *site, callee, args, false);
                        let lock_value = self.handle_value(*dst, *site, callee, args, lock);
                        self.open_critical(func, b, k, lock_value);
                        return Step::Next;
                    }
                    Some(ConcurrencyRole::Unlock { lock }) => {
                        let lock_value = self.handle_value(*dst, *site, callee, args, lock);
                        if !self.end_critical(func, lock_value) {
                            self.call(func, b, k, *dst, *site, callee, args, false);
                        }
                        return Step::Next;
                    }
                    Some(ConcurrencyRole::Join { .. }) | None => {}
                }
                self.call(func, b, k, *dst, *site, callee, args, false);
                if let Some(h) = unwind {
                    self.exception_operand(fg, func, b, k, *site, callee, h.index(), stop);
                }
                Step::Next
            }
            Inst::Alloc {
                dst,
                site,
                ty,
                region,
            } => {
                if *region != Region::Static
                    && ctx.named_symbol(*ty).map_or(false, |s| ctx.is_classifier(s))
                {
                    self.create(func, b, k, *dst, *site, *ty);
                }
                Step::Next
            }
            Inst::Release { place } => {
                self.release(func, b, k, place);
                Step::Next
            }
            Inst::Intrinsic { name, args, .. } => {
                if ctx.module.intrinsic_kind(*name) == Some(IntrinsicKind::MonitorEnter) {
                    let lock = args.first().and_then(|a| a.as_value());
                    self.open_critical(func, b, k, lock);
                }
                Step::Next
            }
            _ => Step::Next,
        }
    }

    // ── calls ──

    fn receiver_of(&self, site: CallSiteId, callee: &Callee) -> Option<ValueId> {
        callee
            .receiver()
            .or_else(|| self.ctx.call_site(site).and_then(|cs| cs.receiver))
    }

    #[allow(clippy::too_many_arguments)]
    fn call(
        &mut self,
        func: &'a FuncIr,
        b: usize,
        k: usize,
        dst: Option<ValueId>,
        site: CallSiteId,
        callee: &Callee,
        args: &[Operand],
        asynchronous: bool,
    ) {
        let ctx = self.ctx;
        let r = self.renderer(func);
        let me = self.current_lifeline();
        let frame_ctx = self.current_context();
        let block = &func.blocks[b];
        let span: Vec<Span> = call_site_span(ctx, site)
            .or_else(|| inst_span(block, k))
            .into_iter()
            .collect();
        let mut targets: Vec<FuncId> = ctx.callees(site).to_vec();
        if targets.is_empty() {
            if let Callee::Static(f) = callee {
                if ctx.func(*f).is_some() {
                    targets.push(*f);
                }
            }
        }
        let arg_text = r.args(ctx, callee, args);
        let receiver = self.receiver_of(site, callee);
        let recv_objs = receiver
            .map(|v| ctx.objects_of(func.id, frame_ctx, v))
            .unwrap_or_default();
        let sort = if asynchronous {
            MessageSort::AsynchCall
        } else {
            MessageSort::SynchCall
        };

        if targets.is_empty() {
            let label = format!("{}({})", callee_text(ctx, &r, callee), arg_text);
            let (to, sort, derivation) = match callee.method_symbol() {
                Some(m) => {
                    let owner = ctx
                        .symbol(m)
                        .and_then(|s| s.parent)
                        .and_then(|p| ctx.classifier_of(p));
                    let to = self.participant(&recv_objs, owner, receiver.is_some(), None, Some(m));
                    let external = ctx.symbol(m).map_or(false, |s| s.external);
                    let derivation = if external {
                        Derivation::Exact
                    } else {
                        Derivation::partial("a called declaration has no lowered body")
                    };
                    (to, sort, derivation)
                }
                None => (
                    self.gate(),
                    MessageSort::Lost,
                    Derivation::partial("a call target is not derivable (unresolved name or function value with no target)"),
                ),
            };
            let Some(mi) = self.emit(
                me,
                to,
                sort,
                label,
                None,
                func.id,
                b,
                Some(site),
                None,
                span.clone(),
                vec![Fact::CallSite(site.raw())],
                derivation,
            ) else {
                return;
            };
            if let (Some(d), MessageSort::SynchCall) = (dst, sort) {
                let number = self.trace.messages[mi].number.clone();
                self.emit(
                    to,
                    me,
                    MessageSort::Reply,
                    r.name(ctx, d),
                    Some(number),
                    func.id,
                    b,
                    Some(site),
                    None,
                    span,
                    vec![Fact::CallSite(site.raw())],
                    Derivation::Exact,
                );
            }
            return;
        }

        let groups = self.dispatch_groups(callee, &targets, &recv_objs);
        if groups.len() > 1 {
            let frag = self.open_fragment(
                FragmentKind::Alt,
                func.id,
                b,
                span.first().copied(),
                Some(Fact::CallSite(site.raw())),
            );
            let recv_name = receiver.map(|v| r.name(ctx, v));
            for (target, objs) in groups {
                let guard = self.dispatch_guard(recv_name.as_deref(), target, &objs);
                self.open_operand(frag, Some(guard));
                self.call_target(func, b, site, target, &objs, receiver, &arg_text, dst, sort, &span);
                self.close_operand();
                if self.halted {
                    return;
                }
            }
        } else if let Some((target, objs)) = groups.into_iter().next() {
            self.call_target(func, b, site, target, &objs, receiver, &arg_text, dst, sort, &span);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn call_target(
        &mut self,
        func: &'a FuncIr,
        b: usize,
        site: CallSiteId,
        target: FuncId,
        objs: &[ObjId],
        receiver: Option<ValueId>,
        arg_text: &str,
        dst: Option<ValueId>,
        sort: MessageSort,
        span: &[Span],
    ) {
        let ctx = self.ctx;
        let me = self.current_lifeline();
        let tf = ctx.func(target);
        let owner = tf.and_then(|f| ctx.owner_type(f));
        let to = self.participant(objs, owner, receiver.is_some(), tf, None);
        let name = tf
            .map(|f| ctx.func_name(f))
            .unwrap_or_else(|| format!("{}", target));
        let derivation = if ctx.edge_is_exact(site, target) {
            Derivation::Exact
        } else {
            Derivation::OverApproximate
        };
        let Some(mi) = self.emit(
            me,
            to,
            sort,
            format!("{}({})", name, arg_text),
            None,
            func.id,
            b,
            Some(site),
            Some(target),
            span.to_vec(),
            vec![
                Fact::CallSite(site.raw()),
                Fact::CallEdge {
                    site: site.raw(),
                    callee: target.raw(),
                },
            ],
            derivation,
        ) else {
            return;
        };
        let entered = self.descend(func, site, target, to, mi, sort == MessageSort::AsynchCall);
        if !entered && sort == MessageSort::SynchCall {
            if let Some(d) = dst {
                let r = self.renderer(func);
                let number = self.trace.messages[mi].number.clone();
                self.emit(
                    to,
                    me,
                    MessageSort::Reply,
                    r.name(ctx, d),
                    Some(number),
                    func.id,
                    b,
                    Some(site),
                    Some(target),
                    span.to_vec(),
                    vec![Fact::CallSite(site.raw())],
                    Derivation::Exact,
                );
            }
        }
    }

    /// Enters `target` as a new frame; false when it is not entered (no body, recursion,
    /// depth limit).
    fn descend(
        &mut self,
        caller: &'a FuncIr,
        site: CallSiteId,
        target: FuncId,
        lifeline: usize,
        message: usize,
        asynchronous: bool,
    ) -> bool {
        let ctx = self.ctx;
        let Some(tf) = ctx.func(target) else {
            return false;
        };
        if tf.blocks.is_empty() {
            return false;
        }
        if self.frames.iter().any(|f| f.func == target) {
            self.trace.messages[message].recursive = true;
            self.note(format!(
                "recursion folded at {}: the recursive call is shown, not entered",
                ctx.func_qualified_name(tf)
            ));
            return false;
        }
        if self.frames.len() >= ctx.limits.max_call_depth {
            self.partial(format!(
                "calls nested deeper than {} levels are shown but not entered",
                ctx.limits.max_call_depth
            ));
            return false;
        }
        let caller_ctx = self.current_context();
        let callee_ctx = ctx.callee_context(caller.id, caller_ctx, site, target);
        let caller_lifeline = self.current_lifeline();
        let number = self.trace.messages[message].number.clone();
        self.frames.push(Frame {
            func: target,
            lifeline,
            context: callee_ctx,
            number,
            children: 0,
            active_loops: Vec::new(),
            caller_lifeline: Some(caller_lifeline),
            asynchronous,
        });
        self.walk_function(tf);
        self.frames.pop();
        true
    }

    /// Groups the receiver objects by the target each dispatches to: through the frontend's
    /// override relation when available, else to the target declared in the nearest
    /// ancestor of the object's dynamic type.
    fn dispatch_groups(&self, callee: &Callee, targets: &[FuncId], objs: &[ObjId]) -> Vec<(FuncId, Vec<ObjId>)> {
        let ctx = self.ctx;
        if targets.len() == 1 {
            return vec![(targets[0], objs.to_vec())];
        }
        let mut groups: BTreeMap<FuncId, Vec<ObjId>> =
            targets.iter().map(|&t| (t, Vec::new())).collect();
        let Some(heap) = ctx.heap else {
            return groups.into_iter().collect();
        };
        let loc = heap.locations();
        let method = callee.method_symbol();
        for &o in objs {
            let Some(dyn_ty) = loc.object(o).dynamic_type else {
                continue;
            };
            let mut chosen: Vec<FuncId> = Vec::new();
            if let (Some(m), Some(h)) = (method, ctx.hierarchy) {
                if let Dispatch::Resolved { targets: syms, .. } = h.dispatch(dyn_ty, m) {
                    chosen = syms
                        .iter()
                        .filter_map(|&s| ctx.func_of_symbol(s))
                        .filter(|f| groups.contains_key(f))
                        .collect();
                }
            }
            if chosen.is_empty() {
                let levels = ancestor_levels(ctx, dyn_ty);
                let best = targets
                    .iter()
                    .filter_map(|&t| {
                        let owner = ctx.func(t).and_then(|f| ctx.owner_type(f))?;
                        levels.get(&owner).map(|&lvl| (lvl, t))
                    })
                    .min();
                if let Some((lvl, _)) = best {
                    chosen = targets
                        .iter()
                        .copied()
                        .filter(|&t| {
                            ctx.func(t)
                                .and_then(|f| ctx.owner_type(f))
                                .and_then(|o| levels.get(&o))
                                == Some(&lvl)
                        })
                        .collect();
                }
            }
            for f in chosen {
                if let Some(list) = groups.get_mut(&f) {
                    list.push(o);
                }
            }
        }
        groups.into_iter().collect()
    }

    fn dispatch_guard(&self, receiver: Option<&str>, target: FuncId, objs: &[ObjId]) -> String {
        let ctx = self.ctx;
        if let Some(heap) = ctx.heap {
            if !objs.is_empty() {
                let loc = heap.locations();
                let mut types: Vec<String> = objs
                    .iter()
                    .filter_map(|&o| object_type(ctx, loc, o))
                    .collect();
                types.sort();
                types.dedup();
                if !types.is_empty() {
                    let kinds: Vec<String> = types
                        .iter()
                        .map(|t| match receiver {
                            Some(r) => format!("{}.oclIsTypeOf({})", r, t),
                            None => format!("oclIsTypeOf({})", t),
                        })
                        .collect();
                    return kinds.join(" or ");
                }
            }
        }
        let owner = ctx
            .func(target)
            .and_then(|f| ctx.owner_type(f))
            .map(|o| ctx.symbol_name(o))
            .unwrap_or_else(|| format!("{}", target));
        match receiver {
            Some(r) => format!("{} dispatches to {}", r, owner),
            None => format!("target is {}", owner),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn exception_operand(
        &mut self,
        fg: &FlowGraph,
        func: &'a FuncIr,
        b: usize,
        k: usize,
        site: CallSiteId,
        callee: &Callee,
        handler: usize,
        stop: &[usize],
    ) {
        let ctx = self.ctx;
        let r = self.renderer(func);
        let merge = fg.ipdom_block(b);
        let mut hstop = stop.to_vec();
        hstop.extend(merge);
        if hstop.contains(&handler) {
            return;
        }
        let frag = self.open_fragment(
            FragmentKind::Break,
            func.id,
            b,
            inst_span(&func.blocks[b], k),
            Some(Fact::CallSite(site.raw())),
        );
        self.open_operand(frag, Some(format!("exception from {}", callee_text(ctx, &r, callee))));
        self.walk_arm(fg, func, handler, &hstop);
        self.close_operand();
    }

    // ── concurrency ──

    fn handle_value(
        &self,
        dst: Option<ValueId>,
        site: CallSiteId,
        callee: &Callee,
        args: &[Operand],
        source: HandleSource,
    ) -> Option<ValueId> {
        match source {
            HandleSource::Result => dst,
            HandleSource::Receiver => self.receiver_of(site, callee),
            HandleSource::Arg(i) => {
                let skip = self.receiver_of(site, callee).is_some() as usize;
                args.get(skip + i).and_then(|a| a.as_value())
            }
            HandleSource::None => None,
        }
    }

    fn same_handle(&mut self, func: &'a FuncIr, a: ValueId, b: ValueId) -> bool {
        let r = self.renderer(func);
        if r.resolve_copy(a) == r.resolve_copy(b) {
            return true;
        }
        let ctx_now = self.current_context();
        let pa = self.ctx.objects_of(func.id, ctx_now, a);
        let pb = self.ctx.objects_of(func.id, ctx_now, b);
        !pa.is_empty() && pa.iter().any(|o| pb.contains(o))
    }

    /// Bodies of a spawned task.
    fn task_bodies(&mut self, func: &'a FuncIr, site: CallSiteId, callee: &Callee, args: &[Operand], task: TaskSource) -> Vec<FuncId> {
        let ctx = self.ctx;
        let mut out: Vec<FuncId> = match task {
            TaskSource::CallTargets | TaskSource::Receiver => ctx.callees(site).to_vec(),
            TaskSource::Arg(i) => {
                let skip = self.receiver_of(site, callee).is_some() as usize;
                let mut found = Vec::new();
                if let Some(v) = args.get(skip + i).and_then(|a| a.as_value()) {
                    let r = self.renderer(func);
                    if let Some(Inst::MakeClosure { func: f, .. }) = r.def(r.resolve_copy(v)) {
                        found.push(*f);
                    }
                    if let Some(heap) = ctx.heap {
                        let loc = heap.locations();
                        for o in ctx.objects_of(func.id, self.current_context(), v) {
                            if let HeapBase::Closure(c) = loc.object(o).object.base {
                                found.push(loc.closure_site(c).target);
                            }
                        }
                    }
                }
                found
            }
        };
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Position of the join of `handle` in the straight-line continuation after (b, k).
    fn find_join(&mut self, fg: &FlowGraph, func: &'a FuncIr, b: usize, k: usize, handle: Option<ValueId>, stop: &[usize]) -> Option<(usize, usize)> {
        let ctx = self.ctx;
        let handle = handle?;
        let mut block = b;
        let mut from = k + 1;
        for _ in 0..=fg.len {
            let insts = &func.blocks[block].insts;
            for idx in from..insts.len() {
                if let Inst::Call {
                    dst, site, callee, args, ..
                } = &insts[idx]
                {
                    if let Some(ConcurrencyRole::Join { handle: hs }) = ctx.program.concurrency(*site) {
                        if let Some(h) = self.handle_value(*dst, *site, callee, args, hs) {
                            if self.same_handle(func, h, handle) {
                                return Some((block, idx));
                            }
                        }
                    }
                }
            }
            match &func.blocks[block].term {
                Terminator::Jump(t) => {
                    let t = t.index();
                    if t >= fg.len
                        || stop.contains(&t)
                        || fg.loop_of_header[t].is_some()
                        || fg.pred_blocks(t).len() != 1
                    {
                        return None;
                    }
                    block = t;
                    from = 0;
                }
                _ => return None,
            }
        }
        None
    }

    #[allow(clippy::too_many_arguments)]
    fn spawn(
        &mut self,
        fg: &FlowGraph,
        func: &'a FuncIr,
        b: usize,
        k: usize,
        dst: Option<ValueId>,
        site: CallSiteId,
        callee: &Callee,
        args: &[Operand],
        task: TaskSource,
        handle: HandleSource,
        stop: &[usize],
    ) -> RegionEnd {
        let ctx = self.ctx;
        let bodies = self.task_bodies(func, site, callee, args, task);
        let span = call_site_span(ctx, site).or_else(|| inst_span(&func.blocks[b], k));
        let frag = self.open_fragment(
            FragmentKind::Par,
            func.id,
            b,
            span,
            Some(Fact::Concurrency(site.raw())),
        );
        let names: Vec<String> = bodies
            .iter()
            .filter_map(|&f| ctx.func(f))
            .map(|f| ctx.func_name(f))
            .collect();
        self.open_operand(
            frag,
            Some(if names.is_empty() {
                "spawned task".to_string()
            } else {
                format!("spawned {}", names.join(", "))
            }),
        );
        if bodies.is_empty() {
            let me = self.current_lifeline();
            let gate = self.gate();
            let r = self.renderer(func);
            self.emit(
                me,
                gate,
                MessageSort::Lost,
                format!("{}({})", callee_text(ctx, &r, callee), r.args(ctx, callee, args)),
                None,
                func.id,
                b,
                Some(site),
                None,
                span.into_iter().collect(),
                vec![Fact::Concurrency(site.raw())],
                Derivation::partial("the body of a spawned task is not derivable"),
            );
        } else {
            let receiver = self.receiver_of(site, callee);
            let recv_objs = receiver
                .map(|v| ctx.objects_of(func.id, self.current_context(), v))
                .unwrap_or_default();
            let r = self.renderer(func);
            let arg_text = r.args(ctx, callee, args);
            for body in bodies {
                let objs: Vec<ObjId> = if task == TaskSource::Receiver {
                    recv_objs.clone()
                } else {
                    Vec::new()
                };
                self.call_target(
                    func,
                    b,
                    site,
                    body,
                    &objs,
                    if task == TaskSource::Receiver { receiver } else { None },
                    &arg_text,
                    None,
                    MessageSort::AsynchCall,
                    &span.into_iter().collect::<Vec<_>>(),
                );
                if self.halted {
                    break;
                }
            }
        }
        self.close_operand();
        let handle_value = self.handle_value(dst, site, callee, args, handle);
        let join = self.find_join(fg, func, b, k, handle_value, stop);
        self.open_operand(frag, None);
        let end = self.walk_region(fg, func, b, k + 1, stop, join);
        self.close_operand();
        if join.is_none() {
            self.trace.fragments[frag].derivation = Derivation::OverApproximate;
            self.note(
                "a spawned task has no join in the spawner's straight-line continuation: its par fragment extends to the end of the enclosing region"
                    .to_string(),
            );
        }
        match (end, join) {
            (RegionEnd::Joined, Some((jb, jk))) => self.walk_region(fg, func, jb, jk, stop, None),
            (other, _) => other,
        }
    }

    fn open_critical(&mut self, func: &'a FuncIr, b: usize, k: usize, lock: Option<ValueId>) {
        let ctx = self.ctx;
        let lock_objects = lock
            .map(|v| ctx.objects_of(func.id, self.current_context(), v))
            .unwrap_or_default();
        let frag = self.open_fragment(
            FragmentKind::Critical,
            func.id,
            b,
            inst_span(&func.blocks[b], k),
            Some(Fact::Inst {
                func: func.id.raw(),
                block: b as u32,
                index: k as u32,
            }),
        );
        let guard = lock.map(|v| {
            let r = self.renderer(func);
            format!("lock {}", r.name(ctx, v))
        });
        self.open_operand(frag, guard);
        self.criticals.push(CriticalRegion {
            lock,
            lock_objects,
            depth: self.open.len(),
            frame: self.frames.len(),
        });
    }

    fn close_critical(&mut self) {
        if let Some(cr) = self.criticals.pop() {
            while self.open.len() >= cr.depth && !self.open.is_empty() {
                let top_is_critical = self
                    .open
                    .last()
                    .map_or(false, |o| o.kind == FragmentKind::Critical);
                self.close_operand();
                if top_is_critical {
                    break;
                }
            }
        }
    }

    /// Handles the release of a lock: closes the innermost critical region holding it when
    /// the release is at the region's own nesting level. Returns true when the release
    /// belongs to an open critical region (whatever its level).
    fn end_critical(&mut self, func: &'a FuncIr, lock: Option<ValueId>) -> bool {
        let Some(lock) = lock else {
            return false;
        };
        let frame = self.frames.len();
        let ctx_now = self.current_context();
        let objs = self.ctx.objects_of(func.id, ctx_now, lock);
        let r = self.renderer(func);
        let resolved = r.resolve_copy(lock);
        let pos = self.criticals.iter().rposition(|cr| {
            cr.frame == frame
                && (cr.lock.map(|l| r.resolve_copy(l)) == Some(resolved)
                    || (!objs.is_empty() && objs.iter().any(|o| cr.lock_objects.contains(o))))
        });
        let Some(pos) = pos else {
            return false;
        };
        if pos + 1 == self.criticals.len() && self.open.len() == self.criticals[pos].depth {
            self.close_critical();
        }
        true
    }

    // ── object lifecycle ──

    fn create(&mut self, func: &'a FuncIr, b: usize, k: usize, dst: ValueId, site: AllocSiteId, ty: TypeId) {
        let ctx = self.ctx;
        let me = self.current_lifeline();
        let mut objs: Vec<ObjId> = ctx.objects_of(func.id, self.current_context(), dst);
        if let Some(heap) = ctx.heap {
            let loc = heap.locations();
            objs.retain(|&o| loc.object(o).object.base == HeapBase::Site(site));
        }
        let to = if objs.is_empty() {
            let ty_name = ctx.render_type(ty);
            let label = match ctx.value_name(func, dst) {
                Some(n) => format!("{} : {}", n, ty_name),
                None => format!(": {}", ty_name),
            };
            let span = ctx.alloc_site(site).map(|a| a.span);
            self.lifeline(Lifeline {
                key: format!(
                    "alloc:{}:{}",
                    ctx.func_qualified_name(func),
                    span.map(|s| s.to_string()).unwrap_or_else(|| format!("{}", site))
                ),
                label,
                kind: LifelineKind::Objects,
                symbol: ctx.named_symbol(ty),
                objects: Vec::new(),
                spans: span.into_iter().filter(|s| !s.is_none()).collect(),
                facts: vec![Fact::AllocSite(site.raw())],
                derivation: if ctx.heap.is_some() {
                    Derivation::partial("an allocation has no abstract object in this context")
                } else {
                    Derivation::Exact
                },
            })
        } else {
            self.object_lifeline(&objs)
        };
        let spans: Vec<Span> = inst_span(&func.blocks[b], k).into_iter().collect();
        self.emit(
            me,
            to,
            MessageSort::CreateMessage,
            ctx.render_type(ty),
            None,
            func.id,
            b,
            None,
            None,
            spans,
            vec![
                Fact::AllocSite(site.raw()),
                Fact::Inst {
                    func: func.id.raw(),
                    block: b as u32,
                    index: k as u32,
                },
            ],
            Derivation::Exact,
        );
    }

    fn release(&mut self, func: &'a FuncIr, b: usize, k: usize, place: &Place) {
        let ctx = self.ctx;
        let Some(base) = place.base() else {
            return;
        };
        if self.end_critical(func, Some(base)) {
            return;
        }
        let objs = ctx.objects_of(func.id, self.current_context(), base);
        if objs.is_empty() {
            return;
        }
        let Some(heap) = ctx.heap else {
            return;
        };
        let loc = heap.locations();
        let destroys = objs.iter().all(|&o| {
            let info = loc.object(o);
            info.region == Some(Region::Stack) && info.func == Some(func.id)
        });
        let me = self.current_lifeline();
        let to = self.object_lifeline(&objs);
        let r = self.renderer(func);
        self.emit(
            me,
            to,
            if destroys {
                MessageSort::DeleteMessage
            } else {
                MessageSort::SynchCall
            },
            format!("release {}", r.place(ctx, place)),
            None,
            func.id,
            b,
            None,
            None,
            inst_span(&func.blocks[b], k).into_iter().collect(),
            vec![Fact::Inst {
                func: func.id.raw(),
                block: b as u32,
                index: k as u32,
            }],
            Derivation::OverApproximate,
        );
    }

    fn emit_return(&mut self, func: &'a FuncIr, b: usize, value: Option<&Operand>, exceptional: bool) {
        let ctx = self.ctx;
        let Some(frame) = self.frames.last() else {
            return;
        };
        if frame.asynchronous {
            return;
        }
        let Some(to) = frame.caller_lifeline else {
            return;
        };
        let from = frame.lifeline;
        let number = frame.number.clone();
        let r = self.renderer(func);
        let label = match (value, exceptional) {
            (Some(v), true) => format!("raise {}", r.operand(ctx, v)),
            (Some(v), false) => r.operand(ctx, v),
            (None, _) => String::new(),
        };
        let mut facts = vec![Fact::Terminator {
            func: func.id.raw(),
            block: b as u32,
        }];
        if exceptional {
            facts.push(Fact::Inst {
                func: func.id.raw(),
                block: b as u32,
                index: u32::MAX,
            });
        }
        self.emit(
            from,
            to,
            MessageSort::Reply,
            label,
            Some(number),
            func.id,
            b,
            None,
            None,
            term_span(&func.blocks[b]).into_iter().collect(),
            facts,
            Derivation::Exact,
        );
    }

    // ── result ──

    /// Drops fragments without messages, turns alternatives with a single non-empty
    /// operand into options, and re-links parents to the nearest kept ancestor.
    fn finish(mut self) -> InteractionTrace {
        let frags = std::mem::take(&mut self.trace.fragments);
        let mut keep: Vec<Option<usize>> = vec![None; frags.len()];
        let mut out: Vec<TraceFragment> = Vec::new();
        for (i, mut f) in frags.clone().into_iter().enumerate() {
            match f.kind {
                FragmentKind::Alt => {
                    f.operands.retain(|o| o.end > o.start);
                    if f.operands.len() == 1 {
                        f.kind = FragmentKind::Opt;
                    }
                    if f.operands.is_empty() {
                        continue;
                    }
                }
                FragmentKind::Par => {
                    if f.operands.iter().all(|o| o.end == o.start) {
                        continue;
                    }
                }
                _ => {
                    if f.operands.iter().all(|o| o.end == o.start) {
                        continue;
                    }
                }
            }
            keep[i] = Some(out.len());
            out.push(f);
        }
        for f in out.iter_mut() {
            let mut p = f.parent;
            let mut guard = 0;
            while let Some(pi) = p {
                if let Some(new) = keep[pi] {
                    f.parent = Some(new);
                    break;
                }
                p = frags[pi].parent;
                guard += 1;
                if guard > frags.len() {
                    p = None;
                }
            }
            if p.is_none() {
                f.parent = None;
            }
        }
        self.trace.fragments = out;
        self.trace
    }
}

/// Distance of every ancestor of `sym` (itself at 0) through declared supertypes.
fn ancestor_levels(ctx: &DeriveContext, sym: u32) -> HashMap<u32, usize> {
    let mut levels = HashMap::new();
    let mut queue = VecDeque::new();
    levels.insert(sym, 0usize);
    queue.push_back(sym);
    while let Some(t) = queue.pop_front() {
        let lvl = levels[&t];
        for sup in ctx.program.supertypes(t) {
            if !levels.contains_key(&sup.target) {
                levels.insert(sup.target, lvl + 1);
                queue.push_back(sup.target);
            }
        }
    }
    levels
}

// ── Entry points ─────────────────────────────────────────────────────────────

/// Stable identity of an entry point's interaction.
pub fn trace_key(ctx: &DeriveContext, func: FuncId) -> String {
    match ctx.func(func) {
        Some(f) => format!("{}#{}", ctx.func_qualified_name(f), func.raw()),
        None => format!("{}", func),
    }
}

/// The interaction of one entry point.
pub fn trace_entry(ctx: &DeriveContext, entry: &EntryPointFact) -> InteractionTrace {
    let key = trace_key(ctx, entry.func);
    let title = match (&entry.name, ctx.func(entry.func)) {
        (Some(n), Some(f)) => format!("{} ({})", n, ctx.func_qualified_name(f)),
        (None, Some(f)) => ctx.func_qualified_name(f),
        (Some(n), None) => n.clone(),
        (None, None) => key.clone(),
    };
    let mut walker = Walker::new(ctx, entry, key, title);
    walker.run();
    if !entry.kind.is_declared() {
        walker.note(format!(
            "the interaction starts at a {} entry: no frontend fact classifies how it is invoked",
            entry.kind.mnemonic()
        ));
    }
    walker.finish()
}

/// Interactions of every entry point (frontend entries, solver entries, callbacks).
pub fn trace_entries(ctx: &DeriveContext) -> Traces {
    let mut entries = ctx.entry_points();
    let omitted = cap(&mut entries, ctx.limits.max_diagrams);
    Traces {
        traces: entries.iter().map(|e| trace_entry(ctx, e)).collect(),
        omitted,
    }
}

// ── Model ────────────────────────────────────────────────────────────────────

pub fn lifeline_element_id(trace: &InteractionTrace, lifeline: usize) -> ElementId {
    IdBuilder::new("lifeline")
        .text(&trace.key)
        .text(&trace.lifelines[lifeline].key)
        .finish()
}

pub fn message_element_id(trace: &InteractionTrace, msg: &TraceMessage) -> ElementId {
    IdBuilder::new("msg")
        .text(&trace.key)
        .text(&msg.number)
        .text(msg.sort.mnemonic())
        .num(msg.order as u64)
        .finish()
}

pub(crate) fn fragment_element_id(trace: &InteractionTrace, index: usize) -> ElementId {
    let f = &trace.fragments[index];
    IdBuilder::new("frag")
        .text(&trace.key)
        .text(f.kind.mnemonic())
        .num(f.func.raw() as u64)
        .num(f.block as u64)
        .num(f.operands.first().map_or(0, |o| o.start) as u64)
        .num(index as u64)
        .finish()
}

pub(crate) fn lifeline_node(trace: &InteractionTrace, i: usize) -> DiagramNode {
    let l = &trace.lifelines[i];
    let kind = match l.kind {
        LifelineKind::Actor => NodeKind::Actor,
        LifelineKind::Gate => NodeKind::Gate,
        _ => NodeKind::Lifeline,
    };
    let mut node = DiagramNode::new(lifeline_element_id(trace, i), kind, l.label.clone())
        .with_spans(l.spans.iter().copied())
        .with_derivation(l.derivation.clone());
    for f in &l.facts {
        node = node.with_fact(*f);
    }
    if let Some(s) = l.symbol {
        node = node.with_symbol(s);
    }
    match l.kind {
        LifelineKind::Classifier => node = node.with_property("represents", "type"),
        LifelineKind::Namespace => node = node.with_property("represents", "namespace"),
        LifelineKind::File => node = node.with_property("represents", "file"),
        LifelineKind::Objects if l.objects.len() > 1 => {
            node = node.with_property("abstract objects", l.objects.len().to_string())
        }
        _ => {}
    }
    node
}

/// The sequence diagram of one interaction.
pub fn model_of(ctx: &DeriveContext, trace: &InteractionTrace) -> DiagramModel {
    let mut model = DiagramModel::new(DiagramKind::Sequence, format!("sd {}", trace.title))
        .with_scope(trace.title.clone());
    for i in 0..trace.lifelines.len() {
        model.add_node(lifeline_node(trace, i));
    }
    let msg_ids: Vec<ElementId> = trace
        .messages
        .iter()
        .map(|m| message_element_id(trace, m))
        .collect();
    for (m, id) in trace.messages.iter().zip(&msg_ids) {
        let mut edge = DiagramEdge::new(
            id.clone(),
            EdgeKind::Message,
            lifeline_element_id(trace, m.from),
            lifeline_element_id(trace, m.to),
        )
        .with_label(m.label.clone())
        .with_order(m.order)
        .with_sequence_number(m.number.clone())
        .with_message_sort(m.sort)
        .with_spans(m.spans.iter().copied())
        .with_facts(m.facts.iter().copied())
        .with_derivation(m.derivation.clone());
        if m.recursive {
            edge = edge.with_property("recursive", "true");
        }
        if let Some(c) = m.callee.and_then(|f| ctx.func(f)) {
            edge = edge.with_property("callee", ctx.func_qualified_name(c));
        }
        model.add_edge(edge);
    }
    let frag_ids: Vec<ElementId> = (0..trace.fragments.len())
        .map(|i| fragment_element_id(trace, i))
        .collect();
    for (i, f) in trace.fragments.iter().enumerate() {
        let mut covered: Vec<usize> = vec![f.owner];
        let operands: Vec<FragmentOperand> = f
            .operands
            .iter()
            .map(|op| {
                let members: Vec<ElementId> = (op.start..op.end)
                    .filter_map(|o| msg_ids.get(o as usize).cloned())
                    .collect();
                for o in op.start..op.end {
                    if let Some(m) = trace.messages.get(o as usize) {
                        covered.push(m.from);
                        covered.push(m.to);
                    }
                }
                FragmentOperand {
                    guard: op.guard.clone(),
                    members,
                }
            })
            .collect();
        covered.sort_unstable();
        covered.dedup();
        model.add_fragment(Fragment {
            id: frag_ids[i].clone(),
            kind: f.kind,
            parent: f.parent.map(|p| frag_ids[p].clone()),
            covered: covered
                .into_iter()
                .filter(|&l| l < trace.lifelines.len())
                .map(|l| lifeline_element_id(trace, l))
                .collect(),
            operands,
            spans: f.spans.clone(),
            facts: f.facts.clone(),
            derivation: f.derivation.clone(),
        });
    }
    for n in &trace.notes {
        model.note(n.clone());
    }
    model.weaken(&trace.derivation);
    model.finalize(ctx.limits.max_spans);
    model
}

/// One sequence diagram per entry point.
pub fn derive_from_traces(ctx: &DeriveContext, traces: &Traces) -> Vec<DiagramModel> {
    if traces.traces.is_empty() {
        return vec![DiagramModel::not_derivable(
            DiagramKind::Sequence,
            DiagramKind::Sequence.title(),
            "no entry point has a lowered body: interactions start at entry points",
        )];
    }
    let mut out: Vec<DiagramModel> = traces.traces.iter().map(|t| model_of(ctx, t)).collect();
    if traces.omitted > 0 {
        out.push(DiagramModel::omission(
            DiagramKind::Sequence,
            traces.omitted,
            ctx.limits.max_diagrams,
        ));
    }
    out
}

pub fn derive(ctx: &DeriveContext) -> Vec<DiagramModel> {
    derive_from_traces(ctx, &trace_entries(ctx))
}
