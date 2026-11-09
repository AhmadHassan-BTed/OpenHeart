//! Symbol-table index for lowering: member lists, declaration-node lookup, the type
//! hierarchy, and the scoped lookups compilers perform for types, fields, methods and
//! constructors.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::core::types::symbol::{
    SymbolKind, SymbolModifiers, SymbolRecord, SymbolVisibility, THRelation,
};
use crate::core::types::token::unpack_sort_key;
use crate::ingestion::serializer::TokenCorpusArtifact;
use crate::symbol::qual_name_table::QualifiedNameTable;
use crate::symbol::serializer::SymbolTableArtifact;

pub struct SymbolIndex<'a> {
    pub sta: &'a SymbolTableArtifact,
    members: Vec<Vec<u32>>,
    by_node: HashMap<u32, u32>,
    top_level_by_file: HashMap<(u16, u32), Vec<u32>>,
    by_package: HashMap<(String, u32), Vec<u32>>,
    by_qual_name: HashMap<String, u32>,
    supers: Vec<Vec<(u32, THRelation)>>,
    file_of: Vec<Option<u16>>,
}

/// Symbol kinds that declare types.
pub fn is_type_kind(k: SymbolKind) -> bool {
    matches!(
        k,
        SymbolKind::SK_CLASS
            | SymbolKind::SK_INTERFACE
            | SymbolKind::SK_ENUM
            | SymbolKind::SK_RECORD
            | SymbolKind::SK_ANNOTATION_TYPE
            | SymbolKind::SK_ANON_CLASS
    )
}

/// Symbol kinds that own executable bodies.
pub fn is_function_kind(k: SymbolKind) -> bool {
    matches!(
        k,
        SymbolKind::SK_METHOD
            | SymbolKind::SK_CONSTRUCTOR
            | SymbolKind::SK_STATIC_INIT
            | SymbolKind::SK_INSTANCE_INIT
            | SymbolKind::SK_LAMBDA
    )
}

impl<'a> SymbolIndex<'a> {
    pub fn build(sta: &'a SymbolTableArtifact, tca: &'a TokenCorpusArtifact) -> Self {
        let n = sta.symbol_records.len();
        let mut members: Vec<Vec<u32>> = vec![Vec::new(); n];
        let mut by_node: HashMap<u32, u32> = HashMap::with_capacity(n);
        let mut file_of: Vec<Option<u16>> = vec![None; n];

        for (i, s) in sta.symbol_records.iter().enumerate() {
            let id = i as u32;
            if (s.parent_sym as usize) < n && s.parent_sym != id {
                members[s.parent_sym as usize].push(id);
            }
            if s.decl_node != u32::MAX {
                by_node.entry(s.decl_node).or_insert(id);
            }
            if s.first_token_id != u32::MAX {
                file_of[i] = tca
                    .token_records
                    .get(s.first_token_id as usize)
                    .map(|r| unpack_sort_key(r.sort_key).0);
            }
        }
        for list in members.iter_mut() {
            list.sort_by_key(|&m| (sta.symbol_records[m as usize].first_token_id, m));
        }

        let mut supers: Vec<Vec<(u32, THRelation)>> = vec![Vec::new(); n];
        for e in &sta.th_edges {
            if matches!(e.relation, THRelation::TH_EXTENDS | THRelation::TH_IMPLEMENTS)
                && (e.from_sym as usize) < n
                && e.from_sym != e.to_sym
            {
                let list = &mut supers[e.from_sym as usize];
                if !list.iter().any(|&(t, r)| t == e.to_sym && r == e.relation) {
                    list.push((e.to_sym, e.relation));
                }
            }
        }

        let mut top_level_by_file: HashMap<(u16, u32), Vec<u32>> = HashMap::new();
        let mut by_package: HashMap<(String, u32), Vec<u32>> = HashMap::new();
        let mut by_qual_name: HashMap<String, u32> = HashMap::new();
        for (i, s) in sta.symbol_records.iter().enumerate() {
            let kind = SymbolKind::from(s.kind);
            if !is_type_kind(kind) || kind == SymbolKind::SK_ANON_CLASS {
                continue;
            }
            let id = i as u32;
            let parent_is_type = (s.parent_sym as usize) < n
                && is_type_kind(SymbolKind::from(sta.symbol_records[s.parent_sym as usize].kind));
            let parent_is_function = (s.parent_sym as usize) < n
                && is_function_kind(SymbolKind::from(
                    sta.symbol_records[s.parent_sym as usize].kind,
                ));
            if !parent_is_type && !parent_is_function {
                if let Some(f) = file_of[i] {
                    top_level_by_file.entry((f, s.name_id)).or_default().push(id);
                    if let Some(pkg) = sta.file_package_names.get(&f) {
                        by_package
                            .entry((pkg.clone(), s.name_id))
                            .or_default()
                            .push(id);
                    }
                }
            }
            if let Some(q) = qual_name(sta, s.qual_name_id) {
                by_qual_name.entry(q.to_string()).or_insert(id);
            }
        }

        Self {
            sta,
            members,
            by_node,
            top_level_by_file,
            by_package,
            by_qual_name,
            supers,
            file_of,
        }
    }

    #[inline]
    pub fn count(&self) -> u32 {
        self.sta.symbol_records.len() as u32
    }

    #[inline]
    pub fn symbol(&self, s: u32) -> Option<&'a SymbolRecord> {
        let sta: &'a SymbolTableArtifact = self.sta;
        sta.symbol_records.get(s as usize)
    }

    pub fn kind(&self, s: u32) -> Option<SymbolKind> {
        self.symbol(s).map(|r| SymbolKind::from(r.kind))
    }

    pub fn is_type(&self, s: u32) -> bool {
        self.kind(s).is_some_and(is_type_kind)
    }

    pub fn is_interface(&self, s: u32) -> bool {
        matches!(
            self.kind(s),
            Some(SymbolKind::SK_INTERFACE | SymbolKind::SK_ANNOTATION_TYPE)
        )
    }

    pub fn is_function(&self, s: u32) -> bool {
        self.kind(s).is_some_and(is_function_kind)
    }

    pub fn is_field(&self, s: u32) -> bool {
        matches!(
            self.kind(s),
            Some(SymbolKind::SK_FIELD | SymbolKind::SK_ENUM_CONSTANT)
        )
    }

    /// Static members, and enum constants (static by definition of the kind).
    pub fn is_static(&self, s: u32) -> bool {
        match self.symbol(s) {
            Some(r) => {
                r.modifiers & SymbolModifiers::STATIC != 0
                    || SymbolKind::from(r.kind) == SymbolKind::SK_ENUM_CONSTANT
            }
            None => false,
        }
    }

    pub fn is_abstract(&self, s: u32) -> bool {
        self.symbol(s)
            .is_some_and(|r| r.modifiers & SymbolModifiers::ABSTRACT != 0)
    }

    pub fn is_varargs(&self, s: u32) -> bool {
        self.symbol(s)
            .is_some_and(|r| r.modifiers & SymbolModifiers::VARARGS != 0)
    }

    pub fn is_private(&self, s: u32) -> bool {
        self.symbol(s)
            .is_some_and(|r| SymbolVisibility::from(r.visibility) == SymbolVisibility::Private)
    }

    /// Interned simple name (corpus text id), `u32::MAX` when absent.
    pub fn name(&self, s: u32) -> u32 {
        self.symbol(s).map(|r| r.name_id).unwrap_or(u32::MAX)
    }

    pub fn parent(&self, s: u32) -> Option<u32> {
        match self.symbol(s) {
            Some(r) if (r.parent_sym as usize) < self.sta.symbol_records.len() => {
                Some(r.parent_sym)
            }
            _ => None,
        }
    }

    pub fn members(&self, s: u32) -> &[u32] {
        self.members.get(s as usize).map(|m| m.as_slice()).unwrap_or(&[])
    }

    /// Symbol declared by an AST node.
    pub fn at_node(&self, node: u32) -> Option<u32> {
        self.by_node.get(&node).copied()
    }

    pub fn file_of(&self, s: u32) -> Option<u16> {
        self.file_of.get(s as usize).copied().flatten()
    }

    pub fn package_of_file(&self, file: u16) -> Option<&'a str> {
        let sta: &'a SymbolTableArtifact = self.sta;
        sta.file_package_names.get(&file).map(|s| s.as_str())
    }

    /// Declared type of a member as recorded by the symbol table: a type symbol id.
    pub fn declared_type_symbol(&self, s: u32) -> Option<u32> {
        let t = self.symbol(s)?.type_id;
        if self.is_type(t) || self.kind(t) == Some(SymbolKind::SK_TYPE_PARAM) {
            Some(t)
        } else {
            None
        }
    }

    /// Nearest enclosing type of a symbol (exclusive).
    pub fn enclosing_type(&self, s: u32) -> Option<u32> {
        let mut cur = self.parent(s);
        let mut guard = 0usize;
        while let Some(p) = cur {
            if self.is_type(p) {
                return Some(p);
            }
            guard += 1;
            if guard > self.sta.symbol_records.len() {
                return None;
            }
            cur = self.parent(p);
        }
        None
    }

    /// Nearest enclosing function of a symbol (exclusive).
    pub fn enclosing_function(&self, s: u32) -> Option<u32> {
        let mut cur = self.parent(s);
        let mut guard = 0usize;
        while let Some(p) = cur {
            if self.is_function(p) {
                return Some(p);
            }
            if self.is_type(p) {
                return None;
            }
            guard += 1;
            if guard > self.sta.symbol_records.len() {
                return None;
            }
            cur = self.parent(p);
        }
        None
    }

    pub fn supertypes(&self, s: u32) -> &[(u32, THRelation)] {
        self.supers.get(s as usize).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// Direct superclass (the extended type that is not an interface) of a class.
    pub fn superclass(&self, s: u32) -> Option<u32> {
        if self.is_interface(s) {
            return None;
        }
        self.supertypes(s)
            .iter()
            .find(|&&(t, r)| r == THRelation::TH_EXTENDS && !self.is_interface(t))
            .map(|&(t, _)| t)
    }

    /// `s` and all of its supertypes in breadth-first order (nearest first).
    pub fn ancestors(&self, s: u32) -> Vec<u32> {
        let mut out = vec![s];
        let mut seen: HashSet<u32> = HashSet::new();
        seen.insert(s);
        let mut queue: VecDeque<u32> = VecDeque::new();
        queue.push_back(s);
        while let Some(c) = queue.pop_front() {
            for &(t, _) in self.supertypes(c) {
                if seen.insert(t) {
                    out.push(t);
                    queue.push_back(t);
                }
            }
        }
        out
    }

    /// `sub` is `sup` or one of its subtypes.
    pub fn is_subtype(&self, sub: u32, sup: u32) -> bool {
        sub == sup || self.ancestors(sub).contains(&sup)
    }

    fn members_of_kind(&self, owner: u32, pred: impl Fn(SymbolKind) -> bool) -> Vec<u32> {
        self.members(owner)
            .iter()
            .copied()
            .filter(|&m| self.kind(m).is_some_and(&pred))
            .collect()
    }

    /// Formal parameters of a function in declaration order.
    pub fn params(&self, f: u32) -> Vec<u32> {
        self.members_of_kind(f, |k| k == SymbolKind::SK_PARAM)
    }

    pub fn type_params(&self, owner: u32) -> Vec<u32> {
        self.members_of_kind(owner, |k| k == SymbolKind::SK_TYPE_PARAM)
    }

    pub fn constructors(&self, class: u32) -> Vec<u32> {
        self.members_of_kind(class, |k| k == SymbolKind::SK_CONSTRUCTOR)
    }

    /// Field named `name` visible as a member of `owner`: declared by it or inherited
    /// (the nearest declaration wins). Returns the field and its declaring type.
    pub fn field_in_hierarchy(&self, owner: u32, name: u32) -> Option<(u32, u32)> {
        for t in self.ancestors(owner) {
            for &m in self.members(t) {
                if self.is_field(m) && self.name(m) == name {
                    return Some((m, t));
                }
            }
        }
        None
    }

    /// Member type named `name` of `owner` or of one of its supertypes.
    pub fn member_type_in_hierarchy(&self, owner: u32, name: u32) -> Option<u32> {
        for t in self.ancestors(owner) {
            for &m in self.members(t) {
                if self.is_type(m) && self.name(m) == name && self.kind(m) != Some(SymbolKind::SK_ANON_CLASS) {
                    return Some(m);
                }
            }
        }
        None
    }

    /// Methods named `name` that are members of `owner` (declared or inherited). A method
    /// overridden by a nearer declaration with the same parameter types is dropped.
    pub fn methods_in_hierarchy(&self, owner: u32, name: u32) -> Vec<u32> {
        let mut out: Vec<u32> = Vec::new();
        let mut signatures: Vec<Vec<u32>> = Vec::new();
        for t in self.ancestors(owner) {
            for &m in self.members(t) {
                if self.kind(m) != Some(SymbolKind::SK_METHOD) || self.name(m) != name {
                    continue;
                }
                let sig = self.signature(m);
                if signatures.iter().any(|s| *s == sig) {
                    continue;
                }
                signatures.push(sig);
                out.push(m);
            }
        }
        out
    }

    /// Parameter type symbols of a method (`u32::MAX` for unknown types).
    pub fn signature(&self, m: u32) -> Vec<u32> {
        self.params(m)
            .into_iter()
            .map(|p| self.symbol(p).map(|r| r.type_id).unwrap_or(u32::MAX))
            .collect()
    }

    /// Number of declared parameters.
    pub fn arity(&self, m: u32) -> usize {
        let params = self.params(m).len();
        if params > 0 {
            params
        } else {
            self.symbol(m).map(|r| r.param_count as usize).unwrap_or(0)
        }
    }

    /// Type parameter named `name` declared by `owner`.
    pub fn type_param_named(&self, owner: u32, name: u32) -> Option<u32> {
        self.type_params(owner)
            .into_iter()
            .find(|&p| self.name(p) == name)
    }

    /// Top-level types of a file named `name`.
    pub fn top_level_types(&self, file: u16, name: u32) -> &[u32] {
        self.top_level_by_file
            .get(&(file, name))
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Top-level types of a package named `name`.
    pub fn package_types(&self, package: &str, name: u32) -> &[u32] {
        self.by_package
            .get(&(package.to_string(), name))
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Type declared with the qualified name `qname`.
    pub fn qualified_type(&self, qname: &str) -> Option<u32> {
        self.by_qual_name.get(qname).copied()
    }

    /// Qualified name of a symbol.
    pub fn qualified_name(&self, s: u32) -> Option<&'a str> {
        let r = self.symbol(s)?;
        let sta: &'a SymbolTableArtifact = self.sta;
        qual_name(sta, r.qual_name_id)
    }
}

fn qual_name(sta: &SymbolTableArtifact, id: u32) -> Option<&str> {
    if id == u32::MAX {
        return None;
    }
    let raw = if id >= QualifiedNameTable::BASE_QUAL_NAME_ID {
        id - QualifiedNameTable::BASE_QUAL_NAME_ID
    } else {
        id
    };
    sta.qual_names
        .get(raw as usize)
        .map(|s| s.as_str())
        .filter(|s| !s.is_empty())
}
