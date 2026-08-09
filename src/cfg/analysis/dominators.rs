//! Cooper–Harvey–Kennedy Iterative Dominator Algorithm (Cooper et al. 2001).

pub const UNDEFINED_IDOM: u32 = u32::MAX;

/// Computes `idom[b]` for all blocks `b` in function CFG using Cooper et al. 2001.
/// Returns `idom[]` array where `idom[b] = immediate dominator of b`.
/// `idom[ENTRY]` = ENTRY (block 0 dominates itself).
pub fn compute_idom_cooper(n: usize, preds: &[Vec<u32>], rpo: &[u32]) -> Vec<u32> {
    if n == 0 || rpo.is_empty() {
        return Vec::new();
    }

    let mut rpo_num = vec![0u32; n];
    for (pos, &b) in rpo.iter().enumerate() {
        if (b as usize) < n {
            rpo_num[b as usize] = pos as u32;
        }
    }

    let mut idom = vec![UNDEFINED_IDOM; n];
    let entry = rpo[0] as usize;
    if entry < n {
        idom[entry] = rpo[0];
    }

    let mut changed = true;
    while changed {
        changed = false;

        for &b in &rpo[1..] {
            let b_idx = b as usize;
            if b_idx >= n {
                continue;
            }

            let processed_preds: Vec<u32> = preds[b_idx]
                .iter()
                .copied()
                .filter(|&p| (p as usize) < n && idom[p as usize] != UNDEFINED_IDOM)
                .collect();

            if processed_preds.is_empty() {
                continue;
            }

            let mut new_idom = processed_preds[0];
            for &p in &processed_preds[1..] {
                new_idom = intersect(p, new_idom, &idom, &rpo_num);
            }

            if idom[b_idx] != new_idom {
                idom[b_idx] = new_idom;
                changed = true;
            }
        }
    }

    idom
}

#[inline]
fn intersect(mut b1: u32, mut b2: u32, idom: &[u32], rpo_num: &[u32]) -> u32 {
    let n = idom.len();
    while b1 != b2 {
        while (b1 as usize) < n && (b2 as usize) < n && rpo_num[b1 as usize] > rpo_num[b2 as usize]
        {
            b1 = idom[b1 as usize];
        }
        while (b1 as usize) < n && (b2 as usize) < n && rpo_num[b2 as usize] > rpo_num[b1 as usize]
        {
            b2 = idom[b2 as usize];
        }
    }
    b1
}
