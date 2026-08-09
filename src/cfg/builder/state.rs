//! CFGBuilderState: maintains live state during construction of a single function CFG.

use crate::ast::BPASTArtifact;
use crate::core::types::cfg::{
    BasicBlock, BreakFrame, CFGEdgeType, ContinueFrame, ExceptionFrame, PendingEdge,
};

#[derive(Debug, Clone)]
pub struct CFGBuilderState {
    pub blocks: Vec<BasicBlock>,
    pub current_block: u32,
    pub pending_exits: Vec<PendingEdge>,
    pub edges: Vec<(u32, u32, CFGEdgeType)>,
    pub break_stack: Vec<BreakFrame>,
    pub continue_stack: Vec<ContinueFrame>,
    pub exception_stack: Vec<ExceptionFrame>,
    pub exit_block: u32,
}

impl Default for CFGBuilderState {
    fn default() -> Self {
        Self::new()
    }
}

impl CFGBuilderState {
    pub fn new() -> Self {
        Self {
            blocks: Vec::new(),
            current_block: 0,
            pending_exits: Vec::new(),
            edges: Vec::new(),
            break_stack: Vec::new(),
            continue_stack: Vec::new(),
            exception_stack: Vec::new(),
            exit_block: 0,
        }
    }

    pub fn new_block(&mut self) -> u32 {
        let id = self.blocks.len() as u32;
        self.blocks.push(BasicBlock::new(id));
        id
    }

    pub fn add_edge(&mut self, from: u32, to: u32, edge_type: CFGEdgeType) {
        if from != u32::MAX && to != u32::MAX {
            self.edges.push((from, to, edge_type));
        }
    }

    pub fn add_stmt_to_current(&mut self, pre_idx: u32, bpa: &BPASTArtifact) {
        let cur = self.current_block as usize;
        if cur < self.blocks.len() {
            self.blocks[cur].stmts.push(pre_idx);
            let (ft, lt) = bpa.token_range(pre_idx);
            if ft != u32::MAX {
                self.blocks[cur].first_token = self.blocks[cur].first_token.min(ft);
                self.blocks[cur].last_token = self.blocks[cur].last_token.max(lt);
            }
        }
    }

    pub fn add_stmt_to_block(&mut self, block_id: u32, pre_idx: u32, bpa: &BPASTArtifact) {
        let b = block_id as usize;
        if b < self.blocks.len() {
            self.blocks[b].stmts.push(pre_idx);
            let (ft, lt) = bpa.token_range(pre_idx);
            if ft != u32::MAX {
                self.blocks[b].first_token = self.blocks[b].first_token.min(ft);
                self.blocks[b].last_token = self.blocks[b].last_token.max(lt);
            }
        }
    }

    pub fn add_pending_exit(&mut self, from: u32, edge_type: CFGEdgeType) {
        self.pending_exits.push(PendingEdge { from, edge_type });
    }

    pub fn flush_pending(&mut self, target_block: u32) {
        let pending = std::mem::take(&mut self.pending_exits);
        for pe in pending {
            self.add_edge(pe.from, target_block, pe.edge_type);
        }
    }

    pub fn flush_pending_with_type(&mut self, target_block: u32, edge_type: CFGEdgeType) {
        let pending = std::mem::take(&mut self.pending_exits);
        for pe in pending {
            self.add_edge(pe.from, target_block, edge_type);
        }
    }

    pub fn drain_pending(&mut self) -> Vec<PendingEdge> {
        std::mem::take(&mut self.pending_exits)
    }

    pub fn push_break(&mut self, target: u32, label: Option<u32>) {
        self.break_stack.push(BreakFrame { target, label });
    }

    pub fn pop_break(&mut self) {
        self.break_stack.pop();
    }

    pub fn push_continue(&mut self, target: u32, label: Option<u32>) {
        self.continue_stack.push(ContinueFrame { target, label });
    }

    pub fn pop_continue(&mut self) {
        self.continue_stack.pop();
    }

    pub fn push_exception(&mut self, frame: ExceptionFrame) {
        self.exception_stack.push(frame);
    }

    pub fn pop_exception(&mut self) {
        self.exception_stack.pop();
    }

    pub fn succ_lists(&self) -> Vec<Vec<u32>> {
        let n = self.blocks.len();
        let mut succs = vec![Vec::new(); n];
        for &(from, to, _) in &self.edges {
            if (from as usize) < n && (to as usize) < n {
                succs[from as usize].push(to);
            }
        }
        succs
    }

    pub fn pred_lists(&self) -> Vec<Vec<u32>> {
        let n = self.blocks.len();
        let mut preds = vec![Vec::new(); n];
        for &(from, to, _) in &self.edges {
            if (from as usize) < n && (to as usize) < n {
                preds[to as usize].push(from);
            }
        }
        preds
    }
}
