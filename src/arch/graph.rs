//! Architecture dependency graph: typed, weighted dependencies with source provenance at
//! type, file, package/namespace and project level.
//!
//! **Units.** The finest level has one node per *architectural unit*: a named type (nested
//! named types are units of their own, linked to their outer type by `parent`), a member
//! declared outside every type (free function, global variable), the file-scope code of a
//! compilation unit, or an external declaration the project references. Members, closures and
//! anonymous classes fold into the nearest enclosing named unit: nothing outside can name
//! them, so they never form a separate dependency target.
//!
//! **Facts.** An edge `from → to` of kind `k` is the union of its *evidence*: dependency sites
//! (a declaration, an instruction, a call site, an allocation site, an import, an annotation
//! use), each with its source span and strength. The weight of an edge is its number of
//! distinct sites; lifting to a coarser level unions evidence by site, so one site reaching
//! two types of the same package counts once. Fact sources:
//!
//! - symbol table: declared supertypes, declared field / parameter / local / return type
//!   heads, field associations;
//! - IR: allocations, static callees and the declared method of virtual and interface calls,
//!   instance and static field accesses, casts and type tests, closures and function
//!   references, and the complete field and return type trees (generic arguments count);
//! - points-to call graph: runtime dispatch targets of virtual, interface and indirect calls;
//! - Phase 6 symbol-level call graph, for pipelines that do not lower to the IR yet;
//! - frontend facts the symbol table does not persist: imports, annotation uses and the
//!   project membership of files.
//!
//! **Externals.** A unit is external when its symbol is `SK_EXTERNAL` or its declaration has
//! no source location in the analysed corpus. At every level coarser than type, external
//! units are aggregated to their namespace.
//!
//! **What is not derivable** is recorded, never filled in: unbound callees, indirect calls the
//! call graph leaves unresolved and types the IR could not determine become
//! [`UnresolvedFact`]s, and [`Coverage`] states which fact sources were supplied, so that a
//! consumer can tell "no edge of this kind exists" from "this kind was not analysed".

use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::core::types::cg::{
    CallGraphArtifact, CallSite, CG_EDGE_CONSTRUCTOR, CG_EDGE_DIRECT, CG_EDGE_INTERFACE,
    CG_EDGE_SPECIAL, CG_EDGE_VIRTUAL,
};
use crate::core::types::symbol::{SymbolKind, SymbolModifiers, SymbolVisibility, THRelation};
use crate::ingestion::TokenCorpusArtifact;
use crate::ir::{
    CallSiteId, CallSiteIr, Callee, FieldId, FieldIr, FuncId, FuncIr, Inst, IrType, ModuleIr,
    Place, Span, TypeId, TypeTable,
};
use crate::memory::callgraph::CallGraph;
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
    /// A field of the unit mentions the target in its declared type.
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
    /// The unit reads, writes or releases a field declared by the target.
    FieldAccess = 10,
    /// The unit casts to, tests against or declares a local of a type mentioning the target.
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

/// How a result relates to the program, matching the derivation tags of the diagram model.
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

/// The program site that establishes one piece of evidence. Two facts with the same origin
/// are the same evidence, whichever source reported them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Origin {
    /// A declaration whose declared type or annotation creates the dependency.
    Symbol(u32),
    /// A declared supertype relation of the symbol table.
    Hierarchy { sub: u32, sup: u32 },
    /// One IR instruction.
    Instruction { func: u32, block: u32, index: u32 },
    /// An IR call site.
    CallSite(u32),
    /// An IR allocation site.
    AllocSite(u32),
    /// An IR field that has no matching declaration in the symbol table.
    Field(u32),
    /// An IR function that has no symbol.
    Function(u32),
    /// A Phase 6 call site (symbol-level call graph).
    SymbolCallSite(u32),
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

    /// True when `id` takes part in analyses that include or exclude external nodes.
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
    /// Type-level unit that owns the function.
    pub owner: NodeId,
    pub role: MemberRole,
    /// For closures: the named callable whose body creates the closure, when the symbol
    /// table records the nesting.
    pub enclosing_member: Option<u32>,
    pub span: Span,
    /// Fields accessed through an object (`Place::Field`), sorted and distinct.
    pub instance_fields: Vec<FieldId>,
    /// Static fields accessed (`Place::Static`): owner symbol and field, sorted and distinct.
    pub static_fields: Vec<(u32, FieldId)>,
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
    /// Matching declaration in the symbol table (same owner and name).
    pub symbol: Option<u32>,
    /// From the declaration's modifiers; `None` when no declaration matches.
    pub is_static: Option<bool>,
}

/// One resolved call edge at function granularity.
#[derive(Debug, Clone, Copy)]
pub struct CallFact {
    pub site: CallSiteId,
    pub caller: FuncId,
    pub callee: FuncId,
    pub span: Option<Span>,
    /// `Exact` for static calls and for sites with a single points-to target.
    pub strength: Strength,
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
    pub ir: bool,
    pub call_graph: bool,
    pub symbol_calls: bool,
    pub imports: bool,
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
        match kind {
            DepKind::Import if self.imports => Certainty::Exact,
            DepKind::Import => missing("no import facts were supplied by the language frontend"),
            DepKind::AnnotationUse if self.annotations => Certainty::Exact,
            DepKind::AnnotationUse => {
                missing("no annotation-use facts were supplied by the language frontend")
            }
            DepKind::Inheritance | DepKind::Realization | DepKind::ParameterType
                if self.declarations =>
            {
                Certainty::Exact
            }
            DepKind::Inheritance | DepKind::Realization | DepKind::ParameterType => {
                missing("the symbol-table declarations were not added")
            }
            DepKind::FieldType | DepKind::ReturnType if self.ir => Certainty::Exact,
            DepKind::FieldType | DepKind::ReturnType if self.declarations => Certainty::Partial {
                reason: "only declared type heads are known; generic arguments require the IR"
                    .to_string(),
            },
            DepKind::FieldType | DepKind::ReturnType => {
                missing("neither declarations nor the IR were added")
            }
            DepKind::Call | DepKind::Instantiation if self.ir => Certainty::Exact,
            DepKind::Call | DepKind::Instantiation if self.symbol_calls => Certainty::Partial {
                reason: "calls come from the Phase 6 symbol-level call graph".to_string(),
            },
            DepKind::Call | DepKind::Instantiation => missing("the IR was not added"),
            DepKind::Dispatch if self.call_graph => Certainty::OverApproximate,
            DepKind::Dispatch if self.symbol_calls => Certainty::Partial {
                reason: "dispatch targets come from the Phase 6 symbol-level call graph"
                    .to_string(),
            },
            DepKind::Dispatch => missing("the points-to call graph was not added"),
            DepKind::FieldAccess | DepKind::Reference if self.ir => Certainty::Exact,
            DepKind::FieldAccess | DepKind::Reference => missing("the IR was not added"),
            DepKind::TypeUse if self.ir => Certainty::Exact,
            DepKind::TypeUse if self.declarations => Certainty::Partial {
                reason: "only declared local types are known; casts and type tests require the IR"
                    .to_string(),
            },
            DepKind::TypeUse => missing("neither declarations nor the IR were added"),
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
    pub functions: Vec<FunctionFacts>,
    pub fields: Vec<FieldFacts>,
    /// Function-level call edges, sorted by `(site, callee)`.
    pub calls: Vec<CallFact>,
    /// Functions the points-to call graph reaches, sorted; `None` without a call graph.
    pub reachable: Option<Vec<FuncId>>,
    pub unresolved: Vec<UnresolvedFact>,
    /// Import facts that have no target at file level (namespace imports).
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

    /// Index of each function's facts by `FuncId`.
    pub fn function_index(&self) -> HashMap<u32, usize> {
        self.functions
            .iter()
            .enumerate()
            .map(|(i, f)| (f.func.0, i))
            .collect()
    }
}

// ── Inputs supplied by the language frontends ────────────────────────────────

/// Target of an import, include or namespace-opening directive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportTarget {
    /// The directive names one declaration.
    Symbol(u32),
    /// The directive opens a whole namespace (on-demand import, `using namespace`).
    Namespace(String),
    /// The directive names something no declaration or namespace binds.
    Unresolved(String),
}

#[derive(Debug, Clone)]
pub struct ImportFact {
    pub file_id: u32,
    pub target: ImportTarget,
    pub span: Option<Span>,
}

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
            .map(|&(first, end, file_id)| Span {
                file_id,
                first_token: first,
                last_token: end.saturating_sub(1).max(first),
            })
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
    /// Namespace name ("" is the global namespace).
    package: String,
    package_symbol: Option<u32>,
    /// Name of an enclosing module declaration.
    module: Option<String>,
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

/// Lookup tables over one IR module (ids are not assumed to equal vector indices).
struct IrIndex {
    funcs: HashMap<u32, usize>,
    fields: HashMap<u32, usize>,
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
            unresolved: Vec::new(),
            coverage: Coverage::default(),
        }
    }

    pub fn file_index(&self) -> &FileIndex {
        &self.files
    }

    // ── Symbol table ─────────────────────────────────────────────────────────

    /// Adds every project declaration as a unit, the declared supertypes, the declared
    /// field / parameter / local / return type heads and the field associations.
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
            match edge.relation {
                THRelation::TH_EXTENDS => {
                    self.record(from, to, DepKind::Inheritance, origin, Strength::Exact, span, None)
                }
                THRelation::TH_IMPLEMENTS => {
                    self.record(from, to, DepKind::Realization, origin, Strength::Exact, span, None)
                }
                THRelation::TH_CREATES => self.record(
                    from,
                    to,
                    DepKind::Instantiation,
                    origin,
                    Strength::Exact,
                    span,
                    None,
                ),
                // Phase 3 derives `uses` from field types; it is kept only where no field-type
                // fact explains it (decided in `finish`, once every source is in).
                THRelation::TH_USES => self.pending_uses.push((from, to, origin, span)),
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
            let Some(type_sym) = self.declared_type_symbol(rec.type_id) else {
                continue;
            };
            let (Some(from), Some(to)) = (self.unit_of_symbol(sym), self.unit_of_symbol(type_sym))
            else {
                continue;
            };
            let span = self.symbol_span(sym);
            self.record(from, to, dep, Origin::Symbol(sym), Strength::Exact, span, None);
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

    // ── IR ───────────────────────────────────────────────────────────────────

    /// Adds the field types and the body facts of every function of `module`.
    pub fn add_ir(&mut self, module: &ModuleIr) {
        self.coverage.ir = true;
        let index = IrIndex::new(module);

        for field in &module.fields {
            let Some(owner) = self.unit_of_symbol(field.owner_symbol) else {
                self.unresolved.push(UnresolvedFact {
                    from: None,
                    kind: DepKind::FieldType,
                    span: None,
                    name: Some(field.name),
                    reason: "the owner symbol of the IR field is not in the symbol table"
                        .to_string(),
                });
                continue;
            };
            let symbol = self.member_symbol(field.owner_symbol, field.name, |k| {
                matches!(k, SymbolKind::SK_FIELD | SymbolKind::SK_ENUM_CONSTANT)
            });
            let origin = symbol.map_or(Origin::Field(field.id.0), Origin::Symbol);
            let span = symbol.and_then(|s| self.symbol_span(s));
            self.record_type_tree(&module.types, owner, field.ty, DepKind::FieldType, origin, span);
            let is_static = symbol.map(|s| {
                self.sta.symbol_records[s as usize].modifiers & SymbolModifiers::STATIC != 0
            });
            self.fields.push(FieldFacts {
                field: field.id,
                owner: NodeId(owner),
                owner_symbol: field.owner_symbol,
                symbol,
                is_static,
            });
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
        let func_origin = func.symbol.map_or(Origin::Function(func.id.0), Origin::Symbol);
        self.record_type_tree(
            &module.types,
            owner,
            func.ret,
            DepKind::ReturnType,
            func_origin,
            Some(func.span),
        );

        let mut facts = FunctionFacts {
            func: func.id,
            symbol: func.symbol,
            owner: NodeId(owner),
            role,
            enclosing_member,
            span: func.span,
            instance_fields: Vec::new(),
            static_fields: Vec::new(),
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
                let span = block.inst_spans.get(i).copied();
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

        facts.instance_fields.sort_unstable_by_key(|f| f.0);
        facts.instance_fields.dedup_by_key(|f| f.0);
        facts.static_fields.sort_unstable_by_key(|&(o, f)| (o, f.0));
        facts.static_fields.dedup_by_key(|&mut (o, f)| (o, f.0));
        facts.static_callees.sort_unstable_by_key(|f| f.0);
        facts.static_callees.dedup_by_key(|f| f.0);
        facts.symbol_callees.sort_unstable();
        facts.symbol_callees.dedup();
        facts.closures.sort_unstable_by_key(|f| f.0);
        facts.closures.dedup_by_key(|f| f.0);
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
            | Callee::Interface { method: sym, .. } => match self.unit_of_symbol(*sym) {
                Some(to) => {
                    self.record(owner, to, DepKind::Call, origin, Strength::Exact, span, None);
                    facts.symbol_callees.push(*sym);
                }
                None => {
                    facts.unresolved_calls += 1;
                    self.unresolved.push(UnresolvedFact {
                        from: Some(NodeId(owner)),
                        kind: DepKind::Call,
                        span,
                        name: None,
                        reason: "the callee symbol is not in the symbol table".to_string(),
                    });
                }
            },
            Callee::Indirect(_) => self.pending_indirect.push((site.0, owner, span)),
            Callee::Unresolved(name) => {
                facts.unresolved_calls += 1;
                self.unresolved.push(UnresolvedFact {
                    from: Some(NodeId(owner)),
                    kind: DepKind::Call,
                    span,
                    name: Some(*name),
                    reason: "the callee name is not bound to any declaration".to_string(),
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
            Place::Field { field, .. } => match index.field(module, *field) {
                Some(field_ir) => {
                    if let Some(to) = self.unit_of_symbol(field_ir.owner_symbol) {
                        self.record(owner, to, DepKind::FieldAccess, at, Strength::Exact, span, None);
                    }
                    facts.instance_fields.push(*field);
                }
                None => self.unresolved.push(UnresolvedFact {
                    from: Some(NodeId(owner)),
                    kind: DepKind::FieldAccess,
                    span,
                    name: None,
                    reason: "the accessed field is not part of the IR module".to_string(),
                }),
            },
            Place::Static {
                owner: owner_sym,
                field,
                ..
            } => {
                match self.unit_of_symbol(*owner_sym) {
                    Some(to) => {
                        self.record(owner, to, DepKind::FieldAccess, at, Strength::Exact, span, None)
                    }
                    None => self.unresolved.push(UnresolvedFact {
                        from: Some(NodeId(owner)),
                        kind: DepKind::FieldAccess,
                        span,
                        name: None,
                        reason: "the owner of the static field is not in the symbol table"
                            .to_string(),
                    }),
                }
                facts.static_fields.push((*owner_sym, *field));
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
            match self.unit_of_symbol(sym) {
                Some(to) => self.record(from, to, dep, origin, Strength::Exact, span, None),
                None => self.unresolved.push(UnresolvedFact {
                    from: Some(NodeId(from)),
                    kind: dep,
                    span,
                    name: None,
                    reason: "the IR names a type symbol that is not in the symbol table".to_string(),
                }),
            }
        }
        if unknown {
            self.unresolved.push(UnresolvedFact {
                from: Some(NodeId(from)),
                kind,
                span,
                name: None,
                reason: "the IR could not determine (part of) the type".to_string(),
            });
        }
    }

    // ── Call graphs ──────────────────────────────────────────────────────────

    /// Adds the runtime dispatch targets of the points-to call graph and its reachable set.
    /// Sites with a single target are exact; sites with several are over-approximations.
    pub fn add_call_graph(&mut self, module: &ModuleIr, call_graph: &CallGraph) {
        self.coverage.call_graph = true;
        let index = IrIndex::new(module);

        let mut static_target: HashMap<u32, u32> = HashMap::new();
        for func in &module.funcs {
            for block in &func.blocks {
                for inst in &block.insts {
                    if let Inst::Call {
                        site,
                        callee: Callee::Static(target),
                        ..
                    } = inst
                    {
                        static_target.insert(site.0, target.0);
                    }
                }
            }
        }

        let mut targets_per_site: HashMap<u32, u32> = HashMap::new();
        for (site, _) in &call_graph.edges {
            *targets_per_site.entry(site.0).or_insert(0) += 1;
        }

        for &(site, callee) in &call_graph.edges {
            self.resolved_sites.insert(site.0);
            let Some(site_ir) = index.call_site(module, site) else {
                self.unresolved.push(UnresolvedFact {
                    from: None,
                    kind: DepKind::Dispatch,
                    span: None,
                    name: None,
                    reason: "the call graph names a call site that is not part of the IR module"
                        .to_string(),
                });
                continue;
            };
            let (Some(caller), Some(target)) =
                (index.func(module, site_ir.func), index.func(module, callee))
            else {
                self.unresolved.push(UnresolvedFact {
                    from: None,
                    kind: DepKind::Dispatch,
                    span: Some(site_ir.span),
                    name: None,
                    reason: "the call graph names a function that is not part of the IR module"
                        .to_string(),
                });
                continue;
            };
            let span = Some(site_ir.span);
            let strength = if targets_per_site.get(&site.0).copied() == Some(1) {
                Strength::Exact
            } else {
                Strength::OverApproximate
            };
            let entry = self.calls.entry((site.0, callee.0)).or_insert(CallFact {
                site,
                caller: caller.id,
                callee,
                span,
                strength,
            });
            if strength < entry.strength {
                entry.strength = strength;
            }
            if static_target.get(&site.0) == Some(&callee.0) {
                // The static call edge already records this dependency exactly.
                continue;
            }
            let from = self.function_owner(caller);
            let to = self.function_owner(target);
            self.record(from, to, DepKind::Dispatch, Origin::CallSite(site.0), strength, span, None);
        }

        let mut reachable = call_graph.reachable.clone();
        reachable.sort_unstable_by_key(|f| f.0);
        reachable.dedup_by_key(|f| f.0);
        self.reachable = Some(reachable);
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
            let (Some(from), Some(to)) = (self.unit_of_symbol(caller), self.unit_of_symbol(callee))
            else {
                continue;
            };
            let site = sites.get(&site_id).copied();
            let span = site.and_then(|s| self.token_span(s.call_token));
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
                    Some("the Phase 6 call site record is missing; the edge kind is unknown"),
                ),
            };
            self.record(from, to, kind, Origin::SymbolCallSite(site_id), strength, span, reason);
        }
    }

    // ── Frontend facts ───────────────────────────────────────────────────────

    /// Adds the import, include and namespace-opening directives of every file. Calling it
    /// states that the frontend supplied all of them.
    pub fn add_imports<I: IntoIterator<Item = ImportFact>>(&mut self, imports: I) {
        self.coverage.imports = true;
        for fact in imports {
            let origin = Origin::Import {
                file: fact.file_id,
                token: fact.span.map_or(u32::MAX, |s| s.first_token),
            };
            let key = match fact.target {
                ImportTarget::Symbol(sym) => match self.unit_of_symbol(sym) {
                    Some(unit) => ImportKey::Unit(unit),
                    None => {
                        self.unresolved.push(UnresolvedFact {
                            from: None,
                            kind: DepKind::Import,
                            span: fact.span,
                            name: None,
                            reason: "the imported symbol is not in the symbol table".to_string(),
                        });
                        continue;
                    }
                },
                ImportTarget::Namespace(name) => ImportKey::Namespace(name),
                ImportTarget::Unresolved(text) => {
                    self.unresolved.push(UnresolvedFact {
                        from: None,
                        kind: DepKind::Import,
                        span: fact.span,
                        name: None,
                        reason: format!("the import `{}` binds no declaration or namespace", text),
                    });
                    continue;
                }
            };
            self.imports
                .entry((fact.file_id, key))
                .or_default()
                .add(origin, Strength::Exact, fact.span, None);
        }
    }

    /// Adds annotation, attribute and decorator uses. Calling it states that the frontend
    /// supplied all of them.
    pub fn add_annotation_uses<I: IntoIterator<Item = AnnotationFact>>(&mut self, uses: I) {
        self.coverage.annotations = true;
        for fact in uses {
            let (Some(from), Some(to)) = (
                self.unit_of_symbol(fact.annotated_symbol),
                self.unit_of_symbol(fact.annotation_symbol),
            ) else {
                self.unresolved.push(UnresolvedFact {
                    from: None,
                    kind: DepKind::AnnotationUse,
                    span: fact.span,
                    name: None,
                    reason: "the annotated declaration or the annotation type is not in the symbol table"
                        .to_string(),
                });
                continue;
            };
            let origin = Origin::Annotation {
                target: fact.annotated_symbol,
                token: fact.span.map_or(u32::MAX, |s| s.first_token),
            };
            self.record(from, to, DepKind::AnnotationUse, origin, Strength::Exact, fact.span, None);
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

    // ── Units ────────────────────────────────────────────────────────────────

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
        self.facts
            .entry((from, to, kind))
            .or_default()
            .add(origin, strength, span, reason);
    }

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
        let name = self
            .symbol_name(sym)
            .unwrap_or_else(|| format!("#{}", sym));
        let (package, package_symbol, module) = self.namespace_of(sym, file);

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
            package_symbol,
            module,
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
        let package = self.file_package_name(file).unwrap_or_default();
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
            package_symbol: None,
            module: None,
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

    fn member_role(&self, func: &FuncIr, owner: u32) -> MemberRole {
        match func.symbol.and_then(|s| self.kind_of(s)) {
            Some(SymbolKind::SK_CONSTRUCTOR) => MemberRole::Constructor,
            Some(SymbolKind::SK_STATIC_INIT) | Some(SymbolKind::SK_INSTANCE_INIT) => {
                MemberRole::Initializer
            }
            Some(SymbolKind::SK_LAMBDA) => MemberRole::Closure,
            _ if func.is_closure => MemberRole::Closure,
            _ if self.units[owner as usize].kind == NodeKind::Type => MemberRole::Method,
            _ => MemberRole::Function,
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
        if rec.first_token_id == u32::MAX {
            return None;
        }
        let file_id = self.files.file_of_token(rec.first_token_id)?;
        let last_token = if rec.last_token_id == u32::MAX || rec.last_token_id < rec.first_token_id
        {
            rec.first_token_id
        } else {
            rec.last_token_id
        };
        Some(Span {
            file_id,
            first_token: rec.first_token_id,
            last_token,
        })
    }

    fn token_span(&self, token: u32) -> Option<Span> {
        if token == u32::MAX {
            return None;
        }
        let file_id = self.files.file_of_token(token)?;
        Some(Span {
            file_id,
            first_token: token,
            last_token: token,
        })
    }

    /// Qualified name as recorded (package display names first, then the qualified-name
    /// table, then the simple name).
    fn symbol_name(&self, sym: u32) -> Option<String> {
        let rec = self.sta.symbol_records.get(sym as usize)?;
        if let Some(name) = self.sta.custom_package_names.get(&sym) {
            if !name.is_empty() {
                return Some(name.clone());
            }
        }
        if let Some(name) = qualified_name(self.sta, rec.qual_name_id) {
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

    /// Namespace name and symbol of the unit `sym`, and the name of an enclosing module
    /// declaration. Without a package ancestor, the file's recorded package applies; without
    /// either, the unit is in the global namespace.
    fn namespace_of(&self, sym: u32, file: Option<u32>) -> (String, Option<u32>, Option<String>) {
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
            Some(p) => self.symbol_name(p).unwrap_or_default(),
            None => file
                .and_then(|f| self.file_package_name(f))
                .unwrap_or_default(),
        };
        (name, package, module)
    }

    /// The child of `owner` named `name` whose kind `accept`s.
    fn member_symbol(&self, owner: u32, name: u32, accept: fn(SymbolKind) -> bool) -> Option<u32> {
        let records = &self.sta.symbol_records;
        let mut child = records.get(owner as usize)?.first_child;
        let mut steps = 0usize;
        while (child as usize) < records.len() && steps <= records.len() {
            let rec = &records[child as usize];
            if rec.name_id == name && accept(SymbolKind::from(rec.kind)) {
                return Some(child);
            }
            child = rec.next_sibling;
            steps += 1;
        }
        None
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
                        "the points-to call graph resolves no target for this indirect call"
                    } else {
                        "the target of an indirect call needs the points-to call graph, which was not supplied"
                    }
                    .to_string(),
                });
            }
        }

        // ── Type level: order units by name, then key ──
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

        // ── File level ──
        let mut file_groups = GroupTable::default();
        let mut unit_file = vec![0u32; self.units.len()];
        for &old in &order {
            let u = &self.units[old as usize];
            unit_file[old as usize] = match (u.external, u.file) {
                (false, Some(file)) => self.file_group(&mut file_groups, file),
                _ => namespace_group(&mut file_groups, &u.package, true, None),
            };
        }
        let mut import_files: BTreeSet<u32> = BTreeSet::new();
        for (file, _) in self.imports.keys() {
            import_files.insert(*file);
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
            .map(|&f| {
                let g = file_groups.by_key[&file_key(f)];
                (f, vec![(g, Strength::Exact)])
            })
            .collect();
        let mut unlifted = 0u32;
        for (_, key) in self.imports.keys() {
            if matches!(key, ImportKey::Namespace(_)) {
                unlifted += 1;
            }
        }
        let files = self.lift_level(
            Level::File,
            file_groups.nodes,
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
                namespace_group(&mut package_groups, &u.package, u.external, None);
        }
        for (_, key) in self.imports.keys() {
            if let ImportKey::Namespace(name) = key {
                namespace_group(&mut package_groups, name, true, None);
            }
        }
        // A file whose imports no project unit receives imports into its recorded package.
        let files_with_units: HashSet<u32> = self
            .units
            .iter()
            .filter(|u| !u.external)
            .filter_map(|u| u.file)
            .collect();
        for &file in &import_files {
            if !files_with_units.contains(&file) {
                let name = self.file_package_name(file).unwrap_or_default();
                namespace_group(&mut package_groups, &name, false, None);
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
            let parent =
                namespace_group(&mut package_groups, &parent_name, external, Some(parent_sym));
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
                namespace_group(&mut project_groups, &u.package, true, None)
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
        for (_, key) in self.imports.keys() {
            if let ImportKey::Namespace(name) = key {
                namespace_group(&mut project_groups, name, true, None);
            }
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
            if !u.external {
                namespace_projects
                    .entry(u.package.clone())
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
                        Some(set) if set.len() == 1 => {
                            set.iter().map(|&g| (g, Strength::Exact)).collect()
                        }
                        Some(set) if !set.is_empty() => {
                            set.iter().map(|&g| (g, Strength::OverApproximate)).collect()
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
        functions.sort_by_key(|f| f.func.0);
        let mut fields = std::mem::take(&mut self.fields);
        for f in &mut fields {
            f.owner = NodeId(remap[f.owner.index()]);
        }
        fields.sort_by_key(|f| f.field.0);
        let mut unresolved = std::mem::take(&mut self.unresolved);
        for u in &mut unresolved {
            u.from = u.from.map(|n| NodeId(remap[n.index()]));
        }

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
            unresolved,
            namespace_imports_without_file_target: unlifted,
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

/// One-call construction from the available artifacts.
pub fn build_model(
    sta: &SymbolTableArtifact,
    tca: &TokenCorpusArtifact,
    module: Option<&ModuleIr>,
    call_graph: Option<&CallGraph>,
    config: GraphConfig,
) -> ArchModel {
    let mut builder = ArchGraphBuilder::new(sta, tca, config);
    builder.add_declarations();
    if let Some(module) = module {
        builder.add_ir(module);
        if let Some(cg) = call_graph {
            builder.add_call_graph(module, cg);
        }
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

/// Namespace group `name`; a group stays external only while every member is external.
fn namespace_group(
    groups: &mut GroupTable,
    name: &str,
    external: bool,
    symbol: Option<u32>,
) -> u32 {
    let id = groups.intern(namespace_key(name), |id| ArchNode {
        id,
        kind: NodeKind::Package,
        key: namespace_key(name),
        name: name.to_string(),
        symbol,
        symbol_kind: symbol.map(|_| SymbolKind::SK_PACKAGE),
        file: None,
        external,
        is_abstract: false,
        visibility: None,
        parent: None,
        spans: Vec::new(),
    });
    if !external {
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

/// Sort and identity key of a span.
#[inline]
pub fn span_key(span: &Span) -> (u32, u32, u32) {
    (span.file_id, span.first_token, span.last_token)
}

/// Qualified name stored for `qual_name_id` (ids at or above the table base index the
/// table from the base; smaller ids index it directly).
pub fn qualified_name(sta: &SymbolTableArtifact, qual_name_id: u32) -> Option<&str> {
    const BASE: u32 = crate::symbol::QualifiedNameTable::BASE_QUAL_NAME_ID;
    if qual_name_id == u32::MAX {
        return None;
    }
    let index = if qual_name_id >= BASE {
        qual_name_id - BASE
    } else {
        qual_name_id
    };
    sta.qual_names.get(index as usize).map(String::as_str)
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

/// The type table entry of `id`.
#[inline]
fn ir_type(types: &TypeTable, id: TypeId) -> Option<&IrType> {
    types.get(id)
}

/// Named symbols in the type tree of `root`, each flagged when it is the head (the type
/// itself, through pointers, references and generic bases), and whether part of the tree is
/// unknown. Type parameters and primitive types are not dependencies.
pub fn named_in_type(types: &TypeTable, root: TypeId) -> (Vec<(u32, bool)>, bool) {
    let mut named = Vec::new();
    let mut unknown = false;
    let mut seen: HashSet<(u32, bool)> = HashSet::new();
    let mut stack = vec![(root, true)];
    while let Some((id, head)) = stack.pop() {
        if !seen.insert((id.0, head)) {
            continue;
        }
        let Some(ty) = ir_type(types, id) else {
            unknown = true;
            continue;
        };
        match ty {
            IrType::Named(sym) => named.push((*sym, head)),
            IrType::Generic { base, args, .. } => {
                stack.push((*base, head));
                stack.extend(args.iter().map(|&a| (a, false)));
            }
            IrType::Ptr(target) => stack.push((*target, head)),
            IrType::Ref { target, .. } => stack.push((*target, head)),
            IrType::Array { elem, .. } => stack.push((*elem, false)),
            IrType::Func { params, ret, .. } => {
                stack.extend(params.iter().map(|&p| (p, false)));
                stack.push((*ret, false));
            }
            IrType::Unknown => unknown = true,
            _ => {}
        }
    }
    named.sort_unstable();
    named.dedup();
    (named, unknown)
}
