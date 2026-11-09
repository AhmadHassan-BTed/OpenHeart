//! In-house decision procedure for symbolic path feasibility.
//!
//! A path condition is a conjunction of canonical [`Atom`]s over integer, Boolean and
//! reference terms. [`Solver::check`] decides it with a layered procedure:
//!
//! ```text
//! 1. closure       atoms ∪ definitions(vars) ∪ type ranges(vars), canonicalized
//!                  (coefficients divided by their gcd, ≤-constants tightened to integers)
//! 2. congruence    union-find over x = y, x = c and reference equalities; a class holding
//!                  two distinct constants, or a disequality inside one class ⇒ Unsat
//! 3. substitution  equalities with a unit coefficient (x = e) are eliminated exactly
//! 4. projection    bounded Fourier–Motzkin over the remaining ≤-system; 0 ≤ −k ⇒ Unsat
//! 5. model         back-substitution in reverse elimination order picks integer values;
//!                  a violated disequality e ≠ 0 splits into e ≤ −1 ∨ e ≥ 1 (bounded)
//! 6. verification  every atom and every computed term is re-evaluated under the model
//! ```
//!
//! **Soundness.** Steps 2–4 only derive consequences that hold over the integers (rational
//! infeasibility implies integer infeasibility, and gcd tightening is valid for integer
//! variables), so `Unsat` is never claimed for a satisfiable condition. `Sat` is claimed only
//! with a model that step 6 verified against every atom. Everything else — a resource limit,
//! an integer gap the projection cannot rule out, an uninterpreted term whose value cannot be
//! computed — is [`SatResult::Unknown`] with its reason; it is never rounded to either answer.
//!
//! **Terms.** The solver does not know what a [`Var`] stands for. A [`TermOracle`] (the
//! symbolic expression arena of `symex::state`) describes each variable: its value range,
//! exact definitional constraints (e.g. the wrap-around encoding `t = x + y − 2ʷ·q` of a
//! fixed-width addition), and whether its value can be recomputed from its operands.

use std::collections::{BTreeMap, HashMap, HashSet};

/// A solver variable. Its meaning is supplied by a [`TermOracle`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Var(pub u32);

/// `Σ coeff·var + constant` with terms sorted by variable and no zero coefficient.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LinExpr {
    pub terms: Vec<(Var, i128)>,
    pub constant: i128,
}

impl LinExpr {
    /// The constant expression `c`.
    pub fn constant(c: i128) -> Self {
        Self {
            terms: Vec::new(),
            constant: c,
        }
    }

    /// The expression `v`.
    pub fn var(v: Var) -> Self {
        Self {
            terms: vec![(v, 1)],
            constant: 0,
        }
    }

    /// The expression `coeff·v`.
    pub fn term(v: Var, coeff: i128) -> Self {
        if coeff == 0 {
            return Self::constant(0);
        }
        Self {
            terms: vec![(v, coeff)],
            constant: 0,
        }
    }

    pub fn is_constant(&self) -> bool {
        self.terms.is_empty()
    }

    /// The coefficient of `v` (0 when absent).
    pub fn coeff(&self, v: Var) -> i128 {
        match self.terms.binary_search_by_key(&v, |&(x, _)| x) {
            Ok(i) => self.terms[i].1,
            Err(_) => 0,
        }
    }

    pub fn vars(&self) -> impl Iterator<Item = Var> + '_ {
        self.terms.iter().map(|&(v, _)| v)
    }

    /// `self + other`, `None` on i128 overflow.
    pub fn checked_add(&self, other: &LinExpr) -> Option<LinExpr> {
        let mut terms = Vec::with_capacity(self.terms.len() + other.terms.len());
        let (mut i, mut j) = (0, 0);
        while i < self.terms.len() || j < other.terms.len() {
            let pick = match (self.terms.get(i), other.terms.get(j)) {
                (Some(&(a, ca)), Some(&(b, cb))) if a == b => {
                    i += 1;
                    j += 1;
                    (a, ca.checked_add(cb)?)
                }
                (Some(&(a, ca)), Some(&(b, _))) if a < b => {
                    i += 1;
                    (a, ca)
                }
                (Some(_), Some(&(b, cb))) => {
                    j += 1;
                    (b, cb)
                }
                (Some(&(a, ca)), None) => {
                    i += 1;
                    (a, ca)
                }
                (None, Some(&(b, cb))) => {
                    j += 1;
                    (b, cb)
                }
                (None, None) => break,
            };
            if pick.1 != 0 {
                terms.push(pick);
            }
        }
        Some(LinExpr {
            terms,
            constant: self.constant.checked_add(other.constant)?,
        })
    }

    /// `k·self`, `None` on overflow.
    pub fn checked_scale(&self, k: i128) -> Option<LinExpr> {
        if k == 0 {
            return Some(LinExpr::constant(0));
        }
        let mut terms = Vec::with_capacity(self.terms.len());
        for &(v, c) in &self.terms {
            terms.push((v, c.checked_mul(k)?));
        }
        Some(LinExpr {
            terms,
            constant: self.constant.checked_mul(k)?,
        })
    }

    /// `self − other`, `None` on overflow.
    pub fn checked_sub(&self, other: &LinExpr) -> Option<LinExpr> {
        self.checked_add(&other.checked_scale(-1)?)
    }

    /// `self + c`, `None` on overflow.
    pub fn checked_add_const(&self, c: i128) -> Option<LinExpr> {
        Some(LinExpr {
            terms: self.terms.clone(),
            constant: self.constant.checked_add(c)?,
        })
    }

    /// `self[v := with]`, `None` on overflow.
    pub fn substitute(&self, v: Var, with: &LinExpr) -> Option<LinExpr> {
        let c = self.coeff(v);
        if c == 0 {
            return Some(self.clone());
        }
        let rest = LinExpr {
            terms: self.terms.iter().copied().filter(|&(x, _)| x != v).collect(),
            constant: self.constant,
        };
        rest.checked_add(&with.checked_scale(c)?)
    }

    /// The value under `value`; `None` when a variable is unassigned or on overflow.
    pub fn eval(&self, value: &dyn Fn(Var) -> Option<i128>) -> Option<i128> {
        let mut acc = self.constant;
        for &(v, c) in &self.terms {
            acc = acc.checked_add(c.checked_mul(value(v)?)?)?;
        }
        Some(acc)
    }

    /// gcd of the coefficients (0 for a constant expression).
    fn content(&self) -> i128 {
        self.terms.iter().fold(0, |g, &(_, c)| gcd(g, c))
    }

    fn divided_terms(&self, g: i128) -> Vec<(Var, i128)> {
        self.terms.iter().map(|&(v, c)| (v, c / g)).collect()
    }
}

/// A reference-valued term. `Null` and the integer 0 are the same value; distinct
/// `Object`s are distinct from each other and from every integer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RefTerm {
    Var(Var),
    Null,
    Object(u32),
}

/// A canonical path-condition atom.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Atom {
    /// `e = 0`.
    Eq(LinExpr),
    /// `e ≠ 0`.
    Ne(LinExpr),
    /// `e ≤ 0`.
    Le(LinExpr),
    /// Two references denote the same object (or are both null).
    RefEq(RefTerm, RefTerm),
    /// Two references denote different objects.
    RefNe(RefTerm, RefTerm),
    /// An uninterpreted Boolean term has the given truth value.
    Holds(Var, bool),
    /// A decided atom.
    Const(bool),
}

impl Atom {
    /// `a = b`.
    pub fn eq(a: &LinExpr, b: &LinExpr) -> Option<Atom> {
        Some(Atom::Eq(a.checked_sub(b)?).canonical())
    }

    /// `a ≠ b`.
    pub fn ne(a: &LinExpr, b: &LinExpr) -> Option<Atom> {
        Some(Atom::Ne(a.checked_sub(b)?).canonical())
    }

    /// `a ≤ b`.
    pub fn le(a: &LinExpr, b: &LinExpr) -> Option<Atom> {
        Some(Atom::Le(a.checked_sub(b)?).canonical())
    }

    /// `a < b`, i.e. `a − b + 1 ≤ 0` over the integers.
    pub fn lt(a: &LinExpr, b: &LinExpr) -> Option<Atom> {
        Some(Atom::Le(a.checked_sub(b)?.checked_add_const(1)?).canonical())
    }

    /// The logical negation; `None` on coefficient overflow.
    pub fn negate(&self) -> Option<Atom> {
        Some(match self {
            Atom::Eq(e) => Atom::Ne(e.clone()),
            Atom::Ne(e) => Atom::Eq(e.clone()),
            // ¬(e ≤ 0) ⇔ e ≥ 1 ⇔ −e + 1 ≤ 0
            Atom::Le(e) => Atom::Le(e.checked_scale(-1)?.checked_add_const(1)?).canonical(),
            Atom::RefEq(a, b) => Atom::RefNe(*a, *b),
            Atom::RefNe(a, b) => Atom::RefEq(*a, *b),
            Atom::Holds(v, b) => Atom::Holds(*v, !b),
            Atom::Const(b) => Atom::Const(!b),
        })
    }

    /// Normal form: decided atoms fold to `Const`, coefficients are divided by their gcd,
    /// `≤`-constants are rounded up (valid over the integers), and (dis)equalities have a
    /// positive leading coefficient.
    pub fn canonical(self) -> Atom {
        match self {
            Atom::Le(e) => {
                if e.is_constant() {
                    return Atom::Const(e.constant <= 0);
                }
                let g = e.content();
                Atom::Le(LinExpr {
                    terms: e.divided_terms(g),
                    constant: ceil_div(e.constant, g),
                })
            }
            Atom::Eq(e) => {
                if e.is_constant() {
                    return Atom::Const(e.constant == 0);
                }
                let g = e.content();
                if e.constant % g != 0 {
                    return Atom::Const(false);
                }
                Atom::Eq(sign_normalized(LinExpr {
                    terms: e.divided_terms(g),
                    constant: e.constant / g,
                }))
            }
            Atom::Ne(e) => {
                if e.is_constant() {
                    return Atom::Const(e.constant != 0);
                }
                let g = e.content();
                if e.constant % g != 0 {
                    return Atom::Const(true);
                }
                Atom::Ne(sign_normalized(LinExpr {
                    terms: e.divided_terms(g),
                    constant: e.constant / g,
                }))
            }
            Atom::RefEq(a, b) => match ref_decided(a, b) {
                Some(same) => Atom::Const(same),
                None => {
                    let (x, y) = if a <= b { (a, b) } else { (b, a) };
                    Atom::RefEq(x, y)
                }
            },
            Atom::RefNe(a, b) => match ref_decided(a, b) {
                Some(same) => Atom::Const(!same),
                None => {
                    let (x, y) = if a <= b { (a, b) } else { (b, a) };
                    Atom::RefNe(x, y)
                }
            },
            other => other,
        }
    }

    /// The variables the atom mentions.
    pub fn vars(&self) -> Vec<Var> {
        match self {
            Atom::Eq(e) | Atom::Ne(e) | Atom::Le(e) => e.vars().collect(),
            Atom::RefEq(a, b) | Atom::RefNe(a, b) => {
                let mut out = Vec::new();
                for t in [a, b] {
                    if let RefTerm::Var(v) = t {
                        out.push(*v);
                    }
                }
                out
            }
            Atom::Holds(v, _) => vec![*v],
            Atom::Const(_) => Vec::new(),
        }
    }
}

/// Equality of two reference terms when it is decided without variables.
fn ref_decided(a: RefTerm, b: RefTerm) -> Option<bool> {
    if a == b {
        return Some(true);
    }
    match (a, b) {
        (RefTerm::Var(_), _) | (_, RefTerm::Var(_)) => None,
        _ => Some(false),
    }
}

fn sign_normalized(e: LinExpr) -> LinExpr {
    match e.terms.first() {
        Some(&(_, c)) if c < 0 => LinExpr {
            terms: e.terms.iter().map(|&(v, c)| (v, -c)).collect(),
            constant: -e.constant,
        },
        _ => e,
    }
}

fn gcd(a: i128, b: i128) -> i128 {
    let (mut a, mut b) = (a.unsigned_abs(), b.unsigned_abs());
    while b != 0 {
        let t = a % b;
        a = b;
        b = t;
    }
    a.min(i128::MAX as u128) as i128
}

/// ⌊a / b⌋ for b > 0.
pub fn floor_div(a: i128, b: i128) -> i128 {
    let q = a / b;
    if (a % b != 0) && ((a < 0) != (b < 0)) {
        q - 1
    } else {
        q
    }
}

/// ⌈a / b⌉ for b > 0.
pub fn ceil_div(a: i128, b: i128) -> i128 {
    let q = a / b;
    if (a % b != 0) && ((a < 0) == (b < 0)) {
        q + 1
    } else {
        q
    }
}

// ── Term descriptions ────────────────────────────────────────────────────────

/// The value range of a variable (`None` = unbounded on that side).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Domain {
    pub lo: Option<i128>,
    pub hi: Option<i128>,
}

impl Domain {
    pub const UNBOUNDED: Domain = Domain { lo: None, hi: None };

    pub fn range(lo: i128, hi: i128) -> Self {
        Self {
            lo: Some(lo),
            hi: Some(hi),
        }
    }

    pub fn contains(&self, x: i128) -> bool {
        self.lo.map_or(true, |l| x >= l) && self.hi.map_or(true, |h| x <= h)
    }

    /// The value of the domain closest to 0.
    pub fn default_value(&self) -> i128 {
        match (self.lo, self.hi) {
            (Some(l), _) if l > 0 => l,
            (_, Some(h)) if h < 0 => h,
            _ => 0,
        }
    }
}

/// How a variable's value is determined.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TermKind {
    /// An input: any value of its domain.
    Free,
    /// Fully captured by its definition atoms (exact linear encoding).
    Defined,
    /// A function of its operands that the oracle can evaluate under a model.
    Computed,
    /// Uninterpreted: neither encoded nor evaluable.
    Opaque,
}

/// What the oracle knows about one variable.
#[derive(Clone, Debug)]
pub struct TermInfo {
    pub domain: Domain,
    pub kind: TermKind,
    /// Exact constraints relating the variable to other variables.
    pub definition: Vec<Atom>,
    /// Variables the value depends on (for `Computed` and `Defined` terms).
    pub operands: Vec<Var>,
}

impl TermInfo {
    pub fn free(domain: Domain) -> Self {
        Self {
            domain,
            kind: TermKind::Free,
            definition: Vec::new(),
            operands: Vec::new(),
        }
    }
}

/// Supplies the meaning of solver variables.
pub trait TermOracle {
    fn describe(&self, v: Var) -> TermInfo;

    /// The value of a `Computed` variable given values of the free variables it depends on;
    /// `None` when it cannot be evaluated (division by zero, an opaque operand, overflow).
    fn compute(&self, v: Var, value: &dyn Fn(Var) -> Option<i128>) -> Option<i128>;
}

/// An oracle for plain unbounded integer variables.
pub struct FreeIntegers;

impl TermOracle for FreeIntegers {
    fn describe(&self, _v: Var) -> TermInfo {
        TermInfo::free(Domain::UNBOUNDED)
    }

    fn compute(&self, _v: Var, _value: &dyn Fn(Var) -> Option<i128>) -> Option<i128> {
        None
    }
}

// ── Results ──────────────────────────────────────────────────────────────────

/// Limits of one feasibility query.
#[derive(Clone, Debug)]
pub struct SolverLimits {
    /// Maximum `≤`-constraints alive during Fourier–Motzkin projection.
    pub max_constraints: usize,
    /// Maximum variables in the definition closure of a query.
    pub max_terms: usize,
    /// Maximum disequality case splits per query.
    pub max_disequality_splits: u32,
    /// Candidate values tried per variable during model construction.
    pub max_model_candidates: u32,
}

impl Default for SolverLimits {
    fn default() -> Self {
        Self {
            max_constraints: 4096,
            max_terms: 2048,
            max_disequality_splits: 16,
            max_model_candidates: 64,
        }
    }
}

/// A value in a model.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ModelValue {
    Int(i128),
    /// A concrete object constant.
    Object(u32),
    /// Some object distinct from every other `Distinct` class and every constant.
    Distinct(u32),
}

impl ModelValue {
    pub fn as_int(&self) -> Option<i128> {
        match self {
            ModelValue::Int(i) => Some(*i),
            _ => None,
        }
    }
}

/// A verified satisfying assignment.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Model {
    pub values: BTreeMap<Var, ModelValue>,
}

impl Model {
    pub fn int(&self, v: Var) -> Option<i128> {
        self.values.get(&v).and_then(|m| m.as_int())
    }
}

/// Why a query was not decided.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum UnknownReason {
    /// Fourier–Motzkin exceeded `max_constraints`.
    ConstraintLimit,
    /// The definition closure exceeded `max_terms`.
    TermLimit,
    /// More disequality splits were needed than `max_disequality_splits`.
    DisequalitySplits,
    /// The rational projection is feasible but no integer point was found.
    IntegerGap,
    /// Coefficients overflowed i128.
    CoefficientOverflow,
    /// The condition depends on an uninterpreted term.
    Opaque(Var),
    /// A computed term disagreed with, or could not be evaluated under, the model.
    Unverified(Var),
    /// A reference-valued term occurs in arithmetic.
    MixedTheories(Var),
    /// The query only examined a slice; the rest of the path condition is itself undecided.
    BaseUndecided,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SatResult {
    Sat(Model),
    Unsat,
    Unknown(UnknownReason),
}

impl SatResult {
    pub fn is_unsat(&self) -> bool {
        matches!(self, SatResult::Unsat)
    }

    /// True unless the condition was proved unsatisfiable.
    pub fn may_be_sat(&self) -> bool {
        !self.is_unsat()
    }
}

/// Whether a path condition implies an atom.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Entailment {
    /// `base ⇒ atom` holds.
    Implied,
    /// `base ∧ atom` is unsatisfiable.
    Refuted,
    /// Both `atom` and its negation may hold.
    Undetermined,
}

// ── Decision procedure ───────────────────────────────────────────────────────

/// Union-find node: a variable or a constant of the integer/reference universe.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Node {
    Var(Var),
    Int(i128),
    Object(u32),
}

fn ref_node(t: RefTerm) -> Node {
    match t {
        RefTerm::Var(v) => Node::Var(v),
        RefTerm::Null => Node::Int(0),
        RefTerm::Object(k) => Node::Object(k),
    }
}

#[derive(Default)]
struct UnionFind {
    index: HashMap<Node, usize>,
    nodes: Vec<Node>,
    parent: Vec<usize>,
}

impl UnionFind {
    fn id(&mut self, n: Node) -> usize {
        if let Some(&i) = self.index.get(&n) {
            return i;
        }
        let i = self.nodes.len();
        self.nodes.push(n);
        self.parent.push(i);
        self.index.insert(n, i);
        i
    }

    fn lookup(&self, n: Node) -> Option<usize> {
        self.index.get(&n).copied()
    }

    fn find(&mut self, mut x: usize) -> usize {
        while self.parent[x] != x {
            self.parent[x] = self.parent[self.parent[x]];
            x = self.parent[x];
        }
        x
    }

    fn union(&mut self, a: Node, b: Node) {
        let (ia, ib) = (self.id(a), self.id(b));
        let (ra, rb) = (self.find(ia), self.find(ib));
        if ra != rb {
            let (lo, hi) = if ra < rb { (ra, rb) } else { (rb, ra) };
            self.parent[hi] = lo;
        }
    }
}

/// Constraints over one variable at the moment it was projected out.
struct Elimination {
    var: Var,
    bounds: Vec<LinExpr>,
}

enum Projection {
    Feasible(Vec<Elimination>),
    Unsat,
    Stopped(UnknownReason),
}

enum LinearOutcome {
    Sat(BTreeMap<Var, i128>),
    Unsat,
    Unknown(UnknownReason),
}

/// The canonical atoms of a query together with the descriptions of all their variables.
struct Closure {
    atoms: Vec<Atom>,
    infos: HashMap<Var, TermInfo>,
}

/// Feasibility checker over one term oracle.
pub struct Solver<'a> {
    oracle: &'a dyn TermOracle,
    limits: &'a SolverLimits,
}

impl<'a> Solver<'a> {
    pub fn new(oracle: &'a dyn TermOracle, limits: &'a SolverLimits) -> Self {
        Self { oracle, limits }
    }

    /// Decide the conjunction of `atoms`.
    pub fn check(&self, atoms: &[Atom]) -> SatResult {
        match self.closure(atoms) {
            Ok(closure) => self.decide(&closure),
            Err(reason) => SatResult::Unknown(reason),
        }
    }

    /// Decide `base ∧ extra` by examining only the part of `base` that shares variables
    /// (through definitions and operands) with `extra`. `base_sat` states that `base` alone
    /// was decided satisfiable; without it a satisfiable slice only yields `Unknown`. The
    /// model of a `Sat` answer covers the slice's variables.
    pub fn check_extension(&self, base: &[Atom], base_sat: bool, extra: &[Atom]) -> SatResult {
        let seeds: Vec<Var> = extra.iter().flat_map(|a| a.vars()).collect();
        let mut query = match self.slice(base, &seeds) {
            Ok(s) => s,
            Err(reason) => return SatResult::Unknown(reason),
        };
        query.extend(extra.iter().cloned());
        match self.check(&query) {
            SatResult::Sat(m) if base_sat => SatResult::Sat(m),
            SatResult::Sat(_) => SatResult::Unknown(UnknownReason::BaseUndecided),
            other => other,
        }
    }

    /// Whether `base` implies or refutes `atom`.
    pub fn entails(&self, base: &[Atom], base_sat: bool, atom: &Atom) -> Entailment {
        let negated = match atom.negate() {
            Some(n) => n,
            None => return Entailment::Undetermined,
        };
        if self
            .check_extension(base, base_sat, std::slice::from_ref(&negated))
            .is_unsat()
        {
            return Entailment::Implied;
        }
        if self
            .check_extension(base, base_sat, std::slice::from_ref(atom))
            .is_unsat()
        {
            return Entailment::Refuted;
        }
        Entailment::Undetermined
    }

    /// The single value `e` can take under `base`, when `base` forces one.
    pub fn unique_value(&self, base: &[Atom], base_sat: bool, e: &LinExpr) -> Option<i128> {
        if e.is_constant() {
            return Some(e.constant);
        }
        let seeds: Vec<Var> = e.vars().collect();
        let slice = self.slice(base, &seeds).ok()?;
        let model = match self.check(&slice) {
            SatResult::Sat(m) => m,
            _ => return None,
        };
        // A variable of `e` absent from the model is unconstrained; any candidate value is
        // then refuted by the uniqueness check below.
        let c = e.eval(&|v| model.int(v).or(Some(0)))?;
        let differs = Atom::Ne(e.checked_add_const(c.checked_neg()?)?).canonical();
        if self
            .check_extension(base, base_sat, std::slice::from_ref(&differs))
            .is_unsat()
        {
            Some(c)
        } else {
            None
        }
    }

    /// Atoms of `base` connected to `seeds` through shared variables.
    fn slice(&self, base: &[Atom], seeds: &[Var]) -> Result<Vec<Atom>, UnknownReason> {
        if seeds.is_empty() {
            return Ok(Vec::new());
        }
        let mut parent: HashMap<Var, Var> = HashMap::new();
        fn root(parent: &mut HashMap<Var, Var>, v: Var) -> Var {
            let mut x = v;
            while let Some(&p) = parent.get(&x) {
                if p == x {
                    break;
                }
                x = p;
            }
            parent.insert(v, x);
            x
        }
        fn join(parent: &mut HashMap<Var, Var>, a: Var, b: Var) {
            let (ra, rb) = (root(parent, a), root(parent, b));
            if ra != rb {
                parent.insert(rb, ra);
            }
        }

        // Expand each variable through definitions and operands, linking the expansion.
        let mut described: HashSet<Var> = HashSet::new();
        let mut queue: Vec<Var> = seeds.to_vec();
        for a in base {
            queue.extend(a.vars());
        }
        while let Some(v) = queue.pop() {
            if !described.insert(v) {
                continue;
            }
            if described.len() > self.limits.max_terms {
                return Err(UnknownReason::TermLimit);
            }
            parent.entry(v).or_insert(v);
            let info = self.oracle.describe(v);
            let mut linked: Vec<Var> = info.operands.clone();
            for d in &info.definition {
                linked.extend(d.vars());
            }
            for w in linked {
                parent.entry(w).or_insert(w);
                join(&mut parent, v, w);
                queue.push(w);
            }
        }
        for a in base {
            let vars = a.vars();
            for pair in vars.windows(2) {
                join(&mut parent, pair[0], pair[1]);
            }
        }
        for pair in seeds.windows(2) {
            join(&mut parent, pair[0], pair[1]);
        }

        let target = root(&mut parent, seeds[0]);
        let mut out = Vec::new();
        for a in base {
            if let Some(&v) = a.vars().first() {
                if root(&mut parent, v) == target {
                    out.push(a.clone());
                }
            }
        }
        Ok(out)
    }

    /// Canonical atoms plus definitions and type ranges of every reachable variable.
    fn closure(&self, atoms: &[Atom]) -> Result<Closure, UnknownReason> {
        let mut out: Vec<Atom> = Vec::with_capacity(atoms.len());
        let mut infos: HashMap<Var, TermInfo> = HashMap::new();
        let mut queue: Vec<Var> = Vec::new();
        for a in atoms {
            queue.extend(a.vars());
            out.push(a.clone().canonical());
        }
        while let Some(v) = queue.pop() {
            if infos.contains_key(&v) {
                continue;
            }
            if infos.len() >= self.limits.max_terms {
                return Err(UnknownReason::TermLimit);
            }
            let info = self.oracle.describe(v);
            if let Some(lo) = info.domain.lo {
                // lo ≤ v  ⇔  −v + lo ≤ 0
                out.push(
                    Atom::Le(LinExpr {
                        terms: vec![(v, -1)],
                        constant: lo,
                    })
                    .canonical(),
                );
            }
            if let Some(hi) = info.domain.hi {
                // v ≤ hi  ⇔  v − hi ≤ 0
                let neg = hi.checked_neg().ok_or(UnknownReason::CoefficientOverflow)?;
                out.push(
                    Atom::Le(LinExpr {
                        terms: vec![(v, 1)],
                        constant: neg,
                    })
                    .canonical(),
                );
            }
            for d in &info.definition {
                queue.extend(d.vars());
                out.push(d.clone().canonical());
            }
            queue.extend(info.operands.iter().copied());
            infos.insert(v, info);
        }
        Ok(Closure { atoms: out, infos })
    }

    fn decide(&self, c: &Closure) -> SatResult {
        let mut uf = UnionFind::default();
        let mut holds: HashMap<Var, bool> = HashMap::new();
        let mut ref_ne: Vec<(RefTerm, RefTerm)> = Vec::new();
        let mut ref_vars: HashSet<Var> = HashSet::new();
        let mut eqs: Vec<LinExpr> = Vec::new();
        let mut nes: Vec<LinExpr> = Vec::new();
        let mut les: Vec<LinExpr> = Vec::new();

        // ── Congruence closure ───────────────────────────────────────────────
        for atom in &c.atoms {
            match atom {
                Atom::Const(false) => return SatResult::Unsat,
                Atom::Const(true) => {}
                Atom::Holds(v, b) => {
                    if let Some(prev) = holds.insert(*v, *b) {
                        if prev != *b {
                            return SatResult::Unsat;
                        }
                    }
                }
                Atom::RefEq(a, b) | Atom::RefNe(a, b) => {
                    for t in [a, b] {
                        if let RefTerm::Var(v) = t {
                            ref_vars.insert(*v);
                        }
                    }
                    if matches!(atom, Atom::RefEq(..)) {
                        uf.union(ref_node(*a), ref_node(*b));
                    } else {
                        uf.id(ref_node(*a));
                        uf.id(ref_node(*b));
                        ref_ne.push((*a, *b));
                    }
                }
                Atom::Eq(e) => match e.terms.as_slice() {
                    [(x, 1), (y, -1)] if e.constant == 0 => uf.union(Node::Var(*x), Node::Var(*y)),
                    [(x, 1)] => match e.constant.checked_neg() {
                        Some(k) => uf.union(Node::Var(*x), Node::Int(k)),
                        None => eqs.push(e.clone()),
                    },
                    _ => eqs.push(e.clone()),
                },
                Atom::Ne(e) => nes.push(e.clone()),
                Atom::Le(e) => les.push(e.clone()),
            }
        }

        let mut class_const: HashMap<usize, Node> = HashMap::new();
        let mut class_rep: HashMap<usize, Var> = HashMap::new();
        for i in 0..uf.nodes.len() {
            let node = uf.nodes[i];
            let r = uf.find(i);
            match node {
                Node::Var(v) => {
                    let slot = class_rep.entry(r).or_insert(v);
                    if v < *slot {
                        *slot = v;
                    }
                }
                _ => match class_const.get(&r) {
                    Some(&other) if other != node => return SatResult::Unsat,
                    _ => {
                        class_const.insert(r, node);
                    }
                },
            }
        }
        for &(a, b) in &ref_ne {
            let (ia, ib) = (uf.id(ref_node(a)), uf.id(ref_node(b)));
            if uf.find(ia) == uf.find(ib) {
                return SatResult::Unsat;
            }
        }

        // ── Substitute class representatives ─────────────────────────────────
        let mut lin_le: Vec<LinExpr> = Vec::new();
        let mut lin_eq: Vec<LinExpr> = Vec::new();
        let mut lin_ne: Vec<LinExpr> = Vec::new();
        for (src, kind) in [(&les, Rel::Le), (&eqs, Rel::Eq), (&nes, Rel::Ne)] {
            for e in src.iter() {
                let r = match rewrite_classes(e, &mut uf, &class_const, &class_rep) {
                    Ok(r) => r,
                    Err(reason) => return SatResult::Unknown(reason),
                };
                match kind.atom(r) {
                    Atom::Const(false) => return SatResult::Unsat,
                    Atom::Le(x) => lin_le.push(x),
                    Atom::Eq(x) => lin_eq.push(x),
                    Atom::Ne(x) => lin_ne.push(x),
                    _ => {}
                }
            }
        }

        // ── Gaussian substitution of unit-coefficient equalities ─────────────
        let mut solved: Vec<(Var, LinExpr)> = Vec::new();
        loop {
            let pick = lin_eq.iter().enumerate().find_map(|(i, e)| {
                e.terms
                    .iter()
                    .find(|&&(_, c)| c == 1 || c == -1)
                    .map(|&(v, c)| (i, v, c))
            });
            let (i, x, a) = match pick {
                Some(p) => p,
                None => break,
            };
            let e = lin_eq.swap_remove(i);
            // a·x + r = 0 with a = ±1  ⇒  x = −a·r
            let value = match e
                .substitute(x, &LinExpr::constant(0))
                .and_then(|rest| rest.checked_scale(-a))
            {
                Some(v) => v,
                None => return SatResult::Unknown(UnknownReason::CoefficientOverflow),
            };
            for (list, kind) in [(&mut lin_le, Rel::Le), (&mut lin_eq, Rel::Eq), (&mut lin_ne, Rel::Ne)] {
                let mut kept = Vec::with_capacity(list.len());
                for e in list.drain(..) {
                    let s = match e.substitute(x, &value) {
                        Some(s) => s,
                        None => return SatResult::Unknown(UnknownReason::CoefficientOverflow),
                    };
                    match kind.atom(s) {
                        Atom::Const(false) => return SatResult::Unsat,
                        Atom::Le(y) | Atom::Eq(y) | Atom::Ne(y) => kept.push(y),
                        _ => {}
                    }
                }
                *list = kept;
            }
            solved.push((x, value));
        }
        for e in lin_eq {
            match e.checked_scale(-1) {
                Some(neg) => {
                    lin_le.push(e);
                    lin_le.push(neg);
                }
                None => return SatResult::Unknown(UnknownReason::CoefficientOverflow),
            }
        }

        // ── Projection and model construction ────────────────────────────────
        let mut splits = self.limits.max_disequality_splits;
        let mut ints = match self.solve_linear(lin_le, &lin_ne, &mut splits) {
            LinearOutcome::Sat(m) => m,
            LinearOutcome::Unsat => return SatResult::Unsat,
            LinearOutcome::Unknown(r) => return SatResult::Unknown(r),
        };
        for (x, expr) in solved.iter().rev() {
            match expr.eval(&|v| Some(ints.get(&v).copied().unwrap_or(0))) {
                Some(val) => {
                    ints.insert(*x, val);
                }
                None => return SatResult::Unknown(UnknownReason::CoefficientOverflow),
            }
        }

        // ── Full assignment ──────────────────────────────────────────────────
        let mut values: BTreeMap<Var, ModelValue> = BTreeMap::new();
        let mut computed: Vec<Var> = Vec::new();
        let mut constrained: HashSet<Var> = HashSet::new();
        let mut opaque: Option<Var> = None;
        let mut vars: Vec<Var> = c.infos.keys().copied().collect();
        vars.sort_unstable();
        for v in vars {
            let info = &c.infos[&v];
            let assigned = match uf.lookup(Node::Var(v)) {
                Some(i) => {
                    let r = uf.find(i);
                    match class_const.get(&r) {
                        Some(Node::Int(k)) => Some(ModelValue::Int(*k)),
                        Some(Node::Object(k)) => Some(ModelValue::Object(*k)),
                        _ => {
                            let rep = class_rep.get(&r).copied().unwrap_or(v);
                            match ints.get(&rep) {
                                Some(&k) => Some(ModelValue::Int(k)),
                                None if ref_vars.contains(&rep) || ref_vars.contains(&v) => {
                                    Some(ModelValue::Distinct(r as u32))
                                }
                                None => None,
                            }
                        }
                    }
                }
                None => ints.get(&v).map(|&k| ModelValue::Int(k)),
            };
            match info.kind {
                TermKind::Opaque => {
                    if opaque.is_none() {
                        opaque = Some(v);
                    }
                }
                TermKind::Computed => computed.push(v),
                TermKind::Free | TermKind::Defined => {}
            }
            let value = match assigned {
                Some(val) => {
                    constrained.insert(v);
                    val
                }
                None => match holds.get(&v) {
                    Some(&b) => {
                        constrained.insert(v);
                        ModelValue::Int(b as i128)
                    }
                    None => ModelValue::Int(info.domain.default_value()),
                },
            };
            values.insert(v, value);
        }
        if let Some(v) = opaque {
            return SatResult::Unknown(UnknownReason::Opaque(v));
        }

        // Computed terms must agree with the value the constraints gave them.
        for &v in &computed {
            let actual = {
                let lookup = |w: Var| values.get(&w).and_then(|m| m.as_int());
                match self.oracle.compute(v, &lookup) {
                    Some(x) => x,
                    None => return SatResult::Unknown(UnknownReason::Unverified(v)),
                }
            };
            if constrained.contains(&v) {
                if values.get(&v).and_then(|m| m.as_int()) != Some(actual) {
                    return SatResult::Unknown(UnknownReason::Unverified(v));
                }
            } else {
                values.insert(v, ModelValue::Int(actual));
            }
        }

        // ── Verification ─────────────────────────────────────────────────────
        let model = Model { values };
        for atom in &c.atoms {
            if let Err(reason) = verify(atom, &model) {
                return SatResult::Unknown(reason);
            }
        }
        SatResult::Sat(model)
    }

    fn solve_linear(&self, les: Vec<LinExpr>, nes: &[LinExpr], splits: &mut u32) -> LinearOutcome {
        let elims = match self.project(les.clone()) {
            Projection::Feasible(e) => e,
            Projection::Unsat => return LinearOutcome::Unsat,
            Projection::Stopped(r) => return LinearOutcome::Unknown(r),
        };
        let mut model = match self.build_model(&elims, nes) {
            Some(m) => m,
            None => return LinearOutcome::Unknown(UnknownReason::IntegerGap),
        };
        self.assign_remaining(&mut model, nes);
        let violated = nes
            .iter()
            .find(|e| e.eval(&|v| model.get(&v).copied()) == Some(0))
            .cloned();
        let e = match violated {
            None => return LinearOutcome::Sat(model),
            Some(e) => e,
        };
        if *splits == 0 {
            return LinearOutcome::Unknown(UnknownReason::DisequalitySplits);
        }
        *splits -= 1;
        // e ≠ 0  ⇔  e ≤ −1  ∨  e ≥ 1
        let below = e.checked_add_const(1);
        let above = e.checked_scale(-1).and_then(|n| n.checked_add_const(1));
        let (below, above) = match (below, above) {
            (Some(b), Some(a)) => (b, a),
            _ => return LinearOutcome::Unknown(UnknownReason::CoefficientOverflow),
        };
        let mut left = les.clone();
        left.push(below);
        let first = self.solve_linear(left, nes, splits);
        if let LinearOutcome::Sat(m) = first {
            return LinearOutcome::Sat(m);
        }
        let mut right = les;
        right.push(above);
        let second = self.solve_linear(right, nes, splits);
        match (first, second) {
            (_, LinearOutcome::Sat(m)) => LinearOutcome::Sat(m),
            (LinearOutcome::Unsat, LinearOutcome::Unsat) => LinearOutcome::Unsat,
            (LinearOutcome::Unknown(r), _) | (_, LinearOutcome::Unknown(r)) => {
                LinearOutcome::Unknown(r)
            }
            (LinearOutcome::Sat(m), _) => LinearOutcome::Sat(m),
        }
    }

    /// Bounded Fourier–Motzkin projection of `e ≤ 0` constraints.
    fn project(&self, input: Vec<LinExpr>) -> Projection {
        let mut system: BTreeMap<Vec<(Var, i128)>, i128> = BTreeMap::new();
        for e in input {
            if insert_le(&mut system, e).is_err() {
                return Projection::Unsat;
            }
        }
        let mut elims = Vec::new();
        loop {
            if system.len() > self.limits.max_constraints {
                return Projection::Stopped(UnknownReason::ConstraintLimit);
            }
            let mut counts: BTreeMap<Var, (i128, i128)> = BTreeMap::new();
            for terms in system.keys() {
                for &(v, c) in terms {
                    let slot = counts.entry(v).or_insert((0, 0));
                    if c > 0 {
                        slot.0 += 1;
                    } else {
                        slot.1 += 1;
                    }
                }
            }
            // Eliminate the variable with the smallest net growth p·n − (p + n).
            let var = match counts
                .iter()
                .min_by_key(|(_, &(p, n))| p * n - (p + n))
                .map(|(&v, _)| v)
            {
                Some(v) => v,
                None => break,
            };
            let mut pos = Vec::new();
            let mut neg = Vec::new();
            let mut next: BTreeMap<Vec<(Var, i128)>, i128> = BTreeMap::new();
            for (terms, constant) in std::mem::take(&mut system) {
                let e = LinExpr { terms, constant };
                let a = e.coeff(var);
                if a > 0 {
                    pos.push(e);
                } else if a < 0 {
                    neg.push(e);
                } else {
                    next.insert(e.terms, e.constant);
                }
            }
            for p in &pos {
                for n in &neg {
                    let cp = p.coeff(var);
                    let cn = -n.coeff(var);
                    let combined = p
                        .checked_scale(cn)
                        .and_then(|x| n.checked_scale(cp).and_then(|y| x.checked_add(&y)));
                    let combined = match combined {
                        Some(e) => e,
                        None => return Projection::Stopped(UnknownReason::CoefficientOverflow),
                    };
                    if insert_le(&mut next, combined).is_err() {
                        return Projection::Unsat;
                    }
                    if next.len() > self.limits.max_constraints {
                        return Projection::Stopped(UnknownReason::ConstraintLimit);
                    }
                }
            }
            let mut bounds = pos;
            bounds.extend(neg);
            elims.push(Elimination { var, bounds });
            system = next;
        }
        Projection::Feasible(elims)
    }

    /// Back-substitute integer values in reverse elimination order.
    fn build_model(&self, elims: &[Elimination], nes: &[LinExpr]) -> Option<BTreeMap<Var, i128>> {
        let mut m: BTreeMap<Var, i128> = BTreeMap::new();
        for el in elims.iter().rev() {
            let (mut lo, mut hi): (Option<i128>, Option<i128>) = (None, None);
            for b in &el.bounds {
                let a = b.coeff(el.var);
                let rest = b.substitute(el.var, &LinExpr::constant(0))?;
                let r = rest.eval(&|v| m.get(&v).copied())?;
                if a > 0 {
                    // a·x + r ≤ 0  ⇒  x ≤ ⌊−r / a⌋
                    let h = floor_div(r.checked_neg()?, a);
                    hi = Some(hi.map_or(h, |x| x.min(h)));
                } else {
                    // −|a|·x + r ≤ 0  ⇒  x ≥ ⌈r / |a|⌉
                    let l = ceil_div(r, a.checked_neg()?);
                    lo = Some(lo.map_or(l, |x| x.max(l)));
                }
            }
            if let (Some(l), Some(h)) = (lo, hi) {
                if l > h {
                    return None;
                }
            }
            let forbidden = forbidden_values(el.var, nes, &m);
            m.insert(el.var, self.pick(lo, hi, &forbidden));
        }
        Some(m)
    }

    /// Give a value to variables that occur only in disequalities.
    fn assign_remaining(&self, m: &mut BTreeMap<Var, i128>, nes: &[LinExpr]) {
        for e in nes {
            let unassigned: Vec<Var> = e.vars().filter(|v| !m.contains_key(v)).collect();
            for (i, &v) in unassigned.iter().enumerate() {
                if i + 1 < unassigned.len() {
                    m.insert(v, 0);
                } else {
                    let forbidden = forbidden_values(v, nes, m);
                    m.insert(v, self.pick(None, None, &forbidden));
                }
            }
        }
    }

    /// The value of `[lo, hi]` closest to 0 that avoids `forbidden`, searching outward.
    fn pick(&self, lo: Option<i128>, hi: Option<i128>, forbidden: &[i128]) -> i128 {
        let inside = |x: i128| lo.map_or(true, |l| x >= l) && hi.map_or(true, |h| x <= h);
        let start = match (lo, hi) {
            (Some(l), _) if l > 0 => l,
            (_, Some(h)) if h < 0 => h,
            _ => 0,
        };
        if !forbidden.contains(&start) {
            return start;
        }
        for step in 1..=self.limits.max_model_candidates as i128 {
            for cand in [start.checked_add(step), start.checked_sub(step)]
                .into_iter()
                .flatten()
            {
                if inside(cand) && !forbidden.contains(&cand) {
                    return cand;
                }
            }
        }
        start
    }
}

/// The relation of a linear constraint being rebuilt.
#[derive(Clone, Copy)]
enum Rel {
    Le,
    Eq,
    Ne,
}

impl Rel {
    fn atom(self, e: LinExpr) -> Atom {
        match self {
            Rel::Le => Atom::Le(e),
            Rel::Eq => Atom::Eq(e),
            Rel::Ne => Atom::Ne(e),
        }
        .canonical()
    }
}

/// Replace every variable by its congruence-class constant or representative.
fn rewrite_classes(
    e: &LinExpr,
    uf: &mut UnionFind,
    class_const: &HashMap<usize, Node>,
    class_rep: &HashMap<usize, Var>,
) -> Result<LinExpr, UnknownReason> {
    let mut out = LinExpr::constant(e.constant);
    for &(v, coeff) in &e.terms {
        let replacement = match uf.lookup(Node::Var(v)) {
            None => LinExpr::var(v),
            Some(i) => {
                let r = uf.find(i);
                match class_const.get(&r) {
                    Some(Node::Int(k)) => LinExpr::constant(*k),
                    Some(_) => return Err(UnknownReason::MixedTheories(v)),
                    None => LinExpr::var(class_rep.get(&r).copied().unwrap_or(v)),
                }
            }
        };
        let scaled = replacement
            .checked_scale(coeff)
            .ok_or(UnknownReason::CoefficientOverflow)?;
        out = out
            .checked_add(&scaled)
            .ok_or(UnknownReason::CoefficientOverflow)?;
    }
    Ok(out)
}

/// Values of `v` that would falsify a disequality whose other variables are assigned.
fn forbidden_values(v: Var, nes: &[LinExpr], m: &BTreeMap<Var, i128>) -> Vec<i128> {
    let mut out = Vec::new();
    for e in nes {
        let a = e.coeff(v);
        if a == 0 {
            continue;
        }
        let rest = match e.substitute(v, &LinExpr::constant(0)) {
            Some(r) => r,
            None => continue,
        };
        if let Some(r) = rest.eval(&|w| m.get(&w).copied()) {
            if r % a == 0 {
                out.push(-(r / a));
            }
        }
    }
    out
}

/// Insert `e ≤ 0` keeping only the tightest constant per left-hand side.
fn insert_le(system: &mut BTreeMap<Vec<(Var, i128)>, i128>, e: LinExpr) -> Result<(), ()> {
    match Atom::Le(e).canonical() {
        Atom::Const(false) => Err(()),
        Atom::Le(LinExpr { terms, constant }) => {
            let slot = system.entry(terms).or_insert(constant);
            if constant > *slot {
                *slot = constant;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// Check one atom under a complete model.
fn verify(atom: &Atom, model: &Model) -> Result<(), UnknownReason> {
    let int_of = |v: Var| -> Result<i128, UnknownReason> {
        match model.values.get(&v) {
            Some(ModelValue::Int(k)) => Ok(*k),
            Some(_) => Err(UnknownReason::MixedTheories(v)),
            None => Err(UnknownReason::Unverified(v)),
        }
    };
    let eval = |e: &LinExpr| -> Result<i128, UnknownReason> {
        let mut acc = e.constant;
        for &(v, c) in &e.terms {
            let term = c
                .checked_mul(int_of(v)?)
                .ok_or(UnknownReason::CoefficientOverflow)?;
            acc = acc
                .checked_add(term)
                .ok_or(UnknownReason::CoefficientOverflow)?;
        }
        Ok(acc)
    };
    let ref_of = |t: &RefTerm| -> Result<ModelValue, UnknownReason> {
        match t {
            RefTerm::Null => Ok(ModelValue::Int(0)),
            RefTerm::Object(k) => Ok(ModelValue::Object(*k)),
            RefTerm::Var(v) => model
                .values
                .get(v)
                .copied()
                .ok_or(UnknownReason::Unverified(*v)),
        }
    };
    let ok = match atom {
        Atom::Eq(e) => eval(e)? == 0,
        Atom::Ne(e) => eval(e)? != 0,
        Atom::Le(e) => eval(e)? <= 0,
        Atom::RefEq(a, b) => ref_of(a)? == ref_of(b)?,
        Atom::RefNe(a, b) => ref_of(a)? != ref_of(b)?,
        Atom::Holds(v, b) => (int_of(*v)? != 0) == *b,
        Atom::Const(b) => *b,
    };
    if ok {
        Ok(())
    } else {
        Err(UnknownReason::Unverified(
            atom.vars().first().copied().unwrap_or(Var(u32::MAX)),
        ))
    }
}
