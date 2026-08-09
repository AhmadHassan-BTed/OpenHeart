# Codebase Onboarding & Contributor Map

This guide provides a structural breakdown of the **OpenHeart** codebase for contributors and maintainers. Created and maintained solely by **Ahmad Hassan (B-Ted)**.

---

## Source Module Layout (5 Active Completed Engine Phases)

```text
src/
├── core/                         # Language-Agnostic Types & Binary I/O Primitives
│   ├── io/                       # BinaryWriter, BinaryReader, MemoryMappedFile, CRC-64
│   └── types/                    # Core Artifact Types (TokenRecord, BPARecord, SymbolRecord, CFGRecord, SSARecord)
│
├── ingestion/                    # Phase 1: Lexical Ingestion & Token Corpus Engine (.tca)
│   ├── adapter/                  # Tree-sitter Language Adapters (Java, C, etc.)
│   ├── parser/                   # CST Parser Wrapper
│   ├── allocator.rs              # Monotonic TokenIdAllocator
│   ├── builder.rs                # TokenCorpusBuilder & Invariants 1–4 Validation
│   ├── interner.rs               # FNV-1a StringInterner
│   └── serializer.rs             # Binary .tca Serializer/Deserializer
│
├── ast/                          # Phase 2: CST Reduction & BP AST Encoding Engine (.bpa)
│   ├── bp_encoder.rs             # Bit-packed 2-bit Balanced Parentheses Bitstring
│   ├── jump_table.rs             # O(1) Matching Parentheses Jump Table
│   ├── rank_select.rs            # Jacobson O(1) Rank/Select Auxiliary Indices
│   ├── rmq.rs                    # Sparse Table RMQ for O(1) LCA Queries
│   └── serializer.rs             # Binary .bpa Serializer/Deserializer
│
├── symbol/                       # Phase 3: Symbol Table & Type Hierarchy Engine (.sta)
│   ├── passes/                   # 5-Pass DFS Symbol Discovery & Scope Graph Resolution
│   ├── scope.rs                  # Scope Graph Node & Resolution Hierarchy ($V_{\text{sym}}$)
│   └── serializer.rs             # Binary .sta Serializer/Deserializer
│
├── cfg/                          # Phase 4: Control Flow Graph & Dominator Analysis (.cfa)
│   ├── stmts/                    # Branching & Statement CFG Builders (if, while, for, try-catch)
│   ├── dominators.rs             # Cooper Iterative Immediate Dominators (`idom[]`)
│   ├── frontier.rs               # Cytron Dominance Frontier Computation ($DF[b]$)
│   ├── loops.rs                  # Loop Nesting Forest & Back-Edge Detection
│   └── serializer.rs             # Binary .cfa Serializer/Deserializer
│
├── ssa/                          # Phase 5: SSA Conversion, CDG & IFDS Engine (.ssa)
│   ├── liveness.rs               # Pruned SSA Backward Liveness Fixpoint
│   ├── placement.rs              # Cytron $\phi$-Function Placement Worklist
│   ├── renaming.rs               # Dominator Tree DFS Variable Renaming & `VersionStack`
│   ├── cdg.rs                    # Control Dependence Graph via Reversed Post-Dominators
│   ├── ifds.rs                   # Reps-Horwitz-Sagiv Polynomial IFDS Solvers (Taint, Null, Type-State)
│   └── serializer.rs             # Binary .ssa Serializer/Deserializer
│
├── main.rs                       # CLI Binary (`openheart analyze`, `openheart inspect`)
└── lib.rs                        # Core Library Crate Root
```

---

## Integration Test Suite Layout

```text
tests/
├── ingestion_tests.rs            # Phase 1 Ingestion & Token Invariants Integration Tests
├── ast_tests.rs                  # Phase 2 BP AST Encoding & Rank/Select Integration Tests
├── symbol_tests.rs               # Phase 3 Symbol Table & Scope Resolution Integration Tests
├── cfg_tests.rs                  # Phase 4 CFG & Dominator Tree Integration Tests
├── ssa_tests.rs                  # Phase 5 SSA Conversion & IFDS Solver Integration Tests
└── pipeline_accuracy_tests.rs    # Ruthless Line-by-Line Multi-Phase Pipeline Verification
```

---

## Primary Engine Binary Artifact Formats

| Phase | Artifact Extension | Binary Format Header | Core Persisted Data Structures |
|---|---|---|---|
| **Phase 1** | `.tca` | `TCA\0` Magic | `SourceFileRecord[]`, `TokenRecord[]`, FNV-1a `StringInterner` |
| **Phase 2** | `.bpa` | `BPA\0` Magic | BP bitstring (`u64[]`), `JumpTable`, `RankSelectIndex`, `SparseTableRMQ` |
| **Phase 3** | `.sta` | `STA\0` Magic | `SymbolRecord[]`, `ScopeNode[]`, `TypeHierarchyCSR` |
| **Phase 4** | `.cfa` | `CFA\0` Magic | `SuccessorCSR`, `PredecessorCSR`, `idom[]`, `DominanceFrontierCSR` |
| **Phase 5** | `.ssa` | `SSA\0` Magic | `SSARecord[]` (16B), `PhiRecord[]`, `DefUseCSR`, `CDGCSR`, IFDS Facts |
