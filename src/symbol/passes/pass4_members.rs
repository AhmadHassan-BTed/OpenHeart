//! Pass 4 — Member Declaration Type Resolution
//! Populates `type_id` fields on fields, parameters, local variables, and method return types from Pass 3 resolutions.

use crate::ast::BPASTArtifact;
use crate::core::types::ast::ASTNodeType;
use crate::core::types::symbol::SymbolKind;
use crate::symbol::builder::SymbolTableBuilder;

pub struct Pass4Members;

impl Pass4Members {
    pub fn run(bpa: &BPASTArtifact, builder: &mut SymbolTableBuilder) {
        for sym_id in 0..builder.symbol_count() as u32 {
            let (kind, decl_node) = match builder.symbol(sym_id) {
                Some(s) => (s.kind, s.decl_node),
                None => continue,
            };

            if decl_node == u32::MAX {
                continue;
            }

            if kind == SymbolKind::SK_FIELD as u8
                || kind == SymbolKind::SK_PARAM as u8
                || kind == SymbolKind::SK_LOCAL_VAR as u8
                || kind == SymbolKind::SK_METHOD as u8
            {
                if let Some(type_ref_node) = Self::find_type_ref_child(decl_node, bpa) {
                    if let Some(type_sym_id) = builder.get_type_ref_resolution(type_ref_node) {
                        builder.set_type_id(sym_id, type_sym_id);
                    }
                }
            }
        }
    }

    fn find_type_ref_child(decl_node: u32, bpa: &BPASTArtifact) -> Option<u32> {
        let mut cur = bpa.first_child(decl_node);
        while let Some(c) = cur {
            if bpa.node_type(c) == ASTNodeType::NN_TYPE_REF {
                return Some(c);
            }
            cur = bpa.next_sibling(c);
        }
        None
    }
}
