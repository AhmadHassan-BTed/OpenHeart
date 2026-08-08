# Codebase Onboarding & Contributor Map

This guide provides a structural breakdown of the **OpenHeart** codebase for new contributors and maintainers.

---

## 📁 Source Module Breakdown

```text
src/
├── core/                         # Core Data Structures (Language-Agnostic)
│   ├── io/                       # Binary Serialization & I/O
│   │   ├── binary.rs             # LittleEndian BinaryWriter & BinaryReader
│   │   ├── mmap.rs               # MemoryMappedFile wrapper over memmap2
│   │   └── mod.rs                # I/O module exports
│   └── types/                    # Fundamental Data Types & Binary Records
│       ├── artifact.rs           # Artifact base trait
│       ├── source.rs             # SourceFileRecord (64B), SourceManifest, TokenFilter
│       ├── token.rs              # TokenRecord (16B), TokenEntry (16B), TokenType (Σ_T), build_sort_key
│       └── mod.rs                # Types module exports
│
├── phase1/                       # Phase 1: Lexical Ingestion & Token Corpus Engine
│   ├── adapter/                  # Language Adapters (Tree-sitter kind -> Σ_T)
│   │   ├── java.rs               # JavaLanguageAdapter
│   │   ├── registry.rs           # AdapterRegistry with extension auto-detection
│   │   └── mod.rs                # LanguageAdapter trait
│   ├── parser/                   # CST Parser Wrapper
│   │   ├── tree_sitter.rs        # TreeSitterParser wrapper
│   │   └── mod.rs                # CSTParser trait
│   ├── allocator.rs              # TokenIdAllocator (AtomicU32 monotonic counter)
│   ├── builder.rs                # TokenCorpusBuilder & Invariants 1-4 validation
│   ├── interner.rs               # FNV-1a 64-bit open-addressing StringInterner
│   ├── manifest.rs               # SourceManifestBuilder (lexicographical file sorting)
│   ├── serializer.rs             # TokenCorpusSerializer (.tca writer/reader & CRC-64)
│   ├── walker.rs                 # walk_cst (left-to-right DFS CST leaf token walker)
│   └── mod.rs                    # Phase1Stage::run orchestrator
│
└── lib.rs                        # Root library entry point
```

---

## 🧪 Testing Layout

```text
tests/
└── phase1_tests.rs              # End-to-end integration tests:
                                 # - sort_key packing/unpacking
                                 # - StringInterner lookup & deduplication
                                 # - JavaLanguageAdapter mapping
                                 # - Phase1Stage pipeline execution on Java code
                                 # - .tca binary serialization & CRC-64 verification
                                 # - Forward index sorting & Corpus Invariants 1-4
```

---

## 🛠️ Key Data Types Quick Reference

| Data Type | Module Path | Purpose / Size |
|---|---|---|
| `TokenRecord` | `crate::core::types::token` | 16-byte cache-line aligned forward index entry. |
| `TokenEntry` | `crate::core::types::token` | 16-byte backward index entry indexed by `token_id`. |
| `SourceFileRecord` | `crate::core::types::source` | 64-byte fixed binary record for file metadata. |
| `StringInterner` | `crate::phase1::interner` | 64-bit FNV-1a hash table with open addressing. |
| `TokenCorpusArtifact` | `crate::phase1::serializer` | Parsed token corpus containing files, records, and interner. |
