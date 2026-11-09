//! Reaching definitions and def-use chains on the IR.
//!
//! **SSA values** need no data flow: every value has one definition, so [`ValueDefUse`]
//! records it together with every instruction and terminator that reads the value.
//!
//! **Memory** does. Locations are named by [`MemLoc`], the canonical form of an IR `Place`
//! with copy-resolved SSA bases. A definition of a location is a `Store`, a `Release`
//! (end of lifetime: destructor, dispose, unlock), a possible write by a `Call`, or the
//! *initial content* of the location — at function entry, or where the SSA base it is named
//! through is defined (an `Alloc` yields fresh content; any other definition yields whatever
//! the object held when the value was obtained).
//!
//! Strong updates follow the alias oracle's `must_alias`. Because a location named through
//! an SSA value `v` denotes the object of `v`'s *current dynamic instance*, executing the
//! definition of `v` again (a loop) turns every reaching definition keyed on `v` *stale*:
//! a stale definition still reaches later loads that may alias it, but it can no longer be
//! killed by a store through the new instance of `v`. The domain is therefore a bit set over
//! `2·D` bits — bit `d` for "definition `d` reaches with the current instance of its base",
//! bit `D + d` for "reaches through an earlier instance".
//!
//! A load's chain is every reaching definition the oracle says may write the loaded location.

use std::collections::HashMap;

use crate::analysis::lattice::BitSet;
use crate::analysis::solver::{
    def_points, for_each_inst_use, for_each_term_use, inst_def, replay_forward, resolve_copies,
    solve_forward, value_count, DataflowResult, DefPoint, ForwardTransfer, InstPoint, IrCfg,
    ResolvedOperand, SolverOptions,
};
use crate::ir::{Callee, CallSiteId, FieldId, FuncIr, GlobalId, Inst, Place, ValueId};

// ── SSA def-use chains ───────────────────────────────────────────────────────

/// A read of an SSA value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum UseSite {
    /// Instruction `index` of block position `block` (phis included).
    Inst { block: u32, index: u32 },
    /// The terminator of block position `block`.
    Terminator { block: u32 },
}

impl UseSite {
    pub fn block(self) -> u32 {
        match self {
            UseSite::Inst { block, .. } | UseSite::Terminator { block } => block,
        }
    }
}

/// Definition and uses of every SSA value of one function, indexed by `ValueId.0`.
#[derive(Clone, Debug, Default)]
pub struct ValueDefUse {
    pub defs: Vec<Option<DefPoint>>,
    pub uses: Vec<Vec<UseSite>>,
}

impl ValueDefUse {
    pub fn build(func: &FuncIr) -> ValueDefUse {
        let n = value_count(func);
        let mut defs = def_points(func);
        defs.resize(n, None);
        let mut uses: Vec<Vec<UseSite>> = vec![Vec::new(); n];
        for (b, block) in func.blocks.iter().enumerate() {
            for (i, inst) in block.insts.iter().enumerate() {
                let site = UseSite::Inst {
                    block: b as u32,
                    index: i as u32,
                };
                for_each_inst_use(inst, |v| push_use(&mut uses, v, site));
            }
            let site = UseSite::Terminator { block: b as u32 };
            for_each_term_use(&block.term, |v| push_use(&mut uses, v, site));
        }
        ValueDefUse { defs, uses }
    }

    pub fn def_of(&self, v: ValueId) -> Option<DefPoint> {
        self.defs.get(v.0 as usize).copied().flatten()
    }

    pub fn uses_of(&self, v: ValueId) -> &[UseSite] {
        self.uses.get(v.0 as usize).map_or(&[], |u| u.as_slice())
    }

    /// Values that are read but defined neither by a parameter nor by an instruction of the
    /// function (malformed IR, or values the lowering left to an enclosing function).
    pub fn undefined_uses(&self) -> Vec<ValueId> {
        self.uses
            .iter()
            .enumerate()
            .filter(|(v, u)| !u.is_empty() && self.defs.get(*v).copied().flatten().is_none())
            .map(|(v, _)| ValueId(v as u32))
            .collect()
    }

    /// Values that are defined but never read.
    pub fn unused_defs(&self) -> Vec<ValueId> {
        self.defs
            .iter()
            .enumerate()
            .filter(|(v, d)| d.is_some() && self.uses.get(*v).is_none_or(|u| u.is_empty()))
            .map(|(v, _)| ValueId(v as u32))
            .collect()
    }
}

fn push_use(uses: &mut [Vec<UseSite>], v: ValueId, site: UseSite) {
    if let Some(list) = uses.get_mut(v.0 as usize) {
        if list.last() != Some(&site) {
            list.push(site);
        }
    }
}

// ── Memory locations ─────────────────────────────────────────────────────────

/// Canonical name of a memory location.
///
/// `Local` names a slot whose address is never taken: storage whose address escapes is an
/// `Alloc` in the stack region accessed through `Deref`, so `Local` slots alias only
/// themselves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MemLoc {
    Local(ValueId),
    Field { base: ValueId, field: FieldId },
    /// An element; `index` is the constant index when the IR operand is a constant.
    Element { base: ValueId, index: Option<i128> },
    Global(GlobalId),
    Static { owner: u32, field: FieldId },
    Deref(ValueId),
}

impl MemLoc {
    /// The location of `place`, with SSA bases and indices resolved through copies.
    pub fn from_place(func: &FuncIr, defs: &[Option<DefPoint>], place: &Place) -> MemLoc {
        let canon = |v: ValueId| match resolve_copies(func, defs, &v) {
            ResolvedOperand::Value(r) => r,
            _ => v,
        };
        match place {
            Place::Local(v) => MemLoc::Local(*v),
            Place::Field { base, field } => MemLoc::Field {
                base: canon(*base),
                field: *field,
            },
            Place::Index { base, index } => MemLoc::Element {
                base: canon(*base),
                index: match resolve_copies(func, defs, index) {
                    ResolvedOperand::Const(c) => c.as_int(),
                    _ => None,
                },
            },
            Place::Global(g) => MemLoc::Global(*g),
            Place::Static { owner, field } => MemLoc::Static {
                owner: *owner,
                field: *field,
            },
            Place::Deref(v) => MemLoc::Deref(canon(*v)),
        }
    }

    /// The SSA value the location is named through.
    pub fn base(&self) -> Option<ValueId> {
        match self {
            MemLoc::Local(v) | MemLoc::Deref(v) => Some(*v),
            MemLoc::Field { base, .. } | MemLoc::Element { base, .. } => Some(*base),
            MemLoc::Global(_) | MemLoc::Static { .. } => None,
        }
    }

    /// True when the name denotes exactly one location at any moment, so a write to it can
    /// be a strong update. An element with an unknown index denotes many.
    pub fn is_singular(&self) -> bool {
        !matches!(self, MemLoc::Element { index: None, .. })
    }
}

/// Alias and side-effect knowledge about memory locations, supplied by the memory model
/// (`crate::memory`: points-to sets and mod/ref summaries).
pub trait AliasOracle {
    /// True when `a` and `b` may name the same location.
    fn may_alias(&self, func: &FuncIr, a: &MemLoc, b: &MemLoc) -> bool;

    /// True when every write to `a` certainly writes `b` (a strong update of `a` kills `b`).
    fn must_alias(&self, func: &FuncIr, a: &MemLoc, b: &MemLoc) -> bool;

    /// True when the call at `site` may write `loc`.
    fn call_may_write(&self, func: &FuncIr, site: CallSiteId, callee: &Callee, loc: &MemLoc) -> bool;
}

/// The alias oracle that uses only the IR's own structure.
///
/// `typed_memory` states the memory model of the source language as reported by its
/// frontend: with typed memory (objects with declared fields, no pointer arithmetic, no
/// overlapping storage) distinct fields, distinct constant indices and different kinds of
/// storage never overlap; without it (flat memory with unions and pointer arithmetic) any two
/// heap locations may overlap.
#[derive(Clone, Copy, Debug)]
pub struct SyntacticAlias {
    pub typed_memory: bool,
}

impl AliasOracle for SyntacticAlias {
    fn may_alias(&self, _func: &FuncIr, a: &MemLoc, b: &MemLoc) -> bool {
        use MemLoc::*;
        if a == b {
            return true;
        }
        match (a, b) {
            (Local(_), _) | (_, Local(_)) => false,
            (Deref(_), _) | (_, Deref(_)) => true,
            (Global(_), Global(_)) | (Static { .. }, Static { .. }) => false,
            (Field { field: f1, .. }, Field { field: f2, .. }) => !self.typed_memory || f1 == f2,
            (Element { index: i1, .. }, Element { index: i2, .. }) => {
                !self.typed_memory || !matches!((i1, i2), (Some(x), Some(y)) if x != y)
            }
            _ => !self.typed_memory,
        }
    }

    fn must_alias(&self, _func: &FuncIr, a: &MemLoc, b: &MemLoc) -> bool {
        a == b && a.is_singular()
    }

    fn call_may_write(&self, _func: &FuncIr, _site: CallSiteId, _callee: &Callee, loc: &MemLoc) -> bool {
        !matches!(loc, MemLoc::Local(_))
    }
}

// ── Memory definitions ───────────────────────────────────────────────────────

/// Where the initial content of a location comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InitOrigin {
    /// The location exists on entry (globals, statics, slots, locations of parameters).
    Entry,
    /// The location is named through the value defined at `(block, index)`; `fresh` when that
    /// instruction is an `Alloc` (newly created storage).
    Inst { block: u32, index: u32, fresh: bool },
}

/// A definition of a memory location.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MemDef {
    Store { block: u32, index: u32, loc: MemLoc },
    Release { block: u32, index: u32, loc: MemLoc },
    /// A possible write by the call at `(block, index)`; which locations it may write is
    /// answered by [`AliasOracle::call_may_write`].
    Call { block: u32, index: u32, site: CallSiteId },
    Initial { loc: MemLoc, origin: InitOrigin },
}

impl MemDef {
    pub fn loc(&self) -> Option<&MemLoc> {
        match self {
            MemDef::Store { loc, .. } | MemDef::Release { loc, .. } | MemDef::Initial { loc, .. } => {
                Some(loc)
            }
            MemDef::Call { .. } => None,
        }
    }
}

/// The definitions that may supply the value of one `Load`.
#[derive(Clone, Debug)]
pub struct LoadChain {
    pub block: u32,
    pub index: u32,
    pub loc: MemLoc,
    /// Indices into [`ReachingDefinitions::defs`], ascending.
    pub defs: Vec<u32>,
}

/// Reaching definitions of memory locations for one function.
#[derive(Clone, Debug)]
pub struct ReachingDefinitions {
    pub defs: Vec<MemDef>,
    /// Bits `d` (current instance) and `defs.len() + d` (stale instance) per program point.
    pub result: DataflowResult<BitSet>,
    pub loads: Vec<LoadChain>,
    /// Per definition: the indices into `loads` it reaches.
    pub def_uses: Vec<Vec<u32>>,
    /// True when the function had more distinct locations than the pairwise must-alias
    /// budget, so only identical names were checked for strong updates (fewer kills: still
    /// sound, less precise).
    pub kills_restricted: bool,
}

impl ReachingDefinitions {
    /// The definitions reaching the program point whose state is `state`, current and stale
    /// instances merged.
    pub fn reaching_in(&self, state: &BitSet) -> Vec<u32> {
        let d = self.defs.len();
        let mut out: Vec<u32> = state
            .iter()
            .map(|bit| if bit >= d { bit - d } else { bit } as u32)
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }
}

/// Number of distinct locations up to which every pair is checked for must-alias.
const MAX_PAIRWISE_LOCATIONS: usize = 2048;

struct ReachingTransfer {
    n_defs: usize,
    /// Store / Release / Call definition generated at `(block, index)`.
    inst_def: HashMap<(u32, u32), u32>,
    /// Current-instance kill set of each Store / Release definition.
    kills: HashMap<u32, BitSet>,
    /// Keyed definitions per SSA base value.
    based_on: HashMap<ValueId, BitSet>,
    /// Initial definitions generated by the instruction at `(block, index)`.
    inits_at: HashMap<(u32, u32), Vec<u32>>,
    entry_inits: BitSet,
    defs: Vec<MemDef>,
}

impl ForwardTransfer<BitSet> for ReachingTransfer {
    fn entry_state(&self, _func: &FuncIr, _cfg: &IrCfg) -> BitSet {
        self.entry_inits.clone()
    }

    fn transfer_inst(&self, at: InstPoint<'_>, inst: &Inst, state: &mut BitSet) {
        let key = (at.block, at.index);
        if let Some(&d) = self.inst_def.get(&key) {
            if let Some(kill) = self.kills.get(&d) {
                state.subtract(kill);
            }
            state.insert(d as usize);
        }
        if let Some(v) = inst_def(inst) {
            if let Some(based) = self.based_on.get(&v) {
                let mut moving = state.clone();
                moving.intersect_with(based);
                for d in moving.iter() {
                    state.remove(d);
                    state.insert(self.n_defs + d);
                }
            }
            if let Some(inits) = self.inits_at.get(&key) {
                for &d in inits {
                    state.insert(d as usize);
                }
            }
        }
    }

    fn transfer_unwind(&self, at: InstPoint<'_>, _inst: &Inst, state: &mut BitSet) {
        if let Some(&d) = self.inst_def.get(&(at.block, at.index)) {
            if matches!(self.defs[d as usize], MemDef::Call { .. }) {
                state.insert(d as usize);
            }
        }
    }
}

/// Computes reaching definitions of memory locations and the chain of every `Load`.
pub fn reaching_definitions(
    func: &FuncIr,
    cfg: &IrCfg,
    alias: &dyn AliasOracle,
    options: &SolverOptions,
) -> ReachingDefinitions {
    let vdefs = def_points(func);

    // ── 1. Enumerate Store / Release / Call definitions and every named location ──
    let mut defs: Vec<MemDef> = Vec::new();
    let mut inst_def_map: HashMap<(u32, u32), u32> = HashMap::new();
    let mut keys: Vec<MemLoc> = Vec::new();
    let mut key_index: HashMap<MemLoc, usize> = HashMap::new();
    let mut note_key = |loc: MemLoc, keys: &mut Vec<MemLoc>| {
        key_index.entry(loc).or_insert_with(|| {
            keys.push(loc);
            keys.len() - 1
        });
    };
    let mut load_sites: Vec<(u32, u32, MemLoc)> = Vec::new();
    for (b, block) in func.blocks.iter().enumerate() {
        let (b, reachable) = (b as u32, cfg.is_reachable(b as u32));
        if !reachable {
            continue;
        }
        for (i, inst) in block.insts.iter().enumerate() {
            let i = i as u32;
            match inst {
                Inst::Store { place, .. } => {
                    let loc = MemLoc::from_place(func, &vdefs, place);
                    note_key(loc, &mut keys);
                    inst_def_map.insert((b, i), defs.len() as u32);
                    defs.push(MemDef::Store {
                        block: b,
                        index: i,
                        loc,
                    });
                }
                Inst::Release { place } => {
                    let loc = MemLoc::from_place(func, &vdefs, place);
                    note_key(loc, &mut keys);
                    inst_def_map.insert((b, i), defs.len() as u32);
                    defs.push(MemDef::Release {
                        block: b,
                        index: i,
                        loc,
                    });
                }
                Inst::Call { site, .. } => {
                    inst_def_map.insert((b, i), defs.len() as u32);
                    defs.push(MemDef::Call {
                        block: b,
                        index: i,
                        site: *site,
                    });
                }
                Inst::Load { place, .. } => {
                    let loc = MemLoc::from_place(func, &vdefs, place);
                    note_key(loc, &mut keys);
                    load_sites.push((b, i, loc));
                }
                _ => {}
            }
        }
    }

    // ── 2. One initial definition per location, generated where its base is defined ──
    let mut inits_at: HashMap<(u32, u32), Vec<u32>> = HashMap::new();
    let mut entry_inits = BitSet::new();
    for &loc in &keys {
        let origin = match loc.base().and_then(|v| vdefs.get(v.0 as usize).copied().flatten()) {
            Some(DefPoint::Inst { block, index }) => {
                let fresh = matches!(
                    func.blocks[block as usize].insts.get(index as usize),
                    Some(Inst::Alloc { .. })
                );
                InitOrigin::Inst { block, index, fresh }
            }
            Some(DefPoint::Param(_)) | None => InitOrigin::Entry,
        };
        let d = defs.len() as u32;
        defs.push(MemDef::Initial { loc, origin });
        match origin {
            InitOrigin::Entry => {
                entry_inits.insert(d as usize);
            }
            InitOrigin::Inst { block, index, .. } => inits_at.entry((block, index)).or_default().push(d),
        }
    }
    let n_defs = defs.len();

    // ── 3. Definitions per location and per SSA base ──
    let mut defs_by_key: Vec<BitSet> = vec![BitSet::new(); keys.len()];
    let mut based_on: HashMap<ValueId, BitSet> = HashMap::new();
    for (d, def) in defs.iter().enumerate() {
        if let Some(loc) = def.loc() {
            defs_by_key[key_index[loc]].insert(d);
            if let Some(v) = loc.base() {
                based_on.entry(v).or_default().insert(d);
            }
        }
    }

    // ── 4. Strong-update kill sets ──
    let kills_restricted = keys.len() > MAX_PAIRWISE_LOCATIONS;
    let mut kill_by_key: Vec<BitSet> = vec![BitSet::new(); keys.len()];
    for (k, key) in keys.iter().enumerate() {
        if kills_restricted {
            if alias.must_alias(func, key, key) {
                kill_by_key[k].union_with(&defs_by_key[k]);
            }
        } else {
            for (k2, other) in keys.iter().enumerate() {
                if alias.must_alias(func, key, other) {
                    kill_by_key[k].union_with(&defs_by_key[k2]);
                }
            }
        }
    }
    let mut kills: HashMap<u32, BitSet> = HashMap::new();
    for (d, def) in defs.iter().enumerate() {
        if let MemDef::Store { loc, .. } | MemDef::Release { loc, .. } = def {
            kills.insert(d as u32, kill_by_key[key_index[loc]].clone());
        }
    }

    // ── 5. Solve ──
    let transfer = ReachingTransfer {
        n_defs,
        inst_def: inst_def_map,
        kills,
        based_on,
        inits_at,
        entry_inits,
        defs,
    };
    let result = solve_forward(func, cfg, &transfer, options);

    // ── 6. Load chains ──
    let mut loads: Vec<LoadChain> = Vec::with_capacity(load_sites.len());
    let mut load_at: HashMap<(u32, u32), usize> = HashMap::new();
    for &(b, i, loc) in &load_sites {
        load_at.insert((b, i), loads.len());
        loads.push(LoadChain {
            block: b,
            index: i,
            loc,
            defs: Vec::new(),
        });
    }
    for &b in &cfg.rpo {
        replay_forward(func, cfg, &transfer, &result, b, |at, _inst, state| {
            let Some(&li) = load_at.get(&(at.block, at.index)) else {
                return;
            };
            let loc = loads[li].loc;
            let mut chain: Vec<u32> = Vec::new();
            for bit in state.iter() {
                let d = if bit >= n_defs { bit - n_defs } else { bit };
                let reaches = match &transfer.defs[d] {
                    MemDef::Store { loc: l, .. }
                    | MemDef::Release { loc: l, .. }
                    | MemDef::Initial { loc: l, .. } => alias.may_alias(func, l, &loc),
                    MemDef::Call { block, index, site } => match func.blocks[*block as usize].insts.get(*index as usize) {
                        Some(Inst::Call { callee, .. }) => {
                            alias.call_may_write(func, *site, callee, &loc)
                        }
                        _ => true,
                    },
                };
                if reaches {
                    chain.push(d as u32);
                }
            }
            chain.sort_unstable();
            chain.dedup();
            loads[li].defs = chain;
        });
    }
    let mut def_uses: Vec<Vec<u32>> = vec![Vec::new(); n_defs];
    for (li, load) in loads.iter().enumerate() {
        for &d in &load.defs {
            def_uses[d as usize].push(li as u32);
        }
    }

    ReachingDefinitions {
        defs: transfer.defs,
        result,
        loads,
        def_uses,
        kills_restricted,
    }
}
