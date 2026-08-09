//! Core Symbol Table, Scope Graph, and UML Data Structures for Phase 3.

/// Symbol Kind Alphabet (Σ_K)
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(non_camel_case_types)]
pub enum SymbolKind {
    SK_PACKAGE = 0x00,
    SK_CLASS = 0x01,
    SK_INTERFACE = 0x02,
    SK_ENUM = 0x03,
    SK_RECORD = 0x04,
    SK_ANNOTATION_TYPE = 0x05,
    SK_METHOD = 0x06,
    SK_CONSTRUCTOR = 0x07,
    SK_FIELD = 0x08,
    SK_ENUM_CONSTANT = 0x09,
    SK_PARAM = 0x0A,
    SK_LOCAL_VAR = 0x0B,
    SK_TYPE_PARAM = 0x0C,
    SK_LAMBDA = 0x0D,
    SK_ANON_CLASS = 0x0E,
    SK_STATIC_INIT = 0x0F,
    SK_INSTANCE_INIT = 0x10,
    SK_MODULE = 0x11,
    SK_EXTERNAL = 0x12,
}

impl From<u8> for SymbolKind {
    fn from(val: u8) -> Self {
        match val {
            0x00 => SymbolKind::SK_PACKAGE,
            0x01 => SymbolKind::SK_CLASS,
            0x02 => SymbolKind::SK_INTERFACE,
            0x03 => SymbolKind::SK_ENUM,
            0x04 => SymbolKind::SK_RECORD,
            0x05 => SymbolKind::SK_ANNOTATION_TYPE,
            0x06 => SymbolKind::SK_METHOD,
            0x07 => SymbolKind::SK_CONSTRUCTOR,
            0x08 => SymbolKind::SK_FIELD,
            0x09 => SymbolKind::SK_ENUM_CONSTANT,
            0x0A => SymbolKind::SK_PARAM,
            0x0B => SymbolKind::SK_LOCAL_VAR,
            0x0C => SymbolKind::SK_TYPE_PARAM,
            0x0D => SymbolKind::SK_LAMBDA,
            0x0E => SymbolKind::SK_ANON_CLASS,
            0x0F => SymbolKind::SK_STATIC_INIT,
            0x10 => SymbolKind::SK_INSTANCE_INIT,
            0x11 => SymbolKind::SK_MODULE,
            _ => SymbolKind::SK_EXTERNAL,
        }
    }
}

/// Modifiers bit flags (u16)
pub struct SymbolModifiers;
impl SymbolModifiers {
    pub const STATIC: u16 = 1 << 0;
    pub const FINAL: u16 = 1 << 1;
    pub const ABSTRACT: u16 = 1 << 2;
    pub const SYNCHRONIZED: u16 = 1 << 3;
    pub const NATIVE: u16 = 1 << 4;
    pub const VOLATILE: u16 = 1 << 5;
    pub const TRANSIENT: u16 = 1 << 6;
    pub const STRICTFP: u16 = 1 << 7;
    pub const DEFAULT: u16 = 1 << 8;
    pub const SEALED: u16 = 1 << 9;
    pub const NON_SEALED: u16 = 1 << 10;
    pub const VARARGS: u16 = 1 << 11;
    pub const BRIDGE: u16 = 1 << 12;
    pub const SYNTHETIC: u16 = 1 << 13;
    pub const DEPRECATED: u16 = 1 << 14;
    pub const RECORD_COMPONENT: u16 = 1 << 15;
}

/// Visibility specifier (u8)
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SymbolVisibility {
    Package = 0,
    Public = 1,
    Private = 2,
    Protected = 3,
}

impl From<u8> for SymbolVisibility {
    fn from(val: u8) -> Self {
        match val {
            1 => SymbolVisibility::Public,
            2 => SymbolVisibility::Private,
            3 => SymbolVisibility::Protected,
            _ => SymbolVisibility::Package,
        }
    }
}

/// 64-byte fixed, cache-aligned SymbolRecord representation
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SymbolRecord {
    pub symbol_id: u32,
    pub name_id: u32,
    pub qual_name_id: u32,
    pub type_id: u32,
    pub decl_node: u32,
    pub def_node: u32,
    pub parent_sym: u32,
    pub first_child: u32,
    pub next_sibling: u32,
    pub scope_id: u32,
    pub uml_meta_offset: u32,
    pub param_count: u16,
    pub modifiers: u16,
    pub kind: u8,
    pub visibility: u8,
    pub type_param_count: u8,
    pub flags: u8,
    pub first_token_id: u32,
    pub last_token_id: u32,
    pub _reserved: u32,
}

impl SymbolRecord {
    pub const UNINIT: Self = Self {
        symbol_id: u32::MAX,
        name_id: u32::MAX,
        qual_name_id: u32::MAX,
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
        kind: SymbolKind::SK_EXTERNAL as u8,
        visibility: SymbolVisibility::Package as u8,
        type_param_count: 0,
        flags: 0,
        first_token_id: u32::MAX,
        last_token_id: u32::MAX,
        _reserved: 0,
    };
}

/// Scope Kind
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScopeKind {
    File = 0,
    Class = 1,
    Method = 2,
    Block = 3,
    Lambda = 4,
    Anon = 5,
}

impl From<u8> for ScopeKind {
    fn from(val: u8) -> Self {
        match val {
            1 => ScopeKind::Class,
            2 => ScopeKind::Method,
            3 => ScopeKind::Block,
            4 => ScopeKind::Lambda,
            5 => ScopeKind::Anon,
            _ => ScopeKind::File,
        }
    }
}

/// 32-byte fixed ScopeRecord
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScopeRecord {
    pub scope_id: u32,
    pub parent_scope: u32,
    pub owner_symbol: u32,
    pub first_decl: u32,
    pub decl_count: u32,
    pub import_count: u16,
    pub scope_kind: u8,
    pub flags: u8,
    pub import_table_off: u32,
    pub _reserved: u32,
}

/// UML Association Kind
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AssocKind {
    None = 0,
    Dependency = 1,
    Association = 2,
    Aggregation = 3,
    Composition = 4,
}

impl From<u8> for AssocKind {
    fn from(val: u8) -> Self {
        match val {
            1 => AssocKind::Dependency,
            2 => AssocKind::Association,
            3 => AssocKind::Aggregation,
            4 => AssocKind::Composition,
            _ => AssocKind::None,
        }
    }
}

/// 28-byte fixed UMLAssociationRecord
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UMLAssociationRecord {
    pub from_symbol_id: u32,
    pub to_symbol_id: u32,
    pub field_symbol_id: u32,
    pub assoc_kind: u8,
    pub mult_min: u16,
    pub mult_max: u16,
    pub is_navigable: u8,
    pub role_name_id: u32,
    pub _reserved: u32,
    pub _padding: u16,
}

impl Default for UMLAssociationRecord {
    fn default() -> Self {
        Self {
            from_symbol_id: u32::MAX,
            to_symbol_id: u32::MAX,
            field_symbol_id: u32::MAX,
            assoc_kind: AssocKind::None as u8,
            mult_min: 0,
            mult_max: 1,
            is_navigable: 1,
            role_name_id: u32::MAX,
            _reserved: 0,
            _padding: 0,
        }
    }
}

/// Type Hierarchy Relation Type (Σ_TH)
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(non_camel_case_types)]
pub enum THRelation {
    TH_EXTENDS = 0,
    TH_IMPLEMENTS = 1,
    TH_USES = 2,
    TH_CREATES = 3,
}

impl From<u8> for THRelation {
    fn from(val: u8) -> Self {
        match val {
            1 => THRelation::TH_IMPLEMENTS,
            2 => THRelation::TH_USES,
            3 => THRelation::TH_CREATES,
            _ => THRelation::TH_EXTENDS,
        }
    }
}
