//! Backward Index Builders (§7.3).

pub mod ast_bi;
pub mod blk_bi;
pub mod cs_bi;
pub mod ssa_bi;
pub mod sym_bi;

pub use ast_bi::ASTBackwardIndex;
pub use blk_bi::{owning_span, BlockBackwardIndex, FunctionBlockRange};
pub use cs_bi::{CallSiteBackwardIndex, CallSiteIndexReport};
pub use ssa_bi::{FunctionSsaRange, SSABackwardIndex};
pub use sym_bi::SymbolBackwardIndex;
