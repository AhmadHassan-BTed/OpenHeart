//! Ruthless Multi-Phase Deep Pipeline Integration & Accuracy Tests.
//! Authored by Ahmad Hassan (B-Ted).

use openheart::ast::{ASTStage, ASTStageInput};
use openheart::cfg::serializer::CFGArtifact;
use openheart::cfg::Phase4Stage;
use openheart::core::io::mmap::MemoryMappedFile;
use openheart::core::types::artifact::Artifact;
use openheart::ingestion::manifest::SourceManifest;
use openheart::ingestion::serializer::TokenCorpusSerializer;
use openheart::ingestion::IngestionStage;
use openheart::ssa::serializer::SSASerializer;
use openheart::ssa::Phase5Stage;
use openheart::symbol::serializer::SymbolTableArtifact;
use openheart::symbol::Phase3Stage;

use std::fs;
use tempfile::tempdir;

#[test]
fn test_ruthless_line_by_line_5_phase_pipeline_accuracy() {
    let dir = tempdir().unwrap();

    // 1. Create a complex, realistic multi-class Java codebase
    let file1_path = dir.path().join("OrderService.java");
    let file2_path = dir.path().join("PaymentProcessor.java");

    let file1_code = r#"
package com.openheart.service;

import com.openheart.payment.PaymentProcessor;

public class OrderService {
    private PaymentProcessor processor;
    private int orderCount;

    public OrderService(PaymentProcessor proc) {
        this.processor = proc;
        this.orderCount = 0;
    }

    public int processOrder(int orderId, int amount) {
        int status = 0;
        if (amount > 1000) {
            status = 2;
        } else {
            status = 1;
        }

        int attempt = 0;
        while (attempt < 3) {
            if (status == 1) {
                status = status + 10;
            } else {
                status = status + 20;
            }
            attempt = attempt + 1;
        }

        return status;
    }
}
"#;

    let file2_code = r#"
package com.openheart.payment;

public class PaymentProcessor {
    public boolean execute(int id, int amt) {
        if (amt <= 0) {
            return false;
        }
        return true;
    }
}
"#;

    fs::write(&file1_path, file1_code).unwrap();
    fs::write(&file2_path, file2_code).unwrap();

    let tca_path = dir.path().join("corpus.tca");
    let bpa_path = dir.path().join("ast.bpa");
    let sta_path = dir.path().join("symbols.sta");
    let cfa_path = dir.path().join("cfg.cfa");
    let ssa_path = dir.path().join("ssa.ssa");

    // ── PHASE 1: Lexical Ingestion ──
    let manifest = SourceManifest::new(vec![file1_path, file2_path]);
    let tca_artifact = IngestionStage::run(manifest, &tca_path).unwrap();
    let tca_bytes = fs::read(&tca_path).unwrap();

    // Ruthlessly inspect Phase 1 outputs
    let read_tca = TokenCorpusSerializer::read(&tca_path).unwrap();
    assert_eq!(read_tca.format_version(), 1);
    assert_eq!(read_tca.file_records.len(), 2);
    assert!(read_tca.token_records.len() > 100);

    // Verify Invariant 1: Monotonic sort keys
    for window in read_tca.token_records.windows(2) {
        assert!(window[0].sort_key <= window[1].sort_key);
    }

    // ── PHASE 2: CST Reduction & BP AST Encoding ──
    let stage_input = ASTStageInput {
        tca: MemoryMappedFile::open(&tca_path).unwrap(),
    };
    let bpa_artifact = ASTStage::run(&stage_input, &bpa_path).unwrap();
    let bpa_bytes = fs::read(&bpa_path).unwrap();

    // Ruthlessly inspect Phase 2 outputs
    assert!(bpa_artifact.node_count > 50);

    // ── PHASE 3: Symbol Table & Type Hierarchy Construction ──
    let sta_artifact =
        Phase3Stage::run(&tca_artifact, &bpa_artifact, &tca_bytes, &bpa_bytes).unwrap();
    let sta_bytes = sta_artifact.serialize();
    fs::write(&sta_path, &sta_bytes).unwrap();

    // Ruthlessly inspect Phase 3 outputs
    let read_sta = SymbolTableArtifact::deserialize(&sta_bytes).unwrap();
    assert_eq!(read_sta.format_version, 1);
    assert!(read_sta.symbol_count >= 10);
    assert!(read_sta.scope_count >= 10);
    assert_eq!(
        read_sta.bpa_hash,
        openheart::ingestion::serializer::crc64_ecma(&bpa_bytes)
    );
    assert_eq!(
        read_sta.tca_hash,
        openheart::ingestion::serializer::crc64_ecma(&tca_bytes)
    );

    // ── PHASE 4: Control Flow Graph & Dominator Analysis ──
    let cfa_artifact = Phase4Stage::run(
        &bpa_artifact,
        &sta_artifact,
        &sta_bytes,
        &bpa_bytes,
        &cfa_path,
    )
    .unwrap();
    let cfa_bytes = fs::read(&cfa_path).unwrap();

    // Ruthlessly inspect Phase 4 outputs
    let read_cfa = CFGArtifact::deserialize(&cfa_bytes).unwrap();
    assert_eq!(read_cfa.format_version, 1);
    assert!(read_cfa.function_count >= 2);
    assert!(read_cfa.total_blocks >= 8);
    assert!(read_cfa.total_edges >= 7);

    for func in &read_cfa.functions {
        // Assert Inv 1: ENTRY is block 0
        assert_eq!(func.blocks[0].id, 0);
        assert!(func.blocks[0].is_entry);

        // Assert Inv 1: Unique EXIT block exists
        let exit_blocks = func.blocks.iter().filter(|b| b.is_exit).count();
        assert_eq!(
            exit_blocks, 1,
            "Function sym_id {} must have exactly 1 EXIT block",
            func.sym_id
        );

        // Assert Inv 3: idom[0] == 0 (ENTRY dominates itself)
        assert_eq!(func.idom[0], 0);
    }

    // ── PHASE 5: SSA Conversion & Data Flow Graph Construction ──
    let ssa_artifact = Phase5Stage::run(
        &bpa_artifact,
        &sta_artifact,
        &cfa_artifact,
        &cfa_bytes,
        &ssa_path,
    )
    .unwrap();

    // Ruthlessly inspect Phase 5 outputs
    let read_ssa = SSASerializer::read(&ssa_path).unwrap();
    assert_eq!(read_ssa.format_version, 1);
    assert_eq!(
        read_ssa.cfa_hash,
        openheart::ingestion::serializer::crc64_ecma(&cfa_bytes)
    );
    assert_eq!(read_ssa.function_count, cfa_artifact.function_count);
    assert!(read_ssa.total_ssa_vars >= 4);

    // Verify SSA invariants across functions
    for func_ssa in &ssa_artifact.functions {
        // Verify Invariant 1: Single assignment per SSA variable
        let mut ssa_ids = std::collections::HashSet::new();
        for ssa in &func_ssa.ssa_records {
            assert!(
                ssa_ids.insert(ssa.ssa_id),
                "SSA variable v{} has duplicate definition in sym_id={}",
                ssa.ssa_id,
                func_ssa.sym_id
            );
        }

        // Verify Invariant 3: Def-Use CSR consistency
        let offsets = &func_ssa.def_use.def_offsets;
        let adj = &func_ssa.def_use.use_adj;
        assert_eq!(offsets.len(), func_ssa.ssa_records.len() + 1);
        assert_eq!(*offsets.last().unwrap() as usize, adj.len());

        // Verify CDG offsets consistency
        assert!(!func_ssa.cdg.cd_offsets.is_empty());
    }
}
