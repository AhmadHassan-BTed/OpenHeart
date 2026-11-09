//! Program-wide indexes for Phase 6, built once per run.
//!
//! Every query that used to rescan the whole symbol table per call site — enclosing
//! function of a node, members of a class by name, supertypes and subtypes, types by
//! simple name — is answered from these maps in O(1) / O(answer).
//!
//! Class hierarchy: `TH_EXTENDS` / `TH_IMPLEMENTS` edges of the symbol table
//! (`from_sym` is the subtype). Method selection is by simple name and arity; overloads
//! of equal arity cannot be told apart without argument types and are all kept.

use crate::ast::BPASTArtifact;
use crate::cfg::ast_query::{is_type_decl, subtree_end};
use crate::cfg::is_callable_kind;
use crate::cfg::source::SourceText;
use crate::core::types::symbol::{SymbolKind, SymbolModifiers, SymbolRecord, THRelation};
use crate::symbol::SymbolTableArtifact;
use std::collections::{HashMap, HashSet, VecDeque};

#[inline]
pub fn is_class_like(kind: u8) -> bool {
    kind == SymbolKind::SK_CLASS as u8
        || kind == SymbolKind::SK_INTERFACE as u8
        || kind == SymbolKind::SK_ENUM as u8
        || kind == SymbolKind::SK_RECORD as u8
        || kind == SymbolKind::SK_ANNOTATION_TYPE as u8
        || kind == SymbolKind::SK_ANON_CLASS as u8
}

pub struct ProgramIndex<'a> {
    pub sta: &'a SymbolTableArtifact,
    pub bpa: &'a BPASTArtifact,
    pub src: Option<SourceText<'a>>,
    /// Innermost function symbol containing each AST node (`u32::MAX` outside functions).
    fn_of_node: Vec<u32>,
    sym_at_decl: HashMap<u32, u32>,
    /// (class, name) -> methods declared in the class.
    members: HashMap<(u32, u32), Vec<u32>>,
    /// class -> constructors declared in the class.
    constructors: HashMap<u32, Vec<u32>>,
    /// (class, field name) -> declared type symbol of the field.
    field_types: HashMap<(u32, u32), u32>,
    /// (class, field name) present (with or without a known type).
    field_names: HashSet<(u32, u32)>,
    /// name -> every method with that name (receiver type unknown).
    methods_by_name: HashMap<u32, Vec<u32>>,
    /// simple name -> class-like symbols (project and external).
    types_by_name: HashMap<u32, Vec<u32>>,
    /// class -> direct superclasses (`TH_EXTENDS`).
    extends: HashMap<u32, Vec<u32>>,
    /// class -> direct super-interfaces (`TH_IMPLEMENTS`, and interface `extends`).
    implements: HashMap<u32, Vec<u32>>,
    /// type -> direct subtypes.
    subtypes: HashMap<u32, Vec<u32>>,
    /// Every callable symbol (methods, constructors, initializers, lambdas).
    pub callables: Vec<u32>,
}

impl<'a> ProgramIndex<'a> {
    pub fn build(
        bpa: &'a BPASTArtifact,
        sta: &'a SymbolTableArtifact,
        src: Option<SourceText<'a>>,
    ) -> Self {
        let mut sym_at_decl = HashMap::new();
        let mut members: HashMap<(u32, u32), Vec<u32>> = HashMap::new();
        let mut constructors: HashMap<u32, Vec<u32>> = HashMap::new();
        let mut field_types = HashMap::new();
        let mut field_names = HashSet::new();
        let mut methods_by_name: HashMap<u32, Vec<u32>> = HashMap::new();
        let mut types_by_name: HashMap<u32, Vec<u32>> = HashMap::new();
        let mut callables = Vec::new();

        for sym in &sta.symbol_records {
            let id = sym.symbol_id;
            if sym.decl_node != u32::MAX {
                sym_at_decl.entry(sym.decl_node).or_insert(id);
            }
            if is_callable_kind(sym.kind) {
                callables.push(id);
            }
            if is_class_like(sym.kind) || sym.kind == SymbolKind::SK_EXTERNAL as u8 {
                if sym.name_id != u32::MAX {
                    types_by_name.entry(sym.name_id).or_default().push(id);
                }
            }
            if sym.kind == SymbolKind::SK_METHOD as u8 && sym.name_id != u32::MAX {
                members.entry((sym.parent_sym, sym.name_id)).or_default().push(id);
                methods_by_name.entry(sym.name_id).or_default().push(id);
            }
            if sym.kind == SymbolKind::SK_CONSTRUCTOR as u8 {
                constructors.entry(sym.parent_sym).or_default().push(id);
            }
            if sym.kind == SymbolKind::SK_FIELD as u8 && sym.name_id != u32::MAX {
                field_names.insert((sym.parent_sym, sym.name_id));
                if sym.type_id != u32::MAX {
                    field_types.insert((sym.parent_sym, sym.name_id), sym.type_id);
                }
            }
        }
        // Field declarators (`private Foo a, b;` registers `a` and `b` as locals whose
        // parent is the field symbol).
        for sym in &sta.symbol_records {
            if sym.kind != SymbolKind::SK_LOCAL_VAR as u8 || sym.name_id == u32::MAX {
                continue;
            }
            if let Some(field) = sta.symbol(sym.parent_sym) {
                if field.kind == SymbolKind::SK_FIELD as u8 && field.parent_sym != u32::MAX {
                    let key = (field.parent_sym, sym.name_id);
                    field_names.insert(key);
                    let ty = if sym.type_id != u32::MAX {
                        sym.type_id
                    } else {
                        field.type_id
                    };
                    if ty != u32::MAX {
                        field_types.insert(key, ty);
                    }
                }
            }
        }

        let mut extends: HashMap<u32, Vec<u32>> = HashMap::new();
        let mut implements: HashMap<u32, Vec<u32>> = HashMap::new();
        let mut subtypes: HashMap<u32, Vec<u32>> = HashMap::new();
        for edge in &sta.th_edges {
            if edge.from_sym == edge.to_sym {
                continue;
            }
            let target = match edge.relation {
                THRelation::TH_EXTENDS => {
                    // `interface A extends B` is a super-interface relation.
                    let from_is_interface = sta
                        .symbol(edge.from_sym)
                        .is_some_and(|s| s.kind == SymbolKind::SK_INTERFACE as u8);
                    if from_is_interface {
                        &mut implements
                    } else {
                        &mut extends
                    }
                }
                THRelation::TH_IMPLEMENTS => &mut implements,
                _ => continue,
            };
            let list = target.entry(edge.from_sym).or_default();
            if !list.contains(&edge.to_sym) {
                list.push(edge.to_sym);
                subtypes.entry(edge.to_sym).or_default().push(edge.from_sym);
            }
        }

        // Innermost enclosing function per node: outer functions start earlier in
        // preorder, so filling in increasing decl order lets inner ones overwrite.
        let mut fn_of_node = vec![u32::MAX; bpa.node_count as usize];
        let mut fns: Vec<(u32, u32)> = callables
            .iter()
            .filter_map(|&id| {
                let s = sta.symbol(id)?;
                let d = if s.decl_node != u32::MAX {
                    s.decl_node
                } else {
                    s.def_node
                };
                if d < bpa.node_count {
                    Some((d, id))
                } else {
                    None
                }
            })
            .collect();
        fns.sort_unstable();
        for (d, id) in fns {
            let end = subtree_end(bpa, d).min(bpa.node_count);
            for slot in &mut fn_of_node[d as usize..end as usize] {
                *slot = id;
            }
        }

        Self {
            sta,
            bpa,
            src,
            fn_of_node,
            sym_at_decl,
            members,
            constructors,
            field_types,
            field_names,
            methods_by_name,
            types_by_name,
            extends,
            implements,
            subtypes,
            callables,
        }
    }

    #[inline]
    pub fn symbol(&self, id: u32) -> Option<&'a SymbolRecord> {
        self.sta.symbol(id)
    }

    pub fn enclosing_function(&self, node: u32) -> Option<u32> {
        self.fn_of_node
            .get(node as usize)
            .copied()
            .filter(|&f| f != u32::MAX)
    }

    pub fn symbol_at_decl(&self, node: u32) -> Option<u32> {
        self.sym_at_decl.get(&node).copied()
    }

    /// Nearest class-like symbol enclosing a symbol (walking `parent_sym`).
    pub fn enclosing_class(&self, sym: u32) -> Option<u32> {
        let mut cur = self.symbol(sym)?.parent_sym;
        let mut guard = 0usize;
        while cur != u32::MAX && guard < 4096 {
            let s = self.symbol(cur)?;
            if is_class_like(s.kind) {
                return Some(cur);
            }
            cur = s.parent_sym;
            guard += 1;
        }
        None
    }

    /// Class whose declaration lexically contains `node` (for field initializers).
    pub fn class_containing_node(&self, node: u32) -> Option<u32> {
        let mut cur = node;
        let mut guard = 0usize;
        while cur != u32::MAX && guard < 1 << 20 {
            if is_type_decl(self.bpa.node_type(cur)) {
                return self.symbol_at_decl(cur);
            }
            let p = self.bpa.parent(cur);
            if p == cur {
                break;
            }
            cur = p;
            guard += 1;
        }
        None
    }

    pub fn types_named(&self, name: u32) -> &[u32] {
        self.types_by_name
            .get(&name)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    pub fn constructors_of(&self, class: u32) -> &[u32] {
        self.constructors
            .get(&class)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    pub fn methods_named(&self, name: u32) -> &[u32] {
        self.methods_by_name
            .get(&name)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    pub fn direct_supers(&self, class: u32) -> impl Iterator<Item = u32> + '_ {
        let e = self.extends.get(&class).map(|v| v.as_slice()).unwrap_or(&[]);
        let i = self.implements.get(&class).map(|v| v.as_slice()).unwrap_or(&[]);
        e.iter().chain(i.iter()).copied()
    }

    pub fn superclasses(&self, class: u32) -> &[u32] {
        self.extends.get(&class).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// `class` and all its transitive subtypes.
    pub fn subtype_closure(&self, class: u32) -> Vec<u32> {
        let mut out = vec![class];
        let mut seen: HashSet<u32> = HashSet::from([class]);
        let mut i = 0;
        while i < out.len() {
            let c = out[i];
            if let Some(subs) = self.subtypes.get(&c) {
                for &s in subs {
                    if seen.insert(s) {
                        out.push(s);
                    }
                }
            }
            i += 1;
        }
        out
    }

    /// True when `sub` is `sup` or a transitive subtype of it.
    pub fn is_subtype(&self, sub: u32, sup: u32) -> bool {
        if sub == sup {
            return true;
        }
        let mut seen = HashSet::from([sub]);
        let mut work: VecDeque<u32> = VecDeque::from([sub]);
        while let Some(c) = work.pop_front() {
            for s in self.direct_supers(c) {
                if s == sup {
                    return true;
                }
                if seen.insert(s) {
                    work.push_back(s);
                }
            }
        }
        false
    }

    fn arity_matches(sym: &SymbolRecord, arity: usize) -> bool {
        let pc = sym.param_count as usize;
        pc == arity || ((sym.modifiers & SymbolModifiers::VARARGS) != 0 && arity + 1 >= pc)
    }

    /// Methods named `name` declared directly in `class` that accept `arity` arguments.
    fn declared_matching(&self, class: u32, name: u32, arity: Option<usize>) -> Vec<u32> {
        self.members
            .get(&(class, name))
            .map(|list| {
                list.iter()
                    .copied()
                    .filter(|&m| match (arity, self.symbol(m)) {
                        (Some(a), Some(s)) => Self::arity_matches(s, a),
                        (None, Some(_)) => true,
                        _ => false,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Compile-time method selection (JLS 15.12.1-2 without argument types): members of
    /// `class` and its supertypes named `name` with a matching arity, nearest declaration
    /// first; a supertype method is dropped when a subtype already declares one with the
    /// same arity (it is overridden or hidden).
    pub fn select_methods(&self, class: u32, name: u32, arity: Option<usize>) -> Vec<u32> {
        let mut out: Vec<u32> = Vec::new();
        let mut seen_arities: HashSet<u16> = HashSet::new();
        let mut visited = HashSet::from([class]);
        let mut work: VecDeque<u32> = VecDeque::from([class]);
        while let Some(c) = work.pop_front() {
            let found = self.declared_matching(c, name, arity);
            let mut level_arities = Vec::new();
            for m in found {
                let pc = self.symbol(m).map(|s| s.param_count).unwrap_or(0);
                if !seen_arities.contains(&pc) {
                    out.push(m);
                    level_arities.push(pc);
                }
            }
            seen_arities.extend(level_arities);
            for s in self.direct_supers(c) {
                if visited.insert(s) {
                    work.push_back(s);
                }
            }
        }
        out
    }

    /// Run-time dispatch (JVMS 5.4.6) of selector `(name, arity)` on an object whose class
    /// is `class`: the nearest non-abstract declaration along the superclass chain, else
    /// non-abstract (default) methods of its interfaces.
    pub fn dispatch(&self, class: u32, name: u32, arity: Option<usize>) -> Vec<u32> {
        let concrete = |m: &u32| {
            self.symbol(*m)
                .is_some_and(|s| (s.modifiers & SymbolModifiers::ABSTRACT) == 0)
        };
        // Superclass chain.
        let mut visited = HashSet::from([class]);
        let mut chain: VecDeque<u32> = VecDeque::from([class]);
        let mut order = Vec::new();
        while let Some(c) = chain.pop_front() {
            order.push(c);
            let found: Vec<u32> = self
                .declared_matching(c, name, arity)
                .into_iter()
                .filter(concrete)
                .collect();
            if !found.is_empty() {
                return found;
            }
            for &s in self.superclasses(c) {
                if visited.insert(s) {
                    chain.push_back(s);
                }
            }
        }
        // Default methods of the interfaces of the class and its superclasses.
        let mut ivisited: HashSet<u32> = HashSet::new();
        let mut iwork: VecDeque<u32> = VecDeque::new();
        for c in order {
            if let Some(list) = self.implements.get(&c) {
                for &i in list {
                    if ivisited.insert(i) {
                        iwork.push_back(i);
                    }
                }
            }
        }
        let mut defaults = Vec::new();
        while let Some(i) = iwork.pop_front() {
            defaults.extend(
                self.declared_matching(i, name, arity)
                    .into_iter()
                    .filter(concrete),
            );
            for s in self.direct_supers(i) {
                if ivisited.insert(s) {
                    iwork.push_back(s);
                }
            }
        }
        defaults.sort_unstable();
        defaults.dedup();
        defaults
    }

    /// Declared type of a field `name` visible in `class` (walking superclasses).
    pub fn field_type(&self, class: u32, name: u32) -> Option<u32> {
        let mut visited = HashSet::from([class]);
        let mut work: VecDeque<u32> = VecDeque::from([class]);
        while let Some(c) = work.pop_front() {
            if let Some(&t) = self.field_types.get(&(c, name)) {
                return Some(t);
            }
            for s in self.direct_supers(c) {
                if visited.insert(s) {
                    work.push_back(s);
                }
            }
        }
        None
    }

    /// True when `class` or a supertype declares a field `name`.
    pub fn has_field(&self, class: u32, name: u32) -> bool {
        let mut visited = HashSet::from([class]);
        let mut work: VecDeque<u32> = VecDeque::from([class]);
        while let Some(c) = work.pop_front() {
            if self.field_names.contains(&(c, name)) {
                return true;
            }
            for s in self.direct_supers(c) {
                if visited.insert(s) {
                    work.push_back(s);
                }
            }
        }
        false
    }

    /// Declared type of a variable symbol: its own type, or that of the enclosing
    /// declaration for declarators (`int a = 1, b;` registers the type on the outer
    /// declaration). Only class-like types are returned.
    pub fn declared_type(&self, sym: u32) -> Option<u32> {
        let s = self.symbol(sym)?;
        let mut ty = s.type_id;
        if ty == u32::MAX {
            let parent = self.symbol(s.parent_sym)?;
            if parent.kind == SymbolKind::SK_LOCAL_VAR as u8 || parent.kind == SymbolKind::SK_FIELD as u8 {
                ty = parent.type_id;
            }
        }
        let t = self.symbol(ty)?;
        if is_class_like(t.kind) || t.kind == SymbolKind::SK_EXTERNAL as u8 {
            Some(ty)
        } else {
            None
        }
    }

    /// Return type of a method (class-like types only).
    pub fn return_type(&self, method: u32) -> Option<u32> {
        let s = self.symbol(method)?;
        let t = self.symbol(s.type_id)?;
        if is_class_like(t.kind) || t.kind == SymbolKind::SK_EXTERNAL as u8 {
            Some(s.type_id)
        } else {
            None
        }
    }

    pub fn is_interface(&self, class: u32) -> bool {
        self.symbol(class)
            .is_some_and(|s| s.kind == SymbolKind::SK_INTERFACE as u8)
    }

    /// Class symbols that can be the run-time class of an object (not interfaces, not
    /// abstract, not external).
    pub fn is_instantiable(&self, class: u32) -> bool {
        self.symbol(class).is_some_and(|s| {
            (s.kind == SymbolKind::SK_CLASS as u8
                || s.kind == SymbolKind::SK_ENUM as u8
                || s.kind == SymbolKind::SK_RECORD as u8)
                && (s.modifiers & SymbolModifiers::ABSTRACT) == 0
        })
    }
}
