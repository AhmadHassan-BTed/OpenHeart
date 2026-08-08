---

## Phase 1: Lexical Ingestion & Token Corpus Construction — Full Specification

### 1.1 Phase Mandate & Exact Boundaries

Phase 1 is the only phase that ever touches raw source bytes. Its single contractual obligation is: consume source text, produce an immutable, mathematically stable Token Corpus. The `token_id` assigned in this phase is the **universal traceability anchor** — a monotonic u32 that propagates upward through all nine subsequent phases and is embedded into every generated UML element. No upstream phase ever re-assigns or re-interprets a token_id.

Phase 1 explicitly does NOT perform AST reduction (Phase 2), semantic resolution (Phase 3), or any graph construction (Phases 4–6). Its output is a pure lexical artifact: source text mapped to a typed, indexed, sorted token corpus.

---

### 1.2 Input Schema: `SourceManifest`

```rust
/// The sole input to Phase 1. Constructed by the caller (IDE plugin,
/// CLI tool, or CI orchestrator) before the pipeline is invoked.
pub struct SourceManifest {
    /// Absolute paths to all source files to be ingested.
    /// Order is irrelevant — Phase 1 assigns file_ids deterministically
    /// by sorting paths lexicographically before ID assignment.
    pub file_paths: Vec<PathBuf>,

    /// Optional: force a specific language for a file extension.
    /// e.g., { ".kt" => LangId::Kotlin }
    /// If absent, language is auto-detected from extension via AdapterRegistry.
    pub language_overrides: HashMap<OsString, LangId>,

    /// Filtering configuration: which token types to retain.
    /// Skipping whitespace and comments is the default for pipeline efficiency.
    pub filter: TokenFilter,
}

pub struct TokenFilter {
    pub include_whitespace:     bool,  // default: false
    pub include_line_comments:  bool,  // default: false
    pub include_block_comments: bool,  // default: false
    pub include_doc_comments:   bool,  // default: true (Javadoc → UML metadata)
}
```

The `SourceManifest` is the only mutable object in the entire OpenHeart pipeline. Once Phase 1 begins, it is consumed and never modified.

---

### 1.3 Mathematical Foundations

#### 1.3.1 Token Type Alphabet Σ_T

Σ_T is a finite set encoded as a u8 (256 values). The high bit partitions the space: 0x00–0x7F are language-agnostic core types; 0x80–0xFF are language-specific extension types assigned per `LangId`.

```
Core token types (0x00–0x7F):
  0x00  TT_UNKNOWN
  0x01  TT_IDENTIFIER          # variable names, class names, method names
  0x02  TT_KEYWORD             # if, for, class, return, public, ...
  0x03  TT_OPERATOR            # +, -, *, /, ==, !=, <=, &&, ||, ...
  0x04  TT_PUNCTUATION         # ; , . ( ) [ ] { } < >
  0x05  TT_INTEGER_LITERAL     # 42, 0xFF, 0b1010, 1_000_000L
  0x06  TT_FLOAT_LITERAL       # 3.14f, 2.7e10, 1.0d
  0x07  TT_STRING_LITERAL      # "hello", """text block"""
  0x08  TT_CHAR_LITERAL        # 'a', '\n'
  0x09  TT_BOOLEAN_LITERAL     # true, false
  0x0A  TT_NULL_LITERAL        # null (Java), nil (Kotlin/Swift), None (Python)
  0x0B  TT_COMMENT_LINE        # // line comment
  0x0C  TT_COMMENT_BLOCK       # /* block comment */
  0x0D  TT_COMMENT_DOC         # /** Javadoc */ (retained by default)
  0x0E  TT_WHITESPACE          # space, tab (configurable skip)
  0x0F  TT_NEWLINE             # \n, \r\n (configurable skip)
  0x10  TT_ANNOTATION          # @Override, @SuppressWarnings
  0x11  TT_TYPE_PARAMETER      # <T>, <E extends Comparable>
  0x12  TT_LABELED_STMT        # label: in switch/loop
  0x13–0x7F  reserved for future core types

Java-specific extension range (LangId::Java → 0x80–0xFF):
  0x80  TT_JAVA_ANNOTATION_MARKER    # the '@' prefix of @Annotation
  0x81  TT_JAVA_GENERIC_DIAMOND     # <> in new ArrayList<>()
  0x82  TT_JAVA_VAR_KEYWORD         # 'var' in local variable type inference
  0x83  TT_JAVA_SEALED_KEYWORD      # 'sealed', 'permits', 'non-sealed'
  0x84–0xFF  reserved for further Java extensions
```

#### 1.3.2 TokenRecord Layout (16 bytes, cache-line aligned)

Every 4 consecutive `TokenRecord` values fit in a single 64-byte cache line, maximizing binary search performance.

```
┌─────────────────────────────────────────────────────┐
│  TokenRecord  (16 bytes)                            │
├──────────┬──────────────────────────────────────────┤
│ Offset 0 │ sort_key   : u64  (8 bytes)              │
│ Offset 8 │ text_id    : u32  (4 bytes)              │
│ Offset 12│ len        : u16  (2 bytes)              │
│ Offset 14│ token_type : u8   (1 byte)               │
│ Offset 15│ _padding   : u8 = 0x00 (1 byte)          │
└──────────┴──────────────────────────────────────────┘
```

**sort_key bit layout (u64):**

```
Bit 63 ────── Bit 48    Bit 47 ──── Bit 24    Bit 23 ─── Bit 8    Bit 7 ── Bit 0
│   file_id   │          │   line     │         │    col   │        │ flags │
│   u16       │          │   u24      │         │   u16    │        │  u8   │
│  (65,536    │          │  (16.7M    │         │ (65,536  │        │  (0)  │
│   files)    │          │   lines)   │         │  cols)   │        │       │
```

**sort_key construction (Rust):**

```rust
#[inline(always)]
pub fn build_sort_key(file_id: u16, line: u32, col: u16) -> u64 {
    // line is 1-indexed from Tree-sitter (row + 1)
    // Preconditions: line <= 0x00FFFFFF (24-bit max = 16,777,215)
    //               col  <= 0x0000FFFF (16-bit max = 65,535)
    debug_assert!(line <= 0x00FF_FFFF, "line number exceeds 24-bit range");
    ((file_id as u64) << 48)
    | ((line   as u64) << 24)
    | ((col    as u64) <<  8)
    // bits 7-0 (flags) = 0 for sort stability
}
```

This packing guarantees lexicographic sort order on (file_id, line, col) via a single u64 comparison — the sort is branchless and SIMD-vectorizable.

**Forward lookup proof (O(log n)):** The sorted `TokenRecord` array acts as its own Forward Index. Given a source position (file_id, line, col), construct the sort_key and run `lower_bound` binary search on the array. Search space: n values, each comparison is one 64-bit integer compare → T_fwd = O(log n) with constant ≈ 1 ns on modern hardware (2 cache lines touched at array midpoint per step).

**Backward lookup proof (O(1)):** The `TokenEntry` array is indexed directly by `token_id`. Since token_ids are monotonically assigned during Phase 1, `BI[token_id]` is a direct array index. Zero pointer chasing, zero hash computation → T_bwd = O(1).

#### 1.3.3 StringInterner: FNV-1a Hash Table

The `StringInterner` deduplicates all token text. In a typical Java codebase of 500K LOC, empirically 25–40% of all tokens are identical strings (keywords, common identifiers, operators). Deduplication reduces text storage by 3–5×.

**Hash function — FNV-1a 64-bit:**

FNV-1a is chosen over SipHash (used by Rust's standard HashMap) because: (1) it is avalanche-free for short strings (most tokens are 1–30 characters), (2) it has zero state initialization overhead, (3) it produces well-distributed hashes for the ASCII-dominated token text.

```rust
const FNV1A_PRIME:  u64 = 0x0000_0100_0000_01B3;
const FNV1A_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;

#[inline]
pub fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash = FNV1A_OFFSET;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(FNV1A_PRIME);
    }
    hash
}
```

**Table structure:**

```rust
pub struct StringInterner {
    /// Open-addressing hash table: slot = (hash:u64, text_id:u32)
    /// Sentinel for empty slot: hash = 0, text_id = u32::MAX
    table:        Vec<(u64, u32)>,
    table_mask:   usize,       // capacity - 1, capacity always a power of 2
    count:        u32,         // number of interned strings
    load_limit:   usize,       // resize when count > capacity * 0.75

    /// Contiguous string storage: [u16_len][utf8_bytes...]
    storage:      Vec<u8>,
}

impl StringInterner {
    pub fn intern(&mut self, text: &[u8]) -> u32 {
        let hash = fnv1a_64(text);
        // Ensure hash != 0 (reserved as empty sentinel)
        let hash = if hash == 0 { 1 } else { hash };

        let mut slot = (hash as usize) & self.table_mask;
        loop {
            let (h, id) = self.table[slot];
            if h == 0 {
                // Empty slot: insert new string
                let text_id = self.count;
                self.store_string(text);
                self.table[slot] = (hash, text_id);
                self.count += 1;
                if self.count > self.load_limit {
                    self.resize(); // Double capacity, rehash all
                }
                return text_id;
            }
            if h == hash && self.lookup_text(id) == text {
                return id; // Hash AND content match: existing string
            }
            // Collision: linear probe
            slot = (slot + 1) & self.table_mask;
        }
    }

    fn store_string(&mut self, text: &[u8]) {
        let len = text.len() as u16;
        self.storage.extend_from_slice(&len.to_le_bytes());
        self.storage.extend_from_slice(text);
    }

    fn lookup_text(&self, text_id: u32) -> &[u8] {
        // Offset lookup: requires offset table (u32 per string)
        // stored in a parallel Vec<u32> for O(1) random access
        let offset = self.offsets[text_id as usize] as usize;
        let len = u16::from_le_bytes(
            self.storage[offset..offset+2].try_into().unwrap()
        ) as usize;
        &self.storage[offset+2..offset+2+len]
    }
}
```

**Load factor proof (amortized O(1)):** Open-addressing with linear probing at load factor α = 0.75 gives expected probe length `1/(1-α) = 4` for lookups. Resize doubles capacity and rehashes: total work over n insertions is O(n) amortized (geometric series: n + n/2 + n/4 + ... = 2n). Combined with O(|text|) FNV-1a: T_intern = O(|text|) amortized per insertion.

---

### 1.4 Module Architecture

Strict separation of concerns. Each file has exactly one responsibility. No module imports from a sibling module — all dependencies flow downward (core → phase1/adapter → phase1/parser → phase1/walker → phase1/builder → phase1/serializer).

```
openheart/
│
├── core/                             # Language-agnostic, imported by ALL phases
│   ├── types/
│   │   ├── token.rs                  # TokenRecord, TokenEntry, TokenType (Σ_T), LangId
│   │   ├── source.rs                 # SourceFileRecord, SourceManifest, TokenFilter
│   │   └── artifact.rs               # Artifact trait (all phase outputs implement this)
│   └── io/
│       ├── binary.rs                 # LittleEndian BinaryWriter + BinaryReader
│       └── mmap.rs                   # MemoryMappedFile (read-only mmap wrapper)
│
└── phase1/
    ├── mod.rs                        # Phase1Stage::run(manifest) → TokenCorpusArtifact
    │                                 # Orchestrates sub-modules, owns the pipeline
    │
    ├── manifest.rs                   # SourceManifestBuilder
    │                                 # Discovers files on disk, sorts paths for
    │                                 # deterministic file_id assignment
    │
    ├── adapter/
    │   ├── mod.rs                    # LanguageAdapter trait (see §1.6.1)
    │   ├── registry.rs               # AdapterRegistry: extension → LanguageAdapter
    │   │                             # dispatch; auto-detects language from file ext
    │   └── java.rs                   # JavaLanguageAdapter (first concrete impl)
    │                                 # Maps tree-sitter Java node types → Σ_T
    │
    ├── parser/
    │   ├── mod.rs                    # CSTParser trait: parse(file_path) → TSTree
    │   └── tree_sitter.rs            # TreeSitterParser: wraps tree-sitter C API via FFI
    │                                 # One Parser instance per thread (not Send)
    │
    ├── walker.rs                     # CSTWalker: depth-first CST traversal
    │                                 # Emits raw token stream for one file
    │                                 # Pure function: (TSTree, source, adapter) → Vec<RawToken>
    │
    ├── allocator.rs                  # TokenIdAllocator
    │                                 # AtomicU32 counter; supports parallel file processing
    │                                 # Guarantees global monotonicity across threads
    │
    ├── interner.rs                   # StringInterner (defined above)
    │                                 # Single-threaded; wrapped in Mutex for parallel use
    │
    ├── builder.rs                    # TokenCorpusBuilder
    │                                 # Accumulates TokenRecord + TokenEntry pairs;
    │                                 # performs the final sort; builds FileRegistry
    │
    └── serializer.rs                 # TokenCorpusSerializer
                                      # Writes TokenCorpusArtifact to disk in the
                                      # binary format specified in §1.7
```

**Dependency graph** (edges = "imports from"):

```
Phase1Stage → ManifestBuilder, AdapterRegistry, TreeSitterParser,
              CSTWalker, TokenIdAllocator, StringInterner,
              TokenCorpusBuilder, TokenCorpusSerializer
CSTWalker  → LanguageAdapter (trait object)
JavaAdapter → core::types::token (Σ_T types)
All        → core::types, core::io
```

Zero circular dependencies. Every module is independently unit-testable with mock implementations of its trait dependencies.

---

### 1.5 Data Structure Specifications (Complete Byte Layouts)

**SourceFileRecord (64 bytes, fixed):**

```
Offset  Size  Type     Field
 0       2    u16_le   file_id           (0-indexed, assigned by sorted path order)
 2       1    u8       language_id       (LangId enum: Java=0x01, Kotlin=0x02, ...)
 3       1    u8       flags             (bit 0: has_bom, bit 1: crlf_line_endings)
 4      32    [u8;32]  content_sha256    (SHA-256 of raw file bytes)
36       4    u32_le   path_str_offset   (byte offset into path string section)
40       8    u64_le   file_size_bytes
48       8    u64_le   mtime_ns          (last modified, Unix nanoseconds)
56       4    u32_le   first_token_id    (token_id of first token in this file)
60       4    u32_le   file_token_count  (number of tokens belonging to this file)
             ───────
             64 bytes total
```

**TokenRecord (16 bytes):** Defined in §1.3.2.

**TokenEntry (16 bytes, Backward Index — indexed by token_id):**

```
Offset  Size  Type     Field
 0       8    u64_le   sort_key      (identical to TokenRecord.sort_key)
 8       4    u32_le   text_id
12       2    u16_le   len
14       1    u8       token_type
15       1    u8       _padding = 0
```

`TokenEntry` is structurally identical to `TokenRecord`. They are separate types because their access patterns differ: `TokenRecord` arrays are sorted and binary-searched; `TokenEntry` arrays are randomly accessed by token_id index. In a future optimization, the two can be aliased if profiling shows the distinction is unnecessary.

**RawToken (transient, not persisted):**

```rust
/// Emitted by CSTWalker, consumed immediately by TokenCorpusBuilder.
/// Lives only in the hot path of Phase 1; never serialized.
struct RawToken {
    file_id:    u16,
    line:       u32,       // 1-indexed
    col:        u16,
    len:        u16,
    token_type: TokenType,
    text:       &[u8],     // slice into source buffer (zero-copy)
}
```

---

### 1.6 Algorithm Specifications

#### 1.6.1 LanguageAdapter Trait

```rust
/// The language-agnostic interface that all language frontends implement.
/// Implementations MUST be stateless (or use interior mutability) to
/// support multi-threaded parallel file processing.
pub trait LanguageAdapter: Send + Sync + 'static {

    /// Unique language identifier (matches LangId enum).
    fn language_id(&self) -> LangId;

    /// File extensions this adapter handles (lowercase, without leading dot).
    /// e.g., &["java"] for JavaLanguageAdapter
    fn file_extensions(&self) -> &[&str];

    /// Returns the tree-sitter language descriptor for this language.
    fn ts_language(&self) -> tree_sitter::Language;

    /// Maps a tree-sitter node type name to a Σ_T TokenType.
    /// Called for every leaf node during CSTWalker traversal.
    /// Must be O(1) — implemented as a perfect hash map or a match
    /// statement compiled to a jump table by the optimizer.
    fn map_node_type(&self, ts_node_kind: &str) -> TokenType;

    /// Returns true if an anonymous (non-named) tree-sitter node of the
    /// given kind should be included as a token.
    /// e.g., Java's '{' and '}' are anonymous but must be retained for AST.
    fn include_anonymous(&self, ts_node_kind: &str) -> bool;

    /// Returns true if this token type should be excluded from the corpus.
    /// Driven by the SourceManifest::TokenFilter configuration.
    fn should_skip(&self, token_type: TokenType, filter: &TokenFilter) -> bool;
}
```

**Java adapter type mapping (selected entries):**

```rust
impl LanguageAdapter for JavaLanguageAdapter {
    fn map_node_type(&self, kind: &str) -> TokenType {
        // This match compiles to a perfect hash lookup (rustc optimizes
        // small string matches to jump tables via LLVM)
        match kind {
            "identifier"                => TokenType::Identifier,
            "type_identifier"           => TokenType::Identifier,
            "decimal_integer_literal"   => TokenType::IntegerLiteral,
            "hex_integer_literal"       => TokenType::IntegerLiteral,
            "binary_integer_literal"    => TokenType::IntegerLiteral,
            "decimal_floating_point_literal" => TokenType::FloatLiteral,
            "string_literal"            => TokenType::StringLiteral,
            "text_block"                => TokenType::StringLiteral,
            "character_literal"         => TokenType::CharLiteral,
            "true" | "false"            => TokenType::BooleanLiteral,
            "null_literal"              => TokenType::NullLiteral,
            "line_comment"              => TokenType::CommentLine,
            "block_comment"             => TokenType::CommentBlock,
            "comment"                   => TokenType::CommentBlock,
            // All Java operators
            "+" | "-" | "*" | "/" | "%" => TokenType::Operator,
            "==" | "!=" | "<" | ">" | "<=" | ">=" => TokenType::Operator,
            "&&" | "||" | "!" | "&" | "|" | "^"  => TokenType::Operator,
            "++" | "--" | "~" | "<<" | ">>" | ">>>" => TokenType::Operator,
            "=" | "+=" | "-=" | "*=" | "/=" | "%=" => TokenType::Operator,
            // Java punctuation
            ";" | "," | "." | "(" | ")" | "[" | "]" | "{" | "}" => TokenType::Punctuation,
            "..." | "::" | "->"  => TokenType::Punctuation,
            // Java keywords
            "if" | "else" | "for" | "while" | "do" | "switch" => TokenType::Keyword,
            "case" | "break" | "continue" | "return" | "throw" => TokenType::Keyword,
            "try" | "catch" | "finally" | "new" | "instanceof" => TokenType::Keyword,
            "class" | "interface" | "enum" | "record"          => TokenType::Keyword,
            "extends" | "implements" | "throws" | "import"     => TokenType::Keyword,
            "public" | "private" | "protected" | "static"      => TokenType::Keyword,
            "final" | "abstract" | "native" | "synchronized"   => TokenType::Keyword,
            "volatile" | "transient" | "strictfp" | "default"  => TokenType::Keyword,
            "void" | "boolean" | "byte" | "char" | "short"     => TokenType::Keyword,
            "int" | "long" | "float" | "double"                => TokenType::Keyword,
            "this" | "super" | "package" | "assert" | "yield"  => TokenType::Keyword,
            "var"                => TokenType::JavaVarKeyword,  // 0x82
            "marker_annotation"
            | "annotation"       => TokenType::Annotation,
            "@"                  => TokenType::JavaAnnotationMarker, // 0x80
            _                    => TokenType::Unknown,
        }
    }
}
```

#### 1.6.2 CSTWalker Algorithm

The walker is a pure function — it takes immutable inputs and returns a `Vec<RawToken>`. No side effects, fully unit-testable in isolation.

```rust
/// Walk the tree-sitter CST for one file, extracting all leaf tokens.
/// Returns tokens in source order (DFS left-to-right pre-order).
///
/// Complexity: O(n_cst) time, O(depth_max) stack space (DFS recursion depth)
/// For Java: depth_max ≤ 50 for typical code, never exceeds 200.
pub fn walk_cst(
    node:    Node,             // tree-sitter node (copy, no lifetime)
    source:  &[u8],           // raw source bytes for this file
    file_id: u16,
    adapter: &dyn LanguageAdapter,
    filter:  &TokenFilter,
) -> Vec<RawToken> {
    let mut tokens = Vec::with_capacity(node.descendant_count());
    walk_recursive(node, source, file_id, adapter, filter, &mut tokens);
    tokens
}

fn walk_recursive(
    node:    Node,
    source:  &[u8],
    file_id: u16,
    adapter: &dyn LanguageAdapter,
    filter:  &TokenFilter,
    out:     &mut Vec<RawToken>,
) {
    if node.child_count() == 0 {
        // ── LEAF NODE = TOKEN ─────────────────────────────────────────
        let ts_kind = node.kind();

        // For anonymous nodes (punctuation, operators), check inclusion
        if !node.is_named() && !adapter.include_anonymous(ts_kind) {
            return;
        }

        let token_type = adapter.map_node_type(ts_kind);

        // Apply token filter from SourceManifest
        if adapter.should_skip(token_type, filter) {
            return;
        }

        let start     = node.start_position();
        let byte_range = node.byte_range();

        // len is byte length of the token in source
        // For ASCII (all Java tokens), byte_len == char_len.
        // For Unicode identifiers (legal in Java), byte_len >= char_len.
        // We store byte length here; Phase 9 UML metadata handles char count.
        let len = (byte_range.end - byte_range.start).min(u16::MAX as usize) as u16;

        out.push(RawToken {
            file_id,
            line:       (start.row + 1) as u32,   // 1-indexed
            col:        start.column as u16,
            len,
            token_type,
            text_start: byte_range.start,          // offset into source buffer
            text_len:   len as usize,
        });
        return;
    }

    // ── INTERNAL NODE = RECURSE INTO CHILDREN ─────────────────────────
    let mut cursor = node.walk();
    cursor.goto_first_child();
    loop {
        walk_recursive(cursor.node(), source, file_id, adapter, filter, out);
        if !cursor.goto_next_sibling() { break; }
    }
}
```

#### 1.6.3 Phase1Stage Orchestrator (Top-Level Algorithm)

```rust
impl Phase1Stage {
    /// Entry point. Consumes the SourceManifest, produces TokenCorpusArtifact.
    /// The entire Phase 1 contract is fulfilled by this function.
    pub fn run(manifest: SourceManifest, out_path: &Path) -> Result<TokenCorpusArtifact> {

        // Step 1: Normalize and sort file paths for deterministic file_id assignment
        let mut files: Vec<PathBuf> = manifest.file_paths;
        files.sort_unstable();   // O(F log F), F = file count

        // Step 2: Build file registry (assigns file_ids 0..F-1)
        let mut file_records: Vec<SourceFileRecord> = Vec::with_capacity(files.len());
        for (file_id, path) in files.iter().enumerate() {
            let meta    = fs::metadata(path)?;
            let content = fs::read(path)?;  // read file into memory
            let sha256  = sha256_hash(&content);
            let lang_id = AdapterRegistry::detect(&manifest.language_overrides, path)?;
            file_records.push(SourceFileRecord {
                file_id: file_id as u16,
                language_id: lang_id,
                content_sha256: sha256,
                path_str_offset: 0,    // filled during serialization
                file_size_bytes: meta.len(),
                mtime_ns: mtime_to_nanos(meta.modified()?),
                first_token_id: 0,     // filled after walking
                file_token_count: 0,   // filled after walking
            });
        }

        // Step 3: Initialize shared state
        let allocator = TokenIdAllocator::new();   // AtomicU32
        let interner  = Mutex::new(StringInterner::with_capacity(65536));
        let mut builder = TokenCorpusBuilder::new();

        // Step 4: Parse + walk each file (parallelizable via rayon)
        let parser     = TreeSitterParser::new();
        let adapter_reg = AdapterRegistry::global();

        for (file_id, (path, record)) in files.iter().zip(&mut file_records).enumerate() {
            let source   = fs::read(path)?;
            let adapter  = adapter_reg.get(record.language_id)?;
            let tree     = parser.parse(&source, adapter.ts_language())?;

            let first_id = allocator.current();

            // Walk: O(n_cst_file) — produces RawTokens in source order
            let raw_tokens = walk_cst(
                tree.root_node(), &source, file_id as u16,
                adapter.as_ref(), &manifest.filter
            );

            // Assign token_ids + intern strings (single-threaded for interner lock)
            {
                let mut intern = interner.lock().unwrap();
                for rt in &raw_tokens {
                    let token_id = allocator.next_id();  // fetch-and-increment
                    let text     = &source[rt.text_start..rt.text_start + rt.text_len];
                    let text_id  = intern.intern(text);
                    let sort_key = build_sort_key(rt.file_id, rt.line, rt.col);

                    builder.push(token_id, TokenRecord {
                        sort_key, text_id,
                        len: rt.len, token_type: rt.token_type, _padding: 0,
                    });
                }
            }

            record.first_token_id  = first_id;
            record.file_token_count = raw_tokens.len() as u32;
        }

        // Step 5: Sort TokenRecord array by sort_key for forward lookup
        // sort_unstable_by_key is introsort: O(n log n), in-place, cache-friendly
        builder.sort_records();   // sorts token_records by sort_key

        // Step 6: Serialize to .tca binary (see §1.7)
        let artifact = builder.finalize(file_records, interner.into_inner().unwrap());
        TokenCorpusSerializer::write(&artifact, out_path)?;

        Ok(artifact)
    }
}
```

---

### 1.7 Output Schema: `TokenCorpusArtifact` Binary Format (`.tca`)

```
╔══════════════════════════════════════════════════════════════╗
║  TCA FILE FORMAT v1.0  (all integers: little-endian)        ║
╠══════════════╦════════╦═════════════════════════════════════╣
║ Section      ║ Size   ║ Description                         ║
╠══════════════╬════════╬═════════════════════════════════════╣
║ HEADER       ║ 64 B   ║ Magic, version, counts, SHA-256     ║
╠══════════════╬════════╬═════════════════════════════════════╣
║ FILE REG.    ║ F×64 B ║ SourceFileRecord array              ║
╠══════════════╬════════╬═════════════════════════════════════╣
║ FILE PATHS   ║ var.   ║ [u16_len][utf8_bytes] per path,     ║
║              ║        ║ padded to 8-byte boundary           ║
╠══════════════╬════════╬═════════════════════════════════════╣
║ TOKEN TABLE  ║ n×16 B ║ Sorted TokenRecord[], sorted by     ║
║              ║        ║ sort_key ascending. Forward index.  ║
╠══════════════╬════════╬═════════════════════════════════════╣
║ ENTRY MAP    ║ n×16 B ║ TokenEntry[], indexed by token_id.  ║
║              ║        ║ Backward index. Dense array.        ║
╠══════════════╬════════╬═════════════════════════════════════╣
║ STR HEADERS  ║ s×12 B ║ (hash:u64, str_offset:u32) per      ║
║              ║        ║ unique string. Sorted by hash.      ║
╠══════════════╬════════╬═════════════════════════════════════╣
║ STR STORAGE  ║ var.   ║ [u16_len][utf8_bytes] per string.   ║
║              ║        ║ No null terminator. Length-prefixed.║
╠══════════════╬════════╬═════════════════════════════════════╣
║ CHECKSUM     ║ 8 B    ║ CRC-64/ECMA of all preceding bytes  ║
╚══════════════╩════════╩═════════════════════════════════════╝
```

**HEADER (64 bytes, exact):**

```
Offset  Size  Field                Value
 0       8    magic                0x544F4B434F525001  ("TOKCORP\x01")
 8       4    format_version       0x00000001
12       4    token_count          n_tok (u32)
16       2    file_count           n_files (u16)
18       4    string_count         n_strings (u32)
22       2    flags                bit0: whitespace_included
                                   bit1: line_comments_included
                                   bit2: block_comments_included
                                   bit3: doc_comments_included
24      32    source_tree_hash     SHA-256(sorted(SHA-256(file_i)))
56       8    creation_ts_ns       Unix timestamp, nanoseconds (u64)
             ────────────────
             64 bytes total
```

**Size calculation for 1M token / 100-file Java project:**

```
Header:           64 B
File Registry:    100 × 64 = 6,400 B ≈  6.3 KB
File Paths:       100 × 50 (avg) = 5,000 B ≈  4.9 KB (8-byte padded)
Token Table:      1,000,000 × 16 = 16,000,000 B = 15.3 MB
Entry Map:        1,000,000 × 16 = 16,000,000 B = 15.3 MB
String Headers:   50,000 × 12  =    600,000 B =  0.6 MB
String Storage:   50,000 × 10 (avg) = 500,000 B =  0.5 MB
Checksum:         8 B
───────────────────────────────────────────────────────────
Total:                             ≈ 31.7 MB uncompressed
After LZ4 HC compression:          ≈  7–10 MB
(Token Table rows are highly regular; LZ4 achieves ~3–4× on sorted u64 arrays)
```

---

### 1.8 Complexity Proofs

**Time Complexity:**

Let N = total source bytes across all files, n_tok = total tokens, F = number of files.

| Operation | Complexity | Notes |
|---|---|---|
| Path sorting | O(F log F) | sort_unstable on PathBuf |
| SHA-256 per file | O(N) | linear scan of source bytes |
| Tree-sitter parse per file | O(\|file\|) | GLL parser, linear |
| CST walk per file | O(n_cst_file) | DFS, n_cst ≈ 3×n_tok |
| FNV-1a per token | O(\|text\|) | linear in token text length |
| StringInterner lookup | O(1) amortized | open-addressing, α=0.75 |
| Token record push | O(1) | Vec append |
| Sort TokenRecord[] | O(n_tok log n_tok) | introsort, in-place |
| Serialization | O(n_tok) | sequential write |

**Dominant term:** O(N + n_tok log n_tok). Since N ≥ n_tok (every token is ≥1 byte) and empirically N ≈ 6×n_tok for Java (avg identifier length ≈ 6 chars), the total is O(N + n_tok log n_tok). For n_tok = 10^6: sort term ≈ 2×10^7 comparisons, each a 64-bit integer compare ≈ 1 ns → ~20 ms for sort alone on modern hardware.

**Space Complexity:**

| Structure | Size |
|---|---|
| Source buffer (one file at a time) | O(max\|file\|) |
| CST (one file, transient) | O(n_cst_file) |
| token_records Vec | 16 × n_tok bytes |
| token_entries Vec | 16 × n_tok bytes |
| StringInterner table | 12 × n_strings bytes |
| StringInterner storage | ≈ 10 × n_strings bytes |

Peak memory at n_tok = 10^6, n_strings = 50K, max file = 5K LOC:

- token_records: 16 MB
- token_entries: 16 MB
- StringInterner: ~1.1 MB
- One CST (tree-sitter, ~50 bytes/node × 15K nodes): ~750 KB
- One file buffer (5K LOC × 50 chars/line): ~250 KB
- **Peak total: ≈ 34 MB**

This is O(n_tok) — dominated by the two token arrays.

---

### 1.9 Memory Optimization: Streaming Per-File Sort + K-Way Merge

For projects with n_tok > 5×10^6 (multi-million LOC monorepos), holding both the `token_records` and `token_entries` arrays in RAM simultaneously may be undesirable. The streaming optimization:

**Step 1 (per file):** Walk CST, produce a small local `Vec<TokenRecord>` for that file only. Sort it in-place — it fits in L3 cache (< 1 MB for files < 65K tokens). Write to a temporary file `tmp_{file_id}.tca`. Free all transient per-file memory.

**Step 2 (merge):** After all files are processed, execute a **k-way merge** of the F temporary files. Each temporary file is already sorted by sort_key. Merge using a min-heap of size F. The heap key is the current sort_key from each file's read cursor.

```
K-way merge complexity:
  Time:  O(n_tok × log F)  — each of n_tok records does one heap op
  Space: O(F × buffer_size + n_tok × 16)  — F read buffers + output
         For F=100 files with 64KB read buffers: 6.4 MB of buffers
```

This reduces peak RAM for the token_records array to O(F × buffer_size) during merge — for 100 files × 64 KB = 6.4 MB instead of 16 MB. The token_entries array (backward index) is built in a single O(n_tok) pass after the merge is complete, indexed by the final sorted order.

---

Now the Phase 1 internal architecture visualization:---

### Implementation Checklist for Phase 1

Everything Phase 2 (BP AST Encoding) will need from Phase 1 is guaranteed by the following invariants that must be enforced as assertions in the `TokenCorpusBuilder::finalize()` call:

**Invariant 1 (Monotonicity):** `∀ i < j: token_id(i) < token_id(j)` — token IDs are strictly monotonically increasing. Asserted via a debug check that `token_entries.len()` equals `next_token_id` at finalization.

**Invariant 2 (Injectivity):** `∀ i ≠ j: sort_key(i) ≠ sort_key(j)` — no two tokens occupy the same source position. Verified by a post-sort adjacent-duplicate scan in O(n) before serialization.

**Invariant 3 (Completeness):** `∀ file f: Σ token_count(f) = total_token_count` — no tokens lost during per-file processing. Verified by summing `file_record.file_token_count` across all files and asserting equality with `token_count` in the header.

**Invariant 4 (Forward-Backward Consistency):** `∀ token_id t: BI[t].sort_key = FI[binary_search(BI[t].sort_key)].sort_key` — every entry in the backward index has a corresponding entry in the forward index at the correct sort position. Spot-checked on 1% random sample during integration tests.

When these four invariants hold, Phase 2 can ingest the `TokenCorpusArtifact` with zero defensive programming overhead — the contract is mathematically guaranteed by Phase 1's construction.