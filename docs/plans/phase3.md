---

## Phase 3: Symbol Table & Type Hierarchy Construction

### 3.1 Phase Mandate & Exact Boundaries

Phase 3 is the first phase that requires semantic understanding of the target language. It walks the BP-encoded AST from Phase 2, constructs a scope graph for name resolution, discovers and resolves all declarations, and builds three interlinked outputs: the symbol table (V_sym vertices), the type hierarchy DAG (E^TH edges), and the UML semantic metadata records (ρ function). Every downstream phase — CFG construction (4), SSA (5), call graph (6) — indexes into the symbol table as its authoritative name-to-node mapping.

Phase 3 does NOT build the CFG (Phase 4), perform SSA conversion (Phase 5), or construct path summaries (Phase 8). Its entire output is purely structural and declarative — no control flow analysis occurs.

**Why multiple passes are mathematically necessary:** Java allows forward references. A class C appearing on line 1 may extend class D declared on line 1000. A single-pass resolution algorithm would fail to bind the reference C→D because D's symbol does not yet exist. The multi-pass architecture is not an engineering shortcut — it is a formal requirement imposed by the grammar's allowance of forward declarations. Two passes are sufficient for Java: Pass A discovers all top-level declarations; Pass B resolves all type references against the fully populated discovery set.

**Inputs:** `TokenCorpusArtifact (.tca)` — for string intern table lookups, qualified name construction, and token-to-text resolution. `BPASTArtifact (.bpa)` — for pre-order traversal, node type queries, token range lookups, and parent map navigation.

**Output:** `SymbolTableArtifact (.sta)` — the complete symbol table, type hierarchy graph, scope graph, qualified name table, UML metadata section, and annotation table.

---

### 3.2 Mathematical Foundations

#### 3.2.1 Scope Graph Formalism

**Definition (Scope Graph, Van Antwerpen et al. 2016):** A scope graph G_scope = (S, E_p, E_I, D, R) where:

S is a finite set of scope nodes. Each scope s ∈ S represents a lexical region in the source text (a file, class body, method body, block, or lambda capture boundary).

E_p ⊆ S × S is the parent-edge relation (lexical parent scoping). An edge (s_child, s_parent) ∈ E_p means declarations in s_parent are visible from s_child if not shadowed by a closer declaration. This forms a forest (no scope has more than one parent in the primary lexical hierarchy).

E_I ⊆ S × S is the import-edge relation. For Java: if file F has `import java.util.List`, then F's file scope s_F has an import edge to the scope of the `java.util` package, granting resolution access to all declarations within that package scope.

D: S → 2^{Symbol_id} maps each scope to the set of symbol_ids declared directly within it (not inherited).

R: S → 2^{Reference} maps each scope to the set of unresolved type references made within it.

**Resolution function:** Given a reference r (a type name as a string) within scope s:

```
resolve(r, s):
  1. For each scope s' reachable from s via E_p* ∪ E_I*
     (traversing parent edges and import edges in BFS order,
      innermost scope first):
     If ∃ d ∈ D(s') such that name(d) = r → return symbol_id(d)
  2. If not found: resolve_java_lang(r)  ← java.lang.* always implicitly visible
  3. If still not found: create_external_symbol(r) → SK_EXTERNAL
```

The ordering of s' in step 1 is critical — innermost scope wins, which implements Java's shadowing rules.

**Scope construction during Phase 3.1 (Declaration Discovery):**

For each declaration node in the BP AST, Phase 3 creates one scope node:

```
NN_MODULE       → file scope (root of the lexical hierarchy for this file)
NN_CLASS_DECL   → class scope (members declared here)
NN_METHOD_DECL  → method scope (parameters + local vars declared here)
NN_CONSTRUCTOR  → constructor scope
NN_FOR_STMT     → for-loop scope (loop variable declared here)
NN_CATCH_CLAUSE → catch scope (exception variable)
NN_LAMBDA_EXPR  → lambda scope (capture boundary)
NN_BLOCK        → block scope (for-each, try-with-resources, etc.)
```

Anonymous classes additionally have a capture edge to their enclosing method scope (to model access to effectively-final locals — the Java specification's requirement for captured variables).

#### 3.2.2 Symbol Kind Alphabet Σ_K

```
SymbolKind (u8):
0x00  SK_PACKAGE          # com.example, java.util
0x01  SK_CLASS            # class Foo { }
0x02  SK_INTERFACE        # interface Bar { }
0x03  SK_ENUM             # enum Color { RED, GREEN }
0x04  SK_RECORD           # record Point(int x, int y) {}  (Java 16+)
0x05  SK_ANNOTATION_TYPE  # @interface MyAnnotation {}
0x06  SK_METHOD           # void myMethod(int x) {}
0x07  SK_CONSTRUCTOR      # Foo(int x) {}
0x08  SK_FIELD            # private int count;
0x09  SK_ENUM_CONSTANT    # RED, GREEN, BLUE in enum
0x0A  SK_PARAM            # formal parameter
0x0B  SK_LOCAL_VAR        # local variable declaration
0x0C  SK_TYPE_PARAM       # <T>, <E extends Comparable>
0x0D  SK_LAMBDA           # lambda expression body
0x0E  SK_ANON_CLASS       # new Runnable() { ... }
0x0F  SK_STATIC_INIT      # static { ... }
0x10  SK_INSTANCE_INIT    # { ... } (instance initializer block)
0x11  SK_MODULE           # Java 9+ module-info.java module declaration
0x12  SK_EXTERNAL         # referenced but not defined in this codebase
```

#### 3.2.3 SymbolRecord Layout (64 bytes, fixed, cache-aligned)

```
Offset  Size  Field
 0       4    symbol_id       : u32   unique monotonic ID, array index
 4       4    name_id         : u32   simple name, TCA string table
 8       4    qual_name_id    : u32   fully qualified name, QualifiedNameTable
12       4    type_id         : u32   symbol_id of declared type (u32::MAX = void/none)
16       4    decl_node       : u32   BP AST pre-order index of declaration
20       4    def_node        : u32   BP AST pre-order index of definition body
24       4    parent_sym      : u32   symbol_id of enclosing symbol; u32::MAX = root
28       4    first_child     : u32   first child symbol (linked list head)
32       4    next_sibling    : u32   next sibling symbol (linked list node)
36       4    scope_id        : u32   scope graph node ID for this symbol's scope
40       4    uml_meta_offset : u32   byte offset into UML Metadata section
44       2    param_count     : u16   # parameters (methods/constructors); 0 otherwise
46       2    modifiers       : u16   bit flags (see §3.2.4)
48       1    kind            : u8    SymbolKind enum (Σ_K)
49       1    visibility      : u8    0=package 1=public 2=private 3=protected
50       1    type_param_count: u8    # generic type parameters on this symbol
51       1    flags           : u8    is_synthetic:1 is_anonymous:1 is_deprecated:1
                                      is_record_component:1 is_varargs:1 reserved:3
52       4    first_token_id  : u32   from TCA (Phase 7 traceability seed)
56       4    last_token_id   : u32   from TCA (Phase 7 traceability seed)
60       4    _reserved       : u32 = 0
             ─────────────────
             64 bytes total
```

**Modifiers bit flags (u16):**

```
bit  0 = STATIC           bit  8 = DEFAULT (interface default method)
bit  1 = FINAL            bit  9 = SEALED (Java 15+)
bit  2 = ABSTRACT         bit 10 = NON_SEALED
bit  3 = SYNCHRONIZED     bit 11 = VARARGS
bit  4 = NATIVE           bit 12 = BRIDGE (synthetic bridge)
bit  5 = VOLATILE         bit 13 = SYNTHETIC (compiler-generated)
bit  6 = TRANSIENT        bit 14 = DEPRECATED (has @Deprecated)
bit  7 = STRICTFP         bit 15 = RECORD_COMPONENT
```

#### 3.2.4 Type Hierarchy Graph — Formal Definition

The type hierarchy is a DAG H = (V_type, E^TH) where V_type ⊆ V_sym contains only class-like symbols (SK_CLASS, SK_INTERFACE, SK_ENUM, SK_RECORD, SK_ANNOTATION_TYPE, SK_EXTERNAL), and E^TH ⊆ V_type × V_type × Σ_TH where:

```
Σ_TH = { TH_EXTENDS,      # class A extends class B
          TH_IMPLEMENTS,   # class A implements interface B
          TH_USES,         # class A has a field of type B (in-codebase dependency)
          TH_CREATES }     # class A creates instances of B via new B(...) (for composition)
```

The DAG property is guaranteed by Java's type system: classes form a tree (single inheritance) and interfaces form a DAG. The combination of TH_EXTENDS and TH_IMPLEMENTS edges forms a DAG over all type nodes.

**Anti-cycle invariant:** If (A, B, TH_EXTENDS) ∈ E^TH, then (B, A, TH_EXTENDS) ∉ E^TH (Java prohibition on cyclic inheritance). Verified during construction.

**Topological sort:** The type hierarchy can be topologically sorted in O(V + E) using Kahn's algorithm. This sort order determines the order in which UML class diagrams lay out inheritance hierarchies.

#### 3.2.5 UML Association Kind Determination

The association type of a field-based relationship is determined by the following decision function:

```
assoc_kind(class C, field f: Type T):
  if T is primitive or String or boxed primitive → NONE (no UML relationship)
  if T is not in V_sym (external type) → DEPENDENCY (dashed arrow in UML)
  if T is in {List<X>, Set<X>, Map<K,V>, Collection<X>, Iterable<X>}:
    → AGGREGATION with multiplicity 0..* (hollow diamond)
  if T[] (array type):
    → AGGREGATION with multiplicity 0..* (hollow diamond)
  if f has @Nonnull or @NotNull annotation AND f is final AND
     T is instantiated in C's constructor(s) via new T(...):
    → COMPOSITION (filled diamond)
  if T is only referenced in C (no other class holds a reference):
    → COMPOSITION (filled diamond, conservative heuristic)
  else:
    → ASSOCIATION (plain line)
```

This is a conservative heuristic — it may incorrectly classify some associations but will not produce false compositions. Full composition detection requires alias analysis (Phase 5 SSA), so Phase 3 stores a preliminary `assoc_kind` that Phase 9 refines using SSA-based alias information.

---

### 3.3 Module Architecture

```
phase3/
│
├── mod.rs                        # Phase3Stage::run(tca, bpa) → SymbolTableArtifact
│                                 # Orchestrates the 5 resolution passes in sequence
│
├── scope_graph/
│   ├── mod.rs                    # ScopeGraph: (S, E_p, E_I, D, R) data structure
│   │                             # ScopeNode: {scope_id, parent_scope, import_targets[]}
│   │                             # Backed by two CSR graphs: parent-edges + import-edges
│   └── resolver.rs               # NameResolver: resolve(name, scope_id) → symbol_id
│                                 # Implements BFS over E_p* ∪ E_I* with innermost-first ordering
│
├── passes/
│   ├── mod.rs                    # PassPipeline: runs passes 1-5 in order
│   ├── pass1_discovery.rs        # Declaration discovery: DFS over BP AST, create skeletal
│   │                             # SymbolRecords, build scope nodes, link parent-child chains
│   ├── pass2_imports.rs          # Import resolver: build per-file import maps
│   │                             # (simple-name → qual-name) and wildcard import scopes
│   ├── pass3_types.rs            # Type reference resolver: resolve all NN_TYPE_REF nodes
│   │                             # to symbol_ids using scope graph + import maps
│   ├── pass4_members.rs          # Member resolver: fill type_id fields for fields,
│   │                             # params, locals; resolve method return types
│   └── pass5_hierarchy.rs        # Type hierarchy builder: create E^TH CSR from resolved
│                                 # extends/implements relationships + field type analysis
│
├── adapter/
│   ├── mod.rs                    # SemanticAdapter trait
│   └── java.rs                   # JavaSemanticAdapter: Java-specific resolution rules
│                                 # (java.lang.* implicit import, primitive types, boxing)
│
├── std_library/
│   ├── mod.rs                    # StdLibStubs: pre-built SK_EXTERNAL SymbolRecords
│   └── java_stubs.rs             # java.lang.Object, String, Integer, List, Map, etc.
│                                 # Loaded at Phase3 initialization (symbol_ids ≥ 0xC0000000)
│
├── uml_meta/
│   ├── mod.rs                    # UMLMetaExtractor: builds UMLMeta records per symbol
│   ├── associations.rs           # AssociationDetector: implements assoc_kind() heuristic
│   └── patterns.rs               # PatternDetector: Singleton, Factory, Observer detection
│
├── qual_name_table.rs            # QualifiedNameTable: interning for FQNs not in TCA
│                                 # Separate from TCA StringInternTable — stores built names
│
├── builder.rs                    # SymbolTableBuilder: accumulates all structures
└── serializer.rs                 # SymbolTableSerializer: writes .sta binary
```

---

### 3.4 Data Structure Specifications

**ScopeRecord (32 bytes):**

```
Offset  Size  Field
 0       4    scope_id         : u32
 4       4    parent_scope     : u32    (u32::MAX for root/file scopes)
 8       4    owner_symbol     : u32    symbol_id of the declaration that opened this scope
12       4    first_decl       : u32    first symbol_id declared directly in this scope
16       4    decl_count       : u32    count of direct declarations
20       2    import_count     : u16    number of import edges from this scope
22       1    scope_kind       : u8     FILE=0 CLASS=1 METHOD=2 BLOCK=3 LAMBDA=4 ANON=5
23       1    flags            : u8     has_wildcard_import:1 has_static_import:1 reserved:6
24       4    import_table_off : u32    byte offset into ScopeImportTable section
28       4    _reserved        : u32 = 0
             ──────────────────
             32 bytes total
```

**UMLAssociationRecord (28 bytes, for fields establishing UML relationships):**

```
Offset  Size  Field
 0       4    from_symbol_id   : u32    the class symbol owning the field
 4       4    to_symbol_id     : u32    the type symbol of the field
 8       4    field_symbol_id  : u32    the field symbol itself
12       1    assoc_kind       : u8     0=NONE 1=DEPENDENCY 2=ASSOCIATION 3=AGGREGATION 4=COMPOSITION
13       2    mult_min         : u16    multiplicity lower bound (0 = optional)
15       2    mult_max         : u16    multiplicity upper bound (u16::MAX = unbounded)
17       1    is_navigable     : u8     1 = navigable arrow in UML
18       4    role_name_id     : u32    field name as association role name (TCA string id)
22       4    _reserved        : u32 = 0
26       2    _padding         : u16 = 0
             ──────────────────
             28 bytes total
```

**QualifiedNameTable (variable-length, similar to TCA StringInternTable):**

Same structure as Phase 1's StringInternTable: FNV-1a hash table + length-prefixed UTF-8 storage. Stores constructed fully-qualified names like `com.example.service.UserService`, `java.util.concurrent.ConcurrentHashMap`, etc. Assigned `qual_name_id` values starting from 0x80000000 to distinguish from TCA text_ids.

---

### 3.5 Algorithm Specifications

#### 3.5.1 Pass 1 — Declaration Discovery

Declaration Discovery walks the BP AST in pre-order using the `parent_map` and `first_child`/`next_sibling` navigation operations defined in Phase 2. It uses an explicit stack (not recursion) to track the current scope and parent symbol context.

```rust
pub fn discover_declarations(
    bpa:     &BPASTArtifact,
    tca:     &TokenCorpusArtifact,
    adapter: &dyn SemanticAdapter,
    builder: &mut SymbolTableBuilder,
) {
    // Explicit DFS stack: (pre_order_idx, scope_frame)
    // scope_frame = (current_scope_id, current_parent_sym_id)
    struct Frame { pre_idx: u32, scope_id: u32, parent_sym: u32 }
    let mut stack: Vec<Frame> = vec![Frame {
        pre_idx: 0,
        scope_id: builder.create_scope(u32::MAX, ScopeKind::File),
        parent_sym: u32::MAX,
    }];
    let mut visited: Vec<bool> = vec![false; bpa.node_count as usize];

    while let Some(frame) = stack.last().cloned() {
        let Frame { pre_idx, scope_id, parent_sym } = frame;
        let node_type = bpa.node_type(pre_idx);

        if !visited[pre_idx as usize] {
            visited[pre_idx as usize] = true;

            // ── DECLARATION NODES: open a new symbol + scope ──────────────
            if adapter.is_declaration(node_type) {
                let name_id   = extract_name_token(pre_idx, bpa, tca, adapter);
                let vis       = extract_visibility(pre_idx, bpa, adapter);
                let mods      = extract_modifiers(pre_idx, bpa, adapter);
                let kind      = adapter.symbol_kind(node_type);
                let (ft, lt)  = bpa.token_range(pre_idx);

                let sym_id = builder.create_symbol(SymbolRecord {
                    name_id, kind, visibility: vis, modifiers: mods,
                    decl_node: pre_idx, def_node: pre_idx,
                    parent_sym, scope_id,
                    first_token_id: ft, last_token_id: lt,
                    // type_id, qual_name_id: filled in Pass 3/4
                    ..SymbolRecord::UNINIT
                });

                // Link symbol into parent's child chain
                builder.append_child(parent_sym, sym_id);

                // Open a child scope for the new declaration
                let child_scope = builder.create_scope(sym_id, adapter.scope_kind(node_type));
                builder.set_scope_owner(child_scope, sym_id);

                // Update stack frame: children see the new symbol as parent
                stack.last_mut().unwrap().scope_id  = child_scope;
                stack.last_mut().unwrap().parent_sym = sym_id;
            }

            // ── PUSH FIRST CHILD if any ───────────────────────────────────
            if let Some(child) = bpa.first_child(pre_idx) {
                stack.push(Frame {
                    pre_idx: child,
                    scope_id: stack.last().unwrap().scope_id,
                    parent_sym: stack.last().unwrap().parent_sym,
                });
                continue;
            }
        }

        // ── POP: try next sibling, then parent ────────────────────────────
        stack.pop();
        let parent_frame = stack.last().cloned();
        if let Some(sib) = bpa.next_sibling(pre_idx) {
            let (sc, ps) = parent_frame
                .map(|f| (f.scope_id, f.parent_sym))
                .unwrap_or((ROOT_SCOPE, u32::MAX));
            stack.push(Frame { pre_idx: sib, scope_id: sc, parent_sym: ps });
        }
    }
}
```

Time: O(n_ast) — one DFS pass over the BP AST using O(1) navigation operations. Space: O(depth_max) for the stack ≈ O(200) for typical Java.

#### 3.5.2 Pass 2 — Import Resolution

For Java, import statements appear as children of the NN_MODULE node. Pass 2 scans only the top-level children of each file root, collecting:

**Simple imports** (`import java.util.List`): maps `"List"` → qualified name `"java.util.List"` in that file's import map.

**Wildcard imports** (`import java.util.*`): records `"java.util"` as a wildcard package for that file scope. During resolution (Pass 3), if a simple name fails direct lookup, Phase 3 probes all wildcard packages.

**Static imports** (`import static java.lang.Math.PI`): maps `"PI"` → the static field symbol in the Math class. Stored separately in a per-file static import map.

```rust
pub fn resolve_imports(
    bpa:     &BPASTArtifact,
    tca:     &TokenCorpusArtifact,
    symbols: &SymbolTableBuilder,
    scope_graph: &mut ScopeGraph,
) {
    // For each file's root NN_MODULE node (pre_order 0 is always the first file root)
    for file_root in bpa.module_nodes() {
        let file_scope = symbols.scope_of(file_root);

        let mut child = bpa.first_child(file_root);
        while let Some(c) = child {
            if bpa.node_type(c) == NN_IMPORT_DECL {
                let import_text = extract_import_text(c, bpa, tca);
                if import_text.ends_with(".*") {
                    // Wildcard import: add import edge from file scope to package scope
                    let pkg_name = &import_text[..import_text.len()-2];
                    scope_graph.add_import_edge(file_scope, pkg_name);
                } else {
                    // Simple import: add (simple_name, qual_name) to file's import map
                    let simple_name = import_text.rsplit('.').next().unwrap();
                    scope_graph.add_import_mapping(file_scope, simple_name, &import_text);
                }
            }
            child = bpa.next_sibling(c);
        }
    }
}
```

#### 3.5.3 Pass 3 — Type Reference Resolution

Pass 3 walks every NN_TYPE_REF node in the BP AST and resolves its text to a symbol_id. Type references appear as children of: field declarations, method return types, parameter types, local variable declarations, extends/implements clauses, cast expressions, instanceof checks, and new expressions.

```rust
pub fn resolve_type_refs(
    bpa:       &BPASTArtifact,
    tca:       &TokenCorpusArtifact,
    symbols:   &mut SymbolTableBuilder,
    scope_graph: &ScopeGraph,
    adapter:   &dyn SemanticAdapter,
    qual_names: &mut QualifiedNameTable,
) {
    for pre_idx in 0..bpa.node_count {
        if bpa.node_type(pre_idx) != NN_TYPE_REF { continue; }

        // Retrieve the type name text from TCA
        let (first_tok, _) = bpa.token_range(pre_idx);
        let name_text = tca.token_text(first_tok);

        // Resolve primitive types immediately
        if let Some(prim_id) = adapter.primitive_type_id(name_text) {
            symbols.set_type_ref_resolution(pre_idx, prim_id);
            continue;
        }

        // Determine the enclosing scope of this type reference
        let scope_id = scope_of_node(pre_idx, bpa, symbols);

        // Resolution order (Java spec §6.5):
        let resolved = scope_graph.resolve(name_text, scope_id)
            .or_else(|| scope_graph.resolve_via_imports(name_text, scope_id))
            .or_else(|| resolve_same_package(name_text, scope_id, symbols))
            .or_else(|| resolve_java_lang(name_text, adapter))  // implicit java.lang.*
            .unwrap_or_else(|| {
                // Create an SK_EXTERNAL stub symbol for unresolvable references
                create_external(name_text, symbols, qual_names)
            });

        symbols.set_type_ref_resolution(pre_idx, resolved);
    }
}

/// Determine scope_id for a BP AST node by walking up parent_map until hitting a scope-opening node
fn scope_of_node(pre_idx: u32, bpa: &BPASTArtifact, sym: &SymbolTableBuilder) -> u32 {
    let mut cur = pre_idx;
    loop {
        if let Some(sym_id) = sym.symbol_at_node(cur) {
            return sym.symbol(sym_id).scope_id;
        }
        cur = bpa.parent(cur);
        if cur == u32::MAX { return ROOT_SCOPE; }
    }
}
```

**Handling generic types:** `List<String>` has NN_TYPE_REF node for `List` with a child NN_TYPE_PARAM node for `String`. Pass 3 resolves both: the outer type reference resolves `List` → its symbol_id; the inner type argument resolves `String` → its symbol_id. Both resolutions are stored in the type_ref_resolution map.

#### 3.5.4 Pass 4 — Member Declaration Type Resolution

Pass 4 fills `type_id` fields in SymbolRecords for fields, parameters, local variables, and method return types. It reads the resolved type from the Pass 3 output for each declaration's NN_TYPE_REF child.

```rust
pub fn resolve_member_types(symbols: &mut SymbolTableBuilder, type_refs: &TypeRefResolutions) {
    for sym_id in 0..symbols.symbol_count() {
        let kind = symbols.symbol(sym_id).kind;
        if !matches!(kind, SK_FIELD|SK_PARAM|SK_LOCAL_VAR|SK_METHOD|SK_CONSTRUCTOR) { continue; }

        let decl_node = symbols.symbol(sym_id).decl_node;

        // For fields, params, local vars: find the NN_TYPE_REF child of the declaration node
        if let Some(type_ref_node) = find_type_ref_child(decl_node, symbols.bpa()) {
            let type_sym_id = type_refs.get(type_ref_node).unwrap_or(UNRESOLVED_TYPE_ID);
            symbols.set_type_id(sym_id, type_sym_id);
        }

        // For methods: the NN_TYPE_REF child of NN_METHOD_DECL is the return type
        // It appears before the method name identifier in the AST
        // (Java grammar: modifiers type_spec name params body)
    }
}
```

#### 3.5.5 Pass 5 — Type Hierarchy Construction

Pass 5 builds E^TH from two sources: explicit extends/implements relationships extracted from class declarations, and implicit TH_USES relationships from field type analysis.

```rust
pub fn build_type_hierarchy(
    symbols:   &SymbolTableBuilder,
    type_refs: &TypeRefResolutions,
    bpa:       &BPASTArtifact,
    builder:   &mut TypeHierarchyBuilder,
) {
    // Source 1: explicit inheritance (from class declaration nodes)
    for sym_id in 0..symbols.symbol_count() {
        if !matches!(symbols.symbol(sym_id).kind, SK_CLASS|SK_INTERFACE|SK_ENUM|SK_RECORD) { continue; }

        let decl_node = symbols.symbol(sym_id).decl_node;

        // Find "extends" clause: in the BP AST, the superclass type ref appears
        // as the first NN_TYPE_REF child NOT in the formal_parameters subtree
        if let Some(super_node) = find_superclass_ref(decl_node, bpa) {
            let super_sym = type_refs.get(super_node).unwrap_or(JAVA_OBJECT_SYM_ID);
            builder.add_edge(sym_id, super_sym, TH_EXTENDS);
        }

        // Find "implements" clause: one or more NN_TYPE_REF nodes from the implements list
        for iface_node in find_implements_refs(decl_node, bpa) {
            let iface_sym = type_refs.get(iface_node).unwrap_or(UNRESOLVED_TYPE_ID);
            if iface_sym != UNRESOLVED_TYPE_ID {
                builder.add_edge(sym_id, iface_sym, TH_IMPLEMENTS);
            }
        }
    }

    // Source 2: field-based dependencies (TH_USES edges for UML associations)
    for sym_id in 0..symbols.symbol_count() {
        if symbols.symbol(sym_id).kind != SK_FIELD { continue; }

        let field_type_id = symbols.symbol(sym_id).type_id;
        if field_type_id == UNRESOLVED_TYPE_ID { continue; }

        let owner_sym_id = symbols.symbol(sym_id).parent_sym;
        let field_type_kind = symbols.symbol(field_type_id).kind;

        // Only add TH_USES edges for non-primitive, non-external types
        if matches!(field_type_kind, SK_CLASS|SK_INTERFACE|SK_ENUM|SK_RECORD) {
            builder.add_edge(owner_sym_id, field_type_id, TH_USES);

            // UML association detection
            let kind = detect_association(sym_id, owner_sym_id, field_type_id, symbols, bpa);
            builder.record_association(UMLAssociationRecord {
                from_symbol_id: owner_sym_id,
                to_symbol_id:   field_type_id,
                field_symbol_id: sym_id,
                assoc_kind: kind,
                mult_min: association_multiplicity_min(sym_id, symbols),
                mult_max: association_multiplicity_max(sym_id, symbols),
                role_name_id: symbols.symbol(sym_id).name_id,
                ..Default::default()
            });
        }
    }

    // Sort and deduplicate edges, then encode as CSR + wavelet tree
    builder.finalize_csr();
}

fn detect_association(
    field_sym: u32, owner_sym: u32, type_sym: u32,
    sym: &SymbolTableBuilder, bpa: &BPASTArtifact,
) -> AssocKind {
    let is_collection = sym.is_collection_type(type_sym);
    let is_array      = sym.symbol(field_sym).flags & FLAG_IS_ARRAY != 0;
    let is_final      = sym.symbol(field_sym).modifiers & FINAL != 0;
    let has_nonnull   = sym.symbol(field_sym).flags & FLAG_HAS_NONNULL != 0;
    let is_created_in_ctor = is_instantiated_in_constructor(field_sym, owner_sym, sym, bpa);

    if is_collection || is_array       { AssocKind::Aggregation }
    else if is_final && has_nonnull && is_created_in_ctor { AssocKind::Composition }
    else                               { AssocKind::Association }
}
```

**`is_instantiated_in_constructor` algorithm:** Scan the BP AST subtrees of all NN_CONSTRUCTOR_DECL nodes belonging to `owner_sym`. Within each constructor body, look for NN_ASSIGN_EXPR nodes where the left-hand side is the field name token and the right-hand side is NN_NEW_EXPR with the field's type. This is an O(n_ast_within_constructors) scan, bounded in practice to small subtrees.

---

### 3.6 Output Schema: `SymbolTableArtifact` Binary Format (`.sta`)

```
╔═══════════════════════════════════════════════════════════════════╗
║  STA FILE FORMAT v1.0  (all integers: little-endian)             ║
╠════════════════════╦══════════════╦════════════════════════════════╣
║ Section            ║ Size         ║ Description                    ║
╠════════════════════╬══════════════╬════════════════════════════════╣
║ HEADER             ║ 64 B         ║ Magic, counts, bpa_hash link   ║
╠════════════════════╬══════════════╬════════════════════════════════╣
║ SYMBOL TABLE       ║ n_sym × 64 B ║ SymbolRecord[] indexed by      ║
║                    ║              ║ symbol_id. O(1) access.        ║
╠════════════════════╬══════════════╬════════════════════════════════╣
║ NAME INDEX         ║ n_sym × 8 B  ║ Sorted (name_id, sym_id) pairs.║
║                    ║              ║ O(log n) lookup by simple name. ║
╠════════════════════╬══════════════╬════════════════════════════════╣
║ SCOPE GRAPH        ║ var.         ║ ScopeRecord[] + CSR parent     ║
║                    ║              ║ edges + import edge table.     ║
╠════════════════════╬══════════════╬════════════════════════════════╣
║ TYPE HIERARCHY     ║ var.         ║ CSR offsets[] + adj[] + wavelet ║
║ (CSR + WT)         ║              ║ tree edge labels (Σ_TH).       ║
╠════════════════════╬══════════════╬════════════════════════════════╣
║ QUAL NAME TABLE    ║ var.         ║ Interned FQNs not in TCA.      ║
║                    ║              ║ FNV-1a hash → offset map.      ║
╠════════════════════╬══════════════╬════════════════════════════════╣
║ UML METADATA       ║ var.         ║ UMLMeta records per symbol.    ║
║                    ║              ║ UMLAssociationRecord[] array.  ║
╠════════════════════╬══════════════╬════════════════════════════════╣
║ ANNOTATION TABLE   ║ var.         ║ Per-symbol annotation records. ║
╠════════════════════╬══════════════╬════════════════════════════════╣
║ CHECKSUM           ║ 8 B          ║ CRC-64/ECMA                    ║
╚════════════════════╩══════════════╩════════════════════════════════╝
```

**HEADER (64 bytes):**

```
Offset  Size  Field
 0       8    magic           0x53544100_01000000  ("STA\x00\x01\x00\x00\x00")
 8       4    format_version  0x00000001
12       4    symbol_count    n_sym (u32)
16       4    scope_count     n_scope (u32)
20       4    th_edge_count   number of type hierarchy edges (u32)
24       4    assoc_count     number of UML association records (u32)
28       4    qual_name_count (u32)
32       8    bpa_hash        CRC-64 of input .bpa file (integrity chain)
40       8    tca_hash        CRC-64 of input .tca file
48      16    _reserved       zeroed
Total: 64 bytes
```

**Size for a medium Java project (50K symbols, 2K classes, 15K methods):**

```
Header:              64 B
Symbol table:    50K × 64 =  3.2 MB
Name index:      50K × 8  =  400 KB
Scope graph:     30K × 32 =  960 KB + CSR edges ≈ 1.2 MB total
Type hierarchy:  2K types × avg 3 edges × 4 + wavelet tree ≈ 80 KB
Qual name table: 10K names × avg 35 chars ≈ 450 KB
UML metadata:    2K type records + 15K method records + 5K assoc. records ≈ 2.5 MB
Annotation table: 8K annotations × avg 30 B = 240 KB
──────────────────────────────────────────────────────────
Total:           ≈ 9.1 MB uncompressed  ·  ≈ 3–4 MB LZ4-compressed
```

---

### 3.7 Complexity Proofs

**Time Complexity:**

| Pass | Operation | Complexity | Notes |
|---|---|---|---|
| 1 | Declaration discovery DFS | O(n_ast) | O(1) BP navigation per node |
| 2 | Import map construction | O(n_import) | n_import = number of import stmts |
| 3 | Type ref resolution | O(n_ref × D) | D = avg import list depth; empirically D ≤ 10 |
| 4 | Member type assignment | O(n_sym) | O(1) per symbol via type_ref lookup |
| 5 | Type hierarchy CSR build | O(n_sym + n_edges) | Kahn's sort + CSR prefix sum |
| 5 | UML association detection | O(n_fields × n_ctor_stmts) | Bounded by constructor body size |
| — | CSR wavelet tree encoding | O(n_edges × log σ) | σ = 4 edge types, log 4 = 2 |

**Dominant term:** O(n_ast + n_ref × D). Since n_ref ≤ n_ast and D ≤ 10 empirically, this is O(10 × n_ast) = O(n_ast). Phase 3 is linear in AST size.

**Space Complexity:**

```
SymbolRecord[]:       64 × n_sym   ≈ 3.2 MB  (50K symbols)
ScopeRecord[]:        32 × n_scope ≈ 960 KB  (30K scopes)
TypeRefResolution[]:  8 × n_ref    ≈ 8 MB    (1M type refs — this is transient)
NameIndex[]:          8 × n_sym    ≈ 400 KB
TH CSR:               4 × (n_sym + n_edges)  ≈ 220 KB
AssociationRecord[]:  28 × n_assoc ≈ 140 KB  (5K associations)
QualNameTable:        ≈ 450 KB
──────────────────────────────────────────────────────────
Peak (including transient TypeRefResolution): ≈ 13.6 MB
Peak after freeing TypeRefResolution: ≈ 5.6 MB permanent
```

---

### 3.8 Phase 3 Invariants for Phase 4

Phase 4 (CFG Construction) reads the SymbolTableArtifact for two critical lookups: identifying which AST nodes are method definitions (to construct per-method CFGs) and resolving method call expressions to their target symbols (for call graph construction in Phase 6). The following invariants must hold:

**Invariant 1 (Method Completeness):** Every NN_METHOD_DECL and NN_CONSTRUCTOR_DECL node in the BP AST has a corresponding SK_METHOD or SK_CONSTRUCTOR SymbolRecord. Verified: `∀ pre_idx where bpa.node_type(pre_idx) ∈ {NN_METHOD_DECL, NN_CONSTRUCTOR_DECL} : ∃ sym_id, sym.decl_node(sym_id) = pre_idx`.

**Invariant 2 (Parent Chain Completeness):** Every symbol except package-level classes has a non-`u32::MAX` parent_sym. Every method/field symbol has a parent of kind SK_CLASS, SK_INTERFACE, SK_ENUM, or SK_RECORD. Verified by traversing the parent chain of every leaf symbol.

**Invariant 3 (Type Hierarchy Acyclicity):** E^TH restricted to TH_EXTENDS edges is acyclic. Verified in O(V + E) using Kahn's algorithm during CSR construction. A cycle would indicate a Java compiler error in the source code — Phase 3 reports it as a diagnostic rather than failing silently.

**Invariant 4 (BPA Hash Chain):** `sta_header.bpa_hash == crc64(bpa_file_bytes)` AND `sta_header.tca_hash == crc64(tca_file_bytes)`. The STA is valid only when consumed together with the exact same TCA and BPA files it was built from. Phase 4 validates both hashes before processing.

**Invariant 5 (Token Range Seed):** Every SymbolRecord with `first_token_id ≠ u32::MAX` has a corresponding entry in the TCA Token Table. This is guaranteed because Phase 1 assigned token_ids to all tokens, and Phase 3's Declaration Discovery extracts token ranges from the BP AST's `token_ranges` section which was populated by Phase 2's construction from Phase 1's TCA.

---

Now the visualization: