//! Visitor Pattern for UML Metadata Analysis & Verification (§9.5).
//!
//! Enables decoupled traversal of UML models for academic metrics computation,
//! security attack surface analysis, and architectural pattern summarization.

use std::collections::{HashMap, HashSet};

use crate::ingestion::TokenCorpusArtifact;
use crate::symbol::SymbolTableArtifact;
use crate::uma::types::*;

/// Visitor interface for decoupled analysis of UML Semantic Metadata.
pub trait UMAVisitor {
    fn visit_class(
        &mut self,
        _class: &ClassRecord,
        _sta: &SymbolTableArtifact,
        _tca: &TokenCorpusArtifact,
    ) {
    }

    fn visit_field(
        &mut self,
        _field: &FieldRecord,
        _parent: &ClassRecord,
        _sta: &SymbolTableArtifact,
        _tca: &TokenCorpusArtifact,
    ) {
    }

    fn visit_method(
        &mut self,
        _method: &MethodRecord,
        _parent: &ClassRecord,
        _sta: &SymbolTableArtifact,
        _tca: &TokenCorpusArtifact,
    ) {
    }

    fn visit_activity(
        &mut self,
        _activity: &ActivityRecord,
        _sta: &SymbolTableArtifact,
        _tca: &TokenCorpusArtifact,
    ) {
    }

    fn visit_sequence(
        &mut self,
        _sequence: &SequenceDiagramRecord,
        _sta: &SymbolTableArtifact,
        _tca: &TokenCorpusArtifact,
    ) {
    }

    fn visit_state_machine(
        &mut self,
        _sm: &StateMachineRecord,
        _sta: &SymbolTableArtifact,
        _tca: &TokenCorpusArtifact,
    ) {
    }

    fn visit_component(
        &mut self,
        _comp: &ComponentRecord,
        _sta: &SymbolTableArtifact,
        _tca: &TokenCorpusArtifact,
    ) {
    }

    fn visit_package(
        &mut self,
        _pkg: &PackageRecord,
        _sta: &SymbolTableArtifact,
        _tca: &TokenCorpusArtifact,
    ) {
    }

    fn visit_design_pattern(
        &mut self,
        _dp: &DesignPatternRecord,
        _sta: &SymbolTableArtifact,
        _tca: &TokenCorpusArtifact,
    ) {
    }
}

/// Element Trait for objects that accept UMA visitors.
pub trait VisitableUMA {
    fn accept<V: UMAVisitor>(
        &self,
        visitor: &mut V,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    );
}

impl VisitableUMA for UMLMetadataArtifact {
    fn accept<V: UMAVisitor>(
        &self,
        visitor: &mut V,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) {
        for class_rec in &self.classes {
            visitor.visit_class(class_rec, sta, tca);
            for field in &class_rec.fields {
                visitor.visit_field(field, class_rec, sta, tca);
            }
            for method in &class_rec.methods {
                visitor.visit_method(method, class_rec, sta, tca);
            }
        }

        for activity in &self.activities {
            visitor.visit_activity(activity, sta, tca);
        }

        for seq in &self.sequences {
            visitor.visit_sequence(seq, sta, tca);
        }

        for sm in &self.state_machines {
            visitor.visit_state_machine(sm, sta, tca);
        }

        for comp in &self.components {
            visitor.visit_component(comp, sta, tca);
        }

        for pkg in &self.packages {
            visitor.visit_package(pkg, sta, tca);
        }

        for dp in &self.design_patterns {
            visitor.visit_design_pattern(dp, sta, tca);
        }
    }
}

// ── Concrete Visitor 1: Architectural Metrics Analyzer ────────────────────────

/// Concrete Visitor extracting formal architectural metrics for research analysis.
#[derive(Default, Debug, Clone)]
pub struct ArchitecturalMetricsVisitor {
    pub total_classes: usize,
    pub total_interfaces: usize,
    pub total_abstract_classes: usize,
    pub total_fields: usize,
    pub total_methods: usize,
    pub total_cyclomatic_complexity: u64,
    pub total_sat_path_count: u64,
    pub pattern_frequency: HashMap<u16, usize>,
    pub inheritance_depth_max: usize,
}

impl ArchitecturalMetricsVisitor {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn abstraction_ratio(&self) -> f64 {
        if self.total_classes == 0 {
            0.0
        } else {
            (self.total_interfaces + self.total_abstract_classes) as f64 / self.total_classes as f64
        }
    }

    pub fn mean_cyclomatic_complexity(&self) -> f64 {
        if self.total_methods == 0 {
            0.0
        } else {
            self.total_cyclomatic_complexity as f64 / self.total_methods as f64
        }
    }
}

impl UMAVisitor for ArchitecturalMetricsVisitor {
    fn visit_class(
        &mut self,
        class_rec: &ClassRecord,
        _sta: &SymbolTableArtifact,
        _tca: &TokenCorpusArtifact,
    ) {
        self.total_classes += 1;
        match class_rec.stereotype {
            STEREOTYPE_INTERFACE => self.total_interfaces += 1,
            STEREOTYPE_ABSTRACT => self.total_abstract_classes += 1,
            _ => {}
        }
    }

    fn visit_field(
        &mut self,
        _field: &FieldRecord,
        _parent: &ClassRecord,
        _sta: &SymbolTableArtifact,
        _tca: &TokenCorpusArtifact,
    ) {
        self.total_fields += 1;
    }

    fn visit_method(
        &mut self,
        method: &MethodRecord,
        _parent: &ClassRecord,
        _sta: &SymbolTableArtifact,
        _tca: &TokenCorpusArtifact,
    ) {
        self.total_methods += 1;
        self.total_cyclomatic_complexity += method.cyclomatic as u64;
        self.total_sat_path_count += method.sat_count;
    }

    fn visit_design_pattern(
        &mut self,
        dp: &DesignPatternRecord,
        _sta: &SymbolTableArtifact,
        _tca: &TokenCorpusArtifact,
    ) {
        *self.pattern_frequency.entry(dp.pattern_kind).or_insert(0) += 1;
    }
}

// ── Concrete Visitor 2: Attack Surface & Security Exposure Visitor ────────────

/// Concrete Visitor mapping public entry points, mutators, and exposed API boundaries.
#[derive(Default, Debug, Clone)]
pub struct AttackSurfaceVisitor {
    pub public_methods: Vec<u32>,
    pub public_fields: Vec<u32>,
    pub controller_classes: HashSet<u32>,
}

impl AttackSurfaceVisitor {
    pub fn new() -> Self {
        Self::default()
    }
}

impl UMAVisitor for AttackSurfaceVisitor {
    fn visit_class(
        &mut self,
        class_rec: &ClassRecord,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) {
        if let Some(s) = sta.symbol(class_rec.sym_id) {
            let bytes = tca.interner.lookup_text(s.name_id);
            if let Ok(name) = std::str::from_utf8(bytes) {
                if name.ends_with("Controller")
                    || name.ends_with("Endpoint")
                    || name.ends_with("Resource")
                {
                    self.controller_classes.insert(class_rec.sym_id);
                }
            }
        }
    }

    fn visit_field(
        &mut self,
        field: &FieldRecord,
        _parent: &ClassRecord,
        _sta: &SymbolTableArtifact,
        _tca: &TokenCorpusArtifact,
    ) {
        if field.visibility == 1 {
            // Public visibility
            self.public_fields.push(field.field_sym_id);
        }
    }

    fn visit_method(
        &mut self,
        method: &MethodRecord,
        _parent: &ClassRecord,
        _sta: &SymbolTableArtifact,
        _tca: &TokenCorpusArtifact,
    ) {
        if method.visibility == 1 {
            // Public visibility
            self.public_methods.push(method.method_sym_id);
        }
    }
}
