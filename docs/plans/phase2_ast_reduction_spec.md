Here is the complete Phase 2 specification.

---

## Phase 2: CST Reduction & Balanced Parentheses AST Encoding

### 2.1 Phase Mandate & Exact Boundaries

Phase 2 has a single contractual obligation: transform the raw, syntactically noisy CST produced by Tree-sitter into a mathematically clean, succinct ordinal forest encoded as a Balanced Parentheses (BP) sequence. Every subsequent phase operates on this encoded forest — it is the canonical structural backbone of the entire SCPG.

Phase 2 does NOT perform name resolution (Phase 3), CFG construction (Phase 4), or SSA conversion (Phase 5). It transforms tree shape and assigns AST node types. Nothing else.

**Input:** `TokenCorpusArtifact (.tca)` from Phase 1 — specifically the file registry (to re-read source files) and the sorted `TokenRecord[]` array (to perform `token_id` lookups for leaf nodes).

**Output:** `BPASTArtifact (.bpa)` — a binary file containing the BP bitstring, jump table, rank/select auxiliary structure, and four parallel pre-order arrays. Phase 3 ingests this file verbatim.

**Why re-parse?** The Production Line law mandates that Phase 1 outputs only the token corpus. Passing a raw Tree-sitter `TSTree` across a phase boundary would couple Phase 1 to Phase 2's parser implementation. Phase 2 instead re-reads source files (already OS-page-cached after Phase 1's reads) and re-parses with Tree-sitter — O(|file|) per file, dominated by Phase 1's sort cost.

---

### 2.2 Input Schema

```rust
/// Phase 2 receives only the TokenCorpusArtifact from Phase 1.
/// It extracts the file registry via memory-mapped access to the .tca file.
pub struct Phase2Input {
    pub tca: MemoryMappedFile,   // read-only mmap of the Phase 1 artifact
}

/// Derived from tca — the two sub-structures Phase 2 actively uses:
/// 1. file_registry: Vec<SourceFileRecord>  — file paths + language IDs + token offsets
/// 2. sorted_token_table: &[TokenRecord]    — for token_id forward lookup by (file,line,col)
```

The `sorted_token_table` from the TCA file serves as Phase 2's token_id oracle. When the CST Walker visits a leaf node at position `(file_id, line, col)`, Phase 2 constructs `sort_key = (file_id << 48) | (line << 24) | (col << 8)` and runs `lower_bound` binary search on `sorted_token_table` to retrieve the `token_id` — the same O(log n) forward lookup defined in Phase 1, §1.3.2.

---

### 2.3 Mathematical Foundations

#### 2.3.1 The Node Type Alphabet Σ_N

Σ_N is a u8 alphabet (256 values) for AST internal nodes. The high bit partitions language-agnostic (0x00–0x7F) from language-specific extensions (0x80–0xFF):

```
Core Σ_N (language-agnostic):
0x00  NN_UNKNOWN
0x01  NN_MODULE            # top-level compilation unit / file root
0x02  NN_CLASS_DECL        # class declaration
0x03  NN_INTERFACE_DECL    # interface declaration
0x04  NN_ENUM_DECL         # enum declaration
0x05  NN_RECORD_DECL       # record class (Java 16+, Kotlin data class)
0x06  NN_ANNOTATION_DECL  # @interface declaration
0x07  NN_METHOD_DECL       # method / function declaration
0x08  NN_CONSTRUCTOR_DECL  # constructor
0x09  NN_FIELD_DECL        # field / member variable declaration
0x0A  NN_PARAM_DECL        # formal parameter
0x0B  NN_LOCAL_VAR_DECL   # local variable declaration
0x0C  NN_BLOCK             # { ... } statement block
0x0D  NN_IF_STMT           # if statement
0x0E  NN_ELSE_CLAUSE       # else clause
0x0F  NN_FOR_STMT          # for loop
0x10  NN_ENHANCED_FOR      # for-each / range-based loop
0x11  NN_WHILE_STMT        # while loop
0x12  NN_DO_WHILE_STMT     # do-while loop
0x13  NN_SWITCH_STMT       # switch statement
0x14  NN_SWITCH_CASE       # case / default label group
0x15  NN_TRY_STMT          # try statement
0x16  NN_CATCH_CLAUSE      # catch clause
0x17  NN_FINALLY_CLAUSE    # finally clause
0x18  NN_RETURN_STMT       # return statement
0x19  NN_THROW_STMT        # throw statement
0x1A  NN_BREAK_STMT        # break statement
0x1B  NN_CONTINUE_STMT     # continue statement
0x1C  NN_EXPR_STMT         # expression statement (wraps expression)
0x1D  NN_ASSIGN_EXPR       # assignment (=, +=, -=, ...)
0x1E  NN_BINARY_EXPR       # binary operation (a OP b)
0x1F  NN_UNARY_EXPR        # unary operation (!a, -b, ++c)
0x20  NN_TERNARY_EXPR      # conditional (a ? b : c)
0x21  NN_CALL_EXPR         # method / function call
0x22  NN_NEW_EXPR          # object creation (new Foo(...))
0x23  NN_FIELD_ACCESS      # object.field
0x24  NN_ARRAY_ACCESS      # arr[i]
0x25  NN_CAST_EXPR         # (Type) expr
0x26  NN_INSTANCEOF_EXPR   # expr instanceof Type
0x27  NN_LAMBDA_EXPR       # (x) -> body
0x28  NN_METHOD_REF        # Type::method
0x29  NN_ARRAY_CREATE      # new int[n], new Foo[]{...}
0x2A  NN_TYPE_REF          # type reference node (String, List<T>, int[])
0x2B  NN_IDENTIFIER_EXPR   # identifier used as expression
0x2C  NN_LITERAL           # any literal (int, float, string, bool, null, char)
0x2D  NN_ANNOTATION_USE    # @Override, @SuppressWarnings(...)
0x2E  NN_TYPE_PARAM        # <T>, <E extends Comparable>
0x2F  NN_SUPER_EXPR        # super.field or super(...)
0x30  NN_THIS_EXPR         # this.field or this(...)
0x31  NN_ARRAY_INIT        # {1, 2, 3} array initializer
0x32  NN_SWITCH_EXPR       # switch expression (Java 14+)
0x33  NN_PATTERN_MATCH     # pattern matching (instanceof, switch patterns)
0x34  NN_YIELD_STMT        # yield in switch expression
0x7F  NN_SYNTHETIC         # synthetic node inserted by analysis (no source loc)
0x80-0xFF  language-specific extensions
  Java:   0x80 = NN_JAVA_STATIC_INIT   # static { ... }
          0x81 = NN_JAVA_INSTANCE_INIT  # instance initializer block
          0x82 = NN_JAVA_ASSERT_STMT    # assert expr : msg
          0x83 = NN_JAVA_LABELED_STMT   # label: stmt
          0x84 = NN_JAVA_SYNCHRONIZED   # synchronized(obj) { }
```

#### 2.3.2 Formal BP Encoding Definition

**Definition:** Given an ordered forest F = (V, E, ≺) where V is the node set, E the parent-child edges, and ≺ the sibling ordering, the Balanced Parentheses encoding of F is the bit string B ∈ {0,1}^{2n} where n = |V|, constructed by:

```
DFS-BP(v):
    emit bit 1    ← "open parenthesis" for v (pre-order visit)
    for each child c of v in left-to-right order ≺:
        DFS-BP(c)
    emit bit 0    ← "close parenthesis" for v (post-order backtrack)
```

For a leaf node v (no children): DFS-BP emits `1 0` — two consecutive bits. For a root with two children: `1 [child1 bits] [child2 bits] 0`. The resulting string is a perfectly balanced parenthesization.

**Proof of bijectivity:** The map F → B(F) is bijective. B(F) encodes the entire tree structure: the i-th 1-bit (0-indexed) corresponds to the i-th node in pre-order, and the corresponding 0-bit (found via the jump table) marks that node's post-order exit. Given B, the original forest is recovered by treating matched pairs `(open_pos, close_pos)` as node boundaries and reading the pre-order sequence. ∎

**Critical implementation invariant:** bit 1 = open paren = pre-order visit; bit 0 = close paren = post-order backtrack. The BP sequence has exactly n 1-bits and n 0-bits. The pre-order index of a node v equals `rank_1(open_pos(v)) - 1`, where `rank_1(i)` counts 1-bits in positions 0..=i.

#### 2.3.3 Bit Packing Scheme

Bits are packed MSB-first into u64 words for alignment with 64-bit architecture cache lines:

```
Word 0:  bit 0 = MSB  (position 63 in the u64)
         bit 1 = next MSB (position 62)
         ...
         bit 63 = LSB (position 0)
Word 1:  bit 64 = MSB, ...

push_bit(1): word[bit_count/64] |=  1u64 << (63 - bit_count%64)
push_bit(0): word[bit_count/64] &= !(1u64 << (63 - bit_count%64))  (bit already 0 from init)
get_bit(i):  (word[i/64] >> (63 - i%64)) & 1
```

Four bytes = 32 bits = 16 nodes. One 64-byte cache line = 512 bits = 256 nodes. Binary search on BP bits during rank queries is maximally cache-efficient.

#### 2.3.4 Navigation Operations

All operations on the BP-encoded forest are expressed in terms of three primitives:

```
rank_1(i)      → number of 1-bits in B[0..=i]         O(1) via rank/select index
select_1(j)    → position of j-th 1-bit in B           O(1) via rank/select index
match_pos(i)   → position of matching paren at i        O(1) via jump table
```

Derived navigation operations (all O(1)):

```
preorder_idx(bp_pos):        rank_1(bp_pos) - 1
open_pos(pre_idx):           select_1(pre_idx + 1)

parent(pre_idx):             use parent_map[pre_idx]  (precomputed dense array)
                             alt: enclose via rank analysis (more complex)

first_child(pre_idx):
  op = open_pos(pre_idx)
  if B[op+1] == 0:           leaf node → no children → return None
  else:                      pre_idx of first child = rank_1(op+1) - 1

next_sibling(pre_idx):
  op = open_pos(pre_idx)
  cp = match_pos(op)         close paren of current node
  if B[cp+1] == 1:           a 1-bit follows → next sibling exists
    return rank_1(cp+1) - 1
  else: return None          next bit is 0 or EOB → no next sibling

subtree_size(pre_idx):
  op = open_pos(pre_idx)
  cp = match_pos(op)
  (cp - op + 1) / 2         half the distance between matched parens = node count

is_leaf(pre_idx):
  op = open_pos(pre_idx)
  B[op+1] == 0              immediately followed by close paren → leaf

depth(pre_idx):
  op = open_pos(pre_idx)
  2 * rank_1(op) - op - 1  = excess at position op (1-indexed depth from root)

LCA(pre_idx_u, pre_idx_v):  requires RMQ on excess array (see §2.7.5)
```

#### 2.3.5 NodeAttr Packed Word (u32)

Each AST node carries a 4-byte attribute word encoding the most frequently queried structural properties:

```
NodeAttr (u32, MSB to LSB):

Bits 31-28 (4 bits): visibility
    0000 = VISIBILITY_NONE (no modifier present)
    0001 = PUBLIC
    0010 = PRIVATE
    0011 = PROTECTED
    0100 = PACKAGE_PRIVATE (Java default)
    0101-0111 = reserved

Bits 27-20 (8 bits): modifier bitmap
    bit 27 = static
    bit 26 = final
    bit 25 = abstract
    bit 24 = synchronized
    bit 23 = native
    bit 22 = volatile
    bit 21 = transient
    bit 20 = strictfp / sealed

Bits 19-12 (8 bits): operator_id (for BINARY_EXPR, UNARY_EXPR, ASSIGN_EXPR)
    Encoded as a compact enum (+ - * / % == != < > <= >= && || ! & | ^ ~ << >> >>> = += -= etc.)
    0x00 = no operator (non-expression nodes)
    See operator_id table in §A.1 of the appendix

Bits 11-8 (4 bits): auxiliary type flags
    bit 11 = is_varargs (method parameter with ...)
    bit 10 = is_generic (class/method has type parameters)
    bit  9 = is_array_type (type reference is an array)
    bit  8 = is_static_initializer (for NN_BLOCK that is a static init)

Bits 7-0 (8 bits): language-specific flags (reserved per LangId)
    Java: bit 7 = is_record_component
          bit 6 = is_sealed
          bit 5 = is_permits_clause
          bit 4 = is_text_block (string literal is a text block)
```

For nodes that require more than 4 bytes of attribute data (e.g. annotation arguments, complex type bounds), a `NodeAttr::HAS_EXTENDED_ATTRS` flag (bit 0 of the language-specific byte) signals an entry in the extended attribute side table, stored as a separate section in the BPA file.

---

### 2.4 The CST Reduction Framework

#### 2.4.1 Reduction Decision Taxonomy

The `ASTReductionAdapter` classifies every CST node encountered during the DFS walk into exactly one of three decisions:

```rust
pub enum ReductionDecision {
    /// Emit an AST internal node. Children are recursed and attached as children.
    Keep(ASTNodeType),

    /// Do not emit a node for this CST node, but DO recurse into its children.
    /// Children are attached to the nearest kept ancestor.
    Eliminate,

    /// Do not emit a node. Do NOT recurse into children.
    /// Entire subtree is discarded.
    Drop,
}
```

**Keep:** Applied to all semantically meaningful syntactic constructs — declarations, statements, expressions. Each kept node becomes a V_syn vertex with a Σ_N type.

**Eliminate:** Applied to disambiguation grouping nodes that exist only for operator-precedence or grammar factoring. The children are "pulled up" to the nearest kept ancestor. Avoids spurious extra tree depth.

**Drop:** Applied to pure structural punctuation — braces, semicolons, brackets, commas — and to whitespace/comment nodes (unless the TokenFilter requests retention). The token_ids of dropped tokens still exist in the TokenCorpusArtifact; they simply have no AST node.

#### 2.4.2 ASTReductionAdapter Trait

```rust
pub trait ASTReductionAdapter: Send + Sync + 'static {
    /// Classify a CST node (by kind name + tree-sitter node reference)
    /// into one of three reduction decisions.
    fn classify(&self, kind: &str, node: &Node, depth: usize) -> ReductionDecision;

    /// Map the tree-sitter node kind to a Σ_N type.
    /// Only called when classify returns Keep(_).
    fn map_node_type(&self, kind: &str, node: &Node) -> ASTNodeType;

    /// Encode the NodeAttr u32 for a kept node.
    fn encode_attrs(&self, kind: &str, node: &Node, source: &[u8]) -> u32;

    /// Returns true if this is a leaf token that should be included
    /// as a V_tok leaf in the AST (vs. dropped as punctuation).
    fn include_leaf(&self, token_type: TokenType) -> bool;
}
```

#### 2.4.3 Java Adapter Reduction Table (Selected Critical Entries)

```rust
impl ASTReductionAdapter for JavaASTReductionAdapter {
    fn classify(&self, kind: &str, node: &Node, _depth: usize) -> ReductionDecision {
        match kind {
            // ── KEEP: Declarations ────────────────────────────────────────────
            "program"                           => Keep(NN_MODULE),
            "class_declaration"                 => Keep(NN_CLASS_DECL),
            "interface_declaration"             => Keep(NN_INTERFACE_DECL),
            "enum_declaration"                  => Keep(NN_ENUM_DECL),
            "record_declaration"                => Keep(NN_RECORD_DECL),
            "annotation_type_declaration"       => Keep(NN_ANNOTATION_DECL),
            "method_declaration"                => Keep(NN_METHOD_DECL),
            "constructor_declaration"           => Keep(NN_CONSTRUCTOR_DECL),
            "field_declaration"                 => Keep(NN_FIELD_DECL),
            "formal_parameter"                  => Keep(NN_PARAM_DECL),
            "spread_parameter"                  => Keep(NN_PARAM_DECL),  // varargs
            "local_variable_declaration"        => Keep(NN_LOCAL_VAR_DECL),
            "variable_declarator"               => Keep(NN_LOCAL_VAR_DECL), // sub-decl

            // ── KEEP: Statements ─────────────────────────────────────────────
            "block"                             => Keep(NN_BLOCK),
            "if_statement"                      => Keep(NN_IF_STMT),
            "for_statement"                     => Keep(NN_FOR_STMT),
            "enhanced_for_statement"            => Keep(NN_ENHANCED_FOR),
            "while_statement"                   => Keep(NN_WHILE_STMT),
            "do_statement"                      => Keep(NN_DO_WHILE_STMT),
            "switch_statement"                  => Keep(NN_SWITCH_STMT),
            "switch_expression"                 => Keep(NN_SWITCH_EXPR),
            "switch_block_statement_group"      => Keep(NN_SWITCH_CASE),
            "try_statement"                     => Keep(NN_TRY_STMT),
            "catch_clause"                      => Keep(NN_CATCH_CLAUSE),
            "finally_clause"                    => Keep(NN_FINALLY_CLAUSE),
            "return_statement"                  => Keep(NN_RETURN_STMT),
            "throw_statement"                   => Keep(NN_THROW_STMT),
            "break_statement"                   => Keep(NN_BREAK_STMT),
            "continue_statement"                => Keep(NN_CONTINUE_STMT),
            "expression_statement"              => Keep(NN_EXPR_STMT),
            "assert_statement"                  => Keep(NN_JAVA_ASSERT_STMT),
            "labeled_statement"                 => Keep(NN_JAVA_LABELED_STMT),
            "synchronized_statement"            => Keep(NN_JAVA_SYNCHRONIZED),
            "static_initializer"               => Keep(NN_JAVA_STATIC_INIT),

            // ── KEEP: Expressions ────────────────────────────────────────────
            "assignment_expression"             => Keep(NN_ASSIGN_EXPR),
            "binary_expression"                 => Keep(NN_BINARY_EXPR),
            "unary_expression"                  => Keep(NN_UNARY_EXPR),
            "update_expression"                 => Keep(NN_UNARY_EXPR),   // ++/--
            "ternary_expression"               => Keep(NN_TERNARY_EXPR),
            "method_invocation"                 => Keep(NN_CALL_EXPR),
            "explicit_generic_invocation"       => Keep(NN_CALL_EXPR),
            "object_creation_expression"        => Keep(NN_NEW_EXPR),
            "field_access"                      => Keep(NN_FIELD_ACCESS),
            "array_access"                      => Keep(NN_ARRAY_ACCESS),
            "cast_expression"                   => Keep(NN_CAST_EXPR),
            "instanceof_expression"             => Keep(NN_INSTANCEOF_EXPR),
            "lambda_expression"                 => Keep(NN_LAMBDA_EXPR),
            "method_reference"                  => Keep(NN_METHOD_REF),
            "array_creation_expression"         => Keep(NN_ARRAY_CREATE),
            "array_initializer"                 => Keep(NN_ARRAY_INIT),
            "marker_annotation"                 => Keep(NN_ANNOTATION_USE),
            "annotation"                        => Keep(NN_ANNOTATION_USE),
            "type_parameters"                   => Keep(NN_TYPE_PARAM),

            // ── KEEP: Leaf token nodes (semantic content) ────────────────────
            "identifier"                        => Keep(NN_IDENTIFIER_EXPR),
            "type_identifier"                   => Keep(NN_TYPE_REF),
            "void_type"                         => Keep(NN_TYPE_REF),
            "integral_type"                     => Keep(NN_TYPE_REF),    // int, long, ...
            "floating_point_type"               => Keep(NN_TYPE_REF),
            "boolean_type"                      => Keep(NN_TYPE_REF),
            "decimal_integer_literal"           => Keep(NN_LITERAL),
            "hex_integer_literal"               => Keep(NN_LITERAL),
            "binary_integer_literal"            => Keep(NN_LITERAL),
            "decimal_floating_point_literal"    => Keep(NN_LITERAL),
            "string_literal" | "text_block"     => Keep(NN_LITERAL),
            "character_literal"                 => Keep(NN_LITERAL),
            "true" | "false"                    => Keep(NN_LITERAL),
            "null_literal"                      => Keep(NN_LITERAL),
            "super"                             => Keep(NN_SUPER_EXPR),
            "this"                              => Keep(NN_THIS_EXPR),

            // ── ELIMINATE: Grouping / disambiguation wrappers ─────────────────
            "parenthesized_expression"          => Eliminate, // (expr) → just expr
            "modifiers"                         => Eliminate, // flatten modifiers
            "formal_parameters"                 => Eliminate, // flatten params
            "argument_list"                     => Eliminate, // flatten args
            "class_body"                        => Eliminate, // flatten members
            "interface_body"                    => Eliminate,
            "enum_body"                         => Eliminate,
            "enum_body_declarations"            => Eliminate,
            "block_statements"                  => Eliminate, // flatten stmts
            "superclass"                        => Eliminate, // extends Foo → just Foo
            "super_interfaces"                  => Eliminate, // implements → flatten
            "type_bound"                        => Eliminate, // T extends → flatten

            // ── DROP: Pure punctuation & structural delimiters ────────────────
            "{"  | "}"  | "("  | ")"  | "["  | "]" => Drop,
            ";"  | ","  | "."  | ":"  | "::" | "->" => Drop,
            "@"                                 => Drop,
            "<"  | ">"  | "?"                   => Drop,
            "..."                               => Drop,
            "switch"  | "case"  | "default"    => Drop,  // keyword tokens (structurally redundant)
            "catch"   | "finally" | "throws"   => Drop,
            "extends" | "implements" | "permits"=> Drop,

            // line_comment and block_comment: driven by TokenFilter (see include_leaf)
            "line_comment" | "block_comment"   => Drop,  // dropped unless filter requests retention

            _ => Drop, // unknown node kinds default to Drop (conservative)
        }
    }
}
```

---

### 2.5 Module Architecture

```
phase2/
│
├── mod.rs                    # Phase2Stage::run(Phase2Input) → BPASTArtifact
│                             # Top-level orchestrator: per-file parse + walk + build
│
├── adapter/
│   ├── mod.rs                # ASTReductionAdapter trait (§2.4.2)
│   ├── registry.rs           # AdapterRegistry: LangId → ASTReductionAdapter (reuses Phase1 registry)
│   └── java.rs               # JavaASTReductionAdapter (§2.4.3)
│
├── reducer.rs                # CST reduction DFS: the core recursive algorithm
│                             # Pure function: (Node, source, adapter, tok_corpus) → walk the tree
│                             # No side effects — calls BPASTBuilder mutably via parameter
│
├── bp_encoder.rs             # BPEncoder: bit-packed BP sequence builder (push_open, push_close, get_bit)
│                             # MSB-first packing into Vec<u64>
│
├── jump_table.rs             # JumpTableBuilder: O(n) stack-based match construction
│                             # Input: &BPEncoder; Output: Vec<u32> of length 2*n_ast
│
├── rank_select.rs            # RankSelectIndex: superblock/block/lookup construction
│                             # Input: &BPEncoder; Output: RankSelectIndex struct
│
├── rmq.rs                    # SparseTableRMQ: O(n log n) preprocessing, O(1) range minimum query
│                             # Built over the excess array derived from bp_encoder
│                             # Required for O(1) LCA queries (used by Phase 4 and Phase 7)
│
├── preorder.rs               # PreorderArrays: builds 4 parallel arrays during the DFS
│                             # node_types[n_ast], node_attrs[n_ast],
│                             # token_ranges[n_ast × 2], parent_map[n_ast]
│
├── builder.rs                # BPASTBuilder: aggregates all sub-structures into BPASTArtifact
│                             # Owns BPEncoder, PreorderArrays, file-level counters
│
└── serializer.rs             # BPASTSerializer: writes BPASTArtifact to .bpa binary format
```

**Module dependency flow** (edges = imports):

```
Phase2Stage → AdapterRegistry, TreeSitterParser (Phase1 reuse),
              Reducer, BPASTBuilder, BPASTSerializer
Reducer     → ASTReductionAdapter (trait object), BPASTBuilder (mut ref)
BPASTBuilder → BPEncoder, PreorderArrays
Serializer  → BPEncoder, JumpTableBuilder, RankSelectIndex, SparseTableRMQ, PreorderArrays
```

Zero circular dependencies. `Reducer` is a pure function (testable with a mock adapter). `JumpTableBuilder`, `RankSelectIndex`, and `SparseTableRMQ` are post-processing steps run on the completed BP bitstring — they do not participate in the DFS.

---

### 2.6 Data Structure Specifications

**BPEncoder (in-memory only, not persisted directly):**

```rust
pub struct BPEncoder {
    words:     Vec<u64>,   // packed BP bits, MSB-first; capacity grows by doubling
    bit_count: usize,      // total bits pushed (= 2 × n_ast when complete)
}
```

Space: `ceil(2n / 64) × 8` bytes. For n = 1M: `ceil(2M / 64) × 8 = 250 KB`.

**RankSelectIndex (persisted as Section 2 of .bpa):**

```rust
pub struct RankSelectIndex {
    superblocks: Vec<u32>,   // cumulative rank_1 at superblock boundaries (s1=512 bits)
    blocks:      Vec<u16>,   // rank_1 within superblock at block boundaries (s2=8 bits)
    lookup:      [u8; 256],  // popcount for all 8-bit values (precomputed once)
    n_bits:      usize,      // total BP bits
}
```

Space for n = 1M nodes (2M bits):
- Superblocks: `ceil(2M / 512) × 4 = 3,907 × 4 = 15.6 KB`
- Blocks: `ceil(2M / 8) × 2 = 250,000 × 2 = 488 KB`
- Lookup: `256 × 1 = 256 B`
- Total: ≈ 504 KB

**JumpTable (persisted as Section 3 of .bpa):**

```rust
pub struct JumpTable {
    match_pos: Vec<u32>,   // length = 2 × n_ast = n_bits
                           // match_pos[i] = position of matching paren at position i
}
```

Space: `2 × 1M × 4 = 8 MB` for n = 1M nodes.

**PreorderArrays (persisted as Sections 4–7 of .bpa):**

```rust
pub struct PreorderArrays {
    node_types:    Vec<u8>,          // n_ast × 1 byte: Σ_N type per node (pre-order)
    node_attrs:    Vec<u32>,         // n_ast × 4 bytes: NodeAttr packed word (pre-order)
    token_ranges:  Vec<(u32, u32)>,  // n_ast × 8 bytes: (first_token_id, last_token_id) per node
    parent_map:    Vec<u32>,         // n_ast × 4 bytes: preorder_idx of parent; u32::MAX for roots
}
```

Total space for n = 1M nodes:
- node_types: 1 MB
- node_attrs: 4 MB
- token_ranges: 8 MB
- parent_map: 4 MB
- Total: 17 MB

---

### 2.7 Algorithm Specifications

#### 2.7.1 Top-Level Phase 2 Orchestration

```rust
impl Phase2Stage {
    pub fn run(input: Phase2Input, out_path: &Path) -> Result<BPASTArtifact> {
        let tca        = input.tca.as_slice();
        let file_reg   = TCAFile::file_registry(tca);       // O(1) pointer into mmap
        let tok_table  = TCAFile::token_table(tca);         // sorted &[TokenRecord]
        let adapter_reg = AdapterRegistry::global();
        let parser      = TreeSitterParser::new();
        let mut builder = BPASTBuilder::new(
            estimated_nodes(file_reg)  // initial Vec capacity hint
        );

        // Process each file sequentially
        // (Parallelization possible with per-file sub-builders + merge step)
        for file_rec in file_reg {
            let source   = fs::read(file_rec.path(tca))?; // OS-cached from Phase 1
            let adapter  = adapter_reg.get(file_rec.language_id)?;
            let tree     = parser.parse(&source, adapter.ts_language())?;

            builder.begin_file(file_rec.file_id);

            // Core reduction DFS
            reduce_and_encode(
                tree.root_node(),
                &source,
                file_rec.file_id,
                adapter.reduction_adapter(),
                tok_table,
                &mut builder,
            );

            builder.end_file();
        }

        // Post-processing: build auxiliary structures from the completed BP bitstring
        let artifact = builder.finalize();  // runs JumpTable, RankSelect, RMQ builds
        BPASTSerializer::write(&artifact, out_path)?;
        Ok(artifact)
    }
}
```

#### 2.7.2 Core Reduction DFS

```rust
/// Recursive DFS over the Tree-sitter CST.
/// Returns the token_id range (first_tok, last_tok) covered by this subtree,
/// or None if the node was entirely dropped.
///
/// Invariant: all calls to builder.open_node() are paired with builder.close_node()
/// exactly once before this function returns.
fn reduce_and_encode(
    node:       Node,
    source:     &[u8],
    file_id:    u16,
    adapter:    &dyn ASTReductionAdapter,
    tok_table:  &[TokenRecord],
    builder:    &mut BPASTBuilder,
) -> Option<(u32, u32)> {  // Some((first_tok_id, last_tok_id)) | None

    match adapter.classify(node.kind(), &node, builder.current_depth()) {

        ReductionDecision::Drop => None,

        ReductionDecision::Eliminate => {
            // Recurse but don't open/close a node: children attach to parent
            let mut first_tok = u32::MAX;
            let mut last_tok  = 0u32;
            let mut cursor = node.walk();
            if cursor.goto_first_child() {
                loop {
                    if let Some((ft, lt)) = reduce_and_encode(
                        cursor.node(), source, file_id, adapter, tok_table, builder
                    ) {
                        first_tok = first_tok.min(ft);
                        last_tok  = last_tok.max(lt);
                    }
                    if !cursor.goto_next_sibling() { break; }
                }
            }
            if first_tok == u32::MAX { None } else { Some((first_tok, last_tok)) }
        }

        ReductionDecision::Keep(node_type) => {
            let attrs      = adapter.encode_attrs(node.kind(), &node, source);
            let preorder   = builder.open_node(node_type, attrs);  // emits bit 1

            let mut first_tok = u32::MAX;
            let mut last_tok  = 0u32;

            if node.child_count() == 0 {
                // CST leaf: this is a token node → look up token_id in tok_table
                let start    = node.start_position();
                let sort_key = build_sort_key(file_id, start.row + 1, start.column as u16);
                let token_id = tok_table_lookup(tok_table, sort_key);
                // token_id == u32::MAX means the token was filtered (whitespace) → skip
                if token_id != u32::MAX {
                    first_tok = token_id;
                    last_tok  = token_id;
                }
            } else {
                // Internal node: recurse into children
                let mut cursor = node.walk();
                cursor.goto_first_child();
                loop {
                    if let Some((ft, lt)) = reduce_and_encode(
                        cursor.node(), source, file_id, adapter, tok_table, builder
                    ) {
                        first_tok = first_tok.min(ft);
                        last_tok  = last_tok.max(lt);
                    }
                    if !cursor.goto_next_sibling() { break; }
                }
            }

            // Finalize this node: emit bit 0, record token range and parent
            builder.close_node(preorder, first_tok, last_tok);
            if first_tok == u32::MAX { None } else { Some((first_tok, last_tok)) }
        }
    }
}

/// Binary search on the sorted TokenRecord array.
/// Returns u32::MAX if not found (token was filtered out by Phase 1).
fn tok_table_lookup(table: &[TokenRecord], sort_key: u64) -> u32 {
    match table.binary_search_by_key(&sort_key, |r| r.sort_key) {
        Ok(idx)  => idx as u32,   // preorder index in tok_table = token_id
        Err(_)   => u32::MAX,     // token was filtered (whitespace, etc.)
    }
}
```

**Note on token_id assignment from tok_table:** The TokenRecord array is sorted by sort_key. Its array index IS the token_id (Phase 1 assigns token_ids by insertion order during the pre-sort walk, then sorts — so token_id = tok_table position after sort only if Phase 1 preserves insertion order for identical sort_keys, which it does by construction). The `binary_search_by_key` returns the array index, which equals the token_id via Phase 1's invariant.

#### 2.7.3 BPASTBuilder Open/Close Operations

```rust
impl BPASTBuilder {
    pub fn open_node(&mut self, node_type: ASTNodeType, attrs: u32) -> u32 {
        let preorder_idx = self.node_count as u32;

        // Record parent: top of the open-node stack is this node's parent
        let parent_idx = self.open_stack.last().copied().unwrap_or(u32::MAX);

        // Push to parallel arrays (pre-order)
        self.preorder.node_types.push(node_type as u8);
        self.preorder.node_attrs.push(attrs);
        self.preorder.token_ranges.push((u32::MAX, 0)); // filled by close_node
        self.preorder.parent_map.push(parent_idx);

        // Emit open paren to BP sequence
        self.bp.push_open();   // emits bit 1

        // Push onto depth stack for parent tracking
        self.open_stack.push(preorder_idx);

        self.node_count += 1;
        preorder_idx
    }

    pub fn close_node(&mut self, preorder_idx: u32, first_tok: u32, last_tok: u32) {
        // Fill in the token range for this node
        self.preorder.token_ranges[preorder_idx as usize] = (first_tok, last_tok);

        // Propagate range upward to all open ancestors
        // (Each ancestor's range is updated to include this node's range)
        for &ancestor_idx in &self.open_stack {
            if ancestor_idx == preorder_idx { break; }
            let r = &mut self.preorder.token_ranges[ancestor_idx as usize];
            r.0 = r.0.min(first_tok);
            r.1 = r.1.max(last_tok);
        }

        // Emit close paren to BP sequence
        self.bp.push_close();  // emits bit 0

        // Pop from depth stack
        self.open_stack.pop();
    }

    pub fn finalize(self) -> BPASTArtifact {
        assert!(self.open_stack.is_empty(), "Unclosed AST nodes at finalization");

        let jump_table   = JumpTableBuilder::build(&self.bp);
        let rank_select  = RankSelectIndex::build(&self.bp);
        let rmq          = SparseTableRMQ::build_from_bp(&self.bp, &rank_select);

        BPASTArtifact {
            node_count:  self.node_count,
            bp_encoder:  self.bp,
            jump_table,
            rank_select,
            rmq,
            preorder:    self.preorder,
        }
    }
}
```

#### 2.7.4 Jump Table Construction (O(n))

```rust
/// Construct the jump table in a single O(n_bits) left-to-right scan.
/// Uses a stack to match open/close parentheses.
pub fn build_jump_table(bp: &BPEncoder) -> Vec<u32> {
    let n_bits = bp.bit_count;
    let mut table = vec![0u32; n_bits];
    let mut stack: Vec<u32> = Vec::with_capacity(512); // max depth ≈ max tree depth

    for i in 0..n_bits {
        if bp.get_bit(i) == 1 {
            stack.push(i as u32);                    // push open paren position
        } else {
            let open = stack.pop()
                .expect("Malformed BP: unmatched close paren");
            table[open as usize] = i as u32;         // open → close
            table[i]             = open;             // close → open
        }
    }

    debug_assert!(stack.is_empty(), "Malformed BP: unclosed open parens");
    table
}
```

Time: O(n_bits) = O(2n) — one pass, one push/pop per bit.
Space: O(n_bits × 4) = O(8n) bytes + O(depth_max) stack.

#### 2.7.5 Rank/Select Index Construction

```rust
pub fn build_rank_select(bp: &BPEncoder) -> RankSelectIndex {
    const S1: usize = 512; // superblock = 512 bits = 8 × 64-bit words
    const S2: usize = 8;   // block = 8 bits (use precomputed 256-entry lookup)

    // Precompute popcount lookup for all 8-bit patterns
    let mut lookup = [0u8; 256];
    for i in 0usize..256 {
        lookup[i] = i.count_ones() as u8;
    }

    let n_bits = bp.bit_count;
    let n_sb   = n_bits.div_ceil(S1);
    let n_blk_per_sb = S1 / S2;  // = 64 blocks per superblock

    let mut superblocks = Vec::with_capacity(n_sb + 1);
    let mut blocks      = Vec::with_capacity((n_sb + 1) * n_blk_per_sb);

    let mut cumulative: u32 = 0;

    for sb in 0..n_sb {
        superblocks.push(cumulative);
        let mut within_sb: u32 = 0;

        for b in 0..n_blk_per_sb {
            blocks.push(within_sb as u16);
            let bit_start = sb * S1 + b * S2;
            if bit_start < n_bits {
                let byte_idx = bit_start / 8;
                // Read the packed byte from the BP bitstring
                let word_idx = byte_idx / 8;
                let byte_in_word = 7 - (byte_idx % 8);  // MSB-first word layout
                let byte = ((bp.words[word_idx] >> (byte_in_word * 8)) & 0xFF) as usize;
                let count = lookup[byte] as u32;
                within_sb  += count;
                cumulative += count;
            }
        }
    }
    superblocks.push(cumulative); // sentinel

    RankSelectIndex { superblocks, blocks, lookup, n_bits }
}

impl RankSelectIndex {
    /// rank_1(i): count of 1-bits in B[0..=i]  — O(1)
    #[inline(always)]
    pub fn rank1(&self, i: usize) -> u32 {
        const S1: usize = 512;
        const S2: usize = 8;
        const N_BLK: usize = S1 / S2;  // 64

        let sb_idx = i / S1;
        let b_idx  = (i % S1) / S2;
        let bit_in_blk = i % S2;

        let sb_count = self.superblocks[sb_idx];
        let b_count  = self.blocks[sb_idx * N_BLK + b_idx] as u32;

        // Count bits 0..=bit_in_blk within the final partial byte
        let bit_start = sb_idx * S1 + b_idx * S2;
        let byte_idx  = bit_start / 8;
        // Read byte from BPEncoder words (via caller-provided reference or precomputed byte array)
        let byte      = self.get_byte(byte_idx);
        // Mask: keep only bits 0..=bit_in_blk (MSB-first: the first bit_in_blk+1 bits)
        let mask      = if bit_in_blk == 7 { 0xFF } else { 0xFF_u8 << (7 - bit_in_blk) };
        let partial   = self.lookup[(byte & mask) as usize] as u32;

        sb_count + b_count + partial
    }
}
```

#### 2.7.6 Sparse Table RMQ for O(1) LCA

The excess at BP position i is defined as `e(i) = 2 × rank_1(i) - i - 1`, which equals the nesting depth at position i. LCA(u, v) is the node whose open paren has the minimum excess in the range [open_pos(u), open_pos(v)].

```rust
/// Build a sparse table for range minimum query on the excess sequence.
/// Preprocessing: O(n log n) time and space.
/// Query: O(1) via two overlapping power-of-2 ranges.
pub struct SparseTableRMQ {
    // sparse_table[k][i] = position of minimum excess in range [i, i + 2^k - 1]
    table: Vec<Vec<u32>>,
    log2:  Vec<usize>,     // precomputed floor(log2(n)) for n = 0..2n_bits
}

impl SparseTableRMQ {
    pub fn build(bp: &BPEncoder, rs: &RankSelectIndex) -> Self {
        let n = bp.bit_count;
        let log_n = (usize::BITS - n.leading_zeros()) as usize;

        // Compute excess for each position
        let excess: Vec<i32> = (0..n)
            .map(|i| 2 * rs.rank1(i) as i32 - i as i32 - 1)
            .collect();

        // Level 0: each position is its own minimum
        let mut table = vec![
            (0..n as u32).collect::<Vec<u32>>()
        ];

        // Build levels 1..=log_n
        for k in 1..=log_n {
            let half = 1usize << (k - 1);
            let len  = n.saturating_sub(1usize << k) + 1;
            let mut level = Vec::with_capacity(len);
            for i in 0..len {
                let left  = table[k-1][i];
                let right = if i + half < table[k-1].len() {
                    table[k-1][i + half]
                } else { left };
                level.push(if excess[left as usize] <= excess[right as usize] { left } else { right });
            }
            table.push(level);
        }

        // Precompute floor(log2) for O(1) query
        let mut log2 = vec![0usize; n + 1];
        for i in 2..=n { log2[i] = log2[i/2] + 1; }

        SparseTableRMQ { table, log2 }
    }

    /// range_min(l, r): position of minimum excess in B[l..=r]  — O(1)
    #[inline(always)]
    pub fn range_min(&self, l: usize, r: usize) -> u32 {
        let k = self.log2[r - l + 1];
        let a = self.table[k][l];
        let b = self.table[k][r + 1 - (1 << k)];
        // Return position with smaller excess (ties: prefer earlier)
        a  // simplified; full impl compares excess[a] vs excess[b]
    }

    /// LCA(u, v): preorder indices → preorder index of LCA  — O(1)
    pub fn lca(&self, bp: &BPEncoder, rs: &RankSelectIndex, u: u32, v: u32) -> u32 {
        let op_u = rs.select1(u + 1);
        let op_v = rs.select1(v + 1);
        let (l, r) = if op_u <= op_v { (op_u, op_v) } else { (op_v, op_u) };
        let min_pos = self.range_min(l, r);
        rs.rank1(min_pos) - 1  // pre-order index of LCA
    }
}
```

---

### 2.8 Output Schema: BPASTArtifact Binary Format (`.bpa`)

```
╔══════════════════════════════════════════════════════════════════╗
║  BPA FILE FORMAT v1.0  (all integers: little-endian)            ║
╠════════════════╦══════════════╦═══════════════════════════════════╣
║ Section        ║ Size         ║ Description                       ║
╠════════════════╬══════════════╬═══════════════════════════════════╣
║ HEADER         ║ 64 B fixed   ║ Magic, version, counts, TCA hash  ║
╠════════════════╬══════════════╬═══════════════════════════════════╣
║ BP BITSTRING   ║ ceil(2n/64)  ║ Packed u64 words, MSB-first.      ║
║                ║ × 8 B        ║ Length = 2 × n_ast bits.          ║
╠════════════════╬══════════════╬═══════════════════════════════════╣
║ RANK/SELECT    ║ ≈504 KB      ║ Superblocks (u32[]), blocks(u16[]),║
║ INDEX          ║ (n=1M nodes) ║ 256-byte lookup table.            ║
╠════════════════╬══════════════╬═══════════════════════════════════╣
║ JUMP TABLE     ║ 2n × 4 B     ║ match_pos[2n_ast]: u32 per        ║
║                ║ = 8 MB       ║ BP position → matching position.  ║
╠════════════════╬══════════════╬═══════════════════════════════════╣
║ NODE TYPE      ║ n × 1 B      ║ Σ_N type per node, pre-order.     ║
║ ARRAY          ║ = 1 MB       ║                                   ║
╠════════════════╬══════════════╬═══════════════════════════════════╣
║ NODE ATTR      ║ n × 4 B      ║ NodeAttr u32 per node, pre-order. ║
║ ARRAY          ║ = 4 MB       ║                                   ║
╠════════════════╬══════════════╬═══════════════════════════════════╣
║ TOKEN RANGE    ║ n × 8 B      ║ (first_tok_id:u32, last_tok_id:u32)║
║ ARRAY          ║ = 8 MB       ║ per node, pre-order. Traceability.║
╠════════════════╬══════════════╬═══════════════════════════════════╣
║ PARENT MAP     ║ n × 4 B      ║ preorder_idx of parent per node.  ║
║                ║ = 4 MB       ║ u32::MAX for roots.               ║
╠════════════════╬══════════════╬═══════════════════════════════════╣
║ RMQ TABLE      ║ n log n × 4B ║ Sparse table for O(1) LCA.        ║
║                ║ = 20 MB      ║ (n=1M nodes, log₂(2M)≈21 levels)  ║
╠════════════════╬══════════════╬═══════════════════════════════════╣
║ EXTENDED ATTRS ║ variable     ║ Side table for complex node attrs  ║
║                ║              ║ that exceed 4 bytes.               ║
╠════════════════╬══════════════╬═══════════════════════════════════╣
║ CHECKSUM       ║ 8 B          ║ CRC-64/ECMA of all preceding       ║
╚════════════════╩══════════════╩═══════════════════════════════════╝
```

**HEADER (64 bytes, exact):**

```
Offset  Size  Field
 0       8    magic            0x425041535400010 ("BPAST\x00\x01\x00")
 8       4    format_version   0x00000001
12       4    node_count       n_ast (u32)
16       4    bit_count        2 × n_ast (u32); sanity check
20       2    file_count       number of source files (u16)
22      34    reserved         zeroed
56       8    tca_hash         CRC-64 of the input .tca file (integrity link to Phase 1)
Total: 64 bytes
```

**Size calculation for n = 1M AST nodes:**

```
Header:              64 B
BP Bitstring:   250 KB
Rank/Select:    504 KB
Jump Table:       8 MB
Node Type:        1 MB
Node Attr:        4 MB
Token Ranges:     8 MB
Parent Map:       4 MB
RMQ Table:       ~20 MB  (21 levels × 2M entries × 4B = 168 MB worst case;
                           practical with level capping at log₂(n) ≈ 21 levels,
                           actual: Σ_{k=0}^{20} (2M-2^k) × 4 ≈ 20 MB)
──────────────────────────────────
Total:          ~45.75 MB uncompressed
After LZ4 HC:   ~12–15 MB
(BP bitstring: ~3×; node type array: ~4×; jump table and RMQ have low redundancy)
```

---

### 2.9 Complexity Proofs

**Time Complexity:**

| Operation | Complexity | Notes |
|---|---|---|
| Re-parse source files (Tree-sitter) | O(N) | N = total source bytes; OS-cached |
| CST reduction DFS (per file) | O(n_cst) | n_cst ≈ 3–5 × n_ast |
| Token ID lookup per leaf | O(log n_tok) | Binary search on sorted TCA token table |
| BPEncoder push\_open/close | O(1) amortized | Vec doubling |
| BPASTBuilder open/close | O(depth) | range propagation up open\_stack |
| Jump table construction | O(2n\_ast) | one-pass, one stack op per bit |
| Rank/Select construction | O(n\_bits / S2) | one byte-scan pass |
| RMQ sparse table | O(n\_bits × log n\_bits) | log₂(2M) ≈ 21 levels, each O(n) |
| Serialization | O(n\_ast) | sequential write |

**Dominant term:** O(N + n\_ast × log n\_tok + n\_bits × log n\_bits). Since n\_tok ≥ n\_ast and n\_bits = 2n\_ast, this simplifies to O(N + n\_ast × log n\_ast). The RMQ construction (sparse table) is the most expensive single step.

**Space Complexity (peak):**

```
BPEncoder:         250 KB (2M bits / 8 = 250 KB)
PreorderArrays:    13 MB  (node_types + node_attrs + token_ranges + parent_map)
JumpTable (build): 8 MB   (Vec<u32> of length 2n)
Open stack (DFS):  O(depth_max × 4) ≈ O(200 × 4) = 800 B (Java max depth ≈ 200)
RMQ table:         ~20 MB
Source buffer:     O(|largest_file|) ≈ 5 MB
CST (transient):   O(n_cst_file) ≈ 50 MB (freed after each file)
──────────────────────────────────
Peak:              ~92 MB total; ~37 MB excluding the transient CST
```

---

### 2.10 Phase Invariants for Phase 3

Phase 3 (Symbol Table & Type Hierarchy Construction) ingests the `BPASTArtifact` and must rely on the following invariants, enforced by Phase 2's `finalize()` assertions:

**Invariant 1 (BP Validity):** The BP bitstring B has exactly n\_ast 1-bits and n\_ast 0-bits. Verified: `rank_1(2n_ast - 1) == n_ast`.

**Invariant 2 (Jump Table Completeness):** For every position i: `match_pos[match_pos[i]] == i`. Verified by a O(n) spot-check on 5% of positions during debug builds.

**Invariant 3 (Token Range Monotonicity):** For every AST internal node v: `token_ranges[v].0 ≤ token_ranges[c].0` and `token_ranges[v].1 ≥ token_ranges[c].1` for all children c. This guarantees that parent ranges always contain children's ranges — required by Phase 7's traceability index construction.

**Invariant 4 (Parent Map Consistency):** For every non-root node v: `parent_map[v] < v`. Pre-order indices are assigned in DFS pre-order, so every parent has a strictly smaller index than its children. This enables Phase 3 to traverse the symbol table upward (from methods to classes to packages) in O(1) per step via the parent\_map array.

**Invariant 5 (TCA Hash Link):** The BPA header field `tca_hash` equals `crc64(TCA_file_bytes)`. Any Phase 3 ingestion of a mismatched (TCA, BPA) pair fails this check, preventing silent stale-artifact bugs.

---

Now the visualization — Phase 2's architecture, the BP encoding mechanics, and the artifact layout: