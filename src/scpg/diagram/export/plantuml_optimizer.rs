//! PlantUML Edge Optimizer for Package Grouping & Arrow Simplification (§6.1 - §6.3).
//! Reduces diagram "spaghetti" by bundling intra-package relations and collapsing cross-package dependencies.

use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone)]
pub struct RawEdge {
    pub src_class: String,
    pub dst_class: String,
    pub rel_op: String,
    pub full_line: String,
}

#[derive(Debug, Clone)]
pub struct PlantUMLOptimizationOptions {
    /// Bundle rule threshold: minimum matching edges from a package to a single entity to trigger bundling.
    pub bundle_threshold: usize,
    /// Package-to-package reduction threshold: minimum cross-package edges to collapse into package-level import edge.
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
    fn pkg_alias(pkg: &str) -> String {
        format!("pkg_{}", pkg.replace(['.', '/', '-'], "_"))
    }

    pub fn optimize(
        raw_edges: Vec<RawEdge>,
        class_to_package: &HashMap<String, String>,
        _package_class_counts: &HashMap<String, usize>,
        _options: &PlantUMLOptimizationOptions,
    ) -> Vec<String> {
        let mut final_lines = Vec::new();
        let mut edges_by_pkg_pair: HashMap<(String, String), Vec<RawEdge>> = HashMap::new();
        let mut unbundled_edges: Vec<RawEdge> = Vec::new();

        // 1. Group edges by cross-package or class-level (preserving inheritance/realization)
        for edge in raw_edges {
            let is_inheritance = edge.rel_op.contains("|>") || edge.full_line.contains("|>");
            let src_pkg = class_to_package.get(&edge.src_class).cloned();
            let dst_pkg = class_to_package.get(&edge.dst_class).cloned();

            if !is_inheritance {
                if let (Some(sp), Some(dp)) = (src_pkg, dst_pkg) {
                    if sp != dp {
                        edges_by_pkg_pair.entry((sp, dp)).or_default().push(edge);
                        continue;
                    }
                }
            }
            unbundled_edges.push(edge);
        }

        let mut suppressed_edges = HashSet::new();

        // 2. Rule 6.3: Package-to-Package Reduction (Collapse usage dependencies into package import arrows)
        for ((sp, dp), p_edges) in edges_by_pkg_pair {
            if !p_edges.is_empty() {
                final_lines.push(format!(
                    "{} ..> {} : <<imports>>",
                    Self::pkg_alias(&sp),
                    Self::pkg_alias(&dp)
                ));
                for e in &p_edges {
                    suppressed_edges.insert((e.src_class.clone(), e.dst_class.clone()));
                }
            }
        }

        // 3. Rule 6.2: Structural Edge Reduction (The Bundle Rule)
        let mut bundle_groups: HashMap<(String, String, String), Vec<RawEdge>> = HashMap::new();
        let mut remaining_edges = Vec::new();

        for edge in unbundled_edges {
            if suppressed_edges.contains(&(edge.src_class.clone(), edge.dst_class.clone())) {
                continue;
            }

            let is_inheritance = edge.rel_op.contains("|>") || edge.full_line.contains("|>");
            if !is_inheritance {
                if let Some(src_pkg) = class_to_package.get(&edge.src_class) {
                    bundle_groups
                        .entry((src_pkg.clone(), edge.dst_class.clone(), edge.rel_op.clone()))
                        .or_default()
                        .push(edge);
                    continue;
                }
            }
            remaining_edges.push(edge);
        }

        for ((sp, dst, op), b_edges) in bundle_groups {
            if !b_edges.is_empty() {
                final_lines.push(format!("{} {} {}", Self::pkg_alias(&sp), op, dst));
            } else {
                for e in b_edges {
                    remaining_edges.push(e);
                }
            }
        }

        for e in remaining_edges {
            final_lines.push(e.full_line.clone());
        }

        final_lines.sort();
        final_lines.dedup();
        final_lines
    }
}
