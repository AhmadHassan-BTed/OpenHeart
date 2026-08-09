//! Anderson Inclusion-Based Points-To Analysis (§6.2.5).
//! Authored solely by Ahmad Hassan (B-Ted).

use crate::ast::BPASTArtifact;
use crate::core::types::ASTNodeType::*;
use crate::ssa::SSAArtifact;
use crate::symbol::SymbolTableArtifact;
use std::collections::{HashMap, HashSet, VecDeque};

/// Anderson Points-To Analysis Fixpoint Solver
pub struct AndersonPointsTo;

impl AndersonPointsTo {
    /// Solve points-to relation `pts: SsaId -> Set<AllocTypeSymId>` over all SSA variables
    pub fn run(
        ssa: &SSAArtifact,
        bpa: &BPASTArtifact,
        _sta: &SymbolTableArtifact,
    ) -> HashMap<u32, HashSet<u32>> {
        let mut pts: HashMap<u32, HashSet<u32>> = HashMap::new();
        let mut copy_edges: HashMap<u32, Vec<u32>> = HashMap::new();
        let mut worklist: VecDeque<u32> = VecDeque::new();

        // Pass 1: Process Allocation Sites (seed pts) and Copy Edges across functions
        for func in &ssa.functions {
            for rec in &func.ssa_records {
                let v = rec.ssa_id;
                if rec.def_stmt != u32::MAX {
                    let ntype = bpa.node_type(rec.def_stmt);
                    if ntype == NN_NEW_EXPR {
                        let alloc_type = rec.orig_sym_id;
                        pts.entry(v).or_default().insert(alloc_type);
                        worklist.push_back(v);
                    }
                }
                if rec.is_phi() {
                    for phi in &func.phi_records {
                        if phi.ssa_id == v {
                            for arg in &phi.args {
                                copy_edges.entry(arg.arg_ssa_id).or_default().push(v);
                            }
                        }
                    }
                }
            }
        }

        // Pass 2: Worklist Fixpoint Propagation
        while let Some(v) = worklist.pop_front() {
            let v_pts = match pts.get(&v) {
                Some(set) => set.clone(),
                None => continue,
            };

            if let Some(succs) = copy_edges.get(&v) {
                for &w in succs {
                    let w_pts = pts.entry(w).or_default();
                    let mut changed = false;
                    for &alloc in &v_pts {
                        if w_pts.insert(alloc) {
                            changed = true;
                        }
                    }
                    if changed {
                        worklist.push_back(w);
                    }
                }
            }
        }

        pts
    }
}
