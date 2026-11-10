//! UMABuilder — aggregates all diagram record vectors into UMLMetadataArtifact (§9.3).

use std::collections::HashMap;

use crate::uma::behavioral::BehavioralRecords;
use crate::uma::structural::StructuralRecords;
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

    pub fn set_objects(&mut self, objects: Vec<ObjectRecord>, links: Vec<ObjectLinkRecord>) {
        self.artifact.objects = objects;
        self.artifact.object_links = links;
    }

    pub fn set_packages(
        &mut self,
        packages: Vec<PackageRecord>,
        dependencies: Vec<PackageDependencyRecord>,
        metrics: Vec<PackageMetricsRecord>,
    ) {
        self.artifact.packages = packages;
        self.artifact.package_dependencies = dependencies;
        self.artifact.package_metrics = metrics;
    }

    pub fn set_components(&mut self, components: Vec<ComponentRecord>) {
        self.artifact.components = components;
    }

    pub fn set_composite_structures(&mut self, structures: Vec<CompositeStructureRecord>) {
        self.artifact.composite_structures = structures;
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

    pub fn set_interactions(
        &mut self,
        communications: Vec<CommunicationDiagramRecord>,
        overviews: Vec<InteractionOverviewRecord>,
    ) {
        self.artifact.communications = communications;
        self.artifact.interaction_overviews = overviews;
    }

    pub fn set_patterns(
        &mut self,
        patterns: Vec<DesignPatternRecord>,
        instances: Vec<PatternInstanceRecord>,
    ) {
        self.artifact.design_patterns = patterns;
        self.artifact.pattern_instances = instances;
    }

    pub fn set_labels(&mut self, labels: HashMap<u32, String>) {
        self.artifact.label_texts = labels;
    }

    pub fn set_structural(&mut self, s: StructuralRecords) {
        self.set_classes(s.classes);
        self.set_objects(s.objects, s.object_links);
        self.set_packages(s.packages, s.package_dependencies, s.package_metrics);
        self.set_components(s.components);
        self.set_composite_structures(s.composite_structures);
    }

    pub fn set_behavioral(&mut self, b: BehavioralRecords) {
        self.set_activities(b.activities);
        self.set_state_machines(b.state_machines);
        self.set_sequences(b.sequences);
        self.set_interactions(b.communications, b.interaction_overviews);
    }

    /// Validate cross-record invariants (§9.7) and return the artifact.
    pub fn finalize(self) -> Result<UMLMetadataArtifact, String> {
        let a = &self.artifact;
        for class_rec in &a.classes {
            if class_rec.sym_id == UNKNOWN_SYM {
                return Err("Invariant 1 violated: ClassRecord with invalid sym_id".to_string());
            }
        }
        let object_count = a.objects.len();
        for l in &a.object_links {
            if l.from_object as usize >= object_count || l.to_object as usize >= object_count {
                return Err(format!(
                    "Object link references object {} / {} beyond {} objects",
                    l.from_object, l.to_object, object_count
                ));
            }
        }
        for inst in &a.pattern_instances {
            if inst.score > 100 {
                return Err(format!(
                    "Pattern instance {} has score > 100",
                    inst.instance_id
                ));
            }
        }
        for row in &a.design_patterns {
            if row.instance_id as usize >= a.pattern_instances.len() {
                return Err(format!(
                    "Design pattern row references missing instance {}",
                    row.instance_id
                ));
            }
        }
        for (i, seq) in a.sequences.iter().enumerate() {
            for m in &seq.messages {
                let known = |l: u32| seq.lifelines.iter().any(|x| x.sym_id == l);
                if !known(m.from_lifeline) || !known(m.to_lifeline) {
                    return Err(format!(
                        "Sequence {} has a message between undeclared lifelines",
                        i
                    ));
                }
            }
        }
        if a.label_texts.contains_key(&NO_LABEL) {
            return Err("Label id 0 is reserved".to_string());
        }
        Ok(self.artifact)
    }
}
