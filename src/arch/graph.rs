//! Architecture dependency graph: typed, weighted dependencies with source provenance at
//! type, file, package/namespace and project level.
//!
//! **Units.** The finest level has one node per *architectural unit*: a named type (nested
//! named types are units of their own, linked to their outer type by `parent`), a member
//! declared outside every type (free function, module-level variable), the file-scope code
//! of a compilation unit, or an external declaration the project references. Members,
//! closures and anonymous classes fold into the nearest enclosing named unit: nothing
//! outside can name them, so they never form a separate dependency target.
//!
//! **Facts.** An edge `from → to` of kind `k` is the union of its *evidence*: dependency sites
//! (a declaration, an instruction, a call site, an allocation site, an import, an annotation
//! use, a name occurrence), each with its source span and strength. The weight of an edge is
//! its number of distinct sites; lifting to a coarser level unions evidence by site, so one
//! site that reaches two types of the same package counts once. Fact sources:
//!
//! - symbol table: declared supertypes with their type arguments, type-parameter bounds,
//!   declared field / parameter / local / return types (complete `TypeRef` trees), field
//!   associations, resolved imports and resolved name occurrences in bodies;
//! - IR: allocations, static callees and the declared method of virtual and interface calls,
//!   instance, static and module-variable accesses, casts and type tests, closures and
//!   function references, and the field, parameter, local and return types of the lowered
//!   program;
//! - points-to call graph: runtime dispatch targets with the solver's precision, external
//!   targets, unresolved sites, entry points and callbacks from external code;
//! - Phase 6 symbol-level call graph, for pipelines that do not lower to the IR;
//! - frontend facts the symbol table does not record: annotation uses and the project
//!   membership of files.
//!
//! Sources that describe the same site under the same origin (a declaration seen by the
//! symbol table and by the IR, a call site seen by the IR and by the call graph) merge into
//! one piece of evidence. Name occurrences and IR instructions describe body code at
//! different granularity, so `build_model` uses occurrences only when no IR is supplied.
//!
//! **Externals.** A unit is external when its symbol is `SK_EXTERNAL` or its declaration has
//! no source location in the analysed corpus. At every level coarser than type, external
//! units are aggregated to their namespace.
//!
//! **What is not derivable** is recorded, never filled in: unbound names, indirect calls the
//! call graph leaves unresolved and types that could not be determined become
//! [`UnresolvedFact`]s, dependencies on declarations whose binding Phase 3 could only infer
//! are `Partial` with the reason, and [`Coverage`] states which sources were supplied, so a
//! consumer can tell "no edge of this kind exists" from "this kind was not analysed".

use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::core::types::cg::{
    CallGraphArtifact, CallSite, CG_EDGE_CONSTRUCTOR, CG_EDGE_DIRECT, CG_EDGE_INTERFACE,
    CG_EDGE_SPECIAL, CG_EDGE_VIRTUAL,
};
use crate::core::types::symbol::{SymbolKind, SymbolModifiers, SymbolVisibility, THRelation};
use crate::derive::model::Derivation;
use crate::ingestion::TokenCorpusArtifact;
use crate::ir::func::{FuncIr, FuncKind};
use crate::ir::inst::{Callee, Inst};
use crate::ir::module::{CallSiteIr, FieldIr, GlobalIr, ModuleIr};
use crate::ir::types::{CallSiteId, FieldId, FuncId, GlobalId, IrType, Span, TypeId, TypeTable};
use crate::ir::value::Place;
use crate::memory::callgraph::{
    CallGraph, CallKind, CallbackOrigin, EdgePrecision, ExternalTarget, UnresolvedReason,
};
use crate::symbol::model::{
    EdgeFlags, OccurrenceRole, ResolutionStatus, SymbolFlags, TypeRef, TypeRefFlags,
};
use crate::symbol::scope_graph::ImportKind;
use crate::symbol::SymbolTableArtifact;

// ── Identifiers and vocabularies ─────────────────────────────────────────────

/// Index of a node in one [`DependencyGraph`]. Ids are dense and follow the order of node
/// names and keys, so identical inputs give identical ids whatever the fact insertion order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(pub u32);

impl NodeId {
    #[inline]
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// Granularity of a dependency graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Level {
    /// Architectural units: named types, members declared outside types, file-scope code and
    /// referenced external declarations.
    Type,
    /// Compilation units. External units are grouped by namespace.
    File,
    /// Packages and namespaces.
    Package,
    /// Projects: build modules, assemblies, targets or module declarations.
    Project,
}

impl Level {
    pub const ALL: [Level; 4] = [Level::Type, Level::File, Level::Package, Level::Project];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Type => "type",
            Self::File => "file",
            Self::Package => "package",
            Self::Project => "project",
        }
    }

    /// The level whose nodes contain the nodes of this one in the namespace hierarchy
    /// (types and files live in packages, packages in projects).
    pub fn container(self) -> Option<Level> {
        match self {
            Self::Type | Self::File => Some(Self::Package),
            Self::Package => Some(Self::Project),
            Self::Project => None,
        }
    }
}

/// Kind of a dependency edge.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DepKind {
    /// A compilation unit imports, includes or opens a declaration or namespace.
    Import = 0,
    /// Generalization: the unit extends the target.
    Inheritance = 1,
    /// Interface realization: the unit implements the target.
    Realization = 2,
    /// A field or module variable of the unit mentions the target in its declared type.
    FieldType = 3,
    /// A parameter of a function of the unit mentions the target in its declared type.
    ParameterType = 4,
    /// A function of the unit returns a type that mentions the target.
    ReturnType = 5,
    /// A call names a function declared by the target: the static callee, or the declared
    /// method of a virtual or interface call.
    Call = 6,
    /// The points-to call graph dispatches a call of the unit to a function of the target.
    Dispatch = 7,
    /// The unit allocates an instance of the target.
    Instantiation = 8,
    /// A declaration of the unit carries the target annotation or attribute type.
    AnnotationUse = 9,
    /// The unit reads, writes or releases a field or module variable of the target.
    FieldAccess = 10,
    /// The unit casts to, tests against, binds a type argument or a bound to, or declares a
    /// local of a type mentioning the target.
    TypeUse = 11,
    /// The unit creates a closure over, or a reference to, a function of the target.
    Reference = 12,
}

impl DepKind {
    pub const COUNT: usize = 13;

    pub const ALL: [DepKind; DepKind::COUNT] = [
        DepKind::Import,
        DepKind::Inheritance,
        DepKind::Realization,
        DepKind::FieldType,
        DepKind::ParameterType,
        DepKind::ReturnType,
        DepKind::Call,
        DepKind::Dispatch,
        DepKind::Instantiation,
        DepKind::AnnotationUse,
        DepKind::FieldAccess,
        DepKind::TypeUse,
        DepKind::Reference,
    ];

    #[inline]
    pub fn index(self) -> usize {
        self as usize
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Import => "import",
            Self::Inheritance => "inheritance",
            Self::Realization => "realization",
            Self::FieldType => "field_type",
            Self::ParameterType => "parameter_type",
            Self::ReturnType => "return_type",
            Self::Call => "call",
            Self::Dispatch => "dispatch",
            Self::Instantiation => "instantiation",
            Self::AnnotationUse => "annotation_use",
            Self::FieldAccess => "field_access",
            Self::TypeUse => "type_use",
            Self::Reference => "reference",
        }
    }

    /// True for dependencies the program text states (compile-time dependencies); false for
    /// runtime dispatch targets computed by the points-to analysis.
    pub fn is_static(self) -> bool {
        self != Self::Dispatch
    }
}

/// A set of dependency kinds (bit set over `DepKind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KindSet(u32);

impl KindSet {
    pub const NONE: KindSet = KindSet(0);
    pub const ALL: KindSet = KindSet((1u32 << DepKind::COUNT) - 1);
    /// Compile-time dependencies: every kind except points-to dispatch targets.
    pub const STATIC: KindSet =
        KindSet(((1u32 << DepKind::COUNT) - 1) & !(1u32 << DepKind::Dispatch as u32));

    pub fn of(kinds: &[DepKind]) -> Self {
        kinds.iter().fold(Self::NONE, |set, &k| set.with(k))
    }

    #[inline]
    pub fn contains(self, kind: DepKind) -> bool {
        self.0 & (1u32 << kind as u32) != 0
    }

    pub fn with(self, kind: DepKind) -> Self {
        Self(self.0 | (1u32 << kind as u32))
    }

    pub fn without(self, kind: DepKind) -> Self {
        Self(self.0 & !(1u32 << kind as u32))
    }

    pub fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub fn iter(self) -> impl Iterator<Item = DepKind> {
        DepKind::ALL.into_iter().filter(move |&k| self.contains(k))
    }
}

/// Strength of one piece of evidence. The derived order makes the stronger value smaller.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Strength {
    /// The dependency holds on every execution that reaches the site.
    Exact,
    /// The site may produce the dependency: a sound analysis could not exclude it.
    OverApproximate,
    /// The site establishes the dependency only under an assumption recorded as the reason.
    Partial,
}

/// How a result relates to the program; maps one to one onto the diagram model's
/// `Derivation`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Certainty {
    Exact,
    OverApproximate,
    Partial { reason: String },
    NotDerivable { reason: String },
}

impl Certainty {
    fn rank(&self) -> u8 {
        match self {
            Self::Exact => 0,
            Self::OverApproximate => 1,
            Self::Partial { .. } => 2,
            Self::NotDerivable { .. } => 3,
        }
    }

    pub fn from_strength(strength: Strength, reason: Option<&str>) -> Self {
        match strength {
            Strength::Exact => Self::Exact,
            Strength::OverApproximate => Self::OverApproximate,
            Strength::Partial => Self::Partial {
                reason: reason
                    .unwrap_or("the evidence holds only under an assumption of its source")
                    .to_string(),
            },
        }
    }

    pub fn is_exact(&self) -> bool {
        matches!(self, Self::Exact)
    }

    /// The stronger of two certainties (an edge exists as soon as one site establishes it).
    pub fn strongest(self, other: Self) -> Self {
        if other.rank() < self.rank() {
            other
        } else {
            self
        }
    }

    /// The weaker of two certainties (a result combining both is only as strong as its
    /// weakest input).
    pub fn weakest(self, other: Self) -> Self {
        if other.rank() > self.rank() {
            other
        } else {
            self
        }
    }
}

impl From<Certainty> for Derivation {
    fn from(c: Certainty) -> Self {
        match c {
            Certainty::Exact => Derivation::Exact,
            Certainty::OverApproximate => Derivation::OverApproximate,
            Certainty::Partial { reason } => Derivation::Partial { reason },
            Certainty::NotDerivable { reason } => Derivation::NotDerivable { reason },
        }
    }
}

/// The program site that establishes one piece of evidence. Two facts with the same origin
/// are the same evidence, whichever source reported them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Origin {
    /// A declaration whose declared type or annotation creates the dependency.
    Symbol(u32),
    /// A declared supertype relation (or type-parameter bound) of the symbol table.
    Hierarchy { sub: u32, sup: u32 },
    /// One IR instruction.
    Instruction { func: u32, block: u32, index: u32 },
    /// An IR call site.
    CallSite(u32),
    /// An IR allocation site.
    AllocSite(u32),
    /// An IR field without a declaring symbol.
    Field(u32),
    /// An IR module variable without a declaring symbol.
    Global(u32),
    /// A declared parameter of an IR function whose value names no symbol.
    Parameter { func: u32, index: u32 },
    /// An IR function without a symbol.
    Function(u32),
    /// A Phase 6 call site (symbol-level call graph).
    SymbolCallSite(u32),
    /// A name occurrence recorded by Phase 3, identified by its token.
    Occurrence(u32),
    /// An import of a compilation unit, identified by its first token.
    Import { file: u32, token: u32 },
    /// An annotation use on a declaration, identified by the annotation's first token.
    Annotation { target: u32, token: u32 },
    /// A fact supplied through [`ArchGraphBuilder::add_fact`], identified by its supplier.
    Supplied(u64),
}

/// One dependency site of an edge.
#[derive(Debug, Clone, Copy)]
pub struct Evidence {
    pub origin: Origin,
    pub strength: Strength,
    pub span: Option<Span>,
}

/// Kind of a node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum NodeKind {
    /// A named type or a top-level anonymous type.
    Type,
    /// A callable declared outside every type.
    Function,
    /// A variable declared outside every type.
    Variable,
    /// Code at file scope that no declaration encloses.
    FileScope,
    /// An external declaration whose declaration kind the symbol table does not record.
    External,
    File,
    Package,
    Project,
}

/// Role of an IR function inside its unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MemberRole {
    Constructor,
    Initializer,
    Method,
    Closure,
    /// A function that is not a member of a type.
    Function,
}

// ── Graph data ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ArchNode {
    pub id: NodeId,
    pub kind: NodeKind,
    /// Unique, stable key: the qualified name made unique by the symbol id, or the
    /// level-prefixed name of a group.
    pub key: String,
    /// Qualified display name as recorded by the symbol table (empty for the global
    /// namespace and for an unnamed default project).
    pub name: String,
    pub symbol: Option<u32>,
    pub symbol_kind: Option<SymbolKind>,
    /// Source file of the declaration (file nodes: the file itself).
    pub file: Option<u32>,
    pub external: bool,
    /// Interface, annotation type or abstract class (Martin's abstractness numerator).
    pub is_abstract: bool,
    pub visibility: Option<SymbolVisibility>,
    /// Lexical container at the same level: the outer type of a nested type, the parent
    /// namespace of a namespace.
    pub parent: Option<NodeId>,
    /// Declaration spans (packages: every package declaration; files: the whole file).
    pub spans: Vec<Span>,
}

#[derive(Debug, Clone)]
pub struct ArchEdge {
    pub from: NodeId,
    pub to: NodeId,
    pub kind: DepKind,
    /// Distinct dependency sites, sorted by origin.
    pub evidence: Vec<Evidence>,
    /// The strongest certainty among the sites.
    pub certainty: Certainty,
}

impl ArchEdge {
    /// Number of distinct dependency sites.
    pub fn weight(&self) -> u32 {
        self.evidence.len() as u32
    }

    /// Number of sites whose evidence is exact.
    pub fn exact_weight(&self) -> u32 {
        self.evidence
            .iter()
            .filter(|e| e.strength == Strength::Exact)
            .count() as u32
    }

    /// Distinct source spans of the sites, in source order.
    pub fn spans(&self) -> Vec<Span> {
        let mut seen = BTreeMap::new();
        for e in &self.evidence {
            if let Some(span) = e.span {
                seen.entry(span_key(&span)).or_insert(span);
            }
        }
        seen.into_values().collect()
    }
}

/// A dependency graph at one level. Edges are sorted by `(from, to, kind)`; dependencies
/// between members of the same node are not edges but count in `internal_weight`.
#[derive(Debug, Clone)]
pub struct DependencyGraph {
    pub level: Level,
    pub nodes: Vec<ArchNode>,
    pub edges: Vec<ArchEdge>,
    /// Distinct dependency sites between members of each node.
    pub internal_weight: Vec<u32>,
    out_offsets: Vec<u32>,
    in_offsets: Vec<u32>,
    in_edges: Vec<u32>,
    key_index: HashMap<String, u32>,
}

impl DependencyGraph {
    fn assemble(
        level: Level,
        nodes: Vec<ArchNode>,
        mut edges: Vec<ArchEdge>,
        internal_weight: Vec<u32>,
    ) -> Self {
        edges.sort_by(|a, b| (a.from, a.to, a.kind).cmp(&(b.from, b.to, b.kind)));
        let n = nodes.len();

        let mut out_offsets = vec![0u32; n + 1];
        let mut in_offsets = vec![0u32; n + 1];
        for e in &edges {
            out_offsets[e.from.index() + 1] += 1;
            in_offsets[e.to.index() + 1] += 1;
        }
        for i in 0..n {
            out_offsets[i + 1] += out_offsets[i];
            in_offsets[i + 1] += in_offsets[i];
        }

        let mut in_edges: Vec<u32> = (0..edges.len() as u32).collect();
        in_edges.sort_by_key(|&e| {
            let e = &edges[e as usize];
            (e.to, e.from, e.kind)
        });

        let key_index = nodes
            .iter()
            .map(|node| (node.key.clone(), node.id.0))
            .collect();

        Self {
            level,
            nodes,
            edges,
            internal_weight,
            out_offsets,
            in_offsets,
            in_edges,
            key_index,
        }
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    pub fn node(&self, id: NodeId) -> &ArchNode {
        &self.nodes[id.index()]
    }

    pub fn find(&self, key: &str) -> Option<NodeId> {
        self.key_index.get(key).map(|&i| NodeId(i))
    }

    /// Edges leaving `id`, sorted by `(to, kind)`.
    pub fn outgoing(&self, id: NodeId) -> &[ArchEdge] {
        let start = self.out_offsets[id.index()] as usize;
        let end = self.out_offsets[id.index() + 1] as usize;
        &self.edges[start..end]
    }

    /// Edges entering `id`, sorted by `(from, kind)`.
    pub fn incoming(&self, id: NodeId) -> impl Iterator<Item = &ArchEdge> + '_ {
        let start = self.in_offsets[id.index()] as usize;
        let end = self.in_offsets[id.index() + 1] as usize;
        self.in_edges[start..end]
            .iter()
            .map(move |&e| &self.edges[e as usize])
    }

    /// Edges `from → to` of every kind.
    pub fn edges_between(&self, from: NodeId, to: NodeId) -> &[ArchEdge] {
        let out = self.outgoing(from);
        let lo = out.partition_point(|e| e.to < to);
        let hi = out.partition_point(|e| e.to <= to);
        &out[lo..hi]
    }

    /// True when `id` takes part in an analysis that does or does not include externals.
    #[inline]
    pub fn included(&self, id: NodeId, include_external: bool) -> bool {
        include_external || !self.nodes[id.index()].external
    }

    /// Weighted adjacency over the edges of `kinds` (weights summed over kinds), restricted to
    /// project nodes unless `include_external`. Excluded nodes keep their index but have no
    /// incident edges.
    pub fn adjacency(&self, kinds: KindSet, include_external: bool) -> WeightedDigraph {
        WeightedDigraph::from_edges(
            self.nodes.len(),
            self.edges
                .iter()
                .filter(|e| {
                    kinds.contains(e.kind)
                        && self.included(e.from, include_external)
                        && self.included(e.to, include_external)
                })
                .map(|e| (e.from.0, e.to.0, u64::from(e.weight()))),
        )
    }
}

/// Compact weighted digraph (CSR, targets sorted, no self loops, parallel edges merged) used
/// by the partitioning, layering, clustering and cycle algorithms.
#[derive(Debug, Clone, Default)]
pub struct WeightedDigraph {
    n: usize,
    offsets: Vec<u32>,
    targets: Vec<u32>,
    weights: Vec<u64>,
}

impl WeightedDigraph {
    /// Builds the graph from `(from, to, weight)` triples. Self loops, zero weights and
    /// endpoints outside `0..n` are dropped; parallel edges are merged by summing weights.
    pub fn from_edges(n: usize, edges: impl IntoIterator<Item = (u32, u32, u64)>) -> Self {
        let mut list: Vec<(u32, u32, u64)> = edges
            .into_iter()
            .filter(|&(u, v, w)| u != v && (u as usize) < n && (v as usize) < n && w > 0)
            .collect();
        list.sort_unstable_by_key(|&(u, v, _)| (u, v));

        let mut offsets = vec![0u32; n + 1];
        let mut targets = Vec::with_capacity(list.len());
        let mut weights: Vec<u64> = Vec::with_capacity(list.len());
        let mut last: Option<(u32, u32)> = None;
        for (u, v, w) in list {
            if last == Some((u, v)) {
                if let Some(acc) = weights.last_mut() {
                    *acc = acc.saturating_add(w);
                }
                continue;
            }
            last = Some((u, v));
            offsets[u as usize + 1] += 1;
            targets.push(v);
            weights.push(w);
        }
        for i in 0..n {
            offsets[i + 1] += offsets[i];
        }
        Self {
            n,
            offsets,
            targets,
            weights,
        }
    }

    pub fn node_count(&self) -> usize {
        self.n
    }

    pub fn edge_count(&self) -> usize {
        self.targets.len()
    }

    /// Sorted successors of `v`.
    pub fn targets(&self, v: u32) -> &[u32] {
        let (s, e) = self.range(v);
        &self.targets[s..e]
    }

    pub fn successors(&self, v: u32) -> impl Iterator<Item = (u32, u64)> + '_ {
        let (s, e) = self.range(v);
        self.targets[s..e]
            .iter()
            .copied()
            .zip(self.weights[s..e].iter().copied())
    }

    /// Weight of `u → v`, 0 when absent.
    pub fn weight(&self, u: u32, v: u32) -> u64 {
        let (s, e) = self.range(u);
        match self.targets[s..e].binary_search(&v) {
            Ok(i) => self.weights[s + i],
            Err(_) => 0,
        }
    }

    pub fn out_degree(&self, v: u32) -> usize {
        let (s, e) = self.range(v);
        e - s
    }

    pub fn out_weight(&self, v: u32) -> u64 {
        let (s, e) = self.range(v);
        self.weights[s..e].iter().sum()
    }

    /// The graph with every edge reversed.
    pub fn transpose(&self) -> Self {
        let mut edges = Vec::with_capacity(self.targets.len());
        for u in 0..self.n as u32 {
            for (v, w) in self.successors(u) {
                edges.push((v, u, w));
            }
        }
        Self::from_edges(self.n, edges)
    }

    #[inline]
    fn range(&self, v: u32) -> (usize, usize) {
        let i = v as usize;
        if i >= self.n {
            return (0, 0);
        }
        (self.offsets[i] as usize, self.offsets[i + 1] as usize)
    }
}

// ── Facts carried next to the graphs ─────────────────────────────────────────

/// Callee of one call site as the IR states it.
#[derive(Debug, Clone, Copy)]
pub enum CallTarget {
    /// A statically bound function of the module.
    Function(FuncId),
    /// A declaration named by the call: an unlowered or external function, or the declared
    /// method of a virtual or interface call.
    Symbol(u32),
    /// A call through a function value.
    Indirect,
    /// A callee name bound to no declaration (interned name).
    Unresolved(u32),
}

/// One call site of a function body.
#[derive(Debug, Clone, Copy)]
pub struct SiteCall {
    pub site: CallSiteId,
    pub target: CallTarget,
    pub span: Option<Span>,
}

/// Body facts of one IR function, the input of LCOM4, function fan-in/fan-out and the
/// boundary analyses.
#[derive(Debug, Clone)]
pub struct FunctionFacts {
    pub func: FuncId,
    pub symbol: Option<u32>,
    pub kind: FuncKind,
    /// Type-level unit that owns the function.
    pub owner: NodeId,
    pub role: MemberRole,
    /// For closures: the named callable whose body creates the closure, when the symbol
    /// table records the nesting.
    pub enclosing_member: Option<u32>,
    pub span: Option<Span>,
    /// Fields accessed through an object (`Place::Field`), sorted and distinct.
    pub instance_fields: Vec<FieldId>,
    /// Static fields accessed (`Place::Static`): owner symbol and field, sorted and distinct.
    pub static_fields: Vec<(u32, FieldId)>,
    /// Module variables accessed, sorted and distinct.
    pub globals: Vec<GlobalId>,
    /// Statically bound callees, sorted and distinct.
    pub static_callees: Vec<FuncId>,
    /// Callee symbols of calls that are not lowered, external, virtual or interface calls,
    /// sorted and distinct.
    pub symbol_callees: Vec<u32>,
    /// Functions referenced by closures created in the body, sorted and distinct.
    pub closures: Vec<FuncId>,
    /// Call sites in instruction order.
    pub calls: Vec<SiteCall>,
    pub unresolved_calls: u32,
    pub instructions: u32,
}

/// One IR field with its owning unit.
#[derive(Debug, Clone)]
pub struct FieldFacts {
    pub field: FieldId,
    pub owner: NodeId,
    pub owner_symbol: u32,
    pub symbol: Option<u32>,
    pub is_static: bool,
    /// Synthesized by lowering (closure captures).
    pub synthetic: bool,
}

/// One resolved call edge at function granularity.
#[derive(Debug, Clone, Copy)]
pub struct CallFact {
    pub site: CallSiteId,
    pub caller: FuncId,
    pub callee: FuncId,
    pub span: Option<Span>,
    /// `Exact` for statically named callees and for targets derived from the receiver's
    /// dynamic type; `OverApproximate` for class-hierarchy and ambiguity fallbacks.
    pub strength: Strength,
}

/// Why external code may invoke module code that has no call site in the module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallbackKind {
    /// The receiver reached external code and implements the external declaration.
    ExternalOverride { declaration: u32 },
    /// A closure or function value reached external code.
    ExternalFunctionValue,
    /// An intrinsic invokes one of its arguments.
    Intrinsic { name: u32 },
}

/// A function the points-to analysis found invocable from external code.
#[derive(Debug, Clone, Copy)]
pub struct CallbackFact {
    pub func: FuncId,
    pub kind: CallbackKind,
    pub span: Option<Span>,
}

/// A dependency the facts name but do not resolve.
#[derive(Debug, Clone)]
pub struct UnresolvedFact {
    /// Unit that has the dependency, when known.
    pub from: Option<NodeId>,
    pub kind: DepKind,
    pub span: Option<Span>,
    /// Interned name of the unbound reference, when the source carries one.
    pub name: Option<u32>,
    pub reason: String,
}

/// Which fact sources were supplied to the builder.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Coverage {
    pub declarations: bool,
    pub imports: bool,
    pub occurrences: bool,
    pub ir: bool,
    pub call_graph: bool,
    pub symbol_calls: bool,
    pub annotations: bool,
    pub projects: bool,
}

impl Coverage {
    /// Whether the absence of `kind` edges means the program has none (`Exact`), or which
    /// part of the facts was not supplied.
    pub fn completeness(&self, kind: DepKind) -> Certainty {
        let missing = |reason: &str| Certainty::NotDerivable {
            reason: reason.to_string(),
        };
        let body = self.ir || self.occurrences;
        match kind {
            DepKind::Import if self.imports => Certainty::Exact,
            DepKind::Import => missing("no import facts were added"),
            DepKind::AnnotationUse if self.annotations => Certainty::Exact,
            DepKind::AnnotationUse => {
                missing("no annotation-use facts were supplied by the language frontend")
            }
            DepKind::Inheritance | DepKind::Realization if self.declarations => Certainty::Exact,
            DepKind::Inheritance | DepKind::Realization => {
                missing("the symbol-table declarations were not added")
            }
            DepKind::FieldType | DepKind::ParameterType | DepKind::ReturnType
                if self.declarations || self.ir =>
            {
                Certainty::Exact
            }
            DepKind::FieldType | DepKind::ParameterType | DepKind::ReturnType => {
                missing("neither declarations nor the IR were added")
            }
            DepKind::Call | DepKind::Instantiation | DepKind::FieldAccess if body => {
                Certainty::Exact
            }
            DepKind::Call | DepKind::Instantiation if self.symbol_calls => Certainty::Partial {
                reason: "calls come only from the Phase 6 symbol-level call graph".to_string(),
            },
            DepKind::Call | DepKind::Instantiation | DepKind::FieldAccess => {
                missing("neither the IR nor the symbol-table occurrences were added")
            }
            DepKind::Dispatch if self.call_graph => Certainty::OverApproximate,
            DepKind::Dispatch if self.symbol_calls => Certainty::Partial {
                reason: "dispatch targets come from the Phase 6 symbol-level call graph"
                    .to_string(),
            },
            DepKind::Dispatch => missing("the points-to call graph was not added"),
            DepKind::TypeUse if body => Certainty::Exact,
            DepKind::TypeUse if self.declarations => Certainty::Partial {
                reason: "only declared local types and type arguments are known; casts and type tests need the IR or the occurrences"
                    .to_string(),
            },
            DepKind::TypeUse => missing("neither declarations nor body facts were added"),
            DepKind::Reference if self.ir => Certainty::Exact,
            DepKind::Reference => missing("closures and function references need the IR"),
        }
    }
}

/// The dependency graphs at every level, their containment maps and the facts behind them.
#[derive(Debug, Clone)]
pub struct ArchModel {
    pub types: DependencyGraph,
    pub files: DependencyGraph,
    pub packages: DependencyGraph,
    pub projects: DependencyGraph,
    /// File-level node of each type-level node.
    pub type_file: Vec<NodeId>,
    /// Package-level node of each type-level node.
    pub type_package: Vec<NodeId>,
    /// Project-level node of each type-level node.
    pub type_project: Vec<NodeId>,
    /// Project-level node of each file-level node (external namespace groups map to their
    /// project-level namespace group); `None` when no unit or import ties the node to one.
    pub file_project: Vec<Option<NodeId>>,
    /// Body facts of every IR function, sorted by function id.
    pub functions: Vec<FunctionFacts>,
    /// IR fields, sorted by field id.
    pub fields: Vec<FieldFacts>,
    /// Function-level call edges, sorted by `(site, callee)`.
    pub calls: Vec<CallFact>,
    /// Functions the points-to call graph reaches, sorted; `None` without a call graph.
    pub reachable: Option<Vec<FuncId>>,
    /// Entry points the points-to solve started from; `None` without a call graph.
    pub entries: Option<Vec<FuncId>>,
    pub callbacks: Vec<CallbackFact>,
    pub unresolved: Vec<UnresolvedFact>,
    /// Namespace imports, which have no target at file level.
    pub namespace_imports_without_file_target: u32,
    pub coverage: Coverage,
}

impl ArchModel {
    pub fn graph(&self, level: Level) -> &DependencyGraph {
        match level {
            Level::Type => &self.types,
            Level::File => &self.files,
            Level::Package => &self.packages,
            Level::Project => &self.projects,
        }
    }

    /// Node of `level` containing the type-level node `unit`.
    pub fn container_of(&self, unit: NodeId, level: Level) -> NodeId {
        match level {
            Level::Type => unit,
            Level::File => self.type_file[unit.index()],
            Level::Package => self.type_package[unit.index()],
            Level::Project => self.type_project[unit.index()],
        }
    }

    /// Container node at `level` of every type-level node, indexed by type-level node id.
    pub fn type_partition(&self, level: Level) -> Vec<NodeId> {
        match level {
            Level::Type => (0..self.types.node_count() as u32).map(NodeId).collect(),
            Level::File => self.type_file.clone(),
            Level::Package => self.type_package.clone(),
            Level::Project => self.type_project.clone(),
        }
    }

    /// Position of each function's facts in `functions`, by `FuncId`.
    pub fn function_index(&self) -> HashMap<FuncId, usize> {
        self.functions
            .iter()
            .enumerate()
            .map(|(i, f)| (f.func, i))
            .collect()
    }

    /// The facts of `func`.
    pub fn function(&self, func: FuncId) -> Option<&FunctionFacts> {
        self.functions
            .binary_search_by_key(&func, |f| f.func)
            .ok()
            .map(|i| &self.functions[i])
    }
}

// ── Inputs supplied by the language frontends ────────────────────────────────

#[derive(Debug, Clone)]
pub struct AnnotationFact {
    /// Annotated declaration.
    pub annotated_symbol: u32,
    /// Annotation, attribute or decorator type.
    pub annotation_symbol: u32,
    pub span: Option<Span>,
}

/// A dependency fact of any kind supplied by another analysis.
#[derive(Debug, Clone)]
pub struct SuppliedFact {
    pub from_symbol: u32,
    pub to_symbol: u32,
    pub kind: DepKind,
    /// Identifies the site within the supplier; equal ids are the same evidence.
    pub site: u64,
    pub span: Option<Span>,
    pub strength: Strength,
    /// Assumption under which a `Partial` fact holds.
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct GraphConfig {
    /// Project of files that have neither an explicit assignment nor an enclosing module
    /// declaration. Empty means the analysed sources form one unnamed project.
    pub default_project: String,
}

/// Maps global token ids to source files and names files.
#[derive(Debug, Clone, Default)]
pub struct FileIndex {
    /// `(first_token, end_token_exclusive, file_id)`, sorted.
    ranges: Vec<(u32, u32, u32)>,
    paths: BTreeMap<u32, String>,
}

impl FileIndex {
    pub fn from_corpus(tca: &TokenCorpusArtifact) -> Self {
        let mut ranges = Vec::new();
        let mut paths = BTreeMap::new();
        for rec in &tca.file_records {
            let file = u32::from(rec.file_id);
            let path = String::from_utf8_lossy(tca.interner.lookup_text(rec.path_str_offset));
            paths.insert(file, path.into_owned());
            if rec.file_token_count > 0 && rec.first_token_id != u32::MAX {
                let end = rec.first_token_id.saturating_add(rec.file_token_count);
                ranges.push((rec.first_token_id, end, file));
            }
        }
        ranges.sort_unstable();
        Self { ranges, paths }
    }

    pub fn file_of_token(&self, token: u32) -> Option<u32> {
        let idx = self.ranges.partition_point(|r| r.0 <= token);
        let (_, end, file) = *self.ranges.get(idx.checked_sub(1)?)?;
        (token < end).then_some(file)
    }

    pub fn path(&self, file: u32) -> Option<&str> {
        self.paths.get(&file).map(String::as_str)
    }

    /// Span covering every token of `file`.
    pub fn file_span(&self, file: u32) -> Option<Span> {
        self.ranges
            .iter()
            .find(|r| r.2 == file)
            .map(|&(first, end, file_id)| {
                Span::new(file_id, first, end.saturating_sub(1).max(first))
            })
    }

    /// Span of the tokens `first..=last`, when they lie in one file.
    pub fn token_span(&self, first: u32, last: u32) -> Option<Span> {
        if first == u32::MAX {
            return None;
        }
        let file = self.file_of_token(first)?;
        let last = if last == u32::MAX || last < first || self.file_of_token(last) != Some(file) {
            first
        } else {
            last
        };
        Some(Span::new(file, first, last))
    }
}

// ── Builder ──────────────────────────────────────────────────────────────────

/// Draft of a type-level unit; ids are insertion indices until `finish` orders them.
#[derive(Debug, Clone)]
struct UnitDraft {
    kind: NodeKind,
    key: String,
    name: String,
    symbol: Option<u32>,
    symbol_kind: Option<SymbolKind>,
    file: Option<u32>,
    external: bool,
    is_abstract: bool,
    visibility: Option<SymbolVisibility>,
    parent: Option<u32>,
    /// Namespace name ("" is the global namespace); `None` for an external declaration
    /// whose namespace the symbol table does not record.
    package: Option<String>,
    /// Name of an enclosing module declaration.
    module: Option<String>,
    /// Why a dependency on this unit holds only partially (its binding was inferred).
    binding_note: Option<&'static str>,
    spans: Vec<Span>,
}

#[derive(Debug, Clone, Default)]
struct EvidenceAcc {
    sites: BTreeMap<Origin, (Strength, Option<Span>)>,
    partial_reason: Option<String>,
}

impl EvidenceAcc {
    fn add(&mut self, origin: Origin, strength: Strength, span: Option<Span>, reason: Option<&str>) {
        match self.sites.entry(origin) {
            Entry::Vacant(v) => {
                v.insert((strength, span));
            }
            Entry::Occupied(mut o) => {
                let slot = o.get_mut();
                if strength < slot.0 {
                    slot.0 = strength;
                }
                if slot.1.is_none() {
                    slot.1 = span;
                }
            }
        }
        if strength == Strength::Partial && self.partial_reason.is_none() {
            self.partial_reason = reason.map(str::to_string);
        }
    }

    /// Merges `other`, weakening every site to at least `cap`.
    fn merge(&mut self, other: &EvidenceAcc, cap: Strength) {
        for (&origin, &(strength, span)) in &other.sites {
            self.add(origin, strength.max(cap), span, other.partial_reason.as_deref());
        }
        if self.partial_reason.is_none() {
            self.partial_reason = other.partial_reason.clone();
        }
    }

    fn into_edge(self, from: NodeId, to: NodeId, kind: DepKind) -> ArchEdge {
        let best = self
            .sites
            .values()
            .map(|&(s, _)| s)
            .min()
            .unwrap_or(Strength::Partial);
        let certainty = Certainty::from_strength(best, self.partial_reason.as_deref());
        let evidence = self
            .sites
            .into_iter()
            .map(|(origin, (strength, span))| Evidence {
                origin,
                strength,
                span,
            })
            .collect();
        ArchEdge {
            from,
            to,
            kind,
            evidence,
            certainty,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum ImportKey {
    Unit(u32),
    Namespace(String),
}

/// Lookup tables over one IR module (ids index the owning vectors, but the tables do not
/// rely on it).
struct IrIndex {
    funcs: HashMap<u32, usize>,
    fields: HashMap<u32, usize>,
    globals: HashMap<u32, usize>,
    call_sites: HashMap<u32, usize>,
}

impl IrIndex {
    fn new(module: &ModuleIr) -> Self {
        Self {
            funcs: module
                .funcs
                .iter()
                .enumerate()
                .map(|(i, f)| (f.id.0, i))
                .collect(),
            fields: module
                .fields
                .iter()
                .enumerate()
                .map(|(i, f)| (f.id.0, i))
                .collect(),
            globals: module
                .globals
                .iter()
                .enumerate()
                .map(|(i, g)| (g.id.0, i))
                .collect(),
            call_sites: module
                .call_sites
                .iter()
                .enumerate()
                .map(|(i, c)| (c.id.0, i))
                .collect(),
        }
    }

    fn func<'m>(&self, module: &'m ModuleIr, id: FuncId) -> Option<&'m FuncIr> {
        self.funcs.get(&id.0).map(|&i| &module.funcs[i])
    }

    fn field<'m>(&self, module: &'m ModuleIr, id: FieldId) -> Option<&'m FieldIr> {
        self.fields.get(&id.0).map(|&i| &module.fields[i])
    }

    fn global<'m>(&self, module: &'m ModuleIr, id: GlobalId) -> Option<&'m GlobalIr> {
        self.globals.get(&id.0).map(|&i| &module.globals[i])
    }

    fn call_site<'m>(&self, module: &'m ModuleIr, id: CallSiteId) -> Option<&'m CallSiteIr> {
        self.call_sites.get(&id.0).map(|&i| &module.call_sites[i])
    }
}

/// Accumulates facts from every source and assembles the [`ArchModel`].
pub struct ArchGraphBuilder<'a> {
    sta: &'a SymbolTableArtifact,
    tca: &'a TokenCorpusArtifact,
    config: GraphConfig,
    files: FileIndex,
    units: Vec<UnitDraft>,
    unit_by_symbol: HashMap<u32, u32>,
    unit_by_file_scope: HashMap<u32, u32>,
    unit_cache: HashMap<u32, Option<u32>>,
    facts: BTreeMap<(u32, u32, DepKind), EvidenceAcc>,
    imports: BTreeMap<(u32, ImportKey), EvidenceAcc>,
    /// Declared `uses` relations, kept only where no field type explains them.
    pending_uses: Vec<(u32, u32, Origin, Option<Span>)>,
    /// Indirect call sites `(site, unit, span)` awaiting call-graph resolution.
    pending_indirect: Vec<(u32, u32, Option<Span>)>,
    resolved_sites: HashSet<u32>,
    file_projects: HashMap<u32, String>,
    functions: Vec<FunctionFacts>,
    fields: Vec<FieldFacts>,
    calls: BTreeMap<(u32, u32), CallFact>,
    reachable: Option<Vec<FuncId>>,
    entries: Option<Vec<FuncId>>,
    callbacks: Vec<CallbackFact>,
    unresolved: Vec<UnresolvedFact>,
    coverage: Coverage,
}

impl<'a> ArchGraphBuilder<'a> {
    pub fn new(
        sta: &'a SymbolTableArtifact,
        tca: &'a TokenCorpusArtifact,
        config: GraphConfig,
    ) -> Self {
        Self {
            sta,
            tca,
            config,
            files: FileIndex::from_corpus(tca),
            units: Vec::new(),
            unit_by_symbol: HashMap::new(),
            unit_by_file_scope: HashMap::new(),
            unit_cache: HashMap::new(),
            facts: BTreeMap::new(),
            imports: BTreeMap::new(),
            pending_uses: Vec::new(),
            pending_indirect: Vec::new(),
            resolved_sites: HashSet::new(),
            file_projects: HashMap::new(),
            functions: Vec::new(),
            fields: Vec::new(),
            calls: BTreeMap::new(),
            reachable: None,
            entries: None,
            callbacks: Vec::new(),
            unresolved: Vec::new(),
            coverage: Coverage::default(),
        }
    }

    pub fn file_index(&self) -> &FileIndex {
        &self.files
    }

    // ── Symbol table: declarations ───────────────────────────────────────────

    /// Adds every project declaration as a unit, the declared supertypes (with their type
    /// arguments), type-parameter bounds, the declared field / parameter / local / return
    /// types and the field associations.
    pub fn add_declarations(&mut self) {
        if self.coverage.declarations {
            return;
        }
        self.coverage.declarations = true;
        let sta = self.sta;
        let count = sta.symbol_records.len() as u32;

        // Units for every declaration of the project, referenced or not, so that isolated
        // types still count in abstractness, cohesion and the public surface.
        for sym in 0..count {
            let kind = SymbolKind::from(sta.symbol_records[sym as usize].kind);
            if is_namespace_kind(kind) || kind == SymbolKind::SK_EXTERNAL {
                continue;
            }
            if self.symbol_span(sym).is_some() {
                self.unit_of_symbol(sym);
            }
        }

        for edge in &sta.th_edges {
            let (Some(from), Some(to)) = (
                self.unit_of_symbol(edge.from_sym),
                self.unit_of_symbol(edge.to_sym),
            ) else {
                continue;
            };
            let origin = Origin::Hierarchy {
                sub: edge.from_sym,
                sup: edge.to_sym,
            };
            let span = self.symbol_span(edge.from_sym);
            let (strength, reason) = if edge.flags & EdgeFlags::RELATION_ASSUMED != 0 {
                (
                    Strength::Partial,
                    Some("extends versus implements was assumed: the supertype's kind is unknown"),
                )
            } else {
                (Strength::Exact, None)
            };
            match edge.relation {
                THRelation::TH_EXTENDS => {
                    self.record(from, to, DepKind::Inheritance, origin, strength, span, reason)
                }
                THRelation::TH_IMPLEMENTS => {
                    self.record(from, to, DepKind::Realization, origin, strength, span, reason)
                }
                THRelation::TH_CREATES => {
                    self.record(from, to, DepKind::Instantiation, origin, strength, span, reason)
                }
                // Phase 3 derives `uses` from field types; it is kept only where no field-type
                // fact explains it (decided in `finish`, once every source is in).
                THRelation::TH_USES => self.pending_uses.push((from, to, origin, span)),
            }
        }

        // Type arguments of written supertypes and the bounds of type parameters.
        for record in &sta.supertypes {
            let Some(from) = self.unit_of_symbol(record.sub) else {
                continue;
            };
            let Some(type_ref) = sta.type_ref(record.type_ref) else {
                continue;
            };
            let origin = Origin::Hierarchy {
                sub: record.sub,
                sup: record.sup,
            };
            let span = self
                .files
                .token_span(type_ref.first_token, type_ref.last_token)
                .or_else(|| self.symbol_span(record.sub));
            let is_bound = self.kind_of(record.sub) == Some(SymbolKind::SK_TYPE_PARAM);
            let mut mentions = Vec::new();
            let mut unknown = false;
            self.type_ref_mentions(type_ref, true, &mut mentions, &mut unknown);
            for (sym, head, binding) in mentions {
                // The head of a supertype is the inheritance edge itself.
                if head && !is_bound {
                    continue;
                }
                self.record_symbol_dependency(from, sym, DepKind::TypeUse, origin, span, binding);
            }
            if unknown {
                self.unresolved_type(from, DepKind::TypeUse, span);
            }
        }

        for sym in 0..count {
            let rec = sta.symbol_records[sym as usize];
            let kind = SymbolKind::from(rec.kind);
            let dep = match kind {
                SymbolKind::SK_FIELD | SymbolKind::SK_ENUM_CONSTANT => DepKind::FieldType,
                SymbolKind::SK_PARAM => DepKind::ParameterType,
                SymbolKind::SK_METHOD => DepKind::ReturnType,
                SymbolKind::SK_LOCAL_VAR => DepKind::TypeUse,
                _ => continue,
            };
            let Some(from) = self.unit_of_symbol(sym) else {
                continue;
            };
            let span = self.symbol_span(sym);
            let origin = Origin::Symbol(sym);
            match sta.type_ref_of(sym) {
                Some(type_ref) => {
                    let mut mentions = Vec::new();
                    let mut unknown = false;
                    self.type_ref_mentions(type_ref, true, &mut mentions, &mut unknown);
                    for (target, _, binding) in mentions {
                        self.record_symbol_dependency(from, target, dep, origin, span, binding);
                    }
                    if unknown {
                        self.unresolved_type(from, dep, span);
                    }
                }
                None => {
                    if let Some(target) = self.declared_type_symbol(rec.type_id) {
                        self.record_symbol_dependency(from, target, dep, origin, span, None);
                    }
                }
            }
        }

        for assoc in &sta.associations {
            let (Some(from), Some(to)) = (
                self.unit_of_symbol(assoc.from_symbol_id),
                self.unit_of_symbol(assoc.to_symbol_id),
            ) else {
                continue;
            };
            let (origin, span) = if assoc.field_symbol_id != u32::MAX {
                (
                    Origin::Symbol(assoc.field_symbol_id),
                    self.symbol_span(assoc.field_symbol_id),
                )
            } else {
                (
                    Origin::Hierarchy {
                        sub: assoc.from_symbol_id,
                        sup: assoc.to_symbol_id,
                    },
                    self.symbol_span(assoc.from_symbol_id),
                )
            };
            self.record(from, to, DepKind::FieldType, origin, Strength::Exact, span, None);
        }
    }

    /// Adds the import table of the symbol table. Each import is bound through the Phase 3
    /// occurrence at its last path segment; an on-demand import of a namespace targets the
    /// namespace, and an import Phase 3 could not bind is recorded as unresolved.
    pub fn add_symbol_imports(&mut self) {
        if self.coverage.imports {
            return;
        }
        self.coverage.imports = true;
        let sta = self.sta;
        for (_, decl) in &sta.imports {
            let Some(file) = self.files.file_of_token(decl.first_token) else {
                continue;
            };
            let span = self.files.token_span(decl.first_token, decl.name_token);
            let origin = Origin::Import {
                file,
                token: decl.first_token,
            };
            let on_demand = matches!(decl.kind, ImportKind::OnDemand);
            let binding = sta
                .occurrences_at_token(decl.name_token)
                .iter()
                .find(|o| o.role == OccurrenceRole::Import as u8);
            let status = binding.and_then(|o| ResolutionStatus::from_u8(o.status));
            let bound = binding
                .map(|o| o.symbol)
                .filter(|&s| (s as usize) < sta.symbol_records.len());

            let key = match bound {
                Some(sym) if self.kind_of(sym).is_some_and(is_namespace_kind) => {
                    ImportKey::Namespace(self.symbol_name(sym).unwrap_or_else(|| decl.path.clone()))
                }
                Some(sym) => match self.unit_of_symbol(sym) {
                    Some(unit) => ImportKey::Unit(unit),
                    None => continue,
                },
                None if on_demand => ImportKey::Namespace(decl.path.clone()),
                None => {
                    let reason = match status {
                        Some(ResolutionStatus::Ambiguous) => format!(
                            "the import `{}` matches {} declarations",
                            decl.path,
                            binding.map_or(0, |o| o.candidates.len())
                        ),
                        _ => format!("the import `{}` binds no declaration", decl.path),
                    };
                    self.unresolved.push(UnresolvedFact {
                        from: None,
                        kind: DepKind::Import,
                        span,
                        name: None,
                        reason,
                    });
                    continue;
                }
            };
            let (strength, reason) = binding_strength(status);
            self.imports
                .entry((file, key))
                .or_default()
                .add(origin, strength, span, reason);
        }
    }

    /// Adds the body dependencies Phase 3 resolved by name: type references, field reads
    /// and writes, calls and instantiations. Type references inside declared types are
    /// skipped (the declarations already record them). Use either this or `add_ir` for body
    /// facts: the two describe the same code at different granularity.
    pub fn add_occurrences(&mut self) {
        if self.coverage.occurrences {
            return;
        }
        self.coverage.occurrences = true;
        let sta = self.sta;
        let count = sta.symbol_records.len();

        // Token ranges of declared types, merged into disjoint intervals, so that references
        // inside declarations are not counted twice.
        let mut spans: Vec<(u32, u32)> = sta
            .symbol_type_refs
            .iter()
            .filter_map(|&id| sta.type_ref(id))
            .chain(sta.supertypes.iter().filter_map(|r| sta.type_ref(r.type_ref)))
            .filter(|t| t.first_token != u32::MAX)
            .map(|t| (t.first_token, t.last_token.max(t.first_token)))
            .collect();
        spans.sort_unstable();
        let mut declared: Vec<(u32, u32)> = Vec::with_capacity(spans.len());
        for (first, last) in spans {
            match declared.last_mut() {
                Some(prev) if first <= prev.1.saturating_add(1) => prev.1 = prev.1.max(last),
                _ => declared.push((first, last)),
            }
        }
        let in_declared_type = |token: u32| {
            let i = declared.partition_point(|r| r.0 <= token);
            i > 0 && declared[i - 1].1 >= token
        };

        // Declaration ranges for the innermost enclosing declaration of each occurrence.
        let mut ranges: Vec<(u32, u32, u32)> = (0..count as u32)
            .filter_map(|sym| {
                let rec = &sta.symbol_records[sym as usize];
                let kind = SymbolKind::from(rec.kind);
                (!is_namespace_kind(kind)
                    && kind != SymbolKind::SK_EXTERNAL
                    && rec.first_token_id != u32::MAX)
                    .then(|| {
                        let last = if rec.last_token_id == u32::MAX {
                            rec.first_token_id
                        } else {
                            rec.last_token_id.max(rec.first_token_id)
                        };
                        (rec.first_token_id, last, sym)
                    })
            })
            .collect();
        // Outer ranges before the ranges they contain.
        ranges.sort_unstable_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)).then(a.2.cmp(&b.2)));

        let mut open: Vec<(u32, u32)> = Vec::new();
        let mut next = 0usize;
        for occ in &sta.occurrences {
            let token = occ.token;
            while open.last().is_some_and(|&(last, _)| last < token) {
                open.pop();
            }
            while next < ranges.len() && ranges[next].0 <= token {
                let (_, last, sym) = ranges[next];
                while open.last().is_some_and(|&(l, _)| l < ranges[next].0) {
                    open.pop();
                }
                if last >= token {
                    open.push((last, sym));
                }
                next += 1;
            }
            while open.last().is_some_and(|&(last, _)| last < token) {
                open.pop();
            }

            let Some(role) = occurrence_role(occ.role) else {
                continue;
            };
            let provisional = match role {
                OccurrenceRole::TypeReference if in_declared_type(token) => continue,
                OccurrenceRole::TypeReference => DepKind::TypeUse,
                OccurrenceRole::Read | OccurrenceRole::Write => DepKind::FieldAccess,
                OccurrenceRole::Call => DepKind::Call,
                OccurrenceRole::Instantiation => DepKind::Instantiation,
                OccurrenceRole::Definition | OccurrenceRole::Import => continue,
            };
            let Some(file) = self.files.file_of_token(token) else {
                continue;
            };
            let from = match open.last() {
                Some(&(_, sym)) => match self.unit_of_symbol(sym) {
                    Some(unit) => unit,
                    None => continue,
                },
                None => self.file_scope_unit(file),
            };
            let span = Some(Span::new(file, token, token));
            let status = ResolutionStatus::from_u8(occ.status);
            match status {
                Some(ResolutionStatus::Primitive) | Some(ResolutionStatus::TypeInference) => {
                    continue
                }
                Some(ResolutionStatus::Unresolved) | Some(ResolutionStatus::Ambiguous) | None => {
                    self.unresolved.push(UnresolvedFact {
                        from: Some(NodeId(from)),
                        kind: provisional,
                        span,
                        name: None,
                        reason: if status == Some(ResolutionStatus::Ambiguous) {
                            format!(
                                "the name matches {} declarations at the same precedence",
                                occ.candidates.len()
                            )
                        } else {
                            "the name is not bound to any declaration".to_string()
                        },
                    });
                    continue;
                }
                _ => {}
            }
            if (occ.symbol as usize) >= count {
                continue;
            }
            let dep = match role {
                OccurrenceRole::Read | OccurrenceRole::Write => match self.kind_of(occ.symbol) {
                    Some(SymbolKind::SK_FIELD | SymbolKind::SK_ENUM_CONSTANT) => {
                        DepKind::FieldAccess
                    }
                    // A callable used as a value: a function or method reference.
                    Some(SymbolKind::SK_METHOD | SymbolKind::SK_CONSTRUCTOR) => DepKind::Reference,
                    Some(kind)
                        if is_named_type_kind(kind)
                            || kind == SymbolKind::SK_ANON_CLASS
                            || kind == SymbolKind::SK_EXTERNAL =>
                    {
                        DepKind::TypeUse
                    }
                    // Module-level variables are units; other locals and parameters stay
                    // inside the unit that declares them.
                    Some(SymbolKind::SK_LOCAL_VAR | SymbolKind::SK_PARAM)
                        if self.locate_unit_symbol(occ.symbol) == Some(occ.symbol) =>
                    {
                        DepKind::FieldAccess
                    }
                    _ => continue,
                },
                _ => provisional,
            };
            let (strength, reason) = binding_strength(status);
            if let Some(to) = self.unit_of_symbol(occ.symbol) {
                self.record(from, to, dep, Origin::Occurrence(token), strength, span, reason);
            }
        }
    }

    // ── IR ───────────────────────────────────────────────────────────────────

    /// Adds the field, module-variable, parameter, local and return types and the body facts
    /// of every function of `module`.
    pub fn add_ir(&mut self, module: &ModuleIr) {
        self.coverage.ir = true;
        let index = IrIndex::new(module);

        for field in &module.fields {
            let owner = if field.owner_symbol == u32::MAX {
                None
            } else {
                self.unit_of_symbol(field.owner_symbol)
            };
            let Some(owner) = owner else {
                self.unresolved.push(UnresolvedFact {
                    from: None,
                    kind: DepKind::FieldType,
                    span: None,
                    name: Some(field.name),
                    reason: "the declaring type of the field is not derivable".to_string(),
                });
                continue;
            };
            self.fields.push(FieldFacts {
                field: field.id,
                owner: NodeId(owner),
                owner_symbol: field.owner_symbol,
                symbol: field.symbol,
                is_static: field.is_static,
                synthetic: field.synthetic,
            });
            // Synthesized capture fields restate the types of captured variables, which the
            // enclosing code already depends on.
            if field.synthetic {
                continue;
            }
            let origin = field.symbol.map_or(Origin::Field(field.id.0), Origin::Symbol);
            let span = field.symbol.and_then(|s| self.symbol_span(s));
            self.record_type_tree(&module.types, owner, field.ty, DepKind::FieldType, origin, span);
        }

        for global in &module.globals {
            let owner = self.global_unit(global);
            let origin = global.symbol.map_or(Origin::Global(global.id.0), Origin::Symbol);
            let span = real_span(global.span).or_else(|| global.symbol.and_then(|s| self.symbol_span(s)));
            self.record_type_tree(&module.types, owner, global.ty, DepKind::FieldType, origin, span);
        }

        for func in &module.funcs {
            self.add_function(module, &index, func);
        }
    }

    fn add_function(&mut self, module: &ModuleIr, index: &IrIndex, func: &FuncIr) {
        let owner = self.function_owner(func);
        let role = self.member_role(func, owner);
        let enclosing_member = match (role, func.symbol) {
            (MemberRole::Closure, Some(sym)) => self.enclosing_callable(sym),
            _ => None,
        };
        let func_span = real_span(func.span);
        let func_origin = func.symbol.map_or(Origin::Function(func.id.0), Origin::Symbol);
        if !matches!(
            func.kind,
            FuncKind::Constructor
                | FuncKind::ImplicitConstructor
                | FuncKind::StaticInit
                | FuncKind::InstanceInit
                | FuncKind::ModuleInit
        ) {
            let span = func.symbol.and_then(|s| self.symbol_span(s)).or(func_span);
            self.record_type_tree(&module.types, owner, func.ret, DepKind::ReturnType, func_origin, span);
        }

        // Declared parameters and declared locals with their lowered types.
        for (i, &param) in func.declared_params().iter().enumerate() {
            let symbol = func.value_info(param).and_then(|v| v.symbol);
            let origin = symbol.map_or(
                Origin::Parameter {
                    func: func.id.0,
                    index: i as u32,
                },
                Origin::Symbol,
            );
            let span = symbol.and_then(|s| self.symbol_span(s)).or(func_span);
            let ty = func.value_type(param);
            self.record_type_tree(&module.types, owner, ty, DepKind::ParameterType, origin, span);
        }
        let mut locals: BTreeMap<u32, TypeId> = BTreeMap::new();
        for info in &func.values {
            if let Some(sym) = info.symbol {
                if self.kind_of(sym) == Some(SymbolKind::SK_LOCAL_VAR) {
                    locals.entry(sym).or_insert(info.ty);
                }
            }
        }
        for (sym, ty) in locals {
            let span = self.symbol_span(sym);
            self.record_type_tree(&module.types, owner, ty, DepKind::TypeUse, Origin::Symbol(sym), span);
        }

        let mut facts = FunctionFacts {
            func: func.id,
            symbol: func.symbol,
            kind: func.kind,
            owner: NodeId(owner),
            role,
            enclosing_member,
            span: func_span,
            instance_fields: Vec::new(),
            static_fields: Vec::new(),
            globals: Vec::new(),
            static_callees: Vec::new(),
            symbol_callees: Vec::new(),
            closures: Vec::new(),
            calls: Vec::new(),
            unresolved_calls: 0,
            instructions: 0,
        };

        for block in &func.blocks {
            for (i, inst) in block.insts.iter().enumerate() {
                facts.instructions += 1;
                let span = block.inst_spans.get(i).copied().and_then(real_span);
                let at = Origin::Instruction {
                    func: func.id.0,
                    block: block.id.0,
                    index: i as u32,
                };
                match inst {
                    Inst::Alloc { site, ty, .. } => self.record_type_tree(
                        &module.types,
                        owner,
                        *ty,
                        DepKind::Instantiation,
                        Origin::AllocSite(site.0),
                        span,
                    ),
                    Inst::Call { site, callee, .. } => {
                        self.add_call(module, index, func, owner, *site, callee, span, &mut facts)
                    }
                    Inst::Load { place, .. }
                    | Inst::Store { place, .. }
                    | Inst::Release { place, .. } => {
                        self.add_place(module, index, owner, place, at, span, &mut facts)
                    }
                    Inst::Cast { ty, .. } | Inst::TypeTest { ty, .. } => self.record_type_tree(
                        &module.types,
                        owner,
                        *ty,
                        DepKind::TypeUse,
                        at,
                        span,
                    ),
                    Inst::MakeClosure { func: target, .. } => match index.func(module, *target) {
                        Some(target_ir) => {
                            let to = self.function_owner(target_ir);
                            self.record(owner, to, DepKind::Reference, at, Strength::Exact, span, None);
                            facts.closures.push(*target);
                        }
                        None => self.unresolved.push(UnresolvedFact {
                            from: Some(NodeId(owner)),
                            kind: DepKind::Reference,
                            span,
                            name: None,
                            reason: "the closure's function is not part of the IR module".to_string(),
                        }),
                    },
                    _ => {}
                }
            }
        }

        facts.instance_fields.sort_unstable();
        facts.instance_fields.dedup();
        facts.static_fields.sort_unstable();
        facts.static_fields.dedup();
        facts.globals.sort_unstable();
        facts.globals.dedup();
        facts.static_callees.sort_unstable();
        facts.static_callees.dedup();
        facts.symbol_callees.sort_unstable();
        facts.symbol_callees.dedup();
        facts.closures.sort_unstable();
        facts.closures.dedup();
        self.functions.push(facts);
    }

    #[allow(clippy::too_many_arguments)]
    fn add_call(
        &mut self,
        module: &ModuleIr,
        index: &IrIndex,
        func: &FuncIr,
        owner: u32,
        site: CallSiteId,
        callee: &Callee,
        span: Option<Span>,
        facts: &mut FunctionFacts,
    ) {
        let span = span.or_else(|| index.call_site(module, site).and_then(|c| real_span(c.span)));
        let target = match callee {
            Callee::Static(f) => CallTarget::Function(*f),
            Callee::Symbol(sym)
            | Callee::Virtual { method: sym, .. }
            | Callee::Interface { method: sym, .. } => CallTarget::Symbol(*sym),
            Callee::Indirect(_) => CallTarget::Indirect,
            Callee::Unresolved(name) => CallTarget::Unresolved(*name),
        };
        facts.calls.push(SiteCall { site, target, span });
        let origin = Origin::CallSite(site.0);
        match callee {
            Callee::Static(target) => match index.func(module, *target) {
                Some(target_ir) => {
                    let to = self.function_owner(target_ir);
                    self.record(owner, to, DepKind::Call, origin, Strength::Exact, span, None);
                    facts.static_callees.push(*target);
                    self.calls.entry((site.0, target.0)).or_insert(CallFact {
                        site,
                        caller: func.id,
                        callee: *target,
                        span,
                        strength: Strength::Exact,
                    });
                }
                None => {
                    facts.unresolved_calls += 1;
                    self.unresolved.push(UnresolvedFact {
                        from: Some(NodeId(owner)),
                        kind: DepKind::Call,
                        span,
                        name: None,
                        reason: "the static callee is not part of the IR module".to_string(),
                    });
                }
            },
            Callee::Symbol(sym)
            | Callee::Virtual { method: sym, .. }
            | Callee::Interface { method: sym, .. } => {
                if self.record_symbol_dependency(owner, *sym, DepKind::Call, origin, span, None) {
                    facts.symbol_callees.push(*sym);
                } else {
                    facts.unresolved_calls += 1;
                }
            }
            Callee::Indirect(_) => self.pending_indirect.push((site.0, owner, span)),
            Callee::Unresolved(name) => {
                facts.unresolved_calls += 1;
                let candidates = index
                    .call_site(module, site)
                    .map_or(0, |c| c.candidates.len());
                self.unresolved.push(UnresolvedFact {
                    from: Some(NodeId(owner)),
                    kind: DepKind::Call,
                    span,
                    name: Some(*name),
                    reason: if candidates > 1 {
                        format!("overload resolution left {} applicable methods", candidates)
                    } else {
                        "the callee name is not bound to any declaration".to_string()
                    },
                });
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn add_place(
        &mut self,
        module: &ModuleIr,
        index: &IrIndex,
        owner: u32,
        place: &Place,
        at: Origin,
        span: Option<Span>,
        facts: &mut FunctionFacts,
    ) {
        match place {
            Place::Field { field, .. } => {
                facts.instance_fields.push(*field);
                match index.field(module, *field) {
                    Some(field_ir) if field_ir.owner_symbol != u32::MAX => {
                        self.record_symbol_dependency(
                            owner,
                            field_ir.owner_symbol,
                            DepKind::FieldAccess,
                            at,
                            span,
                            None,
                        );
                    }
                    _ => self.unresolved.push(UnresolvedFact {
                        from: Some(NodeId(owner)),
                        kind: DepKind::FieldAccess,
                        span,
                        name: index.field(module, *field).map(|f| f.name),
                        reason: "the declaring type of the accessed field is not derivable"
                            .to_string(),
                    }),
                }
            }
            Place::Static {
                owner: owner_sym,
                field,
                ..
            } => {
                facts.static_fields.push((*owner_sym, *field));
                self.record_symbol_dependency(owner, *owner_sym, DepKind::FieldAccess, at, span, None);
            }
            Place::Global(global) => {
                facts.globals.push(*global);
                match index.global(module, *global) {
                    Some(global_ir) => {
                        let to = self.global_unit(global_ir);
                        self.record(owner, to, DepKind::FieldAccess, at, Strength::Exact, span, None);
                    }
                    None => self.unresolved.push(UnresolvedFact {
                        from: Some(NodeId(owner)),
                        kind: DepKind::FieldAccess,
                        span,
                        name: None,
                        reason: "the accessed module variable is not part of the IR module"
                            .to_string(),
                    }),
                }
            }
            _ => {}
        }
    }

    /// Records one dependency per named type in the tree of `ty`. For instantiations the
    /// allocated head type is the instantiated unit and every other mention is a type use.
    fn record_type_tree(
        &mut self,
        types: &TypeTable,
        from: u32,
        ty: TypeId,
        kind: DepKind,
        origin: Origin,
        span: Option<Span>,
    ) {
        let (named, unknown) = named_in_type(types, ty);
        for (sym, head) in named {
            let dep = if kind == DepKind::Instantiation && !head {
                DepKind::TypeUse
            } else {
                kind
            };
            self.record_symbol_dependency(from, sym, dep, origin, span, None);
        }
        if unknown {
            self.unresolved_type(from, kind, span);
        }
    }

    // ── Call graphs ──────────────────────────────────────────────────────────

    /// Adds the points-to call graph: call and dispatch edges with the solver's precision,
    /// external dispatch targets, the reasons of unresolved sites, the reachable set, the
    /// entry points and the callbacks from external code.
    pub fn add_call_graph(&mut self, module: &ModuleIr, call_graph: &CallGraph) {
        self.coverage.call_graph = true;
        let index = IrIndex::new(module);

        for info in &call_graph.edge_info {
            self.resolved_sites.insert(info.site.0);
            let (Some(caller), Some(target)) =
                (index.func(module, info.caller), index.func(module, info.callee))
            else {
                self.unresolved.push(UnresolvedFact {
                    from: None,
                    kind: DepKind::Dispatch,
                    span: None,
                    name: None,
                    reason: "the call graph names a function that is not part of the IR module"
                        .to_string(),
                });
                continue;
            };
            let span = index
                .call_site(module, info.site)
                .and_then(|c| real_span(c.span));
            let strength = precision_strength(info.precision);
            let entry = self
                .calls
                .entry((info.site.0, info.callee.0))
                .or_insert(CallFact {
                    site: info.site,
                    caller: info.caller,
                    callee: info.callee,
                    span,
                    strength,
                });
            if strength < entry.strength {
                entry.strength = strength;
            }
            let dep = match info.kind {
                CallKind::Static | CallKind::Symbol => DepKind::Call,
                CallKind::Virtual | CallKind::Interface | CallKind::Indirect | CallKind::Unresolved => {
                    DepKind::Dispatch
                }
            };
            let from = self.function_owner(caller);
            let to = self.function_owner(target);
            self.record(from, to, dep, Origin::CallSite(info.site.0), strength, span, None);
        }

        for site in &call_graph.sites {
            self.resolved_sites.insert(site.site.0);
            let Some(caller) = index.func(module, site.caller) else {
                continue;
            };
            let from = self.function_owner(caller);
            let span = real_span(site.span);
            let origin = Origin::CallSite(site.site.0);
            let dynamic = matches!(
                site.kind,
                CallKind::Virtual | CallKind::Interface | CallKind::Indirect | CallKind::Unresolved
            );
            let strength = if site.is_exact() {
                Strength::Exact
            } else {
                Strength::OverApproximate
            };
            for external in &site.external {
                match *external {
                    ExternalTarget::Symbol(sym) => {
                        let dep = if dynamic { DepKind::Dispatch } else { DepKind::Call };
                        if let Some(to) = self.unit_of_symbol(sym) {
                            self.record(from, to, dep, origin, strength, span, None);
                        }
                    }
                    ExternalTarget::Name(name) if !self.coverage.ir => {
                        self.unresolved.push(UnresolvedFact {
                            from: Some(NodeId(from)),
                            kind: DepKind::Call,
                            span,
                            name: Some(name),
                            reason: "the callee name is not bound to any declaration".to_string(),
                        })
                    }
                    ExternalTarget::FunctionValue => self.unresolved.push(UnresolvedFact {
                        from: Some(NodeId(from)),
                        kind: DepKind::Dispatch,
                        span,
                        name: None,
                        reason: "the called function value was created outside the module"
                            .to_string(),
                    }),
                    ExternalTarget::MissingFunction(_) if !self.coverage.ir => {
                        self.unresolved.push(UnresolvedFact {
                            from: Some(NodeId(from)),
                            kind: DepKind::Call,
                            span,
                            name: None,
                            reason: "the callee function is not part of the IR module".to_string(),
                        })
                    }
                    ExternalTarget::Name(_) | ExternalTarget::MissingFunction(_) => {}
                }
            }
            for reason in &site.unresolved {
                let reason = self.unresolved_reason_text(reason);
                self.unresolved.push(UnresolvedFact {
                    from: Some(NodeId(from)),
                    kind: DepKind::Dispatch,
                    span,
                    name: None,
                    reason,
                });
            }
        }

        let mut reachable = call_graph.reachable.clone();
        reachable.sort_unstable();
        reachable.dedup();
        self.reachable = Some(reachable);
        let mut entries = call_graph.entries.clone();
        entries.sort_unstable();
        entries.dedup();
        self.entries = Some(entries);
        for callback in &call_graph.callbacks {
            let kind = match callback.origin {
                CallbackOrigin::ExternalOverride { declaration } => {
                    CallbackKind::ExternalOverride { declaration }
                }
                CallbackOrigin::ExternalFunctionValue => CallbackKind::ExternalFunctionValue,
                CallbackOrigin::Intrinsic { name, .. } => CallbackKind::Intrinsic { name },
            };
            self.callbacks.push(CallbackFact {
                func: callback.func,
                kind,
                span: real_span(callback.span),
            });
        }
    }

    fn unresolved_reason_text(&self, reason: &UnresolvedReason) -> String {
        let name = |sym: u32| {
            self.symbol_name(sym)
                .unwrap_or_else(|| format!("#{}", sym))
        };
        match *reason {
            UnresolvedReason::EmptyReceiverSet => {
                "the receiver or called value points to no object at the fixpoint".to_string()
            }
            UnresolvedReason::NoImplementation { dynamic_type } => format!(
                "the type hierarchy records no implementation of the method in `{}`",
                name(dynamic_type)
            ),
            UnresolvedReason::HierarchyFactsMissing { dynamic_type } => format!(
                "no type-hierarchy facts for the receiver's dynamic type `{}`",
                name(dynamic_type)
            ),
            UnresolvedReason::MethodFactsMissing { method } => format!(
                "no facts about `{}`, so its implementations cannot be enumerated",
                name(method)
            ),
            UnresolvedReason::NonCallableValue { .. } => {
                "the called value points to an object that is not a function value".to_string()
            }
            UnresolvedReason::Truncated => {
                "the points-to solve stopped at its work budget before resolving the site"
                    .to_string()
            }
        }
    }

    /// Adds the symbol-level call edges of a Phase 6 call graph, for pipelines that have not
    /// lowered to the IR. Direct and special calls are exact calls, constructor calls are
    /// exact instantiations, virtual and interface calls are dispatches (exact when the site
    /// has one target), and dynamic or reflective edges are partial dispatches.
    pub fn add_symbol_call_graph(&mut self, cga: &CallGraphArtifact) {
        self.coverage.symbol_calls = true;
        let sites: HashMap<u32, &CallSite> = cga
            .call_sites
            .iter()
            .map(|s| (s.call_site_id, s))
            .collect();
        let mut targets_per_site: HashMap<u32, u32> = HashMap::new();
        for &(_, _, site) in &cga.site_to_edge_map {
            *targets_per_site.entry(site).or_insert(0) += 1;
        }

        for &(caller, callee, site_id) in &cga.site_to_edge_map {
            let Some(from) = self.unit_of_symbol(caller) else {
                continue;
            };
            let site = sites.get(&site_id).copied();
            let span = site.and_then(|s| self.files.token_span(s.call_token, s.call_token));
            let single = targets_per_site.get(&site_id).copied() == Some(1);
            let (kind, strength, reason) = match site.map(|s| s.call_type) {
                Some(CG_EDGE_DIRECT) | Some(CG_EDGE_SPECIAL) => (DepKind::Call, Strength::Exact, None),
                Some(CG_EDGE_CONSTRUCTOR) => (DepKind::Instantiation, Strength::Exact, None),
                Some(CG_EDGE_VIRTUAL) | Some(CG_EDGE_INTERFACE) if single => {
                    (DepKind::Dispatch, Strength::Exact, None)
                }
                Some(CG_EDGE_VIRTUAL) | Some(CG_EDGE_INTERFACE) => {
                    (DepKind::Dispatch, Strength::OverApproximate, None)
                }
                Some(_) => (
                    DepKind::Dispatch,
                    Strength::Partial,
                    Some("dynamic or reflective call bound by Phase 6 name matching"),
                ),
                None => (
                    DepKind::Dispatch,
                    Strength::Partial,
                    Some("the Phase 6 call site record is missing; the call kind is unknown"),
                ),
            };
            let Some(to) = self.unit_of_symbol(callee) else {
                continue;
            };
            self.record(from, to, kind, Origin::SymbolCallSite(site_id), strength, span, reason);
        }
    }

    // ── Frontend facts ───────────────────────────────────────────────────────

    /// Adds annotation, attribute and decorator uses. Calling it states that the frontend
    /// supplied all of them.
    pub fn add_annotation_uses<I: IntoIterator<Item = AnnotationFact>>(&mut self, uses: I) {
        self.coverage.annotations = true;
        for fact in uses {
            let Some(from) = self.unit_of_symbol(fact.annotated_symbol) else {
                self.unresolved.push(UnresolvedFact {
                    from: None,
                    kind: DepKind::AnnotationUse,
                    span: fact.span,
                    name: None,
                    reason: "the annotated declaration is not in the symbol table".to_string(),
                });
                continue;
            };
            let origin = Origin::Annotation {
                target: fact.annotated_symbol,
                token: fact.span.map_or(u32::MAX, |s| s.first_token),
            };
            self.record_symbol_dependency(
                from,
                fact.annotation_symbol,
                DepKind::AnnotationUse,
                origin,
                fact.span,
                None,
            );
        }
    }

    /// Assigns files to projects (build modules, assemblies, targets) as read from the build
    /// configuration. Calling it states that the project model was supplied.
    pub fn assign_projects<I: IntoIterator<Item = (u32, String)>>(&mut self, files: I) {
        self.coverage.projects = true;
        for (file, project) in files {
            self.file_projects.insert(file, project);
        }
    }

    /// Adds one dependency fact computed by another analysis.
    pub fn add_fact(&mut self, fact: SuppliedFact) {
        let (Some(from), Some(to)) = (
            self.unit_of_symbol(fact.from_symbol),
            self.unit_of_symbol(fact.to_symbol),
        ) else {
            self.unresolved.push(UnresolvedFact {
                from: None,
                kind: fact.kind,
                span: fact.span,
                name: None,
                reason: "an endpoint of the supplied fact is not in the symbol table".to_string(),
            });
            return;
        };
        self.record(
            from,
            to,
            fact.kind,
            Origin::Supplied(fact.site),
            fact.strength,
            fact.span,
            fact.reason.as_deref(),
        );
    }

    // ── Recording ────────────────────────────────────────────────────────────

    /// Records a dependency; evidence on a unit whose binding was only inferred is partial.
    #[allow(clippy::too_many_arguments)]
    fn record(
        &mut self,
        from: u32,
        to: u32,
        kind: DepKind,
        origin: Origin,
        strength: Strength,
        span: Option<Span>,
        reason: Option<&str>,
    ) {
        let (strength, reason) = match self.units[to as usize].binding_note {
            Some(note) if strength < Strength::Partial => (Strength::Partial, Some(note)),
            _ => (strength, reason),
        };
        self.facts
            .entry((from, to, kind))
            .or_default()
            .add(origin, strength, span, reason);
    }

    /// Records a dependency on the unit of `sym`, or an unresolved fact when `sym` is not in
    /// the symbol table. Returns whether the dependency was recorded.
    fn record_symbol_dependency(
        &mut self,
        from: u32,
        sym: u32,
        kind: DepKind,
        origin: Origin,
        span: Option<Span>,
        binding: Option<&'static str>,
    ) -> bool {
        match self.unit_of_symbol(sym) {
            Some(to) => {
                let strength = if binding.is_some() {
                    Strength::Partial
                } else {
                    Strength::Exact
                };
                self.record(from, to, kind, origin, strength, span, binding);
                true
            }
            None => {
                self.unresolved.push(UnresolvedFact {
                    from: Some(NodeId(from)),
                    kind,
                    span,
                    name: None,
                    reason: "the referenced symbol is not in the symbol table".to_string(),
                });
                false
            }
        }
    }

    fn unresolved_type(&mut self, from: u32, kind: DepKind, span: Option<Span>) {
        self.unresolved.push(UnresolvedFact {
            from: Some(NodeId(from)),
            kind,
            span,
            name: None,
            reason: "the type (or part of it) could not be determined".to_string(),
        });
    }

    /// Declarations mentioned by a `TypeRef` tree, each with whether it is the head of the
    /// root and the reason its binding is only inferred; type variables and primitives are
    /// not dependencies.
    fn type_ref_mentions(
        &self,
        root: &TypeRef,
        head: bool,
        out: &mut Vec<(u32, bool, Option<&'static str>)>,
        unknown: &mut bool,
    ) {
        let sta = self.sta;
        let mut stack: Vec<(&TypeRef, bool)> = vec![(root, head)];
        let mut steps = 0usize;
        while let Some((t, is_head)) = stack.pop() {
            steps += 1;
            if steps > sta.type_refs.len() + 1 {
                break;
            }
            let skip_head = t.flags & (TypeRefFlags::PRIMITIVE | TypeRefFlags::TYPE_VARIABLE) != 0;
            if t.flags & TypeRefFlags::UNKNOWN != 0 {
                *unknown = true;
            } else if !skip_head && t.head != u32::MAX {
                if (t.head as usize) < sta.symbol_records.len() {
                    let (_, binding) = binding_strength(ResolutionStatus::from_u8(t.status));
                    out.push((t.head, is_head, binding));
                }
            } else if t.head == u32::MAX
                && !skip_head
                && !t.is_wildcard()
                && t.flags & TypeRefFlags::UNION == 0
            {
                // A written type without a binding.
                *unknown = true;
            }
            for &arg in &t.args {
                if let Some(a) = sta.type_ref(arg) {
                    // Alternatives of a union type are heads of their own.
                    let arg_head = is_head && t.flags & TypeRefFlags::UNION != 0;
                    stack.push((a, arg_head));
                }
            }
        }
    }

    // ── Units ────────────────────────────────────────────────────────────────

    /// Unit (draft index) of the declaration `sym`, creating it on first use.
    fn unit_of_symbol(&mut self, sym: u32) -> Option<u32> {
        if let Some(&cached) = self.unit_cache.get(&sym) {
            return cached;
        }
        let unit = self
            .locate_unit_symbol(sym)
            .map(|unit_sym| self.ensure_symbol_unit(unit_sym));
        self.unit_cache.insert(sym, unit);
        unit
    }

    /// The declaration that names the unit containing `sym`: the nearest enclosing named
    /// type, otherwise the outermost declaration below the namespace level.
    fn locate_unit_symbol(&self, sym: u32) -> Option<u32> {
        let records = &self.sta.symbol_records;
        let mut outermost = None;
        let mut cur = sym;
        let mut steps = 0usize;
        while (cur as usize) < records.len() && steps <= records.len() {
            let kind = SymbolKind::from(records[cur as usize].kind);
            if is_namespace_kind(kind) {
                break;
            }
            if is_named_type_kind(kind) {
                return Some(cur);
            }
            outermost = Some(cur);
            cur = records[cur as usize].parent_sym;
            steps += 1;
        }
        outermost
    }

    fn ensure_symbol_unit(&mut self, sym: u32) -> u32 {
        if let Some(&unit) = self.unit_by_symbol.get(&sym) {
            return unit;
        }
        let rec = self.sta.symbol_records[sym as usize];
        let kind = SymbolKind::from(rec.kind);
        let span = self.symbol_span(sym);
        let file = span.map(|s| s.file_id);
        let external = kind == SymbolKind::SK_EXTERNAL || span.is_none();
        let node_kind = match kind {
            SymbolKind::SK_CLASS
            | SymbolKind::SK_INTERFACE
            | SymbolKind::SK_ENUM
            | SymbolKind::SK_RECORD
            | SymbolKind::SK_ANNOTATION_TYPE
            | SymbolKind::SK_ANON_CLASS => NodeKind::Type,
            SymbolKind::SK_METHOD
            | SymbolKind::SK_CONSTRUCTOR
            | SymbolKind::SK_LAMBDA
            | SymbolKind::SK_STATIC_INIT
            | SymbolKind::SK_INSTANCE_INIT => NodeKind::Function,
            SymbolKind::SK_FIELD
            | SymbolKind::SK_ENUM_CONSTANT
            | SymbolKind::SK_PARAM
            | SymbolKind::SK_LOCAL_VAR
            | SymbolKind::SK_TYPE_PARAM => NodeKind::Variable,
            // Namespaces never form units (`locate_unit_symbol` stops below them).
            SymbolKind::SK_EXTERNAL | SymbolKind::SK_PACKAGE | SymbolKind::SK_MODULE => {
                NodeKind::External
            }
        };
        let is_abstract = matches!(
            kind,
            SymbolKind::SK_INTERFACE | SymbolKind::SK_ANNOTATION_TYPE
        ) || (is_named_type_kind(kind) && rec.modifiers & SymbolModifiers::ABSTRACT != 0);
        let binding_note = if rec.flags & SymbolFlags::UNRESOLVED != 0 {
            Some(UNRESOLVED_BINDING_NOTE)
        } else if rec.flags & SymbolFlags::INFERRED_PACKAGE != 0 {
            Some(INFERRED_PACKAGE_NOTE)
        } else {
            None
        };
        let name = self
            .symbol_name(sym)
            .unwrap_or_else(|| format!("#{}", sym));
        let (package, module) = self.namespace_of(sym, file);

        let unit = self.units.len() as u32;
        self.units.push(UnitDraft {
            kind: node_kind,
            key: format!("{}#{}", name, sym),
            name,
            symbol: Some(sym),
            symbol_kind: Some(kind),
            file,
            external,
            is_abstract,
            visibility: Some(SymbolVisibility::from(rec.visibility)),
            parent: None,
            package,
            module,
            binding_note,
            spans: span.into_iter().collect(),
        });
        // Registered before the parent is resolved, so a malformed parent cycle terminates.
        self.unit_by_symbol.insert(sym, unit);

        if (rec.parent_sym as usize) < self.sta.symbol_records.len() {
            if let Some(parent_sym) = self.locate_unit_symbol(rec.parent_sym) {
                if parent_sym != sym {
                    let parent = self.ensure_symbol_unit(parent_sym);
                    self.units[unit as usize].parent = Some(parent);
                }
            }
        }
        unit
    }

    fn file_scope_unit(&mut self, file: u32) -> u32 {
        if let Some(&unit) = self.unit_by_file_scope.get(&file) {
            return unit;
        }
        let path = self
            .files
            .path(file)
            .map(str::to_string)
            .unwrap_or_else(|| format!("file #{}", file));
        let package = Some(self.file_package_name(file).unwrap_or_default());
        let unit = self.units.len() as u32;
        self.units.push(UnitDraft {
            kind: NodeKind::FileScope,
            key: format!("file-scope:{}#{}", path, file),
            name: path,
            symbol: None,
            symbol_kind: None,
            file: Some(file),
            external: false,
            is_abstract: false,
            visibility: None,
            parent: None,
            package,
            module: None,
            binding_note: None,
            spans: self.files.file_span(file).into_iter().collect(),
        });
        self.unit_by_file_scope.insert(file, unit);
        unit
    }

    /// Unit that owns an IR function: the unit of its symbol, else of its enclosing type,
    /// else the file-scope code of its file.
    fn function_owner(&mut self, func: &FuncIr) -> u32 {
        if let Some(unit) = func.symbol.and_then(|s| self.unit_of_symbol(s)) {
            return unit;
        }
        if let Some(unit) = func.enclosing_type.and_then(|t| self.unit_of_symbol(t)) {
            return unit;
        }
        self.file_scope_unit(func.span.file_id)
    }

    /// Unit of a module variable: the unit of its symbol, else the file-scope code of its
    /// file.
    fn global_unit(&mut self, global: &GlobalIr) -> u32 {
        if let Some(unit) = global.symbol.and_then(|s| self.unit_of_symbol(s)) {
            return unit;
        }
        self.file_scope_unit(global.span.file_id)
    }

    fn member_role(&self, func: &FuncIr, owner: u32) -> MemberRole {
        match func.kind {
            FuncKind::Constructor | FuncKind::ImplicitConstructor => MemberRole::Constructor,
            FuncKind::StaticInit | FuncKind::InstanceInit | FuncKind::ModuleInit => {
                MemberRole::Initializer
            }
            FuncKind::Lambda | FuncKind::Adapter => MemberRole::Closure,
            FuncKind::Method if func.is_closure => MemberRole::Closure,
            FuncKind::Method if self.units[owner as usize].kind == NodeKind::Type => {
                MemberRole::Method
            }
            FuncKind::Method => MemberRole::Function,
        }
    }

    /// Nearest enclosing callable of `sym` that is not itself a closure.
    fn enclosing_callable(&self, sym: u32) -> Option<u32> {
        let records = &self.sta.symbol_records;
        let mut cur = records.get(sym as usize)?.parent_sym;
        let mut steps = 0usize;
        while (cur as usize) < records.len() && steps <= records.len() {
            match SymbolKind::from(records[cur as usize].kind) {
                SymbolKind::SK_METHOD
                | SymbolKind::SK_CONSTRUCTOR
                | SymbolKind::SK_STATIC_INIT
                | SymbolKind::SK_INSTANCE_INIT => return Some(cur),
                kind if is_namespace_kind(kind) || is_named_type_kind(kind) => return None,
                _ => {}
            }
            cur = records[cur as usize].parent_sym;
            steps += 1;
        }
        None
    }

    // ── Symbol helpers ───────────────────────────────────────────────────────

    fn kind_of(&self, sym: u32) -> Option<SymbolKind> {
        self.sta
            .symbol_records
            .get(sym as usize)
            .map(|r| SymbolKind::from(r.kind))
    }

    fn symbol_span(&self, sym: u32) -> Option<Span> {
        let rec = self.sta.symbol_records.get(sym as usize)?;
        self.files.token_span(rec.first_token_id, rec.last_token_id)
    }

    /// Qualified name as recorded (package display names first, then the canonical qualified
    /// name, then the simple name).
    fn symbol_name(&self, sym: u32) -> Option<String> {
        let rec = self.sta.symbol_records.get(sym as usize)?;
        if let Some(name) = self.sta.custom_package_names.get(&sym) {
            if !name.is_empty() {
                return Some(name.clone());
            }
        }
        if let Some(name) = self.sta.qualified_name_of(sym) {
            if !name.is_empty() {
                return Some(name.to_string());
            }
        }
        let text = self.tca.interner.lookup_text(rec.name_id);
        if text.is_empty() {
            None
        } else {
            Some(String::from_utf8_lossy(text).into_owned())
        }
    }

    fn file_package_name(&self, file: u32) -> Option<String> {
        let file = u16::try_from(file).ok()?;
        self.sta.file_package_names.get(&file).cloned()
    }

    /// Namespace name of the unit `sym` and the name of an enclosing module declaration.
    /// Without a package ancestor, the file's recorded package applies; without either, a
    /// project unit is in the global namespace and an external unit's namespace is not
    /// recorded.
    fn namespace_of(&self, sym: u32, file: Option<u32>) -> (Option<String>, Option<String>) {
        let records = &self.sta.symbol_records;
        let mut package = None;
        let mut module = None;
        let mut cur = records[sym as usize].parent_sym;
        let mut steps = 0usize;
        while (cur as usize) < records.len() && steps <= records.len() {
            match SymbolKind::from(records[cur as usize].kind) {
                SymbolKind::SK_PACKAGE if package.is_none() => package = Some(cur),
                SymbolKind::SK_MODULE if module.is_none() => module = self.symbol_name(cur),
                _ => {}
            }
            cur = records[cur as usize].parent_sym;
            steps += 1;
        }
        let name = match package {
            Some(p) => Some(self.symbol_name(p).unwrap_or_default()),
            None => match file {
                Some(f) => Some(self.file_package_name(f).unwrap_or_default()),
                None => None,
            },
        };
        (name, module)
    }

    /// `type_id` of a declaration when it denotes a type declaration (type parameters and
    /// primitive pseudo-ids are not dependencies).
    fn declared_type_symbol(&self, type_id: u32) -> Option<u32> {
        let kind = self.kind_of(type_id)?;
        (is_named_type_kind(kind)
            || kind == SymbolKind::SK_ANON_CLASS
            || kind == SymbolKind::SK_EXTERNAL)
            .then_some(type_id)
    }

    // ── Assembly ─────────────────────────────────────────────────────────────

    pub fn finish(mut self) -> ArchModel {
        for (from, to, origin, span) in std::mem::take(&mut self.pending_uses) {
            if !self.facts.contains_key(&(from, to, DepKind::FieldType)) {
                self.record(from, to, DepKind::TypeUse, origin, Strength::Exact, span, None);
            }
        }
        let call_graph = self.coverage.call_graph;
        for (site, from, span) in std::mem::take(&mut self.pending_indirect) {
            if !self.resolved_sites.contains(&site) {
                self.unresolved.push(UnresolvedFact {
                    from: Some(NodeId(from)),
                    kind: DepKind::Dispatch,
                    span,
                    name: None,
                    reason: if call_graph {
                        "the indirect call is unreachable from the call graph's entry points, so its targets were not computed"
                    } else {
                        "the target of an indirect call needs the points-to call graph, which was not supplied"
                    }
                    .to_string(),
                });
            }
        }

        // ── Type level: units ordered by name, then key ──
        let mut order: Vec<u32> = (0..self.units.len() as u32).collect();
        order.sort_by(|&a, &b| {
            let (ua, ub) = (&self.units[a as usize], &self.units[b as usize]);
            ua.name.cmp(&ub.name).then_with(|| ua.key.cmp(&ub.key))
        });
        let mut remap = vec![0u32; self.units.len()];
        for (new, &old) in order.iter().enumerate() {
            remap[old as usize] = new as u32;
        }
        let type_nodes: Vec<ArchNode> = order
            .iter()
            .enumerate()
            .map(|(new, &old)| {
                let u = &self.units[old as usize];
                ArchNode {
                    id: NodeId(new as u32),
                    kind: u.kind,
                    key: u.key.clone(),
                    name: u.name.clone(),
                    symbol: u.symbol,
                    symbol_kind: u.symbol_kind,
                    file: u.file,
                    external: u.external,
                    is_abstract: u.is_abstract,
                    visibility: u.visibility,
                    parent: u.parent.map(|p| NodeId(remap[p as usize])),
                    spans: u.spans.clone(),
                }
            })
            .collect();
        let types = self.lift_level(Level::Type, type_nodes, &remap, &HashMap::new(), None);

        let mut import_files: BTreeSet<u32> = BTreeSet::new();
        let mut namespace_imports: BTreeSet<String> = BTreeSet::new();
        let mut namespace_import_count = 0u32;
        for (file, key) in self.imports.keys() {
            import_files.insert(*file);
            if let ImportKey::Namespace(name) = key {
                namespace_imports.insert(name.clone());
                namespace_import_count += 1;
            }
        }

        // ── File level ──
        let mut file_groups = GroupTable::default();
        let mut unit_file = vec![0u32; self.units.len()];
        for &old in &order {
            let u = &self.units[old as usize];
            unit_file[old as usize] = match (u.external, u.file) {
                (false, Some(file)) => self.file_group(&mut file_groups, file),
                _ => namespace_group(&mut file_groups, u.package.as_deref(), true, None),
            };
        }
        for &file in &import_files {
            self.file_group(&mut file_groups, file);
        }
        let file_remap = file_groups.sort();
        for g in unit_file.iter_mut() {
            *g = file_remap[*g as usize];
        }
        let file_import_sources: HashMap<u32, Vec<(u32, Strength)>> = import_files
            .iter()
            .filter_map(|&f| {
                let g = *file_groups.by_key.get(&file_key(f))?;
                Some((f, vec![(g, Strength::Exact)]))
            })
            .collect();
        let files = self.lift_level(
            Level::File,
            file_groups.nodes.clone(),
            &unit_file,
            &file_import_sources,
            Some(&|key: &ImportKey| match key {
                ImportKey::Unit(u) => vec![(unit_file[*u as usize], Strength::Exact)],
                ImportKey::Namespace(_) => Vec::new(),
            }),
        );

        // ── Package level ──
        let mut package_groups = GroupTable::default();
        let mut unit_package = vec![0u32; self.units.len()];
        for &old in &order {
            let u = &self.units[old as usize];
            unit_package[old as usize] =
                namespace_group(&mut package_groups, u.package.as_deref(), u.external, None);
        }
        for name in &namespace_imports {
            namespace_group(&mut package_groups, Some(name), true, None);
        }
        // A file that holds no project unit receives its imports in its recorded package.
        let files_with_units: HashSet<u32> = self
            .units
            .iter()
            .filter(|u| !u.external)
            .filter_map(|u| u.file)
            .collect();
        for &file in &import_files {
            if !files_with_units.contains(&file) {
                let name = self.file_package_name(file).unwrap_or_default();
                namespace_group(&mut package_groups, Some(&name), false, None);
            }
        }
        // Package declarations contribute their spans and symbol.
        let sta = self.sta;
        for sym in 0..sta.symbol_records.len() as u32 {
            if SymbolKind::from(sta.symbol_records[sym as usize].kind) != SymbolKind::SK_PACKAGE {
                continue;
            }
            let Some(name) = self.symbol_name(sym) else {
                continue;
            };
            let Some(&g) = package_groups.by_key.get(&namespace_key(&name)) else {
                continue;
            };
            let span = self.symbol_span(sym);
            let node = &mut package_groups.nodes[g as usize];
            if node.symbol.is_none() {
                node.symbol = Some(sym);
                node.symbol_kind = Some(SymbolKind::SK_PACKAGE);
            }
            node.spans.extend(span);
        }
        // Namespace nesting along the package symbol chain, creating the enclosing namespaces
        // that declare no unit of their own. A namespace is external only when it encloses
        // external namespaces alone.
        let mut queue: Vec<u32> = (0..package_groups.nodes.len() as u32).collect();
        while let Some(g) = queue.pop() {
            let Some(sym) = package_groups.nodes[g as usize].symbol else {
                continue;
            };
            let Some(parent_sym) = self.parent_package(sym) else {
                continue;
            };
            let Some(parent_name) = self.symbol_name(parent_sym) else {
                continue;
            };
            let created = !package_groups
                .by_key
                .contains_key(&namespace_key(&parent_name));
            let external = package_groups.nodes[g as usize].external;
            let parent = namespace_group(
                &mut package_groups,
                Some(&parent_name),
                external,
                Some(parent_sym),
            );
            if created {
                let span = self.symbol_span(parent_sym);
                package_groups.nodes[parent as usize].spans.extend(span);
                queue.push(parent);
            }
            if parent != g {
                package_groups.nodes[g as usize].parent = Some(NodeId(parent));
            }
        }
        let package_remap = package_groups.sort();
        for g in unit_package.iter_mut() {
            *g = package_remap[*g as usize];
        }
        // Packages that receive a file's imports: the packages its project units declare (all
        // of them, as an over-approximation, when there are several), else the file's package.
        let mut file_unit_packages: HashMap<u32, BTreeSet<u32>> = HashMap::new();
        for &old in &order {
            let u = &self.units[old as usize];
            if let (false, Some(file)) = (u.external, u.file) {
                file_unit_packages
                    .entry(file)
                    .or_default()
                    .insert(unit_package[old as usize]);
            }
        }
        let package_import_sources: HashMap<u32, Vec<(u32, Strength)>> = import_files
            .iter()
            .map(|&f| {
                let sources = match file_unit_packages.get(&f) {
                    Some(set) if !set.is_empty() => {
                        let strength = if set.len() == 1 {
                            Strength::Exact
                        } else {
                            Strength::OverApproximate
                        };
                        set.iter().map(|&g| (g, strength)).collect()
                    }
                    _ => {
                        let name = self.file_package_name(f).unwrap_or_default();
                        package_groups
                            .by_key
                            .get(&namespace_key(&name))
                            .map(|&g| vec![(g, Strength::Exact)])
                            .unwrap_or_default()
                    }
                };
                (f, sources)
            })
            .collect();
        let packages = {
            let by_key = &package_groups.by_key;
            self.lift_level(
                Level::Package,
                package_groups.nodes.clone(),
                &unit_package,
                &package_import_sources,
                Some(&|key: &ImportKey| match key {
                    ImportKey::Unit(u) => vec![(unit_package[*u as usize], Strength::Exact)],
                    ImportKey::Namespace(name) => by_key
                        .get(&namespace_key(name))
                        .map(|&g| vec![(g, Strength::Exact)])
                        .unwrap_or_default(),
                }),
            )
        };

        // ── Project level ──
        let mut project_groups = GroupTable::default();
        let mut unit_project = vec![0u32; self.units.len()];
        for &old in &order {
            let u = &self.units[old as usize];
            unit_project[old as usize] = if u.external {
                namespace_group(&mut project_groups, u.package.as_deref(), true, None)
            } else {
                let name = self.project_name(u);
                project_group(&mut project_groups, &name)
            };
        }
        let mut file_project_group: HashMap<u32, u32> = HashMap::new();
        for &file in &import_files {
            let name = self
                .file_projects
                .get(&file)
                .cloned()
                .unwrap_or_else(|| self.config.default_project.clone());
            let g = project_group(&mut project_groups, &name);
            file_project_group.insert(file, g);
        }
        for name in &namespace_imports {
            namespace_group(&mut project_groups, Some(name), true, None);
        }
        let project_remap = project_groups.sort();
        for g in unit_project.iter_mut() {
            *g = project_remap[*g as usize];
        }
        for g in file_project_group.values_mut() {
            *g = project_remap[*g as usize];
        }
        // Projects that declare units of each namespace, for namespace imports.
        let mut namespace_projects: HashMap<String, BTreeSet<u32>> = HashMap::new();
        for &old in &order {
            let u = &self.units[old as usize];
            if let (false, Some(package)) = (u.external, &u.package) {
                namespace_projects
                    .entry(package.clone())
                    .or_default()
                    .insert(unit_project[old as usize]);
            }
        }
        let project_import_sources: HashMap<u32, Vec<(u32, Strength)>> = file_project_group
            .iter()
            .map(|(&f, &g)| (f, vec![(g, Strength::Exact)]))
            .collect();
        let projects = {
            let by_key = &project_groups.by_key;
            self.lift_level(
                Level::Project,
                project_groups.nodes.clone(),
                &unit_project,
                &project_import_sources,
                Some(&|key: &ImportKey| match key {
                    ImportKey::Unit(u) => vec![(unit_project[*u as usize], Strength::Exact)],
                    ImportKey::Namespace(name) => match namespace_projects.get(name) {
                        Some(set) if !set.is_empty() => {
                            let strength = if set.len() == 1 {
                                Strength::Exact
                            } else {
                                Strength::OverApproximate
                            };
                            set.iter().map(|&g| (g, strength)).collect()
                        }
                        _ => by_key
                            .get(&namespace_key(name))
                            .map(|&g| vec![(g, Strength::Exact)])
                            .unwrap_or_default(),
                    },
                }),
            )
        };

        // ── Containment maps by final type-level id ──
        let mut type_file = vec![NodeId(0); order.len()];
        let mut type_package = vec![NodeId(0); order.len()];
        let mut type_project = vec![NodeId(0); order.len()];
        for &old in &order {
            let new = remap[old as usize] as usize;
            type_file[new] = NodeId(unit_file[old as usize]);
            type_package[new] = NodeId(unit_package[old as usize]);
            type_project[new] = NodeId(unit_project[old as usize]);
        }
        let mut file_project: Vec<Option<NodeId>> = vec![None; files.node_count()];
        for &old in &order {
            let f = unit_file[old as usize] as usize;
            if file_project[f].is_none() {
                file_project[f] = Some(NodeId(unit_project[old as usize]));
            }
        }
        for (file, &g) in &file_project_group {
            if let Some(&fg) = file_groups.by_key.get(&file_key(*file)) {
                let slot = &mut file_project[fg as usize];
                if slot.is_none() {
                    *slot = Some(NodeId(g));
                }
            }
        }

        let mut functions = std::mem::take(&mut self.functions);
        for f in &mut functions {
            f.owner = NodeId(remap[f.owner.index()]);
        }
        functions.sort_by_key(|f| f.func);
        let mut fields = std::mem::take(&mut self.fields);
        for f in &mut fields {
            f.owner = NodeId(remap[f.owner.index()]);
        }
        fields.sort_by_key(|f| f.field);
        let mut unresolved = std::mem::take(&mut self.unresolved);
        for u in &mut unresolved {
            u.from = u.from.map(|n| NodeId(remap[n.index()]));
        }
        let mut callbacks = std::mem::take(&mut self.callbacks);
        callbacks.sort_by_key(|c| (c.func, c.span.map(|s| span_key(&s))));

        ArchModel {
            types,
            files,
            packages,
            projects,
            type_file,
            type_package,
            type_project,
            file_project,
            functions,
            fields,
            calls: std::mem::take(&mut self.calls).into_values().collect(),
            reachable: self.reachable.take(),
            entries: self.entries.take(),
            callbacks,
            unresolved,
            namespace_imports_without_file_target: namespace_import_count,
            coverage: self.coverage,
        }
    }

    /// Project of a project unit: explicit file assignment, else an enclosing module
    /// declaration, else the default project.
    fn project_name(&self, unit: &UnitDraft) -> String {
        unit.file
            .and_then(|f| self.file_projects.get(&f).cloned())
            .or_else(|| unit.module.clone())
            .unwrap_or_else(|| self.config.default_project.clone())
    }

    fn parent_package(&self, sym: u32) -> Option<u32> {
        let records = &self.sta.symbol_records;
        let mut cur = records.get(sym as usize)?.parent_sym;
        let mut steps = 0usize;
        while (cur as usize) < records.len() && steps <= records.len() {
            if SymbolKind::from(records[cur as usize].kind) == SymbolKind::SK_PACKAGE {
                return Some(cur);
            }
            cur = records[cur as usize].parent_sym;
            steps += 1;
        }
        None
    }

    fn file_group(&self, groups: &mut GroupTable, file: u32) -> u32 {
        let path = self
            .files
            .path(file)
            .map(str::to_string)
            .unwrap_or_else(|| format!("file #{}", file));
        let span = self.files.file_span(file);
        groups.intern(file_key(file), |id| ArchNode {
            id,
            kind: NodeKind::File,
            key: file_key(file),
            name: path,
            symbol: None,
            symbol_kind: None,
            file: Some(file),
            external: false,
            is_abstract: false,
            visibility: None,
            parent: None,
            spans: span.into_iter().collect(),
        })
    }

    /// Lifts every fact to the groups of `unit_group` (indexed by draft id), together with
    /// the import facts when `import_target` is given.
    fn lift_level(
        &self,
        level: Level,
        nodes: Vec<ArchNode>,
        unit_group: &[u32],
        import_sources: &HashMap<u32, Vec<(u32, Strength)>>,
        import_target: Option<&dyn Fn(&ImportKey) -> Vec<(u32, Strength)>>,
    ) -> DependencyGraph {
        let mut lifted: BTreeMap<(u32, u32, DepKind), EvidenceAcc> = BTreeMap::new();
        for (&(from, to, kind), acc) in &self.facts {
            let gf = unit_group[from as usize];
            let gt = unit_group[to as usize];
            lifted
                .entry((gf, gt, kind))
                .or_default()
                .merge(acc, Strength::Exact);
        }
        if let Some(target) = import_target {
            for ((file, key), acc) in &self.imports {
                let Some(sources) = import_sources.get(file) else {
                    continue;
                };
                for &(gt, target_cap) in &target(key) {
                    for &(gf, source_cap) in sources {
                        lifted
                            .entry((gf, gt, DepKind::Import))
                            .or_default()
                            .merge(acc, target_cap.max(source_cap));
                    }
                }
            }
        }

        let mut internal = vec![0u32; nodes.len()];
        let mut edges = Vec::new();
        for ((gf, gt, kind), acc) in lifted {
            if gf == gt {
                internal[gf as usize] =
                    internal[gf as usize].saturating_add(acc.sites.len() as u32);
            } else {
                edges.push(acc.into_edge(NodeId(gf), NodeId(gt), kind));
            }
        }
        DependencyGraph::assemble(level, nodes, edges, internal)
    }
}

/// One-call construction from the available artifacts: declarations and imports of the
/// symbol table, then the IR and its call graph when given, else the name occurrences for
/// body facts.
pub fn build_model(
    sta: &SymbolTableArtifact,
    tca: &TokenCorpusArtifact,
    module: Option<&ModuleIr>,
    call_graph: Option<&CallGraph>,
    config: GraphConfig,
) -> ArchModel {
    let mut builder = ArchGraphBuilder::new(sta, tca, config);
    builder.add_declarations();
    builder.add_symbol_imports();
    match module {
        Some(module) => {
            builder.add_ir(module);
            if let Some(cg) = call_graph {
                builder.add_call_graph(module, cg);
            }
        }
        None => builder.add_occurrences(),
    }
    builder.finish()
}

// ── Group tables ─────────────────────────────────────────────────────────────

#[derive(Debug, Default)]
struct GroupTable {
    nodes: Vec<ArchNode>,
    by_key: HashMap<String, u32>,
}

impl GroupTable {
    fn intern(&mut self, key: String, make: impl FnOnce(NodeId) -> ArchNode) -> u32 {
        if let Some(&id) = self.by_key.get(&key) {
            return id;
        }
        let id = self.nodes.len() as u32;
        self.nodes.push(make(NodeId(id)));
        self.by_key.insert(key, id);
        id
    }

    /// Orders groups by (external, name, key), renumbers them and returns old → new ids.
    fn sort(&mut self) -> Vec<u32> {
        let mut order: Vec<u32> = (0..self.nodes.len() as u32).collect();
        order.sort_by(|&a, &b| {
            let (na, nb) = (&self.nodes[a as usize], &self.nodes[b as usize]);
            na.external
                .cmp(&nb.external)
                .then_with(|| na.name.cmp(&nb.name))
                .then_with(|| na.key.cmp(&nb.key))
        });
        let mut remap = vec![0u32; order.len()];
        for (new, &old) in order.iter().enumerate() {
            remap[old as usize] = new as u32;
        }
        let mut nodes: Vec<ArchNode> = order
            .iter()
            .map(|&old| self.nodes[old as usize].clone())
            .collect();
        for (new, node) in nodes.iter_mut().enumerate() {
            node.id = NodeId(new as u32);
            node.parent = node.parent.map(|p| NodeId(remap[p.index()]));
        }
        for id in self.by_key.values_mut() {
            *id = remap[*id as usize];
        }
        self.nodes = nodes;
        remap
    }
}

fn file_key(file: u32) -> String {
    format!("file:{}", file)
}

fn namespace_key(name: &str) -> String {
    format!("ns:{}", name)
}

fn project_key(name: &str) -> String {
    format!("project:{}", name)
}

/// Key of the group that collects external declarations whose namespace the symbol table
/// does not record (kept apart from the project's global namespace).
pub const UNRECORDED_NAMESPACE_KEY: &str = "ns-unrecorded";

/// Namespace group `name` (`None`: the unrecorded-namespace group); a group stays external
/// only while every member is external.
fn namespace_group(
    groups: &mut GroupTable,
    name: Option<&str>,
    external: bool,
    symbol: Option<u32>,
) -> u32 {
    let key = name.map_or_else(|| UNRECORDED_NAMESPACE_KEY.to_string(), namespace_key);
    let display = name.unwrap_or_default().to_string();
    let id = groups.intern(key.clone(), |id| ArchNode {
        id,
        kind: NodeKind::Package,
        key,
        name: display,
        symbol,
        symbol_kind: symbol.map(|_| SymbolKind::SK_PACKAGE),
        file: None,
        external: external || name.is_none(),
        is_abstract: false,
        visibility: None,
        parent: None,
        spans: Vec::new(),
    });
    if !external && name.is_some() {
        groups.nodes[id as usize].external = false;
    }
    id
}

fn project_group(groups: &mut GroupTable, name: &str) -> u32 {
    groups.intern(project_key(name), |id| ArchNode {
        id,
        kind: NodeKind::Project,
        key: project_key(name),
        name: name.to_string(),
        symbol: None,
        symbol_kind: None,
        file: None,
        external: false,
        is_abstract: false,
        visibility: None,
        parent: None,
        spans: Vec::new(),
    })
}

// ── Shared helpers ───────────────────────────────────────────────────────────

const UNRESOLVED_BINDING_NOTE: &str =
    "the referenced type is external and no import, package or catalog entry binds it";
const INFERRED_PACKAGE_NOTE: &str =
    "the referenced type's package is inferred from the only applicable on-demand import";

/// Sort and identity key of a span.
#[inline]
pub fn span_key(span: &Span) -> (u32, u32, u32) {
    (span.file_id, span.first_token, span.last_token)
}

/// `span` unless it is the "no source counterpart" sentinel.
#[inline]
pub fn real_span(span: Span) -> Option<Span> {
    (!span.is_none() && span.file_id != u32::MAX).then_some(span)
}

fn precision_strength(precision: EdgePrecision) -> Strength {
    match precision {
        EdgePrecision::Exact => Strength::Exact,
        EdgePrecision::OverApproximate(_) => Strength::OverApproximate,
    }
}

/// Strength of a Phase 3 binding and the assumption behind a partial one.
fn binding_strength(status: Option<ResolutionStatus>) -> (Strength, Option<&'static str>) {
    match status {
        Some(ResolutionStatus::InferredPackage) => (Strength::Partial, Some(INFERRED_PACKAGE_NOTE)),
        _ => (Strength::Exact, None),
    }
}

fn occurrence_role(role: u8) -> Option<OccurrenceRole> {
    Some(match role {
        0 => OccurrenceRole::Definition,
        1 => OccurrenceRole::TypeReference,
        2 => OccurrenceRole::Read,
        3 => OccurrenceRole::Write,
        4 => OccurrenceRole::Call,
        5 => OccurrenceRole::Import,
        6 => OccurrenceRole::Instantiation,
        _ => return None,
    })
}

#[inline]
fn is_named_type_kind(kind: SymbolKind) -> bool {
    matches!(
        kind,
        SymbolKind::SK_CLASS
            | SymbolKind::SK_INTERFACE
            | SymbolKind::SK_ENUM
            | SymbolKind::SK_RECORD
            | SymbolKind::SK_ANNOTATION_TYPE
    )
}

#[inline]
fn is_namespace_kind(kind: SymbolKind) -> bool {
    matches!(kind, SymbolKind::SK_PACKAGE | SymbolKind::SK_MODULE)
}

/// Named symbols in the type tree of `root`, each flagged when it is the head (the type
/// itself, through pointers, references and generic bases), and whether part of the tree is
/// unknown. Type parameters and primitive types are not dependencies.
pub fn named_in_type(types: &TypeTable, root: TypeId) -> (Vec<(u32, bool)>, bool) {
    let mut named = Vec::new();
    let mut unknown = false;
    let mut seen: HashSet<(TypeId, bool)> = HashSet::new();
    let mut stack = vec![(root, true)];
    while let Some((id, head)) = stack.pop() {
        if !seen.insert((id, head)) {
            continue;
        }
        if id.index() >= types.len() {
            unknown = true;
            continue;
        }
        match types.get(id) {
            IrType::Named(sym) => named.push((*sym, head)),
            IrType::Generic { base, args } => {
                stack.push((*base, head));
                stack.extend(args.iter().map(|&a| (a, false)));
            }
            IrType::Ptr(target) => stack.push((*target, head)),
            IrType::Ref { target, .. } => stack.push((*target, head)),
            IrType::Array { elem, .. } => stack.push((*elem, false)),
            IrType::Func { params, ret } => {
                stack.extend(params.iter().map(|&p| (p, false)));
                stack.push((*ret, false));
            }
            IrType::Unknown => unknown = true,
            IrType::Void
            | IrType::Bool
            | IrType::Int { .. }
            | IrType::Float { .. }
            | IrType::Char
            | IrType::Str
            | IrType::TypeParam(_) => {}
        }
    }
    named.sort_unstable();
    named.dedup();
    (named, unknown)
}
