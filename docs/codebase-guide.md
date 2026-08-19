# Codebase Onboarding & Contributor Map

This guide provides a structural breakdown of the **OpenHeart** codebase for contributors and maintainers. Designed, authored, and maintained solely by **Ahmad Hassan (B-Ted)**.

👉 **[Launch OpenHeart Web Studio Portal (GitHub Pages)](https://ahmadhassan-bted.github.io/OpenHeart/)**

---

## Repository Structure Overview

```text
OpenHeart/
├── src/                          # Core SCPG Engine & Compiler Pipeline (10 Phases)
├── web/                          # OpenHeart Web Studio Frontend (HTML/JS/CSS)
├── tests/                        # Integration & Pipeline Accuracy Test Suite
├── docs/                         # Architecture, Specifications, & Interactive Spec Models
├── scripts/                      # Helper Scripts & Automation Utilities (ci_check.sh, generate_diagrams.py, restart_server.sh)
├── target_repos/                 # Local Cache Directory for Cloned Benchmark Repositories
├── Cargo.toml / Cargo.lock       # Rust Package Dependencies & Build Configuration
├── Makefile                      # Convenient Targets for Build, Test, & Server Launch
├── ruthless_verify.py            # Multi-Repo Pipeline Accuracy & F1-Score Verification Harness
└── ruthless_config.json          # Verification Configuration & Benchmark Repo Registry
```

---

## Detailed Source Module Layout (`src/`)

```text
src/
├── lib.rs                        # Library Crate Root & Engine Public API Exports
├── main.rs                       # CLI Entry Point (`openheart analyze`, `openheart server`, `openheart inspect`)
│
├── adapters/                     # I/O Adapters & External Protocol Interfaces
│   ├── server.rs                 # Native HTTP REST API Web Server (`0.0.0.0:8080`)
│   └── web_repo.rs               # Git Repository Clone & Dynamic Ingestion Adapter
│
├── core/                         # Core Types, Binary Serialization Primitives & Logger
│   ├── io/                       # BinaryWriter, BinaryReader, MemoryMappedFile, CRC-64 Checksum
│   ├── logger.rs                 # Internal Multithreaded Engine Logger
│   └── types/                    # Core Subsystem Struct Definitions
│       ├── artifact.rs           # Artifact Trait & Base Metadata
│       ├── ast.rs                # AST Node Types, Attrs, & Reduction Taxonomy
│       ├── cfg.rs                # Basic Block, Control Flow Edge, & Dominator Records
│       ├── cg.rs                 # Call Graph Node & Edge Records
│       ├── source.rs             # Source File Record Layout
│       ├── ssa.rs                # SSA Variable, Phi Record, & CDG Edge Types
│       ├── symbol.rs             # Symbol Kind, Scope Kind, Visibility, & Modifier Records
│       └── token.rs              # Token Record Layout & 64-bit `sort_key` Bit Packing
│
├── ingestion/                    # Phase 1: Lexical Ingestion & Token Corpus Engine (.tca)
│   ├── adapter/                  # Tree-sitter Language Adapters (Java, Kotlin, Generic)
│   │   ├── generic.rs            # Generic Multi-Language Adapter Fallback
│   │   ├── java.rs               # Java Language AST Adapter
│   │   ├── kotlin.rs             # Kotlin Language AST Adapter
│   │   └── registry.rs           # Language Adapter Dispatch Registry
│   ├── parser/                   # Tree-sitter Parser Integration & Memory Wrapper
│   ├── allocator.rs              # Monotonic 32-bit Token ID Allocator (`TokenIdAllocator`)
│   ├── builder.rs                # TokenCorpusBuilder & Invariants 1–4 Validation
│   ├── interner.rs               # FNV-1a StringInterner with Constant-Time Deduplication
│   ├── manifest.rs               # Source File Ingestion Manifest
│   ├── serializer.rs             # Binary .tca Serializer/Deserializer with CRC-64
│   └── walker.rs                 # Pre-order Tree Traversal Engine
│
├── ast/                          # Phase 2: CST Reduction & BP AST Encoding Engine (.bpa)
│   ├── adapter/                  # CST Reduction Taxonomies per Language
│   ├── bp_encoder.rs             # Bit-packed 2-bit Balanced Parentheses Bitstring Builder
│   ├── jump_table.rs             # O(1) Matching Parentheses Jump Table
│   ├── rank_select.rs            # Jacobson O(1) Rank/Select Auxiliary Index Structures
│   ├── reducer.rs                # CST Reduction Pipeline (Keep / Eliminate / Drop / Token)
│   ├── rmq.rs                    # Sparse Table Range Minimum Query for O(1) LCA Queries
│   └── serializer.rs             # Binary .bpa Serializer/Deserializer
│
├── symbol/                       # Phase 3: Symbol Table & Scope Graph Engine (.sta)
│   ├── adapter/                  # Semantic Symbol Discovery Adapters
│   ├── passes/                   # 5-Pass DFS Symbol Discovery Subsystem
│   │   ├── pass1_discovery.rs    # Pass 1: Declaration Discovery & Structural Parent Linking
│   │   ├── pass2_imports.rs      # Pass 2: Package & Import Resolution
│   │   ├── pass3_types.rs        # Pass 3: Type Reference Resolution
│   │   ├── pass4_members.rs      # Pass 4: Field & Method Member Resolution
│   │   └── pass5_hierarchy.rs    # Pass 5: Type Hierarchy Graph & Topological Acyclicity Verification
│   ├── builder.rs                # SymbolTableBuilder & Phase 3 Invariant Engine
│   ├── qual_name_table.rs        # Qualified Name Lookup Table
│   ├── scope_graph/              # Scope Graph Resolution & Parent Scope Chains
│   ├── serializer.rs             # Binary .sta Serializer/Deserializer
│   ├── std_library/              # Built-in Standard Library Stub Symbols (Java/Kotlin/Rust)
│   └── uml_meta/                 # Early Association & Pattern Feature Extraction
│
├── cfg/                          # Phase 4: Control Flow Graph & Dominator Analysis (.cfa)
│   ├── stmts/                    # Statement CFG Builders (if/else, switch, while, for, try-catch)
│   ├── builder.rs                # CFG Builder Pipeline
│   ├── dominators.rs             # Cooper Iterative Immediate Dominators (`idom[]`)
│   ├── frontier.rs               # Cytron Dominance Frontier Computation ($DF[b]$)
│   ├── loops.rs                  # Loop Nesting Forest & Back-Edge Detection
│   └── serializer.rs             # Binary .cfa Serializer/Deserializer
│
├── ssa/                          # Phase 5: SSA Conversion, CDG & IFDS Engine (.ssa)
│   ├── cdg.rs                    # Control Dependence Graph via Reversed Post-Dominators
│   ├── ifds.rs                   # Reps-Horwitz-Sagiv Polynomial IFDS Solvers (Taint, Null, State)
│   ├── liveness.rs               # Pruned SSA Backward Liveness Fixpoint Analysis
│   ├── placement.rs              # Cytron Phi-Function Placement Worklist
│   ├── renaming.rs               # Dominator Tree DFS Variable Renaming & VersionStack
│   ├── serializer.rs             # Binary .ssa Serializer/Deserializer
│   └── version_stack.rs          # Scoped SSA Version Stack Data Structure
│
├── cg/                           # Phase 6: Inter-Procedural Call Graph & Points-To Engine (.cga)
│   ├── builder.rs                # Call Graph Construction Pipeline
│   ├── points_to.rs              # Class Hierarchy Analysis (CHA) & Andersen Points-To Solver
│   ├── virtual_dispatch.rs       # Polymorphic Virtual Method Dispatch Resolver
│   └── serializer.rs             # Binary .cga Serializer/Deserializer
│
├── tra/                          # Phase 7: Universal Traceability Index (.tra)
│   ├── backward/                 # Backward Projection Maps (Graph Nodes → Token IDs)
│   ├── forward/                  # Forward Projection Maps (Token IDs → Source Positions)
│   ├── delta/                    # Incremental Invalidation & Stale Node Detection
│   ├── uml_link/                 # Bijective UMLLink Record Builder & Hash Chains
│   ├── builder.rs                # Traceability Index Builder
│   ├── serializer.rs             # Binary .tra Serializer/Deserializer
│   └── types.rs                  # Traceability Artifact Struct Definitions
│
├── psa/                          # Phase 8: ROBDD Path Summaries & Feasibility Analysis (.psa)
│   ├── bdd/                      # Reduced Ordered Binary Decision Diagram Library
│   │   ├── apply.rs              # Boolean Apply Operations (AND, OR, XOR, NOT)
│   │   ├── node.rs               # ROBDD Node Layout (12-byte Compact Node)
│   │   ├── restrict.rs           # Variable Restriction Operations
│   │   ├── sat_count.rs          # Exact Feasible Path Counting (#SAT)
│   │   └── unique_table.rs       # Canonical Node Sharing Unique Table
│   ├── construction/             # Path Summary Feasibility & Constraint Solvers
│   ├── ordering/                 # Variable Ordering Algorithms (FORCE, RPO, Sifting)
│   ├── builder.rs                # Path Summary Artifact Builder
│   ├── metrics.rs                # Cyclomatic Complexity & Path Metrics Computation
│   ├── serializer.rs             # Binary .psa Serializer/Deserializer
│   └── types.rs                  # PSA Header & Function Path Records
│
├── uma/                          # Phase 9: UML Semantic Metadata Artifact Extraction (.uma)
│   ├── actor_identification.rs   # External Actor & Entry Point Identification Engine
│   ├── behavioral/               # Behavioral Diagram Extractor Modules
│   │   ├── activity_diagram.rs   # Activity Diagram Flow Extractor
│   │   ├── communication_diagram.rs # Communication Diagram Extractor
│   │   ├── interaction_overview.rs  # Interaction Overview Diagram Extractor
│   │   ├── sequence_diagram.rs   # Sequence Diagram Message Trace Extractor
│   │   ├── state_machine.rs      # State Machine Diagram Extractor
│   │   └── timing_diagram.rs     # Timing Diagram Temporal Constraint Extractor
│   ├── structural/               # Structural Diagram Extractor Modules
│   │   ├── class_diagram.rs      # Class Diagram Extractor
│   │   ├── component_diagram.rs  # Component Diagram Extractor
│   │   ├── composite_diagram.rs  # Composite Structure Diagram Extractor
│   │   ├── object_diagram.rs     # Object Diagram Instance Extractor
│   │   └── package_diagram.rs    # Package Diagram Extractor
│   ├── patterns/                 # GoF Design Pattern Detection Subsystem
│   │   ├── builder.rs            # Builder Pattern Matcher
│   │   ├── factory.rs            # Factory Method Pattern Matcher
│   │   ├── observer.rs           # Observer Pattern Matcher
│   │   ├── singleton.rs          # Singleton Pattern Matcher
│   │   ├── state.rs              # State Pattern Matcher
│   │   └── template_method.rs    # Template Method Pattern Matcher
│   ├── builder.rs                # UMLMetadataArtifact Builder
│   ├── label_extraction.rs       # Dynamic Element Label Extractor
│   ├── serializer.rs             # Binary .uma Serializer/Deserializer
│   └── types.rs                  # UMA Struct Definitions & Stereotypes
│
└── scpg/                         # Phase 10: Succinct Compositional Program Graph (.scpg) & Queries
    ├── api/                      # High-Level SCPG Builder & Engine API
    ├── diagram/                  # PlantUML, Mermaid, XMI, & JSON Diagram Exporters
    │   ├── export/
    │   │   ├── json.rs           # Structured JSON Diagram Serializer
    │   │   ├── mermaid.rs        # Native Mermaid JS Syntax Exporter
    │   │   ├── plantuml.rs       # Native PlantUML 14-Diagram Exporter Engine
    │   │   ├── plantuml_optimizer.rs # Package Bundling & Spaghetti Reduction Engine
    │   │   └── xmi.rs            # OMG XMI 2.5 XML Standard Serializer
    │   ├── renderers.rs          # Visual Diagram Renderer Helpers
    │   └── mod.rs                # Diagram Module Roots
    ├── incremental/              # Incremental Re-computation & Delta Processing
    ├── mmap/                     # OS Page Memory Mapping Engine
    ├── query/                    # High-Performance Graph Query Engines
    │   ├── cache.rs              # Query Cache Layer
    │   ├── cfl.rs                # Context-Free Language Reachability Queries
    │   ├── impact.rs             # Change Impact Analysis Queries
    │   ├── navigation.rs         # O(1) Graph Traversal & Subsumption Navigation
    │   ├── robdd.rs              # ROBDD Path Feasibility Queries
    │   └── slice.rs              # Forward & Backward Program Slicing Queries
    ├── serializer/               # Unified SCPG Binary Serializer & Integrity Checksums
    └── types.rs                  # SCPG Composite Graph Node & Edge Record Definitions
```

---

## Integration Test Suite (`tests/`)

| Test File | Covered Analysis Subsystem | Key Invariants Verified |
|---|---|---|
| `ingestion_tests.rs` | Phase 1 Lexical Ingestion | Token sorting, StringInterner deduplication, token ID monotonicity |
| `ast_tests.rs` | Phase 2 CST & BP AST | Balanced Parentheses bitstring invariants, $O(1)$ Rank/Select, LCA RMQ |
| `symbol_tests.rs` | Phase 3 Symbol Table | Scope resolution, parent linking, type hierarchy acyclicity (Kahn's) |
| `cfg_tests.rs` | Phase 4 CFG Analysis | Basic block boundaries, Cooper immediate dominators, dominance frontiers |
| `ssa_tests.rs` | Phase 5 SSA Engine | Cytron $\phi$-placement, dominator tree variable renaming, CDG edges |
| `cg_tests.rs` | Phase 6 Call Graph | CHA virtual dispatch resolution, points-to call sites |
| `tra_tests.rs` | Phase 7 Traceability | Universal `token_id` forward/backward index integrity, UMLLink hashes |
| `psa_tests.rs` | Phase 8 ROBDD Path Summaries | ROBDD canonical sharing, Shannon expansion, path feasibility counting |
| `uma_tests.rs` | Phase 9 UML Metadata | Structural & behavioral record extraction, design pattern detection |
| `scpg_tests.rs` | Phase 10 & API Integration | Full 10-phase pipeline end-to-end processing & HTTP REST API response |
| `pipeline_accuracy_tests.rs` | Multi-Phase Pipeline Verification | Ground-truth verification across benchmark codebases |
| `surgical_pipeline_dump_test.rs` | Step-by-Step Binary Dump | Per-phase binary artifact integrity & serialization roundtrip tests |

---

## Primary Engine Binary Artifact Formats

| Phase | Extension | Magic Header | Core Data Structures & Storage Strategy |
|---|---|---|---|
| **Phase 1** | `.tca` | `TCA\0` | `SourceFileRecord[]`, `TokenRecord[]` (16B), FNV-1a `StringInterner` |
| **Phase 2** | `.bpa` | `BPA\0` | BP bitstring (`u64[]`), `JumpTable`, `RankSelectIndex`, `SparseTableRMQ` |
| **Phase 3** | `.sta` | `STA\0` | `SymbolRecord[]`, `ScopeNode[]`, `TypeHierarchyEdge[]`, `StdLibManager` |
| **Phase 4** | `.cfa` | `CFA\0` | `SuccessorCSR`, `PredecessorCSR`, `idom[]`, `DominanceFrontierCSR` |
| **Phase 5** | `.ssa` | `SSA\0` | `SSARecord[]`, `PhiRecord[]`, `DefUseCSR`, `CDGCSR`, IFDS Facts |
| **Phase 6** | `.cga` | `CGA\0` | `CallSiteRecord[]`, `CallEdgeCSR`, Points-To Alloc Sets |
| **Phase 7** | `.tra` | `TRA\0` | Forward Index Map, Backward Index Map, `UMLLinkRecord[]` |
| **Phase 8** | `.psa` | `PSA\0` | `FunctionPSAHeader[]`, `ROBDDNodeTable` (12B compact nodes) |
| **Phase 9** | `.uma` | `UMA\0` | `ClassRecord[]`, `ObjectRecord[]`, `SequenceDiagramRecord[]`, Pattern Matches |
| **Phase 10** | `.scpg` | `SCPG` | Unified Memory-Mapped SCPG Header, Layer Maps, Cross-Layer Indexes |
