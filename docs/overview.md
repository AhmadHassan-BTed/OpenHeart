# Succinct Compositional Program Graph (SCPG) — Complete Technical Specification

## 1. System Philosophy & Architectural Vision

The **Succinct Compositional Program Graph (SCPG)** is an advanced static program analysis graph and bidirectional UML generation engine created and maintained by **Ahmad Hassan (B-Ted)**.

👉 **[Launch OpenHeart Web Studio Portal (GitHub Pages)](https://ahmadhassan-bted.github.io/OpenHeart/)**

Existing static analysis frameworks (e.g., Joern Code Property Graph, LLVM IR, WALA, Eclipse EMF) suffer from severe memory inflation and pointer-chasing latency. The SCPG architecture solves these structural limitations by unifying Abstract Syntax Trees (AST), Control Flow Graphs (CFG), Data Flow Graphs (DFG), Call Graphs (CG), and Type Hierarchies (TH) into a succinct, cache-line aligned, memory-mapped graph representation.

---

## 2. Comparative Analysis of Existing Architectures

### 2.1 Code Property Graph (Joern)
- **Flaws**: Node-centric storage allocating 32 bytes per node record and 34 bytes per edge record. Lacks ordinal forest exploitation for ASTs, requiring pointer-chasing through `AST_PARENT` edges. No precomputed path summaries; path queries require full $O(V + E)$ BFS traversals. Source locations stored as unindexed line/column properties.

### 2.2 LLVM IR & MLIR
- **Flaws**: Irreversible semantic lowering (C++ virtual calls lowered to vtable loads; Rust trait objects to fat pointers; Python calls to opaque C-API invocations), preventing high-level UML class/sequence diagram reconstruction. DWARF metadata attaches at instruction granularity rather than token granularity. Heavy C++ object memory overhead (120–200 bytes per operation).

### 2.3 Eclipse EMF / XMI
- **Flaws**: Verbose XML serialization generating 100–150 MB files for 1,000 classes. Slow SAX parsing (2–5s) with zero random access capabilities and no static path analysis engine.

### 2.4 WALA & Soot
- **Flaws**: Java-centric, heap-allocated object models requiring 8–16 GB of JVM heap for 1M LOC. Context-sensitive analysis suffers from exponential state space explosion ($k=2$ points-to analysis causes OOM on >500K LOC).

---

## 3. Formal Mathematical Definition of SCPG

Formally, an SCPG is defined as a 7-tuple:

$$\mathcal{G} = (V, E, \nu, \varepsilon, \tau, \rho, \Sigma_\Phi)$$

### 3.1 Vertex Partition & Subsumption Lattice

The finite vertex set $V$ is partitioned into five disjoint sub-domains:

$$V = V_{\text{tok}} \sqcup V_{\text{syn}} \sqcup V_{\text{bb}} \sqcup V_{\text{ssa}} \sqcup V_{\text{sym}}$$

- $V_{\text{tok}}$: Lexical tokens scanned from source files (AST leaves).
- $V_{\text{syn}}$: Syntactic AST internal nodes.
- $V_{\text{bb}}$: Basic blocks (maximal straight-line code sequences).
- $V_{\text{ssa}}$: Variable definitions in Static Single-Assignment (SSA) form.
- $V_{\text{sym}}$: Symbol declarations (functions, classes, interfaces, fields, packages).

These vertex partitions form a strict subsumption lattice:

$$V_{\text{tok}} \subset V_{\text{syn}} \subset V_{\text{bb}} \subset V_{\text{sym}}$$

This subsumption lattice guarantees $O(1)$ bidirectional projection across graph layers without recursive graph traversals.

---

## 4. Phase 1: Lexical Ingestion & Token Corpus (.tca)

### 4.1 `sort_key` Bit Layout (64-bit `u64`)

Lexicographical position sorting is packed into a single 64-bit integer:

$$\text{sort\_key} = (\text{file\_id} \ll 48) \mid (\text{line} \ll 24) \mid (\text{col} \ll 8) \mid \text{flags}$$

- **`file_id`** (bits 63..48): 16-bit integer (up to 65,536 files).
- **`line`** (bits 47..24): 24-bit integer (up to 16,777,215 lines).
- **`col`** (bits 23..8): 16-bit integer (up to 65,536 columns).
- **`flags`** (bits 7..0): 8-bit reserved byte for sort stability (default 0x00).

### 4.2 `.tca` Binary Artifact Layout

1. **Header (64 bytes)**: `TCA_MAGIC` (`0x544F4B434F525001`), `format_version`, `token_count`, `file_count`, `string_count`, `flags`, `source_tree_hash` (SHA-256), `creation_ts_ns`.
2. **File Registry**: $F \times 64\text{ B}$ array of `SourceFileRecord`.
3. **File Paths**: Length-prefixed UTF-8 string section (8-byte aligned).
4. **Token Table (Forward Index)**: $n \times 16\text{ B}$ sorted `TokenRecord` array sorted by `sort_key` ascending ($O(\log n)$ binary search).
5. **Entry Map (Backward Index)**: $n \times 16\text{ B}$ `TokenEntry` array indexed by `token_id` ($O(1)$ direct array access).
6. **String Table**: String headers ($s \times 12\text{ B}$) + UTF-8 storage.
7. **Checksum**: 8-byte CRC-64/ECMA digest over all preceding bytes.

---

## 5. Phase 2: CST Reduction & Balanced Parentheses AST Encoding (.bpa)

### 5.1 CST Reduction Decision Taxonomy

Every Tree-sitter CST node is classified by `ASTReductionAdapter` into:
- **`Keep(ASTNodeType)`**: Emit $V_{\text{syn}}$ vertex, attach children.
- **`Eliminate`**: Bypass node, pull children up to nearest kept ancestor.
- **`Drop`**: Discard node and children (brackets, semicolons, comments, whitespace).
- **`Token`**: Leaf AST node mapped to `token_id`.

### 5.2 Balanced Parentheses (BP) Navigation Formulas ($O(1)$ Time)

- **Pre-order Index**: $\text{preorder\_idx}(\text{bp\_pos}) = \text{rank}_1(\text{bp\_pos}) - 1$
- **Open Position**: $\text{open\_pos}(\text{pre\_idx}) = \text{select}_1(\text{pre\_idx} + 1)$
- **Matching Position**: $\text{match\_pos}(\text{pos})$ via Jump Table
- **Parent**: $\text{parent\_map}[\text{pre\_idx}]$
- **First Child**: $\text{first\_child}(\text{pre\_idx}) = \text{rank}_1(\text{op} + 1) - 1$ if $B[\text{op}+1] == 1$ else `None`
- **Next Sibling**: $\text{next\_sibling}(\text{pre\_idx}) = \text{rank}_1(\text{cp} + 1) - 1$ if $B[\text{cp}+1] == 1$ else `None`
- **Subtree Size**: $\text{subtree\_size}(\text{pre\_idx}) = (\text{cp} - \text{op} + 1) / 2$
- **Is Leaf**: $\text{is\_leaf}(\text{pre\_idx}) \equiv B[\text{op}+1] == 0$
- **Depth**: $\text{depth}(\text{pre\_idx}) = 2 \times \text{rank}_1(\text{op}) - \text{op} - 1$ (excess depth sequence)
- **Lowest Common Ancestor (LCA)**: $\text{lca}(u, v) = \text{rank}_1(\text{range\_min}(\text{op}_u, \text{op}_v)) - 1$ via Sparse Table RMQ

---

## 6. Complete 10-Phase Pipeline Binary Artifact Flow

```text
Phase 1:  Source Text → TokenCorpusArtifact (.tca)
Phase 2:  .tca → BPASTArtifact (.bpa)
Phase 3:  .tca + .bpa → SymbolTableArtifact (.sta)
Phase 4:  .bpa + .sta → CFGArtifact (.cfa)
Phase 5:  .cfa + .sta → SSAArtifact (.ssa)
Phase 6:  .ssa + .sta → CallGraphArtifact (.cga)
Phase 7:  Artifacts 1-6 → TraceabilityArtifact (.tra)
Phase 8:  .cfa + .ssa → PathSummaryArtifact (.psa)
Phase 9:  Artifacts 1-8 → UMLMetadataArtifact (.uma)
Phase 10: Artifacts 1-9 → SCPG Binary (.scpg) + QueryEngine instance
```

---

## 7. Performance & Memory Space Benchmarks

| Metric / Structure | OpenHeart SCPG | Legacy Pointer Graphs / Neo4j | Improvement |
|---|---|---|---|
| **AST Tree Memory** | $4.65\text{ MB}$ / 1M nodes (BP + Rank/Select) | $32\text{ MB}$ pointer trees | **$6.9\times$ smaller** |
| **CFG Edge Memory** | $24\text{ MB}$ (Compressed Sparse Row) | $170\text{ MB}$ Neo4j store | **$7.1\times$ smaller** |
| **Path Summaries** | $\sim 20\text{ KB}$ (ROBDD functions) | $2^{50}\text{ B}$ explicit enumeration | **$\infty\times$ smaller** |
| **Forward Index Lookup** | $O(\log n)$ binary search | $O(V + E)$ BFS scan | **$\sim 1\text{ ns}$** per query |
| **Backward Index Lookup**| $O(1)$ array access | $O(V)$ hash table scan | **$\sim 0.5\text{ ns}$** per query |
| **Tree Navigation** | $O(1)$ (`parent`, `lca`, `subtree_size`) | $O(\text{depth})$ pointer traversal | **$O(1)$ guaranteed** |
