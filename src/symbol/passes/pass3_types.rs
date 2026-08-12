//! Pass 3 — Type Reference Resolution
//! Resolves all NN_TYPE_REF nodes in BP AST to symbol_ids using scope graph, import maps, java.lang stubs, or SK_EXTERNAL fallbacks.

use crate::ast::BPASTArtifact;
use crate::core::types::ast::ASTNodeType;
use crate::core::types::symbol::{SymbolKind, SymbolRecord, SymbolVisibility};
use crate::ingestion::TokenCorpusArtifact;
use crate::symbol::adapter::SemanticAdapter;
use crate::symbol::builder::SymbolTableBuilder;
use crate::symbol::scope_graph::NameResolver;

pub struct Pass3Types;

impl Pass3Types {
    pub fn run(
        bpa: &BPASTArtifact,
        tca: &TokenCorpusArtifact,
        adapter: &dyn SemanticAdapter,
        builder: &mut SymbolTableBuilder,
    ) {
        for pre_idx in 0..bpa.node_count {
            if bpa.node_type(pre_idx) != ASTNodeType::NN_TYPE_REF {
                continue;
            }

            let (ft, _) = bpa.token_range(pre_idx);
            if ft == u32::MAX || ft >= tca.token_records.len() as u32 {
                continue;
            }

            let text_id = tca.token_records[ft as usize].text_id;
            let bytes = tca.interner.lookup_text(text_id);
            let type_name = match std::str::from_utf8(bytes) {
                Ok(t) if !t.is_empty() => t,
                _ => continue,
            };

            // 1. Primitive types
            if let Some(prim_id) = adapter.primitive_type_id(type_name) {
                builder.set_type_ref_resolution(pre_idx, prim_id);
                continue;
            }

            let scope_id = Self::scope_of_node(pre_idx, bpa, builder);

            // 2. Lexical scope resolution
            let resolved_lexical = NameResolver::resolve_lexical(
                &builder.scope_graph,
                type_name,
                scope_id,
                |sym_id, name| {
                    if let Some(sym) = builder.symbol(sym_id) {
                        if sym.name_id == text_id {
                            return true;
                        }
                        let b = tca.interner.lookup_text(sym.name_id);
                        if let Ok(tname) = std::str::from_utf8(b) {
                            return tname == name;
                        }
                    }
                    false
                },
            );

            if let Some(sym_id) = resolved_lexical {
                builder.set_type_ref_resolution(pre_idx, sym_id);
                continue;
            }

            // 3. Explicit import resolution
            if let Some(qual_name) =
                NameResolver::resolve_via_import_map(&builder.scope_graph, type_name, scope_id)
            {
                let qual_id = builder.qual_names.get_or_intern(&qual_name);
                let ext_sym = Self::get_or_create_external(type_name, qual_id, builder);
                builder.set_type_ref_resolution(pre_idx, ext_sym);
                continue;
            }

            // 4. Standard library / java.lang stubs
            if let Some(stub_sym) = builder.std_lib.lookup(type_name) {
                builder.set_type_ref_resolution(pre_idx, stub_sym);
                continue;
            }

            // 5. Fallback: SK_EXTERNAL
            let qual_id = builder.qual_names.get_or_intern(type_name);
            let ext_sym = Self::get_or_create_external(type_name, qual_id, builder);
            builder.set_type_ref_resolution(pre_idx, ext_sym);
        }
    }

    fn scope_of_node(pre_idx: u32, bpa: &BPASTArtifact, builder: &SymbolTableBuilder) -> u32 {
        let mut cur = pre_idx;
        loop {
            if let Some(sym_id) = builder.symbol_at_node(cur) {
                if let Some(sym) = builder.symbol(sym_id) {
                    return sym.scope_id;
                }
            }
            cur = bpa.parent(cur);
            if cur == u32::MAX {
                return 0; // ROOT_SCOPE
            }
        }
    }

    fn get_or_create_external(
        _simple_name: &str,
        qual_name_id: u32,
        builder: &mut SymbolTableBuilder,
    ) -> u32 {
        for (i, sym) in builder.symbols.iter().enumerate() {
            if sym.kind == SymbolKind::SK_EXTERNAL as u8 && sym.qual_name_id == qual_name_id {
                return i as u32;
            }
        }

        builder.create_symbol(SymbolRecord {
            symbol_id: u32::MAX,
            name_id: 0,
            qual_name_id,
            type_id: u32::MAX,
            decl_node: u32::MAX,
            def_node: u32::MAX,
            parent_sym: u32::MAX,
            first_child: u32::MAX,
            next_sibling: u32::MAX,
            scope_id: 0,
            uml_meta_offset: 0,
            param_count: 0,
            modifiers: 0,
            kind: SymbolKind::SK_EXTERNAL as u8,
            visibility: SymbolVisibility::Public as u8,
            type_param_count: 0,
            flags: 0,
            first_token_id: u32::MAX,
            last_token_id: u32::MAX,
            _reserved: 0,
        })
    }
}
