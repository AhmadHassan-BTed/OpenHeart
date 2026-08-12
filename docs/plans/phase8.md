---

## Phase 8: ROBDD Path Summary Computation

### 8.1 Phase Mandate & Exact Boundaries

Phase 8 converts each function's CFG — a graph of basic blocks and edges — into a compact Boolean formula that encodes exactly the set of all structurally feasible execution paths through that function. This formula is stored as a Reduced Ordered Binary Decision Diagram (ROBDD): a canonical, maximally-compressed directed acyclic graph whose size is exponentially smaller than explicit path enumeration for the vast majority of real-world code.

The PSA artifact produced here is the computational substrate for four capabilities: (1) counting feasible paths in O(|ROBDD|), (2) checking path feasibility in O(|ROBDD|), (3) computing cyclomatic complexity in O(1) from stored metadata, and (4) enabling the Phase 10 CFL-reachability query engine to compose inter-procedural path queries without enumerating paths explicitly.

Phase 8 does NOT generate UML diagrams (Phase 9), serialize the final SCPG (Phase 10), or perform inter-procedural SSA analysis (Phase 5). Its only output is the per-function ROBDD table and path metrics.

**Inputs:** `CFGArtifact (.cfa)`, `SSAArtifact (.ssa)` (for branch condition BDDs), `CallGraphArtifact (.cga)` (for SCC table — handling recursive cycles).

**Output:** `PathSummaryArtifact (.psa)` — per-function ROBDD node arrays, variable ordering tables, and path metrics.

---

### 8.2 Mathematical Foundations

#### 8.2.1 ROBDD: Formal Definition and Reduction Rules

**Definition (BDD Variable Ordering):** A total order < on Boolean variables x₁ < x₂ < ... < xₘ. Every internal node in the ROBDD respects this ordering: if node N has variable xᵢ and child C has variable xⱼ, then i < j. This constraint is what makes the ROBDD canonical.

**Definition (ROBDD Node):** An ROBDD over variables {x₁,...,xₘ} is a DAG where:
- Two terminal nodes exist: `FALSE` (always 0) and `TRUE` (always 1)
- Every internal node N has a variable `var(N)` ∈ {x₁,...,xₘ}, a `lo` edge (the sub-function when `var(N)=0`), and a `hi` edge (when `var(N)=1`)
- The ordering constraint is satisfied at all edges

**Two Reduction Rules (applied exhaustively to obtain the ROBDD):**

**Rule 1 (Elimination):** If `lo(N) == hi(N)`, delete node N and redirect all edges pointing to N directly to `lo(N)`. A node where both branches lead to the same result is redundant — the variable doesn't affect the outcome.

**Rule 2 (Sharing / Merging):** If two nodes N and M have `var(N) == var(M)`, `lo(N) == lo(M)`, and `hi(N) == hi(M)`, merge them into a single node. Two nodes representing the same Boolean function must be physically identical.

**Canonical form theorem:** After exhaustive application of both reduction rules, the resulting ROBDD is unique for any given Boolean function and variable ordering. This means ROBDDs are a canonical form: two Boolean functions are identical if and only if their ROBDDs (with the same ordering) are identical DAGs. This makes equivalence checking O(1). ∎

**The Unique Table:** Phase 8's BDD library maintains a hash table mapping `(var_id, lo_node_id, hi_node_id) → node_id`. Before creating any new node, the library checks whether an identical node already exists. This is what mechanically enforces Rule 2 during ROBDD construction.

```rust
fn make_node(
    var: u16, lo: u32, hi: u32,
    unique_table: &mut HashMap<(u16, u32, u32), u32>,
    nodes: &mut Vec<ROBDDNode>,
) -> u32 {
    if lo == hi { return lo; }                // Rule 1: elimination
    let key = (var, lo, hi);
    if let Some(&existing) = unique_table.get(&key) { return existing; } // Rule 2: sharing
    let new_id = nodes.len() as u32;
    nodes.push(ROBDDNode { var, lo, hi, sat_count: 0 });
    unique_table.insert(key, new_id);
    new_id
}
```

#### 8.2.2 Shannon Expansion and the ITE Construction

Every Boolean function f over {x₁,...,xₘ} can be decomposed by the **Shannon expansion**:

f(x₁,...,xₘ) = (¬xᵢ ∧ f|_{xᵢ=0}) ∨ (xᵢ ∧ f|_{xᵢ=1}) = ITE(xᵢ, f|_{xᵢ=1}, f|_{xᵢ=0})

where ITE(c, t, e) = "if c then t else e" and f|_{xᵢ=v} is the cofactor of f with xᵢ fixed to v.

The ROBDD node for a single variable xᵢ is just `make_node(i, FALSE, TRUE)` — its lo branch is FALSE (xᵢ=0 means the proposition xᵢ is false) and its hi branch is TRUE.

The `apply(op, f, g)` algorithm computes the ROBDD of `f op g` from ROBDDs of f and g:

```rust
fn apply(
    op: BoolOp, f: u32, g: u32,
    nodes: &[ROBDDNode],
    unique_table: &mut HashMap<(u16,u32,u32), u32>,
    all_nodes: &mut Vec<ROBDDNode>,
    cache: &mut HashMap<(u32,u32), u32>,
) -> u32 {
    // Terminal cases
    let result = match (f, g, op) {
        (FALSE, _, BoolOp::And) | (_, FALSE, BoolOp::And) => return FALSE,
        (TRUE, x,  BoolOp::And) | (x, TRUE,  BoolOp::And) => return x,
        (FALSE, x, BoolOp::Or)  | (x, FALSE, BoolOp::Or)  => return x,
        (TRUE, _, BoolOp::Or)   | (_, TRUE,  BoolOp::Or)  => return TRUE,
        _ if f == g => match op {
            BoolOp::And | BoolOp::Or => return f,
            BoolOp::Xor => return FALSE,
        }
        _ => {}
    };

    let key = (f.min(g), f.max(g));   // canonicalize commutative ops
    if let Some(&cached) = cache.get(&key) { return cached; }

    let fn_ = &nodes[f as usize];
    let gn_ = &nodes[g as usize];

    let (var, f_lo, f_hi, g_lo, g_hi) = if fn_.var == gn_.var {
        (fn_.var, fn_.lo, fn_.hi, gn_.lo, gn_.hi)
    } else if fn_.var < gn_.var {
        (fn_.var, fn_.lo, fn_.hi, g, g)        // g doesn't depend on fn_.var
    } else {
        (gn_.var, f, f, gn_.lo, gn_.hi)        // f doesn't depend on gn_.var
    };

    let lo = apply(op, f_lo, g_lo, nodes, unique_table, all_nodes, cache);
    let hi = apply(op, f_hi, g_hi, nodes, unique_table, all_nodes, cache);
    let res = make_node(var, lo, hi, unique_table, all_nodes);
    cache.insert(key, res);
    res
}
```

Time: O(|f| × |g|) — each pair of nodes from f and g is visited at most once due to memoization.

#### 8.2.3 The Structural Path Constraint Formula

For function f with CFG (B, E), assign Boolean variable xₑ ↔ CFG edge e ∈ E. An assignment A: E → {0,1} represents a path iff A encodes a structurally consistent traversal from entry to exit.

The structural path function f\_paths: {0,1}^{|E|} → {0,1} is defined as the conjunction of **flow conservation constraints** Φ\_b for every non-synthetic block b:

**For a block with a single successor** (b has one outgoing edge e\_out and any predecessors p₁,...,pₖ):

Φ\_b = (x\_{p₁→b} ∨ ... ∨ x\_{pₖ→b}) → x\_{e\_out}

"If we enter b, we exit via its only outgoing edge."

**For a block with two successors** (binary branch — the common case for conditionals):

Φ\_b = (x\_{p₁→b} ∨ ... ∨ x\_{pₖ→b}) → (x\_{e\_true} ⊕ x\_{e\_false})

"If we enter b, we take exactly one of the two outgoing edges (exclusive-or)."

**For a switch block** with n successors: generalize to "exactly one outgoing edge is taken" using a pairwise exclusion formula.

**Entry constraint:** The edge from the synthetic ENTRY block is always taken: x\_{entry→first\_real} = 1 (implemented as restricting the ROBDD to xₑ = 1 for this edge, effectively dropping the variable and halving the search space).

**Exit constraint:** At least one edge into the EXIT block is taken (there must be a return or throw).

f\_paths = (∧\_b Φ\_b) ∧ entry\_constraint

Each Φ\_b is built as an ROBDD clause and conjuncted into the accumulating f\_paths using `apply(AND, ...)`.

#### 8.2.4 Variable Ordering — FORCE Algorithm

ROBDD size depends critically on variable ordering. A bad ordering can produce exponentially more nodes than a good one. For the CFG path function, the FORCE algorithm (Aloul et al. 2003) exploits the constraint hypergraph structure.

**Definition (Constraint Hypergraph):** H = (V, H\_E) where V = {x₁,...,xₘ} are the BDD variables (CFG edges) and each hyperedge h ∈ H\_E is a constraint's variable set (e.g. {x\_{e\_true}, x\_{e\_false}} for a binary branch, {x\_{pred\_edge}, x\_{succ\_edge}} for a flow constraint).

**FORCE assigns each variable a continuous position and iterates:**

```rust
fn force_ordering(
    n_vars: usize,
    hyperedges: &[Vec<usize>],  // each entry = set of variable indices in one constraint
    mut pos: Vec<f64>,          // initial: RPO position of each variable's CFG edge
) -> Vec<usize> {               // returns: sorted variable ordering
    let n_iters = (10 * n_vars).min(200);

    for _ in 0..n_iters {
        let mut gravity = vec![0.0f64; n_vars];
        let mut weight  = vec![0usize; n_vars];

        for hedge in hyperedges {
            let center: f64 = hedge.iter().map(|&v| pos[v]).sum::<f64>() / hedge.len() as f64;
            for &v in hedge {
                gravity[v] += center;
                weight[v]  += 1;
            }
        }

        for v in 0..n_vars {
            if weight[v] > 0 {
                pos[v] = gravity[v] / weight[v] as f64;
            }
        }

        // Normalize positions to integers 0..n_vars-1
        let mut order: Vec<usize> = (0..n_vars).collect();
        order.sort_unstable_by(|&a, &b| pos[a].partial_cmp(&pos[b]).unwrap().then(a.cmp(&b)));
        for (new_p, &v) in order.iter().enumerate() { pos[v] = new_p as f64; }
    }

    let mut final_order: Vec<usize> = (0..n_vars).collect();
    final_order.sort_unstable_by(|&a, &b| pos[a].partial_cmp(&pos[b]).unwrap());
    final_order
}
```

FORCE converges to a **low-bandwidth ordering**: variables that appear together in constraints (correlated variables — e.g. the two edges of a binary branch) are placed adjacent in the ordering. Low bandwidth → small ROBDD.

**Sifting refinement (Rudell 1993):** After FORCE, apply per-variable local optimization. For each variable xᵢ (processed in descending order of its node count contribution):
1. Sift xᵢ upward through all variables above it, measuring |ROBDD| after each adjacent swap
2. Sift xᵢ downward to the bottom
3. Move xᵢ to the position that gave minimum |ROBDD| during the sift

Each adjacent swap of variables at positions k and k+1 is performed in O(|nodes at levels k and k+1|) time using local node restructuring. Total sifting cost: O(m × |ROBDD|) amortized.

**Combined strategy:** FORCE (O(m × n\_constraints × 100 iterations)) then sifting (O(m × |ROBDD|)) gives near-optimal ordering in O(m × |ROBDD|) total.

#### 8.2.5 #SAT Computation with Variable Gaps

The number of satisfying assignments of f\_paths = the number of feasible execution paths through the CFG.

When computing #SAT, variables that don't appear in a sub-ROBDD are implicitly free — they can take any value. The gap between a node's variable index and its child's variable index determines how many free variables exist in that sub-problem:

```rust
fn sat_count(
    node: u32, depth: u16,   // depth = how many variables remain from this node downward
    nodes: &[ROBDDNode], n_vars: u16,
    memo: &mut HashMap<u32, u64>,
) -> u64 {
    if node == FALSE { return 0; }
    if node == TRUE  { return 1u64 << depth; }  // all remaining vars are free
    if let Some(&cached) = memo.get(&node) { return cached; }

    let n = &nodes[node as usize];

    // Variables between n.var and n.lo's top variable are free in lo branch
    let lo_top   = if n.lo == FALSE || n.lo == TRUE { n_vars } else { nodes[n.lo as usize].var };
    let lo_gap   = (lo_top - n.var - 1) as u32;
    let lo_count = (1u64 << lo_gap) * sat_count(n.lo, depth - 1 - lo_gap as u16, nodes, n_vars, memo);

    let hi_top   = if n.hi == FALSE || n.hi == TRUE { n_vars } else { nodes[n.hi as usize].var };
    let hi_gap   = (hi_top - n.var - 1) as u32;
    let hi_count = (1u64 << hi_gap) * sat_count(n.hi, depth - 1 - hi_gap as u16, nodes, n_vars, memo);

    let total = lo_count + hi_count;
    memo.insert(node, total);
    total
}
```

**Cyclomatic complexity from the ROBDD:** V(G) = |E| − |B| + 2 is stored directly as a u16 in the function's path metrics record. It equals the number of linearly independent paths — geometrically, the number of binary decision nodes whose both branches are satisfiable. It is NOT derived from the ROBDD at query time; it is computed once from the CFA edge and block counts and stored permanently.

#### 8.2.6 Feasibility Filtering

The structural ROBDD encodes syntactically valid paths. Semantically infeasible paths (e.g. where a condition and its negation are both required) are filtered by intersecting with **branch condition BDDs**:

For each conditional branch edge (b, s, CFG\_TRUE) with condition `cond(b)`:
1. Represent `cond(b)` as a propositional formula over the SSA variable values at block b
2. Build a BDD encoding this condition
3. Conjunct into f\_paths: `f_paths = apply(AND, f_paths, BDD(cond(b) → x_{b→s_true}))`

For simple conditions (linear arithmetic, reference comparisons), the BDD is small. For complex conditions (heap-dependent, non-linear), we skip the filtering for that branch (sound over-approximation: some infeasible paths remain in the ROBDD, but no feasible paths are removed).

**Handling recursive SCCs:** For functions in the SCC table (Phase 6) with `scc_class ≥ 1`:
- Apply bounded unwinding: unroll the call graph edge k=3 times by introducing k copies of the callee's ROBDD as a sub-graph
- Mark the function's PSA record with `unwind_depth = 3`
- Phase 10's query engine uses this bound when composing inter-procedural path queries

---

### 8.3 Module Architecture

```
phase8/
├── mod.rs                          # Phase8Stage::run(cfa, ssa, cga) → PathSummaryArtifact
│                                   # Processes functions in SCC topological order (bottom-up)
│                                   # so callee summaries are available before callers
├── bdd/
│   ├── mod.rs                      # BDDLibrary: shared node table + unique hash table
│   ├── node.rs                     # ROBDDNode (12 bytes): var, lo, hi, sat_count
│   │                               # Terminal constants: FALSE=0, TRUE=1
│   ├── apply.rs                    # apply(op, f, g): AND/OR/XOR via memoized recursion
│   ├── restrict.rs                 # restrict(f, var, val): cofactor — O(|f|)
│   ├── sat_count.rs                # sat_count(f, depth, memo): #SAT via memoized DFS
│   └── unique_table.rs             # HashMap<(var,lo,hi), node_id> enforcing Rule 2
│
├── ordering/
│   ├── mod.rs                      # VariableOrdering: manages the xᵢ ↔ edge_id mapping
│   ├── rpo.rs                      # initial_rpo_ordering(): reverse post-order of edges
│   ├── force.rs                    # FORCEAlgorithm: hypergraph gravity-center iteration
│   └── sifting.rs                  # SiftingOptimizer: Rudell's per-variable local search
│
├── construction/
│   ├── mod.rs                      # FunctionROBDDBuilder: builds ROBDD for one function
│   ├── constraints.rs              # build_phi_b(): structural Φ_b from CFG block topology
│   ├── feasibility.rs              # FeasibilityFilter: intersects structural BDD with
│   │                               # branch condition BDDs derived from SSA Phase 5
│   └── recursive.rs                # RecursiveHandler: bounded unwinding for SCC functions
│
├── metrics.rs                      # PathMetrics: V(G), sat_count, max_path_len per function
├── builder.rs                      # PathSummaryArtifactBuilder: assembles all structures
└── serializer.rs                   # PathSummaryArtifact binary I/O (.psa format)
```

---

### 8.4 Data Structure Specifications

**ROBDDNode (12 bytes, cache-line efficient — 5.3 nodes per 64-byte cache line):**

```
Offset  Size  Field
 0       2    var         : u16   variable index in ordering; 0xFFFF = terminal node
 2       2    _flags      : u16   IS_FALSE_TERMINAL:1, IS_TRUE_TERMINAL:1, reserved:14
 4       4    lo          : u32   node_id of lo (var=0) child; 0=FALSE, 1=TRUE
 8       4    hi          : u32   node_id of hi (var=1) child; 0=FALSE, 1=TRUE
Total: 12 bytes
```

`node_id=0` is always the `FALSE` terminal. `node_id=1` is always the `TRUE` terminal. Both are pre-populated at BDD library initialization. All function-specific nodes start at `node_id=2`.

**FunctionPSAHeader (32 bytes, stored in the PSA function directory):**

```
Offset  Size  Field
 0       4    sym_id           : u32   function symbol_id from STA
 4       8    node_array_offset: u64   byte offset into PSA Section 3 for this function's nodes
12       4    n_vars           : u32   number of Boolean variables (= CFA edge count)
16       4    n_nodes          : u32   number of ROBDD nodes (including shared terminals)
20       4    root_node        : u32   root node index within this function's node array
24       8    sat_count        : u64   #SAT(f_paths) = total feasible paths
32 bytes, but let me recalculate:
 0       4    sym_id
 4       4    n_vars
 8       4    n_nodes
12       4    root_node
16       8    sat_count
24       2    cyclomatic      : u16   V(G) = |E| - |B| + 2
26       2    max_path_len    : u16   length of longest path (number of blocks visited)
28       2    unwind_depth    : u16   0 = non-recursive, >0 = unwinding bound for recursive
30       2    _reserved       : u16
Total: 32 bytes
```

---

### 8.5 Algorithm Specifications

#### 8.5.1 Top-Level Orchestration

```rust
impl Phase8Stage {
    pub fn run(cfa: &CFGArtifact, ssa: &SSAArtifact, cga: &CallGraphArtifact, out: &Path)
        -> PathSummaryArtifact
    {
        let mut artifact = PathSummaryArtifactBuilder::new();
        // Process functions in SCC condensation DAG topological order
        // (leaves/callees first so their summaries exist before callers need them)
        let topo_order = cga.scc_topological_order();

        for scc_id in topo_order {
            let scc = cga.scc(scc_id);
            let is_recursive = scc.scc_class >= 1;

            for &sym_id in &scc.members {
                let cfg = cfa.function(sym_id);
                if cfg.is_none() { continue; }  // abstract method — no body
                let cfg = cfg.unwrap();

                // Build ROBDD for this function
                let robdd = FunctionROBDDBuilder::build(sym_id, cfg, ssa, is_recursive);
                artifact.add_function(robdd);
            }
        }

        PathSummarySerializer::write(&artifact, out);
        artifact
    }
}
```

#### 8.5.2 Per-Function ROBDD Construction

```rust
impl FunctionROBDDBuilder {
    pub fn build(
        sym_id: u32, cfg: &FunctionCFGData,
        ssa: &SSAArtifact, is_recursive: bool,
    ) -> FunctionROBDD {
        let mut bdd = BDDLibrary::new();

        // Step 1: Compute initial variable ordering (RPO of CFG edges)
        let rpo_order  = rpo_edge_ordering(cfg);

        // Step 2: Refine with FORCE algorithm
        let constraints = build_constraint_hyperedges(cfg);
        let init_pos    = rpo_positions(&rpo_order);
        let force_order = force_ordering(rpo_order.len(), &constraints, init_pos);

        // Step 3: Build the ordering structure (edge_id → var_index bijection)
        let ordering = VariableOrdering::from_order(&force_order, cfg);

        // Step 4: Construct structural constraints Φ_b for all blocks
        let mut f_paths = bdd.TRUE;
        for block_id in 0..cfg.block_count {
            let phi_b = build_phi_b(block_id, cfg, &ordering, &mut bdd);
            f_paths   = bdd.apply(BoolOp::And, f_paths, phi_b);
        }

        // Step 5: Apply entry constraint (entry edge always taken)
        let entry_edge_var = ordering.edge_var(cfg.entry_block, cfg.successors(cfg.entry_block)[0]);
        f_paths = bdd.restrict(f_paths, entry_edge_var, 1);  // fix entry edge = 1

        // Step 6: Feasibility filtering from SSA branch conditions
        f_paths = FeasibilityFilter::apply(f_paths, cfg, ssa, &ordering, &mut bdd);

        // Step 7: Sifting optimization
        SiftingOptimizer::optimize(&mut bdd, &mut f_paths, &mut ordering);

        // Step 8: Compute metrics
        let n_vars   = ordering.n_vars();
        let sat      = bdd.sat_count(f_paths, n_vars);
        let cyclo    = cfg.edge_count - cfg.block_count + 2;
        let max_path = compute_max_path_length(cfg);

        FunctionROBDD { sym_id, ordering, bdd, root: f_paths, sat_count: sat,
                        cyclomatic: cyclo as u16, max_path_len: max_path, unwind_depth: 0 }
    }
}
```

#### 8.5.3 Structural Constraint for a Binary Branch Block

```rust
fn build_phi_b(
    block_id: u32, cfg: &FunctionCFGData,
    ordering: &VariableOrdering, bdd: &mut BDDLibrary,
) -> u32 {
    let succs = cfg.successors(block_id);
    let preds = cfg.predecessors(block_id);

    // Construct "is_reachable": OR of all incoming edge variables
    let is_reachable = preds.iter().fold(bdd.FALSE, |acc, &pred| {
        let x_in = bdd.var(ordering.edge_var(pred, block_id));
        bdd.apply(BoolOp::Or, acc, x_in)
    });

    match succs.len() {
        0 => bdd.TRUE,  // EXIT block: no outgoing constraint

        1 => {
            // Unconditional: if reachable, the single outgoing edge is taken
            let x_out = bdd.var(ordering.edge_var(block_id, succs[0]));
            bdd.implies(is_reachable, x_out)   // is_reachable → x_out
        }

        2 => {
            // Binary branch: if reachable, exactly one of {x_true, x_false} is 1
            let x_true  = bdd.var(ordering.edge_var(block_id, succs[0]));  // CFG_TRUE edge
            let x_false = bdd.var(ordering.edge_var(block_id, succs[1]));  // CFG_FALSE edge
            let xor     = bdd.apply(BoolOp::Xor, x_true, x_false);
            bdd.implies(is_reachable, xor)     // is_reachable → (x_true ⊕ x_false)
        }

        _ => {
            // Switch: exactly one successor — use pairwise AND-NOT constraints
            build_switch_constraint(block_id, &succs, is_reachable, ordering, bdd)
        }
    }
}

// BDD for the implication (a → b) ≡ (¬a ∨ b)
fn implies(bdd: &mut BDDLibrary, a: u32, b: u32) -> u32 {
    let not_a = bdd.apply_not(a);
    bdd.apply(BoolOp::Or, not_a, b)
}
```

---

### 8.6 Output Schema: `PathSummaryArtifact` Binary Format (`.psa`)

```
╔══════════════════════════════════════════════════════════════════╗
║  PSA FILE FORMAT v1.0  (all integers: little-endian)            ║
╠═════════════════════╦══════════════╦══════════════════════════════╣
║ Section             ║ Size         ║ Description                  ║
╠═════════════════════╬══════════════╬══════════════════════════════╣
║ HEADER              ║ 64 B         ║ Magic, counts, cfa_hash      ║
╠═════════════════════╬══════════════╬══════════════════════════════╣
║ FUNCTION DIR.       ║ n_f × 32 B   ║ FunctionPSAHeader[] sorted   ║
║                     ║              ║ by sym_id. Binary search.    ║
╠═════════════════════╬══════════════╬══════════════════════════════╣
║ VARIABLE ORDERING   ║ variable     ║ Per-function: edge_id[] of   ║
║ TABLES              ║              ║ length n_vars. Maps          ║
║                     ║              ║ var_idx → CFA edge_id.       ║
╠═════════════════════╬══════════════╬══════════════════════════════╣
║ ROBDD NODE ARRAYS   ║ variable     ║ Per-function: ROBDDNode[]    ║
║                     ║              ║ (12 bytes each). Lazy-loaded ║
║                     ║              ║ on first path query.         ║
╠═════════════════════╬══════════════╬══════════════════════════════╣
║ PATH METRICS TABLE  ║ n_f × 16 B   ║ (cyclomatic:u16,             ║
║                     ║              ║  max_path:u16, sat_lo:u32,   ║
║                     ║              ║  sat_hi:u32, mean:f32)       ║
║                     ║              ║ Hot path — always mmap'd.    ║
╠═════════════════════╬══════════════╬══════════════════════════════╣
║ CHECKSUM            ║ 8 B          ║ CRC-64/ECMA                  ║
╚═════════════════════╩══════════════╩══════════════════════════════╝
```

**Size estimate (15K functions, avg 500 ROBDD nodes, avg 35 variables):**

```
Header:                  64 B
Function directory:   15K × 32  =   480 KB
Variable ordering:    15K × 35×4 =  2.1 MB
ROBDD node arrays:    15K × 500×12 = 90 MB   (dominant section, lazy-loaded)
Path metrics:         15K × 16  =   240 KB
─────────────────────────────────────────────
Total:                           ≈  92.8 MB uncompressed
After LZ4 HC:                    ≈  24 MB   (ROBDD node arrays have ~3.5× compression)
Memory resident (hot path):      ≈  2.8 MB  (header + directory + metrics only)
On-demand loaded per query:      ≈  6 KB    (500 nodes × 12 bytes per function)
```

The ROBDD node arrays are the largest section and are accessed lazily: only the ROBDDs for functions actually queried in a session are paged into physical RAM. For a typical UML generation session that analyzes 20–30 key methods, the working set is under 1 MB.

---

### 8.7 Complexity Proofs

| Operation | Per-function | Total (15K functions) |
|---|---|---|
| RPO edge ordering | O(V + E) | O(n\_ast) |
| FORCE algorithm | O(m × \|constraints\| × 100) | O(m² × 100) |
| Structural constraints (apply AND) | O(\|Φ\_b\|² per block) | O(n\_ast × max\_ROBDD²) |
| Sifting | O(m × \|ROBDD\|) | O(m\_avg × n\_nodes\_avg × 15K) |
| #SAT computation | O(\|ROBDD\|) | O(total\_nodes) |
| Serialization | O(n\_nodes) | O(total\_nodes) |

**Dominant term:** O(m × |ROBDD|) for sifting, where m = edge count and |ROBDD| = node count. For the average function (35 edges, 500 nodes): 35 × 500 = 17,500 operations per function × 15K functions = 262M operations ≈ 1.3 seconds at 5ns/op. This is the most expensive phase after Phase 5's IFDS analyses.

**Space:** O(Σ\_f |ROBDD\_f|) = O(total\_ROBDD\_nodes). For the estimate above: 15K × 500 = 7.5M nodes × 12 bytes = 90 MB on disk, ~6 KB per active function in RAM.

---

### 8.8 Phase 8 Invariants for Phase 9

**Invariant 1 (Path Coverage):** `∀f: sat_count[f] ≥ 1`. Every function with a body has at least one feasible path from entry to exit. A `sat_count == 0` after Phase 8 signals a provably unreachable function body — reported as a diagnostic, not a failure.

**Invariant 2 (Cyclomatic Consistency):** `∀f: cyclomatic[f] == |E_f| - |B_f| + 2`. Cross-verified against CFA metadata at Phase 8 finalization.

**Invariant 3 (ROBDD Canonicity):** The ROBDD for every function satisfies both reduction rules: no node has `lo == hi` (elimination), and no two distinct nodes have identical `(var, lo, hi)` triples (sharing). Verified in O(|ROBDD|) during `BDDLibrary::finalize()` by a single scan of the node array and unique table.

**Invariant 4 (PSA→CFA Hash Chain):** `psa_header.cfa_hash == crc64(cfa_file_bytes)`. Phase 9 validates this before accessing path metrics.

---

Now the visualization: