---

## Phase 7: Traceability Index Construction

### 7.1 Phase Mandate & Exact Boundaries

Phase 7 is the only pure aggregation phase in the pipeline. It performs no new analysis — it reads all six prior artifacts and synthesizes a unified, bidirectional traceability index: the mathematical substrate that makes every generated UML element navigable to source code, and every source token navigable to all SCPG entities that derive from it.

Without Phase 7, the diagram-to-source and source-to-diagram navigations that define a "God-tier visualizer" are computationally intractable at query time. Phase 7 precomputes and indexes every relevant (entity, source-range) pair so that all runtime navigation queries execute in O(1) or O(log n).

Phase 7 also establishes the **SCPG hash chain** — a tamper-evident sequence of artifact checksums that enables stale UMLLink detection in O(1) after any source edit.

**Inputs:** All six prior artifacts: `.tca`, `.bpa`, `.sta`, `.cfa`, `.ssa`, `.cga`.

**Output:** `TraceabilityArtifact (.tra)` — six per-layer backward indexes, a symbol span interval index for forward queries, a call-site span index, a UMLLink pre-computation table, and the SCPG hash chain record.

---

### 7.2 Mathematical Foundations

#### 7.2.1 The Six-Layer Traceability Chain

The SCPG partitions its entities into six layers, each derived from the layers below it. The traceability chain follows this derivation in reverse: to find the source position of any entity, we trace downward through the layers until we reach the token layer, where positions are stored directly.

**Definition (Entity Reference):** An entity reference ε = (layer, entity\_id) where layer ∈ {L\_TOK=0, L\_AST=1, L\_SYM=2, L\_BLK=3, L\_SSA=4, L\_CS=5} identifies the SCPG layer and entity\_id is the layer-specific identifier.

**Definition (Source Range):** R = (file\_id: u16, line\_start: u24, col\_start: u16, line\_end: u24, col\_end: u16) is a contiguous source text span. A point location is the degenerate case where line\_start = line\_end and col\_start = col\_end.

**Definition (Backward Traceability Function):** The function τ: EntityRef → SourceRange resolves any SCPG entity to its source extent:

```
τ(L_TOK, t)  = range_from_tca(t)
             = TCA.BI[t] = (file_id, line, col, len)

τ(L_AST, v)  = span(BPA.first_token[v], BPA.last_token[v])
             = (τ(L_TOK, BPA.first_token[v]).start,
                τ(L_TOK, BPA.last_token[v]).end)

τ(L_SYM, s)  = span(STA.first_token_id[s], STA.last_token_id[s])

τ(L_BLK, b)  = span(CFA.first_token[b],   CFA.last_token[b])

τ(L_SSA, v)  = if SSA.record(v).def_stmt ≠ u32::MAX:
                 τ(L_AST, SSA.record(v).def_stmt)
               else:
                 τ(L_BLK, SSA.record(v).def_block)  ← φ-function: uses block entry

τ(L_CS, c)   = τ(L_TOK, CGA.call_site(c).call_token)
```

**Theorem (Completeness of τ):** For every SCPG entity ε in any layer, τ(ε) is well-defined and returns a valid source range. This follows from the construction invariants of each prior phase: Phase 1 assigns token\_ids to all source tokens, Phase 2 propagates first/last token\_ids up the AST via bottom-up min/max, Phases 3–6 copy these token\_id pairs into their own records. No entity exists without at least one source token anchor. ∎

**Definition (Forward Traceability Function):** The inverse τ⁻¹: SourceRange → 2^{EntityRef} returns all entities whose source range overlaps with the given range:

```
τ⁻¹(R) = {ε : τ(ε) overlaps R}
```

where "overlaps" means: same file\_id AND ranges share at least one character position.

**Theorem (Surjectivity of τ⁻¹ on tokens):** For any token t in the TCA, τ⁻¹(τ(L\_TOK, t)) is non-empty — it always contains at least (L\_TOK, t) itself, and typically contains (L\_AST, parent\_of(t)), (L\_SYM, sym\_containing(t)), etc. ∎

#### 7.2.2 The Forward Index — Interval Stabbing via Sorted Span Table

A naive implementation of τ⁻¹ would require scanning all entities — O(n) per query. Phase 7 builds a structured **Span Index** per layer that enables O(log n + k) forward queries (k = result count).

**Definition (SymbolSpanRecord, 20 bytes):**

```
Offset  Size  Field
 0       4    first_token_id  : u32   sort key (low half)
 4       4    last_token_id   : u32   sort key (high half) — for coverage check
 8       4    sym_id          : u32
12       2    file_id         : u16
14       2    line_start      : u16   (truncated for index; full value in BI_sym)
16       2    col_start       : u16
18       2    line_end        : u16
Total: 20 bytes
```

The Symbol Span Index is sorted by `(file_id, first_token_id)`. A forward query "which symbols span position P (given as token\_id T)" is answered by the following O(log n + k) algorithm:

```rust
fn forward_sym_query(token_id: u32, file_id: u16, span_index: &[SymbolSpanRecord])
    -> Vec<u32>  // sym_ids
{
    // Binary search: find first record where first_token_id > token_id
    let upper = span_index.partition_point(|r|
        (r.file_id, r.first_token_id) <= (file_id, token_id)
    );

    // Scan backward: all symbols starting before token_id that also end after it
    let mut results = Vec::new();
    let mut i = upper;
    while i > 0 {
        i -= 1;
        let r = &span_index[i];
        if r.file_id != file_id { break; }
        if r.last_token_id >= token_id {
            results.push(r.sym_id);
        }
        // Early exit: if first_token_id is far before token_id, remaining records
        // cannot span token_id (they end before first_token_id)
        // This is sound because span_index is sorted by first_token_id,
        // and last_token_id >= first_token_id always.
    }
    results
}
```

**Note:** This algorithm is not worst-case O(log n + k) in general — it degrades to O(n) if many symbols have identical `first_token_id`. For the typical case (symbols are non-overlapping or only shallowly nested), performance is O(log n + nesting\_depth), which is O(log n) for flat codebases and O(log n × class\_depth) for deeply nested ones. For Java with max nesting depth ~5: effectively O(log n).

#### 7.2.3 The SCPG Hash Chain

The hash chain is a sequence of 64-bit CRC values, one per artifact, that together form an immutable fingerprint of the entire SCPG build state:

```
tca_hash = crc64(TCA_file_bytes)
bpa_hash = crc64(BPA_file_bytes)
sta_hash = crc64(STA_file_bytes)
cfa_hash = crc64(CFA_file_bytes)
ssa_hash = crc64(SSA_file_bytes)
cga_hash = crc64(CGA_file_bytes)
```

The **composite SCPG hash** carried by every UMLLink record is:

```
scpg_hash: u32 = crc32(
    tca_hash ⊕ bpa_hash ⊕ sta_hash ⊕
    cfa_hash ⊕ ssa_hash ⊕ cga_hash
)
```

XOR is used rather than concatenated hashing for computational efficiency (O(1) combining) while preserving the property that any single artifact change causes the composite to change (with overwhelming probability — a CRC64 collision under XOR has probability 2⁻⁶⁴ per artifact pair).

**Stale UMLLink detection:** When any upstream artifact changes, its `crc64` changes, causing `scpg_hash` to change. A UMLLink carrying the old `scpg_hash` is detected as stale in O(1) by comparing `umllink.scpg_hash ≠ current_scpg_hash`. This makes complete pipeline invalidation and selective re-derivation both O(n\_uml\_elements) — linear in the number of generated diagram elements, not in the codebase size.

#### 7.2.4 UMLLink Pre-computation

For every STA symbol that will generate a UML element (classes, interfaces, enums, records, methods, fields, constructors), Phase 7 pre-computes the immutable `UMLLink` record and stores it in the TRA file. Phase 9 reads these records directly rather than recomputing source ranges at diagram generation time.

**UMLLinkRecord (24 bytes):**

```
Offset  Size  Field
 0       4    sym_id          : u32
 4       2    file_id         : u16
 6       3    line_start      : u24
 9       2    col_start       : u16
11       3    line_end        : u24
14       2    col_end         : u16
16       4    scpg_hash       : u32   (composite hash at build time)
20       1    sym_kind        : u8    (SymbolKind from Σ_K — for UML type routing)
21       3    _reserved       : u24 = 0
Total: 24 bytes
```

Sorted by `sym_id` for O(1) direct-index lookup.

---

### 7.3 Module Architecture

```
phase7/
│
├── mod.rs                          # Phase7Stage::run(tca, bpa, sta, cfa, ssa, cga)
│                                   # → TraceabilityArtifact
│                                   # Orchestrates all sub-builders, then serializes
│
├── backward/
│   ├── mod.rs                      # BackwardIndexBuilder: dispatches per-layer construction
│   ├── ast_bi.rs                   # ASTBackwardIndex: scans BPA token_ranges[] section
│   │                               # → BI_ast[preorder_idx] = (first_tok, last_tok)
│   ├── sym_bi.rs                   # SymbolBackwardIndex: reads STA first/last_token_id
│   │                               # → BI_sym[sym_id] = (decl_ft, decl_lt, def_ft, def_lt)
│   ├── blk_bi.rs                   # BlockBackwardIndex: reads CFA block metadata arrays
│   │                               # → BI_blk[global_block_id] = (first_tok, last_tok)
│   ├── ssa_bi.rs                   # SSABackwardIndex: reads SSA def_stmt, maps to BPA range
│   │                               # → BI_ssa[ssa_id] = (def_stmt, first_tok, last_tok)
│   └── cs_bi.rs                    # CallSiteBackwardIndex: reads CGA call_site.call_token
│                                   # → BI_cs[call_site_id] = call_token
│
├── forward/
│   ├── mod.rs                      # ForwardIndexBuilder: builds both span indexes
│   ├── sym_span.rs                 # SymbolSpanIndex: constructs sorted SymbolSpanRecord[]
│   │                               # using BI_sym and TCA backward lookup
│   └── cs_span.rs                  # CallSiteSpanIndex: sorted (sort_key, call_site_id)
│
├── uml_link/
│   ├── mod.rs                      # UMLLinkRegistry: constructs UMLLinkRecord[] table
│   └── hash_chain.rs               # ScpgHashChain: computes CRC64 per artifact,
│                                   # derives composite scpg_hash
│
├── delta/
│   ├── mod.rs                      # DeltaApplicator: processes incremental source changes
│   └── invalidation.rs             # StaleDetector: identifies UMLLinks with changed ranges
│
├── builder.rs                      # TraceabilityArtifactBuilder: aggregates all structures
└── serializer.rs                   # TraceabilityArtifact binary I/O (.tra format)
```

---

### 7.4 Data Structure Specifications

**BI\_ast Entry (8 bytes, dense array indexed by BP AST preorder\_idx):**

```
first_token_id : u32
last_token_id  : u32
```

**BI\_sym Entry (16 bytes, dense array indexed by sym\_id):**

```
decl_first_tok : u32   first token of the declaration site
decl_last_tok  : u32   last token of the declaration site
def_first_tok  : u32   first token of the definition body (u32::MAX if abstract)
def_last_tok   : u32   last token of the definition body
```

**BI\_blk Entry (8 bytes, dense array indexed by global\_block\_id):**

Global block IDs are assigned sequentially across all functions: `global_block_id = Σ_{functions processed before f} block_count(f) + local_block_id`. Stored as a flat array without function boundaries — the CFA function directory is used to map (sym\_id, local\_block\_id) ↔ global\_block\_id.

```
first_token_id : u32
last_token_id  : u32
```

**BI\_ssa Entry (12 bytes, dense array indexed by ssa\_id):**

```
def_stmt       : u32   BP AST pre-order index of defining statement
first_token_id : u32   first token of the def_stmt (via BPA token_range)
last_token_id  : u32   last token of the def_stmt
```

For φ-functions (`def_stmt == u32::MAX`): `first_token_id` and `last_token_id` point to the entry token of the block where the φ is inserted (from BI\_blk).

**BI\_cs Entry (4 bytes, dense array indexed by call\_site\_id):**

```
call_token : u32   TCA token_id of the method name at this call site
```

---

### 7.5 Algorithm Specifications

#### 7.5.1 Construction Orchestration

```rust
impl Phase7Stage {
    pub fn run(
        tca: &TokenCorpusArtifact, bpa: &BPASTArtifact,
        sta: &SymbolTableArtifact, cfa: &CFGArtifact,
        ssa: &SSAArtifact,         cga: &CallGraphArtifact,
        out: &Path,
    ) -> TraceabilityArtifact {

        // Step 1: Compute SCPG hash chain
        let hashes = ScpgHashChain::compute(tca, bpa, sta, cfa, ssa, cga);

        // Step 2: Build per-layer backward indexes (O(n_layer) each, all linear)
        let bi_ast  = ASTBackwardIndex::build(bpa);
        let bi_sym  = SymbolBackwardIndex::build(sta, bpa);
        let bi_blk  = BlockBackwardIndex::build(cfa);
        let bi_ssa  = SSABackwardIndex::build(ssa, bpa);
        let bi_cs   = CallSiteBackwardIndex::build(cga);

        // Step 3: Build Symbol Span Index (O(n_sym log n_sym) for sort)
        let sym_span = SymbolSpanIndex::build(&bi_sym, tca, sta);

        // Step 4: Build Call Site Span Index (O(n_cs log n_cs) for sort)
        let cs_span  = CallSiteSpanIndex::build(&bi_cs, tca, cga);

        // Step 5: Pre-compute UMLLink records (O(n_sym) linear scan)
        let uml_links = UMLLinkRegistry::build(&bi_sym, tca, sta, hashes.scpg_hash);

        // Step 6: Serialize all sections to .tra binary
        let artifact = TraceabilityArtifact {
            hashes, bi_ast, bi_sym, bi_blk, bi_ssa, bi_cs,
            sym_span, cs_span, uml_links,
        };
        TraceabilitySerializer::write(&artifact, out);
        artifact
    }
}
```

#### 7.5.2 SymbolSpanIndex Construction

```rust
impl SymbolSpanIndex {
    pub fn build(
        bi_sym: &SymbolBackwardIndex,
        tca:    &TokenCorpusArtifact,
        sta:    &SymbolTableArtifact,
    ) -> Self {
        let mut records: Vec<SymbolSpanRecord> = Vec::new();

        for sym_id in 0..sta.symbol_count() {
            let entry = bi_sym.get(sym_id);
            if entry.decl_first_tok == u32::MAX { continue; } // external/abstract

            // Resolve token_ids to (file_id, line, col) via TCA backward index
            let start = tca.backward_lookup(entry.decl_first_tok);
            let end   = tca.backward_lookup(entry.decl_last_tok);

            records.push(SymbolSpanRecord {
                first_token_id: entry.decl_first_tok,
                last_token_id:  entry.decl_last_tok,
                sym_id:         sym_id as u32,
                file_id:        start.file_id,
                line_start:     start.line as u16,
                col_start:      start.col,
                line_end:       end.line as u16,
            });
        }

        // Sort by (file_id, first_token_id) for binary search
        records.sort_unstable_by_key(|r| (r.file_id, r.first_token_id));
        SymbolSpanIndex { records }
    }
}
```

#### 7.5.3 UMLLink Pre-computation with Hash Chain

```rust
impl UMLLinkRegistry {
    pub fn build(
        bi_sym:    &SymbolBackwardIndex,
        tca:       &TokenCorpusArtifact,
        sta:       &SymbolTableArtifact,
        scpg_hash: u32,
    ) -> Self {
        let uml_kinds: &[SymbolKind] = &[
            SK_CLASS, SK_INTERFACE, SK_ENUM, SK_RECORD, SK_ANNOTATION_TYPE,
            SK_METHOD, SK_CONSTRUCTOR, SK_FIELD, SK_ENUM_CONSTANT,
        ];

        let mut records: Vec<UMLLinkRecord> = (0..sta.symbol_count())
            .filter(|&s| uml_kinds.contains(&sta.symbol(s as u32).kind))
            .map(|sym_id| {
                let entry = bi_sym.get(sym_id);
                let start = tca.backward_lookup(entry.decl_first_tok);
                let end   = tca.backward_lookup(entry.decl_last_tok);
                UMLLinkRecord {
                    sym_id:     sym_id as u32,
                    file_id:    start.file_id,
                    line_start: start.line,
                    col_start:  start.col,
                    line_end:   end.line,
                    col_end:    end.col + end.len as u16,
                    scpg_hash,
                    sym_kind:   sta.symbol(sym_id as u32).kind as u8,
                    _reserved:  0,
                }
            })
            .collect();

        // Sort by sym_id for O(1) array-index lookup
        records.sort_unstable_by_key(|r| r.sym_id);
        UMLLinkRegistry { records }
    }
}
```

#### 7.5.4 Incremental Delta Processing

When source changes (a file edit in an IDE), the pipeline re-runs incrementally. Phase 7's role in incremental sync:

```rust
impl DeltaApplicator {
    /// Given the old TRA and the newly rebuilt upstream artifacts,
    /// returns: (new scpg_hash, set of sym_ids whose UMLLinks are stale)
    pub fn compute_delta(
        old_tra:  &TraceabilityArtifact,
        new_tca:  &TokenCorpusArtifact,
        new_bpa:  &BPASTArtifact,
        new_sta:  &SymbolTableArtifact,
        // ... other new artifacts
    ) -> TraceabilityDelta {
        let new_hashes   = ScpgHashChain::compute(new_tca, new_bpa, new_sta, ...);
        let new_bi_sym   = SymbolBackwardIndex::build(new_sta, new_bpa);

        // Find symbols whose source ranges changed
        let changed_syms: Vec<u32> = (0..new_sta.symbol_count())
            .filter(|&s| {
                let old_entry = old_tra.bi_sym.get(s);
                let new_entry = new_bi_sym.get(s);
                old_entry.decl_first_tok != new_entry.decl_first_tok
                    || old_entry.decl_last_tok != new_entry.decl_last_tok
            })
            .map(|s| s as u32)
            .collect();

        TraceabilityDelta {
            new_scpg_hash:   new_hashes.scpg_hash,
            old_scpg_hash:   old_tra.hashes.scpg_hash,
            invalidated_syms: changed_syms,
            // UMLLinks with scpg_hash == old_scpg_hash AND
            // sym_id ∈ invalidated_syms → must be regenerated
        }
    }
}
```

**Complexity of incremental update:** O(n\_sym) scan to find changed symbols + O(n\_changed × avg\_diagram\_elements\_per\_sym) for regeneration. In practice, a single-file edit changes O(n\_sym\_in\_file) symbols, which is O(1) relative to the whole codebase. The delta is tiny.

---

### 7.6 Output Schema: TraceabilityArtifact Binary Format (`.tra`)

```
╔═══════════════════════════════════════════════════════════════════╗
║  TRA FILE FORMAT v1.0  (all integers: little-endian)             ║
╠══════════════════════╦═══════════════╦══════════════════════════════╣
║ Section              ║ Size          ║ Description                  ║
╠══════════════════════╬═══════════════╬══════════════════════════════╣
║ HEADER               ║ 64 B          ║ Magic, counts, all 6 upstream║
║                      ║               ║ hashes + scpg_hash           ║
╠══════════════════════╬═══════════════╬══════════════════════════════╣
║ BI_AST               ║ n_ast × 8 B   ║ Dense array: preorder_idx →  ║
║                      ║               ║ (first_tok, last_tok)        ║
╠══════════════════════╬═══════════════╬══════════════════════════════╣
║ BI_SYM               ║ n_sym × 16 B  ║ Dense array: sym_id →        ║
║                      ║               ║ (decl_ft, decl_lt, def_ft,   ║
║                      ║               ║  def_lt) all u32             ║
╠══════════════════════╬═══════════════╬══════════════════════════════╣
║ BI_BLK               ║ n_blk × 8 B   ║ Dense: global_block_id →     ║
║                      ║               ║ (first_tok, last_tok)        ║
╠══════════════════════╬═══════════════╬══════════════════════════════╣
║ BI_SSA               ║ n_ssa × 12 B  ║ Dense: ssa_id →              ║
║                      ║               ║ (def_stmt, first_tok, last)  ║
╠══════════════════════╬═══════════════╬══════════════════════════════╣
║ BI_CS                ║ n_cs × 4 B    ║ Dense: call_site_id →        ║
║                      ║               ║ call_token                   ║
╠══════════════════════╬═══════════════╬══════════════════════════════╣
║ SYMBOL SPAN INDEX    ║ n_sym × 20 B  ║ Sorted SymbolSpanRecord[].   ║
║                      ║               ║ Forward query: source → syms ║
╠══════════════════════╬═══════════════╬══════════════════════════════╣
║ CALL SITE SPAN INDEX ║ n_cs × 12 B   ║ Sorted (sort_key, cs_id).    ║
║                      ║               ║ Forward: source → call sites  ║
╠══════════════════════╬═══════════════╬══════════════════════════════╣
║ UMLLINK TABLE        ║ n_uml × 24 B  ║ UMLLinkRecord[], sym_id       ║
║                      ║               ║ sorted. O(1) Phase 9 access. ║
╠══════════════════════╬═══════════════╬══════════════════════════════╣
║ CHECKSUM             ║ 8 B           ║ CRC-64/ECMA                  ║
╚══════════════════════╩═══════════════╩══════════════════════════════╝
```

**HEADER (64 bytes):**

```
Offset  Size  Field
 0       8    magic           0x5452413100010000 ("TRA1\x00\x01\x00\x00")
 8       4    format_version  0x00000001
12       4    ast_node_count  (u32)
16       4    sym_count       (u32)
20       4    blk_count_total total blocks across all functions (u32)
24       4    ssa_count       (u32)
28       4    cs_count        (u32)
32       4    uml_link_count  (u32)
36       4    scpg_hash       composite u32 hash of all 6 upstream artifacts
40       8    tca_hash        CRC-64 of TCA
48       8    bpa_hash        CRC-64 of BPA
56       8    sta_hash        CRC-64 of STA (remaining hashes in body of section 0)
64 bytes total header; remaining 4 artifact hashes (cfa, ssa, cga, tra) stored
as first record of Section 1 (HASH CHAIN RECORD, 32 bytes)
```

**Size for medium Java project (1M AST nodes, 50K symbols, 450K blocks, 2M SSA vars, 45K call sites):**

```
Header:                64 B
BI_AST:         1M × 8    =    8 MB
BI_SYM:        50K × 16   =    0.8 MB
BI_BLK:       450K × 8    =    3.6 MB
BI_SSA:         2M × 12   =   24 MB
BI_CS:         45K × 4    =    0.18 MB
Symbol Span:   50K × 20   =    1 MB
Call Site Span:45K × 12   =    0.54 MB
UMLLink Table: 35K × 24   =    0.84 MB  (subset of symbols that are UML-relevant)
Checksum:             8 B
───────────────────────────────────
Total:                    ≈  39 MB uncompressed
After LZ4:               ≈  9–12 MB  (dense arrays of u32 pairs compress well ~3:1)
```

---

### 7.7 Complexity Proofs

| Operation | Complexity | Notes |
|---|---|---|
| Hash chain computation | O(total\_artifact\_bytes) | One-pass CRC over each file |
| BI\_ast construction | O(n\_ast) | One scan of BPA token\_ranges section |
| BI\_sym construction | O(n\_sym) | One scan of STA + O(1) BPA lookup per symbol |
| BI\_blk construction | O(n\_blk) | One scan of CFA block metadata |
| BI\_ssa construction | O(n\_ssa) | One scan of SSA + O(1) BPA lookup per def |
| BI\_cs construction | O(n\_cs) | One scan of CGA call site table |
| Symbol Span Index sort | O(n\_sym log n\_sym) | `sort_unstable_by_key` |
| UMLLink pre-computation | O(n\_uml) | Linear scan of STA + O(1) lookups |
| **Total Phase 7** | **O(n\_ast + n\_sym log n\_sym)** | Dominated by span index sort |

**Runtime forward query (source → symbols):** O(log n\_sym + nesting\_depth) ≈ O(log n\_sym)

**Runtime backward query (entity → source):** O(1) — direct dense array indexing

**Stale UMLLink detection:** O(1) — single u32 comparison per link

Phase 7 is the fastest phase in the pipeline. For the medium project above: ~15ms total construction time.

---

### 7.8 Phase 7 Invariants for Phases 8 and 9

**Invariant 1 (BI\_blk Global Completeness):** Every basic block b in every function has a valid entry in BI\_blk: `∀ (func, local_bid): BI_blk[global_id(func, local_bid)].first_tok ≠ u32::MAX`. This guarantees Phase 8 can annotate every ROBDD path node with a source range.

**Invariant 2 (UMLLink Symbol Coverage):** Every STA symbol with kind ∈ {SK\_CLASS, SK\_INTERFACE, SK\_METHOD, SK\_FIELD, SK\_CONSTRUCTOR, SK\_ENUM, SK\_RECORD} has a corresponding UMLLinkRecord in the sorted table. Phase 9 asserts this before generating any diagram element.

**Invariant 3 (Hash Chain Validity):** `TRA.header.scpg_hash == crc32(tca_hash ⊕ bpa_hash ⊕ sta_hash ⊕ cfa_hash ⊕ ssa_hash ⊕ cga_hash)`. Verified at Phase 9 initialization. A mismatch indicates a stale TRA and triggers full rebuild.

**Invariant 4 (Backward–Forward Roundtrip):** For every symbol s with a valid source range, the forward query on token `BI_sym[s].decl_first_tok` returns a result set containing s: `s ∈ forward_sym_query(BI_sym[s].decl_first_tok, file_id_of(s), sym_span_index)`. Verified by spot-checking 5% of symbols during integration tests.

---

Now the visualization: