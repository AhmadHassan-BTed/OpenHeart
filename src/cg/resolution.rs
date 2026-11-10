//! Dispatch Resolution Engine for Phase 6.
//! Authored solely by Ahmad Hassan (B-Ted).
//!
//! 1. **Compile-time selection** (JLS 15.12.1-2, without argument types): the receiver's
//!    static type is inferred from the declared type of a local / parameter / field, the
//!    type of a `new` / cast, or the return type of a nested call; members are looked up
//!    by name and arity in that type and its supertypes (inherited and default methods
//!    included). An unqualified call searches the caller's class, then lexically
//!    enclosing classes. A qualifier that names a type makes the call static.
//! 2. **Dispatch** for virtual and interface calls: CHA over the instantiable subtypes of
//!    the static receiver type, each resolved with JVMS 5.4.6 run-time lookup, then
//!    filtered by RTA (classes instantiated anywhere in the program; enum classes count
//!    as instantiated). When no subtype is ever instantiated in project code the CHA set
//!    is kept, since such objects come from library or reflective code.
//! 3. **Points-to refinement** (see `points_to.rs`): when the receiver's points-to set
//!    has only known allocation types, the targets are re-dispatched on exactly those.
//!
//! A call whose receiver type cannot be inferred yields **no** edge: matching every method
//! of the same name and arity would be a guess. The site is recorded as unresolved with
//! its reason (`UnresolvedCause`), and the Phase 3 selection for the same AST node, when
//! there is one, supplies the target instead (`apply_symbol_table`, resolution
//! `symbol_table`). Implicit default constructors, anonymous-class constructors and
//! reflective calls have no symbol and yield no edge either; they are listed with their
//! reason.

use crate::cfg::ast_query::{call_shape, children, CallShape, NewParts};
use crate::cg::call_sites::{FunctionContext, SiteInfo, SiteKind};
use crate::cg::index::ProgramIndex;
use crate::core::types::symbol::*;
use crate::core::types::*;
use crate::ssa::names::ORIG_NODE_TAG;
use crate::symbol::model::{Occurrence, ResolutionStatus};
use std::collections::{HashMap, HashSet, VecDeque};

pub const MOD_ABSTRACT: u16 = SymbolModifiers::ABSTRACT;

/// Why a call site resolved to no target (see `CG_UNRESOLVED_*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnresolvedCause {
    /// `CG_UNRESOLVED_*`.
    pub reason: u8,
    /// Text id of the written name (method or type name), or `u32::MAX`.
    pub name_id: u32,
    /// Type symbol the reason refers to, or `u32::MAX`.
    pub type_sym: u32,
}

impl UnresolvedCause {
    fn new(reason: u8, name_id: Option<u32>, type_sym: Option<u32>) -> Self {
        Self {
            reason,
            name_id: name_id.unwrap_or(u32::MAX),
            type_sym: type_sym.unwrap_or(u32::MAX),
        }
    }
}

/// Static type information about a receiver expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReceiverType {
    /// No qualifier: `this` of the caller's class (or an enclosing class).
    Implicit,
    /// Instance of one of these static types.
    Instance(Vec<u32>),
    /// The qualifier names a type: a static invocation.
    Static(Vec<u32>),
    /// `super.m()`: invokespecial on the superclass(es).
    Super(Vec<u32>),
    Unknown,
}

/// Outcome of resolving one call site.
#[derive(Debug, Clone, Default)]
pub struct Resolution {
    pub targets: Vec<u32>,
    pub call_type: u8,
    /// Static receiver types used for dispatch (empty for non-virtual calls).
    pub receiver_types: Vec<u32>,
    /// Selector `(name, arity)` of a virtual call, used for points-to re-dispatch.
    pub selector: Option<(u32, Option<usize>)>,
    /// How `targets` was established (`CG_RESOLUTION_*`; the default 0 is `exact`).
    pub resolution: u8,
    /// Why `targets` is empty (`None` while targets exist).
    pub unresolved: Option<UnresolvedCause>,
}

/// Dispatch Resolver for Call Sites (§6.2.3, §6.2.4)
pub struct DispatchResolver<'r, 'a> {
    pub index: &'r ProgramIndex<'a>,
    instantiated: HashSet<u32>,
    subtype_cache: HashMap<u32, Vec<u32>>,
}

impl<'r, 'a> DispatchResolver<'r, 'a> {
    /// `instantiated`: class symbols allocated anywhere in the program (RTA).
    pub fn new(index: &'r ProgramIndex<'a>, instantiated: HashSet<u32>) -> Self {
        Self {
            index,
            instantiated,
            subtype_cache: HashMap::new(),
        }
    }

    /// Class symbols instantiated in the program: allocation sites plus enum classes.
    pub fn instantiated_classes(index: &ProgramIndex<'_>) -> HashSet<u32> {
        let bpa = index.bpa;
        let mut out = HashSet::new();
        for sym in &index.sta.symbol_records {
            if sym.kind == SymbolKind::SK_ENUM as u8 {
                out.insert(sym.symbol_id);
            }
        }
        if let Some(src) = index.src.as_ref() {
            for node in 0..bpa.node_count {
                if bpa.node_type(node) != ASTNodeType::NN_NEW_EXPR {
                    continue;
                }
                let parts = NewParts::of(bpa, Some(src), node);
                if let Some(name) = parts.type_name {
                    for &t in index.types_named(name) {
                        out.insert(t);
                    }
                }
            }
        }
        out
    }

    fn caller_class(&self, caller: u32) -> Option<u32> {
        let s = self.index.symbol(caller)?;
        if crate::cg::index::is_class_like(s.kind) {
            Some(caller)
        } else {
            self.index.enclosing_class(caller)
        }
    }

    fn text_id(&self, node: u32) -> Option<u32> {
        self.index
            .src
            .as_ref()
            .and_then(|s| s.node_text_id(self.index.bpa, node))
    }

    /// Prefers project classes over same-named external stubs.
    fn named_types(&self, name: u32) -> Vec<u32> {
        let all = self.index.types_named(name);
        let project: Vec<u32> = all
            .iter()
            .copied()
            .filter(|&t| {
                self.index
                    .symbol(t)
                    .is_some_and(|s| s.kind != SymbolKind::SK_EXTERNAL as u8)
            })
            .collect();
        if project.is_empty() {
            all.to_vec()
        } else {
            project
        }
    }

    /// Declared type of a variable without a symbol (enhanced-for variable, catch
    /// parameter, pattern binding): the `NN_TYPE_REF` immediately preceding its name.
    fn type_of_unregistered_var(&self, name_node: u32) -> Vec<u32> {
        let bpa = self.index.bpa;
        let parent = bpa.parent(name_node);
        if parent == u32::MAX {
            return Vec::new();
        }
        let kids = children(bpa, parent);
        let pos = match kids.iter().position(|&c| c == name_node) {
            Some(p) if p > 0 => p,
            _ => return Vec::new(),
        };
        let prev = kids[pos - 1];
        if bpa.node_type(prev) != ASTNodeType::NN_TYPE_REF {
            return Vec::new();
        }
        self.text_id(prev)
            .map(|t| self.named_types(t))
            .unwrap_or_default()
    }

    /// Static type of a receiver expression evaluated in `ctx`.
    pub fn receiver_type(
        &self,
        expr: u32,
        caller: u32,
        ctx: Option<&FunctionContext>,
        depth: usize,
    ) -> ReceiverType {
        let index = self.index;
        let bpa = index.bpa;
        if depth > 16 {
            return ReceiverType::Unknown;
        }
        let class = self.caller_class(caller);
        match bpa.node_type(expr) {
            ASTNodeType::NN_THIS_EXPR => match class {
                Some(c) => ReceiverType::Instance(vec![c]),
                None => ReceiverType::Unknown,
            },
            ASTNodeType::NN_SUPER_EXPR => match class {
                Some(c) => ReceiverType::Super(index.superclasses(c).to_vec()),
                None => ReceiverType::Unknown,
            },
            ASTNodeType::NN_IDENTIFIER_EXPR => {
                if let Some(c) = ctx {
                    if let Some(var) = c.vars.var_of_occurrence(expr) {
                        let orig = c.vars.orig_id(var);
                        let types = if orig & ORIG_NODE_TAG == 0 {
                            index
                                .declared_type(orig)
                                .map(|t| vec![t])
                                .unwrap_or_default()
                        } else {
                            self.type_of_unregistered_var(orig & !ORIG_NODE_TAG)
                        };
                        return if types.is_empty() {
                            ReceiverType::Unknown
                        } else {
                            ReceiverType::Instance(types)
                        };
                    }
                }
                let name = match self.text_id(expr) {
                    Some(n) => n,
                    None => return ReceiverType::Unknown,
                };
                if let Some(c) = class {
                    if index.has_field(c, name) {
                        return match index.field_type(c, name) {
                            Some(t) => ReceiverType::Instance(vec![t]),
                            None => ReceiverType::Unknown,
                        };
                    }
                }
                let types = self.named_types(name);
                if types.is_empty() {
                    ReceiverType::Unknown
                } else {
                    ReceiverType::Static(types)
                }
            }
            ASTNodeType::NN_NEW_EXPR => {
                let parts = NewParts::of(bpa, index.src.as_ref(), expr);
                match parts.type_name {
                    Some(t) if parts.anonymous_members.is_empty() => {
                        let types = self.named_types(t);
                        if types.is_empty() {
                            ReceiverType::Unknown
                        } else {
                            ReceiverType::Instance(types)
                        }
                    }
                    _ => ReceiverType::Unknown,
                }
            }
            ASTNodeType::NN_CAST_EXPR => {
                let ty = children(bpa, expr)
                    .into_iter()
                    .find(|&c| bpa.node_type(c) == ASTNodeType::NN_TYPE_REF)
                    .and_then(|t| self.text_id(t))
                    .map(|t| self.named_types(t))
                    .unwrap_or_default();
                if ty.is_empty() {
                    ReceiverType::Unknown
                } else {
                    ReceiverType::Instance(ty)
                }
            }
            ASTNodeType::NN_FIELD_ACCESS => {
                let kids = children(bpa, expr);
                if kids.len() < 2 {
                    return ReceiverType::Unknown;
                }
                let field = match self.text_id(kids[kids.len() - 1]) {
                    Some(f) => f,
                    None => return ReceiverType::Unknown,
                };
                let owners = match self.receiver_type(kids[0], caller, ctx, depth + 1) {
                    ReceiverType::Instance(ts) | ReceiverType::Static(ts) => ts,
                    _ => return ReceiverType::Unknown,
                };
                let types: Vec<u32> = owners
                    .iter()
                    .filter_map(|&o| index.field_type(o, field))
                    .collect();
                if types.is_empty() {
                    ReceiverType::Unknown
                } else {
                    ReceiverType::Instance(types)
                }
            }
            ASTNodeType::NN_CALL_EXPR => {
                let src = match index.src.as_ref() {
                    Some(s) => s,
                    None => return ReceiverType::Unknown,
                };
                let (receiver, name, args) = match call_shape(bpa, src, expr) {
                    Some(CallShape::Method {
                        receiver,
                        name,
                        args,
                    }) => (receiver, name, args),
                    _ => return ReceiverType::Unknown,
                };
                let name_id = match src.node_text_id(bpa, name) {
                    Some(n) => n,
                    None => return ReceiverType::Unknown,
                };
                let statics =
                    self.static_targets(receiver, name_id, args.len(), caller, ctx, depth + 1);
                let mut types: Vec<u32> = statics
                    .iter()
                    .filter_map(|&m| index.return_type(m))
                    .collect();
                types.sort_unstable();
                types.dedup();
                if types.is_empty() {
                    ReceiverType::Unknown
                } else {
                    ReceiverType::Instance(types)
                }
            }
            _ => ReceiverType::Unknown,
        }
    }

    /// Compile-time targets of `receiver.name(args)` (no dispatch expansion).
    fn static_targets(
        &self,
        receiver: Option<u32>,
        name: u32,
        arity: usize,
        caller: u32,
        ctx: Option<&FunctionContext>,
        depth: usize,
    ) -> Vec<u32> {
        let index = self.index;
        let rt = match receiver {
            None => ReceiverType::Implicit,
            Some(r) => self.receiver_type(r, caller, ctx, depth),
        };
        match rt {
            ReceiverType::Implicit => {
                // The caller's class, then lexically enclosing classes.
                let mut class = self.caller_class(caller);
                let mut guard = 0;
                while let Some(c) = class {
                    let found = index.select_methods(c, name, Some(arity));
                    if !found.is_empty() {
                        return found;
                    }
                    class = index.enclosing_class(c);
                    guard += 1;
                    if guard > 64 {
                        break;
                    }
                }
                Vec::new()
            }
            ReceiverType::Instance(ts) | ReceiverType::Static(ts) | ReceiverType::Super(ts) => {
                let mut out = Vec::new();
                for t in ts {
                    out.extend(index.select_methods(t, name, Some(arity)));
                }
                out.sort_unstable();
                out.dedup();
                out
            }
            ReceiverType::Unknown => Vec::new(),
        }
    }

    fn subtypes(&mut self, class: u32) -> Vec<u32> {
        if let Some(v) = self.subtype_cache.get(&class) {
            return v.clone();
        }
        let v = self.index.subtype_closure(class);
        self.subtype_cache.insert(class, v.clone());
        v
    }

    /// CHA over the instantiable subtypes of `receiver_types`, filtered by RTA.
    fn virtual_targets(
        &mut self,
        receiver_types: &[u32],
        name: u32,
        arity: Option<usize>,
    ) -> Vec<u32> {
        let mut cha = Vec::new();
        let mut rta = Vec::new();
        let mut any_instantiated = false;
        for &t in receiver_types {
            for s in self.subtypes(t) {
                if !self.index.is_instantiable(s) {
                    continue;
                }
                let found = self.index.dispatch(s, name, arity);
                if self.instantiated.contains(&s) {
                    any_instantiated = true;
                    rta.extend(found.iter().copied());
                }
                cha.extend(found);
            }
        }
        let mut out = if any_instantiated { rta } else { cha };
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Re-dispatches a virtual call on the allocation types of its receiver.
    pub fn dispatch_on(&self, types: &[u32], selector: (u32, Option<usize>)) -> Vec<u32> {
        let mut out = Vec::new();
        for &t in types {
            out.extend(self.index.dispatch(t, selector.0, selector.1));
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    fn classify(&self, targets: &[u32], receiver: &ReceiverType) -> u8 {
        let index = self.index;
        if matches!(receiver, ReceiverType::Super(_)) {
            return CG_EDGE_SPECIAL;
        }
        let syms: Vec<&SymbolRecord> = targets.iter().filter_map(|&t| index.symbol(t)).collect();
        if !syms.is_empty()
            && syms
                .iter()
                .all(|s| (s.modifiers & SymbolModifiers::STATIC) != 0)
        {
            return CG_EDGE_DIRECT;
        }
        if !syms.is_empty()
            && syms
                .iter()
                .all(|s| s.visibility == SymbolVisibility::Private as u8)
        {
            return CG_EDGE_SPECIAL;
        }
        if matches!(receiver, ReceiverType::Static(_)) {
            return CG_EDGE_DIRECT;
        }
        let interface_receiver = match receiver {
            ReceiverType::Instance(ts) => ts.iter().any(|&t| index.is_interface(t)),
            _ => false,
        };
        if interface_receiver {
            CG_EDGE_INTERFACE
        } else {
            CG_EDGE_VIRTUAL
        }
    }

    fn is_external(&self, sym: u32) -> bool {
        self.index
            .symbol(sym)
            .is_some_and(|s| s.kind == SymbolKind::SK_EXTERNAL as u8)
    }

    /// First library (external) type among `types` and their transitive supertypes, in
    /// breadth-first order: a member that is not found in project code may be inherited
    /// from it.
    fn external_in_hierarchy(&self, types: &[u32]) -> Option<u32> {
        let mut seen: HashSet<u32> = HashSet::new();
        let mut work: VecDeque<u32> = VecDeque::new();
        for &t in types {
            if seen.insert(t) {
                work.push_back(t);
            }
        }
        let mut guard = 0usize;
        while let Some(t) = work.pop_front() {
            if self.is_external(t) {
                return Some(t);
            }
            for s in self.index.direct_supers(t) {
                if seen.insert(s) {
                    work.push_back(s);
                }
            }
            guard += 1;
            if guard > 1 << 16 {
                break;
            }
        }
        None
    }

    /// Reason for a member lookup that found nothing in `owners` (and their supertypes).
    fn member_cause(&self, owners: &[u32], name_id: Option<u32>) -> UnresolvedCause {
        if owners.is_empty() {
            return UnresolvedCause::new(CG_UNRESOLVED_RECEIVER_TYPE_UNKNOWN, name_id, None);
        }
        match self.external_in_hierarchy(owners) {
            Some(lib) => UnresolvedCause::new(CG_UNRESOLVED_LIBRARY_MEMBER, name_id, Some(lib)),
            None => UnresolvedCause::new(
                CG_UNRESOLVED_MEMBER_NOT_FOUND,
                name_id,
                owners.first().copied(),
            ),
        }
    }

    /// Reason for a constructor lookup in `owners` that matched nothing for `arity`.
    fn constructor_cause(
        &self,
        owners: &[u32],
        name_id: Option<u32>,
        arity: Option<usize>,
    ) -> UnresolvedCause {
        let index = self.index;
        let first = owners.first().copied();
        if owners.is_empty() {
            return UnresolvedCause::new(CG_UNRESOLVED_TYPE_UNKNOWN, name_id, None);
        }
        if let Some(&lib) = owners.iter().find(|&&o| self.is_external(o)) {
            return UnresolvedCause::new(CG_UNRESOLVED_LIBRARY_MEMBER, name_id, Some(lib));
        }
        let declares_any = owners.iter().any(|&o| !index.constructors_of(o).is_empty());
        if !declares_any && arity.map_or(true, |a| a == 0) {
            UnresolvedCause::new(CG_UNRESOLVED_IMPLICIT_CONSTRUCTOR, name_id, first)
        } else {
            UnresolvedCause::new(CG_UNRESOLVED_NO_MATCHING_CONSTRUCTOR, name_id, first)
        }
    }

    /// Call type of an edge whose target came from the Phase 3 symbol table, derived from
    /// the call syntax and the target's declaration.
    fn symbol_table_call_type(&self, site: &SiteInfo, current: u8, target: u32) -> u8 {
        let index = self.index;
        let rec = match index.symbol(target) {
            Some(r) => r,
            None => return current,
        };
        let node_type = index.bpa.node_type(site.call_node);
        if rec.kind == SymbolKind::SK_CONSTRUCTOR as u8
            || node_type == ASTNodeType::NN_NEW_EXPR
            || matches!(site.kind, SiteKind::ConstructorChain { .. })
        {
            CG_EDGE_CONSTRUCTOR
        } else if node_type == ASTNodeType::NN_METHOD_REF {
            CG_EDGE_DYNAMIC
        } else if (rec.modifiers & SymbolModifiers::STATIC) != 0 {
            CG_EDGE_DIRECT
        } else if current == CG_EDGE_SPECIAL || rec.visibility == SymbolVisibility::Private as u8 {
            CG_EDGE_SPECIAL
        } else if index.is_interface(rec.parent_sym) {
            CG_EDGE_INTERFACE
        } else {
            CG_EDGE_VIRTUAL
        }
    }

    /// Combines the Phase 6 resolution of `site` with the selection Phase 3 recorded for
    /// the same AST node (`occ`, a `Call` or `Instantiation` occurrence):
    /// * a non-virtual site with several same-arity candidates is narrowed to the Phase 3
    ///   choice when that choice is one of them (Phase 3 compares argument types);
    /// * a site without targets takes the Phase 3 target when Phase 3 bound it to a
    ///   project method or constructor (resolution `symbol_table`);
    /// * otherwise the unresolved reason is refined with what Phase 3 established
    ///   (library receiver, ambiguous overloads, implicit default constructor).
    pub fn apply_symbol_table(
        &self,
        site: &SiteInfo,
        res: &mut Resolution,
        occ: Option<&Occurrence>,
    ) {
        let occ = match occ {
            Some(o) => o,
            None => return,
        };
        let index = self.index;
        let status = ResolutionStatus::from_u8(occ.status);
        let target = occ.symbol;
        let target_rec = if target == u32::MAX {
            None
        } else {
            index.symbol(target)
        };
        let callable = target_rec.is_some_and(|s| {
            s.kind == SymbolKind::SK_METHOD as u8 || s.kind == SymbolKind::SK_CONSTRUCTOR as u8
        });
        let project = status == Some(ResolutionStatus::Project);

        if !res.targets.is_empty() {
            if callable
                && project
                && res.resolution == CG_RESOLUTION_EXACT
                && res.targets.len() > 1
                && res.targets.contains(&target)
            {
                res.targets = vec![target];
            }
            return;
        }
        if callable && project {
            res.call_type = self.symbol_table_call_type(site, res.call_type, target);
            res.targets = vec![target];
            res.resolution = CG_RESOLUTION_SYMBOL_TABLE;
            res.receiver_types.clear();
            res.selector = None;
            res.unresolved = None;
            return;
        }

        let (name_id, type_sym, current) = match res.unresolved {
            Some(u) => (u.name_id, u.type_sym, u.reason),
            None => (u32::MAX, u32::MAX, 0),
        };
        let target_is_type = target_rec.is_some_and(|s| {
            crate::cg::index::is_class_like(s.kind) || s.kind == SymbolKind::SK_EXTERNAL as u8
        });
        let refined = match status {
            Some(ResolutionStatus::Library) => Some(UnresolvedCause {
                reason: CG_UNRESOLVED_LIBRARY_MEMBER,
                name_id,
                type_sym: if target_is_type { target } else { type_sym },
            }),
            Some(ResolutionStatus::Ambiguous) => Some(UnresolvedCause {
                reason: CG_UNRESOLVED_AMBIGUOUS,
                name_id,
                type_sym,
            }),
            Some(ResolutionStatus::Project)
                if target_is_type && current != CG_UNRESOLVED_ANONYMOUS_CLASS =>
            {
                Some(UnresolvedCause {
                    reason: CG_UNRESOLVED_IMPLICIT_CONSTRUCTOR,
                    name_id,
                    type_sym: target,
                })
            }
            _ => None,
        };
        if let Some(r) = refined {
            res.unresolved = Some(r);
        }
    }

    /// Resolve targets for a call site.
    pub fn resolve(&mut self, site: &SiteInfo, contexts: &[FunctionContext]) -> Resolution {
        let index = self.index;
        let ctx = site.ctx.map(|c| &contexts[c]);
        let caller = site.caller_sym;

        match &site.kind {
            SiteKind::Lambda => {
                let target = index.symbol_at_decl(site.call_node).filter(|&s| {
                    index
                        .symbol(s)
                        .is_some_and(|r| r.kind == SymbolKind::SK_LAMBDA as u8)
                });
                Resolution {
                    unresolved: match target {
                        Some(_) => None,
                        None => Some(UnresolvedCause::new(
                            CG_UNRESOLVED_LAMBDA_WITHOUT_SYMBOL,
                            None,
                            None,
                        )),
                    },
                    targets: target.into_iter().collect(),
                    call_type: CG_EDGE_DYNAMIC,
                    ..Resolution::default()
                }
            }
            SiteKind::Opaque => Resolution {
                targets: Vec::new(),
                call_type: match index.bpa.node_type(site.call_node) {
                    ASTNodeType::NN_NEW_EXPR => CG_EDGE_CONSTRUCTOR,
                    ASTNodeType::NN_METHOD_REF | ASTNodeType::NN_LAMBDA_EXPR => CG_EDGE_DYNAMIC,
                    _ => CG_EDGE_VIRTUAL,
                },
                unresolved: Some(UnresolvedCause::new(
                    if index.src.is_none() {
                        CG_UNRESOLVED_NO_SOURCE_TEXT
                    } else {
                        CG_UNRESOLVED_CALL_SHAPE
                    },
                    None,
                    None,
                )),
                ..Resolution::default()
            },
            SiteKind::New {
                type_name,
                args,
                anonymous,
                ..
            } => {
                // `new T(..) { .. }` runs a constructor of T as well (for class types);
                // the anonymous subclass itself has no symbol.
                let mut targets = Vec::new();
                let owners: Vec<u32> = match type_name {
                    Some(t) => self.named_types(*t),
                    None => Vec::new(),
                };
                for &c in &owners {
                    targets.extend(index.constructors_of(c).iter().copied().filter(|&k| {
                        index
                            .symbol(k)
                            .is_some_and(|s| s.param_count as usize == args.len())
                    }));
                }
                targets.sort_unstable();
                targets.dedup();
                let unresolved = if !targets.is_empty() {
                    None
                } else if *anonymous && !owners.is_empty() {
                    Some(UnresolvedCause::new(
                        CG_UNRESOLVED_ANONYMOUS_CLASS,
                        *type_name,
                        owners.first().copied(),
                    ))
                } else {
                    Some(self.constructor_cause(&owners, *type_name, Some(args.len())))
                };
                Resolution {
                    targets,
                    call_type: CG_EDGE_CONSTRUCTOR,
                    unresolved,
                    ..Resolution::default()
                }
            }
            SiteKind::ConstructorChain { is_super, args } => {
                let mut targets = Vec::new();
                let owners: Vec<u32> = match self.caller_class(caller) {
                    Some(c) if *is_super => index.superclasses(c).to_vec(),
                    Some(c) => vec![c],
                    None => Vec::new(),
                };
                for &o in &owners {
                    targets.extend(index.constructors_of(o).iter().copied().filter(|&k| {
                        index
                            .symbol(k)
                            .is_some_and(|s| s.param_count as usize == args.len())
                    }));
                }
                targets.sort_unstable();
                targets.dedup();
                let unresolved = if targets.is_empty() {
                    Some(self.constructor_cause(&owners, None, Some(args.len())))
                } else {
                    None
                };
                Resolution {
                    targets,
                    call_type: CG_EDGE_CONSTRUCTOR,
                    unresolved,
                    ..Resolution::default()
                }
            }
            SiteKind::MethodRef {
                qualifier,
                name_id,
                is_new,
                ..
            } => {
                let rt = match qualifier {
                    None => ReceiverType::Implicit,
                    Some(q) => self.receiver_type(*q, caller, ctx, 0),
                };
                let owners: Vec<u32> = match &rt {
                    ReceiverType::Implicit => self.caller_class(caller).into_iter().collect(),
                    ReceiverType::Instance(ts)
                    | ReceiverType::Static(ts)
                    | ReceiverType::Super(ts) => ts.clone(),
                    ReceiverType::Unknown => Vec::new(),
                };
                let mut targets = Vec::new();
                if *is_new {
                    for o in &owners {
                        targets.extend_from_slice(index.constructors_of(*o));
                    }
                } else if let Some(n) = name_id {
                    for &o in &owners {
                        targets.extend(index.select_methods(o, *n, None));
                    }
                }
                targets.sort_unstable();
                targets.dedup();
                let unresolved = if !targets.is_empty() {
                    None
                } else if rt == ReceiverType::Unknown {
                    Some(UnresolvedCause::new(
                        CG_UNRESOLVED_RECEIVER_TYPE_UNKNOWN,
                        *name_id,
                        None,
                    ))
                } else if *is_new {
                    Some(self.constructor_cause(&owners, None, None))
                } else if name_id.is_none() {
                    Some(UnresolvedCause::new(CG_UNRESOLVED_CALL_SHAPE, None, None))
                } else {
                    Some(self.member_cause(&owners, *name_id))
                };
                Resolution {
                    targets,
                    call_type: CG_EDGE_DYNAMIC,
                    unresolved,
                    ..Resolution::default()
                }
            }
            SiteKind::Method {
                receiver,
                name_id,
                args,
                ..
            } => {
                let arity = args.len();
                let rt = match receiver {
                    None => ReceiverType::Implicit,
                    Some(r) => self.receiver_type(*r, caller, ctx, 0),
                };
                let statics = self.static_targets(*receiver, *name_id, arity, caller, ctx, 0);

                if statics.is_empty() {
                    // No compile-time candidate. An unknown receiver type is not replaced
                    // by every same-named method: that would be a guess.
                    let cause = match &rt {
                        ReceiverType::Unknown => UnresolvedCause::new(
                            CG_UNRESOLVED_RECEIVER_TYPE_UNKNOWN,
                            Some(*name_id),
                            None,
                        ),
                        ReceiverType::Implicit => {
                            // The caller's class and its lexically enclosing classes.
                            let mut owners = Vec::new();
                            let mut class = self.caller_class(caller);
                            let mut guard = 0;
                            while let Some(c) = class {
                                owners.push(c);
                                class = index.enclosing_class(c);
                                guard += 1;
                                if guard > 64 {
                                    break;
                                }
                            }
                            if owners.is_empty() {
                                // A free function call (no enclosing class): not in scope.
                                UnresolvedCause::new(
                                    CG_UNRESOLVED_MEMBER_NOT_FOUND,
                                    Some(*name_id),
                                    None,
                                )
                            } else {
                                self.member_cause(&owners, Some(*name_id))
                            }
                        }
                        ReceiverType::Instance(ts)
                        | ReceiverType::Static(ts)
                        | ReceiverType::Super(ts) => self.member_cause(ts, Some(*name_id)),
                    };
                    return Resolution {
                        targets: Vec::new(),
                        call_type: self.classify(&[], &rt),
                        unresolved: Some(cause),
                        ..Resolution::default()
                    };
                }

                let call_type = self.classify(&statics, &rt);
                let is_virtual = call_type == CG_EDGE_VIRTUAL || call_type == CG_EDGE_INTERFACE;
                if !is_virtual {
                    return Resolution {
                        targets: statics,
                        call_type,
                        ..Resolution::default()
                    };
                }

                let receiver_types: Vec<u32> = match &rt {
                    ReceiverType::Instance(ts) => ts.clone(),
                    ReceiverType::Implicit => {
                        // `this` is an instance of the caller's class (or a subclass); when
                        // the method was found in a lexically enclosing class, the receiver
                        // is that outer instance.
                        let mut owners: Vec<u32> = statics
                            .iter()
                            .filter_map(|&m| index.symbol(m).map(|s| s.parent_sym))
                            .collect();
                        owners.sort_unstable();
                        owners.dedup();
                        match self.caller_class(caller) {
                            Some(c) if owners.iter().any(|&o| index.is_subtype(c, o)) => vec![c],
                            _ => owners,
                        }
                    }
                    _ => Vec::new(),
                };
                let selector = (*name_id, Some(arity));
                let mut targets = self.virtual_targets(&receiver_types, *name_id, Some(arity));
                if targets.is_empty() {
                    // No concrete implementation in the project (library subtype or
                    // abstract-only hierarchy): keep the statically selected methods.
                    targets = statics;
                }
                Resolution {
                    targets,
                    call_type,
                    receiver_types,
                    selector: Some(selector),
                    resolution: CG_RESOLUTION_CHA,
                    unresolved: None,
                }
            }
        }
    }
}
