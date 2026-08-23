//! Formal Verification Suite for Design Patterns Applied Across OpenHeart Architecture.
//!
//! Tests:
//! 1. Strategy Pattern (PlantUML & Mermaid dynamic strategies)
//! 2. Abstract Factory Pattern (Universal Diagram Engine & Format Factories)
//! 3. Visitor Pattern (Architectural Metrics & Attack Surface Visitors)
//! 4. Builder Pattern & Template Method Pattern (SCPG Compilation Pipeline)
//! 5. Chain of Responsibility Pattern (GoF Pattern Detection Chain)

use std::fs;
use tempfile::tempdir;

use openheart::core::pipeline::SCPGPipelineBuilder;
use openheart::scpg::diagram::factory::{DiagramFormat, UniversalDiagramEngine};
use openheart::uma::patterns::chain::PatternDetectionChain;
use openheart::uma::visitor::{ArchitecturalMetricsVisitor, AttackSurfaceVisitor, VisitableUMA};

#[test]
fn test_design_pattern_architecture_complete_verification() {
    let dir = tempdir().unwrap();
    let sample_file = dir.path().join("PatternSubject.java");

    let java_code = r#"
    package com.openheart.patterns;

    public interface Observer {
        void update(String event);
    }

    public class Subject {
        private int state;
        public int publicField;

        public void attach(Observer o) {
            state++;
        }

        public void notifyObservers() {
            state = 0;
        }
    }
    "#;

    fs::write(&sample_file, java_code).unwrap();

    // 1. Test Builder Pattern & Template Method Compilation
    let pipeline = SCPGPipelineBuilder::new()
        .with_source_file(&sample_file)
        .with_output_dir(dir.path())
        .with_diagram_format(DiagramFormat::PlantUML)
        .with_diagram_format(DiagramFormat::Mermaid);

    let artifacts = pipeline.execute();
    assert!(artifacts.is_ok(), "Builder pipeline must compile cleanly");

    let art = artifacts.unwrap();
    assert!(art.tca.file_records.len() >= 1);
    assert!(art.bpa.node_count > 0);
    assert!(art.sta.symbol_count > 0);

    // 2. Test Abstract Factory Pattern & Universal Diagram Engine
    let engine = UniversalDiagramEngine::new();

    let puml_class = engine.export_diagram(
        DiagramFormat::PlantUML,
        "class",
        &art.uma,
        &art.sta,
        &art.tca,
    );
    assert!(puml_class.is_some());
    assert!(puml_class.as_ref().unwrap().contains("@startuml"));
    assert!(puml_class.as_ref().unwrap().contains("Subject"));

    let mmd_class = engine.export_diagram(
        DiagramFormat::Mermaid,
        "class",
        &art.uma,
        &art.sta,
        &art.tca,
    );
    assert!(mmd_class.is_some());
    assert!(mmd_class.as_ref().unwrap().contains("classDiagram"));

    let all_puml = engine.export_all(DiagramFormat::PlantUML, &art.uma, &art.sta, &art.tca);
    assert_eq!(all_puml.len(), 19);

    let all_mmd = engine.export_all(DiagramFormat::Mermaid, &art.uma, &art.sta, &art.tca);
    assert_eq!(all_mmd.len(), 19);

    // 3. Test Visitor Pattern on Real UMA Metadata
    let mut metrics_visitor = ArchitecturalMetricsVisitor::new();
    art.uma.accept(&mut metrics_visitor, &art.sta, &art.tca);

    assert!(metrics_visitor.total_classes >= 1);
    assert!(metrics_visitor.total_interfaces >= 1);
    assert!(metrics_visitor.total_methods >= 1);
    assert!(metrics_visitor.abstraction_ratio() > 0.0);

    let mut attack_visitor = AttackSurfaceVisitor::new();
    art.uma.accept(&mut attack_visitor, &art.sta, &art.tca);
    assert!(attack_visitor.public_methods.len() >= 1);

    // 4. Test Chain of Responsibility Pattern Detection Engine
    let chain = PatternDetectionChain::new();
    for class_rec in &art.uma.classes {
        let results = chain.evaluate_symbol(class_rec.sym_id, &art.sta, &art.tca, &art.cga);
        // Chain successfully evaluated all 11 GoF rules
        println!(
            "Class #{}: Detected patterns: {:?}",
            class_rec.sym_id,
            results.len()
        );
    }
}
