# Codebase Onboarding & Contributor Map

This guide provides a structural breakdown of the **OpenHeart** codebase for contributors and maintainers. Designed, authored, and maintained by **Ahmad Hassan (B-Ted)**.

👉 **[Launch OpenHeart Web Studio Portal (GitHub Pages)](https://ahmadhassan-bted.github.io/OpenHeart/)**

> **Status.** The tree lists the modules of the audited baseline (commit `fc218cd`), and each description says what the module *did* there, which in several places was less than its name suggests (for example `ssa/ifds.rs` was not an IFDS solver). Modules added since then are not listed; run `git ls-files src` for the complete list. The capability of each phase, and how it changes, is recorded in [status.md §4](./status.md#4-phase-capability-ledger).

---

## Repository Structure Overview

```text
OpenHeart/
├── src/                          # Rust engine and compiler pipeline (10 phases)
├── web/                          # Web studio frontend (HTML/JS/CSS), deployed to GitHub Pages
├── tests/                        # Integration tests
├── docs/                         # Design documents, phase plans, status ledger, research paper
├── examples/                     # Small Java fixture codebases (see examples/README.md)
├── scripts/                      # Helper scripts (ci_check.sh, generate_diagrams.py, restart_server.sh)
│                                 # and experimental Python harnesses
├── Cargo.toml / Cargo.lock       # Rust manifest (rust-version = 1.88)
├── Makefile                      # Build, quality-gate and release targets (`make help`)
├── ruthless_verify.py            # Experimental class-name comparison script; NOT an accuracy
│                                 # benchmark (its oracle shares exclusion lists with the engine,
│                                 # see status.md §6)
└── ruthless_config.json          # Configuration and repository list for ruthless_verify.py
```

`target_repos/`, which the Python scripts use as a clone cache, is gitignored and not part of the repository.

---

## Source Module Layout (`src/`)

```text
src/
├── lib.rs                        # Library crate root
├── main.rs                       # CLI: `analyze`, `inspect`, `server [PORT]`, `help`
│
├── adapters/                     # I/O adapters
│   ├── server.rs                 # Development HTTP server (`openheart server [PORT]`), local use only
│   └── web_repo.rs               # Git repository clone and ingestion adapter
│
├── core/                         # Core types, binary I/O primitives, pipeline driver and logger
│   ├── io/                       # BinaryWriter/BinaryReader, MemoryMappedFile, CRC-64/ECMA
│   ├── logger.rs                 # Console and file logger (OPENHEART_LOG / RUST_LOG)
│   ├── pipeline.rs               # Phase orchestration
│   └── types/                    # Shared record definitions
│       ├── artifact.rs           # Artifact trait and base metadata
│       ├── ast.rs                # AST node types, attributes and reduction taxonomy
│       ├── cfg.rs                # Basic block, edge and dominator records
│       ├── cg.rs                 # Call graph node and edge records
│       ├── source.rs             # SourceFileRecord and the ingestion manifest
│       ├── ssa.rs                # SSA variable, φ and CDG edge types
│       ├── symbol.rs             # Symbol kinds, scope kinds, visibility and modifiers
│       └── token.rs              # TokenRecord layout and 64-bit sort_key packing
│
├── ingestion/                    # Phase 1: lexical ingestion and token corpus (.tca)
│   ├── adapter/                  # Tree-sitter language adapters
│   │   ├── java.rs               # Java adapter (tree-sitter-java)
│   │   ├── kotlin.rs             # Kotlin adapter — returns the JAVA grammar; Kotlin is unsupported
│   │   ├── generic.rs            # Fallback adapter on the JavaScript grammar
│   │   └── registry.rs           # Extension → adapter dispatch (unknown extensions fall back)
│   ├── parser/                   # Tree-sitter parser wrapper
│   ├── allocator.rs              # Monotonic 32-bit token id allocator
│   ├── builder.rs                # TokenCorpusBuilder and invariants 1–4
│   ├── interner.rs               # FNV-1a StringInterner
│   ├── manifest.rs               # Source file ingestion manifest
│   ├── serializer.rs             # .tca writer/reader with CRC-64/ECMA
│   └── walker.rs                 # Recursive pre-order CST walk
│
├── ast/                          # Phase 2: CST reduction and BP AST (.bpa)
│   ├── adapter/                  # Reduction tables: java.rs and a JavaScript-grammar generic.rs
│   ├── bp_encoder.rs             # Balanced-parentheses bitstring builder
│   ├── builder.rs                # BP AST assembly
│   ├── jump_table.rs             # Matching-parenthesis table (one u32 per BP position)
│   ├── preorder.rs               # Preorder attribute arrays, token ranges, parent_map
│   ├── rank_select.rs            # Rank directory; select1 is a bit-by-bit scan
│   ├── reducer.rs                # Keep / Eliminate / Drop / Token reduction
│   ├── rmq.rs                    # Sparse-table RMQ (O(n log n) words) for LCA
│   └── serializer.rs             # .bpa writer/reader (indexes rebuilt on load)
│
├── symbol/                       # Phase 3: symbol table and scope graph (.sta)
│   ├── adapter/                  # Semantic adapter (Java only)
│   ├── passes/                   # Five passes
│   │   ├── pass1_discovery.rs    # Declaration discovery and parent linking
│   │   ├── pass2_imports.rs      # Import handling (never receives imports: Phase 2 drops them)
│   │   ├── pass3_types.rs        # Type reference resolution (bare identifiers only)
│   │   ├── pass4_members.rs      # Field and method member types
│   │   └── pass5_hierarchy.rs    # Type hierarchy edges and acyclicity check
│   ├── builder.rs                # SymbolTableBuilder and invariants
│   ├── qual_name_table.rs        # Qualified name table
│   ├── scope_graph/              # Scope graph and lexical resolver
│   ├── serializer.rs             # .sta writer/reader
│   ├── std_library/              # Java standard-library stub symbols (java_stubs.rs)
│   └── uml_meta/                 # Early association and pattern features
│
├── cfg/                          # Phase 4: control flow graphs and dominators (.cfa)
│   ├── builder/                  # Per-function CFG builder state
│   ├── stmts/                    # Statement builders (if, switch, loops, try, break, return, throw …)
│   ├── analysis/                 # RPO, Cooper dominators, post-dominators, frontiers, loops
│   └── serializer.rs             # .cfa writer/reader
│
├── ssa/                          # Phase 5: SSA form and CDG (.ssa)
│   ├── cdg.rs                    # Control dependence graph via post-dominators
│   ├── ifds.rs                   # Syntactic per-statement fact marking — NOT an IFDS solver
│   ├── liveness.rs               # Backward liveness
│   ├── placement.rs              # Cytron φ placement
│   ├── renaming.rs               # Dominator-tree renaming
│   ├── version_stack.rs          # SSA version stack
│   └── serializer.rs             # .ssa writer/reader
│
├── cg/                           # Phase 6: call graph (.cga)
│   ├── call_sites.rs             # Call-site extraction
│   ├── resolution.rs             # CHA dispatch and receiver-type lookup (no call strings)
│   ├── points_to.rs              # Allocation-site sets with φ-copy propagation — NOT Andersen
│   ├── scc.rs                    # Tarjan SCCs
│   └── serializer.rs             # .cga writer/reader
│
├── tra/                          # Phase 7: traceability index (.tra)
│   ├── backward/                 # Dense backward maps (entity → token range)
│   ├── forward/                  # Span records for source → entity queries
│   ├── delta/                    # Invalidation helpers (no callers; positional ids)
│   ├── uml_link/                 # UMLLink records and the CRC-32 hash chain
│   ├── builder.rs                # Traceability builder
│   ├── serializer.rs             # .tra writer/reader
│   └── types.rs                  # Record layouts
│
├── psa/                          # Phase 8: ROBDD path summaries (.psa)
│   ├── bdd/                      # ROBDD library: node table, unique table, apply, restrict, sat_count
│   ├── construction/             # Path-constraint encoding (overstates path counts, see status.md)
│   ├── ordering/                 # FORCE, RPO and a sifting pass that has no effect
│   ├── builder.rs                # .psa builder
│   ├── metrics.rs                # Cyclomatic complexity and path metrics
│   ├── serializer.rs             # .psa writer/reader
│   └── types.rs                  # PSA headers and records
│
├── uma/                          # Phase 9: UML metadata (.uma)
│   ├── actor_identification.rs   # Heuristic actor and entry-point identification
│   ├── behavioral/               # Sequence and activity extractors; state machine (template);
│   │                             # communication, interaction overview, timing (empty bodies)
│   ├── structural/               # Class, package, component extractors; composite (empty);
│   │                             # object (misreads AST ids)
│   ├── patterns/                 # GoF pattern matchers (adapter, builder, composite, decorator,
│   │                             # facade, factory, observer, singleton, state, strategy, …)
│   ├── builder.rs                # UMLMetadataArtifact builder
│   ├── label_extraction.rs       # Element label extraction
│   ├── serializer.rs             # .uma writer/reader
│   └── types.rs                  # UMA records and stereotypes
│
└── scpg/                         # Phase 10: .scpg container, query engine and exporters
    ├── api/                      # SCPG builder and engine API
    ├── diagram/                  # Exporter suite and abstract factory
    │   ├── export/
    │   │   ├── json.rs           # JSON graph IR (compiler-IR views are templates)
    │   │   ├── mermaid.rs        # Mermaid exporter
    │   │   ├── plantuml.rs       # PlantUML exporter
    │   │   ├── plantuml_optimizer.rs # Package bundling for large class diagrams
    │   │   └── xmi.rs            # Placeholder XMI 2.1 exporter (class shells only)
    │   ├── factory.rs            # Format strategy dispatch
    │   └── renderers.rs          # Renderer helpers
    ├── incremental/              # Stub: classifies a caller-asserted delta; nothing calls it
    ├── mmap/                     # .scpg loader (reads the whole file into memory)
    ├── query/                    # Query engine
    │   ├── cache.rs              # LRU query cache
    │   ├── cfl.rs                # Breadth-first call-graph reachability (no call/return matching)
    │   ├── impact.rs             # Change impact queries
    │   ├── navigation.rs         # Diagram ↔ source navigation
    │   ├── robdd.rs              # Path-summary queries
    │   └── slice.rs              # Backward slicing
    ├── serializer/               # .scpg writer and integrity checksums
    └── types.rs                  # Header, section directory and record definitions
```

---

## Web Studio Module Layout (`web/`)

```text
web/
├── index.html                    # Single-page application
├── style.css                     # Typography and light/dark themes
│
├── diagrams/                     # Bundled sample diagram set and manifest
│   ├── manifest.json             # 3 categories, 19 diagram types, 16 relationship kinds, 24 node kinds
│   ├── class.json / .puml        # 01 · Class Diagram
│   ├── package.json / .puml      # 02 · Package Diagram
│   ├── component.json / .puml    # 03 · Component Diagram
│   ├── composite.json / .puml    # 04 · Composite Structure Diagram
│   ├── object.json / .puml       # 05 · Object Diagram
│   ├── deployment.json / .puml   # 06 · Deployment Diagram
│   ├── profile.json / .puml      # 07 · Profile Diagram
│   ├── sequence.json / .puml     # 08 · Sequence Diagram
│   ├── statemachine.json / .puml # 09 · State Machine Diagram
│   ├── activity.json / .puml     # 10 · Activity Diagram
│   ├── usecase.json / .puml      # 11 · Use Case Diagram
│   ├── communication.json / .puml# 12 · Communication Diagram
│   ├── interaction.json / .puml  # 13 · Interaction Overview Diagram
│   ├── timing.json / .puml       # 14 · Timing Diagram
│   ├── cfg.json / .puml          # 15 · Control Flow Graph (CFG)
│   ├── dfg.json / .puml          # 16 · Data Flow Graph (DFG)
│   ├── cdg.json / .puml          # 17 · Control Dependence Graph (CDG)
│   ├── callgraph.json / .puml    # 18 · Call Graph (CG)
│   └── robdd.json / .puml        # 19 · ROBDD Graph
│
└── js/                           # ES modules
    ├── themes/                   # Theme tokens and the Cytoscape stylesheet compiler
    ├── uml-card-renderer.js      # SVG card generators
    ├── uml-layout.js             # Spatial layout engines
    ├── puml-parser.js            # PlantUML and Mermaid parser
    ├── graph-canvas.js           # Cytoscape canvas controller
    ├── graph-loader.js           # Graph IR → Cytoscape elements
    ├── editor.js                 # Monaco source synchronizer
    ├── file-tree.js              # File tree explorer
    └── github-engine.js          # In-browser GitHub fetcher and heuristic regex class extractor
                                  # (independent of the Rust engine; samples 30/80 files)
```

---

## Example Codebases (`examples/`)

| Directory | Contents | Size |
|---|---|---|
| `examples/test_patterns_codebase/` | 9 GoF patterns (Factory Method, Builder, Singleton, Adapter, Decorator, Facade, Observer, Strategy, Template Method) | 35 files, 451 lines |
| `examples/big_project_src/` | Small multi-package application | 10 files, 225 lines |
| `examples/huge_enterprise_src/` | Banking-style multi-package application | 12 files, 255 lines |

These are fixtures for exercising the pipeline, not benchmarks.

---

## Integration Test Suite (`tests/`)

Unit tests live beside the code in `src/`. Several integration suites check structure, invariants or non-emptiness rather than expected outputs; the capabilities that have expected-result tests are the ones marked *Implemented* in [status.md](./status.md).

| Test File | Subsystem | What it exercises |
|---|---|---|
| `ingestion_tests.rs` | Phase 1 | Token sorting, interner deduplication, token id monotonicity |
| `ast_tests.rs` | Phase 2 | BP bitstring invariants, rank/select, LCA |
| `symbol_tests.rs` | Phase 3 | Scope resolution, parent linking, hierarchy acyclicity |
| `cfg_tests.rs` | Phase 4 | Basic block boundaries, dominators, dominance frontiers |
| `ssa_tests.rs` | Phase 5 | φ placement, renaming, CDG edges |
| `cg_tests.rs` | Phase 6 | CHA dispatch, call sites |
| `tra_tests.rs` | Phase 7 | Forward/backward index integrity, UMLLink hashes |
| `psa_tests.rs` | Phase 8 | ROBDD canonical sharing, Shannon expansion, sat-count |
| `uma_tests.rs` | Phase 9 | Structural and behavioral record extraction |
| `scpg_tests.rs` | Phase 10 & API | End-to-end pipeline run and API responses |
| `strategy_diagram_tests.rs` | Exporter factory | Strategy registration and PlantUML/Mermaid syntax |
| `research_paper_theorems_tests.rs` | Paper statements | Executable checks of the sort-key lemma and the ROBDD statements as formulated in the paper |
| `design_pattern_architecture_tests.rs` | Pattern detectors | Detector runs over the GoF fixtures |
| `adversarial_trick_tests.rs` | Edge cases | Circular inheritance, self-loops, deep generic nesting, obfuscated identifiers |
| `pipeline_accuracy_tests.rs` | Whole pipeline | End-to-end runs over the example codebases (no independent ground truth) |
| `surgical_pipeline_dump_test.rs` | Artifacts | Per-phase artifact dump and serialization round trips |

---

## Binary Artifact Formats

| Phase | Extension | Magic Header | Contents in the current implementation |
|---|---|---|---|
| **Phase 1** | `.tca` | `TCA\0` | `SourceFileRecord[]`, `TokenRecord[]` (16 B), FNV-1a `StringInterner`, CRC-64/ECMA |
| **Phase 2** | `.bpa` | `BPA\0` | BP bitstring (`u64[]`) and preorder arrays; jump table, rank directory and sparse-table RMQ rebuilt on load |
| **Phase 3** | `.sta` | `STA\0` | `SymbolRecord[]`, `ScopeNode[]`, `TypeHierarchyEdge[]` |
| **Phase 4** | `.cfa` | `CFA\0` | Successor/predecessor CSR, `idom[]`, dominance-frontier CSR |
| **Phase 5** | `.ssa` | `SSA\0` | `SSARecord[]`, `PhiRecord[]`, def-use CSR, CDG CSR, syntactic fact marks |
| **Phase 6** | `.cga` | `CGA\0` | `CallSiteRecord[]`, call-edge CSR, allocation-site sets |
| **Phase 7** | `.tra` | `TRA\0` | Forward span records, backward maps, `UMLLinkRecord[]` |
| **Phase 8** | `.psa` | `PSA\0` | `FunctionPSAHeader[]`, ROBDD node table (12 B nodes) |
| **Phase 9** | `.uma` | `UMA\0` | `ClassRecord[]`, `SequenceDiagramRecord[]`, pattern matches and per-diagram records |
| **Phase 10** | `.scpg` | `SCPG` | Header and 11-entry section directory; only the string table carries data |
