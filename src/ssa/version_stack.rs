//! Version Stack for Dominator Tree DFS Renaming (§5.2.3).
//! Authored by Ahmad Hassan (B-Ted).

use std::collections::HashMap;

#[derive(Debug, Clone, Default)]
pub struct VersionStack {
    stacks: HashMap<u32, Vec<u32>>,
}

impl VersionStack {
    pub fn new() -> Self {
        Self {
            stacks: HashMap::new(),
        }
    }

    pub fn push(&mut self, orig_sym: u32, ssa_id: u32) {
        self.stacks.entry(orig_sym).or_default().push(ssa_id);
    }

    pub fn top(&self, orig_sym: u32) -> u32 {
        self.stacks
            .get(&orig_sym)
            .and_then(|st| st.last().copied())
            .unwrap_or(u32::MAX)
    }

    pub fn save_depths(&self) -> HashMap<u32, usize> {
        self.stacks
            .iter()
            .map(|(&sym, st)| (sym, st.len()))
            .collect()
    }

    pub fn restore_to(&mut self, saved_depths: &HashMap<u32, usize>) {
        for (sym, stack) in &mut self.stacks {
            let target_len = saved_depths.get(sym).copied().unwrap_or(0);
            stack.truncate(target_len);
        }
    }
}
