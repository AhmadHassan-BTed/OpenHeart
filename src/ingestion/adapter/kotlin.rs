//! Kotlin lexical frontend.
//!
//! No Kotlin tree-sitter grammar is linked into this build, so Kotlin sources are not
//! parsed with another language's grammar. Instead this module implements the Kotlin
//! lexical grammar (Kotlin specification, "Lexical grammar"; `KotlinLexer.g4`):
//! nested block comments and KDoc, shebang lines, backtick identifiers, all numeric
//! literal forms (hex, binary, underscores, `L` / `u` suffixes, floating literals), char
//! literals with escapes, single-line and raw (`"""`) strings including `${...}` template
//! expressions with arbitrary nesting, and the full operator set (`?.`, `?:`, `!!`, `::`,
//! `..`, `..<`, `===`, `!in`, `!is`, `as?`, ...).
//!
//! Hard keywords are typed `Keyword`. Soft and modifier keywords (`data`, `value`, `get`,
//! `field`, `override`, ...) are valid identifiers, so they are typed `Keyword` only when
//! the next significant token makes a modifier or declaration reading the only possible
//! one (another identifier, a backtick identifier, or an annotation); otherwise they are
//! typed `SoftKeyword` for consumers to resolve.
//!
//! Files lexed here are flagged `FILE_FLAG_LEXICAL_ONLY`: they contribute an accurate token
//! stream, and Phase 2 reports that no syntax tree exists for them.

use crate::core::types::source::{FileDiagnostics, TokenFilter};
use crate::core::types::token::{LangId, TokenType};
use crate::ingestion::adapter::{filter_allows, SourceLexer};
use crate::ingestion::walker::RawToken;

pub struct KotlinLexer;

impl KotlinLexer {
    pub fn new() -> Self {
        Self
    }
}

impl Default for KotlinLexer {
    fn default() -> Self {
        Self::new()
    }
}

/// Hard keywords (never identifiers).
const HARD_KEYWORDS: &[&str] = &[
    "as", "break", "class", "continue", "do", "else", "for", "fun", "if", "in", "interface",
    "is", "object", "package", "return", "super", "this", "throw", "try", "typealias", "typeof",
    "val", "var", "when", "while",
];

/// Soft keywords that only ever appear in keyword position in practice.
const STRUCTURAL_SOFT_KEYWORDS: &[&str] = &[
    "by", "catch", "constructor", "finally", "import", "init", "where",
];

/// Modifier and soft keywords that are also common identifiers.
const CONTEXTUAL_KEYWORDS: &[&str] = &[
    "abstract", "actual", "annotation", "companion", "const", "crossinline", "data", "delegate",
    "dynamic", "enum", "expect", "external", "field", "file", "final", "get", "infix", "inline",
    "inner", "internal", "lateinit", "noinline", "open", "operator", "out", "override", "param",
    "private", "property", "protected", "public", "receiver", "reified", "sealed", "set",
    "setparam", "suspend", "tailrec", "value", "vararg",
];

const OPERATORS_3: &[&str] = &["===", "!==", "..<"];
const OPERATORS_2: &[&str] = &[
    "==", "!=", "<=", ">=", "&&", "||", "++", "--", "+=", "-=", "*=", "/=", "%=", "->", "?.",
    "?:", "!!", "::", "..",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrevToken {
    None,
    /// Identifier or keyword ending at the recorded offset.
    Word { end: usize },
    Other,
}

struct Lexer<'a, 'd> {
    src: &'a [u8],
    pos: usize,
    line: u32,
    line_start: usize,
    file_id: u16,
    filter: TokenFilter,
    out: Vec<RawToken>,
    diags: &'d mut FileDiagnostics,
    prev: PrevToken,
    /// Set after an annotation `@`: the next identifier is the annotation name.
    expect_annotation_name: bool,
}

impl SourceLexer for KotlinLexer {
    fn language_id(&self) -> LangId {
        LangId::Kotlin
    }

    fn file_extensions(&self) -> &[&str] {
        &["kt", "kts"]
    }

    fn lex(
        &self,
        source: &[u8],
        file_id: u16,
        filter: &TokenFilter,
        diagnostics: &mut FileDiagnostics,
    ) -> Vec<RawToken> {
        let mut lexer = Lexer {
            src: source,
            pos: 0,
            line: 1,
            line_start: 0,
            file_id,
            filter: *filter,
            out: Vec::with_capacity(source.len() / 4),
            diags: diagnostics,
            prev: PrevToken::None,
            expect_annotation_name: false,
        };
        lexer.run();
        lexer.out
    }
}

#[inline]
fn is_ident_start_ascii(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

#[inline]
fn is_ident_part_ascii(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Decodes the UTF-8 scalar at `pos`, returning it and its encoded width.
fn decode_char(src: &[u8], pos: usize) -> Option<(char, usize)> {
    let lead = *src.get(pos)?;
    let width = match lead {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF7 => 4,
        _ => return None,
    };
    let end = pos.checked_add(width)?;
    let bytes = src.get(pos..end)?;
    let s = std::str::from_utf8(bytes).ok()?;
    s.chars().next().map(|c| (c, width))
}

impl<'a, 'd> Lexer<'a, 'd> {
    #[inline]
    fn peek(&self, offset: usize) -> Option<u8> {
        self.src.get(self.pos + offset).copied()
    }

    #[inline]
    fn starts_with_at(&self, pos: usize, pat: &[u8]) -> bool {
        self.src.get(pos..).is_some_and(|rest| rest.starts_with(pat))
    }

    /// Moves to `new_pos`, updating line bookkeeping for every newline crossed.
    fn advance_to(&mut self, new_pos: usize) {
        let end = new_pos.min(self.src.len());
        for i in self.pos..end {
            if self.src[i] == b'\n' {
                self.line = self.line.saturating_add(1);
                self.line_start = i + 1;
            }
        }
        self.pos = end;
    }

    fn emit(&mut self, start: usize, end: usize, token_type: TokenType, line: u32, col: u32) {
        if end <= start || !filter_allows(token_type, &self.filter) {
            return;
        }
        self.out.push(RawToken {
            file_id: self.file_id,
            line,
            col,
            token_type,
            text_start: start,
            text_len: end - start,
        });
    }

    fn error(&mut self, start: usize, end: usize) {
        self.diags.error_count = self.diags.error_count.saturating_add(1);
        self.diags.record_error_range(start, end);
    }

    fn run(&mut self) {
        // UTF-8 byte-order mark.
        if self.starts_with_at(0, b"\xEF\xBB\xBF") {
            let (line, col) = (self.line, 0);
            self.advance_to(3);
            self.emit(0, 3, TokenType::Whitespace, line, col);
        }
        // Shebang line (only at the very start, after an optional BOM).
        if self.starts_with_at(self.pos, b"#!") {
            let start = self.pos;
            let end = self.scan_to_line_end(start);
            self.lex_span(start, end, TokenType::CommentLine);
        }

        while self.pos < self.src.len() {
            let start = self.pos;
            let b = self.src[start];
            match b {
                b'\n' => {
                    let (line, col) = self.position();
                    self.advance_to(start + 1);
                    self.emit(start, start + 1, TokenType::Newline, line, col);
                }
                b' ' | b'\t' | b'\r' | 0x0C => {
                    let mut end = start;
                    while end < self.src.len() && matches!(self.src[end], b' ' | b'\t' | b'\r' | 0x0C)
                    {
                        end += 1;
                    }
                    self.lex_span(start, end, TokenType::Whitespace);
                }
                b'/' if self.peek(1) == Some(b'/') => {
                    let end = self.scan_to_line_end(start);
                    self.lex_span(start, end, TokenType::CommentLine);
                    self.prev = PrevToken::Other;
                }
                b'/' if self.peek(1) == Some(b'*') => {
                    let (end, terminated) = self.scan_block_comment(start);
                    let is_doc =
                        self.starts_with_at(start, b"/**") && !self.starts_with_at(start, b"/**/");
                    if !terminated {
                        self.error(start, end);
                    }
                    let ty = if is_doc {
                        TokenType::CommentDoc
                    } else {
                        TokenType::CommentBlock
                    };
                    self.lex_span(start, end, ty);
                }
                b'"' => {
                    let (end, terminated) = self.scan_string(start);
                    if !terminated {
                        self.error(start, end);
                    }
                    self.lex_span(start, end, TokenType::StringLiteral);
                    self.prev = PrevToken::Other;
                }
                b'\'' => {
                    let (end, valid) = self.scan_char_literal(start);
                    if !valid {
                        self.error(start, end);
                    }
                    self.lex_span(start, end, TokenType::CharLiteral);
                    self.prev = PrevToken::Other;
                }
                b'`' => self.lex_backtick_identifier(start),
                b'0'..=b'9' => self.lex_number(start),
                b'.' if self.peek(1).is_some_and(|c| c.is_ascii_digit()) => self.lex_number(start),
                b'@' => self.lex_at(start),
                _ if is_ident_start_ascii(b) => self.lex_word(start),
                _ if b >= 0x80 => match decode_char(self.src, start) {
                    Some((c, _)) if c.is_alphabetic() => self.lex_word(start),
                    Some((_, width)) => {
                        let end = start + width;
                        self.error(start, end);
                        self.lex_span(start, end, TokenType::Unknown);
                        self.prev = PrevToken::Other;
                    }
                    None => {
                        self.error(start, start + 1);
                        self.lex_span(start, start + 1, TokenType::Unknown);
                        self.prev = PrevToken::Other;
                    }
                },
                _ => self.lex_operator(start),
            }
        }
    }

    #[inline]
    fn position(&self) -> (u32, u32) {
        let col = u32::try_from(self.pos - self.line_start).unwrap_or(u32::MAX);
        (self.line, col)
    }

    /// Emits `[start, end)` as one token positioned at `start` and advances past it.
    fn lex_span(&mut self, start: usize, end: usize, token_type: TokenType) {
        debug_assert_eq!(start, self.pos);
        let (line, col) = self.position();
        self.advance_to(end);
        self.emit(start, end, token_type, line, col);
    }

    fn scan_to_line_end(&self, start: usize) -> usize {
        let mut end = start;
        while end < self.src.len() && self.src[end] != b'\n' {
            end += 1;
        }
        // Keep a CRLF's `\r` out of the comment text.
        if end > start && self.src[end - 1] == b'\r' {
            end -= 1;
        }
        end
    }

    /// Kotlin block comments nest. Returns (end, terminated).
    fn scan_block_comment(&self, start: usize) -> (usize, bool) {
        let mut i = start + 2;
        let mut depth = 1usize;
        while i < self.src.len() {
            if self.starts_with_at(i, b"/*") {
                depth += 1;
                i += 2;
            } else if self.starts_with_at(i, b"*/") {
                depth -= 1;
                i += 2;
                if depth == 0 {
                    return (i, true);
                }
            } else {
                i += 1;
            }
        }
        (self.src.len(), false)
    }

    /// Scans a single-line (`"..."`) or raw (`"""..."""`) string literal starting at
    /// `start`, including nested `${...}` template expressions. Returns (end, terminated).
    fn scan_string(&self, start: usize) -> (usize, bool) {
        if self.starts_with_at(start, b"\"\"\"") {
            let mut i = start + 3;
            while i < self.src.len() {
                if self.starts_with_at(i, b"\"\"\"") {
                    // The literal closes at the end of a run of three or more quotes.
                    let mut j = i;
                    while j < self.src.len() && self.src[j] == b'"' {
                        j += 1;
                    }
                    return (j, true);
                }
                if self.src[i] == b'$' && self.src.get(i + 1) == Some(&b'{') {
                    let (end, ok) = self.scan_template_expression(i);
                    if !ok {
                        return (end, false);
                    }
                    i = end;
                } else {
                    i += 1;
                }
            }
            return (self.src.len(), false);
        }

        let mut i = start + 1;
        while i < self.src.len() {
            match self.src[i] {
                b'\\' => i += 2,
                b'"' => return (i + 1, true),
                b'\n' => return (i, false),
                b'$' if self.src.get(i + 1) == Some(&b'{') => {
                    let (end, ok) = self.scan_template_expression(i);
                    if !ok {
                        return (end, false);
                    }
                    i = end;
                }
                _ => i += 1,
            }
        }
        (self.src.len().min(i), false)
    }

    /// Scans `${ ... }` starting at the `$`, skipping nested braces, strings, char literals
    /// and comments. Returns (end, terminated).
    fn scan_template_expression(&self, dollar: usize) -> (usize, bool) {
        let mut i = dollar + 2;
        let mut depth = 1usize;
        while i < self.src.len() {
            match self.src[i] {
                b'{' => {
                    depth += 1;
                    i += 1;
                }
                b'}' => {
                    depth -= 1;
                    i += 1;
                    if depth == 0 {
                        return (i, true);
                    }
                }
                b'"' => {
                    let (end, ok) = self.scan_string(i);
                    if !ok {
                        return (end, false);
                    }
                    i = end;
                }
                b'\'' => {
                    let (end, _) = self.scan_char_literal(i);
                    i = end.max(i + 1);
                }
                b'/' if self.src.get(i + 1) == Some(&b'/') => {
                    i = self.scan_to_line_end(i).max(i + 2);
                }
                b'/' if self.src.get(i + 1) == Some(&b'*') => {
                    let (end, ok) = self.scan_block_comment(i);
                    if !ok {
                        return (end, false);
                    }
                    i = end;
                }
                _ => i += 1,
            }
        }
        (self.src.len(), false)
    }

    /// Scans a character literal. Returns (end, valid).
    fn scan_char_literal(&self, start: usize) -> (usize, bool) {
        let mut i = start + 1;
        let body_ok = match self.src.get(i) {
            Some(b'\\') => {
                if self.src.get(i + 1) == Some(&b'u') {
                    let hex_end = i + 6;
                    let ok = self
                        .src
                        .get(i + 2..hex_end)
                        .is_some_and(|h| h.iter().all(|c| c.is_ascii_hexdigit()));
                    i = if ok { hex_end } else { i + 2 };
                    ok
                } else {
                    let ok = matches!(
                        self.src.get(i + 1),
                        Some(b't' | b'b' | b'r' | b'n' | b'\'' | b'"' | b'\\' | b'$')
                    );
                    i += 2;
                    ok
                }
            }
            Some(b'\'') | Some(b'\n') | None => false,
            Some(_) => match decode_char(self.src, i) {
                Some((_, width)) => {
                    i += width;
                    true
                }
                None => {
                    i += 1;
                    false
                }
            },
        };
        if body_ok && self.src.get(i) == Some(&b'\'') {
            return (i + 1, true);
        }
        // Malformed: resynchronise at the next quote on the same line, or at the line end.
        let mut j = i.min(self.src.len());
        while j < self.src.len() && self.src[j] != b'\'' && self.src[j] != b'\n' {
            j += 1;
        }
        if j < self.src.len() && self.src[j] == b'\'' {
            j += 1;
        }
        (j.max(start + 1), false)
    }

    fn lex_backtick_identifier(&mut self, start: usize) {
        let mut end = start + 1;
        while end < self.src.len() && self.src[end] != b'`' && self.src[end] != b'\n' {
            end += 1;
        }
        let terminated = end < self.src.len() && self.src[end] == b'`';
        if terminated {
            end += 1;
        } else {
            self.error(start, end);
        }
        let ty = if std::mem::take(&mut self.expect_annotation_name) {
            TokenType::Annotation
        } else {
            TokenType::Identifier
        };
        self.lex_span(start, end, ty);
        self.prev = PrevToken::Word { end };
    }

    fn scan_word_end(&self, start: usize) -> usize {
        let mut end = start;
        while end < self.src.len() {
            let b = self.src[end];
            if is_ident_part_ascii(b) {
                end += 1;
            } else if b >= 0x80 {
                match decode_char(self.src, end) {
                    Some((c, width)) if c.is_alphanumeric() => end += width,
                    _ => break,
                }
            } else {
                break;
            }
        }
        end
    }

    /// True when the significant token after `from` starts an identifier, a backtick
    /// identifier or an annotation, i.e. a modifier / declaration-keyword position.
    fn followed_by_word(&self, from: usize) -> bool {
        let Some(i) = self.next_significant_pos(from) else {
            return false;
        };
        match self.src[i] {
            b'`' | b'@' => true,
            b if is_ident_start_ascii(b) => true,
            b if b >= 0x80 => decode_char(self.src, i).is_some_and(|(c, _)| c.is_alphabetic()),
            _ => false,
        }
    }

    /// Offset of the next byte that is not whitespace or part of a comment.
    fn next_significant_pos(&self, from: usize) -> Option<usize> {
        let mut i = from;
        while i < self.src.len() {
            match self.src[i] {
                b' ' | b'\t' | b'\r' | b'\n' | 0x0C => i += 1,
                b'/' if self.src.get(i + 1) == Some(&b'/') => {
                    while i < self.src.len() && self.src[i] != b'\n' {
                        i += 1;
                    }
                }
                b'/' if self.src.get(i + 1) == Some(&b'*') => {
                    i = self.scan_block_comment(i).0;
                }
                _ => return Some(i),
            }
        }
        None
    }

    fn lex_word(&mut self, start: usize) {
        let src = self.src;
        let end = self.scan_word_end(start);
        let word = std::str::from_utf8(&src[start..end]).unwrap_or("");

        if std::mem::take(&mut self.expect_annotation_name) {
            self.lex_span(start, end, TokenType::Annotation);
            self.prev = PrevToken::Word { end };
            return;
        }

        // `as?` is a single safe-cast token.
        if word == "as" && self.src.get(end) == Some(&b'?') {
            self.lex_span(start, end + 1, TokenType::Keyword);
            self.prev = PrevToken::Other;
            return;
        }

        let ty = match word {
            "true" | "false" => TokenType::BooleanLiteral,
            "null" => TokenType::NullLiteral,
            w if HARD_KEYWORDS.contains(&w) || STRUCTURAL_SOFT_KEYWORDS.contains(&w) => {
                TokenType::Keyword
            }
            w if CONTEXTUAL_KEYWORDS.contains(&w) => {
                if self.followed_by_word(end) {
                    TokenType::Keyword
                } else {
                    TokenType::SoftKeyword
                }
            }
            _ => TokenType::Identifier,
        };
        self.lex_span(start, end, ty);
        self.prev = PrevToken::Word { end };
    }

    fn lex_number(&mut self, start: usize) {
        let src = self.src;
        let mut i = start;
        let mut is_float = false;
        let lower = |b: u8| b.to_ascii_lowercase();

        if src[i] == b'0' && src.get(i + 1).is_some_and(|&c| lower(c) == b'x') {
            i += 2;
            while i < src.len() && (src[i].is_ascii_hexdigit() || src[i] == b'_') {
                i += 1;
            }
        } else if src[i] == b'0' && src.get(i + 1).is_some_and(|&c| lower(c) == b'b') {
            i += 2;
            while i < src.len() && matches!(src[i], b'0' | b'1' | b'_') {
                i += 1;
            }
        } else {
            while i < src.len() && (src[i].is_ascii_digit() || src[i] == b'_') {
                i += 1;
            }
            if i < src.len() && src[i] == b'.' && src.get(i + 1).is_some_and(|c| c.is_ascii_digit())
            {
                is_float = true;
                i += 1;
                while i < src.len() && (src[i].is_ascii_digit() || src[i] == b'_') {
                    i += 1;
                }
            }
            if i < src.len() && lower(src[i]) == b'e' {
                let mut j = i + 1;
                if j < src.len() && matches!(src[j], b'+' | b'-') {
                    j += 1;
                }
                if j < src.len() && src[j].is_ascii_digit() {
                    is_float = true;
                    i = j;
                    while i < src.len() && (src[i].is_ascii_digit() || src[i] == b'_') {
                        i += 1;
                    }
                }
            }
            if i < src.len() && lower(src[i]) == b'f' {
                is_float = true;
                i += 1;
            }
        }

        if !is_float {
            if i < src.len() && lower(src[i]) == b'u' {
                i += 1;
            }
            if i < src.len() && src[i] == b'L' {
                i += 1;
            }
        }

        // A literal glued to identifier characters (`12abc`) is malformed.
        let glued_end = self.scan_word_end(i);
        if glued_end > i {
            self.error(start, glued_end);
            i = glued_end;
        }

        let ty = if is_float {
            TokenType::FloatLiteral
        } else {
            TokenType::IntegerLiteral
        };
        self.lex_span(start, i, ty);
        self.prev = PrevToken::Other;
    }

    fn lex_at(&mut self, start: usize) {
        // `label@` (no space before `@`) declares a label; `return@label` references one;
        // anything else introduces an annotation.
        let label_context = match self.prev {
            PrevToken::Word { end, .. } if end == start => true,
            _ => false,
        };
        self.lex_span(start, start + 1, TokenType::Punctuation);
        let next_is_word = self
            .src
            .get(start + 1)
            .is_some_and(|&b| is_ident_start_ascii(b) || b == b'`' || b >= 0x80);
        self.expect_annotation_name = !label_context && next_is_word;
        self.prev = PrevToken::Other;
    }

    fn lex_operator(&mut self, start: usize) {
        // `!in` / `!is` are single tokens when not followed by an identifier character.
        for kw in [&b"!in"[..], &b"!is"[..]] {
            if self.starts_with_at(start, kw) {
                let after = start + kw.len();
                let glued = self
                    .src
                    .get(after)
                    .is_some_and(|&b| is_ident_part_ascii(b) || b >= 0x80);
                if !glued {
                    self.lex_span(start, after, TokenType::Operator);
                    self.prev = PrevToken::Other;
                    return;
                }
            }
        }

        for op in OPERATORS_3.iter().chain(OPERATORS_2.iter()) {
            if self.starts_with_at(start, op.as_bytes()) {
                self.lex_span(start, start + op.len(), TokenType::Operator);
                self.prev = PrevToken::Other;
                return;
            }
        }

        let b = self.src[start];
        let ty = match b {
            b'(' | b')' | b'[' | b']' | b'{' | b'}' | b',' | b';' | b'.' | b':' => {
                TokenType::Punctuation
            }
            b'+' | b'-' | b'*' | b'/' | b'%' | b'=' | b'<' | b'>' | b'!' | b'?' | b'&' | b'|' => {
                TokenType::Operator
            }
            _ => {
                self.error(start, start + 1);
                TokenType::Unknown
            }
        };
        self.lex_span(start, start + 1, ty);
        self.prev = PrevToken::Other;
    }
}
