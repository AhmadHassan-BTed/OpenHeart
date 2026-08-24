# OpenHeart Technical Documentation Hub

Welcome to the central documentation index for **OpenHeart** (Succinct Compositional Program Graph Engine), authored and maintained solely by **Ahmad Hassan (B-Ted)**.

👉 **[Launch Live OpenHeart Web Studio Portal (GitHub Pages)](https://ahmadhassan-bted.github.io/OpenHeart/)**

---

## 📚 Core System Specifications

| Document | Description | Focus Area |
|---|---|---|
| **[`overview.md`](./overview.md)** | Formal mathematical definitions, comparative analysis vs. Joern/LLVM/EMF, and succinct data structures. | Theory & Architecture |
| **[`architecture.md`](./architecture.md)** | Complete 10-Phase compilation pipeline design and 19 diagram native derivation matrix. | Pipeline Engine |
| **[`codebase-guide.md`](./codebase-guide.md)** | Repository layout, module structure, and internal developer reference across Rust and Web Studio. | Developer Guide |
| **[`getting_started.md`](./getting_started.md)** | Workspace setup, compilation, Web Studio usage, and command-line execution. | Onboarding |
| **[`uml_derivation.md`](./uml_derivation.md)** | Formal derivation rules for all 14 standard UML 2.5 diagram types + 5 compiler pipeline IRs. | UML & Compiler IRs |
| **[`succinct_structures.md`](./succinct_structures.md)** | Balanced Parentheses (BP) ASTs, Compressed Sparse Row (CSR) CFGs, Wavelet Trees, and ROBDD complexity proofs. | Succinct Algorithms |
| **[`traceability_and_incremental.md`](./traceability_and_incremental.md)** | Universal monotonic `token_id` forward/backward indexing and $O(1)$ delta invalidation. | Traceability & Updates |
| **[`security-architecture.md`](./security-architecture.md)** | Security model, boundary validation, and zero-panic memory guarantees. | Security & Safety |
| **[`technical-decisions.md`](./technical-decisions.md)** | Architectural Decision Records (ADRs) detailing trade-offs and design rationale. | Decision Records |
| **[`contributing.md`](./contributing.md)** | Contribution standards, code style guidelines, and pull request verification workflows. | Community |

---

## 🔬 Research & Formal Papers

- **[`openheart_research_paper.tex`](./openheart_research_paper.tex)**: Formal IEEE/ACM formatted research paper detailing succinct graph theorems, lemmas, and empirical benchmark evaluations.
- **[`RESEARCH_PAPER_NEXT_PROMPT.txt`](./RESEARCH_PAPER_NEXT_PROMPT.txt)**: Prompt specification and guidance for paper expansion and LaTeX compilation.

---

## 📁 Historical Phase Plans (`docs/plans/`)

Detailed design and implementation notes for each compilation phase:
- [`phase1_ingestion_spec.md`](./plans/phase1_ingestion_spec.md) · Phase 1: Lexical Ingestion & Token Corpus (`.tca`)
- [`phase2_ast_reduction_spec.md`](./plans/phase2_ast_reduction_spec.md) · Phase 2: CST Reduction & Balanced Parentheses AST (`.bpa`)
- [`phase3.md`](./plans/phase3.md) · Phase 3: Symbol Table & Type Hierarchy (`.sta`)
- [`phase4.md`](./plans/phase4.md) · Phase 4: Control Flow Graph & Dominators (`.cfa`)
- [`phase5.md`](./plans/phase5.md) · Phase 5: SSA Conversion & IFDS Data Flow (`.ssa`)
- [`phase6.md`](./plans/phase6.md) · Phase 6: Inter-procedural Call Graph (`.cga`)
- [`phase7.md`](./plans/phase7.md) · Phase 7: Universal Traceability Index (`.tra`)
- [`phase8.md`](./plans/phase8.md) · Phase 8: ROBDD Path Summaries (`.psa`)
- [`phase9.md`](./plans/phase9.md) · Phase 9: UML Semantic Metadata (`.uma`)
- [`phase10.md`](./plans/phase10.md) · Phase 10: SCPG Binary Serializer & Query Engine (`.scpg`)
