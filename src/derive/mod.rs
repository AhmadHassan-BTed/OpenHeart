//! Diagram derivation: UML 2.5 views and compiler views computed from analysis facts.
//!
//! **Mandate:** every node, edge, fragment and note of every view is a function of facts
//! the analyses established statically: the lowered SSA IR with its explicit memory
//! operations (`crate::ir`), the abstract heap and the on-the-fly call graph
//! (`crate::memory`), dataflow results (`crate::analysis`) and the neutral declarations,
//! idioms and project structure the language frontends report (`crate::lang`). No view is
//! filled from a template, a naming convention or a guess. When the facts do not determine
//! an element, the element or the whole view carries `Derivation::Partial` or
//! `Derivation::NotDerivable` with the reason.
//!
//! **Inputs.** The IR and the call graph are read directly. Facts whose producers keep
//! their own result types (points-to sets, ownership, nullness, typestate, project model,
//! timing constraints, declarations) enter through the provider traits of this module, so
//! each producing phase implements a trait over its own results and the views never
//! depend on a producer's internals. Language knowledge reaches the views only through
//! `ProgramFacts`, which the frontends implement.
//!
//! **Views.**
//! ```text
//! class           classifiers, members, generalization, realization, composition,
//!                 aggregation, association, multiplicities, usage dependencies
//! object          abstract objects (allocation site × heap context), slot values, links
//! package         namespace containment, import and use dependencies
//! component       build units, declared and observed dependencies, interfaces
//! deployment      execution nodes and artifacts from build / deployment descriptors
//! sequence        interprocedural walk from entry points in control-flow order
//! communication   the sequence interactions as numbered messages over links
//! interaction     interaction overview: sequence fragments composed by control flow
//! activity        actions, decisions, merges, loops, exception flow, partitions
//! state           typestate machines of state-carrying fields
//! usecase         entry points, public API and callbacks as use cases, callers as actors
//! timing          state timelines over logical time, timing constraints
//! compiler_views  CFG, DFG, CDG, call graph with SCCs, ROBDD path summaries
//! ```
//!
//! **Determinism.** Element ids are hashes of qualified names, IR ids and spans; every
//! derivation iterates in id order, so equal inputs give byte-identical models.

pub mod model;
pub mod provenance;
pub mod class;
pub mod object;
pub mod package;
pub mod component;
pub mod deployment;
pub mod sequence;
pub mod communication;
pub mod activity;
pub mod state;
pub mod usecase;
pub mod interaction;
pub mod timing;
pub mod compiler_views;

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::rc::Rc;

use crate::derive::compiler_views::BddSummary;
use crate::derive::model::{DiagramKind, DiagramModel, Multiplicity};
use crate::ir::func::FuncIr;
use crate::ir::inst::Inst;
use crate::ir::module::{AllocSiteIr, CallSiteIr, FieldIr, GlobalIr, ModuleIr};
use crate::ir::types::{
    AllocSiteId, CallSiteId, FieldId, FuncId, GlobalId, IrType, RefKind, Span, TypeId, ValueId,
};
use crate::memory::callgraph::{CallGraph, CallbackOrigin, EdgePrecision, InstanceId, TypeHierarchy};
use crate::memory::location::{CellId, Context, LocationTable, ObjId};

// ── Frontend facts: declarations ─────────────────────────────────────────────

/// Neutral category of a declaration, as the language frontend reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SymbolCategory {
    /// Named scope of declarations (package, namespace).
    Namespace,
    /// Compilation or import unit that owns declarations (module, translation unit).
    Module,
    /// Source file without a namespace role.
    File,
    Class,
    Interface,
    /// Interface-like declaration with default members (trait, protocol, mixin).
    Trait,
    /// Value-semantics record (struct).
    Struct,
    Union,
    Enum,
    Record,
    Annotation,
    TypeAlias,
    /// Named function type (delegate, function interface declared as a type).
    Delegate,
    Function,
    Method,
    Constructor,
    Destructor,
    Initializer,
    Field,
    Property,
    EnumConstant,
    Event,
    Parameter,
    TypeParameter,
    Local,
    Closure,
    Other,
}

impl SymbolCategory {
    #[inline]
    pub fn is_namespace(self) -> bool {
        matches!(self, SymbolCategory::Namespace | SymbolCategory::Module)
    }

    #[inline]
    pub fn is_classifier(self) -> bool {
        matches!(
            self,
            SymbolCategory::Class
                | SymbolCategory::Interface
                | SymbolCategory::Trait
                | SymbolCategory::Struct
                | SymbolCategory::Union
                | SymbolCategory::Enum
                | SymbolCategory::Record
                | SymbolCategory::Annotation
                | SymbolCategory::Delegate
        )
    }

    #[inline]
    pub fn is_interface_like(self) -> bool {
        matches!(self, SymbolCategory::Interface | SymbolCategory::Trait)
    }

    #[inline]
    pub fn is_callable(self) -> bool {
        matches!(
            self,
            SymbolCategory::Function
                | SymbolCategory::Method
                | SymbolCategory::Constructor
                | SymbolCategory::Destructor
                | SymbolCategory::Initializer
        )
    }

    #[inline]
    pub fn is_attribute(self) -> bool {
        matches!(
            self,
            SymbolCategory::Field | SymbolCategory::Property | SymbolCategory::Event
        )
    }

    pub fn mnemonic(self) -> &'static str {
        match self {
            SymbolCategory::Namespace => "namespace",
            SymbolCategory::Module => "module",
            SymbolCategory::File => "file",
            SymbolCategory::Class => "class",
            SymbolCategory::Interface => "interface",
            SymbolCategory::Trait => "trait",
            SymbolCategory::Struct => "struct",
            SymbolCategory::Union => "union",
            SymbolCategory::Enum => "enum",
            SymbolCategory::Record => "record",
            SymbolCategory::Annotation => "annotation",
            SymbolCategory::TypeAlias => "type-alias",
            SymbolCategory::Delegate => "delegate",
            SymbolCategory::Function => "function",
            SymbolCategory::Method => "method",
            SymbolCategory::Constructor => "constructor",
            SymbolCategory::Destructor => "destructor",
            SymbolCategory::Initializer => "initializer",
            SymbolCategory::Field => "field",
            SymbolCategory::Property => "property",
            SymbolCategory::EnumConstant => "enum-constant",
            SymbolCategory::Event => "event",
            SymbolCategory::Parameter => "parameter",
            SymbolCategory::TypeParameter => "type-parameter",
            SymbolCategory::Local => "local",
            SymbolCategory::Closure => "closure",
            SymbolCategory::Other => "other",
        }
    }
}

/// Declared accessibility of a symbol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Visibility {
    Public,
    Protected,
    /// Package, assembly or module visibility.
    Internal,
    Private,
    /// The language or the declaration states none.
    Unspecified,
}

impl Visibility {
    /// UML visibility marker (`+ # ~ -`); empty when unspecified.
    pub fn marker(self) -> &'static str {
        match self {
            Visibility::Public => "+",
            Visibility::Protected => "#",
            Visibility::Internal => "~",
            Visibility::Private => "-",
            Visibility::Unspecified => "",
        }
    }

    #[inline]
    pub fn is_public(self) -> bool {
        self == Visibility::Public
    }
}

/// Declaration modifiers as bit flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Modifiers(pub u32);

impl Modifiers {
    pub const STATIC: u32 = 1 << 0;
    pub const ABSTRACT: u32 = 1 << 1;
    pub const FINAL: u32 = 1 << 2;
    pub const CONST: u32 = 1 << 3;
    pub const VIRTUAL: u32 = 1 << 4;
    pub const OVERRIDE: u32 = 1 << 5;
    pub const ASYNC: u32 = 1 << 6;
    /// Synthesized by a language rule; not written in the source.
    pub const SYNTHETIC: u32 = 1 << 7;
    pub const READ_ONLY: u32 = 1 << 8;
    pub const SEALED: u32 = 1 << 9;
    /// Declared here, defined outside the analysed sources.
    pub const EXTERN: u32 = 1 << 10;
    pub const SYNCHRONIZED: u32 = 1 << 11;

    #[inline]
    pub fn has(self, bit: u32) -> bool {
        self.0 & bit != 0
    }
}

/// A declared parameter of a callable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParamFact {
    pub name: Option<String>,
    pub ty: Option<TypeId>,
    /// Type exactly as written, when the frontend keeps it.
    pub type_text: Option<String>,
    pub variadic: bool,
}

/// One declaration of the symbol table in neutral terms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolFact {
    pub id: u32,
    pub name: String,
    pub qualified_name: String,
    pub category: SymbolCategory,
    pub visibility: Visibility,
    pub modifiers: Modifiers,
    /// Lexically enclosing declaration.
    pub parent: Option<u32>,
    pub span: Option<Span>,
    /// Declared type of an attribute, parameter or local, interned in the module's table.
    pub ty: Option<TypeId>,
    pub type_text: Option<String>,
    /// Declared parameters of a callable, receiver excluded.
    pub params: Vec<ParamFact>,
    pub ret: Option<TypeId>,
    pub ret_text: Option<String>,
    /// Type-parameter symbols of a generic declaration, in order.
    pub type_params: Vec<u32>,
    /// Declared outside the analysed sources (library, runtime, SDK).
    pub external: bool,
}

/// Kind of a direct supertype relation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SuperRelation {
    /// Inheritance of implementation (`extends`, base class, mixin inheritance).
    Extends,
    /// Conformance to an interface-like type (`implements`, protocol conformance).
    Implements,
}

/// A declared direct supertype.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupertypeFact {
    pub target: u32,
    pub relation: SuperRelation,
    pub span: Option<Span>,
}

/// Form of an import, include or using declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ImportKind {
    /// One named element.
    Element,
    /// Every member of a namespace.
    Wildcard,
    /// Static members of a type.
    Static,
    /// An element under a local alias.
    Alias,
    /// Textual inclusion of another file.
    Include,
}

impl ImportKind {
    pub fn mnemonic(self) -> &'static str {
        match self {
            ImportKind::Element => "import",
            ImportKind::Wildcard => "import",
            ImportKind::Static => "import",
            ImportKind::Alias => "import",
            ImportKind::Include => "include",
        }
    }
}

/// One import edge: `from` (a namespace, module or file symbol) imports `target`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportFact {
    pub from: u32,
    pub target: u32,
    pub kind: ImportKind,
    pub span: Option<Span>,
}

/// Element structure of a container type, as the frontend's UML semantics define it
/// (which type arguments are elements, which are keys, how many elements it holds).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerShape {
    pub element_args: Vec<usize>,
    pub key_args: Vec<usize>,
    pub multiplicity: Multiplicity,
    pub ordered: bool,
    pub unique: bool,
}

/// Why a function is invoked from outside the analysed code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EntryKind {
    /// Program entry (`main` and its equivalents).
    Main,
    /// Handler of an inbound request (HTTP route, RPC method, message endpoint).
    RequestHandler,
    /// Handler of an event, signal or UI action.
    EventHandler,
    /// Function a scheduler invokes.
    Scheduled,
    /// Exported library or plugin function.
    Exported,
    /// Test case a test runner invokes.
    Test,
    /// Module code external code calls back (an override of an external declaration, or a
    /// function value that reached the external world).
    Callback,
    /// An entry point of the points-to solve that no frontend fact classifies.
    AnalysisRoot,
    /// A reachable function no call site of the module calls.
    Uncalled,
}

impl EntryKind {
    pub fn mnemonic(self) -> &'static str {
        match self {
            EntryKind::Main => "main",
            EntryKind::RequestHandler => "request-handler",
            EntryKind::EventHandler => "event-handler",
            EntryKind::Scheduled => "scheduled",
            EntryKind::Exported => "exported",
            EntryKind::Test => "test",
            EntryKind::Callback => "callback",
            EntryKind::AnalysisRoot => "analysis-root",
            EntryKind::Uncalled => "uncalled",
        }
    }

    /// The kind comes from a frontend fact about the program, not from the call graph shape.
    pub fn is_declared(self) -> bool {
        !matches!(self, EntryKind::AnalysisRoot | EntryKind::Uncalled)
    }
}

/// Who invokes an entry point.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Invoker {
    /// The declaration through which outside code calls in (framework interface, base
    /// class, annotation type, external method declaration).
    Symbol(u32),
    /// A runtime role named by the frontend (process launcher, test runner, scheduler).
    Role(String),
}

/// One entry point of the program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryPointFact {
    pub func: FuncId,
    pub kind: EntryKind,
    /// Route, event or command name exactly as declared.
    pub name: Option<String>,
    /// Further declared attributes (verb, media type, schedule), as written.
    pub details: Vec<(String, String)>,
    pub invoker: Option<Invoker>,
    pub span: Option<Span>,
}

/// Which value of a call designates a task, a handle or a lock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HandleSource {
    /// The call's result.
    Result,
    /// The receiver (`args[0]` of an instance call).
    Receiver,
    /// Declared argument `i` (receiver excluded).
    Arg(usize),
    None,
}

/// Where the body of a spawned task comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TaskSource {
    /// The lowered call-graph targets of the spawning site.
    CallTargets,
    /// The function value passed as declared argument `i`.
    Arg(usize),
    /// The receiver object's run method, as resolved by the call graph.
    Receiver,
}

/// Concurrency meaning of a call, as the frontend's idioms define it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConcurrencyRole {
    /// Starts `task` on another thread of control; `handle` identifies it for joins.
    Spawn { task: TaskSource, handle: HandleSource },
    /// Waits for the task identified by `handle`.
    Join { handle: HandleSource },
    /// Acquires the lock `lock` (released by a matching `Unlock` or by `Inst::Release`).
    Lock { lock: HandleSource },
    Unlock { lock: HandleSource },
}

/// Declarations, idioms and structure the language frontends report.
pub trait ProgramFacts {
    fn symbol(&self, sym: u32) -> Option<SymbolFact>;
    /// Declarations directly enclosed by `sym`, in declaration order.
    fn members(&self, sym: u32) -> Vec<u32>;
    /// Outermost declarations (top-level namespaces, modules, files, types).
    fn top_level(&self) -> Vec<u32>;
    /// Declared direct supertypes of a type.
    fn supertypes(&self, sym: u32) -> Vec<SupertypeFact>;
    fn imports(&self) -> Vec<ImportFact> {
        Vec::new()
    }
    /// Element structure of the container type `base`, when it is one.
    fn container_shape(&self, _base: u32) -> Option<ContainerShape> {
        None
    }
    /// A type in the source language's own syntax.
    fn type_text(&self, _ty: TypeId) -> Option<String> {
        None
    }
    fn entry_points(&self) -> Vec<EntryPointFact> {
        Vec::new()
    }
    fn concurrency(&self, _site: CallSiteId) -> Option<ConcurrencyRole> {
        None
    }
    fn file_name(&self, _file_id: u32) -> Option<String> {
        None
    }
}

// ── Memory facts ─────────────────────────────────────────────────────────────

/// The solved abstract heap.
pub trait HeapFacts {
    /// Abstract objects and cells of the solve.
    fn locations(&self) -> &LocationTable;
    /// Objects `value` may point to in `func`, merged over every context of `func`.
    fn points_to(&self, func: FuncId, value: ValueId) -> Vec<ObjId>;
    /// Objects `value` may point to in one context of `func`.
    fn points_to_in(&self, func: FuncId, _context: Context, value: ValueId) -> Vec<ObjId> {
        self.points_to(func, value)
    }
    /// Objects a cell (an object slot, a global or a static field) may hold.
    fn cell_points_to(&self, cell: CellId) -> Vec<ObjId>;
}

/// Ownership of the objects a field refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PartOwnership {
    /// The holder alone owns the part; the part does not outlive it (composition).
    Exclusive,
    /// The holder shares ownership of the part with other holders (aggregation).
    Shared,
    /// The holder refers to the part without owning it (association).
    NonOwning,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnershipFact {
    pub kind: PartOwnership,
    /// The fact holds on every path (must-ownership), not only on some.
    pub exact: bool,
    /// Allocation, store and release sites that establish the fact.
    pub evidence: Vec<Span>,
}

/// Per-field facts of the memory model and the nullness analysis.
pub trait FieldFacts {
    fn ownership(&self, field: FieldId) -> Option<OwnershipFact>;
    /// `Some(true)` when the field never holds null after construction.
    fn never_null(&self, _field: FieldId) -> Option<bool> {
        None
    }
}

// ── Analysis facts: typestate ────────────────────────────────────────────────

/// One abstract state of a state-carrying location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateFact {
    pub id: u32,
    pub label: String,
    pub spans: Vec<Span>,
}

/// A transition between two states caused by executing `trigger`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransitionFact {
    pub from: u32,
    pub to: u32,
    pub trigger: Option<FuncId>,
    pub guard: Option<String>,
    pub spans: Vec<Span>,
    /// The transition happens on every execution of the trigger from `from`.
    pub exact: bool,
}

/// A state machine of one state-carrying location of a type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateMachineFact {
    /// Type symbol whose instances carry the state.
    pub owner: u32,
    /// The field holding the state; `None` for protocol states not stored in one field.
    pub field: Option<FieldId>,
    pub label: String,
    pub states: Vec<StateFact>,
    /// States right after construction.
    pub initial: Vec<u32>,
    /// Constructors that establish the initial states.
    pub initial_triggers: Vec<FuncId>,
    pub finals: Vec<u32>,
    pub transitions: Vec<TransitionFact>,
    /// (state, operation) pairs: the operation runs in the state without leaving it.
    pub internal: Vec<(u32, FuncId)>,
    pub exact: bool,
    pub notes: Vec<String>,
}

pub trait TypestateFacts {
    fn machines(&self) -> Vec<StateMachineFact>;
}

// ── Project facts ────────────────────────────────────────────────────────────

/// Kind of a build unit of the project model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum UnitKind {
    Workspace,
    Project,
    Module,
    Library,
    Executable,
    Assembly,
    Package,
    TestSuite,
    /// A third-party package the project depends on.
    External,
}

impl UnitKind {
    pub fn mnemonic(self) -> &'static str {
        match self {
            UnitKind::Workspace => "workspace",
            UnitKind::Project => "project",
            UnitKind::Module => "module",
            UnitKind::Library => "library",
            UnitKind::Executable => "executable",
            UnitKind::Assembly => "assembly",
            UnitKind::Package => "package",
            UnitKind::TestSuite => "test-suite",
            UnitKind::External => "external",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectUnitFact {
    pub id: u32,
    pub name: String,
    pub kind: UnitKind,
    pub parent: Option<u32>,
    /// Build file that declares the unit.
    pub descriptor: Option<String>,
    /// Corpus file ids of the unit's sources.
    pub files: Vec<u32>,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DependencyScope {
    Compile,
    Runtime,
    Test,
    Build,
    Provided,
    Optional,
    Unspecified,
}

impl DependencyScope {
    pub fn mnemonic(self) -> &'static str {
        match self {
            DependencyScope::Compile => "compile",
            DependencyScope::Runtime => "runtime",
            DependencyScope::Test => "test",
            DependencyScope::Build => "build",
            DependencyScope::Provided => "provided",
            DependencyScope::Optional => "optional",
            DependencyScope::Unspecified => "unspecified",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitDependencyFact {
    pub from: u32,
    pub to: u32,
    pub scope: DependencyScope,
    pub version: Option<String>,
    pub descriptor: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ExecutionNodeKind {
    Device,
    ExecutionEnvironment,
    Container,
    Host,
    Service,
}

impl ExecutionNodeKind {
    pub fn mnemonic(self) -> &'static str {
        match self {
            ExecutionNodeKind::Device => "device",
            ExecutionNodeKind::ExecutionEnvironment => "executionEnvironment",
            ExecutionNodeKind::Container => "container",
            ExecutionNodeKind::Host => "host",
            ExecutionNodeKind::Service => "service",
        }
    }
}

/// An execution node declared by a build or deployment descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionNodeFact {
    pub id: u32,
    pub name: String,
    pub kind: ExecutionNodeKind,
    pub parent: Option<u32>,
    pub descriptor: String,
    pub properties: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeployedArtifactFact {
    pub name: String,
    pub node: u32,
    /// Build unit the artifact manifests.
    pub unit: Option<u32>,
    pub descriptor: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommunicationPathFact {
    pub from: u32,
    pub to: u32,
    pub protocol: Option<String>,
    pub descriptor: String,
}

/// The language-neutral project graph of `crate::lang::project_model`.
pub trait ProjectFacts {
    fn units(&self) -> Vec<ProjectUnitFact>;
    fn dependencies(&self) -> Vec<UnitDependencyFact>;
    fn execution_nodes(&self) -> Vec<ExecutionNodeFact> {
        Vec::new()
    }
    fn deployed_artifacts(&self) -> Vec<DeployedArtifactFact> {
        Vec::new()
    }
    fn communication_paths(&self) -> Vec<CommunicationPathFact> {
        Vec::new()
    }
}

// ── Timing facts ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TimingConstraintKind {
    Duration,
    Period,
    Deadline,
    Timeout,
    Delay,
}

impl TimingConstraintKind {
    pub fn mnemonic(self) -> &'static str {
        match self {
            TimingConstraintKind::Duration => "duration",
            TimingConstraintKind::Period => "period",
            TimingConstraintKind::Deadline => "deadline",
            TimingConstraintKind::Timeout => "timeout",
            TimingConstraintKind::Delay => "delay",
        }
    }
}

/// A timing constraint the program states (schedule, timeout argument, deadline
/// annotation), with its bound exactly as written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimingConstraintFact {
    pub func: Option<FuncId>,
    pub site: Option<CallSiteId>,
    pub kind: TimingConstraintKind,
    pub bound: String,
    pub span: Option<Span>,
}

pub trait TimingFacts {
    fn constraints(&self) -> Vec<TimingConstraintFact>;
}

// ── Limits ───────────────────────────────────────────────────────────────────

/// Explicit cut-offs. Every derivation that reaches one marks its output `Partial` with
/// the limit in the reason; none is applied silently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeriveLimits {
    /// Interprocedural depth of the sequence walk.
    pub max_call_depth: usize,
    /// Messages per interaction.
    pub max_messages: usize,
    /// Block visits per interaction.
    pub max_walk_steps: usize,
    /// Diagrams per view kind for per-function and per-entry views.
    pub max_diagrams: usize,
    /// Objects of an object diagram.
    pub max_objects: usize,
    /// Spans and facts kept per element.
    pub max_spans: usize,
    /// Nesting of reconstructed expressions in labels and guards.
    pub max_expr_depth: usize,
    /// Distinct values of a state-carrying field.
    pub max_state_domain: usize,
    /// Treat public operations nothing in the module calls as use cases.
    pub include_public_api: bool,
}

impl Default for DeriveLimits {
    fn default() -> Self {
        Self {
            max_call_depth: 12,
            max_messages: 4000,
            max_walk_steps: 200_000,
            max_diagrams: 2000,
            max_objects: 20_000,
            max_spans: 64,
            max_expr_depth: 4,
            max_state_domain: 64,
            include_public_api: true,
        }
    }
}

// ── Context ──────────────────────────────────────────────────────────────────

/// Position of an instruction: function, dense block index, instruction index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct InstPos {
    pub func: FuncId,
    pub block: usize,
    pub inst: usize,
}

/// Lookup structures over the module and the call graph, built once per context.
#[derive(Debug, Default)]
struct ModuleIndex {
    callees: HashMap<CallSiteId, Vec<FuncId>>,
    exact_edges: HashSet<(CallSiteId, FuncId)>,
    incoming: HashMap<FuncId, Vec<(CallSiteId, FuncId)>>,
    reachable: HashSet<FuncId>,
    func_of_symbol: HashMap<u32, FuncId>,
    field_of_symbol: HashMap<u32, FieldId>,
    funcs_of_type: BTreeMap<u32, Vec<FuncId>>,
    fields_of_owner: BTreeMap<u32, Vec<FieldId>>,
    call_pos: HashMap<CallSiteId, InstPos>,
    alloc_pos: HashMap<AllocSiteId, InstPos>,
    instances: HashMap<(FuncId, Context), InstanceId>,
    context_edges: HashMap<(InstanceId, CallSiteId), Vec<InstanceId>>,
}

impl ModuleIndex {
    fn build(module: &ModuleIr, cg: &CallGraph) -> Self {
        let mut idx = ModuleIndex::default();
        for func in &module.funcs {
            if let Some(sym) = func.symbol {
                idx.func_of_symbol.entry(sym).or_insert(func.id);
            }
            if let Some(owner) = func.enclosing_type {
                idx.funcs_of_type.entry(owner).or_default().push(func.id);
            }
            for (bi, block) in func.blocks.iter().enumerate() {
                for (ii, inst) in block.insts.iter().enumerate() {
                    let pos = InstPos {
                        func: func.id,
                        block: bi,
                        inst: ii,
                    };
                    match inst {
                        Inst::Call { site, .. } => {
                            idx.call_pos.insert(*site, pos);
                        }
                        Inst::Alloc { site, .. } => {
                            idx.alloc_pos.insert(*site, pos);
                        }
                        _ => {}
                    }
                }
            }
        }
        for field in &module.fields {
            if let Some(sym) = field.symbol {
                idx.field_of_symbol.entry(sym).or_insert(field.id);
            }
            if field.owner_symbol != u32::MAX {
                idx.fields_of_owner
                    .entry(field.owner_symbol)
                    .or_default()
                    .push(field.id);
            }
        }
        for &(site, callee) in &cg.edges {
            idx.callees.entry(site).or_default().push(callee);
        }
        for info in &cg.edge_info {
            if info.precision == EdgePrecision::Exact {
                idx.exact_edges.insert((info.site, info.callee));
            }
            idx.incoming
                .entry(info.callee)
                .or_default()
                .push((info.site, info.caller));
        }
        for list in idx.callees.values_mut() {
            list.sort_unstable();
            list.dedup();
        }
        for list in idx.incoming.values_mut() {
            list.sort_unstable();
            list.dedup();
        }
        idx.reachable = cg.reachable.iter().copied().collect();
        for (i, inst) in cg.instances.iter().enumerate() {
            idx.instances
                .entry((inst.func, inst.context))
                .or_insert(InstanceId(i as u32));
        }
        for edge in &cg.context_edges {
            idx.context_edges
                .entry((edge.caller, edge.site))
                .or_default()
                .push(edge.callee);
        }
        for list in idx.context_edges.values_mut() {
            list.sort_unstable();
            list.dedup();
        }
        idx
    }
}

/// Adapts `TypeTable::get` whether it returns the type or an optional type.
trait TypeLookupResult<'t> {
    fn into_type(self) -> Option<&'t IrType>;
}

impl<'t> TypeLookupResult<'t> for &'t IrType {
    fn into_type(self) -> Option<&'t IrType> {
        Some(self)
    }
}

impl<'t> TypeLookupResult<'t> for Option<&'t IrType> {
    fn into_type(self) -> Option<&'t IrType> {
        self
    }
}

/// Everything a view is derived from, with shared lookups.
pub struct DeriveContext<'a> {
    pub module: &'a ModuleIr,
    pub call_graph: &'a CallGraph,
    pub program: &'a dyn ProgramFacts,
    /// Override relation used to group receiver objects by dispatch target.
    pub hierarchy: Option<&'a TypeHierarchy>,
    pub heap: Option<&'a dyn HeapFacts>,
    pub fields: Option<&'a dyn FieldFacts>,
    pub typestate: Option<&'a dyn TypestateFacts>,
    pub project: Option<&'a dyn ProjectFacts>,
    pub timing: Option<&'a dyn TimingFacts>,
    pub bdds: &'a [BddSummary],
    pub limits: DeriveLimits,
    index: ModuleIndex,
    symbols: RefCell<HashMap<u32, Option<Rc<SymbolFact>>>>,
}

impl<'a> DeriveContext<'a> {
    pub fn new(
        module: &'a ModuleIr,
        call_graph: &'a CallGraph,
        program: &'a dyn ProgramFacts,
    ) -> Self {
        Self {
            module,
            call_graph,
            program,
            hierarchy: None,
            heap: None,
            fields: None,
            typestate: None,
            project: None,
            timing: None,
            bdds: &[],
            limits: DeriveLimits::default(),
            index: ModuleIndex::build(module, call_graph),
            symbols: RefCell::new(HashMap::new()),
        }
    }

    pub fn with_hierarchy(mut self, hierarchy: &'a TypeHierarchy) -> Self {
        self.hierarchy = Some(hierarchy);
        self
    }

    pub fn with_heap(mut self, heap: &'a dyn HeapFacts) -> Self {
        self.heap = Some(heap);
        self
    }

    pub fn with_fields(mut self, fields: &'a dyn FieldFacts) -> Self {
        self.fields = Some(fields);
        self
    }

    pub fn with_typestate(mut self, typestate: &'a dyn TypestateFacts) -> Self {
        self.typestate = Some(typestate);
        self
    }

    pub fn with_project(mut self, project: &'a dyn ProjectFacts) -> Self {
        self.project = Some(project);
        self
    }

    pub fn with_timing(mut self, timing: &'a dyn TimingFacts) -> Self {
        self.timing = Some(timing);
        self
    }

    pub fn with_bdds(mut self, bdds: &'a [BddSummary]) -> Self {
        self.bdds = bdds;
        self
    }

    pub fn with_limits(mut self, limits: DeriveLimits) -> Self {
        self.limits = limits;
        self
    }

    // ── IR lookups ──

    pub fn func(&self, id: FuncId) -> Option<&'a FuncIr> {
        let module: &'a ModuleIr = self.module;
        module.funcs.get(id.index())
    }

    pub fn funcs(&self) -> &'a [FuncIr] {
        let module: &'a ModuleIr = self.module;
        &module.funcs
    }

    pub fn field(&self, id: FieldId) -> Option<&'a FieldIr> {
        let module: &'a ModuleIr = self.module;
        module.fields.get(id.index())
    }

    pub fn global(&self, id: GlobalId) -> Option<&'a GlobalIr> {
        let module: &'a ModuleIr = self.module;
        module.globals.get(id.index())
    }

    pub fn alloc_site(&self, id: AllocSiteId) -> Option<&'a AllocSiteIr> {
        let module: &'a ModuleIr = self.module;
        module.alloc_sites.get(id.index())
    }

    pub fn call_site(&self, id: CallSiteId) -> Option<&'a CallSiteIr> {
        let module: &'a ModuleIr = self.module;
        module.call_sites.get(id.index())
    }

    pub fn ir_type(&self, id: TypeId) -> Option<&'a IrType> {
        let module: &'a ModuleIr = self.module;
        module.types.get(id).into_type()
    }

    /// Text of an interned id.
    pub fn text(&self, id: u32) -> String {
        if id == u32::MAX {
            return String::new();
        }
        self.module.text_lossy(id)
    }

    pub fn inst_at(&self, pos: InstPos) -> Option<&'a Inst> {
        self.func(pos.func)?.blocks.get(pos.block)?.insts.get(pos.inst)
    }

    pub fn call_pos(&self, site: CallSiteId) -> Option<InstPos> {
        self.index.call_pos.get(&site).copied()
    }

    pub fn alloc_pos(&self, site: AllocSiteId) -> Option<InstPos> {
        self.index.alloc_pos.get(&site).copied()
    }

    // ── Call graph lookups ──

    /// Lowered callees of a call site, sorted.
    pub fn callees(&self, site: CallSiteId) -> &[FuncId] {
        self.index
            .callees
            .get(&site)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// True when the edge derives from a statically named callee or a receiver object's
    /// dynamic type (no class-hierarchy fallback).
    pub fn edge_is_exact(&self, site: CallSiteId, callee: FuncId) -> bool {
        self.index.exact_edges.contains(&(site, callee))
    }

    /// (call site, caller) pairs of the calls that reach `func`, sorted.
    pub fn callers(&self, func: FuncId) -> &[(CallSiteId, FuncId)] {
        self.index
            .incoming
            .get(&func)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    pub fn is_reachable(&self, func: FuncId) -> bool {
        self.index.reachable.contains(&func)
    }

    /// Context of `callee` when called from `caller` (in `caller_context`) through `site`.
    pub fn callee_context(
        &self,
        caller: FuncId,
        caller_context: Context,
        site: CallSiteId,
        callee: FuncId,
    ) -> Context {
        let Some(&inst) = self.index.instances.get(&(caller, caller_context)) else {
            return Context::Empty;
        };
        let Some(targets) = self.index.context_edges.get(&(inst, site)) else {
            return Context::Empty;
        };
        targets
            .iter()
            .map(|&t| self.call_graph.instance(t))
            .find(|fi| fi.func == callee)
            .map(|fi| fi.context)
            .unwrap_or(Context::Empty)
    }

    // ── Symbol lookups ──

    pub fn symbol(&self, sym: u32) -> Option<Rc<SymbolFact>> {
        if sym == u32::MAX {
            return None;
        }
        if let Some(hit) = self.symbols.borrow().get(&sym) {
            return hit.clone();
        }
        let fact = self.program.symbol(sym).map(Rc::new);
        self.symbols.borrow_mut().insert(sym, fact.clone());
        fact
    }

    pub fn category(&self, sym: u32) -> Option<SymbolCategory> {
        self.symbol(sym).map(|s| s.category)
    }

    pub fn is_classifier(&self, sym: u32) -> bool {
        self.category(sym).map_or(false, |c| c.is_classifier())
    }

    /// Simple name: the frontend's name, else the module's symbol-name table.
    pub fn symbol_name(&self, sym: u32) -> String {
        if let Some(s) = self.symbol(sym) {
            if !s.name.is_empty() {
                return s.name.clone();
            }
        }
        match self.module.symbol_name(sym) {
            Some(bytes) => String::from_utf8_lossy(bytes).into_owned(),
            None => format!("#{}", sym),
        }
    }

    pub fn qualified_name(&self, sym: u32) -> String {
        match self.symbol(sym) {
            Some(s) if !s.qualified_name.is_empty() => s.qualified_name.clone(),
            _ => self.symbol_name(sym),
        }
    }

    /// Nearest classifier enclosing `sym`, `sym` included.
    pub fn classifier_of(&self, sym: u32) -> Option<u32> {
        self.ancestor_where(sym, |c| c.is_classifier())
    }

    /// Nearest namespace or module enclosing `sym`, `sym` included.
    pub fn namespace_of(&self, sym: u32) -> Option<u32> {
        self.ancestor_where(sym, |c| c.is_namespace())
    }

    fn ancestor_where(&self, sym: u32, pred: impl Fn(SymbolCategory) -> bool) -> Option<u32> {
        let mut cur = Some(sym);
        let mut seen = HashSet::new();
        while let Some(s) = cur {
            if !seen.insert(s) {
                return None;
            }
            let fact = self.symbol(s)?;
            if pred(fact.category) {
                return Some(s);
            }
            cur = fact.parent;
        }
        None
    }

    /// Ancestors of a type through declared supertypes, `sym` excluded, sorted.
    pub fn supertype_closure(&self, sym: u32) -> Vec<u32> {
        let mut seen = BTreeSet::new();
        let mut queue = VecDeque::new();
        queue.push_back(sym);
        while let Some(t) = queue.pop_front() {
            for sup in self.program.supertypes(t) {
                if sup.target != sym && seen.insert(sup.target) {
                    queue.push_back(sup.target);
                }
            }
        }
        seen.into_iter().collect()
    }

    pub fn func_of_symbol(&self, sym: u32) -> Option<FuncId> {
        self.index.func_of_symbol.get(&sym).copied()
    }

    pub fn field_of_symbol(&self, sym: u32) -> Option<FieldId> {
        self.index.field_of_symbol.get(&sym).copied()
    }

    pub fn funcs_of_type(&self, sym: u32) -> &[FuncId] {
        self.index
            .funcs_of_type
            .get(&sym)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    pub fn fields_of_owner(&self, sym: u32) -> &[FieldId] {
        self.index
            .fields_of_owner
            .get(&sym)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Type a function is a member of: the IR's enclosing type, else the declaring
    /// symbol's nearest classifier.
    pub fn owner_type(&self, func: &FuncIr) -> Option<u32> {
        func.enclosing_type.or_else(|| {
            let sym = func.symbol?;
            let parent = self.symbol(sym)?.parent?;
            self.classifier_of(parent)
        })
    }

    pub fn func_name(&self, func: &FuncIr) -> String {
        match func.symbol {
            Some(sym) => self.symbol_name(sym),
            None => self.text(func.name),
        }
    }

    pub fn func_qualified_name(&self, func: &FuncIr) -> String {
        if let Some(sym) = func.symbol {
            if let Some(s) = self.symbol(sym) {
                if !s.qualified_name.is_empty() {
                    return s.qualified_name.clone();
                }
            }
        }
        match self.owner_type(func) {
            Some(owner) => format!("{}.{}", self.qualified_name(owner), self.func_name(func)),
            None => self.func_name(func),
        }
    }

    /// Source variable name of an SSA value.
    pub fn value_name(&self, func: &FuncIr, value: ValueId) -> Option<String> {
        let info = func.value_info(value)?;
        if let Some(name) = info.name {
            return Some(self.text(name));
        }
        info.symbol.map(|s| self.symbol_name(s))
    }

    pub fn file_label(&self, file_id: u32) -> String {
        self.program
            .file_name(file_id)
            .unwrap_or_else(|| format!("file#{}", file_id))
    }

    // ── Types ──

    /// A type in source syntax when the frontend renders it, else in neutral notation.
    pub fn render_type(&self, ty: TypeId) -> String {
        if let Some(text) = self.program.type_text(ty) {
            return text;
        }
        self.render_type_neutral(ty, 0)
    }

    fn render_type_neutral(&self, ty: TypeId, depth: usize) -> String {
        if depth > 8 {
            return "…".to_string();
        }
        let inner = |t: TypeId| self.render_type_neutral(t, depth + 1);
        match self.ir_type(ty) {
            None | Some(IrType::Unknown) => "?".to_string(),
            Some(IrType::Void) => "void".to_string(),
            Some(IrType::Bool) => "bool".to_string(),
            Some(IrType::Int { bits, signed }) => {
                format!("{}int{}", if *signed { "" } else { "u" }, bits)
            }
            Some(IrType::Float { bits }) => format!("float{}", bits),
            Some(IrType::Char) => "char".to_string(),
            Some(IrType::Str) => "string".to_string(),
            Some(IrType::Ptr(t)) => format!("{}*", inner(*t)),
            Some(IrType::Ref { target, kind }) => match kind {
                RefKind::Borrowed => format!("{}&", inner(*target)),
                RefKind::Shared => inner(*target),
                RefKind::Weak => format!("weak {}", inner(*target)),
                RefKind::Owning => format!("owned {}", inner(*target)),
            },
            Some(IrType::Named(sym)) | Some(IrType::TypeParam(sym)) => self.symbol_name(*sym),
            Some(IrType::Array { elem, len }) => match len {
                Some(n) => format!("{}[{}]", inner(*elem), n),
                None => format!("{}[]", inner(*elem)),
            },
            Some(IrType::Func { params, ret }) => format!(
                "({}) -> {}",
                params.iter().map(|p| inner(*p)).collect::<Vec<_>>().join(", "),
                inner(*ret)
            ),
            Some(IrType::Generic { base, args }) => format!(
                "{}<{}>",
                inner(*base),
                args.iter().map(|a| inner(*a)).collect::<Vec<_>>().join(", ")
            ),
        }
    }

    /// Classifier symbols a type mentions (through references, arrays, generics and
    /// function types), in first-occurrence order.
    pub fn classifiers_in_type(&self, ty: TypeId) -> Vec<u32> {
        let mut out = Vec::new();
        self.collect_classifiers(ty, 0, &mut out);
        out
    }

    fn collect_classifiers(&self, ty: TypeId, depth: usize, out: &mut Vec<u32>) {
        if depth > 16 {
            return;
        }
        match self.ir_type(ty) {
            Some(IrType::Named(sym)) => {
                if self.is_classifier(*sym) && !out.contains(sym) {
                    out.push(*sym);
                }
            }
            Some(IrType::Ptr(t)) => self.collect_classifiers(*t, depth + 1, out),
            Some(IrType::Ref { target, .. }) => self.collect_classifiers(*target, depth + 1, out),
            Some(IrType::Array { elem, .. }) => self.collect_classifiers(*elem, depth + 1, out),
            Some(IrType::Generic { base, args }) => {
                self.collect_classifiers(*base, depth + 1, out);
                for a in args {
                    self.collect_classifiers(*a, depth + 1, out);
                }
            }
            Some(IrType::Func { params, ret }) => {
                for p in params {
                    self.collect_classifiers(*p, depth + 1, out);
                }
                self.collect_classifiers(*ret, depth + 1, out);
            }
            _ => {}
        }
    }

    /// The nominal type a value of type `ty` denotes, through references and pointers.
    pub fn named_symbol(&self, ty: TypeId) -> Option<u32> {
        let mut cur = ty;
        for _ in 0..32 {
            match self.ir_type(cur)? {
                IrType::Named(sym) => return Some(*sym),
                IrType::Generic { base, .. } => cur = *base,
                IrType::Ref { target, .. } => cur = *target,
                IrType::Ptr(target) => cur = *target,
                _ => return None,
            }
        }
        None
    }

    // ── Heap ──

    /// Points-to set of a value in one context, sorted and deduplicated (empty without
    /// heap facts).
    pub fn objects_of(&self, func: FuncId, context: Context, value: ValueId) -> Vec<ObjId> {
        let Some(heap) = self.heap else {
            return Vec::new();
        };
        let mut objs = heap.points_to_in(func, context, value);
        objs.sort_unstable();
        objs.dedup();
        objs
    }

    // ── Entry points ──

    /// Entry points: the frontends' classified entries, the points-to solve's entries and
    /// the callbacks external code makes into the module; when none exist, the reachable
    /// functions that no call site calls. Sorted by function, one fact per function.
    pub fn entry_points(&self) -> Vec<EntryPointFact> {
        let has_body = |f: FuncId| self.func(f).map_or(false, |fi| !fi.blocks.is_empty());
        let mut by_func: BTreeMap<FuncId, EntryPointFact> = BTreeMap::new();
        for e in self.program.entry_points() {
            if has_body(e.func) {
                by_func.entry(e.func).or_insert(e);
            }
        }
        for cb in &self.call_graph.callbacks {
            if !has_body(cb.func) || by_func.contains_key(&cb.func) {
                continue;
            }
            let invoker = match cb.origin {
                CallbackOrigin::ExternalOverride { declaration } => {
                    let owner = self
                        .symbol(declaration)
                        .and_then(|d| d.parent)
                        .and_then(|p| self.classifier_of(p))
                        .unwrap_or(declaration);
                    Some(Invoker::Symbol(owner))
                }
                CallbackOrigin::ExternalFunctionValue => None,
                CallbackOrigin::Intrinsic { name, .. } => Some(Invoker::Role(self.text(name))),
            };
            by_func.insert(
                cb.func,
                EntryPointFact {
                    func: cb.func,
                    kind: EntryKind::Callback,
                    name: None,
                    details: Vec::new(),
                    invoker,
                    span: cb.span,
                },
            );
        }
        for &f in &self.call_graph.entries {
            if has_body(f) && !by_func.contains_key(&f) {
                by_func.insert(
                    f,
                    EntryPointFact {
                        func: f,
                        kind: EntryKind::AnalysisRoot,
                        name: None,
                        details: Vec::new(),
                        invoker: None,
                        span: self.func(f).map(|fi| fi.span).filter(|s| !s.is_none()),
                    },
                );
            }
        }
        if by_func.is_empty() {
            for &f in &self.call_graph.reachable {
                if has_body(f) && self.callers(f).is_empty() {
                    by_func.insert(
                        f,
                        EntryPointFact {
                            func: f,
                            kind: EntryKind::Uncalled,
                            name: None,
                            details: Vec::new(),
                            invoker: None,
                            span: self.func(f).map(|fi| fi.span).filter(|s| !s.is_none()),
                        },
                    );
                }
            }
        }
        by_func.into_values().collect()
    }
}

// ── Derivation entry points ──────────────────────────────────────────────────

/// Derives every diagram of one kind.
pub fn derive(ctx: &DeriveContext, kind: DiagramKind) -> Vec<DiagramModel> {
    match kind {
        DiagramKind::Class => vec![class::derive(ctx)],
        DiagramKind::Object => vec![object::derive(ctx)],
        DiagramKind::Package => vec![package::derive(ctx)],
        DiagramKind::Component => vec![component::derive(ctx)],
        DiagramKind::Deployment => vec![deployment::derive(ctx)],
        DiagramKind::UseCase => vec![usecase::derive(ctx)],
        DiagramKind::Activity => activity::derive(ctx),
        DiagramKind::StateMachine => state::derive(ctx),
        DiagramKind::Sequence => {
            let traces = sequence::trace_entries(ctx);
            sequence::derive_from_traces(ctx, &traces)
        }
        DiagramKind::Communication => {
            let traces = sequence::trace_entries(ctx);
            communication::derive_from_traces(ctx, &traces)
        }
        DiagramKind::InteractionOverview => {
            let traces = sequence::trace_entries(ctx);
            interaction::derive_from_traces(ctx, &traces)
        }
        DiagramKind::Timing => {
            let traces = sequence::trace_entries(ctx);
            let machines = state::machines(ctx);
            vec![timing::derive_from(ctx, &traces, &machines)]
        }
        DiagramKind::Cfg | DiagramKind::Dfg | DiagramKind::Cdg | DiagramKind::CallGraph
        | DiagramKind::Robdd => compiler_views::derive(ctx, kind),
    }
}

/// Derives every view. Interaction traces and state machines are computed once and shared
/// by the sequence, communication, interaction overview and timing views.
pub fn derive_all(ctx: &DeriveContext) -> Vec<DiagramModel> {
    let traces = sequence::trace_entries(ctx);
    let machines = state::machines(ctx);
    let mut out = Vec::new();
    for kind in DiagramKind::ALL {
        match kind {
            DiagramKind::Sequence => out.extend(sequence::derive_from_traces(ctx, &traces)),
            DiagramKind::Communication => {
                out.extend(communication::derive_from_traces(ctx, &traces))
            }
            DiagramKind::InteractionOverview => {
                out.extend(interaction::derive_from_traces(ctx, &traces))
            }
            DiagramKind::Timing => out.push(timing::derive_from(ctx, &traces, &machines)),
            DiagramKind::StateMachine => out.extend(state::derive_from(ctx, &machines)),
            other => out.extend(derive(ctx, other)),
        }
    }
    out
}

/// Keeps the first `limit` items and reports how many were cut.
pub(crate) fn cap<T>(items: &mut Vec<T>, limit: usize) -> usize {
    if items.len() > limit {
        let cut = items.len() - limit;
        items.truncate(limit);
        cut
    } else {
        0
    }
}
