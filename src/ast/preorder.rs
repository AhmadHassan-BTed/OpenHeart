//! Dense Parallel Pre-order Arrays for AST Node Attributes, Types, Token Ranges, Parents
//! and Name Tokens.

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PreorderArrays {
    pub node_types: Vec<u8>,
    pub node_attrs: Vec<u32>,
    pub token_ranges: Vec<(u32, u32)>,
    pub parent_map: Vec<u32>,
    /// Token id of the identifier that names the node (a declaration's `name` field, a
    /// declarator's or parameter's name, a call's method name), or `u32::MAX`.
    pub name_tokens: Vec<u32>,
}

impl PreorderArrays {
    pub fn new() -> Self {
        Self::with_capacity(1024)
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            node_types: Vec::with_capacity(capacity),
            node_attrs: Vec::with_capacity(capacity),
            token_ranges: Vec::with_capacity(capacity),
            parent_map: Vec::with_capacity(capacity),
            name_tokens: Vec::with_capacity(capacity),
        }
    }

    pub fn len(&self) -> usize {
        self.node_types.len()
    }

    pub fn is_empty(&self) -> bool {
        self.node_types.is_empty()
    }

    /// Heap bytes held by the arrays.
    pub fn heap_bytes(&self) -> usize {
        self.node_types.len()
            + self.node_attrs.len() * 4
            + self.token_ranges.len() * 8
            + self.parent_map.len() * 4
            + self.name_tokens.len() * 4
    }
}
