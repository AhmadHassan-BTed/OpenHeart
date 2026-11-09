//! BDD sub-module exports for Phase 8 (§8.2.1, §8.2.2).
//!
//! Re-exports the BDDLibrary (central high-level interface) and the low-level primitives.
//! All code outside the bdd module should interact with BDDLibrary exclusively, except for
//! the read-only counting/model functions, which also operate on node arrays loaded from a
//! `.psa` file.

pub mod apply;
pub mod fxhash;
pub mod library;
pub mod node;
pub mod restrict;
pub mod sat_count;
pub mod unique_table;

// Re-export the central BDD library as the primary public interface.
pub use library::{verify_node_array, BDDLibrary, BddLimit, BddLimits, CompactBdd};
pub use sat_count::{count_models, evaluate, find_model};
