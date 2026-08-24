# Getting Started with OpenHeart (SCPG Engine)

Welcome to **OpenHeart**, an advanced static program analysis compiler and universal UML 2.5 platform designed, created, and maintained solely by **Ahmad Hassan (B-Ted)**.

---

## 🚀 Live Web Studio Portal (GitHub Pages)

Experience real-time interactive 19 diagram projections directly in your browser with zero installation:

👉 **[Launch OpenHeart Web Studio Portal](https://ahmadhassan-bted.github.io/OpenHeart/)**

Paste any public GitHub repository URL (`https://github.com/owner/repo`) to ingest source code, extract AST structures, render vector SVG cards, and explore codebases interactively online.

---

## 🛠️ Prerequisites

- **Rust Toolchain**: Rust 1.75+ (edition 2021)
- **Cargo**: Included with standard Rust toolchain
- **Git**: v2.20 or newer
- **Python**: 3.10+ (optional, for local static web server)

---

## 💻 Local Setup & Quickstart

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

4. **Launch Local HTTP REST API Server**:
   ```bash
   target/release/openheart server 8080
   ```

5. **Serve Web Studio Locally**:
   ```bash
   cd web
   python3 -m http.server 8000
   ```
   Open `http://localhost:8000` in your web browser to interact with the full 19-diagram suite.

6. **Run Full Rust Integration Test Suite**:
   ```bash
   cargo test --all-targets
   ```

---

## 📚 Documentation Index

- **[Live Web Studio](https://ahmadhassan-bted.github.io/OpenHeart/)**: Deployed interactive portal on GitHub Pages.
- **[System Overview](overview.md)**: Formal mathematical definitions, comparative analysis, and succinct data structures.
- **[System Architecture](architecture.md)**: 10-Phase pipeline design and 19 diagram mappings.
- **[Codebase Guide](codebase-guide.md)**: Repository layout, module structure, and internal developer reference.
- **[Succinct Structures](succinct_structures.md)**: BP ASTs, CSR CFGs, Wavelet Trees & ROBDD proof specs.
- **[UML Derivation](uml_derivation.md)**: Formal derivation rules for all 19 diagram projections.
- **[Architectural Roadmap](../ROADMAP.md)**: Complete 10-phase SCPG compilation pipeline overview.
