# OpenHeart Technical Documentation Hub

Welcome to the central documentation index for **OpenHeart** (Succinct Compositional Program Graph Engine), authored and maintained by **Ahmad Hassan (B-Ted)**.

👉 **[Launch Live OpenHeart Web Studio Portal (GitHub Pages)](https://ahmadhassan-bted.github.io/OpenHeart/)**

> **Design versus implementation.** Most documents here describe the SCPG *design*. What the reference implementation actually does — per phase, per language and per diagram — is recorded in **[`status.md`](./status.md)**. Where a design document and `status.md` disagree, `status.md` describes the code.

---

## 📚 Core System Specifications

| Document | Description | Focus Area |
|---|---|---|
| **[`status.md`](./status.md)** | Implementation status per phase, language support matrix, diagram provenance matrix, claims ledger and measurement protocol. | Source of Truth |
| **[`overview.md`](./overview.md)** | Formal model of the SCPG, design goals, and positioning against other program representations. | Theory & Architecture |
| **[`architecture.md`](./architecture.md)** | 10-phase pipeline design and the 19-view derivation matrix. | Pipeline Engine |
| **[`codebase-guide.md`](./codebase-guide.md)** | Repository layout, module structure and internal developer reference across Rust and the web studio. | Developer Guide |
| **[`getting_started.md`](./getting_started.md)** | Workspace setup, compilation, web studio usage and command-line execution. | Onboarding |
| **[`uml_derivation.md`](./uml_derivation.md)** | Derivation rules for the 14 UML view types and 5 compiler-IR views, with per-view implementation status. | UML & Compiler IRs |
| **[`succinct_structures.md`](./succinct_structures.md)** | Balanced Parentheses trees, CSR graphs, interning and ROBDDs: textbook bounds and what the code implements. | Data Structures |
| **[`traceability_and_incremental.md`](./traceability_and_incremental.md)** | `token_id` forward/backward indexing (implemented) and the incremental synchronization design (planned). | Traceability & Updates |
| **[`security-architecture.md`](./security-architecture.md)** | Trust boundaries, artifact integrity, input bounds, `unsafe` inventory and the configuration the engine reads. | Security & Safety |
| **[`technical-decisions.md`](./technical-decisions.md)** | Architecture Decision Records (ADRs) with context, decision and consequences. | Decision Records |
| **[`contributing.md`](./contributing.md)** | Contribution standards, quality gates and the documentation status rule. | Community |

---

## 🔬 Research Paper

- **[`openheart_research_paper.tex`](./openheart_research_paper.tex)**: IEEE-format paper presenting the SCPG as a formal framework with analytical bounds. It contains no empirical evaluation; its "Reference Implementation Status" section lists where the shipped code departs from the model.

---

## 📁 Phase Design Plans (`docs/plans/`)

Design specifications written before or during implementation. Each plan carries a status banner; implementation status lives in [`status.md`](./status.md).

- [`10_phase_pipeline_overview.md`](./plans/10_phase_pipeline_overview.md) · Pipeline overview
- [`phase1_ingestion_spec.md`](./plans/phase1_ingestion_spec.md) · Phase 1: Lexical Ingestion & Token Corpus (`.tca`)
- [`phase2_ast_reduction_spec.md`](./plans/phase2_ast_reduction_spec.md) · Phase 2: CST Reduction & Balanced Parentheses AST (`.bpa`)
- [`phase3.md`](./plans/phase3.md) · Phase 3: Symbol Table & Type Hierarchy (`.sta`)
- [`phase4_cfg_dominator_analysis.html`](./plans/phase4_cfg_dominator_analysis.html) · Phase 4: Control Flow Graph & Dominators (`.cfa`)
- [`phase5.md`](./plans/phase5.md) · Phase 5: SSA Conversion & Data Flow (`.ssa`)
- [`phase6.md`](./plans/phase6.md) · Phase 6: Inter-procedural Call Graph (`.cga`)
- [`phase7.md`](./plans/phase7.md) · Phase 7: Traceability Index (`.tra`)
- [`phase8.md`](./plans/phase8.md) · Phase 8: ROBDD Path Summaries (`.psa`)
- [`phase9.md`](./plans/phase9.md) · Phase 9: UML Semantic Metadata (`.uma`)
- [`phase10.md`](./plans/phase10.md) · Phase 10: SCPG Container & Query Engine (`.scpg`)
