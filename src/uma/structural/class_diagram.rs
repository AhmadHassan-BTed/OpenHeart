//! ClassDiagramExtractor — translates STA symbols & TH into ClassRecord[] (§9.2.1).

use crate::core::types::symbol::{SymbolKind, SymbolModifiers};
use crate::ingestion::TokenCorpusArtifact;
use crate::psa::types::PathSummaryArtifact;
use crate::symbol::SymbolTableArtifact;
use crate::tra::types::{TraceabilityArtifact, UMLLinkRecord};
use crate::uma::types::*;

pub struct ClassDiagramExtractor;

impl ClassDiagramExtractor {
    pub fn extract(
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
        psa: &PathSummaryArtifact,
        tra: &TraceabilityArtifact,
    ) -> Vec<ClassRecord> {
        let mut classes = Vec::new();
        crate::core::logger::log_info(&format!(
            "[DIAG-CD] sta.symbol_count = {}",
            sta.symbol_count
        ));

        let mut children_map: std::collections::HashMap<u32, Vec<u32>> =
            std::collections::HashMap::new();
        for child_sym_id in 0..sta.symbol_count as u32 {
            if let Some(child) = sta.symbol(child_sym_id) {
                if child.parent_sym != u32::MAX {
                    children_map
                        .entry(child.parent_sym)
                        .or_default()
                        .push(child_sym_id);
                }
            }
        }

        for sym_id in 0..sta.symbol_count as u32 {
            let sym = match sta.symbol(sym_id) {
                Some(s) => s,
                None => continue,
            };
            let kind = SymbolKind::from(sym.kind);
            if matches!(
                kind,
                SymbolKind::SK_CLASS
                    | SymbolKind::SK_INTERFACE
                    | SymbolKind::SK_ENUM
                    | SymbolKind::SK_RECORD
            ) {
                let name_bytes = if sym.name_id != u32::MAX {
                    tca.interner.lookup_text(sym.name_id)
                } else {
                    b""
                };
                crate::core::logger::log_info(&format!(
                    "[DIAG-CD] Candidate Class symbol: sym_id={} name={} kind={:?}",
                    sym_id,
                    String::from_utf8_lossy(name_bytes),
                    kind
                ));
            }

            let mut parent_kind = SymbolKind::SK_MODULE;
            if sym.parent_sym != u32::MAX {
                if let Some(parent) = sta.symbol(sym.parent_sym) {
                    parent_kind = SymbolKind::from(parent.kind);
                    if parent_kind == SymbolKind::SK_METHOD
                        || parent_kind == SymbolKind::SK_FIELD
                        || parent_kind == SymbolKind::SK_PARAM
                        || parent_kind == SymbolKind::SK_LOCAL_VAR
                    {
                        if SymbolKind::from(sym.kind) != SymbolKind::SK_CLASS
                            && SymbolKind::from(sym.kind) != SymbolKind::SK_LAMBDA
                        {
                            continue;
                        }
                    }
                }
            }

            if !matches!(
                kind,
                SymbolKind::SK_CLASS
                    | SymbolKind::SK_INTERFACE
                    | SymbolKind::SK_ENUM
                    | SymbolKind::SK_RECORD
            ) {
                continue;
            }

            // ── Keyword Name Rejection: never create ClassRecords for symbols named with keywords ──
            let name_str = if sym.name_id != u32::MAX {
                let name_bytes = tca.interner.lookup_text(sym.name_id);
                std::str::from_utf8(name_bytes).unwrap_or("")
            } else {
                ""
            };

            // Enforce valid identifier syntax (alphanumeric + underscore only)
            let is_valid_ident = !name_str.is_empty()
                && name_str
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_');
            if !is_valid_ident {
                continue;
            }

            // Reject ALL-CAPS constants (e.g. ACTION_START_TRAINING, API_KEY, CHANNEL_ID, NOTIFICATION_ID, RUNNING) and trailing underscores
            let is_all_caps = name_str.len() >= 3
                && name_str.contains('_')
                && name_str
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c == '_' || c.is_ascii_digit());
            if is_all_caps {
                continue;
            }
            let ends_with_underscore = name_str.ends_with('_');

            let starts_uppercase = name_str
                .bytes()
                .next()
                .map_or(false, |b| b.is_ascii_uppercase());

            let mut is_test_file = false;
            let mut matches_file_stem = false;
            if sym.first_token_id != u32::MAX
                && (sym.first_token_id as usize) < tca.token_records.len()
            {
                let fid = crate::core::types::token::unpack_sort_key(
                    tca.token_records[sym.first_token_id as usize].sort_key,
                )
                .0;
                if (fid as usize) < tca.file_records.len() {
                    let path_id = tca.file_records[fid as usize].path_str_offset;
                    let path_bytes = tca.interner.lookup_text(path_id);
                    if let Ok(path_str) = std::str::from_utf8(path_bytes) {
                        let p_lower = path_str.to_lowercase();
                        if p_lower.contains("/src/test/")
                            || p_lower.contains("/src/tests/")
                            || p_lower.contains("/__tests__/")
                        {
                            is_test_file = true;
                        }
                        let filename = std::path::Path::new(path_str)
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or("");
                        if filename == name_str {
                            matches_file_stem = true;
                        }
                    }
                }
            }

            if is_test_file {
                continue;
            }

            if !starts_uppercase && !matches_file_stem {
                continue;
            }

            if ends_with_underscore
                || name_str.ends_with("Binding")
                || Self::is_reserved_keyword(name_str)
            {
                continue;
            }
            if matches!(
                kind,
                SymbolKind::SK_CLASS
                    | SymbolKind::SK_INTERFACE
                    | SymbolKind::SK_ENUM
                    | SymbolKind::SK_RECORD
            ) {
                crate::core::logger::log_debug(&format!(
                    "[DIAG] ClassCandidate sym_id={} name={} kind={:?} parent={}",
                    sym_id, name_str, kind, sym.parent_sym
                ));
            }

            let stereotype = match kind {
                SymbolKind::SK_INTERFACE => STEREOTYPE_INTERFACE,
                SymbolKind::SK_ENUM => STEREOTYPE_ENUM,
                SymbolKind::SK_RECORD => STEREOTYPE_RECORD,
                SymbolKind::SK_ANNOTATION_TYPE => STEREOTYPE_ANNOTATION,
                SymbolKind::SK_CLASS | SymbolKind::SK_LAMBDA => {
                    if (sym.modifiers & SymbolModifiers::ABSTRACT) != 0 {
                        STEREOTYPE_ABSTRACT
                    } else {
                        STEREOTYPE_NONE
                    }
                }
                _ => continue, // Only extract class-like type declarations
            };

            let mut fields = Vec::new();
            let mut methods = Vec::new();
            let mut inner_classes = Vec::new();

            // Collect all member symbols whose parent_sym == sym_id
            if let Some(child_syms) = children_map.get(&sym_id) {
                for &child_sym_id in child_syms {
                    let child = match sta.symbol(child_sym_id) {
                        Some(c) => c,
                        None => continue,
                    };
                    let child_kind = SymbolKind::from(child.kind);

                    match child_kind {
                        SymbolKind::SK_FIELD | SymbolKind::SK_ENUM_CONSTANT => {
                            fields.push(FieldRecord {
                                field_sym_id: child.symbol_id,
                                type_sym_id: child.type_id,
                                visibility: child.visibility,
                                modifiers: child.modifiers as u8,
                                is_collection: 0,
                                _pad: 0,
                                uml_link_node: child.decl_node,
                                _reserved: 0,
                            });
                        }
                        SymbolKind::SK_METHOD | SymbolKind::SK_CONSTRUCTOR => {
                            let (cyc, sat) = if let Some(hdr) = psa.function_header(child.symbol_id)
                            {
                                (hdr.cyclomatic, hdr.sat_count)
                            } else {
                                (1, 1)
                            };

                            methods.push(MethodRecord {
                                method_sym_id: child.symbol_id,
                                return_type_sym_id: child.type_id,
                                visibility: child.visibility,
                                modifiers: child.modifiers as u8,
                                param_count: child.param_count,
                                cyclomatic: cyc,
                                sat_count: sat,
                            });
                        }
                        SymbolKind::SK_CLASS
                        | SymbolKind::SK_INTERFACE
                        | SymbolKind::SK_ENUM
                        | SymbolKind::SK_RECORD => {
                            inner_classes.push(child.symbol_id);
                        }
                        _ => {}
                    }
                }
            }

            let mut association_set = std::collections::HashSet::new();
            let mut implements_set = std::collections::HashSet::new();

            for field in &fields {
                if field.type_sym_id != u32::MAX && field.type_sym_id != sym_id {
                    if let Some(target_sym) = sta.symbol(field.type_sym_id) {
                        let target_kind = SymbolKind::from(target_sym.kind);
                        if matches!(
                            target_kind,
                            SymbolKind::SK_CLASS | SymbolKind::SK_INTERFACE | SymbolKind::SK_ENUM
                        ) {
                            association_set.insert(field.type_sym_id);
                        }
                    }
                }
            }

            for method in &methods {
                if method.return_type_sym_id != u32::MAX && method.return_type_sym_id != sym_id {
                    if let Some(target_sym) = sta.symbol(method.return_type_sym_id) {
                        let target_kind = SymbolKind::from(target_sym.kind);
                        if matches!(
                            target_kind,
                            SymbolKind::SK_CLASS | SymbolKind::SK_INTERFACE | SymbolKind::SK_ENUM
                        ) {
                            association_set.insert(method.return_type_sym_id);
                        }
                    }
                }
            }

            let mut extends_sym = u32::MAX;

            for edge in &sta.th_edges {
                if edge.from_sym == sym_id {
                    match edge.relation {
                        crate::core::types::symbol::THRelation::TH_EXTENDS => {
                            extends_sym = edge.to_sym;
                        }
                        crate::core::types::symbol::THRelation::TH_IMPLEMENTS => {
                            implements_set.insert(edge.to_sym);
                        }
                        _ => {}
                    }
                }
            }

            // NOTE: sym.parent_sym is the AST containment parent (enclosing class/module),
            // NOT the superclass. Inheritance is handled exclusively through sta.th_edges above.

            let association_syms = association_set.into_iter().collect();
            let implements_syms = implements_set.into_iter().collect();

            // Find UMLLink for this class from TRA
            let uml_link = tra
                .uml_links
                .iter()
                .find(|link| link.sym_id == sym_id)
                .cloned()
                .unwrap_or(UMLLinkRecord {
                    sym_id,
                    file_id: 0,
                    line_start: 1,
                    col_start: 1,
                    line_end: 1,
                    col_end: 1,
                    scpg_hash: tra.hashes.scpg_hash,
                    sym_kind: sym.kind,
                    _reserved: [0; 3],
                });

            classes.push(ClassRecord {
                sym_id,
                stereotype,
                visibility: sym.visibility,
                modifiers: sym.modifiers,
                extends_sym,
                field_count: fields.len() as u16,
                method_count: methods.len() as u16,
                inner_count: inner_classes.len() as u16,
                design_pattern: PATTERN_NONE,
                _reserved: 0,
                type_param_count: sym.type_param_count,
                _pad: 0,
                uml_link,
                fields,
                methods,
                inner_classes,
                implements_syms,
                association_syms,
            });
        }

        classes
    }

    fn is_reserved_keyword(name: &str) -> bool {
        matches!(
            name,
            "class"
                | "interface"
                | "enum"
                | "object"
                | "struct"
                | "trait"
                | "impl"
                | "fun"
                | "function"
                | "def"
                | "fn"
                | "val"
                | "var"
                | "let"
                | "const"
                | "package"
                | "import"
                | "module"
                | "export"
                | "require"
                | "from"
                | "public"
                | "private"
                | "protected"
                | "internal"
                | "abstract"
                | "sealed"
                | "data"
                | "open"
                | "inner"
                | "companion"
                | "override"
                | "static"
                | "final"
                | "void"
                | "boolean"
                | "int"
                | "long"
                | "float"
                | "double"
                | "char"
                | "byte"
                | "short"
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
                | "this"
                | "super"
                | "self"
                | "Self"
                | "new"
                | "null"
                | "true"
                | "false"
                | "resolve"
                | "Unknown"
                | "SystemNode"
                | "OBJECT"
                | "CLASS"
                | "Entity"
                | "args"
                | "undefined"
                | "NaN"
                | "Composable"
                | "Deprecated"
                | "OptIn"
                | "Suppress"
                | "Retention"
                | "MustBeDocumented"
                | "Repeatable"
                | "JvmStatic"
                | "JvmOverloads"
                | "JvmField"
                | "JvmName"
                | "Inject"
                | "Singleton"
                | "Provides"
                | "Binds"
                | "HiltViewModel"
                | "PrimaryKey"
                | "ColumnInfo"
                | "Insert"
                | "Update"
                | "SerializedName"
                | "JsonProperty"
                | "AllArgsConstructor"
                | "NoArgsConstructor"
                | "RequiredArgsConstructor"
                | "Autowired"
                | "EnableCaching"
                | "EnableJpaRepositories"
                | "SpringBootApplication"
                | "RestController"
                | "RequestMapping"
                | "Getter"
                | "Setter"
                | "Slf4j"
                | "Value"
                | "EqualsAndHashCode"
                | "ToString"
                | "SneakyThrows"
                | "Synchronized"
                | "With"
                | "AccessLevel"
                | "DiscriminatorValue"
                | "EnableDiscoveryClient"
                | "EnableEurekaServer"
                | "BroadcastReceiver"
                | "Choreographer"
                | "Context"
                | "ContextCompat"
                | "Int"
                | "Intent"
                | "List"
                | "Log"
                | "Runnable"
                | "SeekBar"
                | "TAG"
                | "ViewModelProvider"
                | "ViewPager2"
                | "Arrays"
                | "BlockingQueue"
                | "CHANNEL_ID"
                | "NOTIFICATION_ID"
                | "Some"
                | "None"
                | "Ok"
                | "Err"
                | "OpenHeart"
                | "Configuration"
                | "Double"
                | "Float"
                | "Integer"
                | "Long"
                | "Boolean"
                | "Byte"
                | "Short"
                | "HashSet"
                | "HashMap"
                | "ArrayList"
                | "Optional"
                | "Objects"
                | "Collections"
                | "String"
                | "StringBuilder"
                | "StringBuffer"
                | "Math"
                | "System"
                | "Thread"
                | "Object"
                | "Class"
                | "Exception"
                | "RuntimeException"
                | "Throwable"
                | "Error"
                | "DOMContentLoaded"
                | "EnableScheduling"
                | "EntityScan"
                | "FeignClient"
                | "FunctionalInterface"
                | "HomeScreen"
                | "INVESTOR"
                | "JFrame"
                | "LOGGER"
                | "Logger"
                | "Override"
                | "Root"
                | "KafkaListener"
                | "MappedSuperclass"
                | "JsonProcessingException"
                | "SQLException"
                | "SpringBootConfiguration"
                | "SuppressWarnings"
                | "Table"
                | "Transactional"
                | "Scanner"
                | "constructor"
                | "default"
        )
    }
}
