//! SourceScanner — bounded, bracket-aware token scans over declaration headers and
//! member bodies (§9.2).
//!
//! Phases 2–3 drop structure that the UML views need (generic type arguments,
//! supertype clauses, annotation lists, constructor bodies). The scanner recovers
//! those facts from the token corpus with explicit bracket matching, so type
//! arguments, bounds and annotation arguments never leak into the results.
//! Every scan is bounded and confined to one file; a scan that cannot locate its
//! anchor returns `None` instead of guessing.

use std::collections::HashSet;

use crate::core::types::symbol::SymbolRecord;
use crate::core::types::token::{unpack_sort_key, LangId, TokenType};
use crate::ingestion::TokenCorpusArtifact;

/// Maximum tokens inspected in a declaration header (annotations, modifiers, clauses).
const MAX_HEADER_TOKENS: u32 = 1024;
/// Maximum tokens walked backwards to find the start of a declaration.
const MAX_BACKSCAN_TOKENS: u32 = 96;
/// Maximum source lines a backward scan may cross (annotations above a declaration).
const MAX_BACKSCAN_LINES: u32 = 6;
/// Maximum tokens of one member body.
const MAX_BODY_TOKENS: u32 = 400_000;
/// Maximum tokens of one argument list.
const MAX_ARGUMENT_TOKENS: u32 = 20_000;

/// Supertypes written in a type declaration header, as interned name text ids.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeclaredSupertypes {
    /// Names after `extends` at bracket depth 0 (Java, TypeScript).
    pub extends: Vec<u32>,
    /// Names after `implements` at bracket depth 0 (Java, TypeScript).
    pub implements: Vec<u32>,
    /// Kotlin `:` delegation specifiers: (name, invoked with a constructor call).
    pub delegation: Vec<(u32, bool)>,
}

impl DeclaredSupertypes {
    pub fn is_empty(&self) -> bool {
        self.extends.is_empty() && self.implements.is_empty() && self.delegation.is_empty()
    }
}

/// Shape of a declared field or parameter type.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TypeShape {
    /// Array dimensions or varargs were declared.
    pub is_array: bool,
    /// Kotlin nullable marker `?` on the declared type.
    pub is_nullable: bool,
    /// Identifiers inside the outermost type-argument list, in source order.
    pub type_argument_names: Vec<u32>,
}

/// The value written by an assignment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssignedValue {
    /// `x = new T(...)`.
    New { type_text: u32, new_token: u32 },
    /// `x = y;` or `x = A.B;` — the last identifier of a pure name/qualified name.
    Name { text: u32 },
    /// Any other expression.
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assignment {
    pub lhs_text: u32,
    pub lhs_token: u32,
    /// `this.x = ...`.
    pub via_this: bool,
    /// `other.x = ...` with a receiver other than `this`.
    pub via_other_receiver: bool,
    /// `T x = ...` (a local declaration with initializer).
    pub is_declaration: bool,
    pub value: AssignedValue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReturnedValue {
    This,
    Name(u32),
    New(u32),
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Receiver {
    /// Unqualified call `m(...)` (implicit `this` or a static import).
    Implicit,
    This,
    Super,
    /// `x.m(...)` or `this.x.m(...)` — the text id of `x`.
    Named(u32),
    /// The receiver is a call result, an index or a parenthesized expression.
    Expression,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invocation {
    pub name_token: u32,
    pub method_text: u32,
    pub receiver: Receiver,
    /// Closing parenthesis of the argument list (evaluation-order key).
    pub close_token: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Allocation {
    pub new_token: u32,
    pub type_text: u32,
    /// `new T() { ... }` — an anonymous subclass.
    pub anonymous: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IfRegion {
    pub if_token: u32,
    /// Tokens strictly inside the condition parentheses.
    pub cond: (u32, u32),
    pub then_branch: (u32, u32),
    pub else_branch: Option<(u32, u32)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaseRegion {
    pub case_token: u32,
    /// Tokens strictly inside the switch selector parentheses.
    pub selector: (u32, u32),
    /// Last identifier (or literal) of each case label.
    pub labels: Vec<u32>,
    pub body: (u32, u32),
    pub is_default: bool,
}

/// Lexical facts of one member body (`{ ... }` token range).
#[derive(Debug, Clone, Default)]
pub struct BodyFacts {
    pub range: (u32, u32),
    pub assignments: Vec<Assignment>,
    pub returns: Vec<ReturnedValue>,
    pub invocations: Vec<Invocation>,
    pub allocations: Vec<Allocation>,
    /// Loop statement regions, plus argument regions of `forEach`-style iteration calls.
    pub loops: Vec<(u32, u32)>,
    pub ifs: Vec<IfRegion>,
    pub cases: Vec<CaseRegion>,
}

impl BodyFacts {
    pub fn contains(&self, tok: u32) -> bool {
        self.range.0 <= tok && tok <= self.range.1
    }

    pub fn loop_depth(&self, tok: u32) -> u16 {
        let depth = self
            .loops
            .iter()
            .filter(|&&(a, b)| a <= tok && tok <= b)
            .count();
        u16::try_from(depth).unwrap_or(u16::MAX)
    }

    pub fn in_loop(&self, tok: u32) -> bool {
        self.loops.iter().any(|&(a, b)| a <= tok && tok <= b)
    }

    pub fn returns_this(&self) -> bool {
        self.returns.iter().any(|r| *r == ReturnedValue::This)
    }

    pub fn returns_name(&self, text: u32) -> bool {
        self.returns.iter().any(|r| *r == ReturnedValue::Name(text))
    }

    pub fn assigns(&self, text: u32) -> bool {
        self.assignments
            .iter()
            .any(|a| a.lhs_text == text && !a.is_declaration && !a.via_other_receiver)
    }

    /// Invocations whose receiver is the named variable or field.
    pub fn calls_on(&self, receiver_text: u32) -> impl Iterator<Item = &Invocation> {
        self.invocations
            .iter()
            .filter(move |inv| inv.receiver == Receiver::Named(receiver_text))
    }
}

/// Keywords that can precede `(` but never name an invoked method.
const NON_CALL_WORDS: &[&[u8]] = &[
    b"if",
    b"for",
    b"while",
    b"switch",
    b"catch",
    b"synchronized",
    b"return",
    b"new",
    b"this",
    b"super",
    b"when",
    b"try",
    b"throw",
    b"assert",
    b"do",
    b"else",
    b"yield",
];

/// Library iteration entry points whose lambda argument runs once per element.
const ITERATION_CALLS: &[&[u8]] = &[b"forEach", b"forEachOrdered", b"forEachRemaining"];

pub struct SourceScanner<'a> {
    tca: &'a TokenCorpusArtifact,
}

impl<'a> SourceScanner<'a> {
    pub fn new(tca: &'a TokenCorpusArtifact) -> Self {
        Self { tca }
    }

    pub fn token_count(&self) -> u32 {
        u32::try_from(self.tca.token_records.len()).unwrap_or(u32::MAX)
    }

    pub fn file_of(&self, tok: u32) -> Option<u16> {
        self.tca
            .token_records
            .get(tok as usize)
            .map(|r| unpack_sort_key(r.sort_key).0)
    }

    /// (file, line, column) of a token.
    pub fn position(&self, tok: u32) -> Option<(u16, u32, u16)> {
        self.tca
            .token_records
            .get(tok as usize)
            .map(|r| unpack_sort_key(r.sort_key))
    }

    pub fn line_of(&self, tok: u32) -> u32 {
        self.position(tok).map(|p| p.1).unwrap_or(0)
    }

    pub fn text(&self, tok: u32) -> &'a [u8] {
        match self.tca.token_records.get(tok as usize) {
            Some(r) => self.tca.interner.lookup_text(r.text_id),
            None => b"",
        }
    }

    pub fn text_id(&self, tok: u32) -> u32 {
        self.tca
            .token_records
            .get(tok as usize)
            .map(|r| r.text_id)
            .unwrap_or(u32::MAX)
    }

    pub fn token_len(&self, tok: u32) -> u16 {
        self.tca
            .token_records
            .get(tok as usize)
            .map(|r| r.len)
            .unwrap_or(0)
    }

    fn token_type(&self, tok: u32) -> TokenType {
        self.tca
            .token_records
            .get(tok as usize)
            .map(|r| TokenType::from_u8(r.token_type))
            .unwrap_or(TokenType::Unknown)
    }

    pub fn is_trivia(&self, tok: u32) -> bool {
        matches!(
            self.token_type(tok),
            TokenType::CommentLine
                | TokenType::CommentBlock
                | TokenType::CommentDoc
                | TokenType::Whitespace
                | TokenType::Newline
        )
    }

    /// Identifier-like token that is not a reserved keyword.
    pub fn is_ident(&self, tok: u32) -> bool {
        match self.token_type(tok) {
            TokenType::Keyword
            | TokenType::Operator
            | TokenType::Punctuation
            | TokenType::StringLiteral
            | TokenType::CharLiteral
            | TokenType::IntegerLiteral
            | TokenType::FloatLiteral
            | TokenType::BooleanLiteral
            | TokenType::NullLiteral
            | TokenType::JavaAnnotationMarker => return false,
            _ => {}
        }
        is_identifier_text(self.text(tok))
    }

    pub fn is(&self, tok: u32, s: &[u8]) -> bool {
        self.text(tok) == s
    }

    /// Next non-trivia token in the same file.
    pub fn next_code(&self, tok: u32, file: u16) -> Option<u32> {
        let n = self.token_count();
        let mut t = tok.checked_add(1)?;
        while t < n {
            if self.file_of(t) != Some(file) {
                return None;
            }
            if !self.is_trivia(t) {
                return Some(t);
            }
            t += 1;
        }
        None
    }

    /// Previous non-trivia token in the same file.
    pub fn prev_code(&self, tok: u32, file: u16) -> Option<u32> {
        let mut t = tok;
        while t > 0 {
            t -= 1;
            if self.file_of(t) != Some(file) {
                return None;
            }
            if !self.is_trivia(t) {
                return Some(t);
            }
        }
        None
    }

    /// Code tokens of `[start, end]` (inclusive) in one file.
    fn code_tokens(&self, start: u32, end: u32, file: u16) -> Vec<u32> {
        let mut out = Vec::new();
        let n = self.token_count();
        let mut t = start;
        while t <= end && t < n {
            if self.file_of(t) != Some(file) {
                break;
            }
            if !self.is_trivia(t) {
                out.push(t);
            }
            t += 1;
        }
        out
    }

    /// First token of a declaration, walking back over annotations and modifiers
    /// to the previous statement boundary (`;`, `{`, `}`).
    pub fn declaration_start(&self, sym: &SymbolRecord) -> Option<(u32, u16)> {
        let first = sym.first_token_id;
        let file = self.file_of(first)?;
        let first_line = self.line_of(first);
        let mut start = first;
        let mut steps = 0u32;
        let mut paren_depth = 0i32;
        let mut angle_depth = 0i32;
        while let Some(p) = self.prev_code(start, file) {
            steps += 1;
            if steps > MAX_BACKSCAN_TOKENS {
                break;
            }
            let t = self.text(p);
            if paren_depth == 0 && (t == b";" || t == b"{" || t == b"}" || t == b"=") {
                break;
            }
            if paren_depth == 0 && angle_depth == 0 && t == b"," {
                // Previous parameter or declarator.
                break;
            }
            if first_line.saturating_sub(self.line_of(p)) > MAX_BACKSCAN_LINES {
                break;
            }
            match t {
                b")" => paren_depth += 1,
                b"(" => {
                    if paren_depth == 0 {
                        // The declaration sits inside a parameter list.
                        break;
                    }
                    paren_depth -= 1;
                }
                b">" => angle_depth += 1,
                b">>" => angle_depth += 2,
                b"<" => angle_depth = (angle_depth - 1).max(0),
                _ => {}
            }
            start = p;
        }
        Some((start, file))
    }

    /// The token that carries the declared name of `sym` (skipping annotation names).
    pub fn name_token(&self, sym: &SymbolRecord) -> Option<u32> {
        if sym.name_id == u32::MAX {
            return None;
        }
        let (start, file) = self.declaration_start(sym)?;
        let mut paren = 0i32;
        let mut t = start;
        let mut steps = 0u32;
        loop {
            if paren == 0 && self.text_id(t) == sym.name_id && self.is_ident(t) {
                let prev = self.prev_code(t, file);
                let after_marker = prev
                    .map(|p| self.is(p, b"@") || self.is(p, b"."))
                    .unwrap_or(false);
                if !after_marker {
                    return Some(t);
                }
            }
            let txt = self.text(t);
            if txt == b"(" {
                paren += 1;
            } else if txt == b")" {
                paren = (paren - 1).max(0);
            } else if paren == 0 && (txt == b"{" || txt == b";") && t > sym.first_token_id {
                return None;
            }
            steps += 1;
            if steps > MAX_HEADER_TOKENS {
                return None;
            }
            t = self.next_code(t, file)?;
        }
    }

    /// The code token immediately preceding the declared name (e.g. `object`, `class`).
    pub fn keyword_before_name(&self, sym: &SymbolRecord) -> Option<&'a [u8]> {
        let name = self.name_token(sym)?;
        let file = self.file_of(name)?;
        let prev = self.prev_code(name, file)?;
        Some(self.text(prev))
    }

    /// Simple names of the annotations written on a declaration (not on its parameters).
    pub fn annotations(&self, sym: &SymbolRecord) -> Vec<u32> {
        let mut out = Vec::new();
        let (start, file) = match self.declaration_start(sym) {
            Some(s) => s,
            None => return out,
        };
        let mut paren = 0i32;
        let mut t = start;
        let mut steps = 0u32;
        loop {
            let txt = self.text(t);
            if txt == b"(" {
                paren += 1;
            } else if txt == b")" {
                paren = (paren - 1).max(0);
            } else if paren == 0 && (txt == b"{" || txt == b";" || txt == b"=") {
                break;
            } else if paren == 0 && txt == b"@" {
                let mut last = None;
                let mut cur = self.next_code(t, file);
                while let Some(c) = cur {
                    if !self.is_ident(c) {
                        break;
                    }
                    last = Some(c);
                    match self.next_code(c, file) {
                        Some(dot) if self.is(dot, b".") => cur = self.next_code(dot, file),
                        _ => break,
                    }
                }
                if let Some(l) = last {
                    out.push(self.text_id(l));
                    t = l;
                }
            }
            steps += 1;
            if steps > MAX_HEADER_TOKENS {
                break;
            }
            t = match self.next_code(t, file) {
                Some(n) => n,
                None => break,
            };
        }
        out
    }

    /// Supertypes written in the header of a type declaration.
    ///
    /// Returns `None` when the language's header syntax is not modelled or the
    /// declared name cannot be located; `Some(empty)` means the header was read
    /// and declares no supertypes at bracket depth 0.
    pub fn declared_supertypes(
        &self,
        sym: &SymbolRecord,
        lang: LangId,
    ) -> Option<DeclaredSupertypes> {
        let kotlin_style = match lang {
            LangId::Java | LangId::JavaScript | LangId::TypeScript => false,
            LangId::Kotlin => true,
            _ => return None,
        };
        let name = self.name_token(sym)?;
        let file = self.file_of(name)?;

        #[derive(PartialEq, Clone, Copy)]
        enum Mode {
            None,
            Extends,
            Implements,
            Delegation,
            DelegationSkip,
        }

        let mut out = DeclaredSupertypes::default();
        let mut mode = Mode::None;
        let mut angle = 0i32;
        let mut paren = 0i32;
        let mut item_last: Option<u32> = None;
        let mut item_called = false;
        let mut steps = 0u32;
        let mut t = name;

        let flush = |mode: Mode,
                     item_last: &mut Option<u32>,
                     item_called: &mut bool,
                     out: &mut DeclaredSupertypes| {
            if let Some(id) = item_last.take() {
                match mode {
                    Mode::Extends => out.extends.push(id),
                    Mode::Implements => out.implements.push(id),
                    Mode::Delegation => out.delegation.push((id, *item_called)),
                    _ => {}
                }
            }
            *item_called = false;
        };

        while let Some(n) = self.next_code(t, file) {
            t = n;
            steps += 1;
            if steps > MAX_HEADER_TOKENS {
                break;
            }
            let txt = self.text(t);
            match txt {
                b"<" => {
                    angle += 1;
                    continue;
                }
                b">" => {
                    angle = (angle - 1).max(0);
                    continue;
                }
                b">>" => {
                    angle = (angle - 2).max(0);
                    continue;
                }
                b">>>" => {
                    angle = (angle - 3).max(0);
                    continue;
                }
                b"(" => {
                    if paren == 0 && angle == 0 && mode == Mode::Delegation && item_last.is_some() {
                        item_called = true;
                    }
                    paren += 1;
                    continue;
                }
                b")" => {
                    paren = (paren - 1).max(0);
                    continue;
                }
                _ => {}
            }
            if angle > 0 || paren > 0 {
                continue;
            }
            match txt {
                b"{" | b";" => break,
                b"extends" => {
                    flush(mode, &mut item_last, &mut item_called, &mut out);
                    mode = Mode::Extends;
                }
                b"implements" => {
                    flush(mode, &mut item_last, &mut item_called, &mut out);
                    mode = Mode::Implements;
                }
                b"permits" | b"where" | b"throws" => {
                    flush(mode, &mut item_last, &mut item_called, &mut out);
                    mode = Mode::None;
                }
                b":" if kotlin_style => {
                    flush(mode, &mut item_last, &mut item_called, &mut out);
                    mode = Mode::Delegation;
                }
                b"," => {
                    flush(mode, &mut item_last, &mut item_called, &mut out);
                    if mode == Mode::DelegationSkip {
                        mode = Mode::Delegation;
                    }
                }
                b"by" if mode == Mode::Delegation => {
                    flush(mode, &mut item_last, &mut item_called, &mut out);
                    mode = Mode::DelegationSkip;
                }
                b"." | b"?." => {}
                _ => {
                    if kotlin_style
                        && mode == Mode::None
                        && self.line_of(t) > self.line_of(name)
                        && (txt == b"class" || txt == b"fun" || txt == b"val" || txt == b"var")
                    {
                        // A Kotlin declaration without a body ended on the previous line.
                        break;
                    }
                    if self.is_ident(t)
                        && matches!(mode, Mode::Extends | Mode::Implements | Mode::Delegation)
                    {
                        item_last = Some(self.text_id(t));
                    }
                }
            }
        }
        flush(mode, &mut item_last, &mut item_called, &mut out);
        Some(out)
    }

    /// `{ ... }` token range of a member or type body; `None` for bodiless declarations.
    pub fn body_range(&self, sym: &SymbolRecord) -> Option<(u32, u32)> {
        let (start, file) = self.declaration_start(sym)?;
        let mut paren = 0i32;
        let mut t = start;
        let mut steps = 0u32;
        let open = loop {
            let txt = self.text(t);
            if txt == b"(" {
                paren += 1;
            } else if txt == b")" {
                paren = (paren - 1).max(0);
            } else if paren == 0 {
                if txt == b"{" {
                    break t;
                }
                if txt == b";" || (txt == b"=" && t > sym.first_token_id) {
                    return None;
                }
            }
            steps += 1;
            if steps > MAX_HEADER_TOKENS {
                return None;
            }
            t = self.next_code(t, file)?;
        };
        let mut depth = 0i64;
        let mut t = open;
        let mut steps = 0u32;
        loop {
            let txt = self.text(t);
            if txt == b"{" {
                depth += 1;
            } else if txt == b"}" {
                depth -= 1;
                if depth == 0 {
                    return Some((open, t));
                }
            }
            steps += 1;
            if steps > MAX_BODY_TOKENS {
                return None;
            }
            t = self.next_code(t, file)?;
        }
    }

    /// Closing `)` of the argument list that follows a call's name token (also
    /// skips a diamond or explicit type-argument list).
    pub fn call_close_paren(&self, name_tok: u32) -> Option<u32> {
        let file = self.file_of(name_tok)?;
        let mut t = self.next_code(name_tok, file)?;
        if self.is(t, b"<") {
            let mut angle = 0i32;
            loop {
                match self.text(t) {
                    b"<" => angle += 1,
                    b">" => angle -= 1,
                    b">>" => angle -= 2,
                    _ => {}
                }
                if angle <= 0 {
                    break;
                }
                t = self.next_code(t, file)?;
            }
            t = self.next_code(t, file)?;
        }
        if !self.is(t, b"(") {
            return None;
        }
        let mut depth = 0i32;
        let mut steps = 0u32;
        loop {
            match self.text(t) {
                b"(" => depth += 1,
                b")" => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(t);
                    }
                }
                _ => {}
            }
            steps += 1;
            if steps > MAX_ARGUMENT_TOKENS {
                return None;
            }
            t = self.next_code(t, file)?;
        }
    }

    /// Declared type shape of a field, property or parameter.
    pub fn type_shape(&self, sym: &SymbolRecord) -> TypeShape {
        let mut shape = TypeShape::default();
        let name = match self.name_token(sym) {
            Some(n) => n,
            None => return shape,
        };
        let file = match self.file_of(name) {
            Some(f) => f,
            None => return shape,
        };
        // Kotlin / TypeScript style: `name: Type`.
        if let Some(after) = self.next_code(name, file) {
            if self.is(after, b":") {
                self.read_type_tokens_forward(after, file, &mut shape);
                return shape;
            }
            if self.is(after, b"[") {
                shape.is_array = true;
            }
        }
        // Java style: type tokens precede the name.
        let start = match self.declaration_start(sym) {
            Some((s, _)) => s,
            None => return shape,
        };
        let mut angle = 0i32;
        let mut t = start;
        while t < name {
            let txt = self.text(t);
            match txt {
                b"<" => angle += 1,
                b">" => angle = (angle - 1).max(0),
                b">>" => angle = (angle - 2).max(0),
                b">>>" => angle = (angle - 3).max(0),
                b"[" | b"..." => {
                    if angle == 0 {
                        shape.is_array = true;
                    }
                }
                _ => {
                    if angle > 0 && self.is_ident(t) {
                        shape.type_argument_names.push(self.text_id(t));
                    }
                }
            }
            t = match self.next_code(t, file) {
                Some(n) => n,
                None => break,
            };
        }
        shape
    }

    fn read_type_tokens_forward(&self, colon: u32, file: u16, shape: &mut TypeShape) {
        let mut angle = 0i32;
        let mut t = colon;
        let mut steps = 0u32;
        let line = self.line_of(colon);
        while let Some(n) = self.next_code(t, file) {
            t = n;
            steps += 1;
            if steps > MAX_HEADER_TOKENS {
                break;
            }
            let txt = self.text(t);
            if angle == 0
                && (txt == b"=" || txt == b"," || txt == b")" || txt == b";" || txt == b"{")
            {
                break;
            }
            if angle == 0 && self.line_of(t) > line && txt != b"<" && txt != b"." {
                break;
            }
            match txt {
                b"<" => angle += 1,
                b">" => angle = (angle - 1).max(0),
                b">>" => angle = (angle - 2).max(0),
                b"?" => {
                    if angle == 0 {
                        shape.is_nullable = true;
                    }
                }
                b"[" => {
                    if angle == 0 {
                        shape.is_array = true;
                    }
                }
                _ => {
                    if angle > 0 && self.is_ident(t) {
                        shape.type_argument_names.push(self.text_id(t));
                    }
                }
            }
        }
    }

    /// Initializer tokens `= <expr>` of a field or property, exclusive of `=` and the terminator.
    pub fn initializer_range(&self, sym: &SymbolRecord) -> Option<(u32, u32)> {
        let name = self.name_token(sym)?;
        let file = self.file_of(name)?;
        let mut depth = 0i32;
        let mut t = name;
        let mut steps = 0u32;
        let eq = loop {
            t = self.next_code(t, file)?;
            steps += 1;
            if steps > MAX_HEADER_TOKENS {
                return None;
            }
            match self.text(t) {
                b"(" | b"[" | b"<" => depth += 1,
                b")" | b"]" | b">" => depth = (depth - 1).max(0),
                b"=" if depth == 0 => break t,
                b";" | b"," | b"{" | b"}" if depth == 0 => return None,
                _ => {}
            }
        };
        let first = self.next_code(eq, file)?;
        let mut last = first;
        let mut depth = 0i32;
        let mut t = first;
        let line = self.line_of(eq);
        let mut steps = 0u32;
        loop {
            match self.text(t) {
                b"(" | b"[" | b"{" => depth += 1,
                b")" | b"]" | b"}" => {
                    if depth == 0 {
                        break;
                    }
                    depth -= 1;
                }
                b";" | b"," if depth == 0 => break,
                _ => {}
            }
            last = t;
            steps += 1;
            if steps > MAX_ARGUMENT_TOKENS {
                break;
            }
            let next = match self.next_code(t, file) {
                Some(n) => n,
                None => break,
            };
            if depth == 0 && self.line_of(next) > line && !continues_expression(self.text(t)) {
                // Statement without a terminator (Kotlin) ended on this line.
                if !continues_expression(self.text(next)) {
                    break;
                }
            }
            t = next;
        }
        Some((first, last))
    }

    /// Single-pass lexical analysis of a `{ ... }` body.
    pub fn scan_body(&self, range: (u32, u32), lang: LangId) -> BodyFacts {
        let mut facts = BodyFacts {
            range,
            ..Default::default()
        };
        let file = match self.file_of(range.0) {
            Some(f) => f,
            None => return facts,
        };
        let code = self.code_tokens(range.0, range.1, file);
        if code.len() < 2 {
            return facts;
        }
        let terminated = lang == LangId::Java;
        let walker = BodyWalker::new(self, &code, terminated);
        let n = code.len();
        let mut consumed_while: HashSet<usize> = HashSet::new();

        let mut i = 1usize;
        while i + 1 < n {
            let txt = walker.txt(i);
            match txt {
                b"for" | b"while" if walker.txt(i + 1) == b"(" => {
                    if !consumed_while.contains(&i) {
                        let pc = walker.close_of(i + 1);
                        let end = walker.stmt_end(pc + 1, 0);
                        facts.loops.push((code[i], code[end.min(n - 1)]));
                    }
                }
                b"do" => {
                    let body_end = walker.stmt_end(i + 1, 0);
                    let mut end = body_end;
                    if body_end + 2 < n
                        && walker.txt(body_end + 1) == b"while"
                        && walker.txt(body_end + 2) == b"("
                    {
                        consumed_while.insert(body_end + 1);
                        end = walker.close_of(body_end + 2);
                    }
                    facts.loops.push((code[i], code[end.min(n - 1)]));
                }
                b"if" if walker.txt(i + 1) == b"(" => {
                    let pc = walker.close_of(i + 1);
                    if pc > i + 2 && pc + 1 < n {
                        let then_end = walker.stmt_end(pc + 1, 0);
                        let else_branch = if then_end + 2 < n && walker.txt(then_end + 1) == b"else"
                        {
                            let e_end = walker.stmt_end(then_end + 2, 0);
                            Some((code[then_end + 2], code[e_end.min(n - 1)]))
                        } else {
                            None
                        };
                        facts.ifs.push(IfRegion {
                            if_token: code[i],
                            cond: (code[i + 2], code[pc - 1]),
                            then_branch: (code[pc + 1], code[then_end.min(n - 1)]),
                            else_branch,
                        });
                    }
                }
                b"switch" | b"when" if walker.txt(i + 1) == b"(" => {
                    let pc = walker.close_of(i + 1);
                    if pc > i + 2 && pc + 1 < n && walker.txt(pc + 1) == b"{" {
                        let selector = (code[i + 2], code[pc - 1]);
                        self.collect_cases(&walker, &code, pc + 1, selector, &mut facts);
                    }
                }
                b"=" => {
                    if let Some(a) = walker.assignment_at(i) {
                        facts.assignments.push(a);
                    }
                }
                b"return" => {
                    facts.returns.push(walker.returned_at(i));
                }
                b"new" => {
                    if let Some(a) = walker.allocation_at(i) {
                        facts.allocations.push(a);
                    }
                }
                _ => {
                    if walker.txt(i + 1) == b"(" && self.is_ident(code[i]) {
                        if let Some(inv) = walker.invocation_at(i) {
                            if ITERATION_CALLS.contains(&self.text(inv.name_token)) {
                                // The region starts at the iterated receiver
                                // (`items.forEach(...)`) so it names the collection.
                                let start = if i >= 2 && walker.txt(i - 1) == b"." {
                                    code[i - 2]
                                } else {
                                    code[i + 1]
                                };
                                facts.loops.push((start, inv.close_token));
                            }
                            facts.invocations.push(inv);
                        }
                    }
                }
            }
            i += 1;
        }
        facts
    }

    fn collect_cases(
        &self,
        walker: &BodyWalker<'_, 'a>,
        code: &[u32],
        open_idx: usize,
        selector: (u32, u32),
        facts: &mut BodyFacts,
    ) {
        let n = code.len();
        let close_idx = walker.close_of(open_idx).min(n - 1);
        let mut positions = Vec::new();
        let mut j = open_idx + 1;
        while j < close_idx {
            let t = walker.txt(j);
            if t == b"{" || t == b"(" || t == b"[" {
                j = walker.close_of(j) + 1;
                continue;
            }
            if t == b"case" || t == b"default" {
                positions.push(j);
            }
            j += 1;
        }
        // Labels of empty fall-through cases (`case A: case B: body`) share the next body.
        let mut pending_labels: Vec<u32> = Vec::new();
        let mut pending_default = false;
        for (k, &cpos) in positions.iter().enumerate() {
            let next = positions.get(k + 1).copied().unwrap_or(close_idx);
            let is_default = walker.txt(cpos) == b"default" || pending_default;
            let mut labels = std::mem::take(&mut pending_labels);
            let mut last_label: Option<u32> = None;
            let mut j = cpos + 1;
            let mut label_end = cpos;
            while j < next {
                let t = walker.txt(j);
                if t == b":" || t == b"->" {
                    label_end = j;
                    break;
                }
                if t == b"(" || t == b"[" {
                    j = walker.close_of(j) + 1;
                    continue;
                }
                if t == b"," {
                    if let Some(l) = last_label.take() {
                        labels.push(l);
                    }
                } else if self.is_ident(code[j])
                    || matches!(
                        self.token_type(code[j]),
                        TokenType::IntegerLiteral
                            | TokenType::StringLiteral
                            | TokenType::CharLiteral
                    )
                {
                    last_label = Some(self.text_id(code[j]));
                }
                j += 1;
            }
            if let Some(l) = last_label {
                labels.push(l);
            }
            if label_end == cpos {
                continue;
            }
            if label_end + 1 >= next {
                pending_labels = labels;
                pending_default = is_default;
                continue;
            }
            pending_default = false;
            facts.cases.push(CaseRegion {
                case_token: code[cpos],
                selector,
                labels,
                body: (code[label_end + 1], code[(next - 1).max(label_end + 1)]),
                is_default,
            });
        }
    }
}

/// Index-based navigation over the code tokens of one body.
struct BodyWalker<'s, 'a> {
    scanner: &'s SourceScanner<'a>,
    code: &'s [u32],
    /// Index of the matching bracket for every `(`, `[` and `{` (or `usize::MAX`).
    matching: Vec<usize>,
    /// Statements are `;`-terminated (Java); otherwise line breaks also end statements.
    terminated: bool,
}

impl<'s, 'a> BodyWalker<'s, 'a> {
    fn new(scanner: &'s SourceScanner<'a>, code: &'s [u32], terminated: bool) -> Self {
        let mut matching = vec![usize::MAX; code.len()];
        let mut stack: Vec<(usize, u8)> = Vec::new();
        for (i, &tok) in code.iter().enumerate() {
            match scanner.text(tok) {
                b"(" => stack.push((i, b')')),
                b"[" => stack.push((i, b']')),
                b"{" => stack.push((i, b'}')),
                t @ (b")" | b"]" | b"}") => {
                    if let Some(pos) = stack.iter().rposition(|&(_, c)| c == t[0]) {
                        let (open, _) = stack[pos];
                        stack.truncate(pos);
                        matching[open] = i;
                    }
                }
                _ => {}
            }
        }
        Self {
            scanner,
            code,
            matching,
            terminated,
        }
    }

    fn len(&self) -> usize {
        self.code.len()
    }

    fn txt(&self, i: usize) -> &'a [u8] {
        match self.code.get(i) {
            Some(&t) => self.scanner.text(t),
            None => b"",
        }
    }

    fn tid(&self, i: usize) -> u32 {
        self.code
            .get(i)
            .map(|&t| self.scanner.text_id(t))
            .unwrap_or(u32::MAX)
    }

    fn ident(&self, i: usize) -> bool {
        self.code
            .get(i)
            .map(|&t| self.scanner.is_ident(t))
            .unwrap_or(false)
    }

    fn line(&self, i: usize) -> u32 {
        self.code
            .get(i)
            .map(|&t| self.scanner.line_of(t))
            .unwrap_or(0)
    }

    /// Matching close bracket index, or the last index when unbalanced.
    fn close_of(&self, i: usize) -> usize {
        match self.matching.get(i) {
            Some(&m) if m != usize::MAX => m,
            _ => self.len().saturating_sub(1),
        }
    }

    /// Statement ends at index `j` because of a line break (unterminated languages).
    fn ends_by_line(&self, j: usize) -> bool {
        if self.terminated || j + 1 >= self.len() {
            return false;
        }
        self.line(j + 1) > self.line(j)
            && !continues_expression(self.txt(j))
            && !continues_expression(self.txt(j + 1))
    }

    /// Index of the last token of the statement that starts at `i`.
    fn stmt_end(&self, i: usize, depth: u32) -> usize {
        let last = self.len().saturating_sub(1);
        if i >= last || depth > 256 {
            return i.min(last);
        }
        let t = self.txt(i);
        if t == b"{" {
            return self.close_of(i);
        }
        if (t == b"if" || t == b"while" || t == b"for" || t == b"synchronized" || t == b"switch")
            && self.txt(i + 1) == b"("
        {
            let pc = self.close_of(i + 1);
            if t == b"switch" {
                return if self.txt(pc + 1) == b"{" {
                    self.close_of(pc + 1)
                } else {
                    pc
                };
            }
            let body_end = self.stmt_end(pc + 1, depth + 1);
            if t == b"if" && body_end + 2 <= last && self.txt(body_end + 1) == b"else" {
                return self.stmt_end(body_end + 2, depth + 1);
            }
            return body_end;
        }
        if t == b"do" {
            let body_end = self.stmt_end(i + 1, depth + 1);
            if body_end + 2 <= last
                && self.txt(body_end + 1) == b"while"
                && self.txt(body_end + 2) == b"("
            {
                let pc = self.close_of(body_end + 2);
                return if self.txt(pc + 1) == b";" { pc + 1 } else { pc };
            }
            return body_end;
        }
        if t == b"try" {
            let block = if self.txt(i + 1) == b"(" {
                self.close_of(i + 1) + 1
            } else {
                i + 1
            };
            if self.txt(block) != b"{" {
                return block.min(last);
            }
            let mut end = self.close_of(block);
            loop {
                if end + 2 <= last && self.txt(end + 1) == b"catch" && self.txt(end + 2) == b"(" {
                    let pc = self.close_of(end + 2);
                    if self.txt(pc + 1) == b"{" {
                        end = self.close_of(pc + 1);
                        continue;
                    }
                    break;
                }
                if end + 2 <= last && self.txt(end + 1) == b"finally" && self.txt(end + 2) == b"{" {
                    end = self.close_of(end + 2);
                }
                break;
            }
            return end;
        }
        if t == b"else" {
            return self.stmt_end(i + 1, depth + 1);
        }
        let mut j = i;
        while j <= last {
            let tj = self.txt(j);
            if tj == b"(" || tj == b"[" || tj == b"{" {
                j = self.close_of(j);
                if self.ends_by_line(j) {
                    return j;
                }
                j += 1;
                continue;
            }
            if tj == b";" {
                return j;
            }
            if tj == b"}" {
                return j.saturating_sub(1).max(i);
            }
            if self.ends_by_line(j) {
                return j;
            }
            j += 1;
        }
        last
    }

    /// True when index `j` ends a simple expression statement.
    fn is_terminator(&self, j: usize) -> bool {
        let t = self.txt(j);
        t == b";" || t == b"}" || t == b"," || t == b")"
    }

    fn assignment_at(&self, i: usize) -> Option<Assignment> {
        if i == 0 || !self.ident(i - 1) {
            return None;
        }
        let l = i - 1;
        let via_this = l >= 2 && self.txt(l - 1) == b"." && self.txt(l - 2) == b"this";
        let via_other_receiver = l >= 1 && self.txt(l - 1) == b"." && !via_this;
        let is_declaration = l >= 1
            && !via_this
            && !via_other_receiver
            && (self.ident(l - 1)
                || self.txt(l - 1) == b">"
                || self.txt(l - 1) == b"]"
                || self.txt(l - 1) == b"?"
                || self.txt(l - 1) == b"val"
                || self.txt(l - 1) == b"var");
        let v = i + 1;
        let value = if v < self.len() && self.txt(v) == b"new" {
            match self.allocation_at(v) {
                Some(a) => AssignedValue::New {
                    type_text: a.type_text,
                    new_token: a.new_token,
                },
                None => AssignedValue::Other,
            }
        } else {
            self.pure_name_value(v)
        };
        Some(Assignment {
            lhs_text: self.tid(l),
            lhs_token: self.code[l],
            via_this,
            via_other_receiver,
            is_declaration,
            value,
        })
    }

    /// `A.B.C` followed by a terminator → `Name(C)`; anything else → `Other`.
    fn pure_name_value(&self, v: usize) -> AssignedValue {
        let mut j = v;
        loop {
            if j >= self.len() || !self.ident(j) {
                return AssignedValue::Other;
            }
            let text = self.tid(j);
            if self.ends_by_line(j) {
                return AssignedValue::Name { text };
            }
            if self.txt(j + 1) == b"." {
                j += 2;
                continue;
            }
            let next = j + 1;
            return if next >= self.len() || self.is_terminator(next) {
                AssignedValue::Name { text }
            } else {
                AssignedValue::Other
            };
        }
    }

    fn returned_at(&self, i: usize) -> ReturnedValue {
        let v = i + 1;
        if v >= self.len() {
            return ReturnedValue::Other;
        }
        if self.txt(v) == b"this" {
            if self.txt(v + 1) == b"." {
                if let AssignedValue::Name { text } = self.pure_name_value(v + 2) {
                    return ReturnedValue::Name(text);
                }
                return ReturnedValue::Other;
            }
            if v + 1 >= self.len() || self.is_terminator(v + 1) || self.ends_by_line(v) {
                return ReturnedValue::This;
            }
            return ReturnedValue::Other;
        }
        if self.txt(v) == b"new" {
            return match self.allocation_at(v) {
                Some(a) => ReturnedValue::New(a.type_text),
                None => ReturnedValue::Other,
            };
        }
        match self.pure_name_value(v) {
            AssignedValue::Name { text } => ReturnedValue::Name(text),
            _ => ReturnedValue::Other,
        }
    }

    fn allocation_at(&self, i: usize) -> Option<Allocation> {
        let mut j = i + 1;
        let mut last: Option<usize> = None;
        while j < self.len() && self.ident(j) {
            last = Some(j);
            if self.txt(j + 1) == b"." {
                j += 2;
            } else {
                break;
            }
        }
        let name_idx = last?;
        let mut k = name_idx + 1;
        if self.txt(k) == b"[" {
            return None; // array creation, not an object allocation
        }
        if self.txt(k) == b"<" {
            let mut angle = 0i32;
            while k < self.len() {
                match self.txt(k) {
                    b"<" => angle += 1,
                    b">" => angle -= 1,
                    b">>" => angle -= 2,
                    _ => {}
                }
                k += 1;
                if angle <= 0 {
                    break;
                }
            }
        }
        if self.txt(k) != b"(" {
            return None;
        }
        let pc = self.close_of(k);
        let anonymous = self.txt(pc + 1) == b"{";
        Some(Allocation {
            new_token: self.code[i],
            type_text: self.tid(name_idx),
            anonymous,
        })
    }

    fn invocation_at(&self, i: usize) -> Option<Invocation> {
        let name = self.txt(i);
        if NON_CALL_WORDS.contains(&name) {
            return None;
        }
        let close_token = self.code[self.close_of(i + 1)];
        let receiver = if i >= 1 && (self.txt(i - 1) == b"." || self.txt(i - 1) == b"?.") {
            if i < 2 {
                Receiver::Expression
            } else {
                let r = i - 2;
                match self.txt(r) {
                    b"this" => Receiver::This,
                    b"super" => Receiver::Super,
                    b")" | b"]" | b">" => Receiver::Expression,
                    _ if self.ident(r) => Receiver::Named(self.tid(r)),
                    _ => Receiver::Expression,
                }
            }
        } else if i >= 1 {
            let p = self.txt(i - 1);
            if p == b"new" || p == b"@" || p == b"fun" || p == b"function" || p == b"def" {
                return None;
            }
            if self.ident(i - 1) || p == b">" || p == b"]" {
                // `Type name(` — a declaration, not a call.
                return None;
            }
            Receiver::Implicit
        } else {
            Receiver::Implicit
        };
        Some(Invocation {
            name_token: self.code[i],
            method_text: self.tid(i),
            receiver,
            close_token,
        })
    }
}

/// Tokens after which an expression continues onto the next line.
fn continues_expression(t: &[u8]) -> bool {
    matches!(
        t,
        b"." | b"?."
            | b","
            | b"("
            | b"["
            | b"="
            | b"+"
            | b"-"
            | b"*"
            | b"/"
            | b"%"
            | b"&&"
            | b"||"
            | b"?"
            | b":"
            | b"->"
            | b"=="
            | b"!="
            | b"<"
            | b">"
            | b"<="
            | b">="
            | b"&"
            | b"|"
            | b"^"
            | b"?:"
            | b"+="
            | b"-="
    )
}

/// Full Unicode identifier check (JLS 3.8 / Kotlin): letters, digits, `_`, `$`.
pub fn is_identifier_text(bytes: &[u8]) -> bool {
    let s = match std::str::from_utf8(bytes) {
        Ok(s) if !s.is_empty() => s,
        _ => return false,
    };
    let mut chars = s.chars();
    let first = match chars.next() {
        Some(c) => c,
        None => return false,
    };
    if !(first.is_alphabetic() || first == '_' || first == '$') {
        return false;
    }
    chars.all(|c| c.is_alphanumeric() || c == '_' || c == '$')
}
