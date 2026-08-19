<div align="center">

# OpenHeart

<p align="center">
  <img src="docs/assets/web_preview.webp" alt="OpenHeart Live Web Studio Studio Preview" width="100%" />
</p>

### Succinct Compositional Program Graph (SCPG) Engine

[![Language: Rust](https://img.shields.io/badge/language-Rust_1.75+-orange.svg?style=flat-square&logo=rust)](https://www.rust-lang.org/)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg?style=flat-square)](LICENSE)
[![Live Studio](https://img.shields.io/badge/Live_Studio-GitHub_Pages-success.svg?style=flat-square&logo=github)](https://ahmadhassan-bted.github.io/OpenHeart/)
[![CI Pipeline](https://img.shields.io/badge/CI-passing-brightgreen.svg?style=flat-square)](https://github.com/AhmadHassan-BTed/OpenHeart/actions)
[![Security Policy](https://img.shields.io/badge/security-enforced-success.svg?style=flat-square)](SECURITY.md)
[![Maintainer: Ahmad Hassan (B-Ted)](https://img.shields.io/badge/maintainer-Ahmad_Hassan_(B--Ted)-blueviolet.svg?style=flat-square)](https://github.com/AhmadHassan-BTed)

<br/>

<p align="center">
  <img src="docs/assets/scpg_overview.svg" alt="OpenHeart SCPG Architecture" width="100%" />
</p>

<p align="center">
  A high-performance static program analysis engine and bidirectional UML generation platform built on succinct data structures, memory-mapped binary layouts, and formal graph representations.
</p>

</div>

---

## System Philosophy & Human Intent

Software codebases are living artifacts of human intellect designed to express domain logic, structural design, and operational intent. Traditional program analysis frameworks—such as pointer-heavy Code Property Graphs or lowered compiler IRs—strip away this high-level context, introducing massive memory inflation and pointer-chasing latency that isolate static analysis from real-time developer workflows.

Designed, created, and maintained solely by **Ahmad Hassan (B-Ted)**, **OpenHeart** bridges this gap by introducing the **Succinct Compositional Program Graph (SCPG)**—a formal 7-tuple graph representation:

$$\mathcal{G} = (V, E, \nu, \varepsilon, \tau, \rho, \Sigma_\Phi)$$

By combining Succinct Balanced Parentheses (BP) trees, Compressed Sparse Row (CSR) control flow graphs, Static Single-Assignment (SSA) data flow representations, and Reduced Ordered Binary Decision Diagram (ROBDD) path summaries, OpenHeart achieves up to **128× memory compression** while maintaining **$O(1)$ bidirectional source-to-diagram traceability**.

---

## Key Technical Features

- **128× Memory Compression**: Replaces pointer-heavy AST nodes with Succinct Balanced Parentheses (BP) trees and $O(1)$ Rank/Select indices.
- **$O(1)$ Universal Traceability**: Monotonic 32-bit `token_id` anchors link raw source positions directly to IR graph nodes and derived UML elements.
- **14 Native UML 2.5 Diagram Derivations**: Deterministically generates Class, Sequence, Activity, Component, State Machine, and 9 other UML diagram types directly from graph layers.
- **Zero-Copy Memory Mapping**: All 10 compilation pipeline phases serialize into CRC-64 verified binary artifacts mapped directly into OS page memory.
- **Perfect Multi-Repo Convergence**: Validated with $F_1 = 1.0000$ precision across Android, Java, Kotlin, Python, Rust, and JavaScript codebases.

---

## Master 10-Phase SCPG Compilation Pipeline

```text
Phase 1 (COMPLETED) ──► Phase 2 (COMPLETED) ──► Phase 3 (COMPLETED) ──► Phase 4 (COMPLETED) ──► Phase 5 (COMPLETED)
Lexical Ingestion       BP AST & Succinct      Symbol Table &         CFG CSR & Dominator    SSA Conversion, CDG
& Token Corpus (.tca)   Reduction (.bpa)       Hierarchy (.sta)       Tree (.cfa)            & IFDS Solver (.ssa)

        │
        ▼
Phase 6 (COMPLETED) ──► Phase 7 (COMPLETED) ──► Phase 8 (COMPLETED) ──► Phase 9 (COMPLETED) ──► Phase 10 (COMPLETED)
Inter-procedural        Traceability Index     ROBDD Path             UML Semantic           SCPG Binary (.scpg)
Call Graph (.cga)       Forward/Backward       Summaries (.psa)       Metadata (.uma)        & Query Engine
```

---

## Repository Map

```text
OpenHeart/
├── src/                          # Native Rust Engine Implementation (10 Compilation Phases)
│   ├── adapters/                 # HTTP REST API Server & Git Clone Ingestion Adapter
│   ├── ast/                      # Phase 2: Balanced Parentheses (BP) AST & Rank/Select LCA Engine
│   ├── cfg/                      # Phase 4: Control Flow Graph (CSR) & Cooper Dominators
│   ├── cg/                       # Phase 6: Call Graph & Andersen Points-To Analysis
│   ├── core/                     # Core Types, Binary Serialization Primitives & Logger
│   ├── ingestion/                # Phase 1: Tree-sitter Lexical Ingestion & Token Corpus (.tca)
│   ├── psa/                      # Phase 8: ROBDD Path Summaries & #SAT Path Counting (.psa)
│   ├── scpg/                     # Phase 10: SCPG Binary Serializer, Query Engine, & PlantUML Exporters
│   ├── ssa/                      # Phase 5: SSA Form, CDG, & IFDS Data Flow Solvers (.ssa)
│   ├── symbol/                   # Phase 3: Symbol Table & Scope Graph Engine (.sta)
│   ├── tra/                      # Phase 7: Universal Traceability Index (.tra) & UMLLinks
│   ├── uma/                      # Phase 9: UML Semantic Metadata Artifact (.uma) & Pattern Detectors
│   ├── lib.rs                    # Library Crate Root
│   └── main.rs                   # Engine CLI Entrypoint
│
├── web/                          # OpenHeart Web Studio Portal (HTML5/Vanilla CSS/JavaScript)
├── tests/                        # 68 Unit & Integration Tests across all 10 Phases
├── docs/                         # Specifications, Architecture Docs, & Interactive Spec Models
├── scripts/                      # Helper Scripts & Automation Utilities
├── Makefile                      # Convenient Targets for Build, Test, & Server Launch
├── ruthless_verify.py            # Multi-Repo Ground-Truth Pipeline Accuracy Harness
└── ruthless_config.json          # Benchmark Repository Registry Config
```

---

## Quickstart & Local Execution

### 1. Build Engine
```bash
cargo build --release
```

### 2. Run Single Codebase Analysis
```bash
target/release/openheart analyze ./my_project ./output_dir
```

### 3. Launch Web Server & Interactive Web Studio
```bash
target/release/openheart server 8080
```
Then visit `http://localhost:8080` in your web browser.

### 4. Run Integration Test Suite
```bash
cargo test
```

### 5. Run Multi-Repo Convergence Harness
```bash
python3 ruthless_verify.py
```

---

## Documentation Index

- **[System Overview](docs/overview.md)**: Formal mathematical definitions, comparative analysis, and succinct data structures.
- **[System Architecture](docs/architecture.md)**: 10-Phase pipeline design and 14 UML diagram mappings.
- **[Codebase Guide](docs/codebase-guide.md)**: Repository layout, module structure, and internal developer reference.
- **[Getting Started](docs/getting_started.md)**: Workspace setup, Web Studio usage, and command-line execution.
- **[Succinct Structures](docs/succinct_structures.md)**: BP ASTs, CSR CFGs, Wavelet Trees & ROBDD proof specs.
- **[UML Derivation](docs/uml_derivation.md)**: Derivation rules for structural & behavioral diagrams.
- **[Architectural Roadmap](ROADMAP.md)**: Complete 10-phase SCPG compilation pipeline overview.

---

## License & Author

Authored, created, and maintained by **Ahmad Hassan (B-Ted)** under the [MIT License](LICENSE).
