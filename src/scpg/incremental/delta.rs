//! SourceDelta, RebuildPlan & UpdateResult data structures for incremental rebuilds (§10.6.2).
//!
//! A `SourceDelta` is derived from real file contents (old and new text), never from
//! caller-asserted flags. Its `TextEdit`s use tree-sitter's conventions (byte offsets,
//! 0-based rows, byte columns) and convert directly into `tree_sitter::InputEdit`, so the
//! ingestion layer can apply them with `Tree::edit` before an incremental reparse.

use serde::Serialize;
use sha2::{Digest, Sha256};

/// How much of the pipeline an edit invalidates. Ordered: a delta's scope is the maximum
/// over its edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub enum RebuildScope {
    /// Texts are identical.
    None,
    /// Only comments/whitespace changed: token positions and source spans shift, no analysis
    /// result changes (Phases 1 and 7 for the file).
    Positional,
    /// Only method bodies changed (CFG, SSA, call sites, path summaries of those methods).
    Behavioral,
    /// Declarations changed (symbols, hierarchy, UML structure).
    Structural,
    /// Changes outside every known declaration (imports, package clause) or in an unknown
    /// file: everything derived from the file is rebuilt.
    Full,
}

impl RebuildScope {
    /// Pipeline phases (1-10) that must re-run for the dirty entities at this scope.
    pub fn phases(&self) -> &'static [u8] {
        match self {
            RebuildScope::None => &[],
            RebuildScope::Positional => &[1, 7, 10],
            RebuildScope::Behavioral => &[1, 2, 4, 5, 6, 7, 8, 9, 10],
            RebuildScope::Structural | RebuildScope::Full => &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10],
        }
    }
}

/// 0-based row and byte column (tree-sitter `Point`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct TextPoint {
    pub row: usize,
    pub column: usize,
}

impl TextPoint {
    /// Position of byte offset `offset` in `text` (offsets past the end clamp to the end).
    pub fn of_offset(text: &[u8], offset: usize) -> Self {
        let prefix = &text[..offset.min(text.len())];
        let row = prefix.iter().filter(|&&b| b == b'\n').count();
        let line_start = prefix
            .iter()
            .rposition(|&b| b == b'\n')
            .map(|p| p + 1)
            .unwrap_or(0);
        Self {
            row,
            column: prefix.len() - line_start,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct TextEdit {
    pub start_byte: usize,
    pub old_end_byte: usize,
    pub new_end_byte: usize,
    pub start_point: TextPoint,
    pub old_end_point: TextPoint,
    pub new_end_point: TextPoint,
}

impl TextEdit {
    /// The single minimal edit turning `old` into `new` (common prefix / suffix), or `None`
    /// when the texts are identical.
    pub fn diff(old: &[u8], new: &[u8]) -> Option<Self> {
        if old == new {
            return None;
        }
        let prefix = old.iter().zip(new).take_while(|(a, b)| a == b).count();
        let max_suffix = old.len().min(new.len()) - prefix;
        let suffix = old
            .iter()
            .rev()
            .zip(new.iter().rev())
            .take(max_suffix)
            .take_while(|(a, b)| a == b)
            .count();
        let old_end = old.len() - suffix;
        let new_end = new.len() - suffix;
        Some(Self {
            start_byte: prefix,
            old_end_byte: old_end,
            new_end_byte: new_end,
            start_point: TextPoint::of_offset(old, prefix),
            old_end_point: TextPoint::of_offset(old, old_end),
            new_end_point: TextPoint::of_offset(new, new_end),
        })
    }

    /// Whether the edit's byte ranges lie inside the two texts and are ordered.
    pub fn is_valid_for(&self, old: &[u8], new: &[u8]) -> bool {
        self.start_byte <= self.old_end_byte
            && self.start_byte <= self.new_end_byte
            && self.old_end_byte <= old.len()
            && self.new_end_byte <= new.len()
    }
}

impl From<&TextEdit> for tree_sitter::InputEdit {
    fn from(e: &TextEdit) -> Self {
        tree_sitter::InputEdit {
            start_byte: e.start_byte,
            old_end_byte: e.old_end_byte,
            new_end_byte: e.new_end_byte,
            start_position: tree_sitter::Point::new(e.start_point.row, e.start_point.column),
            old_end_position: tree_sitter::Point::new(e.old_end_point.row, e.old_end_point.column),
            new_end_position: tree_sitter::Point::new(e.new_end_point.row, e.new_end_point.column),
        }
    }
}

/// A change to one source file, with both texts so edits can be classified exactly.
#[derive(Debug, Clone)]
pub struct SourceDelta {
    pub file_id: u16,
    pub old_text: Vec<u8>,
    pub new_text: Vec<u8>,
    pub old_sha256: [u8; 32],
    pub new_sha256: [u8; 32],
    /// Edits in application order against `old_text` (non-overlapping, ascending).
    pub edits: Vec<TextEdit>,
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(bytes);
    h.finalize().into()
}

impl SourceDelta {
    /// Derives the delta from the previous and current file contents.
    pub fn from_texts(file_id: u16, old_text: Vec<u8>, new_text: Vec<u8>) -> Self {
        let edits = TextEdit::diff(&old_text, &new_text).into_iter().collect();
        Self::from_edits(file_id, old_text, new_text, edits)
    }

    /// Uses caller-provided edits (e.g. LSP `didChange` ranges). Edits that do not fit the
    /// texts are replaced by the minimal prefix/suffix edit, so classification never reads
    /// outside the texts.
    pub fn from_edits(
        file_id: u16,
        old_text: Vec<u8>,
        new_text: Vec<u8>,
        mut edits: Vec<TextEdit>,
    ) -> Self {
        edits.sort_by_key(|e| e.start_byte);
        let overlapping = edits
            .windows(2)
            .any(|w| w[0].old_end_byte > w[1].start_byte);
        if overlapping || edits.iter().any(|e| !e.is_valid_for(&old_text, &new_text)) {
            edits = TextEdit::diff(&old_text, &new_text).into_iter().collect();
        }
        Self {
            file_id,
            old_sha256: sha256(&old_text),
            new_sha256: sha256(&new_text),
            old_text,
            new_text,
            edits,
        }
    }

    pub fn is_identity(&self) -> bool {
        self.old_sha256 == self.new_sha256
    }
}

/// Why a single edit received its scope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum EditClass {
    /// Whitespace between tokens or text strictly inside one comment.
    Trivia,
    /// Inside the body of these callables (innermost first).
    Body { methods: Vec<u32> },
    /// Touches the declaration (not the body) of this entity.
    Declaration { entity: u32 },
    /// Outside every declaration known to the snapshot.
    OutsideDeclarations,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EditClassification {
    pub edit: TextEdit,
    pub class: EditClass,
    pub scope: RebuildScope,
}

/// What must be recomputed for a delta, expressed against the *old* snapshot's ids.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RebuildPlan {
    pub file_id: u16,
    pub scope: RebuildScope,
    pub phases: Vec<u8>,
    pub edits: Vec<EditClassification>,
    /// Callables whose body changed.
    pub dirty_methods: Vec<u32>,
    /// Entities whose declaration changed.
    pub dirty_declarations: Vec<u32>,
    /// Transitive callers of dirty methods (inter-procedural summaries to refresh) and the
    /// impact set of dirty declarations.
    pub affected: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct UpdateResult {
    pub revision: u64,
    pub scope: RebuildScope,
    /// Stable ids of entities that appeared, disappeared, or changed.
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub signature_changed: Vec<String>,
    pub body_changed: Vec<String>,
    /// Entities whose content is unchanged but whose source span moved.
    pub moved: Vec<String>,
    /// Entities whose recomputed fingerprints equal the previous ones (backdated).
    pub unchanged: usize,
    /// UMLLinks to re-render: new-snapshot ids of added/changed/moved entities.
    pub stale_links: Vec<u32>,
    /// `stale_links.len()` plus the links of removed entities.
    pub stale_link_count: usize,
    /// Fraction of entities whose analysis results changed.
    pub changed_fraction: f64,
    pub old_scpg_hash: u32,
    pub new_scpg_hash: u32,
    pub new_digest: String,
}
