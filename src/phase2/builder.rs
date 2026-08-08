//! BPASTBuilder aggregating BPEncoder, PreorderArrays, and building auxiliary structures.

use super::bp_encoder::BPEncoder;
use super::jump_table::{JumpTable, JumpTableBuilder};
use super::preorder::PreorderArrays;
use super::rank_select::RankSelectIndex;
use super::rmq::SparseTableRMQ;
use crate::core::types::ast::ASTNodeType;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BPASTArtifact {
    pub node_count: u32,
    pub bp_encoder: BPEncoder,
    pub jump_table: JumpTable,
    pub rank_select: RankSelectIndex,
    pub rmq: SparseTableRMQ,
    pub preorder: PreorderArrays,
    pub tca_hash: u64,
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

        self.bp.push_open();
        self.open_stack.push(preorder_idx);
        self.node_count += 1;

        preorder_idx
    }

    pub fn close_node(&mut self, preorder_idx: u32, first_tok: u32, last_tok: u32) {
        self.preorder.token_ranges[preorder_idx as usize] = (first_tok, last_tok);

        for &ancestor_idx in &self.open_stack {
            if ancestor_idx == preorder_idx {
                break;
            }
            let r = &mut self.preorder.token_ranges[ancestor_idx as usize];
            r.0 = r.0.min(first_tok);
            r.1 = r.1.max(last_tok);
        }

        self.bp.push_close();
        self.open_stack.pop();
    }

    pub fn finalize(self) -> BPASTArtifact {
        assert!(
            self.open_stack.is_empty(),
            "Unclosed AST nodes at finalization"
        );

        let jump_table = JumpTableBuilder::build(&self.bp);
        let rank_select = RankSelectIndex::build(&self.bp);
        let rmq = SparseTableRMQ::build(&self.bp, &rank_select);

        let artifact = BPASTArtifact {
            node_count: self.node_count as u32,
            bp_encoder: self.bp,
            jump_table,
            rank_select,
            rmq,
            preorder: self.preorder,
            tca_hash: self.tca_hash,
        };

        // Invariant 1: BP Balance Check
        if artifact.node_count > 0 {
            assert_eq!(
                artifact
                    .rank_select
                    .rank1(&artifact.bp_encoder, artifact.bp_encoder.bit_count - 1),
                artifact.node_count,
                "Invariant 1 Violated: BP rank1 count mismatch"
            );
        }

        artifact
    }
}
