//! Dense Parallel Pre-order Arrays for AST Node Attributes, Types, Token Ranges, and Parents.

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PreorderArrays {
    pub node_types: Vec<u8>,
    pub node_attrs: Vec<u32>,
    pub token_ranges: Vec<(u32, u32)>,
    pub parent_map: Vec<u32>,
}

impl PreorderArrays {
    pub fn new() -> Self {
        Self {
            node_types: Vec::with_capacity(1024),
            node_attrs: Vec::with_capacity(1024),
            token_ranges: Vec::with_capacity(1024),
            parent_map: Vec::with_capacity(1024),
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            node_types: Vec::with_capacity(capacity),
            node_attrs: Vec::with_capacity(capacity),
            token_ranges: Vec::with_capacity(capacity),
            parent_map: Vec::with_capacity(capacity),
        }
    }

    pub fn len(&self) -> usize {
        self.node_types.len()
    }

    pub fn is_empty(&self) -> bool {
        self.node_types.is_empty()
    }
}
