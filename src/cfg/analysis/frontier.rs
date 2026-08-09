//! Dominance Frontier Computation using Cytron et al. 1991 join-point algorithm.

pub fn compute_dominance_frontiers(
    n: usize,
    _succs: &[Vec<u32>],
    preds: &[Vec<u32>],
    idom: &[u32],
) -> Vec<Vec<u32>> {
    let mut df = vec![Vec::new(); n];

    for b in 0..n as u32 {
        let b_idx = b as usize;
        if b_idx >= preds.len() || b_idx >= idom.len() {
            continue;
        }

        let preds_b = &preds[b_idx];
        if preds_b.len() >= 2 {
            for &p in preds_b {
                let mut runner = p;
                let target_dom = idom[b_idx];

                while (runner as usize) < n && runner != target_dom && runner != u32::MAX {
                    let runner_idx = runner as usize;
                    if !df[runner_idx].contains(&b) {
                        df[runner_idx].push(b);
                    }
                    let next_runner = idom[runner_idx];
                    if next_runner == runner {
                        break; // reached root
                    }
                    runner = next_runner;
                }
            }
        }
    }

    // Sort each DF list for deterministic ordering
    for list in &mut df {
        list.sort_unstable();
    }

    df
}
