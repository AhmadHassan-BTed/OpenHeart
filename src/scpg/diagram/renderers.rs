//! Diagram inventory: how much each UML diagram type can show for an analysis (§10.4).

use std::collections::{BTreeSet, HashSet};

use crate::uma::types::*;

pub struct DiagramRenderers;

impl DiagramRenderers {
    /// Element count behind each of the 14 UML diagram types. `None` marks a diagram type that
    /// is not statically derivable from the UMA artifact (deployment, use case, timing), so
    /// callers never report a count for a projection that has no data behind it.
    pub fn render_all_14_diagrams(
        artifact: &UMLMetadataArtifact,
    ) -> Vec<(&'static str, Option<usize>)> {
        let classes: HashSet<u32> = artifact.classes.iter().map(|c| c.sym_id).collect();
        let mut stereotypes: BTreeSet<String> = artifact
            .pattern_instances
            .iter()
            .map(|p| pattern_kind_name(p.pattern_kind).to_string())
            .filter(|n| !n.is_empty())
            .collect();
        for class_rec in &artifact.classes {
            if class_rec.stereotype == STEREOTYPE_ANNOTATION {
                stereotypes.insert(format!("annotation#{}", class_rec.sym_id));
            }
        }
        vec![
            ("class_diagram", Some(classes.len())),
            ("object_diagram", Some(artifact.objects.len())),
            ("package_diagram", Some(artifact.packages.len())),
            ("component_diagram", Some(artifact.components.len())),
            (
                "composite_structure_diagram",
                Some(artifact.composite_structures.len()),
            ),
            ("deployment_diagram", None),
            ("profile_diagram", Some(stereotypes.len())),
            ("use_case_diagram", None),
            ("activity_diagram", Some(artifact.activities.len())),
            ("state_machine_diagram", Some(artifact.state_machines.len())),
            ("sequence_diagram", Some(artifact.sequences.len())),
            ("communication_diagram", Some(artifact.sequences.len())),
            (
                "interaction_overview_diagram",
                Some(artifact.interaction_overviews.len()),
            ),
            ("timing_diagram", None),
        ]
    }

    /// Count of one diagram type in an inventory (`None` when absent or not derivable).
    pub fn count(stats: &[(&'static str, Option<usize>)], name: &str) -> Option<usize> {
        stats.iter().find(|(n, _)| *n == name).and_then(|(_, c)| *c)
    }
}
