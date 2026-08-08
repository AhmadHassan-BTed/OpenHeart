//! Jump Table Builder for O(1) matching parenthesis lookup.

use super::bp_encoder::BPEncoder;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JumpTable {
    pub match_pos: Vec<u32>,
}

pub struct JumpTableBuilder;

impl JumpTableBuilder {
    /// Builds the O(n) match_pos lookup table from a completed BPEncoder sequence.
    pub fn build(bp: &BPEncoder) -> JumpTable {
        let n_bits = bp.bit_count;
        let mut match_pos = vec![0u32; n_bits];
        let mut stack: Vec<u32> = Vec::with_capacity(512);

        for i in 0..n_bits {
            if bp.get_bit(i) == 1 {
                stack.push(i as u32);
            } else {
                let open = stack.pop().expect("Malformed BP: unmatched close paren");
                match_pos[open as usize] = i as u32;
                match_pos[i] = open;
            }
        }

        assert!(stack.is_empty(), "Malformed BP: unclosed open parens");
        JumpTable { match_pos }
    }
}
