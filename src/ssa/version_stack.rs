//! Version Stack for Dominator Tree DFS Renaming (§5.2.3).
//! Authored by Ahmad Hassan (B-Ted).
//!
//! One stack of SSA ids per dense variable id, plus a push log: leaving a dominator-tree
//! node pops exactly the versions pushed while it was processed, so no per-block snapshot
//! of the whole stack map is needed.

#[derive(Debug, Clone, Default)]
pub struct VersionStack {
    stacks: Vec<Vec<u32>>,
    log: Vec<u32>,
}

impl VersionStack {
    pub fn new(var_count: usize) -> Self {
        Self {
            stacks: vec![Vec::new(); var_count],
            log: Vec::new(),
        }
    }

    pub fn push(&mut self, var: u32, ssa_id: u32) {
        if let Some(st) = self.stacks.get_mut(var as usize) {
            st.push(ssa_id);
            self.log.push(var);
        }
    }

    /// Current SSA id of `var`, `u32::MAX` when no definition reaches this point.
    pub fn top(&self, var: u32) -> u32 {
        self.stacks
            .get(var as usize)
            .and_then(|st| st.last().copied())
            .unwrap_or(u32::MAX)
    }

    /// Position to return to when the current dominator-tree node is left.
    pub fn mark(&self) -> usize {
        self.log.len()
    }

    /// Pops every version pushed since `mark`.
    pub fn restore_to(&mut self, mark: usize) {
        while self.log.len() > mark {
            if let Some(var) = self.log.pop() {
                if let Some(st) = self.stacks.get_mut(var as usize) {
                    st.pop();
                }
            }
        }
    }
}
