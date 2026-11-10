//! BDDLibrary — node store, unique table, computed tables and resource limits (§8.2.1, §8.3).
//!
//! Owns the node array (pre-populated with FALSE=0, TRUE=1), the UniqueTable (enforcing
//! Rule 2/sharing) and the computed tables that memoize `apply`/`not` across calls.
//!
//! **Memory safety.** Every operation reads nodes by value (`ROBDDNode` is `Copy`) through
//! `self.nodes[id]` and only then appends; no reference into the node vector is ever held
//! across an allocation, so vector growth cannot invalidate anything. The module tree is
//! compiled under `#![forbid(unsafe_code)]`.
//!
//! **Bounded resources.** All operations are iterative (explicit stacks), so recursion depth
//! never depends on the BDD size. The `try_*` operations enforce the library's
//! [`BddLimits`] — a node budget and an optional wall-clock deadline, in the spirit of CUDD's
//! `Cudd_SetMaxMemory`/`Cudd_SetTimeLimit` — and return [`BddLimit`] when either is hit.
//! The plain operations (`apply`, `restrict`, …) are unbudgeted conveniences for small,
//! trusted inputs.

use std::fmt;
use std::time::Instant;

use super::fxhash::{fx_map_with_capacity, FxHashMap};
use super::node::{is_terminal_id, ROBDDNode, FALSE_ID, TERMINAL_VAR, TRUE_ID};
use super::sat_count::{count_models, evaluate, find_model};
use super::unique_table::UniqueTable;
use crate::psa::count::PathCount;
use crate::psa::types::BoolOp;

/// Why a budgeted BDD operation was abandoned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BddLimit {
    /// The node store reached the configured maximum number of nodes.
    NodeBudget { max_nodes: usize },
    /// The configured wall-clock deadline passed.
    Deadline,
}

impl fmt::Display for BddLimit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BddLimit::NodeBudget { max_nodes } => {
                write!(f, "ROBDD node budget of {} nodes exhausted", max_nodes)
            }
            BddLimit::Deadline => f.write_str("ROBDD construction deadline exceeded"),
        }
    }
}

/// Resource limits enforced by the `try_*` operations.
#[derive(Clone, Copy, Debug)]
pub struct BddLimits {
    /// Maximum number of nodes in the store (terminals included).
    pub max_nodes: usize,
    /// Wall-clock instant after which budgeted operations fail with [`BddLimit::Deadline`].
    pub deadline: Option<Instant>,
}

impl BddLimits {
    /// No node budget and no deadline.
    pub const fn unlimited() -> Self {
        Self {
            max_nodes: usize::MAX,
            deadline: None,
        }
    }

    /// A node budget and an optional deadline.
    pub const fn new(max_nodes: usize, deadline: Option<Instant>) -> Self {
        Self {
            max_nodes,
            deadline,
        }
    }
}

/// The deadline is consulted once per this many units of work (node visits or allocations).
const WORK_CHECK_INTERVAL: u32 = 4096;

/// Upper bound on memoized `apply` results; the table is a lossy cache and is cleared when
/// it reaches this size, which only costs recomputation, never correctness.
const COMPUTED_TABLE_CAP: usize = 1 << 21;

/// Upper bound on memoized `not` results (lossy, like the computed table).
const NOT_CACHE_CAP: usize = 1 << 20;

/// The sub-DAG of one root, renumbered densely (see [`BDDLibrary::compact`]).
#[derive(Clone, Debug)]
pub struct CompactBdd {
    /// FALSE at 0, TRUE at 1, then the reachable internal nodes children-before-parents.
    pub nodes: Vec<ROBDDNode>,
    /// The root's id in `nodes`.
    pub root: u32,
}

/// The central BDD library for one function's ROBDD construction.
pub struct BDDLibrary {
    /// All nodes. Index 0 = FALSE terminal, index 1 = TRUE terminal. Append-only.
    pub(super) nodes: Vec<ROBDDNode>,
    pub(super) unique_table: UniqueTable,
    /// Persistent computed table `(op, min(f,g), max(f,g)) → result`. Node ids are never
    /// reused, so entries stay valid for the lifetime of the library.
    pub(super) computed: FxHashMap<(u8, u32, u32), u32>,
    /// Persistent `¬f` memo (`f → ¬f`, stored in both directions).
    pub(super) not_cache: FxHashMap<u32, u32>,
    limits: BddLimits,
    work_since_check: u32,
}

impl BDDLibrary {
    /// Create a new, unbudgeted library pre-populated with both terminal nodes.
    pub fn new() -> Self {
        Self::with_limits(BddLimits::unlimited())
    }

    /// Create a library whose `try_*` operations enforce `limits`.
    pub fn with_limits(limits: BddLimits) -> Self {
        Self {
            nodes: vec![
                ROBDDNode::false_terminal(), // node_id=0
                ROBDDNode::true_terminal(),  // node_id=1
            ],
            unique_table: UniqueTable::with_capacity(1024),
            computed: fx_map_with_capacity(1024),
            not_cache: fx_map_with_capacity(256),
            limits,
            work_since_check: 0,
        }
    }

    /// The limits enforced by the `try_*` operations.
    #[inline]
    pub fn limits(&self) -> BddLimits {
        self.limits
    }

    /// The FALSE terminal node_id (always 0).
    #[inline]
    pub fn false_id(&self) -> u32 {
        FALSE_ID
    }

    /// The TRUE terminal node_id (always 1).
    #[inline]
    pub fn true_id(&self) -> u32 {
        TRUE_ID
    }

    /// Look up an allocated ROBDD node by its identifier.
    #[inline]
    pub fn node(&self, id: u32) -> &ROBDDNode {
        &self.nodes[id as usize]
    }

    /// Construct the ROBDD node for a single variable xᵢ: `make_node(i, FALSE, TRUE)`.
    ///
    /// `var_idx` must not be `0xFFFF`, which is reserved for terminal nodes.
    pub fn var(&mut self, var_idx: u16) -> u32 {
        unbudgeted(self.var_core(var_idx, false))
    }

    /// Budgeted [`BDDLibrary::var`].
    pub fn try_var(&mut self, var_idx: u16) -> Result<u32, BddLimit> {
        self.var_core(var_idx, true)
    }

    fn var_core(&mut self, var_idx: u16, enforce: bool) -> Result<u32, BddLimit> {
        assert!(
            var_idx != TERMINAL_VAR,
            "BDD variable index 0xFFFF is reserved for terminal nodes"
        );
        self.mk(var_idx, FALSE_ID, TRUE_ID, enforce)
    }

    /// Compute `f op g` via Shannon-expansion apply (§8.2.2). Unbudgeted.
    pub fn apply(&mut self, op: BoolOp, f: u32, g: u32) -> u32 {
        unbudgeted(self.apply_core(op, f, g, false))
    }

    /// Budgeted [`BDDLibrary::apply`].
    pub fn try_apply(&mut self, op: BoolOp, f: u32, g: u32) -> Result<u32, BddLimit> {
        self.apply_core(op, f, g, true)
    }

    /// Compute `¬f` (NOT). Unbudgeted.
    pub fn apply_not(&mut self, f: u32) -> u32 {
        unbudgeted(self.not_core(f, false))
    }

    /// Budgeted [`BDDLibrary::apply_not`].
    pub fn try_not(&mut self, f: u32) -> Result<u32, BddLimit> {
        self.not_core(f, true)
    }

    /// Compute `a → b` (implication ≡ ¬a ∨ b) (§8.5.3). Unbudgeted.
    pub fn implies(&mut self, a: u32, b: u32) -> u32 {
        let not_a = self.apply_not(a);
        self.apply(BoolOp::Or, not_a, b)
    }

    /// Budgeted [`BDDLibrary::implies`].
    pub fn try_implies(&mut self, a: u32, b: u32) -> Result<u32, BddLimit> {
        let not_a = self.try_not(a)?;
        self.try_apply(BoolOp::Or, not_a, b)
    }

    /// Budgeted `a ↔ b` (≡ ¬(a ⊕ b)).
    pub fn try_iff(&mut self, a: u32, b: u32) -> Result<u32, BddLimit> {
        let x = self.try_apply(BoolOp::Xor, a, b)?;
        self.try_not(x)
    }

    /// If-then-else `ITE(f, g, h) = (f ∧ g) ∨ (¬f ∧ h)`. Unbudgeted.
    pub fn ite(&mut self, f: u32, g: u32, h: u32) -> u32 {
        unbudgeted(self.ite_core(f, g, h, false))
    }

    /// Budgeted [`BDDLibrary::ite`].
    pub fn try_ite(&mut self, f: u32, g: u32, h: u32) -> Result<u32, BddLimit> {
        self.ite_core(f, g, h, true)
    }

    fn ite_core(&mut self, f: u32, g: u32, h: u32, enforce: bool) -> Result<u32, BddLimit> {
        self.assert_id(f);
        self.assert_id(g);
        self.assert_id(h);
        // Standard ITE terminal cases (Brace, Rudell & Bryant, DAC 1990).
        if f == TRUE_ID || g == h {
            return Ok(g);
        }
        if f == FALSE_ID {
            return Ok(h);
        }
        if g == TRUE_ID && h == FALSE_ID {
            return Ok(f);
        }
        if g == FALSE_ID && h == TRUE_ID {
            return self.not_core(f, enforce);
        }
        let then_part = self.apply_core(BoolOp::And, f, g, enforce)?;
        let not_f = self.not_core(f, enforce)?;
        let else_part = self.apply_core(BoolOp::And, not_f, h, enforce)?;
        self.apply_core(BoolOp::Or, then_part, else_part, enforce)
    }

    /// Compute `f|_{var=val}` — the cofactor of f (§8.5.2). `val` is 0 or non-zero (= 1).
    /// Unbudgeted.
    pub fn restrict(&mut self, f: u32, var: u16, val: u8) -> u32 {
        unbudgeted(self.restrict_core(f, var, val != 0, false))
    }

    /// Budgeted [`BDDLibrary::restrict`].
    pub fn try_restrict(&mut self, f: u32, var: u16, val: bool) -> Result<u32, BddLimit> {
        self.restrict_core(f, var, val, true)
    }

    /// Existential quantification `∃ vars. f`. `quantified[v]` selects variable `v`;
    /// variables beyond the slice are kept. Unbudgeted.
    pub fn exists(&mut self, f: u32, quantified: &[bool]) -> u32 {
        unbudgeted(self.exists_core(f, quantified, false))
    }

    /// Budgeted [`BDDLibrary::exists`].
    pub fn try_exists(&mut self, f: u32, quantified: &[bool]) -> Result<u32, BddLimit> {
        self.exists_core(f, quantified, true)
    }

    /// The value of `f` under `assignment` (`assignment[v]` = value of variable `v`).
    ///
    /// Fails when `f` tests a variable the assignment does not cover.
    pub fn eval(&self, f: u32, assignment: &[bool]) -> Result<bool, String> {
        evaluate(&self.nodes, f, assignment)
    }

    /// Compute #SAT(f) over `n_vars` variables, clamped to `u64::MAX`.
    ///
    /// Counts above 2^64 are clamped; use [`BDDLibrary::sat_count_exact`] wherever the
    /// result is persisted or aggregated so the clamping can be recorded.
    ///
    /// # Panics
    /// If `f` mentions a variable `>= n_vars`.
    pub fn sat_count(&self, f: u32, n_vars: u16) -> u64 {
        self.sat_count_exact(f, n_vars).saturating_u64()
    }

    /// Exact #SAT(f) over `n_vars` variables (§8.2.5).
    ///
    /// # Panics
    /// If `f` mentions a variable `>= n_vars`.
    pub fn sat_count_exact(&self, f: u32, n_vars: u16) -> PathCount {
        match count_models(&self.nodes, f, n_vars, None) {
            Ok(c) => c,
            Err(msg) => panic!("sat_count: {}", msg),
        }
    }

    /// Exact #SAT(f) over `n_vars` variables where `fixed[v] = Some(b)` pins variable `v` to
    /// `b` (a pinned variable contributes one value instead of two). `fixed.len()` must equal
    /// `n_vars`; an error describes any inconsistency between `f` and `n_vars`.
    pub fn try_sat_count_fixed(
        &self,
        f: u32,
        n_vars: u16,
        fixed: &[Option<bool>],
    ) -> Result<PathCount, String> {
        count_models(&self.nodes, f, n_vars, Some(fixed))
    }

    /// One satisfying assignment of `f` over `n_vars` variables (`assignment[v]` is the value
    /// of variable `v`; variables `f` does not depend on are set to `false`), or `None` when
    /// `f` is unsatisfiable.
    ///
    /// # Panics
    /// If `f` mentions a variable `>= n_vars`.
    pub fn any_sat(&self, f: u32, n_vars: u16) -> Option<Vec<bool>> {
        match find_model(&self.nodes, f, n_vars, None) {
            Ok(m) => m,
            Err(msg) => panic!("any_sat: {}", msg),
        }
    }

    /// Verify ROBDD canonicity of the whole node store (§8.8 Invariant 3): Rule 1
    /// (no `lo == hi`), Rule 2 (no duplicate `(var, lo, hi)`), the variable-order invariant
    /// (`var(n) < var(child)`) and acyclicity (children are allocated before parents).
    pub fn verify_canonicity(&self) -> Result<(), String> {
        verify_node_array(&self.nodes, None)
    }

    /// Total node count (including terminals and nodes no longer reachable from any root).
    #[inline]
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Number of internal (non-terminal) nodes reachable from `root`: the size of the BDD of
    /// `root` alone, independent of every other node in the store.
    pub fn dag_size(&self, root: u32) -> usize {
        let reachable = self.reachable_node_count(root);
        let terminals = if is_terminal_id(root) {
            1
        } else {
            // A non-constant function reaches both terminals.
            2
        };
        reachable - terminals
    }

    /// Number of nodes reachable from `root`, terminals included.
    pub fn reachable_node_count(&self, root: u32) -> usize {
        self.assert_id(root);
        let mut seen = vec![false; self.nodes.len()];
        let mut stack = vec![root];
        let mut count = 0usize;
        while let Some(id) = stack.pop() {
            let idx = id as usize;
            if seen[idx] {
                continue;
            }
            seen[idx] = true;
            count += 1;
            if !is_terminal_id(id) {
                let n = self.nodes[idx];
                stack.push(n.lo);
                stack.push(n.hi);
            }
        }
        count
    }

    /// Mark-compact: copy the sub-DAG reachable from `root` into a fresh, dense node array
    /// (FALSE=0, TRUE=1, internal nodes children-before-parents), dropping every intermediate
    /// node created during construction.
    ///
    /// `relabel`, when given, maps each variable index to its new index; it must be strictly
    /// increasing on the variables that occur in the sub-DAG so the order invariant holds.
    pub fn compact(&self, root: u32, relabel: Option<&[u16]>) -> CompactBdd {
        self.assert_id(root);
        let mut out: Vec<ROBDDNode> = vec![ROBDDNode::false_terminal(), ROBDDNode::true_terminal()];
        if is_terminal_id(root) {
            return CompactBdd { nodes: out, root };
        }
        let mut new_id = vec![u32::MAX; self.nodes.len()];
        new_id[FALSE_ID as usize] = FALSE_ID;
        new_id[TRUE_ID as usize] = TRUE_ID;

        // Iterative post-order: (node, children_done).
        let mut stack: Vec<(u32, bool)> = vec![(root, false)];
        while let Some((id, children_done)) = stack.pop() {
            let idx = id as usize;
            if new_id[idx] != u32::MAX {
                continue;
            }
            let n = self.nodes[idx];
            if !children_done {
                stack.push((id, true));
                if new_id[n.hi as usize] == u32::MAX {
                    stack.push((n.hi, false));
                }
                if new_id[n.lo as usize] == u32::MAX {
                    stack.push((n.lo, false));
                }
            } else {
                let var = match relabel {
                    Some(map) => map[n.var as usize],
                    None => n.var,
                };
                let id_new = out.len() as u32;
                out.push(ROBDDNode::internal(
                    var,
                    new_id[n.lo as usize],
                    new_id[n.hi as usize],
                ));
                new_id[idx] = id_new;
            }
        }
        let root_new = new_id[root as usize];
        CompactBdd {
            nodes: out,
            root: root_new,
        }
    }

    /// Consume the library and return the raw node store (including unreachable nodes).
    /// Phase 8 serializes [`BDDLibrary::compact`] output instead.
    pub fn into_nodes(self) -> Vec<ROBDDNode> {
        self.nodes
    }

    // ── Internal primitives shared by apply/not/restrict/exists ──────────────

    /// `make_node` with both reduction rules and (when `enforce`) the node budget.
    #[inline]
    pub(super) fn mk(
        &mut self,
        var: u16,
        lo: u32,
        hi: u32,
        enforce: bool,
    ) -> Result<u32, BddLimit> {
        if lo == hi {
            return Ok(lo);
        }
        if let Some(existing) = self.unique_table.lookup(var, lo, hi) {
            return Ok(existing);
        }
        if enforce {
            if self.nodes.len() >= self.limits.max_nodes {
                return Err(BddLimit::NodeBudget {
                    max_nodes: self.limits.max_nodes,
                });
            }
            self.tick(true)?;
        }
        Ok(self.unique_table.insert_new(var, lo, hi, &mut self.nodes))
    }

    /// Account one unit of work and consult the deadline periodically.
    #[inline]
    pub(super) fn tick(&mut self, enforce: bool) -> Result<(), BddLimit> {
        if !enforce {
            return Ok(());
        }
        self.work_since_check += 1;
        if self.work_since_check >= WORK_CHECK_INTERVAL {
            self.work_since_check = 0;
            if let Some(deadline) = self.limits.deadline {
                if Instant::now() >= deadline {
                    return Err(BddLimit::Deadline);
                }
            }
        }
        Ok(())
    }

    /// Memoize an `apply` result, clearing the lossy table when it is full.
    #[inline]
    pub(super) fn remember_computed(&mut self, key: (u8, u32, u32), result: u32) {
        if self.computed.len() >= COMPUTED_TABLE_CAP {
            self.computed.clear();
        }
        self.computed.insert(key, result);
    }

    /// Memoize `¬f = r` (and `¬r = f`), clearing the lossy table when it is full.
    #[inline]
    pub(super) fn remember_not(&mut self, f: u32, r: u32) {
        if self.not_cache.len() >= NOT_CACHE_CAP {
            self.not_cache.clear();
        }
        self.not_cache.insert(f, r);
        self.not_cache.insert(r, f);
    }

    /// Panic with a clear message when a caller passes a node id this library never issued.
    #[inline]
    pub(super) fn assert_id(&self, id: u32) {
        assert!(
            (id as usize) < self.nodes.len(),
            "BDD node id {} out of range (library holds {} nodes)",
            id,
            self.nodes.len()
        );
    }
}

impl Default for BDDLibrary {
    fn default() -> Self {
        Self::new()
    }
}

/// Unwrap the result of an operation run with `enforce = false`, which never consults the
/// limits and therefore cannot fail.
#[inline]
fn unbudgeted(r: Result<u32, BddLimit>) -> u32 {
    match r {
        Ok(id) => id,
        Err(limit) => unreachable!("unbudgeted BDD operation reported a limit: {}", limit),
    }
}

/// Verify a node array: terminals at ids 0 and 1, Rule 1, Rule 2, the variable-order
/// invariant, acyclicity (children ids below the parent id) and, when `n_vars` is given,
/// that every variable index is below `n_vars`.
pub fn verify_node_array(nodes: &[ROBDDNode], n_vars: Option<u16>) -> Result<(), String> {
    if nodes.len() < 2 {
        return Err(format!(
            "node array has {} entries; terminals missing",
            nodes.len()
        ));
    }
    if nodes[FALSE_ID as usize] != ROBDDNode::false_terminal() {
        return Err("node 0 is not the FALSE terminal".to_string());
    }
    if nodes[TRUE_ID as usize] != ROBDDNode::true_terminal() {
        return Err("node 1 is not the TRUE terminal".to_string());
    }
    let mut seen: FxHashMap<(u16, u32, u32), u32> = fx_map_with_capacity(nodes.len());
    for (id, node) in nodes.iter().enumerate().skip(2) {
        if node.is_terminal() || node.flags != 0 {
            return Err(format!(
                "node_id={} is marked terminal but is not node 0 or 1",
                id
            ));
        }
        if let Some(limit) = n_vars {
            if node.var >= limit {
                return Err(format!(
                    "node_id={} uses variable {} but the function has {} variables",
                    id, node.var, limit
                ));
            }
        }
        if node.lo == node.hi {
            return Err(format!(
                "Rule 1 violation (Elimination): node_id={} has lo==hi=={}",
                id, node.lo
            ));
        }
        for child in [node.lo, node.hi] {
            if child as usize >= id {
                return Err(format!(
                    "node_id={} references child {} that is not allocated before it",
                    id, child
                ));
            }
            let child_var = nodes[child as usize].var;
            if child_var <= node.var {
                return Err(format!(
                    "Order violation: node_id={} (var={}) has child {} with var={}",
                    id, node.var, child, child_var
                ));
            }
        }
        let key = (node.var, node.lo, node.hi);
        if let Some(&existing) = seen.get(&key) {
            return Err(format!(
                "Rule 2 violation (Sharing): node_id={} duplicates node_id={} \
                 with (var={}, lo={}, hi={})",
                id, existing, node.var, node.lo, node.hi
            ));
        }
        seen.insert(key, id as u32);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_has_two_terminals() {
        let lib = BDDLibrary::new();
        assert_eq!(lib.node_count(), 2);
    }

    #[test]
    fn var_deduplication() {
        let mut lib = BDDLibrary::new();
        let a = lib.var(0);
        let b = lib.var(0);
        assert_eq!(a, b);
        assert_eq!(lib.node_count(), 3);
    }

    #[test]
    fn and_with_true_is_identity() {
        let mut lib = BDDLibrary::new();
        let x = lib.var(0);
        let r = lib.apply(BoolOp::And, x, lib.true_id());
        assert_eq!(r, x);
    }

    #[test]
    fn restrict_entry_to_one_gives_true() {
        let mut lib = BDDLibrary::new();
        let x = lib.var(0);
        let r = lib.restrict(x, 0, 1);
        assert_eq!(r, TRUE_ID);
    }

    #[test]
    fn sat_count_single_var() {
        let mut lib = BDDLibrary::new();
        let x = lib.var(0);
        assert_eq!(lib.sat_count(x, 1), 1);
    }

    #[test]
    fn canonicity_after_construction() {
        let mut lib = BDDLibrary::new();
        let x0 = lib.var(0);
        let x1 = lib.var(1);
        lib.apply(BoolOp::And, x0, x1);
        assert!(lib.verify_canonicity().is_ok());
    }
}
