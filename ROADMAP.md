# OpenHeart Architectural Roadmap

This document tracks the development phases, technical milestones, and architectural goals for **OpenHeart** (Succinct Compositional Program Graph Engine), authored and maintained by **Ahmad Hassan (B-Ted)**.

👉 **[Launch OpenHeart Web Studio Portal (GitHub Pages)](https://ahmadhassan-bted.github.io/OpenHeart/)**

---

## Status Rule

Every item carries one of four marks:

| Mark | Meaning |
|---|---|
| `[x]` **Implemented** | Linked to the module that implements it **and** to a test that asserts a concrete expected result. |
| `[~]` **Partial** | The module exists and runs, but its behaviour deviates from the design; the deviation is stated. |
| `[ ]` **Planned** | Not in the code, or present only as a stub, template or placeholder. |
| `[?]` **Unverified** | The module exists, but no test checks its output against an expected result. |

An item is ticked only in the pull request that adds both links. The marks below record the code as audited at commit `fc218cd`; the detailed ledger, including every withdrawn claim, is **[docs/status.md](docs/status.md)**.

---

## Master 10-Phase SCPG Pipeline

```text
Phase 1 ──────────────► Phase 2 ──────────────► Phase 3 ──────────────► Phase 4 ──────────────► Phase 5
Lexical Ingestion       BP AST Encoding         Symbol Table &          CFG CSR &               SSA Form & CDG
& Token Corpus (.tca)   (.bpa)                  Type Hierarchy (.sta)   Dominators (.cfa)       (.ssa)
[~] partial             [~] partial             [~] partial             [?] unverified          [~] partial

        │
        ▼
Phase 6 ──────────────► Phase 7 ──────────────► Phase 8 ──────────────► Phase 9 ──────────────► Phase 10
Call Graph (.cga)       Traceability Index      ROBDD Path              UML Metadata            SCPG Container
                        (.tra)                  Summaries (.psa)        (.uma)                  (.scpg)
[~] partial             [~] partial             [~] partial             [~] partial             [~] partial
```

Every phase runs end to end on Java input and writes its artifact. None of the phases is complete against its design document.

---

### Phase 1: Lexical Ingestion & Token Corpus Construction *(Partial)*

- [~] Tree-sitter scanner integration — Java only (`tree-sitter-java` 0.20.2). Kotlin uses the Java grammar; every other extension falls back to the JavaScript grammar (`src/ingestion/adapter/registry.rs`).
- [?] Monotonic 32-bit `token_id` allocation as the traceability anchor (`src/ingestion/allocator.rs`).
- [?] 64-bit FNV-1a deduplicating `StringInterner` (`src/ingestion/interner.rs`).
- [?] 16-byte `TokenRecord` forward index and `TokenEntry` backward index.
- [~] `.tca` serializer with CRC-64/ECMA trailer — the Phase 2 reader does not check the magic, version or CRC.
- [ ] Syntax-error model: record `ERROR`/`MISSING` counts, byte ranges, BOM and CRLF per file in the `.tca`.
- [ ] Parse timeout and cancellation; iterative CST traversal.
- [ ] Upgrade to tree-sitter 0.25+ and a current `tree-sitter-java`.

---

### Phase 2: CST Reduction & BP AST Encoding *(Partial)*

- [?] Balanced Parentheses sequence builder ($B \in \{(, )\}^{2n_{\text{ast}}}$).
- [~] Navigation indexes — implemented with a `u32` jump-table entry per BP position, a rank directory and a full sparse-table RMQ (O(n log n) words); `select1` scans bit by bit. This is not a succinct structure.
- [~] CST reduction — the Java reducer drops `import_declaration`, generic, array, scoped and wildcard type nodes, and `ERROR` subtrees.
- [?] Token range propagation per AST node.
- [~] `.bpa` serializer — readers trust file-supplied lengths; indexes are rebuilt on every load.
- [ ] Range-min-max tree (Navarro–Sadakane) or equivalent o(n)-bit navigation, with a measured bits-per-node figure (suite S15).

---

### Phase 3: Symbol Table & Type Hierarchy Construction *(Partial)*

- [~] 5-pass symbol resolution — imports never reach Pass 2, there is no same-package lookup, lookup ignores symbol kind, and several passes are quadratic.
- [~] Scope graph name binding — scopes open per declaration, not per block, `for`, `catch` or lambda.
- [?] Type hierarchy edge builder ($E^{\text{TH}}$: `extends`, `implements`).
- [~] `.sta` serializer — type-reference resolutions and import edges are not persisted; output order follows hash iteration.
- [ ] JLS 6.4.1 / 7.5 resolution order (local and member types, single-type imports, same package, on-demand imports, `java.lang`).
- [ ] Structured type references (generics, arrays, qualified and wildcard types, type parameters).
- [ ] Stable symbol identities (qualified name plus signature) instead of positional ids.

---

### Phase 4: Control Flow Graph & Dominator Analysis *(Unverified)*

- [?] Basic block partitioning with reachability pruning (`src/cfg/`).
- [?] CSR adjacency encoder (`offsets`, `adj`).
- [?] Statement dispatch for `if`, `while`, `for`, `do-while`, `try-catch`.
- [?] Cooper et al. iterative immediate dominators and dominance frontiers.
- [?] `.cfa` serializer.
- [ ] Differential tests against hand-checked fixture CFGs.

---

### Phase 5: SSA Conversion & CDG *(Partial)*

- [?] Pruned-SSA liveness, Cytron φ placement, dominator-tree renaming.
- [?] Control Dependence Graph via post-dominators.
- [ ] **IFDS data-flow framework** (Reps–Horwitz–Sagiv tabulation). `src/ssa/ifds.rs` marks statements by AST shape and propagates nothing; it is not IFDS.
- [ ] IDE extension for typestate and constant propagation; configurable sources, sinks and sanitizers.
- [?] `.ssa` serializer.

---

### Phase 6: Inter-procedural Call Graph & Points-To Analysis *(Partial)*

- [?] Call-site extraction and call graph ($E^{\text{CG}}$).
- [~] Class Hierarchy Analysis dispatch.
- [ ] **1-CFA** — the current step looks up receiver types and has no call-string context.
- [ ] **Andersen points-to** — `src/cg/points_to.rs` seeds allocation sets and propagates only along φ copies; there are no assign, load, store, call or return constraints.
- [ ] RTA and VTA.
- [?] `.cga` serializer.

---

### Phase 7: Bidirectional Traceability Index *(Partial)*

- [?] Aggregation of `token_id` anchors across the prior artifacts.
- [~] Forward index (source → entity) — binary search followed by a scan; not O(log n + k).
- [?] Dense backward index (entity → source) with O(1) lookup.
- [~] UMLLink hash chain — bitwise CRC-32.
- [ ] Incremental invalidation — `src/tra/delta/` has no callers and compares positional ids.
- [?] `.tra` serializer.

---

### Phase 8: ROBDD Path Summary Computation *(Partial)*

- [~] Path-constraint encoder $f_{\text{paths}}$ — leaves the edges of unreached blocks free and has no exit constraint, so #SAT overstates path counts.
- [?] ROBDD library (unique table, apply, restrict, sat-count).
- [ ] Exact acyclic path counting (Ball–Larus numbering), differentially tested against brute-force enumeration.
- [ ] **Rudell sifting** — the current pass permutes the ordering table but never rewrites the BDD, so it has no effect.
- [ ] Memory-safety evidence for `src/psa/bdd/` from a clean `make miri` run.
- [?] `.psa` serializer.

---

### Phase 9: UML Semantic Metadata Extraction *(Partial)*

- [~] Class, package, component, sequence, activity and use-case extractors — with the gaps listed in [docs/status.md §5](docs/status.md#5-diagram-provenance-matrix).
- [ ] State machine extraction — the current output is a fixed two-state template per class.
- [ ] Composite structure, communication, interaction overview and timing extractors — empty bodies.
- [ ] Object diagram extraction — misreads AST ids as symbol ids; objects are never rendered.
- [ ] Remove the built-in class-name blocklist; filter by symbol kind and origin only.
- [ ] Deterministic record order (sorted by stable keys).
- [?] `.uma` serializer.

---

### Phase 10: SCPG Container, Query Engine & Diagram Studio *(Partial)*

- [~] 11-section `.scpg` writer — only the string-table section carries data; the others hold element counts.
- [ ] Memory-mapped open — `MemoryMappedSCPG::open` reads the whole file; no query reads from it.
- [ ] Validated loading (magic, version, bounded section count, checked offsets, checksums) with corruption tests and a fuzz target.
- [ ] `source_hash` computed from per-file SHA-256 digests (currently zeros).
- [ ] CFL-reachability with call/return matching — `src/scpg/query/cfl.rs` is plain breadth-first reachability.
- [?] LRU query cache.
- [ ] Incremental delta update — `src/scpg/incremental/` is a stub that XORs the hash with a constant.
- [ ] Compiler-IR views (CFG, DFG, CDG, call graph, ROBDD) rendered from `.cfa`, `.ssa`, `.cga` and `.psa` — currently templates.
- [ ] XMI 2.5.1 / UML 2.5.1 export — the current exporter emits placeholder class shells.
- [ ] Byte-identical output across runs.
- [?] Declarative `web/diagrams/manifest.json` defining 19 diagram types, 16 relationship kinds and 24 node kinds.
- [~] Zero-backend web studio on GitHub Pages — uses a heuristic JavaScript extractor over a sample of files, not the Rust engine.

---

## Planned Platform Work

Ordered by dependency: each item builds on the ones above it.

- [ ] **Benchmark and evaluation harness** following the [measurement protocol](docs/status.md#7-measurement-protocol): independent oracles, pinned corpora, committed JSON results, and README numbers generated from them. CI tiers: a pull-request smoke tier and a nightly full tier.
- [ ] **Language frontends with a versioned support matrix**: a real Kotlin grammar (`tree-sitter-kotlin-ng`) with Phase 1, 2 and 3 adapters; registration of one adapter per language for every phase from a single table; unsupported extensions rejected with a diagnostic instead of the JavaScript fallback; per-language conformance fixtures.
- [ ] **Polyglot JVM model**: one namespace keyed by binary names and descriptors, Kotlin facade classes and accessor aliases, and explicit boundary nodes for JNI `native` methods.
- [ ] **Real interprocedural analyses**: field-sensitive Andersen over assign/load/store/call/return constraints with on-the-fly call-graph construction, RTA and VTA, IFDS/IDE tabulation over an ICFG, and a source/sink/sanitizer specification format.
- [ ] **Incremental, commit-aware analysis**: stable file-local ids, per-file caches keyed by content hash, tree-sitter edit-based reparsing, demand-driven recomputation of only the affected functions, and a `--baseline <commit>` mode.
- [ ] **Standard interop**: stable SCIP-style symbol strings, SCIP export, a query language, and an LSP server backed by the traceability index.
- [ ] **WebAssembly engine target**: compile the Rust pipeline to WebAssembly so the studio runs the same extractor as the CLI.
- [ ] **Bidirectional UML-to-code editing**: propagate canvas edits back into source.
- [ ] **Real-time collaborative canvas**.
