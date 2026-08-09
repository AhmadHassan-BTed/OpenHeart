---

## Phase 6: Inter-procedural Call Graph & Points-To Analysis

### 6.1 Phase Mandate & Exact Boundaries

Phase 6 assembles the inter-procedural skeleton of the SCPG: the call graph. Every call expression identified in the BP AST — direct, virtual, interface, constructor, or dynamic — is resolved to a set of possible callee symbols and encoded as a typed edge. The call graph is the structural substrate for Phase 8's CFL-reachability path queries, Phase 9's sequence and use-case diagram generation, and the final SCPG query engine's inter-procedural slicing capability.

Phase 6 does NOT build path BDDs (Phase 8), extract UML metadata (Phase 9), or serialize the final SCPG binary (Phase 10). Its single output is the call graph: topology, call site annotations, points-to summaries, and recursive cycle table.

**Inputs:** `BPASTArtifact (.bpa)`, `SymbolTableArtifact (.sta)`, `CFGArtifact (.cfa)`, `SSAArtifact (.ssa)`.

**Output:** `CallGraphArtifact (.cga)` — call site table, callee/caller CSR adjacency, call edge type wavelet tree, points-to summaries, and strongly connected components.

---

### 6.2 Mathematical Foundations

#### 6.2.1 Call Graph Formal Definition

**Definition:** The call graph G\_CG = (V\_CG, E\_CG) where:

V\_CG ⊆ V\_sym is the set of callable symbols — exactly the STA symbols with `kind ∈ {SK_METHOD, SK_CONSTRUCTOR, SK_STATIC_INIT, SK_LAMBDA}`. Every callable symbol appears in V\_CG, including those with no call edges (leaf methods and unreachable methods).

E\_CG ⊆ V\_CG × V\_CG × Σ\_CG × CallSiteId is the typed, annotated edge set, where each edge carries a reference to the call site that generates it.

**The call edge type alphabet:**

```
Σ_CG (u8):
0x00  CG_DIRECT       static method call — resolved to exactly one target
0x01  CG_SPECIAL      private / constructor / super call — exactly one target
0x02  CG_VIRTUAL      instance method call — resolved by CHA/1-CFA to target set
0x03  CG_INTERFACE    interface method call — resolved by CHA/1-CFA to target set
0x04  CG_CONSTRUCTOR  new T() or super() — resolved to exactly one constructor
0x05  CG_DYNAMIC      lambda/method-reference — resolved via BootstrapMethod analysis
0x06  CG_REFLECTION   reflection (java.lang.reflect.Method.invoke) — conservative
```

CG\_DIRECT and CG\_SPECIAL edges are monomorphic — each call site has exactly one target. CG\_VIRTUAL, CG\_INTERFACE, and CG\_DYNAMIC edges are polymorphic — one call site may generate multiple edges (one per possible callee), each carrying the same `CallSiteId`.

#### 6.2.2 Java Call Type Taxonomy

Java's bytecode instruction set provides four call opcodes, each with distinct dispatch semantics. We map these to our Σ\_CG types:

| JVM opcode | Java source pattern | Dispatch | Σ\_CG type |
|---|---|---|---|
| `invokestatic` | `Foo.staticMethod()` | None — target is fixed | CG\_DIRECT |
| `invokespecial` | `new Foo()`, `super.m()`, `this.private_m()` | Fixed — class known at compile time | CG\_SPECIAL |
| `invokevirtual` | `obj.method()` where obj is a class type | Virtual — receiver type at runtime | CG\_VIRTUAL |
| `invokeinterface` | `iface.method()` where iface is an interface | Interface dispatch | CG\_INTERFACE |
| `invokedynamic` | lambda expressions, method references | Bootstrap method at runtime | CG\_DYNAMIC |

**Resolution precision principle:** CG\_DIRECT and CG\_SPECIAL are always resolved to a single target via the STA symbol table lookup — no analysis required. CG\_VIRTUAL and CG\_INTERFACE require CHA and optionally 1-CFA refinement. CG\_DYNAMIC requires bootstrap method analysis from the bytecode or heuristic from the lambda expression type.

#### 6.2.3 Class Hierarchy Analysis (CHA)

CHA is the baseline resolution algorithm for virtual and interface dispatch. It is sound (never misses a real callee) but not precise (may include impossible callees).

```
cha_targets(
    method_name: &str,
    descriptor:  &MethodDescriptor,
    receiver_declared_type: SymbolId,
    sta: &SymbolTableArtifact,
) → Set<SymbolId>:

    result = {}

    // Walk the type hierarchy downward from receiver_declared_type
    worklist = {receiver_declared_type}
    visited  = {}

    while worklist ≠ ∅:
        t = worklist.pop_any()
        if t ∈ visited: continue
        visited.add(t)

        // Check if type t has a concrete implementation of (method_name, descriptor)
        if let Some(m) = sta.lookup_method(t, method_name, descriptor):
            if !sta.symbol(m).modifiers.has(ABSTRACT):
                result.insert(m)

        // Add all subclasses and implementing classes of t
        for each s in sta.subclasses(t):   // E^TH TH_EXTENDS edges, reversed
            worklist.add(s)
        for each s in sta.implementors(t): // E^TH TH_IMPLEMENTS edges, reversed
            worklist.add(s)

    result
```

**Soundness proof:** Every class that can appear as the runtime type of the receiver must be a subclass or implementor of `receiver_declared_type` (guaranteed by Java's type system). CHA includes all such classes. Therefore no real callee is missed. ∎

**Imprecision:** CHA includes methods on classes that are never instantiated in the program, or never assigned to the receiver variable. This is where 1-CFA improves precision.

#### 6.2.4 One-CFA: Allocation-Site Sensitivity

1-CFA refines CHA using the SSA information from Phase 5. The key insight: if the receiver `o^i` is an SSA variable, its definition site tells us its allocation type.

```
one_cfa_targets(
    call_site:  &CallSite,
    ssa:        &SSAArtifact,
    sta:        &SymbolTableArtifact,
) → Set<SymbolId>:

    receiver_ssa = call_site.receiver_ssa
    if receiver_ssa == u32::MAX:
        return cha_targets(...)  // static call — should not reach here

    record = ssa.record(receiver_ssa)

    if record.flags.IS_PHI:
        // φ-function: receiver may be any of the incoming versions
        types = {}
        for each arg_ssa in ssa.phi_args(receiver_ssa):
            types.union(one_cfa_types(arg_ssa, ssa, sta))
        // Run CHA restricted to types in the union
        return restricted_cha(call_site, types, sta)

    if record.flags.IS_CONST and record.def_stmt == u32::MAX:
        // null literal — skip (will generate NullPointerException, not a valid call)
        return {}

    // Check if the def-stmt is a new expression (alloc site)
    def_node = record.def_stmt
    if bpa.node_type(def_node) == NN_NEW_EXPR:
        // Exact allocation type — single target
        alloc_type = extract_type_from_new_expr(def_node, bpa, sta)
        if let Some(m) = sta.lookup_method(alloc_type, call_site.method_name, call_site.desc):
            return {m}

    // Fallback: use CHA on the declared static type
    declared_type = sta.type_of_ssa(receiver_ssa)
    return cha_targets(call_site.method_name, call_site.desc, declared_type, sta)
```

**Precision gain:** For `o = new ConcreteList(); o.add(x)`, CHA returns all `add()` implementations across the entire `Collection` hierarchy (potentially dozens). 1-CFA sees that `o^i` is defined by `new ConcreteList()` and returns exactly `{ConcreteList.add}`. This reduces average virtual call fan-out from ~8 targets (CHA) to ~1.3 targets (1-CFA) on typical Java codebases (empirical from Lhoták & Hendren 2003 benchmarks).

#### 6.2.5 Anderson's Inclusion-Based Points-To Analysis

For a more globally precise (but more expensive) alternative to per-call-site 1-CFA, Anderson's algorithm (1994) computes a global points-to relation `pts: SSAVar → 2^{AllocSite}` over all pointer-typed SSA variables.

**Constraint generation:** For each SSA statement, emit inclusion constraints:

```
Statement form             Constraint
─────────────────────────────────────────────────────────────
v^i = new T()           → {alloc_T} ⊆ pts(v^i)
v^i = v^j (copy/phi)    → pts(v^j) ⊆ pts(v^i)
v^i = v^j.f             → ∀ o ∈ pts(v^j): pts(field(o,f)) ⊆ pts(v^i)
v^i.f = v^j             → ∀ o ∈ pts(v^i): pts(v^j) ⊆ pts(field(o,f))
v^i = v^j.m(args...)    → pts(return(m)) ⊆ pts(v^i)
                           (connect return value to all possible callees)
```

**Worklist fixpoint solver:**

```rust
fn anderson_points_to(stmts: &[SSAStmt]) -> HashMap<SsaId, BitSet<AllocId>> {
    let mut pts:  HashMap<SsaId, BitSet<AllocId>> = HashMap::new();
    let mut graph: HashMap<SsaId, Vec<SsaId>>     = HashMap::new(); // copy edges
    let mut worklist: VecDeque<SsaId>              = VecDeque::new();

    // Phase 1: Process allocation sites → seed pts sets
    for stmt in stmts {
        if let New(v, alloc_T) = stmt {
            pts.entry(*v).or_default().insert(alloc_T.id);
            worklist.push_back(*v);
        }
    }

    // Phase 2: Process copy constraints → build propagation graph
    for stmt in stmts {
        if let Copy(dst, src) = stmt {
            graph.entry(*src).or_default().push(*dst);
        }
    }

    // Phase 3: Fixpoint propagation
    while let Some(v) = worklist.pop_front() {
        for &w in graph.get(&v).unwrap_or(&vec![]) {
            let added = pts[&v].difference(&pts.get(&w).cloned().unwrap_or_default());
            if !added.is_empty() {
                pts.entry(w).or_default().union_with(&added);
                worklist.push_back(w);
            }
        }
    }
    pts
}
```

Time: O(n\_vars² × n\_alloc\_sites) in the worst case. For practical Java: O(n^1.5) empirically with BDD-based `pts` representation.

**Phase 6 uses Anderson's output to refine virtual dispatch:** After computing `pts`, a virtual call `o^i.m()` has target set = {C.m : alloc\_C ∈ pts(o^i)}.

#### 6.2.6 Tarjan's SCC Algorithm for Recursive Cycle Detection

The call graph may contain cycles — direct recursion (`f` calls `f`) or mutual recursion (`f` calls `g`, `g` calls `f`). Cycles are significant for:
- Phase 8 ROBDD path summaries (loops in the call graph require bounded unwinding)
- Phase 9 sequence diagram generation (recursive calls produce self-referential arrows)
- UML interaction diagrams (recursive patterns are a distinct UML construct)

**Tarjan's algorithm** (1972) finds all strongly connected components (SCCs) in O(V + E):

```rust
fn tarjan_scc(n: usize, adj: &[Vec<u32>]) -> Vec<Vec<u32>> {
    let mut index_counter = 0u32;
    let mut stack  = Vec::new();
    let mut on_stack  = vec![false; n];
    let mut index    = vec![u32::MAX; n];  // u32::MAX = unvisited
    let mut lowlink  = vec![0u32; n];
    let mut sccs     = Vec::new();

    fn strongconnect(
        v: u32, adj: &[Vec<u32>], counter: &mut u32,
        stack: &mut Vec<u32>, on_stack: &mut Vec<bool>,
        index: &mut Vec<u32>, lowlink: &mut Vec<u32>,
        sccs: &mut Vec<Vec<u32>>,
    ) {
        index[v as usize]   = *counter;
        lowlink[v as usize] = *counter;
        *counter += 1;
        stack.push(v);
        on_stack[v as usize] = true;

        for &w in &adj[v as usize] {
            if index[w as usize] == u32::MAX {
                // w not yet visited — recurse
                strongconnect(w, adj, counter, stack, on_stack, index, lowlink, sccs);
                lowlink[v as usize] = lowlink[v as usize].min(lowlink[w as usize]);
            } else if on_stack[w as usize] {
                // w is on stack → back edge → w is ancestor of v in DFS tree
                lowlink[v as usize] = lowlink[v as usize].min(index[w as usize]);
            }
        }

        // If v is the root of an SCC (lowlink[v] == index[v]): pop the SCC
        if lowlink[v as usize] == index[v as usize] {
            let mut scc = Vec::new();
            loop {
                let w = stack.pop().unwrap();
                on_stack[w as usize] = false;
                scc.push(w);
                if w == v { break; }
            }
            sccs.push(scc);
        }
    }

    for v in 0..n as u32 {
        if index[v as usize] == u32::MAX {
            strongconnect(v, adj, &mut index_counter,
                &mut stack, &mut on_stack, &mut index, &mut lowlink, &mut sccs);
        }
    }
    sccs
}
```

**SCC classification:**
- SCC of size 1 with no self-loop: non-recursive method (the common case)
- SCC of size 1 with a self-loop edge (v → v): directly recursive method
- SCC of size > 1: mutually recursive group

The condensation DAG — the directed acyclic graph formed by contracting each SCC to a single node — gives the topological order in which procedures should be analyzed (leaves first, entry points last).

---

### 6.3 Module Architecture

```
phase6/
│
├── mod.rs                          # Phase6Stage::run(bpa, sta, cfa, ssa) → CallGraphArtifact
│
├── call_sites/
│   ├── mod.rs                      # CallSiteExtractor: DFS over BP AST finding NN_CALL_EXPR,
│   │                               # NN_NEW_EXPR, NN_METHOD_REF, NN_LAMBDA_EXPR
│   ├── classifier.rs               # CallTypeClassifier: maps each call node to Σ_CG type
│   │                               # based on receiver expression, method modifiers, etc.
│   └── locator.rs                  # finds containing method (parent_sym via STA) and
│                                   # basic block (via CFA block_first/last_token lookup)
│
├── resolution/
│   ├── mod.rs                      # DispatchResolver trait
│   ├── direct.rs                   # DirectResolver: name lookup in STA for CG_DIRECT/SPECIAL
│   ├── cha.rs                      # CHAResolver: type-hierarchy walk for virtual/interface
│   └── one_cfa.rs                  # OneCFAResolver: SSA def-site inspection for allocation
│                                   # sites — refines CHA results per call site
│
├── points_to/
│   ├── mod.rs                      # PointsToAnalysis: Anderson constraint solver
│   ├── constraints.rs              # ConstraintSet: generates inclusions from SSA statements
│   └── worklist.rs                 # WorklistSolver: fixpoint propagation with BitSet pts sets
│
├── scc.rs                          # TarjanSCC: O(V+E) strongly connected components
│                                   # Classifies each method as non-recursive, self-recursive,
│                                   # or mutually recursive
│
├── builder.rs                      # CallGraphBuilder: accumulates CallSites, CallEdges,
│                                   # deduplicates polymorphic edges, builds CSR
└── serializer.rs                   # CallGraphArtifact binary I/O (.cga format)
```

---

### 6.4 Data Structure Specifications

**CallSite (28 bytes, fixed):**

```
Offset  Size  Field
 0       4    call_site_id   : u32   monotonically assigned, unique across codebase
 4       4    caller_sym     : u32   STA symbol_id of the containing method
 8       4    call_node      : u32   BP AST pre-order index of NN_CALL_EXPR / NN_NEW_EXPR
12       4    receiver_ssa   : u32   SSA variable_id of receiver (u32::MAX for static calls)
16       4    call_block     : u32   CFA basic block_id within the caller's CFG
20       4    call_token     : u32   TCA token_id of the method name identifier token
24       1    call_type      : u8    Σ_CG call type
25       1    flags          : u8    IS_POLYMORPHIC:1, HAS_NULL_RECEIVER:1, IS_TAIL_CALL:1
26       2    arg_count      : u16   number of arguments at this call site
Total: 28 bytes
```

`call_token` gives the traceability anchor: it is the token_id of the method name lexeme at this call site. Phase 7 uses this to build the traceability link `call_site → source_position`. Phase 9 uses it for sequence diagram timestamp ordering.

**CallEdge (16 bytes, stored in the CallerSym→CalleeSym CSR):**

```
Offset  Size  Field
 0       4    callee_sym     : u32   target method symbol_id
 4       4    call_site_id   : u32   generating call site (links back to CallSite table)
 8       1    edge_type      : u8    Σ_CG edge type
 9       3    _padding       : u24
12       4    _reserved      : u32
Total: 16 bytes
```

**SCCRecord (12 bytes, for the recursive cycle table):**

```
Offset  Size  Field
 0       4    scc_id           : u32   SCC index (0 = topological root, i.e., no callers outside)
 4       4    member_offset    : u32   byte offset into SCC member array
 8       2    member_count     : u16   number of methods in this SCC
10       1    scc_class        : u8    0=non-recursive, 1=self-recursive, 2=mutual-recursive
11       1    _padding         : u8
Total: 12 bytes
```

---

### 6.5 Algorithm Specifications

#### 6.5.1 Call Site Identification

```rust
pub fn extract_call_sites(
    bpa:   &BPASTArtifact,
    sta:   &SymbolTableArtifact,
    cfa:   &CFGArtifact,
    ssa:   &SSAArtifact,
) -> Vec<CallSite> {
    let mut sites = Vec::new();
    let mut id_counter = 0u32;

    for pre_idx in 0..bpa.node_count {
        let ntype = bpa.node_type(pre_idx);
        if !matches!(ntype, NN_CALL_EXPR | NN_NEW_EXPR | NN_METHOD_REF | NN_LAMBDA_EXPR) {
            continue;
        }

        let caller_sym   = find_enclosing_method(pre_idx, bpa, sta);
        let call_type    = classify_call(pre_idx, ntype, bpa, sta);
        let receiver_ssa = extract_receiver_ssa(pre_idx, bpa, ssa);
        let call_block   = find_block_for_stmt(pre_idx, caller_sym, cfa);
        let call_token   = find_method_name_token(pre_idx, bpa);
        let arg_count    = count_arguments(pre_idx, bpa) as u16;

        sites.push(CallSite {
            call_site_id: id_counter,
            caller_sym, call_node: pre_idx, receiver_ssa,
            call_block, call_token, call_type, arg_count,
            flags: 0,
        });
        id_counter += 1;
    }

    // Sort by caller_sym for locality in downstream analysis
    sites.sort_unstable_by_key(|s| s.caller_sym);
    sites
}

/// Classify a call node into Σ_CG by inspecting the call expression structure.
fn classify_call(
    call_node: u32, ntype: ASTNodeType,
    bpa: &BPASTArtifact, sta: &SymbolTableArtifact,
) -> CallType {
    match ntype {
        NN_NEW_EXPR        => CallType::Constructor,
        NN_METHOD_REF
        | NN_LAMBDA_EXPR   => CallType::Dynamic,
        NN_CALL_EXPR => {
            let receiver = get_receiver_child(call_node, bpa);
            if receiver.is_none() {
                return CallType::Special; // unqualified call — this.method() or super.method()
            }
            let method_name = get_called_method_name(call_node, bpa);
            if let Some(sym) = sta.resolve_method_call(call_node, bpa) {
                if sta.symbol(sym).modifiers.has(STATIC) {
                    CallType::Direct
                } else if sta.symbol(sym).visibility == PRIVATE {
                    CallType::Special
                } else if sta.symbol(sym).kind == SK_INTERFACE {
                    CallType::Interface
                } else {
                    CallType::Virtual
                }
            } else {
                CallType::Virtual // conservative fallback
            }
        }
        _ => CallType::Direct,
    }
}
```

#### 6.5.2 Call Graph Assembly

```rust
pub fn build_call_graph(
    call_sites: &[CallSite],
    bpa: &BPASTArtifact, sta: &SymbolTableArtifact, ssa: &SSAArtifact,
    pts: &HashMap<SsaId, BitSet<AllocId>>,
) -> Vec<(CallSite, Vec<SymbolId>)> {
    let resolver_direct   = DirectResolver::new(sta);
    let resolver_cha      = CHAResolver::new(sta);
    let resolver_one_cfa  = OneCFAResolver::new(sta, ssa, pts);

    call_sites.iter().map(|site| {
        let targets: Vec<SymbolId> = match site.call_type {
            CallType::Direct | CallType::Special | CallType::Constructor => {
                // Monomorphic: exactly one target
                resolver_direct.resolve(site, bpa).into_iter().collect()
            }
            CallType::Virtual | CallType::Interface => {
                // Try 1-CFA first (more precise), fall back to CHA
                let cfa_targets = resolver_one_cfa.resolve(site, bpa);
                if cfa_targets.is_empty() {
                    resolver_cha.resolve(site, bpa)
                } else {
                    cfa_targets
                }
            }
            CallType::Dynamic => {
                // Lambda/method reference: use bootstrap method analysis heuristic
                resolve_dynamic(site, bpa, sta)
            }
            CallType::Reflection => {
                vec![] // conservative: no targets (sound approximation for CFL-reachability)
            }
        };
        (site.clone(), targets)
    }).collect()
}
```

**Deduplication of polymorphic edges:** Multiple call sites in the same caller method may call the same callee via different CG\_VIRTUAL edges. During CSR encoding, we deduplicate: if `(caller_sym, callee_sym)` already has an edge, we add the new `call_site_id` to a side-table mapping that edge to its call sites, rather than inserting a duplicate edge.

---

### 6.6 Output Schema: `CallGraphArtifact` Binary Format (`.cga`)

```
╔═══════════════════════════════════════════════════════════════════╗
║  CGA FILE FORMAT v1.0  (all integers: little-endian)             ║
╠═════════════════════╦════════════════╦══════════════════════════════╣
║ Section             ║ Size           ║ Description                  ║
╠═════════════════════╬════════════════╬══════════════════════════════╣
║ HEADER              ║ 64 B           ║ Magic, counts, ssa_hash link ║
╠═════════════════════╬════════════════╬══════════════════════════════╣
║ CALL SITE TABLE     ║ n_sites × 28 B ║ CallSite[n_sites], sorted    ║
║                     ║                ║ by caller_sym. O(log n)     ║
║                     ║                ║ binary search.              ║
╠═════════════════════╬════════════════╬══════════════════════════════╣
║ CALLEE CSR          ║ variable       ║ outgoing edges per method.   ║
║                     ║                ║ offsets[] + adj[] + edge     ║
║                     ║                ║ wavelet tree for Σ_CG.      ║
╠═════════════════════╬════════════════╬══════════════════════════════╣
║ CALLER CSR          ║ variable       ║ incoming edges (fan-in).     ║
║                     ║                ║ offsets[] + adj[].          ║
╠═════════════════════╬════════════════╬══════════════════════════════╣
║ CALL SITE → EDGE    ║ variable       ║ Maps each unique (from,to)   ║
║ MAP                 ║                ║ pair to its call_site_ids.   ║
╠═════════════════════╬════════════════╬══════════════════════════════╣
║ POINTS-TO TABLE     ║ variable       ║ Per-method: (ssa_id,         ║
║                     ║                ║ alloc_type_sym_id) pairs     ║
║                     ║                ║ from Anderson analysis.      ║
╠═════════════════════╬════════════════╬══════════════════════════════╣
║ SCC TABLE           ║ variable       ║ SCCRecord[] + member arrays. ║
║                     ║                ║ Sorted in condensation DAG   ║
║                     ║                ║ reverse topological order.  ║
╠═════════════════════╬════════════════╬══════════════════════════════╣
║ CHECKSUM            ║ 8 B            ║ CRC-64/ECMA                  ║
╚═════════════════════╩════════════════╩══════════════════════════════╝
```

**HEADER (64 bytes):**

```
Offset  Size  Field
 0       8    magic           0x4347410001000000 ("CGA\x00\x01\x00\x00\x00")
 8       4    format_version  0x00000001
12       4    method_count    |V_CG| (u32)
16       4    call_site_count (u32)
20       4    call_edge_count |E_CG| (u32, deduplicated)
24       8    ssa_hash        CRC-64 of input .ssa file
32       8    sta_hash        CRC-64 of input .sta file
40      24    _reserved       zeroed
Total: 64 bytes
```

**Size estimate (15K methods, 45K call sites, 60K call edges):**

```
Header:                64 B
Call site table:    45K × 28   =   1.26 MB
Callee CSR:        (15K+60K)×4 =  300 KB + wavelet tree ~30 KB
Caller CSR:         same       =  300 KB
Site→Edge map:     ~60K × 8   =  480 KB
Points-to table:    variable   ≈    5 MB (Anderson output)
SCC table:         15K × 12   =  180 KB + member arrays ~60 KB
───────────────────────────────────────────────
Total:                         ≈  7.6 MB uncompressed · ≈ 2–3 MB LZ4
```

---

### 6.7 Complexity Proofs

| Operation | Complexity | Notes |
|---|---|---|
| Call site identification DFS | O(n\_ast) | One BP AST scan |
| CHA per virtual call site | O(\|subclasses(T)\|) | Bounded by hierarchy depth |
| 1-CFA per virtual call site | O(1) to O(\|pts(v)\|) | SSA lookup + pts lookup |
| Anderson points-to | O(n\_ptr^{1.5}) empirical | BDD-based pts sets |
| Tarjan SCC | O(V\_CG + E\_CG) | Single DFS |
| CSR encoding | O(V\_CG + E\_CG) | Prefix sum |

**Total Phase 6:** O(n\_ast + n\_call\_sites × avg\_cha\_fan\_out + Anderson\_cost). Dominated by Anderson's points-to for large codebases, but typically under 3 seconds for a 500K LOC Java project with BDD-based pts sets.

---

### 6.8 Phase 6 Invariants for Phase 7

**Invariant 1 (Call Site Coverage):** Every NN\_CALL\_EXPR, NN\_NEW\_EXPR, and NN\_METHOD\_REF node in the BP AST with a non-null containing method has a corresponding CallSite record. Verified: count call expression BP AST nodes, assert equal to `call_site_count` in the CGA header.

**Invariant 2 (Call Token Validity):** Every `CallSite.call_token` is a valid TCA token_id corresponding to a TT\_IDENTIFIER token whose text is the callee method name. Phase 7 reads this token to build the source-to-call-edge traceability link.

**Invariant 3 (SCC Topological Completeness):** Every symbol in V\_CG belongs to exactly one SCC. `Σ scc.member_count = |V_CG|`. Verified by asserting total member count equals `method_count`.

**Invariant 4 (CGA→SSA Hash Chain):** `cga_header.ssa_hash == crc64(ssa_file_bytes)`. Phase 7 validates this to ensure the call graph was built from the correct SSA version.

---

Now the visualization: