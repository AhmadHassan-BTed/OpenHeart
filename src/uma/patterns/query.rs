//! Shared Pattern Query & AST Inspection Engine (§9.2.5).
//!
//! Eliminates boilerplate across pattern detectors by providing declarative AST predicates
//! for hierarchy matching, field scanning, constructor visibility, and delegation checks.

use crate::core::types::cg::CallGraphArtifact;
use crate::core::types::symbol::{SymbolKind, SymbolModifiers, SymbolRecord};
use crate::ingestion::TokenCorpusArtifact;
use crate::symbol::SymbolTableArtifact;

/// High-level query inspector for GoF pattern detection.
pub struct PatternInspector;

impl PatternInspector {
    /// Resolves the string name of a symbol.
    pub fn get_name<'a>(
        sta: &SymbolTableArtifact,
        tca: &'a TokenCorpusArtifact,
        sym_id: u32,
    ) -> &'a str {
        if let Some(sym) = sta.symbol(sym_id) {
            let bytes = tca.interner.lookup_text(sym.name_id);
            std::str::from_utf8(bytes).unwrap_or("")
        } else {
            ""
        }
    }

    /// Checks if a symbol's name contains or ends with any of the candidate strings.
    pub fn name_matches(
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
        sym_id: u32,
        candidates: &[&str],
    ) -> bool {
        let name = Self::get_name(sta, tca, sym_id);
        candidates.iter().any(|&c| name.contains(c))
    }

    /// Checks if a symbol has any outbound Type Hierarchy edges (implements or extends).
    pub fn has_type_hierarchy_edge(sta: &SymbolTableArtifact, sym_id: u32) -> bool {
        sta.th_edges.iter().any(|e| e.from_sym == sym_id)
    }

    /// Returns all child field symbols of a class.
    pub fn get_fields<'a>(sta: &'a SymbolTableArtifact, sym_id: u32) -> Vec<&'a SymbolRecord> {
        let mut fields = Vec::new();
        if let Some(sym) = sta.symbol(sym_id) {
            let mut child_id = sym.first_child;
            while child_id != u32::MAX && (child_id as usize) < sta.symbol_records.len() {
                let child = &sta.symbol_records[child_id as usize];
                if child.kind == SymbolKind::SK_FIELD as u8 {
                    fields.push(child);
                }
                child_id = child.next_sibling;
            }
        }
        fields
    }

    /// Returns all child method symbols of a class.
    pub fn get_methods<'a>(sta: &'a SymbolTableArtifact, sym_id: u32) -> Vec<&'a SymbolRecord> {
        let mut methods = Vec::new();
        if let Some(sym) = sta.symbol(sym_id) {
            let mut child_id = sym.first_child;
            while child_id != u32::MAX && (child_id as usize) < sta.symbol_records.len() {
                let child = &sta.symbol_records[child_id as usize];
                if child.kind == SymbolKind::SK_METHOD as u8 {
                    methods.push(child);
                }
                child_id = child.next_sibling;
            }
        }
        methods
    }

    /// Checks if a class has a private constructor.
    pub fn has_private_constructor(
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
        sym_id: u32,
    ) -> bool {
        let class_name = Self::get_name(sta, tca, sym_id);
        for m in Self::get_methods(sta, sym_id) {
            let m_name = Self::get_name(sta, tca, m.symbol_id);
            if (m_name == class_name || m_name == "<init>") && m.visibility == 2 {
                return true;
            }
        }
        false
    }

    /// Checks if a class has a static field referencing an instance of itself (Singleton signature).
    pub fn has_static_self_reference(sta: &SymbolTableArtifact, sym_id: u32) -> bool {
        for f in Self::get_fields(sta, sym_id) {
            if (f.modifiers & SymbolModifiers::STATIC) != 0 && f.type_id == sym_id {
                return true;
            }
        }
        false
    }

    /// Checks if a class contains a collection or array field of a given component type (Composite signature).
    pub fn has_collection_field_of(
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
        sym_id: u32,
    ) -> bool {
        for f in Self::get_fields(sta, sym_id) {
            let f_name = Self::get_name(sta, tca, f.symbol_id);
            let type_name = Self::get_name(sta, tca, f.type_id);
            if f_name.ends_with("s")
                || type_name.contains("List")
                || type_name.contains("Set")
                || type_name.contains("Collection")
            {
                return true;
            }
        }
        false
    }

    /// Checks if methods in a class delegate calls to another class (Decorator/Facade/Strategy signature).
    pub fn has_interprocedural_delegation(
        cga: &CallGraphArtifact,
        caller_methods: &[u32],
        callee_sym: u32,
    ) -> bool {
        cga.site_to_edge_map
            .iter()
            .any(|(caller, callee, _)| caller_methods.contains(caller) && *callee == callee_sym)
    }
}
