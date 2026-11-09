//! SymbolTableSerializer and SymbolTableArtifact binary serialization (.sta format, version 2).
//!
//! Layout (all integers little-endian):
//!  1. Header (64 B): magic, version, symbol/scope/TH-edge/association/qual-name counts,
//!     BPA and TCA hashes, then type-ref, occurrence and diagnostic counts and a flags word.
//!  2. SymbolRecord[]        (64 B each)
//!  3. Name index            (name_id, symbol_id) sorted by (name_id, symbol_id)
//!  4. ScopeRecord[]         (32 B each)
//!  5. Type hierarchy as CSR: `symbol_count + 1` row offsets, then (to, relation, flags) entries
//!     grouped by source symbol and sorted by (to, relation)
//!  6. UMLAssociationRecord[] (28 B each)
//!  7. Qualified names (canonical names and stable descriptors)
//!  8. Package names by package symbol, sorted
//!  9. Package names by file id, sorted
//! 10. Import table, ordered by compilation-unit scope (ScopeRecord::import_table_off indexes it)
//! 11. TypeRef table
//! 12. Declared TypeRef of every symbol
//! 13. Supertype records (written supertypes with their type arguments; type-parameter bounds)
//! 14. Type-reference bindings by AST node
//! 15. Occurrences, sorted by token
//! 16. Stable descriptor of every symbol (qualified-name id)
//! 17. Bidirectional association pairs
//! 18. Diagnostics
//! 19. CRC-64 over everything above
//!
//! Every map is written in sorted order, so two runs over the same input produce identical
//! bytes. The reader is bounds-checked: counts are validated against the remaining input
//! before anything is allocated, and truncated or malformed input yields `InvalidData`.

use crate::core::types::symbol::{ScopeRecord, SymbolKind, SymbolRecord, THRelation, UMLAssociationRecord};
use crate::ingestion::serializer::crc64_ecma;
use crate::symbol::builder::{SymbolTableBuilder, TypeHierarchyEdge};
use crate::symbol::model::{
    Diagnostic, DiagnosticCode, Occurrence, OccurrenceRole, ResolutionStatus, SupertypeRecord,
    TypeRef,
};
use crate::symbol::qual_name_table::QualifiedNameTable;
use crate::symbol::scope_graph::{ImportDecl, ImportKind};
use std::collections::{BTreeMap, HashMap};
use std::io::{Error, ErrorKind, Result};

pub const STA_MAGIC: u64 = 0x53544100_01000000; // "STA\x00\x01\x00\x00\x00"
pub const STA_FORMAT_VERSION: u32 = 2;

const HEADER_SIZE: usize = 64;
const SYMBOL_RECORD_SIZE: usize = 64;
const SCOPE_RECORD_SIZE: usize = 32;
const ASSOCIATION_RECORD_SIZE: usize = 28;

#[derive(Debug, Clone)]
pub struct SymbolTableArtifact {
    pub magic: u64,
    pub format_version: u32,
    pub symbol_count: u32,
    pub scope_count: u32,
    pub th_edge_count: u32,
    pub assoc_count: u32,
    pub qual_name_count: u32,
    pub bpa_hash: u64,
    pub tca_hash: u64,

    pub symbol_records: Vec<SymbolRecord>,
    pub name_index: Vec<(u32, u32)>, // (name_id, symbol_id)
    pub scope_records: Vec<ScopeRecord>,
    /// Hierarchy edges sorted by (from, to, relation).
    pub th_edges: Vec<TypeHierarchyEdge>,
    pub associations: Vec<UMLAssociationRecord>,
    pub qual_names: Vec<String>,
    /// Package name of every package symbol (written sorted by symbol id).
    pub custom_package_names: HashMap<u32, String>,
    /// Package name of every file with a package declaration (written sorted by file id).
    pub file_package_names: HashMap<u16, String>,
    /// Import declarations as (compilation-unit scope, import), ordered by scope.
    pub imports: Vec<(u32, ImportDecl)>,
    pub type_refs: Vec<TypeRef>,
    /// Declared TypeRef id of every symbol (`u32::MAX` when it has none).
    pub symbol_type_refs: Vec<u32>,
    pub supertypes: Vec<SupertypeRecord>,
    /// Binding of every type-reference AST node: (node, symbol or primitive id, status).
    pub type_ref_bindings: Vec<(u32, u32, u8)>,
    /// Occurrences sorted by token.
    pub occurrences: Vec<Occurrence>,
    /// Stable descriptor (qualified-name id) of every symbol.
    pub descriptors: Vec<u32>,
    pub association_pairs: Vec<(u32, u32)>,
    pub diagnostics: Vec<Diagnostic>,
    pub crc64_checksum: u64,
}

impl SymbolTableArtifact {
    pub fn symbol(&self, sym_id: u32) -> Option<&SymbolRecord> {
        self.symbol_records.get(sym_id as usize)
    }

    fn qual_name(&self, id: u32) -> Option<&str> {
        if id < QualifiedNameTable::BASE_QUAL_NAME_ID {
            return None;
        }
        self.qual_names
            .get((id - QualifiedNameTable::BASE_QUAL_NAME_ID) as usize)
            .map(|s| s.as_str())
    }

    /// Canonical qualified name of a type, package or external symbol.
    pub fn qualified_name_of(&self, sym_id: u32) -> Option<&str> {
        self.symbol(sym_id).and_then(|s| self.qual_name(s.qual_name_id))
    }

    /// Stable, position-independent descriptor of a symbol (SemanticDB grammar).
    pub fn descriptor_of(&self, sym_id: u32) -> Option<&str> {
        self.descriptors
            .get(sym_id as usize)
            .and_then(|&d| self.qual_name(d))
    }

    pub fn type_ref(&self, id: u32) -> Option<&TypeRef> {
        self.type_refs.get(id as usize)
    }

    /// Declared type of a symbol with its type arguments and array dimensions.
    pub fn type_ref_of(&self, sym_id: u32) -> Option<&TypeRef> {
        let id = *self.symbol_type_refs.get(sym_id as usize)?;
        self.type_ref(id)
    }

    /// True for declarations of the analysed project that are types.
    pub fn is_project_type(&self, sym_id: u32) -> bool {
        matches!(
            self.symbol(sym_id).map(|s| SymbolKind::from(s.kind)),
            Some(
                SymbolKind::SK_CLASS
                    | SymbolKind::SK_INTERFACE
                    | SymbolKind::SK_ENUM
                    | SymbolKind::SK_RECORD
                    | SymbolKind::SK_ANNOTATION_TYPE
                    | SymbolKind::SK_ANON_CLASS
            )
        )
    }

    /// Direct supertypes of every type, restricted to EXTENDS/IMPLEMENTS edges.
    pub fn supertype_index(&self) -> HashMap<u32, Vec<u32>> {
        let mut index: HashMap<u32, Vec<u32>> = HashMap::new();
        for e in &self.th_edges {
            if matches!(e.relation, THRelation::TH_EXTENDS | THRelation::TH_IMPLEMENTS) {
                index.entry(e.from_sym).or_default().push(e.to_sym);
            }
        }
        index
    }

    /// Occurrences recorded at `token`.
    pub fn occurrences_at_token(&self, token: u32) -> &[Occurrence] {
        let start = self.occurrences.partition_point(|o| o.token < token);
        let end = self.occurrences.partition_point(|o| o.token <= token);
        &self.occurrences[start..end]
    }

    /// Occurrence of the given role recorded for an AST node.
    pub fn occurrence_at_node(&self, node: u32, role: OccurrenceRole) -> Option<&Occurrence> {
        self.occurrences
            .iter()
            .find(|o| o.node == node && o.role == role as u8)
    }

    /// Method or constructor selected for a call or instance creation node, when Phase 3
    /// could select exactly one.
    pub fn call_target(&self, call_node: u32) -> Option<u32> {
        self.occurrences
            .iter()
            .find(|o| {
                o.node == call_node
                    && (o.role == OccurrenceRole::Call as u8
                        || o.role == OccurrenceRole::Instantiation as u8)
            })
            .map(|o| o.symbol)
            .filter(|&s| s != u32::MAX && (s as usize) < self.symbol_records.len())
    }

    /// Binding of a type-reference node.
    pub fn type_ref_binding(&self, node: u32) -> Option<(u32, ResolutionStatus)> {
        let i = self
            .type_ref_bindings
            .binary_search_by_key(&node, |&(n, _, _)| n)
            .ok()?;
        let (_, sym, status) = self.type_ref_bindings[i];
        Some((sym, ResolutionStatus::from_u8(status)?))
    }

    pub fn build(builder: &SymbolTableBuilder, bpa_bytes: &[u8], tca_bytes: &[u8]) -> Self {
        Self::from_builder(builder, crc64_ecma(bpa_bytes), crc64_ecma(tca_bytes))
    }

    pub fn from_builder(builder: &SymbolTableBuilder, bpa_hash: u64, tca_hash: u64) -> Self {
        let symbol_count = builder.symbols.len() as u32;

        let mut name_index: Vec<(u32, u32)> = builder
            .symbols
            .iter()
            .map(|s| (s.name_id, s.symbol_id))
            .collect();
        name_index.sort_unstable();

        let mut th_edges: Vec<TypeHierarchyEdge> = builder
            .th_edges
            .iter()
            .filter(|e| e.from_sym < symbol_count && e.to_sym < symbol_count)
            .cloned()
            .collect();
        th_edges.sort_by_key(|e| (e.from_sym, e.to_sym, e.relation as u8));

        let imports: Vec<(u32, ImportDecl)> = builder
            .scope_graph
            .import_tables()
            .iter()
            .flat_map(|(&scope, list)| list.iter().map(move |i| (scope, i.clone())))
            .collect();

        let mut occurrences = builder.occurrences.clone();
        occurrences.sort_by(|a, b| (a.token, a.role, a.node).cmp(&(b.token, b.role, b.node)));

        Self {
            magic: STA_MAGIC,
            format_version: STA_FORMAT_VERSION,
            symbol_count,
            scope_count: builder.scope_graph.scope_count() as u32,
            th_edge_count: th_edges.len() as u32,
            assoc_count: builder.associations.len() as u32,
            qual_name_count: builder.qual_names.len() as u32,
            bpa_hash,
            tca_hash,
            symbol_records: builder.symbols.clone(),
            name_index,
            scope_records: builder.scope_graph.scopes.clone(),
            th_edges,
            associations: builder.associations.clone(),
            qual_names: builder.qual_names.all_names().to_vec(),
            custom_package_names: builder
                .custom_package_names
                .iter()
                .map(|(&k, v)| (k, v.clone()))
                .collect(),
            file_package_names: builder
                .file_package_names
                .iter()
                .map(|(&k, v)| (k, v.clone()))
                .collect(),
            imports,
            type_refs: builder.type_refs.clone(),
            symbol_type_refs: builder.symbol_type_refs.clone(),
            supertypes: builder.supertypes.clone(),
            type_ref_bindings: builder.type_ref_bindings(),
            occurrences,
            descriptors: builder.descriptors.clone(),
            association_pairs: builder.association_pairs.clone(),
            diagnostics: builder.diagnostics.clone(),
            crc64_checksum: 0,
        }
    }

    /// Serializes the artifact. Panics only if a section exceeds the u32 length fields of the
    /// format, which cannot happen for tables indexed by u32 ids.
    pub fn serialize(&self) -> Vec<u8> {
        match self.try_serialize() {
            Ok(bytes) => bytes,
            Err(e) => panic!("STA serialization failed: {}", e),
        }
    }

    pub fn try_serialize(&self) -> Result<Vec<u8>> {
        let mut w = ByteWriter::default();

        // 1. Header (64 bytes).
        w.u64(self.magic);
        w.u32(self.format_version);
        w.u32(len_u32(self.symbol_records.len())?);
        w.u32(len_u32(self.scope_records.len())?);
        w.u32(len_u32(self.th_edges.len())?);
        w.u32(len_u32(self.associations.len())?);
        w.u32(len_u32(self.qual_names.len())?);
        w.u64(self.bpa_hash);
        w.u64(self.tca_hash);
        w.u32(len_u32(self.type_refs.len())?);
        w.u32(len_u32(self.occurrences.len())?);
        w.u32(len_u32(self.diagnostics.len())?);
        w.u32(0); // flags

        // 2. Symbols.
        for sym in &self.symbol_records {
            w.u32(sym.symbol_id);
            w.u32(sym.name_id);
            w.u32(sym.qual_name_id);
            w.u32(sym.type_id);
            w.u32(sym.decl_node);
            w.u32(sym.def_node);
            w.u32(sym.parent_sym);
            w.u32(sym.first_child);
            w.u32(sym.next_sibling);
            w.u32(sym.scope_id);
            w.u32(sym.uml_meta_offset);
            w.u16(sym.param_count);
            w.u16(sym.modifiers);
            w.u8(sym.kind);
            w.u8(sym.visibility);
            w.u8(sym.type_param_count);
            w.u8(sym.flags);
            w.u32(sym.first_token_id);
            w.u32(sym.last_token_id);
            w.u32(sym._reserved);
        }

        // 3. Name index.
        let mut name_index = self.name_index.clone();
        name_index.sort_unstable();
        for &(name_id, sym_id) in &name_index {
            w.u32(name_id);
            w.u32(sym_id);
        }

        // 4. Scopes.
        for sc in &self.scope_records {
            w.u32(sc.scope_id);
            w.u32(sc.parent_scope);
            w.u32(sc.owner_symbol);
            w.u32(sc.first_decl);
            w.u32(sc.decl_count);
            w.u16(sc.import_count);
            w.u8(sc.scope_kind);
            w.u8(sc.flags);
            w.u32(sc.import_table_off);
            w.u32(sc._reserved);
        }

        // 5. Type hierarchy (CSR).
        let n = self.symbol_records.len();
        let mut edges = self.th_edges.clone();
        edges.sort_by_key(|e| (e.from_sym, e.to_sym, e.relation as u8));
        let mut offsets = vec![0u32; n + 1];
        for e in &edges {
            if e.from_sym as usize >= n {
                return Err(invalid("hierarchy edge source outside the symbol table"));
            }
            offsets[e.from_sym as usize + 1] += 1;
        }
        for i in 0..n {
            offsets[i + 1] += offsets[i];
        }
        for off in &offsets {
            w.u32(*off);
        }
        for e in &edges {
            w.u32(e.to_sym);
            w.u8(e.relation as u8);
            w.u8(e.flags);
        }

        // 6. Associations.
        for assoc in &self.associations {
            w.u32(assoc.from_symbol_id);
            w.u32(assoc.to_symbol_id);
            w.u32(assoc.field_symbol_id);
            w.u8(assoc.assoc_kind);
            w.u16(assoc.mult_min);
            w.u16(assoc.mult_max);
            w.u8(assoc.is_navigable);
            w.u32(assoc.role_name_id);
            w.u32(assoc._reserved);
            w.u16(assoc._padding);
        }

        // 7. Qualified names.
        for name in &self.qual_names {
            w.string(name)?;
        }

        // 8./9. Package maps, sorted by key so the bytes do not depend on hash order.
        let custom: BTreeMap<u32, &String> = self
            .custom_package_names
            .iter()
            .map(|(&k, v)| (k, v))
            .collect();
        w.u32(len_u32(custom.len())?);
        for (sym_id, name) in custom {
            w.u32(sym_id);
            w.string(name)?;
        }
        let files: BTreeMap<u16, &String> = self
            .file_package_names
            .iter()
            .map(|(&k, v)| (k, v))
            .collect();
        w.u32(len_u32(files.len())?);
        for (fid, name) in files {
            w.u16(fid);
            w.string(name)?;
        }

        // 10. Imports.
        w.u32(len_u32(self.imports.len())?);
        for (scope, imp) in &self.imports {
            w.u32(*scope);
            w.u8(imp.kind as u8);
            w.u32(imp.first_token);
            w.u32(imp.name_token);
            w.string(&imp.path)?;
            w.string(&imp.visible_name)?;
        }

        // 11. TypeRefs.
        for tr in &self.type_refs {
            w.u32(tr.head);
            w.u32(tr.written_name_id);
            w.u8(tr.array_dims);
            w.u8(tr.wildcard);
            w.u8(tr.flags);
            w.u8(tr.status);
            w.u32(tr.first_token);
            w.u32(tr.last_token);
            w.u32(len_u32(tr.args.len())?);
            for &a in &tr.args {
                w.u32(a);
            }
        }

        // 12. Declared TypeRef per symbol.
        for i in 0..n {
            w.u32(self.symbol_type_refs.get(i).copied().unwrap_or(u32::MAX));
        }

        // 13. Supertype records.
        w.u32(len_u32(self.supertypes.len())?);
        for s in &self.supertypes {
            w.u32(s.sub);
            w.u32(s.sup);
            w.u8(s.relation as u8);
            w.u32(s.type_ref);
        }

        // 14. Type-reference bindings.
        let mut bindings = self.type_ref_bindings.clone();
        bindings.sort_unstable();
        w.u32(len_u32(bindings.len())?);
        for (node, sym, status) in bindings {
            w.u32(node);
            w.u32(sym);
            w.u8(status);
        }

        // 15. Occurrences.
        for o in &self.occurrences {
            w.u32(o.token);
            w.u32(o.node);
            w.u32(o.symbol);
            w.u8(o.role);
            w.u8(o.status);
            w.u32(len_u32(o.candidates.len())?);
            for &c in &o.candidates {
                w.u32(c);
            }
        }

        // 16. Descriptors.
        for i in 0..n {
            w.u32(self.descriptors.get(i).copied().unwrap_or(u32::MAX));
        }

        // 17. Association pairs.
        w.u32(len_u32(self.association_pairs.len())?);
        for &(a, b) in &self.association_pairs {
            w.u32(a);
            w.u32(b);
        }

        // 18. Diagnostics.
        for d in &self.diagnostics {
            w.u8(d.code as u8);
            w.u32(d.symbol);
            w.u32(d.token);
            w.u32(len_u32(d.related.len())?);
            for &r in &d.related {
                w.u32(r);
            }
            w.string(&d.message)?;
        }

        // 19. CRC-64 over all payload bytes.
        let checksum = crc64_ecma(&w.buf);
        w.u64(checksum);
        Ok(w.buf)
    }

    pub fn deserialize(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < HEADER_SIZE + 8 {
            return Err(invalid("file too small for STA artifact"));
        }
        let payload_len = bytes.len() - 8;
        let payload = &bytes[..payload_len];
        let mut crc_cursor = ByteCursor::new(&bytes[payload_len..]);
        let expected_checksum = crc_cursor.u64()?;
        let actual_checksum = crc64_ecma(payload);
        if expected_checksum != actual_checksum {
            return Err(Error::new(
                ErrorKind::InvalidData,
                format!(
                    "STA checksum mismatch: expected {:#x}, got {:#x}",
                    expected_checksum, actual_checksum
                ),
            ));
        }

        let mut r = ByteCursor::new(payload);
        let magic = r.u64()?;
        if magic != STA_MAGIC {
            return Err(Error::new(
                ErrorKind::InvalidData,
                format!("Invalid STA magic: {:#x}", magic),
            ));
        }
        let format_version = r.u32()?;
        if format_version != STA_FORMAT_VERSION {
            return Err(Error::new(
                ErrorKind::InvalidData,
                format!(
                    "unsupported STA format version {} (this reader supports {})",
                    format_version, STA_FORMAT_VERSION
                ),
            ));
        }
        let symbol_count = r.u32()?;
        let scope_count = r.u32()?;
        let th_edge_count = r.u32()?;
        let assoc_count = r.u32()?;
        let qual_name_count = r.u32()?;
        let bpa_hash = r.u64()?;
        let tca_hash = r.u64()?;
        let type_ref_count = r.u32()?;
        let occurrence_count = r.u32()?;
        let diagnostic_count = r.u32()?;
        let _flags = r.u32()?;

        let n = r.bounded(symbol_count, SYMBOL_RECORD_SIZE, "symbol")?;
        let mut symbol_records = Vec::with_capacity(n);
        for _ in 0..n {
            symbol_records.push(SymbolRecord {
                symbol_id: r.u32()?,
                name_id: r.u32()?,
                qual_name_id: r.u32()?,
                type_id: r.u32()?,
                decl_node: r.u32()?,
                def_node: r.u32()?,
                parent_sym: r.u32()?,
                first_child: r.u32()?,
                next_sibling: r.u32()?,
                scope_id: r.u32()?,
                uml_meta_offset: r.u32()?,
                param_count: r.u16()?,
                modifiers: r.u16()?,
                kind: r.u8()?,
                visibility: r.u8()?,
                type_param_count: r.u8()?,
                flags: r.u8()?,
                first_token_id: r.u32()?,
                last_token_id: r.u32()?,
                _reserved: r.u32()?,
            });
        }

        r.bounded(symbol_count, 8, "name index")?;
        let mut name_index = Vec::with_capacity(n);
        for _ in 0..n {
            let name_id = r.u32()?;
            let sym_id = r.u32()?;
            name_index.push((name_id, sym_id));
        }

        let sc = r.bounded(scope_count, SCOPE_RECORD_SIZE, "scope")?;
        let mut scope_records = Vec::with_capacity(sc);
        for _ in 0..sc {
            scope_records.push(ScopeRecord {
                scope_id: r.u32()?,
                parent_scope: r.u32()?,
                owner_symbol: r.u32()?,
                first_decl: r.u32()?,
                decl_count: r.u32()?,
                import_count: r.u16()?,
                scope_kind: r.u8()?,
                flags: r.u8()?,
                import_table_off: r.u32()?,
                _reserved: r.u32()?,
            });
        }

        // CSR hierarchy.
        let rows = n.checked_add(1).ok_or_else(|| invalid("symbol count overflow"))?;
        r.require(rows.checked_mul(4).ok_or_else(|| invalid("CSR size overflow"))?, "CSR offsets")?;
        let mut offsets = Vec::with_capacity(rows);
        for _ in 0..rows {
            offsets.push(r.u32()?);
        }
        let ec = r.bounded(th_edge_count, 6, "hierarchy edge")?;
        if offsets[0] != 0
            || offsets[rows - 1] as usize != ec
            || offsets.windows(2).any(|w| w[0] > w[1])
        {
            return Err(invalid("inconsistent hierarchy CSR offsets"));
        }
        let mut th_edges = Vec::with_capacity(ec);
        for from in 0..n {
            for _ in offsets[from]..offsets[from + 1] {
                let to_sym = r.u32()?;
                let relation = relation_from_u8(r.u8()?)?;
                let flags = r.u8()?;
                th_edges.push(TypeHierarchyEdge {
                    from_sym: from as u32,
                    to_sym,
                    relation,
                    flags,
                });
            }
        }

        let ac = r.bounded(assoc_count, ASSOCIATION_RECORD_SIZE, "association")?;
        let mut associations = Vec::with_capacity(ac);
        for _ in 0..ac {
            associations.push(UMLAssociationRecord {
                from_symbol_id: r.u32()?,
                to_symbol_id: r.u32()?,
                field_symbol_id: r.u32()?,
                assoc_kind: r.u8()?,
                mult_min: r.u16()?,
                mult_max: r.u16()?,
                is_navigable: r.u8()?,
                role_name_id: r.u32()?,
                _reserved: r.u32()?,
                _padding: r.u16()?,
            });
        }

        let qc = r.bounded(qual_name_count, 4, "qualified name")?;
        let mut qual_names = Vec::with_capacity(qc);
        for _ in 0..qc {
            qual_names.push(r.string()?);
        }

        let cc = r.count(8, "package name")?;
        let mut custom_package_names = HashMap::with_capacity(cc);
        for _ in 0..cc {
            let sym_id = r.u32()?;
            custom_package_names.insert(sym_id, r.string()?);
        }
        let fc = r.count(6, "file package name")?;
        let mut file_package_names = HashMap::with_capacity(fc);
        for _ in 0..fc {
            let fid = r.u16()?;
            file_package_names.insert(fid, r.string()?);
        }

        let ic = r.count(21, "import")?;
        let mut imports = Vec::with_capacity(ic);
        for _ in 0..ic {
            let scope = r.u32()?;
            let kind = ImportKind::from_u8(r.u8()?).ok_or_else(|| invalid("unknown import kind"))?;
            let first_token = r.u32()?;
            let name_token = r.u32()?;
            let path = r.string()?;
            let visible_name = r.string()?;
            imports.push((
                scope,
                ImportDecl {
                    kind,
                    path,
                    visible_name,
                    first_token,
                    name_token,
                },
            ));
        }

        let tc = r.bounded(type_ref_count, 24, "type reference")?;
        let mut type_refs = Vec::with_capacity(tc);
        for _ in 0..tc {
            let head = r.u32()?;
            let written_name_id = r.u32()?;
            let array_dims = r.u8()?;
            let wildcard = r.u8()?;
            let flags = r.u8()?;
            let status = r.u8()?;
            let first_token = r.u32()?;
            let last_token = r.u32()?;
            let argc = r.count(4, "type argument")?;
            let mut args = Vec::with_capacity(argc);
            for _ in 0..argc {
                args.push(r.u32()?);
            }
            type_refs.push(TypeRef {
                head,
                written_name_id,
                args,
                array_dims,
                wildcard,
                flags,
                status,
                first_token,
                last_token,
            });
        }

        r.require(n.checked_mul(4).ok_or_else(|| invalid("type ref table overflow"))?, "declared types")?;
        let mut symbol_type_refs = Vec::with_capacity(n);
        for _ in 0..n {
            symbol_type_refs.push(r.u32()?);
        }

        let stc = r.count(13, "supertype record")?;
        let mut supertypes = Vec::with_capacity(stc);
        for _ in 0..stc {
            let sub = r.u32()?;
            let sup = r.u32()?;
            let relation = relation_from_u8(r.u8()?)?;
            let type_ref = r.u32()?;
            supertypes.push(SupertypeRecord {
                sub,
                sup,
                relation,
                type_ref,
            });
        }

        let bc = r.count(9, "type reference binding")?;
        let mut type_ref_bindings = Vec::with_capacity(bc);
        for _ in 0..bc {
            let node = r.u32()?;
            let sym = r.u32()?;
            let status = r.u8()?;
            type_ref_bindings.push((node, sym, status));
        }

        let oc = r.bounded(occurrence_count, 18, "occurrence")?;
        let mut occurrences = Vec::with_capacity(oc);
        for _ in 0..oc {
            let token = r.u32()?;
            let node = r.u32()?;
            let symbol = r.u32()?;
            let role = r.u8()?;
            let status = r.u8()?;
            let candc = r.count(4, "candidate")?;
            let mut candidates = Vec::with_capacity(candc);
            for _ in 0..candc {
                candidates.push(r.u32()?);
            }
            occurrences.push(Occurrence {
                token,
                node,
                symbol,
                role,
                status,
                candidates,
            });
        }

        r.require(n.checked_mul(4).ok_or_else(|| invalid("descriptor table overflow"))?, "descriptors")?;
        let mut descriptors = Vec::with_capacity(n);
        for _ in 0..n {
            descriptors.push(r.u32()?);
        }

        let pc = r.count(8, "association pair")?;
        let mut association_pairs = Vec::with_capacity(pc);
        for _ in 0..pc {
            let a = r.u32()?;
            let b = r.u32()?;
            association_pairs.push((a, b));
        }

        let dc = r.bounded(diagnostic_count, 17, "diagnostic")?;
        let mut diagnostics = Vec::with_capacity(dc);
        for _ in 0..dc {
            let code = DiagnosticCode::from_u8(r.u8()?)
                .ok_or_else(|| invalid("unknown diagnostic code"))?;
            let symbol = r.u32()?;
            let token = r.u32()?;
            let relc = r.count(4, "related symbol")?;
            let mut related = Vec::with_capacity(relc);
            for _ in 0..relc {
                related.push(r.u32()?);
            }
            let message = r.string()?;
            diagnostics.push(Diagnostic {
                code,
                symbol,
                token,
                related,
                message,
            });
        }

        if r.remaining() != 0 {
            return Err(invalid("trailing bytes after the last STA section"));
        }

        Ok(Self {
            magic,
            format_version,
            symbol_count,
            scope_count,
            th_edge_count,
            assoc_count,
            qual_name_count,
            bpa_hash,
            tca_hash,
            symbol_records,
            name_index,
            scope_records,
            th_edges,
            associations,
            qual_names,
            custom_package_names,
            file_package_names,
            imports,
            type_refs,
            symbol_type_refs,
            supertypes,
            type_ref_bindings,
            occurrences,
            descriptors,
            association_pairs,
            diagnostics,
            crc64_checksum: expected_checksum,
        })
    }
}

fn invalid(msg: &str) -> Error {
    Error::new(ErrorKind::InvalidData, format!("STA: {}", msg))
}

fn len_u32(len: usize) -> Result<u32> {
    u32::try_from(len).map_err(|_| invalid("section length exceeds u32"))
}

fn relation_from_u8(val: u8) -> Result<THRelation> {
    match val {
        0..=3 => Ok(THRelation::from(val)),
        _ => Err(invalid("unknown hierarchy relation")),
    }
}

#[derive(Default)]
struct ByteWriter {
    buf: Vec<u8>,
}

impl ByteWriter {
    fn u8(&mut self, v: u8) {
        self.buf.push(v);
    }
    fn u16(&mut self, v: u16) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    fn string(&mut self, s: &str) -> Result<()> {
        self.u32(len_u32(s.len())?);
        self.buf.extend_from_slice(s.as_bytes());
        Ok(())
    }
}

/// Bounds-checked little-endian reader over a byte slice.
struct ByteCursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> ByteCursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    fn remaining(&self) -> usize {
        self.bytes.len() - self.pos
    }

    fn require(&self, n: usize, what: &str) -> Result<()> {
        if self.remaining() < n {
            return Err(invalid(&format!("truncated {} section", what)));
        }
        Ok(())
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        self.require(n, "data")?;
        let s = &self.bytes[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16> {
        let s = self.take(2)?;
        Ok(u16::from_le_bytes([s[0], s[1]]))
    }

    fn u32(&mut self) -> Result<u32> {
        let s = self.take(4)?;
        Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }

    fn u64(&mut self) -> Result<u64> {
        let s = self.take(8)?;
        let mut a = [0u8; 8];
        a.copy_from_slice(s);
        Ok(u64::from_le_bytes(a))
    }

    /// Validates a header count against the remaining input, given the minimum encoded size
    /// of one record, before anything is allocated for it.
    fn bounded(&self, count: u32, min_record: usize, what: &str) -> Result<usize> {
        let count = count as usize;
        let needed = count
            .checked_mul(min_record)
            .ok_or_else(|| invalid(&format!("{} count overflow", what)))?;
        self.require(needed, what)?;
        Ok(count)
    }

    /// Reads an inline count and validates it like `bounded`.
    fn count(&mut self, min_record: usize, what: &str) -> Result<usize> {
        let c = self.u32()?;
        self.bounded(c, min_record, what)
    }

    fn string(&mut self) -> Result<String> {
        let len = self.u32()? as usize;
        let bytes = self.take(len)?;
        String::from_utf8(bytes.to_vec()).map_err(|e| invalid(&format!("invalid UTF-8: {}", e)))
    }
}
