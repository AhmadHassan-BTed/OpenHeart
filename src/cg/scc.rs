//! Tarjan's SCC Algorithm for Call Graph Recursive Cycle Detection (§6.2.6).
//! Authored solely by Ahmad Hassan (B-Ted).
//!
//! Iterative formulation (explicit call stack), so call chains thousands of methods deep
//! cannot overflow the thread stack.
//!
//! **Output order.** Tarjan emits an SCC only after every SCC reachable from it, so the
//! records come out in reverse topological order of the condensation: callees before
//! their callers. Consumers that need callers first must iterate in reverse.

use crate::core::types::*;

pub struct TarjanSCC;

impl TarjanSCC {
    /// SCCs over all ids `0..method_count`.
    pub fn compute(method_count: usize, adj: &[Vec<u32>]) -> Result<(Vec<SCCRecord>, Vec<u32>), String> {
        let nodes: Vec<u32> = (0..method_count as u32).collect();
        Self::compute_over(&nodes, adj)
    }

    /// SCCs of the subgraph induced by `nodes` (ids index `adj`); returns the SCC
    /// records (callee-first, see module docs) and the flat member array.
    pub fn compute_over(nodes: &[u32], adj: &[Vec<u32>]) -> Result<(Vec<SCCRecord>, Vec<u32>), String> {
        let n = adj.len();
        let mut in_set = vec![false; n];
        for &v in nodes {
            if (v as usize) < n {
                in_set[v as usize] = true;
            }
        }

        let mut index = vec![u32::MAX; n];
        let mut lowlink = vec![0u32; n];
        let mut on_stack = vec![false; n];
        let mut stack: Vec<u32> = Vec::new();
        let mut counter = 0u32;
        let mut raw_sccs: Vec<Vec<u32>> = Vec::new();
        let mut call: Vec<(u32, usize)> = Vec::new();

        for &root in nodes {
            let r = root as usize;
            if r >= n || index[r] != u32::MAX {
                continue;
            }
            index[r] = counter;
            lowlink[r] = counter;
            counter += 1;
            stack.push(root);
            on_stack[r] = true;
            call.push((root, 0));

            while let Some(&(v, i)) = call.last() {
                let vi = v as usize;
                if i < adj[vi].len() {
                    if let Some(top) = call.last_mut() {
                        top.1 += 1;
                    }
                    let w = adj[vi][i];
                    let wi = w as usize;
                    if wi >= n || !in_set[wi] {
                        continue;
                    }
                    if index[wi] == u32::MAX {
                        index[wi] = counter;
                        lowlink[wi] = counter;
                        counter += 1;
                        stack.push(w);
                        on_stack[wi] = true;
                        call.push((w, 0));
                    } else if on_stack[wi] {
                        lowlink[vi] = lowlink[vi].min(index[wi]);
                    }
                } else {
                    call.pop();
                    if let Some(&(parent, _)) = call.last() {
                        let pi = parent as usize;
                        lowlink[pi] = lowlink[pi].min(lowlink[vi]);
                    }
                    if lowlink[vi] == index[vi] {
                        let mut scc = Vec::new();
                        while let Some(w) = stack.pop() {
                            on_stack[w as usize] = false;
                            scc.push(w);
                            if w == v {
                                break;
                            }
                        }
                        raw_sccs.push(scc);
                    }
                }
            }
        }

        let mut scc_records = Vec::with_capacity(raw_sccs.len());
        let mut scc_members = Vec::new();

        for (scc_id, raw_members) in raw_sccs.iter().enumerate() {
            let offset = u32::try_from(scc_members.len())
                .map_err(|_| "SCC member array exceeds the u32 range".to_string())?;
            let count = u16::try_from(raw_members.len()).map_err(|_| {
                format!(
                    "SCC {} has {} members, more than SCCRecord.member_count (u16) can hold",
                    scc_id,
                    raw_members.len()
                )
            })?;

            let scc_class = if count > 1 {
                SCC_CLASS_MUTUAL_RECURSIVE
            } else {
                let v = raw_members[0];
                if adj[v as usize].contains(&v) {
                    SCC_CLASS_SELF_RECURSIVE
                } else {
                    SCC_CLASS_NON_RECURSIVE
                }
            };

            let id = u32::try_from(scc_id).map_err(|_| "SCC count exceeds the u32 range".to_string())?;
            scc_records.push(SCCRecord::new(id, offset, count, scc_class));
            scc_members.extend_from_slice(raw_members);
        }

        Ok((scc_records, scc_members))
    }
}
