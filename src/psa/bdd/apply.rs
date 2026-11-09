//! `apply(op, f, g)` — Shannon-expansion AND/OR/XOR and `not(f)` (§8.2.2).
//!
//! Computes the ROBDD of `f op g` in O(|f| × |g|) time via memoized Shannon expansion.
//!
//! **Shannon expansion:**
//! ```text
//! f op g = ITE(xᵢ, f|_{xᵢ=1} op g|_{xᵢ=1}, f|_{xᵢ=0} op g|_{xᵢ=0})
//! ```
//! where xᵢ is the smallest-index variable at the top of f or g.
//!
//! Both operations run on an explicit work stack, so their stack depth is O(1) regardless of
//! the number of variables, and both read nodes by value before allocating, so no borrow of
//! the node store is held across a push.

use super::library::{BDDLibrary, BddLimit};
use super::node::{FALSE_ID, TRUE_ID};
use crate::psa::types::BoolOp;

/// Stable computed-table code of each operation.
#[inline]
fn op_code(op: BoolOp) -> u8 {
    match op {
        BoolOp::And => 0,
        BoolOp::Or => 1,
        BoolOp::Xor => 2,
    }
}

/// Work items of the iterative Shannon expansion.
enum ApplyFrame {
    /// Compute `f op g` and push the result.
    Expand(u32, u32),
    /// Pop the hi and lo results, build `(var, lo, hi)`, memoize it under `key`, push it.
    Combine { var: u16, key: (u8, u32, u32) },
}

/// Work items of the iterative negation.
enum NotFrame {
    Expand(u32),
    Combine { source: u32, var: u16 },
}

#[inline]
fn pop_result(results: &mut Vec<u32>) -> u32 {
    match results.pop() {
        Some(r) => r,
        None => unreachable!("BDD work stack underflow: every Combine follows two results"),
    }
}

impl BDDLibrary {
    /// Iterative `f op g`. With `enforce`, the library's limits apply.
    pub(super) fn apply_core(
        &mut self,
        op: BoolOp,
        f: u32,
        g: u32,
        enforce: bool,
    ) -> Result<u32, BddLimit> {
        self.assert_id(f);
        self.assert_id(g);

        let mut stack: Vec<ApplyFrame> = vec![ApplyFrame::Expand(f, g)];
        let mut results: Vec<u32> = Vec::new();

        while let Some(frame) = stack.pop() {
            match frame {
                ApplyFrame::Expand(f, g) => {
                    if let Some(r) = self.apply_terminal_case(op, f, g, enforce)? {
                        results.push(r);
                        continue;
                    }

                    // All three operations are commutative: canonicalize the key.
                    let key = (op_code(op), f.min(g), f.max(g));
                    if let Some(&cached) = self.computed.get(&key) {
                        results.push(cached);
                        continue;
                    }
                    self.tick(enforce)?;

                    // Copy both nodes out of the store before anything is allocated.
                    let fnode = self.nodes[f as usize];
                    let gnode = self.nodes[g as usize];
                    let var = fnode.var.min(gnode.var);
                    let (f_lo, f_hi) = if fnode.var == var {
                        (fnode.lo, fnode.hi)
                    } else {
                        (f, f)
                    };
                    let (g_lo, g_hi) = if gnode.var == var {
                        (gnode.lo, gnode.hi)
                    } else {
                        (g, g)
                    };

                    // Evaluation order: lo, then hi, then Combine.
                    stack.push(ApplyFrame::Combine { var, key });
                    stack.push(ApplyFrame::Expand(f_hi, g_hi));
                    stack.push(ApplyFrame::Expand(f_lo, g_lo));
                }
                ApplyFrame::Combine { var, key } => {
                    let hi = pop_result(&mut results);
                    let lo = pop_result(&mut results);
                    let res = self.mk(var, lo, hi, enforce)?;
                    self.remember_computed(key, res);
                    results.push(res);
                }
            }
        }

        Ok(pop_result(&mut results))
    }

    /// Terminal short-circuit cases of `f op g`; `None` when Shannon expansion is needed.
    fn apply_terminal_case(
        &mut self,
        op: BoolOp,
        f: u32,
        g: u32,
        enforce: bool,
    ) -> Result<Option<u32>, BddLimit> {
        let r = match op {
            BoolOp::And => {
                if f == FALSE_ID || g == FALSE_ID {
                    Some(FALSE_ID)
                } else if f == TRUE_ID {
                    Some(g)
                } else if g == TRUE_ID || f == g {
                    Some(f)
                } else {
                    None
                }
            }
            BoolOp::Or => {
                if f == TRUE_ID || g == TRUE_ID {
                    Some(TRUE_ID)
                } else if f == FALSE_ID {
                    Some(g)
                } else if g == FALSE_ID || f == g {
                    Some(f)
                } else {
                    None
                }
            }
            BoolOp::Xor => {
                if f == g {
                    Some(FALSE_ID)
                } else if f == FALSE_ID {
                    Some(g)
                } else if g == FALSE_ID {
                    Some(f)
                } else if f == TRUE_ID {
                    Some(self.not_core(g, enforce)?)
                } else if g == TRUE_ID {
                    Some(self.not_core(f, enforce)?)
                } else {
                    None
                }
            }
        };
        Ok(r)
    }

    /// Iterative `¬f`. With `enforce`, the library's limits apply.
    pub(super) fn not_core(&mut self, f: u32, enforce: bool) -> Result<u32, BddLimit> {
        self.assert_id(f);

        let mut stack: Vec<NotFrame> = vec![NotFrame::Expand(f)];
        let mut results: Vec<u32> = Vec::new();

        while let Some(frame) = stack.pop() {
            match frame {
                NotFrame::Expand(n) => {
                    if n == FALSE_ID {
                        results.push(TRUE_ID);
                        continue;
                    }
                    if n == TRUE_ID {
                        results.push(FALSE_ID);
                        continue;
                    }
                    if let Some(&cached) = self.not_cache.get(&n) {
                        results.push(cached);
                        continue;
                    }
                    self.tick(enforce)?;
                    let node = self.nodes[n as usize];
                    stack.push(NotFrame::Combine {
                        source: n,
                        var: node.var,
                    });
                    stack.push(NotFrame::Expand(node.hi));
                    stack.push(NotFrame::Expand(node.lo));
                }
                NotFrame::Combine { source, var } => {
                    let hi = pop_result(&mut results);
                    let lo = pop_result(&mut results);
                    let res = self.mk(var, lo, hi, enforce)?;
                    self.remember_not(source, res);
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
    use crate::psa::types::BoolOp;

    #[test]
    fn and_false_is_false() {
        let mut lib = BDDLibrary::new();
        let x = lib.var(0);
        let r = lib.apply(BoolOp::And, FALSE_ID, x);
        assert_eq!(r, FALSE_ID);
    }

    #[test]
    fn or_true_is_true() {
        let mut lib = BDDLibrary::new();
        let x = lib.var(0);
        let r = lib.apply(BoolOp::Or, TRUE_ID, x);
        assert_eq!(r, TRUE_ID);
    }

    #[test]
    fn xor_same_is_false() {
        let mut lib = BDDLibrary::new();
        let r = lib.apply(BoolOp::Xor, FALSE_ID, FALSE_ID);
        assert_eq!(r, FALSE_ID);
    }

    #[test]
    fn not_false_is_true() {
        let mut lib = BDDLibrary::new();
        let r = lib.apply_not(FALSE_ID);
        assert_eq!(r, TRUE_ID);
    }

    #[test]
    fn not_true_is_false() {
        let mut lib = BDDLibrary::new();
        let r = lib.apply_not(TRUE_ID);
        assert_eq!(r, FALSE_ID);
    }
}
