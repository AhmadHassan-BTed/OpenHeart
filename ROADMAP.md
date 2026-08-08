# OpenHeart Architectural Roadmap

This document outlines the development phases, technical milestones, and architectural goals for **OpenHeart** (Succinct Compositional Program Graph Engine).

---

## 🗺️ 5-Phase Development Plan

```text
Phase 1 (COMPLETE) ──► Phase 2 (Q3 2026) ──► Phase 3 (Q4 2026) ──► Phase 4 (Q1 2027) ──► Phase 5 (Q2 2027)
Lexical Ingestion      BP AST & Scope        CFG CSR & SSA        ROBDD Path          IFDS & 14 UML
& Token Corpus         Graphs                DFG Encoding         Summaries           Diagram Engine
```

---

### Phase 1: Lexical Ingestion & Token Corpus Construction *(Completed)*

- [x] Multi-language Tree-sitter scanner integration.
- [x] Monotonic 32-bit `token_id` allocation as universal traceability anchor.
- [x] 64-bit FNV-1a deduplicating `StringInterner`.
- [x] 16-byte `TokenRecord` forward index & `TokenEntry` backward index.
- [x] `.tca` binary file format serializer/deserializer with CRC-64/ECMA verification checksum.
- [x] Enforcement of Corpus Invariants 1–4.

---

### Phase 2: BP AST Encoding & Scope Graph Semantic Resolution *(In Progress)*

- [ ] Balanced Parentheses (BP) sequence builder ($B \in \{(, )\}^{2n_{\text{ast}}}$).
- [ ] Sadakane / Munro-Raman $O(1)$ rank/select auxiliary index (`parent`, `lca`, `subtree_size`).
- [ ] Scope graph name binding and symbol resolution engine ($V_{\text{sym}}$).
- [ ] Type hierarchy edge builder ($E^{\text{TH}}$: `TH_EXTENDS`, `TH_IMPLEMENTS`, `TH_USES`).
- [ ] Initial structural UML diagram generators (Class, Package, Component).

---

### Phase 3: CFG CSR Construction & SSA DFG Encoding

- [ ] Basic block partitioning engine ($V_{\text{bb}}$).
- [ ] Compressed Sparse Row (CSR) control flow graph adjacency encoder (`offsets`, `adj`).
- [ ] Wavelet Tree edge type classifier over $\Sigma_E$.
- [ ] Dominator tree computation via Lengauer-Tarjan algorithm.
- [ ] Static Single-Assignment (SSA) conversion with iterated dominance frontier $\phi$-functions.

---

### Phase 4: ROBDD Path Summaries & Symbolic Execution Tier

- [ ] Control flow Boolean function encoder $f_{\text{paths}}$ via Shannon expansion.
- [ ] Reduced Ordered Binary Decision Diagram (ROBDD) engine with complement-edge optimization.
- [ ] Rudell's sifting algorithm for dynamic BDD variable reordering.
- [ ] Exact #SAT path counting and path feasibility checking.
- [ ] Lazy Z3 SMT solver integration for non-linear arithmetic constraints.

---

### Phase 5: Interprocedural Analysis & Dynamic UML Diagram Engine

- [ ] Exploded supergraph construction $G^\#$.
- [ ] IFDS distributive data flow analysis tabulation.
- [ ] Abstract Interpretation over Interval and Octagon domains for loop state bounds.
- [ ] CFL-reachability query engine over call graph $E^{\text{CG}}$.
- [ ] Bidirectional $O(1)$ source-to-diagram `UMLLink` synchronization engine.
- [ ] Full rendering support for all 14 UML diagram types.
