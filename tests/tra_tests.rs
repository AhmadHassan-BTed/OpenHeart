//! Phase 7 Traceability Index Construction Integration Tests (§7.8).
//! Authored solely by Ahmad Hassan (B-Ted).

use openheart::ast::{ASTStage, ASTStageInput};
use openheart::cfg::Phase4Stage;
use openheart::cg::Phase6Stage;
use openheart::core::io::mmap::MemoryMappedFile;
use openheart::ingestion::manifest::SourceManifest;
use openheart::ingestion::IngestionStage;
use openheart::ssa::Phase5Stage;
use openheart::symbol::Phase3Stage;
use openheart::tra::forward::SymbolSpanIndex;
use openheart::tra::serializer::TraceabilitySerializer;
use openheart::tra::Phase7Stage;

use std::fs;
use tempfile::tempdir;

#[test]
fn test_phase1_to_phase7_full_pipeline_integration() {
    let dir = tempdir().unwrap();
    let src_file = dir.path().join("TraceabilityTest.java");

    let java_code = r#"
package com.openheart.test;

public class TraceabilityTest {
    private int field;

    public TraceabilityTest(int val) {
        this.field = val;
    }

    public int getField() {
        return this.field;
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

    assert!(tra_path.exists());
    assert_eq!(tra.bi_ast.len(), bpa.node_count as usize);
    assert_eq!(tra.bi_sym.len(), sta.symbol_count as usize);
    assert!(!tra.uml_links.is_empty());
    assert!(!tra.sym_span.is_empty());

    // Round-trip deserialization
    let deserialized_tra = TraceabilitySerializer::deserialize(&tra_path).unwrap();
    assert_eq!(tra.hashes.scpg_hash, deserialized_tra.hashes.scpg_hash);
    assert_eq!(tra.uml_links.len(), deserialized_tra.uml_links.len());
    assert_eq!(tra.sym_span.len(), deserialized_tra.sym_span.len());

    // Forward query check
    let first_sym = &tra.sym_span[0];
    let query_res = SymbolSpanIndex::forward_sym_query(
        first_sym.first_token_id,
        first_sym.file_id,
        &tra.sym_span,
    );
    assert!(query_res.contains(&first_sym.sym_id));
}
