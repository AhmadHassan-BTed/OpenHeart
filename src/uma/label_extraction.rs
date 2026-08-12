//! LabelExtractor — extracts human-readable text labels from BP AST statement sub-trees (§9.2.6).

use crate::ast::BPASTArtifact;
use crate::ingestion::TokenCorpusArtifact;
use crate::symbol::SymbolTableArtifact;

pub struct LabelExtractor;

impl LabelExtractor {
    /// Extract a formatted action/condition label from an AST node and intern it.
    pub fn extract_label(
        stmt_node: u32,
        bpa: &BPASTArtifact,
        tca: &TokenCorpusArtifact,
        _sta: &SymbolTableArtifact,
        max_chars: usize,
    ) -> String {
        if stmt_node == u32::MAX || stmt_node >= bpa.node_count {
            return "action".to_string();
        }

        let _node_type = bpa.node_type(stmt_node);
        let token_idx = bpa.node_attr(stmt_node);

        let label_text = if let Some(record) = tca.token_records.get(token_idx as usize) {
            let bytes = tca.interner.lookup_text(record.text_id);
            let text = std::str::from_utf8(bytes).unwrap_or("action");
            format!("{}()", text)
        } else {
            "action".to_string()
        };

        if label_text.len() > max_chars {
            format!("{}...", &label_text[..max_chars.saturating_sub(3)])
        } else {
            label_text
        }
    }
}
