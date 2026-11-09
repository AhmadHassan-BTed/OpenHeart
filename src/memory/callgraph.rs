//! On-the-fly Call Graph and Type-Hierarchy Dispatch (§M.3).
//!
//! The call graph is not computed before the pointer analysis but by it: a virtual or
//! interface call is resolved for every object that reaches its receiver, through that
//! object's dynamic type, and the callee's parameters receive exactly that object. Only the
//! functions the solver reaches from the entry points are ever analyzed.
//!
//! **Dispatch is language-neutral.** Which method implements which is a fact of the source
//! language (Java/C# override rules, C++ virtual functions and final overriders, Kotlin
//! `open`/`override`), so frontends hand it in as a [`TypeHierarchy`]: the supertype edges,
//! the methods each type declares, and the override relation (`m` overrides `n`). Dispatch
//! of method `n` on dynamic type `T` is then the same algorithm for every language: among
//! the concrete members of `n`'s override family declared in `T` or its supertypes, take the
//! most specific ones. More than one survivor (an ambiguous diamond) is reported as an
//! over-approximation, none as "no implementation" — never as a guess.
//!
//! **Fallbacks are explicit.** A receiver object whose type is not derivable (an entry
//! parameter or an object from external code) is dispatched by class-hierarchy analysis
//! over the declared method and every resulting edge is marked over-approximate. Calls into
//! code outside the module become external targets; call sites that resolve to nothing
//! carry the reason.

use std::collections::VecDeque;

use crate::ir::types::{CallSiteId, FuncId, Span};
use crate::memory::location::{Context, FxHashSet, ObjId};
use crate::psa::bdd::fxhash::FxHashMap;

// ── Type hierarchy facts ────────────────────────────────────────────────────

/// Facts about one method symbol, supplied by the frontend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MethodFacts {
    /// Type symbol that declares the method.
    pub declaring_type: u32,
    /// The method has no body (abstract, pure virtual, interface method without default).
    pub is_abstract: bool,
    /// The method takes a receiver, lowered as parameter 0.
    pub is_instance: bool,
    /// Declared outside the analyzed sources (library, runtime, platform SDK).
    pub is_external: bool,
}

/// Outcome of dispatching a method on a dynamic type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dispatch {
    /// The most specific concrete implementations; `exact` when there is exactly one.
    Resolved { targets: Vec<u32>, exact: bool },
    /// The hierarchy knows the type but none of its supertypes declares a concrete member
    /// of the method's override family.
    NoImplementation,
    /// The hierarchy has no facts about the type.
    UnknownType,
}

/// Type hierarchy and override relation of the analyzed program and of the external types
/// a frontend models. All ids are symbol ids of the frontend's symbol table.
#[derive(Debug, Clone, Default)]
pub struct TypeHierarchy {
    types: FxHashSet<u32>,
    abstract_types: FxHashSet<u32>,
    supertypes: FxHashMap<u32, Vec<u32>>,
    subtypes: FxHashMap<u32, Vec<u32>>,
    methods_of: FxHashMap<u32, Vec<u32>>,
    method_facts: FxHashMap<u32, MethodFacts>,
    overrides: FxHashMap<u32, Vec<u32>>,
    overridden_by: FxHashMap<u32, Vec<u32>>,
}

impl TypeHierarchy {
    pub fn new() -> Self {
        Self::default()
    }

    /// Declare a type. `is_abstract`: the type cannot be instantiated (abstract class,
    /// interface, trait), so class-hierarchy analysis never dispatches on it.
    pub fn add_type(&mut self, ty: u32, is_abstract: bool) {
        self.types.insert(ty);
        if is_abstract {
            self.abstract_types.insert(ty);
        }
    }

    /// Declare `sup` a direct supertype of `sub` (extends, implements, base class).
    pub fn add_supertype(&mut self, sub: u32, sup: u32) {
        self.types.insert(sub);
        self.types.insert(sup);
        let s = self.supertypes.entry(sub).or_default();
        if !s.contains(&sup) {
            s.push(sup);
            self.subtypes.entry(sup).or_default().push(sub);
        }
    }

    /// Declare a method.
    pub fn add_method(&mut self, method: u32, facts: MethodFacts) {
        self.types.insert(facts.declaring_type);
        if self.method_facts.insert(method, facts).is_none() {
            self.methods_of
                .entry(facts.declaring_type)
                .or_default()
                .push(method);
        }
    }

    /// Declare that `method` overrides (or implements) `overridden`. A frontend must also
    /// emit the edges for implementations inherited into a type from a supertype that does
    /// not itself declare the interface (`class C extends B implements I` with `B.m`
    /// implementing `I.m`): `B.m` overrides `I.m`.
    pub fn add_override(&mut self, method: u32, overridden: u32) {
        let o = self.overrides.entry(method).or_default();
        if !o.contains(&overridden) {
            o.push(overridden);
            self.overridden_by.entry(overridden).or_default().push(method);
        }
    }

    #[inline]
    pub fn knows_type(&self, ty: u32) -> bool {
        self.types.contains(&ty)
    }

    #[inline]
    pub fn is_abstract_type(&self, ty: u32) -> bool {
        self.abstract_types.contains(&ty)
    }

    #[inline]
    pub fn method(&self, method: u32) -> Option<&MethodFacts> {
        self.method_facts.get(&method)
    }

    #[inline]
    pub fn is_instance_method(&self, method: u32) -> bool {
        self.method_facts
            .get(&method)
            .map(|f| f.is_instance)
            .unwrap_or(false)
    }

    pub fn direct_supertypes(&self, ty: u32) -> &[u32] {
        self.supertypes.get(&ty).map(|v| v.as_slice()).unwrap_or(&[])
    }

    pub fn direct_subtypes(&self, ty: u32) -> &[u32] {
        self.subtypes.get(&ty).map(|v| v.as_slice()).unwrap_or(&[])
    }

    pub fn methods_declared_in(&self, ty: u32) -> &[u32] {
        self.methods_of.get(&ty).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// `ty` and all its transitive supertypes, nearest first.
    pub fn ancestors(&self, ty: u32) -> Vec<u32> {
        Self::closure(ty, &self.supertypes)
    }

    /// `ty` and all its transitive subtypes, nearest first.
    pub fn descendants(&self, ty: u32) -> Vec<u32> {
        Self::closure(ty, &self.subtypes)
    }

    fn closure(start: u32, edges: &FxHashMap<u32, Vec<u32>>) -> Vec<u32> {
        let mut seen: FxHashSet<u32> = FxHashSet::default();
        let mut order = Vec::new();
        let mut queue = VecDeque::new();
        seen.insert(start);
        queue.push_back(start);
        while let Some(t) = queue.pop_front() {
            order.push(t);
            if let Some(next) = edges.get(&t) {
                for &n in next {
                    if seen.insert(n) {
                        queue.push_back(n);
                    }
                }
            }
        }
        order
    }

    /// Reflexive, transitive subtyping.
    pub fn is_subtype(&self, sub: u32, sup: u32) -> bool {
        if sub == sup {
            return true;
        }
        let mut seen: FxHashSet<u32> = FxHashSet::default();
        let mut stack = vec![sub];
        while let Some(t) = stack.pop() {
            if let Some(next) = self.supertypes.get(&t) {
                for &n in next {
                    if n == sup {
                        return true;
                    }
                    if seen.insert(n) {
                        stack.push(n);
                    }
                }
            }
        }
        false
    }

    /// `method` and every method that transitively overrides it.
    pub fn override_family(&self, method: u32) -> Vec<u32> {
        Self::closure(method, &self.overridden_by)
    }

    /// `method` transitively overrides `base` (irreflexive).
    pub fn overrides_transitively(&self, method: u32, base: u32) -> bool {
        let mut seen: FxHashSet<u32> = FxHashSet::default();
        let mut stack = vec![method];
        while let Some(m) = stack.pop() {
            if let Some(next) = self.overrides.get(&m) {
                for &n in next {
                    if n == base {
                        return true;
                    }
                    if seen.insert(n) {
                        stack.push(n);
                    }
                }
            }
        }
        false
    }

    /// Dispatch `method` on an object whose dynamic type is `dynamic_type`.
    pub fn dispatch(&self, dynamic_type: u32, method: u32) -> Dispatch {
        if !self.knows_type(dynamic_type) {
            return Dispatch::UnknownType;
        }
        let ancestors: FxHashSet<u32> = self.ancestors(dynamic_type).into_iter().collect();
        let candidates: Vec<(u32, u32)> = self
            .override_family(method)
            .into_iter()
            .filter_map(|m| {
                let f = self.method_facts.get(&m)?;
                (!f.is_abstract && ancestors.contains(&f.declaring_type))
                    .then_some((m, f.declaring_type))
            })
            .collect();
        // Most specific: drop every candidate another candidate overrides or whose
        // declaring type is a proper supertype of another candidate's declaring type.
        let mut targets: Vec<u32> = candidates
            .iter()
            .filter(|&&(m, decl)| {
                !candidates.iter().any(|&(other, other_decl)| {
                    other != m
                        && (self.overrides_transitively(other, m)
                            || (other_decl != decl && self.is_subtype(other_decl, decl)))
                })
            })
            .map(|&(m, _)| m)
            .collect();
        targets.sort_unstable();
        targets.dedup();
        match targets.len() {
            0 => Dispatch::NoImplementation,
            1 => Dispatch::Resolved {
                targets,
                exact: true,
            },
            _ => Dispatch::Resolved {
                targets,
                exact: false,
            },
        }
    }

    /// Class-hierarchy analysis: every implementation `method` can dispatch to on any
    /// instantiable subtype of its declaring type. A method without facts contributes its
    /// concrete overriders.
    pub fn cha_targets(&self, method: u32) -> Vec<u32> {
        let mut out: FxHashSet<u32> = FxHashSet::default();
        match self.method_facts.get(&method) {
            Some(facts) => {
                for t in self.descendants(facts.declaring_type) {
                    if self.abstract_types.contains(&t) {
                        continue;
                    }
                    if let Dispatch::Resolved { targets, .. } = self.dispatch(t, method) {
                        out.extend(targets);
                    }
                }
            }
            None => {
                for m in self.override_family(method) {
                    if let Some(f) = self.method_facts.get(&m) {
                        if !f.is_abstract {
                            out.insert(m);
                        }
                    }
                }
            }
        }
        let mut v: Vec<u32> = out.into_iter().collect();
        v.sort_unstable();
        v
    }

    /// Methods external code can invoke on an object of `dynamic_type` it holds: the
    /// implementations, with a body in the module (`has_body`), of instance methods declared
    /// by external supertypes. Returns (implementation, external declaration) pairs.
    pub fn external_entry_methods(
        &self,
        dynamic_type: u32,
        has_body: &dyn Fn(u32) -> bool,
    ) -> Vec<(u32, u32)> {
        let mut out: Vec<(u32, u32)> = Vec::new();
        if !self.knows_type(dynamic_type) {
            return out;
        }
        for a in self.ancestors(dynamic_type) {
            for &decl in self.methods_declared_in(a) {
                let Some(f) = self.method_facts.get(&decl) else {
                    continue;
                };
                if !f.is_external || !f.is_instance {
                    continue;
                }
                if let Dispatch::Resolved { targets, .. } = self.dispatch(dynamic_type, decl) {
                    for t in targets {
                        if has_body(t) && !out.iter().any(|&(i, _)| i == t) {
                            out.push((t, decl));
                        }
                    }
                }
            }
        }
        out
    }
}

// ── Call graph ──────────────────────────────────────────────────────────────

/// Dense id of a function instance (a function analyzed in one context).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct InstanceId(pub u32);

/// A function analyzed in one context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FunctionInstance {
    pub func: FuncId,
    pub context: Context,
}

/// Syntactic form of a call site (`Callee` variant).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CallKind {
    Static,
    Symbol,
    Virtual,
    Interface,
    Indirect,
    Unresolved,
}

/// Why an edge is an over-approximation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FallbackReason {
    /// The receiver object's dynamic type is not derivable (entry-parameter or external
    /// object): targets from class-hierarchy analysis of the declared method.
    ReceiverTypeUnknown,
    /// The override facts leave several most-specific implementations.
    AmbiguousOverride,
    /// Non-virtual call to an instance method whose receiver points to no object at the
    /// fixpoint; bound in the caller's context.
    NoReceiverObject,
    /// Overload resolution left several applicable methods (`CallSiteIr::candidates`);
    /// every candidate is a target.
    AmbiguousOverload,
}

/// Precision of one call edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EdgePrecision {
    /// Derived from a statically named callee or from the dynamic type of a receiver object.
    Exact,
    OverApproximate(FallbackReason),
}

/// A callee outside the module.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExternalTarget {
    /// A method or function symbol without a lowered body.
    Symbol(u32),
    /// A callee the frontend could not resolve to a symbol (interned name).
    Name(u32),
    /// A function value created outside the module.
    FunctionValue,
    /// A `Callee::Static` / closure target absent from the module.
    MissingFunction(FuncId),
}

/// Why (part of) a call site did not resolve to a lowered or external target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UnresolvedReason {
    /// The receiver or called value points to no object at the fixpoint: it is always
    /// null/undefined, or defined only by unreachable code.
    EmptyReceiverSet,
    /// The hierarchy knows the dynamic type but no implementation of the method in it; the
    /// call is treated as external so the analysis stays sound.
    NoImplementation { dynamic_type: u32 },
    /// No hierarchy facts for a receiver object's dynamic type; treated as external.
    HierarchyFactsMissing { dynamic_type: u32 },
    /// No facts about the called method, so its implementations cannot be enumerated for
    /// a receiver of unknown type; the declared method (if lowered) is bound and the call is
    /// also treated as external.
    MethodFactsMissing { method: u32 },
    /// The called value points to an object that is not a function value.
    NonCallableValue { object: ObjId },
    /// The solver stopped at its work budget before the site was resolved.
    Truncated,
}

/// Resolution of one call site, merged over all contexts.
#[derive(Debug, Clone)]
pub struct SiteResolution {
    pub site: CallSiteId,
    pub caller: FuncId,
    pub kind: CallKind,
    pub span: Span,
    /// Lowered callees with the best precision any context derived for them.
    pub targets: Vec<(FuncId, EdgePrecision)>,
    pub external: Vec<ExternalTarget>,
    pub unresolved: Vec<UnresolvedReason>,
}

impl SiteResolution {
    /// Every target derives from the program's facts without fallback.
    pub fn is_exact(&self) -> bool {
        self.unresolved.is_empty()
            && self
                .targets
                .iter()
                .all(|&(_, p)| p == EdgePrecision::Exact)
    }

    /// The site reaches at least one lowered or external target.
    pub fn is_resolved(&self) -> bool {
        !self.targets.is_empty() || !self.external.is_empty()
    }
}

/// One resolved call edge.
#[derive(Debug, Clone, Copy)]
pub struct CallEdgeInfo {
    pub site: CallSiteId,
    pub caller: FuncId,
    pub callee: FuncId,
    pub kind: CallKind,
    pub precision: EdgePrecision,
}

/// Why external code may call into the module.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CallbackOrigin {
    /// The object reached the external world and implements the external declaration.
    ExternalOverride { declaration: u32 },
    /// A closure / function value reached the external world.
    ExternalFunctionValue,
    /// An intrinsic modelled as invoking one of its arguments.
    Intrinsic { func: FuncId, name: u32 },
}

/// An invocation of module code that has no call site in the module.
#[derive(Debug, Clone, Copy)]
pub struct CallbackEdge {
    pub func: FuncId,
    /// The receiver or closure object the callback is invoked on.
    pub receiver: ObjId,
    pub origin: CallbackOrigin,
    /// The instruction that hands the object to external code or invokes it (`Span::NONE`
    /// for callbacks of objects that reach the external world flow-insensitively).
    pub span: Span,
}

/// A context-sensitive call edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ContextEdge {
    pub caller: InstanceId,
    pub site: CallSiteId,
    pub callee: InstanceId,
}

/// The call graph the points-to solver built.
#[derive(Debug, Clone, Default)]
pub struct CallGraph {
    /// (call site, lowered callee) pairs, sorted.
    pub edges: Vec<(CallSiteId, FuncId)>,
    /// Functions reachable from the entry points (and from callbacks), sorted.
    pub reachable: Vec<FuncId>,
    /// The entry points the solve started from.
    pub entries: Vec<FuncId>,
    /// Detail of every edge, parallel to `edges`.
    pub edge_info: Vec<CallEdgeInfo>,
    /// Every reachable call site, sorted by site id.
    pub sites: Vec<SiteResolution>,
    pub callbacks: Vec<CallbackEdge>,
    /// Function instances, indexed by `InstanceId`.
    pub instances: Vec<FunctionInstance>,
    pub context_edges: Vec<ContextEdge>,
    site_index: FxHashMap<CallSiteId, usize>,
}

impl CallGraph {
    /// Resolution of one call site, if it is reachable.
    pub fn site(&self, site: CallSiteId) -> Option<&SiteResolution> {
        self.site_index.get(&site).map(|&i| &self.sites[i])
    }

    /// Lowered callees of a call site.
    pub fn callees_of_site(&self, site: CallSiteId) -> Vec<FuncId> {
        self.site(site)
            .map(|s| s.targets.iter().map(|&(f, _)| f).collect())
            .unwrap_or_default()
    }

    /// (call site, caller) pairs of every call that reaches `func`.
    pub fn callers_of(&self, func: FuncId) -> Vec<(CallSiteId, FuncId)> {
        self.edge_info
            .iter()
            .filter(|e| e.callee == func)
            .map(|e| (e.site, e.caller))
            .collect()
    }

    /// Distinct lowered callees of `func`.
    pub fn callees_of(&self, func: FuncId) -> Vec<FuncId> {
        let mut out: Vec<FuncId> = self
            .edge_info
            .iter()
            .filter(|e| e.caller == func)
            .map(|e| e.callee)
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    #[inline]
    pub fn is_reachable(&self, func: FuncId) -> bool {
        self.reachable.binary_search(&func).is_ok()
    }

    #[inline]
    pub fn instance(&self, id: InstanceId) -> &FunctionInstance {
        &self.instances[id.0 as usize]
    }

    /// Reachable sites with at least one unresolved part.
    pub fn unresolved_sites(&self) -> impl Iterator<Item = &SiteResolution> {
        self.sites.iter().filter(|s| !s.unresolved.is_empty())
    }

    /// Reachable sites that call code outside the module.
    pub fn external_sites(&self) -> impl Iterator<Item = &SiteResolution> {
        self.sites.iter().filter(|s| !s.external.is_empty())
    }

    /// Strongly connected components of the reachable call graph, callees before callers
    /// (Tarjan emits SCCs in reverse topological order of the condensation), which is the
    /// order bottom-up summaries need. Each component is sorted.
    pub fn bottom_up_sccs(&self) -> Vec<Vec<FuncId>> {
        let n = self.reachable.len();
        let index_of = |f: FuncId| self.reachable.binary_search(&f).ok();
        let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
        for e in &self.edge_info {
            if let (Some(a), Some(b)) = (index_of(e.caller), index_of(e.callee)) {
                adj[a].push(b);
            }
        }
        for list in &mut adj {
            list.sort_unstable();
            list.dedup();
        }
        tarjan_sccs(&adj)
            .into_iter()
            .map(|comp| {
                let mut c: Vec<FuncId> = comp.into_iter().map(|i| self.reachable[i]).collect();
                c.sort_unstable();
                c
            })
            .collect()
    }

    /// Functions of an SCC that call each other (or themselves): recursive functions.
    pub fn recursive_functions(&self) -> Vec<FuncId> {
        let mut out = Vec::new();
        for comp in self.bottom_up_sccs() {
            if comp.len() > 1 {
                out.extend(comp);
            } else if let Some(&f) = comp.first() {
                if self.edge_info.iter().any(|e| e.caller == f && e.callee == f) {
                    out.push(f);
                }
            }
        }
        out.sort_unstable();
        out
    }
}

/// Iterative Tarjan over a dense adjacency list; components in completion order.
pub(crate) fn tarjan_sccs(adj: &[Vec<usize>]) -> Vec<Vec<usize>> {
    const UNVISITED: usize = usize::MAX;
    let n = adj.len();
    let mut index = vec![UNVISITED; n];
    let mut low = vec![0usize; n];
    let mut on_stack = vec![false; n];
    let mut stack: Vec<usize> = Vec::new();
    let mut frames: Vec<(usize, usize)> = Vec::new();
    let mut counter = 0usize;
    let mut out = Vec::new();

    for root in 0..n {
        if index[root] != UNVISITED {
            continue;
        }
        index[root] = counter;
        low[root] = counter;
        counter += 1;
        stack.push(root);
        on_stack[root] = true;
        frames.push((root, 0));

        while let Some(&(v, next)) = frames.last() {
            if next < adj[v].len() {
                frames.last_mut().expect("frame present").1 += 1;
                let w = adj[v][next];
                if index[w] == UNVISITED {
                    index[w] = counter;
                    low[w] = counter;
                    counter += 1;
                    stack.push(w);
                    on_stack[w] = true;
                    frames.push((w, 0));
                } else if on_stack[w] {
                    low[v] = low[v].min(index[w]);
                }
                continue;
            }
            frames.pop();
            if let Some(&(parent, _)) = frames.last() {
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
    out
}

// ── Builder used by the solver ──────────────────────────────────────────────

/// Accumulates call-graph facts while the points-to solver runs.
#[derive(Debug, Default)]
pub(crate) struct CallGraphBuilder {
    sites: Vec<SiteResolution>,
    site_index: FxHashMap<CallSiteId, usize>,
    target_index: FxHashMap<(CallSiteId, FuncId), usize>,
    context_edges: FxHashSet<ContextEdge>,
    callbacks: Vec<CallbackEdge>,
    callback_index: FxHashSet<(FuncId, ObjId, CallbackOrigin)>,
    reachable: FxHashSet<FuncId>,
    entries: Vec<FuncId>,
}

impl CallGraphBuilder {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Register a reachable call site; returns its index.
    pub(crate) fn note_site(
        &mut self,
        site: CallSiteId,
        caller: FuncId,
        kind: CallKind,
        span: Span,
    ) -> usize {
        if let Some(&i) = self.site_index.get(&site) {
            return i;
        }
        let i = self.sites.len();
        self.sites.push(SiteResolution {
            site,
            caller,
            kind,
            span,
            targets: Vec::new(),
            external: Vec::new(),
            unresolved: Vec::new(),
        });
        self.site_index.insert(site, i);
        i
    }

    pub(crate) fn add_edge(&mut self, site: usize, callee: FuncId, precision: EdgePrecision) {
        let sid = self.sites[site].site;
        match self.target_index.get(&(sid, callee)) {
            Some(&t) => {
                let entry = &mut self.sites[site].targets[t];
                if precision == EdgePrecision::Exact {
                    entry.1 = EdgePrecision::Exact;
                }
            }
            None => {
                let t = self.sites[site].targets.len();
                self.sites[site].targets.push((callee, precision));
                self.target_index.insert((sid, callee), t);
            }
        }
    }

    pub(crate) fn add_external(&mut self, site: usize, target: ExternalTarget) {
        let s = &mut self.sites[site];
        if !s.external.contains(&target) {
            s.external.push(target);
        }
    }

    pub(crate) fn add_unresolved(&mut self, site: usize, reason: UnresolvedReason) {
        let s = &mut self.sites[site];
        if !s.unresolved.contains(&reason) {
            s.unresolved.push(reason);
        }
    }

    pub(crate) fn add_context_edge(&mut self, edge: ContextEdge) {
        self.context_edges.insert(edge);
    }

    pub(crate) fn add_callback(&mut self, edge: CallbackEdge) {
        if self
            .callback_index
            .insert((edge.func, edge.receiver, edge.origin))
        {
            self.callbacks.push(edge);
        }
    }

    pub(crate) fn mark_reachable(&mut self, func: FuncId) {
        self.reachable.insert(func);
    }

    pub(crate) fn add_entry(&mut self, func: FuncId) {
        if !self.entries.contains(&func) {
            self.entries.push(func);
        }
    }

    /// Freeze into a [`CallGraph`]. Sites that resolved to nothing receive their reason:
    /// `Truncated` when the solver stopped early, `EmptyReceiverSet` otherwise.
    pub(crate) fn finish(mut self, instances: Vec<FunctionInstance>, truncated: bool) -> CallGraph {
        for s in &mut self.sites {
            if !s.is_resolved() && s.unresolved.is_empty() {
                s.unresolved.push(if truncated {
                    UnresolvedReason::Truncated
                } else {
                    UnresolvedReason::EmptyReceiverSet
                });
            }
            s.targets.sort_unstable_by_key(|&(f, _)| f);
        }
        self.sites.sort_unstable_by_key(|s| s.site);

        let mut site_index = FxHashMap::default();
        let mut edge_info = Vec::new();
        for (i, s) in self.sites.iter().enumerate() {
            site_index.insert(s.site, i);
            for &(callee, precision) in &s.targets {
                edge_info.push(CallEdgeInfo {
                    site: s.site,
                    caller: s.caller,
                    callee,
                    kind: s.kind,
                    precision,
                });
            }
        }
        let edges = edge_info.iter().map(|e| (e.site, e.callee)).collect();

        let mut reachable: Vec<FuncId> = self.reachable.into_iter().collect();
        reachable.sort_unstable();
        let mut context_edges: Vec<ContextEdge> = self.context_edges.into_iter().collect();
        context_edges.sort_unstable_by_key(|e| (e.caller, e.site, e.callee));

        CallGraph {
            edges,
            reachable,
            entries: self.entries,
            edge_info,
            sites: self.sites,
            callbacks: self.callbacks,
            instances,
            context_edges,
            site_index,
        }
    }
}
