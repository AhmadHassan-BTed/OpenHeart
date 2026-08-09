# OpenHeart Architectural Roadmap

This document outlines the development phases, technical milestones, and architectural goals for **OpenHeart** (Succinct Compositional Program Graph Engine), authored and maintained by **Ahmad Hassan (B-Ted)**.

---

## 5-Phase Active Engine Pipeline (5 of 10 Phases Completed)

```text
Phase 1 (COMPLETE) ──► Phase 2 (COMPLETE) ──► Phase 3 (COMPLETE) ──► Phase 4 (COMPLETE) ──► Phase 5 (COMPLETE)
Lexical Ingestion      BP AST & Succinct     Symbol Table &        CFG CSR & Dominator    SSA Conversion, CDG
& Token Corpus (.tca)  Reduction (.bpa)      Hierarchy (.sta)      Tree (.cfa)            & IFDS Solver (.ssa)
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

### Phase 2: BP AST Encoding & Succinct Tree Reduction *(Completed)*

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

### Phase 4: CFG CSR Construction & Dominator Analysis *(Completed)*

- [x] Basic block partitioning engine ($V_{\text{bb}}$) with reachability pruning.
- [x] Compressed Sparse Row (CSR) control flow graph adjacency encoder (`offsets`, `adj`).
- [x] Control flow statement entry edge dispatch (`if`, `while`, `for`, `do-while`, `try-catch`).
- [x] Cooper's iterative algorithm for immediate dominators (`idom[]`) and dominance frontiers ($DF[b]$).
- [x] `.cfa` binary file format serializer/deserializer with CRC-64 verification.

---

### Phase 5: Static Single Assignment (SSA), CDG & IFDS Data-Flow Engine *(Completed)*

- [x] Pruned SSA backward liveness analysis fixpoint (`LiveIn`, `LiveOut`).
- [x] Cytron's dominance frontier $\phi$-function placement worklist algorithm.
- [x] Dominator tree DFS variable renaming with `VersionStack`.
- [x] Control Dependence Graph (CDG) construction via reversed CFG post-dominators.
- [x] Reps-Horwitz-Sagiv polynomial IFDS data-flow framework (Taint, Nullable pointers, Type-State).
- [x] `.ssa` binary file format serializer/deserializer with CRC-64 verification.
