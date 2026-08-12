# Changelog

All notable changes to the **OpenHeart** project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

👉 **[Launch OpenHeart Web Studio Portal (GitHub Pages)](https://ahmadhassan-bted.github.io/OpenHeart/)**

## [1.0.0] - 2026-08-13

### Added
- **10-Phase Pipeline Engine**: Completed all 10 analysis engine phases (Lexical Ingestion, BP AST, Symbol Table, CFG CSR, SSA Form, Call Graph, Traceability, ROBDD Paths, UML Metadata, SCPG Binary).
- **100% Dynamic 14 UML 2.5 Diagram Exporters**: Native Mermaid exporters for all 14 standard UML diagram types (Class, Object, Component, Deployment, Package, Composite Structure, Profile, Use Case, Activity, State Machine, Sequence, Communication, Interaction Overview, Timing).
- **Zero-Hardcoding Invariant**: 100% generic, AST-driven graph construction and Lowest Common Ancestor (LCA) edge scoping algorithm across any codebase.
- **Web Studio Portal (GitHub Pages)**: Live deployed interactive portal at `https://ahmadhassan-bted.github.io/OpenHeart/`.

## [0.1.0] - 2026-08-08

### Added
- **Phase 1 Engine**: Lexical Ingestion & Token Corpus Construction in Rust.
- **Core Memory Layouts**:
  - `TokenRecord` (16-byte cache-line aligned struct: `sort_key: u64`, `text_id: u32`, `len: u16`, `token_type: u8`, `_padding: u8`).
  - `TokenEntry` (16-byte backward index struct).
  - `SourceFileRecord` (64-byte fixed binary struct).
  - Bit-packed `sort_key` (`build_sort_key` & `unpack_sort_key`) with 48-bit `file_id`, 24-bit `line`, 8-bit `col`.
- **String Interning**:
  - Deduplicating `StringInterner` using 64-bit FNV-1a hash table with open addressing, load factor limit $\alpha = 0.75$, and length-prefixed byte storage.
- **Binary Serializer**:
  - `.tca` (Token Corpus Artifact) format writer and reader with 64-byte header (`0x544F4B434F525001`), section offset table, and CRC-64/ECMA verification checksum.
- **Corpus Invariants (1–4)**:
  - Automated verification of Monotonicity, Injectivity, Completeness, and Forward-Backward Index Consistency.
- **Adapters & Parsers**:
  - Tree-sitter integration, `LanguageAdapter` trait, `JavaLanguageAdapter`, and `AdapterRegistry`.
- **Automated Testing Suite**:
  - Integration tests covering parsing, interning, sorting, binary round-trip, and invariant assertions.
