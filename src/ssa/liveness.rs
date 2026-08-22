//! Liveness Analysis Backward Fixpoint for Pruned SSA Form (§5.2.2).
//! Authored by Ahmad Hassan (B-Ted).

use crate::ast::BPASTArtifact;
use crate::cfg::builder::FunctionCFGData;
use crate::core::types::ast::ASTNodeType;

use std::collections::{HashMap, HashSet};

pub struct LivenessResult {
    /// LiveIn sets per basic block ID: block_id -> HashSet<orig_sym_id>
    pub live_in: HashMap<u32, HashSet<u32>>,
    /// LiveOut sets per basic block ID: block_id -> HashSet<orig_sym_id>
    pub live_out: HashMap<u32, HashSet<u32>>,
}

impl LivenessResult {
    pub fn is_live_in(&self, block_id: u32, sym_id: u32) -> bool {
        self.live_in
            .get(&block_id)
            .is_some_and(|set| set.contains(&sym_id))
    }
}

pub struct LivenessAnalysis;

impl LivenessAnalysis {
    pub fn compute(cfg: &FunctionCFGData, bpa: &BPASTArtifact) -> LivenessResult {
        let block_count = cfg.blocks.len();
        let mut live_in: HashMap<u32, HashSet<u32>> = HashMap::new();
        let mut live_out: HashMap<u32, HashSet<u32>> = HashMap::new();

        // 1. Compute UEVar (Use Before Def) and VarKill (Def) for each basic block
        let mut use_sets: HashMap<u32, HashSet<u32>> = HashMap::new();
        let mut def_sets: HashMap<u32, HashSet<u32>> = HashMap::new();

        for block in &cfg.blocks {
            let b_id = block.id;
            let mut uset = HashSet::new();
            let mut dset = HashSet::new();

            for &stmt_node in &block.stmts {
                // Find all referenced variables in stmt_node
                let (stmt_uses, stmt_defs) = extract_stmt_vars(stmt_node, bpa);
                for u in stmt_uses {
                    if !dset.contains(&u) {
                        uset.insert(u);
                    }
                }
                for d in stmt_defs {
                    dset.insert(d);
                }
            }

            use_sets.insert(b_id, uset);
            def_sets.insert(b_id, dset);
            live_in.insert(b_id, HashSet::new());
            live_out.insert(b_id, HashSet::new());
        }

        // 2. Backward Data-Flow Fixpoint Iteration
        let mut changed = true;
        let mut iter_count = 0;
        while changed && iter_count < 100 {
            changed = false;
            iter_count += 1;

            // Iterate blocks in reverse order
            for i in (0..block_count).rev() {
                let b_id = cfg.blocks[i].id;

                // LiveOut[b] = U_{s in succ(b)} LiveIn[s]
                let succs = get_succs(cfg, b_id);
                let mut new_out = HashSet::new();
                for s in succs {
                    if let Some(in_s) = live_in.get(&s) {
                        new_out.extend(in_s.iter().copied());
                    }
                }

                // LiveIn[b] = UEVar[b] U (LiveOut[b] - VarKill[b])
                let uset = use_sets.get(&b_id).cloned().unwrap_or_default();
                let dset = def_sets.get(&b_id).cloned().unwrap_or_default();

                let mut new_in = uset;
                for &out_var in &new_out {
                    if !dset.contains(&out_var) {
                        new_in.insert(out_var);
                    }
                }

                if new_in != *live_in.get(&b_id).unwrap()
                    || new_out != *live_out.get(&b_id).unwrap()
                {
                    live_in.insert(b_id, new_in);
                    live_out.insert(b_id, new_out);
                    changed = true;
                }
            }
        }

        let total_live_in: usize = live_in.values().map(|s| s.len()).sum();
        crate::core::logger::log_trace(&format!(
            "Liveness Analysis converged in {} iterations ({} live-in entries across {} blocks)",
            iter_count, total_live_in, block_count
        ));

        LivenessResult { live_in, live_out }
    }
}

fn get_succs(cfg: &FunctionCFGData, block_id: u32) -> Vec<u32> {
    let idx = block_id as usize;
    if idx + 1 < cfg.succ_offsets.len() {
        let start = cfg.succ_offsets[idx] as usize;
        let end = cfg.succ_offsets[idx + 1] as usize;
        if start <= end && end <= cfg.succ_adj.len() {
            return cfg.succ_adj[start..end].to_vec();
        }
    }
    Vec::new()
}

pub fn extract_stmt_vars(node: u32, bpa: &BPASTArtifact) -> (Vec<u32>, Vec<u32>) {
    let mut uses = Vec::new();
    let mut defs = Vec::new();

    let ntype = bpa.node_type(node);
    match ntype {
        ASTNodeType::NN_LOCAL_VAR_DECL
        | ASTNodeType::NN_FIELD_DECL
        | ASTNodeType::NN_PARAM_DECL => {
            // Left-hand side definition
            let sym_id = node;
            defs.push(sym_id);
            // Check initializer child for uses
            let mut child = bpa.first_child(node);
            while let Some(c) = child {
                if bpa.node_type(c) == ASTNodeType::NN_IDENTIFIER_EXPR {
                    uses.push(c);
                }
                child = bpa.next_sibling(c);
            }
        }
        ASTNodeType::NN_BINARY_EXPR | ASTNodeType::NN_ASSIGN_EXPR => {
            let lhs = bpa.first_child(node);
            let rhs = lhs.and_then(|l| bpa.next_sibling(l));
            if let Some(l) = lhs {
                if ntype == ASTNodeType::NN_ASSIGN_EXPR {
                    defs.push(l);
                } else {
                    uses.push(l);
                }
            }
            if let Some(r) = rhs {
                uses.push(r);
            }
        }
        ASTNodeType::NN_IDENTIFIER_EXPR => {
            uses.push(node);
        }
        _ => {
            let mut child = bpa.first_child(node);
            while let Some(c) = child {
                let (u, d) = extract_stmt_vars(c, bpa);
                uses.extend(u);
                defs.extend(d);
                child = bpa.next_sibling(c);
            }
        }
    }

    (uses, defs)
}
