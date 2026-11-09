//! ActivityDiagramExtractor — converts CFA basic blocks to ActivityRecord[] (§9.2.2).
//!
//! - Nodes: the entry block is the initial node, exit/sink blocks are final
//!   nodes, targets of exception edges are exception-handler nodes, blocks with
//!   ≥2 successors are decisions and join blocks are merges. Labels are rebuilt
//!   from the statements' token spans and interned into the label table.
//! - Edges: `True`/`False` edges carry `[cond]`/`[else]` guards built from the
//!   branch condition's source text, `Switch` edges carry `[case X]`, `Except`
//!   edges become exception flows, and `LoopBack` edges are marked as back edges.
//! - Loop depth comes from the CFG loop analysis.
//!
//! Fork/join nodes are not produced: the CFG carries no thread-start facts, so
//! no node is invented for them. Swimlanes are not inlined across callees: each
//! activity is one partition (its owning class).

use crate::ast::BPASTArtifact;
use crate::cfg::builder::FunctionCFGData;
use crate::cfg::serializer::CFGArtifact;
use crate::core::types::cfg::CFGEdgeType;
use crate::ingestion::TokenCorpusArtifact;
use crate::psa::types::PathSummaryArtifact;
use crate::uma::label_extraction::{LabelExtractor, LabelTable};
use crate::uma::types::*;

/// Maximum characters of an action label.
const ACTION_LABEL_CHARS: usize = 60;
/// Maximum characters of a guard condition.
const GUARD_LABEL_CHARS: usize = 48;
/// Statements joined into one action label.
const ACTION_LABEL_STATEMENTS: usize = 3;

pub struct ActivityDiagramExtractor;

impl ActivityDiagramExtractor {
    pub fn extract_all(
        cfa: &CFGArtifact,
        bpa: &BPASTArtifact,
        tca: &TokenCorpusArtifact,
        psa: &PathSummaryArtifact,
        labels: &mut LabelTable,
    ) -> Vec<ActivityRecord> {
        cfa.functions
            .iter()
            .filter_map(|f| Self::extract_function(f, bpa, tca, psa, labels))
            .collect()
    }

    fn degree(offsets: &[u32], block: usize) -> usize {
        match (offsets.get(block), offsets.get(block + 1)) {
            (Some(&a), Some(&b)) if b >= a => (b - a) as usize,
            _ => 0,
        }
    }

    pub fn extract_function(
        cfg: &FunctionCFGData,
        bpa: &BPASTArtifact,
        tca: &TokenCorpusArtifact,
        psa: &PathSummaryArtifact,
        labels: &mut LabelTable,
    ) -> Option<ActivityRecord> {
        let n_blocks = cfg.blocks.len();
        if n_blocks == 0 {
            return None;
        }

        let mut is_handler = vec![false; n_blocks];
        for &(_, v, et) in &cfg.edges {
            if et == CFGEdgeType::Except {
                if let Some(slot) = is_handler.get_mut(v as usize) {
                    *slot = true;
                }
            }
        }

        let mut nodes = Vec::with_capacity(n_blocks);
        let mut start_node = 0u32;
        let mut final_count = 0usize;
        let mut cond_labels: Vec<Option<String>> = vec![None; n_blocks];

        for (idx, blk) in cfg.blocks.iter().enumerate() {
            let succs = Self::degree(&cfg.succ_offsets, idx);
            let preds = Self::degree(&cfg.pred_offsets, idx);
            let node_kind = if blk.is_entry {
                start_node = blk.id;
                NODE_KIND_INITIAL
            } else if blk.is_exit || succs == 0 {
                NODE_KIND_FINAL
            } else if is_handler[idx] {
                NODE_KIND_EXCEPTION
            } else if succs >= 2 {
                NODE_KIND_DECISION
            } else if preds >= 2 && succs == 1 {
                NODE_KIND_MERGE
            } else {
                NODE_KIND_ACTION
            };
            if node_kind == NODE_KIND_FINAL {
                final_count += 1;
            }

            let label = match node_kind {
                NODE_KIND_INITIAL | NODE_KIND_FINAL if blk.stmts.is_empty() => None,
                NODE_KIND_DECISION => {
                    let cond = blk
                        .stmts
                        .last()
                        .and_then(|&s| LabelExtractor::statement_label(s, bpa, tca, GUARD_LABEL_CHARS));
                    cond_labels[idx] = cond.clone();
                    cond
                }
                _ => Self::action_label(&blk.stmts, bpa, tca),
            };
            let label_text_id = match &label {
                Some(text) => labels.intern(text),
                None => NO_LABEL,
            };
            let loop_depth = cfg.loop_info.loop_depth.get(idx).copied().unwrap_or(0);
            nodes.push(ActivityNode {
                node_id: blk.id,
                label_text_id,
                node_kind,
                loop_depth,
                guard_text_id: NO_LABEL,
                first_token: blk.first_token,
            });
        }

        let mut edges = Vec::with_capacity(cfg.edges.len());
        for &(u, v, et) in &cfg.edges {
            let (edge_kind, guard) = match et {
                CFGEdgeType::True => (
                    EDGE_KIND_CONTROL,
                    Some(match cond_labels.get(u as usize).and_then(|c| c.as_ref()) {
                        Some(cond) => format!("[{}]", cond),
                        None => "[true]".to_string(),
                    }),
                ),
                CFGEdgeType::False => (EDGE_KIND_CONTROL, Some("[else]".to_string())),
                CFGEdgeType::Switch => {
                    let case_label = cfg
                        .blocks
                        .get(v as usize)
                        .and_then(|b| b.stmts.first())
                        .and_then(|&s| {
                            LabelExtractor::statement_label(s, bpa, tca, GUARD_LABEL_CHARS)
                        });
                    let guard = match case_label {
                        Some(l) => format!("[case {}]", l),
                        None => "[default]".to_string(),
                    };
                    (EDGE_KIND_CONTROL, Some(guard))
                }
                CFGEdgeType::Except => (EDGE_KIND_EXCEPTION, Some("[exception]".to_string())),
                CFGEdgeType::Return | CFGEdgeType::Uncond | CFGEdgeType::LoopBack => {
                    (EDGE_KIND_CONTROL, None)
                }
            };
            let guard_text_id = match guard {
                Some(g) => labels.intern(&g),
                None => NO_LABEL,
            };
            edges.push(ActivityEdge {
                from_node: u,
                to_node: v,
                edge_kind,
                is_back_edge: u8::from(et == CFGEdgeType::LoopBack),
                guard_text_id,
            });
        }

        let cyclomatic = match psa.function_header(cfg.sym_id) {
            Some(hdr) => hdr.cyclomatic,
            None => cfg.cyclomatic,
        };

        Some(ActivityRecord {
            function_sym_id: cfg.sym_id,
            node_count: u32::try_from(nodes.len()).unwrap_or(u32::MAX),
            edge_count: u32::try_from(edges.len()).unwrap_or(u32::MAX),
            start_node,
            end_node_count: u16::try_from(final_count).unwrap_or(u16::MAX),
            swimlane_count: 1,
            cyclomatic,
            max_loop_depth: cfg.loop_info.max_loop_depth,
            _reserved: 0,
            nodes,
            edges,
        })
    }

    fn action_label(stmts: &[u32], bpa: &BPASTArtifact, tca: &TokenCorpusArtifact) -> Option<String> {
        let parts: Vec<String> = stmts
            .iter()
            .take(ACTION_LABEL_STATEMENTS)
            .filter_map(|&s| LabelExtractor::statement_label(s, bpa, tca, ACTION_LABEL_CHARS))
            .collect();
        if parts.is_empty() {
            return None;
        }
        let mut text = parts.join("; ");
        if stmts.len() > ACTION_LABEL_STATEMENTS {
            text.push_str("; ...");
        }
        Some(crate::uma::label_extraction::truncate_chars(&text, ACTION_LABEL_CHARS))
    }
}
