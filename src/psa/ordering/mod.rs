//! VariableOrdering — the BDD level ↔ path-variable bijection (§8.2.4, §8.3).
//!
//! The VariableOrdering is the bridge between abstract BDD levels and the concrete objects
//! the path formula talks about:
//!
//! - one variable per CFA edge (`VarKey::Edge(edge_id)`, `edge_id` = position in
//!   `FunctionCFGData.edges[]`, so parallel edges are distinct variables), and
//! - one variable per loop header (`VarKey::LoopEntry(block_id)`): the Ball-Larus dummy edge
//!   `ENTRY → header` that starts an acyclic path segment after a back edge.
//!
//! Invariants:
//! - `level_to_code[level]` is the encoded key of the variable at that level.
//! - `level_of_edge` / `level_of_loop_entry` are its exact inverse.
//! - At most [`MAX_BDD_VARS`] variables: node labels are `u16` and `0xFFFF` marks terminals.
//!
//! **Serialized form (§8.6 Variable Ordering Tables):** one `u32` code per level. Codes below
//! [`LOOP_ENTRY_FLAG`] are CFA edge ids; `LOOP_ENTRY_FLAG | block_id` is the loop-entry
//! variable of that header block.

pub mod force;
pub mod rpo;
pub mod sifting;

pub use force::{build_constraint_hyperedges, force_ordering};
pub use rpo::locality_seed;
pub use sifting::{SiftResult, SiftingConfig, SiftingOptimizer};

/// Marker bit of loop-entry variable codes in serialized ordering tables.
pub const LOOP_ENTRY_FLAG: u32 = 0x8000_0000;

/// Largest number of variables a function's ROBDD can have (`0xFFFF` is the terminal label).
pub const MAX_BDD_VARS: usize = 0xFFFF;

/// Sentinel for "this edge has no variable" in the inverse tables.
pub const NO_LEVEL: u16 = u16::MAX;

/// What a BDD variable stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum VarKey {
    /// The CFA edge with this id is taken.
    Edge(u32),
    /// The acyclic path segment starts at this loop header (Ball-Larus dummy entry edge).
    LoopEntry(u32),
}

impl VarKey {
    /// Encoded `u32` form used in the `.psa` ordering tables.
    #[inline]
    pub fn code(self) -> u32 {
        match self {
            VarKey::Edge(e) => e,
            VarKey::LoopEntry(b) => LOOP_ENTRY_FLAG | b,
        }
    }

    /// Decode a `.psa` ordering-table code.
    #[inline]
    pub fn from_code(code: u32) -> VarKey {
        if code & LOOP_ENTRY_FLAG != 0 {
            VarKey::LoopEntry(code & !LOOP_ENTRY_FLAG)
        } else {
            VarKey::Edge(code)
        }
    }
}

/// The bijection between BDD levels (0..n_vars-1) and path variables.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VariableOrdering {
    /// `level_to_code[level]` = encoded [`VarKey`].
    level_to_code: Vec<u32>,
    /// `edge_to_level[edge_id]` = level, or [`NO_LEVEL`].
    edge_to_level: Vec<u16>,
    /// `(header_block, level)` sorted by header block.
    loop_entry_to_level: Vec<(u32, u16)>,
}

impl VariableOrdering {
    /// An ordering with no variables.
    pub fn empty() -> Self {
        Self {
            level_to_code: Vec::new(),
            edge_to_level: Vec::new(),
            loop_entry_to_level: Vec::new(),
        }
    }

    /// Build an ordering from keys listed top level first.
    ///
    /// Fails when there are more than [`MAX_BDD_VARS`] keys, when a key repeats, when a key
    /// cannot be encoded (edge id or block id with the [`LOOP_ENTRY_FLAG`] bit set), or when
    /// an edge id is implausibly large for the number of variables (corrupt input).
    pub fn from_keys(keys: &[VarKey]) -> Result<Self, String> {
        if keys.len() > MAX_BDD_VARS {
            return Err(format!(
                "{} path variables exceed the ROBDD limit of {}",
                keys.len(),
                MAX_BDD_VARS
            ));
        }
        // Validate encodability before sizing any table by an edge id.
        for key in keys {
            let id = match *key {
                VarKey::Edge(e) => e,
                VarKey::LoopEntry(b) => b,
            };
            if id & LOOP_ENTRY_FLAG != 0 {
                return Err(format!("{:?} cannot be encoded in an ordering table", key));
            }
        }
        let max_edge = keys
            .iter()
            .filter_map(|k| match k {
                VarKey::Edge(e) => Some(*e as usize + 1),
                VarKey::LoopEntry(_) => None,
            })
            .max()
            .unwrap_or(0);
        if max_edge > keys.len().saturating_mul(64).max(1 << 16) {
            // Edge ids are dense CFA positions; an id far beyond the number of variables
            // cannot come from a real function and would only inflate the inverse table.
            return Err(format!(
                "edge id {} is implausible for {} variables",
                max_edge - 1,
                keys.len()
            ));
        }
        let mut edge_to_level = vec![NO_LEVEL; max_edge];
        let mut loop_entry_to_level: Vec<(u32, u16)> = Vec::new();
        let mut level_to_code = Vec::with_capacity(keys.len());

        for (level, &key) in keys.iter().enumerate() {
            let level = level as u16;
            match key {
                VarKey::Edge(e) => {
                    let slot = &mut edge_to_level[e as usize];
                    if *slot != NO_LEVEL {
                        return Err(format!("edge {} has two variables", e));
                    }
                    *slot = level;
                }
                VarKey::LoopEntry(b) => loop_entry_to_level.push((b, level)),
            }
            level_to_code.push(key.code());
        }

        loop_entry_to_level.sort_unstable();
        if loop_entry_to_level.windows(2).any(|w| w[0].0 == w[1].0) {
            return Err("a loop header has two loop-entry variables".to_string());
        }

        Ok(Self {
            level_to_code,
            edge_to_level,
            loop_entry_to_level,
        })
    }

    /// Rebuild an ordering from a serialized table of codes.
    pub fn from_codes(codes: &[u32]) -> Result<Self, String> {
        let keys: Vec<VarKey> = codes.iter().map(|&c| VarKey::from_code(c)).collect();
        Self::from_keys(&keys)
    }

    /// The ordering after a level permutation: new level `l` holds the variable that was at
    /// level `level_to_old_level[l]`.
    pub fn permuted(&self, level_to_old_level: &[u16]) -> Result<Self, String> {
        if level_to_old_level.len() != self.level_to_code.len() {
            return Err(format!(
                "permutation has {} entries for {} variables",
                level_to_old_level.len(),
                self.level_to_code.len()
            ));
        }
        let mut keys = Vec::with_capacity(level_to_old_level.len());
        for &old in level_to_old_level {
            let code = *self
                .level_to_code
                .get(old as usize)
                .ok_or_else(|| format!("permutation references level {}", old))?;
            keys.push(VarKey::from_code(code));
        }
        Self::from_keys(&keys)
    }

    /// The level of the variable of CFA edge `edge_id`, if it has one.
    #[inline]
    pub fn level_of_edge(&self, edge_id: u32) -> Option<u16> {
        match self.edge_to_level.get(edge_id as usize) {
            Some(&l) if l != NO_LEVEL => Some(l),
            _ => None,
        }
    }

    /// The level of the loop-entry variable of header `block_id`, if it has one.
    #[inline]
    pub fn level_of_loop_entry(&self, block_id: u32) -> Option<u16> {
        self.loop_entry_to_level
            .binary_search_by_key(&block_id, |&(b, _)| b)
            .ok()
            .map(|i| self.loop_entry_to_level[i].1)
    }

    /// The variable at `level`.
    #[inline]
    pub fn key_at(&self, level: u16) -> Option<VarKey> {
        self.level_to_code
            .get(level as usize)
            .map(|&c| VarKey::from_code(c))
    }

    /// Returns the total number of Boolean variables.
    #[inline]
    pub fn n_vars(&self) -> u16 {
        self.level_to_code.len() as u16
    }

    /// The encoded keys, one per level, for PSA serialization (§8.6).
    #[inline]
    pub fn codes(&self) -> &[u32] {
        &self.level_to_code
    }
}
