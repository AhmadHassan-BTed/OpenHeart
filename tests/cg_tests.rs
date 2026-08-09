//! Phase 6 Call Graph & Points-To Analysis Integration Tests.
//! Authored solely by Ahmad Hassan (B-Ted).

use openheart::ast::{ASTStage, ASTStageInput};
use openheart::cfg::Phase4Stage;
use openheart::cg::serializer::CGASerializer;
use openheart::cg::Phase6Stage;
use openheart::core::io::mmap::MemoryMappedFile;
use openheart::ingestion::manifest::SourceManifest;
use openheart::ingestion::IngestionStage;
use openheart::ssa::Phase5Stage;
use openheart::symbol::Phase3Stage;
use std::fs;
use tempfile::tempdir;

#[test]
fn test_phase1_to_phase6_full_pipeline_integration() {
    let dir = tempdir().unwrap();
    let src_file = dir.path().join("CallGraphTest.java");

    let java_code = r#"
package com.openheart.test;

public class CallGraphTest {
    public static void main(String[] args) {
        CallGraphTest t = new CallGraphTest();
        t.helper();
        t.recurse(5);
    }

    public void helper() {
        int x = 10;
    }

    public void recurse(int n) {
        if (n > 0) {
            recurse(n - 1);
        }
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

    assert!(cga_path.exists());
    assert!(cga.method_count > 0);
    assert!(!cga.sccs.is_empty());

    // Test deserialization round-trip
    let deserialized_cga = CGASerializer::deserialize(&cga_path).unwrap();
    assert_eq!(cga.call_site_count, deserialized_cga.call_site_count);
    assert_eq!(cga.call_edge_count, deserialized_cga.call_edge_count);
    assert_eq!(cga.sccs.len(), deserialized_cga.sccs.len());
}
