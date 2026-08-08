# OpenHeart (SCPG) — System Architecture & 5-Phase Pipeline Guide

This document specifies the internal pipeline architecture for the **Succinct Compositional Program Graph (SCPG)** engine.

---

## 🏛️ Pipeline Architecture Overview

```text
[ Raw Source Code ] ──────────────► Phase 1: Lexical Ingestion & Token Corpus (.tca)
                                                │
[ BP AST Sequence & Scope Graphs ] ◄────────────┴── Phase 2: AST Compression & Semantic Resolution
            │
            ├─────────────────────► Phase 3: CFG CSR Construction & SSA DFG Encoding
            │                                   │
[ Interprocedural Supergraph ] ◄─────────────────┴── Phase 4: ROBDD Path Summaries & Symbolic Execution
            │
            └─────────────────────► Phase 5: IFDS Analysis & Dynamic UML Diagram Engine ──► [ 14 UML Diagrams ]
```

---

## 📋 Detailed Phase Breakdown

### Phase 1: Lexical Ingestion & Token Corpus Construction *(Implemented)*
- **Mandate**: Ingest raw source bytes, allocate monotonic `token_id` $u32$ anchors, intern strings via 64-bit FNV-1a hash table, compute file SHA-256 hashes, build forward/backward indexes, validate Invariants 1–4, and serialize to `.tca` binary format with CRC-64 verification.
- **Key Modules**: `TokenRecord` (16B), `SourceFileRecord` (64B), `StringInterner`, `JavaLanguageAdapter`, `TreeSitterParser`, `walk_cst`, `TokenCorpusBuilder`, `TokenCorpusSerializer`.

### Phase 2: BP AST Encoding & Scope Graph Semantic Resolution
- **Mandate**: Convert CST to AST, build Balanced Parentheses (BP) sequence $B \in \{(, )\}^{2n_{\text{ast}}}$, construct rank/select auxiliary structures, and resolve name bindings / type hierarchies via language-agnostic Scope Graphs.
- **Key Output**: Concise AST tree representations ($6.9\times$ compression) and type hierarchy edges $E^{\text{TH}}$.

### Phase 3: CFG Construction & SSA DFG Encoding
- **Mandate**: Partition ASTs into basic blocks, compute Compressed Sparse Row (CSR) adjacency arrays, encode edge labels into a Wavelet Tree over $\Sigma_E$, compute dominator trees (Lengauer-Tarjan), and construct Static Single-Assignment (SSA) form with $\phi$-functions.
- **Key Output**: Intraprocedural CFG ($E^{\text{CFG}}$) and SSA DFG ($E^{\text{DFG}}$).

### Phase 4: ROBDD Path Summaries & Symbolic Execution Tier
- **Mandate**: Encode intraprocedural control flow as Boolean feasibility functions $f_{\text{paths}}$ using Reduced Ordered Binary Decision Diagrams (ROBDDs). Apply Shannon expansion, complement-edge optimization, and Rudell's sifting algorithm for dynamic variable reordering.
- **Key Output**: Function-scoped ROBDD path summaries, exact #SAT path counting, and lazy Z3 SMT constraint solving.

### Phase 5: Interprocedural Analysis & Dynamic UML Diagram Generator
- **Mandate**: Build exploded supergraph $G^\#$, execute IFDS distributive data flow tabulation, run Abstract Interpretation over interval/octagon domains, resolve interprocedural path queries via CFL-reachability, and dynamically render all 14 UML diagram types with $O(1)$ source-to-diagram traceability synchronization.
- **Key Output**: 14 UML diagram views and incremental diff synchronization engine.
