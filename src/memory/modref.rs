//! Mod/Ref Summaries and Alias Queries (§M.6).
//!
//! **Mod/ref.** For every reachable function, the abstract cells it or anything it calls
//! may write (`mods`) and read (`refs`). Direct effects come from the function's own
//! loads, stores, releases, closure creation and intrinsics, mapped to cells through the
//! points-to sets of the place bases. Summaries are composed bottom-up over the strongly
//! connected components of the call graph (members of a component share one summary).
//! Calls into code outside the module may, under the sound external policy, read and
//! write every cell reachable from their arguments and the external world itself.
//!
//! With escape facts, each summary also has a *visible* part: the effects on cells of
//! objects that are frame-local to the function (`NoEscape`) are invisible to callers and
//! are not propagated into them.
//!
//! **Aliasing.** Two values may alias when their points-to sets intersect; they must alias
//! when they are the same SSA value, or both denote the same single object that exists
//! once (static storage). Places alias through their cells; distinct fields of one object
//! are distinct cells, the content slot designates the whole object and so may overlap
//! every slot of it.

use crate::ir::func::FuncIr;
use crate::ir::inst::{Inst, Region};
use crate::ir::module::ModuleIr;
use crate::ir::types::{BlockId, CallSiteId, FuncId, ValueId};
use crate::ir::value::{Operand, Place};
use crate::memory::callgraph::{tarjan_sccs, CallbackOrigin, InstanceId};
use crate::memory::escape::{reachable_objects, EscapeResult, EscapeState};
use crate::memory::location::{Cell, CellId, HeapBase, ObjId, Slot, SparseBitSet};
use crate::memory::points_to::{
    intrinsic_effect, ExternalPolicy, IntrinsicEffect, MemoryConfig, PointsToResult,
};
use crate::psa::bdd::fxhash::FxHashMap;

// ── Mod/ref ─────────────────────────────────────────────────────────────────

/// Whether an instruction or call may read or write a cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModRefInfo {
    NoModRef,
    Ref,
    Mod,
    ModRef,
}

impl ModRefInfo {
    fn of(may_mod: bool, may_ref: bool) -> Self {
        match (may_mod, may_ref) {
            (false, false) => ModRefInfo::NoModRef,
            (false, true) => ModRefInfo::Ref,
            (true, false) => ModRefInfo::Mod,
            (true, true) => ModRefInfo::ModRef,
        }
    }

    #[inline]
    pub fn may_mod(self) -> bool {
        matches!(self, ModRefInfo::Mod | ModRefInfo::ModRef)
    }

    #[inline]
    pub fn may_ref(self) -> bool {
        matches!(self, ModRefInfo::Ref | ModRefInfo::ModRef)
    }
}

/// Mod/ref summary of one function. Sets are over `CellId`.
#[derive(Debug, Clone)]
pub struct FunctionModRef {
    pub func: FuncId,
    /// Cells the function's own instructions may write / read.
    pub direct_mods: SparseBitSet,
    pub direct_refs: SparseBitSet,
    /// Cells the function or its callees may write / read.
    pub mods: SparseBitSet,
    pub refs: SparseBitSet,
    /// `mods` / `refs` without the cells of objects frame-local to the function: what
    /// callers can observe.
    pub visible_mods: SparseBitSet,
    pub visible_refs: SparseBitSet,
    /// The function or a callee calls code outside the module.
    pub calls_external: bool,
}

/// Mod/ref summaries of every reachable function.
#[derive(Debug, Clone)]
pub struct ModRefSummaries {
    pub functions: Vec<FunctionModRef>,
    /// Derived from a truncated points-to solve.
    pub partial: bool,
    index: FxHashMap<FuncId, usize>,
}

/// Effects being accumulated for one function.
#[derive(Default)]
struct Effects {
    mods: SparseBitSet,
    refs: SparseBitSet,
    external_roots: SparseBitSet,
    calls_external: bool,
}

impl ModRefSummaries {
    /// Compute the summaries. `escape` enables the visible-effect filtering.
    pub fn compute(
        module: &ModuleIr,
        pts: &PointsToResult,
        memory: &MemoryConfig,
        escape: Option<&EscapeResult>,
    ) -> Self {
        let reachable: Vec<FuncId> = pts.call_graph.reachable.clone();
        let pos: FxHashMap<FuncId, usize> = reachable
            .iter()
            .enumerate()
            .map(|(i, &f)| (f, i))
            .collect();
        let sound = memory.external == ExternalPolicy::Sound;

        // Direct effects.
        let mut direct: Vec<Effects> = Vec::with_capacity(reachable.len());
        for &f in &reachable {
            let mut e = Effects::default();
            if let Some(func) = module.funcs.get(f.index()).filter(|x| x.id == f) {
                for block in &func.blocks {
                    for inst in &block.insts {
                        inst_effects(module, pts, memory, func, inst, &mut e);
                    }
                }
            }
            if sound && !e.external_roots.is_empty() {
                external_effect(pts, &e.external_roots.clone(), &mut e);
            }
            direct.push(e);
        }

        // Call adjacency: resolved edges plus invocations by intrinsics.
        let mut adj: Vec<Vec<usize>> = vec![Vec::new(); reachable.len()];
        for edge in &pts.call_graph.edge_info {
            if let (Some(&a), Some(&b)) = (pos.get(&edge.caller), pos.get(&edge.callee)) {
                adj[a].push(b);
            }
        }
        for cb in &pts.call_graph.callbacks {
            if let CallbackOrigin::Intrinsic { func, .. } = cb.origin {
                if let (Some(&a), Some(&b)) = (pos.get(&func), pos.get(&cb.func)) {
                    adj[a].push(b);
                }
            }
        }
        for list in &mut adj {
            list.sort_unstable();
            list.dedup();
        }

        // Frame-local cells per function.
        let local_cells = |f: FuncId| -> SparseBitSet {
            let mut out = SparseBitSet::new();
            if let Some(esc) = escape {
                if let Some(fe) = esc.function(f) {
                    for o in fe.frame_local() {
                        for &c in pts.locations.cells_of(o) {
                            out.insert(c.0);
                        }
                    }
                }
            }
            out
        };

        let n = reachable.len();
        let mut mods = vec![SparseBitSet::new(); n];
        let mut refs = vec![SparseBitSet::new(); n];
        let mut vmods = vec![SparseBitSet::new(); n];
        let mut vrefs = vec![SparseBitSet::new(); n];
        let mut ext = vec![false; n];
        // Components arrive callees first.
        for comp in tarjan_sccs(&adj) {
            let mut m = SparseBitSet::new();
            let mut r = SparseBitSet::new();
            let mut x = false;
            for &i in &comp {
                m.union_with(&direct[i].mods);
                r.union_with(&direct[i].refs);
                x |= direct[i].calls_external;
                for &j in &adj[i] {
                    if !comp.contains(&j) {
                        m.union_with(&vmods[j]);
                        r.union_with(&vrefs[j]);
                        x |= ext[j];
                    }
                }
            }
            for &i in &comp {
                let local = local_cells(reachable[i]);
                vmods[i] = m.difference(&local);
                vrefs[i] = r.difference(&local);
                mods[i] = m.clone();
                refs[i] = r.clone();
                ext[i] = x;
            }
        }

        let mut functions = Vec::with_capacity(n);
        let mut index = FxHashMap::default();
        for (i, f) in reachable.iter().enumerate() {
            index.insert(*f, i);
            let d = std::mem::take(&mut direct[i]);
            functions.push(FunctionModRef {
                func: *f,
                direct_mods: d.mods,
                direct_refs: d.refs,
                mods: std::mem::take(&mut mods[i]),
                refs: std::mem::take(&mut refs[i]),
                visible_mods: std::mem::take(&mut vmods[i]),
                visible_refs: std::mem::take(&mut vrefs[i]),
                calls_external: ext[i],
            });
        }
        Self {
            functions,
            partial: !pts.is_complete(),
            index,
        }
    }

    pub fn of(&self, func: FuncId) -> Option<&FunctionModRef> {
        self.index.get(&func).map(|&i| &self.functions[i])
    }

    /// `func` (or a callee) may write `cell`, as observed by its callers.
    pub fn may_mod(&self, func: FuncId, cell: CellId) -> bool {
        self.of(func)
            .map(|s| s.visible_mods.contains(cell.0))
            .unwrap_or(false)
    }

    /// `func` (or a callee) may read `cell`, as observed by its callers.
    pub fn may_ref(&self, func: FuncId, cell: CellId) -> bool {
        self.of(func)
            .map(|s| s.visible_refs.contains(cell.0))
            .unwrap_or(false)
    }

    /// Effect of one call site on `cell`: the visible summaries of its targets and, for
    /// external targets under the sound policy, everything reachable from its arguments.
    pub fn mod_ref_of_call(
        &self,
        module: &ModuleIr,
        pts: &PointsToResult,
        memory: &MemoryConfig,
        site: CallSiteId,
        cell: CellId,
    ) -> ModRefInfo {
        let Some(res) = pts.call_graph.site(site) else {
            return ModRefInfo::NoModRef;
        };
        let mut m = false;
        let mut r = false;
        for &(f, _) in &res.targets {
            m |= self.may_mod(f, cell);
            r |= self.may_ref(f, cell);
        }
        if !res.external.is_empty() && memory.external == ExternalPolicy::Sound {
            if let Some((func, _, _, Inst::Call { args, .. })) = module.call_inst(site) {
                let mut roots = SparseBitSet::new();
                for a in args {
                    roots.union_with(pts.operand_points_to(func.id, a));
                }
                let mut e = Effects::default();
                external_effect(pts, &roots, &mut e);
                m |= e.mods.contains(cell.0);
                r |= e.refs.contains(cell.0);
            }
        }
        ModRefInfo::of(m, r)
    }

    /// Effect of the instruction at (`block`, `index`) of `func` on `cell`.
    #[allow(clippy::too_many_arguments)]
    pub fn mod_ref_of_inst(
        &self,
        module: &ModuleIr,
        pts: &PointsToResult,
        memory: &MemoryConfig,
        func: FuncId,
        block: BlockId,
        index: usize,
        cell: CellId,
    ) -> ModRefInfo {
        let Some(f) = module.funcs.get(func.index()).filter(|x| x.id == func) else {
            return ModRefInfo::NoModRef;
        };
        let Some(inst) = f.inst(block, index) else {
            return ModRefInfo::NoModRef;
        };
        if let Inst::Call { site, .. } = inst {
            return self.mod_ref_of_call(module, pts, memory, *site, cell);
        }
        let mut e = Effects::default();
        inst_effects(module, pts, memory, f, inst, &mut e);
        if memory.external == ExternalPolicy::Sound && !e.external_roots.is_empty() {
            let roots = e.external_roots.clone();
            external_effect(pts, &roots, &mut e);
        }
        ModRefInfo::of(e.mods.contains(cell.0), e.refs.contains(cell.0))
    }
}

fn cells_into(pts: &PointsToResult, func: FuncId, place: &Place, out: &mut SparseBitSet) {
    for c in pts.place_cells(func, place) {
        out.insert(c.0);
    }
}

fn object_cells_into(pts: &PointsToResult, objs: &SparseBitSet, out: &mut SparseBitSet) {
    for o in objs.objects() {
        for &c in pts.locations.cells_of(o) {
            out.insert(c.0);
        }
    }
}

fn slot_cells_into(pts: &PointsToResult, objs: &SparseBitSet, slot: Slot, out: &mut SparseBitSet) {
    for obj in objs.objects() {
        if let Some(c) = pts.locations.find_cell(&Cell::Object { obj, slot }) {
            out.insert(c.0);
        }
    }
}

/// Direct cell effects of one non-call instruction (calls contribute only their external
/// arguments here; their targets are composed through the summaries).
fn inst_effects(
    module: &ModuleIr,
    pts: &PointsToResult,
    memory: &MemoryConfig,
    func: &FuncIr,
    inst: &Inst,
    e: &mut Effects,
) {
    let arg_objs = |args: &[Operand], k: u32| -> SparseBitSet {
        args.get(k as usize)
            .map(|a| pts.operand_points_to(func.id, a).clone())
            .unwrap_or_default()
    };
    match inst {
        Inst::Load { place, .. } => cells_into(pts, func.id, place, &mut e.refs),
        Inst::Store { place, .. } => cells_into(pts, func.id, place, &mut e.mods),
        Inst::Release { place } => {
            // Ending a lifetime reads the place and destroys what it designates.
            cells_into(pts, func.id, place, &mut e.refs);
            let mut released = pts.place_points_to(func.id, place);
            if let Place::Local(v) | Place::Deref(v) = place {
                released.union_with(pts.points_to(func.id, *v));
            }
            object_cells_into(pts, &released, &mut e.mods);
        }
        Inst::MakeClosure { dst, .. } => {
            let objs = pts.points_to(func.id, *dst);
            for obj in objs.objects() {
                if matches!(pts.locations.object(obj).object.base, HeapBase::Closure(_)) {
                    for &c in pts.locations.cells_of(obj) {
                        if matches!(pts.locations.cell(c), Cell::Object { slot: Slot::Capture(_), .. }) {
                            e.mods.insert(c.0);
                        }
                    }
                }
            }
        }
        Inst::Call { site, args, .. } => {
            if let Some(res) = pts.call_graph.site(*site) {
                if !res.external.is_empty() {
                    e.calls_external = true;
                    for a in args {
                        e.external_roots.union_with(pts.operand_points_to(func.id, a));
                    }
                }
            }
        }
        Inst::Intrinsic { name, args, .. } => match intrinsic_effect(module, memory, *name) {
            IntrinsicEffect::CopiesContents { dst, src } => {
                object_cells_into(pts, &arg_objs(args, src), &mut e.refs);
                // Every slot of the source objects is written into the destination objects.
                let src_objs = arg_objs(args, src);
                let dst_objs = arg_objs(args, dst);
                for s in src_objs.objects() {
                    for &c in pts.locations.cells_of(s) {
                        if let Cell::Object { slot, .. } = pts.locations.cell(c) {
                            slot_cells_into(pts, &dst_objs, slot, &mut e.mods);
                        }
                    }
                }
            }
            IntrinsicEffect::StoresElement { array, .. } => {
                slot_cells_into(pts, &arg_objs(args, array), Slot::Element, &mut e.mods)
            }
            IntrinsicEffect::LoadsElement { array } => {
                slot_cells_into(pts, &arg_objs(args, array), Slot::Element, &mut e.refs)
            }
            IntrinsicEffect::IteratesElements { iterable } => {
                slot_cells_into(pts, &arg_objs(args, iterable), Slot::Element, &mut e.refs)
            }
            IntrinsicEffect::Opaque => {
                e.calls_external = true;
                for a in args {
                    e.external_roots.union_with(pts.operand_points_to(func.id, a));
                }
            }
            IntrinsicEffect::Pure
            | IntrinsicEffect::ReturnsArg(_)
            | IntrinsicEffect::Allocates { .. }
            | IntrinsicEffect::ReturnsUnknown
            | IntrinsicEffect::MergesArgs { .. }
            | IntrinsicEffect::Invokes { .. }
            | IntrinsicEffect::VarArg
            | IntrinsicEffect::CurrentException => {}
        },
        Inst::Assign { .. }
        | Inst::Unary { .. }
        | Inst::Binary { .. }
        | Inst::Compare { .. }
        | Inst::Alloc { .. }
        | Inst::Phi { .. }
        | Inst::Cast { .. }
        | Inst::TypeTest { .. } => {}
    }
}

/// External code may read and write every cell reachable from what it is handed, and the
/// external world itself.
fn external_effect(pts: &PointsToResult, roots: &SparseBitSet, e: &mut Effects) {
    let mut all = reachable_objects(pts, roots);
    all.union_with(pts.external_world());
    let mut cells = SparseBitSet::new();
    object_cells_into(pts, &all, &mut cells);
    cells.insert(pts.external_cell().0);
    e.mods.union_with(&cells);
    e.refs.union_with(&cells);
    e.calls_external = true;
}

// ── Alias queries ───────────────────────────────────────────────────────────

/// Outcome of an alias query.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AliasResult {
    NoAlias,
    MayAlias,
    MustAlias,
}

/// Alias queries over a points-to result.
#[derive(Debug, Clone, Copy)]
pub struct AliasOracle<'a> {
    pts: &'a PointsToResult,
}

impl<'a> AliasOracle<'a> {
    pub fn new(pts: &'a PointsToResult) -> Self {
        Self { pts }
    }

    /// Objects `value` of `func` may point to, over all contexts.
    pub fn points_to_of(&self, func: FuncId, value: ValueId) -> &'a SparseBitSet {
        self.pts.points_to(func, value)
    }

    /// Objects `value` may point to in one function instance.
    pub fn points_to_in(&self, instance: InstanceId, value: ValueId) -> &'a SparseBitSet {
        self.pts.points_to_in(instance, value)
    }

    /// The object exists exactly once at run time, so one abstract object is one concrete
    /// object: static storage.
    fn is_singular(&self, o: ObjId) -> bool {
        let info = self.pts.locations.object(o);
        matches!(info.region, Some(Region::Static)) && !info.object.base.is_opaque()
    }

    fn sets_alias(&self, a: &SparseBitSet, b: &SparseBitSet) -> AliasResult {
        if a.is_empty() || b.is_empty() || !a.intersects(b) {
            return AliasResult::NoAlias;
        }
        if a.len() == 1 && a == b {
            if let Some(o) = a.first() {
                if self.is_singular(ObjId(o)) {
                    return AliasResult::MustAlias;
                }
            }
        }
        AliasResult::MayAlias
    }

    /// May `a` of `func_a` and `b` of `func_b` denote the same object?
    pub fn may_alias(&self, func_a: FuncId, a: ValueId, func_b: FuncId, b: ValueId) -> AliasResult {
        if func_a == func_b && a == b {
            return AliasResult::MustAlias;
        }
        self.sets_alias(self.pts.points_to(func_a, a), self.pts.points_to(func_b, b))
    }

    /// Context-sensitive variant of [`AliasOracle::may_alias`].
    pub fn may_alias_in(
        &self,
        inst_a: InstanceId,
        a: ValueId,
        inst_b: InstanceId,
        b: ValueId,
    ) -> AliasResult {
        if inst_a == inst_b && a == b {
            return AliasResult::MustAlias;
        }
        self.sets_alias(self.pts.points_to_in(inst_a, a), self.pts.points_to_in(inst_b, b))
    }

    /// Cells a place may denote.
    pub fn place_cells(&self, func: FuncId, place: &Place) -> Vec<CellId> {
        self.pts.place_cells(func, place)
    }

    /// May two places overlap?
    pub fn places_may_alias(
        &self,
        func_a: FuncId,
        a: &Place,
        func_b: FuncId,
        b: &Place,
    ) -> AliasResult {
        match (a, b) {
            (Place::Global(x), Place::Global(y)) => {
                if x == y {
                    AliasResult::MustAlias
                } else {
                    AliasResult::NoAlias
                }
            }
            (
                Place::Static {
                    owner: oa,
                    field: fa,
                },
                Place::Static {
                    owner: ob,
                    field: fb,
                },
            ) => {
                if oa == ob && fa == fb {
                    AliasResult::MustAlias
                } else {
                    AliasResult::NoAlias
                }
            }
            (Place::Global(_) | Place::Static { .. }, _) | (_, Place::Global(_) | Place::Static { .. }) => {
                AliasResult::NoAlias
            }
            (
                Place::Field {
                    base: ba,
                    field: fa,
                },
                Place::Field {
                    base: bb,
                    field: fb,
                },
            ) => {
                if fa != fb {
                    AliasResult::NoAlias
                } else {
                    self.may_alias(func_a, *ba, func_b, *bb)
                }
            }
            (Place::Index { base: ba, .. }, Place::Index { base: bb, .. }) => {
                // One summarized element slot: indices are not distinguished.
                match self.may_alias(func_a, *ba, func_b, *bb) {
                    AliasResult::NoAlias => AliasResult::NoAlias,
                    _ => AliasResult::MayAlias,
                }
            }
            (Place::Field { .. }, Place::Index { .. }) | (Place::Index { .. }, Place::Field { .. }) => {
                AliasResult::NoAlias
            }
            // A content slot designates the whole object: it overlaps every slot of every
            // object its base may denote.
            _ => {
                let base_a = a.base();
                let base_b = b.base();
                match (base_a, base_b) {
                    (Some(x), Some(y)) => {
                        let both_whole = matches!(a, Place::Local(_) | Place::Deref(_))
                            && matches!(b, Place::Local(_) | Place::Deref(_));
                        match self.may_alias(func_a, x, func_b, y) {
                            AliasResult::NoAlias => AliasResult::NoAlias,
                            AliasResult::MustAlias if both_whole => AliasResult::MustAlias,
                            _ => AliasResult::MayAlias,
                        }
                    }
                    _ => AliasResult::MayAlias,
                }
            }
        }
    }

    /// Escape state of every object a value may denote, highest first.
    pub fn escape_of(&self, escape: &EscapeResult, func: FuncId, value: ValueId) -> Option<EscapeState> {
        self.pts
            .points_to(func, value)
            .objects()
            .map(|o| escape.state(o))
            .max()
    }
}
