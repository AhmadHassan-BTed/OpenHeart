---

## Phase 9: UML Semantic Metadata Extraction

### 9.1 Phase Mandate & Exact Boundaries

Phase 9 is the only phase in the pipeline that speaks UML. It ingests all eight prior artifacts and applies a suite of structural transformation rules and graph pattern queries to produce the semantic metadata records that drive generation of all 14 UML diagram types. It does not perform any new program analysis — every fact it outputs is derived from facts already computed in Phases 1–8.

Phase 9 has exactly one responsibility: translate SCPG entities into UML-meaningful records. Every transformation is a function from a specific SCPG sub-graph to a specific UML diagram element. The intelligence is in the translation rules, not in further analysis.

**Inputs:** All eight prior artifacts (`.tca`, `.bpa`, `.sta`, `.cfa`, `.ssa`, `.cga`, `.tra`, `.psa`).

**Output:** `UMLMetadataArtifact (.uma)` — all 14 diagram type record arrays, design pattern annotations, and the label text table.

---

### 9.2 Mathematical Foundations

#### 9.2.1 The UML Extraction Map

Every UML diagram type has an **extraction source** — a specific subset of SCPG layers — and an **extraction function** — a formal transformation from that sub-graph to diagram elements. The extraction functions are defined as structural transformations over the SCPG's typed edge and node sets:

```
Diagram Type              Primary SCPG Source        Secondary Sources
─────────────────────────────────────────────────────────────────────────
1. Class                  V_sym + E^TH               TRA.uml_links, PSA.cyclomatic
2. Object                 SSA alloc sites             STA type hierarchy
3. Component              STA packages + CGA           CGA callee_csr
4. Deployment             STA module annotations      (external build config)
5. Package                STA namespaces              E^TH (package-level uses)
6. Composite Structure    STA inner classes           STA composition fields
7. Profile                STA annotations             STA annotation type decls
8. Use Case               CGA fan-in=0 methods        STA public visibility
9. Activity               CFA blocks + E^CFG          PSA ROBDD, SSA CDG
10. State Machine         SSA IFDS type-state          STA type hierarchy
11. Sequence              CGA call sites + SCC order  CFA call block topo order
12. Communication         CGA + STA associations      TRA call_site spans
13. Interaction Overview  CFA + CGA hybrid             Activity + Sequence records
14. Timing                SSA thread analysis          STA synchronized methods
```

**Formal extraction function definition:** For diagram type D with primary source P and secondary sources S₁,...,Sₙ:

f\_D: P × S₁ × ... × Sₙ → UMLRecord\_D

Each f\_D is a total function — every SCPG sub-graph produces zero or more UML records. A sub-graph producing zero records means no UML element is generated for that entity (e.g. a private nested utility class produces no class diagram entry if configured to exclude private types).

#### 9.2.2 CFG → Activity Diagram Transformation

The activity diagram transformation maps each CFA basic block to exactly one UML activity node type, using the following classification function:

```
classify_block(b, cfg, bpa, ssa) → ActivityNodeKind:

  if b == cfg.entry_block         → InitialNode
  if b == cfg.exit_block          → ActivityFinalNode
  if cfg.successors(b).len() == 0 → ActivityFinalNode  (unreachable exit)
  if cfg.predecessors(b).len() ≥ 2
     AND cfg.successors(b).len() == 1  → MergeNode   (join of branches)
  if cfg.successors(b).len() == 2
     AND cfg.predecessors(b).len() == 1 → DecisionNode (binary branch)
  if b is a loop header
     (∃ back_edge e: e.to == b)        → DecisionNode + LOOP annotation
  if b is a catch block              → ExceptionHandlerNode
  if cfg.successors(b).len() ≥ 2
     AND all successors are switch cases → DecisionNode (multi-way)
  else                               → ActionNode      (straight-line execution)
```

**Edge transformation:** Each CFG edge becomes an ActivityEdge with guard condition text:

```
classify_edge(e, cfg, bpa, ssa) → (ActivityEdge, Option<GuardLabel>):

  CFG_TRUE  edge: guard = condition_text(source_block(e), bpa, ssa)
  CFG_FALSE edge: guard = "¬" + condition_text(source_block(e), bpa, ssa)
  CFG_UNCOND: guard = None
  CFG_LOOP_BACK: guard = None + IS_BACK_EDGE flag → renders as return arrow
  CFG_EXCEPT: guard = exception_type_name(e, sta)
```

**Compound activity regions:** Sequential ActionNodes with no branches between them are merged into a single compound ActionNode with multiple statements in its label. This reduces visual clutter — a function with 15 consecutive statements without branches produces one action node, not 15.

```rust
fn merge_sequential_actions(
    nodes: &[ActivityNode], cfg: &FunctionCFGData,
) -> Vec<ActivityNode> {
    let mut merged = Vec::new();
    let mut run: Vec<u32> = Vec::new(); // accumulating sequential block ids

    for node in nodes {
        if node.kind == ActionNode && cfg.predecessors(node.block_id).len() == 1
                                   && cfg.successors(node.block_id).len() == 1 {
            run.push(node.block_id);
        } else {
            if run.len() > 1 {
                merged.push(ActivityNode::compound(run.drain(..).collect()));
            } else if let Some(id) = run.pop() {
                merged.push(ActivityNode::single(id));
            }
            merged.push(node.clone());
        }
    }
    merged
}
```

#### 9.2.3 Type-State IFDS → State Machine Transformation

The SSA artifact's IFDS type-state results (Section 5: `IFDS_TYPE_STATE`) store (ssa\_id, state\_id) pairs. Phase 9 extracts state machines by grouping these results per class type:

**Step 1 — Group by class type:** For each (ssa\_id, state\_id) pair in the IFDS results, look up `ssa.record(ssa_id).orig_sym_id` → `sta.symbol(sym_id).type_id` → the class type sym_id. Group all (ssa\_id, state\_id, containing\_method) triples by class type.

**Step 2 — Identify state-triggering methods:** For each class type C with multiple states, find all methods m such that calling m on an object of type C can change its state: scan the CFA and CGA for call sites whose receiver type is C and whose pre-call and post-call states differ in the IFDS results.

**Step 3 — Build transition relation:** For each triggering method m and state transition (s → s'), create a `TransitionRecord`.

**Step 4 — Identify initial and final states:** The initial state is the state that new allocations of C start in (IFDS entry fact at `new C()` allocation sites). Final states are states from which no outgoing transitions exist.

```rust
fn extract_state_machine(
    class_type_id: u32, sta: &STA, ssa: &SSA, cfa: &CFA, cga: &CGA,
) -> Option<StateMachineRecord> {
    let type_state_pairs = ssa.ifds_type_state_for_class(class_type_id);
    if type_state_pairs.is_empty() { return None; }

    // Collect all reachable states
    let states: BTreeSet<u16> = type_state_pairs.iter().map(|p| p.state_id).collect();
    if states.len() < 2 { return None; } // trivial automaton — skip

    // Find transitions from call sites
    let mut transitions = Vec::new();
    for call_site in cga.call_sites_on_type(class_type_id) {
        let pre_state  = ssa.state_before_call(call_site, class_type_id);
        let post_state = ssa.state_after_call(call_site, class_type_id);
        if pre_state != post_state {
            transitions.push(TransitionRecord {
                from_state:  pre_state,
                to_state:    post_state,
                trigger:     call_site.method_sym_id,
                guard:       extract_guard_condition(call_site, cfa, ssa),
                action:      None,
            });
        }
    }

    let initial_state = ssa.initial_state_of_class(class_type_id);
    let final_states  = states.iter()
        .filter(|&&s| !transitions.iter().any(|t| t.from_state == s))
        .copied().collect();

    Some(StateMachineRecord {
        class_sym_id: class_type_id,
        states: states.into_iter().map(|s| StateRecord {
            state_id: s as u32,
            state_name: ssa.state_name(s),
            is_initial: s == initial_state,
            is_final: final_states.contains(&s),
            ..Default::default()
        }).collect(),
        transitions,
        initial_state: initial_state as u32,
        final_states: final_states.iter().map(|&s| s as u32).collect(),
    })
}
```

#### 9.2.4 Call Graph → Sequence Diagram Transformation

A sequence diagram represents a specific execution scenario. Phase 9 generates one sequence diagram per **entry point** — a public method with call-graph fan-in of zero (no in-codebase callers).

**DFS traversal with call ordering:**

```rust
fn extract_sequence_diagram(
    entry_method: u32, cga: &CGA, cfa: &CFA, sta: &STA, tra: &TRA,
) -> SequenceDiagramRecord {
    let mut lifelines: IndexMap<u32, LifelineRecord> = IndexMap::new();
    let mut messages:  Vec<MessageRecord>             = Vec::new();
    let mut ordinal:   u32                            = 0;

    // Actor lifeline for the entry point's implicit caller
    lifelines.insert(EXTERNAL_ACTOR_ID, LifelineRecord {
        sym_id: EXTERNAL_ACTOR_ID, is_actor: true,
        name_id: sta.text_id("<<actor>>"), type_sym_id: EXTERNAL_ACTOR_ID,
    });

    // DFS over the call graph — respects SCC topological order
    let mut stack  = vec![(EXTERNAL_ACTOR_ID, entry_method, vec![])];
    let mut visited: HashSet<(u32,u32)> = HashSet::new(); // (caller, callee)

    while let Some((caller_sym, method_sym, scope_fragments)) = stack.pop() {
        // Add lifeline for this method's declaring class
        let class_sym = sta.symbol(method_sym).parent_sym;
        lifelines.entry(class_sym).or_insert_with(|| {
            LifelineRecord { sym_id: class_sym, is_actor: false,
                name_id: sta.symbol(class_sym).name_id,
                type_sym_id: class_sym }
        });

        // Find all call sites within this method, sorted by CFG topological order
        let mut call_sites: Vec<&CallSite> = cga.call_sites_from(method_sym).collect();
        call_sites.sort_by_key(|cs| cfa.block_topo_order(method_sym, cs.call_block));

        for call_site in &call_sites {
            for &callee_sym in cga.callees_of_site(call_site.call_site_id) {
                if visited.contains(&(method_sym, callee_sym)) { continue; }
                visited.insert((method_sym, callee_sym));

                let callee_class = sta.symbol(callee_sym).parent_sym;
                messages.push(MessageRecord {
                    from_lifeline: class_sym,
                    to_lifeline:   callee_class,
                    call_site_id:  call_site.call_site_id,
                    method_sym_id: callee_sym,
                    message_kind:  classify_message(call_site, sta),
                    ordinal,
                    uml_link: tra.uml_link_for_call_site(call_site.call_site_id),
                });
                ordinal += 1;

                // Check if call is inside a loop or conditional — emit combined fragment
                let cfg_block = cfa.block(method_sym, call_site.call_block);
                if cfg_block.loop_depth > 0 {
                    // Emit LOOP combined fragment for this message
                } else if is_in_conditional_branch(call_site, cfa, method_sym) {
                    // Emit ALT combined fragment
                }

                // Recurse into callee (bounded by SCC depth)
                if !is_recursive_scc(callee_sym, cga) {
                    stack.push((method_sym, callee_sym, scope_fragments.clone()));
                }
            }
        }
    }

    SequenceDiagramRecord {
        scenario_name: entry_method_name(entry_method, sta),
        lifelines: lifelines.into_values().collect(),
        messages,
        combined_fragments: vec![],
    }
}
```

#### 9.2.5 Design Pattern Detection

Phase 9 implements pattern detection as **structural queries over the SCPG**. Each query is a Boolean function over a class symbol's neighborhood in the SCPG:

**Singleton pattern:**

```rust
fn is_singleton(class_sym: u32, sta: &STA) -> bool {
    let methods = sta.methods_of(class_sym);
    let fields  = sta.fields_of(class_sym);

    // ① Has a private or protected constructor
    let has_private_ctor = sta.constructors_of(class_sym)
        .any(|c| sta.symbol(c).visibility == PRIVATE || sta.symbol(c).visibility == PROTECTED);

    // ② Has a static field of its own type
    let has_static_self_field = fields.iter()
        .any(|&f| sta.symbol(f).modifiers & STATIC != 0
              && sta.symbol(f).type_id == class_sym);

    // ③ Has a static method that returns its own type
    let has_static_factory = methods.iter()
        .any(|&m| sta.symbol(m).modifiers & STATIC != 0
              && sta.symbol(m).type_id == class_sym);

    has_private_ctor && (has_static_self_field || has_static_factory)
}
```

**Observer pattern:**

```rust
fn is_observer_subject(class_sym: u32, sta: &STA, cga: &CGA) -> bool {
    let methods = sta.methods_of(class_sym);

    // ① Has a collection field whose element type is an interface
    let has_listener_collection = sta.fields_of(class_sym).iter()
        .any(|&f| sta.is_collection_type(sta.symbol(f).type_id)
              && {
                  let elem_type = sta.collection_element_type(sta.symbol(f).type_id);
                  sta.symbol(elem_type).kind == SK_INTERFACE
              });

    // ② Has addListener/removeListener pattern methods
    let has_add_remove = methods.iter()
        .any(|&m| {
            let name = sta.token_text(sta.symbol(m).name_id);
            (name.starts_with("add") || name.starts_with("register"))
            && name.to_lowercase().contains("listener")
        });

    // ③ Calls methods on the listener interface (via call graph)
    let calls_listener_interface = cga.callees_of(class_sym)
        .any(|callee| sta.symbol(sta.symbol(callee).parent_sym).kind == SK_INTERFACE);

    has_listener_collection && has_add_remove && calls_listener_interface
}
```

**Builder pattern:**

```rust
fn is_builder(class_sym: u32, sta: &STA) -> bool {
    let methods = sta.methods_of(class_sym);

    // ① Most methods return the builder class itself (fluent interface)
    let self_returning = methods.iter()
        .filter(|&&m| sta.symbol(m).type_id == class_sym)
        .count();
    let fluent_ratio = self_returning as f32 / methods.len().max(1) as f32;

    // ② Has exactly one terminal method returning a different type
    let terminal_methods = methods.iter()
        .filter(|&&m| sta.symbol(m).type_id != class_sym
                   && sta.symbol(m).type_id != VOID_TYPE_ID)
        .count();

    fluent_ratio > 0.5 && terminal_methods == 1
}
```

#### 9.2.6 Label Extraction from AST

Activity diagram and sequence diagram nodes need human-readable text labels. Phase 9 extracts these from the BP AST by recursively summarizing the statement sub-tree:

```rust
fn extract_action_label(
    stmt_node: u32, bpa: &BPASTArtifact, tca: &TCA, sta: &STA, max_chars: usize,
) -> u32 // text_id for the extracted label
{
    let label_text = match bpa.node_type(stmt_node) {
        NN_ASSIGN_EXPR     => format!("{} = {}", lhs_name(stmt_node, bpa, tca), rhs_summary(stmt_node, bpa, tca)),
        NN_LOCAL_VAR_DECL  => format!("{} {} = ...", type_name(stmt_node, bpa, tca, sta), var_name(stmt_node, bpa, tca)),
        NN_CALL_EXPR       => format!("{}.{}({})", receiver_name(stmt_node, bpa, tca), method_name(stmt_node, bpa, tca), param_summary(stmt_node, bpa, tca)),
        NN_RETURN_STMT     => format!("return {}", rhs_summary(stmt_node, bpa, tca)),
        NN_THROW_STMT      => format!("throw {}", exception_name(stmt_node, bpa, tca)),
        NN_IF_STMT         => format!("[{}]", condition_text(stmt_node, bpa, tca)),
        NN_WHILE_STMT      => format!("loop [{}]", condition_text(stmt_node, bpa, tca)),
        NN_FOR_STMT        => format!("for {}", loop_var_summary(stmt_node, bpa, tca)),
        NN_ENHANCED_FOR    => format!("for each {} in {}", loop_var_name(stmt_node, bpa, tca), collection_name(stmt_node, bpa, tca)),
        NN_EXPR_STMT       => expr_summary(first_child_of(stmt_node, bpa), bpa, tca),
        _                  => source_text_truncated(stmt_node, bpa, tca, max_chars),
    };

    // Truncate to max_chars with ellipsis
    let final_text = if label_text.len() > max_chars {
        format!("{}...", &label_text[..max_chars.saturating_sub(3)])
    } else {
        label_text
    };

    tca.intern_string(final_text)
}
```

---

### 9.3 Module Architecture

```
phase9/
├── mod.rs                          # Phase9Stage::run(all 8 artifacts) → UMLMetadataArtifact
│                                   # Runs all extractors in dependency order
│
├── structural/
│   ├── mod.rs                      # StructuralExtractor: coordinates structural diagram types
│   ├── class_diagram.rs            # ClassDiagramExtractor → ClassRecord[]
│   │                               # Scans STA V_sym, reads E^TH, populates fields/methods
│   ├── object_diagram.rs           # ObjectDiagramExtractor → ObjectRecord[]
│   │                               # Analyzes SSA alloc sites for runtime instance snapshots
│   ├── component_diagram.rs        # ComponentDiagramExtractor → ComponentRecord[]
│   │                               # Groups STA packages by module boundary + CGA inter-pkg edges
│   ├── package_diagram.rs          # PackageDiagramExtractor → PackageRecord[]
│   │                               # STA namespace hierarchy + TH_USES inter-pkg edges
│   └── composite_diagram.rs        # CompositeDiagramExtractor → CompositeRecord[]
│                                   # STA inner class relationships + port/connector analysis
│
├── behavioral/
│   ├── mod.rs                      # BehavioralExtractor: coordinates behavioral diagram types
│   ├── activity_diagram.rs         # ActivityDiagramExtractor → ActivityRecord[]
│   │                               # CFA blocks → activity nodes, CFG edges → activity edges
│   │                               # sequential ActionNode merging, combined fragment detection
│   ├── state_machine.rs            # StateMachineExtractor → StateMachineRecord[]
│   │                               # SSA IFDS type-state results → state automata
│   ├── sequence_diagram.rs         # SequenceDiagramExtractor → SequenceDiagramRecord[]
│   │                               # DFS over CGA from fan-in=0 entry points
│   ├── communication_diagram.rs    # CommunicationDiagramExtractor → CommunicationRecord[]
│   │                               # Object collaboration from CGA + TRA call site spans
│   ├── interaction_overview.rs     # InteractionOverviewExtractor → InteractionRecord[]
│   │                               # Hybrid: ActivityRecord embedding SequenceDiagramRecord refs
│   └── timing_diagram.rs           # TimingDiagramExtractor → TimingRecord[]
│                                   # SSA thread/synchronized method analysis
│
├── patterns/
│   ├── mod.rs                      # PatternDetector: runs all pattern queries, emits DesignPatternRecord
│   ├── singleton.rs                # Singleton query
│   ├── observer.rs                 # Observer/Listener query
│   ├── factory.rs                  # Factory Method + Abstract Factory
│   ├── builder.rs                  # Builder (fluent interface)
│   ├── state.rs                    # State/Strategy pattern
│   └── template_method.rs         # Template Method (abstract + protected methods)
│
├── label_extraction.rs             # LabelExtractor: BP AST → human-readable text
├── actor_identification.rs         # ActorIdentifier: public fan-in=0 methods → use case actors
├── builder.rs                      # UMABuilder: aggregates all diagram records
└── serializer.rs                   # UMASerializer: .uma binary I/O
```

---

### 9.4 Data Structure Specifications

**ClassRecord (variable-length):**

```
ClassRecord header (32 bytes):
  sym_id:           u32    STA symbol_id
  stereotype:       u8     (0=none, 1=abstract, 2=interface, 3=enum, 4=record, 5=annotation)
  visibility:       u8
  modifiers:        u16    (same bit layout as STA SymbolRecord.modifiers)
  extends_sym:      u32    superclass symbol_id (u32::MAX = java.lang.Object or root)
  field_count:      u16
  method_count:     u16
  inner_count:      u16    number of inner classes
  design_pattern:   u8     (0=none, 1=singleton, 2=observer, 3=factory, 4=builder, 5=state, ...)
  _reserved:        u8
  type_param_count: u8
  _pad:             u8
  uml_link:         UMLLinkRecord (24 bytes)
Header: 56 bytes
Followed by: field_count × FieldRecord + method_count × MethodRecord + inner_count × u32
```

**ActivityRecord (variable-length per function):**

```
ActivityRecord header (20 bytes):
  function_sym_id:  u32
  node_count:       u16
  edge_count:       u16
  start_node:       u16     InitialNode index
  end_node_count:   u8
  swimlane_count:   u8
  cyclomatic:       u16     from PSA (equals number of DecisionNodes + 1)
  _reserved:        u16
Header: 20 bytes
Followed by: node_count × ActivityNode + edge_count × ActivityEdge
```

**ActivityNode (16 bytes):**

```
  node_id:          u32     maps to CFA basic block global_id
  label_text_id:    u32     TCA/phase9 string id for the node label
  node_kind:        u8      ActionNode|DecisionNode|MergeNode|ForkNode|JoinNode|InitialNode|FinalNode|ExceptionHandlerNode
  loop_depth:       u8      from CFA block metadata
  guard_text_id:    u16     for DecisionNode: condition label text_id
Total: 16 bytes
```

**StateMachineRecord (variable-length):**

```
Header (16 bytes):
  class_sym_id:     u32
  state_count:      u16
  transition_count: u16
  initial_state:    u16
  final_state_count:u8
  _reserved:        u8
  _pad:             u32
Followed by: state_count × StateRecord + transition_count × TransitionRecord
```

**SequenceDiagramRecord (variable-length):**

```
Header (12 bytes):
  scenario_name:    u32     text_id
  lifeline_count:   u16
  message_count:    u16
  fragment_count:   u16
  _reserved:        u16
Followed by: lifeline_count × LifelineRecord + message_count × MessageRecord + fragment_count × CombinedFragment
```

---

### 9.5 Output Schema: `UMLMetadataArtifact` Binary Format (`.uma`)

```
╔══════════════════════════════════════════════════════════════════════╗
║  UMA FILE FORMAT v1.0  (all integers: little-endian)                ║
╠═══════════════════════╦══════════════════╦══════════════════════════╣
║ Section               ║ Size             ║ Description              ║
╠═══════════════════════╬══════════════════╬══════════════════════════╣
║ HEADER                ║ 64 B             ║ Magic, counts, tra_hash  ║
╠═══════════════════════╬══════════════════╬══════════════════════════╣
║ CLASS RECORDS         ║ variable         ║ ClassRecord[] — primary  ║
║                       ║                  ║ source for diagrams 1–7  ║
╠═══════════════════════╬══════════════════╬══════════════════════════╣
║ ACTIVITY RECORDS      ║ variable         ║ ActivityRecord[] per fn  ║
╠═══════════════════════╬══════════════════╬══════════════════════════╣
║ STATE MACHINE RECORDS ║ variable         ║ StateMachineRecord[]     ║
╠═══════════════════════╬══════════════════╬══════════════════════════╣
║ SEQUENCE DIAGRAM REC. ║ variable         ║ One per entry point      ║
╠═══════════════════════╬══════════════════╬══════════════════════════╣
║ PACKAGE RECORDS       ║ variable         ║ Package/module hierarchy ║
╠═══════════════════════╬══════════════════╬══════════════════════════╣
║ COMPONENT RECORDS     ║ variable         ║ Component + port model   ║
╠═══════════════════════╬══════════════════╬══════════════════════════╣
║ DESIGN PATTERN TABLE  ║ n_patterns × 12B ║ (class_sym, pattern_kind ║
║                       ║                  ║  confidence) × detected  ║
╠═══════════════════════╬══════════════════╬══════════════════════════╣
║ LABEL TEXT TABLE      ║ variable         ║ Interned UML label text  ║
║                       ║                  ║ not in TCA string table  ║
╠═══════════════════════╬══════════════════╬══════════════════════════╣
║ CHECKSUM              ║ 8 B              ║ CRC-64/ECMA              ║
╚═══════════════════════╩══════════════════╩══════════════════════════╝
```

**Size estimate (2K classes, 15K methods, 200 entry points, 100 detected patterns):**

```
Header:                   64 B
Class records:        2K × ~500 B avg  =   1 MB
Activity records:   15K × avg 30 nodes × 16B + 35 edges × 8B = 12 MB
State machine:       50 automata × avg 150 B =  7.5 KB
Sequence diagrams:  200 × avg 20 msgs × 40B  =  160 KB
Package records:     500 packages × 100 B    =   50 KB
Component records:   100 components × 200 B  =   20 KB
Pattern table:       200 × 12 B              =  2.4 KB
Label text table:   15K fns × avg 5 labels × avg 25 chars ≈ 1.9 MB
──────────────────────────────────────────────────────────
Total:                                        ≈ 15.1 MB uncompressed
After LZ4:                                    ≈ 4–5 MB
```

---

### 9.6 Complexity Proofs

| Extraction | Complexity | Notes |
|---|---|---|
| Class diagram extraction | O(n\_sym) | Linear scan of STA |
| Activity diagram (per function) | O(n\_blocks\_f + n\_edges\_f) | One scan of CFA per function |
| State machine extraction | O(n\_ssa × n\_states) | Grouped IFDS result scan |
| Sequence diagram (per entry point) | O(n\_call\_sites × max\_depth) | DFS bounded by SCC depth |
| Design pattern detection | O(n\_classes × n\_patterns) | Each pattern = O(n\_sym) per class |
| Label extraction | O(n\_ast) | One BP AST scan for all labels |
| **Total Phase 9** | **O(n\_ast + n\_classes × n\_patterns)** | Pattern detection dominates |

For 2K classes × 7 pattern types: 14K pattern evaluations, each O(n\_sym\_neighborhood) ≈ O(100). Total ≈ 1.4M operations ≈ 7ms. Phase 9 is the fastest analysis-intensive phase.

---

### 9.7 Phase 9 Invariants for Phase 10

**Invariant 1 (Class Coverage):** Every STA symbol with `kind ∈ {SK_CLASS, SK_INTERFACE, SK_ENUM, SK_RECORD}` and `visibility == PUBLIC` has a corresponding `ClassRecord` in the UMA. Private inner utility classes may be excluded per configuration.

**Invariant 2 (Activity Completeness):** Every STA symbol with `kind ∈ {SK_METHOD, SK_CONSTRUCTOR}` that has a CFG body has a corresponding `ActivityRecord`. Abstract methods (no CFA body) produce no activity record.

**Invariant 3 (UMLLink Validity):** Every `ClassRecord`, `ActivityNode`, and `MessageRecord` carries a `UMLLinkRecord` whose `scpg_hash` equals `tra.header.scpg_hash`. Phase 10 validates all UMLLinks against the TRA hash chain before rendering.

**Invariant 4 (Pattern Confidence):** Every `DesignPatternRecord` has a `confidence` field in [0, 100] that reflects how many pattern criteria were satisfied. Confidence < 50 patterns are stored but not displayed by default (exposed only via query API). Phase 10 uses confidence thresholds for diagram rendering filters.

