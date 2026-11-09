//! DiagramGenerator — top-level UML diagram generation and multi-format export interface (§10.4).
//!
//! Pipeline: [`DiagramContext`] (artifacts) → `views::build` (one typed [`ViewModel`] per
//! diagram kind, with stable keys, spans and provenance) → format printers (PlantUML,
//! Mermaid, JSON Graph IR, XMI 2.5.1, DOT, GraphML, SARIF).

pub mod common;
pub mod context;
pub mod export;
pub mod factory;
pub mod model;
pub mod renderers;
pub mod views;

pub use common::DiagramUtils;
pub use context::{DiagramContext, SourceSpan, ViewOptions};
pub use model::{Provenance, ViewKind, ViewModel};

pub use export::graph::GraphExporter;
pub use export::json::JSONExporter;
pub use export::mermaid::{MermaidDiagramStrategy, MermaidExporter};
pub use export::plantuml::{PlantUMLDiagramStrategy, PlantUMLExporter};
pub use export::sarif::SarifExporter;
pub use export::xmi::XMIExporter;
pub use factory::{
    DiagramExporterFactory, DiagramFormat, GraphFactory, JSONFactory, MermaidFactory,
    PlantUMLFactory, UniversalDiagramEngine, XMIFactory,
};
pub use renderers::DiagramRenderers;

use crate::uma::types::UMLMetadataArtifact;

pub struct DiagramGenerator;

impl DiagramGenerator {
    pub fn generate_summary(uma: &UMLMetadataArtifact) -> String {
        let stats = DiagramRenderers::render_all_14_diagrams(uma);
        let count = |name: &str| {
            DiagramRenderers::count(&stats, name)
                .map(|c| c.to_string())
                .unwrap_or_else(|| "n/a".to_string())
        };
        let not_derivable: Vec<&str> = stats
            .iter()
            .filter(|(_, c)| c.is_none())
            .map(|(name, _)| *name)
            .collect();
        let with_data = stats
            .iter()
            .filter(|(_, c)| c.map(|n| n > 0).unwrap_or(false))
            .count();
        format!(
            "UML projections: {} classes, {} activities, {} sequence scenarios, {} state machines; {} of {} diagram types have data; not statically derivable: {}.",
            count("class_diagram"),
            count("activity_diagram"),
            count("sequence_diagram"),
            count("state_machine_diagram"),
            with_data,
            stats.len(),
            if not_derivable.is_empty() {
                "none".to_string()
            } else {
                not_derivable.join(", ")
            }
        )
    }
}
