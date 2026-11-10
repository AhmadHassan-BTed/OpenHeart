# Implementation Status, Support Matrix & Claims Ledger

This page is the single source of truth for **what the OpenHeart reference implementation does today**. The other documents in this repository (README, architecture and overview pages, the phase plans under `docs/plans/`, and the research paper) describe the *design* of the Succinct Compositional Program Graph. Wherever a design document and this page disagree, this page describes the shipped code and the design document describes a target.

**Baseline.** §2–§6 record the code as audited at commit `fc218cd`. Every row names the module that implements the behaviour. A change that alters a row's behaviour updates that row in the same pull request, with the evidence required by §1; until then the row states the baseline behaviour, and a fix that has not updated its row is not yet claimed. §8 lists the configuration that the current tree reads.

---

## 1. Status Rule

| Status | Meaning | Required evidence |
|---|---|---|
| **Implemented** | The behaviour exists and is checked against an expected result. | A link to the module **and** a link to a test that asserts a concrete expected output (not just non-emptiness or "did not panic"). |
| **Partial** | The module runs, but its output deviates from the design in the documented ways. | A link to the module and a description of every known deviation. |
| **Template** | The output does not depend on the analysed program (fixed or illustrative content). | A link to the code that emits it. |
| **Not derived** | The extractor exists but is empty, or its output is never rendered. | A link to the empty or unused code. |
| **Planned** | Described in a design document, absent from the code. | — |

An item is promoted only when both pieces of evidence are linked. A design document, a docstring, a checklist tick or a passing smoke test is not evidence.

---

## 2. Language Support Matrix

Grammar dependencies (from `Cargo.lock`): `tree-sitter 0.20.10`, `tree-sitter-java 0.20.2`, `tree-sitter-javascript 0.20.4`. No other grammar is linked.

| Language | Phase 1 grammar | Phase 2 reducer | Phase 3 symbols | Phases 4–5 CFG / SSA | Phase 6 call graph | Tier |
|---|---|---|---|---|---|---|
| **Java** | `tree-sitter-java` 0.20.2 | Java adapter (`src/ast/adapter/java.rs`) | Java semantic adapter, with the limits in §4 | Yes | CHA-based, with the limits in §4 | **Tier 1 — experimental** (no independent accuracy benchmark yet) |
| **Kotlin** | **Java grammar** (`KotlinLanguageAdapter::ts_language` in `src/ingestion/adapter/kotlin.rs` returns `tree_sitter_java::language()`) | No Kotlin reducer; falls back to the JavaScript-grammar generic adapter (`src/ast/adapter/registry.rs`) | Token sweep only; `fun` symbols get no declaration node | None: Phase 4 skips symbols without a declaration node | None | **Unsupported** |
| JavaScript | `tree-sitter-javascript` 0.20.4 via the generic adapter | Generic adapter | No semantic adapter | Not meaningful | Not meaningful | **Unsupported** |
| TypeScript, Python, Rust, C++, Go, others | **No grammar**; unknown extensions fall back to the JavaScript grammar (`src/ingestion/adapter/registry.rs`) | Generic JavaScript-grammar adapter | None | None | None | **Unsupported** |

Consequences:

- Only Java input produces meaningful results. Results for every other language come from tree-sitter error recovery under the wrong grammar.
- The server's `/api/supported-languages` response (`src/adapters/server.rs`) lists `java, python, typescript, javascript, cpp, rust, go`. That list is not derived from the adapter registry and does not reflect this matrix.
- A language is promoted to a tier only with its own grammar dependency, Phase 1 and Phase 2 adapters, fixture tests that assert `!root_node().has_error()`, and a published per-language accuracy result (§7).

---

## 3. Entry Points and Input Selection

The three entry points select input differently, so their results are **not** comparable with each other.

| Entry point | Engine | Files analysed |
|---|---|---|
| CLI `openheart analyze <SRC> [OUT]` (`src/main.rs`) | Rust pipeline, Phases 1–10 | Recursively collects `.java`, `.kt` and `.kts`; does not exclude test directories. |
| Server `POST /api/analyze` (`src/adapters/server.rs`) | Rust pipeline, Phases 1–10 | Its own collector: a wider extension list, and test directories skipped. |
| Web studio on GitHub Pages (`web/js/github-engine.js`) | **Not the Rust engine.** A JavaScript heuristic extractor that finds classes with regular expressions and falls back to classes derived from file names. | A sample of the repository: the first 30 source files of a GitHub repository, or the first 80 files of a local folder, fetched one at a time through the unauthenticated GitHub REST API (60 requests per hour). The reported `execution_time_ms` values are constants, not measurements. |

The CLI accepts these subcommands: `analyze`, `inspect`, `server [PORT]` and `help`. There is no `serve` subcommand.

---

## 4. Phase Capability Ledger

| Phase | Module | Status | What the code does, and the known deviations from the design |
|---|---|---|---|
| 1 Lexical ingestion (`.tca`) | `src/ingestion/` | Partial | Tree-sitter parse, monotonic `token_id`, FNV-1a interning, sorted token records, CRC-64/ECMA trailer. No syntax-error model: `ERROR`/`MISSING` nodes are neither counted nor reported, and `SourceFileRecord.flags` is not populated. The CST walk is recursive. |
| 2 BP AST (`.bpa`) | `src/ast/` | Partial | Balanced-parentheses bitstring plus a `u32` jump table entry per BP position (`jump_table.rs`), a rank directory (`rank_select.rs`), preorder attribute arrays (`preorder.rs`) and a full sparse-table RMQ of O(n log n) words (`rmq.rs`). This is **not** a succinct (2n + o(n)-bit) structure. `select1` scans bit by bit. Phase 2 reads the `.tca` at fixed offsets without checking its magic, version or CRC. The Java reducer drops `import_declaration`, generic, array, scoped and wildcard type nodes. |
| 3 Symbols (`.sta`) | `src/symbol/` | Partial | Five passes run, but import declarations never reach Pass 2 (they are dropped in Phase 2), same-package resolution does not exist, lookup ignores symbol kind, type references are bare identifiers, and several passes are quadratic. Symbol ids are pre-order positions, so they change when any earlier declaration changes. `type_ref_resolutions` and import edges are not serialized. |
| 4 CFG & dominators (`.cfa`) | `src/cfg/` | Partial | CSR CFG per function, Cooper et al. iterative dominators, dominance frontiers. No differential test against a reference CFG builder. |
| 5 SSA & CDG (`.ssa`) | `src/ssa/` | Partial | Liveness, φ placement and renaming modules exist; no differential test against a reference SSA construction. `ifds.rs` is **not** an IFDS solver: it marks statements by AST shape (a "taint source" is a statement with a method-declaration child, "nullable" is any literal child) and propagates nothing. |
| 6 Call graph (`.cga`) | `src/cg/` | Partial | Call-site extraction and CHA dispatch. The "1-CFA" step looks up receiver types and has no call-string context. `points_to.rs` is **not** Andersen's analysis: it seeds sets at allocation statements and propagates only along φ copies, with no assign, load, store, call or return constraints, and SSA ids from different functions share one key space. |
| 7 Traceability (`.tra`) | `src/tra/` | Partial | Dense backward arrays give O(1) entity → source lookups. Forward queries (`forward/sym_span.rs`, `forward/cs_span.rs`) binary-search and then scan, so they are not O(log n + k). The UMLLink hash chain uses a bitwise CRC-32 (`uml_link/hash_chain.rs`). The delta/invalidation modules (`tra/delta/`) have no callers and compare positional ids. |
| 8 Path summaries (`.psa`) | `src/psa/` | Partial | ROBDD library, FORCE ordering, sifting module. The path constraint Φ_b only states `reach(b) → out-constraint`: edges of unreached blocks stay free and there is no exit constraint, so **#SAT(f_paths) overstates the number of paths** for branching functions. Sifting permutes the ordering table but never rewrites the BDD, so it has no effect. An earlier revision aliased the BDD node vector through an `unsafe` slice while appending to it (undefined behaviour on reallocation); the current tree has no `unsafe` under `src/psa/`, and `make miri` is the regression check. |
| 9 UML metadata (`.uma`) | `src/uma/` | Partial | See §5. The class extractor drops classes whose names appear in a built-in name list (`src/uma/structural/class_diagram.rs`), including common real names such as `Configuration`, `Logger`, `Table` and `Root`. Record order follows `HashSet`/`HashMap` iteration, so `.uma` bytes differ between runs. |
| 10 SCPG (`.scpg`) | `src/scpg/` | Partial | The writer emits a header, a section directory and 11 sections, of which only the string table carries data; the others hold element counts. `MemoryMappedSCPG::open` reads the whole file into memory rather than mapping it, and no query reads from it. The header `source_hash` is written as zeros. `creation_ts_ns` makes the bytes differ between runs. The incremental engine (`incremental/`) classifies a caller-asserted delta and XORs the hash with a constant; nothing calls it. "CFL-reachability" (`query/cfl.rs`) is breadth-first call-graph reachability with no call/return matching. |

---

## 5. Diagram Provenance Matrix

"Derived" means the diagram is computed from the analysed program. Exporters live in `src/scpg/diagram/export/` (`plantuml.rs`, `mermaid.rs`, `json.rs`, `xmi.rs`); extractors live in `src/uma/`.

| # | Diagram | Status | Notes |
|---|---|---|---|
| 01 | Class | Partial | Derived from symbols and type-hierarchy edges. Drops blocklisted names; matches types by simple name; edge order follows hash iteration. |
| 02 | Package | Partial | Containment and class counts only; no inter-package dependency edges. |
| 03 | Component | Partial | Components are exactly packages; provided/required interfaces are counted but not rendered. No build metadata is read. |
| 04 | Composite structure | Not derived | `composite_diagram.rs` has an empty body. |
| 05 | Object | Not derived | The extractor indexes symbols with AST node ids, and no exporter reads `uma.objects`. |
| 06 | Deployment | Partial | A relabelled package view. No Dockerfile, compose, Kubernetes or build metadata is read. |
| 07 | Profile | Template (JSON) | The JSON builder emits constants. |
| 08 | Sequence | Partial | Messages come from call sites, but sub-calls are expanded in the wrong order, there are no combined fragments, returns or nesting, and all scenarios are merged into one diagram. |
| 09 | State machine | Template | Two states per class (initial and final) with one transition per method; SSA is not read. |
| 10 | Activity | Partial | Derived from CFG blocks, but guards, exception edges and loop structure are discarded, and labels are `Block_N`. Exporters show only the first few functions. |
| 11 | Use case | Partial | Heuristic actor and entry-point identification; no oracle. |
| 12 | Communication | Not derived | `communication_diagram.rs` has an empty body. |
| 13 | Interaction overview | Not derived | `interaction_overview.rs` has an empty body; the JSON builder emits constants. |
| 14 | Timing | Template | A fixed clock that advances by 50 per message of the first sequence; when there is none, the first 3 classes × 2 methods. No thread analysis. |
| 15 | Control flow graph | Template | The exporters do not receive `.cfa`. Every function is drawn as the same fixed entry/branch/exit shape; the JSON builder describes OpenHeart's own pipeline. |
| 16 | Data flow graph | Template | A fixed φ example (`var_name: retries`). |
| 17 | Control dependence graph | Template | Always `(amount > 0)` → `repository.saveOrder()`. |
| 18 | Call graph | Partial | Derived by Phase 6 from the token corpus: compile-time selection, CHA/RTA dispatch, points-to refinement, and the Phase 3 symbol-table resolution as a fallback. Every edge records its dispatch and resolution kind, and unresolved sites keep the reason. On `examples/test_patterns_codebase`: 68 call sites, 47 edges (10 November 2026). |
| 19 | ROBDD | Template | The JSON builder emits constants; `.psa` is not read. |

XMI export (`xmi.rs`) emits an XMI 2.1 document in the Eclipse UML2 3.0.0 namespace containing one `uml:Class` per class named `Class_<id>`, with no packages, members or relationships. Neither the CLI nor the server writes XMI files. No diagram is checked for UML 2.5 conformance.

**Web studio layout** (`web/js/layout/`): written and tested with `scripts/layout-gate` (10 November 2026). All 54 gate inputs pass every hard check, with deterministic output: the 19 bundled diagrams, the class and package models of `examples/`, `src/` and `web/js/`, and a stress input per diagram type. The hard checks are no overlapping boxes, no edge through an unrelated card or package, and route ends on their nodes. The engine uses ELK (layered with orthogonal routing across package borders, or stress with VPSC overlap removal), its own packing, routing and metrics-scored candidate portfolio, and dedicated sequence and timing layouts. It falls back to the legacy layout (`web/js/uml-layout.js`) when ELK is not loaded, the diagram exceeds 5,000 nodes or 15,000 edges, or the engine fails. Known gaps: about 21 s on 1,700-card models, long edges on dense random graphs, and the Message layer toggle does not hide self messages drawn inside lifelines. The engine's click-to-explore queries ([`engine-query-protocol.md`](engine-query-protocol.md)) answer through `openheart query` and `POST /api/sessions/<id>/query`. The WebAssembly build and the studio's Relations tab are not built yet.

---

## 6. Claims Ledger

Each headline claim that has appeared in this repository, its current status, and what is required before it may be made again.

| Claim (where it appeared) | Status | Evidence | Required to reinstate |
|---|---|---|---|
| "128× memory compression" (README) | **Withdrawn** | 128 is the theoretical ratio between a 32-byte pointer node and 2 bits per node, not a measurement. The shipped AST stores a jump table, a rank directory, preorder arrays and an O(n log n)-word sparse table (§4). | Suite S15 (§7) measured in bits per node against a named pointer baseline. |
| "4.65 MB per 1M AST nodes", "6.9× smaller" (overview, architecture) | **Withdrawn** | Same as above; no measurement exists. | Suite S15. |
| "O(1) bidirectional source-to-diagram traceability" (README) | **Partial** | Backward (entity → source) lookups are O(1) array reads. Forward (source → entity) queries scan after a binary search; `NavigationEngine` lookups are linear. | Nested-span or interval index, plus p50/p99 latency benchmarks at 1M symbols. |
| "Zero-copy memory mapping of all 10 phase artifacts" (README) | **Withdrawn** | Readers copy every array into new `Vec`s and rebuild rank/select and RMQ indexes; `.scpg` open reads the whole file. | Aligned sections read through typed views, plus suite S14. |
| "CRC-64 verified binary artifacts" (README) | **Partial** | Phases 1–9 write CRC-64/ECMA trailers, but readers do not verify uniformly (Phase 2 does not verify the `.tca`). For `.scpg` the current checksum scheme is defined in `src/scpg/serializer/integrity.rs`. | Corruption tests for every reader that expect an error, and fuzz targets. |
| "F1 = 1.0000 across Java, Kotlin, Python, Rust, TypeScript and C++" (README, overview) | **Withdrawn** | The ground truth in `ruthless_verify.py` is a keyword regex keyed by simple class name. A name list in `ruthless_config.json` that repeats nearly all of the engine's own class-name blocklist is removed from both sides, so recall is 1.0 by construction. Only class-name sets are compared; five of the six languages have no grammar (§2). The harness does not run in CI. | Suites S3 and S13 (§7) with independent oracles. |
| "19 projections derived from binary graph layers" (README) | **Withdrawn** | Diagrams 04, 05, 07, 09 and 12–19 are templates or not derived (§5). | Every rendered node and edge traceable to an artifact entity, checked by snapshot tests. |
| "All 14 standard OMG UML 2.5 diagrams" (README, CHANGELOG) | **Withdrawn** | See §5; no conformance checks; XMI is a placeholder. | XMI 2.5.1 / UML 2.5.1 output that loads into Eclipse UML2 with EMF validation. |
| "Deterministic generation" (CHANGELOG) | **Withdrawn** | Hash iteration order reaches `.uma`, `.sta` and exporter output; `.scpg` embeds a timestamp. | A two-process byte-equality test for every artifact and exporter. |
| "IFDS solvers: taint, nullable, type-state" (README, ROADMAP, paper) | **Withdrawn** | §4, Phase 5. | Reps–Horwitz–Sagiv tabulation over an ICFG, plus suite S2. |
| "Andersen points-to", "1-CFA" (README, ROADMAP, paper) | **Withdrawn** | §4, Phase 6. | Constraint-based solver, plus suite S6. |
| "Rudell sifting" (ROADMAP, architecture, paper) | **Withdrawn** | §4, Phase 8. | In-place adjacent-level swaps and a test in which sifting shrinks a bad ordering. |
| "Exact #SAT path counting" (README, ROADMAP, paper) | **Withdrawn** | §4, Phase 8. | A differential test against brute-force path enumeration on random structured CFGs. |
| "CFL-reachability", "IFDS tabulation" in the query engine (ROADMAP, phase 10 plan) | **Withdrawn** | §4, Phase 10. | Call/return-matched tabulation over an ICFG. |
| "O(1) incremental delta update protocol" (ROADMAP, traceability doc) | **Withdrawn** | §4, Phases 7 and 10. | Stable ids, real deltas, and an edit-replay benchmark proving incremental results equal a clean rebuild. |
| "~305 MB uncompressed, ~82 MB with LZ4 for a 500K-LOC project" and per-phase sizes (paper, phase 10 plan) | **Withdrawn** | No LZ4 dependency exists, the corpus is unnamed and no script produces the numbers. | Numbers generated from committed benchmark results on a named corpus and commit. |
| Wavelet-tree edge typing, complement edges (architecture, succinct structures, paper) | **Planned** | Nothing in `src/` implements them. | Implementation plus suite S15. |
| "Universal multi-language support across all 10 phases" (CHANGELOG) | **Withdrawn** | §2. | §2 promotion criteria per language. |
| "In-browser AST parsing" of GitHub repositories (README) | **Withdrawn** | §3: a regex heuristic over a sample of files. | The Rust engine compiled to WebAssembly, or the studio labelled as a heuristic preview. |
| "`#![forbid(unsafe_code)]` in ingestion modules" (security architecture) | **Withdrawn** | No module under `src/ingestion/` carries the attribute. | The attribute on every module that contains no `unsafe`. |
| "Line and column values saturate" (security architecture) | **Withdrawn** | Narrowing casts wrap; `try_build_sort_key` (`src/core/types/token.rs`) rejects out-of-range positions instead. | — (the documentation now describes rejection). |
| `OPENHEART_MAX_MEMORY_MB`, `OPENHEART_ARTIFACT_DIR`, `OPENHEART_STRICT_INVARIANTS` (security architecture, `.env.example`) | **Withdrawn** | No code reads them. | Code that reads and enforces them. |
| "Rust 1.75+" (README, getting started) | **Corrected** | `Cargo.toml` declares `rust-version = "1.88"`. | — |
| Competitor figures: Joern 32 B/node and 34 B/edge, EMF 100–150 MB per 1,000 classes, WALA/Soot 8–16 GB per 1M LOC, a 170 MB Neo4j store (overview) | **Withdrawn** | Unsourced; Joern replaced OverflowDB with flatgraph in 2024. | Primary sources, or numbers reproduced by the harness on the same corpus and hardware, labelled "published" or "reproduced". |
| "CI passing" and "security enforced" badges, "47 unit + 22 integration tests" (README) | **Withdrawn** | The badges were static images; the counts were not maintained. | The workflow status badge; counts generated from CI output. |
| "Enterprise-scale benchmark" (`examples/huge_enterprise_src`) | **Corrected** | It contains 12 Java files and 255 lines. | — |

---

## 7. Measurement Protocol

No accuracy, space or performance number may appear in the README, the docs or the paper unless it is produced by this protocol and committed with its inputs.

### 7.1 Required metadata for every result

- Repository URL, commit SHA and licence of each corpus; the file-selection rule; the entry point (CLI, server or studio, see §3).
- OpenHeart commit SHA, Rust toolchain version, build profile, CPU, memory and operating system.
- Machine-readable results (JSON) committed next to the script that produced them. Documentation numbers are generated from those files, never typed by hand.

### 7.2 Oracle independence

Oracles live in a separately reviewed package and may not share name lists, filters, exclusion lists or heuristics with the engine. Comparison is by fully qualified name, never by simple name.

### 7.3 Suites

| Suite | Question | Oracle / method | Report |
|---|---|---|---|
| **S2** | Taint and IFDS precision/recall | SecuriBench Micro, OWASP Benchmark | P/R per test category |
| **S3** | Declaration accuracy for Java (classes, members, supertypes) | `javac` with SemanticDB, or the JavaParser symbol solver / Eclipse JDT, on pinned corpora (for example spring-petclinic, java-design-patterns, commons-lang) | P/R/F1 per relation with bootstrap confidence intervals |
| **S6** | Points-to and call-graph precision | PointerBench; SootUp/Spark or OPAL call graphs | P/R per edge kind |
| **S11** | Behavioural diagrams | Dynamic traces of executed tests | Message-order and fragment agreement |
| **S13** | Per-language parse quality and declaration accuracy | `ERROR`/`MISSING` node rate under the correct grammar; compiler-backed SCIP indexers | Per-language table; a language is claimed only after it reports here |
| **S14** | Artifact open latency and resident memory | Cold and warm page cache, per artifact | p50/p99 latency, RSS |
| **S15** | Space | Per-structure byte accounting plus peak heap (dhat) for n = 10³ … 10⁷ nodes | Bits per node per structure, against a named pointer baseline |

Micro-benchmarks (rank/select, BP navigation, CRC, interner, BDD apply, traceability queries) use criterion or divan. Macro-benchmarks report LOC/s, peak RSS, artifact bytes per LOC and query-latency percentiles.

---

## 8. Configuration Actually Read by the Engine

Only variables that the code reads are listed. A variable is added here in the same change that makes the code read it, and removed when the code stops reading it.

| Variable | Read by | Effect |
|---|---|---|
| `OPENHEART_LOG` | `src/core/logger.rs` | Console log level (`error`, `warn`, `info`, `debug`, `trace`). Takes precedence over `RUST_LOG`. |
| `RUST_LOG` | `src/core/logger.rs` | Console log level when `OPENHEART_LOG` is unset; an `openheart=<level>` directive wins over a bare global level. |
| `OPENHEART_FILE_LOG` | `src/core/logger.rs` | Log level for the file sinks. |
| `OPENHEART_LOG_MAX_BYTES` | `src/core/logger.rs` | Size cap for the persistent log file. |
| `OPENHEART_NUM_THREADS` | `src/core/types/source.rs` | Worker-thread count recorded in the ingestion manifest; `0`, unset or unparsable means one per core. |
