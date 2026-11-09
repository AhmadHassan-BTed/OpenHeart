# Security Architecture & Memory Safety

This document describes the trust boundaries, memory-safety posture, artifact-integrity mechanisms, input bounds and configuration surface of **OpenHeart**. Every statement here is tied to a code location; a statement that no code backs must be removed rather than kept as an aspiration.

👉 **[Launch OpenHeart Web Studio Portal (GitHub Pages)](https://ahmadhassan-bted.github.io/OpenHeart/)**

---

## 1. Trust Boundaries

| Input | Entry point | Treat as |
|---|---|---|
| Source files under analysis | CLI `analyze`, server `POST /api/analyze` | Untrusted: arbitrary bytes, arbitrary nesting depth, arbitrary size. |
| Binary artifacts (`.tca` … `.scpg`) | Phase readers, CLI `inspect`, `MemoryMappedSCPG::open` | Untrusted unless produced locally by the same build. The readers have no fuzzing or corruption-test evidence of hardening against hostile files (see §3.2). Do not open artifacts from untrusted sources. |
| HTTP requests | `src/adapters/server.rs` | Untrusted. The server is a **development server** for local use. Do not expose it to untrusted networks. |
| Repository URLs for cloning | `src/adapters/web_repo.rs` | Untrusted. |

---

## 2. Memory Safety

- **`unsafe` inventory.** The engine's own `unsafe` code is limited to memory-mapping files through `memmap2` in `src/core/io/mmap.rs` (`MemoryMappedFile::map` is an `unsafe fn`: the caller must guarantee the file is not modified while mapped). Tree-sitter is a C library reached through the `tree-sitter` crate's FFI bindings; `src/ingestion/parser/tree_sitter.rs` wraps the parser so no raw pointer reaches later phases. List the current inventory with `grep -rn "unsafe" src --include=*.rs`.
- **History.** An earlier revision of the ROBDD library (`src/psa/bdd/`) read the node vector through a raw slice while appending to it, which is undefined behaviour once the vector reallocates. The current tree contains no `unsafe` under `src/psa/`. Regressions in that area are checked with `make miri` (Miri on nightly) and `make asan` (AddressSanitizer).
- **`#![forbid(unsafe_code)]`.** Some modules declare it (list them with `grep -rn "forbid(unsafe_code)" src`). At the audited baseline `fc218cd` no module under `src/ingestion/` declared it. Every module that contains no `unsafe` should.
- **Stack depth.** At the audited baseline `fc218cd` the CST walk and AST reduction recursed once per syntax-tree level, so deeply nested input could exhaust the stack of the calling thread. Run analysis of untrusted input on a thread with an explicit stack size.

---

## 3. Integrity & Tamper Detection

### 3.1 What is computed

- **File SHA-256 digests.** Phase 1 hashes every ingested source file with SHA-256 (`sha2` crate) and stores the digest in `SourceFileRecord.content_sha256`.
- **Source-tree digest.** Phase 1 computes a digest over the ordered per-file digests and embeds it in the `.tca` header.
- **CRC-64/ECMA trailers.** The `.tca` and the other Phase 1–9 artifacts end with a CRC-64/ECMA-182 checksum (polynomial `0x42F0E1EBA9EA3693`).
- **`.scpg` checksums.** The checksum scheme of the unified container is defined in `src/scpg/serializer/integrity.rs`.
- **UMLLink hash chain.** `src/tra/uml_link/hash_chain.rs` combines the upstream artifact checksums into the 32-bit `scpg_hash` build tag.

### 3.2 What is verified

- A CRC detects **accidental** corruption only. It is not a MAC: anyone who can modify a file can recompute its CRC. Tamper evidence would require a keyed MAC or a signature over the artifact.
At the audited baseline `fc218cd`:

- **Phase 2 did not verify the `.tca` it read**: it parsed the memory-mapped file at fixed offsets without checking the magic, the format version or the CRC.
- Artifact readers took lengths and element counts from the file and used them for slicing and allocation, so a file with a valid CRC but inconsistent lengths could cause a panic or an oversized allocation instead of an error.
- The `.scpg` header field `source_hash` was written as zeros, so a stale `.scpg` could not be detected from its header.
- The CLI `inspect` command reported an artifact as valid after matching its magic number, without a full validation.

Each item is resolved only when a corruption test shows the reader returning an error.

Hardening target: validate magic, a known major version, bounded counts and checked offsets **before** allocating; verify checksums and return an error on mismatch; write artifacts through a temporary file and an atomic rename; cover every reader with corruption tests and a fuzz target.

---

## 4. Input Validation & Bounds

- **Sort-key ranges.** A token position is packed into a 64-bit sort key with a 16-bit `file_id`, a 24-bit line and a 16-bit column (`src/core/types/token.rs`). Values are **not saturated**: `try_build_sort_key` rejects a position outside these ranges with a `SortKeyError`. Code that narrows a file id, line or column with an `as` cast instead wraps silently, which can make two distinct positions share a key; such casts are defects.
- **Interned string lengths.** The interner stores each string with a 16-bit length prefix (`u16::MAX` = 65,535 bytes per token text).
- **File size and parse time.** The ingestion manifest carries a per-file size cap and a per-file parse timeout (`DEFAULT_MAX_FILE_SIZE_BYTES` and `DEFAULT_PARSE_TIMEOUT_MICROS` in `src/core/types/source.rs`).

---

## 5. Secrets & Configuration

- No API keys, credentials or tokens are stored in the source tree. The web studio calls the GitHub REST API without authentication.
- The engine reads only the environment variables listed in [status.md §8](./status.md#8-configuration-actually-read-by-the-engine), each with the file that reads it. `.env.example` must list exactly those variables. A variable that no code reads must not be documented as a configuration knob; earlier versions of this page listed `OPENHEART_MAX_MEMORY_MB` and `OPENHEART_ARTIFACT_DIR`, which no code reads.
