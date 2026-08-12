# Getting Started with OpenHeart (SCPG)

Welcome to **OpenHeart**, designed and created by **Ahmad Hassan (B-Ted)**. This guide details workspace setup, Web Studio usage, and command-line execution.

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

---

## Local Setup & Quickstart

1. **Clone the Repository**:
   ```bash
   git clone https://github.com/AhmadHassan-BTed/OpenHeart.git
   cd OpenHeart
   ```

2. **Run Pipeline Analysis**:
   ```bash
   cargo run -- analyze <PATH_TO_YOUR_PROJECT> <OUTPUT_DIRECTORY>
   ```

3. **Run Full Test Suite**:
   ```bash
   cargo test --all-targets
   ```

---

## Documentation Index

- **[Live Web Studio](https://ahmadhassan-bted.github.io/OpenHeart/)**: Deployed interactive portal on GitHub Pages.
- **[System Overview](overview.md)**: Formal mathematical definitions, comparative analysis, and succinct data structures.
- **[System Architecture](architecture.md)**: 10-Phase pipeline design and 14 UML diagram mappings.
- **[Succinct Structures](succinct_structures.md)**: BP ASTs, CSR CFGs, Wavelet Trees & ROBDD proof specs.
- **[UML Derivation](uml_derivation.md)**: Derivation rules for structural & behavioral diagrams.
- **[Codebase Guide](codebase-guide.md)**: Repository layout, module structure, and internal developer reference.
- **[10-Phase Pipeline Overview](plans/10_phase_pipeline_overview.md)**: Complete 10-phase static analysis pipeline overview.
