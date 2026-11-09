//! Explicit exploration limits and the record of every limit that was hit.
//!
//! Bounded symbolic execution is complete only within its budget. Every bound the engine
//! enforces is named here, and whenever one stops or degrades exploration a
//! [`BudgetExhausted`] marker is recorded with the function and source span where it
//! happened, so a consumer can tell "no further path exists" apart from "no further path
//! was explored".

use std::time::{Duration, Instant};

use crate::ir::types::{FuncId, Span};
use crate::symex::solver::SolverLimits;

/// Order in which pending states are explored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Strategy {
    /// Last forked state first: finishes paths early, small frontier.
    DepthFirst,
    /// First forked state first: explores all paths to equal depth.
    BreadthFirst,
}

/// Exploration limits.
#[derive(Clone, Debug)]
pub struct Budget {
    pub strategy: Strategy,
    /// Maximum finished paths (returned, thrown, faulted, subsumed or cut).
    pub max_paths: usize,
    /// Maximum pending states in the worklist (breadth bound).
    pub max_frontier: usize,
    /// Maximum call-stack depth; deeper calls are summarized.
    pub max_call_depth: usize,
    /// Maximum instructions and terminators executed along one path.
    pub max_path_steps: usize,
    /// Maximum instructions and terminators executed in the whole exploration.
    pub max_total_steps: u64,
    /// Loop iterations unrolled per loop head and activation before the loop is summarized.
    pub loop_unroll: u32,
    /// Maximum objects in one state's heap.
    pub max_heap_objects: usize,
    /// Wall-clock limit of the exploration.
    pub time_limit: Option<Duration>,
    /// Limits of each feasibility query.
    pub solver: SolverLimits,
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            strategy: Strategy::DepthFirst,
            max_paths: 4096,
            max_frontier: 8192,
            max_call_depth: 24,
            max_path_steps: 100_000,
            max_total_steps: 5_000_000,
            loop_unroll: 2,
            max_heap_objects: 65_536,
            time_limit: Some(Duration::from_secs(10)),
            solver: SolverLimits::default(),
        }
    }
}

/// The limit that was reached.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Limit {
    Paths,
    Frontier,
    CallDepth,
    PathSteps,
    TotalSteps,
    LoopUnrolling,
    HeapObjects,
    Time,
}

impl Limit {
    pub fn as_str(&self) -> &'static str {
        match self {
            Limit::Paths => "paths",
            Limit::Frontier => "frontier",
            Limit::CallDepth => "call_depth",
            Limit::PathSteps => "path_steps",
            Limit::TotalSteps => "total_steps",
            Limit::LoopUnrolling => "loop_unrolling",
            Limit::HeapObjects => "heap_objects",
            Limit::Time => "time",
        }
    }

    /// True when hitting the limit ends the whole exploration rather than one path.
    pub fn is_global(&self) -> bool {
        matches!(self, Limit::Paths | Limit::TotalSteps | Limit::Time)
    }
}

/// A limit was reached; results beyond this point are missing or summarized.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BudgetExhausted {
    pub limit: Limit,
    /// Function being executed, when the limit was hit inside one.
    pub func: Option<FuncId>,
    /// Source span of the instruction or loop head, when known.
    pub span: Option<Span>,
    /// How often the limit was hit at this place.
    pub hits: u64,
}

/// Tracks consumption against a [`Budget`].
#[derive(Debug)]
pub struct BudgetMeter {
    started: Instant,
    deadline: Option<Instant>,
    steps: u64,
    finished_paths: usize,
    stopped: Option<Limit>,
    records: Vec<BudgetExhausted>,
}

/// Steps between two clock reads.
const CLOCK_STRIDE: u64 = 256;

impl BudgetMeter {
    pub fn new(budget: &Budget) -> Self {
        let started = Instant::now();
        Self {
            started,
            deadline: budget.time_limit.map(|d| started + d),
            steps: 0,
            finished_paths: 0,
            stopped: None,
            records: Vec::new(),
        }
    }

    /// Charge one executed instruction or terminator. Returns the global limit that ends
    /// the exploration, if one is reached now.
    pub fn charge_step(&mut self, budget: &Budget, func: FuncId, span: Span) -> Option<Limit> {
        if let Some(l) = self.stopped {
            return Some(l);
        }
        self.steps += 1;
        if self.steps > budget.max_total_steps {
            self.stop(Limit::TotalSteps, Some(func), Some(span));
            return self.stopped;
        }
        if self.steps % CLOCK_STRIDE == 0 {
            if let Some(deadline) = self.deadline {
                if Instant::now() >= deadline {
                    self.stop(Limit::Time, Some(func), Some(span));
                    return self.stopped;
                }
            }
        }
        None
    }

    /// Count one finished path. Returns `Limit::Paths` once the path budget is used up.
    pub fn finish_path(&mut self, budget: &Budget) -> Option<Limit> {
        self.finished_paths += 1;
        if self.finished_paths >= budget.max_paths && self.stopped.is_none() {
            self.stop(Limit::Paths, None, None);
        }
        self.stopped
    }

    /// Record that a per-path or per-site limit degraded exploration.
    pub fn record(&mut self, limit: Limit, func: Option<FuncId>, span: Option<Span>) {
        if let Some(r) = self
            .records
            .iter_mut()
            .find(|r| r.limit == limit && r.func == func && r.span == span)
        {
            r.hits += 1;
            return;
        }
        self.records.push(BudgetExhausted {
            limit,
            func,
            span,
            hits: 1,
        });
    }

    /// Stop the whole exploration because of `limit`.
    pub fn stop(&mut self, limit: Limit, func: Option<FuncId>, span: Option<Span>) {
        if self.stopped.is_none() {
            self.stopped = Some(limit);
        }
        self.record(limit, func, span);
    }

    pub fn stopped(&self) -> Option<Limit> {
        self.stopped
    }

    pub fn steps(&self) -> u64 {
        self.steps
    }

    pub fn finished_paths(&self) -> usize {
        self.finished_paths
    }

    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    /// The recorded markers, ordered by limit, function and span.
    pub fn into_records(mut self) -> Vec<BudgetExhausted> {
        self.records
            .sort_by_key(|r| (r.limit, r.func.map(|f| f.0), r.span));
        self.records
    }
}
