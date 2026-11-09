//! Interprocedural distributive environment problems (Sagiv, Reps & Horwitz 1996).
//!
//! IDE extends IFDS: every exploded-supergraph edge `⟨n, d⟩ → ⟨m, d'⟩` carries an
//! *environment transformer* — an [`EdgeFunction`] `V → V` on a value lattice — and the
//! solution maps every reachable `(node, fact)` to a value.
//!
//! **Phase I** tabulates *jump functions* `JF(d₁, n, d₂)`: the join, over realizable paths
//! from the start of `n`'s function entered with `d₁`, of the composition of the edge
//! functions along the path. End summaries are jump functions to exit nodes; at a call they
//! compose `call ∘ summary ∘ return` into a summary function so callees are not re-analysed.
//! Termination requires the edge-function lattice to have finite height (true for
//! [`LinearConstFn`]); a step budget guards the rest.
//!
//! **Phase II** first propagates values to function starts along call edges
//! (`val(s_q, d₃) ⊔= f_call(JF(d₁, c, d₂)(val(s_p, d₁)))`) to a fixpoint, then evaluates
//! every node: `val(n, d₂) = ⊔_{d₁} JF(d₁, n, d₂)(val(s_p, d₁))`.
//!
//! The supergraph, nodes and return sites are those of [`crate::analysis::ifds`]. As there,
//! the solver propagates `Λ` with the identity along every edge itself.

use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::hash::Hash;

use crate::analysis::ifds::{FactTable, Node, ReturnSite, Supergraph};
use crate::analysis::lattice::{Flat, Lattice};
use crate::analysis::solver::CfgEdge;

// ── Edge functions ───────────────────────────────────────────────────────────

/// A distributive environment transformer `V → V`, closed under composition and join.
pub trait EdgeFunction<V: Lattice>: Clone + Eq + Hash + fmt::Debug {
    /// `λx. x`.
    fn identity() -> Self;
    /// `λx. ⊥`: the function of no path.
    fn all_bottom() -> Self;
    fn is_all_bottom(&self) -> bool;
    /// Applies the function.
    fn compute(&self, v: &V) -> V;
    /// `next ∘ self`: first `self`, then `next`.
    fn then(&self, next: &Self) -> Self;
    /// Pointwise upper bound.
    fn join(&self, other: &Self) -> Self;
}

/// Linear-constant transformers over `Flat<i128>` (Sagiv et al.'s linear constant
/// propagation): `λx. a·x + b`, plus `λx. ⊥` and `λx. ⊤`. Constants are `a = 0`; the
/// identity is `a = 1, b = 0`. Results that overflow 128 bits are `⊤`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LinearConstFn {
    AllBottom,
    Linear { a: i128, b: i128 },
    AllTop,
}

impl LinearConstFn {
    pub fn constant(c: i128) -> Self {
        LinearConstFn::Linear { a: 0, b: c }
    }
}

impl EdgeFunction<Flat<i128>> for LinearConstFn {
    fn identity() -> Self {
        LinearConstFn::Linear { a: 1, b: 0 }
    }

    fn all_bottom() -> Self {
        LinearConstFn::AllBottom
    }

    fn is_all_bottom(&self) -> bool {
        matches!(self, LinearConstFn::AllBottom)
    }

    fn compute(&self, v: &Flat<i128>) -> Flat<i128> {
        match *self {
            LinearConstFn::AllBottom => Flat::Bottom,
            LinearConstFn::AllTop => Flat::Top,
            LinearConstFn::Linear { a: 0, b } => Flat::Value(b),
            LinearConstFn::Linear { a, b } => match v {
                Flat::Bottom => Flat::Bottom,
                Flat::Top => Flat::Top,
                Flat::Value(x) => a
                    .checked_mul(*x)
                    .and_then(|ax| ax.checked_add(b))
                    .map_or(Flat::Top, Flat::Value),
            },
        }
    }

    fn then(&self, next: &Self) -> Self {
        match (*self, *next) {
            (_, LinearConstFn::AllBottom) => LinearConstFn::AllBottom,
            (_, LinearConstFn::AllTop) => LinearConstFn::AllTop,
            (_, LinearConstFn::Linear { a: 0, b }) => LinearConstFn::constant(b),
            (LinearConstFn::AllBottom, _) => LinearConstFn::AllBottom,
            (LinearConstFn::AllTop, _) => LinearConstFn::AllTop,
            (LinearConstFn::Linear { a: a1, b: b1 }, LinearConstFn::Linear { a: a2, b: b2 }) => {
                match (a2.checked_mul(a1), a2.checked_mul(b1).and_then(|x| x.checked_add(b2))) {
                    (Some(a), Some(b)) => LinearConstFn::Linear { a, b },
                    _ => LinearConstFn::AllTop,
                }
            }
        }
    }

    fn join(&self, other: &Self) -> Self {
        match (*self, *other) {
            (LinearConstFn::AllBottom, f) | (f, LinearConstFn::AllBottom) => f,
            (f, g) if f == g => f,
            _ => LinearConstFn::AllTop,
        }
    }
}

// ── Problem interface ────────────────────────────────────────────────────────

/// An IDE problem.
pub trait IdeProblem {
    type Fact: Clone + Eq + Hash + fmt::Debug;
    type Value: Lattice;
    type EdgeFn: EdgeFunction<Self::Value>;

    /// The zero fact `Λ`.
    fn zero(&self) -> Self::Fact;

    /// Initial `(start node, fact, value)` triples.
    fn seeds(&self, sg: &Supergraph<'_>) -> Vec<(Node, Self::Fact, Self::Value)>;

    fn normal_flow(
        &self,
        sg: &Supergraph<'_>,
        from: Node,
        to: Node,
        edge: Option<&CfgEdge>,
        fact: &Self::Fact,
    ) -> Vec<(Self::Fact, Self::EdgeFn)>;

    fn call_flow(
        &self,
        sg: &Supergraph<'_>,
        call: Node,
        callee: u32,
        fact: &Self::Fact,
    ) -> Vec<(Self::Fact, Self::EdgeFn)>;

    fn return_flow(
        &self,
        sg: &Supergraph<'_>,
        call: Node,
        callee: u32,
        exit: Node,
        ret: ReturnSite,
        fact: &Self::Fact,
    ) -> Vec<(Self::Fact, Self::EdgeFn)>;

    fn call_to_return_flow(
        &self,
        sg: &Supergraph<'_>,
        call: Node,
        ret: ReturnSite,
        fact: &Self::Fact,
    ) -> Vec<(Self::Fact, Self::EdgeFn)>;

    /// Flow across a call the call graph does not resolve; defaults to the call-to-return
    /// flow.
    fn unresolved_call_flow(
        &self,
        sg: &Supergraph<'_>,
        call: Node,
        ret: ReturnSite,
        fact: &Self::Fact,
    ) -> Vec<(Self::Fact, Self::EdgeFn)> {
        self.call_to_return_flow(sg, call, ret, fact)
    }
}

/// Solver configuration.
#[derive(Clone, Copy, Debug)]
pub struct IdeConfig {
    /// Budget of Phase I worklist steps.
    pub max_steps: usize,
    /// Budget of Phase II start-value propagation steps.
    pub max_value_steps: usize,
}

impl Default for IdeConfig {
    fn default() -> Self {
        Self {
            max_steps: 50_000_000,
            max_value_steps: 50_000_000,
        }
    }
}

/// Results of an IDE solve.
#[derive(Clone, Debug)]
pub struct IdeResults<F, V> {
    /// Facts by id (`0` is `Λ`).
    pub facts: Vec<F>,
    /// Value of every reachable `(node, fact id)`; `Λ` excluded.
    pub values: HashMap<(Node, u32), V>,
    /// False when a budget was exhausted (values are then not a fixpoint).
    pub complete: bool,
}

impl<F: PartialEq, V> IdeResults<F, V> {
    /// The value of `fact` at `n`.
    pub fn value_at(&self, n: Node, fact: &F) -> Option<&V> {
        let id = self.facts.iter().position(|f| f == fact)? as u32;
        self.values.get(&(n, id))
    }
}

impl<F, V> IdeResults<F, V> {
    /// Every `(fact, value)` at `n`.
    pub fn at(&self, n: Node) -> Vec<(&F, &V)> {
        let mut out: Vec<(u32, &V)> = self
            .values
            .iter()
            .filter(|((m, _), _)| *m == n)
            .map(|((_, d), v)| (*d, v))
            .collect();
        out.sort_unstable_by_key(|(d, _)| *d);
        out.into_iter()
            .map(|(d, v)| (&self.facts[d as usize], v))
            .collect()
    }
}

const ZERO: u32 = 0;

/// An incoming call edge: the caller path edge `(d₁, call, d₂)` and the call-edge function.
#[derive(Clone, Debug)]
struct Incoming<E> {
    call: Node,
    d2: u32,
    d1: u32,
    f_call: E,
}

/// The IDE solver.
pub struct IdeSolver<'s, 'm, P: IdeProblem> {
    sg: &'s Supergraph<'m>,
    problem: &'s P,
    config: IdeConfig,
    facts: FactTable<P::Fact>,
    jump: HashMap<(u32, Node, u32), P::EdgeFn>,
    worklist: VecDeque<(u32, Node, u32)>,
    queued: HashSet<(u32, Node, u32)>,
    /// `(func, d₁) → {(exit, d₂) → JF}`.
    end_summary: HashMap<(u32, u32), HashMap<(Node, u32), P::EdgeFn>>,
    /// `(callee, d₃) → incoming call edges`.
    incoming: HashMap<(u32, u32), Vec<Incoming<P::EdgeFn>>>,
    seeds: Vec<(Node, u32, P::Value)>,
    complete: bool,
}

impl<'s, 'm, P: IdeProblem> IdeSolver<'s, 'm, P> {
    pub fn new(sg: &'s Supergraph<'m>, problem: &'s P, config: IdeConfig) -> Self {
        Self {
            sg,
            problem,
            config,
            facts: FactTable::new(problem.zero()),
            jump: HashMap::new(),
            worklist: VecDeque::new(),
            queued: HashSet::new(),
            end_summary: HashMap::new(),
            incoming: HashMap::new(),
            seeds: Vec::new(),
            complete: true,
        }
    }

    fn jf(&self, d1: u32, n: Node, d2: u32) -> P::EdgeFn {
        self.jump
            .get(&(d1, n, d2))
            .cloned()
            .unwrap_or_else(P::EdgeFn::all_bottom)
    }

    fn propagate(&mut self, d1: u32, n: Node, d2: u32, f: P::EdgeFn) {
        if f.is_all_bottom() {
            return;
        }
        let old = self.jf(d1, n, d2);
        let new = old.join(&f);
        if new == old && self.jump.contains_key(&(d1, n, d2)) {
            return;
        }
        self.jump.insert((d1, n, d2), new);
        if self.queued.insert((d1, n, d2)) {
            self.worklist.push_back((d1, n, d2));
        }
    }

    fn outputs(&mut self, input: u32, produced: Vec<(P::Fact, P::EdgeFn)>) -> Vec<(u32, P::EdgeFn)> {
        let mut out: Vec<(u32, P::EdgeFn)> = produced
            .into_iter()
            .map(|(f, e)| (self.facts.intern(f), e))
            .collect();
        if input == ZERO && !out.iter().any(|(d, _)| *d == ZERO) {
            out.push((ZERO, P::EdgeFn::identity()));
        }
        out
    }

    /// Runs both phases.
    pub fn solve(mut self) -> IdeResults<P::Fact, P::Value> {
        let sg = self.sg;
        for (node, fact, value) in self.problem.seeds(sg) {
            let d = self.facts.intern(fact);
            self.seeds.push((node, d, value));
            self.propagate(d, node, d, P::EdgeFn::identity());
            if d != ZERO {
                self.seeds.push((node, ZERO, P::Value::bottom()));
                self.propagate(ZERO, node, ZERO, P::EdgeFn::identity());
            }
        }

        // ── Phase I: jump functions ──
        let mut steps = 0usize;
        while let Some(item) = self.worklist.pop_front() {
            self.queued.remove(&item);
            steps += 1;
            if steps > self.config.max_steps {
                self.complete = false;
                break;
            }
            let (d1, n, d2) = item;
            if sg.call_site_at(n).is_some() {
                self.process_call(d1, n, d2);
            } else if sg.exit_kind(n).is_some() {
                self.process_exit(d1, n, d2);
            } else {
                self.process_normal(d1, n, d2);
            }
        }

        // ── Phase II ──
        let start_values = self.start_values();
        self.node_values(start_values)
    }

    fn process_normal(&mut self, d1: u32, n: Node, d2: u32) {
        let sg = self.sg;
        let f = self.jf(d1, n, d2);
        let fact = self.facts.get(d2).clone();
        for (m, edge) in sg.successors(n) {
            let edge_ref = edge.map(|e| sg.edge(n.func, e));
            let produced = self.problem.normal_flow(sg, n, m, edge_ref, &fact);
            for (d3, fe) in self.outputs(d2, produced) {
                self.propagate(d1, m, d3, f.then(&fe));
            }
        }
    }

    fn process_call(&mut self, d1: u32, n: Node, d2: u32) {
        let sg = self.sg;
        let Some(site) = sg.call_site_at(n) else {
            return;
        };
        let f = self.jf(d1, n, d2);
        let fact = self.facts.get(d2).clone();
        let returns = sg.return_sites(n);
        let callees: Vec<u32> = sg
            .callees_of(site)
            .iter()
            .copied()
            .filter(|&q| sg.start(q).is_some())
            .collect();

        if callees.is_empty() {
            for r in &returns {
                let produced = self.problem.unresolved_call_flow(sg, n, *r, &fact);
                for (d3, fe) in self.outputs(d2, produced) {
                    self.propagate(d1, r.node, d3, f.then(&fe));
                }
            }
            return;
        }

        for &q in &callees {
            let Some(start) = sg.start(q) else {
                continue;
            };
            let produced = self.problem.call_flow(sg, n, q, &fact);
            for (d3, f_call) in self.outputs(d2, produced) {
                self.propagate(d3, start, d3, P::EdgeFn::identity());
                let list = self.incoming.entry((q, d3)).or_default();
                match list.iter_mut().find(|i| i.call == n && i.d2 == d2 && i.d1 == d1) {
                    Some(existing) => existing.f_call = existing.f_call.join(&f_call),
                    None => list.push(Incoming {
                        call: n,
                        d2,
                        d1,
                        f_call: f_call.clone(),
                    }),
                }
                let ends: Vec<((Node, u32), P::EdgeFn)> = self
                    .end_summary
                    .get(&(q, d3))
                    .map(|m| m.iter().map(|(k, v)| (*k, v.clone())).collect())
                    .unwrap_or_default();
                for ((exit, d4), f_summary) in ends {
                    self.apply_return(n, d1, &f, &f_call, q, exit, d4, &f_summary);
                }
            }
        }
        for r in &returns {
            let produced = self.problem.call_to_return_flow(sg, n, *r, &fact);
            for (d3, fe) in self.outputs(d2, produced) {
                self.propagate(d1, r.node, d3, f.then(&fe));
            }
        }
    }

    /// Composes `f_caller ; f_call ; f_summary ; f_return` into the return sites of `call`.
    #[allow(clippy::too_many_arguments)]
    fn apply_return(
        &mut self,
        call: Node,
        d1: u32,
        f_caller: &P::EdgeFn,
        f_call: &P::EdgeFn,
        q: u32,
        exit: Node,
        d4: u32,
        f_summary: &P::EdgeFn,
    ) {
        let sg = self.sg;
        let Some(kind) = sg.exit_kind(exit) else {
            return;
        };
        let fact = self.facts.get(d4).clone();
        let through = f_caller.then(f_call).then(f_summary);
        for r in sg.return_sites(call) {
            if r.kind != kind {
                continue;
            }
            let produced = self.problem.return_flow(sg, call, q, exit, r, &fact);
            for (d5, f_ret) in self.outputs(d4, produced) {
                self.propagate(d1, r.node, d5, through.then(&f_ret));
            }
        }
    }

    fn process_exit(&mut self, d1: u32, n: Node, d2: u32) {
        let p = n.func;
        let f = self.jf(d1, n, d2);
        let summary = self.end_summary.entry((p, d1)).or_default();
        let old = summary.get(&(n, d2)).cloned();
        let new = match &old {
            Some(o) => o.join(&f),
            None => f.clone(),
        };
        if old.as_ref() == Some(&new) {
            return;
        }
        summary.insert((n, d2), new.clone());
        let callers: Vec<Incoming<P::EdgeFn>> = self.incoming.get(&(p, d1)).cloned().unwrap_or_default();
        for inc in callers {
            let f_caller = self.jf(inc.d1, inc.call, inc.d2);
            self.apply_return(inc.call, inc.d1, &f_caller, &inc.f_call, p, n, d2, &new);
        }
    }

    /// Phase II(i): values at function starts.
    fn start_values(&mut self) -> HashMap<(u32, u32), P::Value> {
        let sg = self.sg;
        let mut values: HashMap<(u32, u32), P::Value> = HashMap::new();
        let mut work: VecDeque<(u32, u32)> = VecDeque::new();
        for (node, d, v) in &self.seeds {
            let slot = values.entry((node.func, *d)).or_insert_with(P::Value::bottom);
            slot.join(v);
            work.push_back((node.func, *d));
        }

        // (caller func, d₁) → [(callee, d₃, call, d₂, f_call)]
        let mut calls_from: HashMap<(u32, u32), Vec<(u32, u32, Node, u32, P::EdgeFn)>> = HashMap::new();
        for (&(q, d3), list) in &self.incoming {
            for inc in list {
                calls_from.entry((inc.call.func, inc.d1)).or_default().push((
                    q,
                    d3,
                    inc.call,
                    inc.d2,
                    inc.f_call.clone(),
                ));
            }
        }

        let mut steps = 0usize;
        while let Some((p, d1)) = work.pop_front() {
            steps += 1;
            if steps > self.config.max_value_steps {
                self.complete = false;
                break;
            }
            let Some(v) = values.get(&(p, d1)).cloned() else {
                continue;
            };
            let Some(edges) = calls_from.get(&(p, d1)) else {
                continue;
            };
            for (q, d3, call, d2, f_call) in edges {
                if sg.start(*q).is_none() {
                    continue;
                }
                let at_call = self.jf(d1, *call, *d2).compute(&v);
                let entering = f_call.compute(&at_call);
                // A start reached for the first time is evaluated even with a `⊥` value: its
                // jump functions from `Λ` may still produce constants.
                let first = !values.contains_key(&(*q, *d3));
                let slot = values.entry((*q, *d3)).or_insert_with(P::Value::bottom);
                if slot.join(&entering) || first {
                    work.push_back((*q, *d3));
                }
            }
        }
        values
    }

    /// Phase II(ii): values at every node.
    fn node_values(self, start_values: HashMap<(u32, u32), P::Value>) -> IdeResults<P::Fact, P::Value> {
        let mut values: HashMap<(Node, u32), P::Value> = HashMap::new();
        for (&(d1, n, d2), f) in &self.jump {
            if d2 == ZERO {
                continue;
            }
            let Some(v) = start_values.get(&(n.func, d1)) else {
                continue;
            };
            let out = f.compute(v);
            let slot = values.entry((n, d2)).or_insert_with(P::Value::bottom);
            slot.join(&out);
        }
        IdeResults {
            facts: self.facts.into_vec(),
            values,
            complete: self.complete,
        }
    }
}
