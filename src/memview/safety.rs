//! Memory-safety findings with witnesses.
//!
//! Every check reads the abstract heap of [`HeapResult`] just before an instruction (or at
//! a function exit) and reports a [`Finding`] only when the state proves the defect on
//! some path the abstraction represents. Each finding carries the program points of a
//! witness path through the CFG and the facts that justify it (where the object was
//! allocated, released and used; which slot was never written; which range the index and
//! the length lie in). Checks whose inputs are missing are not guessed: they are counted
//! in [`SafetyReport::skipped`] with the reason.
//!
//! | Check | Source of truth |
//! |---|---|
//! | use after free / release | lifetime bits of the accessed objects |
//! | double free | lifetime bits of the objects a release releases |
//! | leak | allocations no root reaches at an exit, never released on that path |
//! | null dereference | `Null` in the points-to set of a dereferenced value |
//! | out of bounds | index interval against the array length from types or layouts |
//! | uninitialized read | IR `Undef` operands and reads of never-written slots |
//! | dangling stack reference | stack objects reachable from roots after their frame ends |

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::ir::{
    AllocSiteId, Callee, ConstVal, FuncIr, Inst, ModuleIr, Operand, Place, Terminator, ValueId,
};
use crate::memview::heap_shape::{
    storage, HeapResult, HeapState, MemEventKind, MemRegion, PtsSet, Step, LIFE_LIVE,
    LIFE_RELEASED, REL_DEALLOCATE, REL_DISPOSE, REL_END_OF_SCOPE, REL_UNKNOWN,
};
use crate::memview::{
    AbsObj, Derivation, FieldKey, FuncIndex, MemoryFacts, ModuleIndex, Nullness, OperandRef,
    ProgramPoint, ReleaseKind, SrcSpan,
};

/// Class of a memory-safety defect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingKind {
    UseAfterFree,
    /// Use of an object after a dispose / close protocol (or an unknown release) ended it.
    UseAfterRelease,
    DoubleFree,
    Leak,
    NullDereference,
    OutOfBounds,
    UninitializedRead,
    DanglingStackReference,
}

/// One fact a finding rests on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "fact")]
pub enum WitnessFact {
    Allocation {
        obj: AbsObj,
        point: Option<ProgramPoint>,
        span: SrcSpan,
    },
    Release {
        obj: AbsObj,
        point: ProgramPoint,
        kind: Option<ReleaseKind>,
    },
    /// The object was released inside a callee; the caller only sees the effect.
    ReleasedByCallee { obj: AbsObj },
    Access {
        obj: AbsObj,
        point: ProgramPoint,
        write: bool,
        via_call: bool,
    },
    /// `value` may hold null: its definition at `point` admits it.
    NullSource { value: u32, point: Option<ProgramPoint> },
    IndexRange { value: Option<u32>, lo: i128, hi: i128 },
    LengthRange { obj: AbsObj, lo: i128, hi: i128 },
    /// A read whose source holds an indeterminate value.
    Uninitialized {
        value: Option<u32>,
        obj: Option<AbsObj>,
        slot: Option<FieldKey>,
    },
    /// The frame owning a stack object ends at `point`.
    FrameEnds { obj: AbsObj, func: u32, point: ProgramPoint },
    /// No root reaches the object after `point`.
    Unreachable { obj: AbsObj, point: ProgramPoint },
    /// The object is reachable from a root that outlives the frame.
    ReachableFrom { obj: AbsObj, root: AbsObj },
    /// A comparison against null guards the point; its direction needs predicate facts.
    NullGuard { compare: ProgramPoint },
}

/// A memory-safety defect with its witness.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    pub kind: FindingKind,
    pub func: u32,
    pub point: ProgramPoint,
    pub span: SrcSpan,
    pub objects: Vec<AbsObj>,
    /// Program points of a witness path ending at `point`.
    pub trace: Vec<ProgramPoint>,
    pub trace_spans: Vec<SrcSpan>,
    pub facts: Vec<WitnessFact>,
    pub derivation: Derivation,
}

/// A check that could not run on some inputs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkippedCheck {
    pub check: FindingKind,
    pub count: usize,
    pub derivation: Derivation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SafetyReport {
    pub findings: Vec<Finding>,
    pub skipped: Vec<SkippedCheck>,
    pub derivation: Derivation,
}

/// Runs every check over every function.
pub fn analyze(result: &HeapResult, module: &ModuleIr, facts: &dyn MemoryFacts) -> SafetyReport {
    let mut ctx = Checker {
        result,
        module,
        facts,
        index: &result.index,
        findings: Vec::new(),
        skipped: BTreeMap::new(),
        released_sites: released_groups(result),
    };
    let mut ids: Vec<u32> = result.funcs.keys().copied().collect();
    ids.sort_unstable();
    for f in ids {
        ctx.check_function(f);
    }
    let mut derivation = result.derivation.clone();
    let skipped: Vec<SkippedCheck> = ctx
        .skipped
        .into_iter()
        .map(|(check, (count, reason))| {
            derivation = std::mem::take(&mut derivation).weakest(Derivation::partial(format!(
                "{count} {check:?} checks skipped"
            )));
            SkippedCheck {
                check,
                count,
                derivation: Derivation::not_derivable(reason),
            }
        })
        .collect();
    let mut findings = ctx.findings;
    findings.sort_by(|a, b| (a.func, a.point, a.kind).cmp(&(b.func, b.point, b.kind)));
    findings.dedup_by(|a, b| a.kind == b.kind && a.point == b.point && a.objects == b.objects);
    SafetyReport {
        findings,
        skipped,
        derivation,
    }
}

/// Allocation groups released somewhere in the module (with a known or unknown kind other
/// than unlocking): the program itself shows they follow a release protocol.
fn released_groups(result: &HeapResult) -> BTreeSet<AbsObj> {
    let mut out = BTreeSet::new();
    for e in result.all_events() {
        if let MemEventKind::Released { kind, .. } = &e.kind {
            if *kind != Some(ReleaseKind::Unlock) && *kind != Some(ReleaseKind::EndOfScope) {
                out.insert(e.obj.as_recent());
            }
        }
    }
    out
}

struct Checker<'a> {
    result: &'a HeapResult,
    module: &'a ModuleIr,
    facts: &'a dyn MemoryFacts,
    index: &'a ModuleIndex,
    findings: Vec<Finding>,
    skipped: BTreeMap<FindingKind, (usize, String)>,
    released_sites: BTreeSet<AbsObj>,
}

/// Per-function facts the instruction visitor needs.
struct FuncFacts {
    /// Point -> objects released there (excluding unlocks), with the release kind.
    released: BTreeMap<ProgramPoint, Vec<(AbsObj, Option<ReleaseKind>)>>,
    /// Point -> objects accessed there.
    accessed: BTreeMap<ProgramPoint, Vec<(AbsObj, bool, bool)>>,
    /// Values whose indeterminate contents come from a memory read (reported at the read).
    undef_from_load: BTreeSet<u32>,
    /// Value -> points of branches on a comparison of the value against null.
    null_guards: BTreeMap<u32, Vec<(usize, ProgramPoint)>>,
}

impl<'a> Checker<'a> {
    fn skip(&mut self, check: FindingKind, reason: &str) {
        let e = self
            .skipped
            .entry(check)
            .or_insert_with(|| (0, reason.to_string()));
        e.0 += 1;
    }

    fn check_function(&mut self, func: u32) {
        let (result, index, module, facts) = (self.result, self.index, self.module, self.facts);
        let Some(fh) = result.funcs.get(&func) else {
            return;
        };
        let Some((f, fi)) = index.func(module, func) else {
            return;
        };
        let ff = self.func_facts(func, f, fi);

        let mut pending: Vec<Finding> = Vec::new();
        let mut skips: Vec<(FindingKind, &'static str)> = Vec::new();
        {
            let this = &*self;
            result.replay(module, facts, func, &mut |point, step, st| {
                this.visit(point, step, st, f, fi, &ff, &mut pending, &mut skips);
            });
        }
        for (k, r) in skips {
            self.skip(k, r);
        }
        self.findings.extend(pending);

        // Exit checks: leaks and dangling stack references.
        for exit in &fh.exits {
            self.check_exit(func, f, fi, exit.point, exit.exceptional, &exit.returned, &exit.state);
        }
    }

    fn func_facts(&self, func: u32, f: &FuncIr, fi: &FuncIndex) -> FuncFacts {
        let fh = &self.result.funcs[&func];
        let mut released: BTreeMap<ProgramPoint, Vec<(AbsObj, Option<ReleaseKind>)>> =
            BTreeMap::new();
        let mut accessed: BTreeMap<ProgramPoint, Vec<(AbsObj, bool, bool)>> = BTreeMap::new();
        for e in &fh.events {
            match &e.kind {
                MemEventKind::Released { kind, .. } if *kind != Some(ReleaseKind::Unlock) => {
                    released.entry(e.point).or_default().push((e.obj, *kind));
                }
                MemEventKind::Accessed {
                    write, via_call, ..
                } => {
                    accessed
                        .entry(e.point)
                        .or_default()
                        .push((e.obj, *write, *via_call));
                }
                _ => {}
            }
        }

        // Indeterminate values that originate in memory reads, closed under copies and
        // arithmetic.
        let mut undef_from_load: BTreeSet<u32> = BTreeSet::new();
        let mut changed = true;
        while changed {
            changed = false;
            for &bp in &fi.rpo {
                for inst in &f.blocks[bp].insts {
                    let Some(d) = crate::memview::inst_def(inst) else {
                        continue;
                    };
                    if undef_from_load.contains(&d.0) {
                        continue;
                    }
                    let from_load = match inst {
                        Inst::Load { .. } => fh
                            .values
                            .get(&d.0)
                            .map(|p| p.contains(&AbsObj::Undef))
                            .unwrap_or(false),
                        Inst::Assign { .. }
                        | Inst::Cast { .. }
                        | Inst::Unary { .. }
                        | Inst::Binary { .. }
                        | Inst::Compare { .. }
                        | Inst::Phi { .. } => crate::memview::inst_uses(inst)
                            .iter()
                            .any(|u| undef_from_load.contains(&u.0)),
                        _ => false,
                    };
                    if from_load {
                        undef_from_load.insert(d.0);
                        changed = true;
                    }
                }
            }
        }

        // Comparisons against null that decide a branch.
        let mut compares: BTreeMap<u32, (u32, ProgramPoint)> = BTreeMap::new();
        for (bp, block) in f.blocks.iter().enumerate() {
            for (ii, inst) in block.insts.iter().enumerate() {
                if let Inst::Compare { dst, lhs, rhs, .. } = inst {
                    let l: &dyn OperandRef = lhs;
                    let r: &dyn OperandRef = rhs;
                    let tested = match (is_null(l), is_null(r)) {
                        (true, false) => r.value(),
                        (false, true) => l.value(),
                        _ => None,
                    };
                    if let Some(v) = tested {
                        compares.insert(dst.0, (v.0, self.index.point(self.module, fi.pos, bp, ii)));
                    }
                }
            }
        }
        let mut null_guards: BTreeMap<u32, Vec<(usize, ProgramPoint)>> = BTreeMap::new();
        for (bp, block) in f.blocks.iter().enumerate() {
            if let Terminator::Branch { cond, .. } = &block.term {
                if let Some(c) = cond.value() {
                    if let Some(&(v, cp)) = compares.get(&c.0) {
                        null_guards.entry(v).or_default().push((bp, cp));
                    }
                }
            }
        }
        FuncFacts {
            released,
            accessed,
            undef_from_load,
            null_guards,
        }
    }

    fn trace(&self, fi: &FuncIndex, from: ProgramPoint, to: ProgramPoint) -> Vec<ProgramPoint> {
        if from.func != to.func {
            return vec![to];
        }
        let (Some(&fb), Some(&tb)) = (fi.block_pos.get(&from.block), fi.block_pos.get(&to.block))
        else {
            return vec![to];
        };
        let blocks = if fb == tb && from.index <= to.index {
            Some(vec![fb])
        } else if fb == tb {
            fi.cycle_through(fb)
        } else {
            fi.block_path(fb, tb)
        };
        match blocks {
            Some(b) => self.index.path_points(self.module, fi.pos, &b, from, to),
            None => vec![from, to],
        }
    }

    fn entry_point(&self, f: &FuncIr, fi: &FuncIndex) -> ProgramPoint {
        ProgramPoint {
            func: f.id.0,
            block: f.blocks[fi.entry].id.0,
            index: 0,
        }
    }

    fn alloc_point(&self, obj: AbsObj) -> Option<ProgramPoint> {
        let (fp, bp, ii) = match obj {
            AbsObj::Site { site, .. } => *self.index.site_inst.get(&site)?,
            AbsObj::CallAlloc { call_site, .. } => *self.index.call_inst.get(&call_site)?,
            _ => return None,
        };
        Some(self.index.point(self.module, fp, bp, ii))
    }

    fn alloc_fact(&self, obj: AbsObj) -> WitnessFact {
        let point = self.alloc_point(obj);
        let span = match obj {
            AbsObj::Site { site, .. } => self.index.site_span(self.module, site),
            _ => point
                .map(|p| self.index.span_at(self.module, p))
                .unwrap_or(SrcSpan::NONE),
        };
        WitnessFact::Allocation { obj, point, span }
    }

    fn finding(
        &self,
        kind: FindingKind,
        point: ProgramPoint,
        objects: Vec<AbsObj>,
        trace: Vec<ProgramPoint>,
        facts: Vec<WitnessFact>,
        derivation: Derivation,
    ) -> Finding {
        let trace_spans = trace
            .iter()
            .map(|p| self.index.span_at(self.module, *p))
            .collect();
        Finding {
            kind,
            func: point.func,
            point,
            span: self.index.span_at(self.module, point),
            objects,
            trace,
            trace_spans,
            facts,
            derivation,
        }
    }

    /// A witness path from the allocation (or function entry) through one release to `at`.
    fn release_trace(
        &self,
        f: &FuncIr,
        fi: &FuncIndex,
        obj: AbsObj,
        st: &HeapState,
        at: ProgramPoint,
    ) -> (Vec<ProgramPoint>, Vec<WitnessFact>) {
        let mut facts = vec![self.alloc_fact(obj)];
        let start = self
            .alloc_point(obj)
            .filter(|p| p.func == at.func)
            .unwrap_or_else(|| self.entry_point(f, fi));
        let release = st
            .released_at
            .get(&obj)
            .and_then(|pts| pts.iter().find(|p| p.func == at.func && **p != at).copied());
        let kind = crate::memview::heap_shape::kind_from_bits(
            st.release_kinds.get(&obj).copied().unwrap_or(REL_UNKNOWN),
        );
        let trace = match release {
            Some(rp) => {
                facts.push(WitnessFact::Release {
                    obj,
                    point: rp,
                    kind,
                });
                let mut t = self.trace(fi, start, rp);
                let rest = self.trace(fi, rp, at);
                t.extend(rest.into_iter().skip(1));
                t
            }
            None => {
                facts.push(WitnessFact::ReleasedByCallee { obj });
                self.trace(fi, start, at)
            }
        };
        (trace, facts)
    }

    /// Classifies a use of a released object by how it was released.
    fn use_kind(&self, st: &HeapState, obj: AbsObj) -> (FindingKind, Derivation) {
        let bits = st.release_kinds.get(&obj).copied().unwrap_or(REL_UNKNOWN);
        if bits == REL_END_OF_SCOPE {
            (FindingKind::DanglingStackReference, Derivation::Exact)
        } else if bits & REL_DEALLOCATE != 0 && bits & (REL_DISPOSE | REL_UNKNOWN) == 0 {
            (FindingKind::UseAfterFree, Derivation::Exact)
        } else if bits & REL_DISPOSE != 0 && bits & REL_UNKNOWN == 0 {
            (FindingKind::UseAfterRelease, Derivation::Exact)
        } else {
            (
                FindingKind::UseAfterRelease,
                Derivation::partial("the language layer did not state what the release does"),
            )
        }
    }

    /// Precision of a lifetime fact about `obj` in `st`.
    fn life_precision(&self, st: &HeapState, obj: AbsObj) -> Derivation {
        let bits = st.life_of(obj);
        if obj.is_old() || bits & LIFE_LIVE != 0 {
            Derivation::OverApproximate
        } else {
            Derivation::Exact
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn visit(
        &self,
        point: ProgramPoint,
        step: Step<'_>,
        st: &HeapState,
        f: &FuncIr,
        fi: &FuncIndex,
        ff: &FuncFacts,
        out: &mut Vec<Finding>,
        skips: &mut Vec<(FindingKind, &'static str)>,
    ) {
        // Use after free / release, and dangling use after scope end.
        for &(obj, write, via_call) in ff.accessed.get(&point).map(|v| v.as_slice()).unwrap_or(&[]) {
            if st.life_of(obj) & LIFE_RELEASED == 0 {
                continue;
            }
            let (kind, d) = self.use_kind(st, obj);
            let d = d.weakest(self.life_precision(st, obj));
            let (trace, mut facts) = self.release_trace(f, fi, obj, st, point);
            facts.push(WitnessFact::Access {
                obj,
                point,
                write,
                via_call,
            });
            out.push(self.finding(kind, point, vec![obj], trace, facts, d));
        }

        // Double free.
        for &(obj, kind) in ff.released.get(&point).map(|v| v.as_slice()).unwrap_or(&[]) {
            if kind == Some(ReleaseKind::EndOfScope) {
                continue;
            }
            if st.life_of(obj) & LIFE_RELEASED == 0 {
                continue;
            }
            let mut d = self.life_precision(st, obj);
            if kind.is_none() {
                d = d.weakest(Derivation::partial(
                    "the language layer did not state what the release does",
                ));
            }
            let (trace, mut facts) = self.release_trace(f, fi, obj, st, point);
            facts.push(WitnessFact::Release {
                obj,
                point,
                kind,
            });
            out.push(self.finding(FindingKind::DoubleFree, point, vec![obj], trace, facts, d));
        }

        let Step::Inst(inst) = step else {
            if let Step::Term(term) = step {
                self.visit_term(point, term, st, f, fi, ff, out);
            }
            return;
        };

        // Null dereference and indeterminate addresses.
        for base in deref_bases(inst) {
            self.check_null(point, base, st, f, fi, ff, out);
            self.check_undef_operand(point, &base, st, f, fi, ff, out);
        }

        match inst {
            Inst::Load { dst, place } => {
                if let Place::Index { base, index } = place {
                    self.check_bounds(point, *base, index, st, f, fi, out, skips);
                }
                self.check_uninit_load(point, dst.0, place, st, f, fi, out);
            }
            Inst::Store { place, .. } => {
                if let Place::Index { base, index } = place {
                    self.check_bounds(point, *base, index, st, f, fi, out, skips);
                }
            }
            Inst::Call { args, .. } => {
                for a in args {
                    self.check_undef_operand(point, a, st, f, fi, ff, out);
                }
            }
            _ => {}
        }
        if let Inst::Load { place: Place::Index { index, .. }, .. }
        | Inst::Store { place: Place::Index { index, .. }, .. } = inst
        {
            self.check_undef_operand(point, index, st, f, fi, ff, out);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn visit_term(
        &self,
        point: ProgramPoint,
        term: &Terminator,
        st: &HeapState,
        f: &FuncIr,
        fi: &FuncIndex,
        ff: &FuncFacts,
        out: &mut Vec<Finding>,
    ) {
        match term {
            Terminator::Branch { cond, .. } => self.check_undef_operand(point, cond, st, f, fi, ff, out),
            Terminator::Switch { scrutinee, .. } => {
                self.check_undef_operand(point, scrutinee, st, f, fi, ff, out)
            }
            _ => {}
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn check_null(
        &self,
        point: ProgramPoint,
        base: ValueId,
        st: &HeapState,
        f: &FuncIr,
        fi: &FuncIndex,
        ff: &FuncFacts,
        out: &mut Vec<Finding>,
    ) {
        let pts = st.pts(base.0);
        let supplied = self.facts.nullness(point, base);
        let mut d = match supplied {
            Some(Nullness::NonNull) => return,
            Some(Nullness::Null) => Derivation::Exact,
            Some(Nullness::MaybeNull) => Derivation::OverApproximate,
            None => {
                if !pts.contains(&AbsObj::Null) {
                    return;
                }
                if pts.len() == 1 {
                    Derivation::Exact
                } else {
                    Derivation::OverApproximate
                }
            }
        };
        let mut facts = Vec::new();
        let def_point = fi
            .def_site
            .get(&base.0)
            .map(|&(bp, ii)| self.index.point(self.module, fi.pos, bp, ii));
        facts.push(WitnessFact::NullSource {
            value: base.0,
            point: def_point,
        });
        if supplied.is_none() {
            // A branch on a null comparison of this value that dominates the dereference:
            // only predicate-aware nullness facts can tell which arm is safe.
            if let Some(&bp) = fi.block_pos.get(&point.block) {
                for &(gb, cp) in ff.null_guards.get(&base.0).map(|v| v.as_slice()).unwrap_or(&[]) {
                    let guarded = fi.succs[gb].iter().any(|&s| s != gb && fi.dominates(s, bp));
                    if guarded {
                        facts.push(WitnessFact::NullGuard { compare: cp });
                        d = d.weakest(Derivation::partial(
                            "guarded by a null comparison; branch direction needs the nullness analysis",
                        ));
                    }
                }
            }
        }
        let start = def_point.unwrap_or_else(|| self.entry_point(f, fi));
        let trace = self.trace(fi, start, point);
        out.push(self.finding(FindingKind::NullDereference, point, Vec::new(), trace, facts, d));
    }

    /// An indeterminate value reaching an address, an index, a branch or a call.
    #[allow(clippy::too_many_arguments)]
    fn check_undef_operand(
        &self,
        point: ProgramPoint,
        op: &dyn OperandRef,
        st: &HeapState,
        f: &FuncIr,
        fi: &FuncIndex,
        ff: &FuncFacts,
        out: &mut Vec<Finding>,
    ) {
        let (value, d) = if op.is_undef() {
            (None, Derivation::Exact)
        } else if let Some(v) = op.value() {
            if ff.undef_from_load.contains(&v.0) {
                return;
            }
            let pts = st.pts(v.0);
            if !pts.contains(&AbsObj::Undef) {
                return;
            }
            let d = if pts.len() == 1 {
                Derivation::Exact
            } else {
                Derivation::OverApproximate
            };
            (Some(v.0), d)
        } else {
            return;
        };
        let start = value
            .and_then(|v| fi.def_site.get(&v))
            .map(|&(bp, ii)| self.index.point(self.module, fi.pos, bp, ii))
            .unwrap_or_else(|| self.entry_point(f, fi));
        let facts = vec![WitnessFact::Uninitialized {
            value,
            obj: None,
            slot: None,
        }];
        let trace = self.trace(fi, start, point);
        out.push(self.finding(FindingKind::UninitializedRead, point, Vec::new(), trace, facts, d));
    }

    /// A read of a slot that holds an indeterminate value on some path.
    #[allow(clippy::too_many_arguments)]
    fn check_uninit_load(
        &self,
        point: ProgramPoint,
        dst: u32,
        place: &Place,
        st: &HeapState,
        f: &FuncIr,
        fi: &FuncIndex,
        out: &mut Vec<Finding>,
    ) {
        let (bases, slot): (PtsSet, FieldKey) = match place {
            Place::Local(v) | Place::Deref(v) => (st.pts(v.0), FieldKey::Whole),
            Place::Field { base, field } => (st.pts(base.0), FieldKey::Field(field.0)),
            Place::Index { base, .. } => (st.pts(base.0), FieldKey::Elem),
            Place::Global(_) | Place::Static { .. } => return,
        };
        let mut hit: Vec<AbsObj> = Vec::new();
        let mut only_undef = true;
        for o in storage(&bases) {
            let c = st.contents(&(o, slot), &self.result.sem);
            if c.contains(&AbsObj::Undef) {
                hit.push(o);
                if c.len() > 1 {
                    only_undef = false;
                }
            }
        }
        if hit.is_empty() {
            return;
        }
        let single = hit.len() == 1 && storage(&bases).count() == 1;
        let d = if only_undef && single && !hit[0].is_old() {
            Derivation::Exact
        } else {
            Derivation::OverApproximate
        };
        let mut facts: Vec<WitnessFact> = hit.iter().map(|o| self.alloc_fact(*o)).collect();
        for o in &hit {
            facts.push(WitnessFact::Uninitialized {
                value: Some(dst),
                obj: Some(*o),
                slot: Some(slot),
            });
        }
        let start = self
            .alloc_point(hit[0])
            .filter(|p| p.func == point.func)
            .unwrap_or_else(|| self.entry_point(f, fi));
        let trace = self.trace(fi, start, point);
        out.push(self.finding(FindingKind::UninitializedRead, point, hit, trace, facts, d));
    }

    #[allow(clippy::too_many_arguments)]
    fn check_bounds(
        &self,
        point: ProgramPoint,
        base: ValueId,
        index: &Operand,
        st: &HeapState,
        f: &FuncIr,
        fi: &FuncIndex,
        out: &mut Vec<Finding>,
        skips: &mut Vec<(FindingKind, &'static str)>,
    ) {
        let fh = &self.result.funcs[&point.func];
        if fh.interior.contains(&base.0) {
            skips.push((
                FindingKind::OutOfBounds,
                "the base is an interior pointer whose offset is not tracked",
            ));
            return;
        }
        let range = match index {
            Operand::Const(ConstVal::Int(n)) => Some((*n, *n)),
            Operand::Value(v) => self
                .facts
                .interval(point, *v)
                .or_else(|| const_value(f, fi, v.0).map(|n| (n, n))),
            _ => None,
        };
        let Some((ilo, ihi)) = range else {
            skips.push((
                FindingKind::OutOfBounds,
                "no interval facts for the index value",
            ));
            return;
        };
        let objs: Vec<AbsObj> = storage(&st.pts(base.0)).collect();
        let single = objs.len() == 1;
        for obj in objs {
            let len = match obj {
                AbsObj::Site { site, .. } => self.facts.array_length(AllocSiteId(site)).or_else(|| {
                    self.index
                        .site_type(self.module, site)
                        .and_then(|t| ModuleIndex::fixed_array_len(self.module, t))
                        .map(|n| (n as i128, n as i128))
                }),
                _ => None,
            };
            let Some((llo, lhi)) = len else {
                skips.push((
                    FindingKind::OutOfBounds,
                    "the array length is neither in the type nor in the layout facts",
                ));
                continue;
            };
            let definite = ihi < 0 || ilo >= lhi;
            let possible = ilo < 0 || ihi >= llo;
            if !possible {
                continue;
            }
            let d = if definite && single && !obj.is_old() {
                Derivation::Exact
            } else {
                Derivation::OverApproximate
            };
            let index_value = index.value().map(|v| v.0);
            let facts = vec![
                self.alloc_fact(obj),
                WitnessFact::IndexRange {
                    value: index_value,
                    lo: ilo,
                    hi: ihi,
                },
                WitnessFact::LengthRange {
                    obj,
                    lo: llo,
                    hi: lhi,
                },
            ];
            let start = self
                .alloc_point(obj)
                .filter(|p| p.func == point.func)
                .unwrap_or_else(|| self.entry_point(f, fi));
            let trace = self.trace(fi, start, point);
            out.push(self.finding(FindingKind::OutOfBounds, point, vec![obj], trace, facts, d));
        }
    }

    /// Leaks and dangling stack references at one function exit.
    #[allow(clippy::too_many_arguments)]
    fn check_exit(
        &mut self,
        func: u32,
        f: &FuncIr,
        fi: &FuncIndex,
        point: ProgramPoint,
        exceptional: bool,
        returned: &PtsSet,
        st: &HeapState,
    ) {
        // Roots that outlive the frame: the returned value, caller-visible memory, globals.
        let mut roots: Vec<AbsObj> = storage(returned).collect();
        let objects = st.objects();
        for o in &objects {
            if matches!(o, AbsObj::Ext { .. } | AbsObj::Global(_) | AbsObj::Static { .. }) {
                roots.push(*o);
            }
        }
        let outliving = st.reachable_from(roots.iter().copied());
        let escaped = st.reachable_from(st.escaped.iter().copied());

        for &o in &objects {
            if !o.is_allocation() {
                continue;
            }
            let bits = st.life_of(o);
            let region = self.result.sem.region(o);
            let own_stack = region == Some(MemRegion::Stack)
                && self.index.site_func(self.module, o.site().unwrap_or(u32::MAX)) == Some(func)
                && bits & LIFE_LIVE != 0;
            if own_stack && !outliving.contains(&o) && escaped.contains(&o) {
                self.skip(
                    FindingKind::DanglingStackReference,
                    "a stack object reaches code without modeled semantics, which may retain it",
                );
                continue;
            }

            // Dangling: a stack object of this frame that outlives it.
            if own_stack && outliving.contains(&o) {
                let root = roots
                    .iter()
                    .copied()
                    .find(|r| st.reachable_from([*r]).contains(&o))
                    .unwrap_or(AbsObj::Unknown);
                let direct = storage(returned).any(|r| r == o) && !exceptional;
                let d = if direct && returned.len() == 1 && !o.is_old() {
                    Derivation::Exact
                } else {
                    Derivation::OverApproximate
                };
                let facts = vec![
                    self.alloc_fact(o),
                    WitnessFact::ReachableFrom { obj: o, root },
                    WitnessFact::FrameEnds { obj: o, func, point },
                ];
                let start = self.alloc_point(o).unwrap_or_else(|| self.entry_point(f, fi));
                let trace = self.trace(fi, start, point);
                let finding = self.finding(
                    FindingKind::DanglingStackReference,
                    point,
                    vec![o],
                    trace,
                    facts,
                    d,
                );
                self.findings.push(finding);
                continue;
            }

            // Leak: allocated, never released on some path to this exit, and unreachable.
            if bits & LIFE_LIVE == 0
                || outliving.contains(&o)
                || escaped.contains(&o)
                || region == Some(MemRegion::Stack)
            {
                continue;
            }
            let required = match o {
                AbsObj::Site { site, .. } => self.facts.requires_release(AllocSiteId(site)),
                _ => None,
            };
            let mut d = match required {
                Some(false) => continue,
                Some(true) => Derivation::Exact,
                None if self.released_sites.contains(&o.as_recent()) => Derivation::OverApproximate,
                None => {
                    self.skip(
                        FindingKind::Leak,
                        "whether the objects must be released is neither supplied nor shown by \
                         a release elsewhere in the program",
                    );
                    continue;
                }
            };
            if bits & LIFE_RELEASED != 0 || o.is_old() {
                d = d.weakest(Derivation::OverApproximate);
            }
            if exceptional {
                d = d.weakest(Derivation::OverApproximate);
            }
            let facts = vec![
                self.alloc_fact(o),
                WitnessFact::Unreachable { obj: o, point },
            ];
            let start = self
                .alloc_point(o)
                .filter(|p| p.func == func)
                .unwrap_or_else(|| self.entry_point(f, fi));
            let trace = self.trace(fi, start, point);
            let finding = self.finding(FindingKind::Leak, point, vec![o], trace, facts, d);
            self.findings.push(finding);
        }
    }
}

fn is_null(op: &dyn OperandRef) -> bool {
    matches!(op.constant(), Some(ConstVal::Null))
}

/// The value a place dereferences, if any.
fn place_base(place: &Place) -> Option<ValueId> {
    match place {
        Place::Field { base, .. } | Place::Index { base, .. } => Some(*base),
        Place::Deref(v) => Some(*v),
        Place::Local(_) | Place::Global(_) | Place::Static { .. } => None,
    }
}

/// Values an instruction dereferences (place bases and method receivers).
fn deref_bases(inst: &Inst) -> Vec<ValueId> {
    match inst {
        Inst::Load { place, .. } | Inst::Store { place, .. } => place_base(place).into_iter().collect(),
        Inst::Call {
            callee: Callee::Virtual { receiver, .. } | Callee::Interface { receiver, .. },
            ..
        } => vec![*receiver],
        _ => Vec::new(),
    }
}

/// Integer constant of a value defined by copying a constant (through copies and casts).
fn const_value(f: &FuncIr, fi: &FuncIndex, v: u32) -> Option<i128> {
    let mut cur = v;
    for _ in 0..32 {
        let &(bp, ii) = fi.def_site.get(&cur)?;
        let next = match &f.blocks[bp].insts[ii] {
            Inst::Assign { src, .. } => src,
            _ => return None,
        };
        match next {
            Operand::Const(ConstVal::Int(n)) => return Some(*n),
            Operand::Value(w) => cur = w.0,
            _ => return None,
        }
    }
    None
}
