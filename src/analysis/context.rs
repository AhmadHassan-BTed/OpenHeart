//! Calling contexts for context-sensitive interprocedural analysis.
//!
//! A context is a short string of *context elements*, most recent first, interned in a
//! [`ContextTable`]. The [`ContextSelector`] implements the classic families:
//!
//! - **k-limited call strings** (Sharir & Pnueli 1981): a callee's context is the call site
//!   pushed onto the caller's context, truncated to `k`.
//! - **Object sensitivity** (Milanova, Rountev & Ryder 2005): an instance method's context is
//!   its receiver's abstract object — allocation site followed by the object's heap context —
//!   truncated to `k`. Static calls keep the caller's context (Smaragdakis, Bravenboer &
//!   Lhoták 2011).
//! - **Type sensitivity** (Smaragdakis et al. 2011): as object sensitivity, with every
//!   allocation site replaced by the type that declares the allocating method; an allocating
//!   method outside any type keeps its allocation site.
//! - **Hybrid**: object sensitivity for instance calls, call strings for static calls.
//!
//! **Heap contexts.** An abstract object is an allocation site together with the context of
//! the method that allocated it, truncated to `heap_k`. A truncated context is marked, so
//! clients can tag derived facts as over-approximate when merging happened.
//!
//! [`ContextualCallGraph`] records the context-qualified call edges a solver discovers; returns
//! go back exactly along recorded edges, which keeps call/return matching precise even when
//! k-limiting makes the caller's context unrecoverable from the callee's.

use std::collections::HashMap;

use crate::ir::{AllocSiteId, CallSiteId, FuncId, ModuleIr};

/// One element of a context string.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ContextElement {
    CallSite(CallSiteId),
    Alloc(AllocSiteId),
    /// A type (symbol id) standing for the allocation sites of its methods.
    Type(u32),
}

/// A context string, most recent element first.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Default)]
pub struct Context {
    pub elements: Vec<ContextElement>,
    /// True when elements were dropped by k-limiting on the way to this context.
    pub truncated: bool,
}

/// Interned context id; `CtxId::EMPTY` is the empty context.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CtxId(pub u32);

impl CtxId {
    pub const EMPTY: CtxId = CtxId(0);
}

/// An abstract heap object: allocation site and heap context.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AbstractObject {
    pub site: AllocSiteId,
    pub heap_ctx: CtxId,
}

/// The context abstraction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextPolicy {
    /// One context for everything.
    Insensitive,
    CallString { k: usize, heap_k: usize },
    Object { k: usize, heap_k: usize },
    Type { k: usize, heap_k: usize },
    /// Object sensitivity for instance calls, call strings for static calls.
    Hybrid { k: usize, heap_k: usize },
}

impl ContextPolicy {
    pub fn k(self) -> usize {
        match self {
            ContextPolicy::Insensitive => 0,
            ContextPolicy::CallString { k, .. }
            | ContextPolicy::Object { k, .. }
            | ContextPolicy::Type { k, .. }
            | ContextPolicy::Hybrid { k, .. } => k,
        }
    }

    pub fn heap_k(self) -> usize {
        match self {
            ContextPolicy::Insensitive => 0,
            ContextPolicy::CallString { heap_k, .. }
            | ContextPolicy::Object { heap_k, .. }
            | ContextPolicy::Type { heap_k, .. }
            | ContextPolicy::Hybrid { heap_k, .. } => heap_k,
        }
    }
}

/// Interning table of contexts.
#[derive(Clone, Debug)]
pub struct ContextTable {
    contexts: Vec<Context>,
    index: HashMap<Context, CtxId>,
}

impl Default for ContextTable {
    fn default() -> Self {
        let empty = Context::default();
        let mut index = HashMap::new();
        index.insert(empty.clone(), CtxId::EMPTY);
        Self {
            contexts: vec![empty],
            index,
        }
    }
}

impl ContextTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn intern(&mut self, ctx: Context) -> CtxId {
        if let Some(&id) = self.index.get(&ctx) {
            return id;
        }
        let id = CtxId(self.contexts.len() as u32);
        self.contexts.push(ctx.clone());
        self.index.insert(ctx, id);
        id
    }

    pub fn get(&self, id: CtxId) -> &Context {
        &self.contexts[id.0 as usize]
    }

    pub fn len(&self) -> usize {
        self.contexts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.contexts.is_empty()
    }

    /// `head :: base`, truncated to `k` elements.
    fn push(&mut self, head: ContextElement, base: CtxId, k: usize) -> CtxId {
        if k == 0 {
            return CtxId::EMPTY;
        }
        let base_ctx = self.get(base);
        let mut elements = Vec::with_capacity(k);
        elements.push(head);
        elements.extend(base_ctx.elements.iter().copied().take(k - 1));
        let truncated = base_ctx.truncated || base_ctx.elements.len() > k - 1;
        self.intern(Context {
            elements,
            truncated,
        })
    }

    /// `ctx` truncated to `k` elements.
    fn truncate(&mut self, ctx: CtxId, k: usize) -> CtxId {
        let c = self.get(ctx);
        if c.elements.len() <= k {
            return ctx;
        }
        let elements = c.elements[..k].to_vec();
        self.intern(Context {
            elements,
            truncated: true,
        })
    }
}

/// Chooses callee and heap contexts under a [`ContextPolicy`].
pub struct ContextSelector {
    pub policy: ContextPolicy,
    pub table: ContextTable,
    /// Declaring type of the method that contains each allocation site (type sensitivity).
    alloc_owner: HashMap<AllocSiteId, Option<u32>>,
}

impl ContextSelector {
    pub fn new(module: &ModuleIr, policy: ContextPolicy) -> ContextSelector {
        let func_owner: HashMap<FuncId, Option<u32>> = module
            .funcs
            .iter()
            .map(|f| (f.id, f.enclosing_type))
            .collect();
        let alloc_owner = module
            .alloc_sites
            .iter()
            .map(|a| (a.id, func_owner.get(&a.func).copied().flatten()))
            .collect();
        ContextSelector {
            policy,
            table: ContextTable::new(),
            alloc_owner,
        }
    }

    pub fn context(&self, id: CtxId) -> &Context {
        self.table.get(id)
    }

    /// The element an object contributes to a context under the policy.
    fn object_element(&self, site: AllocSiteId) -> ContextElement {
        match self.policy {
            ContextPolicy::Type { .. } => match self.alloc_owner.get(&site).copied().flatten() {
                Some(t) => ContextElement::Type(t),
                None => ContextElement::Alloc(site),
            },
            _ => ContextElement::Alloc(site),
        }
    }

    /// Context of a callee entered through `site` from a caller analysed in `caller`.
    /// `receiver` is the abstract receiver object of an instance call (dispatch on it chose
    /// the callee), `None` for static and indirect calls.
    pub fn callee_context(
        &mut self,
        caller: CtxId,
        site: CallSiteId,
        receiver: Option<AbstractObject>,
    ) -> CtxId {
        let k = self.policy.k();
        match (self.policy, receiver) {
            (ContextPolicy::Insensitive, _) => CtxId::EMPTY,
            (ContextPolicy::CallString { .. }, _) | (ContextPolicy::Hybrid { .. }, None) => {
                self.table.push(ContextElement::CallSite(site), caller, k)
            }
            (ContextPolicy::Object { .. } | ContextPolicy::Type { .. } | ContextPolicy::Hybrid { .. }, Some(o)) => {
                let head = self.object_element(o.site);
                self.table.push(head, o.heap_ctx, k)
            }
            (ContextPolicy::Object { .. } | ContextPolicy::Type { .. }, None) => caller,
        }
    }

    /// Heap context of an object allocated by a method analysed in `method_ctx`.
    pub fn heap_context(&mut self, method_ctx: CtxId) -> CtxId {
        let heap_k = self.policy.heap_k();
        self.table.truncate(method_ctx, heap_k)
    }

    /// The abstract object allocated at `site` by a method analysed in `method_ctx`.
    pub fn abstract_object(&mut self, method_ctx: CtxId, site: AllocSiteId) -> AbstractObject {
        AbstractObject {
            site,
            heap_ctx: self.heap_context(method_ctx),
        }
    }
}

/// A context-qualified function.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ContextualFunc {
    pub func: FuncId,
    pub ctx: CtxId,
}

/// Context-qualified call edges discovered by a solver.
#[derive(Clone, Debug, Default)]
pub struct ContextualCallGraph {
    edges: Vec<(ContextualFunc, CallSiteId, ContextualFunc)>,
    seen: HashMap<(ContextualFunc, CallSiteId, ContextualFunc), u32>,
    by_callee: HashMap<ContextualFunc, Vec<u32>>,
    by_caller: HashMap<ContextualFunc, Vec<u32>>,
}

impl ContextualCallGraph {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records an edge; returns true when it is new.
    pub fn add(&mut self, caller: ContextualFunc, site: CallSiteId, callee: ContextualFunc) -> bool {
        let key = (caller, site, callee);
        if self.seen.contains_key(&key) {
            return false;
        }
        let id = self.edges.len() as u32;
        self.edges.push(key);
        self.seen.insert(key, id);
        self.by_callee.entry(callee).or_default().push(id);
        self.by_caller.entry(caller).or_default().push(id);
        true
    }

    /// The `(caller, site)` pairs a return from `callee` goes back to.
    pub fn return_targets(&self, callee: ContextualFunc) -> Vec<(ContextualFunc, CallSiteId)> {
        self.by_callee
            .get(&callee)
            .map(|ids| {
                ids.iter()
                    .map(|&i| {
                        let (caller, site, _) = self.edges[i as usize];
                        (caller, site)
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The `(site, callee)` pairs called from `caller`.
    pub fn callees(&self, caller: ContextualFunc) -> Vec<(CallSiteId, ContextualFunc)> {
        self.by_caller
            .get(&caller)
            .map(|ids| {
                ids.iter()
                    .map(|&i| {
                        let (_, site, callee) = self.edges[i as usize];
                        (site, callee)
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn edges(&self) -> &[(ContextualFunc, CallSiteId, ContextualFunc)] {
        &self.edges
    }

    /// The context-insensitive projection: `(site, callee)` edges without duplicates, sorted.
    pub fn project(&self) -> Vec<(CallSiteId, FuncId)> {
        let mut out: Vec<(CallSiteId, FuncId)> =
            self.edges.iter().map(|(_, site, callee)| (*site, callee.func)).collect();
        out.sort_unstable();
        out.dedup();
        out
    }
}
