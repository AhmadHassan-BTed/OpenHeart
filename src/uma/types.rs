//! Core types for Phase 9: UML Semantic Metadata Extraction (§9.4, §9.5).
//!
//! All binary record structures for the 14 UML diagram types, design pattern
//! detection records, label text tables, and `UMLMetadataArtifact (.uma)`.

use crate::tra::types::UMLLinkRecord;
use std::collections::HashMap;

/// UMA binary file magic: `b"OPENHUMA"` as little-endian u64.
pub const UMA_MAGIC: u64 = u64::from_le_bytes(*b"OPENHUMA");
pub const UMA_FORMAT_VERSION: u32 = 1;
pub const UMA_HEADER_SIZE: usize = 64;

// ── Stereotype Flags for ClassRecord ──────────────────────────────────────────
pub const STEREOTYPE_NONE: u8 = 0;
pub const STEREOTYPE_ABSTRACT: u8 = 1;
pub const STEREOTYPE_INTERFACE: u8 = 2;
pub const STEREOTYPE_ENUM: u8 = 3;
pub const STEREOTYPE_RECORD: u8 = 4;
pub const STEREOTYPE_ANNOTATION: u8 = 5;

// ── Design Pattern Kinds ──────────────────────────────────────────────────────
pub const PATTERN_NONE: u8 = 0;
pub const PATTERN_SINGLETON: u8 = 1;
pub const PATTERN_OBSERVER: u8 = 2;
pub const PATTERN_FACTORY: u8 = 3;
pub const PATTERN_BUILDER: u8 = 4;
pub const PATTERN_STATE: u8 = 5;
pub const PATTERN_TEMPLATE_METHOD: u8 = 6;
pub const PATTERN_DECORATOR: u8 = 7;
pub const PATTERN_STRATEGY: u8 = 8;
pub const PATTERN_ADAPTER: u8 = 9;
pub const PATTERN_FACADE: u8 = 10;
pub const PATTERN_COMPOSITE: u8 = 11;

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

// ── Field Record (16 bytes) ───────────────────────────────────────────────────
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldRecord {
    pub field_sym_id: u32,
    pub type_sym_id: u32,
    pub visibility: u8,
    pub modifiers: u8,
    pub is_collection: u8,
    pub _pad: u8,
    pub uml_link_node: u32,
    pub _reserved: u32,
}

// ── Method Record (20 bytes) ──────────────────────────────────────────────────
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MethodRecord {
    pub method_sym_id: u32,
    pub return_type_sym_id: u32,
    pub visibility: u8,
    pub modifiers: u8,
    pub param_count: u16,
    pub cyclomatic: u16,
    pub sat_count: u64,
}

// ── Class Record (variable-length) ───────────────────────────────────────────
#[derive(Clone, Debug)]
pub struct ClassRecord {
    pub sym_id: u32,
    pub stereotype: u8,
    pub visibility: u8,
    pub modifiers: u16,
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
    pub association_syms: Vec<u32>,
}

// ── Activity Node (16 bytes) ──────────────────────────────────────────────────
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActivityNode {
    pub node_id: u32,
    pub label_text_id: u32,
    pub node_kind: u8,
    pub loop_depth: u8,
    pub guard_text_id: u16,
    pub _pad: u32,
}

// ── Activity Edge (12 bytes) ──────────────────────────────────────────────────
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActivityEdge {
    pub from_node: u16,
    pub to_node: u16,
    pub edge_kind: u8,
    pub is_back_edge: u8,
    pub guard_text_id: u32,
    pub _pad: u16,
}

// ── Activity Record (variable-length per function) ───────────────────────────
#[derive(Clone, Debug)]
pub struct ActivityRecord {
    pub function_sym_id: u32,
    pub node_count: u16,
    pub edge_count: u16,
    pub start_node: u16,
    pub end_node_count: u8,
    pub swimlane_count: u8,
    pub cyclomatic: u16,
    pub _reserved: u16,
    pub nodes: Vec<ActivityNode>,
    pub edges: Vec<ActivityEdge>,
}

// ── State Record (12 bytes) ───────────────────────────────────────────────────
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StateRecord {
    pub state_id: u32,
    pub state_name_id: u32,
    pub is_initial: u8,
    pub is_final: u8,
    pub _pad: u16,
}

// ── Transition Record (16 bytes) ──────────────────────────────────────────────
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransitionRecord {
    pub from_state: u16,
    pub to_state: u16,
    pub trigger_method_sym: u32,
    pub guard_text_id: u32,
    pub action_text_id: u32,
}

// ── State Machine Record (variable-length) ────────────────────────────────────
#[derive(Clone, Debug)]
pub struct StateMachineRecord {
    pub class_sym_id: u32,
    pub state_count: u16,
    pub transition_count: u16,
    pub initial_state: u16,
    pub final_state_count: u8,
    pub _reserved: u8,
    pub _pad: u32,
    pub states: Vec<StateRecord>,
    pub transitions: Vec<TransitionRecord>,
}

// ── Lifeline Record (16 bytes) ────────────────────────────────────────────────
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LifelineRecord {
    pub sym_id: u32,
    pub name_id: u32,
    pub type_sym_id: u32,
    pub is_actor: u8,
    pub _pad: [u8; 3],
}

// ── Message Record (24 bytes) ─────────────────────────────────────────────────
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MessageRecord {
    pub from_lifeline: u32,
    pub to_lifeline: u32,
    pub call_site_id: u32,
    pub method_sym_id: u32,
    pub message_kind: u8,
    pub ordinal: u16,
    pub _pad: u16,
    pub uml_link_token: u32,
}

// ── Combined Fragment Record (12 bytes) ───────────────────────────────────────
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CombinedFragment {
    pub fragment_kind: u8, // 0=Loop, 1=Alt, 2=Opt, 3=Par
    pub guard_text_id: u32,
    pub start_message_ordinal: u16,
    pub end_message_ordinal: u16,
    pub _pad: u16,
}

// ── Sequence Diagram Record (variable-length) ─────────────────────────────────
#[derive(Clone, Debug)]
pub struct SequenceDiagramRecord {
    pub scenario_name: u32,
    pub lifeline_count: u16,
    pub message_count: u16,
    pub fragment_count: u16,
    pub _reserved: u16,
    pub lifelines: Vec<LifelineRecord>,
    pub messages: Vec<MessageRecord>,
    pub combined_fragments: Vec<CombinedFragment>,
}

// ── Object Record (16 bytes) ──────────────────────────────────────────────────
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectRecord {
    pub alloc_ssa_id: u32,
    pub type_sym_id: u32,
    pub label_text_id: u32,
    pub containing_method_sym: u32,
}

// ── Package Record (16 bytes) ─────────────────────────────────────────────────
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackageRecord {
    pub package_sym_id: u32,
    pub name_id: u32,
    pub parent_package_sym: u32,
    pub class_count: u16,
    pub subpackage_count: u16,
}

// ── Component Record (16 bytes) ───────────────────────────────────────────────
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComponentRecord {
    pub component_sym_id: u32,
    pub name_id: u32,
    pub provided_interface_count: u16,
    pub required_interface_count: u16,
    pub _pad: u32,
}

// ── Design Pattern Record (12 bytes) ──────────────────────────────────────────
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesignPatternRecord {
    pub class_sym: u32,
    pub pattern_kind: u16,
    pub confidence: u16, // 0..100
    pub _reserved: u32,
}

// ── Full UMA Artifact ─────────────────────────────────────────────────────────
pub struct UMLMetadataArtifact {
    pub format_version: u32,
    pub tra_hash: u64,
    pub classes: Vec<ClassRecord>,
    pub objects: Vec<ObjectRecord>,
    pub activities: Vec<ActivityRecord>,
    pub state_machines: Vec<StateMachineRecord>,
    pub sequences: Vec<SequenceDiagramRecord>,
    pub packages: Vec<PackageRecord>,
    pub components: Vec<ComponentRecord>,
    pub design_patterns: Vec<DesignPatternRecord>,
    pub label_texts: HashMap<u32, String>,
}

impl UMLMetadataArtifact {
    pub fn new(tra_hash: u64) -> Self {
        Self {
            format_version: UMA_FORMAT_VERSION,
            tra_hash,
            classes: Vec::new(),
            objects: Vec::new(),
            activities: Vec::new(),
            state_machines: Vec::new(),
            sequences: Vec::new(),
            packages: Vec::new(),
            components: Vec::new(),
            design_patterns: Vec::new(),
            label_texts: HashMap::new(),
        }
    }
}
