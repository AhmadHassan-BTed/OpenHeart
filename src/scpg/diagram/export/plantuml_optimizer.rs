//! PlantUML Edge Optimizer for Package Grouping & Arrow Simplification (§6.1 - §6.3).
//! Reduces diagram "spaghetti" by bundling intra-package relations and collapsing
//! cross-package dependencies, without ever hiding a structural relationship.
//!
//! Rules:
//! - Generalizations and realizations (`|>` arrows) are always kept verbatim.
//! - Rule 6.3 (package reduction): the non-structural edges between two packages are
//!   collapsed into one `<<imports>>` edge only when there are at least
//!   `pkg_dependency_threshold` of them. Only those collapsed edges are suppressed; another
//!   relationship between the same class pair is unaffected.
//! - Rule 6.2 (bundling): edges with the same operator from at least `bundle_threshold`
//!   distinct classes of one package to the same target are drawn once from the package.
//!
//! `class_to_package` maps a class alias to the alias of its package node, so the emitted
//! package lines reference the same identifiers as the diagram's package declarations.

use std::collections::{BTreeMap, BTreeSet, HashMap};

#[derive(Debug, Clone)]
pub struct RawEdge {
    pub src_class: String,
    pub dst_class: String,
    pub rel_op: String,
    pub full_line: String,
}

impl RawEdge {
    fn is_structural(&self) -> bool {
        self.rel_op.contains("|>")
    }
}

#[derive(Debug, Clone)]
pub struct PlantUMLOptimizationOptions {
    /// Bundle rule threshold: minimum number of distinct classes of one package sharing an
    /// edge (same operator, same target) before the edges are drawn once from the package.
    pub bundle_threshold: usize,
    /// Package-to-package reduction threshold: minimum cross-package edges to collapse into
    /// one package-level import edge.
    pub pkg_dependency_threshold: usize,
}

impl Default for PlantUMLOptimizationOptions {
    fn default() -> Self {
        Self {
            bundle_threshold: 2,
            pkg_dependency_threshold: 3,
        }
    }
}

pub struct PlantUMLOptimizer;

impl PlantUMLOptimizer {
    /// Returns the optimized relationship lines, sorted and deduplicated.
    ///
    /// `package_class_counts` (package alias → number of classes) restricts the bundle rule to
    /// packages that hold more than one class.
    pub fn optimize(
        raw_edges: Vec<RawEdge>,
        class_to_package: &HashMap<String, String>,
        package_class_counts: &HashMap<String, usize>,
        options: &PlantUMLOptimizationOptions,
    ) -> Vec<String> {
        let mut final_lines: BTreeSet<String> = BTreeSet::new();
        let mut cross_package: BTreeMap<(String, String), Vec<usize>> = BTreeMap::new();
        let mut collapsed = vec![false; raw_edges.len()];

        // Rule 6.3: group non-structural cross-package edges by package pair.
        for (idx, edge) in raw_edges.iter().enumerate() {
            if edge.is_structural() {
                continue;
            }
            if let (Some(sp), Some(dp)) = (
                class_to_package.get(&edge.src_class),
                class_to_package.get(&edge.dst_class),
            ) {
                if sp != dp {
                    cross_package
                        .entry((sp.clone(), dp.clone()))
                        .or_default()
                        .push(idx);
                }
            }
        }
        for ((sp, dp), idxs) in &cross_package {
            if idxs.len() >= options.pkg_dependency_threshold.max(1) {
                final_lines.insert(format!("{} ..> {} : <<imports>> ({})", sp, dp, idxs.len()));
                for &i in idxs {
                    collapsed[i] = true;
                }
            }
        }

        // Rule 6.2: bundle identical edges from several classes of one package.
        let mut bundles: BTreeMap<(String, String, String), BTreeSet<usize>> = BTreeMap::new();
        for (idx, edge) in raw_edges.iter().enumerate() {
            if collapsed[idx] || edge.is_structural() {
                continue;
            }
            if let Some(sp) = class_to_package.get(&edge.src_class) {
                bundles
                    .entry((sp.clone(), edge.dst_class.clone(), edge.rel_op.clone()))
                    .or_default()
                    .insert(idx);
            }
        }
        for ((sp, dst, op), idxs) in &bundles {
            let distinct_sources: BTreeSet<&str> = idxs
                .iter()
                .map(|&i| raw_edges[i].src_class.as_str())
                .collect();
            let package_size = package_class_counts.get(sp).copied().unwrap_or(0);
            if distinct_sources.len() >= options.bundle_threshold.max(2) && package_size > 1 {
                final_lines.insert(format!(
                    "{} {} {} : <<{} classes>>",
                    sp,
                    op,
                    dst,
                    distinct_sources.len()
                ));
                for &i in idxs {
                    collapsed[i] = true;
                }
            }
        }

        for (idx, edge) in raw_edges.iter().enumerate() {
            if !collapsed[idx] {
                final_lines.insert(edge.full_line.clone());
            }
        }
        final_lines.into_iter().collect()
    }
}
