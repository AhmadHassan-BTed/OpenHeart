# OpenHeart Architectural Roadmap

This document outlines the development phases, technical milestones, and architectural goals for **OpenHeart** (Succinct Compositional Program Graph Engine), authored and maintained solely by **Ahmad Hassan (B-Ted)**.

👉 **[Launch OpenHeart Web Studio Portal (GitHub Pages)](https://ahmadhassan-bted.github.io/OpenHeart/)**

---

## Master 10-Phase SCPG Compilation Pipeline (All 10 Phases Completed)

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

### Phase 1: Lexical Ingestion & Token Corpus Construction *(Completed)*

- [x] Multi-language Tree-sitter scanner integration.
- [x] Monotonic 32-bit `token_id` allocation as universal traceability anchor.
- [x] 64-bit FNV-1a deduplicating `StringInterner`.
- [x] 16-byte `TokenRecord` forward index & `TokenEntry` backward index.
- [x] `.tca` binary file format serializer/deserializer with CRC-64/ECMA verification checksum.
- [x] Enforcement of Corpus Invariants 1–4.

---

### Phase 2: CST Reduction & BP AST Encoding *(Completed)*

- [x] Balanced Parentheses (BP) sequence builder ($B \in \{(, )\}^{2n_{\text{ast}}}$).
- [x] Jacobson $O(1)$ rank/select auxiliary index (`parent`, `lca`, `subtree_size`).
- [x] CST reduction dropping non-semantic syntax nodes.
- [x] Token range mapping propagation per AST node.
- [x] `.bpa` binary file format serializer/deserializer with CRC-64 verification.

---

### Phase 3: Symbol Table & Type Hierarchy Construction *(Completed)*

- [x] 5-pass symbol resolution pipeline (Declaration Discovery, Scope Imports, Scope BFS, Member Types, Type Hierarchy).
- [x] Scope graph name binding engine ($V_{\text{sym}}$).
- [x] Type hierarchy edge builder ($E^{\text{TH}}$: `extends`, `implements`).
- [x] `.sta` binary file format serializer/deserializer with CRC-64 verification.

---

### Phase 4: Control Flow Graph & Dominator Analysis *(Completed)*

- [x] Basic block partitioning engine ($V_{\text{bb}}$) with reachability pruning.
- [x] Compressed Sparse Row (CSR) control flow graph adjacency encoder (`offsets`, `adj`).
- [x] Control flow statement entry edge dispatch (`if`, `while`, `for`, `do-while`, `try-catch`).
- [x] Cooper's iterative algorithm for immediate dominators (`idom[]`) and dominance frontiers ($DF[b]$).
- [x] `.cfa` binary file format serializer/deserializer with CRC-64 verification.

---

### Phase 5: SSA Conversion, CDG & IFDS Data-Flow Engine *(Completed)*

- [x] Pruned SSA backward liveness analysis fixpoint (`LiveIn`, `LiveOut`).
- [x] Cytron's dominance frontier $\phi$-function placement worklist algorithm.
- [x] Dominator tree DFS variable renaming with `VersionStack`.
- [x] Control Dependence Graph (CDG) construction via reversed CFG post-dominators.
- [x] Reps-Horwitz-Sagiv polynomial IFDS data-flow framework (Taint, Nullable pointers, Type-State).
- [x] `.ssa` binary file format serializer/deserializer with CRC-64 verification.

---

### Phase 6: Inter-procedural Call Graph & Points-To Analysis *(Completed)*

- [x] Call graph ($E^{\text{CG}}$) derivation over $V_{\text{sym}}$ and $V_{\text{ssa}}$.
- [x] Class Hierarchy Analysis (CHA) for fast virtual dispatch over-approximation.
- [x] 1-CFA context-sensitive points-to analysis for dynamic dispatch resolution.
- [x] `.cga` binary file format serializer/deserializer with CRC-64 verification.

---

### Phase 7: Universal Bidirectional Traceability Index *(Completed)*

- [x] Aggregation of `token_id` anchor data across all prior binary artifacts.
- [x] Sorted Forward Index ($O(\log n)$ binary search: source location $\rightarrow$ IR node).
- [x] Dense Backward Index ($O(1)$ lookup: IR node $\rightarrow$ source token range).
- [x] `.tra` binary file format serializer/deserializer.

---

### Phase 8: ROBDD Path Summary Computation *(Completed)*

- [x] Control flow Boolean function encoder $f_{\text{paths}}$ via Shannon expansion.
- [x] Reduced Ordered Binary Decision Diagrams (ROBDDs) for path counting (#SAT).
- [x] Sifting variable reordering optimizer for minimal ROBDD node bounds.
- [x] `.psa` binary file format serializer/deserializer.

---

### Phase 9: UML Semantic Metadata Extraction *(Completed)*

- [x] Synthesis of $\rho$ mapping functions for all 14 standard UML diagram views.
- [x] Behavioral sequence extraction from call graphs and ROBDD path summaries.
- [x] State machine pattern detection on CFG basic blocks and field access automata.
- [x] `.uma` binary file format serializer/deserializer.

---

### Phase 10: SCPG Binary Serialization & Query Engine Bootstrap *(Completed)*

- [x] 11-section memory-mapped `.scpg` binary file format merge serializer.
- [x] Demand-driven query engine with CFL-reachability and IFDS tabulation algorithms.
- [x] LRU query cache and $O(1)$ incremental delta update protocol.
