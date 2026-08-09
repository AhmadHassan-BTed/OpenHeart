---

## Phase 5: SSA Conversion & Data Flow Graph Construction

### 5.1 Phase Mandate & Exact Boundaries

Phase 5 transforms the control flow structure from Phase 4 into its most analytically powerful form: Static Single Assignment (SSA). Every variable is renamed so that each name has exactly one definition site anywhere in the program. This single constraint — one name, one definition — makes def-use relationships explicit, sparse, and O(1)-navigable, which is the mathematical precondition that makes both Phase 8's ROBDD path summaries and the IFDS-based data-flow analyses tractable.

Phase 5 also constructs the Control Dependence Graph (CDG) — the dual structure of the dominator tree — and executes three IFDS distributive data-flow analyses whose results annotate the SSA graph with taint, nullability, and type-state information required by Phase 9's UML behavioral metadata extraction.

Phase 5 does NOT build the call graph (Phase 6), construct path BDDs (Phase 8), or generate UML metadata (Phase 9). Its output is purely intra-procedural SSA and CDG structure.

**Inputs:** `BPASTArtifact (.bpa)`, `SymbolTableArtifact (.sta)`, `CFGArtifact (.cfa)`.

**Output:** `SSAArtifact (.ssa)` — SSA variable table, φ-function table, def-use CSR, CDG CSR, and three IFDS result bitmaps.

---

### 5.2 Mathematical Foundations

#### 5.2.1 SSA Form — Formal Definition

**Definition (SSA Variable):** In SSA form, each original variable v declared in the symbol table (with `symbol_id` s) is split into a set of SSA variables {v^0, v^1, ..., v^k}, one per definition site. Each SSA variable v^i has a unique `ssa_id` and satisfies the single-assignment property: there is exactly one statement in the entire function that writes to v^i.

**Definition (φ-function):** A φ-function inserted at the entry of block B with predecessors {B_1,...,B_p} for original variable v has the form:

v^result = φ(v^{arg_1}, v^{arg_2}, ..., v^{arg_p})

where v^{arg_j} is the SSA version of v that is live at the exit of predecessor B_j. The φ-function semantically reads the argument corresponding to the predecessor through which control actually arrived.

**Key SSA property:** In SSA form, the use-def relation is immediate from the variable name. Given any use of `v^i`, the unique definition of `v^i` is found in O(1) by array lookup: `def_site[ssa_id]`. There are no ambiguous use-def chains; there is no need to run a reaching-definitions analysis at all. This is why SSA is the universal foundation for program analysis.

#### 5.2.2 Cytron's φ-Placement Algorithm (Phase A)

For each original variable v, let defsites(v) = {block b : b contains an assignment to v}.

```
phase_A_place_phi_functions(v, DF[], defsites):
    worklist   = copy(defsites(v))       // blocks that define v
    phi_placed = {}                      // blocks where φ(v) is inserted

    while worklist ≠ ∅:
        b = worklist.pop_any()
        for each y in DF[b]:             // DF from Phase 4 CFA
            if y ∉ phi_placed:
                insert_phi(y, v)         // create new SSA φ at entry of y
                phi_placed.add(y)
                if y ∉ defsites(v):
                    worklist.add(y)      // y now defines v (via φ), propagate
```

**Correctness:** A φ-function for v at block y is needed iff y merges control paths that arrive with different values of v. The dominance frontier captures exactly the set of such blocks: a block y ∈ DF(b) has a predecessor b' dominated by b (where b defines v) and another predecessor not dominated by b. Hence v from b's definition can reach y along one path but not necessarily all paths — a φ is required.

**Termination:** The worklist terminates because each block enters at most once per variable (tracked by `phi_placed`), and `|phi_placed| ≤ |B_f|` is finite.

**Pruned SSA optimization:** The algorithm above inserts "maximal" φ-functions — some inserted φ-functions may be unnecessary if v is dead (not live) at point y. We apply pruned SSA (Briggs et al. 1994): before Phase A, compute liveness by a standard backward data-flow fixpoint, and only insert φ(v) at y if v ∈ LiveIn(y). This eliminates typically 30–50% of φ-functions in practice.

#### 5.2.3 Variable Renaming Algorithm (Phase B)

Phase B performs a DFS over the dominator tree. For each original variable v, a version stack `S[v]` tracks the current live SSA version as we descend the dominator tree.

```
phase_B_rename(block b, S: HashMap<sym_id, Vec<ssa_id>>):
    // ─ Part 1: Process φ-functions at head of b ─────────────────────
    for each φ-function (v, target) at start of b:
        new_id = fresh_ssa_id(v)        // allocate new SSA variable
        S[v].push(new_id)               // push onto version stack
        phi_record[target].ssa_id = new_id

    // ─ Part 2: Process ordinary statements in b ──────────────────────
    for each stmt s in b.stmts (in execution order):
        // Rename all USES first (right-hand side semantics)
        for each variable v used by s:
            s.replace_use(v, top(S[v]))     // substitute current version

        // Rename all DEFS (left-hand side)
        for each variable v defined by s:
            new_id = fresh_ssa_id(v)
            S[v].push(new_id)               // push new version
            record def_site(new_id) = s.pre_order_idx
            record def_block(new_id) = b.id

    // ─ Part 3: Fill φ-function arguments in CFG successors ──────────
    for each successor s of b in the CFG:
        for each φ-function φ(v) in s:
            phi_record.set_arg_for_pred(b.id, v, top(S[v]))

    // ─ Part 4: Recurse over dominator-tree children ──────────────────
    for each child c of b in the idom[] dominator tree:
        phase_B_rename(c, S)

    // ─ Part 5: Restore version stack (pop what we pushed in Parts 1+2)
    // Pop in reverse order of pushes to maintain stack invariant
    for each v that had versions pushed in Parts 1 or 2:
        pop back to the count it had before Part 1
```

**Correctness:** The dominator tree DFS ensures that when we process block b, the stack `S[v]` contains exactly the most recent definition of v that dominates b. This follows from the key property: b's dominators are precisely b's ancestors in the dominator tree — and we've already renamed all of them before reaching b. Therefore `top(S[v])` is the correct SSA version to substitute for each use of v in b.

**The stack restoration in Part 5** is critical: when we return from processing block b (and all of b's dominated subtree), we undo all the pushes made while processing b. This ensures that sibling blocks in the dominator tree — which do not dominate each other — each see the version stack as it existed at their common dominator parent.

#### 5.2.4 SSA Record Layout

```
SSARecord (16 bytes, cache-aligned):
Offset  Size  Field
 0       4    ssa_id        : u32   monotonically assigned, globally unique in function
 4       4    orig_sym_id   : u32   STA symbol_id of the original variable
 8       4    def_stmt      : u32   BP AST pre-order index of defining stmt
                                    u32::MAX for φ-functions (no direct source location)
12       2    version       : u16   SSA version number within orig_sym_id (0-based)
14       1    flags         : u8    IS_PHI:1, IS_PARAM_DEF:1, IS_FIELD_DEF:1,
                                    IS_RETURN_VAL:1, IS_CONST:1
15       1    def_block     : u8    basic block ID (mod 256 — overflow → side table u32)
Total: 16 bytes
```

For the common case (block_id ≤ 255, which covers 99%+ of Java methods), the 1-byte `def_block` is sufficient. Functions with more than 255 blocks store a full u32 in the optional `ExtendedBlockTable` section.

#### 5.2.5 Control Dependence Graph

**Definition (Post-Dominator):** Block d post-dominates block b (d pdom b) iff every path from b to the EXIT block passes through d. The post-dominator tree `T_pdom` is computed by reversing all CFG edges (swapping every edge (u,v) to (v,u)), making EXIT the new entry, and running the Cooper iterative algorithm on the reversed graph.

**Definition (Control Dependence):** Block Y is control-dependent on block X iff there exists a CFG edge (X, S) such that: Y post-dominates S, and Y does not strictly post-dominate X.

Intuitively: X "controls" Y when X is a branch whose outcome determines whether Y executes. If X always exits to a successor that eventually reaches Y (Y pdom S), but X itself can avoid Y (Y not pdom X), then Y's execution is conditional on X's branching decision.

**CDG Construction Algorithm:**

```rust
fn build_cdg(cfg: &FunctionCFG, ipdom: &[u32]) -> Vec<Vec<u32>> {
    let n = cfg.block_count;
    let mut cdg: Vec<Vec<u32>> = vec![Vec::new(); n]; // cdg[x] = blocks Y that x controls

    for (x, succ_s) in cfg.all_edges() {
        // S is a CFG successor of X
        // Walk up the post-dominator tree from S to ipdom(X)
        // Every block on this path is control-dependent on X
        let ipdom_x = ipdom[x as usize];
        let mut runner = succ_s;
        while runner != ipdom_x {
            if !cdg[x as usize].contains(&runner) {
                cdg[x as usize].push(runner);
            }
            runner = ipdom[runner as usize];
        }
    }
    cdg
}
```

Time: O(m × depth\_pdom) where depth\_pdom = post-dominator tree height ≤ n. In practice O(m) amortized.

**CDG Semantics:** For every CDG edge (X, Y), we store the edge type `CD_TRUE` or `CD_FALSE` to capture which branch of X's conditional results in Y's execution. This is used in Phase 9 to generate precise UML activity diagram guards.

#### 5.2.6 IFDS Framework — Polynomial Data-Flow Analysis

**Definition (IFDS Problem):** Given a program supergraph G\* = (N\*, E\*) (a CFG with inter-procedural call/return edges), a finite universe of data-flow facts D, and a family of flow functions {f\_{m,n} : 2^D → 2^D}, the IFDS analysis computes for each program point n the set of facts valid at n.

**The distributivity requirement:** Every flow function must satisfy f(X ∪ Y) = f(X) ∪ f(Y). This algebraic property is what makes polynomial time possible — it allows path merging at join points without loss of precision.

**Theorem (Reps, Horwitz, Sagiv 1995):** For distributive flow functions, the Meet-over-All-Same-Level-Valid-Paths (MSVP) solution is computable in O(|E\*| × |D|^3) time via the tabulation algorithm, equivalently O(|N| × |D|^2) with path-edge storage.

**The Three Phase 5 IFDS Analyses:**

Analysis 1 — Taint Propagation (for UML Sequence Diagram annotations):
- D = set of taint-source identifiers (external API parameters, database results, user inputs)
- Flow function for assignment `x = expr`: if any variable in `expr` is tainted by source s, then x becomes tainted by s
- Flow function for sanitizer call `x = sanitize(v)`: x is not tainted regardless of v's taint
- Boundary facts Λ = {s : annotated @TaintSource or matching taint-source heuristic}

Analysis 2 — Nullable Pointer Analysis (for null-check annotation in activity diagrams):
- D = {v^i : v^i may hold null at some program point}
- Flow function for `v^i = null`: add v^i to D
- Flow function for `v^i = new T(...)`: v^i is definitely non-null (remove from D)
- Flow function for `if (v^i != null)` guard: on the TRUE branch, v^i is non-null (remove)
- Boundary facts: @Nullable-annotated parameters and return values

Analysis 3 — Type-State Analysis (for UML State Machine diagram generation):
- Per-class state automaton: {states} × {transitions triggered by method calls}
- D = {(v^i, state_s) : variable v^i may be in state s}
- Flow function for `v^i.method()`: transitions (v^i, s) → (v^i, s') where method triggers transition s→s'
- Used to detect: reading from closed streams, double-close bugs, API misuse
- The detected automata are the input to Phase 9's state machine diagram extractor

**IFDS Tabulation Algorithm (sketch):**

```rust
fn tabulate_ifds(
    supergraph:  &ExplodedSupergraph,  // (N×D, E*) graph
    boundary:    &HashSet<(u32, u32)>, // (start_node, fact_id) initial facts
) -> HashMap<(u32, u32), bool> {       // (node, fact) → reachable?

    let mut path_edges  = HashSet::new(); // same-level valid path edges
    let mut summary     = HashMap::new(); // procedure summaries
    let mut worklist    = VecDeque::from_iter(boundary.iter().cloned());

    for &(n, d) in boundary {
        path_edges.insert((start_node, d, n, d));
    }

    while let Some((n, d)) = worklist.pop_front() {
        for each successor edge (n, d) → (m, d') in supergraph:
            if path_edges.insert((n_proc_entry, d_entry, m, d')):
                worklist.push_back((m, d'))

        // Handle call edges: apply procedure summary
        // Handle return edges: propagate through matching call
        // Details: see Reps et al. 1995 Algorithm 2
    }

    // Extract solution: (n, d) is valid iff path_edges contains (entry, 0, n, d)
    path_edges.iter()
        .filter(|(s, d_s, _, _)| *s == start && *d_s == LAMBDA)
        .map(|(_, _, n, d)| (*n, *d))
        .collect()
}
```

The IFDS results are stored as sparse bitmaps: for each analysis, a sorted array of `(ssa_id, fact_id)` pairs where the fact holds. Only the non-zero entries are stored — typical density is 1–5% of all `(ssa_id, fact_id)` pairs.

---

### 5.3 Module Architecture

```
phase5/
├── mod.rs                        # Phase5Stage::run(bpa, sta, cfa) → SSAArtifact
├── ssa/
│   ├── mod.rs                    # SSABuilder: runs Phase A then Phase B per function
│   ├── liveness.rs               # LivenessAnalysis: backward fixpoint for pruned SSA
│   ├── placement.rs              # phase_A_place_phi_functions() — DF worklist
│   ├── renaming.rs               # phase_B_rename() — dominator-tree DFS renaming
│   ├── version_stack.rs          # VersionStack: HashMap<sym_id, Vec<ssa_id>>
│   │                             # with push(), top(), pop_to(saved_depth)
│   └── record.rs                 # SSARecord, PhiRecord, PhiArg definitions
├── defuse/
│   ├── mod.rs                    # DefUseBuilder: collects uses during Phase B
│   └── csr.rs                    # Encodes use-list as CSR (offsets[] + adj[])
├── cdg/
│   ├── mod.rs                    # CDGBuilder: orchestrates post-dom + CDG construction
│   ├── postdom.rs                # compute_post_idom(): reverse CFG + Cooper algorithm
│   └── edges.rs                  # build_cdg() from ipdom[] (§5.2.5)
├── ifds/
│   ├── mod.rs                    # IFDSRunner: runs all three analyses
│   ├── supergraph.rs             # ExplodedSupergraph: (N×D, E*) from SSA + CFA
│   ├── taint.rs                  # TaintAnalysis: flow functions for Σ_taint
│   ├── null_analysis.rs          # NullAnalysis: flow functions for Σ_null
│   ├── type_state.rs             # TypeStateAnalysis: per-class automaton-driven IFDS
│   └── tabulation.rs             # Generic IFDS tabulation solver (Reps et al.)
├── builder.rs                    # SSAArtifactBuilder: assembles all sub-structures
└── serializer.rs                 # SSAArtifact binary I/O (.ssa format)
```

---

### 5.4 Data Structure Specifications

**PhiRecord (variable-length):**

```
PhiRecord header (12 bytes):
  ssa_id:    u32   the new SSA variable defined by this φ-function
  block_id:  u32   basic block where φ is inserted (at block entry)
  arg_count: u16   number of φ arguments (= number of CFG predecessors)
  orig_sym:  u16   symbol_id mod 65536 (full sym_id in SSARecord.orig_sym_id)

PhiArg (8 bytes each), arg_count entries:
  pred_block_id: u32   CFG predecessor block_id for this argument
  arg_ssa_id:    u32   SSA variable ID flowing in from pred_block

Total: 12 + arg_count × 8 bytes
```

**DefUseCSR (per function, for each SSA variable's use list):**

```
def_offsets: (n_ssa + 1) × u32   prefix sum of use counts per SSA variable
use_adj:      total_uses × u32    pre-order index of each use-statement
```

Access: `use_adj[def_offsets[ssa_id]..def_offsets[ssa_id+1]]` = all uses of ssa_id.

**CDGCSR (per function):**

```
cd_offsets: (n_blocks + 1) × u32
cd_adj:     n_cdg_edges × u32         block Y that is control-dependent on block X
cd_types:   wavelet tree over cd_adj  (CD_TRUE, CD_FALSE per edge)
```

**IFDSResultSparse (per analysis):**

```
Header: analysis_id:u8, n_entries:u32
Entries (sorted by ssa_id then fact_id):
  (ssa_id:u32, fact_id:u16) × n_entries
```

Binary search lookup: O(log n\_entries) to check if fact fact\_id holds at ssa\_id.

---

### 5.5 Algorithm Specifications

#### 5.5.1 Phase A: φ-Placement with Pruned SSA

```rust
pub fn place_phi_functions(
    func_sym:  u32,
    bpa:       &BPASTArtifact,
    sta:       &SymbolTableArtifact,
    cfa:       &FunctionCFGData,
    liveness:  &LivenessResult,   // precomputed backward fixpoint
    builder:   &mut SSABuilder,
) {
    let block_count = cfa.block_count as usize;

    // Gather all variables in scope of this function
    let scope_vars = sta.variables_in_scope(func_sym);

    for var_sym in scope_vars {
        // Find all blocks where var_sym is defined (assigned)
        let mut defsites: HashSet<u32> = cfa.all_blocks()
            .filter(|&b| block_defines(b, var_sym, bpa, cfa))
            .collect();

        let mut phi_placed: HashSet<u32> = HashSet::new();
        let mut worklist: VecDeque<u32> = defsites.iter().copied().collect();

        while let Some(b) = worklist.pop_front() {
            for &y in cfa.df(b) {   // dominance frontier from Phase 4 CFA
                // Pruned SSA: only place φ if var_sym is live at entry of y
                if !phi_placed.contains(&y) && liveness.live_in(y, var_sym) {
                    builder.insert_phi(y, var_sym);
                    phi_placed.insert(y);
                    if !defsites.contains(&y) {
                        worklist.push_back(y);
                    }
                }
            }
        }
    }
}
```

#### 5.5.2 Phase B: Variable Renaming

```rust
pub fn rename(
    b:       u32,
    bpa:     &BPASTArtifact,
    cfa:     &FunctionCFGData,
    builder: &mut SSABuilder,
    S:       &mut VersionStack,
) {
    // ── Part 1: φ-function targets at head of b ──────────────────────
    let saved_depths = S.save_depths();  // snapshot current stack depths
    for phi_id in builder.phis_at_block(b) {
        let var = builder.phi_var(phi_id);
        let new_ssa = builder.fresh_ssa(var);
        S.push(var, new_ssa);
        builder.set_phi_target(phi_id, new_ssa);
    }

    // ── Part 2: Rename stmts ─────────────────────────────────────────
    for stmt_node in cfa.stmts_in_block(b) {
        // Process uses first (RHS semantics: right-hand side read before write)
        for var in uses_of_stmt(stmt_node, bpa) {
            let current_ver = S.top(var);
            builder.record_use(current_ver, stmt_node);  // add to use-list
            builder.rename_use(stmt_node, var, current_ver);
        }
        // Process defs (LHS)
        for var in defs_of_stmt(stmt_node, bpa) {
            let new_ssa = builder.fresh_ssa(var);
            builder.set_def_site(new_ssa, stmt_node, b);
            builder.record_def(new_ssa, stmt_node);
            S.push(var, new_ssa);
        }
    }

    // ── Part 3: Fill φ-args in CFG successors ────────────────────────
    for succ in cfa.successors(b) {
        for phi_id in builder.phis_at_block(succ) {
            let var = builder.phi_var(phi_id);
            let current_ver = S.top(var);
            builder.record_use(current_ver, SYNTHETIC_PHI_USE);
            builder.set_phi_arg(phi_id, b /* pred */, current_ver);
        }
    }

    // ── Part 4: Recurse over dominator-tree children ─────────────────
    for child in domtree_children(b, cfa.idom()) {
        rename(child, bpa, cfa, builder, S);
    }

    // ── Part 5: Restore version stack ────────────────────────────────
    S.restore_to(saved_depths);
}
```

**VersionStack invariant:** At entry to `rename(b)`, for every variable v, `S.top(v)` = the SSA version of v that is live at the entry of b. This is maintained by the dominator-tree traversal order.

---

### 5.6 Output Schema: SSAArtifact Binary Format (`.ssa`)

```
╔═══════════════════════════════════════════════════════════════════╗
║  SSA FILE FORMAT v1.0  (all integers: little-endian)             ║
╠════════════════════╦══════════════╦════════════════════════════════╣
║ Section            ║ Size         ║ Description                    ║
╠════════════════════╬══════════════╬════════════════════════════════╣
║ HEADER             ║ 64 B         ║ Magic, counts, cfa_hash link   ║
╠════════════════════╬══════════════╬════════════════════════════════╣
║ FUNCTION DIR.      ║ n_func×20 B  ║ (sym_id, offset, n_ssa, n_phi) ║
╠════════════════════╬══════════════╬════════════════════════════════╣
║ SSA VARIABLE TABLE ║ n_ssa×16 B   ║ SSARecord[] — def sites,       ║
║                    ║              ║ versions, flags. O(1) lookup.  ║
╠════════════════════╬══════════════╬════════════════════════════════╣
║ PHI-FUNCTION TABLE ║ variable     ║ PhiRecord[] per function.      ║
║                    ║              ║ 12B header + args×8B each.     ║
╠════════════════════╬══════════════╬════════════════════════════════╣
║ DEF-USE CSR        ║ variable     ║ offsets[]+adj[] per function.  ║
║                    ║              ║ O(|uses|) enumeration.         ║
╠════════════════════╬══════════════╬════════════════════════════════╣
║ CDG CSR + WT       ║ variable     ║ Control dependence graph per   ║
║                    ║              ║ function: offsets+adj+wavelet. ║
╠════════════════════╬══════════════╬════════════════════════════════╣
║ IFDS TAINT         ║ sparse       ║ Sorted (ssa_id, taint_src) pairs║
╠════════════════════╬══════════════╬════════════════════════════════╣
║ IFDS NULLABLE      ║ sparse       ║ Sorted ssa_ids that may be null ║
╠════════════════════╬══════════════╬════════════════════════════════╣
║ IFDS TYPE-STATE    ║ sparse       ║ (ssa_id, state_id) pairs        ║
╠════════════════════╬══════════════╬════════════════════════════════╣
║ CHECKSUM           ║ 8 B          ║ CRC-64/ECMA                    ║
╚════════════════════╩══════════════╩════════════════════════════════╝
```

**Size for 15K functions, 2M SSA variables, 900K φ-functions:**

```
Header:            64 B
Function dir:   15K × 20 =   300 KB
SSA var table:   2M × 16 =    32 MB
φ-function table: 900K × (12 + 2.2×8) = 900K × 29.6 ≈ 26.6 MB
Def-use CSR:     (2M+1)×4 + 6M×4 = 32 MB
CDG CSR+WT:      ≈ 5.4 MB  (similar density to CFG)
IFDS taint:      ≈ 3 MB   (sparse, ~2% density)
IFDS nullable:   ≈ 2 MB   (sparse, ~5% density)
IFDS type-state: ≈ 4 MB   (sparse, per-class automata)
──────────────────────────────────────────────────────
Total:           ≈ 105 MB uncompressed
After LZ4 HC:    ≈ 22–28 MB  (SSA tables are highly regular)
```

This is memory-mapped. Only pages touched by live queries enter physical RAM. A typical call graph construction query (Phase 6) accesses the SSA tables for call-expression SSA variables — roughly 5–10% of the total table.

---

### 5.7 Complexity Proofs

**SSA Construction per function:**

| Operation | Complexity | Notes |
|---|---|---|
| Liveness analysis | O(n × m) fixpoint | n=vars, m=edges; typically 3–5 iters |
| Phase A φ-placement | O(\|D\| × m) | \|D\| = distinct vars, amortized |
| Phase B renaming | O(n\_ast\_f) | DFS over dom tree, O(1) per stmt |
| Def-use CSR build | O(total\_uses) | Collected during Phase B |
| CDG construction | O(m × depth\_pdom) | Typically O(m) |

**IFDS analyses:**

Each IFDS analysis runs in O(\|N\| × \|D\|²) time via tabulation:
- Taint: \|D\| = n\_taint\_sources ≈ 10–20. Cost: O(n × 400) = O(n) per function
- Nullable: \|D\| = n\_nullable\_vars ≈ 50–100 per function. Cost: O(n × 10,000). Mitigated by local analysis (function-level rather than full interprocedural for Phase 5; Phase 6 extends with summaries)
- Type-state: \|D\| = n\_states × n\_tracked\_objects ≈ 5 × 20 = 100 per function

**Total Phase 5:** O(n\_ast × \|D\_max\|²) = O(n\_ast × 10,000) for nullable in the worst case, O(n\_ast) for taint. In practice, the IFDS analyses are bounded by the function size with a small constant: 500ms–2s total for a 500K LOC Java project.

---

### 5.8 Phase 5 Invariants for Phase 6

**Invariant 1 (Single Assignment):** `∀ ssa_id v: |{stmt s : s defines v}| = 1`. Every SSA variable has exactly one definition site recorded in `SSARecord.def_stmt`. Verified by asserting the def-count-per-ssa-id equals 1 during `builder.finalize()`.

**Invariant 2 (φ-argument Count):** For every φ-function in block B with k predecessors: the `PhiRecord.arg_count == k`, and `PhiRecord.args` has exactly one entry per CFG predecessor of B. Verified against `CFA.pred_count(B)`.

**Invariant 3 (Def-Use Completeness):** `∀ ssa_id v: Σ_{stmts s} [s uses v] == len(use_adj[def_offsets[v]..def_offsets[v+1]])`. Every use recorded during Phase B renaming has a corresponding entry in the def-use CSR. Verified by comparing use-count collected during renaming against CSR entry count.

**Invariant 4 (SSA→TCA Traceability Seed):** For every SSARecord where `flags.IS_PHI == 0`: `def_stmt` is a valid BP AST pre-order index. The source location of the def-site is recovered via `bpa.token_range(def_stmt)` → token_ids → `BI[token_id]` in the TCA. Phase 7 reads this chain to build the traceability index for SSA variables.

---

Now the visualization: