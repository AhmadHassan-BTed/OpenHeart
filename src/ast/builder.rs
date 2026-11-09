//! BPASTBuilder aggregating BPEncoder, PreorderArrays, and building auxiliary structures.
//!
//! Navigation is answered by the BP bitstring plus two o(n)-bit indexes: rank/select
//! (`RankSelectIndex`) and the range min-max tree (`RangeMinMaxTree`). The former
//! per-bit jump table (32 bits per parenthesis) and the O(n log n) sparse table are gone;
//! `heap_bytes` reports what the structure actually holds.

use super::bp_encoder::BPEncoder;
use super::preorder::PreorderArrays;
use super::rank_select::RankSelectIndex;
use super::rmq::RangeMinMaxTree;
use crate::core::types::ast::ASTNodeType;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BPASTArtifact {
    pub node_count: u32,
    pub bp_encoder: BPEncoder,
    pub rank_select: RankSelectIndex,
    pub rmq: RangeMinMaxTree,
    pub preorder: PreorderArrays,
    pub tca_hash: u64,
}

/// Byte accounting of a `BPASTArtifact` in memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BpastMemoryUsage {
    pub bp_bits_bytes: usize,
    pub rank_select_bytes: usize,
    pub range_min_max_bytes: usize,
    pub preorder_bytes: usize,
}

impl BpastMemoryUsage {
    pub fn total(&self) -> usize {
        self.bp_bits_bytes + self.rank_select_bytes + self.range_min_max_bytes + self.preorder_bytes
    }

    /// Topology overhead (BP + navigation indexes) in bits per node.
    pub fn topology_bits_per_node(&self, node_count: u32) -> Option<f64> {
        if node_count == 0 {
            return None;
        }
        let bytes = self.bp_bits_bytes + self.rank_select_bytes + self.range_min_max_bytes;
        Some(bytes as f64 * 8.0 / node_count as f64)
    }
}

impl BPASTArtifact {
    #[inline]
    pub fn node_type(&self, preorder_idx: u32) -> ASTNodeType {
        if (preorder_idx as usize) < self.preorder.node_types.len() {
            self.preorder.node_types[preorder_idx as usize].into()
        } else {
            ASTNodeType::NN_UNKNOWN
        }
    }

    #[inline]
    pub fn node_attr(&self, preorder_idx: u32) -> u32 {
        if (preorder_idx as usize) < self.preorder.node_attrs.len() {
            self.preorder.node_attrs[preorder_idx as usize]
        } else {
            0
        }
    }

    #[inline]
    pub fn token_range(&self, preorder_idx: u32) -> (u32, u32) {
        if (preorder_idx as usize) < self.preorder.token_ranges.len() {
            self.preorder.token_ranges[preorder_idx as usize]
        } else {
            (u32::MAX, 0)
        }
    }

    #[inline]
    pub fn parent(&self, preorder_idx: u32) -> u32 {
        if (preorder_idx as usize) < self.preorder.parent_map.len() {
            self.preorder.parent_map[preorder_idx as usize]
        } else {
            u32::MAX
        }
    }

    /// Token id of the identifier that names this node (declaration name, declarator or
    /// parameter name, invoked method name), when Phase 2 recorded one.
    #[inline]
    pub fn name_token(&self, preorder_idx: u32) -> Option<u32> {
        self.preorder
            .name_tokens
            .get(preorder_idx as usize)
            .copied()
            .filter(|&t| t != u32::MAX)
    }

    /// Position of the node's open parenthesis.
    #[inline]
    pub fn open_position(&self, preorder_idx: u32) -> Option<usize> {
        if preorder_idx >= self.node_count {
            return None;
        }
        self.rank_select
            .select1_checked(&self.bp_encoder, preorder_idx + 1)
    }

    /// Position of the node's close parenthesis.
    pub fn close_position(&self, preorder_idx: u32) -> Option<usize> {
        let open = self.open_position(preorder_idx)?;
        self.rmq
            .find_close(&self.bp_encoder, &self.rank_select, open)
    }

    pub fn first_child(&self, preorder_idx: u32) -> Option<u32> {
        let open = self.open_position(preorder_idx)?;
        if open + 1 < self.bp_encoder.bit_count && self.bp_encoder.get_bit(open + 1) == 1 {
            // In pre-order, a node's first child is the very next node.
            Some(preorder_idx + 1)
        } else {
            None
        }
    }

    /// Next sibling in pre-order. Top-level trees of the forest (one per file) are
    /// siblings of each other.
    pub fn next_sibling(&self, preorder_idx: u32) -> Option<u32> {
        let open = self.open_position(preorder_idx)?;
        let close = self
            .rmq
            .find_close(&self.bp_encoder, &self.rank_select, open)?;
        if close + 1 < self.bp_encoder.bit_count && self.bp_encoder.get_bit(close + 1) == 1 {
            let size = ((close - open + 1) / 2) as u32;
            Some(preorder_idx + size)
        } else {
            None
        }
    }

    /// Lowest common ancestor by pre-order index, or `u32::MAX` when the nodes belong to
    /// different trees (files) or an index is out of range.
    pub fn lca(&self, u: u32, v: u32) -> u32 {
        if u >= self.node_count || v >= self.node_count {
            return u32::MAX;
        }
        if u == v {
            return u;
        }
        let (Some(pu), Some(pv)) = (self.open_position(u), self.open_position(v)) else {
            return u32::MAX;
        };
        let (x, y, x_node) = if pu < pv { (pu, pv, u) } else { (pv, pu, v) };
        match self
            .rmq
            .find_close(&self.bp_encoder, &self.rank_select, x)
        {
            Some(close) if close > y => return x_node,
            Some(_) => {}
            None => return u32::MAX,
        }
        let Some(m) = self.rmq.rmq(&self.bp_encoder, &self.rank_select, x, y) else {
            return u32::MAX;
        };
        // The node opening right after the minimum is a child of the LCA.
        let child = self
            .rank_select
            .rank1(&self.bp_encoder, m + 1)
            .saturating_sub(1);
        self.parent(child)
    }

    /// Number of nodes in the subtree rooted at `pre_idx` (including it); 0 for an
    /// index outside the tree.
    pub fn subtree_size(&self, pre_idx: u32) -> u32 {
        match (self.open_position(pre_idx), self.close_position(pre_idx)) {
            (Some(open), Some(close)) if close > open => ((close - open + 1) / 2) as u32,
            _ => 0,
        }
    }

    pub fn is_leaf(&self, pre_idx: u32) -> bool {
        match self.open_position(pre_idx) {
            Some(open) if open + 1 < self.bp_encoder.bit_count => {
                self.bp_encoder.get_bit(open + 1) == 0
            }
            _ => true,
        }
    }

    /// 1-based depth: a top-level (file) node has depth 1. 0 for an index outside the tree.
    pub fn depth(&self, pre_idx: u32) -> u32 {
        match self.open_position(pre_idx) {
            Some(open) => {
                let excess = RangeMinMaxTree::excess_at(&self.bp_encoder, &self.rank_select, open);
                u32::try_from(excess).unwrap_or(0)
            }
            None => 0,
        }
    }

    pub fn memory_usage(&self) -> BpastMemoryUsage {
        BpastMemoryUsage {
            bp_bits_bytes: self.bp_encoder.words.len() * 8,
            rank_select_bytes: self.rank_select.heap_bytes(),
            range_min_max_bytes: self.rmq.heap_bytes(),
            preorder_bytes: self.preorder.heap_bytes(),
        }
    }

    /// Number of top-level trees (one per reduced file).
    pub fn root_count(&self) -> u32 {
        self.preorder
            .parent_map
            .iter()
            .filter(|&&p| p == u32::MAX)
            .count() as u32
    }

    /// Structural validation used by readers: balanced parentheses, array lengths that
    /// match the node count, parents that precede their children.
    pub fn validate(&self) -> Result<(), String> {
        let n = self.node_count as usize;
        if self.bp_encoder.bit_count != 2 * n {
            return Err(format!(
                "bit count {} != 2 x node count {}",
                self.bp_encoder.bit_count, n
            ));
        }
        if self.bp_encoder.words.len() < self.bp_encoder.bit_count.div_ceil(64) {
            return Err(format!(
                "{} BP words cannot hold {} bits",
                self.bp_encoder.words.len(),
                self.bp_encoder.bit_count
            ));
        }
        if self.rank_select.ones as usize != n {
            return Err(format!(
                "{} open parentheses for {} nodes",
                self.rank_select.ones, n
            ));
        }
        self.rmq.validate(&self.rank_select)?;
        let lens = [
            ("node_types", self.preorder.node_types.len()),
            ("node_attrs", self.preorder.node_attrs.len()),
            ("token_ranges", self.preorder.token_ranges.len()),
            ("parent_map", self.preorder.parent_map.len()),
            ("name_tokens", self.preorder.name_tokens.len()),
        ];
        for (name, len) in lens {
            if len != n {
                return Err(format!("{} has {} entries for {} nodes", name, len, n));
            }
        }
        for (i, &p) in self.preorder.parent_map.iter().enumerate() {
            if p != u32::MAX && p as usize >= i {
                return Err(format!("node {} has parent {} that does not precede it", i, p));
            }
        }
        Ok(())
    }
}

pub struct BPASTBuilder {
    pub bp: BPEncoder,
    pub preorder: PreorderArrays,
    pub open_stack: Vec<u32>,
    pub node_count: usize,
    pub tca_hash: u64,
}

impl BPASTBuilder {
    pub fn new(estimated_nodes: usize, tca_hash: u64) -> Self {
        Self {
            bp: BPEncoder::with_node_capacity(estimated_nodes),
            preorder: PreorderArrays::with_capacity(estimated_nodes),
            open_stack: Vec::with_capacity(256),
            node_count: 0,
            tca_hash,
        }
    }

    pub fn current_depth(&self) -> usize {
        self.open_stack.len()
    }

    pub fn open_node(&mut self, node_type: ASTNodeType, attrs: u32) -> u32 {
        let preorder_idx = self.node_count as u32;
        let parent_idx = self.open_stack.last().copied().unwrap_or(u32::MAX);

        self.preorder.node_types.push(node_type as u8);
        self.preorder.node_attrs.push(attrs);
        self.preorder.token_ranges.push((u32::MAX, 0));
        self.preorder.parent_map.push(parent_idx);
        self.preorder.name_tokens.push(u32::MAX);

        self.bp.push_open();
        self.open_stack.push(preorder_idx);
        self.node_count += 1;

        preorder_idx
    }

    /// Records the token naming an open or closed node.
    pub fn set_name_token(&mut self, preorder_idx: u32, token_id: u32) {
        if let Some(slot) = self.preorder.name_tokens.get_mut(preorder_idx as usize) {
            *slot = token_id;
        }
    }

    /// Closes the innermost open node. `first_tok..=last_tok` must be the node's full token
    /// span (`(u32::MAX, 0)` for a node without tokens); ancestors are not widened, since
    /// each node's span is computed from its own source range.
    pub fn close_node(&mut self, preorder_idx: u32, first_tok: u32, last_tok: u32) {
        debug_assert_eq!(
            self.open_stack.last().copied(),
            Some(preorder_idx),
            "close_node must close the innermost open node"
        );
        if let Some(r) = self.preorder.token_ranges.get_mut(preorder_idx as usize) {
            *r = (first_tok, last_tok);
        }
        self.bp.push_close();
        self.open_stack.pop();
    }

    /// Builds the navigation indexes and validates the result.
    pub fn try_finalize(self) -> Result<BPASTArtifact, String> {
        if !self.open_stack.is_empty() {
            return Err(format!(
                "{} AST nodes left open at finalization",
                self.open_stack.len()
            ));
        }
        let node_count = u32::try_from(self.node_count).map_err(|_| {
            format!(
                "{} AST nodes exceed the {} addressable by u32 pre-order ids",
                self.node_count,
                u32::MAX
            )
        })?;

        let rank_select = RankSelectIndex::build(&self.bp);
        let rmq = RangeMinMaxTree::build(&self.bp, &rank_select);

        let artifact = BPASTArtifact {
            node_count,
            bp_encoder: self.bp,
            rank_select,
            rmq,
            preorder: self.preorder,
            tca_hash: self.tca_hash,
        };
        artifact
            .validate()
            .map_err(|e| format!("Invariant 1 Violated: {}", e))?;
        Ok(artifact)
    }

    /// Builds the artifact. Panics if nodes are left open, which is a builder misuse.
    pub fn finalize(self) -> BPASTArtifact {
        match self.try_finalize() {
            Ok(artifact) => artifact,
            Err(e) => panic!("BPASTBuilder::finalize: {}", e),
        }
    }
}
