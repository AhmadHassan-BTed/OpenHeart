//! Pass 2 — Import Resolution
//! Scans import declarations under file root nodes and registers simple, wildcard, and static import mappings.

use crate::ast::BPASTArtifact;
use crate::core::types::ast::ASTNodeType;
use crate::ingestion::TokenCorpusArtifact;
use crate::symbol::builder::SymbolTableBuilder;

pub struct Pass2Imports;

impl Pass2Imports {
    pub fn run(bpa: &BPASTArtifact, tca: &TokenCorpusArtifact, builder: &mut SymbolTableBuilder) {
        for pre_idx in 0..bpa.node_count {
            if bpa.node_type(pre_idx) == ASTNodeType::NN_MODULE {
                let file_scope = builder
                    .symbol_at_node(pre_idx)
                    .map(|sym_id| builder.symbols[sym_id as usize].scope_id)
                    .unwrap_or(0);

                let mut child = bpa.first_child(pre_idx);
                while let Some(c) = child {
                    if bpa.node_type(c) == ASTNodeType::NN_EXPR_STMT
                        || bpa.node_type(c) == ASTNodeType::NN_TYPE_REF
                    {
                        let (ft, lt) = bpa.token_range(c);
                        if ft != u32::MAX && ft <= lt {
                            let text = Self::extract_range_text(ft, lt, tca);
                            if text.starts_with("import ") {
                                let import_text = text
                                    .trim_start_matches("import ")
                                    .trim_end_matches(';')
                                    .trim();
                                if let Some(pkg) = import_text.strip_suffix(".*") {
                                    builder.scope_graph.add_import_edge(file_scope, pkg);
                                } else {
                                    let simple_name =
                                        import_text.rsplit('.').next().unwrap_or(import_text);
                                    builder.scope_graph.add_import_mapping(
                                        file_scope,
                                        simple_name,
                                        import_text,
                                    );
                                }
                            }
                        }
                    }
                    child = bpa.next_sibling(c);
                }
            }
        }
    }

    fn extract_range_text(first_tok: u32, last_tok: u32, tca: &TokenCorpusArtifact) -> String {
        let mut parts = Vec::new();
        let max_idx = last_tok.min(tca.token_records.len() as u32 - 1);
        for tok_idx in first_tok..=max_idx {
            let text_id = tca.token_records[tok_idx as usize].text_id;
            let bytes = tca.interner.lookup_text(text_id);
            if let Ok(t) = std::str::from_utf8(bytes) {
                parts.push(t);
            }
        }
        parts.join(" ")
    }
}
