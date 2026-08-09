//! Tarjan's SCC Algorithm for Call Graph Recursive Cycle Detection (§6.2.6).
//! Authored solely by Ahmad Hassan (B-Ted).

use crate::core::types::*;

pub struct TarjanSCC;

impl TarjanSCC {
    /// Compute SCCs over the Call Graph adjacency and return SCCRecords + flat member array
    pub fn compute(method_count: usize, adj: &[Vec<u32>]) -> (Vec<SCCRecord>, Vec<u32>) {
        let mut index_counter = 0u32;
        let mut stack = Vec::new();
        let mut on_stack = vec![false; method_count];
        let mut index = vec![u32::MAX; method_count];
        let mut lowlink = vec![0u32; method_count];
        let mut raw_sccs: Vec<Vec<u32>> = Vec::new();

        for v in 0..method_count as u32 {
            if index[v as usize] == u32::MAX {
                Self::strongconnect(
                    v,
                    adj,
                    &mut index_counter,
                    &mut stack,
                    &mut on_stack,
                    &mut index,
                    &mut lowlink,
                    &mut raw_sccs,
                );
            }
        }

        let mut scc_records = Vec::new();
        let mut scc_members = Vec::new();

        for (scc_id, raw_members) in raw_sccs.iter().enumerate() {
            let offset = scc_members.len() as u32;
            let count = raw_members.len() as u16;

            let scc_class = if count > 1 {
                SCC_CLASS_MUTUAL_RECURSIVE
            } else if count == 1 {
                let v = raw_members[0];
                if adj[v as usize].contains(&v) {
                    SCC_CLASS_SELF_RECURSIVE
                } else {
                    SCC_CLASS_NON_RECURSIVE
                }
            } else {
                SCC_CLASS_NON_RECURSIVE
            };

            scc_records.push(SCCRecord::new(scc_id as u32, offset, count, scc_class));

            for &m in raw_members {
                scc_members.push(m);
            }
        }

        (scc_records, scc_members)
    }

    fn strongconnect(
        v: u32,
        adj: &[Vec<u32>],
        counter: &mut u32,
        stack: &mut Vec<u32>,
        on_stack: &mut Vec<bool>,
        index: &mut Vec<u32>,
        lowlink: &mut Vec<u32>,
        sccs: &mut Vec<Vec<u32>>,
    ) {
        index[v as usize] = *counter;
        lowlink[v as usize] = *counter;
        *counter += 1;
        stack.push(v);
        on_stack[v as usize] = true;

        if let Some(neighbors) = adj.get(v as usize) {
            for &w in neighbors {
                if (w as usize) >= index.len() {
                    continue;
                }
                if index[w as usize] == u32::MAX {
                    Self::strongconnect(w, adj, counter, stack, on_stack, index, lowlink, sccs);
                    lowlink[v as usize] = lowlink[v as usize].min(lowlink[w as usize]);
                } else if on_stack[w as usize] {
                    lowlink[v as usize] = lowlink[v as usize].min(index[w as usize]);
                }
            }
        }

        if lowlink[v as usize] == index[v as usize] {
            let mut scc = Vec::new();
            loop {
                let w = stack.pop().unwrap();
                on_stack[w as usize] = false;
                scc.push(w);
                if w == v {
                    break;
                }
            }
            sccs.push(scc);
        }
    }
}
