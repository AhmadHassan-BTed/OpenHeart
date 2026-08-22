//! Comprehensive Integration & Unit Tests for Phase 4: Control Flow Graph & Dominator Analysis.

use openheart::ast::{ASTStage, ASTStageInput};
use openheart::cfg::analysis::{
    compute_dominance_frontiers, compute_idom_cooper, reverse_postorder,
};
use openheart::cfg::serializer::CFGArtifact;
use openheart::cfg::Phase4Stage;
use openheart::core::io::mmap::MemoryMappedFile;
use openheart::ingestion::manifest::SourceManifest;
use openheart::ingestion::IngestionStage;
use openheart::symbol::Phase3Stage;
use std::fs;
use tempfile::tempdir;

#[test]
fn test_phase1_to_phase4_full_pipeline_integration() {
    let dir = tempdir().unwrap();
    let src_path = dir.path().join("Calculator.java");

    let sample_code = r#"
package com.example.math;

public class Calculator {
    public int compute(int n) {
        int sum = 0;
        if (n > 0) {
            for (int i = 0; i < n; i++) {
                if (i % 2 == 0) {
                    sum += i;
                } else {
                    sum += 1;
                }
            }
        } else {
            sum = -1;
        }
        return sum;
    }
}
"#;

    fs::write(&src_path, sample_code).unwrap();

    let manifest = SourceManifest::new(vec![src_path.clone()]);
    let tca_path = dir.path().join("corpus.tca");
    let bpa_path = dir.path().join("ast.bpa");
    let sta_path = dir.path().join("symbols.sta");
    let cfa_path = dir.path().join("cfg.cfa");

    // ── STEP 1: Phase 1 Lexical Ingestion ──
    let tca_artifact = IngestionStage::run(manifest, &tca_path).unwrap();
    let tca_bytes = fs::read(&tca_path).unwrap();

    // ── STEP 2: Phase 2 CST Reduction & BP AST Encoding ──
    let input = ASTStageInput {
        tca: MemoryMappedFile::open(&tca_path).unwrap(),
    };
    let bpa_artifact = ASTStage::run(&input, &bpa_path).unwrap();
    let bpa_bytes = fs::read(&bpa_path).unwrap();

    // ── STEP 3: Phase 3 Symbol Table & Type Hierarchy ──
    let sta_artifact =
        Phase3Stage::run(&tca_artifact, &bpa_artifact, &tca_bytes, &bpa_bytes).unwrap();
    let sta_bytes = sta_artifact.serialize();
    fs::write(&sta_path, &sta_bytes).unwrap();

    // ── STEP 4: Phase 4 Control Flow Graph Construction ──
    let cfa_artifact = Phase4Stage::run(
        &bpa_artifact,
        &sta_artifact,
        &sta_bytes,
        &bpa_bytes,
        &cfa_path,
    )
    .unwrap();

    assert!(
        cfa_artifact.function_count > 0,
        "Must build CFG for compute method"
    );
    assert!(cfa_artifact.total_blocks > 0, "Must create basic blocks");
    assert!(cfa_artifact.total_edges > 0, "Must connect CFG edges");

    let calc_func = &cfa_artifact.functions[0];
    println!("calc_func blocks len: {}", calc_func.blocks.len());
    println!("calc_func edges len: {}", calc_func.edges.len());
    println!("calc_func cyclomatic: {}", calc_func.cyclomatic);

    assert!(!calc_func.blocks.is_empty());
    assert_eq!(calc_func.idom[0], 0, "ENTRY block dominates itself");

    // Verify binary format roundtrip
    let cfa_bytes = fs::read(&cfa_path).unwrap();
    let deserialized = CFGArtifact::deserialize(&cfa_bytes).unwrap();
    assert_eq!(deserialized.magic, cfa_artifact.magic);
    assert_eq!(deserialized.function_count, cfa_artifact.function_count);
    assert_eq!(deserialized.total_blocks, cfa_artifact.total_blocks);
    assert_eq!(deserialized.total_edges, cfa_artifact.total_edges);
}

#[test]
fn test_cooper_dominators_and_frontiers_unit() {
    let succs = vec![
        vec![1, 2], // 0
        vec![3],    // 1
        vec![3],    // 2
        vec![],     // 3
    ];
    let preds = vec![
        vec![],     // 0
        vec![0],    // 1
        vec![0],    // 2
        vec![1, 2], // 3
    ];

    let rpo = reverse_postorder(4, &succs);
    let idom = compute_idom_cooper(4, &preds, &rpo);

    assert_eq!(idom[0], 0);
    assert_eq!(idom[1], 0);
    assert_eq!(idom[2], 0);
    assert_eq!(idom[3], 0);

    let df = compute_dominance_frontiers(4, &succs, &preds, &idom);
    assert!(df[0].is_empty());
    assert!(df[1].contains(&3));
    assert!(df[2].contains(&3));
}
