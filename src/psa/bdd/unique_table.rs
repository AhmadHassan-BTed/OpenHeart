//! Unique Table — HashMap<(var, lo, hi), node_id> enforcing Rule 2 (sharing) during ROBDD
//! construction (§8.2.1).
//!
//! Before creating any new node, the BDD library checks whether an identical node already exists.
//! This is what mechanically enforces the Merging / Sharing reduction rule:
//! "If two nodes N and M have var(N)==var(M), lo(N)==lo(M), and hi(N)==hi(M), merge them."
//!
//! The Elimination reduction rule (Rule 1: lo==hi → return lo immediately) is checked before
//! consulting the unique table at all, so no eliminated node ever reaches this table.

use std::collections::HashMap;

use super::node::ROBDDNode;

/// The unique table maps `(var_id, lo_node_id, hi_node_id) → node_id`.
///
/// Queried via [`UniqueTable::get_or_insert`] which simultaneously enforces
/// both reduction rules for every node creation request.
pub struct UniqueTable {
    /// Core hash map: (var, lo, hi) → node_id.
    table: HashMap<(u16, u32, u32), u32>,
}

impl UniqueTable {
    /// Create an empty unique table (no terminals — they are pre-inserted by BDDLibrary::new).
    pub fn new() -> Self {
        Self {
            table: HashMap::new(),
        }
    }

    /// Create an empty unique table with a capacity hint for the expected number of nodes.
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            table: HashMap::with_capacity(cap),
        }
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
        // Both branches lead to the same result, so the variable doesn't affect the outcome.
        if lo == hi {
            return lo;
        }

        // Rule 2 (Sharing): check the unique table.
        let key = (var, lo, hi);
        if let Some(&existing) = self.table.get(&key) {
            return existing;
        }

        // Neither rule fires — allocate a new node.
        let new_id = nodes.len() as u32;
        nodes.push(ROBDDNode::internal(var, lo, hi));
        self.table.insert(key, new_id);
        new_id
    }

    /// Insert a pre-allocated terminal node entry (used during BDDLibrary initialization).
    ///
    /// Terminals are always `FALSE_ID=0` and `TRUE_ID=1`. They are inserted once at
    /// initialization and never re-checked; calling code must ensure they are the first
    /// two entries in the node array.
    pub fn insert_terminal(&mut self, var: u16, lo: u32, hi: u32, node_id: u32) {
        self.table.insert((var, lo, hi), node_id);
    }

    /// Returns the total number of live (non-terminal) nodes currently tracked.
    pub fn len(&self) -> usize {
        self.table.len()
    }

    /// Returns true if the table is empty.
    pub fn is_empty(&self) -> bool {
        self.table.is_empty()
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
        let mut ut = UniqueTable::new();
        let nodes: Vec<ROBDDNode> = vec![
            ROBDDNode::false_terminal(), // node_id=0 (FALSE)
            ROBDDNode::true_terminal(),  // node_id=1 (TRUE)
        ];
        // Register sentinels so they are found via normal (var,lo,hi) lookups if needed.
        ut.insert_terminal(0xFFFF, FALSE_ID, FALSE_ID, FALSE_ID);
        ut.insert_terminal(0xFFFF, TRUE_ID, TRUE_ID, TRUE_ID);
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
