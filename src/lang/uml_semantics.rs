//! C-family UML semantics (§9.2.1): member relationships, supertypes, templates and
//! generics, namespaces, visibility and member flags for C, C++ and C#.
//!
//! The Java extractor reads relationships off the symbol table and the declared type
//! shape. In the C family the declared type carries more meaning than its head symbol:
//! C++ spells ownership in the type (`std::unique_ptr<T>` owns, `std::shared_ptr<T>`
//! shares, `T*`, `T&` and `std::weak_ptr<T>` only refer), and in C# a member's
//! relationship depends on whether the target is a `struct` (value semantics) or a
//! `class` (reference semantics). This module re-reads every C-family type body from the
//! token corpus with bracket-matched recognizers, builds a per-class fact model
//! (`CFamilyUmlModel`) and classifies each attribute against the project's own types, a
//! model of the C++ standard library (and common C++ libraries), and the .NET BCL.
//!
//! Relationship rules:
//! - C/C++: a value member, `std::unique_ptr<T>`, `std::optional<T>` and an owning
//!   container of values or unique pointers is a composition; `std::shared_ptr<T>` is a
//!   shared aggregation; raw pointers, references, `std::weak_ptr<T>`, spans and views are
//!   associations. Along a nested type the weakest link wins (`std::vector<T*>` refers).
//! - C#: a class-, interface- or delegate-typed member is an association (an aggregation
//!   through a collection); a struct-typed member is a composition; an event is an
//!   association to its delegate type with multiplicity `0..*`.
//! - Multiplicities multiply along the type: `T[4]` is `4`, `std::array<T, N>` is `N`,
//!   pointers and `std::optional` are `0..1`, containers, arrays and `IEnumerable<T>` are
//!   `0..*`, C# `T?` is `0..1`.
//! - Interface implementation is a realization, inheritance a generalization (C++ bases
//!   keep their access and `virtual` flag; a pure-abstract C++ class counts as an
//!   interface when `SemanticsOptions::cpp_pure_abstract_as_interface` is set).
//! - Namespaces are packages; template and generic uses carry their bound parameters.
//!
//! **Unknown markers.** A type that cannot be parsed is `None`; a target that resolves to
//! no project, library or built-in type is `TypeOrigin::Unknown` with `UNKNOWN_SYM`; a
//! bound that cannot be derived is `MULTIPLICITY_UNKNOWN`. Ambiguous names stay unknown.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::core::types::symbol::{SymbolKind, SymbolModifiers, SymbolVisibility};
use crate::core::types::token::{unpack_sort_key, LangId, TokenType};
use crate::ingestion::TokenCorpusArtifact;
use crate::symbol::SymbolTableArtifact;
use crate::uma::types::{
    ASSOC_AGGREGATION, ASSOC_ASSOCIATION, ASSOC_COMPOSITION, ASSOC_DEPENDENCY, ASSOC_NONE,
    FIELD_KIND_ATTRIBUTE, FIELD_KIND_ENUM_LITERAL, MULTIPLICITY_MANY, MULTIPLICITY_UNKNOWN,
    STEREOTYPE_ABSTRACT, STEREOTYPE_ENUM, STEREOTYPE_INTERFACE, STEREOTYPE_NONE,
    STEREOTYPE_RECORD, UNKNOWN_SYM,
};

/// Maximum nesting of type arguments and declarators accepted by the type parser.
const MAX_TYPE_DEPTH: u32 = 48;
/// Maximum alias expansions applied to one written type.
const MAX_ALIAS_EXPANSIONS: u32 = 8;
/// Maximum tokens between a `"` token and its closing `"` in a split string literal.
const MAX_SPLIT_STRING_TOKENS: usize = 8;

// ── Language Family ───────────────────────────────────────────────────────────

/// The C-family languages handled by this module.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CFamily {
    C,
    Cpp,
    CSharp,
}

impl CFamily {
    pub fn from_lang_id(lang: LangId) -> Option<Self> {
        match lang {
            LangId::C => Some(CFamily::C),
            LangId::Cpp => Some(CFamily::Cpp),
            LangId::CSharp => Some(CFamily::CSharp),
            _ => None,
        }
    }

    pub fn lang_id(self) -> LangId {
        match self {
            CFamily::C => LangId::C,
            CFamily::Cpp => LangId::Cpp,
            CFamily::CSharp => LangId::CSharp,
        }
    }

    /// C and C++ share headers, declarator syntax and name lookup.
    pub fn is_c_like(self) -> bool {
        matches!(self, CFamily::C | CFamily::Cpp)
    }

    /// Separator of qualified names as written in source.
    pub fn scope_separator(self) -> &'static str {
        match self {
            CFamily::CSharp => ".",
            _ => "::",
        }
    }

    /// Languages whose types can name each other (C and C++ share headers).
    pub fn shares_names_with(self, other: CFamily) -> bool {
        self.is_c_like() == other.is_c_like()
    }
}

// ── Keywords ──────────────────────────────────────────────────────────────────

const C_KEYWORDS: &[&str] = &[
    "auto", "break", "case", "char", "const", "continue", "default", "do", "double", "else",
    "enum", "extern", "float", "for", "goto", "if", "inline", "int", "long", "register",
    "restrict", "return", "short", "signed", "sizeof", "static", "struct", "switch", "typedef",
    "union", "unsigned", "void", "volatile", "while", "_Alignas", "_Alignof", "_Atomic",
    "_Bool", "_Complex", "_Generic", "_Imaginary", "_Noreturn", "_Static_assert",
    "_Thread_local", "bool", "true", "false", "nullptr", "typeof", "constexpr",
    "static_assert", "thread_local", "alignas", "alignof",
];

const CPP_KEYWORDS: &[&str] = &[
    "alignas", "alignof", "and", "and_eq", "asm", "auto", "bitand", "bitor", "bool", "break",
    "case", "catch", "char", "char8_t", "char16_t", "char32_t", "class", "compl", "concept",
    "const", "consteval", "constexpr", "constinit", "const_cast", "continue", "co_await",
    "co_return", "co_yield", "decltype", "default", "delete", "do", "double", "dynamic_cast",
    "else", "enum", "explicit", "export", "extern", "false", "float", "for", "friend", "goto",
    "if", "inline", "int", "long", "mutable", "namespace", "new", "noexcept", "not", "not_eq",
    "nullptr", "operator", "or", "or_eq", "private", "protected", "public", "register",
    "reinterpret_cast", "requires", "return", "short", "signed", "sizeof", "static",
    "static_assert", "static_cast", "struct", "switch", "template", "this", "thread_local",
    "throw", "true", "try", "typedef", "typeid", "typename", "union", "unsigned", "using",
    "virtual", "void", "volatile", "wchar_t", "while", "xor", "xor_eq",
];

const CS_KEYWORDS: &[&str] = &[
    "abstract", "as", "base", "bool", "break", "byte", "case", "catch", "char", "checked",
    "class", "const", "continue", "decimal", "default", "delegate", "do", "double", "else",
    "enum", "event", "explicit", "extern", "false", "finally", "fixed", "float", "for",
    "foreach", "goto", "if", "implicit", "in", "int", "interface", "internal", "is", "lock",
    "long", "namespace", "new", "null", "object", "operator", "out", "override", "params",
    "private", "protected", "public", "readonly", "ref", "return", "sbyte", "sealed", "short",
    "sizeof", "stackalloc", "static", "string", "struct", "switch", "this", "throw", "true",
    "try", "typeof", "uint", "ulong", "unchecked", "unsafe", "ushort", "using", "virtual",
    "void", "volatile", "while",
];

fn is_reserved(lang: CFamily, s: &str) -> bool {
    match lang {
        CFamily::C => C_KEYWORDS.contains(&s),
        CFamily::Cpp => CPP_KEYWORDS.contains(&s),
        CFamily::CSharp => CS_KEYWORDS.contains(&s),
    }
}

/// Identifier spelling (C# `@`-escaped identifiers included); keywords are not excluded.
pub fn is_identifier_spelling(s: &str) -> bool {
    let body = s.strip_prefix('@').unwrap_or(s);
    let mut chars = body.chars();
    match chars.next() {
        Some(c) if c.is_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_alphanumeric() || c == '_')
}

/// A spelling usable as a name in `lang` (an identifier that is not a reserved keyword).
pub fn is_name(lang: CFamily, s: &str) -> bool {
    if !is_identifier_spelling(s) {
        return false;
    }
    if s.starts_with('@') {
        return lang == CFamily::CSharp;
    }
    !is_reserved(lang, s)
}

/// Name without a C# verbatim `@` prefix.
pub fn plain_name(s: &str) -> &str {
    s.strip_prefix('@').unwrap_or(s)
}

/// ALL-CAPS identifier (`Q_OBJECT`, `DLL_EXPORT`): the spelling of a preprocessor macro.
fn is_macro_like(s: &str) -> bool {
    s.len() >= 2
        && s.bytes().any(|b| b.is_ascii_uppercase())
        && s
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
        && s.as_bytes()[0] != b'_'
}

/// String literal spelled as one token (`"C"`, `L"x"`, `u8"x"`, `@"x"`, `$"x"`).
fn is_string_literal_token(s: &str) -> bool {
    s.len() >= 2 && s.ends_with('"') && s.contains('"') && s.find('"') != Some(s.len() - 1)
}

/// Parses a C/C++/C# integer literal (decimal, hex, octal, binary, digit separators,
/// suffixes); `None` for anything else.
pub fn parse_int_literal(s: &str) -> Option<u64> {
    let cleaned: String = s.chars().filter(|&c| c != '\'' && c != '_').collect();
    let lower = cleaned.to_ascii_lowercase();
    let body = lower.trim_end_matches(|c: char| c == 'u' || c == 'l' || c == 'z');
    if body.is_empty() || !body.as_bytes()[0].is_ascii_digit() {
        return None;
    }
    if let Some(h) = body.strip_prefix("0x") {
        return u64::from_str_radix(h, 16).ok();
    }
    if let Some(b) = body.strip_prefix("0b") {
        return u64::from_str_radix(b, 2).ok();
    }
    if body.len() > 1 && body.starts_with('0') {
        return u64::from_str_radix(&body[1..], 8).ok();
    }
    body.parse::<u64>().ok()
}

// ── Code Tokens ───────────────────────────────────────────────────────────────

/// One non-trivia token of a C-family file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CodeTok<'a> {
    /// Global token id in the corpus.
    pub id: u32,
    pub text: &'a str,
    pub line: u32,
}

/// C# nullable annotation context (`<Nullable>` project property, `#nullable` directives).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NullableContext {
    Enabled,
    Disabled,
    /// Neither the project settings nor a directive state the context.
    Unknown,
}

/// A `#nullable` directive: from token index `at` on, the annotation context is
/// `context` (`None` restores the project default).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NullableMark {
    pub at: usize,
    pub context: Option<NullableContext>,
}

/// Code tokens of one C-family file with preprocessor directive lines removed.
#[derive(Debug, Clone)]
pub struct FileTokens<'a> {
    pub file_id: u16,
    pub lang: CFamily,
    pub toks: Vec<CodeTok<'a>>,
    pub nullable_marks: Vec<NullableMark>,
}

impl<'a> FileTokens<'a> {
    /// Token texts of `[start, end)`.
    pub fn texts(&self, start: usize, end: usize) -> Vec<&'a str> {
        let end = end.min(self.toks.len());
        if start >= end {
            return Vec::new();
        }
        self.toks[start..end].iter().map(|t| t.text).collect()
    }

    /// Text of token `i` (`""` past the end).
    pub fn text(&self, i: usize) -> &'a str {
        self.toks.get(i).map(|t| t.text).unwrap_or("")
    }

    /// Global token id of token `i` (`u32::MAX` past the end).
    pub fn token_id(&self, i: usize) -> u32 {
        self.toks.get(i).map(|t| t.id).unwrap_or(u32::MAX)
    }

    /// Local index of the token with global id `id`.
    pub fn index_of(&self, id: u32) -> Option<usize> {
        self.toks.binary_search_by_key(&id, |t| t.id).ok()
    }
}

fn is_trivia(token_type: u8) -> bool {
    matches!(
        TokenType::from_u8(token_type),
        TokenType::CommentLine
            | TokenType::CommentBlock
            | TokenType::CommentDoc
            | TokenType::Whitespace
            | TokenType::Newline
    )
}

/// Collects the code tokens of every C-family file of the corpus, in file-id order.
pub fn collect_file_tokens(tca: &TokenCorpusArtifact) -> Vec<FileTokens<'_>> {
    let mut lang_of: HashMap<u16, CFamily> = HashMap::new();
    for rec in &tca.file_records {
        if let Some(lang) = CFamily::from_lang_id(LangId::from(rec.language_id)) {
            lang_of.insert(rec.file_id, lang);
        }
    }
    let mut by_file: BTreeMap<u16, (CFamily, Vec<CodeTok>)> = BTreeMap::new();
    for (i, rec) in tca.token_records.iter().enumerate() {
        if is_trivia(rec.token_type) {
            continue;
        }
        let (file, line, _) = unpack_sort_key(rec.sort_key);
        let lang = match lang_of.get(&file) {
            Some(&l) => l,
            None => continue,
        };
        let text = match std::str::from_utf8(tca.interner.lookup_text(rec.text_id)) {
            Ok(s) if !s.is_empty() => s,
            _ => continue,
        };
        let id = match u32::try_from(i) {
            Ok(v) => v,
            Err(_) => break,
        };
        by_file
            .entry(file)
            .or_insert_with(|| (lang, Vec::new()))
            .1
            .push(CodeTok { id, text, line });
    }
    by_file
        .into_iter()
        .map(|(file_id, (lang, raw))| {
            let (toks, nullable_marks) = strip_preprocessor(lang, raw);
            FileTokens {
                file_id,
                lang,
                toks,
                nullable_marks,
            }
        })
        .collect()
}

/// Removes preprocessor directive lines (a line whose first token starts with `#`) and
/// records C# `#nullable` directives.
fn strip_preprocessor(lang: CFamily, raw: Vec<CodeTok<'_>>) -> (Vec<CodeTok<'_>>, Vec<NullableMark>) {
    let mut out = Vec::with_capacity(raw.len());
    let mut marks = Vec::new();
    let mut prev_line: Option<u32> = None;
    let mut i = 0usize;
    while i < raw.len() {
        let t = raw[i];
        let first_on_line = prev_line != Some(t.line);
        if first_on_line && t.text.starts_with('#') {
            let line = t.line;
            let mut words: Vec<&str> = Vec::new();
            let mut j = i;
            while j < raw.len() && raw[j].line == line {
                words.extend(raw[j].text.split_whitespace());
                j += 1;
            }
            if lang == CFamily::CSharp {
                if let Some(context) = nullable_directive(&words) {
                    marks.push(NullableMark {
                        at: out.len(),
                        context,
                    });
                }
            }
            prev_line = Some(line);
            i = j;
            continue;
        }
        prev_line = Some(t.line);
        out.push(t);
        i += 1;
    }
    (out, marks)
}

/// Parses `#nullable enable|disable|restore [annotations|warnings]`; `warnings`-only
/// directives do not change the annotation context.
fn nullable_directive(words: &[&str]) -> Option<Option<NullableContext>> {
    let mut it = words
        .iter()
        .map(|w| w.trim_start_matches('#'))
        .filter(|w| !w.is_empty());
    if it.next()? != "nullable" {
        return None;
    }
    let setting = it.next()?;
    if it.next() == Some("warnings") {
        return None;
    }
    match setting {
        "enable" => Some(Some(NullableContext::Enabled)),
        "disable" => Some(Some(NullableContext::Disabled)),
        "restore" => Some(None),
        _ => None,
    }
}

/// Splits written type text (`std::map<int, Foo*>`) into tokens accepted by `parse_type`.
pub fn lex_type_text(text: &str) -> Vec<&str> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        let c = bytes[i];
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        if c == b'_' || c == b'@' || c.is_ascii_alphabetic() || c >= 0x80 {
            i += 1;
            while i < bytes.len()
                && (bytes[i] == b'_' || bytes[i].is_ascii_alphanumeric() || bytes[i] >= 0x80)
            {
                i += 1;
            }
        } else if c.is_ascii_digit() {
            i += 1;
            while i < bytes.len()
                && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'\'' || bytes[i] == b'.')
            {
                i += 1;
            }
        } else {
            let rest = &text[i..];
            let len = ["...", "::", "&&", "->"]
                .iter()
                .find(|p| rest.starts_with(**p))
                .map(|p| p.len())
                .unwrap_or(1);
            i += len;
        }
        out.push(&text[start..i]);
    }
    out
}

// ── Type Expressions ──────────────────────────────────────────────────────────

/// Extent of a C/C++ array declarator.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ArrayExtent {
    Literal(u64),
    /// A named constant or expression (`[MAX_ITEMS]`), kept as written.
    Symbolic(String),
    /// `[]`: flexible, deduced or unknown extent.
    Unspecified,
}

/// One declarator applied to a type, innermost first.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Declarator {
    Pointer { is_const: bool },
    LValueRef,
    RValueRef,
    Array(ArrayExtent),
    /// C# array of the given rank (`T[]` is rank 1, `T[,]` rank 2).
    CsArray { rank: u8 },
    /// C# `T?`.
    Nullable,
    /// C++ pointer to member `C::*`.
    MemberPointer(String),
    /// C++ pack expansion `T...`.
    PackExpansion,
}

/// A template or generic argument.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TypeArg {
    Type(TypeExpr),
    /// A non-type argument (`3`, `N + 1`, `sizeof(T)`), kept as written.
    Value(String),
}

/// One segment of a qualified name with its own argument list.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NameSegment {
    pub name: String,
    pub args: Option<Vec<TypeArg>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TupleElement {
    pub ty: TypeExpr,
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TypeHead {
    /// A (possibly qualified) type name; `global` for a leading `::` or `global::`.
    Named {
        global: bool,
        segments: Vec<NameSegment>,
    },
    /// A built-in keyword type (`int`, `unsigned long`, `string`).
    Builtin(String),
    /// C# tuple type `(T1 a, T2 b)`.
    Tuple(Vec<TupleElement>),
    /// Function type `R(Args...)` (inside `std::function<...>`, function pointers, C#
    /// `delegate*`).
    Function {
        ret: Box<TypeExpr>,
        params: Vec<TypeExpr>,
    },
    /// `auto`, `var`, `decltype(...)`: the type is not written.
    Inferred,
}

/// A written type: head, cv-qualifiers of the head, and declarators from the head
/// outwards (`const T* const&` is head `T` const with `[Pointer{const}, LValueRef]`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TypeExpr {
    pub head: TypeHead,
    pub is_const: bool,
    pub is_volatile: bool,
    pub declarators: Vec<Declarator>,
}

impl TypeExpr {
    pub fn from_head(head: TypeHead) -> Self {
        Self {
            head,
            is_const: false,
            is_volatile: false,
            declarators: Vec::new(),
        }
    }

    /// A one-segment named type without arguments.
    pub fn simple(name: &str) -> Self {
        Self::from_head(TypeHead::Named {
            global: false,
            segments: vec![NameSegment {
                name: name.to_string(),
                args: None,
            }],
        })
    }

    /// Segments of a named type (empty for other heads).
    pub fn segments(&self) -> &[NameSegment] {
        match &self.head {
            TypeHead::Named { segments, .. } => segments,
            _ => &[],
        }
    }

    /// Last segment name of a named type.
    pub fn simple_name(&self) -> Option<&str> {
        self.segments().last().map(|s| s.name.as_str())
    }

    /// Arguments of the last segment.
    pub fn last_args(&self) -> &[TypeArg] {
        self.segments()
            .last()
            .and_then(|s| s.args.as_deref())
            .unwrap_or(&[])
    }

    /// Number of arguments of the last segment.
    pub fn arity(&self) -> usize {
        self.last_args().len()
    }

    /// Segment names joined with `sep`, without arguments.
    pub fn qualified_name(&self, sep: &str) -> Option<String> {
        let segs = self.segments();
        if segs.is_empty() {
            return None;
        }
        Some(
            segs.iter()
                .map(|s| s.name.as_str())
                .collect::<Vec<_>>()
                .join(sep),
        )
    }

    pub fn is_plain(&self) -> bool {
        self.declarators.is_empty()
    }

    /// The head with its arguments, without declarators or cv-qualifiers.
    pub fn without_declarators(&self) -> TypeExpr {
        TypeExpr {
            head: self.head.clone(),
            is_const: false,
            is_volatile: false,
            declarators: Vec::new(),
        }
    }

    /// The type with its outermost declarator removed.
    pub fn peel_outermost(&self) -> TypeExpr {
        let mut t = self.clone();
        t.declarators.pop();
        t
    }

    /// The `k`-th argument of the last segment when it is a type.
    pub fn type_arg(&self, k: usize) -> Option<&TypeExpr> {
        match self.last_args().get(k) {
            Some(TypeArg::Type(t)) => Some(t),
            _ => None,
        }
    }

    /// True when any named type inside `self` (head or arguments) has simple name `name`.
    pub fn mentions(&self, name: &str) -> bool {
        let mut named = Vec::new();
        collect_named(self, true, &mut named);
        named.iter().any(|t| t.simple_name() == Some(name))
    }

    /// Canonical spelling of the type in `lang`.
    pub fn display(&self, lang: CFamily) -> String {
        let mut s = String::new();
        if self.is_const && lang.is_c_like() {
            s.push_str("const ");
        }
        if self.is_volatile {
            s.push_str("volatile ");
        }
        match &self.head {
            TypeHead::Named { global, segments } => {
                if *global {
                    s.push_str(if lang == CFamily::CSharp { "global::" } else { "::" });
                }
                for (k, seg) in segments.iter().enumerate() {
                    if k > 0 {
                        s.push_str(lang.scope_separator());
                    }
                    s.push_str(&seg.name);
                    if let Some(args) = &seg.args {
                        s.push('<');
                        s.push_str(&display_args(args, lang));
                        s.push('>');
                    }
                }
            }
            TypeHead::Builtin(b) => s.push_str(b),
            TypeHead::Tuple(elems) => {
                s.push('(');
                let parts: Vec<String> = elems
                    .iter()
                    .map(|e| match &e.name {
                        Some(n) => format!("{} {}", e.ty.display(lang), n),
                        None => e.ty.display(lang),
                    })
                    .collect();
                s.push_str(&parts.join(", "));
                s.push(')');
            }
            TypeHead::Function { ret, params } => {
                s.push_str(&ret.display(lang));
                s.push('(');
                let parts: Vec<String> = params.iter().map(|p| p.display(lang)).collect();
                s.push_str(&parts.join(", "));
                s.push(')');
            }
            TypeHead::Inferred => s.push_str(if lang == CFamily::CSharp { "var" } else { "auto" }),
        }
        for d in &self.declarators {
            match d {
                Declarator::Pointer { is_const } => {
                    s.push('*');
                    if *is_const {
                        s.push_str(" const");
                    }
                }
                Declarator::LValueRef => s.push('&'),
                Declarator::RValueRef => s.push_str("&&"),
                Declarator::Array(ArrayExtent::Literal(n)) => s.push_str(&format!("[{}]", n)),
                Declarator::Array(ArrayExtent::Symbolic(t)) => s.push_str(&format!("[{}]", t)),
                Declarator::Array(ArrayExtent::Unspecified) => s.push_str("[]"),
                Declarator::CsArray { rank } => {
                    s.push('[');
                    for _ in 1..*rank {
                        s.push(',');
                    }
                    s.push(']');
                }
                Declarator::Nullable => s.push('?'),
                Declarator::MemberPointer(c) => s.push_str(&format!(" {}::*", c)),
                Declarator::PackExpansion => s.push_str("..."),
            }
        }
        s
    }
}

fn display_args(args: &[TypeArg], lang: CFamily) -> String {
    args.iter()
        .map(|a| match a {
            TypeArg::Type(t) => t.display(lang),
            TypeArg::Value(v) => v.clone(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Every named type inside `ty` (depth-first, arguments included).
pub fn collect_named<'t>(ty: &'t TypeExpr, include_self: bool, out: &mut Vec<&'t TypeExpr>) {
    match &ty.head {
        TypeHead::Named { segments, .. } => {
            if include_self {
                out.push(ty);
            }
            for seg in segments {
                if let Some(args) = &seg.args {
                    for a in args {
                        if let TypeArg::Type(t) = a {
                            collect_named(t, true, out);
                        }
                    }
                }
            }
        }
        TypeHead::Tuple(elems) => {
            for e in elems {
                collect_named(&e.ty, true, out);
            }
        }
        TypeHead::Function { ret, params } => {
            collect_named(ret, true, out);
            for p in params {
                collect_named(p, true, out);
            }
        }
        TypeHead::Builtin(_) | TypeHead::Inferred => {}
    }
}

// ── Type Parser ───────────────────────────────────────────────────────────────

/// Why a written type could not be parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeParseError {
    /// Index of the offending input token.
    pub position: usize,
    pub reason: &'static str,
}

impl std::fmt::Display for TypeParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} at token {}", self.reason, self.position)
    }
}

impl std::error::Error for TypeParseError {}

/// Parses a complete written type; every token must belong to it.
pub fn parse_type(lang: CFamily, toks: &[&str]) -> Result<TypeExpr, TypeParseError> {
    let mut p = TypeParser::new(lang, toks);
    let ty = p.full_type()?;
    if !p.at_end() {
        return Err(p.error("unexpected token after the type"));
    }
    Ok(ty)
}

/// Parses the longest type at the start of `toks` (declarators included) and returns it
/// with the number of input tokens consumed.
pub fn parse_type_greedy(lang: CFamily, toks: &[&str]) -> Result<(TypeExpr, usize), TypeParseError> {
    let mut p = TypeParser::new(lang, toks);
    let ty = p.full_type()?;
    Ok((ty, p.consumed()))
}

/// Parses the decl-specifier type at the start of a declaration. For C and C++ the
/// pointer and reference declarators are left unconsumed because they bind to each
/// declarator (`int *a, b;`); for C# the whole type is consumed.
pub fn parse_base_type(lang: CFamily, toks: &[&str]) -> Result<(TypeExpr, usize), TypeParseError> {
    let mut p = TypeParser::new(lang, toks);
    let ty = p.base_type()?;
    Ok((ty, p.consumed()))
}

/// Parses `<A, B, ...>` (the angle brackets included).
pub fn parse_type_arguments(lang: CFamily, toks: &[&str]) -> Result<Vec<TypeArg>, TypeParseError> {
    let mut p = TypeParser::new(lang, toks);
    let args = p.template_args()?;
    if !p.at_end() {
        return Err(p.error("unexpected token after the argument list"));
    }
    Ok(args)
}

const C_BUILTIN_WORDS: &[&str] = &[
    "void", "bool", "_Bool", "char", "wchar_t", "char8_t", "char16_t", "char32_t", "short",
    "int", "long", "signed", "unsigned", "float", "double", "__int8", "__int16", "__int32",
    "__int64", "__int128", "_Complex", "_Imaginary", "__float128", "_Float16", "_Float32",
    "_Float64", "_Float128", "__bf16", "__wchar_t", "__signed", "__signed__", "__unsigned",
];

const CS_BUILTIN_WORDS: &[&str] = &[
    "bool", "byte", "sbyte", "char", "decimal", "double", "float", "int", "uint", "nint",
    "nuint", "long", "ulong", "short", "ushort", "object", "string", "void", "dynamic",
];

/// C/C++ specifiers that may be interleaved with the type (`int static x`) and are not
/// part of it; member flags are read separately.
const C_NON_TYPE_SPECIFIERS: &[&str] = &[
    "static", "inline", "virtual", "explicit", "constexpr", "consteval", "constinit",
    "mutable", "extern", "thread_local", "register", "friend", "__inline", "__forceinline",
    "__inline__", "_Thread_local", "_Noreturn", "__thread",
];

const C_POINTER_QUALIFIERS: &[&str] = &[
    "volatile", "restrict", "__restrict", "__restrict__", "_Nonnull", "_Nullable",
    "_Null_unspecified", "__ptr64", "__ptr32", "__unaligned",
];

struct TypeParser<'t> {
    lang: CFamily,
    toks: Vec<&'t str>,
    /// Input index of each (split) token.
    origin: Vec<usize>,
    input_len: usize,
    pos: usize,
    depth: u32,
}

impl<'t> TypeParser<'t> {
    fn new(lang: CFamily, input: &[&'t str]) -> Self {
        let mut toks = Vec::with_capacity(input.len());
        let mut origin = Vec::with_capacity(input.len());
        for (k, t) in input.iter().enumerate() {
            // `>>` closes two argument lists inside a type.
            if t.len() > 1 && t.bytes().all(|b| b == b'>') {
                for _ in 0..t.len() {
                    toks.push(">");
                    origin.push(k);
                }
            } else {
                toks.push(*t);
                origin.push(k);
            }
        }
        Self {
            lang,
            toks,
            origin,
            input_len: input.len(),
            pos: 0,
            depth: 0,
        }
    }

    fn peek(&self) -> Option<&'t str> {
        self.toks.get(self.pos).copied()
    }

    fn peek_at(&self, k: usize) -> Option<&'t str> {
        self.toks.get(self.pos + k).copied()
    }

    fn at_end(&self) -> bool {
        self.pos >= self.toks.len()
    }

    fn advance(&mut self) -> Option<&'t str> {
        let t = self.peek()?;
        self.pos += 1;
        Some(t)
    }

    fn eat(&mut self, s: &str) -> bool {
        if self.peek() == Some(s) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    /// Input tokens consumed so far.
    fn consumed(&self) -> usize {
        if self.pos >= self.toks.len() {
            self.input_len
        } else {
            self.origin[self.pos]
        }
    }

    fn error(&self, reason: &'static str) -> TypeParseError {
        TypeParseError {
            position: self.consumed(),
            reason,
        }
    }

    fn enter(&mut self) -> Result<(), TypeParseError> {
        self.depth += 1;
        if self.depth > MAX_TYPE_DEPTH {
            Err(self.error("type nesting too deep"))
        } else {
            Ok(())
        }
    }

    fn leave(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    fn full_type(&mut self) -> Result<TypeExpr, TypeParseError> {
        self.enter()?;
        let r = self.full_type_inner();
        self.leave();
        r
    }

    fn full_type_inner(&mut self) -> Result<TypeExpr, TypeParseError> {
        let mut ty = self.base_type()?;
        if self.lang.is_c_like() {
            self.c_declarators(&mut ty)?;
        }
        Ok(ty)
    }

    fn base_type(&mut self) -> Result<TypeExpr, TypeParseError> {
        match self.lang {
            CFamily::CSharp => self.cs_type(),
            _ => self.c_base_type(),
        }
    }

    /// Skips a balanced `( ... )` group starting at the current `(`.
    fn skip_group(&mut self) -> Result<(), TypeParseError> {
        if !self.eat("(") {
            return Err(self.error("expected '('"));
        }
        let mut depth = 1i32;
        while let Some(t) = self.advance() {
            match t {
                "(" => depth += 1,
                ")" => {
                    depth -= 1;
                    if depth == 0 {
                        return Ok(());
                    }
                }
                _ => {}
            }
        }
        Err(self.error("unterminated parenthesis"))
    }

    fn c_base_type(&mut self) -> Result<TypeExpr, TypeParseError> {
        let mut is_const = false;
        let mut is_volatile = false;
        let mut builtin: Vec<&'t str> = Vec::new();
        let mut head: Option<TypeHead> = None;
        while let Some(t) = self.peek() {
            let free = head.is_none() && builtin.is_empty();
            match t {
                "const" => {
                    is_const = true;
                    self.pos += 1;
                }
                "volatile" => {
                    is_volatile = true;
                    self.pos += 1;
                }
                "typename" | "struct" | "class" | "union" | "enum" if free => self.pos += 1,
                "auto" | "__auto_type" if free => {
                    head = Some(TypeHead::Inferred);
                    self.pos += 1;
                }
                "decltype" | "typeof" | "__typeof__" | "__typeof" if free => {
                    self.pos += 1;
                    self.skip_group()?;
                    head = Some(TypeHead::Inferred);
                }
                "_Atomic" if free && self.peek_at(1) == Some("(") => {
                    self.pos += 2;
                    let inner = self.full_type()?;
                    if !self.eat(")") {
                        return Err(self.error("expected ')' after _Atomic type"));
                    }
                    head = Some(inner.head);
                    is_const |= inner.is_const;
                }
                "_Atomic" => self.pos += 1,
                _ if C_NON_TYPE_SPECIFIERS.contains(&t) => self.pos += 1,
                _ if head.is_none() && C_BUILTIN_WORDS.contains(&t) => {
                    builtin.push(t);
                    self.pos += 1;
                }
                _ if free && (t == "::" || is_name(self.lang, t)) => {
                    head = Some(self.qualified_name()?);
                }
                _ => break,
            }
        }
        let head = match head {
            Some(h) => h,
            None if !builtin.is_empty() => TypeHead::Builtin(builtin.join(" ")),
            None => return Err(self.error("expected a type")),
        };
        Ok(TypeExpr {
            head,
            is_const,
            is_volatile,
            declarators: Vec::new(),
        })
    }

    /// True when the tokens ahead spell `A::B::*` (a pointer-to-member declarator).
    fn is_member_pointer_ahead(&self) -> bool {
        let mut k = 0usize;
        loop {
            match self.peek_at(k) {
                Some(t) if is_name(self.lang, t) => {}
                _ => return false,
            }
            if self.peek_at(k + 1) != Some("::") {
                return false;
            }
            if self.peek_at(k + 2) == Some("*") {
                return true;
            }
            k += 2;
            if k > 64 {
                return false;
            }
        }
    }

    fn c_declarators(&mut self, ty: &mut TypeExpr) -> Result<(), TypeParseError> {
        loop {
            match self.peek() {
                Some("*") => {
                    self.pos += 1;
                    let mut is_const = false;
                    while let Some(q) = self.peek() {
                        if q == "const" {
                            is_const = true;
                            self.pos += 1;
                        } else if C_POINTER_QUALIFIERS.contains(&q) {
                            self.pos += 1;
                        } else {
                            break;
                        }
                    }
                    ty.declarators.push(Declarator::Pointer { is_const });
                }
                Some("^") if self.lang == CFamily::Cpp => {
                    // C++/CX and C++/CLI handle: a reference-counted pointer.
                    self.pos += 1;
                    ty.declarators.push(Declarator::Pointer { is_const: false });
                }
                Some("&") => {
                    self.pos += 1;
                    ty.declarators.push(Declarator::LValueRef);
                }
                Some("&&") => {
                    self.pos += 1;
                    ty.declarators.push(Declarator::RValueRef);
                }
                Some("const") => {
                    self.pos += 1;
                    if ty.declarators.is_empty() {
                        ty.is_const = true;
                    }
                }
                Some("volatile") => {
                    self.pos += 1;
                    if ty.declarators.is_empty() {
                        ty.is_volatile = true;
                    }
                }
                Some("...") => {
                    self.pos += 1;
                    ty.declarators.push(Declarator::PackExpansion);
                }
                Some("[") => {
                    let extent = self.array_extent()?;
                    ty.declarators.push(Declarator::Array(extent));
                }
                Some(t) if is_name(self.lang, t) && self.is_member_pointer_ahead() => {
                    let mut class = Vec::new();
                    while let Some(n) = self.peek() {
                        if n == "*" {
                            break;
                        }
                        if n != "::" {
                            class.push(n);
                        }
                        self.pos += 1;
                    }
                    self.pos += 1; // `*`
                    while matches!(self.peek(), Some("const") | Some("volatile")) {
                        self.pos += 1;
                    }
                    ty.declarators.push(Declarator::MemberPointer(class.join("::")));
                }
                _ => return Ok(()),
            }
        }
    }

    fn array_extent(&mut self) -> Result<ArrayExtent, TypeParseError> {
        if !self.eat("[") {
            return Err(self.error("expected '['"));
        }
        if self.eat("]") {
            return Ok(ArrayExtent::Unspecified);
        }
        let start = self.pos;
        let mut depth = 0i32;
        while let Some(t) = self.peek() {
            match t {
                "[" | "(" => depth += 1,
                ")" => depth -= 1,
                "]" if depth == 0 => break,
                "]" => depth -= 1,
                _ => {}
            }
            self.pos += 1;
        }
        let end = self.pos;
        if !self.eat("]") {
            return Err(self.error("unterminated array extent"));
        }
        let parts = &self.toks[start..end];
        if parts.len() == 1 {
            if let Some(n) = parse_int_literal(parts[0]) {
                return Ok(ArrayExtent::Literal(n));
            }
        }
        Ok(ArrayExtent::Symbolic(parts.join(" ")))
    }

    fn qualified_name(&mut self) -> Result<TypeHead, TypeParseError> {
        let mut global = false;
        if self.peek() == Some("::") {
            global = true;
            self.pos += 1;
        } else if self.lang == CFamily::CSharp
            && self.peek() == Some("global")
            && self.peek_at(1) == Some("::")
        {
            global = true;
            self.pos += 2;
        }
        let mut segments = Vec::new();
        loop {
            if self.lang == CFamily::Cpp && self.peek() == Some("template") {
                self.pos += 1;
            }
            let name = match self.peek() {
                Some(t) if is_name(self.lang, t) => t,
                _ => return Err(self.error("expected a type name")),
            };
            self.pos += 1;
            let args = if self.lang != CFamily::C && self.peek() == Some("<") {
                Some(self.template_args()?)
            } else {
                None
            };
            segments.push(NameSegment {
                name: plain_name(name).to_string(),
                args,
            });
            let is_sep = match self.peek() {
                Some("::") => true,
                Some(".") => self.lang == CFamily::CSharp,
                _ => false,
            };
            let continues = self
                .peek_at(1)
                .map(|t| is_name(self.lang, t) || t == "template")
                .unwrap_or(false);
            if is_sep && continues {
                self.pos += 1;
                continue;
            }
            break;
        }
        Ok(TypeHead::Named { global, segments })
    }

    fn template_args(&mut self) -> Result<Vec<TypeArg>, TypeParseError> {
        self.enter()?;
        let r = self.template_args_inner();
        self.leave();
        r
    }

    fn template_args_inner(&mut self) -> Result<Vec<TypeArg>, TypeParseError> {
        if !self.eat("<") {
            return Err(self.error("expected '<'"));
        }
        let mut args = Vec::new();
        if self.eat(">") {
            return Ok(args);
        }
        loop {
            if self.lang == CFamily::CSharp && matches!(self.peek(), Some(",") | Some(">")) {
                // Unbound generic type `Dictionary<,>`.
                args.push(TypeArg::Value(String::new()));
            } else {
                args.push(self.template_arg()?);
            }
            match self.advance() {
                Some(",") => continue,
                Some(">") => return Ok(args),
                _ => return Err(self.error("unterminated type argument list")),
            }
        }
    }

    fn starts_value(&self, t: &str) -> bool {
        let first = t.as_bytes().first().copied().unwrap_or(b' ');
        if first.is_ascii_digit() || first == b'"' || first == b'\'' {
            return true;
        }
        match t {
            "true" | "false" | "nullptr" | "-" | "+" | "!" | "~" | "&" | "sizeof" | "alignof"
            | "noexcept" => true,
            "(" => self.lang.is_c_like(),
            _ => false,
        }
    }

    fn template_arg(&mut self) -> Result<TypeArg, TypeParseError> {
        let save = self.pos;
        let first = match self.peek() {
            Some(t) => t,
            None => return Err(self.error("missing type argument")),
        };
        if !self.starts_value(first) {
            if let Ok(ty) = self.type_in_argument() {
                if matches!(self.peek(), Some(",") | Some(">")) {
                    return Ok(TypeArg::Type(ty));
                }
            }
            self.pos = save;
        }
        self.value_arg()
    }

    /// A type argument, including C++ function types `R(Args...)` and `R(*)(Args...)`.
    fn type_in_argument(&mut self) -> Result<TypeExpr, TypeParseError> {
        let ty = self.full_type()?;
        if self.lang.is_c_like() && self.peek() == Some("(") {
            return self.function_type(ty);
        }
        Ok(ty)
    }

    fn function_type(&mut self, ret: TypeExpr) -> Result<TypeExpr, TypeParseError> {
        let mut outer = Vec::new();
        if matches!(self.peek_at(1), Some("*") | Some("&") | Some("^")) {
            self.pos += 1;
            let mut holder = TypeExpr::from_head(TypeHead::Inferred);
            self.c_declarators(&mut holder)?;
            if !self.eat(")") {
                return Err(self.error("expected ')' in a function pointer type"));
            }
            outer = holder.declarators;
        }
        if !self.eat("(") {
            return Err(self.error("expected a parameter list"));
        }
        let mut params = Vec::new();
        if !self.eat(")") {
            loop {
                if !self.eat("...") {
                    let p = self.full_type()?;
                    if let Some(t) = self.peek() {
                        if is_name(self.lang, t) {
                            self.pos += 1;
                        }
                    }
                    while self.peek() == Some("[") {
                        self.array_extent()?;
                    }
                    let is_void = matches!(&p.head, TypeHead::Builtin(b) if b == "void")
                        && p.declarators.is_empty();
                    if !is_void {
                        params.push(p);
                    }
                }
                match self.advance() {
                    Some(",") => continue,
                    Some(")") => break,
                    _ => return Err(self.error("unterminated parameter list")),
                }
            }
        }
        while let Some(t) = self.peek() {
            match t {
                "const" | "volatile" | "&" | "&&" => self.pos += 1,
                "noexcept" => {
                    self.pos += 1;
                    if self.peek() == Some("(") {
                        self.skip_group()?;
                    }
                }
                _ => break,
            }
        }
        Ok(TypeExpr {
            head: TypeHead::Function {
                ret: Box::new(ret),
                params,
            },
            is_const: false,
            is_volatile: false,
            declarators: outer,
        })
    }

    fn value_arg(&mut self) -> Result<TypeArg, TypeParseError> {
        let start = self.pos;
        let mut depth = 0i32;
        while let Some(t) = self.peek() {
            match t {
                "(" | "[" | "{" => depth += 1,
                ")" | "]" | "}" => {
                    if depth == 0 {
                        break;
                    }
                    depth -= 1;
                }
                "," | ">" if depth == 0 => break,
                _ => {}
            }
            self.pos += 1;
        }
        if self.pos == start {
            return Err(self.error("empty type argument"));
        }
        Ok(TypeArg::Value(self.toks[start..self.pos].join(" ")))
    }

    fn cs_type(&mut self) -> Result<TypeExpr, TypeParseError> {
        while matches!(self.peek(), Some("ref") | Some("readonly") | Some("scoped")) {
            self.pos += 1;
        }
        let mut ty = match self.peek() {
            Some("(") => self.cs_tuple()?,
            Some("delegate") if self.peek_at(1) == Some("*") => self.cs_function_pointer()?,
            Some("var") => {
                self.pos += 1;
                TypeExpr::from_head(TypeHead::Inferred)
            }
            Some(t) if CS_BUILTIN_WORDS.contains(&t) => {
                self.pos += 1;
                TypeExpr::from_head(TypeHead::Builtin(t.to_string()))
            }
            Some(t) if t == "::" || is_name(CFamily::CSharp, t) => {
                let head = self.qualified_name()?;
                TypeExpr::from_head(head)
            }
            _ => return Err(self.error("expected a type")),
        };
        loop {
            match self.peek() {
                Some("?") => {
                    self.pos += 1;
                    ty.declarators.push(Declarator::Nullable);
                }
                Some("*") => {
                    self.pos += 1;
                    ty.declarators.push(Declarator::Pointer { is_const: false });
                }
                Some("[") => {
                    let mut k = 1usize;
                    let mut rank = 1u8;
                    loop {
                        match self.peek_at(k) {
                            Some(",") => {
                                rank = rank.saturating_add(1);
                                k += 1;
                            }
                            Some("]") => break,
                            // A fixed-size buffer or an indexer, not a rank specifier.
                            _ => return Ok(ty),
                        }
                    }
                    self.pos += k + 1;
                    ty.declarators.push(Declarator::CsArray { rank });
                }
                _ => return Ok(ty),
            }
        }
    }

    fn cs_tuple(&mut self) -> Result<TypeExpr, TypeParseError> {
        self.pos += 1;
        let mut elems = Vec::new();
        loop {
            let ty = self.full_type()?;
            let name = match self.peek() {
                Some(t) if is_name(CFamily::CSharp, t) => {
                    self.pos += 1;
                    Some(plain_name(t).to_string())
                }
                _ => None,
            };
            elems.push(TupleElement { ty, name });
            match self.advance() {
                Some(",") => continue,
                Some(")") => break,
                _ => return Err(self.error("unterminated tuple type")),
            }
        }
        if elems.len() < 2 {
            return Err(self.error("a tuple type needs at least two elements"));
        }
        Ok(TypeExpr::from_head(TypeHead::Tuple(elems)))
    }

    /// `delegate* [managed|unmanaged[...]] <P1, ..., R>`.
    fn cs_function_pointer(&mut self) -> Result<TypeExpr, TypeParseError> {
        self.pos += 2;
        if matches!(self.peek(), Some("managed") | Some("unmanaged")) {
            self.pos += 1;
            if self.peek() == Some("[") {
                while let Some(t) = self.advance() {
                    if t == "]" {
                        break;
                    }
                }
            }
        }
        let args = self.template_args()?;
        let mut types: Vec<TypeExpr> = args
            .into_iter()
            .filter_map(|a| match a {
                TypeArg::Type(t) => Some(t),
                TypeArg::Value(_) => None,
            })
            .collect();
        let ret = match types.pop() {
            Some(r) => r,
            None => return Err(self.error("function pointer type without a return type")),
        };
        Ok(TypeExpr {
            head: TypeHead::Function {
                ret: Box::new(ret),
                params: types,
            },
            is_const: false,
            is_volatile: false,
            declarators: vec![Declarator::Pointer { is_const: false }],
        })
    }
}

// ── Multiplicity ──────────────────────────────────────────────────────────────

/// A UML multiplicity `min..max` using the UMA bound markers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Multiplicity {
    pub min: u16,
    /// `MULTIPLICITY_MANY` for `*`, `MULTIPLICITY_UNKNOWN` when not derivable.
    pub max: u16,
}

impl Default for Multiplicity {
    fn default() -> Self {
        Multiplicity::UNKNOWN
    }
}

impl Multiplicity {
    pub const ONE: Multiplicity = Multiplicity { min: 1, max: 1 };
    pub const OPTIONAL: Multiplicity = Multiplicity { min: 0, max: 1 };
    pub const MANY: Multiplicity = Multiplicity {
        min: 0,
        max: MULTIPLICITY_MANY,
    };
    pub const UNKNOWN: Multiplicity = Multiplicity {
        min: MULTIPLICITY_UNKNOWN,
        max: MULTIPLICITY_UNKNOWN,
    };

    /// Exactly `n`; counts beyond the representable range become `?..*`.
    pub fn exactly(n: u64) -> Self {
        if n < u64::from(MULTIPLICITY_UNKNOWN) {
            let v = n as u16;
            Multiplicity { min: v, max: v }
        } else {
            Multiplicity {
                min: MULTIPLICITY_UNKNOWN,
                max: MULTIPLICITY_MANY,
            }
        }
    }

    /// Multiplicity of a part reached through `self` and then `inner`.
    pub fn times(self, inner: Multiplicity) -> Multiplicity {
        Multiplicity {
            min: mul_bound(self.min, inner.min),
            max: mul_bound(self.max, inner.max),
        }
    }

    pub fn is_many(self) -> bool {
        self.max == MULTIPLICITY_MANY
    }

    pub fn is_known(self) -> bool {
        self.min != MULTIPLICITY_UNKNOWN && self.max != MULTIPLICITY_UNKNOWN
    }

    /// UML label: `1`, `0..1`, `0..*`, `4`, `?..1` for an unknown bound.
    pub fn label(self) -> String {
        fn bound(v: u16) -> String {
            match v {
                MULTIPLICITY_MANY => "*".to_string(),
                MULTIPLICITY_UNKNOWN => "?".to_string(),
                n => n.to_string(),
            }
        }
        if self.min == self.max {
            bound(self.min)
        } else {
            format!("{}..{}", bound(self.min), bound(self.max))
        }
    }
}

fn mul_bound(a: u16, b: u16) -> u16 {
    if a == 0 || b == 0 {
        return 0;
    }
    if a == MULTIPLICITY_UNKNOWN || b == MULTIPLICITY_UNKNOWN {
        return MULTIPLICITY_UNKNOWN;
    }
    if a == MULTIPLICITY_MANY || b == MULTIPLICITY_MANY {
        return MULTIPLICITY_MANY;
    }
    let p = u32::from(a) * u32::from(b);
    if p >= u32::from(MULTIPLICITY_UNKNOWN) {
        MULTIPLICITY_MANY
    } else {
        p as u16
    }
}

// ── Type Classification ───────────────────────────────────────────────────────

/// Kind of classifier a type name denotes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TypeCategory {
    Class,
    Struct,
    Union,
    Interface,
    Enum,
    Delegate,
    TypeParameter,
    Builtin,
    Unknown,
}

/// Where a type comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TypeOrigin {
    /// Declared in the analysed sources.
    Project,
    /// A modelled standard-library, BCL or common third-party library type.
    Library,
    Builtin,
    /// A template or generic parameter in scope.
    TypeParameter,
    /// No binding could be computed (not declared in the project, not modelled, or
    /// ambiguous).
    Unknown,
}

/// The classifier a written type name binds to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TypeInfo {
    pub category: TypeCategory,
    pub origin: TypeOrigin,
    /// Index into `CFamilyUmlModel::classes` for project types.
    pub class_index: Option<usize>,
    /// Symbol id of a project type, `UNKNOWN_SYM` otherwise.
    pub sym_id: u32,
}

impl TypeInfo {
    pub const UNKNOWN: TypeInfo = TypeInfo {
        category: TypeCategory::Unknown,
        origin: TypeOrigin::Unknown,
        class_index: None,
        sym_id: UNKNOWN_SYM,
    };
    pub const BUILTIN: TypeInfo = TypeInfo {
        category: TypeCategory::Builtin,
        origin: TypeOrigin::Builtin,
        class_index: None,
        sym_id: UNKNOWN_SYM,
    };
    pub const TYPE_PARAMETER: TypeInfo = TypeInfo {
        category: TypeCategory::TypeParameter,
        origin: TypeOrigin::TypeParameter,
        class_index: None,
        sym_id: UNKNOWN_SYM,
    };

    pub fn library(category: TypeCategory) -> Self {
        TypeInfo {
            category,
            origin: TypeOrigin::Library,
            class_index: None,
            sym_id: UNKNOWN_SYM,
        }
    }

    pub fn project(category: TypeCategory, class_index: usize, sym_id: u32) -> Self {
        TypeInfo {
            category,
            origin: TypeOrigin::Project,
            class_index: Some(class_index),
            sym_id,
        }
    }

    pub fn is_project(&self) -> bool {
        self.origin == TypeOrigin::Project
    }
}

/// Ownership strength of one step of a member type (weakest first).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Ownership {
    /// The holder refers to the part (pointer, reference, weak pointer, view).
    Reference,
    /// The holder shares ownership (`std::shared_ptr`, COM pointers).
    Shared,
    /// The holder owns the part exclusively (value, `std::unique_ptr`, owning container).
    Exclusive,
}

fn min_own(current: Option<Ownership>, hop: Ownership) -> Option<Ownership> {
    Some(match current {
        Some(c) => c.min(hop),
        None => hop,
    })
}

/// How a library template relates its holder to the type arguments it carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HopEffect {
    /// Limits the ownership of the path to at most this strength.
    Owns(Ownership),
    /// Leaves ownership unchanged (`std::atomic`, `Lazy<T>`, .NET collections).
    Transparent,
    /// `gsl::owner<T*>`: the pointer argument owns its pointee.
    OwnsPointee,
}

/// Which type arguments of a library template are the elements it holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ElementArgs {
    First,
    Second,
    All,
}

/// What a modelled library type is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibraryKind {
    /// A template holding elements of (some of) its type arguments.
    Holder,
    /// A callable wrapper or delegate type (`std::function`, `Func<T>`).
    Callable,
    /// A type-erased or non-generic holder whose element type is not written.
    Opaque,
    /// A plain library type used as an attribute value.
    Value,
}

/// Model of one library type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LibrarySpec {
    pub kind: LibraryKind,
    pub category: TypeCategory,
    /// C#: the type is a value type (struct). Always false for C and C++.
    pub value_type: bool,
    /// The type holds a variable number of elements.
    pub collection: bool,
    effect: HopEffect,
    mult: Multiplicity,
    elements: ElementArgs,
    /// Argument that keys the elements (maps): a UML qualifier.
    key: Option<usize>,
    /// Argument that gives a fixed element count (`std::array<T, N>`).
    extent_arg: Option<usize>,
    /// The holder is never empty (`gsl::not_null<T*>`).
    non_null: bool,
}

impl LibrarySpec {
    fn holder(effect: HopEffect, mult: Multiplicity, elements: ElementArgs) -> Self {
        LibrarySpec {
            kind: LibraryKind::Holder,
            category: TypeCategory::Class,
            value_type: false,
            collection: false,
            effect,
            mult,
            elements,
            key: None,
            extent_arg: None,
            non_null: false,
        }
    }

    fn container(elements: ElementArgs) -> Self {
        let mut s = Self::holder(
            HopEffect::Owns(Ownership::Exclusive),
            Multiplicity::MANY,
            elements,
        );
        s.collection = true;
        s
    }

    fn map(key: usize, elements: ElementArgs) -> Self {
        let mut s = Self::container(elements);
        s.key = Some(key);
        s
    }

    fn callable() -> Self {
        let mut s = Self::value(TypeCategory::Delegate);
        s.kind = LibraryKind::Callable;
        s
    }

    fn opaque(collection: bool) -> Self {
        let mut s = Self::value(TypeCategory::Class);
        s.kind = LibraryKind::Opaque;
        s.collection = collection;
        s
    }

    fn value(category: TypeCategory) -> Self {
        LibrarySpec {
            kind: LibraryKind::Value,
            category,
            value_type: false,
            collection: false,
            effect: HopEffect::Transparent,
            mult: Multiplicity::ONE,
            elements: ElementArgs::First,
            key: None,
            extent_arg: None,
            non_null: false,
        }
    }

    fn cs_collection(name: &str, elements: ElementArgs) -> Self {
        let mut s = Self::holder(HopEffect::Transparent, Multiplicity::MANY, elements);
        s.collection = true;
        s.category = cs_category_of_name(name);
        s
    }

    fn with_value_type(mut self) -> Self {
        self.value_type = true;
        if self.category == TypeCategory::Class {
            self.category = TypeCategory::Struct;
        }
        self
    }
}

/// BCL naming convention: `IName` is an interface.
fn cs_category_of_name(name: &str) -> TypeCategory {
    if looks_like_cs_interface(name) {
        TypeCategory::Interface
    } else {
        TypeCategory::Class
    }
}

/// `I` followed by an upper-case letter (`IDisposable`, `IList`).
pub fn looks_like_cs_interface(name: &str) -> bool {
    let b = name.as_bytes();
    b.len() >= 2 && b[0] == b'I' && b[1].is_ascii_uppercase()
}

/// Root namespaces whose C++ types are library types.
const CPP_LIBRARY_ROOTS: &[&str] = &[
    "std", "boost", "gsl", "absl", "folly", "eastl", "tsl", "phmap", "robin_hood", "llvm",
    "fmt", "spdlog", "Eigen", "cv", "Microsoft", "winrt", "Concurrency", "concurrency", "ATL",
    "Platform", "Windows", "nlohmann", "google", "grpc", "tbb", "oneapi", "sycl", "thrust",
    "cuda", "Poco", "QtConcurrent", "ranges",
];

/// Inline, ABI and versioning namespaces that do not change a library type's identity.
fn is_abi_namespace(n: &str) -> bool {
    n.starts_with("__")
        || n == "_V2"
        || n == "pmr"
        || n == "experimental"
        || n == "fundamentals_v1"
        || n == "fundamentals_v2"
        || n == "inline_v1"
}

/// C, POSIX and Win32 typedefs that are plain values.
fn is_c_library_typedef(name: &str) -> bool {
    const NAMES: &[&str] = &[
        "size_t", "ssize_t", "ptrdiff_t", "intptr_t", "uintptr_t", "intmax_t", "uintmax_t",
        "max_align_t", "nullptr_t", "wint_t", "off_t", "pid_t", "time_t", "clock_t", "FILE",
        "fpos_t", "va_list", "errno_t", "sig_atomic_t", "mbstate_t", "rsize_t", "socklen_t",
        "BOOL", "BOOLEAN", "BYTE", "CHAR", "WCHAR", "TCHAR", "WORD", "DWORD", "DWORD32",
        "DWORD64", "DWORDLONG", "QWORD", "UINT", "UINT8", "UINT16", "UINT32", "UINT64", "INT",
        "INT8", "INT16", "INT32", "INT64", "LONG", "ULONG", "LONGLONG", "ULONGLONG", "SHORT",
        "USHORT", "FLOAT", "DOUBLE", "SIZE_T", "SSIZE_T", "LONG_PTR", "ULONG_PTR", "INT_PTR",
        "UINT_PTR", "DWORD_PTR", "HANDLE", "HMODULE", "HINSTANCE", "HWND", "HDC", "HKEY",
        "HRESULT", "NTSTATUS", "LPVOID", "LPCVOID", "PVOID", "LPSTR", "LPCSTR", "LPWSTR",
        "LPCWSTR", "LPTSTR", "LPCTSTR", "BSTR", "GUID", "LARGE_INTEGER", "ULARGE_INTEGER",
        "WPARAM", "LPARAM", "LRESULT", "ATOM", "COLORREF", "VARIANT",
    ];
    if NAMES.contains(&name) {
        return true;
    }
    // `int8_t`, `uint_least16_t`, `int_fast64_t`, ...
    let core = match name.strip_suffix("_t") {
        Some(c) => c,
        None => return false,
    };
    let core = core.strip_prefix('u').unwrap_or(core);
    let core = match core.strip_prefix("int") {
        Some(c) => c,
        None => return false,
    };
    let core = core
        .strip_prefix("_least")
        .or_else(|| core.strip_prefix("_fast"))
        .unwrap_or(core);
    matches!(core, "8" | "16" | "32" | "64" | "128")
}

/// Unqualified standard-library value types commonly used after `using namespace std`.
const CPP_STD_VALUE_NAMES: &[&str] = &[
    "string", "wstring", "u8string", "u16string", "u32string", "string_view", "wstring_view",
    "u8string_view", "mutex", "recursive_mutex", "shared_mutex", "timed_mutex",
    "condition_variable", "condition_variable_any", "thread", "jthread", "byte", "ostream",
    "istream", "iostream", "ofstream", "ifstream", "fstream", "stringstream", "ostringstream",
    "istringstream", "regex", "exception", "runtime_error", "logic_error", "error_code",
    "type_info", "type_index", "random_device", "mt19937", "mt19937_64", "once_flag",
    "stop_token", "stop_source", "bitset", "locale",
];

fn container_spec_with(mult: Multiplicity, effect: HopEffect) -> LibrarySpec {
    let mut s = LibrarySpec::holder(effect, mult, ElementArgs::First);
    s.collection = true;
    s
}

fn cpp_template_spec(name: &str) -> Option<LibrarySpec> {
    use ElementArgs::*;
    use Ownership::*;
    let spec = match name {
        "unique_ptr" | "auto_ptr" | "scoped_ptr" | "local_ptr" => {
            LibrarySpec::holder(HopEffect::Owns(Exclusive), Multiplicity::OPTIONAL, First)
        }
        "indirect" | "polymorphic" | "value_ptr" => {
            LibrarySpec::holder(HopEffect::Owns(Exclusive), Multiplicity::ONE, First)
        }
        "scoped_array" => container_spec_with(Multiplicity::MANY, HopEffect::Owns(Exclusive)),
        "shared_ptr" | "intrusive_ptr" | "local_shared_ptr" | "atomic_shared_ptr" => {
            LibrarySpec::holder(HopEffect::Owns(Shared), Multiplicity::OPTIONAL, First)
        }
        "shared_array" => container_spec_with(Multiplicity::MANY, HopEffect::Owns(Shared)),
        "weak_ptr" | "observer_ptr" | "atomic_weak_ptr" => {
            LibrarySpec::holder(HopEffect::Owns(Reference), Multiplicity::OPTIONAL, First)
        }
        "reference_wrapper" | "atomic_ref" => {
            LibrarySpec::holder(HopEffect::Owns(Reference), Multiplicity::ONE, First)
        }
        "not_null" | "strict_not_null" => {
            let mut s = LibrarySpec::holder(HopEffect::Transparent, Multiplicity::ONE, First);
            s.non_null = true;
            s
        }
        "owner" => LibrarySpec::holder(HopEffect::OwnsPointee, Multiplicity::ONE, First),
        "propagate_const" | "atomic" | "synchronized" | "Synchronized" => {
            LibrarySpec::holder(HopEffect::Transparent, Multiplicity::ONE, First)
        }
        "optional" | "expected" => {
            LibrarySpec::holder(HopEffect::Owns(Exclusive), Multiplicity::OPTIONAL, First)
        }
        "variant" => LibrarySpec::holder(HopEffect::Owns(Exclusive), Multiplicity::OPTIONAL, All),
        "pair" | "tuple" | "compressed_pair" => {
            LibrarySpec::holder(HopEffect::Owns(Exclusive), Multiplicity::ONE, All)
        }
        "function" | "move_only_function" | "copyable_function" | "function_ref"
        | "packaged_task" | "function_view" | "unique_function" => LibrarySpec::callable(),
        "any" => LibrarySpec::opaque(false),
        "vector" | "deque" | "list" | "forward_list" | "set" | "multiset" | "unordered_set"
        | "unordered_multiset" | "flat_set" | "flat_multiset" | "queue" | "stack"
        | "priority_queue" | "valarray" | "inplace_vector" | "hive" | "small_vector"
        | "static_vector" | "stable_vector" | "devector" | "circular_buffer" | "InlinedVector"
        | "FixedArray" | "flat_hash_set" | "node_hash_set" | "btree_set" | "btree_multiset"
        | "SmallVector" | "SmallPtrSet" | "DenseSet" | "SetVector" | "fbvector" | "F14FastSet"
        | "F14ValueSet" | "F14NodeSet" | "F14VectorSet" | "sorted_vector_set" | "ptr_vector"
        | "ptr_list" | "ptr_deque" | "ptr_set" | "ptr_multiset" | "concurrent_vector"
        | "concurrent_queue" | "concurrent_bounded_queue" | "robin_set" | "hopscotch_set"
        | "sparse_set" | "StringMap" => LibrarySpec::container(First),
        "map" | "multimap" | "unordered_map" | "unordered_multimap" | "flat_map"
        | "flat_multimap" | "flat_hash_map" | "node_hash_map" | "btree_map" | "btree_multimap"
        | "DenseMap" | "MapVector" | "F14FastMap" | "F14ValueMap" | "F14NodeMap"
        | "F14VectorMap" | "sorted_vector_map" | "robin_map" | "hopscotch_map" | "sparse_map"
        | "ptr_map" | "ptr_multimap" | "concurrent_hash_map" | "concurrent_unordered_map"
        | "bimap" => LibrarySpec::map(0, Second),
        "array" => {
            let mut s = LibrarySpec::container(First);
            s.extent_arg = Some(1);
            s.mult = Multiplicity::UNKNOWN;
            s
        }
        "span" | "mdspan" | "initializer_list" | "ArrayRef" | "MutableArrayRef" | "ref_view"
        | "array_view" => container_spec_with(Multiplicity::MANY, HopEffect::Owns(Reference)),
        "basic_string" | "basic_string_view" | "bitset" | "basic_ostream" | "basic_istream"
        | "duration" | "time_point" | "basic_regex" | "complex" => {
            LibrarySpec::value(TypeCategory::Class)
        }
        _ => return None,
    };
    Some(spec)
}

/// Library templates recognised in any namespace (Qt, ATL, WRL, WinRT, Chromium, LLVM).
fn cpp_distinctive_spec(name: &str) -> Option<LibrarySpec> {
    use ElementArgs::*;
    use Ownership::*;
    let spec = match name {
        "QScopedPointer" => {
            LibrarySpec::holder(HopEffect::Owns(Exclusive), Multiplicity::OPTIONAL, First)
        }
        "QScopedArrayPointer" => {
            container_spec_with(Multiplicity::MANY, HopEffect::Owns(Exclusive))
        }
        "QSharedPointer" | "QExplicitlySharedDataPointer" | "QSharedDataPointer" | "CComPtr"
        | "CComQIPtr" | "ComPtr" | "com_ptr" | "scoped_refptr" | "IntrusiveRefCntPtr" => {
            LibrarySpec::holder(HopEffect::Owns(Shared), Multiplicity::OPTIONAL, First)
        }
        "QWeakPointer" | "QPointer" | "raw_ptr" | "WeakRef" => {
            LibrarySpec::holder(HopEffect::Owns(Reference), Multiplicity::OPTIONAL, First)
        }
        "raw_ref" => LibrarySpec::holder(HopEffect::Owns(Reference), Multiplicity::ONE, First),
        "QList" | "QVector" | "QSet" | "QQueue" | "QStack" | "QLinkedList" | "QVarLengthArray"
        | "QContiguousCache" | "CAtlArray" | "CAtlList" => LibrarySpec::container(First),
        "QMap" | "QHash" | "QMultiMap" | "QMultiHash" | "CAtlMap" => LibrarySpec::map(0, Second),
        "QPair" => LibrarySpec::holder(HopEffect::Owns(Exclusive), Multiplicity::ONE, All),
        _ => return None,
    };
    Some(spec)
}

fn c_library_spec(ty: &TypeExpr) -> Option<LibrarySpec> {
    let segs = ty.segments();
    if segs.len() != 1 {
        return None;
    }
    if is_c_library_typedef(&segs[0].name) {
        Some(LibrarySpec::value(TypeCategory::Builtin))
    } else {
        None
    }
}

fn cpp_library_spec(ty: &TypeExpr) -> Option<LibrarySpec> {
    let segs = ty.segments();
    let last = segs.last()?;
    let name = last.name.as_str();
    if let Some(spec) = cpp_distinctive_spec(name) {
        return Some(spec);
    }
    let ns: Vec<&str> = segs[..segs.len() - 1]
        .iter()
        .map(|s| s.name.as_str())
        .filter(|n| !is_abi_namespace(n))
        .collect();
    let root = ns.first().copied();
    let in_library = match root {
        Some(r) => CPP_LIBRARY_ROOTS.contains(&r),
        None => true,
    };
    if !in_library {
        return None;
    }
    if let Some(spec) = cpp_template_spec(name) {
        return Some(spec);
    }
    if is_c_library_typedef(name) {
        return Some(LibrarySpec::value(TypeCategory::Builtin));
    }
    match root {
        // Any other `std::` / `boost::` / ... type is a library value.
        Some(_) => Some(LibrarySpec::value(TypeCategory::Class)),
        None if CPP_STD_VALUE_NAMES.contains(&name) => {
            Some(LibrarySpec::value(TypeCategory::Class))
        }
        None if name.len() > 1
            && name.as_bytes()[0] == b'Q'
            && name.as_bytes()[1].is_ascii_uppercase() =>
        {
            // Qt value classes (`QString`, `QVariant`, `QDateTime`, ...).
            Some(LibrarySpec::value(TypeCategory::Class))
        }
        None => None,
    }
}

fn cs_collection_spec(name: &str, arity: usize) -> Option<LibrarySpec> {
    use ElementArgs::*;
    let spec = match (name, arity) {
        (
            "List" | "IList" | "ICollection" | "IEnumerable" | "IReadOnlyList"
            | "IReadOnlyCollection" | "ISet" | "IReadOnlySet" | "HashSet" | "SortedSet"
            | "LinkedList" | "Queue" | "Stack" | "Collection" | "ReadOnlyCollection"
            | "ObservableCollection" | "ReadOnlyObservableCollection" | "BindingList"
            | "ConcurrentBag" | "ConcurrentQueue" | "ConcurrentStack" | "BlockingCollection"
            | "IProducerConsumerCollection" | "ImmutableList" | "ImmutableHashSet"
            | "ImmutableSortedSet" | "ImmutableQueue" | "ImmutableStack" | "IImmutableList"
            | "IImmutableSet" | "IImmutableQueue" | "IImmutableStack" | "FrozenSet"
            | "IAsyncEnumerable" | "IQueryable" | "IOrderedEnumerable" | "IOrderedQueryable"
            | "DbSet" | "IEnumerator" | "IAsyncEnumerator" | "Channel"
            | "SynchronizedCollection",
            1,
        ) => LibrarySpec::cs_collection(name, First),
        (
            "ImmutableArray" | "Span" | "ReadOnlySpan" | "Memory" | "ReadOnlyMemory"
            | "ArraySegment" | "ReadOnlySequence",
            1,
        ) => LibrarySpec::cs_collection(name, First).with_value_type(),
        ("PriorityQueue", 2) => LibrarySpec::cs_collection(name, First),
        (
            "Dictionary" | "IDictionary" | "IReadOnlyDictionary" | "SortedDictionary"
            | "SortedList" | "ConcurrentDictionary" | "ImmutableDictionary"
            | "ImmutableSortedDictionary" | "IImmutableDictionary" | "FrozenDictionary"
            | "ILookup" | "Lookup" | "KeyedCollection" | "OrderedDictionary" | "IGrouping",
            2,
        ) => {
            let mut s = LibrarySpec::cs_collection(name, Second);
            s.key = Some(0);
            s
        }
        ("ConditionalWeakTable", 2) => {
            let mut s = LibrarySpec::cs_collection(name, Second);
            s.effect = HopEffect::Owns(Ownership::Reference);
            s.key = Some(0);
            s
        }
        (
            "ArrayList" | "Hashtable" | "Queue" | "Stack" | "SortedList" | "IEnumerable"
            | "ICollection" | "IList" | "IDictionary" | "NameValueCollection"
            | "StringCollection" | "StringDictionary" | "HybridDictionary" | "ListDictionary"
            | "OrderedDictionary" | "CollectionBase" | "DictionaryBase" | "BitArray",
            0,
        ) => {
            let mut s = LibrarySpec::opaque(true);
            s.category = cs_category_of_name(name);
            s
        }
        _ => return None,
    };
    Some(spec)
}

fn cs_wrapper_spec(name: &str, arity: usize) -> Option<LibrarySpec> {
    use ElementArgs::*;
    let spec = match (name, arity) {
        ("Nullable", 1) => {
            LibrarySpec::holder(HopEffect::Transparent, Multiplicity::OPTIONAL, First)
                .with_value_type()
        }
        ("Lazy" | "ThreadLocal" | "AsyncLocal" | "StrongBox", 1) => {
            LibrarySpec::holder(HopEffect::Transparent, Multiplicity::ONE, First)
        }
        ("Task" | "TaskCompletionSource", 1) => {
            LibrarySpec::holder(HopEffect::Transparent, Multiplicity::OPTIONAL, First)
        }
        ("ValueTask", 1) => {
            LibrarySpec::holder(HopEffect::Transparent, Multiplicity::OPTIONAL, First)
                .with_value_type()
        }
        ("WeakReference", 1) => LibrarySpec::holder(
            HopEffect::Owns(Ownership::Reference),
            Multiplicity::OPTIONAL,
            First,
        ),
        ("KeyValuePair", 2) => {
            LibrarySpec::holder(HopEffect::Transparent, Multiplicity::ONE, All).with_value_type()
        }
        ("Tuple", 1..=8) => LibrarySpec::holder(HopEffect::Transparent, Multiplicity::ONE, All),
        ("ValueTuple", 1..=8) => {
            LibrarySpec::holder(HopEffect::Transparent, Multiplicity::ONE, All).with_value_type()
        }
        ("Action", 0..=16)
        | ("Func", 1..=17)
        | ("Predicate" | "Comparison" | "EventHandler", 1)
        | ("Converter", 2)
        | (
            "EventHandler" | "AsyncCallback" | "ThreadStart" | "ParameterizedThreadStart"
            | "TimerCallback" | "WaitCallback" | "PropertyChangedEventHandler"
            | "PropertyChangingEventHandler" | "NotifyCollectionChangedEventHandler"
            | "RoutedEventHandler" | "ElapsedEventHandler" | "Delegate" | "MulticastDelegate",
            0,
        ) => LibrarySpec::callable(),
        _ => return None,
    };
    Some(spec)
}

fn cs_value_spec(name: &str, arity: usize) -> Option<LibrarySpec> {
    let spec = match (name, arity) {
        (
            "IObservable" | "IObserver" | "IComparer" | "IEqualityComparer" | "IComparable"
            | "IEquatable" | "IProgress" | "IOptions" | "IOptionsMonitor" | "IOptionsSnapshot"
            | "ILogger" | "IParsable" | "ISpanParsable" | "IRequestHandler"
            | "INotificationHandler" | "IValidator" | "IConfigureOptions",
            1..=2,
        )
        | (
            "IDisposable" | "IAsyncDisposable" | "ICloneable" | "IComparable" | "IFormattable"
            | "IConvertible" | "INotifyPropertyChanged" | "INotifyPropertyChanging"
            | "INotifyCollectionChanged" | "INotifyDataErrorInfo" | "IServiceProvider"
            | "ISerializable" | "IAsyncResult" | "IHostedService" | "ILogger" | "ILoggerFactory"
            | "IConfiguration" | "ICommand" | "IValueConverter" | "IEqualityComparer"
            | "IComparer" | "IStructuralEquatable" | "IStructuralComparable"
            | "ISpanFormattable" | "IUtf8SpanFormattable" | "IDataErrorInfo" | "IEditableObject"
            | "IServiceScope" | "IServiceScopeFactory" | "IServiceCollection" | "IHost"
            | "IHostBuilder" | "IHostEnvironment" | "IWebHostEnvironment" | "IHttpClientFactory"
            | "IMediator" | "IEnumerator",
            0,
        ) => LibrarySpec::value(TypeCategory::Interface),
        (
            "DateTime" | "DateTimeOffset" | "TimeSpan" | "Guid" | "Decimal" | "DateOnly"
            | "TimeOnly" | "CancellationToken" | "Int16" | "Int32" | "Int64" | "UInt16"
            | "UInt32" | "UInt64" | "Byte" | "SByte" | "Char" | "Boolean" | "Single" | "Double"
            | "Half" | "Int128" | "UInt128" | "IntPtr" | "UIntPtr" | "BigInteger" | "Complex"
            | "Vector2" | "Vector3" | "Vector4" | "Matrix3x2" | "Matrix4x4" | "Quaternion"
            | "Plane" | "Color" | "Point" | "PointF" | "Size" | "SizeF" | "Rectangle"
            | "RectangleF" | "Index" | "Range" | "SpinLock" | "ValueTuple" | "Rune" | "HashCode"
            | "ConsoleKeyInfo" | "Thickness" | "CornerRadius" | "GridLength" | "ValueTask"
            | "RuntimeTypeHandle" | "TypedReference",
            0,
        ) => LibrarySpec::value(TypeCategory::Struct).with_value_type(),
        (
            "String" | "Object" | "Type" | "Uri" | "Version" | "StringBuilder" | "Regex"
            | "Stream" | "MemoryStream" | "FileStream" | "StreamReader" | "StreamWriter"
            | "TextReader" | "TextWriter" | "BinaryReader" | "BinaryWriter" | "HttpClient"
            | "HttpResponseMessage" | "HttpRequestMessage" | "Thread" | "Timer" | "Random"
            | "CancellationTokenSource" | "SemaphoreSlim" | "Mutex" | "Semaphore"
            | "ManualResetEvent" | "AutoResetEvent" | "ManualResetEventSlim"
            | "ReaderWriterLockSlim" | "Stopwatch" | "Encoding" | "CultureInfo" | "Assembly"
            | "Exception" | "Task" | "Array" | "Attribute" | "EventArgs" | "Lock"
            | "JsonSerializerOptions" | "DbContext" | "Process" | "FileInfo" | "DirectoryInfo"
            | "XmlDocument" | "XElement" | "XDocument" | "DataTable" | "DataSet"
            | "SqlConnection" | "SqlCommand" | "WeakReference" | "Monitor",
            0,
        ) => LibrarySpec::value(TypeCategory::Class),
        _ => return None,
    };
    Some(spec)
}

fn cs_library_spec(ty: &TypeExpr) -> Option<LibrarySpec> {
    let segs = ty.segments();
    let last = segs.last()?;
    let arity = last.args.as_ref().map(|a| a.len()).unwrap_or(0);
    let name = last.name.as_str();
    if let Some(spec) = cs_collection_spec(name, arity)
        .or_else(|| cs_wrapper_spec(name, arity))
        .or_else(|| cs_value_spec(name, arity))
    {
        return Some(spec);
    }
    let system_qualified = segs.len() > 1
        && matches!(
            segs[0].name.as_str(),
            "System" | "Microsoft" | "Windows" | "Android"
        );
    if system_qualified {
        return Some(LibrarySpec::value(cs_category_of_name(name)));
    }
    // Framework convention for delegate types not modelled individually.
    if arity <= 1 && name.ends_with("EventHandler") && name.len() > "EventHandler".len() {
        return Some(LibrarySpec::callable());
    }
    None
}

/// The library model entry for a named type in `lang`.
pub fn library_spec(lang: CFamily, ty: &TypeExpr) -> Option<LibrarySpec> {
    match lang {
        CFamily::C => c_library_spec(ty),
        CFamily::Cpp => cpp_library_spec(ty),
        CFamily::CSharp => cs_library_spec(ty),
    }
}

/// How a member holds what its type names, one level deep.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HolderKind {
    /// The type itself, held by value.
    Value,
    RawPointer,
    Reference,
    UniqueOwner,
    SharedOwner,
    WeakRef,
    Optional,
    /// C# `Lazy<T>` and other deferred holders.
    Lazy,
    /// C# `T?` / `Nullable<T>`.
    Nullable,
    Collection,
    Callable,
}

/// Peels one holder level off a declared type: `std::unique_ptr<T>` is
/// `(UniqueOwner, T)`, `T*` is `(RawPointer, T)`, `List<T>` is `(Collection, T)`.
/// Transparent wrappers (`gsl::not_null`, `std::propagate_const`) are looked through.
pub fn peel_holder(lang: CFamily, ty: &TypeExpr) -> (HolderKind, Option<TypeExpr>) {
    if let Some(d) = ty.declarators.last() {
        let kind = match d {
            Declarator::Pointer { .. } => HolderKind::RawPointer,
            Declarator::LValueRef | Declarator::RValueRef => HolderKind::Reference,
            Declarator::Nullable => HolderKind::Nullable,
            Declarator::Array(_) | Declarator::CsArray { .. } | Declarator::PackExpansion => {
                HolderKind::Collection
            }
            Declarator::MemberPointer(_) => return (HolderKind::Callable, None),
        };
        return (kind, Some(ty.peel_outermost()));
    }
    match &ty.head {
        TypeHead::Function { .. } => return (HolderKind::Callable, None),
        TypeHead::Named { .. } => {}
        _ => return (HolderKind::Value, Some(ty.clone())),
    }
    let name = ty.simple_name().unwrap_or("");
    let spec = match library_spec(lang, ty) {
        Some(s) => s,
        None => return (HolderKind::Value, Some(ty.clone())),
    };
    match spec.kind {
        LibraryKind::Callable => (HolderKind::Callable, None),
        LibraryKind::Opaque => (HolderKind::Collection, None),
        LibraryKind::Value => (HolderKind::Value, Some(ty.clone())),
        LibraryKind::Holder => {
            let first = match spec.elements {
                ElementArgs::Second => ty.type_arg(1).cloned(),
                _ => ty.type_arg(0).cloned(),
            };
            if spec.collection {
                return (HolderKind::Collection, first);
            }
            match spec.effect {
                HopEffect::Owns(Ownership::Exclusive) => {
                    if matches!(name, "optional" | "expected") {
                        (HolderKind::Optional, first)
                    } else if spec.elements == ElementArgs::All {
                        (HolderKind::Value, Some(ty.clone()))
                    } else {
                        (HolderKind::UniqueOwner, first)
                    }
                }
                HopEffect::Owns(Ownership::Shared) => (HolderKind::SharedOwner, first),
                HopEffect::Owns(Ownership::Reference) => {
                    if spec.mult == Multiplicity::ONE {
                        (HolderKind::Reference, first)
                    } else {
                        (HolderKind::WeakRef, first)
                    }
                }
                HopEffect::OwnsPointee => match first {
                    Some(inner) => match inner.declarators.last() {
                        Some(Declarator::Pointer { .. }) => {
                            (HolderKind::UniqueOwner, Some(inner.peel_outermost()))
                        }
                        _ => (HolderKind::Value, Some(inner)),
                    },
                    None => (HolderKind::Value, None),
                },
                HopEffect::Transparent => {
                    if lang == CFamily::CSharp && name == "Nullable" {
                        (HolderKind::Nullable, first)
                    } else if spec.elements == ElementArgs::All {
                        (HolderKind::Value, Some(ty.clone()))
                    } else if lang == CFamily::CSharp {
                        (HolderKind::Lazy, first)
                    } else {
                        match first {
                            Some(inner) => peel_holder(lang, &inner),
                            None => (HolderKind::Value, None),
                        }
                    }
                }
            }
        }
    }
}

/// Peels every holder level until a non-holder type remains
/// (`std::vector<std::unique_ptr<T>>` gives `T`); returns the kinds crossed, outermost
/// first, and the remaining type (`None` when a callable or opaque holder ends the walk).
pub fn innermost_type(lang: CFamily, ty: &TypeExpr) -> (Vec<HolderKind>, Option<TypeExpr>) {
    let mut kinds = Vec::new();
    let mut current = ty.clone();
    for _ in 0..MAX_TYPE_DEPTH {
        let (kind, inner) = peel_holder(lang, &current);
        if kind == HolderKind::Value {
            return (kinds, inner);
        }
        kinds.push(kind);
        match inner {
            Some(i) => current = i,
            None => return (kinds, None),
        }
    }
    (kinds, None)
}

// ── Member Relationships ──────────────────────────────────────────────────────

/// Binds written names to project types and aliases in one lexical context.
pub trait TypeResolver {
    fn language(&self) -> CFamily;
    /// Binds a named type to a project type or an in-scope type parameter;
    /// `TypeInfo::UNKNOWN` when neither applies (library types are classified by the
    /// caller from the library model).
    fn resolve_project(&self, ty: &TypeExpr) -> TypeInfo;
    /// The aliased type when `ty` names a project alias (`using X = ...`, `typedef`),
    /// with `ty`'s declarators applied on top.
    fn expand_alias(&self, ty: &TypeExpr) -> Option<TypeExpr>;
    /// Template / generic parameters of a project type.
    fn template_params(&self, class_index: usize) -> Option<&[TemplateParam]>;
}

/// Full binding of a written type: project type, type parameter, library, built-in, or
/// `TypeInfo::UNKNOWN`. Declarators are ignored; aliases are expanded.
pub fn resolve_type(resolver: &dyn TypeResolver, ty: &TypeExpr) -> TypeInfo {
    let mut current = ty.without_declarators();
    for _ in 0..MAX_ALIAS_EXPANSIONS {
        match resolver.expand_alias(&current) {
            Some(next) => {
                let next = next.without_declarators();
                if next == current {
                    break;
                }
                current = next;
            }
            None => break,
        }
    }
    match &current.head {
        TypeHead::Builtin(_) => TypeInfo::BUILTIN,
        TypeHead::Named { .. } => {
            let info = resolver.resolve_project(&current);
            if info.origin != TypeOrigin::Unknown {
                return info;
            }
            match library_spec(resolver.language(), &current) {
                Some(spec) => TypeInfo::library(spec.category),
                None => TypeInfo::UNKNOWN,
            }
        }
        TypeHead::Tuple(_) => TypeInfo::library(TypeCategory::Struct),
        TypeHead::Function { .. } => TypeInfo::library(TypeCategory::Delegate),
        TypeHead::Inferred => TypeInfo::UNKNOWN,
    }
}

/// What kind of member a type belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MemberRole {
    Field,
    Property,
    Event,
}

/// One UML relationship implied by a member's declared type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberRelation {
    /// ASSOC_* kind.
    pub assoc_kind: u8,
    /// The target classifier as written (holders and declarators peeled off).
    pub target: TypeExpr,
    pub info: TypeInfo,
    /// Multiplicity at the target end (`MULTIPLICITY_UNKNOWN` bounds for dependencies).
    pub mult: Multiplicity,
    pub ownership: Ownership,
    /// The target is reached through a collection, array or container.
    pub through_collection: bool,
    /// Map key type: a UML qualifier on the association end.
    pub qualifier: Option<TypeExpr>,
    /// Template binding when the target is a project template used with arguments.
    pub binding: Option<TemplateBinding>,
}

/// UML reading of a member's declared type.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MemberShape {
    /// Relations to project types: structural ones in source order, then «use»
    /// dependencies carried by callable signatures and event arguments.
    pub relations: Vec<MemberRelation>,
    /// Multiplicity of the attribute (of the primary relation when there is one).
    pub mult: Multiplicity,
    /// The type is an array, a container or a library collection.
    pub is_collection: bool,
    /// The type is (or holds) a callable: `std::function`, function pointer, delegate.
    pub is_callable: bool,
    /// The callable type as written.
    pub callable: Option<TypeExpr>,
    /// Leaf type names that could not be resolved.
    pub unresolved: Vec<String>,
    /// Extra stereotype of the member («event»).
    pub stereotype: Option<&'static str>,
}

impl MemberShape {
    /// The structural (non-dependency) relation that represents the attribute.
    pub fn primary(&self) -> Option<&MemberRelation> {
        self.relations
            .iter()
            .find(|r| r.assoc_kind != ASSOC_DEPENDENCY)
    }

    /// True when every leaf of the type was bound.
    pub fn fully_resolved(&self) -> bool {
        self.unresolved.is_empty()
    }
}

#[derive(Debug, Clone)]
struct HopCtx {
    mult: Multiplicity,
    ownership: Option<Ownership>,
    through_collection: bool,
    qualifier: Option<TypeExpr>,
    depth: u32,
    owns_pointee: bool,
    non_null: bool,
}

struct Leaf {
    ty: TypeExpr,
    info: TypeInfo,
    ctx: HopCtx,
}

struct Walk<'r> {
    lang: CFamily,
    resolver: &'r dyn TypeResolver,
    leaves: Vec<Leaf>,
    is_collection: bool,
    is_callable: bool,
    callable: Option<TypeExpr>,
    unresolved: Vec<String>,
    end_mult: Option<Multiplicity>,
}

impl<'r> Walk<'r> {
    fn note_mult(&mut self, m: Multiplicity) {
        if self.end_mult.is_none() {
            self.end_mult = Some(m);
        }
    }

    fn note_unresolved(&mut self, name: String) {
        if !self.unresolved.contains(&name) {
            self.unresolved.push(name);
        }
    }

    fn leaf(&mut self, ty: TypeExpr, info: TypeInfo, ctx: HopCtx) {
        self.note_mult(ctx.mult);
        self.leaves.push(Leaf { ty, info, ctx });
    }

    fn note_callable(&mut self, ty: &TypeExpr, mult: Multiplicity) {
        self.is_callable = true;
        if self.callable.is_none() {
            self.callable = Some(ty.clone());
        }
        self.note_mult(mult);
    }

    fn expand(&self, ty: &TypeExpr) -> Option<TypeExpr> {
        if !matches!(ty.head, TypeHead::Named { .. }) {
            return None;
        }
        let mut current = ty.clone();
        let mut changed = false;
        for _ in 0..MAX_ALIAS_EXPANSIONS {
            match self.resolver.expand_alias(&current) {
                Some(next) => {
                    if next == current {
                        break;
                    }
                    let stop = !matches!(next.head, TypeHead::Named { .. });
                    current = next;
                    changed = true;
                    if stop {
                        break;
                    }
                }
                None => break,
            }
        }
        if changed {
            Some(current)
        } else {
            None
        }
    }

    fn walk(&mut self, ty: &TypeExpr, ctx: HopCtx) {
        if ctx.depth > MAX_TYPE_DEPTH {
            self.note_unresolved(ty.display(self.lang));
            self.note_mult(Multiplicity::UNKNOWN);
            return;
        }
        let expanded = self.expand(ty);
        let ty = match &expanded {
            Some(e) => e,
            None => ty,
        };
        let mut ctx = ctx;
        for d in ty.declarators.iter().rev() {
            match d {
                Declarator::Pointer { .. } => {
                    if ctx.owns_pointee {
                        ctx.owns_pointee = false;
                        ctx.ownership = min_own(ctx.ownership, Ownership::Exclusive);
                    } else {
                        ctx.ownership = min_own(ctx.ownership, Ownership::Reference);
                    }
                    let hop = if ctx.non_null {
                        ctx.non_null = false;
                        Multiplicity::ONE
                    } else {
                        Multiplicity::OPTIONAL
                    };
                    ctx.mult = ctx.mult.times(hop);
                }
                Declarator::LValueRef | Declarator::RValueRef => {
                    ctx.ownership = min_own(ctx.ownership, Ownership::Reference);
                }
                Declarator::Array(extent) => {
                    self.is_collection = true;
                    ctx.through_collection = true;
                    let m = match extent {
                        ArrayExtent::Literal(n) => Multiplicity::exactly(*n),
                        ArrayExtent::Symbolic(_) => Multiplicity::UNKNOWN,
                        ArrayExtent::Unspecified => Multiplicity::MANY,
                    };
                    ctx.mult = ctx.mult.times(m);
                }
                Declarator::CsArray { .. } | Declarator::PackExpansion => {
                    self.is_collection = true;
                    ctx.through_collection = true;
                    ctx.mult = ctx.mult.times(Multiplicity::MANY);
                }
                Declarator::Nullable => {
                    ctx.mult = ctx.mult.times(Multiplicity::OPTIONAL);
                }
                Declarator::MemberPointer(_) => {
                    // A pointer to member designates a member, not an object.
                    self.note_mult(ctx.mult.times(Multiplicity::OPTIONAL));
                    return;
                }
            }
        }
        let head_ty = ty.without_declarators();
        match &ty.head {
            TypeHead::Builtin(_) => self.leaf(head_ty, TypeInfo::BUILTIN, ctx),
            TypeHead::Inferred => self.leaf(head_ty, TypeInfo::UNKNOWN, ctx),
            TypeHead::Tuple(elems) => {
                for e in elems {
                    let mut c = ctx.clone();
                    c.depth += 1;
                    self.walk(&e.ty, c);
                }
                self.note_mult(ctx.mult);
            }
            TypeHead::Function { .. } => self.note_callable(ty, ctx.mult),
            TypeHead::Named { .. } => self.walk_named(head_ty, ctx),
        }
    }

    fn walk_named(&mut self, head_ty: TypeExpr, ctx: HopCtx) {
        let info = self.resolver.resolve_project(&head_ty);
        if matches!(info.origin, TypeOrigin::Project | TypeOrigin::TypeParameter) {
            self.leaf(head_ty, info, ctx);
            return;
        }
        let spec = match library_spec(self.lang, &head_ty) {
            Some(s) => s,
            None => {
                self.leaf(head_ty, TypeInfo::UNKNOWN, ctx);
                return;
            }
        };
        match spec.kind {
            LibraryKind::Value => self.leaf(head_ty, TypeInfo::library(spec.category), ctx),
            LibraryKind::Callable => self.note_callable(&head_ty, ctx.mult),
            LibraryKind::Opaque => {
                let m = if spec.collection {
                    self.is_collection = true;
                    ctx.mult.times(Multiplicity::MANY)
                } else {
                    ctx.mult
                };
                self.note_mult(m);
            }
            LibraryKind::Holder => self.walk_holder(&head_ty, &spec, ctx),
        }
    }

    fn walk_holder(&mut self, ty: &TypeExpr, spec: &LibrarySpec, mut ctx: HopCtx) {
        let args = ty.last_args();
        match spec.effect {
            HopEffect::Owns(o) => ctx.ownership = min_own(ctx.ownership, o),
            HopEffect::Transparent => {}
            HopEffect::OwnsPointee => ctx.owns_pointee = true,
        }
        if spec.non_null {
            ctx.non_null = true;
        }
        let mut mult = spec.mult;
        if let Some(k) = spec.extent_arg {
            mult = match args.get(k) {
                Some(TypeArg::Value(v)) => parse_int_literal(v)
                    .map(Multiplicity::exactly)
                    .unwrap_or(Multiplicity::UNKNOWN),
                _ => Multiplicity::UNKNOWN,
            };
        }
        let mut collection = spec.collection;
        // `std::unique_ptr<T[]>` owns an array of T.
        let mut first_override: Option<TypeExpr> = None;
        if self.lang.is_c_like() && !collection && matches!(spec.effect, HopEffect::Owns(_)) {
            if let Some(TypeArg::Type(inner)) = args.first() {
                if let Some(Declarator::Array(_)) = inner.declarators.last() {
                    collection = true;
                    mult = Multiplicity::MANY;
                    first_override = Some(inner.peel_outermost());
                }
            }
        }
        if collection {
            self.is_collection = true;
            ctx.through_collection = true;
        }
        ctx.mult = ctx.mult.times(mult);
        ctx.depth += 1;
        let key = spec.key.and_then(|k| match args.get(k) {
            Some(TypeArg::Type(t)) => Some(t.clone()),
            _ => None,
        });
        let positions: Vec<usize> = match spec.elements {
            ElementArgs::First => vec![0],
            ElementArgs::Second => vec![1],
            ElementArgs::All => (0..args.len()).collect(),
        };
        let mut walked = false;
        for k in positions {
            let arg = if k == 0 && first_override.is_some() {
                first_override.as_ref()
            } else {
                match args.get(k) {
                    Some(TypeArg::Type(t)) => Some(t),
                    _ => None,
                }
            };
            if let Some(t) = arg {
                let mut c = ctx.clone();
                if key.is_some() {
                    c.qualifier = key.clone();
                }
                self.walk(t, c);
                walked = true;
            }
        }
        if !walked {
            self.note_mult(ctx.mult);
        }
    }
}

/// C#: whether the declared top-level type is a reference type without a `?`
/// annotation (`Some(true)`), a value type or annotated (`Some(false)`), or unknown.
fn cs_top_level_reference(resolver: &dyn TypeResolver, ty: &TypeExpr) -> Option<bool> {
    match ty.declarators.last() {
        Some(Declarator::Nullable) | Some(Declarator::Pointer { .. }) => return Some(false),
        Some(Declarator::CsArray { .. }) => return Some(true),
        Some(_) => return None,
        None => {}
    }
    match &ty.head {
        TypeHead::Builtin(b) => Some(matches!(b.as_str(), "string" | "object" | "dynamic")),
        TypeHead::Tuple(_) | TypeHead::Function { .. } => Some(false),
        TypeHead::Inferred => None,
        TypeHead::Named { .. } => {
            let info = resolver.resolve_project(ty);
            match info.origin {
                TypeOrigin::Project => Some(!matches!(
                    info.category,
                    TypeCategory::Struct | TypeCategory::Enum | TypeCategory::Union
                )),
                TypeOrigin::TypeParameter => None,
                _ => library_spec(CFamily::CSharp, ty).map(|s| !s.value_type),
            }
        }
    }
}

fn relation_kind(lang: CFamily, leaf: &Leaf) -> (u8, Ownership) {
    let cat = leaf.info.category;
    if lang == CFamily::CSharp {
        if leaf.ctx.ownership == Some(Ownership::Reference) {
            return (ASSOC_ASSOCIATION, Ownership::Reference);
        }
        return match cat {
            TypeCategory::Struct | TypeCategory::Union => {
                (ASSOC_COMPOSITION, Ownership::Exclusive)
            }
            _ if leaf.ctx.through_collection => (ASSOC_AGGREGATION, Ownership::Shared),
            _ => (ASSOC_ASSOCIATION, Ownership::Reference),
        };
    }
    let leaf_strength = match cat {
        TypeCategory::Enum | TypeCategory::Delegate => Ownership::Reference,
        _ => Ownership::Exclusive,
    };
    let own = match leaf.ctx.ownership {
        Some(o) => o.min(leaf_strength),
        None => leaf_strength,
    };
    let kind = match own {
        Ownership::Exclusive => ASSOC_COMPOSITION,
        Ownership::Shared => ASSOC_AGGREGATION,
        Ownership::Reference => ASSOC_ASSOCIATION,
    };
    (kind, own)
}

fn binding_for(
    resolver: &dyn TypeResolver,
    lang: CFamily,
    target: &TypeExpr,
    info: &TypeInfo,
) -> Option<TemplateBinding> {
    let ci = info.class_index?;
    let seg = target.segments().last()?;
    let args = seg.args.as_ref()?;
    let params = resolver.template_params(ci).unwrap_or(&[]);
    Some(TemplateBinding::bind(
        &seg.name,
        Some(ci),
        info.sym_id,
        params,
        args,
        lang,
    ))
}

/// «use» dependencies on project types named inside `ty`.
fn dependency_relations(
    resolver: &dyn TypeResolver,
    ty: &TypeExpr,
    include_self: bool,
) -> Vec<MemberRelation> {
    let mut named = Vec::new();
    collect_named(ty, include_self, &mut named);
    let mut out: Vec<MemberRelation> = Vec::new();
    for n in named {
        let info = resolve_type(resolver, n);
        if !info.is_project() {
            continue;
        }
        if out.iter().any(|r| r.info.class_index == info.class_index) {
            continue;
        }
        out.push(MemberRelation {
            assoc_kind: ASSOC_DEPENDENCY,
            target: n.without_declarators(),
            info,
            mult: Multiplicity::UNKNOWN,
            ownership: Ownership::Reference,
            through_collection: false,
            qualifier: None,
            binding: None,
        });
    }
    out
}

/// Classifies a member's declared type into UML relationships (see the module rules).
/// `nullable` is the C# annotation context at the declaration (ignored for C and C++).
pub fn classify_member_type(
    resolver: &dyn TypeResolver,
    ty: &TypeExpr,
    role: MemberRole,
    nullable: NullableContext,
) -> MemberShape {
    let lang = resolver.language();
    if role == MemberRole::Event {
        return classify_event(resolver, ty);
    }
    let mut walk = Walk {
        lang,
        resolver,
        leaves: Vec::new(),
        is_collection: false,
        is_callable: false,
        callable: None,
        unresolved: Vec::new(),
        end_mult: None,
    };
    let ctx = HopCtx {
        mult: Multiplicity::ONE,
        ownership: None,
        through_collection: false,
        qualifier: None,
        depth: 0,
        owns_pointee: false,
        non_null: false,
    };
    walk.walk(ty, ctx);

    let mut shape = MemberShape {
        relations: Vec::new(),
        mult: walk.end_mult.unwrap_or(Multiplicity::UNKNOWN),
        is_collection: walk.is_collection,
        is_callable: walk.is_callable,
        callable: walk.callable.clone(),
        unresolved: walk.unresolved.clone(),
        stereotype: None,
    };
    let mut primary_mult: Option<Multiplicity> = None;
    for leaf in &walk.leaves {
        match leaf.info.origin {
            TypeOrigin::Project => {}
            TypeOrigin::Unknown => {
                let n = leaf.ty.display(lang);
                if !shape.unresolved.contains(&n) {
                    shape.unresolved.push(n);
                }
                continue;
            }
            _ => continue,
        }
        let (assoc_kind, ownership) = relation_kind(lang, leaf);
        if primary_mult.is_none() {
            primary_mult = Some(leaf.ctx.mult);
        }
        shape.relations.push(MemberRelation {
            assoc_kind,
            target: leaf.ty.clone(),
            info: leaf.info,
            mult: leaf.ctx.mult,
            ownership,
            through_collection: leaf.ctx.through_collection,
            qualifier: leaf.ctx.qualifier.clone(),
            binding: binding_for(resolver, lang, &leaf.ty, &leaf.info),
        });
    }
    if let Some(m) = primary_mult {
        shape.mult = m;
    }
    if let Some(callable) = &walk.callable {
        for dep in dependency_relations(resolver, callable, true) {
            if !shape
                .relations
                .iter()
                .any(|r| r.info.class_index == dep.info.class_index)
            {
                shape.relations.push(dep);
            }
        }
    }
    if lang == CFamily::CSharp {
        let top = cs_top_level_reference(resolver, ty);
        let adjust = |m: Multiplicity| -> Multiplicity {
            if m.min == 0 || m.min == MULTIPLICITY_UNKNOWN {
                return m;
            }
            match (top, nullable) {
                (Some(false), _) | (_, NullableContext::Enabled) => m,
                (Some(true), NullableContext::Disabled) => Multiplicity { min: 0, max: m.max },
                _ => Multiplicity {
                    min: MULTIPLICITY_UNKNOWN,
                    max: m.max,
                },
            }
        };
        shape.mult = adjust(shape.mult);
        for r in shape.relations.iter_mut() {
            if r.assoc_kind != ASSOC_DEPENDENCY {
                r.mult = adjust(r.mult);
            }
        }
    }
    shape
}

/// C# event: an association to the delegate type (multiplicity `0..*`, the invocation
/// list) plus «use» dependencies on project types among the delegate's arguments.
fn classify_event(resolver: &dyn TypeResolver, ty: &TypeExpr) -> MemberShape {
    let lang = resolver.language();
    let mut shape = MemberShape {
        mult: Multiplicity::MANY,
        is_callable: true,
        callable: Some(ty.clone()),
        stereotype: Some("event"),
        ..Default::default()
    };
    let info = resolve_type(resolver, ty);
    match info.origin {
        TypeOrigin::Project => shape.relations.push(MemberRelation {
            assoc_kind: ASSOC_ASSOCIATION,
            target: ty.without_declarators(),
            info,
            mult: Multiplicity::MANY,
            ownership: Ownership::Reference,
            through_collection: false,
            qualifier: None,
            binding: binding_for(resolver, lang, ty, &info),
        }),
        TypeOrigin::Unknown => shape.unresolved.push(ty.display(lang)),
        _ => {}
    }
    for dep in dependency_relations(resolver, ty, false) {
        if !shape
            .relations
            .iter()
            .any(|r| r.info.class_index == dep.info.class_index)
        {
            shape.relations.push(dep);
        }
    }
    shape
}

// ── Visibility ────────────────────────────────────────────────────────────────

/// Declared access of a C-family member or type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AccessLevel {
    Public,
    Protected,
    Private,
    /// C# `internal`: visible within the assembly.
    Internal,
    /// C# `protected internal`: derived types or the same assembly.
    ProtectedInternal,
    /// C# `private protected`: derived types within the same assembly.
    PrivateProtected,
    /// C# 11 `file`: visible within the source file.
    File,
}

/// UML visibility kinds (`+`, `-`, `#`, `~`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UmlVisibility {
    Public,
    Private,
    Protected,
    Package,
}

impl UmlVisibility {
    pub fn symbol(self) -> char {
        match self {
            UmlVisibility::Public => '+',
            UmlVisibility::Private => '-',
            UmlVisibility::Protected => '#',
            UmlVisibility::Package => '~',
        }
    }

    pub fn to_symbol_visibility(self) -> SymbolVisibility {
        match self {
            UmlVisibility::Public => SymbolVisibility::Public,
            UmlVisibility::Private => SymbolVisibility::Private,
            UmlVisibility::Protected => SymbolVisibility::Protected,
            UmlVisibility::Package => SymbolVisibility::Package,
        }
    }
}

impl AccessLevel {
    /// UML visibility: `internal` is package (`~`); the two C# combined levels are shown
    /// as protected with their exact level in `property()`; `file` is private.
    pub fn uml(self) -> UmlVisibility {
        match self {
            AccessLevel::Public => UmlVisibility::Public,
            AccessLevel::Protected
            | AccessLevel::ProtectedInternal
            | AccessLevel::PrivateProtected => UmlVisibility::Protected,
            AccessLevel::Private | AccessLevel::File => UmlVisibility::Private,
            AccessLevel::Internal => UmlVisibility::Package,
        }
    }

    pub fn uml_symbol(self) -> char {
        self.uml().symbol()
    }

    /// UML property string for levels that the four visibility kinds cannot express.
    pub fn property(self) -> Option<&'static str> {
        match self {
            AccessLevel::ProtectedInternal => Some("{protected internal}"),
            AccessLevel::PrivateProtected => Some("{private protected}"),
            AccessLevel::File => Some("{file}"),
            _ => None,
        }
    }

    pub fn keyword(self) -> &'static str {
        match self {
            AccessLevel::Public => "public",
            AccessLevel::Protected => "protected",
            AccessLevel::Private => "private",
            AccessLevel::Internal => "internal",
            AccessLevel::ProtectedInternal => "protected internal",
            AccessLevel::PrivateProtected => "private protected",
            AccessLevel::File => "file",
        }
    }

    /// Combines C# access keywords (`protected internal`, `private protected`).
    pub fn from_keywords(words: &[&str]) -> Option<AccessLevel> {
        let has = |w: &str| words.contains(&w);
        if words.is_empty() {
            return None;
        }
        Some(if has("protected") && has("internal") {
            AccessLevel::ProtectedInternal
        } else if has("private") && has("protected") {
            AccessLevel::PrivateProtected
        } else if has("public") {
            AccessLevel::Public
        } else if has("protected") {
            AccessLevel::Protected
        } else if has("internal") {
            AccessLevel::Internal
        } else if has("file") {
            AccessLevel::File
        } else if has("private") {
            AccessLevel::Private
        } else {
            return None;
        })
    }
}

/// Default access of a member declared in a `container` without an access keyword.
pub fn default_member_access(lang: CFamily, container: TypeDeclKind) -> AccessLevel {
    match lang {
        CFamily::C => AccessLevel::Public,
        CFamily::Cpp => match container {
            TypeDeclKind::Class => AccessLevel::Private,
            _ => AccessLevel::Public,
        },
        CFamily::CSharp => match container {
            TypeDeclKind::Interface | TypeDeclKind::Enum => AccessLevel::Public,
            _ => AccessLevel::Private,
        },
    }
}

/// Default access of a type declaration (`nested_in` is the enclosing type's kind).
pub fn default_type_access(lang: CFamily, nested_in: Option<TypeDeclKind>) -> AccessLevel {
    match (lang, nested_in) {
        (CFamily::CSharp, None) => AccessLevel::Internal,
        (_, Some(kind)) => default_member_access(lang, kind),
        _ => AccessLevel::Public,
    }
}

/// Default access of a C++ base specifier without an access keyword.
pub fn default_base_access(lang: CFamily, derived: TypeDeclKind) -> AccessLevel {
    match (lang, derived) {
        (CFamily::Cpp, TypeDeclKind::Class) => AccessLevel::Private,
        _ => AccessLevel::Public,
    }
}

// ── Member Flags ──────────────────────────────────────────────────────────────

/// C-family declaration flags (`u32` bitset; `CFamilyModifiers::to_symbol_modifiers`
/// maps the subset the symbol table models).
pub struct CFamilyModifiers;

impl CFamilyModifiers {
    pub const STATIC: u32 = 1 << 0;
    pub const ABSTRACT: u32 = 1 << 1;
    pub const VIRTUAL: u32 = 1 << 2;
    pub const OVERRIDE: u32 = 1 << 3;
    /// C++ `final`, C# `sealed`: cannot be overridden or derived from (UML `{leaf}`).
    pub const FINAL: u32 = 1 << 4;
    /// C++ const-qualified data member, C# `const`.
    pub const CONST: u32 = 1 << 5;
    /// C# `readonly` field or struct.
    pub const READONLY: u32 = 1 << 6;
    /// C++ `const` member function, C# `readonly` struct member (UML `{query}`).
    pub const CONST_METHOD: u32 = 1 << 7;
    pub const CONSTEXPR: u32 = 1 << 8;
    pub const INLINE: u32 = 1 << 9;
    pub const EXPLICIT: u32 = 1 << 10;
    /// `= delete`.
    pub const DELETED: u32 = 1 << 11;
    /// `= default`.
    pub const DEFAULTED: u32 = 1 << 12;
    /// C++ pure virtual `= 0`.
    pub const PURE: u32 = 1 << 13;
    pub const MUTABLE: u32 = 1 << 14;
    pub const VOLATILE: u32 = 1 << 15;
    pub const EXTERN: u32 = 1 << 16;
    pub const ASYNC: u32 = 1 << 17;
    /// C# `new` member modifier (hides an inherited member).
    pub const NEW_SLOT: u32 = 1 << 18;
    pub const PARTIAL: u32 = 1 << 19;
    pub const UNSAFE: u32 = 1 << 20;
    pub const REQUIRED: u32 = 1 << 21;
    pub const NOEXCEPT: u32 = 1 << 22;
    pub const FRIEND: u32 = 1 << 23;
    pub const THREAD_LOCAL: u32 = 1 << 24;
    /// C# `implicit` conversion operator.
    pub const IMPLICIT: u32 = 1 << 25;
    /// C# `ref` struct, field or return.
    pub const REF: u32 = 1 << 26;
    pub const CONSTEVAL: u32 = 1 << 27;
    pub const VARIADIC: u32 = 1 << 28;
    /// C# `fixed` size buffer.
    pub const FIXED: u32 = 1 << 29;

    pub fn has(bits: u32, flag: u32) -> bool {
        bits & flag != 0
    }

    /// Flag of a leading declaration keyword in `lang` (0 for anything else). C++ `const`
    /// and `volatile` belong to the type and are not handled here.
    pub fn from_keyword(lang: CFamily, word: &str) -> u32 {
        match lang {
            CFamily::CSharp => match word {
                "static" => Self::STATIC,
                "abstract" => Self::ABSTRACT,
                "virtual" => Self::VIRTUAL,
                "override" => Self::OVERRIDE,
                "sealed" => Self::FINAL,
                "const" => Self::CONST | Self::STATIC,
                "readonly" => Self::READONLY,
                "volatile" => Self::VOLATILE,
                "extern" => Self::EXTERN,
                "async" => Self::ASYNC,
                "new" => Self::NEW_SLOT,
                "partial" => Self::PARTIAL,
                "unsafe" => Self::UNSAFE,
                "required" => Self::REQUIRED,
                "implicit" => Self::IMPLICIT,
                "explicit" => Self::EXPLICIT,
                "ref" => Self::REF,
                "fixed" => Self::FIXED,
                _ => 0,
            },
            _ => match word {
                "static" => Self::STATIC,
                "virtual" => Self::VIRTUAL,
                "inline" | "__inline" | "__inline__" | "__forceinline" => Self::INLINE,
                "explicit" => Self::EXPLICIT,
                "constexpr" => Self::CONSTEXPR,
                "consteval" => Self::CONSTEVAL,
                "constinit" => Self::CONSTEXPR,
                "mutable" => Self::MUTABLE,
                "extern" => Self::EXTERN,
                "thread_local" | "_Thread_local" | "__thread" => Self::THREAD_LOCAL,
                "friend" => Self::FRIEND,
                "register" | "_Noreturn" => 0,
                _ => 0,
            },
        }
    }

    /// Maps to `SymbolModifiers` (static, final, abstract, volatile, native, varargs).
    pub fn to_symbol_modifiers(bits: u32) -> u16 {
        let mut m = 0u16;
        if Self::has(bits, Self::STATIC) {
            m |= SymbolModifiers::STATIC;
        }
        if Self::has(bits, Self::FINAL | Self::CONST | Self::READONLY) {
            m |= SymbolModifiers::FINAL;
        }
        if Self::has(bits, Self::ABSTRACT | Self::PURE) {
            m |= SymbolModifiers::ABSTRACT;
        }
        if Self::has(bits, Self::VOLATILE) {
            m |= SymbolModifiers::VOLATILE;
        }
        if Self::has(bits, Self::EXTERN) {
            m |= SymbolModifiers::NATIVE;
        }
        if Self::has(bits, Self::VARIADIC) {
            m |= SymbolModifiers::VARARGS;
        }
        m
    }

    /// UML property strings of a feature (`{readOnly}`, `{query}`, `{leaf}`, ...). Static
    /// (underlined) and abstract (italic) are rendering styles, not property strings.
    pub fn uml_properties(bits: u32) -> Vec<&'static str> {
        let mut out = Vec::new();
        if Self::has(bits, Self::CONST | Self::READONLY) {
            out.push("readOnly");
        }
        if Self::has(bits, Self::CONST_METHOD) {
            out.push("query");
        }
        if Self::has(bits, Self::FINAL) {
            out.push("leaf");
        }
        if Self::has(bits, Self::VIRTUAL) && !Self::has(bits, Self::PURE | Self::ABSTRACT) {
            out.push("virtual");
        }
        if Self::has(bits, Self::OVERRIDE) {
            out.push("redefines");
        }
        if Self::has(bits, Self::CONSTEXPR | Self::CONSTEVAL) {
            out.push("constexpr");
        }
        if Self::has(bits, Self::NOEXCEPT) {
            out.push("noexcept");
        }
        if Self::has(bits, Self::ASYNC) {
            out.push("async");
        }
        if Self::has(bits, Self::DELETED) {
            out.push("deleted");
        }
        if Self::has(bits, Self::DEFAULTED) {
            out.push("defaulted");
        }
        out
    }

    /// Underlined in UML.
    pub fn is_static(bits: u32) -> bool {
        Self::has(bits, Self::STATIC)
    }

    /// Italic in UML.
    pub fn is_abstract(bits: u32) -> bool {
        Self::has(bits, Self::ABSTRACT | Self::PURE)
    }
}

// ── Templates and Generics ────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TemplateParamKind {
    /// `typename T`, `class T`, C# `T`, a concept-constrained `Concept T`.
    Type,
    /// Non-type parameter of the given type (`int N`, `auto V`).
    Value(String),
    /// Template template parameter `template<class> class TT`.
    Template,
}

/// C# generic variance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Variance {
    Invariant,
    In,
    Out,
}

/// One template or generic parameter of a classifier or operation.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TemplateParam {
    /// `None` for an unnamed parameter (`typename = void`).
    pub name: Option<String>,
    pub kind: TemplateParamKind,
    pub is_pack: bool,
    pub default: Option<String>,
    /// Concepts (C++) or `where` constraints (C#), as written.
    pub constraints: Vec<String>,
    pub variance: Variance,
}

impl TemplateParam {
    /// UML template-parameter label: `T`, `N : int`, `T : class, IFoo`, `Ts...`.
    pub fn label(&self) -> String {
        let mut s = self.name.clone().unwrap_or_else(|| "_".to_string());
        if self.is_pack {
            s.push_str("...");
        }
        match &self.kind {
            TemplateParamKind::Value(t) => s.push_str(&format!(" : {}", t)),
            TemplateParamKind::Template => s.push_str(" : template"),
            TemplateParamKind::Type => {
                if !self.constraints.is_empty() {
                    s.push_str(&format!(" : {}", self.constraints.join(", ")));
                }
            }
        }
        if let Some(d) = &self.default {
            s.push_str(&format!(" = {}", d));
        }
        s
    }
}

/// One argument of a template binding.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BoundArgument {
    /// Parameter name; `None` when the template's signature is not known.
    pub param: Option<String>,
    pub value: String,
    /// The argument comes from the parameter's default.
    pub is_default: bool,
}

/// A UML template binding (`«bind» <T -> int>`) of a project template.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TemplateBinding {
    pub template_name: String,
    pub template_index: Option<usize>,
    pub template_sym: u32,
    pub arguments: Vec<BoundArgument>,
}

impl TemplateBinding {
    /// Binds written arguments to a template signature positionally; a pack parameter
    /// absorbs the remaining arguments and missing arguments take their defaults.
    pub fn bind(
        template_name: &str,
        template_index: Option<usize>,
        template_sym: u32,
        params: &[TemplateParam],
        args: &[TypeArg],
        lang: CFamily,
    ) -> TemplateBinding {
        let texts: Vec<String> = args
            .iter()
            .map(|a| match a {
                TypeArg::Type(t) => t.display(lang),
                TypeArg::Value(v) => v.clone(),
            })
            .collect();
        let mut arguments = Vec::new();
        let mut k = 0usize;
        for p in params {
            if p.is_pack {
                let rest: Vec<String> = texts[k.min(texts.len())..].to_vec();
                k = texts.len();
                arguments.push(BoundArgument {
                    param: p.name.clone(),
                    value: rest.join(", "),
                    is_default: false,
                });
                continue;
            }
            if k < texts.len() {
                arguments.push(BoundArgument {
                    param: p.name.clone(),
                    value: texts[k].clone(),
                    is_default: false,
                });
                k += 1;
            } else if let Some(d) = &p.default {
                arguments.push(BoundArgument {
                    param: p.name.clone(),
                    value: d.clone(),
                    is_default: true,
                });
            }
        }
        for t in texts.iter().skip(k) {
            arguments.push(BoundArgument {
                param: None,
                value: t.clone(),
                is_default: false,
            });
        }
        TemplateBinding {
            template_name: template_name.to_string(),
            template_index,
            template_sym,
            arguments,
        }
    }

    /// UML 2.5 binding label: `Name<T -> int, U -> Foo>`.
    pub fn uml_label(&self) -> String {
        let parts: Vec<String> = self
            .arguments
            .iter()
            .map(|a| match &a.param {
                Some(p) => format!("{} -> {}", p, a.value),
                None => a.value.clone(),
            })
            .collect();
        format!("{}<{}>", self.template_name, parts.join(", "))
    }
}

/// Well-known standard concepts (used to tell `Concept T` from a non-type `Type N`).
fn is_std_concept(name: &str) -> bool {
    const NAMES: &[&str] = &[
        "same_as", "derived_from", "convertible_to", "common_reference_with", "common_with",
        "integral", "signed_integral", "unsigned_integral", "floating_point", "assignable_from",
        "swappable", "swappable_with", "destructible", "constructible_from",
        "default_initializable", "move_constructible", "copy_constructible",
        "equality_comparable", "equality_comparable_with", "totally_ordered",
        "totally_ordered_with", "three_way_comparable", "movable", "copyable", "semiregular",
        "regular", "invocable", "regular_invocable", "predicate", "relation",
        "equivalence_relation", "strict_weak_order", "range", "sized_range", "view",
        "input_range", "output_range", "forward_range", "bidirectional_range",
        "random_access_range", "contiguous_range", "common_range", "input_iterator",
        "output_iterator", "forward_iterator", "bidirectional_iterator",
        "random_access_iterator", "contiguous_iterator", "sentinel_for", "indirectly_readable",
        "arithmetic", "Arithmetic",
    ];
    NAMES.contains(&name)
}

/// Parses the tokens strictly inside a C++ `template< ... >` header.
pub fn parse_cpp_template_params(lang: CFamily, toks: &[&str]) -> Vec<TemplateParam> {
    split_text_commas(toks)
        .into_iter()
        .filter_map(|item| parse_cpp_template_param(lang, item))
        .collect()
}

/// Splits token texts at commas outside `()`, `[]`, `{}` and `<>`.
fn split_text_commas<'s, 't>(toks: &'s [&'t str]) -> Vec<&'s [&'t str]> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    for (k, t) in toks.iter().enumerate() {
        match *t {
            "(" | "[" | "{" | "<" => depth += 1,
            ")" | "]" | "}" => depth -= 1,
            ">" | ">>" | ">>>" => depth -= t.len() as i32,
            "," if depth <= 0 => {
                parts.push(&toks[start..k]);
                start = k + 1;
            }
            _ => {}
        }
    }
    if start < toks.len() {
        parts.push(&toks[start..]);
    }
    parts
}

/// Index of the first `=` outside brackets.
fn find_text_top_level(toks: &[&str], what: &str) -> Option<usize> {
    let mut depth = 0i32;
    for (k, t) in toks.iter().enumerate() {
        match *t {
            "(" | "[" | "{" | "<" => depth += 1,
            ")" | "]" | "}" => depth -= 1,
            ">" | ">>" | ">>>" => depth -= t.len() as i32,
            _ if *t == what && depth <= 0 => return Some(k),
            _ => {}
        }
    }
    None
}

fn parse_cpp_template_param(lang: CFamily, item: &[&str]) -> Option<TemplateParam> {
    if item.is_empty() {
        return None;
    }
    let (body, default) = match find_text_top_level(item, "=") {
        Some(eq) => (&item[..eq], Some(item[eq + 1..].join(" "))),
        None => (item, None),
    };
    let is_pack = body.contains(&"...");
    let body: Vec<&str> = body.iter().copied().filter(|t| *t != "...").collect();
    let mut param = TemplateParam {
        name: None,
        kind: TemplateParamKind::Type,
        is_pack,
        default,
        constraints: Vec::new(),
        variance: Variance::Invariant,
    };
    match body.first().copied() {
        Some("template") => {
            param.kind = TemplateParamKind::Template;
            param.name = body
                .last()
                .filter(|t| is_name(lang, t))
                .map(|t| t.to_string());
        }
        Some("typename") | Some("class") => {
            param.name = body
                .get(1)
                .filter(|t| is_name(lang, t))
                .map(|t| t.to_string());
        }
        Some(_) => match parse_type_greedy(lang, &body) {
            Ok((ty, used)) => {
                param.name = body
                    .get(used)
                    .filter(|t| is_name(lang, t))
                    .map(|t| t.to_string());
                let is_concept = ty.is_plain()
                    && ty
                        .simple_name()
                        .map(is_std_concept)
                        .unwrap_or(false);
                if is_concept {
                    param.constraints.push(ty.display(lang));
                } else {
                    param.kind = TemplateParamKind::Value(ty.display(lang));
                }
            }
            Err(_) => {
                param.kind = TemplateParamKind::Value(body.join(" "));
            }
        },
        None => return None,
    }
    Some(param)
}

/// Parses C# type parameters (the tokens strictly inside `< ... >`).
pub fn parse_cs_type_params(toks: &[&str]) -> Vec<TemplateParam> {
    let mut out = Vec::new();
    for item in split_text_commas(toks) {
        let mut k = 0usize;
        let mut variance = Variance::Invariant;
        while k < item.len() {
            match item[k] {
                "[" => {
                    while k < item.len() && item[k] != "]" {
                        k += 1;
                    }
                    k += 1;
                }
                "in" => {
                    variance = Variance::In;
                    k += 1;
                }
                "out" => {
                    variance = Variance::Out;
                    k += 1;
                }
                _ => break,
            }
        }
        let name = item
            .get(k)
            .filter(|t| is_name(CFamily::CSharp, t))
            .map(|t| plain_name(t).to_string());
        out.push(TemplateParam {
            name,
            kind: TemplateParamKind::Type,
            is_pack: false,
            default: None,
            constraints: Vec::new(),
            variance,
        });
    }
    out
}

/// Applies C# `where T : c1, c2 where U : ...` clauses to `params`.
pub fn apply_cs_constraints(params: &mut [TemplateParam], toks: &[&str]) {
    let mut k = 0usize;
    while k < toks.len() {
        if toks[k] != "where" {
            k += 1;
            continue;
        }
        let name = toks.get(k + 1).copied().unwrap_or("");
        if toks.get(k + 2).copied() != Some(":") {
            k += 1;
            continue;
        }
        let start = k + 3;
        let mut end = start;
        while end < toks.len() && toks[end] != "where" {
            end += 1;
        }
        let constraints: Vec<String> = split_text_commas(&toks[start..end])
            .into_iter()
            .filter(|c| !c.is_empty())
            .map(|c| c.join(" ").replace(" (", "(").replace("( ", "(").replace(" )", ")"))
            .collect();
        for p in params.iter_mut() {
            if p.name.as_deref() == Some(plain_name(name)) {
                p.constraints.extend(constraints.iter().cloned());
            }
        }
        k = end;
    }
}

// ── Packages ──────────────────────────────────────────────────────────────────

/// One namespace segment.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PackageSegment {
    pub name: String,
    /// C++ `inline namespace`: members are also visible in the enclosing namespace.
    pub is_inline: bool,
    /// C++ unnamed namespace (internal linkage).
    pub is_anonymous: bool,
}

/// A namespace path; a UML package (the empty path is the global namespace).
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PackagePath {
    pub segments: Vec<PackageSegment>,
}

impl PackagePath {
    /// Display name of an unnamed C++ namespace.
    pub const ANONYMOUS: &'static str = "(anonymous)";

    pub fn from_names<'s, I: IntoIterator<Item = &'s str>>(names: I) -> PackagePath {
        PackagePath {
            segments: names
                .into_iter()
                .map(|n| PackageSegment {
                    name: n.to_string(),
                    is_inline: false,
                    is_anonymous: false,
                })
                .collect(),
        }
    }

    pub fn is_global(&self) -> bool {
        self.segments.is_empty()
    }

    pub fn names(&self) -> Vec<&str> {
        self.segments.iter().map(|s| s.name.as_str()).collect()
    }

    /// Names with inline namespaces removed (the lookup-visible path).
    pub fn visible_names(&self) -> Vec<&str> {
        self.segments
            .iter()
            .filter(|s| !s.is_inline)
            .map(|s| s.name.as_str())
            .collect()
    }

    /// UML qualified name (`::`-separated).
    pub fn uml_name(&self) -> String {
        self.names().join("::")
    }

    /// Qualified name as written in `lang`.
    pub fn qualified(&self, lang: CFamily) -> String {
        self.names().join(lang.scope_separator())
    }

    pub fn parent(&self) -> Option<PackagePath> {
        if self.segments.is_empty() {
            return None;
        }
        let mut p = self.clone();
        p.segments.pop();
        Some(p)
    }

    /// True when `self` is `other` or encloses it.
    pub fn encloses(&self, other: &PackagePath) -> bool {
        self.segments.len() <= other.segments.len()
            && self
                .segments
                .iter()
                .zip(other.segments.iter())
                .all(|(a, b)| a.name == b.name)
    }

    /// Length of the common leading path of two packages.
    pub fn common_prefix(&self, other: &PackagePath) -> usize {
        self.segments
            .iter()
            .zip(other.segments.iter())
            .take_while(|(a, b)| a.name == b.name)
            .count()
    }

    /// Internal linkage: the path crosses an unnamed namespace.
    pub fn is_internal_linkage(&self) -> bool {
        self.segments.iter().any(|s| s.is_anonymous)
    }
}
