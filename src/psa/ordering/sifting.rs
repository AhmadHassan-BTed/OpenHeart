//! SiftingOptimizer — Rudell's dynamic variable reordering by sifting (§8.2.4).
//!
//! Rudell (ICCAD 1993): each variable in turn is moved through every level by swaps of
//! adjacent levels, the BDD size is measured after every swap, and the variable is left at
//! the level where the BDD was smallest. Phase 8 sifts the final, compacted `f_paths`.
//!
//! **In-place adjacent-level swap.** With `a` at level `i` and `b` at level `i+1`, every node
//! `F = (a, F0, F1)` whose children mention `b` is rewritten *in place* (its id — and so every
//! reference to it — survives) as `(b, (a, F00, F10), (a, F01, F11))`, where `Fxy` are the
//! cofactors w.r.t. `a = x, b = y`. Nodes labelled `a` that do not depend on `b` simply move
//! down a level; nodes labelled `b` move up. Only levels `i` and `i+1` are touched. Per-
//! variable unique subtables keep the result reduced, and reference counts free the `b`
//! nodes that lose their last parent, so the live node count *is* the BDD size.
//!
//! Variables carry fixed labels (their level before sifting); `level_of_var` /
//! `var_at_level` are the CUDD-style perm/invperm indirection, and the final permutation is
//! reported so the caller can permute the variable ordering table consistently.
//!
//! **Cost control.** Sifting stops after `max_swaps` swaps or at the deadline (the variable
//! being sifted is still returned to its best level, so the result is never larger than the
//! input), a direction is abandoned once the size exceeds `max_growth` × the best size seen,
//! and at most `max_vars` variables are sifted, largest levels first.

use std::time::Instant;

use crate::psa::bdd::fxhash::{fx_map_with_capacity, FxHashMap};
use crate::psa::bdd::node::{ROBDDNode, FALSE_ID, TRUE_ID};
use crate::psa::bdd::CompactBdd;

/// Label of terminal nodes inside the sifting manager.
const TERMINAL_LABEL: u32 = u32::MAX;

/// Sifting limits.
#[derive(Clone, Copy, Debug)]
pub struct SiftingConfig {
    /// Abandon a sifting direction once size > `max_growth` × best size (CUDD default 1.2).
    pub max_growth: f64,
    /// Total adjacent swaps allowed for the whole pass.
    pub max_swaps: usize,
    /// Number of variables sifted (largest first).
    pub max_vars: usize,
    /// Stop sifting new variables after this instant.
    pub deadline: Option<Instant>,
}

impl Default for SiftingConfig {
    fn default() -> Self {
        Self {
            max_growth: 1.2,
            max_swaps: 200_000,
            max_vars: 1000,
            deadline: None,
        }
    }
}

/// Result of a sifting pass.
#[derive(Clone, Debug)]
pub struct SiftResult {
    /// Compacted node array under the new order (node `var` = new level).
    pub nodes: Vec<ROBDDNode>,
    /// Root id in `nodes`.
    pub root: u32,
    /// `level_to_old_level[l]` = the level, before sifting, of the variable now at level `l`.
    pub level_to_old_level: Vec<u16>,
    /// Adjacent swaps performed.
    pub swaps: usize,
    /// Internal node count before sifting.
    pub size_before: usize,
    /// Internal node count after sifting.
    pub size_after: usize,
    /// False when the swap budget or the deadline ended the pass early.
    pub completed: bool,
}

/// Rudell sifting optimizer.
pub struct SiftingOptimizer;

impl SiftingOptimizer {
    /// Sift the BDD `bdd` over `n_vars` variables (node `var` fields are levels).
    pub fn sift(bdd: &CompactBdd, n_vars: u16, config: &SiftingConfig) -> SiftResult {
        let mut mgr = Manager::import(bdd, n_vars as usize);
        let size_before = mgr.live;
        let mut completed = true;

        if n_vars > 1 && mgr.live > 1 {
            // Largest subtables first (Rudell's heuristic).
            let mut vars: Vec<u32> = (0..n_vars as u32).collect();
            vars.sort_by(|&a, &b| {
                mgr.unique[b as usize]
                    .len()
                    .cmp(&mgr.unique[a as usize].len())
                    .then(a.cmp(&b))
            });
            for (count, &v) in vars.iter().enumerate() {
                if count >= config.max_vars {
                    completed = false;
                    break;
                }
                if mgr.unique[v as usize].is_empty() {
                    // The function does not depend on v; moving it cannot change the size.
                    continue;
                }
                if !mgr.sift_variable(v, config) {
                    completed = false;
                    break;
                }
            }
        }

        let size_after = mgr.live;
        let (nodes, root) = mgr.export();
        let level_to_old_level: Vec<u16> = mgr.var_at_level.iter().map(|&v| v as u16).collect();
        SiftResult {
            nodes,
            root,
            level_to_old_level,
            swaps: mgr.swaps,
            size_before,
            size_after,
            completed,
        }
    }
}

/// Reference-counted, level-indirected node store used only during sifting.
struct Manager {
    /// Variable label per node (`TERMINAL_LABEL` for the terminals at ids 0 and 1).
    label: Vec<u32>,
    lo: Vec<u32>,
    hi: Vec<u32>,
    refs: Vec<u32>,
    free: Vec<u32>,
    /// Per-label unique subtable `(lo, hi) → id`; its length is the label's node count.
    unique: Vec<FxHashMap<(u32, u32), u32>>,
    level_of_var: Vec<u32>,
    var_at_level: Vec<u32>,
    root: u32,
    live: usize,
    swaps: usize,
}

impl Manager {
    fn import(bdd: &CompactBdd, n_vars: usize) -> Self {
        let n = bdd.nodes.len();
        let mut label = vec![TERMINAL_LABEL; n];
        let mut lo = vec![0u32; n];
        let mut hi = vec![0u32; n];
        let mut refs = vec![0u32; n];
        let mut unique: Vec<FxHashMap<(u32, u32), u32>> =
            (0..n_vars).map(|_| fx_map_with_capacity(0)).collect();
        let mut live = 0usize;

        for (id, node) in bdd.nodes.iter().enumerate().skip(2) {
            let v = node.var as u32;
            label[id] = v;
            lo[id] = node.lo;
            hi[id] = node.hi;
            refs[node.lo as usize] += 1;
            refs[node.hi as usize] += 1;
            unique[v as usize].insert((node.lo, node.hi), id as u32);
            live += 1;
        }
        refs[bdd.root as usize] += 1;

        Self {
            label,
            lo,
            hi,
            refs,
            free: Vec::new(),
            unique,
            level_of_var: (0..n_vars as u32).collect(),
            var_at_level: (0..n_vars as u32).collect(),
            root: bdd.root,
            live,
            swaps: 0,
        }
    }

    #[inline]
    fn is_terminal(&self, id: u32) -> bool {
        id == FALSE_ID || id == TRUE_ID
    }

    /// Find or create `(var, lo, hi)` and take one reference to the result.
    fn find_or_add(&mut self, var: u32, lo: u32, hi: u32) -> u32 {
        if lo == hi {
            self.refs[lo as usize] += 1;
            return lo;
        }
        if let Some(&id) = self.unique[var as usize].get(&(lo, hi)) {
            self.refs[id as usize] += 1;
            return id;
        }
        let id = match self.free.pop() {
            Some(id) => {
                let i = id as usize;
                self.label[i] = var;
                self.lo[i] = lo;
                self.hi[i] = hi;
                self.refs[i] = 1;
                id
            }
            None => {
                self.label.push(var);
                self.lo.push(lo);
                self.hi.push(hi);
                self.refs.push(1);
                (self.label.len() - 1) as u32
            }
        };
        self.refs[lo as usize] += 1;
        self.refs[hi as usize] += 1;
        self.unique[var as usize].insert((lo, hi), id);
        self.live += 1;
        id
    }

    /// Drop one reference to `id`, freeing every node whose count reaches zero.
    fn deref(&mut self, id: u32) {
        let mut stack = vec![id];
        while let Some(n) = stack.pop() {
            if self.is_terminal(n) {
                continue;
            }
            let i = n as usize;
            self.refs[i] -= 1;
            if self.refs[i] == 0 {
                let var = self.label[i] as usize;
                if self.unique[var].get(&(self.lo[i], self.hi[i])) == Some(&n) {
                    self.unique[var].remove(&(self.lo[i], self.hi[i]));
                }
                stack.push(self.lo[i]);
                stack.push(self.hi[i]);
                self.label[i] = TERMINAL_LABEL;
                self.free.push(n);
                self.live -= 1;
            }
        }
    }

    /// Swap the variables at levels `i` and `i + 1` in place.
    fn swap(&mut self, i: usize) {
        let a = self.var_at_level[i];
        let b = self.var_at_level[i + 1];

        // a-nodes with a b-labelled child are rewritten; the others just move down a level.
        let mut dependent: Vec<u32> = Vec::new();
        for &f in self.unique[a as usize].values() {
            let fi = f as usize;
            if self.label[self.lo[fi] as usize] == b || self.label[self.hi[fi] as usize] == b {
                dependent.push(f);
            }
        }
        for &f in &dependent {
            let fi = f as usize;
            let key = (self.lo[fi], self.hi[fi]);
            self.unique[a as usize].remove(&key);
        }

        for &f in &dependent {
            let fi = f as usize;
            let f0 = self.lo[fi];
            let f1 = self.hi[fi];
            let (f00, f01) = if self.label[f0 as usize] == b {
                (self.lo[f0 as usize], self.hi[f0 as usize])
            } else {
                (f0, f0)
            };
            let (f10, f11) = if self.label[f1 as usize] == b {
                (self.lo[f1 as usize], self.hi[f1 as usize])
            } else {
                (f1, f1)
            };
            // New children first (they reference the grandchildren), then release the old
            // children, so no grandchild is freed while still needed.
            let new_lo = self.find_or_add(a, f00, f10);
            let new_hi = self.find_or_add(a, f01, f11);
            self.label[fi] = b;
            self.lo[fi] = new_lo;
            self.hi[fi] = new_hi;
            self.unique[b as usize].insert((new_lo, new_hi), f);
            self.deref(f0);
            self.deref(f1);
        }

        self.var_at_level[i] = b;
        self.var_at_level[i + 1] = a;
        self.level_of_var[a as usize] = (i + 1) as u32;
        self.level_of_var[b as usize] = i as u32;
        self.swaps += 1;
    }

    /// True when the swap budget or the deadline is exhausted.
    fn out_of_budget(&self, config: &SiftingConfig) -> bool {
        if self.swaps >= config.max_swaps {
            return true;
        }
        match config.deadline {
            Some(d) => Instant::now() >= d,
            None => false,
        }
    }

    /// Sift variable `v`; returns false when the budget ended the pass (after `v` has been
    /// returned to its best level).
    fn sift_variable(&mut self, v: u32, config: &SiftingConfig) -> bool {
        let n = self.var_at_level.len();
        let start = self.level_of_var[v as usize] as usize;
        let mut best_size = self.live;
        let mut best_level = start;
        let mut level = start;
        let mut exhausted = false;

        // Visit the nearer end first, then the other end (Rudell).
        let up_first = start < n / 2;
        for pass in 0..2 {
            let going_up = (pass == 0) == up_first;
            loop {
                if exhausted || self.out_of_budget(config) {
                    exhausted = true;
                    break;
                }
                if going_up {
                    if level == 0 {
                        break;
                    }
                    self.swap(level - 1);
                    level -= 1;
                } else {
                    if level + 1 >= n {
                        break;
                    }
                    self.swap(level);
                    level += 1;
                }
                if self.live < best_size {
                    best_size = self.live;
                    best_level = level;
                } else if self.live as f64 > config.max_growth * best_size as f64 {
                    break;
                }
            }
            if exhausted {
                break;
            }
        }

        // Return to the best level (always, even when the budget is exhausted).
        while level > best_level {
            self.swap(level - 1);
            level -= 1;
        }
        while level < best_level {
            self.swap(level);
            level += 1;
        }
        !exhausted
    }

    /// Compact the root's sub-DAG (children before parents) with `var` = current level.
    fn export(&self) -> (Vec<ROBDDNode>, u32) {
        let mut out: Vec<ROBDDNode> = vec![ROBDDNode::false_terminal(), ROBDDNode::true_terminal()];
        if self.is_terminal(self.root) {
            return (out, self.root);
        }
        let mut new_id = vec![u32::MAX; self.label.len()];
        new_id[FALSE_ID as usize] = FALSE_ID;
        new_id[TRUE_ID as usize] = TRUE_ID;
        let mut stack: Vec<(u32, bool)> = vec![(self.root, false)];
        while let Some((id, children_done)) = stack.pop() {
            let i = id as usize;
            if new_id[i] != u32::MAX {
                continue;
            }
            if !children_done {
                stack.push((id, true));
                if new_id[self.hi[i] as usize] == u32::MAX {
                    stack.push((self.hi[i], false));
                }
                if new_id[self.lo[i] as usize] == u32::MAX {
                    stack.push((self.lo[i], false));
                }
            } else {
                let level = self.level_of_var[self.label[i] as usize] as u16;
                new_id[i] = out.len() as u32;
                out.push(ROBDDNode::internal(
                    level,
                    new_id[self.lo[i] as usize],
                    new_id[self.hi[i] as usize],
                ));
            }
        }
        let root = new_id[self.root as usize];
        (out, root)
    }
}
