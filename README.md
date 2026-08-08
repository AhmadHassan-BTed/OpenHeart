<div align="center">

# OpenHeart

### Succinct Compositional Program Graph (SCPG) Engine

[![Language: Rust](https://img.shields.io/badge/language-Rust_1.75+-orange.svg?style=flat-square&logo=rust)](https://www.rust-lang.org/)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg?style=flat-square)](LICENSE)
[![Live Studio](https://img.shields.io/badge/Live_Studio-GitHub_Pages-success.svg?style=flat-square&logo=github)](https://ahmadhassan-bted.github.io/OpenHeart/)
[![CI Pipeline](https://img.shields.io/badge/CI-passing-brightgreen.svg?style=flat-square)](https://github.com/AhmadHassan-BTed/OpenHeart/actions)
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

## Highlights & Performance

- **128× Memory Compression**: Replaces pointer-heavy AST nodes with Succinct Balanced Parentheses (BP) trees and $O(1)$ Rank/Select indices.
- **$O(1)$ Traceability**: Monotonic 32-bit `token_id` anchors link raw source positions directly to IR graph nodes and derived UML elements.
- **14 Native UML Diagrams**: Deterministically generates Class, Sequence, Activity, Component, State Machine, and 9 other UML 2.5 diagram views.
- **Zero-Copy Memory Mapping**: All 10 compilation pipeline phases serialize into CRC-64 verified binary artifacts mapped directly into memory.

---

## Documentation Quick Links

For complete technical specifications, mathematical formalisms, and interactive models, refer to the documentation suite:

- 🏗️ **[Architecture Specification](docs/architecture.md)** — 5-layer succinct storage engine, vertex subsumption lattice, and 14 UML derivation rules.
- 📐 **[Technical Overview](docs/overview.md)** — Comparative analysis vs. legacy CPGs/LLVM IR, 10-phase artifact pipeline map, and complexity bounds.
- ⚡ **[Phase 1 Specification](ImplementationPlan.md)** — Lexical ingestion, 64-bit `sort_key` packing, FNV-1a string interning, and `.tca` schema.
- 🌲 **[Phase 2 Specification](Implementation_plan2.md)** — CST reduction taxonomy, BP encoding, Jacobson rank/select, and `.bpa` schema.
- 🌐 **[Interactive Web Models](docs/architecture/scpg_architecture_diagram.html)** — Interactive browser visualizers for pipeline and graph layers.

---

## Live Web Portal Studio

OpenHeart features an interactive Web Portal Studio hosted on GitHub Pages:

👉 **[Launch OpenHeart Web Studio Portal](https://ahmadhassan-bted.github.io/OpenHeart/)**

Paste any public GitHub repository URL (`https://github.com/owner/repo`) to inspect source structure, select from all 14 UML diagram types, and view live interactive Mermaid visualizers.

---

## Developer Quickstart

```bash
# Clone repository
git clone https://github.com/AhmadHassan-BTed/OpenHeart.git
cd OpenHeart

# Run pre-flight CI checks (cargo check, fmt, test)
make ci

# Run full test suite
make test

# Launch local Web Studio server (port 8080)
make serve
```

---

## Security & Maintainer

OpenHeart enforces strict safety controls with safe Rust, cryptographic SHA-256 digests, and CRC-64 checksum verification. See [SECURITY.md](SECURITY.md) for security policy details.

Open-source software released under the **[MIT License](LICENSE)**.

Designed, authored, and maintained by **Ahmad Hassan (B-Ted)**.
