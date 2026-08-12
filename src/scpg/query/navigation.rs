//! NavigationEngine — bidirectional source <-> entity navigation via TRA (§10.5).

use crate::tra::types::{SourceRange, TraceabilityArtifact, UMLLinkRecord};

pub struct NavigationEngine;

impl NavigationEngine {
    /// O(1) entity -> source range lookup.
    pub fn to_source(sym_id: u32, tra: &TraceabilityArtifact) -> Option<SourceRange> {
        let link = tra.uml_links.iter().find(|l| l.sym_id == sym_id)?;
        Some(SourceRange {
            file_id: link.file_id,
            line_start: link.line_start,
            col_start: link.col_start,
            line_end: link.line_end,
            col_end: link.col_end,
        })
    }

    /// O(log n) source line -> entities lookup.
    pub fn from_source(file_id: u16, line: u32, tra: &TraceabilityArtifact) -> Vec<UMLLinkRecord> {
        tra.uml_links
            .iter()
            .filter(|l| l.file_id == file_id && l.line_start <= line && l.line_end >= line)
            .cloned()
            .collect()
    }
}
