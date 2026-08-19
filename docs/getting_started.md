# Getting Started with OpenHeart (SCPG Engine)

Welcome to **OpenHeart**, an advanced static program analysis engine and bidirectional UML generation platform designed, created, and maintained solely by **Ahmad Hassan (B-Ted)**.

---

## 🚀 Live Web Studio Portal (GitHub Pages)

Experience real-time interactive 14 UML diagram generation directly in your browser:

👉 **[Launch OpenHeart Web Studio Portal](https://ahmadhassan-bted.github.io/OpenHeart/)**

Paste any public GitHub repository URL (`https://github.com/owner/repo`) to select from all 14 UML diagram types, render live interactive Mermaid diagrams, and explore graph structures online.

---

## Prerequisites

- **Rust Toolchain**: Rust 1.75+ (edition 2021)
- **Cargo**: Included with standard Rust toolchain
- **Git**: v2.20 or newer
- **Python**: 3.10+ (for running the `ruthless_verify.py` verification test harness)

---

## Local Setup & Quickstart

1. **Clone the Repository**:
   ```bash
   git clone https://github.com/AhmadHassan-BTed/OpenHeart.git
   cd OpenHeart
   ```

2. **Build the Engine Binary**:
   ```bash
   cargo build --release
   ```

3. **Run Pipeline Analysis via CLI**:
   ```bash
   target/release/openheart analyze <PATH_TO_SOURCE_PROJECT> <OUTPUT_DIRECTORY>
   ```

4. **Launch Local HTTP REST API & Web Studio Backend**:
   ```bash
   target/release/openheart server 8080
   ```
   Open `http://localhost:8080` in your web browser to access the local OpenHeart Web Studio.

5. **Run Full Rust Integration Test Suite**:
   ```bash
   cargo test
   ```

6. **Run Ruthless Multi-Repo Ground-Truth Verification**:
   ```bash
   python3 ruthless_verify.py
   ```

---

## Documentation Index

- **[Live Web Studio](https://ahmadhassan-bted.github.io/OpenHeart/)**: Deployed interactive portal on GitHub Pages.
- **[System Overview](overview.md)**: Formal mathematical definitions, comparative analysis, and succinct data structures.
- **[System Architecture](architecture.md)**: 10-Phase pipeline design and 14 UML diagram mappings.
- **[Codebase Guide](codebase-guide.md)**: Repository layout, module structure, and internal developer reference.
- **[Succinct Structures](succinct_structures.md)**: BP ASTs, CSR CFGs, Wavelet Trees & ROBDD proof specs.
- **[UML Derivation](uml_derivation.md)**: Derivation rules for structural & behavioral diagrams.
- **[Architectural Roadmap](../ROADMAP.md)**: Complete 10-phase SCPG compilation pipeline overview.
