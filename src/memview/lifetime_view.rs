//! Per-object lifetime timelines.
//!
//! A timeline lists, in program order, every event of one allocation site's objects:
//! allocation, stores of references into other objects, accesses, passing to calls,
//! capture by closures, return and throw, escape, release and the exits where no root
//! reaches the object any more. Events come from the abstract heap's recording sweep, so
//! each one is backed by an IR instruction and its source span.
//!
//! Objects travel between functions through summaries: a callee's accesses and releases
//! of an argument appear at the caller's call point (`via_call`), and an object a callee
//! returns continues its timeline in the caller. Timelines are therefore split into one
//! segment per function, ordered callee-first (the order in which a returned object meets
//! them); within a segment, events follow reverse post-order.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde::{Deserialize, Serialize};

use crate::ir::ModuleIr;
use crate::memview::heap_shape::{HeapResult, MemEvent, MemEventKind, MemRegion};
use crate::memview::{AbsObj, Derivation, ProgramPoint, ReleaseKind, SrcSpan};

/// How the lifetime of an object ends, as far as the analyzed code shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "end")]
pub enum LifetimeEnd {
    /// Released explicitly; `kinds` lists the release semantics seen (`None` = unknown).
    Released {
        kinds: Vec<Option<ReleaseKind>>,
        on_every_path: bool,
    },
    /// No root reaches the object after the listed exits.
    BecomesUnreachable { at: Vec<ProgramPoint> },
    /// The object reaches memory the analysis cannot see; its end is outside the IR.
    Escapes,
    /// The object stays reachable from globals or from the outermost analyzed frame.
    OutlivesAnalyzedCode,
}

/// One event in a timeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineEntry {
    pub point: ProgramPoint,
    pub span: SrcSpan,
    /// The concrete abstract object (the recent instance or the old summary).
    pub obj: AbsObj,
    pub event: MemEventKind,
    pub value: Option<u32>,
}

/// The events of one object inside one function.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineSegment {
    pub func: u32,
    /// True for the function that performs the allocation.
    pub allocating: bool,
    pub entries: Vec<TimelineEntry>,
}

/// The lifetime of the objects of one allocation site (or one opaque object).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectTimeline {
    pub key: String,
    /// The recent object naming the group.
    pub obj: AbsObj,
    pub site: Option<u32>,
    pub region: Option<MemRegion>,
    pub type_id: Option<u32>,
    pub alloc_span: Option<SrcSpan>,
    pub segments: Vec<TimelineSegment>,
    pub ends: Vec<LifetimeEnd>,
    pub derivation: Derivation,
}

/// Lifetime timelines of every object of the module.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LifetimeView {
    pub timelines: Vec<ObjectTimeline>,
    pub derivation: Derivation,
    pub notes: Vec<String>,
}

/// Builds the timelines from the abstract-heap events.
pub fn build(result: &HeapResult, module: &ModuleIr) -> LifetimeView {
    let index = &result.index;
    let rank: HashMap<u32, usize> = index
        .bottom_up
        .iter()
        .enumerate()
        .map(|(i, f)| (*f, i))
        .collect();
    let order = |p: &ProgramPoint| -> (usize, usize, u32) {
        let fr = rank.get(&p.func).copied().unwrap_or(usize::MAX);
        let br = index
            .func(module, p.func)
            .and_then(|(_, fi)| fi.block_pos.get(&p.block).map(|&b| fi.rpo_num[b]))
            .unwrap_or(usize::MAX);
        (fr, br, p.index)
    };

    // Group events by allocation site (recent and old together) or by object.
    let mut groups: BTreeMap<AbsObj, Vec<&MemEvent>> = BTreeMap::new();
    for e in result.all_events() {
        groups.entry(e.obj.as_recent()).or_default().push(e);
    }

    let mut timelines = Vec::with_capacity(groups.len());
    let mut derivation = result.derivation.clone();
    for (obj, mut events) in groups {
        events.sort_by_key(|e| order(&e.point));
        let (site, type_id, alloc_span, alloc_func) = match obj {
            AbsObj::Site { site, .. } => (
                Some(site),
                index.site_type(module, site).map(|t| t.0),
                Some(index.site_span(module, site)),
                index.site_func(module, site),
            ),
            AbsObj::CallAlloc { call_site, .. } => (
                Some(call_site),
                None,
                index
                    .call_site_pos
                    .get(&call_site)
                    .map(|&p| SrcSpan::from_ir(&module.call_sites[p].span)),
                index
                    .call_site_pos
                    .get(&call_site)
                    .map(|&p| module.call_sites[p].func.0),
            ),
            AbsObj::Ext { func, .. } | AbsObj::Closure { func, .. } => (None, None, None, Some(func)),
            _ => (None, None, None, None),
        };

        let mut segments: Vec<TimelineSegment> = Vec::new();
        for e in &events {
            let entry = TimelineEntry {
                point: e.point,
                span: e.span,
                obj: e.obj,
                event: e.kind.clone(),
                value: e.value,
            };
            match segments.last_mut() {
                Some(seg) if seg.func == e.point.func => seg.entries.push(entry),
                _ => segments.push(TimelineSegment {
                    func: e.point.func,
                    allocating: alloc_func == Some(e.point.func),
                    entries: vec![entry],
                }),
            }
        }

        let ends = lifetime_ends(&events, result);
        let mut tl_derivation = match obj {
            AbsObj::Ext { .. } => Derivation::partial(
                "allocated outside the analyzed function; earlier events are in its callers",
            ),
            _ => Derivation::Exact,
        };
        if events.iter().any(|e| e.obj.is_old()) {
            tl_derivation = tl_derivation.weakest(Derivation::OverApproximate);
        }
        let unknown_release = events.iter().any(|e| {
            matches!(
                e.kind,
                MemEventKind::Released { kind: None, .. }
            )
        });
        if unknown_release {
            tl_derivation = tl_derivation.weakest(Derivation::partial(
                "the language layer did not state what some releases do",
            ));
        }
        let funcs: BTreeSet<u32> = events.iter().map(|e| e.point.func).collect();
        if funcs.iter().any(|f| {
            result
                .summaries
                .get(f)
                .map(|s| s.reaches_unmodeled)
                .unwrap_or(false)
        }) {
            tl_derivation = tl_derivation.weakest(Derivation::partial(
                "some calls on the object's path have no modeled semantics",
            ));
        }
        derivation = derivation.weakest(tl_derivation.clone());
        timelines.push(ObjectTimeline {
            key: obj.key(),
            obj,
            site,
            region: result.sem.region(obj),
            type_id,
            alloc_span,
            segments,
            ends,
            derivation: tl_derivation,
        });
    }

    LifetimeView {
        timelines,
        derivation,
        notes: result.notes.clone(),
    }
}

/// Derives how a group's lifetime ends from its events.
fn lifetime_ends(events: &[&MemEvent], result: &HeapResult) -> Vec<LifetimeEnd> {
    let mut ends = Vec::new();
    let mut kinds: Vec<Option<ReleaseKind>> = Vec::new();
    let mut must = false;
    let mut released = false;
    let mut unreachable: Vec<ProgramPoint> = Vec::new();
    let mut escaped = false;
    let mut returned_in: BTreeSet<u32> = BTreeSet::new();
    let mut stored_global = false;
    for e in events {
        match &e.kind {
            MemEventKind::Released { kind, must: m } => {
                if *kind == Some(ReleaseKind::Unlock) {
                    continue;
                }
                released = true;
                must |= *m;
                if !kinds.contains(kind) {
                    kinds.push(*kind);
                }
            }
            MemEventKind::Unreachable => unreachable.push(e.point),
            MemEventKind::Escaped | MemEventKind::Thrown => escaped = true,
            MemEventKind::Returned => {
                returned_in.insert(e.point.func);
            }
            MemEventKind::StoredInto { container, .. } => {
                if matches!(container, AbsObj::Global(_) | AbsObj::Static { .. }) {
                    stored_global = true;
                }
            }
            _ => {}
        }
    }
    if released {
        ends.push(LifetimeEnd::Released {
            kinds,
            on_every_path: must,
        });
    }
    if !unreachable.is_empty() {
        ends.push(LifetimeEnd::BecomesUnreachable { at: unreachable });
    }
    if escaped {
        ends.push(LifetimeEnd::Escapes);
    }
    // Returned from a function nobody calls, or parked in a global.
    let returned_to_nobody = returned_in.iter().any(|f| {
        result
            .index
            .callers
            .get(f)
            .map(|c| c.is_empty())
            .unwrap_or(true)
    });
    if stored_global || returned_to_nobody {
        ends.push(LifetimeEnd::OutlivesAnalyzedCode);
    }
    ends
}
