//! Entity Index — stable symbol identity and per-entity content digests (§7.2.3, §7.5.4).
//!
//! `sym_id`s are positions in the STA vector and token ids are corpus-global, so inserting
//! one declaration early in a file renumbers everything after it. Cross-build matching and
//! per-element staleness therefore use two digests per symbol (see [`EntityRecord`]):
//!
//! **Stable key** — SHA-256 (truncated to 128 bits) of
//! `kind ‖ qualified name path ‖ parameter type signature ‖ ordinal`, where
//! - the path is the chain of symbol names from the outermost enclosing symbol down,
//! - the signature (callables) is each parameter's declaration tokens minus its name,
//!   modifiers and annotations, so overloads get distinct keys,
//! - the ordinal disambiguates symbols that are otherwise identical (two locals `i` in one
//!   method), counted in declaration order.
//!
//! **Content digest** — SHA-256 (128 bits) of `kind ‖ modifiers ‖ visibility ‖ arity ‖
//! token stream`, where the token stream is
//! - for type declarations (class, interface, enum, record, annotation type): their own
//!   tokens with every member's span cut out, followed by the members' stable keys — so a
//!   member body edit leaves the type unchanged while adding/removing a member changes it;
//! - for every other symbol: all tokens of its definition (else declaration) span, so a body
//!   edit changes exactly the enclosing callable.
//!
//! Token types and texts are hashed; positions, whitespace, comments filtered by ingestion
//! and file metadata are not, so a `touch` or a pure move changes no digest.

use crate::core::types::symbol::SymbolKind;
use crate::core::types::token::TokenType;
use crate::ingestion::TokenCorpusArtifact;
use crate::symbol::SymbolTableArtifact;
use crate::tra::position::{clamp_span, token_end, token_start};
use crate::tra::types::{BISymEntry, EntityRecord, UML_LINK_NO_FILE, UML_LINK_NO_LINE};
use crate::tra::uml_link::digest::{digest_truncate_128, CanonicalHasher, Digest128};

use std::collections::{HashMap, HashSet};

/// Builder of the per-symbol entity table.
pub struct EntityIndex;

/// True for symbol kinds whose content digest excludes member spans.
fn is_type_declaration(kind: u8) -> bool {
    kind == SymbolKind::SK_CLASS as u8
        || kind == SymbolKind::SK_INTERFACE as u8
        || kind == SymbolKind::SK_ENUM as u8
        || kind == SymbolKind::SK_RECORD as u8
        || kind == SymbolKind::SK_ANNOTATION_TYPE as u8
        || kind == SymbolKind::SK_ANON_CLASS as u8
}

/// True for symbol kinds whose parameters form part of their identity.
fn is_callable(kind: u8) -> bool {
    kind == SymbolKind::SK_METHOD as u8
        || kind == SymbolKind::SK_CONSTRUCTOR as u8
        || kind == SymbolKind::SK_LAMBDA as u8
}

impl EntityIndex {
    /// One [`EntityRecord`] per STA symbol, indexed by sym_id.
    pub fn build(
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
        bi_sym: &[BISymEntry],
    ) -> Vec<EntityRecord> {
        let n = sta.symbol_records.len();
        let ctx = Context { sta, tca, bi_sym };

        // ── Stable keys: identity digests first, then ordinals among equal ones ─
        let mut raw_keys: Vec<Digest128> = Vec::with_capacity(n);
        for sym_id in 0..n {
            raw_keys.push(ctx.identity_digest(sym_id as u32));
        }
        // Ordinal among identical identities, in sym_id (declaration) order.
        let mut seen: HashMap<Digest128, u32> = HashMap::with_capacity(n);
        let mut keys: Vec<Digest128> = Vec::with_capacity(n);
        for raw in &raw_keys {
            let ordinal = seen.entry(*raw).or_insert(0);
            let mut h = CanonicalHasher::new("openheart.entity.key.v1");
            h.fixed(raw);
            h.u32(*ordinal);
            *ordinal += 1;
            keys.push(digest_truncate_128(&h.finish()));
        }

        // ── Content digests and locations ─────────────────────────────────────
        let mut out = Vec::with_capacity(n);
        for sym_id in 0..n {
            let content = ctx.content_digest(sym_id as u32, &keys);
            let (file_id, col_start, line_start, line_end) = ctx.location(sym_id as u32);
            out.push(EntityRecord {
                stable_key: keys[sym_id],
                content_digest: content,
                file_id,
                col_start,
                line_start,
                line_end,
            });
        }
        out
    }
}

struct Context<'a> {
    sta: &'a SymbolTableArtifact,
    tca: &'a TokenCorpusArtifact,
    bi_sym: &'a [BISymEntry],
}

impl<'a> Context<'a> {
    fn name(&self, sym_id: u32) -> &'a [u8] {
        match self.sta.symbol_records.get(sym_id as usize) {
            Some(s) if s.name_id != u32::MAX => self.tca.interner.lookup_text(s.name_id),
            _ => b"",
        }
    }

    /// Valid `(first, last)` token range, clamped to the corpus and to the file of `first`
    /// (a range must never hash or locate tokens of another file).
    fn clamp(&self, first: u32, last: u32) -> Option<(u32, u32)> {
        clamp_span(self.tca, first, last)
    }

    fn decl_range(&self, sym_id: u32) -> Option<(u32, u32)> {
        let e = self.bi_sym.get(sym_id as usize)?;
        self.clamp(e.decl_first_tok, e.decl_last_tok)
    }

    fn def_or_decl_range(&self, sym_id: u32) -> Option<(u32, u32)> {
        let e = self.bi_sym.get(sym_id as usize)?;
        self.clamp(e.def_first_tok, e.def_last_tok)
            .or_else(|| self.clamp(e.decl_first_tok, e.decl_last_tok))
    }

    fn token_text(&self, tok: u32) -> &'a [u8] {
        let rec = &self.tca.token_records[tok as usize];
        self.tca.interner.lookup_text(rec.text_id)
    }

    fn token_type(&self, tok: u32) -> u8 {
        self.tca.token_records[tok as usize].token_type
    }

    /// Direct children of `sym_id` (first_child / next_sibling chain, cycle-safe).
    fn children(&self, sym_id: u32) -> Vec<u32> {
        let n = self.sta.symbol_records.len();
        let mut out = Vec::new();
        let mut visited: HashSet<u32> = HashSet::new();
        visited.insert(sym_id);
        let mut cur = match self.sta.symbol_records.get(sym_id as usize) {
            Some(s) => s.first_child,
            None => return out,
        };
        while cur != u32::MAX && (cur as usize) < n && visited.insert(cur) {
            out.push(cur);
            cur = self.sta.symbol_records[cur as usize].next_sibling;
        }
        out
    }

    /// Kind ‖ name path ‖ signature (callables).
    fn identity_digest(&self, sym_id: u32) -> Digest128 {
        let sym = &self.sta.symbol_records[sym_id as usize];
        let mut h = CanonicalHasher::new("openheart.entity.identity.v1");
        h.u8(sym.kind);

        // Name path, outermost first (parent chain bounded by the symbol count).
        let mut path: Vec<&[u8]> = Vec::new();
        let mut cur = sym_id;
        let mut steps = 0usize;
        while cur != u32::MAX && (cur as usize) < self.sta.symbol_records.len() {
            path.push(self.name(cur));
            steps += 1;
            if steps > self.sta.symbol_records.len() {
                break;
            }
            let parent = self.sta.symbol_records[cur as usize].parent_sym;
            if parent == cur {
                break;
            }
            cur = parent;
        }
        h.seq_len(path.len());
        for name in path.iter().rev() {
            h.bytes(name);
        }

        if is_callable(sym.kind) {
            let params: Vec<u32> = self
                .children(sym_id)
                .into_iter()
                .filter(|&c| self.sta.symbol_records[c as usize].kind == SymbolKind::SK_PARAM as u8)
                .collect();
            h.seq_len(params.len());
            for p in params {
                self.hash_param_type(&mut h, p);
            }
        }
        digest_truncate_128(&h.finish())
    }

    /// A parameter's type tokens: its declaration tokens without its name, annotations and
    /// modifiers.
    fn hash_param_type(&self, h: &mut CanonicalHasher, param: u32) {
        let range = match self.decl_range(param) {
            Some(r) => r,
            None => {
                h.seq_len(0);
                return;
            }
        };
        let name = self.name(param);
        let tokens: Vec<u32> = (range.0..=range.1).collect();
        // The parameter's own name is its last identifier token with that text.
        let name_pos = tokens
            .iter()
            .rposition(|&t| !name.is_empty() && self.token_text(t) == name);
        let mut kept: Vec<u32> = Vec::with_capacity(tokens.len());
        let mut skip_next = false;
        for (i, &t) in tokens.iter().enumerate() {
            if Some(i) == name_pos {
                continue;
            }
            if skip_next {
                skip_next = false;
                continue;
            }
            let ty = self.token_type(t);
            let text = self.token_text(t);
            if ty == TokenType::Annotation as u8 || ty == TokenType::JavaAnnotationMarker as u8 {
                continue;
            }
            if text == b"@" {
                skip_next = true; // `@` followed by the annotation name
                continue;
            }
            if text == b"final" {
                continue;
            }
            kept.push(t);
        }
        h.seq_len(kept.len());
        for t in kept {
            h.u8(self.token_type(t));
            h.bytes(self.token_text(t));
        }
    }

    /// Content digest of `sym_id` (see the module documentation).
    fn content_digest(&self, sym_id: u32, keys: &[Digest128]) -> Digest128 {
        let sym = &self.sta.symbol_records[sym_id as usize];
        let mut h = CanonicalHasher::new("openheart.entity.content.v1");
        h.u8(sym.kind);
        h.u16(sym.modifiers);
        h.u8(sym.visibility);
        h.u8(sym.type_param_count);
        h.u16(sym.param_count);

        let range = self.def_or_decl_range(sym_id);
        if is_type_declaration(sym.kind) {
            let children = self.children(sym_id);
            let mut cut: Vec<(u32, u32)> = children
                .iter()
                .filter_map(|&c| self.def_or_decl_range(c))
                .collect();
            cut.sort_unstable();
            // Own tokens, each preceded by a continuation byte and closed by a terminator
            // (the count is not known up front because member spans are cut out).
            if let Some((first, last)) = range {
                let mut t = first;
                let mut ci = 0usize;
                while t <= last {
                    while ci < cut.len() && cut[ci].1 < t {
                        ci += 1;
                    }
                    if ci < cut.len() && cut[ci].0 <= t {
                        // Inside a member span: jump past it.
                        t = match cut[ci].1.checked_add(1) {
                            Some(next) => next,
                            None => break,
                        };
                        continue;
                    }
                    h.u8(1);
                    h.u8(self.token_type(t));
                    h.bytes(self.token_text(t));
                    t = match t.checked_add(1) {
                        Some(next) => next,
                        None => break,
                    };
                }
            }
            h.u8(0);
            h.seq_len(children.len());
            for c in children {
                h.fixed(&keys[c as usize]);
            }
        } else {
            match range {
                Some((first, last)) => {
                    h.seq_len((last - first + 1) as usize);
                    for t in first..=last {
                        h.u8(self.token_type(t));
                        h.bytes(self.token_text(t));
                    }
                }
                None => h.seq_len(0),
            }
        }
        digest_truncate_128(&h.finish())
    }

    /// `(file_id, col_start, line_start, line_end)` of the declaration span, or sentinels.
    /// `line_end` is the line of the last token's last byte.
    fn location(&self, sym_id: u32) -> (u16, u16, u32, u32) {
        match self.decl_range(sym_id) {
            Some((first, last)) => {
                let (file, line_start, col) = token_start(self.tca, first);
                let (line_end, _) = token_end(self.tca, last);
                (file, col, line_start, line_end)
            }
            None => (UML_LINK_NO_FILE, u16::MAX, UML_LINK_NO_LINE, UML_LINK_NO_LINE),
        }
    }
}
