//! DiagramContext — the analysis artifacts a diagram projection is derived from (§10.4).
//!
//! Exporters never invent content: every projection is computed from the artifacts held
//! here. The Phase 1/3/9 artifacts (`.tca`, `.sta`, `.uma`) are mandatory; the compiler-IR
//! artifacts (`.bpa`, `.cfa`, `.ssa`, `.cga`, `.psa`, `.tra`) are optional, and a projection
//! that needs a missing artifact reports itself as unavailable instead of guessing.
//!
//! Source positions are resolved through [`SourceIndex`], which needs only the token corpus,
//! so models derived outside the UMA pipeline (`crate::derive`) are printed with the same
//! spans as the UMA views.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::ast::BPASTArtifact;
use crate::cfg::builder::FunctionCFGData;
use crate::cfg::serializer::CFGArtifact;
use crate::core::types::cg::CallGraphArtifact;
use crate::core::types::symbol::{SymbolKind, SymbolRecord};
use crate::core::types::token::{unpack_sort_key, TokenEntry, TokenType};
use crate::ingestion::TokenCorpusArtifact;
use crate::psa::types::PathSummaryArtifact;
use crate::scpg::diagram::common::DiagramUtils;
use crate::ssa::serializer::{FunctionSSAData, SSAArtifact};
use crate::symbol::adapter::{
    PRIMITIVE_BOOLEAN_ID, PRIMITIVE_BYTE_ID, PRIMITIVE_CHAR_ID, PRIMITIVE_DOUBLE_ID,
    PRIMITIVE_FLOAT_ID, PRIMITIVE_INT_ID, PRIMITIVE_LONG_ID, PRIMITIVE_SHORT_ID, PRIMITIVE_VOID_ID,
};
use crate::symbol::model::{TypeRefFlags, WildcardKind};
use crate::symbol::SymbolTableArtifact;
use crate::tra::types::{TraceabilityArtifact, UMLLinkRecord, UML_LINK_NO_LINE};
use crate::uma::actor_identification::EXTERNAL_ACTOR_ID;
use crate::uma::types::{uml_link_is_known, ClassRecord, UMLMetadataArtifact};

/// Lifeline id the Phase 9 actor identification uses for the external caller (the one
/// definition lives in `uma::actor_identification`).
pub const EXTERNAL_ACTOR_SYM: u32 = EXTERNAL_ACTOR_ID;

/// Upper bound on parent-chain walks (guards against cyclic parent links in damaged tables).
const MAX_PARENT_DEPTH: usize = 64;
/// Upper bound on tokens joined into one source-text label.
const MAX_TEXT_TOKENS: u32 = 256;
/// Upper bound on the nesting of type arguments rendered from the TypeRef table.
const MAX_TYPE_DEPTH: usize = 12;
/// Maximum characters of a rendered type.
const MAX_TYPE_CHARS: usize = 96;

/// UML name of a primitive pseudo-id of the symbol table (`None` for every other id).
pub fn primitive_type_name(id: u32) -> Option<&'static str> {
    match id {
        PRIMITIVE_VOID_ID => Some("void"),
        PRIMITIVE_BOOLEAN_ID => Some("boolean"),
        PRIMITIVE_BYTE_ID => Some("byte"),
        PRIMITIVE_SHORT_ID => Some("short"),
        PRIMITIVE_CHAR_ID => Some("char"),
        PRIMITIVE_INT_ID => Some("int"),
        PRIMITIVE_LONG_ID => Some("long"),
        PRIMITIVE_FLOAT_ID => Some("float"),
        PRIMITIVE_DOUBLE_ID => Some("double"),
        _ => None,
    }
}

/// True for the pseudo-id of the "no result" type (`void`): an operation returning it has
/// no return type in UML.
pub fn is_void_type_id(id: u32) -> bool {
    id == PRIMITIVE_VOID_ID
}

/// Last segment of a qualified name written with `.` or `::` separators.
pub fn last_segment(qualified: &str) -> &str {
    let tail = qualified.rsplit("::").next().unwrap_or(qualified);
    tail.rsplit('.').next().unwrap_or(tail)
}

/// A resolved source location of a diagram element (1-based lines, 0-based columns).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceSpan {
    pub file_id: u16,
    /// Path of the source file as ingested (forward slashes), `None` when the corpus has no path.
    pub path: Option<String>,
    pub line_start: u32,
    pub col_start: u32,
    pub line_end: u32,
    pub col_end: u32,
    /// First token of the span (`u32::MAX` when the span came from a precomputed UMLLink).
    pub first_token: u32,
    /// Last token of the span (`u32::MAX` when the span came from a precomputed UMLLink).
    pub last_token: u32,
}

/// Colour theme written into the text formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DiagramTheme {
    /// No theme directives: the renderer's own defaults apply (readable on any page).
    #[default]
    None,
    /// The studio's dark palette (transparent background, light text).
    Dark,
}

/// View scoping and size budgets (large-graph strategy).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewOptions {
    /// Restricts a view to elements whose qualified name equals this value, lies below it
    /// (package or type prefix) or ends with it (simple-name match).
    pub focus: Option<String>,
    /// Maximum number of non-container nodes; excess nodes are dropped by ascending degree.
    pub max_nodes: usize,
    /// Maximum number of edges; excess edges are dropped by ascending structural priority.
    pub max_edges: usize,
    /// Maximum number of messages rendered per sequence scenario.
    pub max_messages: usize,
    /// Design-pattern detections with a structural-evidence score below this value (0..100)
    /// are not shown as stereotypes.
    pub min_pattern_score: u16,
    /// Theme directives written by the PlantUML and Mermaid printers.
    pub theme: DiagramTheme,
}

impl Default for ViewOptions {
    fn default() -> Self {
        Self {
            focus: None,
            max_nodes: 300,
            // Mermaid refuses graphs with more than 500 edges by default.
            max_edges: 450,
            // Keeps a sequence page inside PlantUML's default 4096 px image limit.
            max_messages: 120,
            min_pattern_score: 0,
            theme: DiagramTheme::None,
        }
    }
}

/// Two artifacts whose hash links disagree: they were produced by different analysis runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactMismatch {
    /// The two artifacts (pipeline names such as `sta`, `cfa`).
    pub artifacts: (&'static str, &'static str),
    pub detail: String,
}

impl ArtifactMismatch {
    /// True when the mismatch involves `artifact`.
    pub fn involves(&self, artifact: &str) -> bool {
        self.artifacts.0 == artifact || self.artifacts.1 == artifact
    }
}

/// Token-level source lookups over the Phase 1 token corpus: positions, file paths, source
/// text and spans of token ranges.
#[derive(Clone)]
pub struct SourceIndex<'a> {
    pub tca: &'a TokenCorpusArtifact,
    file_paths: HashMap<u16, String>,
}

impl<'a> SourceIndex<'a> {
    pub fn new(tca: &'a TokenCorpusArtifact) -> Self {
        let mut file_paths = HashMap::new();
        for rec in &tca.file_records {
            let bytes = tca.interner.lookup_text(rec.path_str_offset);
            let path = String::from_utf8_lossy(bytes).replace('\\', "/");
            if !path.is_empty() {
                file_paths.insert(rec.file_id, path);
            }
        }
        Self { tca, file_paths }
    }

    /// Text of an interner id, `None` when the id is unknown or the text is empty.
    pub fn text(&self, text_id: u32) -> Option<String> {
        let bytes = self.tca.interner.lookup_text(text_id);
        let text = String::from_utf8_lossy(bytes).trim().to_string();
        if text.is_empty() {
            None
        } else {
            Some(text)
        }
    }

    pub fn token(&self, token_id: u32) -> Option<&'a TokenEntry> {
        if token_id == u32::MAX {
            return None;
        }
        let tca: &'a TokenCorpusArtifact = self.tca;
        tca.token_entries.get(token_id as usize)
    }

    /// (file_id, line, column) of a token.
    pub fn token_position(&self, token_id: u32) -> Option<(u16, u32, u16)> {
        self.token(token_id).map(|t| unpack_sort_key(t.sort_key))
    }

    pub fn token_text(&self, token_id: u32) -> Option<&'a str> {
        let entry = self.token(token_id)?;
        let tca: &'a TokenCorpusArtifact = self.tca;
        std::str::from_utf8(tca.interner.lookup_text(entry.text_id)).ok()
    }

    pub fn file_path(&self, file_id: u16) -> Option<&str> {
        self.file_paths.get(&file_id).map(|p| p.as_str())
    }

    /// Source span covering tokens `first..=last` (clamped to the file of `first`).
    pub fn span_of_tokens(&self, first: u32, last: u32) -> Option<SourceSpan> {
        let (file_id, line_start, col_start) = self.token_position(first)?;
        let start_len = self.token(first).map(|t| t.len as u32).unwrap_or(0);
        let last = if last == u32::MAX || last < first {
            first
        } else {
            last
        };
        let (line_end, col_end, last_token) = match self.token_position(last) {
            Some((end_file, end_line, end_col))
                if end_file == file_id && end_line >= line_start =>
            {
                let len = self.token(last).map(|t| t.len as u32).unwrap_or(0);
                (end_line, end_col as u32 + len, last)
            }
            _ => (line_start, col_start as u32 + start_len, first),
        };
        Some(SourceSpan {
            file_id,
            path: self.file_path(file_id).map(str::to_string),
            line_start,
            col_start: col_start as u32,
            line_end,
            col_end,
            first_token: first,
            last_token,
        })
    }

    /// True for tokens that carry no program text (comments, whitespace).
    fn is_trivia(entry: &TokenEntry) -> bool {
        matches!(
            TokenType::from_u8(entry.token_type),
            TokenType::CommentLine
                | TokenType::CommentBlock
                | TokenType::CommentDoc
                | TokenType::Whitespace
                | TokenType::Newline
        )
    }

    /// Program tokens of `first..=last` inside the file of `first` (at most
    /// `MAX_TEXT_TOKENS + 1`), without trivia.
    fn program_tokens(&self, first: u32, last: u32) -> Vec<&'a str> {
        let file_id = match self.token_position(first) {
            Some((f, _, _)) => f,
            None => return Vec::new(),
        };
        let last = if last == u32::MAX || last < first {
            first
        } else {
            last
        };
        let limit = last.min(first.saturating_add(MAX_TEXT_TOKENS));
        let mut out = Vec::new();
        for tok in first..=limit {
            let entry = match self.token(tok) {
                Some(e) => e,
                None => break,
            };
            if unpack_sort_key(entry.sort_key).0 != file_id {
                break;
            }
            if Self::is_trivia(entry) {
                continue;
            }
            match self.token_text(tok) {
                Some(t) if !t.is_empty() => out.push(t),
                _ => {}
            }
        }
        out
    }

    /// Source text of tokens `first..=last` without comments, joined with conventional
    /// spacing and truncated on a character boundary.
    pub fn source_text(&self, first: u32, last: u32, max_chars: usize) -> Option<String> {
        let mut out = String::new();
        let mut glue_next = false;
        for text in self.program_tokens(first, last) {
            let glue_before = matches!(
                text,
                "." | "," | ";" | ")" | "]" | "(" | "[" | "::" | "?." | "++" | "--"
            );
            if !out.is_empty() && !glue_next && !glue_before {
                out.push(' ');
            }
            out.push_str(text);
            if out.chars().count() > max_chars.saturating_mul(2) {
                break;
            }
            glue_next = matches!(text, "." | "(" | "[" | "::" | "?." | "@" | "!" | "~");
        }
        let line = DiagramUtils::single_line(&out);
        if line.is_empty() {
            None
        } else {
            Some(DiagramUtils::truncate(&line, max_chars))
        }
    }

    /// A written type (`Map<String, List<Order>>`, `int[]`, `String?`, `? extends T`,
    /// `const char *`) as it appears in the source. Tokens are joined without blanks except
    /// between two word tokens and after a comma, which is the spelling every supported
    /// language uses for type syntax.
    pub fn type_text(&self, first: u32, last: u32) -> Option<String> {
        if first == u32::MAX {
            return None;
        }
        let is_word = |c: Option<char>| {
            c.map(|c| c.is_alphanumeric() || c == '_' || c == '$')
                .unwrap_or(false)
        };
        let mut out = String::new();
        let mut prev: Option<&str> = None;
        for text in self.program_tokens(first, last) {
            if let Some(p) = prev {
                let after_word = is_word(p.chars().last()) || matches!(p, "?" | "*" | "&" | "&&");
                if p == "," || (after_word && is_word(text.chars().next())) {
                    out.push(' ');
                }
            }
            out.push_str(text);
            prev = Some(text);
        }
        let line = DiagramUtils::single_line(&out);
        if line.is_empty() {
            None
        } else {
            Some(DiagramUtils::truncate(&line, MAX_TYPE_CHARS))
        }
    }

    /// Condition of a branching statement (`if (c)`, `while (c)`, `for (...)`, `switch (x)`),
    /// read between the header keyword's parentheses. `None` for any other statement shape.
    pub fn condition_text(&self, first: u32, last: u32, max_chars: usize) -> Option<String> {
        let keyword = self.token_text(first)?;
        if !matches!(
            keyword,
            "if" | "while" | "for" | "switch" | "when" | "catch"
        ) {
            return None;
        }
        let (file_id, _, _) = self.token_position(first)?;
        let limit = last.min(first.saturating_add(MAX_TEXT_TOKENS));
        let mut open = None;
        let mut depth = 0i32;
        let mut tok = first.saturating_add(1);
        while tok <= limit {
            if self.token_position(tok).map(|p| p.0) != Some(file_id) {
                return None;
            }
            match self.token_text(tok) {
                Some("(") => {
                    if open.is_none() {
                        open = Some(tok);
                    }
                    depth += 1;
                }
                Some(")") => {
                    depth -= 1;
                    if depth == 0 {
                        let start = open?;
                        if tok <= start + 1 {
                            return None;
                        }
                        return self.source_text(start + 1, tok - 1, max_chars);
                    }
                }
                _ => {
                    if open.is_none() && tok > first + 2 {
                        return None;
                    }
                }
            }
            tok += 1;
        }
        None
    }
}

/// All artifacts a diagram projection may be derived from, plus lookup indexes.
#[derive(Clone)]
pub struct DiagramContext<'a> {
    pub uma: &'a UMLMetadataArtifact,
    pub sta: &'a SymbolTableArtifact,
    pub tca: &'a TokenCorpusArtifact,
    pub bpa: Option<&'a BPASTArtifact>,
    pub cfa: Option<&'a CFGArtifact>,
    pub ssa: Option<&'a SSAArtifact>,
    pub cga: Option<&'a CallGraphArtifact>,
    pub psa: Option<&'a PathSummaryArtifact>,
    pub tra: Option<&'a TraceabilityArtifact>,
    pub options: ViewOptions,
    source: SourceIndex<'a>,
    children: HashMap<u32, Vec<u32>>,
    class_index: HashMap<u32, usize>,
    cfa_index: HashMap<u32, usize>,
    ssa_index: HashMap<u32, usize>,
}

impl<'a> DiagramContext<'a> {
    /// Context over the mandatory artifacts. Compiler-IR projections (CFG, DFG, CDG, call
    /// graph, ROBDD) report themselves unavailable until the matching `with_*` is called.
    pub fn new(
        uma: &'a UMLMetadataArtifact,
        sta: &'a SymbolTableArtifact,
        tca: &'a TokenCorpusArtifact,
    ) -> Self {
        let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
        for (idx, rec) in sta.symbol_records.iter().enumerate() {
            let id = idx as u32;
            if rec.parent_sym != u32::MAX && rec.parent_sym != id {
                children.entry(rec.parent_sym).or_default().push(id);
            }
        }
        for list in children.values_mut() {
            list.sort_by_key(|&s| {
                let tok = sta.symbol(s).map(|r| r.first_token_id).unwrap_or(u32::MAX);
                (tok, s)
            });
        }

        let mut class_index = HashMap::new();
        for (idx, class_rec) in uma.classes.iter().enumerate() {
            class_index.entry(class_rec.sym_id).or_insert(idx);
        }

        Self {
            uma,
            sta,
            tca,
            bpa: None,
            cfa: None,
            ssa: None,
            cga: None,
            psa: None,
            tra: None,
            options: ViewOptions::default(),
            source: SourceIndex::new(tca),
            children,
            class_index,
            cfa_index: HashMap::new(),
            ssa_index: HashMap::new(),
        }
    }

    pub fn with_bpa(mut self, bpa: &'a BPASTArtifact) -> Self {
        self.bpa = Some(bpa);
        self
    }

    pub fn with_cfa(mut self, cfa: &'a CFGArtifact) -> Self {
        let mut index = HashMap::new();
        for (idx, func) in cfa.functions.iter().enumerate() {
            index.entry(func.sym_id).or_insert(idx);
        }
        self.cfa_index = index;
        self.cfa = Some(cfa);
        self
    }

    pub fn with_ssa(mut self, ssa: &'a SSAArtifact) -> Self {
        let mut index = HashMap::new();
        for (idx, func) in ssa.functions.iter().enumerate() {
            index.entry(func.sym_id).or_insert(idx);
        }
        self.ssa_index = index;
        self.ssa = Some(ssa);
        self
    }

    pub fn with_cga(mut self, cga: &'a CallGraphArtifact) -> Self {
        self.cga = Some(cga);
        self
    }

    pub fn with_psa(mut self, psa: &'a PathSummaryArtifact) -> Self {
        self.psa = Some(psa);
        self
    }

    pub fn with_tra(mut self, tra: &'a TraceabilityArtifact) -> Self {
        self.tra = Some(tra);
        self
    }

    pub fn with_options(mut self, options: ViewOptions) -> Self {
        self.options = options;
        self
    }

    /// Same context restricted to `focus` (see [`ViewOptions::focus`]).
    pub fn with_focus(mut self, focus: impl Into<String>) -> Self {
        self.options.focus = Some(focus.into());
        self
    }

    /// Token-level source lookups of this context.
    pub fn source(&self) -> &SourceIndex<'a> {
        &self.source
    }

    /// Names of the artifacts available to this context, in pipeline order.
    pub fn available_artifacts(&self) -> Vec<&'static str> {
        let mut names = vec!["tca", "sta", "uma"];
        if self.bpa.is_some() {
            names.push("bpa");
        }
        if self.cfa.is_some() {
            names.push("cfa");
        }
        if self.ssa.is_some() {
            names.push("ssa");
        }
        if self.cga.is_some() {
            names.push("cga");
        }
        if self.tra.is_some() {
            names.push("tra");
        }
        if self.psa.is_some() {
            names.push("psa");
        }
        names
    }

    /// Hash links between the attached artifacts that disagree. Every check compares two
    /// values the pipeline computes the same way from the same upstream bytes: the CRC-64 of
    /// the `.tca` recorded by the BPA and the STA, of the `.bpa` recorded by the STA and the
    /// CFA, of the `.sta` recorded by the CFA and the CGA, of the `.cfa` recorded by the SSA
    /// and the PSA, the SSA content digest recorded by the PSA and the TRA hash chain, and
    /// the SCPG chain tag the UMA class links copied from the TRA. Unset (zero) values are
    /// not compared. An empty result means the artifacts are mutually consistent.
    pub fn consistency_issues(&self) -> Vec<ArtifactMismatch> {
        let mut issues = Vec::new();
        let mut check = |a: &'static str, b: &'static str, what: &str, x: u64, y: u64| {
            if x != 0 && y != 0 && x != y {
                issues.push(ArtifactMismatch {
                    artifacts: (a, b),
                    detail: format!(
                        "{} records {} {:016x}, {} records {:016x}",
                        a, what, x, b, y
                    ),
                });
            }
        };
        if let Some(bpa) = self.bpa {
            check(
                "bpa",
                "sta",
                "the .tca CRC-64",
                bpa.tca_hash,
                self.sta.tca_hash,
            );
        }
        if let Some(cfa) = self.cfa {
            check(
                "cfa",
                "sta",
                "the .bpa CRC-64",
                cfa.bpa_hash,
                self.sta.bpa_hash,
            );
            if let Some(cga) = self.cga {
                check("cfa", "cga", "the .sta CRC-64", cfa.sta_hash, cga.sta_hash);
            }
        }
        if let (Some(ssa), Some(psa)) = (self.ssa, self.psa) {
            check("ssa", "psa", "the .cfa CRC-64", ssa.cfa_hash, psa.cfa_hash);
        }
        if let (Some(psa), Some(tra)) = (self.psa, self.tra) {
            check(
                "psa",
                "tra",
                "the SSA content digest",
                psa.ssa_hash,
                tra.hashes.ssa_hash,
            );
        }
        if let Some(tra) = self.tra {
            let expected = tra.hashes.scpg_hash;
            if let Some(stale) = self.uma.classes.iter().find(|c| {
                uml_link_is_known(&c.uml_link)
                    && c.uml_link.scpg_hash != 0
                    && expected != 0
                    && c.uml_link.scpg_hash != expected
            }) {
                issues.push(ArtifactMismatch {
                    artifacts: ("uma", "tra"),
                    detail: format!(
                        "uma class links carry SCPG chain tag {:08x}, tra records {:08x}",
                        stale.uml_link.scpg_hash, expected
                    ),
                });
            }
        }
        issues
    }

    /// SHA-256 digest (hex) identifying the analyzed input snapshot: per-file content hashes
    /// and paths, the symbol-table checksum, the UMA hash chain and the shape of every
    /// attached compiler-IR artifact. Deterministic: contains no timestamps.
    pub fn input_digest(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(b"openheart.diagram-input.v1");
        let mut files: Vec<_> = self.tca.file_records.iter().collect();
        files.sort_by_key(|f| f.file_id);
        for file in files {
            hasher.update(file.file_id.to_le_bytes());
            hasher.update(file.content_sha256);
            if let Some(path) = self.source.file_path(file.file_id) {
                hasher.update(path.as_bytes());
            }
        }
        hasher.update((self.tca.token_entries.len() as u64).to_le_bytes());
        hasher.update(self.sta.crc64_checksum.to_le_bytes());
        hasher.update(self.uma.tra_hash.to_le_bytes());
        hasher.update(self.uma.format_version.to_le_bytes());
        if let Some(cfa) = self.cfa {
            hasher.update(b"cfa");
            hasher.update(cfa.function_count.to_le_bytes());
            hasher.update(cfa.total_blocks.to_le_bytes());
            hasher.update(cfa.total_edges.to_le_bytes());
            hasher.update(cfa.sta_hash.to_le_bytes());
        }
        if let Some(ssa) = self.ssa {
            hasher.update(b"ssa");
            hasher.update(ssa.cfa_hash.to_le_bytes());
            hasher.update(ssa.total_ssa_vars.to_le_bytes());
            hasher.update(ssa.total_phi_funcs.to_le_bytes());
        }
        if let Some(cga) = self.cga {
            hasher.update(b"cga");
            hasher.update(cga.ssa_hash.to_le_bytes());
            hasher.update(cga.call_site_count.to_le_bytes());
            hasher.update(cga.call_edge_count.to_le_bytes());
        }
        if let Some(psa) = self.psa {
            hasher.update(b"psa");
            hasher.update(psa.cfa_hash.to_le_bytes());
            hasher.update(psa.total_nodes.to_le_bytes());
        }
        if let Some(tra) = self.tra {
            hasher.update(b"tra");
            hasher.update(tra.hashes.chain_digest);
        }
        let digest: [u8; 32] = hasher.finalize().into();
        DiagramUtils::hex(&digest)
    }

    // ── Symbols & names ──────────────────────────────────────────────────────

    pub fn symbol(&self, sym_id: u32) -> Option<&'a SymbolRecord> {
        let sta: &'a SymbolTableArtifact = self.sta;
        sta.symbol(sym_id)
    }

    pub fn kind_of(&self, sym_id: u32) -> Option<SymbolKind> {
        self.symbol(sym_id).map(|s| SymbolKind::from(s.kind))
    }

    /// Type-like symbol kinds (classifiers that can own members).
    pub fn is_type_kind(kind: SymbolKind) -> bool {
        matches!(
            kind,
            SymbolKind::SK_CLASS
                | SymbolKind::SK_INTERFACE
                | SymbolKind::SK_ENUM
                | SymbolKind::SK_RECORD
                | SymbolKind::SK_ANNOTATION_TYPE
                | SymbolKind::SK_ANON_CLASS
        )
    }

    /// Text of an interner id, `None` when the id is unknown or the text is empty.
    pub fn text(&self, text_id: u32) -> Option<String> {
        self.source.text(text_id)
    }

    /// Declared simple name of a symbol. Symbols the source leaves unnamed get an explicit
    /// name from their kind and position (`<anonymous class L12>`, `<lambda L40>`,
    /// `<static initializer>`), never the text of a neighbouring token.
    pub fn simple_name(&self, sym_id: u32) -> Option<String> {
        if sym_id == EXTERNAL_ACTOR_SYM {
            return Some("Actor".to_string());
        }
        let sym = self.symbol(sym_id)?;
        if let Some(name) = self.text(sym.name_id) {
            return Some(name);
        }
        if let Some(qualified) = self.sta.qualified_name_of(sym_id) {
            let name = last_segment(qualified);
            if !name.is_empty() {
                return Some(name.to_string());
            }
        }
        if let Some(path) = self.sta.custom_package_names.get(&sym_id) {
            return Some(last_segment(path).to_string());
        }
        self.synthetic_name(sym_id, sym)
    }

    /// Name of an unnamed declaration, from its kind and the line it starts on.
    fn synthetic_name(&self, sym_id: u32, sym: &SymbolRecord) -> Option<String> {
        let line = self.token_position(sym.first_token_id).map(|(_, l, _)| l);
        let at = |what: &str| match line {
            Some(l) => format!("<{} L{}>", what, l),
            None => format!("<{} #{}>", what, sym_id),
        };
        match SymbolKind::from(sym.kind) {
            SymbolKind::SK_ANON_CLASS => Some(at("anonymous class")),
            SymbolKind::SK_LAMBDA => Some(at("lambda")),
            SymbolKind::SK_STATIC_INIT => Some("<static initializer>".to_string()),
            SymbolKind::SK_INSTANCE_INIT => Some("<instance initializer>".to_string()),
            _ => None,
        }
    }

    fn parent_of(&self, sym_id: u32) -> Option<u32> {
        let parent = self.symbol(sym_id)?.parent_sym;
        if parent == u32::MAX || parent == sym_id {
            None
        } else {
            Some(parent)
        }
    }

    /// Canonical fully qualified name of a package symbol: the Phase 3 qualified name, else
    /// the recorded package path. `None` when neither is known (a bare segment name is never
    /// returned, so a nested package cannot reappear as a stray root).
    pub fn package_name(&self, pkg_sym: u32) -> Option<String> {
        self.sta
            .qualified_name_of(pkg_sym)
            .filter(|q| !q.is_empty())
            .map(str::to_string)
            .or_else(|| self.sta.custom_package_names.get(&pkg_sym).cloned())
    }

    /// Fully qualified package name containing (or naming) a symbol.
    pub fn package_of(&self, sym_id: u32) -> Option<String> {
        let mut curr = sym_id;
        for _ in 0..MAX_PARENT_DEPTH {
            if self.kind_of(curr) == Some(SymbolKind::SK_PACKAGE) {
                if let Some(pkg) = self.package_name(curr) {
                    return Some(pkg);
                }
            }
            if let Some(pkg) = self.sta.custom_package_names.get(&curr) {
                return Some(pkg.clone());
            }
            match self.parent_of(curr) {
                Some(parent) => curr = parent,
                None => break,
            }
        }
        let sym = self.symbol(sym_id)?;
        let (file_id, _, _) = self.token_position(sym.first_token_id)?;
        self.sta.file_package_names.get(&file_id).cloned()
    }

    /// Simple name prefixed by the enclosing type chain: `Outer.Inner`.
    pub fn nested_type_name(&self, sym_id: u32) -> Option<String> {
        let mut parts = vec![self.simple_name(sym_id)?];
        let mut curr = sym_id;
        for _ in 0..MAX_PARENT_DEPTH {
            let parent = match self.parent_of(curr) {
                Some(p) => p,
                None => break,
            };
            match self.kind_of(parent) {
                Some(kind) if Self::is_type_kind(kind) => {
                    if let Some(name) = self.simple_name(parent) {
                        parts.push(name);
                    }
                    curr = parent;
                }
                _ => break,
            }
        }
        parts.reverse();
        Some(parts.join("."))
    }

    /// Fully qualified name of a type: the Phase 3 canonical name, else `package.Outer.Inner`.
    pub fn type_fqn(&self, sym_id: u32) -> Option<String> {
        if let Some(q) = self.sta.qualified_name_of(sym_id).filter(|q| !q.is_empty()) {
            return Some(q.to_string());
        }
        let nested = self.nested_type_name(sym_id)?;
        match self.package_of(sym_id) {
            Some(pkg) if !pkg.is_empty() => Some(format!("{}.{}", pkg, nested)),
            _ => Some(nested),
        }
    }

    /// Nearest enclosing type of a member symbol.
    pub fn owner_type(&self, sym_id: u32) -> Option<u32> {
        let mut curr = sym_id;
        for _ in 0..MAX_PARENT_DEPTH {
            let parent = self.parent_of(curr)?;
            if self
                .kind_of(parent)
                .map(Self::is_type_kind)
                .unwrap_or(false)
            {
                return Some(parent);
            }
            curr = parent;
        }
        None
    }

    /// `Owner.member` display label of a member symbol.
    pub fn member_label(&self, sym_id: u32) -> Option<String> {
        let name = self.simple_name(sym_id)?;
        match self
            .owner_type(sym_id)
            .and_then(|owner| self.nested_type_name(owner))
        {
            Some(owner) => Some(format!("{}.{}", owner, name)),
            None => Some(name),
        }
    }

    /// Fully qualified name of a member: `package.Owner.member`.
    pub fn member_fqn(&self, sym_id: u32) -> Option<String> {
        let name = self.simple_name(sym_id)?;
        if let Some(owner) = self.owner_type(sym_id) {
            if let Some(owner_fqn) = self.type_fqn(owner) {
                return Some(format!("{}.{}", owner_fqn, name));
            }
        }
        match self.package_of(sym_id) {
            Some(pkg) if !pkg.is_empty() => Some(format!("{}.{}", pkg, name)),
            _ => Some(name),
        }
    }

    /// Display name of any symbol: types by nested name, members by `Owner.member`.
    pub fn display_name(&self, sym_id: u32) -> Option<String> {
        if sym_id == EXTERNAL_ACTOR_SYM {
            return Some("Actor".to_string());
        }
        match self.kind_of(sym_id) {
            Some(kind) if Self::is_type_kind(kind) => self.nested_type_name(sym_id),
            Some(SymbolKind::SK_PACKAGE) => self
                .package_name(sym_id)
                .or_else(|| self.simple_name(sym_id)),
            Some(SymbolKind::SK_EXTERNAL) => self.simple_name(sym_id),
            Some(_) => self.member_label(sym_id),
            None => None,
        }
    }

    /// Qualified name of any symbol (types, members, packages, externals).
    pub fn qualified_name(&self, sym_id: u32) -> Option<String> {
        match self.kind_of(sym_id) {
            Some(kind) if Self::is_type_kind(kind) => self.type_fqn(sym_id),
            Some(SymbolKind::SK_PACKAGE) => self.package_of(sym_id),
            Some(SymbolKind::SK_EXTERNAL) => self.type_fqn(sym_id),
            Some(_) => self.member_fqn(sym_id),
            None => None,
        }
    }

    /// Type parameter names of a generic declaration, in declaration order.
    pub fn type_parameters(&self, sym_id: u32) -> Vec<String> {
        self.children_of(sym_id)
            .iter()
            .filter(|&&c| self.kind_of(c) == Some(SymbolKind::SK_TYPE_PARAM))
            .filter_map(|&c| self.simple_name(c))
            .collect()
    }

    // ── Types ────────────────────────────────────────────────────────────────

    /// Declared type of a symbol (attribute, parameter, operation result) as the program
    /// states it, `None` when the symbol has no declared or inferable type. Written types are
    /// rendered from their source tokens (generic arguments, arrays and nullability exactly
    /// as written); inferred types from the resolved TypeRef structure.
    pub fn declared_type(&self, sym_id: u32) -> Option<String> {
        let id = *self.sta.symbol_type_refs.get(sym_id as usize)?;
        if id == u32::MAX {
            return None;
        }
        self.type_ref_text(id, 0)
    }

    /// True when the declared type of `sym_id` is the "no result" type (`void`).
    pub fn declared_type_is_void(&self, sym_id: u32) -> bool {
        self.sta
            .type_ref_of(sym_id)
            .map(|tr| tr.flags & TypeRefFlags::PRIMITIVE != 0 && is_void_type_id(tr.head))
            .unwrap_or(false)
    }

    /// Text of one TypeRef table entry.
    pub fn type_ref_text(&self, id: u32, depth: usize) -> Option<String> {
        if depth > MAX_TYPE_DEPTH {
            return None;
        }
        let tr = self.sta.type_ref(id)?;
        let inferred = tr.flags & TypeRefFlags::INFERRED != 0;
        if !inferred && tr.first_token != u32::MAX {
            if let Some(text) = self.source.type_text(tr.first_token, tr.last_token) {
                return Some(text);
            }
        }
        if tr.flags & TypeRefFlags::UNKNOWN != 0 && tr.head == u32::MAX {
            return None;
        }
        if tr.flags & TypeRefFlags::UNION != 0 {
            let alternatives: Vec<String> = tr
                .args
                .iter()
                .map(|&a| {
                    self.type_ref_text(a, depth + 1)
                        .unwrap_or_else(|| "?".to_string())
                })
                .collect();
            return if alternatives.is_empty() {
                None
            } else {
                Some(alternatives.join(" | "))
            };
        }
        if tr.wildcard == WildcardKind::Unbounded as u8 {
            return Some("?".to_string());
        }
        if tr.wildcard == WildcardKind::Extends as u8 || tr.wildcard == WildcardKind::Super as u8 {
            let keyword = if tr.wildcard == WildcardKind::Extends as u8 {
                "extends"
            } else {
                "super"
            };
            let bound = tr
                .args
                .first()
                .and_then(|&a| self.type_ref_text(a, depth + 1))
                .unwrap_or_else(|| "?".to_string());
            return Some(format!("? {} {}", keyword, bound));
        }
        let head = if tr.flags & TypeRefFlags::PRIMITIVE != 0 {
            primitive_type_name(tr.head)?.to_string()
        } else if tr.head != u32::MAX {
            match self.nested_type_name(tr.head) {
                Some(name) => name,
                None => self.qual_name_text(tr.written_name_id)?,
            }
        } else {
            self.qual_name_text(tr.written_name_id)?
        };
        let mut text = head;
        if !tr.args.is_empty() {
            let args: Vec<String> = tr
                .args
                .iter()
                .map(|&a| {
                    self.type_ref_text(a, depth + 1)
                        .unwrap_or_else(|| "?".to_string())
                })
                .collect();
            text.push('<');
            text.push_str(&args.join(", "));
            text.push('>');
        }
        for _ in 0..tr.array_dims {
            text.push_str("[]");
        }
        if tr.flags & TypeRefFlags::NULLABLE != 0 {
            text.push('?');
        }
        Some(DiagramUtils::truncate(&text, MAX_TYPE_CHARS))
    }

    /// Text of a Phase 3 qualified-name id (`None` for ids outside the table).
    fn qual_name_text(&self, id: u32) -> Option<String> {
        use crate::symbol::QualifiedNameTable;
        if id == u32::MAX || id < QualifiedNameTable::BASE_QUAL_NAME_ID {
            return None;
        }
        self.sta
            .qual_names
            .get((id - QualifiedNameTable::BASE_QUAL_NAME_ID) as usize)
            .filter(|s| !s.is_empty())
            .cloned()
    }

    // ── Structure lookups ────────────────────────────────────────────────────

    /// Child symbols in declaration order.
    pub fn children_of(&self, sym_id: u32) -> &[u32] {
        self.children
            .get(&sym_id)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    pub fn class_record(&self, sym_id: u32) -> Option<&'a ClassRecord> {
        let idx = *self.class_index.get(&sym_id)?;
        let uma: &'a UMLMetadataArtifact = self.uma;
        uma.classes.get(idx)
    }

    pub fn function_cfg(&self, sym_id: u32) -> Option<&'a FunctionCFGData> {
        let cfa = self.cfa?;
        let idx = *self.cfa_index.get(&sym_id)?;
        cfa.functions.get(idx)
    }

    pub fn function_ssa(&self, sym_id: u32) -> Option<&'a FunctionSSAData> {
        let ssa = self.ssa?;
        let idx = *self.ssa_index.get(&sym_id)?;
        ssa.functions.get(idx)
    }

    /// Index of a function in the PSA directory (binary search, directory sorted by sym_id).
    pub fn robdd_index(&self, sym_id: u32) -> Option<usize> {
        self.psa?.function_index(sym_id)
    }

    /// True when `qualified` lies inside the configured focus (always true without a focus).
    /// Both `.` and `::` count as qualified-name separators.
    pub fn matches_focus(&self, qualified: &str) -> bool {
        match self.options.focus.as_deref() {
            None => true,
            Some(focus) if focus.is_empty() => true,
            Some(focus) => {
                qualified == focus
                    || [".", "::"].iter().any(|sep| {
                        qualified.starts_with(&format!("{}{}", focus, sep))
                            || qualified.ends_with(&format!("{}{}", sep, focus))
                    })
            }
        }
    }

    // ── Tokens & source spans ────────────────────────────────────────────────

    pub fn token(&self, token_id: u32) -> Option<&'a TokenEntry> {
        self.source.token(token_id)
    }

    /// (file_id, line, column) of a token.
    pub fn token_position(&self, token_id: u32) -> Option<(u16, u32, u16)> {
        self.source.token_position(token_id)
    }

    pub fn token_text(&self, token_id: u32) -> Option<&'a str> {
        self.source.token_text(token_id)
    }

    pub fn file_path(&self, file_id: u16) -> Option<&str> {
        self.source.file_path(file_id)
    }

    /// Source span covering tokens `first..=last` (clamped to the file of `first`).
    pub fn span_of_tokens(&self, first: u32, last: u32) -> Option<SourceSpan> {
        self.source.span_of_tokens(first, last)
    }

    /// Span of a precomputed UMLLink record, `None` when the link carries the unknown marker.
    pub fn span_of_link(&self, link: &UMLLinkRecord) -> Option<SourceSpan> {
        if !uml_link_is_known(link) || !link.has_source() || link.line_start == UML_LINK_NO_LINE {
            return None;
        }
        Some(SourceSpan {
            file_id: link.file_id,
            path: self.file_path(link.file_id).map(str::to_string),
            line_start: link.line_start,
            col_start: link.col_start as u32,
            line_end: link.line_end.max(link.line_start),
            col_end: link.col_end as u32,
            first_token: u32::MAX,
            last_token: u32::MAX,
        })
    }

    /// Declaration span of a symbol: its own token range, else the TRA backward index.
    pub fn symbol_span(&self, sym_id: u32) -> Option<SourceSpan> {
        if let Some(sym) = self.symbol(sym_id) {
            if sym.first_token_id != u32::MAX {
                if let Some(span) = self.span_of_tokens(sym.first_token_id, sym.last_token_id) {
                    return Some(span);
                }
            }
        }
        let tra = self.tra?;
        let entry = tra.bi_sym.get(sym_id as usize)?;
        if entry.decl_first_tok == u32::MAX {
            return None;
        }
        self.span_of_tokens(entry.decl_first_tok, entry.decl_last_tok)
    }

    /// Token range of a BP-AST statement node (from the BPA, else the TRA AST index).
    pub fn stmt_tokens(&self, stmt_node: u32) -> Option<(u32, u32)> {
        if stmt_node == u32::MAX {
            return None;
        }
        if let Some(bpa) = self.bpa {
            let (first, last) = bpa.token_range(stmt_node);
            if first != u32::MAX {
                return Some((first, last));
            }
        }
        let tra = self.tra?;
        let entry = tra.bi_ast.get(stmt_node as usize)?;
        if entry.first_token_id == u32::MAX {
            None
        } else {
            Some((entry.first_token_id, entry.last_token_id))
        }
    }

    /// Source text of a statement, truncated to `max_chars` characters.
    pub fn stmt_text(&self, stmt_node: u32, max_chars: usize) -> Option<String> {
        let (first, last) = self.stmt_tokens(stmt_node)?;
        self.source_text(first, last, max_chars)
    }

    /// Source text of tokens `first..=last` without comments, joined with conventional
    /// spacing and truncated on a character boundary.
    pub fn source_text(&self, first: u32, last: u32, max_chars: usize) -> Option<String> {
        self.source.source_text(first, last, max_chars)
    }

    /// Condition of a branching statement (`if (c)`, `while (c)`, `for (...)`, `switch (x)`),
    /// read between the header keyword's parentheses. `None` for any other statement shape.
    pub fn condition_text(&self, first: u32, last: u32, max_chars: usize) -> Option<String> {
        self.source.condition_text(first, last, max_chars)
    }
}
