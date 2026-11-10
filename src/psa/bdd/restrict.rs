//! `restrict(f, var, val)` — cofactor computation, and `exists(vars, f)` — existential
//! quantification (§8.2.2, §8.5.2).
//!
//! Given an ROBDD for function f and a variable xᵢ fixed to value `val` ∈ {0,1},
//! `restrict` computes the ROBDD for `f|_{xᵢ=val}` (the cofactor of f with xᵢ fixed).
//! Time: O(|f|) — one traversal with memoization.
//!
//! `exists` computes `∃ X. f = ⋁ over assignments of X of f|_X`, one variable at a time via
//! `f|_{x=0} ∨ f|_{x=1}`. Phase 8 uses it to project branch-predicate variables out of the
//! path formula after feasibility filtering, so that #SAT counts paths, not
//! (path, predicate valuation) pairs.
//!
//! Both run on explicit work stacks with per-call memo tables.

use super::fxhash::{fx_map_with_capacity, FxHashMap};
use super::library::{BDDLibrary, BddLimit};
use super::node::{FALSE_ID, TRUE_ID};
use crate::psa::types::BoolOp;

/// Work items shared by restrict and exists.
enum Frame {
    /// Process the sub-DAG rooted at this node and push its result.
    Expand(u32),
    /// Pop the hi and lo results of `source`, combine them, memoize and push.
    Combine {
        source: u32,
        var: u16,
        quantify: bool,
    },
}

#[inline]
fn pop_result(results: &mut Vec<u32>) -> u32 {
    match results.pop() {
        Some(r) => r,
        None => unreachable!("BDD work stack underflow: every Combine follows two results"),
    }
}

impl BDDLibrary {
    /// Iterative `f|_{var=val}`. With `enforce`, the library's limits apply.
    pub(super) fn restrict_core(
        &mut self,
        f: u32,
        var: u16,
        val: bool,
        enforce: bool,
    ) -> Result<u32, BddLimit> {
        self.assert_id(f);
        let mut memo: FxHashMap<u32, u32> = fx_map_with_capacity(64);
        let mut stack: Vec<Frame> = vec![Frame::Expand(f)];
        let mut results: Vec<u32> = Vec::new();

        while let Some(frame) = stack.pop() {
            match frame {
                Frame::Expand(n) => {
                    if n == FALSE_ID || n == TRUE_ID {
                        results.push(n);
                        continue;
                    }
                    if let Some(&cached) = memo.get(&n) {
                        results.push(cached);
                        continue;
                    }
                    self.tick(enforce)?;
                    let node = self.nodes[n as usize];
                    if node.var > var {
                        // Variable order: `var` cannot occur below this node.
                        memo.insert(n, n);
                        results.push(n);
                    } else if node.var == var {
                        // Children are strictly below `var`, so the cofactor is the child.
                        let child = if val { node.hi } else { node.lo };
                        memo.insert(n, child);
                        results.push(child);
                    } else {
                        stack.push(Frame::Combine {
                            source: n,
                            var: node.var,
                            quantify: false,
                        });
                        stack.push(Frame::Expand(node.hi));
                        stack.push(Frame::Expand(node.lo));
                    }
                }
                Frame::Combine { source, var: v, .. } => {
                    let hi = pop_result(&mut results);
                    let lo = pop_result(&mut results);
                    let res = self.mk(v, lo, hi, enforce)?;
                    memo.insert(source, res);
                    results.push(res);
                }
            }
        }

        Ok(pop_result(&mut results))
    }

    /// Iterative `∃ {v : quantified[v]}. f`. With `enforce`, the library's limits apply.
    pub(super) fn exists_core(
        &mut self,
        f: u32,
        quantified: &[bool],
        enforce: bool,
    ) -> Result<u32, BddLimit> {
        self.assert_id(f);
        // Variables below the deepest quantified one are untouched.
        let last_quantified = match quantified.iter().rposition(|&q| q) {
            Some(v) => v,
            None => return Ok(f),
        };

        let mut memo: FxHashMap<u32, u32> = fx_map_with_capacity(64);
        let mut stack: Vec<Frame> = vec![Frame::Expand(f)];
        let mut results: Vec<u32> = Vec::new();

        while let Some(frame) = stack.pop() {
            match frame {
                Frame::Expand(n) => {
                    if n == FALSE_ID || n == TRUE_ID {
                        results.push(n);
                        continue;
                    }
                    if let Some(&cached) = memo.get(&n) {
                        results.push(cached);
                        continue;
                    }
                    self.tick(enforce)?;
                    let node = self.nodes[n as usize];
                    if node.var as usize > last_quantified {
                        memo.insert(n, n);
                        results.push(n);
                        continue;
                    }
                    let quantify = quantified.get(node.var as usize).copied().unwrap_or(false);
                    stack.push(Frame::Combine {
                        source: n,
                        var: node.var,
                        quantify,
                    });
                    stack.push(Frame::Expand(node.hi));
                    stack.push(Frame::Expand(node.lo));
                }
                Frame::Combine {
                    source,
                    var,
                    quantify,
                } => {
                    let hi = pop_result(&mut results);
                    let lo = pop_result(&mut results);
                    let res = if quantify {
                        self.apply_core(BoolOp::Or, lo, hi, enforce)?
                    } else {
                        self.mk(var, lo, hi, enforce)?
                    };
                    memo.insert(source, res);
                    results.push(res);
                }
            }
        }

        Ok(pop_result(&mut results))
    }
}

#[cfg(test)]
mod tests {
    use crate::psa::bdd::node::{FALSE_ID, TRUE_ID};
    use crate::psa::bdd::BDDLibrary;

    #[test]
    fn restrict_terminal_false() {
        let mut lib = BDDLibrary::new();
        let r = lib.restrict(FALSE_ID, 2, 0);
        assert_eq!(r, FALSE_ID);
    }

    #[test]
    fn restrict_terminal_true() {
        let mut lib = BDDLibrary::new();
        let r = lib.restrict(TRUE_ID, 2, 1);
        assert_eq!(r, TRUE_ID);
    }

    #[test]
    fn restrict_var_to_one_gives_true() {
        let mut lib = BDDLibrary::new();
        let x2 = lib.var(2);
        // x2 restricted to 1 → follow hi branch → TRUE
        let r = lib.restrict(x2, 2, 1);
        assert_eq!(r, TRUE_ID);
    }

    #[test]
    fn restrict_var_to_zero_gives_false() {
        let mut lib = BDDLibrary::new();
        let x2 = lib.var(2);
        // x2 restricted to 0 → follow lo branch → FALSE
        let r = lib.restrict(x2, 2, 0);
        assert_eq!(r, FALSE_ID);
    }

    #[test]
    fn restrict_unrelated_var_is_noop() {
        let mut lib = BDDLibrary::new();
        let x2 = lib.var(2);
        // Restrict x5 (doesn't appear) — result should be x2 unchanged
        let r = lib.restrict(x2, 5, 1);
        assert_eq!(r, x2);
    }
}
