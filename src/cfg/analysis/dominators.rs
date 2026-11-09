//! Cooper–Harvey–Kennedy Iterative Dominator Algorithm (Cooper et al. 2001).

pub const UNDEFINED_IDOM: u32 = u32::MAX;

/// Computes `idom[b]` for all blocks `b` using Cooper et al. 2001.
///
/// `rpo` is a reverse post-order of the nodes reachable from the root `rpo[0]`; nodes not
/// listed keep `UNDEFINED_IDOM`. `idom[root] = root`. Running it on the reversed graph
/// (with `preds` = forward successors and a reverse-graph RPO) yields post-dominators.
pub fn compute_idom_cooper(n: usize, preds: &[Vec<u32>], rpo: &[u32]) -> Vec<u32> {
    if n == 0 || rpo.is_empty() {
        return Vec::new();
    }

    let mut rpo_num = vec![u32::MAX; n];
    for (pos, &b) in rpo.iter().enumerate() {
        if (b as usize) < n {
            rpo_num[b as usize] = pos as u32;
        }
    }

    let mut idom = vec![UNDEFINED_IDOM; n];
    let root = rpo[0];
    if (root as usize) >= n {
        return idom;
    }
    idom[root as usize] = root;

    let mut changed = true;
    while changed {
        changed = false;

        for &b in &rpo[1..] {
            let b_idx = b as usize;
            if b_idx >= n {
                continue;
            }

            let mut new_idom = UNDEFINED_IDOM;
            for &p in preds.get(b_idx).map(|v| v.as_slice()).unwrap_or(&[]) {
                let p_idx = p as usize;
                if p_idx >= n || idom[p_idx] == UNDEFINED_IDOM {
                    continue;
                }
                new_idom = if new_idom == UNDEFINED_IDOM {
                    p
                } else {
                    intersect(p, new_idom, &idom, &rpo_num)
                };
            }

            if new_idom != UNDEFINED_IDOM && idom[b_idx] != new_idom {
                idom[b_idx] = new_idom;
                changed = true;
            }
        }
    }

    idom
}

/// Walks both fingers up the (partial) dominator tree until they meet. Every node with a
/// defined `idom` is in `rpo`, and the root has the smallest RPO number and is its own
/// `idom`, so both loops terminate.
#[inline]
fn intersect(mut b1: u32, mut b2: u32, idom: &[u32], rpo_num: &[u32]) -> u32 {
    while b1 != b2 {
        while rpo_num[b1 as usize] > rpo_num[b2 as usize] {
            b1 = idom[b1 as usize];
        }
        while rpo_num[b2 as usize] > rpo_num[b1 as usize] {
            b2 = idom[b2 as usize];
        }
    }
    b1
}

/// Dominator tree with O(1) ancestry queries (DFS entry/exit numbering).
#[derive(Debug, Clone)]
pub struct DominatorTree {
    pub root: u32,
    pub children: Vec<Vec<u32>>,
    pre: Vec<u32>,
    post: Vec<u32>,
}

impl DominatorTree {
    /// Builds the tree from an `idom` array whose root satisfies `idom[root] == root`.
    pub fn new(idom: &[u32], root: u32) -> Self {
        let n = idom.len();
        let mut children = vec![Vec::new(); n];
        for (node, &parent) in idom.iter().enumerate() {
            if node as u32 != root && parent != UNDEFINED_IDOM && (parent as usize) < n {
                children[parent as usize].push(node as u32);
            }
        }

        let mut pre = vec![u32::MAX; n];
        let mut post = vec![u32::MAX; n];
        if (root as usize) < n {
            let mut clock = 0u32;
            let mut stack: Vec<(u32, usize)> = vec![(root, 0)];
            pre[root as usize] = clock;
            clock += 1;
            while let Some(top) = stack.last_mut() {
                let b = top.0 as usize;
                if top.1 < children[b].len() {
                    let c = children[b][top.1];
                    top.1 += 1;
                    pre[c as usize] = clock;
                    clock += 1;
                    stack.push((c, 0));
                } else {
                    post[b] = clock;
                    clock += 1;
                    stack.pop();
                }
            }
        }

        Self {
            root,
            children,
            pre,
            post,
        }
    }

    /// True when `a` dominates `b` (reflexive). Nodes outside the tree dominate nothing
    /// and are dominated by nothing.
    #[inline]
    pub fn dominates(&self, a: u32, b: u32) -> bool {
        let (ai, bi) = (a as usize, b as usize);
        if ai >= self.pre.len() || bi >= self.pre.len() {
            return false;
        }
        let (pa, pb) = (self.pre[ai], self.pre[bi]);
        if pa == u32::MAX || pb == u32::MAX {
            return false;
        }
        pa <= pb && self.post[bi] <= self.post[ai]
    }

    #[inline]
    pub fn contains(&self, b: u32) -> bool {
        (b as usize) < self.pre.len() && self.pre[b as usize] != u32::MAX
    }
}
