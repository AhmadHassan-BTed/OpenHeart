# Succinct Compositional Program Graph (SCPG) — Overview & Specification

## 1. Executive Summary & Ideology

The **Succinct Compositional Program Graph (SCPG)** is an advanced static program analysis graph and bidirectional UML generation engine. It addresses the architectural flaws of traditional Code Property Graphs (CPGs) by replacing heap-allocated, pointer-heavy graph structures with succinct data representations, cache-friendly array encodings, and binary decision diagrams.

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

## 3. Mathematical Foundations of SCPG

### 3.1 Formal Definition

An SCPG is a 7-tuple:

$$\mathcal{G} = (V, E, \nu, \varepsilon, \tau, \rho, \Sigma_\Phi)$$

- **$V$**: Vertex set partitioned as $V = V_{\text{tok}} \sqcup V_{\text{syn}} \sqcup V_{\text{bb}} \sqcup V_{\text{ssa}} \sqcup V_{\text{sym}}$.
- **$E$**: Edge set $E \subseteq V \times V \times \Sigma_E$ with $\Sigma_E = \{ \text{AST\_CHILD}, \text{CFG\_TRUE}, \text{CFG\_FALSE}, \text{CFG\_UNCOND}, \text{DFG\_DEF}, \text{DFG\_USE}, \text{CDG\_TRUE}, \text{CDG\_FALSE}, \text{CG\_CALL}, \text{CG\_RETURN}, \text{TH\_EXTENDS}, \text{TH\_IMPLEMENTS}, \text{TH\_USES} \}$.
- **$\tau$**: Source location mapping $\tau: V_{\text{tok}} \to \mathbb{N}^4$ assigning $(\text{file\_id}, \text{line}, \text{col}, \text{len})$.
- **$\Sigma_\Phi$**: Reduced Ordered Binary Decision Diagrams (ROBDDs) encoding feasible path sets per function.

### 3.2 Subsumption Lattice

The vertex partition forms a strict subsumption lattice:

$$V_{\text{tok}} \subset V_{\text{syn}} \subset V_{\text{bb}} \subset V_{\text{sym}}$$

Every token is an AST leaf; every AST node belongs to a basic block; every basic block belongs to a function symbol.

---

## 4. Layered Storage Engine

1. **Layer 1: Balanced Parentheses (BP) AST**
   - Encodes ordered AST forest into a parentheses sequence $B \in \{(, )\}^{2n_{\text{ast}}}$.
   - $O(1)$ operations via Sadakane/Munro-Raman rank/select: `parent`, `first_child`, `next_sibling`, `subtree_size`, `LCA`.
   - **Compression**: 1M AST nodes stored in ~4.65 MB (versus 32 MB pointer trees).

2. **Layer 2: Compressed Sparse Row (CSR) CFG**
   - Adjacency offset array `offsets[0..n_bb]` and successor array `adj[0..m_cfg]`.
   - $O(\text{outdeg})$ block successor enumeration with sequential memory access.

3. **Layer 3: Wavelet Tree Edge Compression**
   - Bit-packed wavelet tree over $\Sigma_E$ supporting $O(\log \sigma)$ edge access and type-filtered neighbor enumeration.

4. **Layer 4: Static Single-Assignment (SSA) DFG**
   - Single definition site per variable $v_i^k$. Sparse def-use storage bounded by $O(3n)$ operand uses.

5. **Layer 5: ROBDD Path Summaries**
   - Propositional path condition encoding $f_{\text{paths}}$ with Shannon expansion and sifting dynamic variable reordering.
   - Exact path counting (#SAT) and path feasibility checks in $O(|\text{ROBDD}|)$ time.

---

## 5. Universal Source Traceability Protocol

- **`token_id` Anchor**: Universal monotonic $u32$ identifier assigned during scanning.
- **Forward Index ($O(\log n)$)**: Source position $(file\_id, line, col) \to token\_id$ via binary search on packed $u48$ keys.
- **Backward Index ($O(1)$)**: Direct array access $BI[token\_id] \to (file\_id, line, col, len)$.
- **UML Element Link**: Embeds `UMLLink` record into every generated UML diagram element for $O(1)$ stale-link invalidation on incremental source edits.
