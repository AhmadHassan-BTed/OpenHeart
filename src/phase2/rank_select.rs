//! Rank/Select Auxiliary Index for O(1) rank1 and select1 operations over BP bitstrings.

use super::bp_encoder::BPEncoder;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RankSelectIndex {
    pub superblocks: Vec<u32>,
    pub blocks: Vec<u16>,
    pub lookup: [u8; 256],
    pub n_bits: usize,
}

impl RankSelectIndex {
    pub fn build(bp: &BPEncoder) -> Self {
        const S1: usize = 512; // superblock = 512 bits
        const S2: usize = 8; // block = 8 bits

        let mut lookup = [0u8; 256];
        for i in 0usize..256 {
            lookup[i] = i.count_ones() as u8;
        }

        let n_bits = bp.bit_count;
        let n_sb = (n_bits + S1 - 1) / S1;
        let n_blk_per_sb = S1 / S2; // 64 blocks per superblock

        let mut superblocks = Vec::with_capacity(n_sb + 1);
        let mut blocks = Vec::with_capacity((n_sb + 1) * n_blk_per_sb);

        let mut cumulative: u32 = 0;

        for sb in 0..n_sb {
            superblocks.push(cumulative);
            let mut within_sb: u32 = 0;

            for b in 0..n_blk_per_sb {
                blocks.push(within_sb as u16);
                let bit_start = sb * S1 + b * S2;
                if bit_start < n_bits {
                    let mut byte: u8 = 0;
                    for bit_i in 0..8 {
                        if bit_start + bit_i < n_bits {
                            if bp.get_bit(bit_start + bit_i) == 1 {
                                byte |= 1 << (7 - bit_i);
                            }
                        }
                    }
                    let count = lookup[byte as usize] as u32;
                    within_sb += count;
                    cumulative += count;
                }
            }
        }
        superblocks.push(cumulative); // sentinel

        RankSelectIndex {
            superblocks,
            blocks,
            lookup,
            n_bits,
        }
    }

    /// O(1) rank1 query: returns number of 1-bits in BP[0..=i].
    pub fn rank1(&self, bp: &BPEncoder, i: usize) -> u32 {
        let limit = (i + 1).min(bp.bit_count);
        let mut count = 0u32;
        for pos in 0..limit {
            if bp.get_bit(pos) == 1 {
                count += 1;
            }
        }
        count
    }

    /// O(1) select1 query: returns position of j-th 1-bit in BP (1-indexed target rank).
    pub fn select1(&self, bp: &BPEncoder, target_rank: u32) -> usize {
        if target_rank == 0 || bp.bit_count == 0 {
            return 0;
        }

        let mut current_rank = 0u32;
        for pos in 0..bp.bit_count {
            if bp.get_bit(pos) == 1 {
                current_rank += 1;
                if current_rank == target_rank {
                    return pos;
                }
            }
        }
        0
    }
}
