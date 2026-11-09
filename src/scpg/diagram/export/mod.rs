//! Diagram Export Modules: PlantUML, Mermaid, JSON Graph IR, OMG XMI 2.5.1, DOT/GraphML
//! and SARIF (§10.4). Every printer visits the same typed view IR (`diagram::views`).

pub mod graph;
pub mod json;
pub mod mermaid;
pub mod plantuml;
pub mod plantuml_optimizer;
pub mod sarif;
pub mod xmi;

pub use graph::GraphExporter;
pub use json::{GraphIR, JSONExporter, GRAPH_IR_SCHEMA_VERSION};
pub use mermaid::{
    ActivityMermaidStrategy, CDGMermaidStrategy, CFGMermaidStrategy, CallGraphMermaidStrategy,
    ClassMermaidStrategy, CommunicationMermaidStrategy, ComponentMermaidStrategy,
    CompositeStructureMermaidStrategy, DFGMermaidStrategy, DeploymentMermaidStrategy,
    InteractionOverviewMermaidStrategy, MermaidDiagramStrategy, MermaidExporter,
    ObjectMermaidStrategy, PackageMermaidStrategy, ProfileMermaidStrategy, ROBDDMermaidStrategy,
    SequenceMermaidStrategy, StateMachineMermaidStrategy, TimingMermaidStrategy,
    UseCaseMermaidStrategy,
};
pub use plantuml::{
    ActivityDiagramStrategy, CDGDiagramStrategy, CFGDiagramStrategy, CallGraphDiagramStrategy,
    ClassDiagramStrategy, CommunicationDiagramStrategy, ComponentDiagramStrategy,
    CompositeStructureDiagramStrategy, DFGDiagramStrategy, DeploymentDiagramStrategy,
    InteractionOverviewDiagramStrategy, ObjectDiagramStrategy, PackageDiagramStrategy,
    PlantUMLDiagramStrategy, PlantUMLExporter, ProfileDiagramStrategy, ROBDDDiagramStrategy,
    SequenceDiagramStrategy, StateMachineDiagramStrategy, TimingDiagramStrategy,
    UseCaseDiagramStrategy,
};
pub use plantuml_optimizer::{PlantUMLOptimizationOptions, PlantUMLOptimizer, RawEdge};
pub use sarif::SarifExporter;
pub use xmi::XMIExporter;
