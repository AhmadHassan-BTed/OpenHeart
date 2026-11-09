//! Stack-frame views along a trace.
//!
//! A [`Trace`] is a sequence of program points with explicit call, return and unwind
//! steps; symbolic execution produces them for feasible paths and the safety checker for
//! its witnesses. Replaying the trace keeps a frame stack: each frame shows its function,
//! the call site that created it, its locals (SSA values with what they reference) and the
//! stack-region objects it owns, plus every reference from the frame into the heap.
//!
//! Values come from the abstract heap at each point, which joins every path reaching the
//! point; the trace selects which frames exist, not which concrete values they hold.
//! Snapshots therefore carry `OverApproximate` unless the point has a single abstract
//! value per local.

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

use crate::ir::{Inst, ModuleIr};
use crate::memview::heap_shape::{storage, HeapResult, HeapState, LifeState, MemRegion, LIFE_LIVE};
use crate::memview::safety::Finding;
use crate::memview::{AbsObj, Derivation, MemoryFacts, ProgramPoint, SrcSpan, NO_ID};

/// One step of an execution trace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "step")]
pub enum TraceStep {
    /// Execution reaches the instruction or terminator at `point` in the current frame.
    At { point: ProgramPoint },
    /// The current frame calls `callee` through `site`; a new frame starts.
    Call { site: u32, callee: u32 },
    /// The current frame returns to its caller.
    Return,
    /// The current frame is left by an exception.
    Unwind,
}

/// An execution trace in the shared IR.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Trace {
    pub steps: Vec<TraceStep>,
    pub derivation: Derivation,
}

impl Trace {
    /// Reconstructs call and return steps from a bare sequence of points: entering a
    /// callee of the previous call instruction is a call; reaching a function already on
    /// the stack is a return to it.
    pub fn from_points(result: &HeapResult, module: &ModuleIr, points: &[ProgramPoint]) -> Self {
        let mut steps = Vec::with_capacity(points.len());
        let mut stack: Vec<u32> = Vec::new();
        let mut derivation = Derivation::Exact;
        let mut prev: Option<ProgramPoint> = None;
        for &p in points {
            match (prev, stack.last().copied()) {
                (None, _) => stack.push(p.func),
                (Some(_), Some(top)) if top == p.func => {}
                (Some(q), _) => {
                    let call = call_at(result, module, q);
                    match call {
                        Some((site, callees)) if callees.contains(&p.func) => {
                            steps.push(TraceStep::Call {
                                site,
                                callee: p.func,
                            });
                            stack.push(p.func);
                        }
                        _ if stack.contains(&p.func) => {
                            while stack.last().copied() != Some(p.func) {
                                stack.pop();
                                steps.push(TraceStep::Return);
                            }
                        }
                        _ => {
                            derivation = std::mem::take(&mut derivation).weakest(
                                Derivation::partial(
                                    "a function change in the trace matches no call or return",
                                ),
                            );
                            steps.push(TraceStep::Call {
                                site: NO_ID,
                                callee: p.func,
                            });
                            stack.push(p.func);
                        }
                    }
                }
            }
            steps.push(TraceStep::At { point: p });
            prev = Some(p);
        }
        Trace { steps, derivation }
    }

    /// The witness path of a safety finding.
    pub fn of_finding(result: &HeapResult, module: &ModuleIr, finding: &Finding) -> Self {
        let mut t = Self::from_points(result, module, &finding.trace);
        t.derivation = std::mem::take(&mut t.derivation).weakest(finding.derivation.clone());
        t
    }
}

/// The call instruction at `point`, with its resolved callees.
fn call_at(result: &HeapResult, module: &ModuleIr, point: ProgramPoint) -> Option<(u32, Vec<u32>)> {
    let (f, fi) = result.index.func(module, point.func)?;
    let bp = *fi.block_pos.get(&point.block)?;
    match f.blocks[bp].insts.get(point.index as usize)? {
        Inst::Call { site, .. } => Some((
            site.0,
            result.index.callees.get(&site.0).cloned().unwrap_or_default(),
        )),
        _ => None,
    }
}

/// One SSA value of a frame.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrameLocal {
    pub value: u32,
    /// Parameter position, for parameters.
    pub param: Option<u32>,
    pub defined_at: Option<ProgramPoint>,
    /// Keys of the objects the value may reference.
    pub refs: Vec<String>,
    pub may_be_null: bool,
    pub may_be_uninitialized: bool,
}

/// A stack-region object owned by a frame.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StackObject {
    pub key: String,
    pub site: u32,
    pub life: LifeState,
    pub span: SrcSpan,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Frame {
    /// 0 for the outermost frame.
    pub depth: u32,
    pub func: u32,
    /// Interned function name.
    pub name: u32,
    /// Call site that created the frame (`None` for the outermost).
    pub call_site: Option<u32>,
    /// Where the frame currently executes (its pending call for non-top frames).
    pub point: ProgramPoint,
    pub span: SrcSpan,
    pub locals: Vec<FrameLocal>,
    pub stack_objects: Vec<StackObject>,
    pub derivation: Derivation,
}

/// A reference from a frame local into a non-stack object.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeapRef {
    pub depth: u32,
    pub value: u32,
    pub to: String,
    pub region: Option<MemRegion>,
}

/// The frame stack at one step of a trace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrameSnapshot {
    pub step: usize,
    pub point: ProgramPoint,
    pub span: SrcSpan,
    /// Outermost first.
    pub frames: Vec<Frame>,
    pub heap_refs: Vec<HeapRef>,
    pub derivation: Derivation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FramesView {
    pub snapshots: Vec<FrameSnapshot>,
    pub derivation: Derivation,
}

/// Abstract states of every point of the functions a trace touches, computed once each.
struct StateCache<'a> {
    result: &'a HeapResult,
    module: &'a ModuleIr,
    facts: &'a dyn MemoryFacts,
    states: HashMap<u32, BTreeMap<ProgramPoint, HeapState>>,
}

impl<'a> StateCache<'a> {
    fn at(&mut self, point: ProgramPoint) -> Option<&HeapState> {
        if !self.states.contains_key(&point.func) {
            let mut map = BTreeMap::new();
            self.result
                .replay(self.module, self.facts, point.func, &mut |p, _, st| {
                    map.entry(p).or_insert_with(|| st.clone());
                });
            self.states.insert(point.func, map);
        }
        self.states.get(&point.func).and_then(|m| m.get(&point))
    }
}

struct OpenFrame {
    func: u32,
    call_site: Option<u32>,
    point: Option<ProgramPoint>,
}

/// Builds one snapshot per `At` step of the trace.
pub fn build(
    result: &HeapResult,
    module: &ModuleIr,
    facts: &dyn MemoryFacts,
    trace: &Trace,
) -> FramesView {
    let mut cache = StateCache {
        result,
        module,
        facts,
        states: HashMap::new(),
    };
    let mut stack: Vec<OpenFrame> = Vec::new();
    let mut snapshots = Vec::new();
    let mut derivation = trace.derivation.clone();
    for (i, step) in trace.steps.iter().enumerate() {
        match *step {
            TraceStep::Call { site, callee } => stack.push(OpenFrame {
                func: callee,
                call_site: if site == NO_ID { None } else { Some(site) },
                point: None,
            }),
            TraceStep::Return | TraceStep::Unwind => {
                stack.pop();
            }
            TraceStep::At { point } => {
                if stack.is_empty() {
                    stack.push(OpenFrame {
                        func: point.func,
                        call_site: None,
                        point: None,
                    });
                }
                if let Some(top) = stack.last_mut() {
                    top.point = Some(point);
                }
                let snap = snapshot(&mut cache, i, point, &stack);
                derivation = derivation.weakest(snap.derivation.clone());
                snapshots.push(snap);
            }
        }
    }
    FramesView {
        snapshots,
        derivation,
    }
}

fn snapshot(
    cache: &mut StateCache<'_>,
    step: usize,
    point: ProgramPoint,
    stack: &[OpenFrame],
) -> FrameSnapshot {
    let result = cache.result;
    let module = cache.module;
    let mut frames = Vec::with_capacity(stack.len());
    let mut heap_refs = Vec::new();
    let mut derivation = Derivation::Exact;
    for (depth, of) in stack.iter().enumerate() {
        let depth = depth as u32;
        let Some(at) = of.point else {
            continue;
        };
        let (name, fi) = match result.index.func(module, of.func) {
            Some((f, fi)) => (f.name, Some(fi)),
            None => (NO_ID, None),
        };
        let Some(st) = cache.at(at).cloned() else {
            frames.push(Frame {
                depth,
                func: of.func,
                name,
                call_site: of.call_site,
                point: at,
                span: result.index.span_at(module, at),
                locals: Vec::new(),
                stack_objects: Vec::new(),
                derivation: Derivation::not_derivable("the point is unreachable in the abstract heap"),
            });
            derivation = derivation.weakest(Derivation::not_derivable(
                "a trace point is unreachable in the abstract heap",
            ));
            continue;
        };
        let mut frame_d = Derivation::Exact;
        let mut locals = Vec::new();
        for (v, pts) in &st.vals {
            let param = fi.and_then(|fi| fi.param_index.get(v).map(|&i| i as u32));
            if pts.is_empty() && param.is_none() {
                continue;
            }
            let defined_at = fi.and_then(|fi| {
                fi.def_site
                    .get(v)
                    .map(|&(bp, ii)| result.index.point(module, fi.pos, bp, ii))
            });
            let refs: Vec<String> = storage(pts).map(|o| o.key()).collect();
            if storage(pts).count() > 1 || (pts.contains(&AbsObj::Null) && pts.len() > 1) {
                frame_d = frame_d.weakest(Derivation::OverApproximate);
            }
            for o in storage(pts) {
                let region = result.sem.region(o);
                if region != Some(MemRegion::Stack) {
                    heap_refs.push(HeapRef {
                        depth,
                        value: *v,
                        to: o.key(),
                        region,
                    });
                }
            }
            locals.push(FrameLocal {
                value: *v,
                param,
                defined_at,
                refs,
                may_be_null: pts.contains(&AbsObj::Null),
                may_be_uninitialized: pts.contains(&AbsObj::Undef),
            });
        }
        let mut stack_objects = Vec::new();
        for o in st.objects() {
            let AbsObj::Site { site, .. } = o else {
                continue;
            };
            if result.sem.region(o) != Some(MemRegion::Stack)
                || result.index.site_func(module, site) != Some(of.func)
                || st.life_of(o) & LIFE_LIVE == 0
            {
                continue;
            }
            if o.is_old() {
                frame_d = frame_d.weakest(Derivation::OverApproximate);
            }
            stack_objects.push(StackObject {
                key: o.key(),
                site,
                life: LifeState::from_bits(st.life_of(o)),
                span: result.index.site_span(module, site),
            });
        }
        derivation = derivation.weakest(frame_d.clone());
        frames.push(Frame {
            depth,
            func: of.func,
            name,
            call_site: of.call_site,
            point: at,
            span: result.index.span_at(module, at),
            locals,
            stack_objects,
            derivation: frame_d,
        });
    }
    FrameSnapshot {
        step,
        point,
        span: result.index.span_at(module, point),
        frames,
        heap_refs,
        derivation,
    }
}
