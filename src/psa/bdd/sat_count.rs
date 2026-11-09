//! Exact #SAT and model extraction over ROBDD node arrays (§8.2.5).
//!
//! The number of satisfying assignments of `f_paths` = the number of execution paths
//! through the CFG (one assignment per path, see `construction::constraints`).
//!
//! **Variable gaps:** variables that don't appear on a BDD edge are free — they can take
//! any value. With `level(terminal) = n_vars`:
//!
//! ```text
//! count(TRUE)  = 1          count(FALSE) = 0
//! count(N)     = 2^(level(N.lo) - N.var - 1) · count(N.lo)
//!              + 2^(level(N.hi) - N.var - 1) · count(N.hi)
//! #SAT(f)      = 2^level(f) · count(f)
//! ```
//!
//! Counts are exact ([`PathCount`]); nothing saturates. Variables can additionally be
//! *fixed* (`fixed[v] = Some(b)`): a fixed variable contributes a factor of 1 instead of 2
//! when it falls in a gap, and only its `b` branch is followed where it labels a node. This
//! is what answers path queries such as "paths through edge e" without building new nodes.
//!
//! Time: O(|ROBDD|) big-integer additions — each node is visited once. All traversals are
//! iterative and validate the array as they go (ids in range, strictly increasing variable
//! order), so they are safe on node arrays read back from disk.
//!
//! **Cyclomatic complexity** is NOT derived from the ROBDD. It is computed once from CFA edge
//! and block counts: `V(G) = |E| - |B| + 2` and stored permanently as a `u16` in the
//! function's path metrics record (§8.2.5).

use super::fxhash::{fx_map_with_capacity, FxHashMap};
use super::node::{ROBDDNode, FALSE_ID, TRUE_ID};
use crate::psa::count::PathCount;

/// Level of node `id`: its variable, or `n_vars` for terminals. Validates the node.
fn level_of(nodes: &[ROBDDNode], id: u32, n_vars: usize) -> Result<usize, String> {
    let node = nodes
        .get(id as usize)
        .ok_or_else(|| format!("node id {} out of range ({} nodes)", id, nodes.len()))?;
    if id == FALSE_ID || id == TRUE_ID {
        return Ok(n_vars);
    }
    if node.is_terminal() {
        return Err(format!("node id {} is a terminal record but not node 0 or 1", id));
    }
    let v = node.var as usize;
    if v >= n_vars {
        return Err(format!(
            "node id {} uses variable {} but only {} variables were declared",
            id, v, n_vars
        ));
    }
    Ok(v)
}

/// `free_prefix[l]` = number of unfixed variables among levels `0..l`, for `l` in `0..=n`.
fn free_prefix(n_vars: usize, fixed: Option<&[Option<bool>]>) -> Result<Vec<u32>, String> {
    if let Some(fx) = fixed {
        if fx.len() != n_vars {
            return Err(format!(
                "fixed-variable table has {} entries for {} variables",
                fx.len(),
                n_vars
            ));
        }
    }
    let mut prefix = vec![0u32; n_vars + 1];
    for l in 0..n_vars {
        let is_free = fixed.map(|fx| fx[l].is_none()).unwrap_or(true);
        prefix[l + 1] = prefix[l] + is_free as u32;
    }
    Ok(prefix)
}

/// Exact number of assignments to the `n_vars` variables that satisfy the function rooted at
/// `root` and agree with `fixed` (when given, `fixed.len()` must equal `n_vars`).
pub fn count_models(
    nodes: &[ROBDDNode],
    root: u32,
    n_vars: u16,
    fixed: Option<&[Option<bool>]>,
) -> Result<PathCount, String> {
    let n = n_vars as usize;
    let prefix = free_prefix(n, fixed)?;
    let root_level = level_of(nodes, root, n)?;
    if root == FALSE_ID {
        return Ok(PathCount::zero());
    }

    let mut memo: FxHashMap<u32, PathCount> = fx_map_with_capacity(64);
    let mut stack: Vec<(u32, bool)> = vec![(root, false)];

    while let Some((id, children_done)) = stack.pop() {
        if id == FALSE_ID || id == TRUE_ID || memo.contains_key(&id) {
            continue;
        }
        let node = nodes[id as usize];
        let v = node.var as usize;
        if !children_done {
            stack.push((id, true));
            for child in [node.hi, node.lo] {
                let child_level = level_of(nodes, child, n)?;
                if child_level <= v {
                    return Err(format!(
                        "order violation: node {} (var {}) has child {} at level {}",
                        id, v, child, child_level
                    ));
                }
                if child != FALSE_ID && child != TRUE_ID && !memo.contains_key(&child) {
                    stack.push((child, false));
                }
            }
            continue;
        }

        let mut total = PathCount::zero();
        for (child, value) in [(node.lo, false), (node.hi, true)] {
            if let Some(fx) = fixed {
                if let Some(required) = fx[v] {
                    if required != value {
                        continue;
                    }
                }
            }
            let child_count = match child {
                FALSE_ID => continue,
                TRUE_ID => PathCount::one(),
                _ => match memo.get(&child) {
                    Some(c) => c.clone(),
                    None => return Err(format!("child {} of node {} was not counted", child, id)),
                },
            };
            if child_count.is_zero() {
                continue;
            }
            let child_level = level_of(nodes, child, n)?;
            let gap = prefix[child_level] - prefix[v + 1];
            total.add_assign(&child_count.shifted(gap));
        }
        memo.insert(id, total);
    }

    let root_count = if root == TRUE_ID {
        PathCount::one()
    } else {
        memo.get(&root)
            .cloned()
            .ok_or_else(|| format!("root {} was not counted", root))?
    };
    Ok(root_count.shifted(prefix[root_level]))
}

/// One assignment (`result[v]` = value of variable `v`) that satisfies the function rooted
/// at `root` and agrees with `fixed`, or `None` if there is none. Variables the function does
/// not constrain take their fixed value, or `false`.
pub fn find_model(
    nodes: &[ROBDDNode],
    root: u32,
    n_vars: u16,
    fixed: Option<&[Option<bool>]>,
) -> Result<Option<Vec<bool>>, String> {
    let n = n_vars as usize;
    // Validates the array and `fixed` exactly as counting does.
    free_prefix(n, fixed)?;
    level_of(nodes, root, n)?;
    if root == FALSE_ID {
        return Ok(None);
    }

    let allowed = |v: usize, value: bool| -> bool {
        fixed
            .and_then(|fx| fx[v])
            .map(|required| required == value)
            .unwrap_or(true)
    };

    // One bottom-up pass: satisfiable[n] = some allowed branch reaches TRUE.
    let mut satisfiable: FxHashMap<u32, bool> = fx_map_with_capacity(64);
    let mut stack: Vec<(u32, bool)> = vec![(root, false)];
    while let Some((id, children_done)) = stack.pop() {
        if id == FALSE_ID || id == TRUE_ID || satisfiable.contains_key(&id) {
            continue;
        }
        let node = nodes[id as usize];
        let v = node.var as usize;
        if !children_done {
            stack.push((id, true));
            for child in [node.hi, node.lo] {
                let child_level = level_of(nodes, child, n)?;
                if child_level <= v {
                    return Err(format!(
                        "order violation: node {} (var {}) has child {} at level {}",
                        id, v, child, child_level
                    ));
                }
                if child != FALSE_ID && child != TRUE_ID && !satisfiable.contains_key(&child) {
                    stack.push((child, false));
                }
            }
            continue;
        }
        let mut sat = false;
        for (child, value) in [(node.lo, false), (node.hi, true)] {
            if !allowed(v, value) {
                continue;
            }
            let child_sat = match child {
                FALSE_ID => false,
                TRUE_ID => true,
                _ => satisfiable.get(&child).copied().unwrap_or(false),
            };
            if child_sat {
                sat = true;
                break;
            }
        }
        satisfiable.insert(id, sat);
    }

    let root_sat = root == TRUE_ID || satisfiable.get(&root).copied().unwrap_or(false);
    if !root_sat {
        return Ok(None);
    }

    let mut assignment: Vec<bool> = (0..n)
        .map(|l| fixed.and_then(|fx| fx[l]).unwrap_or(false))
        .collect();
    let mut current = root;
    while current != TRUE_ID {
        let node = nodes[current as usize];
        let v = node.var as usize;
        let mut next: Option<(u32, bool)> = None;
        for (child, value) in [(node.hi, true), (node.lo, false)] {
            if !allowed(v, value) {
                continue;
            }
            let child_sat = match child {
                FALSE_ID => false,
                TRUE_ID => true,
                _ => satisfiable.get(&child).copied().unwrap_or(false),
            };
            if child_sat {
                next = Some((child, value));
                break;
            }
        }
        match next {
            Some((child, value)) => {
                assignment[v] = value;
                current = child;
            }
            None => {
                return Err(format!(
                    "node {} has no satisfiable branch although it was marked satisfiable",
                    current
                ))
            }
        }
    }
    Ok(Some(assignment))
}

/// The value of the function rooted at `root` under `assignment` (`assignment[v]` = value
/// of variable `v`). Validates every node on the evaluated path (ids in range, strictly
/// increasing variable order), so it is safe on node arrays read back from disk.
pub fn evaluate(nodes: &[ROBDDNode], root: u32, assignment: &[bool]) -> Result<bool, String> {
    let mut current = root;
    let mut last_var: Option<u16> = None;
    // A path visits each variable at most once, so it has at most `nodes.len()` steps.
    for _ in 0..=nodes.len() {
        match current {
            FALSE_ID => return Ok(false),
            TRUE_ID => return Ok(true),
            _ => {}
        }
        let node = nodes
            .get(current as usize)
            .ok_or_else(|| format!("node id {} out of range ({} nodes)", current, nodes.len()))?;
        if node.is_terminal() {
            return Err(format!(
                "node id {} is a terminal record but not node 0 or 1",
                current
            ));
        }
        if let Some(prev) = last_var {
            if node.var <= prev {
                return Err(format!(
                    "order violation: node {} (var {}) follows var {}",
                    current, node.var, prev
                ));
            }
        }
        let value = *assignment.get(node.var as usize).ok_or_else(|| {
            format!(
                "node {} tests variable {} but the assignment covers {} variables",
                current,
                node.var,
                assignment.len()
            )
        })?;
        last_var = Some(node.var);
        current = if value { node.hi } else { node.lo };
    }
    Err(format!("evaluation from root {} did not reach a terminal", root))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::psa::bdd::unique_table::UniqueTable;

    /// Build: x0 (var=0) with lo=FALSE, hi=TRUE → single-variable BDD.
    fn single_var_bdd() -> (Vec<ROBDDNode>, u32) {
        let mut nodes = vec![ROBDDNode::false_terminal(), ROBDDNode::true_terminal()];
        let mut ut = UniqueTable::new();
        let id = ut.make_node(0, FALSE_ID, TRUE_ID, &mut nodes);
        (nodes, id)
    }

    #[test]
    fn sat_false_is_zero() {
        let nodes = vec![ROBDDNode::false_terminal(), ROBDDNode::true_terminal()];
        let result = count_models(&nodes, FALSE_ID, 4, None).unwrap();
        assert!(result.is_zero());
    }

    #[test]
    fn sat_true_with_depth_is_2_pow_depth() {
        let nodes = vec![ROBDDNode::false_terminal(), ROBDDNode::true_terminal()];
        // 3 free variables → 2^3 = 8 assignments
        let result = count_models(&nodes, TRUE_ID, 3, None).unwrap();
        assert_eq!(result.to_u64(), Some(8));
    }

    #[test]
    fn sat_single_variable_bdd_is_one() {
        // x0: lo=FALSE, hi=TRUE → one satisfying assignment (x0=1).
        let (nodes, root) = single_var_bdd();
        let result = count_models(&nodes, root, 1, None).unwrap();
        assert_eq!(result.to_u64(), Some(1), "x0=1 is the only sat assignment");
    }

    #[test]
    fn counting_is_repeatable() {
        let (nodes, root) = single_var_bdd();
        let first = count_models(&nodes, root, 1, None).unwrap();
        let second = count_models(&nodes, root, 1, None).unwrap();
        assert_eq!(first, second);
    }
}
