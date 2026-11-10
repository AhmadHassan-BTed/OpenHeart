//! Typed view IR shared by every diagram printer (§10.4).
//!
//! A view builder computes one [`ViewModel`] per diagram kind from the analysis artifacts;
//! PlantUML, Mermaid, JSON, XMI-adjacent, DOT and GraphML printers only visit it. Elements
//! carry stable unique keys (qualified names or structural ids, never bare simple names),
//! identifier-safe aliases for the text formats, source spans and a provenance tier.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::scpg::diagram::common::DiagramUtils;
use crate::scpg::diagram::context::{SourceSpan, ViewOptions};

/// Maximum number of omitted element names kept verbatim in a model.
const MAX_OMITTED_NAMES: usize = 200;

/// How an element or a whole projection relates to the analysis artifacts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provenance {
    /// Every element maps to an artifact entity and follows from it exactly.
    Derived,
    /// Sound may-information: everything that can occur is shown, possibly more.
    OverApproximate,
    /// Derived from incomplete facts or cut off by a limit; the reason is in the notes.
    Partial,
    /// Elements are inferred by a documented heuristic over artifact data.
    Heuristic,
    /// The projection cannot be derived from the available artifacts.
    Unavailable,
}

impl Provenance {
    /// Serialized name of the tier (JSON, GraphML, text-format headers).
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Derived => "derived",
            Self::OverApproximate => "over_approximate",
            Self::Partial => "partial",
            Self::Heuristic => "heuristic",
            Self::Unavailable => "unavailable",
        }
    }

    /// Stereotype text marking an element of this tier in the text formats (`None` for
    /// exactly derived elements).
    pub fn element_marker(&self) -> Option<&'static str> {
        match self {
            Self::Derived => None,
            Self::OverApproximate => Some("may"),
            Self::Partial => Some("partial"),
            Self::Heuristic => Some("heuristic"),
            Self::Unavailable => Some("not derivable"),
        }
    }

    /// The weaker of two tiers (`Derived` < `OverApproximate` < `Partial` < `Heuristic` <
    /// `Unavailable`).
    pub fn weakest(self, other: Self) -> Self {
        if self >= other {
            self
        } else {
            other
        }
    }
}

/// The diagram projections OpenHeart can print.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum ViewKind {
    Class,
    Object,
    Package,
    Component,
    CompositeStructure,
    Deployment,
    Profile,
    UseCase,
    Activity,
    StateMachine,
    Sequence,
    Communication,
    InteractionOverview,
    Timing,
    Cfg,
    Dfg,
    Cdg,
    CallGraph,
    Robdd,
}

impl ViewKind {
    pub const ALL: [ViewKind; 19] = [
        ViewKind::Class,
        ViewKind::Object,
        ViewKind::Package,
        ViewKind::Component,
        ViewKind::CompositeStructure,
        ViewKind::Deployment,
        ViewKind::Profile,
        ViewKind::UseCase,
        ViewKind::Activity,
        ViewKind::StateMachine,
        ViewKind::Sequence,
        ViewKind::Communication,
        ViewKind::InteractionOverview,
        ViewKind::Timing,
        ViewKind::Cfg,
        ViewKind::Dfg,
        ViewKind::Cdg,
        ViewKind::CallGraph,
        ViewKind::Robdd,
    ];

    /// Canonical strategy key of the PlantUML and Mermaid registries.
    pub fn key(self) -> &'static str {
        match self {
            Self::Class => "class",
            Self::Object => "object",
            Self::Package => "package",
            Self::Component => "component",
            Self::CompositeStructure => "composite",
            Self::Deployment => "deployment",
            Self::Profile => "profile",
            Self::UseCase => "usecase",
            Self::Activity => "activity",
            Self::StateMachine => "statemachine",
            Self::Sequence => "sequence",
            Self::Communication => "communication",
            Self::InteractionOverview => "interaction",
            Self::Timing => "timing",
            Self::Cfg => "cfg",
            Self::Dfg => "dfg",
            Self::Cdg => "cdg",
            Self::CallGraph => "callgraph",
            Self::Robdd => "robdd",
        }
    }

    /// `diagram_type` value of the JSON Graph IR: the canonical key, so JSON, PlantUML and
    /// Mermaid outputs of one view share one name.
    pub fn json_type(self) -> &'static str {
        self.key()
    }

    /// Parses a diagram type name: the canonical key or one of the accepted aliases
    /// (`state`, `use_case`, `cg`, `composite_structure`, `interaction_overview`, ...).
    /// Separators (`_`, `-`, blanks) and case are ignored. `None` for unknown names.
    pub fn parse(name: &str) -> Option<Self> {
        let normalized: String = name
            .trim()
            .chars()
            .filter(|c| !matches!(c, '_' | '-' | ' '))
            .flat_map(|c| c.to_lowercase())
            .collect();
        let kind = match normalized.as_str() {
            "class" => Self::Class,
            "object" => Self::Object,
            "package" => Self::Package,
            "component" => Self::Component,
            "composite" | "compositestructure" => Self::CompositeStructure,
            "deployment" => Self::Deployment,
            "profile" => Self::Profile,
            "usecase" => Self::UseCase,
            "activity" => Self::Activity,
            "statemachine" | "state" => Self::StateMachine,
            "sequence" => Self::Sequence,
            "communication" => Self::Communication,
            "interaction" | "interactionoverview" => Self::InteractionOverview,
            "timing" => Self::Timing,
            "cfg" | "controlflow" | "controlflowgraph" => Self::Cfg,
            "dfg" | "dataflow" | "dataflowgraph" => Self::Dfg,
            "cdg" | "controldependence" | "controldependencegraph" => Self::Cdg,
            "callgraph" | "cg" => Self::CallGraph,
            "robdd" | "bdd" => Self::Robdd,
            _ => return None,
        };
        Some(kind)
    }

    /// Canonical keys of every view kind, in `ALL` order.
    pub fn keys() -> Vec<&'static str> {
        Self::ALL.iter().map(|k| k.key()).collect()
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Class => "UML 2.5 Class Diagram",
            Self::Object => "UML 2.5 Object Diagram (allocation sites)",
            Self::Package => "UML 2.5 Package Diagram",
            Self::Component => "UML 2.5 Component Diagram",
            Self::CompositeStructure => "UML 2.5 Composite Structure Diagram",
            Self::Deployment => "UML 2.5 Deployment Diagram",
            Self::Profile => "UML 2.5 Profile Diagram",
            Self::UseCase => "UML 2.5 Use Case Diagram",
            Self::Activity => "UML 2.5 Activity Diagram",
            Self::StateMachine => "UML 2.5 State Machine Diagram",
            Self::Sequence => "UML 2.5 Sequence Diagram",
            Self::Communication => "UML 2.5 Communication Diagram",
            Self::InteractionOverview => "UML 2.5 Interaction Overview Diagram",
            Self::Timing => "UML 2.5 Timing Diagram",
            Self::Cfg => "Control Flow Graph & Dominator Tree",
            Self::Dfg => "SSA Def-Use Data Flow Graph",
            Self::Cdg => "Control Dependence Graph",
            Self::CallGraph => "Call Graph & Recursive SCCs",
            Self::Robdd => "Reduced Ordered BDD Path Summaries",
        }
    }

    /// True for views whose elements are independent enough to be pruned by degree when the
    /// node budget is exceeded. Per-function graphs are budgeted function by function instead.
    pub fn allows_degree_pruning(self) -> bool {
        matches!(
            self,
            Self::Class
                | Self::Object
                | Self::Package
                | Self::Component
                | Self::CompositeStructure
                | Self::Profile
                | Self::CallGraph
                | Self::Communication
        )
    }
}

/// Node kinds of the view IR.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NodeKind {
    Package,
    /// Generic container: a function scope, a recursive SCC or an object group.
    Group,
    Class,
    AbstractClass,
    Interface,
    Enum,
    Record,
    Annotation,
    /// Value-semantics classifier (struct, union, primitive or other data type).
    DataType,
    Object,
    Component,
    ProvidedInterface,
    /// Structured classifier of a composite structure diagram (container of parts).
    StructuredClass,
    Part,
    Stereotype,
    Metaclass,
    /// Deployable artifact (build output, archive, executable).
    Artifact,
    /// Execution environment or device of a deployment (container of artifacts).
    ExecutionNode,
    Actor,
    UseCase,
    /// System boundary of a use case diagram (container of use cases).
    Subject,
    Lifeline,
    /// Diagram gate: source of found messages and target of lost messages.
    Gate,
    InteractionUse,
    Initial,
    Final,
    /// Flow final node: ends one flow, not the whole activity.
    FlowFinal,
    Action,
    Decision,
    Merge,
    Fork,
    Join,
    ExceptionHandler,
    State,
    BasicBlock,
    Statement,
    SsaValue,
    SsaPhi,
    Function,
    BddVariable,
    BddTrue,
    BddFalse,
}

impl NodeKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Package => "package",
            Self::Group => "group",
            Self::Class => "class",
            Self::AbstractClass => "abstract",
            Self::Interface => "interface",
            Self::Enum => "enum",
            Self::Record => "record",
            Self::Annotation => "annotation",
            Self::DataType => "datatype",
            Self::Object => "object",
            Self::Component => "component",
            Self::ProvidedInterface => "provided_interface",
            Self::StructuredClass => "structured_class",
            Self::Part => "part",
            Self::Stereotype => "stereotype",
            Self::Metaclass => "metaclass",
            Self::Artifact => "artifact",
            Self::ExecutionNode => "execution_node",
            Self::Actor => "actor",
            Self::UseCase => "use_case",
            Self::Subject => "subject",
            Self::Lifeline => "lifeline",
            Self::Gate => "gate",
            Self::InteractionUse => "interaction_use",
            Self::Initial => "initial",
            Self::Final => "final",
            Self::FlowFinal => "flow_final",
            Self::Action => "action",
            Self::Decision => "decision",
            Self::Merge => "merge",
            Self::Fork => "fork",
            Self::Join => "join",
            Self::ExceptionHandler => "exception_handler",
            Self::State => "state",
            Self::BasicBlock => "basic_block",
            Self::Statement => "statement",
            Self::SsaValue => "ssa_value",
            Self::SsaPhi => "ssa_phi",
            Self::Function => "function",
            Self::BddVariable => "bdd_variable",
            Self::BddTrue => "bdd_true",
            Self::BddFalse => "bdd_false",
        }
    }

    /// Kinds that group other nodes.
    pub fn is_container(&self) -> bool {
        matches!(
            self,
            Self::Package
                | Self::Group
                | Self::StructuredClass
                | Self::ExecutionNode
                | Self::Subject
        )
    }

    /// Classifier kinds printed with a member compartment.
    pub fn is_classifier(&self) -> bool {
        matches!(
            self,
            Self::Class
                | Self::AbstractClass
                | Self::Interface
                | Self::Enum
                | Self::Record
                | Self::Annotation
                | Self::DataType
                | Self::Stereotype
                | Self::Metaclass
                | Self::Object
        )
    }
}

/// Edge kinds of the view IR.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EdgeKind {
    Generalization,
    Realization,
    Association,
    Aggregation,
    Composition,
    Dependency,
    /// Nested classifier owned by its enclosing classifier (UML containment, `+--`).
    Nesting,
    Import,
    Provides,
    Requires,
    Connector,
    Extension,
    /// Artifact manifesting a model element (deployment).
    Manifestation,
    /// Communication path between execution nodes (deployment).
    CommunicationPath,
    /// Use case inclusion.
    Include,
    Link,
    Message,
    ControlFlow,
    ExceptionFlow,
    Transition,
    DominatorTree,
    DataUse,
    DataDef,
    PhiArgument,
    ControlDependence,
    Call,
    BddLow,
    BddHigh,
}

impl EdgeKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Generalization => "generalization",
            Self::Realization => "realization",
            Self::Association => "association",
            Self::Aggregation => "aggregation",
            Self::Composition => "composition",
            Self::Dependency => "dependency",
            Self::Nesting => "containment",
            Self::Import => "import",
            Self::Provides => "provides",
            Self::Requires => "requires",
            Self::Connector => "connector",
            Self::Extension => "extension",
            Self::Manifestation => "manifestation",
            Self::CommunicationPath => "communication_path",
            Self::Include => "include",
            Self::Link => "link",
            Self::Message => "message",
            Self::ControlFlow => "control_flow",
            Self::ExceptionFlow => "exception_flow",
            Self::Transition => "transition",
            Self::DominatorTree => "dominator_tree",
            Self::DataUse => "data_use",
            Self::DataDef => "data_def",
            Self::PhiArgument => "phi_argument",
            Self::ControlDependence => "control_dependence",
            Self::Call => "call",
            Self::BddLow => "bdd_low",
            Self::BddHigh => "bdd_high",
        }
    }

    /// Edges drawn with a dashed line in every format.
    pub fn is_dashed(&self) -> bool {
        matches!(
            self,
            Self::Realization
                | Self::Dependency
                | Self::Import
                | Self::Requires
                | Self::Manifestation
                | Self::Include
                | Self::DominatorTree
                | Self::ExceptionFlow
                | Self::PhiArgument
                | Self::BddLow
        )
    }

    /// Edges without an arrowhead.
    pub fn is_undirected(&self) -> bool {
        matches!(
            self,
            Self::Link | Self::Connector | Self::Provides | Self::CommunicationPath
        )
    }

    /// Budget priority: lower values survive edge pruning first.
    pub fn priority(&self) -> u8 {
        match self {
            Self::Generalization | Self::Realization | Self::Extension | Self::Nesting => 0,
            Self::ControlFlow
            | Self::ExceptionFlow
            | Self::Transition
            | Self::Message
            | Self::BddLow
            | Self::BddHigh => 1,
            Self::Composition
            | Self::Aggregation
            | Self::Association
            | Self::Connector
            | Self::CommunicationPath => 2,
            Self::Call
            | Self::ControlDependence
            | Self::DataUse
            | Self::DataDef
            | Self::Include => 3,
            Self::PhiArgument
            | Self::Provides
            | Self::Requires
            | Self::Link
            | Self::Manifestation => 4,
            Self::Import | Self::Dependency => 5,
            Self::DominatorTree => 6,
        }
    }
}

/// A parameter of an operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParamView {
    pub name: Option<String>,
    pub type_name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MemberKind {
    Field,
    Literal,
    Method,
    Constructor,
}

/// An attribute, enumeration literal or operation of a classifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberView {
    pub kind: MemberKind,
    pub visibility: &'static str,
    pub name: String,
    /// Attribute type or operation return type (`None` when void, absent or unresolved).
    pub type_name: Option<String>,
    /// Parameters in declaration order; `None` when the artifacts do not record them.
    pub params: Option<Vec<ParamView>>,
    pub param_count: u16,
    pub is_static: bool,
    pub is_abstract: bool,
    pub is_final: bool,
    /// UML multiplicity of an attribute (`0..*`, `1`, ...), `None` when not derivable.
    pub multiplicity: Option<String>,
    /// True when the declared attribute type is itself a collection or array; its element
    /// multiplicity is then carried by the association end, not by the attribute text.
    pub is_collection: bool,
    /// Declaring symbol (`None` for members that come from a model without symbol ids).
    pub sym_id: Option<u32>,
    pub span: Option<SourceSpan>,
}

impl MemberView {
    pub fn is_operation(&self) -> bool {
        matches!(self.kind, MemberKind::Method | MemberKind::Constructor)
    }

    /// Parameter list text: `a : int, b : String`, or `3 params` when only the arity is known.
    pub fn param_list(&self) -> String {
        match &self.params {
            Some(params) => params
                .iter()
                .map(|p| match (&p.name, &p.type_name) {
                    (Some(n), Some(t)) => format!("{} : {}", n, t),
                    (Some(n), None) => n.clone(),
                    (None, Some(t)) => format!(": {}", t),
                    (None, None) => "?".to_string(),
                })
                .collect::<Vec<_>>()
                .join(", "),
            None if self.param_count == 0 => String::new(),
            None if self.param_count == 1 => "1 param".to_string(),
            None => format!("{} params", self.param_count),
        }
    }

    /// UML notation without visibility: `name(a : T) : R`, `name : T [0..*]`, `LITERAL`.
    pub fn signature(&self) -> String {
        match self.kind {
            MemberKind::Literal => self.name.clone(),
            MemberKind::Field => {
                let mut text = self.name.clone();
                if let Some(t) = &self.type_name {
                    text.push_str(" : ");
                    text.push_str(t);
                }
                if let (Some(m), false) = (&self.multiplicity, self.is_collection) {
                    text.push_str(&format!(" [{}]", m));
                }
                text
            }
            MemberKind::Method | MemberKind::Constructor => {
                let mut text = format!("{}({})", self.name, self.param_list());
                if let Some(t) = &self.type_name {
                    text.push_str(" : ");
                    text.push_str(t);
                }
                text
            }
        }
    }
}

/// A node of a view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewNode {
    /// Globally unique stable key (qualified name, `fqn#sym` on collision, or structural id).
    pub key: String,
    /// Identifier-safe alias, unique within the view, used by the text formats.
    pub alias: String,
    pub label: String,
    pub qualified_name: Option<String>,
    pub kind: NodeKind,
    pub stereotypes: Vec<String>,
    /// Key of the containing node.
    pub parent: Option<String>,
    pub span: Option<SourceSpan>,
    pub members: Vec<MemberView>,
    /// Additional text lines (statements, metrics, ...).
    pub details: Vec<String>,
    pub provenance: Provenance,
    pub sym_id: Option<u32>,
}

impl ViewNode {
    pub fn new(
        key: impl Into<String>,
        alias: impl Into<String>,
        label: impl Into<String>,
        kind: NodeKind,
    ) -> Self {
        Self {
            key: key.into(),
            alias: alias.into(),
            label: label.into(),
            qualified_name: None,
            kind,
            stereotypes: Vec::new(),
            parent: None,
            span: None,
            members: Vec::new(),
            details: Vec::new(),
            provenance: Provenance::Derived,
            sym_id: None,
        }
    }

    pub fn with_parent(mut self, parent: Option<String>) -> Self {
        self.parent = parent;
        self
    }

    pub fn with_span(mut self, span: Option<SourceSpan>) -> Self {
        self.span = span;
        self
    }

    pub fn with_qualified_name(mut self, qualified_name: Option<String>) -> Self {
        self.qualified_name = qualified_name;
        self
    }

    pub fn with_stereotype(mut self, stereotype: impl Into<String>) -> Self {
        let s = stereotype.into();
        if !s.is_empty() && !self.stereotypes.contains(&s) {
            self.stereotypes.push(s);
        }
        self
    }

    pub fn with_details(mut self, details: Vec<String>) -> Self {
        self.details = details;
        self
    }

    pub fn with_provenance(mut self, provenance: Provenance) -> Self {
        self.provenance = provenance;
        self
    }

    pub fn with_sym(mut self, sym_id: u32) -> Self {
        self.sym_id = Some(sym_id);
        self
    }
}

/// An edge of a view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewEdge {
    /// Stable id: hash of (source, kind, target, discriminator).
    pub key: String,
    pub source: String,
    pub target: String,
    pub kind: EdgeKind,
    pub label: Option<String>,
    pub source_multiplicity: Option<String>,
    pub target_multiplicity: Option<String>,
    /// Role name at the target end (association ends).
    pub role: Option<String>,
    /// Message order within an interaction.
    pub order: Option<u32>,
    /// Key of the interaction (sequence scenario) a message belongs to.
    pub scenario: Option<String>,
    /// Tagged values that have no dedicated field (`key`, `value`).
    pub properties: Vec<(String, String)>,
    pub span: Option<SourceSpan>,
    pub provenance: Provenance,
}

impl ViewEdge {
    /// `discriminator` separates parallel edges of the same kind between the same nodes.
    pub fn new(source: &str, target: &str, kind: EdgeKind, discriminator: &str) -> Self {
        Self {
            key: format!(
                "e_{}",
                DiagramUtils::short_hash(&[source, kind.as_str(), target, discriminator])
            ),
            source: source.to_string(),
            target: target.to_string(),
            kind,
            label: None,
            source_multiplicity: None,
            target_multiplicity: None,
            role: None,
            order: None,
            scenario: None,
            properties: Vec::new(),
            span: None,
            provenance: Provenance::Derived,
        }
    }

    pub fn with_label(mut self, label: Option<String>) -> Self {
        self.label = label.filter(|l| !l.is_empty());
        self
    }

    pub fn with_scenario(mut self, scenario: impl Into<String>) -> Self {
        self.scenario = Some(scenario.into());
        self
    }

    pub fn with_property(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.properties.push((key.into(), value.into()));
        self
    }

    pub fn with_role(mut self, role: Option<String>) -> Self {
        self.role = role.filter(|r| !r.is_empty());
        self
    }

    pub fn with_multiplicity(mut self, source: Option<String>, target: Option<String>) -> Self {
        self.source_multiplicity = source;
        self.target_multiplicity = target;
        self
    }

    pub fn with_order(mut self, order: u32) -> Self {
        self.order = Some(order);
        self
    }

    pub fn with_span(mut self, span: Option<SourceSpan>) -> Self {
        self.span = span;
        self
    }

    pub fn with_provenance(mut self, provenance: Provenance) -> Self {
        self.provenance = provenance;
        self
    }
}

/// Combined fragment operators of an interaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FragmentKind {
    Alt,
    Opt,
    Loop,
    Par,
    Break,
    Critical,
}

impl FragmentKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Alt => "alt",
            Self::Opt => "opt",
            Self::Loop => "loop",
            Self::Par => "par",
            Self::Break => "break",
            Self::Critical => "critical",
        }
    }
}

/// One ordered event of an interaction (sequence scenario).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InteractionEvent {
    /// A call message. `activates` is false for asynchronous and found messages whose
    /// receiver activation is not modelled.
    Call {
        from: String,
        to: String,
        label: String,
        number: String,
        is_async: bool,
        is_create: bool,
        activates: bool,
        span: Option<SourceSpan>,
    },
    /// A reply message; `deactivates` closes the sender's current activation.
    Reply {
        from: String,
        to: String,
        label: String,
        deactivates: bool,
    },
    /// Closes an activation that has no reply message (end of a truncated scenario).
    Deactivate {
        lifeline: String,
    },
    FragmentStart {
        kind: FragmentKind,
        guard: Option<String>,
    },
    /// Starts the next operand of the innermost open fragment (`else` of an `alt`, `and`
    /// of a `par`).
    FragmentElse {
        guard: Option<String>,
    },
    FragmentEnd,
    Note {
        over: String,
        text: String,
    },
}

/// An ordered interaction: one sequence diagram page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Interaction {
    pub key: String,
    pub title: String,
    /// Lifeline node keys in order of first appearance.
    pub lifelines: Vec<String>,
    pub events: Vec<InteractionEvent>,
    /// Messages dropped by the message budget or by the extractor's own bounds.
    pub omitted_messages: usize,
    pub truncated: bool,
}

/// Generates identifier-safe aliases that are unique within one view.
#[derive(Debug, Default, Clone)]
pub struct AliasTable {
    by_alias: HashMap<String, String>,
    by_key: HashMap<String, String>,
}

impl AliasTable {
    /// Alias for `key`, derived from `readable` and made unique with a hash suffix on
    /// collision. Repeated calls with the same key return the same alias.
    pub fn alias(&mut self, prefix: &str, readable: &str, key: &str) -> String {
        if let Some(existing) = self.by_key.get(key) {
            return existing.clone();
        }
        let mut fragment = DiagramUtils::sanitize(readable);
        if fragment.chars().count() > 72 {
            fragment = fragment.chars().take(72).collect();
        }
        let base = format!("{}{}", prefix, fragment);
        let mut candidate = base.clone();
        if self.by_alias.contains_key(&candidate) {
            let suffix = DiagramUtils::short_hash(&[key]);
            candidate = format!("{}_{}", base, &suffix[..8]);
            let mut n = 2;
            while self.by_alias.contains_key(&candidate) {
                candidate = format!("{}_{}_{}", base, &suffix[..8], n);
                n += 1;
            }
        }
        self.by_alias.insert(candidate.clone(), key.to_string());
        self.by_key.insert(key.to_string(), candidate.clone());
        candidate
    }
}

/// A complete diagram projection.
#[derive(Debug, Clone)]
pub struct ViewModel {
    pub kind: ViewKind,
    pub title: String,
    pub provenance: Provenance,
    /// Explanations shown with the diagram (provenance, missing inputs, unavailability).
    pub notes: Vec<String>,
    /// Elements left out by budgets or focus, named by qualified name.
    pub omitted: Vec<String>,
    pub nodes: Vec<ViewNode>,
    pub edges: Vec<ViewEdge>,
    pub interactions: Vec<Interaction>,
    /// Artifacts the view was derived from.
    pub sources: Vec<String>,
    node_index: HashMap<String, usize>,
    edge_keys: HashSet<String>,
    omitted_overflow: usize,
}

impl ViewModel {
    pub fn new(kind: ViewKind) -> Self {
        Self {
            kind,
            title: kind.title().to_string(),
            provenance: Provenance::Derived,
            notes: Vec::new(),
            omitted: Vec::new(),
            nodes: Vec::new(),
            edges: Vec::new(),
            interactions: Vec::new(),
            sources: Vec::new(),
            node_index: HashMap::new(),
            edge_keys: HashSet::new(),
            omitted_overflow: 0,
        }
    }

    /// An explicit "not derivable" projection with the reason as its note.
    pub fn unavailable(kind: ViewKind, reason: impl Into<String>) -> Self {
        let mut model = Self::new(kind);
        model.provenance = Provenance::Unavailable;
        model.notes.push(reason.into());
        model
    }

    pub fn is_unavailable(&self) -> bool {
        self.provenance == Provenance::Unavailable
    }

    pub fn note(&mut self, text: impl Into<String>) {
        let t = text.into();
        if !self.notes.contains(&t) {
            self.notes.push(t);
        }
    }

    pub fn source(&mut self, artifact: &str) {
        if !self.sources.iter().any(|s| s == artifact) {
            self.sources.push(artifact.to_string());
        }
    }

    pub fn omit(&mut self, name: impl Into<String>) {
        if self.omitted.len() < MAX_OMITTED_NAMES {
            self.omitted.push(name.into());
        } else {
            self.omitted_overflow += 1;
        }
    }

    /// Number of omitted elements, including those not listed by name.
    pub fn omitted_count(&self) -> usize {
        self.omitted.len() + self.omitted_overflow
    }

    /// Adds a node; returns false (and keeps the first) when the key already exists.
    pub fn add_node(&mut self, node: ViewNode) -> bool {
        if self.node_index.contains_key(&node.key) {
            return false;
        }
        self.node_index.insert(node.key.clone(), self.nodes.len());
        self.nodes.push(node);
        true
    }

    pub fn has_node(&self, key: &str) -> bool {
        self.node_index.contains_key(key)
    }

    pub fn node(&self, key: &str) -> Option<&ViewNode> {
        self.node_index.get(key).and_then(|&i| self.nodes.get(i))
    }

    pub fn node_mut(&mut self, key: &str) -> Option<&mut ViewNode> {
        match self.node_index.get(key) {
            Some(&i) => self.nodes.get_mut(i),
            None => None,
        }
    }

    /// Adds an edge; returns false when an edge with the same key already exists.
    pub fn add_edge(&mut self, edge: ViewEdge) -> bool {
        if !self.edge_keys.insert(edge.key.clone()) {
            return false;
        }
        self.edges.push(edge);
        true
    }

    /// Keeps only `seeds`, their direct neighbours and the containers of kept nodes.
    pub fn retain_neighborhood(&mut self, seeds: &HashSet<String>) {
        let mut keep: HashSet<String> = seeds.clone();
        for edge in &self.edges {
            if seeds.contains(&edge.source) {
                keep.insert(edge.target.clone());
            }
            if seeds.contains(&edge.target) {
                keep.insert(edge.source.clone());
            }
        }
        self.keep_with_ancestors(keep);
    }

    fn keep_with_ancestors(&mut self, mut keep: HashSet<String>) {
        let parents: HashMap<String, Option<String>> = self
            .nodes
            .iter()
            .map(|n| (n.key.clone(), n.parent.clone()))
            .collect();
        let leaves: Vec<String> = keep.iter().cloned().collect();
        for leaf in leaves {
            let mut curr = parents.get(&leaf).cloned().flatten();
            let mut guard = 0;
            while let Some(p) = curr {
                if !keep.insert(p.clone()) || guard > 64 {
                    break;
                }
                guard += 1;
                curr = parents.get(&p).cloned().flatten();
            }
        }
        let removed: Vec<String> = self
            .nodes
            .iter()
            .filter(|n| !keep.contains(&n.key) && !n.kind.is_container())
            .map(|n| n.qualified_name.clone().unwrap_or_else(|| n.label.clone()))
            .collect();
        for name in removed {
            self.omit(name);
        }
        self.nodes.retain(|n| keep.contains(&n.key));
        self.edges
            .retain(|e| keep.contains(&e.source) && keep.contains(&e.target));
        self.reindex();
    }

    fn reindex(&mut self) {
        self.node_index = self
            .nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.key.clone(), i))
            .collect();
        self.edge_keys = self.edges.iter().map(|e| e.key.clone()).collect();
    }

    fn degrees(&self) -> HashMap<String, usize> {
        let mut degree: HashMap<String, usize> = HashMap::new();
        for edge in &self.edges {
            *degree.entry(edge.source.clone()).or_insert(0) += 1;
            *degree.entry(edge.target.clone()).or_insert(0) += 1;
        }
        degree
    }

    fn apply_budget(&mut self, options: &ViewOptions) {
        let leaf_count = self.nodes.iter().filter(|n| !n.kind.is_container()).count();
        if leaf_count > options.max_nodes {
            let degree = self.degrees();
            let mut ranked: Vec<(usize, String)> = self
                .nodes
                .iter()
                .filter(|n| !n.kind.is_container())
                .map(|n| (degree.get(&n.key).copied().unwrap_or(0), n.key.clone()))
                .collect();
            ranked.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
            let keep: HashSet<String> = ranked
                .into_iter()
                .take(options.max_nodes)
                .map(|(_, k)| k)
                .collect();
            self.note(format!(
                "{} of {} elements omitted by the node budget (max {}); kept elements are ranked by degree.",
                leaf_count.saturating_sub(options.max_nodes),
                leaf_count,
                options.max_nodes
            ));
            self.keep_with_ancestors(keep);
            // Drop containers left without any kept descendant.
            let used_parents: HashSet<String> =
                self.nodes.iter().filter_map(|n| n.parent.clone()).collect();
            let edge_endpoints: HashSet<String> = self
                .edges
                .iter()
                .flat_map(|e| [e.source.clone(), e.target.clone()])
                .collect();
            self.nodes.retain(|n| {
                !n.kind.is_container()
                    || used_parents.contains(&n.key)
                    || edge_endpoints.contains(&n.key)
            });
            self.reindex();
        }
        if self.edges.len() > options.max_edges {
            let total = self.edges.len();
            self.edges.sort_by(|a, b| {
                a.kind
                    .priority()
                    .cmp(&b.kind.priority())
                    .then_with(|| a.key.cmp(&b.key))
            });
            self.edges.truncate(options.max_edges);
            self.note(format!(
                "{} of {} relationships omitted by the edge budget (max {}); structural relationships are kept first.",
                total - options.max_edges,
                total,
                options.max_edges
            ));
            self.reindex();
        }
    }

    /// Drops dangling edges and parents, applies budgets and sorts deterministically.
    pub fn finalize(&mut self, options: &ViewOptions, prune: bool) {
        let known: HashSet<String> = self.nodes.iter().map(|n| n.key.clone()).collect();
        let before = self.edges.len();
        self.edges
            .retain(|e| known.contains(&e.source) && known.contains(&e.target));
        let dropped = before - self.edges.len();
        if dropped > 0 {
            self.note(format!(
                "{} relationship(s) dropped because an endpoint is not part of this view.",
                dropped
            ));
        }
        for node in &mut self.nodes {
            let dangling = match &node.parent {
                Some(p) => !known.contains(p) || *p == node.key,
                None => false,
            };
            if dangling {
                node.parent = None;
            }
        }
        if prune {
            self.apply_budget(options);
        }
        self.nodes.sort_by(|a, b| a.key.cmp(&b.key));
        self.edges.sort_by(|a, b| {
            a.order
                .cmp(&b.order)
                .then_with(|| a.source.cmp(&b.source))
                .then_with(|| a.target.cmp(&b.target))
                .then_with(|| a.kind.cmp(&b.kind))
                .then_with(|| a.key.cmp(&b.key))
        });
        self.reindex();
        if self.omitted_overflow > 0 {
            let overflow = self.omitted_overflow;
            self.note(format!(
                "{} further omitted element(s) are not listed.",
                overflow
            ));
        }
    }

    /// Structural checks: unique keys and aliases, resolvable edge endpoints and parents,
    /// acyclic containment. Returns the failed checks (empty when the view is consistent).
    pub fn validate(&self) -> Vec<String> {
        let mut failures = Vec::new();
        let mut keys = HashSet::new();
        let mut aliases = HashSet::new();
        for node in &self.nodes {
            if !keys.insert(node.key.as_str()) {
                failures.push(format!("duplicate node id '{}'", node.key));
            }
            if !aliases.insert(node.alias.as_str()) {
                failures.push(format!("duplicate node alias '{}'", node.alias));
            }
        }
        let mut edge_keys = HashSet::new();
        for edge in &self.edges {
            if !edge_keys.insert(edge.key.as_str()) {
                failures.push(format!("duplicate edge id '{}'", edge.key));
            }
            if !keys.contains(edge.source.as_str()) {
                failures.push(format!(
                    "edge '{}' source '{}' does not resolve",
                    edge.key, edge.source
                ));
            }
            if !keys.contains(edge.target.as_str()) {
                failures.push(format!(
                    "edge '{}' target '{}' does not resolve",
                    edge.key, edge.target
                ));
            }
        }
        let parents: HashMap<&str, Option<&str>> = self
            .nodes
            .iter()
            .map(|n| (n.key.as_str(), n.parent.as_deref()))
            .collect();
        for node in &self.nodes {
            if let Some(p) = node.parent.as_deref() {
                if !keys.contains(p) {
                    failures.push(format!(
                        "node '{}' parent '{}' does not resolve",
                        node.key, p
                    ));
                    continue;
                }
            }
            let mut curr = node.parent.as_deref();
            let mut steps = 0;
            while let Some(p) = curr {
                steps += 1;
                if p == node.key || steps > self.nodes.len() {
                    failures.push(format!(
                        "node '{}' is part of a containment cycle",
                        node.key
                    ));
                    break;
                }
                curr = parents.get(p).copied().flatten();
            }
        }
        for interaction in &self.interactions {
            for lifeline in &interaction.lifelines {
                if !keys.contains(lifeline.as_str()) {
                    failures.push(format!(
                        "interaction '{}' lifeline '{}' does not resolve",
                        interaction.key, lifeline
                    ));
                }
            }
        }
        failures
    }

    /// Children of every container, in node order (`None` key = top level).
    pub fn children_by_parent(&self) -> BTreeMap<Option<String>, Vec<usize>> {
        let mut map: BTreeMap<Option<String>, Vec<usize>> = BTreeMap::new();
        for (idx, node) in self.nodes.iter().enumerate() {
            map.entry(node.parent.clone()).or_default().push(idx);
        }
        map
    }

    /// Chain of container keys from the top level down to (excluding) `key`.
    pub fn ancestors(&self, key: &str) -> Vec<String> {
        let mut chain = Vec::new();
        let mut curr = self.node(key).and_then(|n| n.parent.clone());
        while let Some(p) = curr {
            if chain.contains(&p) || chain.len() > 64 {
                break;
            }
            chain.push(p.clone());
            curr = self.node(&p).and_then(|n| n.parent.clone());
        }
        chain.reverse();
        chain
    }

    /// Containment depth of a node (0 = top level).
    pub fn depth(&self, key: &str) -> usize {
        self.ancestors(key).len()
    }
}
