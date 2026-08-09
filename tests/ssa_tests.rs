//! Integration and Unit Tests for Phase 5 SSA Conversion and Data Flow Graph.
//! Authored by Ahmad Hassan (B-Ted).

use openheart::ast::{ASTStage, ASTStageInput};
use openheart::cfg::Phase4Stage;
use openheart::core::io::mmap::MemoryMappedFile;
use openheart::ingestion::manifest::SourceManifest;
use openheart::ingestion::IngestionStage;
use openheart::ssa::serializer::SSASerializer;
use openheart::ssa::Phase5Stage;
use openheart::symbol::Phase3Stage;

use std::fs;
use tempfile::tempdir;

#[test]
fn test_phase1_to_phase5_full_pipeline_integration() {
    let dir = tempdir().unwrap();
    let sample_java = dir.path().join("Calculator.java");

    let java_code = r#"
package com.example;

public class Calculator {
    public int compute(int a, int b) {
        int res = 0;
        if (a > 0) {
            res = a + b;
        } else {
            res = b - a;
        }
        return res;
    }
}
"#;

    fs::write(&sample_java, java_code).unwrap();

    let tca_path = dir.path().join("corpus.tca");
    let bpa_path = dir.path().join("ast.bpa");
    let sta_path = dir.path().join("symbols.sta");
    let cfa_path = dir.path().join("cfg.cfa");
    let ssa_path = dir.path().join("ssa.ssa");

    // Phase 1: Lexical Ingestion
    let manifest = SourceManifest::new(vec![sample_java]);
    let tca_artifact = IngestionStage::run(manifest, &tca_path).unwrap();
    let tca_bytes = fs::read(&tca_path).unwrap();

    // Phase 2: CST Reduction & BP AST Encoding
    let stage_input = ASTStageInput {
        tca: MemoryMappedFile::open(&tca_path).unwrap(),
    };
    let bpa_artifact = ASTStage::run(&stage_input, &bpa_path).unwrap();
    let bpa_bytes = fs::read(&bpa_path).unwrap();

    // Phase 3: Symbol Table & Type Hierarchy
    let sta_artifact =
        Phase3Stage::run(&tca_artifact, &bpa_artifact, &tca_bytes, &bpa_bytes).unwrap();
    let sta_bytes = sta_artifact.serialize();
    fs::write(&sta_path, &sta_bytes).unwrap();

    // Phase 4: Control Flow Graph & Dominator Analysis
    let cfa_artifact = Phase4Stage::run(
        &bpa_artifact,
        &sta_artifact,
        &sta_bytes,
        &bpa_bytes,
        &cfa_path,
    )
    .unwrap();
    let cfa_bytes = fs::read(&cfa_path).unwrap();

    // Phase 5: SSA Conversion & Data Flow Graph
    let ssa_artifact = Phase5Stage::run(
        &bpa_artifact,
        &sta_artifact,
        &cfa_artifact,
        &cfa_bytes,
        &ssa_path,
    )
    .unwrap();

    assert_eq!(ssa_artifact.format_version, 1);
    assert!(ssa_artifact.function_count >= 1);
    assert!(ssa_artifact.total_ssa_vars >= 1);

    // Validate reading .ssa file
    let read_ssa = SSASerializer::read(&ssa_path).unwrap();
    assert_eq!(read_ssa.cfa_hash, ssa_artifact.cfa_hash);
    assert_eq!(read_ssa.function_count, ssa_artifact.function_count);
    assert_eq!(read_ssa.total_ssa_vars, ssa_artifact.total_ssa_vars);
}
