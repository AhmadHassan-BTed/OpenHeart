---

## Phase 10: SCPG Serialization & Query Engine Bootstrap

> **Status: design specification.** This plan describes the target design of Phase 10. Much of it was not implemented at the audited baseline `fc218cd`: the `.scpg` sections other than the string table hold element counts only, the loader reads the whole file instead of mapping it, the incremental engine is a stub, "CFL-reachability" is plain call-graph reachability, the XMI exporter is a placeholder, and no LSP bridge or file watcher exists. Implementation notes below mark each gap; the authoritative status is [docs/status.md](../status.md). Every size and latency figure in this plan is a design estimate that has never been measured.

### 10.1 Phase Mandate

Phase 10 is the terminus of the OpenHeart pipeline. It does three things simultaneously: it merges all nine prior binary artifacts into a single, memory-mapped `.scpg` file; it bootstraps the demand-driven query engine that answers arbitrary SCPG queries without rerunning any analysis; and it exposes the production public API through which IDEs, CI pipelines, and visualization tools generate all 14 UML diagram types and navigate bidirectionally between source and diagram elements.

In the target design, no further pipeline runs are needed after Phase 10 except when source code changes, and then only the phases affected by the delta re-execute. (Not implemented: every run recomputes all ten phases.)

---

### 10.2 The Final SCPG Binary Format

The `.scpg` binary is a single memory-mapped file with 11 typed sections. Sections are ordered by access frequency — hot sections appear first so the OS page cache keeps them resident automatically.

**Header (128 bytes, fixed):**

```
Offset   Size  Field
  0        8   magic           bytes 53 43 50 47 00 01 00 00  ("SCPG\0\x01\0\0")
  8        4   format_version  SCPG_FORMAT_VERSION (src/scpg/types.rs)
 12        4   section_count   11
 16       32   source_hash     SHA-256 of sorted(SHA-256(each_source_file))
 48        8   creation_ts_ns  Unix nanoseconds
 56        4   scpg_hash       crc32(tca∧bpa∧sta∧cfa∧ssa∧cga∧tra∧uma∧psa hashes XOR'd)
 60        4   language_count  number of distinct source languages ingested
 64       64   _reserved       zeroed
Total: 128 bytes
```

**Magic and versioning.** The magic is the eight bytes `53 43 50 47 00 01 00 00` exactly as they appear on disk. Read as a little-endian `u64` (`SCPG_MAGIC` in `src/scpg/types.rs`) they are `0x0000010047504353`; the big-endian literal `0x5343504700010000` that an earlier version of this table showed is the same bytes read the other way. The bytes after `SCPG` are a fixed part of the magic, **not** a version number: the format version lives only in `format_version`, and a reader must check both. Error messages should print the magic as bytes, not as an integer.

**`source_hash`.** Defined as SHA-256 over the sorted `(relative path, content_sha256)` pairs of the ingested files, so that a stale `.scpg` can be detected by recomputing it from the sources. *Implementation:* the writer currently stores 32 zero bytes, so staleness cannot be detected from the header.

**`creation_ts_ns`.** A wall-clock timestamp makes otherwise identical builds differ byte for byte. It must be excluded from any hashed or compared bytes, and builds should honour `SOURCE_DATE_EPOCH`.

**Section Directory (11 × 24 bytes = 264 bytes):**

```
For each section:
  section_type:  u32   (0x01..0x0B)
  byte_offset:   u64   (absolute offset in file)
  byte_length:   u64
  crc32:         u32   (integrity check of that section's bytes)
```

**The 11 Sections (ordered hot → cold):**

```
Access   Code    Source    Contents
────────────────────────────────────────────────────────────────────────────────
HOT      0x01   TCA       Token Table:      sorted TokenRecord[n_tok], 16B each
HOT      0x02   TCA       String Table:     StringInternHeader[] + string storage
HOT      0x09   TRA       Traceability:     BI_ast, BI_sym, BI_blk, BI_ssa, BI_cs
                                             + Symbol Span Index + UMLLink Table
HOT      0x04   STA       Symbol Table:     SymbolRecord[n_sym], 64B each
HOT      0x08   STA       Type Hierarchy:   CSR offsets+adj (+ wavelet tree Σ_TH: planned)
────────────────────────────────────────────────────────────────────────────────
WARM     0x0A   UMA       Semantic Meta:    All 14 diagram type records + labels
WARM     0x07   CGA       Call Graph:       Call site table + callee/caller CSR
                                             + SCC table + points-to table
────────────────────────────────────────────────────────────────────────────────
COLD     0x03   BPA       BP AST:           BP bitstring + rank/select + jump table
                                             + preorder arrays + RMQ sparse table
COLD     0x05   CFA       CFG:              Per-function CSR + idom[] + DF CSR
                                             + block metadata + exception table
COLD     0x06   SSA       SSA/DFG:          SSA variable table + φ-table
                                             + def-use CSR + CDG + fact marks (IFDS planned)
COLD     0x0B   PSA       Path Summaries:   Per-function ROBDD node arrays
                                             + variable orderings + path metrics
                                             (lazy-loaded per function on demand)
```

> **Implementation (audited baseline `fc218cd`):** today only the String Table section carries data; the other ten sections each hold a single element count, and no section deserializer exists. The checksum scheme for the header, directory and sections is defined in `src/scpg/serializer/integrity.rs`.

**Design estimate of the SCPG binary size (500K LOC Java project; never measured — no corpus, script or LZ4 dependency exists):**

```
Hot  sections: Token Table (16 MB) + String Table (5 MB)
             + Traceability (39 MB) + Symbol Table (3 MB) + Type Hierarchy (80 KB)
             ≈ 63 MB on disk  — OS keeps resident after first diagram query

Warm sections: Semantic Metadata (15 MB) + Call Graph (7.6 MB)
             ≈ 22.6 MB

Cold sections: BP AST (45 MB) + CFG (27 MB) + SSA/DFG (55 MB) + Path Summaries (92 MB)
             ≈ 219 MB lazy-loaded, ~10 MB typical working set

────────────────────────────────────────────────────────────────────────────────
Total uncompressed:           ≈ 305 MB
After per-section LZ4 HC:    ≈  82 MB
Typical RAM resident:        ≈  12 MB  (hot + accessed warm sections)
```

---

### 10.3 Mathematical Foundations: CFL-Reachability

The query engine's inter-procedural path queries are answered by CFL-reachability over the call graph. This is the most mathematically powerful query in the system — it answers "is there a valid, balanced-call-return path from method A to method B?" in polynomial time.

**Definition (Program Supergraph G\*):** Extend the call graph G\_CG with:
- For each call site c (method m calls method g): add a **call edge** `(m, g)` labeled `(`g`
- For each return: add a **return edge** `(g, m_post)` labeled `)`g` (matching the opening bracket)
- All intra-procedural CFG edges are labeled `ε`

**Definition (Same-level valid path):** A path π in G\* is same-level-valid (SLV) iff its edge label sequence is a word in the context-free language:
`L = {w : w is a properly balanced sequence of (f and )f brackets, possibly interleaved with ε}`

Equivalently: L is the Dyck language over the bracket alphabet {(f, )f | f ∈ V\_CG}.

**Theorem (CFL-Reachability):** Node t is CFL-reachable from node s in G\* iff there exists an SLV path from s to t. This is solvable in O(|V|³) via the cubic tabulation algorithm. ∎

**Implementation (worklist-based tabulation):**

```rust
fn cfl_reachable(source: u32, target: u32, scpg: &SCPG) -> bool {
    // summary_edges: (s, e) means there is an SLV path from source to e
    //                starting at source (within the procedure rooted at source)
    let mut summary: HashSet<(u32, u32)> = HashSet::new();
    let mut work: VecDeque<(u32, u32)>   = VecDeque::new();

    // Base: source can reach itself
    summary.insert((source, source));
    work.push_back((source, source));

    while let Some((s, u)) = work.pop_front() {
        // ① ε-edges (intra-procedural CFG successors)
        for v in scpg.cfg_successors(u) {
            if summary.insert((s, v)) { work.push_back((s, v)); }
        }

        // ② Call edges: (s, u) + call(u → callee_entry) →
        //               compute callee summary, propagate to post-call node
        for (callee_entry, callee_exit, post_call) in scpg.call_sites_at(u) {
            // If callee has a summary (entry→exit path), compose it
            if summary.contains(&(callee_entry, callee_exit)) {
                if summary.insert((s, post_call)) {
                    work.push_back((s, post_call));
                }
            }
        }

        // ③ Return edges: filled by recursive calls finding callee summaries
        //    When summary(callee_entry, callee_exit) is first discovered,
        //    propagate through all call sites that called callee
        for (caller_u, post_call) in scpg.call_sites_to(u) {
            if summary.contains(&(s, u)) {
                if summary.insert((s, post_call)) {
                    work.push_back((s, post_call));
                }
            }
        }
    }

    summary.contains(&(source, target))
}
```

Time: with a fixed source the tabulation visits each pair `(source, u)` at most once, O(|V|²) pairs in general. No latency has been measured.

> **Implementation (audited baseline `fc218cd`):** `src/scpg/query/cfl.rs` does not implement this algorithm. It has no call/return labels, no summary edges and no ICFG, so it computes plain breadth-first reachability over call edges, and it scans the whole call-site map for every dequeued node (O(V·E)). Real CFL-reachability requires IFDS-style tabulation over an ICFG with summary edges (Reps–Horwitz–Sagiv 1995; Reps 1998). For plain method reachability, CSR adjacency plus SCC condensation is the appropriate structure.

---

### 10.4 Module Architecture

```
phase10/
├── mod.rs                          # Phase10Stage::run(all 9 artifacts) → OpenHeartEngine
│
├── serializer/
│   ├── mod.rs                      # SCPGSerializer: merges 9 artifacts → .scpg binary
│   ├── section_writer.rs           # Writes each section: reads from source artifact
│   │                               # mmap + streaming copy + CRC-32 per section
│   ├── layout.rs                   # SectionLayoutPlanner: determines byte offsets
│   │                               # for hot/warm/cold ordering
│   └── integrity.rs                # IntegrityVerifier: SHA-256 source hash + per-section CRC
│
├── mmap/
│   ├── mod.rs                      # MemoryMappedSCPG: read-only mmap of the .scpg file
│   ├── section.rs                  # SectionAccessor: typed accessor per section (0x01..0x0B)
│   └── lazy.rs                     # LazySection: demand-loaded cold sections with page tracking
│
├── query/
│   ├── mod.rs                      # QueryEngine: dispatches all query types
│   ├── cache.rs                    # LRUQueryCache: HashMap<QueryKey, QueryResult>
│   │                               # keyed by (query_type, params_hash, scpg_hash)
│   │                               # capacity: 512 entries, eviction: LRU
│   ├── cfl.rs                      # CFLReachability: inter-procedural path queries O(V²)
│   ├── robdd.rs                    # ROBDDQueryEngine: path count, coverage, complexity
│   ├── slice.rs                    # SliceEngine: forward/backward via CDG + def-use CSR
│   ├── navigation.rs               # NavigationEngine: entity↔source via TRA, O(1)/O(log n)
│   └── impact.rs                   # ImpactAnalyzer: change impact via call graph + TH
│
├── diagram/
│   ├── mod.rs                      # DiagramGenerator: public UML generation interface
│   ├── renderer/                   # Per-diagram-type rendering from UMA records
│   │   ├── class.rs                # ClassDiagramRenderer → ClassDiagram
│   │   ├── activity.rs             # ActivityDiagramRenderer → ActivityDiagram
│   │   ├── state_machine.rs        # StateMachineRenderer → StateMachine
│   │   ├── sequence.rs             # SequenceDiagramRenderer → SequenceDiagram
│   │   ├── package.rs              # PackageDiagramRenderer → PackageDiagram
│   │   ├── component.rs            # ComponentDiagramRenderer → ComponentDiagram
│   │   ├── object.rs               # ObjectDiagramRenderer → ObjectDiagram
│   │   ├── composite.rs            # CompositeDiagramRenderer → CompositeStructureDiagram
│   │   ├── use_case.rs             # UseCaseRenderer → UseCaseDiagram
│   │   ├── communication.rs        # CommunicationRenderer → CommunicationDiagram
│   │   ├── interaction.rs          # InteractionOverviewRenderer → InteractionOverview
│   │   └── timing.rs               # TimingDiagramRenderer → TimingDiagram
│   └── export/
│       ├── xmi.rs                  # XMI/UML 2.x export (Eclipse-compatible)
│       ├── plantuml.rs             # PlantUML text export
│       └── json.rs                 # JSON export for web visualizers
│
├── incremental/
│   ├── mod.rs                      # IncrementalEngine: manages source-change lifecycle
│   ├── delta.rs                    # SourceDelta: (file_id, byte_range, new_content)
│   ├── planner.rs                  # RebuildPlanner: determines which phases re-execute
│   │                               # given a set of changed files
│   ├── runner.rs                   # PartialPipelineRunner: re-runs only affected phases
│   └── merger.rs                   # SCPGMerger: applies artifact deltas to .scpg in-place
│                                   # using copy-on-write semantics per modified section
│
├── api/
│   ├── mod.rs                      # OpenHeartEngine: the production public API struct
│   ├── builder.rs                  # EngineBuilder: fluent builder for configuration
│   ├── lsp.rs                      # LSPBridge: Language Server Protocol adapter
│   │                               # Exposes textDocument/definition, references,
│   │                               # documentSymbol, hover for IDE integration
│   └── cli.rs                      # CLIInterface: `openheart generate class-diagram ...`
│
└── engine.rs                       # Final assembly: binds SCPG + QueryEngine + DiagramGenerator
                                    # + IncrementalEngine + API layer into OpenHeartEngine
```

---

### 10.5 Data Structure Specifications

**LRU Query Cache:**

```rust
pub struct LRUQueryCache {
    capacity:   usize,
    current_hash: u32,                              // scpg_hash at last cache fill
    entries:    LinkedHashMap<QueryKey, QueryResult>, // ordered: front=most-recently-used
}

pub struct QueryKey {
    query_type: u8,      // enum: CFL=0, ROBDD=1, SLICE_FWD=2, SLICE_BWD=3,
                         //       NAVIGATE_TO=4, NAVIGATE_FROM=5, DIAGRAM=6,
                         //       COVERAGE=7, IMPACT=8
    params_crc: u64,     // FNV-1a hash of serialized query parameters
    scpg_hash:  u32,     // composite SCPG hash at query time
}

impl LRUQueryCache {
    pub fn get(&mut self, key: &QueryKey) -> Option<&QueryResult> {
        // Invalidate entire cache if SCPG changed since last fill
        if key.scpg_hash != self.current_hash {
            self.entries.clear();
            self.current_hash = key.scpg_hash;
            return None;
        }
        // Move to front (most-recently-used)
        self.entries.get_refresh(key)
    }

    pub fn put(&mut self, key: QueryKey, result: QueryResult) {
        if self.entries.len() >= self.capacity {
            self.entries.pop_back(); // evict LRU entry
        }
        self.current_hash = key.scpg_hash;
        self.entries.insert(key, result);
    }
}
```

Cache capacity: 512 entries. Each QueryResult is bounded (diagrams are bounded by n\_sym, paths by depth limit). Total cache RAM: typically 10–50 MB for large sessions.

**Query complexity guarantees (all cached after first execution):**

```
Query                          First-run complexity     Cached complexity
──────────────────────────────────────────────────────────────────────────
navigate_to_source(entity)     O(1)  BI dense array      O(1)
navigate_from_source(pos)      O(log n_sym)  FI binary   O(1)
cyclomatic(method_sym)         O(1)  PSA metrics table   O(1)
path_count(method_sym)         O(1)  PSA sat_count       O(1)
class_diagram(config)          O(n_sym)  UMA ClassRec[]  O(1)
activity_diagram(method_sym)   O(n_blocks_f)  UMA ActRec  O(1)
sequence_diagram(entry)        O(n_call_sites)  UMA SeqRec O(1)
cfl_reachable(s, t)            O(|V|²)  CFL tabulation   O(1)
backward_slice(sym)            O(n_uses + n_cdg_edges)    O(1)
coverage(traces)               O(|ROBDD| + |traces|)      O(1)
impact_set(sym)                O(|V_CG| + |E_CG|)         O(1)
```

---

### 10.6 Algorithm Specifications

#### 10.6.1 SCPG Serialization

```rust
impl Phase10Stage {
    pub fn run(
        tca: TCA, bpa: BPA, sta: STA, cfa: CFA,
        ssa: SSA, cga: CGA, tra: TRA, uma: UMA, psa: PSA,
        out_path: &Path,
    ) -> Result<OpenHeartEngine> {

        // Step 1: Compute section byte offsets (hot → warm → cold ordering)
        let layout = SectionLayoutPlanner::plan(&[&tca, &bpa, &sta, &cfa, &ssa, &cga, &tra, &uma, &psa]);

        // Step 2: Stream all sections into the output file
        let mut writer = BufWriter::new(File::create(out_path)?);

        // Write header placeholder (filled in at the end with known sizes/hashes)
        writer.write_all(&[0u8; 128 + 264])?;

        let mut section_crcs: [u32; 11] = [0; 11];

        for (section_code, artifact_bytes) in layout.ordered_sections() {
            let pos_before = writer.stream_position()?;
            // LZ4 HC compress each section independently for lazy decompression
            let compressed = lz4_hc::compress(artifact_bytes);
            let crc = crc32(&compressed);
            writer.write_all(&compressed)?;
            section_crcs[section_code as usize] = crc;
            layout.record_actual_size(section_code, writer.stream_position()? - pos_before);
        }

        // Step 3: Rewind and write completed header + directory
        writer.seek(SeekFrom::Start(0))?;
        let source_hash = sha256_all_sources(&[&tca]);
        let scpg_hash   = compose_hash(&tca, &bpa, &sta, &cfa, &ssa, &cga, &tra, &uma, &psa);
        writer.write_all(&build_header(source_hash, scpg_hash))?;
        writer.write_all(&build_section_directory(&layout, &section_crcs))?;
        writer.flush()?;

        // Step 4: Bootstrap query engine on the completed file
        let mmap  = MemoryMappedSCPG::open(out_path)?;
        let cache = LRUQueryCache::new(512);
        let engine = OpenHeartEngine::assemble(mmap, cache);

        Ok(engine)
    }
}
```

#### 10.6.2 Incremental Update — Partial Pipeline Rebuild

> **Implementation (audited baseline `fc218cd`):** not implemented. `src/scpg/incremental/` receives a delta made of a file id and three caller-asserted booleans (no byte range or text), checks the comment-only flag first (so a delta flagged as both body-only and type-changing is classified as behavioural), rebuilds nothing, reports zero stale links and returns `current_hash ^ 0x0101_0101` — so applying two deltas restores the original hash, which is also the key the query cache uses for validity. Nothing calls it. The prerequisites (stable ids, edit-derived deltas, dependency tracking) are listed in [traceability_and_incremental.md §3.1](../traceability_and_incremental.md#31-prerequisites).

When source changes, the `IncrementalEngine` determines the minimal set of phases to re-execute:

```rust
impl IncrementalEngine {
    pub fn apply_delta(&mut self, delta: SourceDelta) -> Result<UpdateResult> {
        // Step 1: Classify the change scope
        let scope = RebuildPlanner::classify(&delta, &self.scpg);
        // scope is one of: TOKEN_ONLY | STRUCTURAL | SEMANTIC | BEHAVIORAL | FULL

        // Step 2: Re-run only the affected phases
        let new_artifacts = PartialPipelineRunner::run(scope, &delta, &self.scpg)?;

        // Step 3: Compute artifact deltas and update SCPG sections in-place
        let updated_sections = SCPGMerger::merge(&self.scpg, &new_artifacts)?;

        // Step 4: Recompute scpg_hash and invalidate cache
        let new_hash = recompute_scpg_hash(&new_artifacts);
        self.cache.invalidate(new_hash);  // O(1): just stores new hash, lazy-clears on next get

        // Step 5: Compute stale UMLLink set (O(n_uml) scan)
        let stale_links = self.scpg.tra().find_stale_links(new_hash);

        // Step 6: Notify all diagram views of invalidated elements
        for view in &self.registered_views {
            view.invalidate(stale_links.clone());
        }

        Ok(UpdateResult {
            phases_re_run: scope.phases(),
            stale_link_count: stale_links.len(),
            new_scpg_hash: new_hash,
        })
    }
}

/// Rebuild scope classification:
/// Given a source delta, which phases must re-execute?
fn classify(delta: &SourceDelta, scpg: &SCPG) -> RebuildScope {
    if delta.moves_no_offsets() {
        RebuildScope::None // nothing to do; scpg_hash unchanged
    } else if delta.affects_only_comments_or_whitespace() {
        // Comment/whitespace edits move the line/column of every later token in the file,
        // so TCA sort keys and TRA spans must be re-indexed (re-lex the file), but no
        // semantic phase re-runs.
        RebuildScope::PositionsOnly
    } else if delta.affects_only_method_bodies() {
        RebuildScope::Behavioral // Re-run Phases 4,5,7,8,9 for changed methods
    } else if delta.affects_type_declarations() {
        RebuildScope::Structural // Re-run Phases 3-9 for affected classes
    } else {
        RebuildScope::Full // Token-level change: re-run all phases
    }
}
```

**Incremental rebuild cost for common change types:**

```
Change type                    Work required
─────────────────────────────────────────────────────────────────────────
Edit that moves no offsets     none (scpg_hash unchanged)
Comment/whitespace only        re-lex the file; re-index TCA sort keys and TRA spans
Method body edit               Phases 4,5,7,8,9 for the changed method
Method signature change        Phases 3,4,5,6,7,9 for the affected class and its dependents
New class added                Phases 3-9 for the new class and its dependents
File-level refactor            Phases 1-9 for the file and its dependents
Full rebuild (cold start)      Phases 1-10

No latency is given for any row: none has been measured.
```

#### 10.6.3 Backward Slice Implementation

```rust
fn backward_slice(
    root_sym: u32, scpg: &SCPG, depth_limit: u32,
) -> SliceResult {
    let mut slice = HashSet::new();
    let mut work  = vec![(root_sym, 0u32)];

    while let Some((sym, depth)) = work.pop() {
        if depth > depth_limit || !slice.insert(sym) { continue; }

        // ① Data dependences: all SSA variables whose def is a transitive use of root
        for ssa_id in scpg.ssa().def_use_predecessors(sym) {
            let def_sym = scpg.ssa().record(ssa_id).orig_sym_id;
            work.push((def_sym, depth + 1));
        }

        // ② Control dependences: all blocks that control sym's execution
        for cd_block in scpg.ssa().cdg_predecessors(sym) {
            let cond_sym = scpg.cfa().condition_sym_of_block(cd_block);
            work.push((cond_sym, depth + 1));
        }

        // ③ Call graph: callers of sym (inter-procedural slice)
        for caller in scpg.cga().callers_of(sym) {
            work.push((caller, depth + 1));
        }
    }

    SliceResult { members: slice, root: root_sym }
}
```

---

### 10.7 The Production Public API

```rust
/// The OpenHeart engine — production entry point for all SCPG operations.
/// Thread-safe: clone the Arc<OpenHeartEngine> for concurrent diagram generation.
pub struct OpenHeartEngine {
    scpg:  Arc<MemoryMappedSCPG>,
    query: Arc<Mutex<QueryEngine>>,
}

impl OpenHeartEngine {

    // ── Construction ────────────────────────────────────────────────────────
    pub fn build(manifest: SourceManifest) -> Result<Self>
    pub fn open(scpg_path: &Path) -> Result<Self>  // open pre-built .scpg
    pub fn builder() -> EngineBuilder              // fluent configuration

    // ── Structural UML Diagrams ──────────────────────────────────────────────
    pub fn class_diagram(&self, cfg: ClassDiagramConfig) -> ClassDiagram
    pub fn object_diagram(&self, snapshot: Option<SnapshotId>) -> ObjectDiagram
    pub fn component_diagram(&self) -> ComponentDiagram
    pub fn deployment_diagram(&self) -> DeploymentDiagram
    pub fn package_diagram(&self, root: Option<PackageId>) -> PackageDiagram
    pub fn composite_diagram(&self, class: SymbolId) -> CompositeDiagram
    pub fn profile_diagram(&self) -> ProfileDiagram

    // ── Behavioral UML Diagrams ──────────────────────────────────────────────
    pub fn use_case_diagram(&self) -> UseCaseDiagram
    pub fn activity_diagram(&self, method: SymbolId) -> ActivityDiagram
    pub fn state_machine(&self, class: SymbolId) -> Option<StateMachine>
    pub fn sequence_diagram(&self, entry: SymbolId, depth: usize) -> SequenceDiagram
    pub fn communication_diagram(&self, entry: SymbolId) -> CommunicationDiagram
    pub fn interaction_overview(&self, method: SymbolId) -> InteractionOverview
    pub fn timing_diagram(&self, thread_entry: SymbolId) -> TimingDiagram

    // ── Path Analysis ────────────────────────────────────────────────────────
    pub fn cyclomatic(&self, method: SymbolId) -> u16
    pub fn path_count(&self, method: SymbolId) -> u64
    pub fn coverage(&self, traces: &[ExecutionTrace]) -> CoverageReport
    pub fn is_reachable(&self, from: SymbolId, to: SymbolId) -> bool   // CFL O(V²)
    pub fn all_paths(&self, method: SymbolId, limit: u32) -> Vec<Path> // ROBDD enumeration

    // ── Navigation (all O(1) or O(log n)) ───────────────────────────────────
    pub fn to_source(&self, entity: EntityRef) -> SourceRange   // backward τ
    pub fn from_source(&self, pos: SourcePos) -> Vec<EntityRef> // forward τ⁻¹

    // ── Structural Queries ───────────────────────────────────────────────────
    pub fn callers_of(&self, method: SymbolId) -> Vec<SymbolId>
    pub fn callees_of(&self, method: SymbolId) -> Vec<SymbolId>
    pub fn subtypes_of(&self, class: SymbolId) -> Vec<SymbolId>
    pub fn supertypes_of(&self, class: SymbolId) -> Vec<SymbolId>
    pub fn backward_slice(&self, sym: SymbolId, depth: u32) -> SliceResult
    pub fn forward_slice(&self, sym: SymbolId, depth: u32) -> SliceResult
    pub fn impact_set(&self, sym: SymbolId) -> ImpactResult

    // ── Export ───────────────────────────────────────────────────────────────
    pub fn export_xmi(&self, diagram: &dyn Diagram) -> String
    pub fn export_plantuml(&self, diagram: &dyn Diagram) -> String
    pub fn export_json(&self, diagram: &dyn Diagram) -> serde_json::Value

    // ── Incremental Update ───────────────────────────────────────────────────
    pub fn update(&self, delta: SourceDelta) -> UpdateResult
    pub fn watch(&self, path: &Path) -> FileWatcher // FS watcher → auto-update

    // ── Inspection ───────────────────────────────────────────────────────────
    pub fn symbol(&self, id: SymbolId) -> &SymbolRecord
    pub fn find_symbol(&self, qualified_name: &str) -> Option<SymbolId>
    pub fn scpg_hash(&self) -> u32    // current composite hash
    pub fn stats(&self) -> SCPGStats  // symbol counts, artifact sizes, etc.
}
```

---

### 10.8 Readiness Checklist

An earlier version of this section ticked every item below as satisfied. That was incorrect: most items describe the design, not the delivered code. Each item now carries its status at the audited baseline `fc218cd` under the rule in [docs/status.md §1](../status.md#1-status-rule) (an item is ✅ only with a linked module **and** a test asserting an expected result). Legend: ✅ implemented · 🟡 partial · ❌ not implemented / stub · ❔ not verified by any test.

| Area | Item | Status | Notes |
|---|---|---|---|
| Correctness | All 14 UML diagram types generated from analysis | ❌ | Several views are templates or empty extractors ([provenance matrix](../status.md#5-diagram-provenance-matrix)). |
| Correctness | Source ↔ diagram navigation at O(1) / O(log n) | 🟡 | Backward O(1); forward queries scan after a binary search. |
| Correctness | Hash-chain integrity across all artifacts | 🟡 | CRC-32 hash chain over upstream checksums; no per-entity hashes. |
| Correctness | Stale UMLLink detection per link | ❌ | The whole-build tag marks every link stale on any edit; the incremental engine is a stub. |
| Correctness | SSA single-assignment verified at finalize | ❔ | No differential test against a reference SSA construction. |
| Correctness | ROBDD canonicity after every sifting pass | ❌ | Sifting has no effect; path counts are overestimates. |
| Performance | Memory-mapped zero-copy access | ❌ | `MemoryMappedSCPG::open` reads the whole file; sections are counts only. |
| Performance | Hot/cold section residency | ❌ | Not applicable until sections carry data. |
| Performance | Navigation O(1) or O(log n) | 🟡 | See above. |
| Performance | Diagram generation O(n_sym), cached O(1) | ❔ | Not measured. |
| Performance | CFL-reachability sub-second for 15K methods | ❌ | Not CFL-reachability (BFS with an O(V·E) inner loop); not measured. |
| Performance | LRU query cache (512 entries, invalidation on hash change) | 🟡 | Cache exists; its validity key comes from the stub incremental engine. |
| Performance | Incremental updates in 80–200 ms | ❌ | Not implemented; not measured. |
| Performance | Cold build 8–15 s for 500K LOC | ❔ | Not measured. |
| Space | ≈ 82 MB LZ4-compressed for 500K LOC | ❌ | No LZ4 dependency; not measured. |
| Space | ~12 MB typical resident set | ❔ | Not measured. |
| Space | Succinct AST 6.9× smaller than pointer-based | ❌ | The AST stores an O(n log n)-word sparse table; not measured. |
| Space | Lazy-loaded per-function ROBDDs | ❌ | Sections carry no ROBDD data. |
| Robustness | Per-section checksum verification at open | ❔ | See `src/scpg/serializer/integrity.rs`; requires corruption tests that expect an error. |
| Robustness | SHA-256 source hash detects stale SCPG | ❌ | `source_hash` is written as zeros. |
| Robustness | Phase invariants verified at finalize | ❔ | |
| Robustness | Rebuild planner classifies scope correctly | ❌ | Stub; comment/whitespace edits were classified as needing no work (see §10.6.2). |
| Robustness | CRC-64/ECMA on every Phase 1–9 artifact | 🟡 | Written; not verified uniformly by readers. |
| Extensibility | Language adapter pattern | 🟡 | Only Java has adapters for every phase; Kotlin uses the Java grammar. |
| Extensibility | New query types without modifying dispatch | ❔ | `query_type` is a fixed `u8`. |
| Extensibility | New output formats in the export layer | 🟡 | PlantUML, Mermaid, JSON; XMI is a placeholder. |
| Extensibility | XMI export implemented | ❌ | Emits XMI 2.1 / Eclipse UML2 3.0.0 class shells named `Class_<id>`; not written by the CLI or server. |
| IDE integration | LSPBridge (definition, references, symbols, hover) | ❌ | Does not exist. |
| IDE integration | UMLLink click-to-source protocol | 🟡 | Records exist; consumed by the web studio only. |
| IDE integration | FileWatcher / `didChange` → `SourceDelta` | ❌ | Does not exist. |
| Multi-language | Language-agnostic model (Phases 2–10) | 🟡 | Model is language-neutral; only Java reaches it with a correct grammar. |
| Multi-language | Mixed Java + Kotlin under one namespace | ❌ | No Kotlin grammar; no polyglot JVM model. |

The original checklist is kept below as the design target; `◻` marks a target, not a delivered item.

```
CORRECTNESS
  ◻ All 14 UML diagram types generated (Phase 9 UMA + Phase 10 renderers)
  ◻ Bidirectional source↔diagram navigation at O(1)/O(log n) (Phase 7 TRA)
  ◻ Strict hash-chain integrity across all 9 artifacts → SCPG (Phase 10 header)
  ◻ Stale UMLLink detection in O(1) per link (Phase 7 scpg_hash protocol)
  ◻ SSA form: single-assignment verified at Phase 5 finalize() (Invariant 1)
  ◻ ROBDD canonicity verified after every sifting pass (Phase 8 Invariant 3)

PERFORMANCE
  ◻ Memory-mapped I/O: zero-copy access, OS-managed LRU page cache
  ◻ Hot sections (63 MB) resident after first query; cold sections lazy-loaded
  ◻ All navigation queries: O(1) or O(log n) via TRA dense arrays + span index
  ◻ All diagram generation: O(n_sym) first run, O(1) cached
  ◻ CFL-reachability: O(|V|²) practical — sub-second for 15K methods
  ◻ LRU query cache (512 entries, full invalidation on scpg_hash change)
  ◻ Incremental updates: O(|delta|) — typically 80-200ms for single-method edits
  ◻ Full cold-start build: ~8-15s for 500K LOC Java (parallelizable Phases 4-8)

SPACE
  ◻ SCPG binary ≈ 82 MB LZ4-compressed for 500K LOC Java (< 200 bytes per LOC)
  ◻ Typical RAM resident: ~12 MB (hot sections + LRU cache)
  ◻ Succinct AST (BP encoding): 6.9× smaller than pointer-based
  ◻ ROBDD path summaries: lazy-loaded per-function (~6 KB each), never all at once

ROBUSTNESS
  ◻ Per-section CRC-32 integrity verification at SCPG open time
  ◻ SHA-256 source hash detects out-of-date SCPG (source changed, SCPG not rebuilt)
  ◻ Phase invariants (10 total across all phases) verified at artifact finalize()
  ◻ Incremental rebuild planner: classifies scope correctly to avoid silent staleness
  ◻ CRC-64/ECMA checksum on every binary artifact (Phases 1-9 output files)

EXTENSIBILITY
  ◻ Language adapter pattern: plug in new grammars without touching core pipeline
  ◻ Java is Phase 1 concrete implementation; Kotlin, Python, Go, Rust, TypeScript
      follow by implementing LanguageAdapter + ASTReductionAdapter + SemanticAdapter
  ◻ Query engine: new query types added without modifying LRU cache or dispatch
  ◻ Diagram renderers: new output formats added to Phase 10 export layer
  ◻ XMI, PlantUML, and JSON export implemented — plug in SVG/D3/Mermaid as needed

IDE INTEGRATION
  ◻ LSPBridge exposes: textDocument/definition, textDocument/references,
      textDocument/documentSymbol, textDocument/hover, workspace/symbol
  ◻ UMLLink protocol: any IDE can render click-to-source for diagram elements
  ◻ FileWatcher: fs::watch() integration for real-time incremental updates
  ◻ SourceDelta API: LSP textDocument/didChange maps directly to SourceDelta

MULTI-LANGUAGE
  ◻ Core SCPG mathematical model is language-agnostic (Phases 2-10)
  ◻ Language isolation: Phase 1 adapters + Phase 3 semantic adapters per language
  ◻ Token type alphabet Σ_T: 0x00-0x7F language-agnostic, 0x80-0xFF per-language
  ◻ Node type alphabet Σ_N: 0x00-0x7F language-agnostic, 0x80-0xFF per-language
  ◻ Multi-language project (e.g. Java+Kotlin+XML): adapters register per file extension,
      PhaseN stage dispatches per file, SCPG unifies under one symbol namespace
```

---

### 10.9 End-to-End Performance Targets (design, unmeasured)

The figures below are design targets. None has been measured; they must not be quoted as results.

```
Operation                              Target latency (500K LOC Java, warm cache)
──────────────────────────────────────────────────────────────────────────
Open pre-built .scpg (mmap)            < 1ms
Generate full class diagram            < 5ms
Generate activity diagram (1 method)   < 1ms
Generate sequence diagram (1 entry)    < 2ms
Navigate: UML element → source line    < 0.1ms  (O(1) BI lookup)
Navigate: source line → UML elements  < 0.5ms  (O(log n) FI lookup)
CFL-reachability query (s → t)         < 100ms  (O(V²), cached after first)
Cyclomatic complexity lookup           < 0.1ms  (O(1) PSA metrics)
Backward slice (depth 5)               < 10ms   (CDG + def-use traversal)
Single-method incremental update       < 80ms   (Phases 4,5,7,8,9 partial)
File-level incremental update          < 500ms  (most phases partial)
```

---

Now the final visualization — the OpenHeart system completion dashboard: