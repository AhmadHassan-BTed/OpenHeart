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

        if state.current_block != exit {
            state.add_edge(state.current_block, exit, CFGEdgeType::Uncond);
        }

        // Guarantee that all sink blocks connect to exit so EXIT is uniquely reachable and present
        let temp_n = state.blocks.len();
        let temp_succs = state.succ_lists();
        for u in 0..temp_n as u32 {
            if u != exit && temp_succs[u as usize].is_empty() {
                state.add_edge(u, exit, CFGEdgeType::Uncond);
            }
        }

        // Ensure ENTRY connects to EXIT if empty function body
        if state
            .edges
            .iter()
            .all(|&(u, v, _)| u != entry || v != first_real)
        {
            state.add_edge(entry, exit, CFGEdgeType::Uncond);
        }

        let raw_n = state.blocks.len();
        let raw_succs = state.succ_lists();

        // ── Compact & Prune Unreachable Blocks ──
        let mut rpo = reverse_postorder(raw_n, &raw_succs);
        if !rpo.contains(&exit) {
            rpo.push(exit);
        }

        let mut old_to_new = vec![u32::MAX; raw_n];
        for (new_id, &old_id) in rpo.iter().enumerate() {
            if (old_id as usize) < raw_n {
                old_to_new[old_id as usize] = new_id as u32;
            }
        }

        let mut blocks = Vec::with_capacity(rpo.len());
        for (new_id, &old_id) in rpo.iter().enumerate() {
            let mut blk = state.blocks[old_id as usize].clone();
            blk.id = new_id as u32;
            blocks.push(blk);
        }

        let mut edges = Vec::new();
        for &(u, v, etype) in &state.edges {
            let new_u = if (u as usize) < raw_n {
                old_to_new[u as usize]
            } else {
                u32::MAX
            };
            let new_v = if (v as usize) < raw_n {
                old_to_new[v as usize]
            } else {
                u32::MAX
            };
            if new_u != u32::MAX && new_v != u32::MAX {
                edges.push((new_u, new_v, etype));
            }
        }

        let n = blocks.len();
        let mut succ_lists = vec![Vec::new(); n];
        let mut pred_lists = vec![Vec::new(); n];

        for &(u, v, _) in &edges {
            if (u as usize) < n && (v as usize) < n {
                succ_lists[u as usize].push(v);
                pred_lists[v as usize].push(u);
            }
        }

        let new_rpo = (0..n as u32).collect::<Vec<u32>>();
        let idom = compute_idom_cooper(n, &pred_lists, &new_rpo);
        let df_lists = compute_dominance_frontiers(n, &succ_lists, &pred_lists, &idom);
        let loop_info = analyze_loops(n, &succ_lists);

        // Build CSR for successors
        let mut succ_offsets = Vec::with_capacity(n + 1);
        let mut succ_adj = Vec::with_capacity(edges.len());
        let mut edge_types = Vec::with_capacity(edges.len());

        succ_offsets.push(0);
        for i in 0..n {
            for &(u, v, etype) in &edges {
                if u as usize == i {
                    succ_adj.push(v);
                    edge_types.push(etype as u8);
                }
            }
            succ_offsets.push(succ_adj.len() as u32);
        }

        // Build CSR for predecessors
        let mut pred_offsets = Vec::with_capacity(n + 1);
        let mut pred_adj = Vec::with_capacity(edges.len());

        pred_offsets.push(0);
        for i in 0..n {
            for &(u, v, _) in &edges {
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

        let num_edges = edges.len() as i32;
        let num_blocks = n as i32;
        let cyclomatic = ((num_edges - num_blocks + 2).max(1)) as u16;

        FunctionCFGData {
            sym_id,
            blocks,
            edges,
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
