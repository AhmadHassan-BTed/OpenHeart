//! Canonical branch-condition atoms.
//!
//! A branch condition is reduced to a *literal* — an atom and a polarity — such that two
//! branches whose conditions are the same pure predicate over the same SSA values get the
//! same atom. SSA identity is what makes this sound: a value has one definition, so equal
//! atoms evaluate equally wherever both are evaluated within one dynamic instance of their
//! operands (the invariant the ROBDD feasibility filter of `crate::psa` relies on).
//!
//! Canonicalisation, applied to the branch operand:
//!
//! 1. **Copies** are followed to the producing value (`Assign` chains).
//! 2. **Boolean negation** (`Not` of a Boolean) flips the polarity; `b == true`, `b != false`
//!    reduce to `b`, `b == false`, `b != true` to `¬b`.
//! 3. **Orientation.** A constant operand goes to the right; two values are ordered by id.
//!    Mirroring the operands mirrors the predicate (`a > b ≡ b < a`).
//! 4. **Negation normal form.** `≠` is `¬(=)` in every domain. In a totally ordered domain
//!    (integers, Booleans, characters, references) `≥ ≡ ¬(<)` and `> ≡ ¬(≤)`, and against an
//!    integer constant `x ≤ c ≡ x < c + 1`, so `x < 5`, `x >= 5`, `5 > x` and `x <= 4` share
//!    one atom. Floating-point comparisons are not negated (`¬(a < b)` is not `a ≥ b` when an
//!    operand is NaN), and comparisons whose operand domain is unknown are treated like
//!    floating point.
//! 5. **Constant folding** of comparisons between constants.
//!
//! Anything else — a Boolean parameter, a loaded flag, a call result — becomes the atom
//! "this SSA value is true".
//!
//! **Guards.** [`FunctionConditions::block_guard`] combines atoms with control dependence:
//! a block executes only if one of the edges it depends on is taken and that edge's source
//! executes, which yields a disjunctive normal form over branch literals.

use std::collections::HashMap;

use crate::analysis::controldep::ControlDependence;
use crate::analysis::solver::{
    def_points, defining_inst, resolve_copies, CmpOp, ConstKey, DefPoint, EdgeKind, IrCfg,
    OperandView, ResolvedOperand, ScalarKind, TypeFacts, UnaryOp,
};
use crate::ir::{FuncIr, Inst, Terminator, TypeId, ValueId};
use crate::psa::construction::feasibility::{BranchPredicate, BranchPredicateTable};

// ── Atoms ────────────────────────────────────────────────────────────────────

/// Dense id of an atom within one function.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AtomId(pub u32);

/// An operand of a comparison atom.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AtomOperand {
    Value(ValueId),
    Const(ConstKey),
}

/// The order structure of a comparison's operands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CmpDomain {
    /// Integers, Booleans, characters, references: a total order (or plain equality).
    Total,
    /// Floating point: unordered operands make every ordered comparison false.
    Float,
    /// Not known; treated like floating point.
    Unknown,
}

/// A canonical predicate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Atom {
    /// The Boolean SSA value is true.
    Truth(ValueId),
    /// `lhs op rhs`, oriented and normalised as described in the module documentation.
    Compare {
        op: CmpOp,
        lhs: AtomOperand,
        rhs: AtomOperand,
        domain: CmpDomain,
    },
    /// The value's dynamic type passes the test against `ty`.
    TypeTest { value: ValueId, ty: TypeId },
}

impl Atom {
    /// The SSA values the atom reads.
    pub fn values(&self) -> Vec<ValueId> {
        match self {
            Atom::Truth(v) | Atom::TypeTest { value: v, .. } => vec![*v],
            Atom::Compare { lhs, rhs, .. } => [lhs, rhs]
                .iter()
                .filter_map(|o| match o {
                    AtomOperand::Value(v) => Some(*v),
                    AtomOperand::Const(_) => None,
                })
                .collect(),
        }
    }
}

/// A reduced condition.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Cond {
    /// The condition is a constant.
    Const(bool),
    /// The condition holds iff `atom` has truth value `positive`.
    Literal { atom: AtomId, positive: bool },
    /// The condition operand is `Undef` or not a Boolean the IR defines; no atom exists.
    Opaque,
}

impl Cond {
    pub fn negated(self) -> Cond {
        match self {
            Cond::Const(b) => Cond::Const(!b),
            Cond::Literal { atom, positive } => Cond::Literal {
                atom,
                positive: !positive,
            },
            Cond::Opaque => Cond::Opaque,
        }
    }
}

/// One literal of a guard.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Literal {
    /// `atom` has truth value `positive`.
    Atom { atom: AtomId, positive: bool },
    /// The call at instruction `inst` of block `block` throws.
    Throws { block: u32, inst: u32 },
}

/// A guard in disjunctive normal form: the block may execute only if some term holds.
/// `terms == [[]]` is `true`, `terms == []` is `false`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Guard {
    pub terms: Vec<Vec<Literal>>,
    /// False when the guard was weakened (term budget, depth budget, opaque conditions):
    /// it is then a necessary condition, not an equivalent one.
    pub exact: bool,
}

impl Guard {
    pub fn always() -> Guard {
        Guard {
            terms: vec![Vec::new()],
            exact: true,
        }
    }

    pub fn never() -> Guard {
        Guard {
            terms: Vec::new(),
            exact: true,
        }
    }

    pub fn is_always(&self) -> bool {
        self.terms.iter().any(|t| t.is_empty())
    }

    pub fn is_never(&self) -> bool {
        self.terms.is_empty()
    }
}

/// The test performed by a block's terminator.
#[derive(Clone, Debug)]
pub enum BlockTest {
    /// A two-way `Branch`.
    Branch {
        cond: Cond,
        true_edge: Option<u32>,
        false_edge: Option<u32>,
    },
    /// A `Switch`: case edge `edge` is taken iff `atom` (`scrutinee == constant`) holds.
    Switch {
        scrutinee: AtomOperand,
        cases: Vec<(u32, Cond)>,
        default_edge: Option<u32>,
    },
}

/// Canonical atoms and branch tests of one function.
#[derive(Clone, Debug, Default)]
pub struct FunctionConditions {
    pub atoms: Vec<Atom>,
    index: HashMap<Atom, AtomId>,
    /// Per block position: the test of its terminator, when it branches.
    pub tests: Vec<Option<BlockTest>>,
}

/// Depth bound of the control-dependence walk behind a guard.
const MAX_GUARD_DEPTH: usize = 256;
/// Term bound of a guard; larger guards are weakened to `true`.
const MAX_GUARD_TERMS: usize = 64;

impl FunctionConditions {
    /// Canonicalises the condition of every `Branch` and the cases of every `Switch`.
    pub fn compute(func: &FuncIr, cfg: &IrCfg, types: &dyn TypeFacts) -> FunctionConditions {
        let defs = def_points(func);
        let mut fc = FunctionConditions {
            atoms: Vec::new(),
            index: HashMap::new(),
            tests: vec![None; cfg.len()],
        };
        let canon = Canonicaliser {
            func,
            defs: &defs,
            types,
        };
        for (b, block) in func.blocks.iter().enumerate() {
            let edge_of = |kind: EdgeKind| {
                cfg.succ_edges[b]
                    .iter()
                    .copied()
                    .find(|&e| cfg.edges[e as usize].kind == kind)
            };
            match &block.term {
                Terminator::Branch { cond, .. } => {
                    let c = canon.condition(cond, &mut fc);
                    fc.tests[b] = Some(BlockTest::Branch {
                        cond: c,
                        true_edge: edge_of(EdgeKind::True),
                        false_edge: edge_of(EdgeKind::False),
                    });
                }
                Terminator::Switch {
                    scrutinee, cases, ..
                } => {
                    let scrut = match resolve_copies(func, &defs, scrutinee) {
                        ResolvedOperand::Value(v) => Some(AtomOperand::Value(v)),
                        ResolvedOperand::Const(c) => Some(AtomOperand::Const(c)),
                        ResolvedOperand::Undef => None,
                    };
                    let Some(scrut) = scrut else {
                        continue;
                    };
                    let mut case_conds = Vec::with_capacity(cases.len());
                    for (k, (value, _)) in cases.iter().enumerate() {
                        let Some(e) = edge_of(EdgeKind::Case { index: k as u32 }) else {
                            continue;
                        };
                        let c = canon.compare(
                            CmpOp::Eq,
                            scrut,
                            AtomOperand::Const(ConstKey::from_ir(value)),
                            true,
                            &mut fc,
                        );
                        case_conds.push((e, c));
                    }
                    fc.tests[b] = Some(BlockTest::Switch {
                        scrutinee: scrut,
                        cases: case_conds,
                        default_edge: edge_of(EdgeKind::Default),
                    });
                }
                _ => {}
            }
        }
        fc
    }

    /// The id of `atom`, interning it on first sight.
    pub fn intern(&mut self, atom: Atom) -> AtomId {
        if let Some(&id) = self.index.get(&atom) {
            return id;
        }
        let id = AtomId(self.atoms.len() as u32);
        self.atoms.push(atom);
        self.index.insert(atom, id);
        id
    }

    /// The id of an already interned atom.
    pub fn lookup(&self, atom: &Atom) -> Option<AtomId> {
        self.index.get(atom).copied()
    }

    pub fn atom(&self, id: AtomId) -> &Atom {
        &self.atoms[id.0 as usize]
    }

    /// The condition of block `block`'s `Branch`, when it has one.
    pub fn branch_condition(&self, block: u32) -> Option<Cond> {
        match self.tests.get(block as usize)? {
            Some(BlockTest::Branch { cond, .. }) => Some(*cond),
            _ => None,
        }
    }

    /// The conjunction of literals that holds whenever `edge` is taken, or `None` when the
    /// edge can never be taken (its condition is constantly false). Opaque conditions
    /// contribute no literal.
    pub fn edge_literals(&self, cfg: &IrCfg, edge: u32) -> Option<Vec<Literal>> {
        let e = cfg.edge(edge);
        let test = self.tests.get(e.from as usize).and_then(|t| t.as_ref());
        let cond_lits = |c: Cond| -> Option<Vec<Literal>> {
            match c {
                Cond::Const(true) | Cond::Opaque => Some(Vec::new()),
                Cond::Const(false) => None,
                Cond::Literal { atom, positive } => Some(vec![Literal::Atom { atom, positive }]),
            }
        };
        match (e.kind, test) {
            (EdgeKind::Unwind { inst }, _) => Some(vec![Literal::Throws {
                block: e.from,
                inst,
            }]),
            (EdgeKind::True, Some(BlockTest::Branch { cond, .. })) => cond_lits(*cond),
            (EdgeKind::False, Some(BlockTest::Branch { cond, .. })) => cond_lits(cond.negated()),
            (EdgeKind::Case { .. }, Some(BlockTest::Switch { cases, .. })) => {
                match cases.iter().find(|(ce, _)| *ce == edge) {
                    Some((_, c)) => cond_lits(*c),
                    None => Some(Vec::new()),
                }
            }
            (EdgeKind::Default, Some(BlockTest::Switch { cases, .. })) => {
                let mut lits = Vec::new();
                for (_, c) in cases {
                    match cond_lits(c.negated()) {
                        Some(mut l) => lits.append(&mut l),
                        None => return None,
                    }
                }
                lits.sort_unstable();
                lits.dedup();
                Some(lits)
            }
            _ => Some(Vec::new()),
        }
    }

    /// The execution condition of `block` in disjunctive normal form, derived from its
    /// control dependences. Dependences that lead back into the walk (loop-carried control)
    /// are skipped: re-executing a block requires having entered it the first time.
    pub fn block_guard(&self, cfg: &IrCfg, cd: &ControlDependence, block: u32) -> Guard {
        let mut memo: HashMap<u32, Guard> = HashMap::new();
        let mut on_stack = vec![false; cfg.len()];
        self.guard_rec(cfg, cd, block, 0, &mut on_stack, &mut memo)
    }

    fn guard_rec(
        &self,
        cfg: &IrCfg,
        cd: &ControlDependence,
        block: u32,
        depth: usize,
        on_stack: &mut [bool],
        memo: &mut HashMap<u32, Guard>,
    ) -> Guard {
        if let Some(g) = memo.get(&block) {
            return g.clone();
        }
        let deps = cd.deps_of(block);
        if deps.is_empty() {
            return Guard::always();
        }
        if depth >= MAX_GUARD_DEPTH {
            return Guard {
                terms: vec![Vec::new()],
                exact: false,
            };
        }
        on_stack[block as usize] = true;
        let mut result = Guard::never();
        let mut any_acyclic = false;
        let mut touched_cycle = false;
        for dep in deps {
            if on_stack[dep.branch as usize] {
                touched_cycle = true;
                continue;
            }
            any_acyclic = true;
            let Some(lits) = self.edge_literals(cfg, dep.edge) else {
                continue;
            };
            let parent = self.guard_rec(cfg, cd, dep.branch, depth + 1, on_stack, memo);
            result.exact &= parent.exact;
            for term in &parent.terms {
                if let Some(t) = conjoin(term, &lits) {
                    if !result.terms.contains(&t) {
                        result.terms.push(t);
                    }
                }
            }
            if result.terms.len() > MAX_GUARD_TERMS {
                result = Guard {
                    terms: vec![Vec::new()],
                    exact: false,
                };
                break;
            }
        }
        on_stack[block as usize] = false;
        if !any_acyclic {
            result = Guard::always();
        }
        if result.terms.iter().any(|t| t.is_empty()) {
            result.terms = vec![Vec::new()];
        }
        if !touched_cycle {
            memo.insert(block, result.clone());
        }
        result
    }

    /// Branch predicates for the ROBDD feasibility filter, keyed by IR block id.
    pub fn branch_predicates(&self, cfg: &IrCfg) -> Vec<BranchPredicate> {
        let mut out = Vec::new();
        for (b, test) in self.tests.iter().enumerate() {
            if let Some(BlockTest::Branch {
                cond: Cond::Literal { atom, positive },
                ..
            }) = test
            {
                out.push(BranchPredicate {
                    block_id: cfg.block_id(b as u32).0,
                    predicate_id: atom.0,
                    true_edge_when: *positive,
                });
            }
        }
        out
    }

    /// Adds this function's branch predicates to `table` under `sym_id`.
    pub fn fill_predicate_table(&self, cfg: &IrCfg, sym_id: u32, table: &mut BranchPredicateTable) {
        for p in self.branch_predicates(cfg) {
            table.insert(sym_id, p);
        }
    }
}

/// `term ∧ lits`, or `None` when the conjunction contains complementary atom literals.
fn conjoin(term: &[Literal], lits: &[Literal]) -> Option<Vec<Literal>> {
    let mut out: Vec<Literal> = term.to_vec();
    for &l in lits {
        if let Literal::Atom { atom, positive } = l {
            if out.contains(&Literal::Atom {
                atom,
                positive: !positive,
            }) {
                return None;
            }
        }
        if !out.contains(&l) {
            out.push(l);
        }
    }
    out.sort_unstable();
    Some(out)
}

// ── Canonicalisation ─────────────────────────────────────────────────────────

struct Canonicaliser<'a> {
    func: &'a FuncIr,
    defs: &'a [Option<DefPoint>],
    types: &'a dyn TypeFacts,
}

impl Canonicaliser<'_> {
    fn resolve<O: OperandView + ?Sized>(&self, op: &O) -> ResolvedOperand {
        resolve_copies(self.func, self.defs, op)
    }

    /// True when `v` is known to be Boolean.
    fn is_boolean(&self, v: ValueId) -> bool {
        match defining_inst(self.func, self.defs, v) {
            Some(Inst::Compare { .. }) | Some(Inst::TypeTest { .. }) => true,
            _ => self.types.value_kind(self.func, v) == ScalarKind::Bool,
        }
    }

    fn operand_domain(&self, o: AtomOperand) -> CmpDomain {
        match o {
            AtomOperand::Const(ConstKey::Float(_)) => CmpDomain::Float,
            AtomOperand::Const(_) => CmpDomain::Total,
            AtomOperand::Value(v) => match self.types.value_kind(self.func, v) {
                ScalarKind::Float { .. } => CmpDomain::Float,
                ScalarKind::Unknown | ScalarKind::Void | ScalarKind::Aggregate => {
                    if self.is_boolean(v) {
                        CmpDomain::Total
                    } else {
                        CmpDomain::Unknown
                    }
                }
                _ => CmpDomain::Total,
            },
        }
    }

    /// Reduces a branch operand to a condition.
    fn condition<O: OperandView + ?Sized>(&self, op: &O, fc: &mut FunctionConditions) -> Cond {
        let mut positive = true;
        let mut cur = self.resolve(op);
        for _ in 0..self.defs.len().max(1) {
            match cur {
                ResolvedOperand::Const(ConstKey::Bool(b)) => return Cond::Const(b == positive),
                ResolvedOperand::Const(_) | ResolvedOperand::Undef => return Cond::Opaque,
                ResolvedOperand::Value(v) => match defining_inst(self.func, self.defs, v) {
                    Some(Inst::Unary { op: uop, src, .. })
                        if UnaryOp::from_ir(uop) == Some(UnaryOp::Not) =>
                    {
                        let inner = self.resolve(src);
                        let boolean = match inner {
                            ResolvedOperand::Value(x) => self.is_boolean(x),
                            ResolvedOperand::Const(ConstKey::Bool(_)) => true,
                            _ => false,
                        };
                        if boolean || self.types.value_kind(self.func, v) == ScalarKind::Bool {
                            positive = !positive;
                            cur = inner;
                        } else {
                            return self.truth(v, positive, fc);
                        }
                    }
                    Some(Inst::Compare { pred, lhs, rhs, .. }) => {
                        let Some(op) = CmpOp::from_ir(pred) else {
                            return self.truth(v, positive, fc);
                        };
                        let l = self.resolve(lhs);
                        let r = self.resolve(rhs);
                        let (l, r) = match (atom_operand(l), atom_operand(r)) {
                            (Some(l), Some(r)) => (l, r),
                            _ => return self.truth(v, positive, fc),
                        };
                        // `b == true`, `b != false`, … over a Boolean value: continue with b.
                        if let Some((x, flip)) = boolean_constant_test(op, l, r) {
                            if self.is_boolean(x) {
                                if flip {
                                    positive = !positive;
                                }
                                cur = ResolvedOperand::Value(x);
                                continue;
                            }
                        }
                        return self.compare(op, l, r, positive, fc);
                    }
                    Some(Inst::TypeTest { src, ty, .. }) => {
                        return match self.resolve(src) {
                            ResolvedOperand::Value(x) => {
                                let atom = fc.intern(Atom::TypeTest { value: x, ty: *ty });
                                Cond::Literal { atom, positive }
                            }
                            _ => self.truth(v, positive, fc),
                        };
                    }
                    _ => return self.truth(v, positive, fc),
                },
            }
        }
        Cond::Opaque
    }

    fn truth(&self, v: ValueId, positive: bool, fc: &mut FunctionConditions) -> Cond {
        Cond::Literal {
            atom: fc.intern(Atom::Truth(v)),
            positive,
        }
    }

    /// Canonical literal of `positive ⇔ (l op r)`.
    fn compare(
        &self,
        op: CmpOp,
        l: AtomOperand,
        r: AtomOperand,
        positive: bool,
        fc: &mut FunctionConditions,
    ) -> Cond {
        if let (AtomOperand::Const(a), AtomOperand::Const(b)) = (l, r) {
            return match fold_const_compare(op, a, b) {
                Some(v) => Cond::Const(v == positive),
                None => Cond::Opaque,
            };
        }
        // Orientation: constant right, smaller value id left.
        let (mut op, mut l, mut r) = (op, l, r);
        let swap = match (l, r) {
            (AtomOperand::Const(_), AtomOperand::Value(_)) => true,
            (AtomOperand::Value(a), AtomOperand::Value(b)) => b < a,
            _ => false,
        };
        if swap {
            std::mem::swap(&mut l, &mut r);
            op = op.swap();
        }
        let domain = match (self.operand_domain(l), self.operand_domain(r)) {
            (CmpDomain::Float, _) | (_, CmpDomain::Float) => CmpDomain::Float,
            (CmpDomain::Total, _) | (_, CmpDomain::Total) => CmpDomain::Total,
            _ => CmpDomain::Unknown,
        };
        let mut positive = positive;
        if op == CmpOp::Ne {
            op = CmpOp::Eq;
            positive = !positive;
        }
        if domain == CmpDomain::Total {
            match op {
                CmpOp::Ge => {
                    op = CmpOp::Lt;
                    positive = !positive;
                }
                CmpOp::Gt => {
                    op = CmpOp::Le;
                    positive = !positive;
                }
                _ => {}
            }
            if op == CmpOp::Le {
                if let AtomOperand::Const(ConstKey::Int(c)) = r {
                    if let Some(c1) = c.checked_add(1) {
                        op = CmpOp::Lt;
                        r = AtomOperand::Const(ConstKey::Int(c1));
                    }
                }
            }
        }
        let atom = fc.intern(Atom::Compare {
            op,
            lhs: l,
            rhs: r,
            domain,
        });
        Cond::Literal { atom, positive }
    }
}

fn atom_operand(r: ResolvedOperand) -> Option<AtomOperand> {
    match r {
        ResolvedOperand::Value(v) => Some(AtomOperand::Value(v)),
        ResolvedOperand::Const(c) => Some(AtomOperand::Const(c)),
        ResolvedOperand::Undef => None,
    }
}

/// For `x op c` / `c op x` with a Boolean constant `c` and `op ∈ {=, ≠}`: the value `x` and
/// whether the test negates it.
fn boolean_constant_test(op: CmpOp, l: AtomOperand, r: AtomOperand) -> Option<(ValueId, bool)> {
    let (x, c) = match (l, r) {
        (AtomOperand::Value(x), AtomOperand::Const(ConstKey::Bool(c)))
        | (AtomOperand::Const(ConstKey::Bool(c)), AtomOperand::Value(x)) => (x, c),
        _ => return None,
    };
    match op {
        CmpOp::Eq => Some((x, !c)),
        CmpOp::Ne => Some((x, c)),
        _ => None,
    }
}

/// Folds a comparison of two constants whose result does not depend on the language:
/// numbers and Booleans by value, `null` against `null` or a string literal by identity.
/// String literals against each other are not folded (identity versus content equality is a
/// language property).
pub fn fold_const_compare(op: CmpOp, a: ConstKey, b: ConstKey) -> Option<bool> {
    match (a, b) {
        (ConstKey::Int(x), ConstKey::Int(y)) => Some(op.eval(Some(x.cmp(&y)))),
        (ConstKey::Bool(x), ConstKey::Bool(y)) => Some(op.eval(Some(x.cmp(&y)))),
        (ConstKey::Float(x), ConstKey::Float(y)) => {
            Some(op.eval(f64::from_bits(x).partial_cmp(&f64::from_bits(y))))
        }
        (ConstKey::Null, ConstKey::Null) => match op {
            CmpOp::Eq => Some(true),
            CmpOp::Ne => Some(false),
            _ => None,
        },
        (ConstKey::Null, ConstKey::Str(_)) | (ConstKey::Str(_), ConstKey::Null) => match op {
            CmpOp::Eq => Some(false),
            CmpOp::Ne => Some(true),
            _ => None,
        },
        _ => None,
    }
}
