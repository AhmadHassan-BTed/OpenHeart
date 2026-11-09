//! Data structure specifications for Phase 7: Traceability Index Construction (§7.4).

use crate::tra::forward::SymbolSpanIndex;

/// Magic of the `.tra` format: the bytes `b"OPENHTRA"`, read as a little-endian `u64`
/// (the convention of every OpenHeart artifact magic, e.g. `PSA_MAGIC`).
pub const TRA_MAGIC: u64 = u64::from_le_bytes(*b"OPENHTRA");

/// Current `.tra` format version. Version 3: byte-string magic and a section directory with
/// offsets (direct access to any section). Since version 2: SHA-256 hash chain, per-entity
/// digests and stable keys, function block/SSA offset tables, 32-bit span lines, span
/// parent links.
pub const TRA_FORMAT_VERSION: u32 = 3;

/// Token id meaning "no source location" (synthetic entity without an owning span).
pub const NO_SOURCE_TOKEN: u32 = u32::MAX;

/// `SymbolSpanRecord::parent` of a span with no enclosing span.
pub const NO_PARENT: u32 = u32::MAX;

/// `SymbolSpanRecord::parent` of every span in a file whose spans are not properly nested;
/// queries in such a file fall back to a scan of the file's spans.
pub const PARENT_NOT_NESTED: u32 = u32::MAX - 1;

/// `FunctionIndexRecord` offset of a function absent from the CFA or SSA artifact.
pub const NO_OFFSET: u32 = u32::MAX;

/// `uml_slots` entry of a symbol without a UMLLink.
pub const NO_SLOT: u32 = u32::MAX;

/// `UMLLinkRecord::file_id` of a symbol without a source location.
pub const UML_LINK_NO_FILE: u16 = u16::MAX;

/// `UMLLinkRecord` line value of a symbol without a source location (the 24-bit maximum,
/// since lines are serialized as u24).
pub const UML_LINK_NO_LINE: u32 = 0x00FF_FFFF;

/// `UMLLinkRecord` column value of a symbol without a source location.
pub const UML_LINK_NO_COL: u16 = u16::MAX;

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LayerKind {
    LTok = 0,
    LAst = 1,
    LSym = 2,
    LBlk = 3,
    LSsa = 4,
    LCs = 5,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EntityRef {
    pub layer: LayerKind,
    pub entity_id: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceRange {
    pub file_id: u16,
    pub line_start: u32,
    pub col_start: u16,
    pub line_end: u32,
    pub col_end: u16,
}

/// AST Backward Index entry (8 bytes, indexed by preorder_idx).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BIAstEntry {
    pub first_token_id: u32,
    pub last_token_id: u32,
}

/// Symbol Backward Index entry (16 bytes, indexed by sym_id).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BISymEntry {
    pub decl_first_tok: u32,
    pub decl_last_tok: u32,
    pub def_first_tok: u32,
    pub def_last_tok: u32,
}

/// Basic Block Backward Index entry (8 bytes, indexed by global_block_id).
///
/// Synthetic blocks (ENTRY, EXIT, empty join blocks) carry the token range of their owning
/// function; [`NO_SOURCE_TOKEN`] marks a block whose function has no source span either.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BIBlkEntry {
    pub first_token_id: u32,
    pub last_token_id: u32,
}

/// SSA Variable Backward Index entry (12 bytes, indexed by global SSA index =
/// `FunctionIndexRecord::first_ssa + ssa_id`).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BISsaEntry {
    pub def_stmt: u32,
    pub first_token_id: u32,
    pub last_token_id: u32,
}

/// Call Site Backward Index entry (4 bytes, indexed by call_site_id).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BICsEntry {
    pub call_token: u32,
}

/// Symbol Span Record for Forward Queries (28 bytes serialized).
///
/// Records are sorted by `(file_id, first_token_id, last_token_id descending, sym_id)`;
/// `parent` is the index of the innermost enclosing record, [`NO_PARENT`], or
/// [`PARENT_NOT_NESTED`] for files whose spans overlap without nesting.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SymbolSpanRecord {
    pub first_token_id: u32,
    pub last_token_id: u32,
    pub sym_id: u32,
    pub file_id: u16,
    pub col_start: u16,
    pub line_start: u32,
    pub line_end: u32,
    pub parent: u32,
}

/// Call Site Span Record for Forward Queries (14 bytes serialized).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CallSiteSpanRecord {
    pub first_token_id: u32,
    pub call_site_id: u32,
    pub file_id: u16,
    pub line_start: u32,
}

/// Pre-computed UMLLink Record for Phase 9 (24 bytes).
///
/// `scpg_hash` is the build tag (`ScpgHashChain::scpg_hash`) shared by every link of one
/// build; per-element staleness is decided by the entity table, not by this tag. A symbol
/// without a source location has `file_id == UML_LINK_NO_FILE` and sentinel lines/columns.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UMLLinkRecord {
    pub sym_id: u32,
    pub file_id: u16,
    pub line_start: u32,
    pub col_start: u16,
    pub line_end: u32,
    pub col_end: u16,
    pub scpg_hash: u32,
    pub sym_kind: u8,
    pub _reserved: [u8; 3],
}

impl UMLLinkRecord {
    /// True iff the link points at a real source location.
    #[inline]
    pub fn has_source(&self) -> bool {
        self.file_id != UML_LINK_NO_FILE
    }
}

/// Per-function offsets into the flat block and SSA backward indices (20 bytes), sorted by
/// `sym_id`. Global block id = `first_block + local block id`; global SSA index =
/// `first_ssa + ssa_id`. Absent sections have offset [`NO_OFFSET`] and count 0.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FunctionIndexRecord {
    pub sym_id: u32,
    pub first_block: u32,
    pub block_count: u32,
    pub first_ssa: u32,
    pub ssa_count: u32,
}

/// Per-symbol identity, content digest and location (44 bytes, indexed by sym_id).
///
/// - `stable_key`: 128-bit digest of the symbol's kind, qualified name path, parameter
///   type signature and an ordinal among otherwise identical keys. It survives rebuilds and
///   edits elsewhere, so it — not the positional `sym_id` — matches symbols across builds.
/// - `content_digest`: 128-bit digest of the symbol's kind, modifiers and token stream
///   (for type declarations: their own tokens plus their members' stable keys; for every
///   other symbol: its whole definition). Positions, mtimes and whitespace are excluded, so
///   a `touch` or a pure move changes nothing and a body edit changes exactly that symbol.
/// - location: first-token file/line/column and last-token line (relocation detection).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntityRecord {
    pub stable_key: [u8; 16],
    pub content_digest: [u8; 16],
    pub file_id: u16,
    pub col_start: u16,
    pub line_start: u32,
    pub line_end: u32,
}

/// Fingerprint Hash Chain across all 6 SCPG build artifacts.
///
/// `artifact_digests[i]` is the SHA-256 canonical content digest of artifact `i` in the
/// order TCA, BPA, STA, CFA, SSA, CGA; `chain_digest` = `hₙ` with
/// `h₀ = SHA-256(domain)`, `hᵢ = SHA-256(hᵢ₋₁ ‖ labelᵢ ‖ artifact_digests[i])`.
/// The 64-bit fields are the first 8 bytes (little-endian) of the corresponding artifact
/// digest and `scpg_hash` the first 4 bytes of `chain_digest`: short display/link tags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScpgHashChain {
    pub tca_hash: u64,
    pub bpa_hash: u64,
    pub sta_hash: u64,
    pub cfa_hash: u64,
    pub ssa_hash: u64,
    pub cga_hash: u64,
    pub scpg_hash: u32,
    pub artifact_digests: [[u8; 32]; 6],
    pub chain_digest: [u8; 32],
}

/// Unified Traceability Artifact (.tra).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceabilityArtifact {
    pub format_version: u32,
    pub hashes: ScpgHashChain,
    pub bi_ast: Vec<BIAstEntry>,
    pub bi_sym: Vec<BISymEntry>,
    pub bi_blk: Vec<BIBlkEntry>,
    pub bi_ssa: Vec<BISsaEntry>,
    pub bi_cs: Vec<BICsEntry>,
    pub sym_span: Vec<SymbolSpanRecord>,
    pub cs_span: Vec<CallSiteSpanRecord>,
    pub uml_links: Vec<UMLLinkRecord>,
    pub function_index: Vec<FunctionIndexRecord>,
    pub entities: Vec<EntityRecord>,
    /// Dense `sym_id → index into uml_links` ([`NO_SLOT`] when absent). Derived from
    /// `uml_links`; rebuilt on load.
    pub uml_slots: Vec<u32>,
}

impl TraceabilityArtifact {
    /// The UMLLink of `sym_id` in O(1).
    pub fn uml_link(&self, sym_id: u32) -> Option<&UMLLinkRecord> {
        match self.uml_slots.get(sym_id as usize) {
            Some(&slot) if slot != NO_SLOT => self.uml_links.get(slot as usize),
            _ => None,
        }
    }

    /// The source range of `sym_id`'s UMLLink in O(1); `None` for symbols without a link or
    /// without a source location.
    pub fn source_range(&self, sym_id: u32) -> Option<SourceRange> {
        let link = self.uml_link(sym_id)?;
        if !link.has_source() {
            return None;
        }
        Some(SourceRange {
            file_id: link.file_id,
            line_start: link.line_start,
            col_start: link.col_start,
            line_end: link.line_end,
            col_end: link.col_end,
        })
    }

    /// The innermost symbol whose declaration span contains `token_id` in `file_id`,
    /// in O(log N + nesting depth).
    pub fn innermost_symbol_at(&self, file_id: u16, token_id: u32) -> Option<u32> {
        SymbolSpanIndex::innermost_sym(token_id, file_id, &self.sym_span)
    }

    /// All symbols whose declaration spans contain `token_id` in `file_id`, innermost first.
    pub fn symbols_at(&self, file_id: u16, token_id: u32) -> Vec<u32> {
        SymbolSpanIndex::forward_sym_query(token_id, file_id, &self.sym_span)
    }

    /// The offsets record of function `sym_id` (binary search).
    pub fn function_record(&self, sym_id: u32) -> Option<&FunctionIndexRecord> {
        self.function_index
            .binary_search_by_key(&sym_id, |r| r.sym_id)
            .ok()
            .map(|i| &self.function_index[i])
    }

    /// The block backward entries of function `sym_id`, indexed by local block id.
    pub fn function_blocks(&self, sym_id: u32) -> Option<&[BIBlkEntry]> {
        let r = self.function_record(sym_id)?;
        if r.first_block == NO_OFFSET {
            return None;
        }
        let start = r.first_block as usize;
        self.bi_blk.get(start..start.checked_add(r.block_count as usize)?)
    }

    /// The SSA backward entries of function `sym_id`, indexed by `ssa_id`.
    pub fn function_ssa(&self, sym_id: u32) -> Option<&[BISsaEntry]> {
        let r = self.function_record(sym_id)?;
        if r.first_ssa == NO_OFFSET {
            return None;
        }
        let start = r.first_ssa as usize;
        self.bi_ssa.get(start..start.checked_add(r.ssa_count as usize)?)
    }

    /// The entity record (stable key, content digest, location) of `sym_id`.
    pub fn entity(&self, sym_id: u32) -> Option<&EntityRecord> {
        self.entities.get(sym_id as usize)
    }
}
