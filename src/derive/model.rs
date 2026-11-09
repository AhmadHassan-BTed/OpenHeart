//! Neutral diagram model shared by every derivation (UML 2.5 views and compiler views).
//!
//! A `DiagramModel` holds nodes, edges, combined fragments and notes. Every element carries
//! a stable id hashed from symbols, IR ids and spans (never from its position in an output
//! list), the token spans it was derived from, the facts that justify it and a
//! `Derivation` tag stating how exactly it reflects the program. Printers only visit the
//! model; they add no semantics to it.

use std::collections::{HashMap, HashSet};
use std::fmt;

use crate::derive::provenance::{normalize_facts, normalize_spans, Fact};
use crate::derive::Visibility;
use crate::ingestion::interner::fnv1a_64;
use crate::ir::types::Span;

/// Every diagram kind the derivations produce.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DiagramKind {
    Class,
    Object,
    Package,
    Component,
    Deployment,
    UseCase,
    Sequence,
    Communication,
    InteractionOverview,
    Timing,
    Activity,
    StateMachine,
    Cfg,
    Dfg,
    Cdg,
    CallGraph,
    Robdd,
}

impl DiagramKind {
    pub const ALL: [DiagramKind; 17] = [
        DiagramKind::Class,
        DiagramKind::Object,
        DiagramKind::Package,
        DiagramKind::Component,
        DiagramKind::Deployment,
        DiagramKind::UseCase,
        DiagramKind::Sequence,
        DiagramKind::Communication,
        DiagramKind::InteractionOverview,
        DiagramKind::Timing,
        DiagramKind::Activity,
        DiagramKind::StateMachine,
        DiagramKind::Cfg,
        DiagramKind::Dfg,
        DiagramKind::Cdg,
        DiagramKind::CallGraph,
        DiagramKind::Robdd,
    ];

    pub fn key(self) -> &'static str {
        match self {
            DiagramKind::Class => "class",
            DiagramKind::Object => "object",
            DiagramKind::Package => "package",
            DiagramKind::Component => "component",
            DiagramKind::Deployment => "deployment",
            DiagramKind::UseCase => "usecase",
            DiagramKind::Sequence => "sequence",
            DiagramKind::Communication => "communication",
            DiagramKind::InteractionOverview => "interaction",
            DiagramKind::Timing => "timing",
            DiagramKind::Activity => "activity",
            DiagramKind::StateMachine => "statemachine",
            DiagramKind::Cfg => "cfg",
            DiagramKind::Dfg => "dfg",
            DiagramKind::Cdg => "cdg",
            DiagramKind::CallGraph => "callgraph",
            DiagramKind::Robdd => "robdd",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            DiagramKind::Class => "Class Diagram",
            DiagramKind::Object => "Object Diagram",
            DiagramKind::Package => "Package Diagram",
            DiagramKind::Component => "Component Diagram",
            DiagramKind::Deployment => "Deployment Diagram",
            DiagramKind::UseCase => "Use Case Diagram",
            DiagramKind::Sequence => "Sequence Diagram",
            DiagramKind::Communication => "Communication Diagram",
            DiagramKind::InteractionOverview => "Interaction Overview Diagram",
            DiagramKind::Timing => "Timing Diagram",
            DiagramKind::Activity => "Activity Diagram",
            DiagramKind::StateMachine => "State Machine Diagram",
            DiagramKind::Cfg => "Control Flow Graph",
            DiagramKind::Dfg => "Data Flow Graph",
            DiagramKind::Cdg => "Control Dependence Graph",
            DiagramKind::CallGraph => "Call Graph",
            DiagramKind::Robdd => "ROBDD Path Summary",
        }
    }
}

/// How exactly an element, or a whole view, reflects the analysed program.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub enum Derivation {
    /// Follows from the facts with no approximation.
    #[default]
    Exact,
    /// Sound may-information: the element can occur, more may be shown than happens.
    OverApproximate,
    /// Derived from incomplete facts or cut off by a limit; `reason` names which.
    Partial { reason: String },
    /// The inputs do not determine the element at all.
    NotDerivable { reason: String },
}

impl Derivation {
    pub fn partial(reason: impl Into<String>) -> Self {
        Derivation::Partial {
            reason: reason.into(),
        }
    }

    pub fn not_derivable(reason: impl Into<String>) -> Self {
        Derivation::NotDerivable {
            reason: reason.into(),
        }
    }

    /// Exact < OverApproximate < Partial < NotDerivable.
    pub fn rank(&self) -> u8 {
        match self {
            Derivation::Exact => 0,
            Derivation::OverApproximate => 1,
            Derivation::Partial { .. } => 2,
            Derivation::NotDerivable { .. } => 3,
        }
    }

    /// The weaker of two tags (the first one on a tie).
    pub fn weaker(self, other: &Derivation) -> Derivation {
        if other.rank() > self.rank() {
            other.clone()
        } else {
            self
        }
    }

    pub fn weaken(&mut self, other: &Derivation) {
        if other.rank() > self.rank() {
            *self = other.clone();
        }
    }

    #[inline]
    pub fn is_exact(&self) -> bool {
        matches!(self, Derivation::Exact)
    }

    pub fn reason(&self) -> Option<&str> {
        match self {
            Derivation::Partial { reason } | Derivation::NotDerivable { reason } => {
                Some(reason.as_str())
            }
            _ => None,
        }
    }

    pub fn mnemonic(&self) -> &'static str {
        match self {
            Derivation::Exact => "exact",
            Derivation::OverApproximate => "over-approximate",
            Derivation::Partial { .. } => "partial",
            Derivation::NotDerivable { .. } => "not-derivable",
        }
    }
}

/// Stable element id: a readable prefix and a 64-bit FNV-1a hash of the identifying parts.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ElementId(String);

impl ElementId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ElementId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Builds an `ElementId` from typed parts; parts are separated by the ASCII unit separator
/// so `["ab", "c"]` and `["a", "bc"]` hash differently.
#[derive(Debug, Clone)]
pub struct IdBuilder {
    prefix: &'static str,
    bytes: Vec<u8>,
}

impl IdBuilder {
    pub fn new(prefix: &'static str) -> Self {
        Self {
            prefix,
            bytes: Vec::with_capacity(64),
        }
    }

    pub fn text(mut self, part: &str) -> Self {
        self.bytes.push(0x1F);
        self.bytes.extend_from_slice(part.as_bytes());
        self
    }

    pub fn num(mut self, part: u64) -> Self {
        self.bytes.push(0x1E);
        self.bytes.extend_from_slice(&part.to_le_bytes());
        self
    }

    pub fn span(self, span: &Span) -> Self {
        self.num(span.file_id as u64)
            .num(span.first_token as u64)
            .num(span.last_token as u64)
    }

    pub fn opt_span(self, span: Option<&Span>) -> Self {
        match span {
            Some(s) => self.span(s),
            None => self.num(u64::MAX),
        }
    }

    pub fn finish(self) -> ElementId {
        ElementId(format!("{}_{:016x}", self.prefix, fnv1a_64(&self.bytes)))
    }
}

/// UML multiplicity `lower..upper`; `upper == None` is unbounded (`*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Multiplicity {
    pub lower: u64,
    pub upper: Option<u64>,
}

impl Multiplicity {
    pub const ONE: Multiplicity = Multiplicity {
        lower: 1,
        upper: Some(1),
    };
    pub const ZERO_OR_ONE: Multiplicity = Multiplicity {
        lower: 0,
        upper: Some(1),
    };
    pub const MANY: Multiplicity = Multiplicity {
        lower: 0,
        upper: None,
    };
    pub const ONE_OR_MORE: Multiplicity = Multiplicity {
        lower: 1,
        upper: None,
    };

    pub fn exactly(n: u64) -> Self {
        Multiplicity {
            lower: n,
            upper: Some(n),
        }
    }

    pub fn range(lower: u64, upper: Option<u64>) -> Self {
        Multiplicity { lower, upper }
    }

    /// Multiplicity of `self` groups of `other` elements each.
    pub fn times(self, other: Multiplicity) -> Multiplicity {
        let upper = match (self.upper, other.upper) {
            (Some(0), _) | (_, Some(0)) => Some(0),
            (Some(a), Some(b)) => Some(a.saturating_mul(b)),
            _ => None,
        };
        Multiplicity {
            lower: self.lower.saturating_mul(other.lower),
            upper,
        }
    }

    pub fn with_lower(self, lower: u64) -> Multiplicity {
        let lower = match self.upper {
            Some(u) => lower.min(u),
            None => lower,
        };
        Multiplicity {
            lower,
            upper: self.upper,
        }
    }
}

impl fmt::Display for Multiplicity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.lower, self.upper) {
            (l, Some(u)) if l == u => write!(f, "{}", l),
            (0, None) => f.write_str("*"),
            (l, None) => write!(f, "{}..*", l),
            (l, Some(u)) => write!(f, "{}..{}", l, u),
        }
    }
}

/// Node kinds of all views.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NodeKind {
    // Structure
    Package,
    Class,
    Interface,
    Enumeration,
    DataType,
    Annotation,
    /// Instance specification: an abstract object of the heap.
    Object,
    /// Static or global storage that roots object links.
    Root,
    Component,
    Artifact,
    ExecutionNode,
    Actor,
    UseCase,
    /// Subject (system boundary) of a use case diagram.
    Subject,
    // Interactions
    Lifeline,
    /// Diagram gate: source of found and target of lost messages.
    Gate,
    InteractionUse,
    // Activities
    Initial,
    ActivityFinal,
    FlowFinal,
    Action,
    Decision,
    Merge,
    Fork,
    Join,
    Partition,
    // State machines and timing
    State,
    InitialState,
    FinalState,
    StateTimeline,
    // Compiler views
    BasicBlock,
    Entry,
    Exit,
    Value,
    Phi,
    Effect,
    Function,
    Group,
    BddNode,
    BddTerminal,
}

impl NodeKind {
    pub fn mnemonic(self) -> &'static str {
        match self {
            NodeKind::Package => "package",
            NodeKind::Class => "class",
            NodeKind::Interface => "interface",
            NodeKind::Enumeration => "enumeration",
            NodeKind::DataType => "datatype",
            NodeKind::Annotation => "annotation",
            NodeKind::Object => "object",
            NodeKind::Root => "root",
            NodeKind::Component => "component",
            NodeKind::Artifact => "artifact",
            NodeKind::ExecutionNode => "node",
            NodeKind::Actor => "actor",
            NodeKind::UseCase => "usecase",
            NodeKind::Subject => "subject",
            NodeKind::Lifeline => "lifeline",
            NodeKind::Gate => "gate",
            NodeKind::InteractionUse => "interaction-use",
            NodeKind::Initial => "initial",
            NodeKind::ActivityFinal => "activity-final",
            NodeKind::FlowFinal => "flow-final",
            NodeKind::Action => "action",
            NodeKind::Decision => "decision",
            NodeKind::Merge => "merge",
            NodeKind::Fork => "fork",
            NodeKind::Join => "join",
            NodeKind::Partition => "partition",
            NodeKind::State => "state",
            NodeKind::InitialState => "initial-state",
            NodeKind::FinalState => "final-state",
            NodeKind::StateTimeline => "state-timeline",
            NodeKind::BasicBlock => "basic-block",
            NodeKind::Entry => "entry",
            NodeKind::Exit => "exit",
            NodeKind::Value => "value",
            NodeKind::Phi => "phi",
            NodeKind::Effect => "effect",
            NodeKind::Function => "function",
            NodeKind::Group => "group",
            NodeKind::BddNode => "bdd-node",
            NodeKind::BddTerminal => "bdd-terminal",
        }
    }

    /// Kinds that contain other nodes (`DiagramNode::parent` points at them).
    pub fn is_container(self) -> bool {
        matches!(
            self,
            NodeKind::Package
                | NodeKind::Subject
                | NodeKind::Partition
                | NodeKind::ExecutionNode
                | NodeKind::Group
                | NodeKind::Component
        )
    }
}

/// Edge kinds of all views.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EdgeKind {
    Generalization,
    Realization,
    Association,
    Aggregation,
    Composition,
    /// Usage dependency; the stereotype names the usage (`use`, `call`, `instantiate`).
    Dependency,
    /// Nested classifier to its owner.
    Nesting,
    Import,
    Provides,
    Requires,
    Manifestation,
    CommunicationPath,
    Link,
    Message,
    Include,
    ControlFlow,
    ExceptionFlow,
    Transition,
    Dominates,
    DataFlow,
    PhiArgument,
    ControlDependence,
    Call,
    BddLow,
    BddHigh,
}

impl EdgeKind {
    pub fn mnemonic(self) -> &'static str {
        match self {
            EdgeKind::Generalization => "generalization",
            EdgeKind::Realization => "realization",
            EdgeKind::Association => "association",
            EdgeKind::Aggregation => "aggregation",
            EdgeKind::Composition => "composition",
            EdgeKind::Dependency => "dependency",
            EdgeKind::Nesting => "nesting",
            EdgeKind::Import => "import",
            EdgeKind::Provides => "provides",
            EdgeKind::Requires => "requires",
            EdgeKind::Manifestation => "manifestation",
            EdgeKind::CommunicationPath => "communication-path",
            EdgeKind::Link => "link",
            EdgeKind::Message => "message",
            EdgeKind::Include => "include",
            EdgeKind::ControlFlow => "control-flow",
            EdgeKind::ExceptionFlow => "exception-flow",
            EdgeKind::Transition => "transition",
            EdgeKind::Dominates => "dominates",
            EdgeKind::DataFlow => "data-flow",
            EdgeKind::PhiArgument => "phi-argument",
            EdgeKind::ControlDependence => "control-dependence",
            EdgeKind::Call => "call",
            EdgeKind::BddLow => "bdd-low",
            EdgeKind::BddHigh => "bdd-high",
        }
    }
}

/// UML `MessageSort`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MessageSort {
    SynchCall,
    AsynchCall,
    Reply,
    CreateMessage,
    DeleteMessage,
    /// Sender unknown (the interaction starts from outside).
    Found,
    /// Receiver unknown (the callee is not derivable).
    Lost,
}

impl MessageSort {
    pub fn mnemonic(self) -> &'static str {
        match self {
            MessageSort::SynchCall => "synchCall",
            MessageSort::AsynchCall => "asynchCall",
            MessageSort::Reply => "reply",
            MessageSort::CreateMessage => "createMessage",
            MessageSort::DeleteMessage => "deleteMessage",
            MessageSort::Found => "found",
            MessageSort::Lost => "lost",
        }
    }
}

/// Combined fragment operators (UML `InteractionOperatorKind`) and activity regions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FragmentKind {
    Alt,
    Opt,
    Loop,
    Par,
    Break,
    Critical,
    /// Structured activity region (loop node of an activity diagram).
    Region,
}

impl FragmentKind {
    pub fn mnemonic(self) -> &'static str {
        match self {
            FragmentKind::Alt => "alt",
            FragmentKind::Opt => "opt",
            FragmentKind::Loop => "loop",
            FragmentKind::Par => "par",
            FragmentKind::Break => "break",
            FragmentKind::Critical => "critical",
            FragmentKind::Region => "region",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CompartmentKind {
    Attributes,
    Operations,
    Literals,
    /// Slot values of an instance specification.
    Slots,
    /// Owned elements of a package or component.
    Contents,
    /// Instructions of a basic block.
    Statements,
    /// Internal transitions and behaviours of a state.
    Internal,
    /// Interfaces a component provides or requires.
    Interfaces,
}

impl CompartmentKind {
    pub fn mnemonic(self) -> &'static str {
        match self {
            CompartmentKind::Attributes => "attributes",
            CompartmentKind::Operations => "operations",
            CompartmentKind::Literals => "literals",
            CompartmentKind::Slots => "slots",
            CompartmentKind::Contents => "contents",
            CompartmentKind::Statements => "statements",
            CompartmentKind::Internal => "internal",
            CompartmentKind::Interfaces => "interfaces",
        }
    }
}

/// A parameter of an operation entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryParam {
    pub name: Option<String>,
    pub type_text: Option<String>,
}

/// One line of a compartment: an attribute, operation, literal, slot or statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompartmentEntry {
    pub name: String,
    pub visibility: Option<Visibility>,
    pub type_text: Option<String>,
    /// Operation parameters; `None` for non-operations.
    pub params: Option<Vec<EntryParam>>,
    pub multiplicity: Option<Multiplicity>,
    /// Slot value, default value or statement detail.
    pub value: Option<String>,
    pub is_static: bool,
    pub is_abstract: bool,
    pub is_read_only: bool,
    pub symbol: Option<u32>,
    pub spans: Vec<Span>,
    pub derivation: Derivation,
}

impl CompartmentEntry {
    pub fn named(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            visibility: None,
            type_text: None,
            params: None,
            multiplicity: None,
            value: None,
            is_static: false,
            is_abstract: false,
            is_read_only: false,
            symbol: None,
            spans: Vec::new(),
            derivation: Derivation::Exact,
        }
    }

    /// UML notation: `+ name(a : T) : R`, `- name : T [0..*] = v`, `LITERAL`.
    pub fn text(&self) -> String {
        let mut out = String::new();
        if let Some(v) = self.visibility {
            let marker = v.marker();
            if !marker.is_empty() {
                out.push_str(marker);
                out.push(' ');
            }
        }
        out.push_str(&self.name);
        if let Some(params) = &self.params {
            let list: Vec<String> = params
                .iter()
                .map(|p| match (&p.name, &p.type_text) {
                    (Some(n), Some(t)) => format!("{} : {}", n, t),
                    (Some(n), None) => n.clone(),
                    (None, Some(t)) => t.clone(),
                    (None, None) => "?".to_string(),
                })
                .collect();
            out.push('(');
            out.push_str(&list.join(", "));
            out.push(')');
        }
        if let Some(t) = &self.type_text {
            out.push_str(" : ");
            out.push_str(t);
        }
        if let Some(m) = &self.multiplicity {
            out.push_str(&format!(" [{}]", m));
        }
        if let Some(v) = &self.value {
            out.push_str(" = ");
            out.push_str(v);
        }
        out
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Compartment {
    pub kind: CompartmentKind,
    pub entries: Vec<CompartmentEntry>,
}

/// One end of a binary relationship.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EdgeEnd {
    pub role: Option<String>,
    pub multiplicity: Option<Multiplicity>,
    pub navigable: bool,
    /// Qualifier of a qualified association (the key of a map-like container).
    pub qualifier: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagramNode {
    pub id: ElementId,
    pub kind: NodeKind,
    pub label: String,
    pub qualified_name: Option<String>,
    pub stereotypes: Vec<String>,
    /// Containing node (package, partition, subject, execution node, group).
    pub parent: Option<ElementId>,
    pub compartments: Vec<Compartment>,
    /// Tagged values: facts that have no dedicated field.
    pub properties: Vec<(String, String)>,
    pub symbol: Option<u32>,
    pub spans: Vec<Span>,
    pub facts: Vec<Fact>,
    pub derivation: Derivation,
}

impl DiagramNode {
    pub fn new(id: ElementId, kind: NodeKind, label: impl Into<String>) -> Self {
        Self {
            id,
            kind,
            label: label.into(),
            qualified_name: None,
            stereotypes: Vec::new(),
            parent: None,
            compartments: Vec::new(),
            properties: Vec::new(),
            symbol: None,
            spans: Vec::new(),
            facts: Vec::new(),
            derivation: Derivation::Exact,
        }
    }

    pub fn with_parent(mut self, parent: Option<ElementId>) -> Self {
        self.parent = parent;
        self
    }

    pub fn with_symbol(mut self, symbol: u32) -> Self {
        self.symbol = Some(symbol);
        self
    }

    pub fn with_qualified_name(mut self, name: impl Into<String>) -> Self {
        self.qualified_name = Some(name.into());
        self
    }

    pub fn with_stereotype(mut self, stereotype: impl Into<String>) -> Self {
        let s = stereotype.into();
        if !s.is_empty() && !self.stereotypes.contains(&s) {
            self.stereotypes.push(s);
        }
        self
    }

    pub fn with_property(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.properties.push((key.into(), value.into()));
        self
    }

    pub fn with_span(mut self, span: Option<Span>) -> Self {
        if let Some(s) = span {
            self.spans.push(s);
        }
        self
    }

    pub fn with_spans(mut self, spans: impl IntoIterator<Item = Span>) -> Self {
        self.spans.extend(spans);
        self
    }

    pub fn with_fact(mut self, fact: Fact) -> Self {
        self.facts.push(fact);
        self
    }

    pub fn with_derivation(mut self, derivation: Derivation) -> Self {
        self.derivation = derivation;
        self
    }

    pub fn with_compartment(mut self, kind: CompartmentKind, entries: Vec<CompartmentEntry>) -> Self {
        if !entries.is_empty() {
            self.compartments.push(Compartment { kind, entries });
        }
        self
    }

    pub fn property(&self, key: &str) -> Option<&str> {
        self.properties
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagramEdge {
    pub id: ElementId,
    pub kind: EdgeKind,
    pub source: ElementId,
    pub target: ElementId,
    pub label: Option<String>,
    pub stereotypes: Vec<String>,
    pub source_end: Option<EdgeEnd>,
    pub target_end: Option<EdgeEnd>,
    /// Guard of a control flow or transition, condition of a message.
    pub guard: Option<String>,
    /// Position in an ordered interaction.
    pub order: Option<u32>,
    /// Decimal sequence number of a communication message (`1.2.1`).
    pub sequence_number: Option<String>,
    pub message_sort: Option<MessageSort>,
    pub properties: Vec<(String, String)>,
    pub spans: Vec<Span>,
    pub facts: Vec<Fact>,
    pub derivation: Derivation,
}

impl DiagramEdge {
    pub fn new(id: ElementId, kind: EdgeKind, source: ElementId, target: ElementId) -> Self {
        Self {
            id,
            kind,
            source,
            target,
            label: None,
            stereotypes: Vec::new(),
            source_end: None,
            target_end: None,
            guard: None,
            order: None,
            sequence_number: None,
            message_sort: None,
            properties: Vec::new(),
            spans: Vec::new(),
            facts: Vec::new(),
            derivation: Derivation::Exact,
        }
    }

    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        let l = label.into();
        if !l.is_empty() {
            self.label = Some(l);
        }
        self
    }

    pub fn with_stereotype(mut self, stereotype: impl Into<String>) -> Self {
        let s = stereotype.into();
        if !s.is_empty() && !self.stereotypes.contains(&s) {
            self.stereotypes.push(s);
        }
        self
    }

    pub fn with_ends(mut self, source: Option<EdgeEnd>, target: Option<EdgeEnd>) -> Self {
        self.source_end = source;
        self.target_end = target;
        self
    }

    pub fn with_guard(mut self, guard: Option<String>) -> Self {
        self.guard = guard.filter(|g| !g.is_empty());
        self
    }

    pub fn with_order(mut self, order: u32) -> Self {
        self.order = Some(order);
        self
    }

    pub fn with_sequence_number(mut self, number: impl Into<String>) -> Self {
        self.sequence_number = Some(number.into());
        self
    }

    pub fn with_message_sort(mut self, sort: MessageSort) -> Self {
        self.message_sort = Some(sort);
        self
    }

    pub fn with_property(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.properties.push((key.into(), value.into()));
        self
    }

    pub fn with_span(mut self, span: Option<Span>) -> Self {
        if let Some(s) = span {
            self.spans.push(s);
        }
        self
    }

    pub fn with_spans(mut self, spans: impl IntoIterator<Item = Span>) -> Self {
        self.spans.extend(spans);
        self
    }

    pub fn with_fact(mut self, fact: Fact) -> Self {
        self.facts.push(fact);
        self
    }

    pub fn with_facts(mut self, facts: impl IntoIterator<Item = Fact>) -> Self {
        self.facts.extend(facts);
        self
    }

    pub fn with_derivation(mut self, derivation: Derivation) -> Self {
        self.derivation = derivation;
        self
    }
}

/// One operand of a combined fragment: its guard and the elements it covers in order
/// (message edges of an interaction, nodes of an activity region).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FragmentOperand {
    pub guard: Option<String>,
    pub members: Vec<ElementId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fragment {
    pub id: ElementId,
    pub kind: FragmentKind,
    /// Enclosing fragment.
    pub parent: Option<ElementId>,
    /// Lifelines (interactions) or nodes (activities) the fragment covers.
    pub covered: Vec<ElementId>,
    pub operands: Vec<FragmentOperand>,
    pub spans: Vec<Span>,
    pub facts: Vec<Fact>,
    pub derivation: Derivation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    pub id: ElementId,
    pub text: String,
    pub attached: Vec<ElementId>,
}

/// One diagram.
#[derive(Debug, Clone)]
pub struct DiagramModel {
    pub kind: DiagramKind,
    pub title: String,
    /// What the diagram is about (entry point, function, type), when it is not the program.
    pub scope: Option<String>,
    pub derivation: Derivation,
    pub nodes: Vec<DiagramNode>,
    pub edges: Vec<DiagramEdge>,
    pub fragments: Vec<Fragment>,
    pub notes: Vec<Note>,
    node_index: HashMap<ElementId, usize>,
    edge_index: HashMap<ElementId, usize>,
}

impl DiagramModel {
    pub fn new(kind: DiagramKind, title: impl Into<String>) -> Self {
        Self {
            kind,
            title: title.into(),
            scope: None,
            derivation: Derivation::Exact,
            nodes: Vec::new(),
            edges: Vec::new(),
            fragments: Vec::new(),
            notes: Vec::new(),
            node_index: HashMap::new(),
            edge_index: HashMap::new(),
        }
    }

    /// A view the inputs do not determine; the reason is its derivation and its note.
    pub fn not_derivable(kind: DiagramKind, title: impl Into<String>, reason: impl Into<String>) -> Self {
        let reason = reason.into();
        let mut model = Self::new(kind, title);
        model.note(reason.clone());
        model.derivation = Derivation::not_derivable(reason);
        model
    }

    /// Stands for the diagrams of a kind that a `max_diagrams` cut-off left out.
    pub fn omission(kind: DiagramKind, omitted: usize, limit: usize) -> Self {
        let reason = format!(
            "{} further {} diagram(s) not derived: the per-kind limit is {}",
            omitted,
            kind.key(),
            limit
        );
        let mut model = Self::new(kind, format!("{} (omitted)", kind.title()));
        model.note(reason.clone());
        model.derivation = Derivation::partial(reason);
        model
    }

    pub fn with_scope(mut self, scope: impl Into<String>) -> Self {
        self.scope = Some(scope.into());
        self
    }

    /// Adds a node, or merges it into the node with the same id (spans, facts, stereotypes
    /// and properties are united; the weaker derivation wins).
    pub fn add_node(&mut self, node: DiagramNode) -> ElementId {
        let id = node.id.clone();
        if let Some(&i) = self.node_index.get(&id) {
            let existing = &mut self.nodes[i];
            existing.spans.extend(node.spans);
            existing.facts.extend(node.facts);
            for s in node.stereotypes {
                if !existing.stereotypes.contains(&s) {
                    existing.stereotypes.push(s);
                }
            }
            for p in node.properties {
                if !existing.properties.contains(&p) {
                    existing.properties.push(p);
                }
            }
            if existing.compartments.is_empty() {
                existing.compartments = node.compartments;
            }
            if existing.parent.is_none() {
                existing.parent = node.parent;
            }
            existing.derivation.weaken(&node.derivation);
            return id;
        }
        self.node_index.insert(id.clone(), self.nodes.len());
        self.nodes.push(node);
        id
    }

    /// Adds an edge, or merges it into the edge with the same id.
    pub fn add_edge(&mut self, edge: DiagramEdge) -> ElementId {
        let id = edge.id.clone();
        if let Some(&i) = self.edge_index.get(&id) {
            let existing = &mut self.edges[i];
            existing.spans.extend(edge.spans);
            existing.facts.extend(edge.facts);
            for s in edge.stereotypes {
                if !existing.stereotypes.contains(&s) {
                    existing.stereotypes.push(s);
                }
            }
            for p in edge.properties {
                if !existing.properties.contains(&p) {
                    existing.properties.push(p);
                }
            }
            existing.derivation.weaken(&edge.derivation);
            return id;
        }
        self.edge_index.insert(id.clone(), self.edges.len());
        self.edges.push(edge);
        id
    }

    pub fn add_fragment(&mut self, fragment: Fragment) {
        if !self.fragments.iter().any(|f| f.id == fragment.id) {
            self.fragments.push(fragment);
        }
    }

    /// Adds a note to the diagram (once per text).
    pub fn note(&mut self, text: impl Into<String>) {
        self.note_on(text, Vec::new());
    }

    pub fn note_on(&mut self, text: impl Into<String>, attached: Vec<ElementId>) {
        let text = text.into();
        if text.is_empty() || self.notes.iter().any(|n| n.text == text && n.attached == attached) {
            return;
        }
        let mut builder = IdBuilder::new("note").text(&text);
        for a in &attached {
            builder = builder.text(a.as_str());
        }
        self.notes.push(Note {
            id: builder.finish(),
            text,
            attached,
        });
    }

    pub fn has_node(&self, id: &ElementId) -> bool {
        self.node_index.contains_key(id)
    }

    pub fn node(&self, id: &ElementId) -> Option<&DiagramNode> {
        self.node_index.get(id).map(|&i| &self.nodes[i])
    }

    pub fn node_mut(&mut self, id: &ElementId) -> Option<&mut DiagramNode> {
        match self.node_index.get(id) {
            Some(&i) => self.nodes.get_mut(i),
            None => None,
        }
    }

    pub fn has_edge(&self, id: &ElementId) -> bool {
        self.edge_index.contains_key(id)
    }

    pub fn edge_mut(&mut self, id: &ElementId) -> Option<&mut DiagramEdge> {
        match self.edge_index.get(id) {
            Some(&i) => self.edges.get_mut(i),
            None => None,
        }
    }

    /// Weakens the view's own derivation and records the reason as a note.
    pub fn weaken(&mut self, derivation: &Derivation) {
        if let Some(r) = derivation.reason() {
            self.note(r.to_string());
        }
        self.derivation.weaken(derivation);
    }

    pub fn is_derivable(&self) -> bool {
        !matches!(self.derivation, Derivation::NotDerivable { .. })
    }

    /// Closes the model: drops relationships whose endpoints are not part of it (with a
    /// note), clears unresolvable parents, normalizes spans and facts, and lowers the
    /// view's derivation to the weakest element derivation. An element that is not
    /// derivable makes the view `Partial`, not `NotDerivable`.
    pub fn finalize(&mut self, max_spans: usize) {
        let ids: HashSet<ElementId> = self.nodes.iter().map(|n| n.id.clone()).collect();
        let before = self.edges.len();
        self.edges
            .retain(|e| ids.contains(&e.source) && ids.contains(&e.target));
        let dropped = before - self.edges.len();
        if dropped > 0 {
            self.note(format!(
                "{} relationship(s) dropped: an endpoint is not an element of this view",
                dropped
            ));
        }
        for node in &mut self.nodes {
            if let Some(p) = &node.parent {
                if !ids.contains(p) || *p == node.id {
                    node.parent = None;
                }
            }
            normalize_spans(&mut node.spans, max_spans);
            normalize_facts(&mut node.facts, max_spans);
            for c in &mut node.compartments {
                for e in &mut c.entries {
                    normalize_spans(&mut e.spans, max_spans);
                }
            }
        }
        let edge_ids: HashSet<ElementId> = self.edges.iter().map(|e| e.id.clone()).collect();
        let fragment_ids: HashSet<ElementId> =
            self.fragments.iter().map(|f| f.id.clone()).collect();
        for edge in &mut self.edges {
            normalize_spans(&mut edge.spans, max_spans);
            normalize_facts(&mut edge.facts, max_spans);
        }
        for frag in &mut self.fragments {
            frag.covered.retain(|c| ids.contains(c));
            for op in &mut frag.operands {
                op.members
                    .retain(|m| ids.contains(m) || edge_ids.contains(m));
            }
            if let Some(p) = &frag.parent {
                if !fragment_ids.contains(p) {
                    frag.parent = None;
                }
            }
            normalize_spans(&mut frag.spans, max_spans);
            normalize_facts(&mut frag.facts, max_spans);
        }
        for note in &mut self.notes {
            note.attached
                .retain(|a| ids.contains(a) || edge_ids.contains(a));
        }

        let mut reasons: Vec<String> = Vec::new();
        let mut weakest = Derivation::Exact;
        let tags = self
            .nodes
            .iter()
            .map(|n| &n.derivation)
            .chain(self.edges.iter().map(|e| &e.derivation))
            .chain(self.fragments.iter().map(|f| &f.derivation));
        for tag in tags {
            let lowered = match tag {
                Derivation::NotDerivable { reason } => Derivation::partial(reason.clone()),
                other => other.clone(),
            };
            if let Some(r) = lowered.reason() {
                if !reasons.iter().any(|x| x == r) {
                    reasons.push(r.to_string());
                }
            }
            weakest.weaken(&lowered);
        }
        for r in reasons {
            self.note(r);
        }
        self.derivation.weaken(&weakest);

        self.node_index = self
            .nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.id.clone(), i))
            .collect();
        self.edge_index = self
            .edges
            .iter()
            .enumerate()
            .map(|(i, e)| (e.id.clone(), i))
            .collect();
    }
}
