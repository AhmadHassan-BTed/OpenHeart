# Getting Started with OpenHeart (SCPG)

Welcome to **OpenHeart**, created and maintained by **Ahmad Hassan (B-Ted)**. This guide details local workspace setup and repository navigation.

---

## Prerequisites

- **Rust Toolchain**: Rust 1.75+ (edition 2021)
- **Cargo**: Included with Rust toolchain
- **Git**: v2.20 or newer

---

## Workspace Setup

1. **Clone the Repository**:
   ```bash
   git clone https://github.com/AhmadHassan-BTed/OpenHeart.git
   cd OpenHeart
   ```

2. **Verify Build**:
   ```bash
   cargo check
   ```

3. **Run Automated Test Suite**:
   ```bash
   cargo test
   ```

---

## Documentation Index

- **[System Overview](overview.md)**: Formal mathematical definitions, comparative analysis, and succinct data structures.
- **[System Architecture](architecture.md)**: 5-Phase pipeline design and 14 UML diagram mappings.
- **[Succinct Structures](succinct_structures.md)**: BP ASTs, CSR CFGs, Wavelet Trees & ROBDD proof specs.
- **[UML Derivation](uml_derivation.md)**: Derivation rules for structural & behavioral diagrams.
- **[Phase 1 Implementation Plan](plans/phase1_ingestion_spec.md)**: Full technical specification for Phase 1 (Lexical Ingestion & Token Corpus Construction).
