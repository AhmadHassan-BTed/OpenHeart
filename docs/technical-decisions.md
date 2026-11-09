# Technical Decisions & Architecture Decision Records (ADRs)

This document records the major architectural choices in **OpenHeart**, with the context, the decision and its consequences — including the costs. Records follow the [MADR](https://adr.github.io/madr/) structure in condensed form. A record is never deleted; when a decision changes, the old record is marked *Superseded* and points to its replacement.

👉 **[Launch OpenHeart Web Studio Portal (GitHub Pages)](https://ahmadhassan-bted.github.io/OpenHeart/)**

---

## ADR-001: 64-bit FNV-1a Hash Function for String Interning

- **Status**: Accepted.
- **Context**: Token text is dominated by short strings (keywords, identifiers, operators). Interning happens once per token during Phase 1.
- **Decision**: Use 64-bit FNV-1a rather than SipHash for the interner's table.
- **Consequences**: FNV-1a needs no key setup and is simple to implement inline. It is not resistant to hash flooding: crafted input can force long probe sequences, so the interner must not be relied on as a defence against adversarial source files. Its throughput relative to SipHash has not been benchmarked in OpenHeart.

---

## ADR-002: Monotonic 32-bit `token_id` as the Traceability Anchor

- **Status**: Accepted; the corpus-global numbering is under review (see ADR-007).
- **Context**: Upper layers (AST, CFG, SSA, UML elements) need to map back to exact source positions.
- **Decision**: Phase 1 assigns one monotonic `u32` `token_id` per token; no later phase re-assigns or re-interprets it. Upper layers store `[first_token_id, last_token_id]` ranges.
- **Consequences**: Backward traceability is a dense-array lookup. Because ids are numbered across the whole corpus, inserting one token shifts every later id, which blocks incremental reuse; the design in [traceability_and_incremental.md §3.1](./traceability_and_incremental.md#31-prerequisites) requires file-local ids.

---

## ADR-003: 16-Byte `TokenRecord` Layout

- **Status**: Accepted.
- **Context**: The forward index is a binary search over all token records.
- **Decision**: Pack `sort_key: u64`, `text_id: u32`, `len: u16`, `token_type: u8`, `_padding: u8` into a 16-byte `#[repr(C)]` struct.
- **Consequences**: Four records share a 64-byte cache line, which keeps the records touched by a binary search compact. No per-comparison latency has been measured.

---

## ADR-004: CRC-64/ECMA Trailers on Phase 1–9 Artifacts

- **Status**: Accepted.
- **Context**: Phases hand artifacts to each other through files (ADR-007), so corruption must be detectable before a later phase consumes a file.
- **Decision**: End every Phase 1–9 artifact with a CRC-64/ECMA-182 checksum (polynomial `0x42F0E1EBA9EA3693`).
- **Consequences**: Detects accidental corruption only; a CRC is not a MAC and gives no tamper evidence. A checksum protects nothing unless the reader verifies it before trusting lengths in the file; Phase 2 currently does not verify the `.tca` (see [security-architecture.md §3.2](./security-architecture.md#32-what-is-verified)).

---

## ADR-005: Hand-Rolled HTTP Server over `std::net`

- **Status**: Accepted for local development use.
- **Context**: The web studio needs a local backend that runs the pipeline and serves static files. Adding an async runtime and web framework would multiply the dependency graph.
- **Decision**: Implement the server on `std::net::TcpListener` with hand-written request parsing in `src/adapters/`.
- **Consequences**: No framework dependency. The server owns every protocol concern itself — request-size limits, timeouts, header parsing, concurrency, path normalisation and origin checks — and each of these is a place where a defect becomes a security defect. The server is therefore documented as a development server not to be exposed to untrusted networks ([security-architecture.md §1](./security-architecture.md#1-trust-boundaries)).

---

## ADR-006: Hand-Rolled Logger

- **Status**: Accepted.
- **Context**: The CLI writes a session log and a persistent log next to its output, and the console level must be controllable from the environment.
- **Decision**: Implement logging in `src/core/logger.rs` instead of depending on the `log`/`tracing` ecosystem. The console level is read from `OPENHEART_LOG`, falling back to an `env_logger`-style `RUST_LOG`.
- **Consequences**: No logging dependency, but no interoperability with `log`/`tracing` subscribers either, and log output from dependencies is not captured.

---

## ADR-007: Files as the Hand-Off Between Phases

- **Status**: Accepted; reuse across runs is planned.
- **Context**: Each phase consumes the artifacts of earlier phases. Keeping phases separable makes each one testable in isolation and lets a run be inspected phase by phase.
- **Decision**: Every phase serialises exactly one artifact (`.tca` … `.uma`), and later phases read those artifacts back. Phase 10 combines them into `.scpg`.
- **Consequences**: Each artifact is a stable inspection point. The price is serialisation and re-parsing on every hand-off: readers copy arrays into memory and rebuild their indexes, so nothing is zero-copy. Because ids are corpus-global and positional (ADR-002), no artifact is reused across runs; every run recomputes all ten phases.

---

## ADR-008: Typed Errors per Layer

- **Status**: Accepted; migration in progress.
- **Context**: Early phase entry points returned `Result<_, String>`, some phases panicked on write failures, and Phase 2 skipped files it could not read without reporting them. String errors cannot be matched on, lose their source chain, and invite re-stringification with `{:?}`.
- **Decision**: Each layer defines a typed error enum with `thiserror` (for example artifact errors such as *truncated*, *bad magic*, *checksum mismatch* and *unsupported version*, and ingestion errors that carry the file path). Every stage returns `Result`; per-file failures are recorded as diagnostics rather than skipped; narrowing integer conversions use `try_from` with an explicit error. Only the CLI flattens errors into messages.
- **Consequences**: Library callers can react to specific failures. Until the migration completes, some signatures still return `String` errors.

---

## ADR-009: Pinned tree-sitter 0.20 Grammar Stack

- **Status**: Accepted; an upgrade is planned.
- **Context**: Phase 1 and Phase 2 adapters classify CST node kinds by name, so a grammar upgrade can change the node kinds the adapters see.
- **Decision**: Pin `tree-sitter` 0.20 with `tree-sitter-java` 0.20 and `tree-sitter-javascript` 0.20.
- **Consequences**: Stable node kinds, but a 2023-era grammar for current Java syntax, and no grammar at all for Kotlin or any other language (see the [language support matrix](./status.md#2-language-support-matrix)). The upgrade must regenerate or re-test the adapters' classification tables against each grammar's `node-types.json`.

---

## ADR-010: Documentation Status Rule

- **Status**: Accepted.
- **Context**: Earlier documentation described design targets as delivered features and published numbers that no script produced.
- **Decision**: [status.md](./status.md) is the source of truth for implementation status. A feature is described as implemented only with links to its module and to a test that asserts an expected result. A number appears in the documentation only if the [measurement protocol](./status.md#7-measurement-protocol) produced it and its result file is committed.
- **Consequences**: Design documents stay ambitious, but every gap between design and code is stated where a reader will meet it.
