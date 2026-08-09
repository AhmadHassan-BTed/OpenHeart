//! FunctionCFGBuilder orchestrator for building single-function CFGs (§4.5.2).

pub mod state;

use crate::ast::BPASTArtifact;
use crate::cfg::analysis::{
    analyze_loops, compute_dominance_frontiers, compute_idom_cooper, reverse_postorder, LoopInfo,
};
use crate::cfg::builder::state::CFGBuilderState;
use crate::cfg::stmts::dispatch_stmt;
use crate::core::types::cfg::{BasicBlock, CFGEdgeType};
use crate::symbol::SymbolTableArtifact;

#[derive(Debug, Clone)]
pub struct FunctionCFGData {
    pub sym_id: u32,
    pub blocks: Vec<BasicBlock>,
    pub edges: Vec<(u32, u32, CFGEdgeType)>,
    pub succ_offsets: Vec<u32>,
    pub succ_adj: Vec<u32>,
    pub pred_offsets: Vec<u32>,
    pub pred_adj: Vec<u32>,
    pub edge_types: Vec<u8>,
    pub idom: Vec<u32>,
    pub df_offsets: Vec<u32>,
    pub df_adj: Vec<u32>,
    pub loop_info: LoopInfo,
    pub cyclomatic: u16,
}

pub struct FunctionCFGBuilder;

impl FunctionCFGBuilder {
    pub fn build(
        sym_id: u32,
        body_node: u32,
        bpa: &BPASTArtifact,
        sta: &SymbolTableArtifact,
    ) -> FunctionCFGData {
        let mut state = CFGBuilderState::new();

        // Block 0 = ENTRY (synthetic)
        let entry = state.new_block();
        debug_assert_eq!(entry, 0);
        state.blocks[0].is_entry = true;

        // Create initial exit block placeholder (block_id determined at finalization)
        let exit_placeholder = state.new_block();
        state.blocks[exit_placeholder as usize].is_exit = true;
        state.exit_block = exit_placeholder;

        // First real statement block
        let first_real = state.new_block();
        state.add_edge(entry, first_real, CFGEdgeType::Uncond);
        state.current_block = first_real;

        let mut stmt = bpa.first_child(body_node);
        while let Some(s) = stmt {
            dispatch_stmt(s, &mut state, bpa, sta);
            stmt = bpa.next_sibling(s);
        }

        let exit = state.exit_block;
        state.flush_pending(exit);

        // Ensure ENTRY connects to EXIT if empty function body
        if state
            .edges
            .iter()
            .all(|&(u, v, _)| u != entry || v != first_real)
        {
            state.add_edge(entry, exit, CFGEdgeType::Uncond);
        }

        let n = state.blocks.len();
        let succ_lists = state.succ_lists();
        let pred_lists = state.pred_lists();

        let rpo = reverse_postorder(n, &succ_lists);
        let idom = compute_idom_cooper(n, &pred_lists, &rpo);
        let df_lists = compute_dominance_frontiers(n, &succ_lists, &pred_lists, &idom);
        let loop_info = analyze_loops(n, &succ_lists);

        // Build CSR for successors
        let mut succ_offsets = Vec::with_capacity(n + 1);
        let mut succ_adj = Vec::with_capacity(state.edges.len());
        let mut edge_types = Vec::with_capacity(state.edges.len());

        succ_offsets.push(0);
        for i in 0..n {
            for &(u, v, etype) in &state.edges {
                if u as usize == i {
                    succ_adj.push(v);
                    edge_types.push(etype as u8);
                }
            }
            succ_offsets.push(succ_adj.len() as u32);
        }

        // Build CSR for predecessors
        let mut pred_offsets = Vec::with_capacity(n + 1);
        let mut pred_adj = Vec::with_capacity(state.edges.len());

        pred_offsets.push(0);
        for i in 0..n {
            for &(u, v, _) in &state.edges {
                if v as usize == i {
                    pred_adj.push(u);
                }
            }
            pred_offsets.push(pred_adj.len() as u32);
        }

        // Build CSR for Dominance Frontiers
        let mut df_offsets = Vec::with_capacity(n + 1);
        let mut df_adj = Vec::new();
        df_offsets.push(0);
        for list in &df_lists {
            df_adj.extend_from_slice(list);
            df_offsets.push(df_adj.len() as u32);
        }

        let num_edges = state.edges.len() as i32;
        let num_blocks = n as i32;
        let cyclomatic = ((num_edges - num_blocks + 2).max(1)) as u16;

        FunctionCFGData {
            sym_id,
            blocks: state.blocks,
            edges: state.edges,
            succ_offsets,
            succ_adj,
            pred_offsets,
            pred_adj,
            edge_types,
            idom,
            df_offsets,
            df_adj,
            loop_info,
            cyclomatic,
        }
    }
}
