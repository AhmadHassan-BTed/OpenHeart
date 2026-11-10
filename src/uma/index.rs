//! UMAIndex — CSR indexes over STA, CGA and TRA, built once per Phase 9 run (§9.3).
//!
//! Every extractor and pattern rule reads containment, supertypes, call sites and
//! UML links through this index, so Phase 9 runs in O(N + E) instead of
//! rescanning `th_edges`, `call_sites` or `uml_links` per class.
//!
//! **Sources of truth.**
//! - Containment is derived from `parent_sym` only; `first_child`/`next_sibling`
//!   chains are not trusted (they can hold re-linked or orphaned entries).
//! - Supertypes come from `TH_EXTENDS`/`TH_IMPLEMENTS` edges validated against the
//!   declaration header: an edge whose target is not written at bracket depth 0 of
//!   the `extends`/`implements`/`:` clause (a type argument or bound) is dropped,
//!   and a header supertype without an edge is recovered by name resolution.
//! - Call edges are kept only when the callee's name matches the invoked
//!   identifier at the call site.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::core::types::cg::{CallGraphArtifact, CallSite, CG_EDGE_CONSTRUCTOR};
use crate::core::types::symbol::{SymbolKind, SymbolModifiers, SymbolRecord, THRelation};
use crate::core::types::token::{unpack_sort_key, LangId};
use crate::ingestion::TokenCorpusArtifact;
use crate::symbol::SymbolTableArtifact;
use crate::symbol::{
    get_java_stubs, BASE_STUB_SYMBOL_ID, COLLECTION_STUB_SYM_ID, ITERABLE_STUB_SYM_ID,
    LIST_STUB_SYM_ID, MAP_STUB_SYM_ID, PRIMITIVE_BOOLEAN_ID, PRIMITIVE_BYTE_ID, PRIMITIVE_CHAR_ID,
    PRIMITIVE_DOUBLE_ID, PRIMITIVE_FLOAT_ID, PRIMITIVE_INT_ID, PRIMITIVE_LONG_ID,
    PRIMITIVE_SHORT_ID, PRIMITIVE_VOID_ID, SET_STUB_SYM_ID,
};
use crate::tra::types::{TraceabilityArtifact, UMLLinkRecord};
use crate::uma::source::{DeclaredSupertypes, SourceScanner};
use crate::uma::types::UNKNOWN_SYM;

/// Library collection and container types (JDK `java.util`, Kotlin stdlib) whose
/// instances hold many elements. Matched only against library (external/stub)
/// symbols, never against user-declared types.
const LIBRARY_COLLECTION_NAMES: &[&[u8]] = &[
    b"List",
    b"ArrayList",
    b"LinkedList",
    b"Vector",
    b"Stack",
    b"CopyOnWriteArrayList",
    b"Set",
    b"HashSet",
    b"LinkedHashSet",
    b"TreeSet",
    b"SortedSet",
    b"NavigableSet",
    b"EnumSet",
    b"CopyOnWriteArraySet",
    b"Map",
    b"HashMap",
    b"LinkedHashMap",
    b"TreeMap",
    b"SortedMap",
    b"NavigableMap",
    b"EnumMap",
    b"WeakHashMap",
    b"IdentityHashMap",
    b"ConcurrentHashMap",
    b"ConcurrentMap",
    b"ConcurrentSkipListMap",
    b"ConcurrentSkipListSet",
    b"Collection",
    b"Iterable",
    b"Queue",
    b"Deque",
    b"ArrayDeque",
    b"PriorityQueue",
    b"BlockingQueue",
    b"LinkedBlockingQueue",
    b"ArrayBlockingQueue",
    b"ConcurrentLinkedQueue",
    b"MutableList",
    b"MutableSet",
    b"MutableMap",
    b"MutableCollection",
    b"Array",
    b"Sequence",
    b"Stream",
];

/// Compressed sparse rows: `key -> [values]`.
#[derive(Debug, Clone, Default)]
pub struct Csr {
    offsets: Vec<u32>,
    items: Vec<u32>,
}

impl Csr {
    /// Build from `(key, value)` pairs; values keep the order of `pairs` within a key.
    pub fn from_pairs(key_count: usize, pairs: &[(u32, u32)]) -> Self {
        let mut offsets = vec![0u32; key_count + 1];
        for &(k, _) in pairs {
            if (k as usize) < key_count {
                offsets[k as usize + 1] += 1;
            }
        }
        for i in 0..key_count {
            offsets[i + 1] += offsets[i];
        }
        let mut cursor = offsets.clone();
        let mut items = vec![0u32; offsets[key_count] as usize];
        for &(k, v) in pairs {
            if (k as usize) < key_count {
                let slot = &mut cursor[k as usize];
                items[*slot as usize] = v;
                *slot += 1;
            }
        }
        Self { offsets, items }
    }

    pub fn get(&self, key: u32) -> &[u32] {
        let k = key as usize;
        if k + 1 >= self.offsets.len() {
            return &[];
        }
        &self.items[self.offsets[k] as usize..self.offsets[k + 1] as usize]
    }
}

/// A call edge whose callee name agrees with the invoked identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValidatedCall {
    pub site_index: u32,
    pub callee: u32,
}

pub struct UMAIndex<'a> {
    pub sta: &'a SymbolTableArtifact,
    pub tca: &'a TokenCorpusArtifact,
    pub cga: &'a CallGraphArtifact,
    pub tra: Option<&'a TraceabilityArtifact>,
    pub scanner: SourceScanner<'a>,
    sym_count: usize,
    children: Csr,
    extends: Csr,
    implements: Csr,
    subtypes: Csr,
    superclass: Vec<u32>,
    uses: Csr,
    /// Call-site indices (into `cga.call_sites`) per caller, in evaluation order.
    sites_by_caller: Csr,
    /// Validated callees per call-site index.
    site_callees: Csr,
    callees_of: Csr,
    callers_of: Csr,
    /// Evaluation-order key (closing parenthesis token) per call-site index.
    site_order_key: Vec<u32>,
    /// Indices into `tra.uml_links` sorted by sym_id.
    uml_link_order: Vec<u32>,
    stub_names: HashMap<u32, &'static [u8]>,
    /// SK_EXTERNAL placeholders (created for imported names) that denote a type
    /// declared in the analyzed sources, mapped to that declaration.
    canonical_ext: HashMap<u32, u32>,
    pub dropped_supertype_edges: usize,
    pub recovered_supertype_edges: usize,
    pub dropped_call_edges: usize,
}

impl<'a> UMAIndex<'a> {
    pub fn build(
        sta: &'a SymbolTableArtifact,
        tca: &'a TokenCorpusArtifact,
        cga: &'a CallGraphArtifact,
        tra: Option<&'a TraceabilityArtifact>,
    ) -> Self {
        let sym_count = sta.symbol_records.len();
        let scanner = SourceScanner::new(tca);

        let mut stub_names: HashMap<u32, &'static [u8]> = HashMap::new();
        for stub in get_java_stubs() {
            stub_names.insert(stub.symbol_id, stub.simple_name.as_bytes());
        }
        for (id, name) in [
            (PRIMITIVE_VOID_ID, &b"void"[..]),
            (PRIMITIVE_BOOLEAN_ID, &b"boolean"[..]),
            (PRIMITIVE_BYTE_ID, &b"byte"[..]),
            (PRIMITIVE_SHORT_ID, &b"short"[..]),
            (PRIMITIVE_CHAR_ID, &b"char"[..]),
            (PRIMITIVE_INT_ID, &b"int"[..]),
            (PRIMITIVE_LONG_ID, &b"long"[..]),
            (PRIMITIVE_FLOAT_ID, &b"float"[..]),
            (PRIMITIVE_DOUBLE_ID, &b"double"[..]),
        ] {
            stub_names.insert(id, name);
        }

        // ── Containment from parent_sym, ordered by declaration position ──
        let mut child_pairs: Vec<(u32, u32)> = Vec::with_capacity(sym_count);
        for (i, sym) in sta.symbol_records.iter().enumerate() {
            if sym.parent_sym != u32::MAX && (sym.parent_sym as usize) < sym_count {
                if let Ok(id) = u32::try_from(i) {
                    if sym.parent_sym != id {
                        child_pairs.push((sym.parent_sym, id));
                    }
                }
            }
        }
        child_pairs.sort_by_key(|&(p, c)| {
            let tok = sta.symbol_records[c as usize].first_token_id;
            (p, tok, c)
        });
        let children = Csr::from_pairs(sym_count, &child_pairs);

        let mut index = Self {
            sta,
            tca,
            cga,
            tra,
            scanner,
            sym_count,
            children,
            extends: Csr::default(),
            implements: Csr::default(),
            subtypes: Csr::default(),
            superclass: vec![UNKNOWN_SYM; sym_count],
            uses: Csr::default(),
            sites_by_caller: Csr::default(),
            site_callees: Csr::default(),
            callees_of: Csr::default(),
            callers_of: Csr::default(),
            site_order_key: Vec::new(),
            uml_link_order: Vec::new(),
            stub_names,
            canonical_ext: HashMap::new(),
            dropped_supertype_edges: 0,
            recovered_supertype_edges: 0,
            dropped_call_edges: 0,
        };

        index.build_canonical_externals();
        index.build_hierarchy();
        index.build_calls();
        index.build_uml_links();
        index
    }

    // ── Construction ──────────────────────────────────────────────────────────

    fn qual_name_text(&self, qual_name_id: u32) -> Option<&'a str> {
        if qual_name_id == u32::MAX {
            return None;
        }
        let base = crate::symbol::QualifiedNameTable::BASE_QUAL_NAME_ID;
        let idx = if qual_name_id >= base {
            (qual_name_id - base) as usize
        } else {
            qual_name_id as usize
        };
        self.sta.qual_names.get(idx).map(|s| s.as_str())
    }

    /// Phase 3 turns every imported type name into an SK_EXTERNAL placeholder, even
    /// when the import names a type declared in another analyzed file. Map such
    /// placeholders to the declaration: by exact qualified name, or by a unique
    /// simple name whose package does not contradict the import.
    fn build_canonical_externals(&mut self) {
        let sta = self.sta;
        for (i, sym) in sta.symbol_records.iter().enumerate() {
            if sym.kind != SymbolKind::SK_EXTERNAL as u8 || sym.name_id == u32::MAX {
                continue;
            }
            let candidates: Vec<u32> = self
                .symbols_named(sym.name_id)
                .into_iter()
                .filter(|&c| self.is_user_type(c))
                .collect();
            if candidates.is_empty() {
                continue;
            }
            let qual = self.qual_name_text(sym.qual_name_id);
            let exact = qual.and_then(|q| {
                candidates
                    .iter()
                    .copied()
                    .find(|&c| self.qualified_name(c) == q)
            });
            let chosen = match exact {
                Some(c) => Some(c),
                None if candidates.len() == 1 => {
                    let c = candidates[0];
                    let import_pkg = qual.and_then(|q| q.rsplit_once('.').map(|(p, _)| p));
                    match (import_pkg, self.package_name_of(c)) {
                        // `ip` may also name an enclosing class: `pkg.Outer` for `pkg.Outer.Inner`.
                        (Some(ip), Some(cp))
                            if ip != cp && !ip.starts_with(&format!("{}.", cp)) =>
                        {
                            None
                        }
                        _ => Some(c),
                    }
                }
                None => None,
            };
            if let Some(c) = chosen {
                if let Ok(id) = u32::try_from(i) {
                    self.canonical_ext.insert(id, c);
                }
            }
        }
    }

    /// The declaration a type reference denotes (identity for non-placeholders).
    pub fn canon(&self, id: u32) -> u32 {
        self.canonical_ext.get(&id).copied().unwrap_or(id)
    }

    fn build_hierarchy(&mut self) {
        let sta = self.sta;
        let mut raw: Vec<(u32, u32, u8)> = Vec::new();
        let mut uses_pairs: Vec<(u32, u32)> = Vec::new();
        for e in &sta.th_edges {
            let to = self.canon(e.to_sym);
            if e.from_sym == to || (e.from_sym as usize) >= self.sym_count {
                continue;
            }
            match e.relation {
                THRelation::TH_EXTENDS => raw.push((e.from_sym, to, 0)),
                THRelation::TH_IMPLEMENTS => raw.push((e.from_sym, to, 1)),
                THRelation::TH_USES | THRelation::TH_CREATES => {
                    if (to as usize) < self.sym_count {
                        uses_pairs.push((e.from_sym, to));
                    }
                }
            }
        }
        raw.sort_unstable();
        raw.dedup();
        uses_pairs.sort_unstable();
        uses_pairs.dedup();
        self.uses = Csr::from_pairs(self.sym_count, &uses_pairs);

        // (from, to, is_extends, order_key)
        let mut validated: Vec<(u32, u32, bool, u32)> = Vec::new();
        let mut start = 0usize;
        while start < raw.len() {
            let from = raw[start].0;
            let mut end = start;
            while end < raw.len() && raw[end].0 == from {
                end += 1;
            }
            let group = &raw[start..end];
            self.validate_supertypes(from, group, &mut validated);
            start = end;
        }

        // Header supertypes that Phase 5 did not record at all.
        let mut has_group: HashSet<u32> = raw.iter().map(|r| r.0).collect();
        for sym_id in 0..self.sym_count {
            let id = sym_id as u32;
            if has_group.contains(&id) || !self.is_user_type(id) {
                continue;
            }
            if let Some(declared) = self.declared_header(id) {
                if !declared.is_empty() {
                    self.recover_header_supertypes(id, &declared, &[], &mut validated);
                    has_group.insert(id);
                }
            }
        }

        validated.sort_by_key(|&(f, t, is_ext, k)| (f, k, t, is_ext));
        validated.dedup_by_key(|v| (v.0, v.1, v.2));
        let mut ext_pairs = Vec::new();
        let mut impl_pairs = Vec::new();
        let mut sub_pairs = Vec::new();
        for &(from, to, is_ext, _) in &validated {
            if is_ext {
                ext_pairs.push((from, to));
            } else {
                impl_pairs.push((from, to));
            }
            if (to as usize) < self.sym_count {
                sub_pairs.push((to, from));
            }
        }
        sub_pairs.sort_unstable();
        sub_pairs.dedup();
        self.extends = Csr::from_pairs(self.sym_count, &ext_pairs);
        self.implements = Csr::from_pairs(self.sym_count, &impl_pairs);
        self.subtypes = Csr::from_pairs(self.sym_count, &sub_pairs);

        // Unique superclass per class-kind declaration (JLS 8.1.4).
        for sym_id in 0..self.sym_count {
            let id = sym_id as u32;
            let kind = match self.kind(id) {
                Some(k) => k,
                None => continue,
            };
            if !matches!(
                kind,
                SymbolKind::SK_CLASS | SymbolKind::SK_ENUM | SymbolKind::SK_RECORD
            ) {
                continue;
            }
            let candidates: Vec<u32> = self
                .extends
                .get(id)
                .iter()
                .copied()
                .filter(|&t| !self.is_interface(t))
                .collect();
            if candidates.len() == 1 || (!candidates.is_empty() && self.header_validated(id)) {
                self.superclass[sym_id] = candidates[0];
            } else if candidates.len() > 1 {
                crate::core::logger::log_debug(&format!(
                    "[UMA] Ambiguous superclass for sym {}: {} candidates, none selected",
                    id,
                    candidates.len()
                ));
            }
        }
    }

    fn header_validated(&self, sym: u32) -> bool {
        self.declared_header(sym).is_some()
    }

    fn declared_header(&self, sym: u32) -> Option<DeclaredSupertypes> {
        let rec = self.sta.symbol(sym)?;
        if rec.first_token_id == u32::MAX {
            return None;
        }
        let lang = self.language_of_sym(sym)?;
        self.scanner.declared_supertypes(rec, lang)
    }

    fn validate_supertypes(
        &mut self,
        from: u32,
        group: &[(u32, u32, u8)],
        out: &mut Vec<(u32, u32, bool, u32)>,
    ) {
        let from_is_interface = self.is_interface(from);
        let declared = self.declared_header(from);
        match declared {
            Some(d) => {
                let mut matched: Vec<u32> = Vec::new();
                for &(_, to, _) in group {
                    let target = self.name_bytes(to);
                    if target.is_empty() {
                        self.dropped_supertype_edges += 1;
                        continue;
                    }
                    if let Some(p) = self.position_of_name(&d.extends, target) {
                        matched.push(d.extends[p]);
                        out.push((from, to, true, p as u32));
                    } else if let Some(p) = self.position_of_name(&d.implements, target) {
                        matched.push(d.implements[p]);
                        out.push((from, to, from_is_interface, 1000 + p as u32));
                    } else if let Some(p) = d
                        .delegation
                        .iter()
                        .position(|&(tid, _)| self.tca.interner.lookup_text(tid) == target)
                    {
                        let (tid, called) = d.delegation[p];
                        matched.push(tid);
                        let is_ext = if from_is_interface {
                            true
                        } else if self.is_interface(to) {
                            false
                        } else {
                            called || self.is_user_type(to)
                        };
                        out.push((from, to, is_ext, p as u32));
                    } else {
                        // Written only inside type arguments or bounds, or not at all.
                        self.dropped_supertype_edges += 1;
                    }
                }
                self.recover_header_supertypes(from, &d, &matched, out);
            }
            None => {
                for &(_, to, rel) in group {
                    // JLS 9.1.3: an interface extends (never implements) its super-interfaces.
                    let is_ext = rel == 0 || (from_is_interface && self.is_interface(to));
                    out.push((from, to, is_ext, to));
                }
            }
        }
    }

    fn position_of_name(&self, list: &[u32], target: &[u8]) -> Option<usize> {
        list.iter()
            .position(|&tid| self.tca.interner.lookup_text(tid) == target)
    }

    /// Resolve header supertypes that have no matching TH edge.
    fn recover_header_supertypes(
        &mut self,
        from: u32,
        declared: &DeclaredSupertypes,
        already_matched: &[u32],
        out: &mut Vec<(u32, u32, bool, u32)>,
    ) {
        let from_is_interface = self.is_interface(from);
        let mut items: Vec<(u32, bool, u32)> = Vec::new();
        for (p, &tid) in declared.extends.iter().enumerate() {
            items.push((tid, true, p as u32));
        }
        for (p, &tid) in declared.implements.iter().enumerate() {
            items.push((tid, from_is_interface, 1000 + p as u32));
        }
        for (p, &(tid, called)) in declared.delegation.iter().enumerate() {
            items.push((tid, called || from_is_interface, p as u32));
        }
        for (tid, is_ext, order) in items {
            if already_matched.contains(&tid) {
                continue;
            }
            let target = self.resolve_type_text(tid, from);
            if target == UNKNOWN_SYM || target == from {
                continue;
            }
            let is_ext = if self.is_interface(target) && !from_is_interface {
                false
            } else {
                is_ext || from_is_interface
            };
            self.recovered_supertype_edges += 1;
            out.push((from, target, is_ext, order));
        }
    }

    fn build_calls(&mut self) {
        let cga = self.cga;
        let n_sites = cga.call_sites.len();

        let site_pos: HashMap<u32, u32> = cga
            .call_sites
            .iter()
            .enumerate()
            .filter_map(|(i, s)| u32::try_from(i).ok().map(|p| (s.call_site_id, p)))
            .collect();

        // Evaluation order: a call completes when its argument list closes.
        self.site_order_key = cga
            .call_sites
            .iter()
            .map(|s| {
                if s.call_token == u32::MAX {
                    u32::MAX
                } else {
                    self.scanner
                        .call_close_paren(s.call_token)
                        .unwrap_or(s.call_token)
                }
            })
            .collect();

        let mut by_caller: Vec<(u32, u32)> = cga
            .call_sites
            .iter()
            .enumerate()
            .filter(|(_, s)| (s.caller_sym as usize) < self.sym_count)
            .map(|(i, s)| (s.caller_sym, i as u32))
            .collect();
        by_caller.sort_by_key(|&(c, i)| (c, self.site_order_key[i as usize], i));
        self.sites_by_caller = Csr::from_pairs(self.sym_count, &by_caller);

        let mut site_pairs: Vec<(u32, u32)> = Vec::new();
        let mut method_pairs: Vec<(u32, u32)> = Vec::new();
        for &(caller, callee, site_id) in &cga.site_to_edge_map {
            let pos = match site_pos.get(&site_id) {
                Some(&p) => p,
                None => {
                    self.dropped_call_edges += 1;
                    continue;
                }
            };
            let site = &cga.call_sites[pos as usize];
            if site.caller_sym != caller || !self.call_target_is_credible(site, callee) {
                self.dropped_call_edges += 1;
                continue;
            }
            site_pairs.push((pos, callee));
            method_pairs.push((caller, callee));
        }
        site_pairs.sort_unstable();
        site_pairs.dedup();
        self.site_callees = Csr::from_pairs(n_sites, &site_pairs);

        let mut seen: HashSet<(u32, u32)> = HashSet::new();
        let mut callee_pairs = Vec::new();
        let mut caller_pairs = Vec::new();
        for &(caller, callee) in &method_pairs {
            if seen.insert((caller, callee)) {
                callee_pairs.push((caller, callee));
                if (callee as usize) < self.sym_count {
                    caller_pairs.push((callee, caller));
                }
            }
        }
        callee_pairs.sort_unstable();
        caller_pairs.sort_unstable();
        self.callees_of = Csr::from_pairs(self.sym_count, &callee_pairs);
        self.callers_of = Csr::from_pairs(self.sym_count, &caller_pairs);
    }

    /// A resolved callee is credible when its name is the identifier written at the
    /// call site (method name, or the class name for constructor calls).
    fn call_target_is_credible(&self, site: &CallSite, callee: u32) -> bool {
        let rec = match self.sta.symbol(callee) {
            Some(r) => r,
            None => return false,
        };
        let kind = SymbolKind::from(rec.kind);
        let callable = matches!(
            kind,
            SymbolKind::SK_METHOD | SymbolKind::SK_CONSTRUCTOR | SymbolKind::SK_LAMBDA
        ) || (site.call_type == CG_EDGE_CONSTRUCTOR && Self::is_type_kind(kind));
        if !callable {
            return false;
        }
        if site.call_token == u32::MAX || kind == SymbolKind::SK_LAMBDA {
            return true;
        }
        let written = self.scanner.text(site.call_token);
        if written.is_empty() {
            return true;
        }
        if self.name_bytes(callee) == written {
            return true;
        }
        if site.call_type == CG_EDGE_CONSTRUCTOR && kind == SymbolKind::SK_CONSTRUCTOR {
            let owner = self.enclosing_type(callee);
            return owner != UNKNOWN_SYM && self.name_bytes(owner) == written;
        }
        false
    }

    fn build_uml_links(&mut self) {
        let tra = match self.tra {
            Some(t) => t,
            None => return,
        };
        let mut order: Vec<u32> = (0..tra.uml_links.len() as u32).collect();
        order.sort_by_key(|&i| tra.uml_links[i as usize].sym_id);
        self.uml_link_order = order;
    }

    // ── Symbol queries ────────────────────────────────────────────────────────

    pub fn symbol_count(&self) -> usize {
        self.sym_count
    }

    pub fn symbol(&self, id: u32) -> Option<&'a SymbolRecord> {
        self.sta.symbol_records.get(id as usize)
    }

    pub fn kind(&self, id: u32) -> Option<SymbolKind> {
        self.symbol(id).map(|s| SymbolKind::from(s.kind))
    }

    pub fn is_type_kind(kind: SymbolKind) -> bool {
        matches!(
            kind,
            SymbolKind::SK_CLASS
                | SymbolKind::SK_INTERFACE
                | SymbolKind::SK_ENUM
                | SymbolKind::SK_RECORD
                | SymbolKind::SK_ANNOTATION_TYPE
        )
    }

    /// Declared in the analyzed sources (not a stub, primitive or external).
    pub fn is_user_type(&self, id: u32) -> bool {
        self.kind(id).map(Self::is_type_kind).unwrap_or(false)
    }

    pub fn is_interface(&self, id: u32) -> bool {
        self.kind(id) == Some(SymbolKind::SK_INTERFACE)
    }

    pub fn is_abstract(&self, id: u32) -> bool {
        self.symbol(id)
            .map(|s| (s.modifiers & SymbolModifiers::ABSTRACT) != 0)
            .unwrap_or(false)
    }

    /// Interface or abstract class declared in the sources.
    pub fn is_abstract_type(&self, id: u32) -> bool {
        self.is_user_type(id) && (self.is_interface(id) || self.is_abstract(id))
    }

    pub fn is_concrete_class(&self, id: u32) -> bool {
        matches!(
            self.kind(id),
            Some(SymbolKind::SK_CLASS) | Some(SymbolKind::SK_ENUM) | Some(SymbolKind::SK_RECORD)
        ) && !self.is_abstract(id)
    }

    pub fn is_primitive(id: u32) -> bool {
        (PRIMITIVE_VOID_ID..=PRIMITIVE_DOUBLE_ID).contains(&id)
    }

    pub fn is_void(id: u32) -> bool {
        id == PRIMITIVE_VOID_ID
    }

    pub fn is_boolean(id: u32) -> bool {
        id == PRIMITIVE_BOOLEAN_ID
    }

    /// Primitive, standard-library stub or unresolved external type.
    pub fn is_library_type(&self, id: u32) -> bool {
        if id == UNKNOWN_SYM {
            return false;
        }
        Self::is_primitive(id)
            || id >= BASE_STUB_SYMBOL_ID
            || self.kind(id) == Some(SymbolKind::SK_EXTERNAL)
    }

    /// Library collection/container type (never a user-declared type).
    pub fn is_collection_type(&self, id: u32) -> bool {
        if matches!(
            id,
            LIST_STUB_SYM_ID
                | MAP_STUB_SYM_ID
                | SET_STUB_SYM_ID
                | COLLECTION_STUB_SYM_ID
                | ITERABLE_STUB_SYM_ID
        ) {
            return true;
        }
        if self.kind(id) == Some(SymbolKind::SK_EXTERNAL) {
            let name = self.name_bytes(id);
            return LIBRARY_COLLECTION_NAMES.contains(&name);
        }
        false
    }

    /// Simple name bytes of a symbol, stub or primitive (`b""` when unknown).
    pub fn name_bytes(&self, id: u32) -> &'a [u8] {
        if let Some(sym) = self.symbol(id) {
            return self.tca.interner.lookup_text(sym.name_id);
        }
        self.stub_names.get(&id).copied().unwrap_or(b"")
    }

    pub fn name(&self, id: u32) -> &'a str {
        std::str::from_utf8(self.name_bytes(id)).unwrap_or("")
    }

    pub fn name_id(&self, id: u32) -> u32 {
        self.symbol(id).map(|s| s.name_id).unwrap_or(u32::MAX)
    }

    /// Children (declared members) of a symbol in declaration order.
    pub fn children(&self, id: u32) -> &[u32] {
        self.children.get(id)
    }

    pub fn children_of_kind(&self, id: u32, kind: SymbolKind) -> impl Iterator<Item = u32> + '_ {
        self.children(id)
            .iter()
            .copied()
            .filter(move |&c| self.kind(c) == Some(kind))
    }

    pub fn fields(&self, id: u32) -> Vec<u32> {
        self.children_of_kind(id, SymbolKind::SK_FIELD).collect()
    }

    pub fn methods(&self, id: u32) -> Vec<u32> {
        self.children_of_kind(id, SymbolKind::SK_METHOD).collect()
    }

    pub fn constructors(&self, id: u32) -> Vec<u32> {
        self.children_of_kind(id, SymbolKind::SK_CONSTRUCTOR)
            .collect()
    }

    pub fn params(&self, method: u32) -> Vec<u32> {
        self.children_of_kind(method, SymbolKind::SK_PARAM)
            .collect()
    }

    pub fn param_types(&self, method: u32) -> Vec<u32> {
        self.params(method)
            .iter()
            .map(|&p| self.type_of(p))
            .collect()
    }

    /// Declared (canonical) type of a field, parameter, variable or method result.
    pub fn type_of(&self, id: u32) -> u32 {
        match self.symbol(id) {
            Some(s) if s.type_id != u32::MAX => self.canon(s.type_id),
            _ => UNKNOWN_SYM,
        }
    }

    pub fn is_static(&self, id: u32) -> bool {
        self.symbol(id)
            .map(|s| (s.modifiers & SymbolModifiers::STATIC) != 0)
            .unwrap_or(false)
    }

    pub fn is_final(&self, id: u32) -> bool {
        self.symbol(id)
            .map(|s| (s.modifiers & SymbolModifiers::FINAL) != 0)
            .unwrap_or(false)
    }

    pub fn visibility(&self, id: u32) -> u8 {
        self.symbol(id).map(|s| s.visibility).unwrap_or(0)
    }

    /// Nearest enclosing class-like declaration (excluding `id` itself).
    pub fn enclosing_type(&self, id: u32) -> u32 {
        let mut cur = self.symbol(id).map(|s| s.parent_sym).unwrap_or(u32::MAX);
        let mut guard = 0;
        while cur != u32::MAX && guard < 256 {
            if self.is_user_type(cur) {
                return cur;
            }
            cur = self.symbol(cur).map(|s| s.parent_sym).unwrap_or(u32::MAX);
            guard += 1;
        }
        UNKNOWN_SYM
    }

    /// Nearest enclosing method/constructor/initializer (lambdas are transparent).
    pub fn enclosing_member(&self, id: u32) -> u32 {
        let mut cur = id;
        let mut guard = 0;
        while cur != u32::MAX && guard < 256 {
            match self.kind(cur) {
                Some(SymbolKind::SK_METHOD)
                | Some(SymbolKind::SK_CONSTRUCTOR)
                | Some(SymbolKind::SK_STATIC_INIT)
                | Some(SymbolKind::SK_INSTANCE_INIT) => return cur,
                None => return UNKNOWN_SYM,
                _ => {}
            }
            cur = self.symbol(cur).map(|s| s.parent_sym).unwrap_or(u32::MAX);
            guard += 1;
        }
        UNKNOWN_SYM
    }

    /// Lambda symbols nested (transitively) in a member.
    pub fn nested_lambdas(&self, member: u32) -> Vec<u32> {
        let mut out = Vec::new();
        let mut stack: Vec<u32> = self.children(member).to_vec();
        let mut guard = 0usize;
        while let Some(c) = stack.pop() {
            guard += 1;
            if guard > 100_000 {
                break;
            }
            if self.kind(c) == Some(SymbolKind::SK_LAMBDA) {
                out.push(c);
                stack.extend_from_slice(self.children(c));
            } else if matches!(
                self.kind(c),
                Some(SymbolKind::SK_LOCAL_VAR) | Some(SymbolKind::SK_PARAM)
            ) {
                stack.extend_from_slice(self.children(c));
            }
        }
        out.sort_unstable();
        out
    }

    // ── Hierarchy queries ─────────────────────────────────────────────────────

    /// Unique superclass (UNKNOWN_SYM when none or ambiguous).
    pub fn superclass(&self, id: u32) -> u32 {
        self.superclass
            .get(id as usize)
            .copied()
            .unwrap_or(UNKNOWN_SYM)
    }

    /// Validated generalization targets (superclass, or super-interfaces of an interface).
    pub fn extends_of(&self, id: u32) -> &[u32] {
        self.extends.get(id)
    }

    /// Validated realization targets.
    pub fn implements_of(&self, id: u32) -> &[u32] {
        self.implements.get(id)
    }

    pub fn direct_supertypes(&self, id: u32) -> Vec<u32> {
        let mut v: Vec<u32> = self
            .extends_of(id)
            .iter()
            .chain(self.implements_of(id).iter())
            .copied()
            .collect();
        v.sort_unstable();
        v.dedup();
        v
    }

    pub fn direct_subtypes(&self, id: u32) -> &[u32] {
        self.subtypes.get(id)
    }

    /// All transitive supertypes (cycle-safe, excluding `id`).
    pub fn ancestors(&self, id: u32) -> Vec<u32> {
        let mut seen: HashSet<u32> = HashSet::new();
        let mut queue: VecDeque<u32> = self.direct_supertypes(id).into_iter().collect();
        let mut out = Vec::new();
        while let Some(t) = queue.pop_front() {
            if t == id || !seen.insert(t) {
                continue;
            }
            out.push(t);
            if (t as usize) < self.sym_count {
                queue.extend(self.direct_supertypes(t));
            }
        }
        out
    }

    /// All transitive subtypes (cycle-safe, excluding `id`).
    pub fn descendants(&self, id: u32) -> Vec<u32> {
        let mut seen: HashSet<u32> = HashSet::new();
        let mut queue: VecDeque<u32> = self.direct_subtypes(id).iter().copied().collect();
        let mut out = Vec::new();
        while let Some(t) = queue.pop_front() {
            if t == id || !seen.insert(t) {
                continue;
            }
            out.push(t);
            queue.extend(self.direct_subtypes(t).iter().copied());
        }
        out.sort_unstable();
        out
    }

    pub fn concrete_descendants(&self, id: u32) -> Vec<u32> {
        self.descendants(id)
            .into_iter()
            .filter(|&d| self.is_concrete_class(d))
            .collect()
    }

    pub fn is_subtype_of(&self, sub: u32, sup: u32) -> bool {
        sub == sup || self.ancestors(sub).contains(&sup)
    }

    /// True when `a` and `b` are in one inheritance line (either is a subtype of the other).
    pub fn related(&self, a: u32, b: u32) -> bool {
        self.is_subtype_of(a, b) || self.is_subtype_of(b, a)
    }

    /// Methods of `owner` with the same name and arity as `method`.
    pub fn same_signature_in(&self, method: u32, owner: u32) -> Option<u32> {
        let m = self.symbol(method)?;
        self.children(owner).iter().copied().find(|&c| {
            self.kind(c) == Some(SymbolKind::SK_METHOD)
                && self
                    .symbol(c)
                    .map(|s| s.name_id == m.name_id && s.param_count == m.param_count)
                    .unwrap_or(false)
        })
    }

    /// The supertype method that `method` overrides, if any (searches all ancestors).
    pub fn overridden_method(&self, method: u32) -> Option<u32> {
        let owner = self.enclosing_type(method);
        if owner == UNKNOWN_SYM {
            return None;
        }
        self.ancestors(owner)
            .into_iter()
            .filter(|&a| (a as usize) < self.sym_count)
            .find_map(|a| self.same_signature_in(method, a))
    }

    /// TH_USES/TH_CREATES targets inside the analyzed sources.
    pub fn uses_of(&self, id: u32) -> &[u32] {
        self.uses.get(id)
    }

    // ── Call graph queries ────────────────────────────────────────────────────

    /// Call sites of a caller symbol in evaluation order.
    pub fn sites_of(&self, caller: u32) -> impl Iterator<Item = &'a CallSite> + '_ {
        let cga = self.cga;
        self.sites_by_caller
            .get(caller)
            .iter()
            .map(move |&i| &cga.call_sites[i as usize])
    }

    pub fn site_indices_of(&self, caller: u32) -> &[u32] {
        self.sites_by_caller.get(caller)
    }

    pub fn site(&self, site_index: u32) -> &'a CallSite {
        &self.cga.call_sites[site_index as usize]
    }

    pub fn site_order_key(&self, site_index: u32) -> u32 {
        self.site_order_key
            .get(site_index as usize)
            .copied()
            .unwrap_or(u32::MAX)
    }

    /// Validated callees of a call site (by index into `cga.call_sites`).
    pub fn callees_at(&self, site_index: u32) -> &[u32] {
        self.site_callees.get(site_index)
    }

    /// Validated calls made by a member, including those inside its lambdas,
    /// in evaluation order.
    pub fn calls_of_member(&self, member: u32) -> Vec<ValidatedCall> {
        let mut callers = vec![member];
        callers.extend(self.nested_lambdas(member));
        let mut sites: Vec<u32> = callers
            .iter()
            .flat_map(|&c| self.site_indices_of(c).iter().copied())
            .collect();
        sites.sort_by_key(|&i| (self.site_order_key(i), i));
        let mut out = Vec::new();
        for s in sites {
            for &callee in self.callees_at(s) {
                out.push(ValidatedCall {
                    site_index: s,
                    callee,
                });
            }
        }
        out
    }

    pub fn callees_of(&self, method: u32) -> &[u32] {
        self.callees_of.get(method)
    }

    pub fn callers_of(&self, method: u32) -> &[u32] {
        self.callers_of.get(method)
    }

    // ── Source location queries ───────────────────────────────────────────────

    pub fn file_of_sym(&self, id: u32) -> Option<u16> {
        let sym = self.symbol(id)?;
        let rec = self.tca.token_records.get(sym.first_token_id as usize)?;
        Some(unpack_sort_key(rec.sort_key).0)
    }

    pub fn language_of_file(&self, fid: u16) -> Option<LangId> {
        self.tca
            .file_records
            .get(fid as usize)
            .map(|f| LangId::from(f.language_id))
    }

    pub fn language_of_sym(&self, id: u32) -> Option<LangId> {
        self.language_of_file(self.file_of_sym(id)?)
    }

    /// Lower-cased path with `/` separators (independent of the host OS).
    pub fn normalized_path(&self, fid: u16) -> Option<String> {
        let rec = self.tca.file_records.get(fid as usize)?;
        let bytes = self.tca.interner.lookup_text(rec.path_str_offset);
        let s = String::from_utf8_lossy(bytes);
        Some(s.replace('\\', "/").to_lowercase())
    }

    /// The file sits in a conventional test source root.
    pub fn is_test_file(&self, fid: u16) -> bool {
        let path = match self.normalized_path(fid) {
            Some(p) => format!("/{}", p.trim_start_matches('/')),
            None => return false,
        };
        const TEST_ROOTS: &[&str] = &[
            "/src/test/",
            "/src/tests/",
            "/src/testfixtures/",
            "/src/integrationtest/",
            "/src/androidtest/",
            "/__tests__/",
        ];
        TEST_ROOTS.iter().any(|root| path.contains(root))
    }

    /// TRA UMLLink for a symbol, if TRA recorded one.
    pub fn uml_link(&self, id: u32) -> Option<UMLLinkRecord> {
        let tra = self.tra?;
        let pos = self
            .uml_link_order
            .binary_search_by_key(&id, |&i| tra.uml_links[i as usize].sym_id)
            .ok()?;
        Some(tra.uml_links[self.uml_link_order[pos] as usize])
    }

    /// Package symbol enclosing a declaration (UNKNOWN_SYM when none).
    pub fn package_of(&self, id: u32) -> u32 {
        let mut cur = self.symbol(id).map(|s| s.parent_sym).unwrap_or(u32::MAX);
        let mut guard = 0;
        while cur != u32::MAX && guard < 256 {
            if self.kind(cur) == Some(SymbolKind::SK_PACKAGE) {
                return cur;
            }
            cur = self.symbol(cur).map(|s| s.parent_sym).unwrap_or(u32::MAX);
            guard += 1;
        }
        UNKNOWN_SYM
    }

    /// Dotted package name of a declaration (`None` when unknown).
    pub fn package_name_of(&self, id: u32) -> Option<String> {
        let pkg = self.package_of(id);
        if pkg != UNKNOWN_SYM {
            if let Some(name) = self.sta.custom_package_names.get(&pkg) {
                return Some(name.clone());
            }
        }
        let fid = self.file_of_sym(id)?;
        self.sta.file_package_names.get(&fid).cloned()
    }

    /// Fully-qualified name: package, enclosing types and simple name.
    pub fn qualified_name(&self, id: u32) -> String {
        let mut parts: Vec<&str> = vec![self.name(id)];
        let mut cur = self.enclosing_type(id);
        let mut guard = 0;
        while cur != UNKNOWN_SYM && guard < 64 {
            parts.push(self.name(cur));
            cur = self.enclosing_type(cur);
            guard += 1;
        }
        parts.reverse();
        let local = parts.join(".");
        match self.package_name_of(id) {
            Some(pkg) if !pkg.is_empty() => format!("{}.{}", pkg, local),
            _ => local,
        }
    }

    // ── Name resolution ───────────────────────────────────────────────────────

    /// Symbols whose simple name has the given interned text id.
    pub fn symbols_named(&self, text_id: u32) -> Vec<u32> {
        let idx = &self.sta.name_index;
        let start = idx.partition_point(|&(n, _)| n < text_id);
        let mut out = Vec::new();
        let mut i = start;
        while i < idx.len() && idx[i].0 == text_id {
            out.push(idx[i].1);
            i += 1;
        }
        out
    }

    /// Resolve a written type name to a user-declared type, preferring types nested
    /// in the context's enclosing types, then the same file, then the same package.
    /// Returns UNKNOWN_SYM when no candidate exists or the name stays ambiguous.
    pub fn resolve_type_text(&self, text_id: u32, context: u32) -> u32 {
        if text_id == u32::MAX {
            return UNKNOWN_SYM;
        }
        let mut candidates: Vec<u32> = self
            .symbols_named(text_id)
            .into_iter()
            .filter(|&c| self.is_user_type(c))
            .collect();
        candidates.sort_unstable();
        candidates.dedup();
        match candidates.len() {
            0 => return UNKNOWN_SYM,
            1 => return candidates[0],
            _ => {}
        }
        let mut scopes = Vec::new();
        let mut cur = if self.is_user_type(context) {
            context
        } else {
            self.enclosing_type(context)
        };
        while cur != UNKNOWN_SYM && scopes.len() < 64 {
            scopes.push(cur);
            cur = self.enclosing_type(cur);
        }
        for scope in &scopes {
            let nested: Vec<u32> = candidates
                .iter()
                .copied()
                .filter(|&c| self.symbol(c).map(|s| s.parent_sym) == Some(*scope))
                .collect();
            if nested.len() == 1 {
                return nested[0];
            }
        }
        let ctx_file = self.file_of_sym(context);
        if ctx_file.is_some() {
            let same_file: Vec<u32> = candidates
                .iter()
                .copied()
                .filter(|&c| self.file_of_sym(c) == ctx_file)
                .collect();
            if same_file.len() == 1 {
                return same_file[0];
            }
        }
        let ctx_pkg = self.package_name_of(context);
        if ctx_pkg.is_some() {
            let same_pkg: Vec<u32> = candidates
                .iter()
                .copied()
                .filter(|&c| self.package_name_of(c) == ctx_pkg)
                .collect();
            if same_pkg.len() == 1 {
                return same_pkg[0];
            }
        }
        UNKNOWN_SYM
    }
}
