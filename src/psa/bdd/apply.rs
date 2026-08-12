//! `apply(op, f, g)` — Shannon-expansion recursive AND/OR/XOR (§8.2.2).
//!
//! Computes the ROBDD of `f op g` in O(|f| × |g|) time via memoized Shannon expansion.
//!
//! **Shannon expansion:**
//! ```text
//! f op g = ITE(xᵢ, f|_{xᵢ=1} op g|_{xᵢ=1}, f|_{xᵢ=0} op g|_{xᵢ=0})
//! ```
//! where xᵢ is the smallest-index variable at the top of f or g.

use std::collections::HashMap;

use crate::psa::bdd::node::{ROBDDNode, FALSE_ID, TRUE_ID};
use crate::psa::bdd::unique_table::UniqueTable;
use crate::psa::types::BoolOp;

/// Compute `f op g` given ROBDDs `f` and `g` identified by root node IDs.
///
/// - `nodes`: read-only view of the current node array.
/// - `unique_table`: enforces both reduction rules via `make_node`.
/// - `all_nodes`: mutable node array for new node allocation.
/// - `cache`: memoization map `(min(f,g), max(f,g)) → result`.
pub fn apply(
    op: BoolOp,
    f: u32,
    g: u32,
    nodes: &[ROBDDNode],
    unique_table: &mut UniqueTable,
    all_nodes: &mut Vec<ROBDDNode>,
    cache: &mut HashMap<(u32, u32), u32>,
) -> u32 {
    // ── Terminal short-circuit cases ──────────────────────────────────────────
    match op {
        BoolOp::And => {
            if f == FALSE_ID || g == FALSE_ID { return FALSE_ID; }
            if f == TRUE_ID { return g; }
            if g == TRUE_ID { return f; }
        }
        BoolOp::Or => {
            if f == TRUE_ID || g == TRUE_ID { return TRUE_ID; }
            if f == FALSE_ID { return g; }
            if g == FALSE_ID { return f; }
        }
        BoolOp::Xor => {
            if f == FALSE_ID { return g; }
            if g == FALSE_ID { return f; }
            if f == TRUE_ID {
                return apply_not(g, nodes, unique_table, all_nodes, &mut HashMap::new());
            }
            if g == TRUE_ID {
                return apply_not(f, nodes, unique_table, all_nodes, &mut HashMap::new());
            }
        }
    }

    // f == g trivial cases
    if f == g {
        return match op {
            BoolOp::And | BoolOp::Or => f,
            BoolOp::Xor => FALSE_ID,
        };
    }

    // ── Memoization lookup (canonicalized key for commutativity) ──────────────
    let key = (f.min(g), f.max(g));
    if let Some(&cached) = cache.get(&key) {
        return cached;
    }

    // ── Shannon expansion: split on the smallest-index variable ──────────────
    if f as usize >= nodes.len() || g as usize >= nodes.len() {
        // Defensive: node IDs out of range — return FALSE as safe fallback.
        return FALSE_ID;
    }
    let fn_ = &nodes[f as usize];
    let gn_ = &nodes[g as usize];

    let (var, f_lo, f_hi, g_lo, g_hi) = if fn_.var == gn_.var {
        (fn_.var, fn_.lo, fn_.hi, gn_.lo, gn_.hi)
    } else if fn_.var < gn_.var {
        (fn_.var, fn_.lo, fn_.hi, g, g)
    } else {
        (gn_.var, f, f, gn_.lo, gn_.hi)
    };

    // Recurse on both cofactors.
    let lo = apply(op, f_lo, g_lo, nodes, unique_table, all_nodes, cache);
    let hi = apply(op, f_hi, g_hi, nodes, unique_table, all_nodes, cache);

    // Combine with make_node (enforces both reduction rules).
    let res = unique_table.make_node(var, lo, hi, all_nodes);
    cache.insert(key, res);
    res
}

/// Compute `¬f` (NOT) via Shannon expansion.
pub fn apply_not(
    f: u32,
    nodes: &[ROBDDNode],
    unique_table: &mut UniqueTable,
    all_nodes: &mut Vec<ROBDDNode>,
    cache: &mut HashMap<(u32, u32), u32>,
) -> u32 {
    if f == FALSE_ID { return TRUE_ID; }
    if f == TRUE_ID  { return FALSE_ID; }

    let key = (f, u32::MAX);
    if let Some(&cached) = cache.get(&key) { return cached; }

    if f as usize >= nodes.len() { return FALSE_ID; }

    let fn_ = &nodes[f as usize];
    let lo = apply_not(fn_.lo, nodes, unique_table, all_nodes, cache);
    let hi = apply_not(fn_.hi, nodes, unique_table, all_nodes, cache);
    let var = fn_.var;
    let res = unique_table.make_node(var, lo, hi, all_nodes);
    cache.insert(key, res);
    res
}

#[cfg(test)]
mod tests {
    use crate::psa::bdd::BDDLibrary;
    use crate::psa::types::BoolOp;
    use crate::psa::bdd::node::{FALSE_ID, TRUE_ID};

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
