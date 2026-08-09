//! Qualified Name Table for interning fully qualified names (e.g. "java.util.List").

use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct QualifiedNameTable {
    names: Vec<String>,
    lookup: HashMap<String, u32>,
}

impl Default for QualifiedNameTable {
    fn default() -> Self {
        Self::new()
    }
}

impl QualifiedNameTable {
    pub const BASE_QUAL_NAME_ID: u32 = 0x8000_0000;

    pub fn new() -> Self {
        Self {
            names: Vec::new(),
            lookup: HashMap::new(),
        }
    }

    /// Intern a qualified name, returning its `qual_name_id` (>= 0x80000000).
    pub fn get_or_intern(&mut self, name: &str) -> u32 {
        if let Some(&id) = self.lookup.get(name) {
            return id;
        }

        let index = self.names.len() as u32;
        let id = Self::BASE_QUAL_NAME_ID + index;
        self.names.push(name.to_string());
        self.lookup.insert(name.to_string(), id);
        id
    }

    pub fn get_name(&self, qual_name_id: u32) -> Option<&str> {
        if qual_name_id >= Self::BASE_QUAL_NAME_ID {
            let index = (qual_name_id - Self::BASE_QUAL_NAME_ID) as usize;
            self.names.get(index).map(|s| s.as_str())
        } else {
            self.names.get(qual_name_id as usize).map(|s| s.as_str())
        }
    }

    pub fn lookup_by_id(&self, qual_name_id: u32) -> Option<&str> {
        self.get_name(qual_name_id)
    }

    pub fn len(&self) -> usize {
        self.names.len()
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    pub fn all_names(&self) -> &[String] {
        &self.names
    }
}
