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
