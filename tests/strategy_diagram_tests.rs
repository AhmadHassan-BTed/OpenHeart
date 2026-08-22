//! Test for Strategy Design Pattern implementation in PlantUML Exporter (§10.4).

use openheart::ast::{ASTStage, ASTStageInput};
use openheart::cfg::Phase4Stage;
use openheart::cg::Phase6Stage;
use openheart::core::io::mmap::MemoryMappedFile;
use openheart::ingestion::manifest::SourceManifest;
use openheart::ingestion::IngestionStage;
use openheart::ingestion::TokenCorpusArtifact;
use openheart::psa::Phase8Stage;
use openheart::scpg::diagram::export::{PlantUMLDiagramStrategy, PlantUMLExporter};
use openheart::ssa::Phase5Stage;
use openheart::symbol::{Phase3Stage, SymbolTableArtifact};
use openheart::tra::Phase7Stage;
use openheart::uma::types::UMLMetadataArtifact;
use openheart::uma::Phase9Stage;

use std::fs;
use tempfile::tempdir;

/// A custom user-defined PlantUML strategy to verify dynamic extensibility (adding a strategy).
struct CustomSubsystemDiagramStrategy;

impl PlantUMLDiagramStrategy for CustomSubsystemDiagramStrategy {
    fn diagram_type(&self) -> &'static str {
        "custom_subsystem"
    }

    fn export(
        &self,
        _uma: &UMLMetadataArtifact,
        _sta: &SymbolTableArtifact,
        _tca: &TokenCorpusArtifact,
    ) -> String {
        "@startuml\n' Custom Subsystem Architecture\n[CustomService] --> [Database]\n@enduml\n"
            .to_string()
    }
}

#[test]
fn test_plantuml_strategy_pattern_registration_and_subtraction() {
    let mut exporter = PlantUMLExporter::new();

    // 1. Verify default 14 strategies are registered
    assert!(exporter.has_strategy("class"));
    assert!(exporter.has_strategy("sequence"));
    assert!(exporter.has_strategy("activity"));
    assert!(exporter.has_strategy("statemachine"));
    assert_eq!(exporter.strategy_types().len(), 14);

    // 2. Test subtracting a strategy
    let removed = exporter.unregister_strategy("timing");
    assert!(removed.is_some());
    assert!(!exporter.has_strategy("timing"));
    assert_eq!(exporter.strategy_types().len(), 13);

    // 3. Test adding / registering a custom strategy
    exporter.register_strategy(Box::new(CustomSubsystemDiagramStrategy));
    assert!(exporter.has_strategy("custom_subsystem"));
    assert_eq!(exporter.strategy_types().len(), 14);

    // 4. Test dynamic strategy execution with real artifacts
    let dir = tempdir().unwrap();
    let src_file = dir.path().join("StrategySample.java");
    let java_code =
        "package com.example;\npublic class StrategySample {\n  public void run() {}\n}\n";
    fs::write(&src_file, java_code).unwrap();

    let manifest = SourceManifest::new(vec![src_file]);
    let tca_path = dir.path().join("corpus.tca");
    let bpa_path = dir.path().join("ast.bpa");
    let sta_path = dir.path().join("symbols.sta");
    let cfa_path = dir.path().join("cfg.cfa");
    let ssa_path = dir.path().join("ssa.ssa");
    let cga_path = dir.path().join("callgraph.cga");
    let tra_path = dir.path().join("traceability.tra");
    let psa_path = dir.path().join("paths.psa");
    let uma_path = dir.path().join("metadata.uma");

    let tca = IngestionStage::run(manifest, &tca_path).unwrap();
    let tca_bytes = fs::read(&tca_path).unwrap();

    let stage_input = ASTStageInput {
        tca: MemoryMappedFile::open(&tca_path).unwrap(),
    };
    let bpa = ASTStage::run(&stage_input, &bpa_path).unwrap();
    let bpa_bytes = fs::read(&bpa_path).unwrap();

    let sta = Phase3Stage::run(&tca, &bpa, &tca_bytes, &bpa_bytes).unwrap();
    let sta_bytes = sta.serialize();
    fs::write(&sta_path, &sta_bytes).unwrap();

    let cfa = Phase4Stage::run(&bpa, &sta, &sta_bytes, &bpa_bytes, &cfa_path).unwrap();
    let cfa_bytes = fs::read(&cfa_path).unwrap();

    let ssa = Phase5Stage::run(&bpa, &sta, &cfa, &cfa_bytes, &ssa_path).unwrap();
    let ssa_bytes = fs::read(&ssa_path).unwrap();

    let cga = Phase6Stage::run(&bpa, &sta, &cfa, &ssa, &ssa_bytes, &sta_bytes, &cga_path).unwrap();
    let tra = Phase7Stage::run(&tca, &bpa, &sta, &cfa, &ssa, &cga, &tra_path);
    let tra_bytes = fs::read(&tra_path).unwrap();
    let psa = Phase8Stage::run(&cfa, &ssa, &cga, &cfa_bytes, &psa_path);
    let uma = Phase9Stage::run(
        &tca, &bpa, &sta, &cfa, &ssa, &cga, &tra, &psa, &tra_bytes, &uma_path,
    );

    let custom_output = exporter.export("custom_subsystem", &uma, &sta, &tca);
    assert!(custom_output.is_some());
    assert!(custom_output
        .unwrap()
        .contains("[CustomService] --> [Database]"));

    let class_output = exporter.export("class", &uma, &sta, &tca);
    assert!(class_output.is_some());
    assert!(class_output.unwrap().contains("StrategySample"));
}

struct CustomMermaidStrategy;
impl openheart::scpg::diagram::export::MermaidDiagramStrategy for CustomMermaidStrategy {
    fn diagram_type(&self) -> &'static str {
        "custom_mermaid"
    }
    fn export(
        &self,
        _uma: &UMLMetadataArtifact,
        _sta: &SymbolTableArtifact,
        _tca: &TokenCorpusArtifact,
    ) -> String {
        "graph TD\n    A[CustomService] --> B[Database]\n".to_string()
    }
}

#[test]
fn test_mermaid_strategy_pattern_registration_and_subtraction() {
    let mut exporter = openheart::scpg::diagram::export::MermaidExporter::new();

    // 1. Verify 14 default strategies
    assert!(exporter.has_strategy("class"));
    assert!(exporter.has_strategy("sequence"));
    assert!(exporter.has_strategy("activity"));
    assert_eq!(exporter.strategy_types().len(), 14);

    // 2. Subtract strategy
    let removed = exporter.unregister_strategy("timing");
    assert!(removed.is_some());
    assert!(!exporter.has_strategy("timing"));
    assert_eq!(exporter.strategy_types().len(), 13);

    // 3. Add custom strategy
    exporter.register_strategy(Box::new(CustomMermaidStrategy));
    assert!(exporter.has_strategy("custom_mermaid"));
    assert_eq!(exporter.strategy_types().len(), 14);

    // 4. Test dynamic strategy execution with real artifacts
    let dir = tempdir().unwrap();
    let src_file = dir.path().join("MermaidSample.java");
    let java_code =
        "package com.example;\npublic class MermaidSample {\n  public void run() {}\n}\n";
    fs::write(&src_file, java_code).unwrap();

    let manifest = SourceManifest::new(vec![src_file]);
    let tca_path = dir.path().join("corpus.tca");
    let bpa_path = dir.path().join("ast.bpa");
    let sta_path = dir.path().join("symbols.sta");
    let cfa_path = dir.path().join("cfg.cfa");
    let ssa_path = dir.path().join("ssa.ssa");
    let cga_path = dir.path().join("callgraph.cga");
    let tra_path = dir.path().join("traceability.tra");
    let psa_path = dir.path().join("paths.psa");
    let uma_path = dir.path().join("metadata.uma");

    let tca = IngestionStage::run(manifest, &tca_path).unwrap();
    let tca_bytes = fs::read(&tca_path).unwrap();

    let stage_input = ASTStageInput {
        tca: MemoryMappedFile::open(&tca_path).unwrap(),
    };
    let bpa = ASTStage::run(&stage_input, &bpa_path).unwrap();
    let bpa_bytes = fs::read(&bpa_path).unwrap();

    let sta = Phase3Stage::run(&tca, &bpa, &tca_bytes, &bpa_bytes).unwrap();
    let sta_bytes = sta.serialize();
    fs::write(&sta_path, &sta_bytes).unwrap();

    let cfa = Phase4Stage::run(&bpa, &sta, &sta_bytes, &bpa_bytes, &cfa_path).unwrap();
    let cfa_bytes = fs::read(&cfa_path).unwrap();

    let ssa = Phase5Stage::run(&bpa, &sta, &cfa, &cfa_bytes, &ssa_path).unwrap();
    let ssa_bytes = fs::read(&ssa_path).unwrap();

    let cga = Phase6Stage::run(&bpa, &sta, &cfa, &ssa, &ssa_bytes, &sta_bytes, &cga_path).unwrap();
    let tra = Phase7Stage::run(&tca, &bpa, &sta, &cfa, &ssa, &cga, &tra_path);
    let tra_bytes = fs::read(&tra_path).unwrap();
    let psa = Phase8Stage::run(&cfa, &ssa, &cga, &cfa_bytes, &psa_path);
    let uma = Phase9Stage::run(
        &tca, &bpa, &sta, &cfa, &ssa, &cga, &tra, &psa, &tra_bytes, &uma_path,
    );

    let output = exporter.export("custom_mermaid", &uma, &sta, &tca);
    assert!(output.is_some());
    assert!(output.unwrap().contains("A[CustomService] --> B[Database]"));

    let class_output = exporter.export("class", &uma, &sta, &tca);
    assert!(class_output.is_some());
    assert!(class_output.unwrap().contains("MermaidSample"));
}
