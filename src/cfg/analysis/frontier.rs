//! Dominance Frontier Computation using Cytron et al. 1991 join-point algorithm
//! (in the runner formulation of Cooper, Harvey & Kennedy 2001).

use super::dominators::UNDEFINED_IDOM;

pub fn compute_dominance_frontiers(
    n: usize,
    _succs: &[Vec<u32>],
    preds: &[Vec<u32>],
    idom: &[u32],
) -> Vec<Vec<u32>> {
    let mut df = vec![Vec::new(); n];

    for b in 0..n as u32 {
        let b_idx = b as usize;
        if b_idx >= preds.len() || b_idx >= idom.len() || idom[b_idx] == UNDEFINED_IDOM {
            continue;
        }

        let preds_b = &preds[b_idx];
        if preds_b.len() < 2 {
            continue;
        }
        let target_dom = idom[b_idx];
        for &p in preds_b {
            let mut runner = p;
            while (runner as usize) < n
                && runner != target_dom
                && idom[runner as usize] != UNDEFINED_IDOM
            {
                let runner_idx = runner as usize;
                // All insertions for a given `b` happen in this loop, so checking the last
                // element is enough to avoid duplicates.
                if df[runner_idx].last() != Some(&b) {
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

    // `b` is visited in increasing order, so every list is already sorted.
    df
}
