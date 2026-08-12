//! ROBDD node representation (12 bytes, cache-line efficient — 5.3 nodes per 64-byte cache line).
//!
//! Layout (§8.4):
//! ```text
//! Offset  Size  Field
//!  0       2    var         : u16   variable index in ordering; 0xFFFF = terminal node
//!  2       2    _flags      : u16   IS_FALSE_TERMINAL:1, IS_TRUE_TERMINAL:1, reserved:14
//!  4       4    lo          : u32   node_id of lo (var=0) child; 0=FALSE, 1=TRUE
//!  8       4    hi          : u32   node_id of hi (var=1) child; 0=FALSE, 1=TRUE
//! Total: 12 bytes
//! ```
//!
//! `node_id=0` is always the `FALSE` terminal.
//! `node_id=1` is always the `TRUE` terminal.
//! Both are pre-populated at BDD library initialization.
//! All function-specific nodes start at `node_id=2`.

/// The sentinel node_id for the FALSE terminal.
pub const FALSE_ID: u32 = 0;

/// The sentinel node_id for the TRUE terminal.
pub const TRUE_ID: u32 = 1;

/// Bitmask for the IS_FALSE_TERMINAL flag in `_flags`.
pub const FLAG_FALSE_TERMINAL: u16 = 0b0000_0000_0000_0001;

/// Bitmask for the IS_TRUE_TERMINAL flag in `_flags`.
pub const FLAG_TRUE_TERMINAL: u16 = 0b0000_0000_0000_0010;

/// The `var` field value used for terminal nodes (0xFFFF — no variable).
pub const TERMINAL_VAR: u16 = 0xFFFF;

/// A single ROBDD node.
///
/// 12 bytes — fits 5.3 nodes per 64-byte cache line. Every internal node has a
/// variable `var(N)` ∈ {x₁,...,xₘ} that respects the total ordering constraint:
/// if node N has variable xᵢ and child C has variable xⱼ, then i < j.
///
/// Two reduction rules are mechanically enforced during construction:
/// - **Rule 1 (Elimination):** `lo == hi` ⟹ node is deleted (both branches are identical).
/// - **Rule 2 (Sharing):** `(var, lo, hi)` collision ⟹ existing node is returned (unique table).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct ROBDDNode {
    /// Variable index in the current ordering. `TERMINAL_VAR` (0xFFFF) for terminal nodes.
    pub var: u16,

    /// Flags:
    /// - bit 0: IS_FALSE_TERMINAL
    /// - bit 1: IS_TRUE_TERMINAL
    /// - bits 2-15: reserved (must be zero)
    pub flags: u16,

    /// node_id of the lo (var=0) child. `FALSE_ID` or `TRUE_ID` for terminals.
    pub lo: u32,

    /// node_id of the hi (var=1) child. `FALSE_ID` or `TRUE_ID` for terminals.
    pub hi: u32,
}

impl ROBDDNode {
    /// Construct the pre-populated FALSE terminal node (always node_id=0).
    #[inline]
    pub fn false_terminal() -> Self {
        Self {
            var: TERMINAL_VAR,
            flags: FLAG_FALSE_TERMINAL,
            lo: FALSE_ID,
            hi: FALSE_ID,
        }
    }

    /// Construct the pre-populated TRUE terminal node (always node_id=1).
    #[inline]
    pub fn true_terminal() -> Self {
        Self {
            var: TERMINAL_VAR,
            flags: FLAG_TRUE_TERMINAL,
            lo: TRUE_ID,
            hi: TRUE_ID,
        }
    }

    /// Construct a regular internal ROBDD node.
    #[inline]
    pub fn internal(var: u16, lo: u32, hi: u32) -> Self {
        Self {
            var,
            flags: 0,
            lo,
            hi,
        }
    }

    /// Returns true iff this node is the FALSE terminal.
    #[inline]
    pub fn is_false(&self) -> bool {
        self.flags & FLAG_FALSE_TERMINAL != 0
    }

    /// Returns true iff this node is the TRUE terminal.
    #[inline]
    pub fn is_true(&self) -> bool {
        self.flags & FLAG_TRUE_TERMINAL != 0
    }

    /// Returns true iff this node is any terminal (FALSE or TRUE).
    #[inline]
    pub fn is_terminal(&self) -> bool {
        self.var == TERMINAL_VAR
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn node_is_exactly_12_bytes() {
        assert_eq!(size_of::<ROBDDNode>(), 12, "ROBDDNode must be exactly 12 bytes for cache-line efficiency");
    }

    #[test]
    fn false_terminal_id_is_zero() {
        assert_eq!(FALSE_ID, 0);
    }

    #[test]
    fn true_terminal_id_is_one() {
        assert_eq!(TRUE_ID, 1);
    }

    #[test]
    fn false_terminal_flags() {
        let n = ROBDDNode::false_terminal();
        assert!(n.is_false());
        assert!(!n.is_true());
        assert!(n.is_terminal());
    }

    #[test]
    fn true_terminal_flags() {
        let n = ROBDDNode::true_terminal();
        assert!(n.is_true());
        assert!(!n.is_false());
        assert!(n.is_terminal());
    }

    #[test]
    fn internal_node_not_terminal() {
        let n = ROBDDNode::internal(3, FALSE_ID, TRUE_ID);
        assert!(!n.is_terminal());
        assert!(!n.is_false());
        assert!(!n.is_true());
        assert_eq!(n.var, 3);
        assert_eq!(n.lo, FALSE_ID);
        assert_eq!(n.hi, TRUE_ID);
    }
}
