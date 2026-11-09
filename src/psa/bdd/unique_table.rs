//! Unique Table — `(var, lo, hi) → node_id` enforcing Rule 2 (sharing) during ROBDD
//! construction (§8.2.1).
//!
//! Before creating any new node, the BDD library checks whether an identical node already exists.
//! This is what mechanically enforces the Merging / Sharing reduction rule:
//! "If two nodes N and M have var(N)==var(M), lo(N)==lo(M), and hi(N)==hi(M), merge them."
//!
//! The Elimination reduction rule (Rule 1: lo==hi → return lo immediately) is checked before
//! consulting the unique table at all, so no eliminated node ever reaches this table.
//!
//! Nodes are only ever appended, and always after both of their children exist, so every
//! internal node's id is strictly greater than its children's ids. The library relies on
//! this to keep the node store acyclic and to verify it cheaply.

use super::fxhash::{fx_map_with_capacity, FxHashMap};
use super::node::{ROBDDNode, TERMINAL_VAR};

/// The unique table maps `(var_id, lo_node_id, hi_node_id) → node_id` for internal nodes.
///
/// Terminals are never registered: they are the fixed ids 0 and 1 of the node store, and
/// no `(var, lo, hi)` triple with `lo != hi` can denote them.
pub struct UniqueTable {
    /// Core hash map: (var, lo, hi) → node_id.
    table: FxHashMap<(u16, u32, u32), u32>,
}

impl UniqueTable {
    /// Create an empty unique table.
    pub fn new() -> Self {
        Self {
            table: fx_map_with_capacity(0),
        }
    }

    /// Create an empty unique table with a capacity hint for the expected number of nodes.
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            table: fx_map_with_capacity(cap),
        }
    }

    /// Look up an existing node `(var, lo, hi)` without allocating.
    #[inline]
    pub fn lookup(&self, var: u16, lo: u32, hi: u32) -> Option<u32> {
        self.table.get(&(var, lo, hi)).copied()
    }

    /// Append a node that the caller has verified is neither redundant (`lo != hi`) nor
    /// already present (`lookup` returned `None`), and register it.
    ///
    /// # Panics
    /// If `var` is [`TERMINAL_VAR`] (the label reserved for the terminal records), or if the
    /// store already holds `u32::MAX` nodes (ids must stay below the `u32::MAX` sentinel).
    #[inline]
    pub fn insert_new(&mut self, var: u16, lo: u32, hi: u32, nodes: &mut Vec<ROBDDNode>) -> u32 {
        assert!(
            var != TERMINAL_VAR,
            "BDD variable index 0xFFFF is reserved for terminal nodes"
        );
        debug_assert_ne!(lo, hi, "insert_new called for a redundant node");
        let new_id = match u32::try_from(nodes.len()) {
            Ok(id) if id != u32::MAX => id,
            _ => panic!("BDD node store exhausted the 32-bit node id space"),
        };
        nodes.push(ROBDDNode::internal(var, lo, hi));
        self.table.insert((var, lo, hi), new_id);
        new_id
    }

    /// The core `make_node` logic — enforces both ROBDD reduction rules.
    ///
    /// Given `(var, lo, hi)` and a mutable node vec, returns the node_id for the ROBDD
    /// node representing `ITE(x_var, hi_sub_function, lo_sub_function)`.
    ///
    /// - **Rule 1 (Elimination):** If `lo == hi`, neither a new node nor a table entry is
    ///   created — `lo` is returned directly. The variable is redundant.
    /// - **Rule 2 (Sharing):** If a node with the same `(var, lo, hi)` already exists,
    ///   its `node_id` is returned without allocating a new node.
    ///
    /// Only if no existing node matches is a new one pushed onto `nodes`.
    pub fn make_node(&mut self, var: u16, lo: u32, hi: u32, nodes: &mut Vec<ROBDDNode>) -> u32 {
        // Rule 1 (Elimination): lo == hi ⟹ this node is redundant.
        if lo == hi {
            return lo;
        }
        // Rule 2 (Sharing): check the unique table.
        if let Some(existing) = self.lookup(var, lo, hi) {
            return existing;
        }
        // Neither rule fires — allocate a new node.
        self.insert_new(var, lo, hi, nodes)
    }
}

impl Default for UniqueTable {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::psa::bdd::node::{FALSE_ID, TRUE_ID};

    fn make_lib() -> (UniqueTable, Vec<ROBDDNode>) {
        let ut = UniqueTable::new();
        let nodes: Vec<ROBDDNode> = vec![
            ROBDDNode::false_terminal(), // node_id=0 (FALSE)
            ROBDDNode::true_terminal(),  // node_id=1 (TRUE)
        ];
        (ut, nodes)
    }

    #[test]
    fn rule1_elimination_lo_eq_hi() {
        let (mut ut, mut nodes) = make_lib();
        let result = ut.make_node(3, TRUE_ID, TRUE_ID, &mut nodes);
        // Rule 1: lo == hi → return lo without allocating
        assert_eq!(result, TRUE_ID);
        // No extra nodes allocated (still just 2 terminals)
        assert_eq!(nodes.len(), 2);
    }

    #[test]
    fn rule2_sharing_deduplicates() {
        let (mut ut, mut nodes) = make_lib();
        let first = ut.make_node(2, FALSE_ID, TRUE_ID, &mut nodes);
        assert_eq!(nodes.len(), 3, "first call should allocate one node");

        let second = ut.make_node(2, FALSE_ID, TRUE_ID, &mut nodes);
        assert_eq!(nodes.len(), 3, "second call must not allocate");
        assert_eq!(first, second, "same (var,lo,hi) must return same node_id");
    }

    #[test]
    fn distinct_nodes_both_allocated() {
        let (mut ut, mut nodes) = make_lib();
        let a = ut.make_node(1, FALSE_ID, TRUE_ID, &mut nodes);
        let b = ut.make_node(2, FALSE_ID, TRUE_ID, &mut nodes);
        assert_ne!(a, b);
        assert_eq!(nodes.len(), 4);
    }
}
