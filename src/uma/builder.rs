//! UMABuilder — aggregates all diagram record vectors into UMLMetadataArtifact (§9.3).

use crate::uma::types::*;

pub struct UMABuilder {
    artifact: UMLMetadataArtifact,
}

impl UMABuilder {
    pub fn new(tra_hash: u64) -> Self {
        Self {
            artifact: UMLMetadataArtifact::new(tra_hash),
        }
    }

    pub fn set_classes(&mut self, classes: Vec<ClassRecord>) {
        self.artifact.classes = classes;
    }

    pub fn set_objects(&mut self, objects: Vec<ObjectRecord>) {
        self.artifact.objects = objects;
    }

    pub fn set_packages(&mut self, packages: Vec<PackageRecord>) {
        self.artifact.packages = packages;
    }

    pub fn set_components(&mut self, components: Vec<ComponentRecord>) {
        self.artifact.components = components;
    }

    pub fn set_activities(&mut self, activities: Vec<ActivityRecord>) {
        self.artifact.activities = activities;
    }

    pub fn set_state_machines(&mut self, state_machines: Vec<StateMachineRecord>) {
        self.artifact.state_machines = state_machines;
    }

    pub fn set_sequences(&mut self, sequences: Vec<SequenceDiagramRecord>) {
        self.artifact.sequences = sequences;
    }

    pub fn set_patterns(&mut self, patterns: Vec<DesignPatternRecord>) {
        self.artifact.design_patterns = patterns;
    }

    pub fn finalize(self) -> UMLMetadataArtifact {
        // Assert Invariant 1 (§9.7): Every ClassRecord has valid structure
        #[cfg(debug_assertions)]
        {
            for class_rec in &self.artifact.classes {
                assert!(class_rec.sym_id != u32::MAX, "ClassRecord sym_id invalid");
            }
        }
        self.artifact
    }
}
