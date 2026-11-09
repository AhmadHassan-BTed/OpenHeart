//! BuildLevels — BDD levels used while `f_paths` is being constructed (§8.5.2).
//!
//! During construction the BDD has two kinds of variables:
//!
//! - **path variables** ([`VarKey`]: CFA edges and loop entries) — the variables of the
//!   final summary, and
//! - **predicate variables** — one per canonical branch predicate used by the feasibility
//!   filter. They occupy their own levels (never an edge's level, so a predicate can no
//!   longer alias an unrelated edge) and are existentially quantified out before counting.
//!
//! After quantification [`BuildLevels::final_relabel`] removes the predicate levels: the
//! relabelling is strictly increasing on path levels, so it preserves the variable order and
//! canonicity.

use crate::psa::bdd::fxhash::{fx_map_with_capacity, FxHashMap};
use crate::psa::ordering::{VarKey, MAX_BDD_VARS, NO_LEVEL};

/// One construction-time variable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuildVar {
    /// A variable of the final summary.
    Path(VarKey),
    /// A canonical branch predicate (quantified out before counting).
    Predicate(u32),
}

/// Level assignment of every construction-time variable.
pub struct BuildLevels {
    vars: Vec<BuildVar>,
    edge_level: Vec<u16>,
    loop_entry_level: FxHashMap<u32, u16>,
    predicate_level: FxHashMap<u32, u16>,
}

impl BuildLevels {
    /// Assign levels top-down in the order of `vars`.
    ///
    /// Fails when there are more than [`MAX_BDD_VARS`] variables or a variable repeats.
    pub fn new(vars: Vec<BuildVar>, n_edges: usize) -> Result<Self, String> {
        if vars.len() > MAX_BDD_VARS {
            return Err(format!(
                "{} construction variables exceed the ROBDD limit of {}",
                vars.len(),
                MAX_BDD_VARS
            ));
        }
        let mut edge_level = vec![NO_LEVEL; n_edges];
        let mut loop_entry_level: FxHashMap<u32, u16> = fx_map_with_capacity(8);
        let mut predicate_level: FxHashMap<u32, u16> = fx_map_with_capacity(8);
        for (level, var) in vars.iter().enumerate() {
            let level = level as u16;
            let duplicate = match *var {
                BuildVar::Path(VarKey::Edge(e)) => match edge_level.get_mut(e as usize) {
                    Some(slot) if *slot == NO_LEVEL => {
                        *slot = level;
                        false
                    }
                    Some(_) => true,
                    None => return Err(format!("edge {} is outside the CFG", e)),
                },
                BuildVar::Path(VarKey::LoopEntry(b)) => loop_entry_level.insert(b, level).is_some(),
                BuildVar::Predicate(p) => predicate_level.insert(p, level).is_some(),
            };
            if duplicate {
                return Err(format!("construction variable {:?} appears twice", var));
            }
        }
        Ok(Self {
            vars,
            edge_level,
            loop_entry_level,
            predicate_level,
        })
    }

    /// Number of construction levels.
    #[inline]
    pub fn n_levels(&self) -> usize {
        self.vars.len()
    }

    /// Level of edge `e`'s variable (every CFA edge has one).
    #[inline]
    pub fn edge(&self, e: u32) -> Option<u16> {
        match self.edge_level.get(e as usize) {
            Some(&l) if l != NO_LEVEL => Some(l),
            _ => None,
        }
    }

    /// Level of the loop-entry variable of header `block`.
    #[inline]
    pub fn loop_entry(&self, block: u32) -> Option<u16> {
        self.loop_entry_level.get(&block).copied()
    }

    /// Level of canonical predicate `predicate_id`.
    #[inline]
    pub fn predicate(&self, predicate_id: u32) -> Option<u16> {
        self.predicate_level.get(&predicate_id).copied()
    }

    /// Number of predicate levels.
    #[inline]
    pub fn predicate_count(&self) -> usize {
        self.predicate_level.len()
    }

    /// `mask[level]` is true for predicate levels (the variables to quantify out).
    pub fn predicate_mask(&self) -> Vec<bool> {
        self.vars
            .iter()
            .map(|v| matches!(v, BuildVar::Predicate(_)))
            .collect()
    }

    /// `fixed[level]` = `Some(false)` on predicate levels, `None` elsewhere: counting with
    /// this table counts every path once while predicate variables are still present.
    pub fn predicates_fixed_false(&self) -> Vec<Option<bool>> {
        self.vars
            .iter()
            .map(|v| match v {
                BuildVar::Predicate(_) => Some(false),
                BuildVar::Path(_) => None,
            })
            .collect()
    }

    /// The relabelling `old level → new level` that drops predicate levels (predicate
    /// levels map to [`NO_LEVEL`]) and the path keys of the remaining levels, top first.
    pub fn final_relabel(&self) -> (Vec<u16>, Vec<VarKey>) {
        let mut relabel = Vec::with_capacity(self.vars.len());
        let mut keys = Vec::with_capacity(self.vars.len());
        for var in &self.vars {
            match *var {
                BuildVar::Path(key) => {
                    relabel.push(keys.len() as u16);
                    keys.push(key);
                }
                BuildVar::Predicate(_) => relabel.push(NO_LEVEL),
            }
        }
        (relabel, keys)
    }
}
