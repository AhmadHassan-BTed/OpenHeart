# OpenHeart: Succinct Compositional Program Graph (SCPG) Engine

[![Rust](https://img.shields.io/badge/language-Rust-orange.svg)](https://www.rust-lang.org/)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Build Status](https://img.shields.io/badge/build-passing-brightgreen.svg)]()

> **OpenHeart** is a next-generation static program analysis engine and bidirectional UML generation platform powered by the **Succinct Compositional Program Graph (SCPG)** architecture.

---

## 🏷️ Tags & Categories

`#compiler-theory` `#succinct-data-structures` `#program-analysis` `#uml-generator` `#rust` `#tree-sitter` `#static-analysis` `#robdd` `#ast` `#cfg` `#dfg` `#code-property-graph`

---

## 💡 About & Core Ideology

Traditional Code Property Graphs (CPGs) rely on pointer-heavy node-centric storage models that suffer from severe memory inflation (allocating 30–40 bytes per node/edge) and pointer-chasing traversal overhead. 

**OpenHeart** replaces legacy graph engines with succinct, cache-line aligned, memory-mapped data structures that achieve up to **128× compression** for syntax trees and $O(1)$ bidirectional source-to-diagram traceability.

### The 7-Tuple SCPG Specification

Formally, an SCPG is defined as a 7-tuple:

$$\mathcal{G} = (V, E, \nu, \varepsilon, \tau, \rho, \Sigma_\Phi)$$

- **Vertex Partition** $V = V_{\text{tok}} \sqcup V_{\text{syn}} \sqcup V_{\text{bb}} \sqcup V_{\text{ssa}} \sqcup V_{\text{sym}}$: Strict subsumption lattice enabling $O(1)$ projection between tokens, AST nodes, basic blocks, SSA definitions, and symbol declarations.
- **Typed Edge Set** $E = E^{\text{AST}} \cup E^{\text{CFG}} \cup E^{\text{DFG}} \cup E^{\text{CDG}} \cup E^{\text{CG}} \cup E^{\text{TH}}$: Unifies structural, control, data flow, call graph, and type hierarchy edges.
- **Source Traceability Anchor** $\tau: V_{\text{tok}} \to \mathbb{N}^4$: Maps every token to a monotonic 32-bit `token_id` propagating upward through all pipeline stages.
- **Symbolic Path Summaries** $\Sigma_\Phi$: Reduced Ordered Binary Decision Diagrams (ROBDDs) encoding feasible path sets per function.

---

## 🏗️ 5-Layer Storage Architecture

| Layer | Storage Technology | Computational Guarantee | Compression vs. Legacy |
|---|---|---|---|
| **Layer 1: AST** | Balanced Parentheses (BP) + Rank/Select | $O(1)$ tree navigation (`parent`, `lca`, `subtree_size`) | **128×** vs pointer trees |
| **Layer 2: CFG** | Compressed Sparse Row (CSR) | $O(\text{outdeg})$ sequential cache-friendly block traversal | **9×** vs property graphs |
| **Layer 3: Edges** | Wavelet Tree | $O(\log \sigma)$ type-filtered edge enumeration | **2×** bit compression |
| **Layer 4: DFG** | SSA Form + Dominance Frontiers | $O(1)$ array lookup for variable definition sites | Sparse $O(3n)$ storage |
| **Layer 5: Paths** | ROBDDs + Sifting Optimizer | $O(|\text{ROBDD}|)$ path counting (#SAT) & feasibility checks | Compact BDD encoding |

---

## 📊 14 Native UML Diagram Derivations

The SCPG provides native, deterministic derivation for all 14 UML diagram types:

```text
SCPG Sub-Graph Layer                       Derived UML Diagram Types
├── E^TH (Type Hierarchy) + V_sym ────────► [1] Class, [2] Object, [3] Component, [5] Package, Composite
├── E^CG (Call Graph) + ROBDD Summaries ──► [11] Sequence, [12] Communication, [13] Interaction Overview
├── E^CFG (Control Flow) + AbsInt ────────► [9] Activity, [10] State Machine
└── E^DFG (Data Flow) + IFDS Taint ──────► Interprocedural Taint & Data Flow Overlay
```

---

## 🗺️ 5-Phase Implementation Roadmap

- [x] **Phase 1: Lexical Ingestion & Token Corpus Construction** *(Completed)*
  - Tree-sitter scanner integration, monotonic `token_id` assignment, 64-bit FNV-1a `StringInterner`, `.tca` binary file format serializer with CRC-64 verification, and enforcement of Invariants 1–4.
- [ ] **Phase 2: BP AST Encoding & Scope Graph Semantic Resolution**
  - Balanced Parentheses AST sequence builder, rank/select auxiliary index, and scope graph name binding.
- [ ] **Phase 3: CFG CSR Construction & SSA DFG Encoding**
  - Basic block partitioning, CSR adjacency encoding, Wavelet Tree edge label compression, and Lengauer-Tarjan dominance frontiers.
- [ ] **Phase 4: ROBDD Path Summaries & Symbolic Execution Tier**
  - Shannon expansion, ROBDD construction, sifting dynamic variable reordering, and Z3 SMT solver integration.
- [ ] **Phase 5: Interprocedural Analysis & Dynamic UML Diagram Generator**
  - IFDS framework tabulation, Abstract Interpretation (Interval/Octagon domains), CFL-reachability query engine, and incremental UML visualization synchronization.

---

## 🛠️ Repository Layout

```text
OpenHeart/
├── src/
│   ├── core/
│   │   ├── io/             # Binary Little-Endian reader/writer & mmap wrapper
│   │   └── types/          # TokenRecord (16B), TokenEntry (16B), SourceFileRecord (64B)
│   ├── phase1/
│   │   ├── adapter/        # LanguageAdapter trait & JavaLanguageAdapter
│   │   ├── parser/         # Tree-sitter CST parser integration
│   │   ├── allocator.rs    # Monotonic AtomicU32 TokenIdAllocator
│   │   ├── builder.rs      # Forward/Backward index builder & invariant checks
│   │   ├── interner.rs     # FNV-1a open-addressing StringInterner
│   │   ├── serializer.rs   # Binary .tca format serializer & CRC-64 verification
│   │   └── walker.rs       # Left-to-right DFS CST leaf token walker
│   └── lib.rs              # Root library exports
├── tests/
│   └── phase1_tests.rs     # Integration and end-to-end pipeline tests
├── docs/
│   ├── overview.md                     # Detailed SCPG mathematical specification & formal analysis
│   ├── architecture.md                 # 5-Phase pipeline system architecture guide
│   ├── succinct_structures.md          # BP ASTs, CSR CFGs, Wavelet Trees & ROBDD proof specs
│   ├── uml_derivation.md               # 14 UML diagram derivation rules & query formulas
│   ├── traceability_and_incremental.md # Universal token_id traceability & incremental diff engine
│   ├── getting_started.md             # Quickstart & developer onboarding
│   └── contributing.md                 # Contribution standards & commit guidelines
├── Cargo.toml                          # Rust crate manifest
├── ImplementationPlan.md               # Complete Phase 1 technical specification
└── README.md                           # Repository documentation
```

---

## 🚀 Building & Testing

### Run Automated Tests

```bash
cargo test
```

### Compile Crate

```bash
cargo check
```

---

## 📄 License

This project is licensed under the MIT License.
