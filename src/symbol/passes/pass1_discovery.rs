//! Pass 1 — Declaration Discovery
//! Walks BP AST in non-recursive pre-order DFS, builds skeletal SymbolRecords, scope graph, and child/sibling chains.

use crate::ast::BPASTArtifact;
use crate::core::types::ast::{ASTNodeType, NodeAttr};
use crate::core::types::symbol::{
    ScopeKind, SymbolKind, SymbolModifiers, SymbolRecord, SymbolVisibility,
};
use crate::core::types::token::unpack_sort_key;
use crate::ingestion::TokenCorpusArtifact;
use crate::symbol::adapter::SemanticAdapter;
use crate::symbol::builder::SymbolTableBuilder;
use std::collections::HashMap;

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

        // ── Secondary Token-Level Discovery Sweep for Kotlin & Multi-Lang Declarations ──
        let mut registered_name_ids: std::collections::HashSet<u32> = builder
            .symbols
            .iter()
            .map(|s| s.name_id)
            .filter(|&id| id != u32::MAX)
            .collect();

        // Default top-level class symbol to parent orphan top-level methods/fields
        let mut default_file_class = u32::MAX;
        for sym_id in 0..builder.symbols.len() as u32 {
            if let Some(sym) = builder.symbols.get(sym_id as usize) {
                if sym.kind == crate::core::types::symbol::SymbolKind::SK_CLASS as u8
                    || sym.kind == crate::core::types::symbol::SymbolKind::SK_MODULE as u8
                {
                    default_file_class = sym_id;
                    break;
                }
            }
        }
        if default_file_class == u32::MAX {
            let mut sym_rec = SymbolRecord::UNINIT;
            sym_rec.kind = crate::core::types::symbol::SymbolKind::SK_CLASS as u8;
            sym_rec.visibility = crate::core::types::symbol::SymbolVisibility::Public as u8;
            sym_rec.scope_id = root_scope;
            default_file_class = builder.create_symbol(sym_rec);
        }

        let mut current_class_sym = default_file_class;
        let mut current_pkg_sym = u32::MAX;
        let mut tok_idx = 0usize;
        while tok_idx < tca.token_records.len() {
            let rec = &tca.token_records[tok_idx];
            let bytes = tca.interner.lookup_text(rec.text_id);
            if let Ok(text) = std::str::from_utf8(bytes) {
                let sym_kind = match text {
                    "class" | "object" => Some(crate::core::types::symbol::SymbolKind::SK_CLASS),
                    "interface" => Some(crate::core::types::symbol::SymbolKind::SK_INTERFACE),
                    "enum" => Some(crate::core::types::symbol::SymbolKind::SK_ENUM),
                    _ => None,
                };

                if let Some(kind) = sym_kind {
                    let mut lookahead = tok_idx + 1;
                    while lookahead < tca.token_records.len() && lookahead < tok_idx + 10 {
                        let next_rec = &tca.token_records[lookahead];
                        let next_bytes = tca.interner.lookup_text(next_rec.text_id);
                        if let Ok(next_text) = std::str::from_utf8(next_bytes) {
                            if !next_text.is_empty()
                                && (next_text.chars().next().unwrap_or('\0').is_alphabetic()
                                    || next_text.starts_with('_'))
                                && ![
                                    "class",
                                    "interface",
                                    "object",
                                    "enum",
                                    "fun",
                                    "val",
                                    "var",
                                    "public",
                                    "private",
                                    "protected",
                                    "internal",
                                    "data",
                                    "sealed",
                                    "open",
                                    "abstract",
                                    "companion",
                                    "constructor",
                                ]
                                .contains(&next_text)
                            {
                                if registered_name_ids.insert(next_rec.text_id) {
                                    let mut sym_rec = SymbolRecord::UNINIT;
                                    sym_rec.name_id = next_rec.text_id;
                                    sym_rec.kind = kind as u8;
                                    sym_rec.visibility =
                                        crate::core::types::symbol::SymbolVisibility::Public as u8;
                                    sym_rec.scope_id = root_scope;
                                    if current_pkg_sym != u32::MAX {
                                        sym_rec.parent_sym = current_pkg_sym;
                                    }
                                    let sym_id = builder.create_symbol(sym_rec);
                                    if current_pkg_sym != u32::MAX {
                                        builder.append_child(current_pkg_sym, sym_id);
                                    }
                                    current_class_sym = sym_id;
                                }
                                break;
                            }
                        }
                        lookahead += 1;
                    }
                } else if text == "fun" || text == "val" || text == "var" {
                    let is_func = text == "fun";
                    let kind = if is_func {
                        crate::core::types::symbol::SymbolKind::SK_METHOD
                    } else {
                        crate::core::types::symbol::SymbolKind::SK_FIELD
                    };
                    let mut lookahead = tok_idx + 1;
                    while lookahead < tca.token_records.len() && lookahead < tok_idx + 10 {
                        let next_rec = &tca.token_records[lookahead];
                        let next_bytes = tca.interner.lookup_text(next_rec.text_id);
                        if let Ok(next_text) = std::str::from_utf8(next_bytes) {
                            if !next_text.is_empty()
                                && (next_text.chars().next().unwrap_or('\0').is_alphabetic()
                                    || next_text.starts_with('_'))
                                && ![
                                    "class",
                                    "interface",
                                    "object",
                                    "enum",
                                    "fun",
                                    "val",
                                    "var",
                                    "public",
                                    "private",
                                    "protected",
                                    "internal",
                                    "data",
                                    "sealed",
                                    "open",
                                    "abstract",
                                    "override",
                                ]
                                .contains(&next_text)
                            {
                                if registered_name_ids.insert(next_rec.text_id) {
                                    let target_parent = if current_class_sym != u32::MAX {
                                        current_class_sym
                                    } else {
                                        default_file_class
                                    };
                                    let mut sym_rec = SymbolRecord::UNINIT;
                                    sym_rec.name_id = next_rec.text_id;
                                    sym_rec.kind = kind as u8;
                                    sym_rec.visibility =
                                        crate::core::types::symbol::SymbolVisibility::Public as u8;
                                    sym_rec.scope_id = root_scope;
                                    sym_rec.parent_sym = target_parent;
                                    let sym_id = builder.create_symbol(sym_rec);
                                    builder.append_child(target_parent, sym_id);
                                }
                                break;
                            }
                        }
                        lookahead += 1;
                    }
                }
            }
            tok_idx += 1;
        }

        // ── Link class symbols to their file package symbols ──
        let mut file_pkg_map: HashMap<u16, u32> = HashMap::new();
        let mut visited_pkg_files: std::collections::HashSet<u16> =
            std::collections::HashSet::new();
        let mut package_path_ids: HashMap<String, u32> = HashMap::new();
        let mut tok_idx = 0usize;
        while tok_idx < tca.token_records.len() {
            let rec = &tca.token_records[tok_idx];
            let rec_fid = unpack_sort_key(rec.sort_key).0;
            let bytes = tca.interner.lookup_text(rec.text_id);
            if let Ok(text) = std::str::from_utf8(bytes) {
                if text == "package" && visited_pkg_files.insert(rec_fid) {
                    let mut lookahead = tok_idx + 1;
                    let mut pkg_parts: Vec<(u32, String)> = Vec::new();
                    let pkg_line = unpack_sort_key(rec.sort_key).1;
                    let mut expecting_ident = true;

                    while lookahead < tca.token_records.len() && lookahead < tok_idx + 40 {
                        let next_rec = &tca.token_records[lookahead];
                        let (next_fid, next_line, _) = unpack_sort_key(next_rec.sort_key);
                        if next_fid != rec_fid || next_line != pkg_line {
                            break;
                        }
                        let next_bytes = tca.interner.lookup_text(next_rec.text_id);
                        if let Ok(next_text) = std::str::from_utf8(next_bytes) {
                            if next_text == ";" || next_text == "\n" {
                                break;
                            }
                            if expecting_ident {
                                let is_ident = !next_text.is_empty()
                                    && next_text.chars().all(|c| c.is_alphanumeric() || c == '_')
                                    && ![
                                        "package",
                                        "import",
                                        "class",
                                        "interface",
                                        "fun",
                                        "val",
                                        "var",
                                        "public",
                                        "private",
                                        "object",
                                        "enum",
                                        "data",
                                        "sealed",
                                        "open",
                                        "abstract",
                                    ]
                                    .contains(&next_text);
                                if is_ident {
                                    pkg_parts.push((next_rec.text_id, next_text.to_string()));
                                    expecting_ident = false;
                                } else {
                                    break;
                                }
                            } else if next_text == "." {
                                expecting_ident = true;
                            } else {
                                break;
                            }
                        }
                        lookahead += 1;
                    }

                    if !pkg_parts.is_empty() {
                        let mut current_pkg_sym = u32::MAX;
                        let mut current_path = String::new();
                        for (name_id, part) in pkg_parts {
                            current_path = if current_path.is_empty() {
                                part.clone()
                            } else {
                                format!("{}.{}", current_path, part)
                            };

                            let pkg_sym_id =
                                if let Some(existing) = package_path_ids.get(&current_path) {
                                    *existing
                                } else {
                                    let mut sym_rec = SymbolRecord::UNINIT;
                                    sym_rec.name_id = name_id;
                                    sym_rec.kind =
                                        crate::core::types::symbol::SymbolKind::SK_PACKAGE as u8;
                                    sym_rec.visibility =
                                        crate::core::types::symbol::SymbolVisibility::Public as u8;
                                    sym_rec.scope_id = root_scope;
                                    sym_rec.parent_sym = current_pkg_sym;
                                    let pkg_sym_id = builder.create_symbol(sym_rec);
                                    if current_pkg_sym != u32::MAX {
                                        builder.append_child(current_pkg_sym, pkg_sym_id);
                                    }
                                    builder
                                        .custom_package_names
                                        .insert(pkg_sym_id, current_path.clone());
                                    package_path_ids.insert(current_path.clone(), pkg_sym_id);
                                    pkg_sym_id
                                };

                            current_pkg_sym = pkg_sym_id;
                        }

                        if current_pkg_sym != u32::MAX {
                            file_pkg_map.insert(rec_fid, current_pkg_sym);
                            builder
                                .file_package_names
                                .insert(rec_fid, current_path.clone());
                        }
                    }
                }
            }
            tok_idx += 1;
        }

        for sym_idx in 0..builder.symbols.len() {
            let kind = SymbolKind::from(builder.symbols[sym_idx].kind);
            if matches!(
                kind,
                SymbolKind::SK_CLASS | SymbolKind::SK_INTERFACE | SymbolKind::SK_ENUM
            ) {
                let decl_node = builder.symbols[sym_idx].decl_node;
                let fid = if decl_node != u32::MAX {
                    let (ft, _) = bpa.token_range(decl_node);
                    if (ft as usize) < tca.token_records.len() {
                        unpack_sort_key(tca.token_records[ft as usize].sort_key).0
                    } else {
                        u16::MAX
                    }
                } else {
                    let ft = builder.symbols[sym_idx].first_token_id;
                    if (ft as usize) < tca.token_records.len() {
                        unpack_sort_key(tca.token_records[ft as usize].sort_key).0
                    } else {
                        u16::MAX
                    }
                };

                if fid != u16::MAX {
                    if let Some(&pkg_sym_id) = file_pkg_map.get(&fid) {
                        if builder.symbols[sym_idx].parent_sym == u32::MAX
                            || builder.symbols[sym_idx].parent_sym == default_file_class
                        {
                            builder.symbols[sym_idx].parent_sym = pkg_sym_id;
                            builder.append_child(pkg_sym_id, sym_idx as u32);
                        }
                    }
                }
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
            let keywords = [
                "package",
                "import",
                "public",
                "private",
                "protected",
                "static",
                "final",
                "abstract",
                "class",
                "interface",
                "enum",
                "record",
                "extends",
                "implements",
                "fun",
                "val",
                "var",
                "object",
                "companion",
                "data",
                "sealed",
                "open",
                "override",
                "internal",
                "void",
                "synchronized",
                "transient",
                "volatile",
                "default",
                "strictfp",
                "native",
                "throws",
            ];
            let ntype = bpa.node_type(pre_idx);
            let end_tok = lt.min(tca.token_records.len() as u32 - 1);

            // For field declarations, pick the identifier immediately preceding = or ;
            if matches!(
                ntype,
                ASTNodeType::NN_FIELD_DECL | ASTNodeType::NN_LOCAL_VAR_DECL
            ) {
                let mut candidate = u32::MAX;
                for tok_idx in ft..=end_tok {
                    let rec = &tca.token_records[tok_idx as usize];
                    let bytes = tca.interner.lookup_text(rec.text_id);
                    if let Ok(text) = std::str::from_utf8(bytes) {
                        if text == "=" || text == ";" {
                            if candidate != u32::MAX {
                                return candidate;
                            }
                        }
                        if !text.is_empty()
                            && (text.chars().next().unwrap_or('\0').is_alphabetic()
                                || text.starts_with('_'))
                            && !keywords.contains(&text)
                        {
                            candidate = rec.text_id;
                        }
                    }
                }
                if candidate != u32::MAX {
                    return candidate;
                }
            }

            for tok_idx in ft..=end_tok {
                let rec = &tca.token_records[tok_idx as usize];
                let bytes = tca.interner.lookup_text(rec.text_id);
                if let Ok(text) = std::str::from_utf8(bytes) {
                    if !text.is_empty()
                        && (text.chars().next().unwrap_or('\0').is_alphabetic()
                            || text.starts_with('_'))
                        && !keywords.contains(&text)
                    {
                        return rec.text_id;
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
