//! ActivityDiagramExtractor — converts CFA basic blocks to ActivityRecord[] (§9.2.2).

use crate::ast::BPASTArtifact;
use crate::cfg::builder::FunctionCFGData;
use crate::cfg::serializer::CFGArtifact;
use crate::ingestion::TokenCorpusArtifact;
use crate::psa::types::PathSummaryArtifact;
use crate::symbol::SymbolTableArtifact;
use crate::uma::label_extraction::LabelExtractor;
use crate::uma::types::*;

pub struct ActivityDiagramExtractor;

impl ActivityDiagramExtractor {
    pub fn extract_all(
        cfa: &CFGArtifact,
        bpa: &BPASTArtifact,
        tca: &TokenCorpusArtifact,
        sta: &SymbolTableArtifact,
        psa: &PathSummaryArtifact,
    ) -> Vec<ActivityRecord> {
        let mut activities = Vec::new();

        for func_cfg in &cfa.functions {
            if let Some(record) = Self::extract_function(func_cfg, bpa, tca, sta, psa) {
                activities.push(record);
            }
        }

        activities
    }

    pub fn extract_function(
        cfg: &FunctionCFGData,
        bpa: &BPASTArtifact,
        tca: &TokenCorpusArtifact,
        sta: &SymbolTableArtifact,
        psa: &PathSummaryArtifact,
    ) -> Option<ActivityRecord> {
        let n_blocks = cfg.blocks.len();
        if n_blocks == 0 {
            return None;
        }

        let mut nodes = Vec::with_capacity(n_blocks);
        let mut edges = Vec::with_capacity(cfg.edges.len());
        let mut start_node = 0u16;

        for block_id in 0..n_blocks as u32 {
            let blk = &cfg.blocks[block_id as usize];

            let succs_len = if (block_id as usize) + 1 < cfg.succ_offsets.len() {
                (cfg.succ_offsets[(block_id as usize) + 1] - cfg.succ_offsets[block_id as usize])
                    as usize
            } else {
                0
            };

            let preds_len = if (block_id as usize) + 1 < cfg.pred_offsets.len() {
                (cfg.pred_offsets[(block_id as usize) + 1] - cfg.pred_offsets[block_id as usize])
                    as usize
            } else {
                0
            };

            // Classification function (§9.2.2)
            let node_kind = if blk.is_entry {
                start_node = block_id as u16;
                NODE_KIND_INITIAL
            } else if blk.is_exit || succs_len == 0 {
                NODE_KIND_FINAL
            } else if preds_len >= 2 && succs_len == 1 {
                NODE_KIND_MERGE
            } else if succs_len == 2 {
                NODE_KIND_DECISION
            } else {
                NODE_KIND_ACTION
            };

            let stmt_node = blk.stmts.first().copied().unwrap_or(u32::MAX);

            let _label_text = if blk.is_entry || blk.is_exit {
                if blk.is_entry {
                    "Initial".to_string()
                } else {
                    "Final".to_string()
                }
            } else {
                LabelExtractor::extract_label(stmt_node, bpa, tca, sta, 30)
            };

            // String ID representation: use block_id as index identifier
            let label_text_id = block_id;

            nodes.push(ActivityNode {
                node_id: block_id,
                label_text_id,
                node_kind,
                loop_depth: 0,
                guard_text_id: 0,
                _pad: 0,
            });
        }

        // Edge transformation (§9.2.2)
        for &(u, v, et) in &cfg.edges {
            let is_back = if et == crate::core::types::cfg::CFGEdgeType::LoopBack {
                1
            } else {
                0
            };
            edges.push(ActivityEdge {
                from_node: u as u16,
                to_node: v as u16,
                edge_kind: EDGE_KIND_CONTROL,
                is_back_edge: is_back,
                guard_text_id: 0,
                _pad: 0,
            });
        }

        let cyc = if let Some(hdr) = psa.function_header(cfg.sym_id) {
            hdr.cyclomatic
        } else {
            cfg.cyclomatic
        };

        Some(ActivityRecord {
            function_sym_id: cfg.sym_id,
            node_count: nodes.len() as u16,
            edge_count: edges.len() as u16,
            start_node,
            end_node_count: 1,
            swimlane_count: 1,
            cyclomatic: cyc,
            _reserved: 0,
            nodes,
            edges,
        })
    }
}
