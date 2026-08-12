//! Phase 9 UML Semantic Metadata Extraction Integration & Invariant Tests (§9.7).

use openheart::ast::{ASTStage, ASTStageInput};
use openheart::cfg::Phase4Stage;
use openheart::cg::Phase6Stage;
use openheart::core::io::mmap::MemoryMappedFile;
use openheart::ingestion::manifest::SourceManifest;
use openheart::ingestion::IngestionStage;
use openheart::psa::Phase8Stage;
use openheart::ssa::Phase5Stage;
use openheart::symbol::Phase3Stage;
use openheart::tra::Phase7Stage;
use openheart::uma::serializer::UMASerializer;
use openheart::uma::Phase9Stage;

use std::fs;
use tempfile::tempdir;

#[test]
fn test_phase1_to_phase9_full_pipeline_integration_and_invariants() {
    let dir = tempdir().unwrap();
    let src_file = dir.path().join("UmlMetadataTest.java");

    let java_code = r#"
package com.openheart.test;

public class UmlMetadataTest {
    private static UmlMetadataTest instance;

    private UmlMetadataTest() {}

    public static synchronized UmlMetadataTest getInstance() {
        if (instance == null) {
            instance = new UmlMetadataTest();
        }
        return instance;
    }

    public int processData(int value) {
        int result = 0;
        if (value > 0) {
            result = value * 2;
        } else {
            result = -value;
        }
        return result;
    }
}
"#;
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

    // Phase 1
    let tca = IngestionStage::run(manifest, &tca_path).unwrap();
    let tca_bytes = fs::read(&tca_path).unwrap();

    // Phase 2
    let stage_input = ASTStageInput {
        tca: MemoryMappedFile::open(&tca_path).unwrap(),
    };
    let bpa = ASTStage::run(&stage_input, &bpa_path).unwrap();
    let bpa_bytes = fs::read(&bpa_path).unwrap();

    // Phase 3
    let sta = Phase3Stage::run(&tca, &bpa, &tca_bytes, &bpa_bytes).unwrap();
    let sta_bytes = sta.serialize();
    fs::write(&sta_path, &sta_bytes).unwrap();

    // Phase 4
    let cfa = Phase4Stage::run(&bpa, &sta, &sta_bytes, &bpa_bytes, &cfa_path).unwrap();
    let cfa_bytes = fs::read(&cfa_path).unwrap();

    // Phase 5
    let ssa = Phase5Stage::run(&bpa, &sta, &cfa, &cfa_bytes, &ssa_path).unwrap();
    let ssa_bytes = fs::read(&ssa_path).unwrap();

    // Phase 6
    let cga = Phase6Stage::run(&bpa, &sta, &cfa, &ssa, &ssa_bytes, &sta_bytes, &cga_path).unwrap();

    // Phase 7
    let tra = Phase7Stage::run(&tca, &bpa, &sta, &cfa, &ssa, &cga, &tra_path);
    let tra_bytes = fs::read(&tra_path).unwrap();

    // Phase 8
    let psa = Phase8Stage::run(&cfa, &ssa, &cga, &cfa_bytes, &psa_path);

    // Phase 9
    let uma = Phase9Stage::run(
        &tca, &bpa, &sta, &cfa, &ssa, &cga, &tra, &psa, &tra_bytes, &uma_path,
    );

    // Verify .uma binary output file creation
    assert!(uma_path.exists(), "metadata.uma file must exist");
    assert!(!uma.classes.is_empty(), "UMA must extract class records");
    assert!(
        !uma.activities.is_empty(),
        "UMA must extract activity records"
    );

    // Deserialization check
    let loaded_uma = UMASerializer::read(&uma_path).expect("UMA deserialization must succeed");
    assert_eq!(loaded_uma.format_version, uma.format_version);
    assert_eq!(loaded_uma.tra_hash, uma.tra_hash);

    // ── Invariant 1 (Class Coverage): Every public class has a ClassRecord ──
    let target_sym_id = sta.symbol(1).expect("Symbol 1 must exist").symbol_id;
    let class_rec = uma
        .classes
        .iter()
        .find(|c| c.sym_id == target_sym_id)
        .expect("Invariant 1 failure: UmlMetadataTest class record missing");
    assert_eq!(class_rec.sym_id, target_sym_id);

    // ── Invariant 2 (Activity Completeness): Methods with CFG have ActivityRecords ──
    for func_cfg in &cfa.functions {
        if !func_cfg.blocks.is_empty() {
            let has_activity = uma
                .activities
                .iter()
                .any(|a| a.function_sym_id == func_cfg.sym_id);
            assert!(
                has_activity,
                "Invariant 2 failure: Method sym_id={} has CFG body but no ActivityRecord",
                func_cfg.sym_id
            );
        }
    }

    // ── Invariant 3 (UMLLink Validity): UMLLinkRecord scpg_hash equals tra scpg_hash ──
    for class in &uma.classes {
        assert_eq!(
            class.uml_link.scpg_hash, tra.hashes.scpg_hash,
            "Invariant 3 failure: ClassRecord UMLLink scpg_hash mismatch"
        );
    }

    // ── Invariant 4 (Pattern Confidence): Pattern confidence in [0, 100] ──
    for pattern in &uma.design_patterns {
        assert!(
            pattern.confidence <= 100,
            "Invariant 4 failure: Pattern confidence > 100"
        );
    }

    // Singleton pattern detection check on UmlMetadataTest
    let has_singleton = uma.design_patterns.iter().any(|p| {
        p.class_sym == target_sym_id
            && p.pattern_kind == (openheart::uma::types::PATTERN_SINGLETON as u16)
    });
    assert!(
        has_singleton,
        "Design pattern query failed: UmlMetadataTest must be detected as Singleton"
    );
}
