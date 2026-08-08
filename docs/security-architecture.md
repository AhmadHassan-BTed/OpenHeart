# Security Architecture & Memory Safety Specification

This document details the security posture, memory safety guarantees, input sanitization boundaries, and cryptographic verification mechanisms of **OpenHeart**.

---

## 1. Zero-Unsafe Ingestion Policy

- **Safe Rust Guarantee**: Phase 1 ingestion, CST walking, FNV-1a interning, and token recording operate entirely within safe Rust boundaries (`#![forbid(unsafe_code)]` in ingestion modules).
- **Tree-sitter FFI Boundary**: Tree-sitter parser C-bindings are encapsulated within `TreeSitterParser` (`src/phase1/parser/tree_sitter.rs`), preventing raw pointer exposure to upper stages.

---

## 2. Cryptographic Integrity & Tamper Protection

- **File SHA-256 Hashing**: Every ingested source file's raw bytes are hashed using SHA-256 (`sha2` crate). The resulting 32-byte digest is stored in `SourceFileRecord.content_sha256`.
- **Source Tree Root Hash**: A master SHA-256 digest is computed over all ordered file hashes and embedded in the `.tca` binary header.
- **CRC-64/ECMA Checksum**: All `.tca` binary output files conclude with a 64-bit ECMA-182 polynomial checksum (`0x42F0E1EBA9EA3693`). Deserialization rejects any file with a checksum mismatch prior to parsing payloads.

---

## 3. Input Validation & Bounds Protection

- **Sort Key Line/Col Saturation**: Line numbers are capped to 24 bits (`0x00FFFFFF` = 16,777,215 lines) and columns to 16 bits (`0x0000FFFF` = 65,535 columns), preventing arithmetic overflow during `sort_key` packing.
- **String Interner Allocation Bounds**: Length-prefixed string storage uses 16-bit length headers (`u16::MAX` = 65,535 bytes per token text), preventing unbounded memory allocation on malformed input files.

---

## 4. Secret Sanitization & Environment Handling

- No API keys, credentials, local path assumptions, or sensitive tokens are stored in source code.
- Environment variables (`OPENHEART_MAX_MEMORY_MB`, `OPENHEART_NUM_THREADS`, `OPENHEART_ARTIFACT_DIR`) are managed via `.env` configuration with safe fallback defaults.
