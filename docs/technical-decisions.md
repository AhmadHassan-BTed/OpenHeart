# Technical Decisions & Architecture Decision Records (ADRs)

This document records the major architectural choices, design trade-offs, and engineering rationales in **OpenHeart**.

👉 **[Launch OpenHeart Web Studio Portal (GitHub Pages)](https://ahmadhassan-bted.github.io/OpenHeart/)**

---

## ADR-001: 64-bit FNV-1a Hash Function for String Interning

- **Context**: Token text in codebases is dominated by short strings (1–30 characters: keywords, identifiers, operators).
- **Decision**: Choose 64-bit FNV-1a over SipHash.
- **Rationale**: FNV-1a has zero state initialization overhead, avalanche-free distribution for short ASCII strings, and linear scan efficiency. SipHash adds key initialization overhead that slows down single-pass token interning.

---

## ADR-002: Monotonic 32-bit `token_id` as Universal Traceability Anchor

- **Context**: Upstream analysis layers (AST, CFG, DFG, UML diagrams) need $O(1)$ mapping back to exact source file positions.
- **Decision**: Assign a single monotonic `u32` `token_id` during Phase 1 scanner traversal and mandate that no upstream phase ever re-assigns or re-interprets a `token_id`.
- **Rationale**: Eliminates string/position pointer chasing. Allows all upper layers to inherit source spans by storing `[min_token_id, max_token_id]` ranges.

---

## ADR-003: 16-Byte Cache-Line Aligned `TokenRecord` Layout

- **Context**: Forward index search requires binary search over millions of token records.
- **Decision**: Pack `sort_key: u64`, `text_id: u32`, `len: u16`, `token_type: u8`, `_padding: u8` into a 16-byte `#[repr(C)]` struct.
- **Rationale**: Exactly 4 `TokenRecord` instances fit in a single 64-byte L1 cache line. Binary search midpoint comparisons touch minimal cache lines, achieving $\approx 1 \text{ ns}$ per comparison step.

---

## ADR-004: CRC-64/ECMA Verification for `.tca` Binary Artifacts

- **Context**: Inter-phase binary artifacts must be validated against corruption before ingestion by Phase 2.
- **Decision**: Append a 64-bit CRC-64/ECMA polynomial checksum (`0x42F0E1EBA9EA3693`) to the end of every `.tca` file.
- **Rationale**: Provides fast hardware-friendly verification of binary integrity before allocation or parsing.
