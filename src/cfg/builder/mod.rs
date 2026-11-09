//! FunctionCFGBuilder orchestrator for building single-function CFGs (§4.5.2).

pub mod exceptions;
pub mod state;

use crate::ast::BPASTArtifact;
use crate::cfg::analysis::{
    analyze_loops, compute_dominance_frontiers, compute_idom_cooper, reverse_postorder,
    DominatorTree, LoopInfo,
};
use crate::cfg::ast_query::{children, is_statement_kind, is_type_decl};
use crate::cfg::builder::exceptions::ExceptionTypes;
use crate::cfg::builder::state::{CFGBuilderState, CfgDiagnostic, CfgDiagnosticKind, Jump};
use crate::cfg::source::SourceText;
use crate::cfg::stmts::{dispatch_stmt, lower_embedded};
use crate::core::types::ast::ASTNodeType;
use crate::core::types::cfg::{BasicBlock, CFGEdgeType};
use crate::symbol::SymbolTableArtifact;

/// Where the statements of a function live in the BP AST.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FunctionBody {
    /// A block whose children are the body statements.
    Block(u32),
    /// A declaration whose statement-kind children are the body (a constructor whose
    /// `constructor_body` wrapper was eliminated by Phase 2).
    Statements(u32),
    /// The expression of an expression-bodied lambda.
    Expression(u32),
}

impl FunctionBody {
    pub fn node(&self) -> u32 {
        match *self {
            FunctionBody::Block(n) | FunctionBody::Statements(n) | FunctionBody::Expression(n) => n,
        }
    }
}

/// CFG of one function.
///
/// Edges are stored in canonical order, grouped by source block: edge index `k` is the
/// `k`-th entry of `succ_adj` / `edge_types`, so `edges[k] == (b, succ_adj[k], ty)` for
/// the block `b` whose `succ_offsets` range contains `k`. `pred_edge_idx` maps each
/// predecessor entry back to its edge index, giving O(1) edge-id lookups in both
/// directions and keeping parallel edges distinct.
#[derive(Debug, Clone)]
pub struct FunctionCFGData {
    pub sym_id: u32,
    /// Declaration node of the function (method, constructor, lambda, initializer).
    pub decl_node: u32,
    /// Node holding the body statements (see [`FunctionBody`]).
    pub body_node: u32,
    pub blocks: Vec<BasicBlock>,
    pub edges: Vec<(u32, u32, CFGEdgeType)>,
    pub succ_offsets: Vec<u32>,
    pub succ_adj: Vec<u32>,
    pub pred_offsets: Vec<u32>,
    pub pred_adj: Vec<u32>,
    /// Edge index of each `pred_adj` entry.
    pub pred_edge_idx: Vec<u32>,
    pub edge_types: Vec<u8>,
    pub idom: Vec<u32>,
    pub df_offsets: Vec<u32>,
    pub df_adj: Vec<u32>,
    pub loop_info: LoopInfo,
    /// McCabe V(G) = decision points + 1 (see `FunctionCFGBuilder::build`).
    pub cyclomatic: u16,
    /// Id of the unique EXIT block. Blocks are numbered in reverse post-order, so EXIT is
    /// not necessarily the last block.
    pub exit_block: u32,
    /// Sorted expression nodes whose evaluation was lowered into their own blocks
    /// (short-circuit operators, conditional and switch expressions). A statement in a
    /// block that contains such a node must not be re-walked into it: its operands are
    /// statements of other blocks.
    pub lowered_nodes: Vec<u32>,
    /// Places where construction had to approximate.
    pub diagnostics: Vec<CfgDiagnostic>,
}

impl FunctionCFGData {
    #[inline]
    fn csr_slice<'s>(offsets: &[u32], adj: &'s [u32], b: u32) -> &'s [u32] {
        let i = b as usize;
        if i + 1 < offsets.len() {
            let (s, e) = (offsets[i] as usize, offsets[i + 1] as usize);
            if s <= e && e <= adj.len() {
                return &adj[s..e];
            }
        }
        &[]
    }

    pub fn succs_of(&self, b: u32) -> &[u32] {
        Self::csr_slice(&self.succ_offsets, &self.succ_adj, b)
    }

    pub fn preds_of(&self, b: u32) -> &[u32] {
        Self::csr_slice(&self.pred_offsets, &self.pred_adj, b)
    }

    /// Edge indices of the outgoing edges of `b`.
    pub fn succ_edge_range(&self, b: u32) -> std::ops::Range<usize> {
        let i = b as usize;
        if i + 1 < self.succ_offsets.len() {
            self.succ_offsets[i] as usize..self.succ_offsets[i + 1] as usize
        } else {
            0..0
        }
    }

    /// Edge indices of the incoming edges of `b`.
    pub fn pred_edge_ids(&self, b: u32) -> &[u32] {
        Self::csr_slice(&self.pred_offsets, &self.pred_edge_idx, b)
    }

    pub fn is_lowered(&self, node: u32) -> bool {
        self.lowered_nodes.binary_search(&node).is_ok()
    }
}

pub struct FunctionCFGBuilder;

impl FunctionCFGBuilder {
    pub fn build(
        sym_id: u32,
        decl_node: u32,
        body: FunctionBody,
        bpa: &BPASTArtifact,
        sta: &SymbolTableArtifact,
        src: Option<SourceText<'_>>,
        exceptions: &ExceptionTypes<'_>,
    ) -> Result<FunctionCFGData, String> {
        let mut state = CFGBuilderState::new(src, exceptions);

        // Block 0 = ENTRY (synthetic)
        let entry = state.new_block();
        state.blocks[entry as usize].is_entry = true;
        state.mark_root(entry);

        // Unique EXIT block: normal returns, uncaught exceptions and falling off the end.
        let exit = state.new_block();
        state.blocks[exit as usize].is_exit = true;
        state.exit_block = exit;

        // First real statement block
        let first_real = state.new_block();
        state.add_edge(entry, first_real, CFGEdgeType::Uncond);
        state.current_block = first_real;

        match body {
            FunctionBody::Block(b) => {
                let mut stmt = bpa.first_child(b);
                while let Some(s) = stmt {
                    dispatch_stmt(s, &mut state, bpa, sta);
                    stmt = bpa.next_sibling(s);
                }
            }
            FunctionBody::Statements(container) => {
                for c in children(bpa, container) {
                    if is_body_statement(bpa, c) {
                        dispatch_stmt(c, &mut state, bpa, sta);
                    }
                }
            }
            FunctionBody::Expression(e) => {
                lower_embedded(e, &mut state, bpa, sta);
                state.add_stmt(e, bpa);
                let from = state.current_block;
                state.route_jump(from, Jump::Return);
                state.terminate();
            }
        }

        // Falling off the end of the body.
        let tail = state.current_block;
        state.add_edge(tail, exit, CFGEdgeType::Uncond);

        Self::finalize(sym_id, decl_node, body.node(), state)
    }

    fn finalize(
        sym_id: u32,
        decl_node: u32,
        body_node: u32,
        state: CFGBuilderState,
    ) -> Result<FunctionCFGData, String> {
        let raw_n = state.blocks.len();
        let exit = state.exit_block;

        // ── Compact & prune unreachable blocks (RPO numbering) ──
        let mut rpo = reverse_postorder(raw_n, state.succ_lists());
        if !rpo.contains(&exit) {
            // EXIT is unreachable only when the body never terminates (infinite loop).
            rpo.push(exit);
        }

        let mut old_to_new = vec![u32::MAX; raw_n];
        for (new_id, &old_id) in rpo.iter().enumerate() {
            old_to_new[old_id as usize] = to_u32(new_id, "block id")?;
        }

        let mut blocks = Vec::with_capacity(rpo.len());
        for (new_id, &old_id) in rpo.iter().enumerate() {
            let mut blk = state.blocks[old_id as usize].clone();
            blk.id = to_u32(new_id, "block id")?;
            blocks.push(blk);
        }
        let exit_block = old_to_new[exit as usize];

        let mut edges: Vec<(u32, u32, CFGEdgeType)> = state
            .edges
            .iter()
            .filter_map(|&(u, v, et)| {
                let (nu, nv) = (old_to_new[u as usize], old_to_new[v as usize]);
                if nu != u32::MAX && nv != u32::MAX {
                    Some((nu, nv, et))
                } else {
                    None
                }
            })
            .collect();
        // Canonical order: grouped by source block, insertion order within a block.
        edges.sort_by_key(|&(u, _, _)| u);

        let n = blocks.len();
        let n_edges = edges.len();

        // ── Successor CSR (edges are grouped by source) ──
        let mut succ_offsets = vec![0u32; n + 1];
        for &(u, _, _) in &edges {
            succ_offsets[u as usize + 1] += 1;
        }
        for i in 0..n {
            succ_offsets[i + 1] += succ_offsets[i];
        }
        let succ_adj: Vec<u32> = edges.iter().map(|&(_, v, _)| v).collect();
        let edge_types: Vec<u8> = edges.iter().map(|&(_, _, et)| et as u8).collect();

        // ── Predecessor CSR (counting sort by target) ──
        let mut pred_offsets = vec![0u32; n + 1];
        for &(_, v, _) in &edges {
            pred_offsets[v as usize + 1] += 1;
        }
        for i in 0..n {
            pred_offsets[i + 1] += pred_offsets[i];
        }
        let mut cursor: Vec<u32> = pred_offsets[..n].to_vec();
        let mut pred_adj = vec![0u32; n_edges];
        let mut pred_edge_idx = vec![0u32; n_edges];
        for (k, &(u, v, _)) in edges.iter().enumerate() {
            let pos = cursor[v as usize] as usize;
            pred_adj[pos] = u;
            pred_edge_idx[pos] = to_u32(k, "edge id")?;
            cursor[v as usize] += 1;
        }

        // ── Only EXIT may lack successors: every other block ends in an edge ──
        for b in 0..n {
            if b as u32 != exit_block && succ_offsets[b] == succ_offsets[b + 1] {
                return Err(format!(
                    "CFG construction error: block {} of function sym {} has no successor",
                    b, sym_id
                ));
            }
        }

        let succ_lists: Vec<Vec<u32>> = (0..n)
            .map(|b| succ_adj[succ_offsets[b] as usize..succ_offsets[b + 1] as usize].to_vec())
            .collect();
        let pred_lists: Vec<Vec<u32>> = (0..n)
            .map(|b| pred_adj[pred_offsets[b] as usize..pred_offsets[b + 1] as usize].to_vec())
            .collect();

        // Blocks are numbered in RPO, so the identity order is a valid RPO for Cooper.
        let new_rpo: Vec<u32> = (0..n as u32).collect();
        let idom = compute_idom_cooper(n, &pred_lists, &new_rpo);
        let df_lists = compute_dominance_frontiers(n, &succ_lists, &pred_lists, &idom);
        let dom_tree = DominatorTree::new(&idom, 0);
        let loop_info = analyze_loops(n, &succ_lists, &pred_lists, &dom_tree);

        // Build CSR for Dominance Frontiers
        let mut df_offsets = Vec::with_capacity(n + 1);
        let mut df_adj = Vec::new();
        df_offsets.push(0);
        for list in &df_lists {
            df_adj.extend_from_slice(list);
            df_offsets.push(to_u32(df_adj.len(), "dominance frontier size")?);
        }

        // ── V(G) = decision points + 1 ──
        // A block with k normal (non-exceptional) successors contributes k - 1 decisions;
        // each catch clause contributes one. Implicit exceptional edges are excluded, as
        // in the usual source-level definition.
        let mut decisions: u64 = u64::from(state.catch_clauses);
        for b in 0..n {
            let normal_out = edge_types[succ_offsets[b] as usize..succ_offsets[b + 1] as usize]
                .iter()
                .filter(|&&t| t != CFGEdgeType::Except as u8)
                .count() as u64;
            decisions += normal_out.saturating_sub(1);
        }
        let mut diagnostics = state.diagnostics.clone();
        let cyclomatic = match u16::try_from(decisions + 1) {
            Ok(v) => v,
            Err(_) => {
                diagnostics.push(CfgDiagnostic {
                    kind: CfgDiagnosticKind::CyclomaticSaturated,
                    node: body_node,
                });
                u16::MAX
            }
        };

        let mut lowered_nodes: Vec<u32> = state.lowered.iter().copied().collect();
        lowered_nodes.sort_unstable();

        Ok(FunctionCFGData {
            sym_id,
            decl_node,
            body_node,
            blocks,
            edges,
            succ_offsets,
            succ_adj,
            pred_offsets,
            pred_adj,
            pred_edge_idx,
            edge_types,
            idom,
            df_offsets,
            df_adj,
            loop_info,
            cyclomatic,
            exit_block,
            lowered_nodes,
            diagnostics,
        })
    }
}

/// A direct child of a declaration that belongs to its body when Phase 2 eliminated the
/// body wrapper: statements, plus explicit constructor invocations (`this(...)` /
/// `super(...)`, reduced to a call expression).
pub fn is_body_statement(bpa: &BPASTArtifact, node: u32) -> bool {
    let t = bpa.node_type(node);
    (is_statement_kind(t) && !is_type_decl(t)) || t == ASTNodeType::NN_CALL_EXPR
}

/// Checked conversion of an in-memory count/index to the u32 used by the artifacts.
fn to_u32(v: usize, what: &str) -> Result<u32, String> {
    u32::try_from(v).map_err(|_| format!("CFG {} {} exceeds the u32 range", what, v))
}
