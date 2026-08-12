//! Pass 1 — Declaration Discovery
//! Walks BP AST in non-recursive pre-order DFS, builds skeletal SymbolRecords, scope graph, and child/sibling chains.

use crate::ast::BPASTArtifact;
use crate::core::types::ast::{ASTNodeType, NodeAttr};
use crate::core::types::symbol::{ScopeKind, SymbolModifiers, SymbolRecord, SymbolVisibility};
use crate::ingestion::TokenCorpusArtifact;
use crate::symbol::adapter::SemanticAdapter;
use crate::symbol::builder::SymbolTableBuilder;

pub struct Pass1Discovery;

impl Pass1Discovery {
    pub fn run(
        bpa: &BPASTArtifact,
        tca: &TokenCorpusArtifact,
        adapter: &dyn SemanticAdapter,
        builder: &mut SymbolTableBuilder,
    ) {
        if bpa.node_count == 0 {
            return;
        }

        #[derive(Debug, Clone, Copy)]
        struct Frame {
            pre_idx: u32,
            scope_id: u32,
            parent_sym: u32,
        }

        let root_scope = builder
            .scope_graph
            .create_scope(u32::MAX, u32::MAX, ScopeKind::File);

        let mut stack: Vec<Frame> = vec![Frame {
            pre_idx: 0,
            scope_id: root_scope,
            parent_sym: u32::MAX,
        }];

        let mut visited = vec![false; bpa.node_count as usize];

        while let Some(frame) = stack.last().cloned() {
            let Frame {
                pre_idx,
                scope_id,
                parent_sym,
            } = frame;

            let node_type = bpa.node_type(pre_idx);

            if (pre_idx as usize) < visited.len() && !visited[pre_idx as usize] {
                visited[pre_idx as usize] = true;

                let mut current_scope = scope_id;
                let mut current_parent = parent_sym;

                if adapter.is_declaration(node_type) {
                    let name_id = Self::extract_name_token(pre_idx, bpa, tca, adapter);
                    let vis = Self::extract_visibility(pre_idx, bpa, adapter);
                    let mods = Self::extract_modifiers(pre_idx, bpa, adapter);
                    let kind = adapter.symbol_kind(node_type);
                    let (ft, lt) = bpa.token_range(pre_idx);

                    let sym_id = builder.create_symbol(SymbolRecord {
                        symbol_id: u32::MAX,
                        name_id,
                        qual_name_id: u32::MAX,
                        type_id: u32::MAX,
                        decl_node: pre_idx,
                        def_node: pre_idx,
                        parent_sym: current_parent,
                        first_child: u32::MAX,
                        next_sibling: u32::MAX,
                        scope_id: current_scope,
                        uml_meta_offset: 0,
                        param_count: Self::count_params(pre_idx, bpa),
                        modifiers: mods,
                        kind: kind as u8,
                        visibility: vis as u8,
                        type_param_count: 0,
                        flags: 0,
                        first_token_id: ft,
                        last_token_id: lt,
                        _reserved: 0,
                    });

                    builder.append_child(current_parent, sym_id);

                    let child_scope =
                        builder.create_scope(sym_id, current_scope, adapter.scope_kind(node_type));

                    current_scope = child_scope;
                    current_parent = sym_id;

                    stack.last_mut().unwrap().scope_id = child_scope;
                    stack.last_mut().unwrap().parent_sym = sym_id;
                }

                if let Some(child) = bpa.first_child(pre_idx) {
                    stack.push(Frame {
                        pre_idx: child,
                        scope_id: current_scope,
                        parent_sym: current_parent,
                    });
                    continue;
                }
            }

            stack.pop();
            let parent_frame = stack.last().cloned();
            if let Some(sib) = bpa.next_sibling(pre_idx) {
                let (sc, ps) = parent_frame
                    .map(|f| (f.scope_id, f.parent_sym))
                    .unwrap_or((root_scope, u32::MAX));

                stack.push(Frame {
                    pre_idx: sib,
                    scope_id: sc,
                    parent_sym: ps,
                });
            }
        }
    }

    fn extract_name_token(
        pre_idx: u32,
        bpa: &BPASTArtifact,
        tca: &TokenCorpusArtifact,
        _adapter: &dyn SemanticAdapter,
    ) -> u32 {
        let (ft, lt) = bpa.token_range(pre_idx);
        if ft != u32::MAX && ft < tca.token_records.len() as u32 {
            for tok_idx in ft..=lt.min(tca.token_records.len() as u32 - 1) {
                let text_id = tca.token_records[tok_idx as usize].text_id;
                let bytes = tca.interner.lookup_text(text_id);
                if let Ok(text) = std::str::from_utf8(bytes) {
                    if !text.is_empty() && text.chars().next().unwrap_or('\0').is_alphabetic() {
                        return text_id;
                    }
                }
            }
            return tca.token_records[ft as usize].text_id;
        }
        u32::MAX
    }

    fn extract_visibility(
        pre_idx: u32,
        bpa: &BPASTArtifact,
        adapter: &dyn SemanticAdapter,
    ) -> SymbolVisibility {
        let attr = bpa.node_attr(pre_idx);
        let raw_vis = NodeAttr::unpack_visibility(attr);
        match raw_vis {
            NodeAttr::VISIBILITY_PUBLIC => SymbolVisibility::Public,
            NodeAttr::VISIBILITY_PRIVATE => SymbolVisibility::Private,
            NodeAttr::VISIBILITY_PROTECTED => SymbolVisibility::Protected,
            NodeAttr::VISIBILITY_PACKAGE_PRIVATE => SymbolVisibility::Package,
            _ => {
                let kind = adapter.symbol_kind(bpa.node_type(pre_idx));
                adapter.default_visibility(kind)
            }
        }
    }

    fn extract_modifiers(pre_idx: u32, bpa: &BPASTArtifact, _adapter: &dyn SemanticAdapter) -> u16 {
        let mut mods = 0;
        let attr = bpa.node_attr(pre_idx);
        let unpacked = NodeAttr::unpack_modifiers(attr);
        if (unpacked & NodeAttr::MOD_STATIC) != 0 {
            mods |= SymbolModifiers::STATIC;
        }
        if (unpacked & NodeAttr::MOD_FINAL) != 0 {
            mods |= SymbolModifiers::FINAL;
        }
        if (unpacked & NodeAttr::MOD_ABSTRACT) != 0 {
            mods |= SymbolModifiers::ABSTRACT;
        }
        if (unpacked & NodeAttr::MOD_SYNCHRONIZED) != 0 {
            mods |= SymbolModifiers::SYNCHRONIZED;
        }
        if (unpacked & NodeAttr::MOD_NATIVE) != 0 {
            mods |= SymbolModifiers::NATIVE;
        }
        if (unpacked & NodeAttr::MOD_VOLATILE) != 0 {
            mods |= SymbolModifiers::VOLATILE;
        }
        if (unpacked & NodeAttr::MOD_TRANSIENT) != 0 {
            mods |= SymbolModifiers::TRANSIENT;
        }
        if (unpacked & NodeAttr::MOD_SEALED) != 0 {
            mods |= SymbolModifiers::SEALED;
        }
        mods
    }

    fn count_params(pre_idx: u32, bpa: &BPASTArtifact) -> u16 {
        let mut count = 0;
        let mut cur = bpa.first_child(pre_idx);
        while let Some(c) = cur {
            if bpa.node_type(c) == ASTNodeType::NN_PARAM_DECL {
                count += 1;
            }
            cur = bpa.next_sibling(c);
        }
        count
    }
}
