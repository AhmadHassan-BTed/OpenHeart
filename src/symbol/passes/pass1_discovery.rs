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
        let pass1_start = std::time::Instant::now();

        let root_scope = builder
            .scope_graph
            .create_scope(u32::MAX, u32::MAX, ScopeKind::File);

        let blocklist_raw = [
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
            "package",
            "import",
            "override",
            "struct",
            "trait",
            "impl",
            "let",
            "const",
            "def",
            "fn",
            "new",
            "this",
            "super",
            "null",
            "true",
            "false",
            "void",
            "return",
            "if",
            "else",
            "for",
            "while",
            "do",
            "switch",
            "case",
            "break",
            "continue",
            "try",
            "catch",
            "finally",
            "throw",
            "by",
            "as",
            "in",
            "is",
            "default",
            "export",
            "declare",
            "async",
            "yield",
            "await",
            "function",
        ];
        let blocklist_ids: std::collections::HashSet<u32> = blocklist_raw
            .iter()
            .filter_map(|s| {
                let id = tca.interner.find_id(s.as_bytes());
                if id != u32::MAX {
                    Some(id)
                } else {
                    None
                }
            })
            .collect();

        let mut sym_id_by_node: Vec<u32> = vec![u32::MAX; bpa.node_count as usize];
        let mut scope_id_by_node: Vec<u32> = vec![root_scope; bpa.node_count as usize];

        for pre_idx in 0..bpa.node_count {
            let node_type = bpa.node_type(pre_idx);
            let parent_node = bpa.parent(pre_idx);

            let (parent_scope, parent_sym) =
                if parent_node != u32::MAX && (parent_node as usize) < (bpa.node_count as usize) {
                    (
                        scope_id_by_node[parent_node as usize],
                        sym_id_by_node[parent_node as usize],
                    )
                } else {
                    (root_scope, u32::MAX)
                };

            let mut current_scope = parent_scope;
            let mut current_parent = parent_sym;

            if pre_idx < 100 {
                crate::core::logger::log_debug(&format!(
                    "[DIAG-PASS1] pre_idx={} node_type={:?} is_decl={}",
                    pre_idx,
                    node_type,
                    adapter.is_declaration(node_type)
                ));
            }
            if adapter.is_declaration(node_type) {
                let name_id = Self::extract_name_token(pre_idx, bpa, tca, adapter, &blocklist_ids);
                let vis = Self::extract_visibility(pre_idx, bpa, adapter);
                let mods = Self::extract_modifiers(pre_idx, bpa, adapter);
                let kind = adapter.symbol_kind(node_type);
                let (ft, lt) = bpa.token_range(pre_idx);

                if kind == SymbolKind::SK_CLASS
                    || kind == SymbolKind::SK_INTERFACE
                    || kind == SymbolKind::SK_ENUM
                    || kind == SymbolKind::SK_RECORD
                {
                    let name_bytes = if name_id != u32::MAX {
                        tca.interner.lookup_text(name_id)
                    } else {
                        b""
                    };
                    crate::core::logger::log_info(&format!(
                        "[DIAG-PASS1] Class Discovered: pre_idx={} name_id={} name={} kind={:?}",
                        pre_idx,
                        name_id,
                        String::from_utf8_lossy(name_bytes),
                        kind
                    ));
                }

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
            }

            sym_id_by_node[pre_idx as usize] = current_parent;
            scope_id_by_node[pre_idx as usize] = current_scope;
        }
        crate::core::logger::log_info(&format!(
            "[PASS1] Primary loop done in {:?}",
            pass1_start.elapsed()
        ));

        // ── Secondary Token-Level Discovery Sweep for Kotlin & Multi-Lang Declarations ──
        let mut registered_name_ids: std::collections::HashSet<(u16, u32)> = builder
            .symbols
            .iter()
            .filter_map(|s| {
                if s.name_id != u32::MAX
                    && s.first_token_id != u32::MAX
                    && (s.first_token_id as usize) < tca.token_records.len()
                {
                    let fid =
                        unpack_sort_key(tca.token_records[s.first_token_id as usize].sort_key).0;
                    let kind = SymbolKind::from(s.kind);
                    if matches!(
                        kind,
                        SymbolKind::SK_CLASS
                            | SymbolKind::SK_INTERFACE
                            | SymbolKind::SK_ENUM
                            | SymbolKind::SK_RECORD
                    ) {
                        Some((fid, s.name_id))
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
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
            sym_rec.kind = crate::core::types::symbol::SymbolKind::SK_MODULE as u8;
            sym_rec.visibility = crate::core::types::symbol::SymbolVisibility::Public as u8;
            sym_rec.scope_id = root_scope;
            default_file_class = builder.create_symbol(sym_rec);
        }

        let kw_class = tca.interner.find_id(b"class");
        let kw_interface = tca.interner.find_id(b"interface");
        let kw_object = tca.interner.find_id(b"object");
        let kw_enum = tca.interner.find_id(b"enum");
        let kw_record = tca.interner.find_id(b"record");
        let kw_struct = tca.interner.find_id(b"struct");
        let kw_trait = tca.interner.find_id(b"trait");

        let mut current_class_sym = default_file_class;
        let mut tok_idx = 0usize;
        while tok_idx < tca.token_records.len() {
            let rec = &tca.token_records[tok_idx];
            if rec.token_type == crate::core::types::token::TokenType::StringLiteral as u8
                || rec.token_type == crate::core::types::token::TokenType::CommentLine as u8
                || rec.token_type == crate::core::types::token::TokenType::CommentBlock as u8
                || rec.token_type == crate::core::types::token::TokenType::CommentDoc as u8
            {
                tok_idx += 1;
                continue;
            }

            let cur_fid = unpack_sort_key(rec.sort_key).0;
            let tid = rec.text_id;

            // kw_* variables already computed before loop (lines 249-255)
            let kw_fun = tca.interner.find_id(b"fun");
            let kw_val = tca.interner.find_id(b"val");
            let kw_var = tca.interner.find_id(b"var");

            let sym_kind = if tid == kw_class || tid == kw_object || tid == kw_struct {
                Some(crate::core::types::symbol::SymbolKind::SK_CLASS)
            } else if tid == kw_interface || tid == kw_trait {
                Some(crate::core::types::symbol::SymbolKind::SK_INTERFACE)
            } else if tid == kw_enum {
                Some(crate::core::types::symbol::SymbolKind::SK_ENUM)
            } else if tid == kw_record {
                Some(crate::core::types::symbol::SymbolKind::SK_RECORD)
            } else {
                None
            };

            if let Some(kind) = sym_kind {
                let mut lookahead = tok_idx + 1;
                while lookahead < tca.token_records.len() && lookahead < tok_idx + 10 {
                    let next_rec = &tca.token_records[lookahead];
                    if unpack_sort_key(next_rec.sort_key).0 != cur_fid {
                        break;
                    }
                    if next_rec.token_type
                        == crate::core::types::token::TokenType::StringLiteral as u8
                        || next_rec.token_type
                            == crate::core::types::token::TokenType::CommentLine as u8
                        || next_rec.token_type
                            == crate::core::types::token::TokenType::CommentBlock as u8
                        || next_rec.token_type
                            == crate::core::types::token::TokenType::CommentDoc as u8
                    {
                        break;
                    }
                    let ntid = next_rec.text_id;
                    let next_bytes = tca.interner.lookup_text(ntid);
                    if matches!(next_bytes, b"{" | b"}" | b";" | b"=") {
                        break;
                    }
                    if lookahead > 0 {
                        let prev_rec = &tca.token_records[lookahead - 1];
                        let prev_bytes = tca.interner.lookup_text(prev_rec.text_id);
                        if matches!(
                            prev_bytes,
                            b"fun"
                                | b"function"
                                | b"def"
                                | b"fn"
                                | b"val"
                                | b"var"
                                | b"let"
                                | b"const"
                                | b"import"
                                | b"package"
                        ) {
                            lookahead += 1;
                            continue;
                        }
                    }
                    let next_bytes = tca.interner.lookup_text(ntid);
                    if matches!(
                        next_bytes,
                        b"default"
                            | b"abstract"
                            | b"export"
                            | b"public"
                            | b"private"
                            | b"protected"
                            | b"internal"
                            | b"pub"
                    ) {
                        lookahead += 1;
                        continue;
                    }

                    if !blocklist_ids.contains(&ntid) {
                        if let Ok(next_str) = std::str::from_utf8(next_bytes) {
                            if !next_str.is_empty()
                                && !next_str.starts_with('"')
                                && !next_str.starts_with('\'')
                                && (next_str.as_bytes()[0].is_ascii_uppercase()
                                    || (next_str.contains('_') && !next_str.starts_with('_')))
                                && !matches!(
                                    next_str,
                                    "Class"
                                        | "Interface"
                                        | "Enum"
                                        | "Object"
                                        | "String"
                                        | "Boolean"
                                        | "Number"
                                        | "Function"
                                        | "Array"
                                        | "Map"
                                        | "Set"
                                        | "Promise"
                                )
                            {
                                if registered_name_ids.contains(&(cur_fid, ntid)) {
                                    if let Some(pos) = builder.symbols.iter().position(|s| {
                                        s.name_id == ntid
                                            && s.first_token_id != u32::MAX
                                            && (s.first_token_id as usize) < tca.token_records.len()
                                            && unpack_sort_key(
                                                tca.token_records[s.first_token_id as usize]
                                                    .sort_key,
                                            )
                                            .0 == cur_fid
                                    }) {
                                        let ex_kind = SymbolKind::from(builder.symbols[pos].kind);
                                        if ex_kind == SymbolKind::SK_METHOD {
                                            break;
                                        }
                                        if ex_kind != SymbolKind::SK_CLASS
                                            && ex_kind != SymbolKind::SK_INTERFACE
                                            && ex_kind != SymbolKind::SK_ENUM
                                            && ex_kind != SymbolKind::SK_RECORD
                                        {
                                            builder.symbols[pos].kind = kind as u8;
                                        }
                                        current_class_sym = pos as u32;
                                    }
                                } else if registered_name_ids.insert((cur_fid, ntid)) {
                                    let mut sym_rec = SymbolRecord::UNINIT;
                                    sym_rec.name_id = ntid;
                                    sym_rec.kind = kind as u8;
                                    sym_rec.visibility =
                                        crate::core::types::symbol::SymbolVisibility::Public as u8;
                                    sym_rec.scope_id = root_scope;
                                    sym_rec.first_token_id = lookahead as u32;
                                    sym_rec.last_token_id = lookahead as u32;
                                    let sym_id = builder.create_symbol(sym_rec);
                                    current_class_sym = sym_id;
                                }
                                break;
                            }
                        }
                    }
                    lookahead += 1;
                }
            } else if tid == kw_fun || tid == kw_val || tid == kw_var {
                let is_func = tid == kw_fun;
                let kind = if is_func {
                    crate::core::types::symbol::SymbolKind::SK_METHOD
                } else {
                    crate::core::types::symbol::SymbolKind::SK_FIELD
                };
                let mut lookahead = tok_idx + 1;
                while lookahead < tca.token_records.len() && lookahead < tok_idx + 10 {
                    let next_rec = &tca.token_records[lookahead];
                    if unpack_sort_key(next_rec.sort_key).0 != cur_fid {
                        break;
                    }
                    let ntid = next_rec.text_id;
                    if !blocklist_ids.contains(&ntid) {
                        let next_bytes = tca.interner.lookup_text(ntid);
                        if !next_bytes.is_empty()
                            && (next_bytes[0].is_ascii_alphabetic() || next_bytes[0] == b'_')
                        {
                            if registered_name_ids.insert((cur_fid, ntid)) {
                                let target_parent = if current_class_sym != u32::MAX {
                                    current_class_sym
                                } else {
                                    default_file_class
                                };
                                let mut sym_rec = SymbolRecord::UNINIT;
                                sym_rec.name_id = ntid;
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
            tok_idx += 1;
        }
        crate::core::logger::log_info(&format!(
            "[PASS1] Secondary sweep done in {:?}",
            pass1_start.elapsed()
        ));

        // ── Link class symbols to their file package symbols ──
        let mut file_pkg_map: HashMap<u16, u32> = HashMap::new();
        let kw_package = tca.interner.find_id(b"package");

        let mut visited_pkg_files: std::collections::HashSet<u16> =
            std::collections::HashSet::new();
        let mut package_path_ids: HashMap<String, u32> = HashMap::new();
        let mut tok_idx = 0usize;
        while tok_idx < tca.token_records.len() {
            let rec = &tca.token_records[tok_idx];
            let rec_fid = unpack_sort_key(rec.sort_key).0;
            if rec.text_id == kw_package
                && kw_package != u32::MAX
                && visited_pkg_files.insert(rec_fid)
            {
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

                        let pkg_sym_id = if let Some(existing) = package_path_ids.get(&current_path)
                        {
                            *existing
                        } else {
                            let mut sym_rec = SymbolRecord::UNINIT;
                            sym_rec.name_id = name_id;
                            sym_rec.kind = crate::core::types::symbol::SymbolKind::SK_PACKAGE as u8;
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
            tok_idx += 1;
        }
        crate::core::logger::log_info(&format!(
            "[PASS1] Package statement parse done in {:?}",
            pass1_start.elapsed()
        ));

        for sym_idx in 0..builder.symbols.len() {
            let kind = SymbolKind::from(builder.symbols[sym_idx].kind);
            if matches!(
                kind,
                SymbolKind::SK_CLASS
                    | SymbolKind::SK_INTERFACE
                    | SymbolKind::SK_ENUM
                    | SymbolKind::SK_RECORD
            ) {
                let decl_node = builder.symbols[sym_idx].decl_node;
                let ft = if builder.symbols[sym_idx].first_token_id != u32::MAX {
                    builder.symbols[sym_idx].first_token_id
                } else if decl_node != u32::MAX {
                    bpa.token_range(decl_node).0
                } else {
                    u32::MAX
                };
                let fid = if ft != u32::MAX && (ft as usize) < tca.token_records.len() {
                    unpack_sort_key(tca.token_records[ft as usize].sort_key).0
                } else {
                    u16::MAX
                };

                if fid != u16::MAX {
                    if let Some(&pkg_sym_id) = file_pkg_map.get(&fid) {
                        let parent_is_module = builder.symbols[sym_idx].parent_sym == u32::MAX
                            || builder.symbols[sym_idx].parent_sym == 0
                            || builder.symbols[sym_idx].parent_sym == default_file_class
                            || ((builder.symbols[sym_idx].parent_sym as usize)
                                < builder.symbols.len()
                                && SymbolKind::from(
                                    builder.symbols[builder.symbols[sym_idx].parent_sym as usize]
                                        .kind,
                                ) == SymbolKind::SK_MODULE);
                        if parent_is_module {
                            builder.symbols[sym_idx].parent_sym = pkg_sym_id;
                            builder.append_child(pkg_sym_id, sym_idx as u32);
                        }
                    }
                }
            }
        }

        // Universal parent safety sweep: Ensure Invariant 2 holds for all symbols across all languages
        for i in 1..builder.symbols.len() {
            if builder.symbols[i].parent_sym == u32::MAX {
                builder.symbols[i].parent_sym = 0;
                builder.append_child(0, i as u32);
            }
        }
        crate::core::logger::log_info(&format!(
            "[PASS1] Total Pass 1 done in {:?}",
            pass1_start.elapsed()
        ));
    }

    fn extract_name_token(
        pre_idx: u32,
        bpa: &BPASTArtifact,
        tca: &TokenCorpusArtifact,
        _adapter: &dyn SemanticAdapter,
        blocklist_ids: &std::collections::HashSet<u32>,
    ) -> u32 {
        let (ft, lt) = bpa.token_range(pre_idx);
        if ft != u32::MAX && (ft as usize) < tca.token_records.len() {
            let ntype = bpa.node_type(pre_idx);
            let end_tok = lt.min(tca.token_records.len() as u32 - 1).min(ft + 15);

            // For class/interface/enum declarations, locate the keyword and pick the identifier immediately after it
            if matches!(
                ntype,
                ASTNodeType::NN_CLASS_DECL
                    | ASTNodeType::NN_INTERFACE_DECL
                    | ASTNodeType::NN_ENUM_DECL
                    | ASTNodeType::NN_RECORD_DECL
            ) {
                let mut kw_pos = u32::MAX;
                for tok_idx in ft..=end_tok {
                    let rec = &tca.token_records[tok_idx as usize];
                    let tid = rec.text_id;
                    let bytes = tca.interner.lookup_text(tid);
                    if matches!(
                        bytes,
                        b"class" | b"interface" | b"enum" | b"struct" | b"trait" | b"record"
                    ) {
                        kw_pos = tok_idx;
                        break;
                    }
                }
                if kw_pos != u32::MAX {
                    for tok_idx in (kw_pos + 1)..=end_tok {
                        let rec = &tca.token_records[tok_idx as usize];
                        let tid = rec.text_id;
                        let bytes = tca.interner.lookup_text(tid);
                        if matches!(
                            bytes,
                            b"default"
                                | b"export"
                                | b"abstract"
                                | b"public"
                                | b"private"
                                | b"protected"
                                | b"internal"
                        ) {
                            continue;
                        }
                        if !blocklist_ids.contains(&tid) {
                            if !bytes.is_empty()
                                && (bytes[0].is_ascii_alphabetic() || bytes[0] == b'_')
                            {
                                return tid;
                            }
                        }
                    }
                }
            }

            // For field declarations, pick the identifier immediately preceding = or ;
            if matches!(
                ntype,
                ASTNodeType::NN_FIELD_DECL | ASTNodeType::NN_LOCAL_VAR_DECL
            ) {
                let mut candidate = u32::MAX;
                for tok_idx in ft..=end_tok {
                    let rec = &tca.token_records[tok_idx as usize];
                    let tid = rec.text_id;
                    if !blocklist_ids.contains(&tid) {
                        let bytes = tca.interner.lookup_text(tid);
                        if !bytes.is_empty() && (bytes[0].is_ascii_alphabetic() || bytes[0] == b'_')
                        {
                            candidate = tid;
                        }
                    }
                }
                if candidate != u32::MAX {
                    return candidate;
                }
            }

            let fid_ft = unpack_sort_key(tca.token_records[ft as usize].sort_key).0;
            for tok_idx in ft..=end_tok {
                let rec = &tca.token_records[tok_idx as usize];
                if unpack_sort_key(rec.sort_key).0 != fid_ft {
                    break;
                }
                let tid = rec.text_id;
                let bytes = tca.interner.lookup_text(tid);
                if let Ok(text) = std::str::from_utf8(bytes) {
                    if !text.is_empty()
                        && (text.as_bytes()[0].is_ascii_alphabetic() || text.as_bytes()[0] == b'_')
                    {
                        if !matches!(
                            text,
                            "class"
                                | "interface"
                                | "object"
                                | "enum"
                                | "fun"
                                | "val"
                                | "var"
                                | "public"
                                | "private"
                                | "protected"
                                | "internal"
                                | "data"
                                | "sealed"
                                | "open"
                                | "abstract"
                                | "companion"
                                | "constructor"
                                | "package"
                                | "import"
                                | "override"
                                | "struct"
                                | "trait"
                                | "impl"
                                | "let"
                                | "const"
                                | "def"
                                | "fn"
                                | "new"
                                | "this"
                                | "super"
                                | "null"
                                | "true"
                                | "false"
                                | "void"
                                | "return"
                                | "if"
                                | "else"
                                | "for"
                                | "while"
                                | "do"
                                | "switch"
                                | "case"
                                | "break"
                                | "continue"
                                | "try"
                                | "catch"
                                | "finally"
                                | "throw"
                                | "by"
                                | "as"
                                | "in"
                                | "is"
                                | "default"
                                | "export"
                                | "declare"
                                | "async"
                                | "yield"
                                | "await"
                                | "function"
                                | "AllArgsConstructor"
                                | "NoArgsConstructor"
                                | "RequiredArgsConstructor"
                                | "Autowired"
                                | "EnableCaching"
                                | "EnableJpaRepositories"
                                | "SpringBootApplication"
                                | "Service"
                                | "Repository"
                                | "Component"
                                | "RestController"
                                | "RequestMapping"
                                | "Controller"
                                | "Getter"
                                | "Setter"
                                | "Slf4j"
                                | "Value"
                                | "Builder"
                                | "Data"
                        ) {
                            return tid;
                        }
                    }
                }
            }
            return u32::MAX;
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
