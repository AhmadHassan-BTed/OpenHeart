//! Sparse Table Range Minimum Query over the excess depth sequence for O(1) LCA.

use super::bp_encoder::BPEncoder;
use super::rank_select::RankSelectIndex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SparseTableRMQ {
    pub table: Vec<Vec<u32>>,
    pub excess: Vec<i32>,
    pub log2: Vec<usize>,
}

impl SparseTableRMQ {
    pub fn build(bp: &BPEncoder, rs: &RankSelectIndex) -> Self {
        let n = bp.bit_count;
        if n == 0 {
            return Self {
                table: Vec::new(),
                excess: Vec::new(),
                log2: Vec::new(),
            };
        }

        let excess: Vec<i32> = (0..n)
            .map(|i| 2 * rs.rank1(bp, i) as i32 - i as i32 - 1)
            .collect();

        let log_n = (usize::BITS - n.leading_zeros()) as usize;
        let mut table = vec![(0..n as u32).collect::<Vec<u32>>()];

        for k in 1..=log_n {
            let half = 1usize << (k - 1);
            let len = n.saturating_sub(1usize << k) + 1;
            let mut level = Vec::with_capacity(len);
            for i in 0..len {
                let left = table[k - 1][i];
                let right = if i + half < table[k - 1].len() {
                    table[k - 1][i + half]
                } else {
                    left
                };
                level.push(if excess[left as usize] <= excess[right as usize] {
                    left
                } else {
                    right
                });
            }
            table.push(level);
        }

        let mut log2 = vec![0usize; n + 1];
        for i in 2..=n {
            log2[i] = log2[i / 2] + 1;
        }

        SparseTableRMQ {
            table,
            excess,
            log2,
        }
    }

    /// Range minimum query: returns position with minimum excess in range [l, r].
    pub fn range_min(&self, l: usize, r: usize) -> u32 {
        if l >= self.excess.len() || r >= self.excess.len() {
            return 0;
        }
        let (l, r) = if l <= r { (l, r) } else { (r, l) };
        let len = r - l + 1;
        let k = self.log2[len];
        let a = self.table[k][l];
        let b = self.table[k][r + 1 - (1 << k)];
        if self.excess[a as usize] <= self.excess[b as usize] {
            a
        } else {
            b
        }
    }

    /// O(1) Lowest Common Ancestor (LCA) query for two AST nodes by pre-order indices.
    pub fn lca(&self, bp: &BPEncoder, rs: &RankSelectIndex, u: u32, v: u32) -> u32 {
        if u == v {
            return u;
        }
        let op_u = rs.select1(bp, u + 1);
        let op_v = rs.select1(bp, v + 1);
        let l = op_u.min(op_v);
        let r = op_u.max(op_v);
        let min_pos = self.range_min(l.saturating_sub(1), r);
        rs.rank1(bp, min_pos as usize).saturating_sub(1)
    }
}
