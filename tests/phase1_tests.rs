use std::fs;
use tempfile::tempdir;

use openheart::core::types::token::{build_sort_key, unpack_sort_key, LangId, TokenType};
use openheart::phase1::adapter::java::JavaLanguageAdapter;
use openheart::phase1::adapter::LanguageAdapter;
use openheart::phase1::interner::StringInterner;
use openheart::phase1::manifest::SourceManifestBuilder;
use openheart::phase1::serializer::TokenCorpusSerializer;
use openheart::phase1::Phase1Stage;

#[test]
fn test_sort_key_packing_and_unpacking() {
    let file_id = 42u16;
    let line = 1337u32;
    let col = 80u16;

    let sort_key = build_sort_key(file_id, line, col);
    let (unpacked_file, unpacked_line, unpacked_col) = unpack_sort_key(sort_key);

    assert_eq!(unpacked_file, file_id);
    assert_eq!(unpacked_line, line);
    assert_eq!(unpacked_col, col);
}

#[test]
fn test_string_interner_deduplication_and_lookup() {
    let mut interner = StringInterner::with_capacity(16);

    let id1 = interner.intern(b"class");
    let id2 = interner.intern(b"HeartRateMonitor");
    let id3 = interner.intern(b"class");
    let id4 = interner.intern(b"bpm");

    assert_eq!(id1, id3, "Identical strings must return identical text_id");
    assert_ne!(id1, id2);
    assert_ne!(id2, id4);
    assert_eq!(interner.count(), 3);

    assert_eq!(interner.lookup_text(id1), b"class");
    assert_eq!(interner.lookup_text(id2), b"HeartRateMonitor");
    assert_eq!(interner.lookup_text(id4), b"bpm");
}

#[test]
fn test_java_adapter_mapping() {
    let adapter = JavaLanguageAdapter::new();

    assert_eq!(adapter.language_id(), LangId::Java);
    assert_eq!(adapter.file_extensions(), &["java"]);

    assert_eq!(adapter.map_node_type("class"), TokenType::Keyword);
    assert_eq!(adapter.map_node_type("identifier"), TokenType::Identifier);
    assert_eq!(
        adapter.map_node_type("decimal_integer_literal"),
        TokenType::IntegerLiteral
    );
    assert_eq!(adapter.map_node_type("+"), TokenType::Operator);
    assert_eq!(adapter.map_node_type(";"), TokenType::Punctuation);
}

#[test]
fn test_phase1_end_to_end_on_java_sample() {
    let dir = tempdir().expect("Failed to create temp dir");
    let java_file = dir.path().join("PatientHeartMonitor.java");

    let sample_code = r#"
package com.openheart.cardiac;

/**
 * Cardiac Telemetry Signal Monitor
 */
public class PatientHeartMonitor {
    private int heartRateBpm = 72;
    private double ecgVoltage = 1.25;

    public boolean isTachycardia() {
        if (heartRateBpm > 100) {
            return true;
        }
        return false;
    }
}
"#;

    fs::write(&java_file, sample_code).expect("Failed to write test java file");

    let manifest = SourceManifestBuilder::new().add_file(&java_file).build();

    let out_tca_path = dir.path().join("output.tca");

    let artifact = Phase1Stage::run(manifest, &out_tca_path).expect("Phase1Stage execution failed");

    assert_eq!(artifact.file_records.len(), 1);
    assert!(artifact.token_records.len() > 10);
    assert_eq!(artifact.token_records.len(), artifact.token_entries.len());

    // Verify .tca binary file was created
    assert!(out_tca_path.exists());

    // Verify binary deserialization & CRC-64 checksum
    let read_artifact =
        TokenCorpusSerializer::read(&out_tca_path).expect("Failed to read serialized .tca file");

    assert_eq!(
        read_artifact.file_records.len(),
        artifact.file_records.len()
    );
    assert_eq!(
        read_artifact.token_records.len(),
        artifact.token_records.len()
    );
    assert_eq!(
        read_artifact.token_entries.len(),
        artifact.token_entries.len()
    );
    assert_eq!(read_artifact.interner.count(), artifact.interner.count());

    for i in 0..read_artifact.interner.count() {
        assert_eq!(
            read_artifact.interner.lookup_text(i),
            artifact.interner.lookup_text(i),
            "Interned string mismatch at text_id {}",
            i
        );
    }

    // Check forward index sorting invariant
    for i in 1..read_artifact.token_records.len() {
        assert!(
            read_artifact.token_records[i - 1].sort_key < read_artifact.token_records[i].sort_key,
            "Forward index records must be strictly sorted by sort_key"
        );
    }
}
