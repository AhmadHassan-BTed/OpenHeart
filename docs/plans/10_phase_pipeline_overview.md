# OpenHeart Master 10-Phase SCPG Compilation Pipeline Specification

Authored and maintained solely by **Ahmad Hassan (B-Ted)**.

The **OpenHeart** static analysis engine transforms raw source code repositories into a memory-mapped **Succinct Compositional Program Graph (SCPG)** through a 10-phase deterministic compilation pipeline.

---

## Master 10-Phase Pipeline Summary Table

| Phase | Pipeline Stage | Primary Input | Serialized Output | Algorithmic Guarantee & Key Responsibility | Engine Status |
|---|---|---|---|---|---|
| **1** | **Lexical Ingestion & Token Corpus** | Source Manifest | `TokenCorpusArtifact (.tca)` | FNV-1a string interning, sorted forward index, `token_id` allocation | **COMPLETED** |
| **2** | **CST Reduction & BP AST Encoding** | `.tca` | `BPASTArtifact (.bpa)` | Reduced ordinal AST forest, 2-bit/node BP bitstring, jump table, RMQ LCA | **COMPLETED** |
| **3** | **Symbol Table & Type Hierarchy** | `.tca`, `.bpa` | `SymbolTableArtifact (.sta)` | Scope resolution via scope graphs, $V_{\text{sym}}$ DAG, $E^{\text{TH}}$ type hierarchy | **COMPLETED** |
| **4** | **CFG CSR & Dominator Analysis** | `.bpa`, `.sta` | `CFGArtifact (.cfa)` | Basic block partitioning ($V_{\text{bb}}$), CSR adjacency lists, Cooper dominators | **COMPLETED** |
| **5** | **SSA Form, CDG & IFDS Data-Flow** | `.bpa`, `.sta`, `.cfa` | `SSAArtifact (.ssa)` | Cytron $\phi$-placement, VersionStack DFS renaming, post-dominator CDG, IFDS solver | **COMPLETED** |
| **6** | **Inter-procedural Call Graph** | `.ssa`, `.sta` | `CallGraphArtifact (.cga)` | Call graph ($E^{\text{CG}}$) derivation, Class Hierarchy Analysis (CHA), 1-CFA points-to | *PLANNED* |
| **7** | **Universal Traceability Index** | `.tca` ... `.cga` | `TraceabilityArtifact (.tra)` | Bidirectional Forward Index ($O(\log n)$) & dense Backward Index ($O(1)$) | *PLANNED* |
| **8** | **ROBDD Path Summaries** | `.cfa`, `.ssa` | `PathSummaryArtifact (.psa)` | Reduced Ordered BDD path functions ($f_{\text{paths}}$), FORCE/sifting reordering, #SAT | *PLANNED* |
| **9** | **UML Semantic Metadata** | Artifacts 1–8 | `UMLMetadataArtifact (.uma)` | Synthesis of $\rho$ mapping functions for all 14 standard UML 2.5 diagram views | *PLANNED* |
| **10**| **SCPG Query Bootstrap** | Artifacts 1–9 | `SCPG Binary (.scpg)` | 11-section memory-mapped `.scpg` binary file & CFL-reachability query engine | *PLANNED* |

---

## Detailed Phase Mandates & Boundaries

### Phase 1: Lexical Ingestion & Token Corpus Construction
- **Mandate**: Ingest raw source files via language adapters (Tree-sitter), assign monotonically increasing 32-bit `token_id` anchors, intern all string literals using FNV-1a, and produce a sorted binary Token Table (`.tca`).
- **Input**: Source Manifest.
- **Output**: `TokenCorpusArtifact (.tca)`.
- **Status**: **COMPLETED**.

### Phase 2: CST Reduction & BP AST Encoding
- **Mandate**: Reduce the concrete syntax tree into an abstract syntax forest (dropping whitespace/punctuation), encode the ordinal forest as a 2-bit-per-node Balanced Parentheses (`BP`) sequence, and construct Jacobson $O(1)$ Rank/Select and Sparse Table RMQ LCA indices (`.bpa`).
- **Input**: `.tca`.
- **Output**: `BPASTArtifact (.bpa)`.
- **Status**: **COMPLETED**.

### Phase 3: Symbol Table & Type Hierarchy Construction
- **Mandate**: Execute a 5-pass symbol resolution pipeline (Declaration Discovery, Scope Imports, Scope BFS, Member Types, Type Hierarchy) over the AST to produce the $V_{\text{sym}}$ vertex layer and $E^{\text{TH}}$ type hierarchy DAG (`.sta`).
- **Input**: `.tca`, `.bpa`.
- **Output**: `SymbolTableArtifact (.sta)`.
- **Status**: **COMPLETED**.

### Phase 4: Control Flow Graph & Dominator Analysis
- **Mandate**: Partition each function body into basic blocks ($V_{\text{bb}}$), construct typed CFG edges (`CFG_TRUE`, `CFG_FALSE`, `CFG_UNCOND`, `CFG_EXCEPT`, `CFG_RETURN`), compute immediate dominators (`idom[]`) via Cooper's algorithm, derive dominance frontiers ($DF[b]$), and detect loop back-edges (`.cfa`).
- **Input**: `.bpa`, `.sta`.
- **Output**: `CFGArtifact (.cfa)`.
- **Status**: **COMPLETED**.

### Phase 5: SSA Conversion, CDG & IFDS Data-Flow Engine
- **Mandate**: Compute backward liveness fixpoint (`LiveIn`, `LiveOut`), insert $\phi$-functions via Cytron's dominance frontier worklist with Pruned SSA optimization, rename variables via dominator tree DFS with `VersionStack`, construct the Control Dependence Graph (`CDGCSR`) via post-dominators, and execute Reps-Horwitz-Sagiv polynomial IFDS data-flow analyses (Taint, Nullable pointers, Type-State) (`.ssa`).
- **Input**: `.bpa`, `.sta`, `.cfa`.
- **Output**: `SSAArtifact (.ssa)`.
- **Status**: **COMPLETED**.

### Phase 6: Inter-procedural Call Graph & Points-To Analysis
- **Mandate**: Construct the inter-procedural call graph ($E^{\text{CG}}$) over $V_{\text{sym}}$ and $V_{\text{ssa}}$, resolve virtual dispatch via Class Hierarchy Analysis (CHA), and resolve dynamic targets via 1-CFA context-sensitive points-to analysis (`.cga`).
- **Input**: `.ssa`, `.sta`.
- **Output**: `CallGraphArtifact (.cga)`.
- **Status**: *PLANNED*.

### Phase 7: Universal Bidirectional Traceability Index
- **Mandate**: Aggregate `token_id` anchor data across all prior binary artifacts into a sorted Forward Index ($O(\log n)$ binary search: source location $\rightarrow$ IR node) and dense Backward Index ($O(1)$ lookup: IR node $\rightarrow$ source token range) (`.tra`).
- **Input**: Artifacts 1–6.
- **Output**: `TraceabilityArtifact (.tra)`.
- **Status**: *PLANNED*.

### Phase 8: ROBDD Path Summary Computation
- **Mandate**: Encode control flow Boolean functions $f_{\text{paths}}$ over CFG edge variables into Reduced Ordered Binary Decision Diagrams (ROBDDs), apply FORCE/sifting variable reordering, and perform #SAT exact path counting (`.psa`).
- **Input**: `.cfa`, `.ssa`.
- **Output**: `PathSummaryArtifact (.psa)`.
- **Status**: *PLANNED*.

### Phase 9: UML Semantic Metadata Extraction
- **Mandate**: Synthesize all prior artifacts into a comprehensive UML metadata layer ($\rho$ mapping functions), extracting structural DAGs, behavioral sequences, state machine automata, and actor surfaces for all 14 standard UML 2.5 diagram types (`.uma`).
- **Input**: Artifacts 1–8.
- **Output**: `UMLMetadataArtifact (.uma)`.
- **Status**: *PLANNED*.

### Phase 10: SCPG Binary Serialization & Query Engine Bootstrap
- **Mandate**: Merge all 9 artifacts into an 11-section memory-mapped `.scpg` binary file, verify CRC-64 checksums, and initialize the demand-driven CFL-reachability and IFDS tabulation query engine (`.scpg`).
- **Input**: Artifacts 1–9.
- **Output**: `SCPG Binary (.scpg) + QueryEngine instance`.
- **Status**: *PLANNED*.
