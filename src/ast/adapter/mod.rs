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

    /// Emit two nested AST nodes for one CST node: the first type wraps the second, and
    /// the CST children attach to the inner node. Used where the AST needs a node the
    /// grammar folds away (an instance initializer is a bare `block` in a class body;
    /// it becomes `NN_JAVA_INSTANCE_INIT` around an `NN_BLOCK`).
    Wrap(ASTNodeType, ASTNodeType),

    /// Do not emit a node for this CST node, but DO recurse into its children.
    /// Children are attached to the nearest kept ancestor. This is the default for any
    /// kind an adapter does not list, so unknown constructs never lose their contents.
    Eliminate,

    /// Do not emit a node. Do NOT recurse into children. Entire subtree is discarded.
    /// Reserved for an explicit allowlist of trivia (comments, punctuation-only wrappers,
    /// literal internals).
    Drop,
}

/// Where a CST node sits, for context-dependent classification.
#[derive(Debug, Clone, Copy)]
pub struct ReductionContext<'a> {
    /// CST kinds of the node's ancestors, root first (the last entry is the parent).
    pub ancestors: &'a [&'static str],
    /// Field name under which the node appears in its parent, if any.
    pub field_name: Option<&'static str>,
    /// Number of AST nodes currently open above this node.
    pub depth: usize,
}

impl<'a> ReductionContext<'a> {
    /// A context for a node with no known ancestors (a root, or a standalone call).
    pub fn root() -> ReductionContext<'static> {
        ReductionContext {
            ancestors: &[],
            field_name: None,
            depth: 0,
        }
    }

    #[inline]
    pub fn parent_kind(&self) -> Option<&'static str> {
        self.ancestors.last().copied()
    }

    #[inline]
    pub fn grandparent_kind(&self) -> Option<&'static str> {
        self.ancestor_kind(1)
    }

    /// Kind of the `n`-th ancestor above the parent (0 = parent).
    #[inline]
    pub fn ancestor_kind(&self, n: usize) -> Option<&'static str> {
        let len = self.ancestors.len();
        if n < len {
            Some(self.ancestors[len - 1 - n])
        } else {
            None
        }
    }
}

pub trait ASTReductionAdapter: Send + Sync {
    /// Classify a named CST node into Keep, Wrap, Eliminate, or Drop. Anonymous tokens and
    /// MISSING nodes never reach the adapter (the reducer drops them).
    fn classify(&self, kind: &str, node: &Node, ctx: &ReductionContext) -> ReductionDecision;

    /// Encode the NodeAttr u32 for a kept node.
    fn encode_attrs(&self, kind: &str, node: &Node, source: &[u8], ctx: &ReductionContext)
        -> u32;

    /// Returns the tree-sitter Language associated with this adapter.
    fn ts_language(&self) -> tree_sitter::Language;

    /// The CST node holding the identifier that names a kept node (declaration name,
    /// declarator or parameter name, invoked method name). Its first token is recorded in
    /// the AST's name-token side array.
    fn name_node<'tree>(&self, _kind: &str, _node: &Node<'tree>) -> Option<Node<'tree>> {
        None
    }
}

/// Direct children of `node`, collected with one cursor.
pub(crate) fn children_of<'tree>(node: &Node<'tree>) -> Vec<Node<'tree>> {
    let mut cursor = node.walk();
    node.children(&mut cursor).collect()
}

/// First direct child of the given kind.
pub(crate) fn child_of_kind<'tree>(node: &Node<'tree>, kind: &str) -> Option<Node<'tree>> {
    let mut cursor = node.walk();
    let found = node.children(&mut cursor).find(|c| c.kind() == kind);
    found
}
