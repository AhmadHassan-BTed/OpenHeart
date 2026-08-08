# OpenHeart

> High-performance lexical ingestion, token corpus construction, and cardiac/code processing pipeline in Rust.

---

## 📌 Overview

**OpenHeart** is an open-source Rust project designed to provide high-performance code ingestion, concrete syntax tree (CST) tokenization, deduplicated string interning, and token corpus serialization.

Its primary engine (**Phase 1: Lexical Ingestion & Token Corpus Construction**) converts raw source text into an immutable, mathematically stable Token Corpus artifact (`.tca`) anchored by a monotonic 32-bit `token_id`.

---

## 📁 Repository Structure

```text
OpenHeart/
├── src/
│   ├── core/
│   │   ├── io/             # Binary Little-Endian reader/writer & mmap
│   │   └── types/          # TokenRecord (16B), TokenEntry (16B), SourceFileRecord (64B)
│   ├── phase1/
│   │   ├── adapter/        # LanguageAdapter trait & JavaLanguageAdapter
│   │   ├── parser/         # Tree-sitter CST parser integration
│   │   ├── allocator.rs    # Monotonic AtomicU32 TokenIdAllocator
│   │   ├── builder.rs      # Forward/Backward index builder & invariant checks
│   │   ├── interner.rs     # FNV-1a open-addressing StringInterner
│   │   ├── serializer.rs   # Binary .tca format serializer & CRC-64 verification
│   │   └── walker.rs       # Left-to-right DFS CST leaf token walker
│   └── lib.rs              # Root library exports
├── tests/
│   └── phase1_tests.rs     # Integration and end-to-end pipeline tests
├── docs/                   # Documentation & overview guides
├── Cargo.toml              # Rust crate manifest
├── ImplementationPlan.md   # Complete Phase 1 technical specification
├── .gitignore              # Git ignore rules
└── README.md               # Repository documentation
```

---

## 🚀 Building & Testing

### Prerequisites

- **Rust** (1.75+ or 2021 edition)
- **Cargo**

### Run Tests

Execute the automated test suite covering string interning, sort key packing, CST walking, binary `.tca` serialization, CRC-64 checksums, and invariant checks:

```bash
cargo test
```

### Check Build

```bash
cargo check
```

---

## 📚 Key Technical Features

1. **Cache-Line Aligned Layouts**:
   - `TokenRecord`: 16 bytes (`sort_key: u64`, `text_id: u32`, `len: u16`, `token_type: u8`, `_padding: u8`).
   - `SourceFileRecord`: 64 bytes fixed binary structure.
2. **64-bit FNV-1a String Interning**:
   - Deduplicates all token string text into contiguous length-prefixed storage with $O(1)$ amortized open-addressing lookup.
3. **Mathematical Corpus Invariants**:
   - Enforces Monotonicity, Injectivity, Completeness, and Forward-Backward Index Alignment prior to serialization.
4. **Binary `.tca` Output**:
   - Serializes Token Corpus Artifacts with a 64-byte header, file registry, path table, forward/backward indexes, string tables, and a trailing CRC-64/ECMA verification checksum.

---

## 📄 License

Licensed under the MIT License.
