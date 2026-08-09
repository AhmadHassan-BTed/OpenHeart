//! Comprehensive Integration Tests for Phase 3: Symbol Table & Type Hierarchy Construction.

use openheart::ast::{ASTStage, ASTStageInput, BPASTBuilder};
use openheart::core::io::mmap::MemoryMappedFile;
use openheart::core::types::ast::ASTNodeType;
use openheart::core::types::symbol::{SymbolKind, THRelation};
use openheart::ingestion::manifest::SourceManifest;
use openheart::symbol::{
    JavaSemanticAdapter, Pass1Discovery, Phase3Stage, SymbolTableArtifact, SymbolTableBuilder,
};
use std::fs;
use tempfile::tempdir;

#[test]
fn test_phase1_to_phase2_to_phase3_full_pipeline_integration() {
    let dir = tempdir().unwrap();
    let src_path = dir.path().join("SampleService.java");

    let sample_code = r#"
package com.example.service;

import java.util.List;

public class SampleService extends BaseService implements Runnable {
    private List items;
    private int count;

    public SampleService(int count) {
        this.count = count;
    }

    public void processItems(int limit) {
        int total = limit + count;
    }

    public void run() {
    }
}

class BaseService {
}
"#;

    fs::write(&src_path, sample_code).unwrap();

    let manifest = SourceManifest::new(vec![src_path.clone()]);
    let tca_path = dir.path().join("corpus.tca");

    // ── STEP 1: Phase 1 Lexical Ingestion ──
    let tca_artifact = openheart::ingestion::IngestionStage::run(manifest, &tca_path).unwrap();
    let tca_bytes = fs::read(&tca_path).unwrap();

    assert!(
        tca_artifact.token_records.len() > 0,
        "Phase 1 must produce tokens"
    );

    // ── STEP 2: Phase 2 CST Reduction & BP AST Encoding ──
    let bpa_path = dir.path().join("ast.bpa");
    let input = ASTStageInput {
        tca: MemoryMappedFile::open(&tca_path).unwrap(),
    };
    let bpa_artifact = ASTStage::run(&input, &bpa_path).unwrap();
    let bpa_bytes = fs::read(&bpa_path).unwrap();

    assert!(
        bpa_artifact.node_count > 0,
        "Phase 2 must produce AST nodes from Phase 1 TCA"
    );

    // ── STEP 3: Phase 3 Symbol Table & Type Hierarchy Construction ──
    let sta_artifact =
        Phase3Stage::run(&tca_artifact, &bpa_artifact, &tca_bytes, &bpa_bytes).unwrap();

    // Verify Symbol Table Counts & Attributes from Phase 1 + Phase 2 integration
    assert!(
        sta_artifact.symbol_count > 0,
        "Phase 3 symbol_count should be > 0"
    );
    assert!(
        sta_artifact.scope_count > 0,
        "Phase 3 scope_count should be > 0"
    );

    // Verify Symbol Kinds Discovered
    let mut discovered_kinds = Vec::new();
    for sym in &sta_artifact.symbol_records {
        discovered_kinds.push(sym.kind);
    }
    assert!(discovered_kinds.contains(&(SymbolKind::SK_CLASS as u8)));
    assert!(discovered_kinds.contains(&(SymbolKind::SK_FIELD as u8)));
    assert!(discovered_kinds.contains(&(SymbolKind::SK_METHOD as u8)));

    // Verify Serialization Roundtrip of .sta binary artifact
    let sta_bytes = sta_artifact.serialize();
    let deserialized = SymbolTableArtifact::deserialize(&sta_bytes).unwrap();

    assert_eq!(deserialized.magic, sta_artifact.magic);
    assert_eq!(deserialized.symbol_count, sta_artifact.symbol_count);
    assert_eq!(deserialized.scope_count, sta_artifact.scope_count);
    assert_eq!(deserialized.th_edge_count, sta_artifact.th_edge_count);
    assert_eq!(deserialized.bpa_hash, sta_artifact.bpa_hash);
    assert_eq!(deserialized.tca_hash, sta_artifact.tca_hash);
}

#[test]
fn test_pass1_declaration_discovery() {
    let mut builder = SymbolTableBuilder::new();
    let adapter = JavaSemanticAdapter::new();

    // Build synthetic BP AST: Root Module -> Class Foo -> Method bar & Field count
    let mut bpa_builder = BPASTBuilder::new(16, 0x12345678);

    let mod_node = bpa_builder.open_node(ASTNodeType::NN_MODULE, 0);
    let class_node = bpa_builder.open_node(ASTNodeType::NN_CLASS_DECL, 0);
    let field_node = bpa_builder.open_node(ASTNodeType::NN_FIELD_DECL, 0);
    bpa_builder.close_node(field_node, 0, 1);

    let method_node = bpa_builder.open_node(ASTNodeType::NN_METHOD_DECL, 0);
    bpa_builder.close_node(method_node, 2, 3);

    bpa_builder.close_node(class_node, 0, 3);
    bpa_builder.close_node(mod_node, 0, 3);

    let bpa = bpa_builder.finalize();

    let tca = openheart::ingestion::TokenCorpusArtifact {
        file_records: vec![],
        token_records: vec![],
        token_entries: vec![],
        interner: openheart::ingestion::interner::StringInterner::new(),
    };

    Pass1Discovery::run(&bpa, &tca, &adapter, &mut builder);

    assert_eq!(builder.symbols.len(), 4); // module, class, field, method
    assert_eq!(builder.symbols[1].kind, SymbolKind::SK_CLASS as u8);
    assert_eq!(builder.symbols[2].kind, SymbolKind::SK_FIELD as u8);
    assert_eq!(builder.symbols[3].kind, SymbolKind::SK_METHOD as u8);

    assert_eq!(builder.symbols[2].parent_sym, 1);
    assert_eq!(builder.symbols[3].parent_sym, 1);
}

#[test]
fn test_pass5_hierarchy_acyclicity() {
    let mut builder = SymbolTableBuilder::new();

    let class_a = builder.create_symbol(openheart::core::types::symbol::SymbolRecord {
        kind: SymbolKind::SK_CLASS as u8,
        ..openheart::core::types::symbol::SymbolRecord::UNINIT
    });

    let class_b = builder.create_symbol(openheart::core::types::symbol::SymbolRecord {
        kind: SymbolKind::SK_CLASS as u8,
        ..openheart::core::types::symbol::SymbolRecord::UNINIT
    });

    // Valid hierarchy: B extends A
    builder.add_th_edge(class_b, class_a, THRelation::TH_EXTENDS);

    let bpa = BPASTBuilder::new(0, 0).finalize();
    let tca = openheart::ingestion::TokenCorpusArtifact {
        file_records: vec![],
        token_records: vec![],
        token_entries: vec![],
        interner: openheart::ingestion::interner::StringInterner::new(),
    };

    assert!(builder.verify_invariants(&bpa, &tca).is_ok());

    // Introduce cycle: A extends B
    builder.add_th_edge(class_a, class_b, THRelation::TH_EXTENDS);
    assert!(builder.verify_invariants(&bpa, &tca).is_err());
}
