//! Pass 4 — Member Declaration Type Resolution
//! Populates `type_id` fields on fields, parameters, local variables, and method return types from Pass 3 resolutions and token sweeps.

use std::collections::HashMap;

use crate::ast::BPASTArtifact;
use crate::core::types::ast::ASTNodeType;
use crate::core::types::symbol::SymbolKind;
use crate::core::types::token::unpack_sort_key;
use crate::ingestion::TokenCorpusArtifact;
use crate::symbol::builder::SymbolTableBuilder;

pub struct Pass4Members;

impl Pass4Members {
    pub fn run(
        bpa: &BPASTArtifact,
        tca: &TokenCorpusArtifact,
        builder: &mut SymbolTableBuilder,
    ) {
        // Build fast symbol name lookup table for type binding
        let mut class_name_to_sym: HashMap<String, u32> = HashMap::new();
        for sym_id in 0..builder.symbol_count() as u32 {
            if let Some(s) = builder.symbol(sym_id) {
                let kind = SymbolKind::from(s.kind);
                if matches!(
                    kind,
                    SymbolKind::SK_CLASS
                        | SymbolKind::SK_INTERFACE
                        | SymbolKind::SK_ENUM
                        | SymbolKind::SK_RECORD
                ) {
                    let bytes = tca.interner.lookup_text(s.name_id);
                    if let Ok(name) = std::str::from_utf8(bytes) {
                        if !name.is_empty() {
                            class_name_to_sym.insert(name.to_string(), sym_id);
                        }
                    }
                }
            }
        }

        // 1. AST-based Type Reference binding
        for sym_id in 0..builder.symbol_count() as u32 {
            let (kind, decl_node) = match builder.symbol(sym_id) {
                Some(s) => (s.kind, s.decl_node),
                None => continue,
            };

            if decl_node != u32::MAX
                && (kind == SymbolKind::SK_FIELD as u8
                    || kind == SymbolKind::SK_PARAM as u8
                    || kind == SymbolKind::SK_LOCAL_VAR as u8
                    || kind == SymbolKind::SK_METHOD as u8)
            {
                if let Some(type_ref_node) = Self::find_type_ref_child(decl_node, bpa) {
                    if let Some(type_sym_id) = builder.get_type_ref_resolution(type_ref_node) {
                        builder.set_type_id(sym_id, type_sym_id);
                    }
                }
            }
        }

        // 2. Token-level Type Reference binding for Kotlin & Multi-lang member declarations
        for sym_id in 0..builder.symbol_count() as u32 {
            let (kind, type_id, first_token) = match builder.symbol(sym_id) {
                Some(s) => (s.kind, s.type_id, s.first_token_id),
                None => continue,
            };

            if type_id != u32::MAX
                || first_token == u32::MAX
                || (first_token as usize) >= tca.token_records.len()
            {
                continue;
            }

            if kind == SymbolKind::SK_FIELD as u8 || kind == SymbolKind::SK_PARAM as u8 {
                let rec = &tca.token_records[first_token as usize];
                let cur_fid = unpack_sort_key(rec.sort_key).0;

                let mut lookahead = (first_token as usize) + 1;
                let limit = (first_token as usize) + 25;

                let mut found_colon = false;

                while lookahead < tca.token_records.len() && lookahead < limit {
                    let next_rec = &tca.token_records[lookahead];
                    if unpack_sort_key(next_rec.sort_key).0 != cur_fid {
                        break;
                    }

                    let bytes = tca.interner.lookup_text(next_rec.text_id);
                    if bytes == b"=" || bytes == b";" || bytes == b"{" || bytes == b"}" {
                        break;
                    }

                    if bytes == b":" {
                        found_colon = true;
                        lookahead += 1;
                        continue;
                    }

                    if found_colon {
                        if let Ok(type_text) = std::str::from_utf8(bytes) {
                            let clean_name = type_text
                                .trim()
                                .trim_end_matches('?')
                                .trim_matches(|c: char| !c.is_alphanumeric() && c != '_');

                            if !clean_name.is_empty() {
                                if let Some(&target_sym) = class_name_to_sym.get(clean_name) {
                                    builder.set_type_id(sym_id, target_sym);
                                    break;
                                }
                            }
                        }
                    }

                    lookahead += 1;
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
