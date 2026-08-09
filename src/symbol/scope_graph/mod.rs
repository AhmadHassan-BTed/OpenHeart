pub mod resolver;

pub use resolver::*;

use crate::core::types::symbol::{ScopeKind, ScopeRecord};
use std::collections::HashMap;

pub const ROOT_SCOPE: u32 = 0;

#[derive(Debug, Clone)]
pub struct ScopeGraph {
    pub scopes: Vec<ScopeRecord>,
    pub scope_declarations: HashMap<u32, Vec<u32>>,
    pub parent_edges: HashMap<u32, u32>,
    pub import_edges: HashMap<u32, Vec<String>>,
    pub import_mappings: HashMap<u32, HashMap<String, String>>,
}

impl Default for ScopeGraph {
    fn default() -> Self {
        Self::new()
    }
}

impl ScopeGraph {
    pub fn new() -> Self {
        let mut sg = Self {
            scopes: Vec::new(),
            scope_declarations: HashMap::new(),
            parent_edges: HashMap::new(),
            import_edges: HashMap::new(),
            import_mappings: HashMap::new(),
        };

        // Create root file scope 0
        sg.create_scope(u32::MAX, u32::MAX, ScopeKind::File);
        sg
    }

    pub fn create_scope(&mut self, owner_symbol: u32, parent_scope: u32, kind: ScopeKind) -> u32 {
        let scope_id = self.scopes.len() as u32;

        let record = ScopeRecord {
            scope_id,
            parent_scope,
            owner_symbol,
            first_decl: u32::MAX,
            decl_count: 0,
            import_count: 0,
            scope_kind: kind as u8,
            flags: 0,
            import_table_off: 0,
            _reserved: 0,
        };

        self.scopes.push(record);
        if parent_scope != u32::MAX {
            self.parent_edges.insert(scope_id, parent_scope);
        }

        self.scope_declarations.insert(scope_id, Vec::new());
        self.import_edges.insert(scope_id, Vec::new());
        self.import_mappings.insert(scope_id, HashMap::new());

        scope_id
    }

    pub fn add_declaration(&mut self, scope_id: u32, symbol_id: u32) {
        if let Some(scope) = self.scopes.get_mut(scope_id as usize) {
            if scope.first_decl == u32::MAX {
                scope.first_decl = symbol_id;
            }
            scope.decl_count += 1;
        }

        if let Some(decls) = self.scope_declarations.get_mut(&scope_id) {
            decls.push(symbol_id);
        }
    }

    pub fn add_import_edge(&mut self, scope_id: u32, wildcard_pkg: &str) {
        if let Some(edges) = self.import_edges.get_mut(&scope_id) {
            edges.push(wildcard_pkg.to_string());
        }
        if let Some(scope) = self.scopes.get_mut(scope_id as usize) {
            scope.import_count += 1;
            scope.flags |= 1; // has_wildcard_import
        }
    }

    pub fn add_import_mapping(&mut self, scope_id: u32, simple_name: &str, qual_name: &str) {
        if let Some(map) = self.import_mappings.get_mut(&scope_id) {
            map.insert(simple_name.to_string(), qual_name.to_string());
        }
        if let Some(scope) = self.scopes.get_mut(scope_id as usize) {
            scope.import_count += 1;
        }
    }

    pub fn declarations(&self, scope_id: u32) -> &[u32] {
        self.scope_declarations
            .get(&scope_id)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    pub fn get_scope(&self, scope_id: u32) -> Option<&ScopeRecord> {
        self.scopes.get(scope_id as usize)
    }

    pub fn scope_count(&self) -> usize {
        self.scopes.len()
    }
}
