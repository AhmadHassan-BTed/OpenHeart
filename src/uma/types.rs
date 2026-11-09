//! Core types for Phase 9: UML Semantic Metadata Extraction (§9.4, §9.5).
//!
//! All binary record structures for the UML diagram types, design pattern
//! instance records, label text tables, and `UMLMetadataArtifact (.uma)`.
//!
//! **Unknown markers.** Wherever a value cannot be derived from the prior
//! artifacts, records carry an explicit marker instead of a plausible default:
//! `UNKNOWN_SYM` for symbol references, `NO_LABEL` for label references,
//! `UML_LINK_UNKNOWN_FILE` / line 0 for source locations, and
//! `MULTIPLICITY_UNKNOWN` for multiplicities.

use crate::tra::types::UMLLinkRecord;
use std::collections::HashMap;

/// UMA binary file magic: `b"OPENHUMA"` as little-endian u64.
pub const UMA_MAGIC: u64 = u64::from_le_bytes(*b"OPENHUMA");
/// Version 2: u32 section counts, 16-byte pattern table, objects, labels,
/// pattern instances, package dependencies and derived interaction views.
pub const UMA_FORMAT_VERSION: u32 = 2;
pub const UMA_HEADER_SIZE: usize = 64;

// ── Explicit Unknown Markers ──────────────────────────────────────────────────
/// Symbol reference that could not be resolved from STA/CGA facts.
pub const UNKNOWN_SYM: u32 = u32::MAX;
/// Label id 0 is reserved and never present in `label_texts`.
pub const NO_LABEL: u32 = 0;
/// `UMLLinkRecord.file_id` value meaning "no source location is known".
pub const UML_LINK_UNKNOWN_FILE: u16 = u16::MAX;
/// Multiplicity bound value meaning "unbounded" (`*`).
pub const MULTIPLICITY_MANY: u16 = u16::MAX;
/// Multiplicity bound value meaning "not derivable from the source".
pub const MULTIPLICITY_UNKNOWN: u16 = u16::MAX - 1;

/// Returns true when a UMLLink points at a real declaring token range.
pub fn uml_link_is_known(link: &UMLLinkRecord) -> bool {
    link.file_id != UML_LINK_UNKNOWN_FILE && link.line_start != 0
}

// ── Stereotype Flags for ClassRecord ──────────────────────────────────────────
pub const STEREOTYPE_NONE: u8 = 0;
pub const STEREOTYPE_ABSTRACT: u8 = 1;
pub const STEREOTYPE_INTERFACE: u8 = 2;
pub const STEREOTYPE_ENUM: u8 = 3;
pub const STEREOTYPE_RECORD: u8 = 4;
pub const STEREOTYPE_ANNOTATION: u8 = 5;

// ── Design Pattern Kinds (all 23 GoF patterns) ────────────────────────────────
pub const PATTERN_NONE: u8 = 0;
pub const PATTERN_SINGLETON: u8 = 1;
pub const PATTERN_OBSERVER: u8 = 2;
/// Factory Method (GoF). Kept under its historical constant name.
pub const PATTERN_FACTORY: u8 = 3;
pub const PATTERN_BUILDER: u8 = 4;
pub const PATTERN_STATE: u8 = 5;
pub const PATTERN_TEMPLATE_METHOD: u8 = 6;
pub const PATTERN_DECORATOR: u8 = 7;
pub const PATTERN_STRATEGY: u8 = 8;
pub const PATTERN_ADAPTER: u8 = 9;
pub const PATTERN_FACADE: u8 = 10;
pub const PATTERN_COMPOSITE: u8 = 11;
pub const PATTERN_PROXY: u8 = 12;
pub const PATTERN_VISITOR: u8 = 13;
pub const PATTERN_COMMAND: u8 = 14;
pub const PATTERN_CHAIN_OF_RESPONSIBILITY: u8 = 15;
pub const PATTERN_ABSTRACT_FACTORY: u8 = 16;
pub const PATTERN_PROTOTYPE: u8 = 17;
pub const PATTERN_ITERATOR: u8 = 18;
pub const PATTERN_MEDIATOR: u8 = 19;
pub const PATTERN_MEMENTO: u8 = 20;
pub const PATTERN_FLYWEIGHT: u8 = 21;
pub const PATTERN_BRIDGE: u8 = 22;
pub const PATTERN_INTERPRETER: u8 = 23;

/// Canonical display name of a pattern kind (`""` for `PATTERN_NONE`/unknown).
pub fn pattern_kind_name(kind: u8) -> &'static str {
    match kind {
        PATTERN_SINGLETON => "Singleton",
        PATTERN_OBSERVER => "Observer",
        PATTERN_FACTORY => "FactoryMethod",
        PATTERN_BUILDER => "Builder",
        PATTERN_STATE => "State",
        PATTERN_TEMPLATE_METHOD => "TemplateMethod",
        PATTERN_DECORATOR => "Decorator",
        PATTERN_STRATEGY => "Strategy",
        PATTERN_ADAPTER => "Adapter",
        PATTERN_FACADE => "Facade",
        PATTERN_COMPOSITE => "Composite",
        PATTERN_PROXY => "Proxy",
        PATTERN_VISITOR => "Visitor",
        PATTERN_COMMAND => "Command",
        PATTERN_CHAIN_OF_RESPONSIBILITY => "ChainOfResponsibility",
        PATTERN_ABSTRACT_FACTORY => "AbstractFactory",
        PATTERN_PROTOTYPE => "Prototype",
        PATTERN_ITERATOR => "Iterator",
        PATTERN_MEDIATOR => "Mediator",
        PATTERN_MEMENTO => "Memento",
        PATTERN_FLYWEIGHT => "Flyweight",
        PATTERN_BRIDGE => "Bridge",
        PATTERN_INTERPRETER => "Interpreter",
        _ => "",
    }
}

// ── Pattern Participant Roles ─────────────────────────────────────────────────
pub const ROLE_SINGLETON: u8 = 1;
pub const ROLE_SUBJECT: u8 = 2;
pub const ROLE_OBSERVER: u8 = 3;
pub const ROLE_CONCRETE_OBSERVER: u8 = 4;
pub const ROLE_CREATOR: u8 = 5;
pub const ROLE_CONCRETE_CREATOR: u8 = 6;
pub const ROLE_PRODUCT: u8 = 7;
pub const ROLE_CONCRETE_PRODUCT: u8 = 8;
pub const ROLE_BUILDER: u8 = 9;
pub const ROLE_CONTEXT: u8 = 10;
pub const ROLE_STATE: u8 = 11;
pub const ROLE_CONCRETE_STATE: u8 = 12;
pub const ROLE_STRATEGY: u8 = 13;
pub const ROLE_CONCRETE_STRATEGY: u8 = 14;
pub const ROLE_ABSTRACT_CLASS: u8 = 15;
pub const ROLE_CONCRETE_CLASS: u8 = 16;
pub const ROLE_TEMPLATE_METHOD: u8 = 17;
pub const ROLE_PRIMITIVE_OPERATION: u8 = 18;
pub const ROLE_COMPONENT: u8 = 19;
pub const ROLE_CONCRETE_COMPONENT: u8 = 20;
pub const ROLE_DECORATOR: u8 = 21;
pub const ROLE_TARGET: u8 = 22;
pub const ROLE_ADAPTER: u8 = 23;
pub const ROLE_ADAPTEE: u8 = 24;
pub const ROLE_FACADE: u8 = 25;
pub const ROLE_SUBSYSTEM: u8 = 26;
pub const ROLE_COMPOSITE: u8 = 27;
pub const ROLE_LEAF: u8 = 28;
pub const ROLE_PROXY: u8 = 29;
pub const ROLE_REAL_SUBJECT: u8 = 30;
pub const ROLE_VISITOR: u8 = 31;
pub const ROLE_CONCRETE_VISITOR: u8 = 32;
pub const ROLE_ELEMENT: u8 = 33;
pub const ROLE_CONCRETE_ELEMENT: u8 = 34;
pub const ROLE_COMMAND: u8 = 35;
pub const ROLE_CONCRETE_COMMAND: u8 = 36;
pub const ROLE_INVOKER: u8 = 37;
pub const ROLE_RECEIVER: u8 = 38;
pub const ROLE_HANDLER: u8 = 39;
pub const ROLE_CONCRETE_HANDLER: u8 = 40;
pub const ROLE_ABSTRACT_FACTORY: u8 = 41;
pub const ROLE_CONCRETE_FACTORY: u8 = 42;
pub const ROLE_PROTOTYPE: u8 = 43;
pub const ROLE_CONCRETE_PROTOTYPE: u8 = 44;
pub const ROLE_ITERATOR: u8 = 45;
pub const ROLE_CONCRETE_ITERATOR: u8 = 46;
pub const ROLE_AGGREGATE: u8 = 47;
pub const ROLE_MEDIATOR: u8 = 48;
pub const ROLE_CONCRETE_MEDIATOR: u8 = 49;
pub const ROLE_COLLEAGUE: u8 = 50;
pub const ROLE_ORIGINATOR: u8 = 51;
pub const ROLE_MEMENTO: u8 = 52;
pub const ROLE_CARETAKER: u8 = 53;
pub const ROLE_FLYWEIGHT_FACTORY: u8 = 54;
pub const ROLE_FLYWEIGHT: u8 = 55;
pub const ROLE_ABSTRACTION: u8 = 56;
pub const ROLE_REFINED_ABSTRACTION: u8 = 57;
pub const ROLE_IMPLEMENTOR: u8 = 58;
pub const ROLE_CONCRETE_IMPLEMENTOR: u8 = 59;
pub const ROLE_ABSTRACT_EXPRESSION: u8 = 60;
pub const ROLE_TERMINAL_EXPRESSION: u8 = 61;
pub const ROLE_NONTERMINAL_EXPRESSION: u8 = 62;
pub const ROLE_CLIENT: u8 = 63;
pub const ROLE_CONCRETE_FLYWEIGHT: u8 = 64;

// ── Pattern Evidence Facts (bitset in PatternInstanceRecord.evidence_mask) ───
pub const EVIDENCE_PRIVATE_CONSTRUCTORS: u64 = 1 << 0;
pub const EVIDENCE_STATIC_SELF_FIELD: u64 = 1 << 1;
pub const EVIDENCE_STATIC_SELF_ACCESSOR: u64 = 1 << 2;
pub const EVIDENCE_ACCESSOR_RETURNS_FIELD: u64 = 1 << 3;
pub const EVIDENCE_LANGUAGE_SINGLETON: u64 = 1 << 4;
pub const EVIDENCE_HOLDER_IDIOM: u64 = 1 << 5;
pub const EVIDENCE_SUPERTYPE: u64 = 1 << 6;
pub const EVIDENCE_WRAPPED_FIELD: u64 = 1 << 7;
pub const EVIDENCE_FORWARDING_CALL: u64 = 1 << 8;
pub const EVIDENCE_COLLECTION_FIELD: u64 = 1 << 9;
pub const EVIDENCE_LOOP_DISPATCH: u64 = 1 << 10;
pub const EVIDENCE_REGISTRATION_METHOD: u64 = 1 << 11;
pub const EVIDENCE_OVERRIDE: u64 = 1 << 12;
pub const EVIDENCE_ALLOCATES_SUBTYPE: u64 = 1 << 13;
pub const EVIDENCE_HOOK_CALL: u64 = 1 << 14;
pub const EVIDENCE_FLUENT_RETURNS: u64 = 1 << 15;
pub const EVIDENCE_BUILD_METHOD: u64 = 1 << 16;
pub const EVIDENCE_POLYMORPHIC_DELEGATE: u64 = 1 << 17;
pub const EVIDENCE_INTERNAL_REASSIGNMENT: u64 = 1 << 18;
pub const EVIDENCE_INJECTED_DELEGATE: u64 = 1 << 19;
pub const EVIDENCE_DOUBLE_DISPATCH: u64 = 1 << 20;
pub const EVIDENCE_SUCCESSOR_FIELD: u64 = 1 << 21;
pub const EVIDENCE_COPY_METHOD: u64 = 1 << 22;
pub const EVIDENCE_MULTIPLE_IMPLEMENTORS: u64 = 1 << 23;
pub const EVIDENCE_SUBSYSTEM_CALLS: u64 = 1 << 24;
pub const EVIDENCE_INVOKER: u64 = 1 << 25;
pub const EVIDENCE_CACHE_LOOKUP: u64 = 1 << 26;
pub const EVIDENCE_STATE_SAVE_RESTORE: u64 = 1 << 27;
pub const EVIDENCE_COLLEAGUE_CALLS: u64 = 1 << 28;
pub const EVIDENCE_REFINED_ABSTRACTION: u64 = 1 << 29;
pub const EVIDENCE_RECURSIVE_STRUCTURE: u64 = 1 << 30;
pub const EVIDENCE_TRANSLATED_CALL: u64 = 1 << 31;
pub const EVIDENCE_FINAL_TEMPLATE: u64 = 1 << 32;
pub const EVIDENCE_ITERATION_PROTOCOL: u64 = 1 << 33;
pub const EVIDENCE_BACK_REFERENCE: u64 = 1 << 34;
pub const EVIDENCE_NAME_PRIOR: u64 = 1 << 35;
pub const EVIDENCE_TYPED_COLLECTION: u64 = 1 << 36;
pub const EVIDENCE_ADDS_TO_COLLECTION: u64 = 1 << 37;
pub const EVIDENCE_CONCRETE_PARTICIPANTS: u64 = 1 << 38;
pub const EVIDENCE_FOREIGN_PACKAGE: u64 = 1 << 39;
pub const EVIDENCE_ENCAPSULATED: u64 = 1 << 40;
pub const EVIDENCE_VALUE_RESULT: u64 = 1 << 41;

// ── Activity Node Kinds (§9.2.2) ─────────────────────────────────────────────
pub const NODE_KIND_INITIAL: u8 = 0;
pub const NODE_KIND_ACTION: u8 = 1;
pub const NODE_KIND_DECISION: u8 = 2;
pub const NODE_KIND_MERGE: u8 = 3;
pub const NODE_KIND_FORK: u8 = 4;
pub const NODE_KIND_JOIN: u8 = 5;
pub const NODE_KIND_FINAL: u8 = 6;
pub const NODE_KIND_EXCEPTION: u8 = 7;

// ── Activity Edge Kinds ───────────────────────────────────────────────────────
pub const EDGE_KIND_CONTROL: u8 = 0;
pub const EDGE_KIND_OBJECT: u8 = 1;
pub const EDGE_KIND_EXCEPTION: u8 = 2;

// ── Association Kinds (FieldRecord.assoc_kind, mirrors STA AssocKind) ────────
pub const ASSOC_NONE: u8 = 0;
pub const ASSOC_DEPENDENCY: u8 = 1;
pub const ASSOC_ASSOCIATION: u8 = 2;
pub const ASSOC_AGGREGATION: u8 = 3;
pub const ASSOC_COMPOSITION: u8 = 4;

// ── Method Metric Availability (MethodRecord.metrics_known bits) ─────────────
/// `cyclomatic` was computed from the method's CFG/PSA.
pub const METRIC_CYCLOMATIC_KNOWN: u8 = 1 << 0;
/// `sat_count` (feasible path count) was computed by PSA.
pub const METRIC_SAT_COUNT_KNOWN: u8 = 1 << 1;

// ── Field Kinds ───────────────────────────────────────────────────────────────
pub const FIELD_KIND_ATTRIBUTE: u8 = 0;
pub const FIELD_KIND_ENUM_LITERAL: u8 = 1;

// ── Sequence Message Kinds ────────────────────────────────────────────────────
pub const MSG_KIND_SYNC: u8 = 0;
pub const MSG_KIND_REPLY: u8 = 1;
pub const MSG_KIND_ASYNC: u8 = 2;
pub const MSG_KIND_CREATE: u8 = 3;
/// Entry message from the external actor into the scenario's entry method.
pub const MSG_KIND_FOUND: u8 = 4;

// ── Combined Fragment Kinds ───────────────────────────────────────────────────
pub const FRAGMENT_LOOP: u8 = 0;
pub const FRAGMENT_ALT: u8 = 1;
pub const FRAGMENT_OPT: u8 = 2;
pub const FRAGMENT_PAR: u8 = 3;
pub const FRAGMENT_BREAK: u8 = 4;

// ── Scenario Entry Kinds (SequenceDiagramRecord.entry_kind) ───────────────────
/// `public static void main(...)`.
pub const ENTRY_KIND_MAIN: u8 = 0;
/// Framework-invoked handler (HTTP mapping, message listener, scheduler, test).
pub const ENTRY_KIND_ANNOTATED: u8 = 1;
/// Framework lifecycle callback overriding a library supertype (e.g. Android).
pub const ENTRY_KIND_LIFECYCLE: u8 = 2;
/// Visible method that no analyzed code calls (used only when no framework
/// entry point exists).
pub const ENTRY_KIND_UNREFERENCED: u8 = 3;

// ── State Machine Markers ─────────────────────────────────────────────────────
/// `TransitionRecord.from_state` value: the source state is not constrained by a
/// guard on the state variable, so the transition fires from any state.
pub const TRANSITION_FROM_ANY: u16 = u16::MAX;
/// `StateMachineRecord.initial_state` value: no initializer establishes the start state.
pub const INITIAL_STATE_UNKNOWN: u16 = u16::MAX;

// ── Component Kinds ───────────────────────────────────────────────────────────
pub const COMPONENT_KIND_PACKAGE: u8 = 0;
/// A dependency-injection bean (class annotated as a framework component).
pub const COMPONENT_KIND_BEAN: u8 = 1;

// ── Field Record ──────────────────────────────────────────────────────────────
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldRecord {
    pub field_sym_id: u32,
    pub type_sym_id: u32,
    pub visibility: u8,
    pub modifiers: u16,
    /// 1 when the declared type is an array or a library collection type.
    pub is_collection: u8,
    /// FIELD_KIND_* (attribute or enumeration literal).
    pub field_kind: u8,
    /// ASSOC_* relationship kind to the target type (ASSOC_NONE for primitives/libraries).
    pub assoc_kind: u8,
    pub mult_min: u16,
    /// MULTIPLICITY_MANY for `*`, MULTIPLICITY_UNKNOWN when not derivable.
    pub mult_max: u16,
    /// Collection element type (last type argument), UNKNOWN_SYM when unresolved.
    pub element_type_sym_id: u32,
    pub uml_link_node: u32,
}

// ── Method Record ─────────────────────────────────────────────────────────────
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MethodRecord {
    pub method_sym_id: u32,
    pub return_type_sym_id: u32,
    pub visibility: u8,
    pub modifiers: u16,
    pub param_count: u16,
    pub cyclomatic: u16,
    pub sat_count: u64,
    pub is_constructor: u8,
    /// METRIC_*_KNOWN bits; a cleared bit means the value is 0 because no body was analyzed.
    pub metrics_known: u8,
    /// Parameter symbols in declaration order.
    pub param_syms: Vec<u32>,
    /// Parameter types in declaration order (UNKNOWN_SYM when unresolved).
    pub param_type_syms: Vec<u32>,
}

// ── Class Record (variable-length) ───────────────────────────────────────────
#[derive(Clone, Debug)]
pub struct ClassRecord {
    pub sym_id: u32,
    pub stereotype: u8,
    pub visibility: u8,
    pub modifiers: u16,
    /// Unique direct superclass, UNKNOWN_SYM when none or when ambiguous.
    pub extends_sym: u32,
    pub field_count: u16,
    pub method_count: u16,
    pub inner_count: u16,
    pub design_pattern: u8,
    pub _reserved: u8,
    pub type_param_count: u8,
    pub _pad: u8,
    pub uml_link: UMLLinkRecord,
    pub fields: Vec<FieldRecord>,
    pub methods: Vec<MethodRecord>,
    pub inner_classes: Vec<u32>,
    pub implements_syms: Vec<u32>,
    /// Structural associations (attribute-typed references), sorted by sym id.
    pub association_syms: Vec<u32>,
    /// All direct generalization targets (superclass and super-interfaces), sorted.
    pub generalization_syms: Vec<u32>,
    /// «use»/«create» dependencies: return, parameter and instantiated types, sorted.
    pub dependency_syms: Vec<u32>,
    /// Label id of the fully-qualified name (NO_LABEL when no package is known).
    pub qualified_name_id: u32,
}

// ── Activity Node ─────────────────────────────────────────────────────────────
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActivityNode {
    pub node_id: u32,
    pub label_text_id: u32,
    pub node_kind: u8,
    pub loop_depth: u8,
    pub guard_text_id: u32,
    /// First source token of the block (u32::MAX for synthetic entry/exit blocks).
    pub first_token: u32,
}

// ── Activity Edge ─────────────────────────────────────────────────────────────
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActivityEdge {
    pub from_node: u32,
    pub to_node: u32,
    pub edge_kind: u8,
    pub is_back_edge: u8,
    /// Guard label (`[cond]`, `[else]`, `[case X]`, `[exception]`), NO_LABEL when unguarded.
    pub guard_text_id: u32,
}

// ── Activity Record (variable-length per function) ───────────────────────────
#[derive(Clone, Debug)]
pub struct ActivityRecord {
    pub function_sym_id: u32,
    pub node_count: u32,
    pub edge_count: u32,
    pub start_node: u32,
    pub end_node_count: u16,
    pub swimlane_count: u16,
    pub cyclomatic: u16,
    pub max_loop_depth: u8,
    pub _reserved: u8,
    pub nodes: Vec<ActivityNode>,
    pub edges: Vec<ActivityEdge>,
}

// ── State Record ──────────────────────────────────────────────────────────────
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StateRecord {
    pub state_id: u32,
    pub state_name_id: u32,
    pub is_initial: u8,
    pub is_final: u8,
    pub _pad: u16,
}

// ── Transition Record ─────────────────────────────────────────────────────────
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransitionRecord {
    /// TRANSITION_FROM_ANY when no guard constrains the source state.
    pub from_state: u16,
    pub to_state: u16,
    pub trigger_method_sym: u32,
    pub guard_text_id: u32,
    pub action_text_id: u32,
    /// Token of the assignment that performs the transition (traceability).
    pub source_token: u32,
}

// ── State Machine Record (variable-length) ────────────────────────────────────
#[derive(Clone, Debug)]
pub struct StateMachineRecord {
    pub class_sym_id: u32,
    /// The enum-typed field whose assignments define the states.
    pub state_field_sym: u32,
    pub state_count: u16,
    pub transition_count: u16,
    /// INITIAL_STATE_UNKNOWN when no initializer establishes the start state.
    pub initial_state: u16,
    pub final_state_count: u8,
    pub _reserved: u8,
    pub states: Vec<StateRecord>,
    pub transitions: Vec<TransitionRecord>,
}

// ── Lifeline Record ───────────────────────────────────────────────────────────
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LifelineRecord {
    pub sym_id: u32,
    pub name_id: u32,
    pub type_sym_id: u32,
    pub is_actor: u8,
    pub _pad: [u8; 3],
}

// ── Message Record ────────────────────────────────────────────────────────────
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MessageRecord {
    pub from_lifeline: u32,
    pub to_lifeline: u32,
    pub call_site_id: u32,
    pub method_sym_id: u32,
    /// MSG_KIND_* (sync, reply, async, create, found).
    pub message_kind: u8,
    /// CGA call edge type (CG_EDGE_*) of the originating call site.
    pub call_edge_type: u8,
    pub ordinal: u16,
    /// Activation nesting depth of the sender when the message is sent.
    pub nesting_depth: u16,
    pub uml_link_token: u32,
}

// ── Combined Fragment Record ──────────────────────────────────────────────────
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CombinedFragment {
    pub fragment_kind: u8, // FRAGMENT_* (Loop, Alt, Opt, Par, Break)
    pub guard_text_id: u32,
    pub start_message_ordinal: u16,
    pub end_message_ordinal: u16,
    /// Nesting depth of the fragment inside the scenario.
    pub nesting_depth: u16,
}

// ── Sequence Diagram Record (variable-length) ─────────────────────────────────
#[derive(Clone, Debug)]
pub struct SequenceDiagramRecord {
    pub scenario_name: u32,
    pub entry_method_sym: u32,
    pub lifeline_count: u16,
    pub message_count: u16,
    pub fragment_count: u16,
    /// 1 when the depth or message bound cut the scenario short.
    pub truncated: u8,
    /// ENTRY_KIND_* that selected the scenario's entry method.
    pub entry_kind: u8,
    pub lifelines: Vec<LifelineRecord>,
    pub messages: Vec<MessageRecord>,
    pub combined_fragments: Vec<CombinedFragment>,
}

// ── Object Record (allocation site) ───────────────────────────────────────────
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectRecord {
    /// Token of the `new` keyword of the allocation site.
    pub alloc_token: u32,
    pub type_sym_id: u32,
    /// Label "variable : Type" (NO_LABEL when the instance is anonymous).
    pub label_text_id: u32,
    pub containing_method_sym: u32,
    /// Field of the containing class that stores the instance (UNKNOWN_SYM for locals/temporaries).
    pub assigned_field_sym: u32,
}

// ── Object Link Record (field points-to between allocation sites) ────────────
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectLinkRecord {
    /// Index into `UMLMetadataArtifact.objects` of the owning instance.
    pub from_object: u32,
    /// Index into `UMLMetadataArtifact.objects` of the referenced instance.
    pub to_object: u32,
    pub via_field_sym: u32,
}

// ── Package Record ────────────────────────────────────────────────────────────
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackageRecord {
    pub package_sym_id: u32,
    pub name_id: u32,
    pub parent_package_sym: u32,
    pub class_count: u16,
    pub subpackage_count: u16,
}

// ── Package Dependency Record ─────────────────────────────────────────────────
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackageDependencyRecord {
    pub from_package_sym: u32,
    pub to_package_sym: u32,
    /// Number of type-level references (inheritance, attributes, calls) behind the edge.
    pub reference_count: u32,
    /// 1 when both packages are in the same dependency cycle (Tarjan SCC).
    pub in_cycle: u8,
}

// ── Package Metrics Record (Martin, 1994) ─────────────────────────────────────
#[derive(Clone, Debug, PartialEq)]
pub struct PackageMetricsRecord {
    pub package_sym_id: u32,
    /// Afferent coupling: packages depending on this one.
    pub afferent: u32,
    /// Efferent coupling: packages this one depends on.
    pub efferent: u32,
    /// Instability I = Ce / (Ca + Ce); NaN when the package has no couplings.
    pub instability: f32,
    /// Abstractness A = abstract types / types; NaN when the package declares no types.
    pub abstractness: f32,
    /// Distance from the main sequence D = |A + I - 1|; NaN when I or A is undefined.
    pub distance: f32,
    /// Dependency cycle id (index of the SCC), u32::MAX when acyclic.
    pub cycle_id: u32,
}

// ── Component Record ──────────────────────────────────────────────────────────
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComponentRecord {
    pub component_sym_id: u32,
    pub name_id: u32,
    pub provided_interface_count: u16,
    pub required_interface_count: u16,
    /// COMPONENT_KIND_* (package or dependency-injection bean).
    pub component_kind: u8,
    /// Interfaces realized by the component, sorted.
    pub provided_interfaces: Vec<u32>,
    /// Interfaces used by the component but declared elsewhere, sorted.
    pub required_interfaces: Vec<u32>,
}

// ── Design Pattern Record (one row per participant) ───────────────────────────
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesignPatternRecord {
    pub class_sym: u32,
    pub pattern_kind: u16,
    /// Structural-evidence score 0..100 (satisfied / specified constraints, uncalibrated).
    pub confidence: u16,
    /// Index into `UMLMetadataArtifact.pattern_instances`.
    pub instance_id: u32,
    /// ROLE_* the class plays in the instance.
    pub role: u8,
}

// ── Pattern Instance Record ───────────────────────────────────────────────────
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PatternRoleBinding {
    pub role: u8,
    pub sym_id: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PatternInstanceRecord {
    pub instance_id: u32,
    pub pattern_kind: u8,
    /// Structural-evidence score 0..100 (satisfied / specified constraints, uncalibrated).
    pub score: u8,
    /// Class that anchors the instance (Context, Subject, Composite, ...).
    pub primary_sym: u32,
    /// EVIDENCE_* facts that held.
    pub evidence_mask: u64,
    /// Role bindings sorted by (role, sym_id).
    pub roles: Vec<PatternRoleBinding>,
}

// ── Communication Diagram Records (derived from sequence scenarios) ──────────
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommunicationLinkRecord {
    pub from_lifeline: u32,
    pub to_lifeline: u32,
    /// Ordinals of the scenario messages carried by this link, in send order.
    pub message_ordinals: Vec<u16>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommunicationDiagramRecord {
    /// Index into `UMLMetadataArtifact.sequences`.
    pub sequence_index: u32,
    pub links: Vec<CommunicationLinkRecord>,
}

// ── Interaction Overview Records ──────────────────────────────────────────────
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InteractionReferenceRecord {
    /// Activity node of the entry method's activity that contains the call.
    pub activity_node: u32,
    /// Index into `UMLMetadataArtifact.sequences` of the referenced interaction.
    pub sequence_index: u32,
    pub call_site_id: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InteractionOverviewRecord {
    pub entry_method_sym: u32,
    /// Index into `UMLMetadataArtifact.activities` of the entry method's control flow.
    pub activity_index: u32,
    pub references: Vec<InteractionReferenceRecord>,
}

// ── Composite Structure Records ───────────────────────────────────────────────
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompositePartRecord {
    /// Field (or nested class) that realizes the part.
    pub part_sym: u32,
    pub type_sym_id: u32,
    pub mult_min: u16,
    pub mult_max: u16,
    /// 1 for a nested type, 0 for a composition-owned attribute.
    pub is_nested_type: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompositeConnectorRecord {
    pub from_part: u32,
    pub to_part: u32,
    /// Method of the source part's type that calls into the target part's type.
    pub via_method_sym: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompositeStructureRecord {
    pub class_sym_id: u32,
    pub parts: Vec<CompositePartRecord>,
    pub connectors: Vec<CompositeConnectorRecord>,
}

// ── Full UMA Artifact ─────────────────────────────────────────────────────────
pub struct UMLMetadataArtifact {
    pub format_version: u32,
    pub tra_hash: u64,
    pub classes: Vec<ClassRecord>,
    pub objects: Vec<ObjectRecord>,
    pub object_links: Vec<ObjectLinkRecord>,
    pub activities: Vec<ActivityRecord>,
    pub state_machines: Vec<StateMachineRecord>,
    pub sequences: Vec<SequenceDiagramRecord>,
    pub communications: Vec<CommunicationDiagramRecord>,
    pub interaction_overviews: Vec<InteractionOverviewRecord>,
    pub packages: Vec<PackageRecord>,
    pub package_dependencies: Vec<PackageDependencyRecord>,
    pub package_metrics: Vec<PackageMetricsRecord>,
    pub components: Vec<ComponentRecord>,
    pub composite_structures: Vec<CompositeStructureRecord>,
    pub design_patterns: Vec<DesignPatternRecord>,
    pub pattern_instances: Vec<PatternInstanceRecord>,
    pub label_texts: HashMap<u32, String>,
}

impl UMLMetadataArtifact {
    pub fn new(tra_hash: u64) -> Self {
        Self {
            format_version: UMA_FORMAT_VERSION,
            tra_hash,
            classes: Vec::new(),
            objects: Vec::new(),
            object_links: Vec::new(),
            activities: Vec::new(),
            state_machines: Vec::new(),
            sequences: Vec::new(),
            communications: Vec::new(),
            interaction_overviews: Vec::new(),
            packages: Vec::new(),
            package_dependencies: Vec::new(),
            package_metrics: Vec::new(),
            components: Vec::new(),
            composite_structures: Vec::new(),
            design_patterns: Vec::new(),
            pattern_instances: Vec::new(),
            label_texts: HashMap::new(),
        }
    }

    /// Resolve a label id to its text (`None` for `NO_LABEL` or unknown ids).
    pub fn label(&self, label_id: u32) -> Option<&str> {
        if label_id == NO_LABEL {
            return None;
        }
        self.label_texts.get(&label_id).map(|s| s.as_str())
    }
}
