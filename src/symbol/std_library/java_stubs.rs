//! Pre-built Java Standard Library Stub Symbols (Object, String, Integer, etc.)
//! Loaded at Phase 3 initialization with symbol_ids >= 0xC0000000.

use crate::core::types::symbol::{SymbolKind, SymbolRecord, SymbolVisibility};
use crate::symbol::qual_name_table::QualifiedNameTable;
use std::collections::HashMap;

pub const BASE_STUB_SYMBOL_ID: u32 = 0xC000_0000;

pub const OBJECT_STUB_SYM_ID: u32 = BASE_STUB_SYMBOL_ID;
pub const STRING_STUB_SYM_ID: u32 = BASE_STUB_SYMBOL_ID + 1;
pub const INTEGER_STUB_SYM_ID: u32 = BASE_STUB_SYMBOL_ID + 2;
pub const BOOLEAN_STUB_SYM_ID: u32 = BASE_STUB_SYMBOL_ID + 3;
pub const LIST_STUB_SYM_ID: u32 = BASE_STUB_SYMBOL_ID + 4;
pub const MAP_STUB_SYM_ID: u32 = BASE_STUB_SYMBOL_ID + 5;
pub const SET_STUB_SYM_ID: u32 = BASE_STUB_SYMBOL_ID + 6;
pub const COLLECTION_STUB_SYM_ID: u32 = BASE_STUB_SYMBOL_ID + 7;
pub const ITERABLE_STUB_SYM_ID: u32 = BASE_STUB_SYMBOL_ID + 8;
pub const EXCEPTION_STUB_SYM_ID: u32 = BASE_STUB_SYMBOL_ID + 9;
pub const RUNTIME_EXCEPTION_STUB_SYM_ID: u32 = BASE_STUB_SYMBOL_ID + 10;
pub const THROWABLE_STUB_SYM_ID: u32 = BASE_STUB_SYMBOL_ID + 11;

#[derive(Debug, Clone)]
pub struct StdLibStub {
    pub symbol_id: u32,
    pub simple_name: &'static str,
    pub qual_name: &'static str,
    pub kind: SymbolKind,
}

pub fn get_java_stubs() -> Vec<StdLibStub> {
    vec![
        StdLibStub {
            symbol_id: OBJECT_STUB_SYM_ID,
            simple_name: "Object",
            qual_name: "java.lang.Object",
            kind: SymbolKind::SK_CLASS,
        },
        StdLibStub {
            symbol_id: STRING_STUB_SYM_ID,
            simple_name: "String",
            qual_name: "java.lang.String",
            kind: SymbolKind::SK_CLASS,
        },
        StdLibStub {
            symbol_id: INTEGER_STUB_SYM_ID,
            simple_name: "Integer",
            qual_name: "java.lang.Integer",
            kind: SymbolKind::SK_CLASS,
        },
        StdLibStub {
            symbol_id: BOOLEAN_STUB_SYM_ID,
            simple_name: "Boolean",
            qual_name: "java.lang.Boolean",
            kind: SymbolKind::SK_CLASS,
        },
        StdLibStub {
            symbol_id: LIST_STUB_SYM_ID,
            simple_name: "List",
            qual_name: "java.util.List",
            kind: SymbolKind::SK_INTERFACE,
        },
        StdLibStub {
            symbol_id: MAP_STUB_SYM_ID,
            simple_name: "Map",
            qual_name: "java.util.Map",
            kind: SymbolKind::SK_INTERFACE,
        },
        StdLibStub {
            symbol_id: SET_STUB_SYM_ID,
            simple_name: "Set",
            qual_name: "java.util.Set",
            kind: SymbolKind::SK_INTERFACE,
        },
        StdLibStub {
            symbol_id: COLLECTION_STUB_SYM_ID,
            simple_name: "Collection",
            qual_name: "java.util.Collection",
            kind: SymbolKind::SK_INTERFACE,
        },
        StdLibStub {
            symbol_id: ITERABLE_STUB_SYM_ID,
            simple_name: "Iterable",
            qual_name: "java.lang.Iterable",
            kind: SymbolKind::SK_INTERFACE,
        },
        StdLibStub {
            symbol_id: EXCEPTION_STUB_SYM_ID,
            simple_name: "Exception",
            qual_name: "java.lang.Exception",
            kind: SymbolKind::SK_CLASS,
        },
        StdLibStub {
            symbol_id: RUNTIME_EXCEPTION_STUB_SYM_ID,
            simple_name: "RuntimeException",
            qual_name: "java.lang.RuntimeException",
            kind: SymbolKind::SK_CLASS,
        },
        StdLibStub {
            symbol_id: THROWABLE_STUB_SYM_ID,
            simple_name: "Throwable",
            qual_name: "java.lang.Throwable",
            kind: SymbolKind::SK_CLASS,
        },
    ]
}

#[derive(Debug, Clone)]
pub struct StdLibManager {
    pub stubs: HashMap<String, u32>,
    pub stub_records: HashMap<u32, SymbolRecord>,
}

impl StdLibManager {
    pub fn new(qual_table: &mut QualifiedNameTable) -> Self {
        let mut stubs = HashMap::new();
        let mut stub_records = HashMap::new();

        for stub in get_java_stubs() {
            let qual_name_id = qual_table.get_or_intern(stub.qual_name);

            let record = SymbolRecord {
                symbol_id: stub.symbol_id,
                name_id: u32::MAX,
                qual_name_id,
                type_id: u32::MAX,
                decl_node: u32::MAX,
                def_node: u32::MAX,
                parent_sym: u32::MAX,
                first_child: u32::MAX,
                next_sibling: u32::MAX,
                scope_id: u32::MAX,
                uml_meta_offset: 0,
                param_count: 0,
                modifiers: 0,
                kind: stub.kind as u8,
                visibility: SymbolVisibility::Public as u8,
                type_param_count: 0,
                flags: 0,
                first_token_id: u32::MAX,
                last_token_id: u32::MAX,
                _reserved: 0,
            };

            stubs.insert(stub.simple_name.to_string(), stub.symbol_id);
            stubs.insert(stub.qual_name.to_string(), stub.symbol_id);
            stub_records.insert(stub.symbol_id, record);
        }

        Self {
            stubs,
            stub_records,
        }
    }

    pub fn lookup(&self, name: &str) -> Option<u32> {
        self.stubs.get(name).copied()
    }
}
