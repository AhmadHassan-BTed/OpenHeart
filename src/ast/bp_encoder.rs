//! Balanced Parentheses Bitstring Encoder.
//! Packs BP bits MSB-first into u64 words for cache alignment.

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BPEncoder {
    pub words: Vec<u64>,
    pub bit_count: usize,
}

impl BPEncoder {
    pub fn new() -> Self {
        Self {
            words: Vec::with_capacity(32),
            bit_count: 0,
        }
    }

    pub fn with_node_capacity(node_count: usize) -> Self {
        let bit_cap = node_count * 2;
        let word_cap = bit_cap.div_ceil(64);
        Self {
            words: Vec::with_capacity(word_cap),
            bit_count: 0,
        }
    }

    /// Push an open parenthesis (bit 1: pre-order visit).
    #[inline]
    pub fn push_open(&mut self) {
        self.push_bit(1);
    }

    /// Push a close parenthesis (bit 0: post-order backtrack).
    #[inline]
    pub fn push_close(&mut self) {
        self.push_bit(0);
    }

    #[inline]
    pub fn push_bit(&mut self, bit: u8) {
        let word_idx = self.bit_count / 64;
        let bit_pos = 63 - (self.bit_count % 64);

        if word_idx >= self.words.len() {
            self.words.push(0);
        }

        if bit != 0 {
            self.words[word_idx] |= 1u64 << bit_pos;
        }
        self.bit_count += 1;
    }

    /// Returns the bit at 0-indexed position i (1 or 0).
    #[inline]
    pub fn get_bit(&self, i: usize) -> u8 {
        if i >= self.bit_count {
            return 0;
        }
        let word_idx = i / 64;
        let bit_pos = 63 - (i % 64);
        ((self.words[word_idx] >> bit_pos) & 1) as u8
    }

    pub fn len(&self) -> usize {
        self.bit_count
    }

    pub fn is_empty(&self) -> bool {
        self.bit_count == 0
    }
}
