//! DiagramGenerator — top-level UML diagram generation and multi-format export interface (§10.4).

pub mod export;
pub mod renderers;

pub use export::json::JSONExporter;
pub use export::mermaid::MermaidExporter;
pub use export::plantuml::PlantUMLExporter;
pub use export::xmi::XMIExporter;
pub use renderers::DiagramRenderers;

use crate::uma::types::UMLMetadataArtifact;

pub struct DiagramGenerator;

impl DiagramGenerator {
    pub fn generate_summary(uma: &UMLMetadataArtifact) -> String {
        let stats = DiagramRenderers::render_all_14_diagrams(uma);
        format!(
            "Generated all 14 UML Diagram Types: {} classes, {} activities, {} sequence scenarios, {} state machines.",
            stats[0].1, stats[8].1, stats[10].1, stats[9].1
        )
    }
}
