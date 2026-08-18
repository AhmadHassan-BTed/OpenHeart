//! AST Reduction Adapter Trait and Classifier Types.

use crate::core::types::ast::ASTNodeType;
use tree_sitter::Node;

pub mod generic;
pub mod java;
pub mod registry;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReductionDecision {
    /// Emit an AST internal node. Children are recursed and attached as children.
    Keep(ASTNodeType),

    /// Do not emit a node for this CST node, but DO recurse into its children.
    /// Children are attached to the nearest kept ancestor.
    Eliminate,

    /// Do not emit a node. Do NOT recurse into children. Entire subtree is discarded.
    Drop,
}

pub trait ASTReductionAdapter: Send + Sync {
    /// Classify a CST node into Keep, Eliminate, or Drop.
    fn classify(&self, kind: &str, node: &Node, depth: usize) -> ReductionDecision;

    /// Encode the NodeAttr u32 for a kept node.
    fn encode_attrs(&self, kind: &str, node: &Node, source: &[u8]) -> u32;

    /// Returns the tree-sitter Language associated with this adapter.
    fn ts_language(&self) -> tree_sitter::Language;
}
