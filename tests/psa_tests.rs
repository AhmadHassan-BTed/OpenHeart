//! Phase 8 ROBDD Path Summary Computation Integration & Invariant Tests (§8.8).

use openheart::ast::{ASTStage, ASTStageInput};
use openheart::cfg::Phase4Stage;
use openheart::cg::Phase6Stage;
use openheart::core::io::mmap::MemoryMappedFile;
use openheart::ingestion::manifest::SourceManifest;
use openheart::ingestion::serializer::crc64_ecma;
use openheart::ingestion::IngestionStage;
use openheart::psa::serializer::PathSummarySerializer;
use openheart::psa::Phase8Stage;
use openheart::ssa::Phase5Stage;
use openheart::symbol::Phase3Stage;

use std::fs;
use tempfile::tempdir;

#[test]
fn test_phase1_to_phase8_full_pipeline_integration_and_invariants() {
    let dir = tempdir().unwrap();
    let src_file = dir.path().join("PathSummaryTest.java");

    let java_code = r#"
package com.openheart.test;

public class PathSummaryTest {
    public int compute(int a, int b) {
        int res = 0;
        if (a > 0) {
            res += a;
        } else {
            res -= a;
        }

        if (b > 10) {
            res *= 2;
        }

        return res;
    }

    public int simpleLoop(int n) {
        int sum = 0;
        for (int i = 0; i < n; i++) {
            sum += i;
        }
        return sum;
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
    let psa_path = dir.path().join("paths.psa");

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

    // Phase 8
    let psa = Phase8Stage::run(&cfa, &ssa, &cga, &cfa_bytes, &psa_path);

    // Verify .psa binary file creation
    assert!(psa_path.exists(), "paths.psa output file must exist");
    assert!(psa.function_count() > 0, "PSA must contain processed functions");

    // Deserialization check
    let loaded_psa = PathSummarySerializer::read(&psa_path).expect("PSA deserialization must succeed");
    assert_eq!(loaded_psa.function_count(), psa.function_count());
    assert_eq!(loaded_psa.total_nodes, psa.total_nodes);

    // ── Invariant 1 (Path Coverage): ∀f with body: sat_count >= 1 ──
    for header in &loaded_psa.function_dir {
        assert!(
            header.sat_count >= 1,
            "Invariant 1 failure: sym_id={} has sat_count=0",
            header.sym_id
        );
    }

    // ── Invariant 2 (Cyclomatic Consistency): cyclomatic == |E| - |B| + 2 ──
    for header in &loaded_psa.function_dir {
        let cfa_func = cfa
            .functions
            .iter()
            .find(|f| f.sym_id == header.sym_id)
            .expect("Function must exist in CFA");
        let expected_vg = cfa_func.cyclomatic;
        assert_eq!(
            header.cyclomatic, expected_vg,
            "Invariant 2 failure: cyclomatic complexity mismatch for sym_id={}",
            header.sym_id
        );
    }

    // ── Invariant 3 (ROBDD Canonicity): no lo==hi, no duplicate (var,lo,hi) ──
    for node_array in &loaded_psa.node_arrays {
        let mut seen = std::collections::HashSet::new();
        for node in node_array {
            if node.is_terminal() {
                continue;
            }
            assert_ne!(
                node.lo, node.hi,
                "Invariant 3 failure: Rule 1 elimination violated (lo == hi)"
            );
            let key = (node.var, node.lo, node.hi);
            assert!(
                seen.insert(key),
                "Invariant 3 failure: Rule 2 sharing violated (duplicate (var,lo,hi))"
            );
        }
    }

    // ── Invariant 4 (PSA -> CFA Hash Chain): psa.cfa_hash == crc64(cfa_bytes) ──
    let expected_cfa_hash = crc64_ecma(&cfa_bytes);
    assert_eq!(
        loaded_psa.cfa_hash, expected_cfa_hash,
        "Invariant 4 failure: PSA -> CFA hash link mismatch"
    );
}
