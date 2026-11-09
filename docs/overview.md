# Succinct Compositional Program Graph (SCPG) — Technical Overview

> **Status.** This page describes the SCPG *design*. The reference implementation realises part of it; per-phase status, the language and diagram support matrices and the list of withdrawn claims are in **[status.md](./status.md)**.

## 1. System Philosophy & Architectural Vision

The **Succinct Compositional Program Graph (SCPG)** is a static program-analysis graph and UML generation design created and maintained by **Ahmad Hassan (B-Ted)**.

👉 **[Launch OpenHeart Web Studio Portal (GitHub Pages)](https://ahmadhassan-bted.github.io/OpenHeart/)**

The design goal is a single representation that unifies Abstract Syntax Trees (AST), Control Flow Graphs (CFG), data flow in SSA form, Call Graphs (CG) and Type Hierarchies (TH), anchored to source tokens so that every derived element — including UML diagram elements — can be traced back to an exact source range. The design targets compact, array-based encodings (balanced parentheses for trees, CSR for graphs) instead of per-node heap objects. Whether the reference implementation achieves any particular space or time figure is an empirical question that has not yet been measured (see [§6](#6-accuracy-and-performance-evaluation)).

---

## 2. Positioning Against Existing Program Representations

The comparison below is qualitative. It deliberately contains no storage or memory figures for other systems: none have been reproduced on a common corpus and hardware, and published figures for older storage backends no longer describe current releases. Any future quantitative comparison must label each number as **published** (with a primary-source citation) or **reproduced** (by the harness in [status.md §7](./status.md#7-measurement-protocol)).

### 2.1 Code Property Graph (Joern)
- Joern stores AST, CFG and program-dependence information in one property graph and offers a query language over it. Since 2024 Joern uses the columnar **flatgraph** storage backend; per-record costs quoted for its earlier OverflowDB backend do not apply to current releases.
- **Design difference**: the SCPG targets an ordinal (balanced-parentheses) AST encoding, precomputed per-function path summaries, and a dense token-level traceability index with a stated round-trip property.

### 2.2 CodeQL
- CodeQL extracts each supported language into a relational database and evaluates Datalog-style queries over it; it publishes per-language and per-version support. Its Kotlin extractor writes into the Java database schema, so mixed Java/Kotlin code shares one model.
- **Design difference**: the SCPG targets UML projection and source-to-diagram traceability as first-class outputs, which CodeQL does not provide.

### 2.3 LLVM IR & MLIR
- Compiler IRs are designed for optimisation and code generation. Lowering replaces source-level constructs (for example C++ virtual calls with vtable loads), so class hierarchies are not available for UML reconstruction, and debug locations attach to instructions rather than tokens.

### 2.4 Eclipse EMF / XMI
- EMF/XMI is a modelling and serialisation standard. It provides no program-analysis layer: analysis requires deserialising the model into an in-memory object graph first.

### 2.5 JVM analysis frameworks (WALA, Soot/SootUp, OPAL, Tai-e)
- These frameworks provide mature interprocedural analyses (call-graph construction, points-to, IFDS/IDE) over bytecode or a typed intermediate language. They do not generate UML views or token-level traceability. They are the natural **reference oracles** for OpenHeart's own call-graph and points-to results (see [status.md §7.3](./status.md#73-suites)).

---

## 3. Formal Definition of SCPG

Formally, an SCPG is a 7-tuple:

$$\mathcal{G} = (V, E, \nu, \varepsilon, \tau, \rho, \Sigma_\Phi)$$

### 3.1 Vertex Partition & Anchoring

The finite vertex set $V$ is partitioned into five disjoint sub-domains:

$$V = V_{\text{tok}} \sqcup V_{\text{syn}} \sqcup V_{\text{bb}} \sqcup V_{\text{ssa}} \sqcup V_{\text{sym}}$$

- $V_{\text{tok}}$: Lexical tokens scanned from source files (AST leaves).
- $V_{\text{syn}}$: Syntactic AST internal nodes.
- $V_{\text{bb}}$: Basic blocks (maximal straight-line code sequences).
- $V_{\text{ssa}}$: Variable definitions in Static Single-Assignment (SSA) form.
- $V_{\text{sym}}$: Symbol declarations (functions, classes, interfaces, fields, packages).

The layers are disjoint, but each layer is **anchored** in the layers below it: every AST node spans a contiguous token range, every basic block corresponds to a statement sub-sequence, every SSA definition traces to a defining statement, and every symbol traces to its declaration node. Through these anchors every entity has a source range.

- **Backward projection** (entity → source range) is a dense-array lookup, O(1) per entity.
- **Forward projection** (source position → entities) is a search over a sorted span index. The design target is $O(\log n + k)$ for $k$ results; at the audited baseline `fc218cd` the implementation scanned after the binary search (see [status.md §4](./status.md#4-phase-capability-ledger)).

---

## 4. Phase Specifications & Binary Artifact Layouts

### 4.1 Phase 1: Lexical Ingestion & Token Corpus (.tca)

Each token's source position is packed into a single 64-bit sort key (`build_sort_key` in `src/core/types/token.rs`):

$$\text{sort\_key} = (\text{file\_id} \ll 48) \mid (\text{line} \ll 24) \mid (\text{col} \ll 8)$$

- **`file_id`** (bits 63..48): 16-bit integer (up to 65,536 files).
- **`line`** (bits 47..24): 24-bit integer (up to 16,777,215 lines).
- **`col`** (bits 23..8): 16-bit integer (up to 65,536 columns).
- **bits 7..0**: reserved (zero).

Positions outside these ranges cannot be represented. `try_build_sort_key` rejects them with an error rather than truncating them.

### 4.2 Phase 2: CST Reduction & Balanced Parentheses AST Encoding (.bpa)

Every tree-sitter CST node is classified by an `ASTReductionAdapter` as `Keep`, `Eliminate`, `Drop` or `Token`. Only Java has a dedicated reduction adapter; see [status.md §2](./status.md#2-language-support-matrix).

#### Balanced Parentheses (BP) Navigation Formulas

- **Pre-order Index**: $\text{preorder\_idx}(\text{bp\_pos}) = \text{rank}_1(\text{bp\_pos}) - 1$
- **Open Position**: $\text{open\_pos}(\text{pre\_idx}) = \text{select}_1(\text{pre\_idx} + 1)$
- **Matching Position**: $\text{match\_pos}(\text{pos})$ via the jump table
- **Parent**: $\text{parent\_map}[\text{pre\_idx}]$
- **Subtree Size**: $\text{subtree\_size}(\text{pre\_idx}) = (\text{cp} - \text{op} + 1) / 2$
- **Lowest Common Ancestor (LCA)**: $\text{lca}(u, v) = \text{rank}_1(\text{range\_min}(\text{op}_u, \text{op}_v)) - 1$ via sparse-table RMQ

At the audited baseline `fc218cd`, rank, the jump table, `parent_map` and the sparse-table RMQ answered in O(1), but `select1` scanned bit by bit, so operations that need `open_pos` were not O(1). The jump table and sparse table also made the structure O(n log n) words rather than succinct; [succinct_structures.md](./succinct_structures.md) compares the textbook bounds with the implemented layout.

---

## 5. 10-Phase SCPG Pipeline Artifact Flow

```text
Phase 1:  Source Text ────────► TokenCorpusArtifact (.tca)
Phase 2:  .tca ───────────────► BPASTArtifact (.bpa)
Phase 3:  .tca + .bpa ────────► SymbolTableArtifact (.sta)
Phase 4:  .bpa + .sta ────────► CFGArtifact (.cfa)
Phase 5:  .cfa + .sta ────────► SSAArtifact (.ssa)
Phase 6:  .ssa + .sta ────────► CallGraphArtifact (.cga)
Phase 7:  Artifacts 1-6 ──────► TraceabilityArtifact (.tra)
Phase 8:  .cfa + .ssa ────────► PathSummaryArtifact (.psa)
Phase 9:  Artifacts 1-8 ──────► UMLMetadataArtifact (.uma)
Phase 10: Artifacts 1-9 ──────► SCPG Container (.scpg) & QueryEngine
```

---

## 6. Accuracy and Performance Evaluation

**No accuracy, space or performance result has been published for OpenHeart.**

An earlier version of this page reported $F_1 = 1.0000$ on six repositories from `ruthless_verify.py`. That result is **withdrawn**: its ground truth was a keyword regular expression keyed by simple class name, an exclusion list shared with the engine's own class-name blocklist was removed from both sides, only class-name sets were compared, and the repositories were not pinned to commits. It could not detect wrong packages, nesting errors, duplicate simple names or any error in the semantic phases. The earlier performance table (4.65 MB per 1M AST nodes, CFG and lookup timings, comparisons with an uncited Neo4j store) was not measured and is also withdrawn.

Future results must follow the [measurement protocol](./status.md#7-measurement-protocol): independent oracles that share no lists or heuristics with the engine, fully qualified names, pinned corpora with commit SHAs, bootstrap confidence intervals, and machine-readable results committed with the scripts that produced them.

What can be stated from the code without measurement is the asymptotic shape of the structures implemented at the audited baseline `fc218cd`:

| Structure | Implemented layout | Asymptotic size | Lookup |
|---|---|---|---|
| AST topology | BP bitstring, `u32` jump table per BP position, rank directory, preorder arrays, full sparse-table RMQ | O(n log n) words (dominated by the sparse table) | O(1) rank, parent, match and LCA; `select1` linear scan |
| CFG adjacency | CSR `offsets` + `adj` per function | O(n_bb + m_cfg) words | O(outdeg) successor scan |
| Backward traceability | Dense arrays indexed by entity id | O(n) words | O(1) |
| Forward traceability | Sorted span records | O(n) words | Binary search followed by a scan |
| Path summaries | ROBDD node table, 12 bytes per node | O(Σ |ROBDD(f)|) | — |
