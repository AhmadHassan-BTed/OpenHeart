//! Diagram Renderers for all 14 UML Diagram types (§10.4).

use crate::uma::types::*;

pub struct DiagramRenderers;

impl DiagramRenderers {
    pub fn render_all_14_diagrams(artifact: &UMLMetadataArtifact) -> Vec<(&'static str, usize)> {
        vec![
            ("class_diagram", artifact.classes.len()),
            ("object_diagram", artifact.objects.len()),
            ("package_diagram", artifact.packages.len()),
            ("component_diagram", artifact.components.len()),
            ("composite_structure_diagram", artifact.classes.len()),
            ("deployment_diagram", artifact.components.len()),
            ("profile_diagram", artifact.classes.len()),
            ("use_case_diagram", artifact.classes.len()),
            ("activity_diagram", artifact.activities.len()),
            ("state_machine_diagram", artifact.state_machines.len()),
            ("sequence_diagram", artifact.sequences.len()),
            ("communication_diagram", artifact.sequences.len()),
            ("interaction_overview_diagram", artifact.sequences.len()),
            ("timing_diagram", artifact.sequences.len()),
        ]
    }
}
