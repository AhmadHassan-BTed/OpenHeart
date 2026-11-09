//! Token-level syntax recognizers for constructs that Phase 2 drops or flattens.
//!
//! Every recognizer starts at a token position that the AST (or an enclosing recognizer)
//! identified structurally and follows the grammar production for that position; none of them
//! searches a token window for a plausible identifier. Brackets and punctuation are matched by
//! token kind as well as text, so string fragments that contain `{` or `(` never unbalance a scan.
//!
//! Phase 1 does not emit every terminal: for Java, `?`, `:`, `permits`, `sealed`, `non-sealed`
//! and `@interface` are absent from the token corpus. The recognizers below account for those
//! gaps explicitly where they matter (wildcards, `permits` lists).

use crate::core::types::symbol::{SymbolModifiers, SymbolVisibility};
use crate::core::types::token::{unpack_sort_key, TokenType};
use crate::ingestion::TokenCorpusArtifact;
use crate::symbol::model::Dialect;
use std::collections::BTreeMap;

/// Read-only view over the token corpus with kind-aware predicates.
#[derive(Clone, Copy)]
pub struct TokenView<'a> {
    tca: &'a TokenCorpusArtifact,
}

impl<'a> TokenView<'a> {
    pub fn new(tca: &'a TokenCorpusArtifact) -> Self {
        Self { tca }
    }

    pub fn len(&self) -> u32 {
        u32::try_from(self.tca.token_records.len()).unwrap_or(u32::MAX)
    }

    pub fn is_empty(&self) -> bool {
        self.tca.token_records.is_empty()
    }

    pub fn valid(&self, tok: u32) -> bool {
        (tok as usize) < self.tca.token_records.len()
    }

    pub fn text_id(&self, tok: u32) -> u32 {
        self.tca
            .token_records
            .get(tok as usize)
            .map(|r| r.text_id)
            .unwrap_or(u32::MAX)
    }

    pub fn text(&self, tok: u32) -> &'a [u8] {
        let tca: &'a TokenCorpusArtifact = self.tca;
        match tca.token_records.get(tok as usize) {
            Some(r) => tca.interner.lookup_text(r.text_id),
            None => b"",
        }
    }

    pub fn text_str(&self, tok: u32) -> &'a str {
        std::str::from_utf8(self.text(tok)).unwrap_or("")
    }

    /// Text of an interned id of this corpus.
    pub fn interned(&self, text_id: u32) -> &'a str {
        let tca: &'a TokenCorpusArtifact = self.tca;
        std::str::from_utf8(tca.interner.lookup_text(text_id)).unwrap_or("")
    }

    /// Interned id of `text`, or `u32::MAX` when no token of the corpus has that text.
    pub fn find_text_id(&self, text: &str) -> u32 {
        self.tca.interner.find_id(text.as_bytes())
    }

    pub fn kind(&self, tok: u32) -> TokenType {
        self.tca
            .token_records
            .get(tok as usize)
            .map(|r| TokenType::from_u8(r.token_type))
            .unwrap_or(TokenType::Unknown)
    }

    pub fn file(&self, tok: u32) -> Option<u16> {
        self.tca
            .token_records
            .get(tok as usize)
            .map(|r| unpack_sort_key(r.sort_key).0)
    }

    pub fn line(&self, tok: u32) -> u32 {
        self.tca
            .token_records
            .get(tok as usize)
            .map(|r| unpack_sort_key(r.sort_key).1)
            .unwrap_or(0)
    }

    pub fn is_trivia(&self, tok: u32) -> bool {
        matches!(
            self.kind(tok),
            TokenType::CommentLine
                | TokenType::CommentBlock
                | TokenType::CommentDoc
                | TokenType::Whitespace
                | TokenType::Newline
        )
    }

    /// Punctuation or operator token with exactly this text.
    pub fn punct(&self, tok: u32, s: &str) -> bool {
        matches!(
            self.kind(tok),
            TokenType::Punctuation | TokenType::Operator | TokenType::JavaAnnotationMarker
        ) && self.text(tok) == s.as_bytes()
    }

    /// Keyword or identifier token with exactly this text.
    pub fn word(&self, tok: u32, s: &str) -> bool {
        matches!(
            self.kind(tok),
            TokenType::Keyword
                | TokenType::SoftKeyword
                | TokenType::Identifier
                | TokenType::JavaVarKeyword
                | TokenType::JavaSealedKeyword
        ) && self.text(tok) == s.as_bytes()
    }

    fn identifier_text(&self, tok: u32) -> bool {
        match self.text(tok).first() {
            Some(&b) => b.is_ascii_alphabetic() || b == b'_' || b == b'$' || b == b'`' || b >= 0x80,
            None => false,
        }
    }

    /// Identifier token (contextual keywords that are valid identifiers included).
    pub fn is_ident(&self, tok: u32) -> bool {
        matches!(
            self.kind(tok),
            TokenType::Identifier | TokenType::SoftKeyword
        ) && self.identifier_text(tok)
    }

    /// Identifier token, including annotation names (which Phase 1 types `Annotation`).
    pub fn is_name(&self, tok: u32) -> bool {
        (self.is_ident(tok) || self.kind(tok) == TokenType::Annotation) && self.identifier_text(tok)
    }

    /// First significant (non-comment) token at or after `pos` and before `limit`.
    pub fn sig(&self, pos: u32, limit: u32) -> Option<u32> {
        let limit = limit.min(self.len());
        let mut i = pos;
        while i < limit {
            if !self.is_trivia(i) {
                return Some(i);
            }
            i += 1;
        }
        None
    }

    /// Next significant token strictly after `tok` and before `limit`.
    pub fn next(&self, tok: u32, limit: u32) -> Option<u32> {
        self.sig(tok.checked_add(1)?, limit)
    }

    /// Previous significant token strictly before `tok` and at or after `floor`.
    pub fn prev(&self, tok: u32, floor: u32) -> Option<u32> {
        let mut i = tok.min(self.len());
        while i > floor {
            i -= 1;
            if !self.is_trivia(i) {
                return Some(i);
            }
        }
        None
    }

    pub fn is_opener(&self, tok: u32) -> bool {
        self.punct(tok, "(") || self.punct(tok, "[") || self.punct(tok, "{")
    }

    pub fn is_closer(&self, tok: u32) -> bool {
        self.punct(tok, ")") || self.punct(tok, "]") || self.punct(tok, "}")
    }

    fn closer_for(&self, open: u32) -> Option<&'static str> {
        if self.punct(open, "(") {
            Some(")")
        } else if self.punct(open, "[") {
            Some("]")
        } else if self.punct(open, "{") {
            Some("}")
        } else {
            None
        }
    }

    fn opener_for(&self, close: u32) -> Option<&'static str> {
        if self.punct(close, ")") {
            Some("(")
        } else if self.punct(close, "]") {
            Some("[")
        } else if self.punct(close, "}") {
            Some("{")
        } else {
            None
        }
    }

    /// Token that closes the bracket opened at `open`, or `None` when the brackets in
    /// `[open, limit)` are unbalanced or mismatched.
    pub fn matching_close(&self, open: u32, limit: u32) -> Option<u32> {
        self.closer_for(open)?;
        let limit = limit.min(self.len());
        let mut stack: Vec<&'static str> = Vec::new();
        let mut i = open;
        while i < limit {
            if !self.is_trivia(i) {
                if let Some(closer) = self.closer_for(i) {
                    stack.push(closer);
                } else if self.is_closer(i) {
                    let expected = stack.pop()?;
                    if !self.punct(i, expected) {
                        return None;
                    }
                    if stack.is_empty() {
                        return Some(i);
                    }
                }
            }
            i += 1;
        }
        None
    }

    /// Token that opens the bracket closed at `close`, scanning backwards to `floor`.
    pub fn matching_open(&self, close: u32, floor: u32) -> Option<u32> {
        self.opener_for(close)?;
        let mut stack: Vec<&'static str> = Vec::new();
        let mut i = close.checked_add(1)?.min(self.len());
        while i > floor {
            i -= 1;
            if self.is_trivia(i) {
                continue;
            }
            if let Some(opener) = self.opener_for(i) {
                stack.push(opener);
            } else if self.is_opener(i) {
                let expected = stack.pop()?;
                if !self.punct(i, expected) {
                    return None;
                }
                if stack.is_empty() {
                    return Some(i);
                }
            }
        }
        None
    }

    /// `[start, end)` token range of every file in the corpus (tokens are sorted by file).
    pub fn file_ranges(&self) -> BTreeMap<u16, (u32, u32)> {
        let mut out: BTreeMap<u16, (u32, u32)> = BTreeMap::new();
        for (i, rec) in self.tca.token_records.iter().enumerate() {
            let fid = unpack_sort_key(rec.sort_key).0;
            let i = u32::try_from(i).unwrap_or(u32::MAX);
            out.entry(fid)
                .and_modify(|r| {
                    r.0 = r.0.min(i);
                    r.1 = r.1.max(i.saturating_add(1));
                })
                .or_insert((i, i.saturating_add(1)));
        }
        out
    }
}

// ── Modifiers and annotations ───────────────────────────────────────────────

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModifierScan {
    pub modifiers: u16,
    pub visibility: Option<SymbolVisibility>,
    /// Token of the simple name of each annotation (`Deprecated` for `@java.lang.Deprecated`).
    pub annotations: Vec<u32>,
    /// Tokens of the modifier keywords.
    pub keyword_tokens: Vec<u32>,
    /// First position after the modifier list.
    pub end: u32,
}

/// Skips `@Name(.Name)*[(...)]` starting at the `@` token. Returns the token of the annotation's
/// simple name and the position after the annotation.
pub fn skip_annotation(tv: &TokenView, at: u32, limit: u32) -> Option<(u32, u32)> {
    if !tv.punct(at, "@") {
        return None;
    }
    let mut name = tv.next(at, limit)?;
    if !tv.is_name(name) {
        return None;
    }
    // Kotlin use-site target: `@field:Json(...)`.
    if let Some(colon) = tv.next(name, limit) {
        if tv.punct(colon, ":") && tv.line(colon) == tv.line(name) {
            if let Some(target) = tv.next(colon, limit) {
                if tv.is_name(target) {
                    name = target;
                }
            }
        }
    }
    let mut pos = name + 1;
    while let Some(dot) = tv.sig(pos, limit) {
        if !tv.punct(dot, ".") {
            break;
        }
        match tv.next(dot, limit) {
            Some(seg) if tv.is_name(seg) => {
                name = seg;
                pos = seg + 1;
            }
            _ => break,
        }
    }
    if let Some(open) = tv.sig(pos, limit) {
        if tv.punct(open, "(") {
            let close = tv.matching_close(open, limit)?;
            pos = close + 1;
        }
    }
    Some((name, pos))
}

fn java_modifier(text: &[u8]) -> Option<(u16, Option<SymbolVisibility>)> {
    Some(match text {
        b"public" => (0, Some(SymbolVisibility::Public)),
        b"protected" => (0, Some(SymbolVisibility::Protected)),
        b"private" => (0, Some(SymbolVisibility::Private)),
        b"static" => (SymbolModifiers::STATIC, None),
        b"final" => (SymbolModifiers::FINAL, None),
        b"abstract" => (SymbolModifiers::ABSTRACT, None),
        b"synchronized" => (SymbolModifiers::SYNCHRONIZED, None),
        b"native" => (SymbolModifiers::NATIVE, None),
        b"volatile" => (SymbolModifiers::VOLATILE, None),
        b"transient" => (SymbolModifiers::TRANSIENT, None),
        b"strictfp" => (SymbolModifiers::STRICTFP, None),
        b"default" => (SymbolModifiers::DEFAULT, None),
        b"sealed" => (SymbolModifiers::SEALED, None),
        b"non-sealed" => (SymbolModifiers::NON_SEALED, None),
        _ => return None,
    })
}

/// Scans the Java modifier list (keywords and annotations, in any order) starting at `start`.
/// Keywords are matched as whole tokens, so annotation arguments such as
/// `@Column(name = "is_public")` never contribute modifiers.
pub fn scan_java_modifiers(tv: &TokenView, start: u32, limit: u32) -> ModifierScan {
    let mut scan = ModifierScan {
        end: start,
        ..Default::default()
    };
    let mut pos = start;
    while let Some(t) = tv.sig(pos, limit) {
        if tv.punct(t, "@") {
            match skip_annotation(tv, t, limit) {
                Some((name, next)) => {
                    scan.annotations.push(name);
                    pos = next;
                    continue;
                }
                None => {
                    pos = t;
                    break;
                }
            }
        }
        if tv.word(t, "non") {
            let sealed = tv
                .next(t, limit)
                .filter(|&dash| tv.punct(dash, "-"))
                .and_then(|dash| tv.next(dash, limit))
                .filter(|&s| tv.word(s, "sealed"));
            if let Some(s) = sealed {
                scan.modifiers |= SymbolModifiers::NON_SEALED;
                scan.keyword_tokens.push(t);
                pos = s + 1;
                continue;
            }
        }
        let is_word = matches!(
            tv.kind(t),
            TokenType::Keyword | TokenType::JavaSealedKeyword | TokenType::Identifier
        );
        match (is_word, java_modifier(tv.text(t))) {
            (true, Some((bits, vis))) => {
                scan.modifiers |= bits;
                if vis.is_some() {
                    scan.visibility = vis;
                }
                scan.keyword_tokens.push(t);
                pos = t + 1;
            }
            _ => {
                pos = t;
                break;
            }
        }
    }
    scan.end = pos;
    scan
}

// ── Types ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameSegment {
    pub tok: u32,
    pub args: Vec<TypeSyntax>,
    /// An argument list `<>` with no argument tokens was written. In a declaration this is a
    /// Java `<?>` whose `?` Phase 1 does not tokenize; after `new` it is the diamond.
    pub empty_args: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WildcardSyntax {
    Unbounded,
    Extends(Box<TypeSyntax>),
    Super(Box<TypeSyntax>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeBase {
    Primitive(u32),
    Named(Vec<NameSegment>),
    Wildcard(WildcardSyntax),
    /// `var` (JLS 14.4.1).
    Inferred(u32),
    /// Kotlin function type `(A) -> B`.
    Function,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeSyntax {
    pub base: TypeBase,
    pub dims: u8,
    pub varargs: bool,
    pub nullable: bool,
    pub annotations: Vec<u32>,
    pub first: u32,
    pub last: u32,
}

impl TypeSyntax {
    fn wildcard(w: WildcardSyntax, first: u32, last: u32) -> Self {
        Self {
            base: TypeBase::Wildcard(w),
            dims: 0,
            varargs: false,
            nullable: false,
            annotations: Vec::new(),
            first,
            last,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct TypeOptions {
    pub allow_varargs: bool,
    pub allow_var: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeParamSyntax {
    pub name_tok: u32,
    pub bounds: Vec<TypeSyntax>,
    pub first: u32,
    pub last: u32,
}

pub fn is_java_primitive(text: &[u8]) -> bool {
    matches!(
        text,
        b"boolean" | b"byte" | b"short" | b"char" | b"int" | b"long" | b"float" | b"double"
            | b"void"
    )
}

/// Recursive-descent parser for written types. A closing `>>`/`>>>` token closes several
/// argument lists at once; the surplus is carried in `pending_gt`.
pub struct TypeParser<'v, 'a> {
    tv: &'v TokenView<'a>,
    limit: u32,
    dialect: Dialect,
    pending_gt: u8,
    pending_tok: u32,
}

impl<'v, 'a> TypeParser<'v, 'a> {
    pub fn new(tv: &'v TokenView<'a>, limit: u32, dialect: Dialect) -> Self {
        Self {
            tv,
            limit,
            dialect,
            pending_gt: 0,
            pending_tok: u32::MAX,
        }
    }

    /// Parses one type starting at `pos`; returns it and the position after it.
    pub fn parse(&mut self, pos: u32, opts: TypeOptions) -> Option<(TypeSyntax, u32)> {
        self.pending_gt = 0;
        let result = self.parse_type(pos, opts)?;
        if self.pending_gt != 0 {
            return None;
        }
        Some(result)
    }

    /// Parses a type parameter list starting at its `<` token. Returns the parameters, the
    /// position after the list and the closing token.
    /// Parses a single type parameter (annotations, name, bounds) starting at `pos`, as the
    /// node of one `type_parameter` covers it. Returns the parameter and the position after
    /// it; a closing `>>` consumed by a bound is left pending.
    pub fn parse_type_param(&mut self, pos: u32) -> Option<(TypeParamSyntax, u32)> {
        let tv = self.tv;
        let mut first = u32::MAX;
        let mut p = pos;
        loop {
            let t = tv.sig(p, self.limit)?;
            if tv.punct(t, "@") {
                let (_, next) = skip_annotation(tv, t, self.limit)?;
                if first == u32::MAX {
                    first = t;
                }
                p = next;
            } else if self.dialect == Dialect::Kotlin
                && (tv.word(t, "reified") || tv.word(t, "in") || tv.word(t, "out"))
            {
                if first == u32::MAX {
                    first = t;
                }
                p = t + 1;
            } else {
                break;
            }
        }
        let name = tv.sig(p, self.limit)?;
        if !tv.is_ident(name) {
            return None;
        }
        if first == u32::MAX {
            first = name;
        }
        let mut bounds = Vec::new();
        let mut last = name;
        let mut pos = name + 1;
        if let Some(t) = tv.sig(pos, self.limit) {
            let opens_bound = match self.dialect {
                Dialect::Kotlin => tv.punct(t, ":"),
                _ => tv.word(t, "extends"),
            };
            if opens_bound {
                let mut bp = t + 1;
                loop {
                    let (bound, next) = self.parse_type(bp, TypeOptions::default())?;
                    last = bound.last;
                    bounds.push(bound);
                    pos = next;
                    if self.pending_gt > 0 {
                        break;
                    }
                    match tv.sig(pos, self.limit) {
                        Some(amp) if tv.punct(amp, "&") => bp = amp + 1,
                        _ => break,
                    }
                }
            }
        }
        Some((
            TypeParamSyntax {
                name_tok: name,
                bounds,
                first,
                last,
            },
            pos,
        ))
    }

    pub fn parse_type_params(&mut self, lt: u32) -> Option<(Vec<TypeParamSyntax>, u32, u32)> {
        let tv = self.tv;
        self.pending_gt = 0;
        if !tv.punct(lt, "<") {
            return None;
        }
        let mut params = Vec::new();
        let mut pos = lt + 1;
        loop {
            let (param, next) = self.parse_type_param(pos)?;
            pos = next;
            params.push(param);
            if self.pending_gt > 0 {
                self.pending_gt -= 1;
                if self.pending_gt != 0 {
                    return None;
                }
                return Some((params, pos, self.pending_tok));
            }
            let t = tv.sig(pos, self.limit)?;
            if tv.punct(t, ",") {
                pos = t + 1;
                continue;
            }
            let (next, close) = self.close_angle(pos)?;
            if self.pending_gt != 0 {
                return None;
            }
            return Some((params, next, close));
        }
    }

    fn close_angle(&mut self, pos: u32) -> Option<(u32, u32)> {
        if self.pending_gt > 0 {
            self.pending_gt -= 1;
            return Some((pos, self.pending_tok));
        }
        let t = self.tv.sig(pos, self.limit)?;
        let extra = if self.tv.punct(t, ">") {
            0
        } else if self.tv.punct(t, ">>") {
            1
        } else if self.tv.punct(t, ">>>") {
            2
        } else {
            return None;
        };
        self.pending_gt = extra;
        self.pending_tok = t;
        Some((t + 1, t))
    }

    fn is_angle_closer(&self, tok: u32) -> bool {
        self.tv.punct(tok, ">") || self.tv.punct(tok, ">>") || self.tv.punct(tok, ">>>")
    }

    fn skip_leading_annotations(&self, pos: u32) -> Option<(Vec<u32>, u32, u32)> {
        let tv = self.tv;
        let mut annotations = Vec::new();
        let mut first = u32::MAX;
        let mut pos = pos;
        loop {
            let t = tv.sig(pos, self.limit)?;
            if !tv.punct(t, "@") {
                break;
            }
            let (name, next) = skip_annotation(tv, t, self.limit)?;
            if first == u32::MAX {
                first = t;
            }
            annotations.push(name);
            pos = next;
        }
        Some((annotations, first, pos))
    }

    fn parse_type(&mut self, pos: u32, opts: TypeOptions) -> Option<(TypeSyntax, u32)> {
        let tv = self.tv;
        let (annotations, mut first, mut pos) = self.skip_leading_annotations(pos)?;
        let t = tv.sig(pos, self.limit)?;
        if first == u32::MAX {
            first = t;
        }
        let mut last = t;
        let base;

        if self.dialect == Dialect::Java
            && tv.kind(t) == TokenType::Keyword
            && is_java_primitive(tv.text(t))
        {
            base = TypeBase::Primitive(t);
            pos = t + 1;
        } else if self.dialect == Dialect::Java
            && opts.allow_var
            && tv.word(t, "var")
            && tv.next(t, self.limit).is_some_and(|n| tv.is_ident(n))
        {
            base = TypeBase::Inferred(t);
            pos = t + 1;
        } else if self.dialect == Dialect::Kotlin
            && tv.word(t, "suspend")
            && tv.next(t, self.limit).is_some_and(|n| tv.punct(n, "("))
        {
            return self.parse_type(t + 1, opts);
        } else if self.dialect == Dialect::Kotlin && tv.punct(t, "(") {
            let close = tv.matching_close(t, self.limit)?;
            match tv.next(close, self.limit) {
                Some(arrow) if tv.punct(arrow, "->") => {
                    let (ret, next) = self.parse_type(arrow + 1, TypeOptions::default())?;
                    base = TypeBase::Function;
                    last = ret.last;
                    pos = next;
                }
                _ => {
                    let (mut inner, next) = self.parse_type(t + 1, opts)?;
                    if tv.sig(next, self.limit)? != close {
                        return None;
                    }
                    inner.first = first;
                    inner.last = close;
                    pos = close + 1;
                    if let Some(q) = tv.sig(pos, self.limit) {
                        if tv.punct(q, "?") {
                            inner.nullable = true;
                            inner.last = q;
                            pos = q + 1;
                        }
                    }
                    return Some((inner, pos));
                }
            }
        } else if tv.is_ident(t) {
            let mut segments = Vec::new();
            let mut seg = t;
            loop {
                let mut args = Vec::new();
                let mut empty_args = false;
                last = seg;
                pos = seg + 1;
                if self.pending_gt == 0 {
                    if let Some(lt) = tv.sig(pos, self.limit) {
                        if tv.punct(lt, "<") {
                            let (parsed, next, close) = self.parse_type_args(lt)?;
                            empty_args = parsed.is_empty();
                            args = parsed;
                            pos = next;
                            last = close;
                        }
                    }
                }
                segments.push(NameSegment {
                    tok: seg,
                    args,
                    empty_args,
                });
                if self.pending_gt > 0 {
                    break;
                }
                let dot = match tv.sig(pos, self.limit) {
                    Some(d) if tv.punct(d, ".") => d,
                    _ => break,
                };
                let mut p = dot + 1;
                while let Some(at) = tv.sig(p, self.limit) {
                    if !tv.punct(at, "@") {
                        break;
                    }
                    let (_, next) = skip_annotation(tv, at, self.limit)?;
                    p = next;
                }
                match tv.sig(p, self.limit) {
                    // `Foo.class`, `Foo.this`, `pkg.*` and the like end the type before the dot.
                    Some(next_seg) if tv.is_ident(next_seg) => seg = next_seg,
                    _ => break,
                }
            }
            base = TypeBase::Named(segments);
        } else {
            return None;
        }

        let mut dims = 0u8;
        let mut varargs = false;
        let mut nullable = false;
        if self.pending_gt == 0 {
            if self.dialect == Dialect::Kotlin {
                if let Some(q) = tv.sig(pos, self.limit) {
                    if tv.punct(q, "?") {
                        nullable = true;
                        last = q;
                        pos = q + 1;
                    }
                }
            } else {
                loop {
                    let mut p = pos;
                    while let Some(at) = tv.sig(p, self.limit) {
                        if !tv.punct(at, "@") {
                            break;
                        }
                        match skip_annotation(tv, at, self.limit) {
                            Some((_, next)) => p = next,
                            None => break,
                        }
                    }
                    let open = match tv.sig(p, self.limit) {
                        Some(o) if tv.punct(o, "[") => o,
                        _ => break,
                    };
                    match tv.next(open, self.limit) {
                        Some(close) if tv.punct(close, "]") => {
                            dims = dims.saturating_add(1);
                            last = close;
                            pos = close + 1;
                        }
                        _ => break,
                    }
                }
                if opts.allow_varargs {
                    if let Some(e) = tv.sig(pos, self.limit) {
                        if tv.punct(e, "...") {
                            varargs = true;
                            dims = dims.saturating_add(1);
                            last = e;
                            pos = e + 1;
                        }
                    }
                }
            }
        }

        Some((
            TypeSyntax {
                base,
                dims,
                varargs,
                nullable,
                annotations,
                first,
                last,
            },
            pos,
        ))
    }

    fn parse_type_args(&mut self, lt: u32) -> Option<(Vec<TypeSyntax>, u32, u32)> {
        let mut args = Vec::new();
        let mut pos = lt + 1;
        if let Some((next, close)) = self.close_angle(pos) {
            return Some((args, next, close));
        }
        loop {
            let (arg, next) = self.parse_type_arg(pos)?;
            args.push(arg);
            pos = next;
            if self.pending_gt > 0 {
                self.pending_gt -= 1;
                return Some((args, pos, self.pending_tok));
            }
            let t = self.tv.sig(pos, self.limit)?;
            if self.tv.punct(t, ",") {
                pos = t + 1;
                continue;
            }
            let (next, close) = self.close_angle(pos)?;
            return Some((args, next, close));
        }
    }

    fn parse_type_arg(&mut self, pos: u32) -> Option<(TypeSyntax, u32)> {
        let tv = self.tv;
        let t = tv.sig(pos, self.limit)?;
        if self.dialect == Dialect::Kotlin {
            if tv.punct(t, "*") {
                return Some((TypeSyntax::wildcard(WildcardSyntax::Unbounded, t, t), t + 1));
            }
            let is_out = tv.word(t, "out");
            if is_out || tv.word(t, "in") {
                let (bound, next) = self.parse_type(t + 1, TypeOptions::default())?;
                let last = bound.last;
                let w = if is_out {
                    WildcardSyntax::Extends(Box::new(bound))
                } else {
                    WildcardSyntax::Super(Box::new(bound))
                };
                return Some((TypeSyntax::wildcard(w, t, last), next));
            }
            return self.parse_type(t, TypeOptions::default());
        }

        // Java. Phase 1 drops the `?` token, so a wildcard is either nothing (unbounded) or a
        // bare `extends`/`super` bound. A `?` that is present is consumed as well.
        let (start, qmark) = if tv.punct(t, "?") {
            (tv.next(t, self.limit)?, Some(t))
        } else {
            (t, None)
        };
        let wild_first = qmark.unwrap_or(start);
        if tv.punct(start, ",") || self.is_angle_closer(start) {
            let last = qmark.unwrap_or(start);
            let next = qmark.map(|q| q + 1).unwrap_or(start);
            return Some((
                TypeSyntax::wildcard(WildcardSyntax::Unbounded, wild_first, last),
                next,
            ));
        }
        let is_extends = tv.word(start, "extends");
        if is_extends || tv.word(start, "super") {
            let (bound, next) = self.parse_type(start + 1, TypeOptions::default())?;
            let last = bound.last;
            let w = if is_extends {
                WildcardSyntax::Extends(Box::new(bound))
            } else {
                WildcardSyntax::Super(Box::new(bound))
            };
            return Some((TypeSyntax::wildcard(w, wild_first, last), next));
        }
        if qmark.is_some() {
            return None;
        }
        self.parse_type(start, TypeOptions::default())
    }
}

/// Parses `[ ]` pairs (C-style array dimensions after a declarator name).
pub fn parse_trailing_dims(tv: &TokenView, pos: u32, limit: u32) -> (u8, u32, u32) {
    let mut dims = 0u8;
    let mut pos = pos;
    let mut last = u32::MAX;
    while let Some(open) = tv.sig(pos, limit) {
        if !tv.punct(open, "[") {
            break;
        }
        match tv.next(open, limit) {
            Some(close) if tv.punct(close, "]") => {
                dims = dims.saturating_add(1);
                last = close;
                pos = close + 1;
            }
            _ => break,
        }
    }
    (dims, pos, last)
}

/// Skips an expression up to (not including) the first `,` or `;` at bracket depth zero.
/// Returns the position of that delimiter and the last token of the expression.
pub fn skip_expression(tv: &TokenView, pos: u32, limit: u32) -> Option<(u32, u32)> {
    let mut pos = pos;
    let mut last = u32::MAX;
    while let Some(t) = tv.sig(pos, limit) {
        if tv.punct(t, ",") || tv.punct(t, ";") {
            return Some((t, last));
        }
        if tv.is_opener(t) {
            let close = tv.matching_close(t, limit)?;
            last = close;
            pos = close + 1;
            continue;
        }
        if tv.is_closer(t) {
            return None;
        }
        last = t;
        pos = t + 1;
    }
    None
}

fn skip_past_semicolon(tv: &TokenView, pos: u32, limit: u32) -> Option<u32> {
    let mut pos = pos;
    while let Some(t) = tv.sig(pos, limit) {
        if tv.punct(t, ";") {
            return Some(t + 1);
        }
        if tv.is_opener(t) {
            pos = tv.matching_close(t, limit)? + 1;
            continue;
        }
        pos = t + 1;
    }
    None
}

// ── Type declaration headers ────────────────────────────────────────────────

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TypeHeaderSyntax {
    pub type_params: Vec<TypeParamSyntax>,
    pub extends: Vec<TypeSyntax>,
    pub implements: Vec<TypeSyntax>,
    pub permits: Vec<TypeSyntax>,
    pub body_open: Option<u32>,
    pub malformed: bool,
}

#[derive(Clone, Copy)]
enum HeaderClause {
    Extends,
    Implements,
    Permits,
}

/// Parses a Java type header after its name token: type parameters, record components,
/// `extends`, `implements` and `permits` clauses, up to the opening brace of the body.
pub fn parse_java_type_header(tv: &TokenView, name_tok: u32, limit: u32) -> TypeHeaderSyntax {
    let mut h = TypeHeaderSyntax::default();
    let mut parser = TypeParser::new(tv, limit, Dialect::Java);
    let mut pos = name_tok + 1;

    if let Some(t) = tv.sig(pos, limit) {
        if tv.punct(t, "<") {
            match parser.parse_type_params(t) {
                Some((params, next, _)) => {
                    h.type_params = params;
                    pos = next;
                }
                None => {
                    h.malformed = true;
                    return h;
                }
            }
        }
    }
    if let Some(t) = tv.sig(pos, limit) {
        if tv.punct(t, "(") {
            match tv.matching_close(t, limit) {
                Some(close) => pos = close + 1,
                None => {
                    h.malformed = true;
                    return h;
                }
            }
        }
    }

    while let Some(t) = tv.sig(pos, limit) {
        if tv.punct(t, "{") {
            h.body_open = Some(t);
            break;
        }
        let (clause, start) = if tv.word(t, "extends") {
            (HeaderClause::Extends, t + 1)
        } else if tv.word(t, "implements") {
            (HeaderClause::Implements, t + 1)
        } else if tv.word(t, "permits") {
            (HeaderClause::Permits, t + 1)
        } else if tv.is_ident(t) {
            // Phase 1 does not tokenize `permits`, so a type name in clause position is the
            // first entry of a `permits` list.
            (HeaderClause::Permits, t)
        } else {
            h.malformed = true;
            break;
        };
        let mut list = Vec::new();
        let mut p = start;
        let mut ok = true;
        loop {
            match parser.parse(p, TypeOptions::default()) {
                Some((ty, next)) => {
                    list.push(ty);
                    p = next;
                }
                None => {
                    ok = false;
                    break;
                }
            }
            match tv.sig(p, limit) {
                Some(c) if tv.punct(c, ",") => p = c + 1,
                _ => break,
            }
        }
        match clause {
            HeaderClause::Extends => h.extends.extend(list),
            HeaderClause::Implements => h.implements.extend(list),
            HeaderClause::Permits => h.permits.extend(list),
        }
        if !ok {
            h.malformed = true;
            break;
        }
        pos = p;
    }
    h
}

// ── Compilation unit header (package and imports) ───────────────────────────

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportSyntax {
    pub is_static: bool,
    pub segments: Vec<u32>,
    pub on_demand: bool,
    /// Kotlin `import a.B as C`: token of `C`, otherwise `u32::MAX`.
    pub alias: u32,
    pub first: u32,
    pub last: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CompilationUnitHeader {
    pub package: Vec<u32>,
    pub package_token: u32,
    pub imports: Vec<ImportSyntax>,
    /// First position after the header.
    pub end: u32,
}

/// Parses the package declaration and import declarations of the file whose tokens occupy
/// `[start, end)`. Phase 2 drops both, so they are recovered here from the token stream.
pub fn parse_compilation_unit_header(
    tv: &TokenView,
    start: u32,
    end: u32,
    dialect: Dialect,
) -> CompilationUnitHeader {
    let mut h = CompilationUnitHeader {
        package_token: u32::MAX,
        end: start,
        ..Default::default()
    };
    let mut pos = start;

    // Package annotations (`package-info.java`, Kotlin `@file:`) belong to the header only when
    // a package declaration follows; otherwise they annotate the first type declaration.
    let mut p = pos;
    while let Some(t) = tv.sig(p, end) {
        if !tv.punct(t, "@") {
            break;
        }
        match skip_annotation(tv, t, end) {
            Some((_, next)) => p = next,
            None => break,
        }
    }
    if let Some(t) = tv.sig(p, end) {
        if tv.word(t, "package") {
            if let Some((segments, next, _)) = parse_dotted_name(tv, t, end, dialect) {
                h.package = segments;
                h.package_token = t;
                pos = next;
            }
        }
    }

    while let Some(t) = tv.sig(pos, end) {
        if tv.punct(t, ";") {
            pos = t + 1;
            continue;
        }
        if !tv.word(t, "import") {
            break;
        }
        match parse_import(tv, t, end, dialect) {
            Some((import, next)) => {
                h.imports.push(import);
                pos = next;
            }
            None => break,
        }
    }
    h.end = pos;
    h
}

/// Parses `keyword Ident(.Ident)* [;]` (package declaration). Kotlin declarations end at the
/// end of the line when no semicolon is written.
fn parse_dotted_name(
    tv: &TokenView,
    keyword: u32,
    end: u32,
    dialect: Dialect,
) -> Option<(Vec<u32>, u32, u32)> {
    let line = tv.line(keyword);
    let mut segments = Vec::new();
    let mut pos = keyword + 1;
    loop {
        let seg = tv.sig(pos, end)?;
        if !tv.is_ident(seg) || (dialect == Dialect::Kotlin && tv.line(seg) != line) {
            return None;
        }
        segments.push(seg);
        pos = seg + 1;
        match tv.sig(pos, end) {
            Some(dot) if tv.punct(dot, ".") => pos = dot + 1,
            Some(semi) if tv.punct(semi, ";") => return Some((segments, semi + 1, semi)),
            _ if dialect == Dialect::Kotlin => return Some((segments, pos, seg)),
            _ => return None,
        }
    }
}

fn parse_import(
    tv: &TokenView,
    keyword: u32,
    end: u32,
    dialect: Dialect,
) -> Option<(ImportSyntax, u32)> {
    let line = tv.line(keyword);
    let same_line = |t: u32| dialect != Dialect::Kotlin || tv.line(t) == line;
    let mut import = ImportSyntax {
        alias: u32::MAX,
        first: keyword,
        last: keyword,
        ..Default::default()
    };
    let mut pos = keyword + 1;
    if dialect == Dialect::Java {
        if let Some(s) = tv.sig(pos, end) {
            if tv.word(s, "static") {
                import.is_static = true;
                pos = s + 1;
            }
        }
    }
    loop {
        let seg = tv.sig(pos, end)?;
        if !tv.is_ident(seg) || !same_line(seg) {
            return None;
        }
        import.segments.push(seg);
        import.last = seg;
        pos = seg + 1;
        let dot = match tv.sig(pos, end) {
            Some(d) if tv.punct(d, ".") && same_line(d) => d,
            _ => break,
        };
        let next = tv.next(dot, end)?;
        if tv.text(next) == b"*" {
            import.on_demand = true;
            import.last = next;
            pos = next + 1;
            break;
        }
        pos = dot + 1;
    }
    if dialect == Dialect::Kotlin && !import.on_demand {
        if let Some(as_kw) = tv.sig(pos, end) {
            if tv.word(as_kw, "as") && same_line(as_kw) {
                let alias = tv.next(as_kw, end)?;
                if !tv.is_ident(alias) {
                    return None;
                }
                import.alias = alias;
                import.last = alias;
                pos = alias + 1;
            }
        }
    }
    match tv.sig(pos, end) {
        Some(semi) if tv.punct(semi, ";") => {
            import.last = semi;
            Some((import, semi + 1))
        }
        _ if dialect == Dialect::Kotlin => Some((import, pos)),
        _ => None,
    }
}

// ── Constructs Phase 2 drops ────────────────────────────────────────────────

/// `( a, b ) ->` lambda parameters (tree-sitter `inferred_parameters`, which Phase 2 drops).
/// Returns `None` when the parenthesized list is not an inferred parameter list.
pub fn parse_inferred_lambda_params(tv: &TokenView, open: u32, limit: u32) -> Option<Vec<u32>> {
    if !tv.punct(open, "(") {
        return None;
    }
    let close = tv.matching_close(open, limit)?;
    let arrow = tv.next(close, limit)?;
    if !tv.punct(arrow, "->") {
        return None;
    }
    let mut names = Vec::new();
    let mut expect_ident = true;
    let mut pos = open + 1;
    while let Some(t) = tv.sig(pos, close) {
        if expect_ident {
            if !tv.is_ident(t) {
                return None;
            }
            names.push(t);
            expect_ident = false;
        } else {
            if !tv.punct(t, ",") {
                return None;
            }
            expect_ident = true;
        }
        pos = t + 1;
    }
    if expect_ident && !names.is_empty() {
        return None;
    }
    Some(names)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnumConstantSyntax {
    pub name_tok: u32,
    pub first: u32,
    pub last: u32,
}

/// Parses the enum constant list that opens an enum body (`{ A, B(1), C { ... } ; ...`).
/// Phase 2 drops `enum_constant`, so the constants are recovered here.
pub fn parse_enum_constants(tv: &TokenView, body_open: u32, limit: u32) -> Vec<EnumConstantSyntax> {
    let mut out = Vec::new();
    let body_close = match tv.matching_close(body_open, limit) {
        Some(c) => c,
        None => return out,
    };
    let mut pos = body_open + 1;
    loop {
        let mut first = u32::MAX;
        while let Some(t) = tv.sig(pos, body_close) {
            if !tv.punct(t, "@") {
                break;
            }
            match skip_annotation(tv, t, body_close) {
                Some((_, next)) => {
                    if first == u32::MAX {
                        first = t;
                    }
                    pos = next;
                }
                None => return out,
            }
        }
        let name = match tv.sig(pos, body_close) {
            Some(t) if tv.is_ident(t) => t,
            _ => return out,
        };
        if first == u32::MAX {
            first = name;
        }
        let mut last = name;
        pos = name + 1;
        if let Some(open) = tv.sig(pos, body_close) {
            if tv.punct(open, "(") {
                match tv.matching_close(open, body_close) {
                    Some(c) => {
                        last = c;
                        pos = c + 1;
                    }
                    None => return out,
                }
            }
        }
        if let Some(open) = tv.sig(pos, body_close) {
            if tv.punct(open, "{") {
                match tv.matching_close(open, body_close) {
                    Some(c) => {
                        last = c;
                        pos = c + 1;
                    }
                    None => return out,
                }
            }
        }
        out.push(EnumConstantSyntax {
            name_tok: name,
            first,
            last,
        });
        match tv.sig(pos, body_close) {
            Some(c) if tv.punct(c, ",") => pos = c + 1,
            _ => return out,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeclaratorSyntax {
    pub name_tok: u32,
    pub first: u32,
    pub last: u32,
    pub extra_dims: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldLikeSyntax {
    pub modifiers: ModifierScan,
    pub type_first: u32,
    pub type_last: u32,
    pub declarators: Vec<DeclaratorSyntax>,
    pub first: u32,
    pub last: u32,
}

/// Parses every `{modifiers} Type declarator {, declarator} ;` declaration in `[start, end)`,
/// skipping token runs that are not field declarations (such as the parameter list and
/// `throws` clause of a bodiless method, which Phase 2 reduces away).
pub fn parse_field_declarations(tv: &TokenView, start: u32, end: u32) -> Vec<FieldLikeSyntax> {
    let mut out = Vec::new();
    let mut pos = start;
    while let Some(t) = tv.sig(pos, end) {
        match parse_one_field_declaration(tv, t, end) {
            Some((decl, next)) => {
                out.push(decl);
                pos = next;
            }
            None => match skip_past_semicolon(tv, t, end) {
                Some(next) => pos = next,
                None => break,
            },
        }
    }
    out
}

fn parse_one_field_declaration(
    tv: &TokenView,
    start: u32,
    end: u32,
) -> Option<(FieldLikeSyntax, u32)> {
    let mods = scan_java_modifiers(tv, start, end);
    let mut parser = TypeParser::new(tv, end, Dialect::Java);
    let (ty, mut pos) = parser.parse(mods.end, TypeOptions::default())?;
    let mut declarators = Vec::new();
    loop {
        let name = tv.sig(pos, end)?;
        if !tv.is_ident(name) {
            return None;
        }
        let (extra_dims, after_dims, dims_last) = parse_trailing_dims(tv, name + 1, end);
        let mut last = if extra_dims > 0 { dims_last } else { name };
        pos = after_dims;
        let t = tv.sig(pos, end)?;
        if tv.punct(t, "=") {
            let (stop, init_last) = skip_expression(tv, t + 1, end)?;
            if init_last != u32::MAX {
                last = init_last;
            }
            pos = stop;
        }
        declarators.push(DeclaratorSyntax {
            name_tok: name,
            first: name,
            last,
            extra_dims,
        });
        let t = tv.sig(pos, end)?;
        if tv.punct(t, ",") {
            pos = t + 1;
            continue;
        }
        if tv.punct(t, ";") {
            let first = tv.sig(start, end).unwrap_or(start);
            return Some((
                FieldLikeSyntax {
                    modifiers: mods,
                    type_first: ty.first,
                    type_last: ty.last,
                    declarators,
                    first,
                    last: t,
                },
                t + 1,
            ));
        }
        return None;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatchParamSyntax {
    pub modifiers: ModifierScan,
    pub alternatives: Vec<TypeSyntax>,
    pub name_tok: u32,
    pub type_first: u32,
    pub type_last: u32,
}

/// Parses `catch ( {modifiers} Type {| Type} name )`; Phase 2 drops `catch_formal_parameter`.
pub fn parse_catch_parameter(tv: &TokenView, catch_tok: u32, limit: u32) -> Option<CatchParamSyntax> {
    if !tv.word(catch_tok, "catch") {
        return None;
    }
    let open = tv.next(catch_tok, limit)?;
    if !tv.punct(open, "(") {
        return None;
    }
    let close = tv.matching_close(open, limit)?;
    let mods = scan_java_modifiers(tv, open + 1, close);
    let mut parser = TypeParser::new(tv, close, Dialect::Java);
    let mut alternatives = Vec::new();
    let mut pos = mods.end;
    loop {
        let (ty, next) = parser.parse(pos, TypeOptions::default())?;
        alternatives.push(ty);
        pos = next;
        match tv.sig(pos, close) {
            Some(bar) if tv.punct(bar, "|") => pos = bar + 1,
            _ => break,
        }
    }
    let name = tv.sig(pos, close)?;
    if !tv.is_ident(name) {
        return None;
    }
    let type_first = alternatives.first()?.first;
    let type_last = alternatives.last()?.last;
    Some(CatchParamSyntax {
        modifiers: mods,
        alternatives,
        name_tok: name,
        type_first,
        type_last,
    })
}

/// Counts the arguments of the parenthesized list opened at `open` (top-level commas).
/// Returns the count and the closing token.
pub fn count_call_arguments(tv: &TokenView, open: u32, limit: u32) -> Option<(u16, u32)> {
    if !tv.punct(open, "(") {
        return None;
    }
    let close = tv.matching_close(open, limit)?;
    let mut commas = 0u16;
    let mut saw_token = false;
    let mut pos = open + 1;
    while let Some(t) = tv.sig(pos, close) {
        saw_token = true;
        if tv.is_opener(t) {
            pos = tv.matching_close(t, close)? + 1;
            continue;
        }
        if tv.punct(t, ",") {
            commas = commas.saturating_add(1);
        }
        pos = t + 1;
    }
    Some((if saw_token { commas.saturating_add(1) } else { 0 }, close))
}
