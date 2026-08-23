//! Shared Diagram Utilities and Package Tree Abstractions (§10.4).
//!
//! Eliminates repetitive symbol resolution, identifier sanitization, visibility formatting,
//! and hierarchical package graph construction across all diagram format exporters.

use std::collections::{HashMap, HashSet};

use crate::ingestion::TokenCorpusArtifact;
use crate::symbol::SymbolTableArtifact;

/// Common utilities for diagram symbol resolution and identifier formatting.
pub struct DiagramUtils;

impl DiagramUtils {
    /// Resolves the raw name string for a symbol ID from the string interner.
    pub fn resolve_name<'a>(
        sta: &SymbolTableArtifact,
        tca: &'a TokenCorpusArtifact,
        sym_id: u32,
    ) -> &'a str {
        if sym_id == u32::MAX || sym_id == u32::MAX - 1 {
            return "Actor";
        }
        if let Some(sym) = sta.symbol(sym_id) {
            let bytes = tca.interner.lookup_text(sym.name_id);
            std::str::from_utf8(bytes).unwrap_or("")
        } else {
            ""
        }
    }

    /// Sanitizes an identifier for safe PlantUML and Mermaid notation.
    pub fn sanitize(name: &str) -> String {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return String::new();
        }
        let clean: String = trimmed
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();

        if clean.is_empty() {
            String::new()
        } else if clean.chars().next().unwrap().is_numeric() {
            format!("_{}", clean)
        } else {
            clean
        }
    }

    /// Checks if a type name is a language primitive, system type, or synthetic node.
    pub fn is_primitive_or_system(name: &str) -> bool {
        matches!(
            name,
            "int"
                | "long"
                | "short"
                | "byte"
                | "float"
                | "double"
                | "boolean"
                | "char"
                | "void"
                | "String"
                | "Object"
                | "List"
                | "Map"
                | "Set"
                | "Collection"
                | "Optional"
                | "Integer"
                | "Long"
                | "Boolean"
                | "Double"
                | "Float"
                | "Byte"
                | "Short"
                | "Character"
                | "true"
                | "false"
                | "null"
                | ""
        )
    }

    /// Returns the standard UML visibility marker for a visibility code.
    pub fn visibility_symbol(vis: u8) -> &'static str {
        match vis {
            1 => "+",
            2 => "-",
            3 => "#",
            _ => "~",
        }
    }

    /// Resolves the fully qualified package name for a symbol.
    pub fn resolve_sym_package(
        sta: &SymbolTableArtifact,
        _tca: &TokenCorpusArtifact,
        sym_id: u32,
        class_package_by_sym: &HashMap<u32, String>,
        package_path_by_sym: &HashMap<u32, String>,
    ) -> Option<String> {
        if let Some(pkg) = class_package_by_sym.get(&sym_id) {
            return Some(pkg.clone());
        }
        if let Some(sym) = sta.symbol(sym_id) {
            let mut curr_scope = sym.scope_id;
            while curr_scope != u32::MAX && (curr_scope as usize) < sta.scope_records.len() {
                let scope_rec = &sta.scope_records[curr_scope as usize];
                if scope_rec.scope_kind == 1 {
                    // Package scope
                    if let Some(pkg_sym_id) = sta.symbol_records.iter().position(|s| {
                        s.kind == crate::core::types::symbol::SymbolKind::SK_PACKAGE as u8
                            && s.scope_id == curr_scope
                    }) {
                        if let Some(path) = package_path_by_sym.get(&(pkg_sym_id as u32)) {
                            return Some(path.clone());
                        }
                    }
                }
                curr_scope = scope_rec.parent_scope;
            }
        }
        None
    }

    /// Resolves a container package ID, disambiguating duplicates.
    pub fn resolve_container_pkg_id(pkg_name: &str, duplicate_names: &HashSet<String>) -> String {
        let parts: Vec<&str> = pkg_name.split('.').filter(|s| !s.is_empty()).collect();
        if parts.is_empty() {
            "root_pkg".to_string()
        } else {
            let leaf = parts.last().unwrap();
            let safe_leaf = Self::sanitize(leaf);
            if duplicate_names.contains(&safe_leaf) {
                Self::sanitize(pkg_name)
            } else {
                safe_leaf
            }
        }
    }
}
