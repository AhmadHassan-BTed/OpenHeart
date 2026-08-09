//! NameResolver: implements scope graph resolution over parent & import edges.

use crate::symbol::scope_graph::ScopeGraph;

pub struct NameResolver;

impl NameResolver {
    /// Resolves `name` starting at `scope_id`, traversing parent scopes E_p* innermost first.
    pub fn resolve_lexical<F>(
        scope_graph: &ScopeGraph,
        name: &str,
        scope_id: u32,
        lookup_name: F,
    ) -> Option<u32>
    where
        F: Fn(u32, &str) -> bool,
    {
        let mut cur_scope = scope_id;
        while cur_scope != u32::MAX {
            for &sym_id in scope_graph.declarations(cur_scope) {
                if lookup_name(sym_id, name) {
                    return Some(sym_id);
                }
            }
            cur_scope = scope_graph
                .parent_edges
                .get(&cur_scope)
                .copied()
                .unwrap_or(u32::MAX);
        }
        None
    }

    /// Checks explicit import mappings (e.g. `import java.util.List` -> `"List" -> "java.util.List"`).
    pub fn resolve_via_import_map(
        scope_graph: &ScopeGraph,
        name: &str,
        scope_id: u32,
    ) -> Option<String> {
        let mut cur_scope = scope_id;
        while cur_scope != u32::MAX {
            if let Some(map) = scope_graph.import_mappings.get(&cur_scope) {
                if let Some(qual_name) = map.get(name) {
                    return Some(qual_name.clone());
                }
            }
            cur_scope = scope_graph
                .parent_edges
                .get(&cur_scope)
                .copied()
                .unwrap_or(u32::MAX);
        }
        None
    }

    /// Gets wildcard package import targets for a scope.
    pub fn wildcard_imports(scope_graph: &ScopeGraph, scope_id: u32) -> Vec<String> {
        let mut pkgs = Vec::new();
        let mut cur_scope = scope_id;
        while cur_scope != u32::MAX {
            if let Some(imports) = scope_graph.import_edges.get(&cur_scope) {
                for imp in imports {
                    pkgs.push(imp.clone());
                }
            }
            cur_scope = scope_graph
                .parent_edges
                .get(&cur_scope)
                .copied()
                .unwrap_or(u32::MAX);
        }
        pkgs
    }
}
