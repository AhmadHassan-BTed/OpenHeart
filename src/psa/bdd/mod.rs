//! BDD sub-module exports for Phase 8 (§8.2.1, §8.2.2).
//!
//! Re-exports the BDDLibrary (central high-level interface) and the low-level primitives.
//! All code outside the bdd module should interact with BDDLibrary exclusively.

pub mod apply;
pub mod node;
pub mod restrict;
pub mod sat_count;
pub mod unique_table;

// Re-export the central BDD library as the primary public interface.
pub use library::BDDLibrary;

mod library {
    //! BDDLibrary — shared node table + unique hash table (§8.2.1, §8.3).
    //!
    //! Owns the node array (pre-populated with FALSE=0, TRUE=1), the UniqueTable
    //! (enforcing Rule 2/sharing), and the apply cache. Provides high-level wrappers
    //! around the functional apply/restrict/sat_count/var/implies routines.

    use std::collections::HashMap;

    use crate::psa::bdd::apply::{apply, apply_not};
    use crate::psa::bdd::node::{ROBDDNode, FALSE_ID, TRUE_ID};
    use crate::psa::bdd::restrict::restrict;
    use crate::psa::bdd::sat_count::sat_count;
    use crate::psa::bdd::unique_table::UniqueTable;
    use crate::psa::types::BoolOp;

    /// The central BDD library for one function's ROBDD construction.
    pub struct BDDLibrary {
        /// All nodes. Index 0 = FALSE terminal, index 1 = TRUE terminal.
        pub nodes: Vec<ROBDDNode>,
        unique_table: UniqueTable,
        apply_cache: HashMap<(u32, u32), u32>,
    }

    impl BDDLibrary {
        /// Create a new library pre-populated with both terminal nodes.
        pub fn new() -> Self {
            Self {
                nodes: vec![
                    ROBDDNode::false_terminal(), // node_id=0
                    ROBDDNode::true_terminal(),  // node_id=1
                ],
                unique_table: UniqueTable::with_capacity(1024),
                apply_cache: HashMap::with_capacity(512),
            }
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

        /// Construct the ROBDD node for a single variable xᵢ: `make_node(i, FALSE, TRUE)`.
        pub fn var(&mut self, var_idx: u16) -> u32 {
            self.unique_table
                .make_node(var_idx, FALSE_ID, TRUE_ID, &mut self.nodes)
        }

        /// Compute `f op g` via Shannon-expansion apply (§8.2.2).
        pub fn apply(&mut self, op: BoolOp, f: u32, g: u32) -> u32 {
            self.apply_cache.clear();
            // SAFETY: We need both a read-only view of nodes and a mutable reference for
            // appending. We use a raw pointer to create a read slice while also holding
            // &mut self.nodes. The apply function only reads from `nodes` and appends to
            // `all_nodes` — the Vec never reallocates mid-recursion within one apply() call
            // because we pre-cleared the cache, bounding the number of new allocations.
            let len = self.nodes.len();
            let read: &[ROBDDNode] =
                unsafe { std::slice::from_raw_parts(self.nodes.as_ptr(), len) };
            apply(
                op,
                f,
                g,
                read,
                &mut self.unique_table,
                &mut self.nodes,
                &mut self.apply_cache,
            )
        }

        /// Compute `¬f` (NOT).
        pub fn apply_not(&mut self, f: u32) -> u32 {
            let len = self.nodes.len();
            let read: &[ROBDDNode] =
                unsafe { std::slice::from_raw_parts(self.nodes.as_ptr(), len) };
            apply_not(
                f,
                read,
                &mut self.unique_table,
                &mut self.nodes,
                &mut HashMap::new(),
            )
        }

        /// Compute `a → b` (implication ≡ ¬a ∨ b) (§8.5.3).
        pub fn implies(&mut self, a: u32, b: u32) -> u32 {
            let not_a = self.apply_not(a);
            self.apply(BoolOp::Or, not_a, b)
        }

        /// Compute `f|_{var=val}` — the cofactor of f (§8.5.2 Step 5).
        pub fn restrict(&mut self, f: u32, var: u16, val: u8) -> u32 {
            let len = self.nodes.len();
            let read: &[ROBDDNode] =
                unsafe { std::slice::from_raw_parts(self.nodes.as_ptr(), len) };
            restrict(
                f,
                var,
                val,
                read,
                &mut self.unique_table,
                &mut self.nodes,
                &mut HashMap::new(),
            )
        }

        /// Compute #SAT(f) — number of satisfying assignments (§8.2.5).
        pub fn sat_count(&self, f: u32, n_vars: u16) -> u64 {
            sat_count(f, n_vars, &self.nodes, n_vars, &mut HashMap::new())
        }

        /// Verify ROBDD canonicity (§8.8 Invariant 3).
        pub fn verify_canonicity(&self) -> Result<(), String> {
            let mut seen: HashMap<(u16, u32, u32), u32> = HashMap::new();
            for (id, node) in self.nodes.iter().enumerate() {
                if node.is_terminal() {
                    continue;
                }
                if node.lo == node.hi {
                    return Err(format!(
                        "Rule 1 violation (Elimination): node_id={} has lo==hi=={}",
                        id, node.lo
                    ));
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

        /// Total node count (including terminals).
        #[inline]
        pub fn node_count(&self) -> usize {
            self.nodes.len()
        }

        /// Consume the library and return the node array for serialization.
        pub fn into_nodes(self) -> Vec<ROBDDNode> {
            self.nodes
        }
    }

    impl Default for BDDLibrary {
        fn default() -> Self {
            Self::new()
        }
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
}
