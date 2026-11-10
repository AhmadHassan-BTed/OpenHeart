//! SymbolTableBuilder: accumulates symbols, scopes, the TypeRef table, the type hierarchy, UML
//! associations, occurrences and diagnostics for Phase 3.
//!
//! Containment is recorded only through `parent_sym`; the `first_child`/`next_sibling` chains
//! are derived from it in one pass (`rebuild_child_chains`), so they are acyclic and agree with
//! `parent_sym` by construction. Hierarchy cycles are diagnosed, never repaired by deleting
//! edges, and `verify_invariants` only inspects the table.

use crate::ast::BPASTArtifact;
use crate::core::types::ast::ASTNodeType;
use crate::core::types::symbol::{
    ScopeKind, SymbolKind, SymbolModifiers, SymbolRecord, SymbolVisibility, THRelation,
    UMLAssociationRecord,
};
use crate::core::types::token::unpack_sort_key;
use crate::ingestion::TokenCorpusArtifact;
use crate::symbol::model::{
    Diagnostic, DiagnosticCode, Dialect, EdgeFlags, FrontEnd, Occurrence, ResolutionStatus,
    SupertypeRecord, SymbolFlags, TypeRef,
};
use crate::symbol::qual_name_table::QualifiedNameTable;
use crate::symbol::scope_graph::{Namespace, ScopeGraph, TypeBinding};
use crate::symbol::std_library::{LibraryCategory, LibraryType, StdLibManager};
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeHierarchyEdge {
    pub from_sym: u32,
    pub to_sym: u32,
    pub relation: THRelation,
    /// `EdgeFlags` bits.
    pub flags: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PackageEntry {
    pub sym: u32,
    pub scope: u32,
}

/// One compilation unit (source file) as seen by Phase 3.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitInfo {
    pub file_id: Option<u16>,
    /// AST root node of the file, or `u32::MAX` when Phase 2 produced no tree for it.
    pub root_node: u32,
    /// `[start, end)` preorder range of the file's AST, or `(u32::MAX, u32::MAX)`.
    pub node_range: (u32, u32),
    pub module_sym: u32,
    pub cu_scope: u32,
    pub package_scope: u32,
    /// Package symbol, or `u32::MAX` for the unnamed package.
    pub package_sym: u32,
    pub package_name: String,
    pub dialect: Dialect,
    pub front_end: FrontEnd,
    /// First token after the package/import header.
    pub header_end: u32,
    /// `[start, end)` token range of the file, or `(u32::MAX, u32::MAX)` when it has no tokens.
    pub token_range: (u32, u32),
}

/// Token span `[first, last]` of a declaration's written type, recorded by the front ends and
/// parsed into the TypeRef table by Pass 4.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeSpan {
    pub first: u32,
    pub last: u32,
    pub dialect: Dialect,
    /// C-style array dimensions written after the declarator name (`int a[]`).
    pub extra_dims: u8,
    /// Spans of the alternatives of a multi-catch union type (empty otherwise).
    pub union_parts: Vec<(u32, u32)>,
}

/// A supertype written in a declaration discovered without a structural AST.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LexicalSupertype {
    pub sub: u32,
    pub first: u32,
    pub last: u32,
    /// Kotlin `Base(args)`: the supertype list invokes a constructor, so it is the superclass.
    pub invokes_constructor: bool,
    pub dialect: Dialect,
}

#[derive(Debug, Clone)]
pub struct SymbolTableBuilder {
    pub symbols: Vec<SymbolRecord>,
    node_to_symbol: Vec<u32>,
    pub scope_graph: ScopeGraph,
    pub qual_names: QualifiedNameTable,
    pub std_lib: StdLibManager,
    pub th_edges: Vec<TypeHierarchyEdge>,
    th_edge_keys: HashSet<(u32, u32, u8)>,
    pub associations: Vec<UMLAssociationRecord>,
    /// Pairs of association indices that are the two ends of one bidirectional association.
    pub association_pairs: Vec<(u32, u32)>,
    type_ref_resolutions: Vec<u32>,
    type_ref_status: Vec<u8>,
    pub custom_package_names: BTreeMap<u32, String>,
    pub file_package_names: BTreeMap<u16, String>,
    /// Global index of project types by canonical name (JLS 6.7).
    pub types_by_fqn: HashMap<String, u32>,
    packages: BTreeMap<String, PackageEntry>,
    externals: HashMap<String, u32>,
    library_symbols: HashMap<usize, u32>,
    library_of_symbol: HashMap<u32, usize>,
    pub units: Vec<UnitInfo>,
    unit_by_cu_scope: HashMap<u32, usize>,
    owned_scopes: Vec<u32>,
    pub type_refs: Vec<TypeRef>,
    /// Declared type (TypeRef id) of every symbol, `u32::MAX` when it has none.
    pub symbol_type_refs: Vec<u32>,
    pub supertypes: Vec<SupertypeRecord>,
    pub occurrences: Vec<Occurrence>,
    pub diagnostics: Vec<Diagnostic>,
    /// Stable descriptor (qualified-name id) of every symbol, `u32::MAX` when not assigned.
    pub descriptors: Vec<u32>,
    scope_by_node: Vec<u32>,
    enclosing_by_node: Vec<u32>,
    /// Preorder ranges of AST subtrees that no structural front end processed.
    excluded_node_ranges: Vec<(u32, u32)>,
    /// Written type of every typed declaration (fields, variables, parameters, return types).
    pub type_spans: HashMap<u32, TypeSpan>,
    /// Bound spans of every type parameter that declares bounds.
    pub type_param_bounds: HashMap<u32, Vec<(u32, u32)>>,
    pub lexical_supertypes: Vec<LexicalSupertype>,
    /// Implicit member → symbol whose declared type it shares (record field → component).
    pub implicit_type_sources: HashMap<u32, u32>,
    /// Anonymous classes: (symbol, instance creation node, `new` token).
    pub anonymous_classes: Vec<(u32, u32, u32)>,
    /// Type declarations discovered from the AST: (symbol, declaration node, name token).
    pub structural_types: Vec<(u32, u32, u32)>,
}

impl Default for SymbolTableBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl SymbolTableBuilder {
    pub fn new() -> Self {
        Self {
            symbols: Vec::new(),
            node_to_symbol: Vec::new(),
            scope_graph: ScopeGraph::new(),
            qual_names: QualifiedNameTable::new(),
            std_lib: StdLibManager::new(),
            th_edges: Vec::new(),
            th_edge_keys: HashSet::new(),
            associations: Vec::new(),
            association_pairs: Vec::new(),
            type_ref_resolutions: Vec::new(),
            type_ref_status: Vec::new(),
            custom_package_names: BTreeMap::new(),
            file_package_names: BTreeMap::new(),
            types_by_fqn: HashMap::new(),
            packages: BTreeMap::new(),
            externals: HashMap::new(),
            library_symbols: HashMap::new(),
            library_of_symbol: HashMap::new(),
            units: Vec::new(),
            unit_by_cu_scope: HashMap::new(),
            owned_scopes: Vec::new(),
            type_refs: Vec::new(),
            symbol_type_refs: Vec::new(),
            supertypes: Vec::new(),
            occurrences: Vec::new(),
            diagnostics: Vec::new(),
            descriptors: Vec::new(),
            scope_by_node: Vec::new(),
            enclosing_by_node: Vec::new(),
            excluded_node_ranges: Vec::new(),
            type_spans: HashMap::new(),
            type_param_bounds: HashMap::new(),
            lexical_supertypes: Vec::new(),
            implicit_type_sources: HashMap::new(),
            anonymous_classes: Vec::new(),
            structural_types: Vec::new(),
        }
    }

    // ── Symbols and scopes ──────────────────────────────────────────────────

    pub fn create_scope(&mut self, owner_symbol: u32, parent_scope: u32, kind: ScopeKind) -> u32 {
        self.scope_graph
            .create_scope(owner_symbol, parent_scope, kind)
    }

    /// Appends a symbol. The first symbol created for an AST node becomes that node's primary
    /// symbol (`symbol_at_node`); later symbols sharing the node do not displace it.
    pub fn create_symbol(&mut self, mut record: SymbolRecord) -> u32 {
        let symbol_id = self.symbols.len() as u32;
        record.symbol_id = symbol_id;

        if record.decl_node != u32::MAX {
            let idx = record.decl_node as usize;
            if idx >= self.node_to_symbol.len() {
                self.node_to_symbol.resize(idx + 1, u32::MAX);
            }
            if self.node_to_symbol[idx] == u32::MAX {
                self.node_to_symbol[idx] = symbol_id;
            }
        }

        if record.scope_id != u32::MAX {
            self.scope_graph.add_declaration(record.scope_id, symbol_id);
        }

        self.symbols.push(record);
        self.owned_scopes.push(u32::MAX);
        self.symbol_type_refs.push(u32::MAX);
        self.descriptors.push(u32::MAX);
        symbol_id
    }

    /// Records `parent` as the container of `child`. Self-containment and links that would
    /// close a containment cycle are refused.
    pub fn set_parent(&mut self, child: u32, parent: u32) {
        let n = self.symbols.len();
        if child as usize >= n || child == parent {
            return;
        }
        if parent != u32::MAX {
            if parent as usize >= n {
                return;
            }
            let mut cur = parent;
            let mut steps = 0usize;
            while cur != u32::MAX && (cur as usize) < n && steps <= n {
                if cur == child {
                    return;
                }
                cur = self.symbols[cur as usize].parent_sym;
                steps += 1;
            }
        }
        self.symbols[child as usize].parent_sym = parent;
    }

    pub fn set_owned_scope(&mut self, symbol_id: u32, scope_id: u32) {
        if let Some(s) = self.owned_scopes.get_mut(symbol_id as usize) {
            *s = scope_id;
        }
    }

    /// Scope opened by a declaration (class body, method, lambda), or `u32::MAX`.
    pub fn owned_scope(&self, symbol_id: u32) -> u32 {
        self.owned_scopes
            .get(symbol_id as usize)
            .copied()
            .unwrap_or(u32::MAX)
    }

    pub fn bind_name(
        &mut self,
        scope_id: u32,
        ns: Namespace,
        name_id: u32,
        symbol_id: u32,
        order_token: u32,
    ) {
        self.scope_graph
            .bind_name(scope_id, ns, name_id, symbol_id, order_token);
    }

    pub fn set_type_id(&mut self, symbol_id: u32, type_id: u32) {
        if let Some(sym) = self.symbols.get_mut(symbol_id as usize) {
            sym.type_id = type_id;
        }
    }

    pub fn add_flags(&mut self, symbol_id: u32, flags: u8) {
        if let Some(sym) = self.symbols.get_mut(symbol_id as usize) {
            sym.flags |= flags;
        }
    }

    pub fn symbol(&self, symbol_id: u32) -> Option<&SymbolRecord> {
        self.symbols.get(symbol_id as usize)
    }

    pub fn symbol_kind(&self, symbol_id: u32) -> Option<SymbolKind> {
        self.symbol(symbol_id).map(|s| SymbolKind::from(s.kind))
    }

    pub fn symbol_at_node(&self, pre_idx: u32) -> Option<u32> {
        self.node_to_symbol
            .get(pre_idx as usize)
            .copied()
            .filter(|&s| s != u32::MAX)
    }

    pub fn symbol_count(&self) -> usize {
        self.symbols.len()
    }

    // ── Per-node context recorded by Pass 1 ─────────────────────────────────

    pub fn init_node_tables(&mut self, node_count: u32) {
        let n = node_count as usize;
        self.scope_by_node = vec![u32::MAX; n];
        self.enclosing_by_node = vec![u32::MAX; n];
        self.type_ref_resolutions = vec![u32::MAX; n];
        self.type_ref_status = vec![u8::MAX; n];
        if self.node_to_symbol.len() < n {
            self.node_to_symbol.resize(n, u32::MAX);
        }
    }

    pub fn set_node_context(&mut self, node: u32, scope: u32, enclosing_symbol: u32) {
        let i = node as usize;
        if i < self.scope_by_node.len() {
            self.scope_by_node[i] = scope;
            self.enclosing_by_node[i] = enclosing_symbol;
        }
    }

    /// Scope in effect for the children of `node` (the scope a type reference under it sees).
    pub fn scope_of_node(&self, node: u32) -> u32 {
        self.scope_by_node
            .get(node as usize)
            .copied()
            .unwrap_or(u32::MAX)
    }

    /// Innermost declaration enclosing (or declared at) `node`.
    pub fn enclosing_symbol_of_node(&self, node: u32) -> u32 {
        self.enclosing_by_node
            .get(node as usize)
            .copied()
            .unwrap_or(u32::MAX)
    }

    pub fn exclude_node_range(&mut self, start: u32, end: u32) {
        if start < end {
            self.excluded_node_ranges.push((start, end));
        }
    }

    /// True when `node` belongs to a file that no AST-driven front end processed.
    pub fn is_node_excluded(&self, node: u32) -> bool {
        self.excluded_node_ranges
            .iter()
            .any(|&(s, e)| node >= s && node < e)
    }

    pub fn set_type_ref_resolution(
        &mut self,
        pre_idx: u32,
        resolved: u32,
        status: ResolutionStatus,
    ) {
        let i = pre_idx as usize;
        if i >= self.type_ref_resolutions.len() {
            self.type_ref_resolutions.resize(i + 1, u32::MAX);
            self.type_ref_status.resize(i + 1, u8::MAX);
        }
        self.type_ref_resolutions[i] = resolved;
        self.type_ref_status[i] = status as u8;
    }

    /// Symbol (or primitive pseudo-id) bound to the type reference node `pre_idx`.
    pub fn get_type_ref_resolution(&self, pre_idx: u32) -> Option<u32> {
        self.type_ref_resolutions
            .get(pre_idx as usize)
            .copied()
            .filter(|&s| s != u32::MAX)
    }

    pub fn type_ref_status(&self, pre_idx: u32) -> Option<ResolutionStatus> {
        self.type_ref_status
            .get(pre_idx as usize)
            .and_then(|&s| ResolutionStatus::from_u8(s))
    }

    /// All recorded type-reference bindings as (node, symbol, status), in node order.
    pub fn type_ref_bindings(&self) -> Vec<(u32, u32, u8)> {
        self.type_ref_status
            .iter()
            .enumerate()
            .filter(|(_, &st)| st != u8::MAX)
            .map(|(i, &st)| (i as u32, self.type_ref_resolutions[i], st))
            .collect()
    }

    // ── Packages, units and the global type index ───────────────────────────

    /// Returns the package symbol and scope for `path`, creating the package and all its
    /// prefixes. `segment_name_ids` are the token-corpus ids of the path segments.
    pub fn ensure_package(&mut self, path: &[String], segment_name_ids: &[u32]) -> PackageEntry {
        let mut parent_sym = u32::MAX;
        let mut entry = PackageEntry {
            sym: u32::MAX,
            scope: u32::MAX,
        };
        let mut full = String::new();
        for (i, seg) in path.iter().enumerate() {
            if !full.is_empty() {
                full.push('.');
            }
            full.push_str(seg);
            let existing = self.packages.get(&full).copied();
            entry = match existing {
                Some(e) => e,
                None => {
                    let mut rec = SymbolRecord::UNINIT;
                    rec.kind = SymbolKind::SK_PACKAGE as u8;
                    rec.visibility = SymbolVisibility::Public as u8;
                    rec.name_id = segment_name_ids.get(i).copied().unwrap_or(u32::MAX);
                    rec.qual_name_id = self.qual_names.get_or_intern(&full);
                    rec.scope_id = crate::symbol::scope_graph::ROOT_SCOPE;
                    let sym = self.create_symbol(rec);
                    self.set_parent(sym, parent_sym);
                    let scope = self.create_scope(
                        sym,
                        crate::symbol::scope_graph::ROOT_SCOPE,
                        ScopeKind::File,
                    );
                    self.scope_graph
                        .add_flags(scope, crate::symbol::scope_graph::ScopeFlags::PACKAGE);
                    self.set_owned_scope(sym, scope);
                    self.custom_package_names.insert(sym, full.clone());
                    let e = PackageEntry { sym, scope };
                    self.packages.insert(full.clone(), e);
                    e
                }
            };
            parent_sym = entry.sym;
        }
        entry
    }

    /// Scope of the unnamed package (created on first use, owner-less).
    pub fn unnamed_package_scope(&mut self) -> u32 {
        if let Some(e) = self.packages.get("") {
            return e.scope;
        }
        let scope = self.create_scope(
            u32::MAX,
            crate::symbol::scope_graph::ROOT_SCOPE,
            ScopeKind::File,
        );
        self.scope_graph
            .add_flags(scope, crate::symbol::scope_graph::ScopeFlags::PACKAGE);
        self.packages.insert(
            String::new(),
            PackageEntry {
                sym: u32::MAX,
                scope,
            },
        );
        scope
    }

    pub fn package_entry(&self, path: &str) -> Option<PackageEntry> {
        if path.is_empty() {
            return None;
        }
        self.packages.get(path).copied()
    }

    pub fn is_project_package(&self, path: &str) -> bool {
        !path.is_empty() && self.packages.contains_key(path)
    }

    pub fn add_unit(&mut self, unit: UnitInfo) -> usize {
        let idx = self.units.len();
        self.unit_by_cu_scope.insert(unit.cu_scope, idx);
        if let (Some(fid), false) = (unit.file_id, unit.package_name.is_empty()) {
            self.file_package_names
                .insert(fid, unit.package_name.clone());
        }
        self.units.push(unit);
        idx
    }

    pub fn unit_of_cu_scope(&self, cu_scope: u32) -> Option<&UnitInfo> {
        self.unit_by_cu_scope
            .get(&cu_scope)
            .and_then(|&i| self.units.get(i))
    }

    pub fn dialect_of_unit(&self, cu_scope: u32) -> Dialect {
        self.unit_of_cu_scope(cu_scope)
            .map(|u| u.dialect)
            .unwrap_or(Dialect::Java)
    }

    /// Registers the canonical name of a project type and stores it as its `qual_name_id`.
    pub fn register_type_fqn(&mut self, fqn: &str, symbol_id: u32) {
        let qid = self.qual_names.get_or_intern(fqn);
        if let Some(sym) = self.symbols.get_mut(symbol_id as usize) {
            sym.qual_name_id = qid;
        }
        self.types_by_fqn
            .entry(fqn.to_string())
            .or_insert(symbol_id);
    }

    pub fn set_qualified_name(&mut self, symbol_id: u32, name: &str) {
        let qid = self.qual_names.get_or_intern(name);
        if let Some(sym) = self.symbols.get_mut(symbol_id as usize) {
            sym.qual_name_id = qid;
        }
    }

    pub fn qualified_name_of(&self, symbol_id: u32) -> Option<&str> {
        self.symbol(symbol_id)
            .and_then(|s| self.qual_names.get_name(s.qual_name_id))
    }

    // ── Externals and library types ─────────────────────────────────────────

    /// Library catalog entry backing a materialized symbol.
    pub fn library_type_of(&self, symbol_id: u32) -> Option<&LibraryType> {
        self.library_of_symbol
            .get(&symbol_id)
            .and_then(|&i| self.std_lib.get(i))
    }

    pub fn library_category_of(&self, symbol_id: u32) -> Option<LibraryCategory> {
        self.library_type_of(symbol_id).map(|t| t.category)
    }

    /// Kind of a type symbol, looking through materialized library externals.
    pub fn effective_type_kind(&self, symbol_id: u32) -> Option<SymbolKind> {
        if let Some(lib) = self.library_type_of(symbol_id) {
            return Some(lib.kind);
        }
        match self.symbol_kind(symbol_id)? {
            SymbolKind::SK_EXTERNAL => None,
            k => Some(k),
        }
    }

    pub fn is_project_type(&self, symbol_id: u32) -> bool {
        matches!(
            self.symbol_kind(symbol_id),
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

    /// Materializes a catalog entry as an SK_EXTERNAL symbol, together with its library
    /// supertypes and the hierarchy edges between them.
    pub fn materialize_library(&mut self, idx: usize) -> u32 {
        if let Some(&s) = self.library_symbols.get(&idx) {
            return s;
        }
        let lib = match self.std_lib.get(idx) {
            Some(&t) => t,
            None => return u32::MAX,
        };
        let mut rec = SymbolRecord::UNINIT;
        rec.kind = SymbolKind::SK_EXTERNAL as u8;
        rec.visibility = SymbolVisibility::Public as u8;
        rec.name_id = self.std_lib.name_id(idx);
        rec.qual_name_id = self.qual_names.get_or_intern(lib.fqn);
        rec.type_param_count = lib.type_params;
        if lib.is_interface() {
            rec.modifiers |= SymbolModifiers::ABSTRACT;
        }
        let sym = self.create_symbol(rec);
        self.library_symbols.insert(idx, sym);
        self.library_of_symbol.insert(sym, idx);
        self.externals.insert(lib.fqn.to_string(), sym);

        for sup in lib.supertypes {
            if let Some(sidx) = self.std_lib.lookup_fqn(sup) {
                let sup_is_interface = self
                    .std_lib
                    .get(sidx)
                    .map(|t| t.is_interface())
                    .unwrap_or(false);
                let ssym = self.materialize_library(sidx);
                let relation = if !lib.is_interface() && sup_is_interface {
                    THRelation::TH_IMPLEMENTS
                } else {
                    THRelation::TH_EXTENDS
                };
                self.add_th_edge(sym, ssym, relation);
            }
        }
        sym
    }

    fn external_symbol(&mut self, key: &str, qual_name: &str, name_id: u32, flags: u8) -> u32 {
        if let Some(&s) = self.externals.get(key) {
            if flags & SymbolFlags::INFERRED_PACKAGE == 0 {
                // An explicit qualified name confirms a package that was only inferred before.
                if let Some(rec) = self.symbols.get_mut(s as usize) {
                    rec.flags &= !SymbolFlags::INFERRED_PACKAGE;
                }
            }
            if name_id != u32::MAX {
                if let Some(rec) = self.symbols.get_mut(s as usize) {
                    if rec.name_id == u32::MAX {
                        rec.name_id = name_id;
                    }
                }
            }
            return s;
        }
        let mut rec = SymbolRecord::UNINIT;
        rec.kind = SymbolKind::SK_EXTERNAL as u8;
        rec.visibility = SymbolVisibility::Public as u8;
        rec.name_id = name_id;
        rec.qual_name_id = self.qual_names.get_or_intern(qual_name);
        rec.flags = flags;
        let sym = self.create_symbol(rec);
        self.externals.insert(key.to_string(), sym);
        sym
    }

    /// Turns a resolver binding into a symbol id. Externals are keyed by qualified name, so
    /// same-named types of different packages stay distinct; unresolved names are keyed by
    /// their simple name together with the candidate packages.
    pub fn materialize_binding(
        &mut self,
        binding: TypeBinding,
        name_id: u32,
        token: u32,
    ) -> (u32, ResolutionStatus) {
        match binding {
            TypeBinding::Symbol(s) => (s, ResolutionStatus::Project),
            TypeBinding::Library(idx) => (self.materialize_library(idx), ResolutionStatus::Library),
            TypeBinding::Qualified(fqn) => (
                self.external_symbol(&fqn, &fqn, name_id, 0),
                ResolutionStatus::Qualified,
            ),
            TypeBinding::InferredPackage(fqn) => (
                self.external_symbol(&fqn, &fqn, name_id, SymbolFlags::INFERRED_PACKAGE),
                ResolutionStatus::InferredPackage,
            ),
            TypeBinding::Ambiguous(candidates) => {
                let simple = candidates
                    .first()
                    .and_then(|c| c.rsplit('.').next())
                    .unwrap_or("")
                    .to_string();
                let key = format!("?{}|{}", simple, candidates.join(","));
                let existed = self.externals.contains_key(&key);
                let sym = self.external_symbol(&key, &simple, name_id, SymbolFlags::UNRESOLVED);
                if !existed {
                    let related = candidates
                        .iter()
                        .filter_map(|c| self.types_by_fqn.get(c).copied())
                        .collect();
                    self.add_diagnostic(
                        DiagnosticCode::AmbiguousType,
                        sym,
                        token,
                        related,
                        format!(
                            "type name '{}' is supplied by several on-demand imports: {}",
                            simple,
                            candidates.join(", ")
                        ),
                    );
                }
                (sym, ResolutionStatus::Ambiguous)
            }
            TypeBinding::Unresolved { simple, candidates } => {
                let key = format!("?{}|{}", simple, candidates.join(","));
                let existed = self.externals.contains_key(&key);
                let sym = self.external_symbol(&key, &simple, name_id, SymbolFlags::UNRESOLVED);
                if !existed {
                    let message = if candidates.is_empty() {
                        format!(
                            "type '{}' is not declared in the project, the library catalog or any import",
                            simple
                        )
                    } else {
                        format!(
                            "type '{}' is not declared in the project; candidates: {}",
                            simple,
                            candidates.join(", ")
                        )
                    };
                    self.add_diagnostic(
                        DiagnosticCode::UnresolvedType,
                        sym,
                        token,
                        Vec::new(),
                        message,
                    );
                }
                (sym, ResolutionStatus::Unresolved)
            }
        }
    }

    // ── TypeRefs, hierarchy, associations, occurrences, diagnostics ─────────

    pub fn add_type_ref(&mut self, tr: TypeRef) -> u32 {
        let id = self.type_refs.len() as u32;
        self.type_refs.push(tr);
        id
    }

    pub fn type_ref(&self, id: u32) -> Option<&TypeRef> {
        self.type_refs.get(id as usize)
    }

    pub fn set_symbol_type_ref(&mut self, symbol_id: u32, type_ref: u32) {
        if let Some(t) = self.symbol_type_refs.get_mut(symbol_id as usize) {
            *t = type_ref;
        }
    }

    pub fn symbol_type_ref(&self, symbol_id: u32) -> Option<&TypeRef> {
        let id = self.symbol_type_refs.get(symbol_id as usize).copied()?;
        self.type_ref(id)
    }

    pub fn add_th_edge(&mut self, from_sym: u32, to_sym: u32, relation: THRelation) {
        self.add_th_edge_with_flags(from_sym, to_sym, relation, 0);
    }

    pub fn add_th_edge_with_flags(
        &mut self,
        from_sym: u32,
        to_sym: u32,
        relation: THRelation,
        flags: u8,
    ) {
        if from_sym == to_sym || from_sym == u32::MAX || to_sym == u32::MAX {
            return;
        }
        if !self.th_edge_keys.insert((from_sym, to_sym, relation as u8)) {
            return;
        }
        self.th_edges.push(TypeHierarchyEdge {
            from_sym,
            to_sym,
            relation,
            flags,
        });
    }

    /// Direct supertypes (EXTENDS/IMPLEMENTS targets) of every type, built once.
    pub fn supertype_index(&self) -> HashMap<u32, Vec<u32>> {
        let mut index: HashMap<u32, Vec<u32>> = HashMap::new();
        for e in &self.th_edges {
            if matches!(
                e.relation,
                THRelation::TH_EXTENDS | THRelation::TH_IMPLEMENTS
            ) {
                index.entry(e.from_sym).or_default().push(e.to_sym);
            }
        }
        index
    }

    pub fn add_association(&mut self, assoc: UMLAssociationRecord) -> u32 {
        let id = self.associations.len() as u32;
        self.associations.push(assoc);
        id
    }

    pub fn add_occurrence(&mut self, occ: Occurrence) {
        if occ.token != u32::MAX {
            self.occurrences.push(occ);
        }
    }

    pub fn add_diagnostic(
        &mut self,
        code: DiagnosticCode,
        symbol: u32,
        token: u32,
        related: Vec<u32>,
        message: String,
    ) {
        self.diagnostics.push(Diagnostic {
            code,
            symbol,
            token,
            related,
            message,
        });
    }

    // ── Finalization ────────────────────────────────────────────────────────

    /// Derives `first_child`/`next_sibling` from `parent_sym`, ordering siblings by declaration
    /// position (first token, then creation order).
    pub fn rebuild_child_chains(&mut self) {
        let n = self.symbols.len();
        let mut kids: Vec<(u32, u32, u32)> = Vec::new();
        for s in &mut self.symbols {
            s.first_child = u32::MAX;
            s.next_sibling = u32::MAX;
        }
        for s in &self.symbols {
            if s.parent_sym != u32::MAX && (s.parent_sym as usize) < n {
                kids.push((s.parent_sym, s.first_token_id, s.symbol_id));
            }
        }
        kids.sort_unstable();
        let mut i = 0usize;
        while i < kids.len() {
            let parent = kids[i].0;
            self.symbols[parent as usize].first_child = kids[i].2;
            let mut j = i;
            while j + 1 < kids.len() && kids[j + 1].0 == parent {
                self.symbols[kids[j].2 as usize].next_sibling = kids[j + 1].2;
                j += 1;
            }
            i = j + 1;
        }
    }

    /// Finds the strongly connected components of the EXTENDS/IMPLEMENTS graph (Tarjan),
    /// flags their edges and members, and reports each cycle as a diagnostic. No edge is
    /// removed: a cycle is either illegal source or a resolution error, and both must stay
    /// visible.
    pub fn mark_hierarchy_cycles(&mut self) {
        let mut nodes: Vec<u32> = Vec::new();
        let mut index_of: HashMap<u32, usize> = HashMap::new();
        let mut adj: Vec<Vec<usize>> = Vec::new();
        for e in &self.th_edges {
            if !matches!(
                e.relation,
                THRelation::TH_EXTENDS | THRelation::TH_IMPLEMENTS
            ) {
                continue;
            }
            let mut idx = |sym: u32, nodes: &mut Vec<u32>, adj: &mut Vec<Vec<usize>>| -> usize {
                *index_of.entry(sym).or_insert_with(|| {
                    nodes.push(sym);
                    adj.push(Vec::new());
                    nodes.len() - 1
                })
            };
            let a = idx(e.from_sym, &mut nodes, &mut adj);
            let b = idx(e.to_sym, &mut nodes, &mut adj);
            adj[a].push(b);
        }

        let components = tarjan_scc(&adj);
        let mut component_of = vec![usize::MAX; nodes.len()];
        let mut cyclic: Vec<Vec<u32>> = Vec::new();
        for comp in components {
            if comp.len() > 1 {
                let cid = cyclic.len();
                for &v in &comp {
                    component_of[v] = cid;
                }
                let mut members: Vec<u32> = comp.iter().map(|&v| nodes[v]).collect();
                members.sort_unstable();
                cyclic.push(members);
            }
        }
        if cyclic.is_empty() {
            return;
        }

        for e in &mut self.th_edges {
            if !matches!(
                e.relation,
                THRelation::TH_EXTENDS | THRelation::TH_IMPLEMENTS
            ) {
                continue;
            }
            let (a, b) = match (index_of.get(&e.from_sym), index_of.get(&e.to_sym)) {
                (Some(&a), Some(&b)) => (a, b),
                _ => continue,
            };
            if component_of[a] != usize::MAX && component_of[a] == component_of[b] {
                e.flags |= EdgeFlags::CYCLE;
            }
        }
        for members in cyclic {
            for &m in &members {
                self.add_flags(m, SymbolFlags::HIERARCHY_CYCLE);
            }
            let first = members[0];
            let token = self
                .symbol(first)
                .map(|s| s.first_token_id)
                .unwrap_or(u32::MAX);
            let message = format!(
                "cyclic type hierarchy among {} types (symbols {:?})",
                members.len(),
                members
            );
            self.add_diagnostic(
                DiagnosticCode::HierarchyCycle,
                first,
                token,
                members,
                message,
            );
        }
    }

    /// Assigns a stable, position-independent descriptor to every symbol, following the
    /// SemanticDB symbol grammar (`com/acme/User#find(+1).`). Locals, lambdas, anonymous and
    /// local classes get document-scoped `local<n>` descriptors.
    pub fn assign_descriptors(&mut self, tca: &TokenCorpusArtifact) {
        let n = self.symbols.len();
        let mut desc: Vec<Option<String>> = vec![None; n];
        let mut local_counters: HashMap<u16, u32> = HashMap::new();

        // Overload index of each method/constructor among same-named siblings.
        let mut overload: Vec<u32> = vec![0; n];
        let mut seen: HashMap<(u32, u8, u32), u32> = HashMap::new();
        let mut order: Vec<(u32, u32, u32)> = self
            .symbols
            .iter()
            .filter(|s| {
                matches!(
                    SymbolKind::from(s.kind),
                    SymbolKind::SK_METHOD | SymbolKind::SK_CONSTRUCTOR
                )
            })
            .map(|s| (s.parent_sym, s.first_token_id, s.symbol_id))
            .collect();
        order.sort_unstable();
        for (parent, _, id) in order {
            let s = &self.symbols[id as usize];
            let key = (parent, s.kind, s.name_id);
            let k = seen.entry(key).or_insert(0);
            overload[id as usize] = *k;
            *k += 1;
        }

        let text = |name_id: u32| -> String {
            match std::str::from_utf8(tca.interner.lookup_text(name_id)) {
                Ok(t) if !t.is_empty() => t.to_string(),
                _ => String::new(),
            }
        };

        for id in 0..n {
            if desc[id].is_some() {
                continue;
            }
            // Resolve the ancestor chain first (parents normally precede children).
            let mut chain = vec![id];
            let mut cur = self.symbols[id].parent_sym;
            let mut steps = 0usize;
            while cur != u32::MAX
                && (cur as usize) < n
                && desc[cur as usize].is_none()
                && steps <= n
            {
                chain.push(cur as usize);
                cur = self.symbols[cur as usize].parent_sym;
                steps += 1;
            }
            for &sid in chain.iter().rev() {
                let s = self.symbols[sid];
                let parent_desc = if s.parent_sym != u32::MAX && (s.parent_sym as usize) < n {
                    desc[s.parent_sym as usize].clone()
                } else {
                    None
                };
                let file = if (s.first_token_id as usize) < tca.token_records.len() {
                    Some(unpack_sort_key(tca.token_records[s.first_token_id as usize].sort_key).0)
                } else {
                    None
                };
                let mut local = || -> String {
                    let fid = file.unwrap_or(u16::MAX);
                    let c = local_counters.entry(fid).or_insert(0);
                    let d = format!("local{}", c);
                    *c += 1;
                    d
                };
                let name = text(s.name_id);
                let is_local_owner = parent_desc
                    .as_deref()
                    .map(|p| p.starts_with("local"))
                    .unwrap_or(false);
                let kind = SymbolKind::from(s.kind);
                let d = match kind {
                    SymbolKind::SK_PACKAGE => self
                        .qual_names
                        .get_name(s.qual_name_id)
                        .map(|q| format!("{}/", q.replace('.', "/")))
                        .unwrap_or_else(&mut local),
                    SymbolKind::SK_MODULE => match file {
                        Some(fid) => format!("_file{}_/", fid),
                        None => format!("_unit{}_/", sid),
                    },
                    SymbolKind::SK_EXTERNAL => {
                        if s.flags & SymbolFlags::UNRESOLVED != 0 {
                            format!("_unresolved_/{}#", name)
                        } else {
                            let q = self.qual_names.get_name(s.qual_name_id).unwrap_or("");
                            external_descriptor(q)
                        }
                    }
                    SymbolKind::SK_CLASS
                    | SymbolKind::SK_INTERFACE
                    | SymbolKind::SK_ENUM
                    | SymbolKind::SK_RECORD
                    | SymbolKind::SK_ANNOTATION_TYPE => {
                        let parent_kind = self.symbol_kind(s.parent_sym);
                        match (parent_kind, &parent_desc) {
                            _ if name.is_empty() || is_local_owner => local(),
                            (Some(SymbolKind::SK_PACKAGE), Some(p)) => format!("{}{}#", p, name),
                            (Some(SymbolKind::SK_MODULE), _) | (None, _) => {
                                match self.qualified_name_of(sid as u32) {
                                    Some(q) => external_descriptor(q),
                                    None => format!("_empty_/{}#", name),
                                }
                            }
                            (
                                Some(
                                    SymbolKind::SK_CLASS
                                    | SymbolKind::SK_INTERFACE
                                    | SymbolKind::SK_ENUM
                                    | SymbolKind::SK_RECORD
                                    | SymbolKind::SK_ANNOTATION_TYPE,
                                ),
                                Some(p),
                            ) => format!("{}{}#", p, name),
                            _ => local(),
                        }
                    }
                    SymbolKind::SK_METHOD | SymbolKind::SK_CONSTRUCTOR => match &parent_desc {
                        Some(p) if !is_local_owner => {
                            let mname = if kind == SymbolKind::SK_CONSTRUCTOR {
                                "<init>".to_string()
                            } else {
                                name.clone()
                            };
                            let k = overload[sid];
                            if mname.is_empty() {
                                local()
                            } else if k == 0 {
                                format!("{}{}().", p, mname)
                            } else {
                                format!("{}{}(+{}).", p, mname, k)
                            }
                        }
                        _ => local(),
                    },
                    SymbolKind::SK_FIELD | SymbolKind::SK_ENUM_CONSTANT => match &parent_desc {
                        Some(p) if !is_local_owner && !name.is_empty() => {
                            format!("{}{}.", p, name)
                        }
                        _ => local(),
                    },
                    SymbolKind::SK_PARAM => match &parent_desc {
                        Some(p) if !is_local_owner && !name.is_empty() && p.ends_with('.') => {
                            format!("{}({})", p, name)
                        }
                        _ => local(),
                    },
                    SymbolKind::SK_TYPE_PARAM => match &parent_desc {
                        Some(p) if !is_local_owner && !name.is_empty() => {
                            format!("{}[{}]", p, name)
                        }
                        _ => local(),
                    },
                    SymbolKind::SK_STATIC_INIT => match &parent_desc {
                        Some(p) if !is_local_owner => format!("{}<clinit>().", p),
                        _ => local(),
                    },
                    _ => local(),
                };
                desc[sid] = Some(d);
            }
        }

        for (id, d) in desc.into_iter().enumerate() {
            if let Some(d) = d {
                let qid = self.qual_names.get_or_intern(&d);
                self.descriptors[id] = qid;
            }
        }
    }

    /// Sorts occurrences by token and role, removing duplicates.
    pub fn finalize_occurrences(&mut self) {
        self.occurrences
            .sort_by(|a, b| (a.token, a.role, a.node).cmp(&(b.token, b.role, b.node)));
        self.occurrences
            .dedup_by(|a, b| a.token == b.token && a.role == b.role);
    }

    // ── Invariants ──────────────────────────────────────────────────────────

    /// Checks that the BP AST was built from the token corpus Phase 3 received (Invariant 4).
    pub fn verify_artifact_link(&self, bpa: &BPASTArtifact, tca_hash: u64) -> Result<(), String> {
        if bpa.tca_hash != tca_hash {
            return Err(format!(
                "Invariant 4 Violated: BP AST was built from a different token corpus (BPA link 0x{:016X}, TCA hash 0x{:016X})",
                bpa.tca_hash, tca_hash
            ));
        }
        Ok(())
    }

    /// Verifies the Phase 3 invariants without modifying the table:
    /// 1. every method and constructor node of an AST-driven file has a symbol;
    /// 2. containment is consistent: parent links are in range and acyclic, and every
    ///    child chain is acyclic and lists exactly the symbols whose parent is its head;
    /// 3. the EXTENDS/IMPLEMENTS graph without diagnosed cycle edges is acyclic, and every
    ///    flagged cycle edge was reported;
    /// 5. token ranges, declaration nodes and scope ids are in range;
    /// 6. every type reference node of an AST-driven file has an explicit binding.
    pub fn verify_invariants(
        &self,
        bpa: &BPASTArtifact,
        tca: &TokenCorpusArtifact,
    ) -> Result<(), String> {
        let n = self.symbols.len();

        // Invariant 1: method completeness.
        for pre_idx in 0..bpa.node_count {
            let ntype = bpa.node_type(pre_idx);
            if (ntype == ASTNodeType::NN_METHOD_DECL || ntype == ASTNodeType::NN_CONSTRUCTOR_DECL)
                && !self.is_node_excluded(pre_idx)
                && self.symbol_at_node(pre_idx).is_none()
            {
                return Err(format!(
                    "Invariant 1 Violated: AST node {} ({:?}) has no corresponding SymbolRecord",
                    pre_idx, ntype
                ));
            }
        }

        // Invariant 2: containment.
        for s in &self.symbols {
            if s.parent_sym != u32::MAX && s.parent_sym as usize >= n {
                return Err(format!(
                    "Invariant 2 Violated: Symbol {} has out-of-range parent {}",
                    s.symbol_id, s.parent_sym
                ));
            }
            let member = matches!(
                SymbolKind::from(s.kind),
                SymbolKind::SK_METHOD
                    | SymbolKind::SK_CONSTRUCTOR
                    | SymbolKind::SK_FIELD
                    | SymbolKind::SK_ENUM_CONSTANT
                    | SymbolKind::SK_STATIC_INIT
                    | SymbolKind::SK_INSTANCE_INIT
            );
            if member && s.parent_sym == u32::MAX {
                return Err(format!(
                    "Invariant 2 Violated: Symbol {} (kind {:?}) has no parent symbol",
                    s.symbol_id, s.kind
                ));
            }
        }
        for s in &self.symbols {
            let mut cur = s.parent_sym;
            let mut steps = 0usize;
            while cur != u32::MAX {
                steps += 1;
                if steps > n {
                    return Err(format!(
                        "Invariant 2 Violated: containment cycle through symbol {}",
                        s.symbol_id
                    ));
                }
                cur = self.symbols[cur as usize].parent_sym;
            }
        }
        let mut chain_children = 0usize;
        for s in &self.symbols {
            let mut cur = s.first_child;
            let mut steps = 0usize;
            while cur != u32::MAX {
                if cur as usize >= n {
                    return Err(format!(
                        "Invariant 2 Violated: child chain of symbol {} leaves the table",
                        s.symbol_id
                    ));
                }
                if self.symbols[cur as usize].parent_sym != s.symbol_id {
                    return Err(format!(
                        "Invariant 2 Violated: symbol {} is chained under {} but its parent is {}",
                        cur, s.symbol_id, self.symbols[cur as usize].parent_sym
                    ));
                }
                steps += 1;
                if steps > n {
                    return Err(format!(
                        "Invariant 2 Violated: cyclic child chain under symbol {}",
                        s.symbol_id
                    ));
                }
                cur = self.symbols[cur as usize].next_sibling;
            }
            chain_children += steps;
        }
        let parented = self
            .symbols
            .iter()
            .filter(|s| s.parent_sym != u32::MAX)
            .count();
        let chains_built = self.symbols.iter().any(|s| s.first_child != u32::MAX);
        if chains_built && chain_children != parented {
            return Err(format!(
                "Invariant 2 Violated: child chains list {} symbols but {} symbols have a parent",
                chain_children, parented
            ));
        }

        // Invariant 3: hierarchy acyclicity modulo diagnosed cycles.
        self.verify_th_acyclicity()?;

        // Invariant 5: token ranges, declaration nodes, scopes.
        let token_count = tca.token_records.len() as u32;
        let scope_count = self.scope_graph.scope_count() as u32;
        for sym in &self.symbols {
            if sym.first_token_id != u32::MAX && sym.first_token_id >= token_count {
                return Err(format!(
                    "Invariant 5 Violated: Symbol {} first_token_id {} out of bounds",
                    sym.symbol_id, sym.first_token_id
                ));
            }
            if sym.last_token_id != u32::MAX && sym.last_token_id >= token_count {
                return Err(format!(
                    "Invariant 5 Violated: Symbol {} last_token_id {} out of bounds",
                    sym.symbol_id, sym.last_token_id
                ));
            }
            if sym.decl_node != u32::MAX && sym.decl_node >= bpa.node_count {
                return Err(format!(
                    "Invariant 5 Violated: Symbol {} decl_node {} out of bounds",
                    sym.symbol_id, sym.decl_node
                ));
            }
            if sym.scope_id != u32::MAX && sym.scope_id >= scope_count {
                return Err(format!(
                    "Invariant 5 Violated: Symbol {} scope {} out of bounds",
                    sym.symbol_id, sym.scope_id
                ));
            }
        }

        // Invariant 6: every type reference of a structurally processed file has an explicit
        // binding (resolved, primitive, inferred `var`, or an explicit unresolved external).
        for unit in &self.units {
            if unit.front_end != FrontEnd::Structural || unit.node_range.0 == u32::MAX {
                continue;
            }
            let end = unit.node_range.1.min(bpa.node_count);
            for pre_idx in unit.node_range.0..end {
                if bpa.node_type(pre_idx) == ASTNodeType::NN_TYPE_REF
                    && self.type_ref_status(pre_idx).is_none()
                {
                    return Err(format!(
                        "Invariant 6 Violated: type reference node {} has no binding",
                        pre_idx
                    ));
                }
            }
        }

        Ok(())
    }

    fn verify_th_acyclicity(&self) -> Result<(), String> {
        let mut in_degree: HashMap<u32, usize> = HashMap::new();
        let mut adj: HashMap<u32, Vec<u32>> = HashMap::new();
        let mut flagged = 0usize;

        for edge in &self.th_edges {
            if !matches!(
                edge.relation,
                THRelation::TH_EXTENDS | THRelation::TH_IMPLEMENTS
            ) {
                continue;
            }
            if edge.flags & EdgeFlags::CYCLE != 0 {
                flagged += 1;
                continue;
            }
            in_degree.entry(edge.to_sym).or_insert(0);
            *in_degree.entry(edge.from_sym).or_insert(0) += 1;
            adj.entry(edge.to_sym).or_default().push(edge.from_sym);
        }

        let mut queue: Vec<u32> = in_degree
            .iter()
            .filter(|&(_, &deg)| deg == 0)
            .map(|(&node, _)| node)
            .collect();
        let mut visited_count = 0;
        let total_nodes = in_degree.len();
        while let Some(node) = queue.pop() {
            visited_count += 1;
            if let Some(neighbors) = adj.get(&node) {
                for &neighbor in neighbors {
                    if let Some(deg) = in_degree.get_mut(&neighbor) {
                        *deg -= 1;
                        if *deg == 0 {
                            queue.push(neighbor);
                        }
                    }
                }
            }
        }
        if visited_count < total_nodes {
            return Err(format!(
                "Invariant 3 Violated: undiagnosed cycle in the EXTENDS/IMPLEMENTS graph ({} / {} nodes visited)",
                visited_count, total_nodes
            ));
        }
        if flagged > 0
            && !self
                .diagnostics
                .iter()
                .any(|d| d.code == DiagnosticCode::HierarchyCycle)
        {
            return Err(
                "Invariant 3 Violated: hierarchy edges are flagged as cyclic but no cycle was reported"
                    .to_string(),
            );
        }
        Ok(())
    }
}

/// SemanticDB-style descriptor of an external qualified name: package segments become
/// `seg/`, type segments `Name#`.
fn external_descriptor(qualified: &str) -> String {
    let mut out = String::new();
    let mut in_types = false;
    for seg in qualified.split('.') {
        if seg.is_empty() {
            continue;
        }
        if !in_types && seg.chars().next().is_some_and(|c| c.is_uppercase()) {
            in_types = true;
        }
        out.push_str(seg);
        out.push(if in_types { '#' } else { '/' });
    }
    if out.is_empty() {
        out.push_str("_unresolved_/");
    }
    out
}

/// Iterative Tarjan strongly connected components over an adjacency list.
fn tarjan_scc(adj: &[Vec<usize>]) -> Vec<Vec<usize>> {
    let n = adj.len();
    let mut index = vec![usize::MAX; n];
    let mut low = vec![0usize; n];
    let mut on_stack = vec![false; n];
    let mut stack: Vec<usize> = Vec::new();
    let mut next_index = 0usize;
    let mut out = Vec::new();

    for root in 0..n {
        if index[root] != usize::MAX {
            continue;
        }
        let mut work: Vec<(usize, usize)> = vec![(root, 0)];
        index[root] = next_index;
        low[root] = next_index;
        next_index += 1;
        stack.push(root);
        on_stack[root] = true;

        while let Some(top) = work.last_mut() {
            let v = top.0;
            if top.1 < adj[v].len() {
                let w = adj[v][top.1];
                top.1 += 1;
                if index[w] == usize::MAX {
                    index[w] = next_index;
                    low[w] = next_index;
                    next_index += 1;
                    stack.push(w);
                    on_stack[w] = true;
                    work.push((w, 0));
                } else if on_stack[w] {
                    low[v] = low[v].min(index[w]);
                }
            } else {
                work.pop();
                if let Some(&(parent, _)) = work.last() {
                    low[parent] = low[parent].min(low[v]);
                }
                if low[v] == index[v] {
                    let mut comp = Vec::new();
                    while let Some(w) = stack.pop() {
                        on_stack[w] = false;
                        comp.push(w);
                        if w == v {
                            break;
                        }
                    }
                    out.push(comp);
                }
            }
        }
    }
    out
}
