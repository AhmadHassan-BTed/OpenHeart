//! C Semantic Adapter implementing C declaration & fundamental types, plus the C-family
//! lexical toolkit shared with the C++ adapter: the significant-token view over the corpus,
//! the C/C++ declaration grammar (decl-specifiers, declarators, function pointers, template
//! headers, class / enum / namespace heads, using-forms), the `#define` / `#include`
//! scanners and the linkage rules of both languages.
//!
//! Name resolution itself (scopes, tag and ordinary name spaces, merging, lookup) is done by
//! the C-family engine in `crate::lang::cpp::semantics`, which applies C rules to C
//! translation units; [`CSemanticView`] exposes the C-specific queries over that engine.

use std::collections::{HashMap, HashSet};
use std::fmt;

use crate::core::types::ast::ASTNodeType;
use crate::core::types::symbol::{ScopeKind, SymbolKind, SymbolVisibility};
use crate::core::types::token::{unpack_sort_key, TokenType};
use crate::ingestion::TokenCorpusArtifact;
use crate::lang::c::Dialect;
use crate::lang::cpp::semantics::{
    CppSemanticModel, EntityFlags, EntityId, EntityKind, LookupFilter, LookupResult,
    MemberLookup, ScopeId, GLOBAL_SCOPE,
};
use crate::symbol::adapter::SemanticAdapter;

pub struct CSemanticAdapter;

impl CSemanticAdapter {
    pub fn new() -> Self {
        Self
    }
}

impl Default for CSemanticAdapter {
    fn default() -> Self {
        Self::new()
    }
}

/// Base of the C-family fundamental-type pseudo-ids (shared by C and C++; Java uses
/// `0xB000_0000`).
pub const C_PRIMITIVE_BASE_ID: u32 = 0xB100_0000;

impl SemanticAdapter for CSemanticAdapter {
    fn is_declaration(&self, node_type: ASTNodeType) -> bool {
        matches!(
            node_type,
            ASTNodeType::NN_MODULE
                | ASTNodeType::NN_STRUCT_DECL
                | ASTNodeType::NN_UNION_DECL
                | ASTNodeType::NN_ENUM_DECL
                | ASTNodeType::NN_ENUM_CONSTANT
                | ASTNodeType::NN_TYPEDEF_DECL
                | ASTNodeType::NN_FUNCTION_DECL
                | ASTNodeType::NN_FIELD_DECL
                | ASTNodeType::NN_PARAM_DECL
                | ASTNodeType::NN_LOCAL_VAR_DECL
        )
    }

    fn symbol_kind(&self, node_type: ASTNodeType) -> SymbolKind {
        match node_type {
            ASTNodeType::NN_MODULE => SymbolKind::SK_MODULE,
            // A typedef name is a type; like `lang::neutral::TypeKind::TypeAlias` it is stored
            // as the closest type kind.
            ASTNodeType::NN_STRUCT_DECL
            | ASTNodeType::NN_UNION_DECL
            | ASTNodeType::NN_TYPEDEF_DECL => SymbolKind::SK_CLASS,
            ASTNodeType::NN_ENUM_DECL => SymbolKind::SK_ENUM,
            ASTNodeType::NN_ENUM_CONSTANT => SymbolKind::SK_ENUM_CONSTANT,
            ASTNodeType::NN_FUNCTION_DECL => SymbolKind::SK_METHOD,
            ASTNodeType::NN_FIELD_DECL => SymbolKind::SK_FIELD,
            ASTNodeType::NN_PARAM_DECL => SymbolKind::SK_PARAM,
            ASTNodeType::NN_LOCAL_VAR_DECL => SymbolKind::SK_LOCAL_VAR,
            _ => SymbolKind::SK_EXTERNAL,
        }
    }

    fn scope_kind(&self, node_type: ASTNodeType) -> ScopeKind {
        match node_type {
            ASTNodeType::NN_MODULE => ScopeKind::File,
            ASTNodeType::NN_STRUCT_DECL | ASTNodeType::NN_UNION_DECL | ASTNodeType::NN_ENUM_DECL => {
                ScopeKind::Class
            }
            ASTNodeType::NN_FUNCTION_DECL => ScopeKind::Method,
            _ => ScopeKind::Block,
        }
    }

    fn primitive_type_id(&self, text: &str) -> Option<u32> {
        FundamentalType::from_spelling(text, Dialect::C).map(FundamentalType::type_id)
    }

    /// C has no library collection types; arrays are declarator derivations, not types
    /// with a name.
    fn is_collection_type(&self, _type_name: &str) -> bool {
        false
    }

    /// C has no access control: every member and file-scope name is public.
    fn default_visibility(&self, _kind: SymbolKind) -> SymbolVisibility {
        SymbolVisibility::Public
    }
}

// ── Fundamental types ────────────────────────────────────────────────────────

/// The arithmetic, character, `void` and `nullptr_t` types of C and C++.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FundamentalType {
    Void = 0,
    Bool = 1,
    Char = 2,
    SignedChar = 3,
    UnsignedChar = 4,
    WChar = 5,
    Char8 = 6,
    Char16 = 7,
    Char32 = 8,
    Short = 9,
    UnsignedShort = 10,
    Int = 11,
    UnsignedInt = 12,
    Long = 13,
    UnsignedLong = 14,
    LongLong = 15,
    UnsignedLongLong = 16,
    Int128 = 17,
    UnsignedInt128 = 18,
    Float = 19,
    Double = 20,
    LongDouble = 21,
    ComplexFloat = 22,
    ComplexDouble = 23,
    ComplexLongDouble = 24,
    NullPtr = 25,
}

impl FundamentalType {
    pub const ALL: [FundamentalType; 26] = [
        FundamentalType::Void,
        FundamentalType::Bool,
        FundamentalType::Char,
        FundamentalType::SignedChar,
        FundamentalType::UnsignedChar,
        FundamentalType::WChar,
        FundamentalType::Char8,
        FundamentalType::Char16,
        FundamentalType::Char32,
        FundamentalType::Short,
        FundamentalType::UnsignedShort,
        FundamentalType::Int,
        FundamentalType::UnsignedInt,
        FundamentalType::Long,
        FundamentalType::UnsignedLong,
        FundamentalType::LongLong,
        FundamentalType::UnsignedLongLong,
        FundamentalType::Int128,
        FundamentalType::UnsignedInt128,
        FundamentalType::Float,
        FundamentalType::Double,
        FundamentalType::LongDouble,
        FundamentalType::ComplexFloat,
        FundamentalType::ComplexDouble,
        FundamentalType::ComplexLongDouble,
        FundamentalType::NullPtr,
    ];

    /// Pseudo symbol id used for type-reference resolutions.
    pub fn type_id(self) -> u32 {
        C_PRIMITIVE_BASE_ID + self as u32
    }

    pub fn from_type_id(id: u32) -> Option<Self> {
        let off = id.checked_sub(C_PRIMITIVE_BASE_ID)?;
        Self::ALL.iter().copied().find(|t| *t as u32 == off)
    }

    pub fn spelling(self) -> &'static str {
        match self {
            FundamentalType::Void => "void",
            FundamentalType::Bool => "bool",
            FundamentalType::Char => "char",
            FundamentalType::SignedChar => "signed char",
            FundamentalType::UnsignedChar => "unsigned char",
            FundamentalType::WChar => "wchar_t",
            FundamentalType::Char8 => "char8_t",
            FundamentalType::Char16 => "char16_t",
            FundamentalType::Char32 => "char32_t",
            FundamentalType::Short => "short",
            FundamentalType::UnsignedShort => "unsigned short",
            FundamentalType::Int => "int",
            FundamentalType::UnsignedInt => "unsigned int",
            FundamentalType::Long => "long",
            FundamentalType::UnsignedLong => "unsigned long",
            FundamentalType::LongLong => "long long",
            FundamentalType::UnsignedLongLong => "unsigned long long",
            FundamentalType::Int128 => "__int128",
            FundamentalType::UnsignedInt128 => "unsigned __int128",
            FundamentalType::Float => "float",
            FundamentalType::Double => "double",
            FundamentalType::LongDouble => "long double",
            FundamentalType::ComplexFloat => "_Complex float",
            FundamentalType::ComplexDouble => "_Complex double",
            FundamentalType::ComplexLongDouble => "_Complex long double",
            FundamentalType::NullPtr => "std::nullptr_t",
        }
    }

    /// True for a word that can be part of a fundamental type specifier sequence.
    pub fn is_specifier_word(word: &str, dialect: Dialect) -> bool {
        match word {
            "void" | "char" | "short" | "int" | "long" | "signed" | "unsigned" | "float"
            | "double" | "_Bool" | "_Complex" | "_Imaginary" | "__int128" | "__int64"
            | "__int32" | "__int16" | "__int8" | "__signed" | "__signed__" | "__unsigned" => true,
            "bool" => true,
            "wchar_t" | "char8_t" | "char16_t" | "char32_t" => dialect == Dialect::Cpp,
            _ => false,
        }
    }

    /// Normalizes a specifier sequence (`unsigned long long int`, `long double`, `signed`).
    /// `None` when the words do not form a fundamental type.
    pub fn from_words(words: &[&str], dialect: Dialect) -> Option<Self> {
        let mut signed = false;
        let mut unsigned = false;
        let mut short = 0u8;
        let mut long = 0u8;
        let mut int = false;
        let mut complex = false;
        let mut base: Option<&str> = None;
        for &w in words {
            // In C these are typedef names of <stddef.h> / <uchar.h>, not keywords.
            if dialect == Dialect::C && matches!(w, "wchar_t" | "char8_t" | "char16_t" | "char32_t")
            {
                return None;
            }
            match w {
                "signed" | "__signed" | "__signed__" => signed = true,
                "unsigned" | "__unsigned" => unsigned = true,
                "short" | "__int16" => short += 1,
                "long" => long += 1,
                "int" | "__int32" => int = true,
                "__int64" => long = 2,
                "_Complex" | "_Imaginary" => complex = true,
                "void" | "char" | "float" | "double" | "_Bool" | "bool" | "wchar_t"
                | "char8_t" | "char16_t" | "char32_t" | "__int128" | "__int8" => {
                    if base.is_some() {
                        return None;
                    }
                    base = Some(w);
                }
                _ => return None,
            }
        }
        if signed && unsigned || short > 1 || long > 2 || (short > 0 && long > 0) {
            return None;
        }
        let integral_modifiers = signed || unsigned || short > 0 || long > 0 || int;
        Some(match base {
            Some("void") if !integral_modifiers && !complex => FundamentalType::Void,
            // `bool` is a keyword in C++ and C23; earlier C spells it `_Bool` (or through the
            // <stdbool.h> macro), and both name the same type.
            Some("_Bool") | Some("bool") if !integral_modifiers => FundamentalType::Bool,
            Some("char") | Some("__int8") if short == 0 && long == 0 && !int => {
                if signed {
                    FundamentalType::SignedChar
                } else if unsigned {
                    FundamentalType::UnsignedChar
                } else {
                    FundamentalType::Char
                }
            }
            Some("wchar_t") if !integral_modifiers => FundamentalType::WChar,
            Some("char8_t") if !integral_modifiers => FundamentalType::Char8,
            Some("char16_t") if !integral_modifiers => FundamentalType::Char16,
            Some("char32_t") if !integral_modifiers => FundamentalType::Char32,
            Some("float") if !integral_modifiers => {
                if complex {
                    FundamentalType::ComplexFloat
                } else {
                    FundamentalType::Float
                }
            }
            Some("double") if !signed && !unsigned && short == 0 && !int && long <= 1 => {
                match (long == 1, complex) {
                    (true, true) => FundamentalType::ComplexLongDouble,
                    (true, false) => FundamentalType::LongDouble,
                    (false, true) => FundamentalType::ComplexDouble,
                    (false, false) => FundamentalType::Double,
                }
            }
            Some("__int128") if short == 0 && long == 0 => {
                if unsigned {
                    FundamentalType::UnsignedInt128
                } else {
                    FundamentalType::Int128
                }
            }
            None if integral_modifiers && !complex => match (short, long, unsigned) {
                (1, _, false) => FundamentalType::Short,
                (1, _, true) => FundamentalType::UnsignedShort,
                (_, 1, false) => FundamentalType::Long,
                (_, 1, true) => FundamentalType::UnsignedLong,
                (_, 2, false) => FundamentalType::LongLong,
                (_, 2, true) => FundamentalType::UnsignedLongLong,
                (_, _, false) => FundamentalType::Int,
                (_, _, true) => FundamentalType::UnsignedInt,
            },
            None if complex => FundamentalType::ComplexDouble,
            _ => return None,
        })
    }

    /// Parses a whitespace-separated spelling (`"unsigned long"`, `"int"`).
    pub fn from_spelling(text: &str, dialect: Dialect) -> Option<Self> {
        let trimmed = text.trim();
        if trimmed == "nullptr_t" || trimmed == "std::nullptr_t" {
            return if dialect == Dialect::Cpp {
                Some(FundamentalType::NullPtr)
            } else {
                None
            };
        }
        let words: Vec<&str> = trimmed.split_whitespace().collect();
        if words.is_empty() {
            return None;
        }
        Self::from_words(&words, dialect)
    }

    pub fn is_integral(self) -> bool {
        matches!(
            self,
            FundamentalType::Bool
                | FundamentalType::Char
                | FundamentalType::SignedChar
                | FundamentalType::UnsignedChar
                | FundamentalType::WChar
                | FundamentalType::Char8
                | FundamentalType::Char16
                | FundamentalType::Char32
                | FundamentalType::Short
                | FundamentalType::UnsignedShort
                | FundamentalType::Int
                | FundamentalType::UnsignedInt
                | FundamentalType::Long
                | FundamentalType::UnsignedLong
                | FundamentalType::LongLong
                | FundamentalType::UnsignedLongLong
                | FundamentalType::Int128
                | FundamentalType::UnsignedInt128
        )
    }

    pub fn is_floating(self) -> bool {
        matches!(
            self,
            FundamentalType::Float
                | FundamentalType::Double
                | FundamentalType::LongDouble
                | FundamentalType::ComplexFloat
                | FundamentalType::ComplexDouble
                | FundamentalType::ComplexLongDouble
        )
    }

    pub fn is_arithmetic(self) -> bool {
        self.is_integral() || self.is_floating()
    }

    /// Target of the integral or floating-point promotion of this type, when one applies
    /// (C11 6.3.1.1, C++ [conv.prom], [conv.fpprom]).
    pub fn promotion(self) -> Option<FundamentalType> {
        match self {
            FundamentalType::Bool
            | FundamentalType::Char
            | FundamentalType::SignedChar
            | FundamentalType::UnsignedChar
            | FundamentalType::Short
            | FundamentalType::UnsignedShort
            | FundamentalType::Char8 => Some(FundamentalType::Int),
            FundamentalType::WChar | FundamentalType::Char16 => Some(FundamentalType::Int),
            FundamentalType::Char32 => Some(FundamentalType::UnsignedInt),
            FundamentalType::Float => Some(FundamentalType::Double),
            _ => None,
        }
    }
}

// ── Significant-token view ───────────────────────────────────────────────────

/// One significant (non-trivia) token. `index` is the `token_records` index, or `u32::MAX`
/// for tokens lexed from catalog text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CTok<'a> {
    pub index: u32,
    pub text: &'a str,
    pub kind: TokenType,
    pub text_id: u32,
    pub line: u32,
    pub col: u16,
    pub len: u16,
}

impl<'a> CTok<'a> {
    /// True when `self` ends exactly where `next` starts (no whitespace in between).
    pub fn adjacent_to(&self, next: &CTok<'_>) -> bool {
        self.line == next.line && self.col as u32 + self.len as u32 == next.col as u32
    }

    pub fn is_literal(&self) -> bool {
        matches!(
            self.kind,
            TokenType::IntegerLiteral
                | TokenType::FloatLiteral
                | TokenType::StringLiteral
                | TokenType::CharLiteral
                | TokenType::BooleanLiteral
                | TokenType::NullLiteral
        ) || self
            .text
            .as_bytes()
            .first()
            .is_some_and(|b| b.is_ascii_digit() || *b == b'"' || *b == b'\'')
            || (self.text.len() > 1
                && matches!(self.text.as_bytes()[0], b'L' | b'u' | b'U' | b'R')
                && self.text.contains(['"', '\'']))
    }
}

fn is_trivia(kind: TokenType) -> bool {
    matches!(
        kind,
        TokenType::CommentLine
            | TokenType::CommentBlock
            | TokenType::CommentDoc
            | TokenType::Whitespace
            | TokenType::Newline
    )
}

/// The significant tokens of the corpus grouped per file, in source order.
pub struct CorpusTokens<'a> {
    by_file: HashMap<u16, Vec<CTok<'a>>>,
}

impl<'a> CorpusTokens<'a> {
    /// Collects the significant tokens of the given files (all files when `files` is `None`).
    pub fn build(tca: &'a TokenCorpusArtifact, files: Option<&[u16]>) -> Self {
        let mut by_file: HashMap<u16, Vec<CTok<'a>>> = HashMap::new();
        for (i, rec) in tca.token_records.iter().enumerate() {
            let kind = TokenType::from_u8(rec.token_type);
            if is_trivia(kind) {
                continue;
            }
            let (file, line, col) = unpack_sort_key(rec.sort_key);
            if let Some(fs) = files {
                if !fs.contains(&file) {
                    continue;
                }
            }
            let text = match std::str::from_utf8(tca.interner.lookup_text(rec.text_id)) {
                Ok(t) => t.trim(),
                Err(_) => continue,
            };
            if text.is_empty() {
                continue;
            }
            by_file.entry(file).or_default().push(CTok {
                index: i as u32,
                text,
                kind,
                text_id: rec.text_id,
                line,
                col,
                len: rec.len,
            });
        }
        Self { by_file }
    }

    pub fn file(&self, file: u16) -> &[CTok<'a>] {
        self.by_file.get(&file).map(|v| v.as_slice()).unwrap_or(&[])
    }

    pub fn files(&self) -> impl Iterator<Item = u16> + '_ {
        self.by_file.keys().copied()
    }

    /// Significant tokens of `file` whose record index lies in `first..=last`.
    pub fn slice(&self, file: u16, first: u32, last: u32) -> &[CTok<'a>] {
        let toks = self.file(file);
        if first == u32::MAX || first > last {
            return &[];
        }
        let lo = toks.partition_point(|t| t.index < first);
        let hi = toks.partition_point(|t| t.index <= last);
        &toks[lo..hi.max(lo)]
    }

    /// Position of the first significant token of `file` with record index `>= index`.
    pub fn position(&self, file: u16, index: u32) -> usize {
        self.file(file).partition_point(|t| t.index < index)
    }

    /// The significant token immediately before record index `index` in `file`.
    pub fn previous(&self, file: u16, index: u32) -> Option<&CTok<'a>> {
        let p = self.position(file, index);
        if p == 0 {
            None
        } else {
            self.file(file).get(p - 1)
        }
    }

    /// The significant token immediately after record index `index` in `file`.
    pub fn next(&self, file: u16, index: u32) -> Option<&CTok<'a>> {
        let toks = self.file(file);
        let p = toks.partition_point(|t| t.index <= index);
        toks.get(p)
    }
}

const PUNCTUATORS: [&str; 30] = [
    "...", "<<=", ">>=", "->*", "<=>", "::", "->", "++", "--", "<<", ">>", "<=", ">=", "==",
    "!=", "&&", "||", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", ".*", "##", "[[", "]]",
    "#",
];

/// Lexes C/C++ declaration text (catalog entries of the standard-library models) into
/// synthetic tokens. `[[` / `]]` are produced as two tokens each, as tree-sitter does.
pub fn lex_c_text(src: &str) -> Vec<CTok<'_>> {
    let bytes = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    let mut line = 1u32;
    let mut line_start = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'\n' {
            line += 1;
            i += 1;
            line_start = i;
            continue;
        }
        if b.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        let kind;
        if b.is_ascii_alphabetic() || b == b'_' {
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            kind = if is_c_family_keyword(&src[start..i], Dialect::Cpp) {
                TokenType::Keyword
            } else {
                TokenType::Identifier
            };
        } else if b.is_ascii_digit() {
            while i < bytes.len()
                && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'.' || bytes[i] == b'\'')
            {
                i += 1;
            }
            kind = if src[start..i].contains('.') {
                TokenType::FloatLiteral
            } else {
                TokenType::IntegerLiteral
            };
        } else if b == b'"' || b == b'\'' {
            i += 1;
            while i < bytes.len() && bytes[i] != b {
                if bytes[i] == b'\\' {
                    i += 1;
                }
                i += 1;
            }
            i = (i + 1).min(bytes.len());
            kind = if b == b'"' {
                TokenType::StringLiteral
            } else {
                TokenType::CharLiteral
            };
        } else {
            let rest = &src[i..];
            let p = PUNCTUATORS
                .iter()
                .filter(|p| **p != "[[" && **p != "]]")
                .find(|p| rest.starts_with(**p));
            i += p.map(|p| p.len()).unwrap_or_else(|| {
                rest.chars().next().map(|c| c.len_utf8()).unwrap_or(1)
            });
            kind = if matches!(&src[start..i], ";" | "," | "(" | ")" | "[" | "]" | "{" | "}" | "::" | ".") {
                TokenType::Punctuation
            } else {
                TokenType::Operator
            };
        }
        out.push(CTok {
            index: u32::MAX,
            text: &src[start..i],
            kind,
            text_id: u32::MAX,
            line,
            col: (start - line_start).min(u16::MAX as usize) as u16,
            len: (i - start).min(u16::MAX as usize) as u16,
        });
    }
    out
}

/// Reserved words of C (C23 with the GNU / MSVC spellings tree-sitter-c accepts) and, for
/// `Dialect::Cpp`, of C++23. Contextual keywords (`override`, `final`, `import`, `module`)
/// are identifiers.
pub fn is_c_family_keyword(word: &str, dialect: Dialect) -> bool {
    const C_KEYWORDS: &[&str] = &[
        "auto", "break", "case", "char", "const", "continue", "default", "do", "double",
        "else", "enum", "extern", "float", "for", "goto", "if", "inline", "int", "long",
        "register", "restrict", "return", "short", "signed", "sizeof", "static", "struct",
        "switch", "typedef", "union", "unsigned", "void", "volatile", "while", "_Alignas",
        "_Alignof", "_Atomic", "_Bool", "_Complex", "_Generic", "_Imaginary", "_Noreturn",
        "_Static_assert", "_Thread_local", "alignas", "alignof", "bool", "constexpr",
        "false", "nullptr", "static_assert", "thread_local", "true", "typeof",
        "typeof_unqual", "__attribute__", "__attribute", "__declspec", "__extension__",
        "__inline", "__inline__", "__restrict", "__restrict__", "__thread", "__typeof__",
        "__typeof", "__asm__", "__asm", "asm", "__volatile__", "__const", "__int128",
        "__int64", "__int32", "__int16", "__int8", "__forceinline", "__cdecl", "__stdcall",
        "__fastcall", "__thiscall", "__vectorcall",
    ];
    const CPP_KEYWORDS: &[&str] = &[
        "and", "and_eq", "bitand", "bitor", "catch", "char8_t", "char16_t", "char32_t",
        "class", "co_await", "co_return", "co_yield", "compl", "concept", "consteval",
        "constinit", "const_cast", "decltype", "delete", "dynamic_cast", "explicit",
        "export", "friend", "mutable", "namespace", "new", "noexcept", "not", "not_eq",
        "operator", "or", "or_eq", "private", "protected", "public", "reinterpret_cast",
        "requires", "static_cast", "template", "this", "throw", "try", "typeid", "typename",
        "using", "virtual", "wchar_t", "xor", "xor_eq",
    ];
    C_KEYWORDS.contains(&word) || (dialect == Dialect::Cpp && CPP_KEYWORDS.contains(&word))
}

/// True for an identifier spelling that is not a keyword of the dialect.
pub fn is_identifier(text: &str, dialect: Dialect) -> bool {
    let mut chars = text.chars();
    match chars.next() {
        Some(c) if c.is_alphabetic() || c == '_' || c == '$' => {}
        _ => return false,
    }
    chars.all(|c| c.is_alphanumeric() || c == '_' || c == '$') && !is_c_family_keyword(text, dialect)
}

/// FNV-1a hash of a token-text sequence; equal definitions in two files have equal
/// fingerprints (used by the one-definition check).
pub fn fingerprint(toks: &[CTok<'_>]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for t in toks {
        for b in t.text.bytes().chain(std::iter::once(0u8)) {
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    h
}

// ── Access and linkage ───────────────────────────────────────────────────────

/// Member access of C++ (C members are always public).
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Access {
    Public = 1,
    Private = 2,
    Protected = 3,
}

impl Access {
    pub fn from_keyword(word: &str) -> Option<Access> {
        match word {
            "public" => Some(Access::Public),
            "protected" => Some(Access::Protected),
            "private" => Some(Access::Private),
            _ => None,
        }
    }

    pub fn visibility(self) -> SymbolVisibility {
        match self {
            Access::Public => SymbolVisibility::Public,
            Access::Protected => SymbolVisibility::Protected,
            Access::Private => SymbolVisibility::Private,
        }
    }

    /// Ordering by openness: public > protected > private.
    pub fn openness(self) -> u8 {
        match self {
            Access::Public => 2,
            Access::Protected => 1,
            Access::Private => 0,
        }
    }

    /// Access of a base-class member as a member of the derived class after inheriting it
    /// with `inheritance` access ([class.access.base]/1). `None`: the member was private in
    /// the base and is not accessible as a member of the derived class at all.
    pub fn through_inheritance(self, inheritance: Access) -> Option<Access> {
        if self == Access::Private {
            return None;
        }
        Some(if inheritance.openness() < self.openness() {
            inheritance
        } else {
            self
        })
    }
}

/// Linkage of a name ([basic.link], C11 6.2.2).
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Linkage {
    External = 0,
    Internal = 1,
    None = 2,
}

// ── Declaration grammar: data model ──────────────────────────────────────────

pub const CV_CONST: u8 = 1 << 0;
pub const CV_VOLATILE: u8 = 1 << 1;
pub const CV_RESTRICT: u8 = 1 << 2;
pub const CV_ATOMIC: u8 = 1 << 3;

/// Storage-class specifiers.
pub const SC_TYPEDEF: u16 = 1 << 0;
pub const SC_STATIC: u16 = 1 << 1;
pub const SC_EXTERN: u16 = 1 << 2;
pub const SC_REGISTER: u16 = 1 << 3;
pub const SC_THREAD_LOCAL: u16 = 1 << 4;
pub const SC_MUTABLE: u16 = 1 << 5;
/// C `auto` storage class (in C++11 and later `auto` is a placeholder type instead).
pub const SC_AUTO: u16 = 1 << 6;

/// Function specifiers and the other non-type decl-specifiers.
pub const FS_INLINE: u16 = 1 << 0;
pub const FS_VIRTUAL: u16 = 1 << 1;
pub const FS_EXPLICIT: u16 = 1 << 2;
pub const FS_CONSTEXPR: u16 = 1 << 3;
pub const FS_CONSTEVAL: u16 = 1 << 4;
pub const FS_CONSTINIT: u16 = 1 << 5;
pub const FS_FRIEND: u16 = 1 << 6;
pub const FS_NORETURN: u16 = 1 << 7;

/// Virt-specifiers of a member function declarator.
pub const VIRT_OVERRIDE: u8 = 1 << 0;
pub const VIRT_FINAL: u8 = 1 << 1;

/// One component of a qualified name, with its template argument list when written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamePart {
    pub name: String,
    /// Record index of the name token, or `u32::MAX`.
    pub token: u32,
    pub args: Option<Vec<TemplateArg>>,
}

/// A possibly qualified name as written: `::a::B<int>::c`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct QualName {
    /// Leading `::` (lookup starts at the global namespace).
    pub global: bool,
    pub parts: Vec<NamePart>,
}

impl QualName {
    pub fn simple(name: &str, token: u32) -> Self {
        Self {
            global: false,
            parts: vec![NamePart {
                name: name.to_string(),
                token,
                args: None,
            }],
        }
    }

    pub fn last(&self) -> Option<&NamePart> {
        self.parts.last()
    }

    pub fn last_name(&self) -> &str {
        self.parts.last().map(|p| p.name.as_str()).unwrap_or("")
    }

    pub fn last_token(&self) -> u32 {
        self.parts.last().map(|p| p.token).unwrap_or(u32::MAX)
    }

    pub fn is_qualified(&self) -> bool {
        self.global || self.parts.len() > 1
    }

    pub fn is_empty(&self) -> bool {
        self.parts.is_empty()
    }

    /// The nested-name-specifier: every component but the last.
    pub fn qualifier(&self) -> QualName {
        let n = self.parts.len().saturating_sub(1);
        QualName {
            global: self.global,
            parts: self.parts[..n].to_vec(),
        }
    }

    /// `a::b::c` without template arguments.
    pub fn plain(&self) -> String {
        let mut s = String::new();
        if self.global {
            s.push_str("::");
        }
        for (i, p) in self.parts.iter().enumerate() {
            if i > 0 {
                s.push_str("::");
            }
            s.push_str(&p.name);
        }
        s
    }

    /// The name as written, template arguments included.
    pub fn spelled(&self) -> String {
        let mut s = String::new();
        if self.global {
            s.push_str("::");
        }
        for (i, p) in self.parts.iter().enumerate() {
            if i > 0 {
                s.push_str("::");
            }
            s.push_str(&p.name);
            if let Some(args) = &p.args {
                s.push('<');
                for (j, a) in args.iter().enumerate() {
                    if j > 0 {
                        s.push_str(", ");
                    }
                    s.push_str(&a.spelled());
                }
                s.push('>');
            }
        }
        s
    }

    /// Parses a name such as `std::vector<int>::iterator`; `None` when `text` is not one.
    pub fn parse_str(text: &str, dialect: Dialect) -> Option<QualName> {
        let toks = lex_c_text(text);
        let oracle = NoTypeOracle;
        let mut p = DeclParser::new(&toks, DeclContext::plain(dialect, &oracle));
        match p.parse_id_expression() {
            Some((qn, _)) if p.at_end() => Some(qn),
            _ => None,
        }
    }
}

/// A template argument: a type-id, or an expression kept as text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TemplateArg {
    Type(TypeSpec),
    Expr(String),
}

impl TemplateArg {
    pub fn spelled(&self) -> String {
        match self {
            TemplateArg::Type(t) => t.spelled(),
            TemplateArg::Expr(e) => e.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TagKey {
    Struct,
    Class,
    Union,
    Enum,
}

impl TagKey {
    pub fn keyword(self) -> &'static str {
        match self {
            TagKey::Struct => "struct",
            TagKey::Class => "class",
            TagKey::Union => "union",
            TagKey::Enum => "enum",
        }
    }
}

/// The type named by the decl-specifier sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BaseType {
    /// No type specifier: constructors, destructors, conversion functions, or an error.
    None,
    Fundamental(FundamentalType),
    /// A class, enum, typedef or template name; `key` is set for elaborated specifiers.
    Named { key: Option<TagKey>, name: QualName },
    /// C++ `auto` placeholder.
    Auto,
    /// `decltype(auto)`.
    DecltypeAuto,
    /// `decltype(expr)` / `typeof(expr)`; the operand is kept as text.
    Decltype(String),
    /// A specifier sequence that names no type (e.g. `long char`); kept verbatim.
    Unknown(String),
}

impl BaseType {
    pub fn spelled(&self) -> String {
        match self {
            BaseType::None => String::new(),
            BaseType::Fundamental(f) => f.spelling().to_string(),
            BaseType::Named { key, name } => match key {
                Some(k) => format!("{} {}", k.keyword(), name.spelled()),
                None => name.spelled(),
            },
            BaseType::Auto => "auto".to_string(),
            BaseType::DecltypeAuto => "decltype(auto)".to_string(),
            BaseType::Decltype(e) => format!("decltype({})", e),
            BaseType::Unknown(s) => s.clone(),
        }
    }
}

/// One type constructor applied by a declarator, outermost first: in `int *a[3]` the
/// derivations of `a` are `[Array, Pointer]` (array of pointers to int).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Derivation {
    Pointer { cv: u8 },
    LValueRef,
    RValueRef,
    Array { size: Option<String> },
    Function(Box<FunctionShape>),
    MemberPointer { class: QualName, cv: u8 },
}

/// A complete written type: decl-specifier base, top-level cv of the base, derivations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeSpec {
    pub base: BaseType,
    pub cv: u8,
    pub derivations: Vec<Derivation>,
    /// Pack expansion (`Ts...`).
    pub pack: bool,
}

impl TypeSpec {
    pub fn of(base: BaseType) -> Self {
        Self {
            base,
            cv: 0,
            derivations: Vec::new(),
            pack: false,
        }
    }

    /// The function type this declarator declares (`f` in `int f(int)`), if any.
    pub fn function(&self) -> Option<&FunctionShape> {
        match self.derivations.first() {
            Some(Derivation::Function(f)) => Some(f),
            _ => None,
        }
    }

    /// The pointee function type of a function pointer (`fp` in `int (*fp)(int)`).
    pub fn function_pointer(&self) -> Option<&FunctionShape> {
        let mut it = self.derivations.iter();
        let mut d = it.next();
        if matches!(d, Some(Derivation::LValueRef) | Some(Derivation::RValueRef)) {
            d = it.next();
        }
        match (d, it.next()) {
            (Some(Derivation::Pointer { .. }), Some(Derivation::Function(f))) => Some(f),
            (Some(Derivation::MemberPointer { .. }), Some(Derivation::Function(f))) => Some(f),
            _ => None,
        }
    }

    pub fn is_reference(&self) -> bool {
        matches!(
            self.derivations.first(),
            Some(Derivation::LValueRef) | Some(Derivation::RValueRef)
        )
    }

    pub fn is_rvalue_reference(&self) -> bool {
        matches!(self.derivations.first(), Some(Derivation::RValueRef))
    }

    /// Number of pointer / array levels after an outer reference, up to the first
    /// function derivation.
    pub fn pointer_depth(&self) -> u8 {
        let mut n = 0u8;
        for d in &self.derivations {
            match d {
                Derivation::LValueRef | Derivation::RValueRef => {}
                Derivation::Pointer { .. }
                | Derivation::Array { .. }
                | Derivation::MemberPointer { .. } => n = n.saturating_add(1),
                Derivation::Function(_) => break,
            }
        }
        n
    }

    /// True when the object designated (after an outer reference) is const-qualified.
    pub fn is_const_object(&self) -> bool {
        for d in &self.derivations {
            match d {
                Derivation::LValueRef | Derivation::RValueRef => continue,
                Derivation::Pointer { cv } | Derivation::MemberPointer { cv, .. } => {
                    return cv & CV_CONST != 0
                }
                Derivation::Array { .. } => continue,
                Derivation::Function(_) => return false,
            }
        }
        self.cv & CV_CONST != 0
    }

    /// True when the referred-to or pointed-to base type is const-qualified.
    pub fn is_const_base(&self) -> bool {
        self.cv & CV_CONST != 0
    }

    pub fn base_name(&self) -> Option<&QualName> {
        match &self.base {
            BaseType::Named { name, .. } => Some(name),
            _ => None,
        }
    }

    /// Key of the derivation chain used by signature matching. With `decay_top`, a
    /// top-level array or function decays to a pointer and top-level cv is dropped, as for
    /// function parameter types ([dcl.fct]/5).
    pub fn derivation_key(&self, decay_top: bool) -> String {
        let mut s = String::new();
        for (i, d) in self.derivations.iter().enumerate() {
            match d {
                Derivation::Pointer { cv } => {
                    s.push('*');
                    if !(decay_top && i == 0) {
                        push_cv(&mut s, *cv);
                    }
                }
                Derivation::LValueRef => s.push('&'),
                Derivation::RValueRef => s.push_str("&&"),
                Derivation::Array { .. } => {
                    if decay_top && i == 0 {
                        s.push('*');
                    } else {
                        s.push_str("[]");
                    }
                }
                Derivation::Function(f) => {
                    if decay_top && i == 0 {
                        s.push('*');
                    }
                    s.push('(');
                    s.push_str(&f.params.len().to_string());
                    if f.variadic {
                        s.push_str(",...");
                    }
                    s.push(')');
                }
                Derivation::MemberPointer { class, cv } => {
                    s.push_str(&class.plain());
                    s.push_str("::*");
                    push_cv(&mut s, *cv);
                }
            }
        }
        s
    }

    pub fn spelled(&self) -> String {
        let mut s = String::new();
        if self.cv & CV_CONST != 0 {
            s.push_str("const ");
        }
        if self.cv & CV_VOLATILE != 0 {
            s.push_str("volatile ");
        }
        s.push_str(&self.base.spelled());
        for d in self.derivations.iter().rev() {
            match d {
                Derivation::Pointer { cv } => {
                    s.push('*');
                    if cv & CV_CONST != 0 {
                        s.push_str(" const");
                    }
                }
                Derivation::LValueRef => s.push('&'),
                Derivation::RValueRef => s.push_str("&&"),
                Derivation::Array { size } => {
                    s.push('[');
                    if let Some(sz) = size {
                        s.push_str(sz);
                    }
                    s.push(']');
                }
                Derivation::Function(f) => {
                    s.push('(');
                    for (i, p) in f.params.iter().enumerate() {
                        if i > 0 {
                            s.push_str(", ");
                        }
                        s.push_str(&p.ty.spelled());
                    }
                    if f.variadic {
                        if !f.params.is_empty() {
                            s.push_str(", ");
                        }
                        s.push_str("...");
                    }
                    s.push(')');
                }
                Derivation::MemberPointer { class, .. } => {
                    s.push(' ');
                    s.push_str(&class.plain());
                    s.push_str("::*");
                }
            }
        }
        if self.pack {
            s.push_str("...");
        }
        s
    }
}

fn push_cv(s: &mut String, cv: u8) {
    if cv & CV_CONST != 0 {
        s.push('c');
    }
    if cv & CV_VOLATILE != 0 {
        s.push('v');
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RefQualifier {
    #[default]
    None,
    LValue,
    RValue,
}

/// One function parameter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParamSpec {
    pub name: Option<String>,
    pub token: u32,
    pub ty: TypeSpec,
    pub has_default: bool,
}

/// The parameter list and trailing qualifiers of a function declarator.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FunctionShape {
    pub params: Vec<ParamSpec>,
    /// C-style `...` ellipsis.
    pub variadic: bool,
    /// C declarator without a prototype: `f()` before C23, or a K&R identifier list.
    pub unprototyped: bool,
    /// K&R identifier list (`int f(a, b) int a; char *b; {}`).
    pub kr_names: Vec<String>,
    /// cv-qualifiers of a member function (`void f() const`).
    pub cv: u8,
    pub ref_qual: RefQualifier,
    pub noexcept: bool,
    pub trailing_return: Option<Box<TypeSpec>>,
    /// `VIRT_OVERRIDE` / `VIRT_FINAL`.
    pub virt: u8,
}

impl FunctionShape {
    /// Arguments that must be supplied (parameters without default arguments and not packs).
    pub fn min_arity(&self) -> usize {
        if self.unprototyped {
            return 0;
        }
        self.params
            .iter()
            .filter(|p| !p.has_default && !p.ty.pack)
            .count()
    }

    /// Largest accepted argument count; `None` when unbounded (ellipsis, pack, no prototype).
    pub fn max_arity(&self) -> Option<usize> {
        if self.variadic || self.unprototyped || self.params.iter().any(|p| p.ty.pack) {
            None
        } else {
            Some(self.params.len())
        }
    }

    pub fn accepts_arity(&self, n: usize) -> bool {
        n >= self.min_arity() && self.max_arity().map_or(true, |m| n <= m)
    }
}

/// A class, struct, union or enum specifier met inside a decl-specifier sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagSpec {
    pub key: TagKey,
    pub name: Option<QualName>,
    /// Record index of the class-key token.
    pub first_token: u32,
    /// Record indices of `{` and `}` of the member list, when the specifier defines the type.
    pub body: Option<(u32, u32)>,
    /// `enum class` / `enum struct`.
    pub scoped_enum: bool,
}

/// A decl-specifier sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclSpecifiers {
    pub storage: u16,
    pub specs: u16,
    /// String of `extern "C"` / `extern "C++"`.
    pub extern_lang: Option<String>,
    pub base: BaseType,
    pub cv: u8,
    pub tag: Option<TagSpec>,
    /// C89 implicit `int` (no type specifier but other specifiers present).
    pub implicit_int: bool,
    /// `[[deprecated]]` / `__attribute__((deprecated))` / `__declspec(deprecated)`.
    pub deprecated: bool,
}

impl Default for DeclSpecifiers {
    fn default() -> Self {
        Self {
            storage: 0,
            specs: 0,
            extern_lang: None,
            base: BaseType::None,
            cv: 0,
            tag: None,
            implicit_int: false,
            deprecated: false,
        }
    }
}

/// What a declarator declares as its name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NameKind {
    Identifier,
    Destructor,
    /// Operator function; the payload is the operator spelling (`+`, `()`, `new[]`, `""_km`).
    Operator(String),
    /// Conversion function `operator T()`.
    Conversion(Box<TypeSpec>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitKind {
    None,
    Expression,
    Braced,
    Parenthesized,
    PureVirtual,
    Defaulted,
    Deleted,
}

/// One init-declarator of a declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declarator {
    pub name: Option<QualName>,
    pub name_kind: NameKind,
    pub name_token: u32,
    pub ty: TypeSpec,
    pub init: InitKind,
    pub bitfield: bool,
    /// One name of a C++17 structured binding (`auto [a, b] = ...`).
    pub structured_binding: bool,
    pub first_token: u32,
    pub last_token: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplateParamKind {
    Type,
    NonType,
    Template,
}

/// One template parameter of a template header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateParamSpec {
    pub kind: TemplateParamKind,
    pub name: Option<String>,
    pub token: u32,
    pub pack: bool,
    pub default: Option<String>,
    /// Type of a non-type parameter.
    pub ty: Option<TypeSpec>,
    /// Concept of a constrained type parameter (`std::integral T`).
    pub constraint: Option<QualName>,
}

/// A simple-declaration or function-definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedDeclaration {
    /// Template headers written before the declaration, outermost first. An empty header
    /// (`template<>`) introduces an explicit specialization.
    pub templates: Vec<Vec<TemplateParamSpec>>,
    pub specifiers: DeclSpecifiers,
    pub declarators: Vec<Declarator>,
    /// Record indices of `{` and `}` of a function body.
    pub body: Option<(u32, u32)>,
    /// The definition has a constructor mem-initializer list.
    pub ctor_initializer: bool,
    pub first_token: u32,
    pub last_token: u32,
}

/// A base-specifier of a class head.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaseSpec {
    pub name: QualName,
    /// Written access; `None` means the default of the class key.
    pub access: Option<Access>,
    pub is_virtual: bool,
    pub pack: bool,
    pub token: u32,
}

/// A class-head and the position of its member list in the parser's token vector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassHead {
    pub templates: Vec<Vec<TemplateParamSpec>>,
    pub key: TagKey,
    pub name: Option<QualName>,
    pub is_final: bool,
    pub bases: Vec<BaseSpec>,
    /// Parser positions of `{` and `}`.
    pub body: Option<(usize, usize)>,
    /// The specifier is the subject of a `friend` declaration.
    pub friend: bool,
    pub first_token: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnumeratorSpec {
    pub name: String,
    pub token: u32,
    pub value: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnumHead {
    pub scoped: bool,
    pub name: Option<QualName>,
    pub underlying: Option<TypeSpec>,
    /// Parser positions of `{` and `}`; `None` for an opaque declaration.
    pub body: Option<(usize, usize)>,
    pub enumerators: Vec<EnumeratorSpec>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamespaceHead {
    /// Components of the (possibly nested, C++17) namespace name with their `inline` flag;
    /// empty for an unnamed namespace.
    pub parts: Vec<(String, u32, bool)>,
    /// `inline namespace` written before an unnamed namespace.
    pub inline_unnamed: bool,
    /// `namespace X = A::B;`.
    pub alias_target: Option<QualName>,
    pub body: Option<(usize, usize)>,
}

/// The forms introduced by the `using` keyword.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UsingForm {
    /// `using namespace N;`
    Directive { target: QualName },
    /// `using N::x;` / `using typename B::type;` / `using Base::Base;`
    Declaration { target: QualName, typename: bool },
    /// `using T = type-id;` (an alias template when preceded by a template header).
    Alias {
        name: String,
        token: u32,
        target: TypeSpec,
    },
    /// `using enum E;` (C++20).
    Enum { target: QualName },
}

/// A `friend` declaration found in a member list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FriendSpec {
    Class {
        name: QualName,
        token: u32,
    },
    Function {
        name: QualName,
        token: u32,
        arity: usize,
        has_body: bool,
    },
}

/// The parts of a lambda-expression that open scopes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LambdaHead {
    pub templates: Vec<TemplateParamSpec>,
    pub params: Option<FunctionShape>,
    /// Record indices of `{` and `}` of the body.
    pub body: Option<(u32, u32)>,
}

/// A declaration the grammar could not parse; `token` is where parsing stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclError {
    pub token: u32,
    pub reason: String,
}

impl fmt::Display for DeclError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.token == u32::MAX {
            write!(f, "{}", self.reason)
        } else {
            write!(f, "{} (token {})", self.reason, self.token)
        }
    }
}

impl std::error::Error for DeclError {}

/// Answers whether a name denotes a type, which the C/C++ grammar needs to tell
/// declarations from expressions (`T * x;`, `T x(a);`).
pub trait TypeOracle {
    fn is_type_name(&self, name: &str) -> bool;
    /// Object-like macros whose expansion is empty or only attributes (`DLL_EXPORT`); the
    /// grammar skips them where decl-specifiers are expected.
    fn is_ignorable_macro(&self, _name: &str) -> bool {
        false
    }
}

/// Oracle that knows no type names (catalog text, name parsing).
pub struct NoTypeOracle;

impl TypeOracle for NoTypeOracle {
    fn is_type_name(&self, _name: &str) -> bool {
        false
    }
}

/// Parse settings of one declaration.
#[derive(Clone, Copy)]
pub struct DeclContext<'p> {
    pub dialect: Dialect,
    pub oracle: &'p dyn TypeOracle,
    /// Name of the class whose member list is being parsed (constructor recognition).
    pub class_name: Option<&'p str>,
    /// The AST node is known to declare a function (`T f(x);` is then a prototype).
    pub prefer_function: bool,
    /// Declarations at block scope prefer the object reading of `T x(a);`.
    pub block_scope: bool,
}

impl<'p> DeclContext<'p> {
    pub fn plain(dialect: Dialect, oracle: &'p dyn TypeOracle) -> Self {
        Self {
            dialect,
            oracle,
            class_name: None,
            prefer_function: false,
            block_scope: false,
        }
    }
}

// ── Declaration grammar: parser ──────────────────────────────────────────────

const CALLING_CONVENTIONS: &[&str] = &[
    "__cdecl",
    "__stdcall",
    "__fastcall",
    "__thiscall",
    "__vectorcall",
    "__clrcall",
    "_cdecl",
    "_stdcall",
    "WINAPI",
    "APIENTRY",
    "CALLBACK",
];

const IGNORED_QUALIFIER_WORDS: &[&str] = &[
    "__extension__",
    "__w64",
    "__ptr32",
    "__ptr64",
    "__unaligned",
    "_Nullable",
    "_Nonnull",
    "_Null_unspecified",
    "__nullable",
    "__nonnull",
    "__kindof",
];

const ATTRIBUTE_CALLS: &[&str] = &[
    "__attribute__",
    "__attribute",
    "__declspec",
    "alignas",
    "_Alignas",
    "__asm__",
    "__asm",
    "asm",
    "__pragma",
    "_Pragma",
];

fn is_operator_symbol(t: &str) -> bool {
    matches!(
        t,
        "+" | "-"
            | "*"
            | "/"
            | "%"
            | "^"
            | "&"
            | "|"
            | "~"
            | "!"
            | "="
            | "<"
            | ">"
            | "+="
            | "-="
            | "*="
            | "/="
            | "%="
            | "^="
            | "&="
            | "|="
            | "<<="
            | ">>="
            | "=="
            | "!="
            | "<="
            | ">="
            | "<=>"
            | "&&"
            | "||"
            | "++"
            | "--"
            | ","
            | "->*"
            | "->"
            | "<<"
            | ">>"
    )
}

/// Spelling of an operator function name: `operator+`, `operator new[]`, `operator()`.
pub fn operator_function_name(op: &str) -> String {
    if op.starts_with(|c: char| c.is_alphabetic()) {
        format!("operator {}", op)
    } else {
        format!("operator{}", op)
    }
}

/// Recursive-descent parser of C and C++ declarations over a significant-token slice.
/// Positions are indices into the parser's own token vector (`>>` is split into two `>`
/// so that nested template argument lists close correctly).
pub struct DeclParser<'p, 'a> {
    toks: Vec<CTok<'a>>,
    pos: usize,
    cx: DeclContext<'p>,
    deprecated: bool,
}

impl<'p, 'a> DeclParser<'p, 'a> {
    pub fn new(toks: &[CTok<'a>], cx: DeclContext<'p>) -> Self {
        let mut v = Vec::with_capacity(toks.len() + 4);
        for t in toks {
            if t.text == ">>" && cx.dialect == Dialect::Cpp {
                let mut a = *t;
                a.text = ">";
                a.len = 1;
                let mut b = a;
                b.col = t.col.saturating_add(1);
                v.push(a);
                v.push(b);
            } else {
                v.push(*t);
            }
        }
        Self {
            toks: v,
            pos: 0,
            cx,
            deprecated: false,
        }
    }

    fn sub(&self, from: usize, to: usize) -> DeclParser<'p, 'a> {
        let to = to.min(self.toks.len());
        let from = from.min(to);
        DeclParser {
            toks: self.toks[from..to].to_vec(),
            pos: 0,
            cx: self.cx,
            deprecated: false,
        }
    }

    pub fn tokens(&self) -> &[CTok<'a>] {
        &self.toks
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn set_pos(&mut self, pos: usize) {
        self.pos = pos.min(self.toks.len());
    }

    pub fn at_end(&self) -> bool {
        self.pos >= self.toks.len()
    }

    fn text(&self, k: usize) -> &'a str {
        self.text_at(self.pos + k)
    }

    pub fn text_at(&self, p: usize) -> &'a str {
        self.toks.get(p).map(|t| t.text).unwrap_or("")
    }

    pub fn at(&self, s: &str) -> bool {
        self.text(0) == s
    }

    pub fn eat(&mut self, s: &str) -> bool {
        if self.at(s) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    /// Record index of the token at parser position `p` (`u32::MAX` past the end).
    pub fn index_at(&self, p: usize) -> u32 {
        self.toks.get(p).map(|t| t.index).unwrap_or(u32::MAX)
    }

    fn cur_index(&self) -> u32 {
        self.index_at(self.pos)
    }

    fn last_index(&self) -> u32 {
        if self.pos == 0 {
            self.index_at(0)
        } else {
            self.index_at(self.pos.min(self.toks.len()) - 1)
        }
    }

    fn ident_at(&self, p: usize) -> bool {
        is_identifier(self.text_at(p), self.cx.dialect)
    }

    fn cpp(&self) -> bool {
        self.cx.dialect == Dialect::Cpp
    }

    fn error(&self, reason: &str) -> DeclError {
        DeclError {
            token: self.cur_index(),
            reason: reason.to_string(),
        }
    }

    /// Joins the token texts of positions `from..to`.
    pub fn join(&self, from: usize, to: usize) -> String {
        let to = to.min(self.toks.len());
        let mut s = String::new();
        for p in from..to {
            let t = &self.toks[p];
            if p > from {
                let prev = &self.toks[p - 1];
                if !prev.adjacent_to(t) {
                    s.push(' ');
                }
            }
            s.push_str(t.text);
        }
        s
    }

    /// Position of the bracket closing the `(`, `[` or `{` at `open`.
    pub fn matching(&self, open: usize) -> Option<usize> {
        if !matches!(self.text_at(open), "(" | "[" | "{") {
            return None;
        }
        let mut stack: Vec<&str> = Vec::new();
        for p in open..self.toks.len() {
            let t = self.toks[p].text;
            match t {
                "(" | "[" | "{" => stack.push(t),
                ")" | "]" | "}" => {
                    let want = match t {
                        ")" => "(",
                        "]" => "[",
                        _ => "{",
                    };
                    if stack.pop() != Some(want) {
                        return None;
                    }
                    if stack.is_empty() {
                        return Some(p);
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// Position of the `>` closing the template argument (or parameter) list at `open`.
    pub fn matching_angle(&self, open: usize) -> Option<usize> {
        if self.text_at(open) != "<" || !self.cpp() {
            return None;
        }
        let mut depth = 0i32;
        let mut p = open;
        while p < self.toks.len() {
            match self.toks[p].text {
                "<" => depth += 1,
                ">" => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(p);
                    }
                }
                "(" | "[" => {
                    p = self.matching(p)?;
                }
                ";" | "{" | "}" | ")" | "]" => return None,
                _ => {}
            }
            p += 1;
        }
        None
    }

    /// First position at or after the cursor holding one of `stops` outside brackets.
    fn find_at_depth0(&self, stops: &[&str]) -> Option<usize> {
        let mut p = self.pos;
        while p < self.toks.len() {
            let t = self.toks[p].text;
            if stops.contains(&t) {
                return Some(p);
            }
            if matches!(t, "(" | "[" | "{") {
                p = self.matching(p)? + 1;
                continue;
            }
            p += 1;
        }
        None
    }

    /// Splits `from..to` at commas outside brackets (and outside template argument lists
    /// when `angles`). Empty segments are dropped.
    pub fn split_commas(&self, from: usize, to: usize, angles: bool) -> Vec<(usize, usize)> {
        let to = to.min(self.toks.len());
        let mut segs = Vec::new();
        if from >= to {
            return segs;
        }
        let mut start = from;
        let mut p = from;
        while p < to {
            let t = self.toks[p].text;
            if matches!(t, "(" | "[" | "{") {
                match self.matching(p) {
                    Some(c) if c < to => {
                        p = c + 1;
                        continue;
                    }
                    _ => {
                        p = to;
                        break;
                    }
                }
            }
            if angles && t == "<" && p > from {
                let prev = self.text_at(p - 1);
                if is_identifier(prev, self.cx.dialect) || prev == ">" {
                    if let Some(c) = self.matching_angle(p) {
                        if c < to {
                            p = c + 1;
                            continue;
                        }
                    }
                }
            }
            if t == "," {
                if p > start {
                    segs.push((start, p));
                }
                start = p + 1;
            }
            p += 1;
        }
        if start < to {
            segs.push((start, to));
        }
        segs
    }

    /// Skips attribute and vendor-extension syntax. Returns true when one of them was
    /// a deprecation attribute.
    pub fn skip_attributes(&mut self) -> bool {
        let mut deprecated = false;
        loop {
            let t = self.text(0);
            if t.is_empty() {
                break;
            }
            if t == "[" && self.text(1) == "[" {
                match self.matching(self.pos) {
                    Some(c) => {
                        if self.join(self.pos, c).contains("deprecated") {
                            deprecated = true;
                        }
                        self.pos = c + 1;
                        continue;
                    }
                    None => break,
                }
            }
            if ATTRIBUTE_CALLS.contains(&t) && self.text(1) == "(" {
                match self.matching(self.pos + 1) {
                    Some(c) => {
                        if self.join(self.pos, c).contains("deprecated") {
                            deprecated = true;
                        }
                        self.pos = c + 1;
                        continue;
                    }
                    None => break,
                }
            }
            if IGNORED_QUALIFIER_WORDS.contains(&t) || CALLING_CONVENTIONS.contains(&t) {
                self.pos += 1;
                continue;
            }
            if self.cx.oracle.is_ignorable_macro(t) && is_identifier(t, self.cx.dialect) {
                self.pos += 1;
                if self.at("(") {
                    if let Some(c) = self.matching(self.pos) {
                        self.pos = c + 1;
                    }
                }
                continue;
            }
            break;
        }
        if deprecated {
            self.deprecated = true;
        }
        deprecated
    }

    fn parse_cv(&mut self) -> u8 {
        let mut cv = 0u8;
        loop {
            self.skip_attributes();
            match self.text(0) {
                "const" | "__const" => cv |= CV_CONST,
                "volatile" | "__volatile__" | "__volatile" => cv |= CV_VOLATILE,
                "restrict" | "__restrict" | "__restrict__" => cv |= CV_RESTRICT,
                "_Atomic" if self.text(1) != "(" => cv |= CV_ATOMIC,
                _ => break,
            }
            self.pos += 1;
        }
        cv
    }

    fn parse_template_args(&self, open: usize, close: usize) -> Vec<TemplateArg> {
        self.split_commas(open + 1, close, true)
            .into_iter()
            .map(|(a, b)| {
                let mut sub = self.sub(a, b);
                match sub.parse_type_id() {
                    Ok(ts) if sub.at_end() => TemplateArg::Type(ts),
                    _ => TemplateArg::Expr(self.join(a, b)),
                }
            })
            .collect()
    }

    fn starts_id(&self) -> bool {
        let t = self.text(0);
        is_identifier(t, self.cx.dialect)
            || (t == "::" && (self.ident_at(self.pos + 1) || self.text(1) == "operator"))
            || (self.cpp() && t == "~" && self.ident_at(self.pos + 1))
            || (self.cpp() && t == "operator")
    }

    /// Parses an id-expression: an optionally qualified identifier with template
    /// arguments, a destructor name, an operator function name or a conversion function
    /// name. Restores the cursor and returns `None` when none starts here.
    pub fn parse_id_expression(&mut self) -> Option<(QualName, NameKind)> {
        let start = self.pos;
        let global = self.at("::")
            && matches!(self.text(1), "~" | "operator" | "template")
            || (self.at("::") && self.ident_at(self.pos + 1));
        if global {
            self.pos += 1;
        }
        let mut parts: Vec<NamePart> = Vec::new();
        loop {
            if self.cpp() && self.at("template") && self.ident_at(self.pos + 1) {
                self.pos += 1;
            }
            let t = self.text(0);
            if self.cpp() && t == "~" && self.ident_at(self.pos + 1) {
                let token = self.index_at(self.pos + 1);
                let name = format!("~{}", self.text(1));
                self.pos += 2;
                if self.at("<") {
                    if let Some(c) = self.matching_angle(self.pos) {
                        self.pos = c + 1;
                    }
                }
                parts.push(NamePart {
                    name,
                    token,
                    args: None,
                });
                return Some((QualName { global, parts }, NameKind::Destructor));
            }
            if self.cpp() && t == "operator" {
                let token = self.cur_index();
                self.pos += 1;
                match self.parse_operator_name() {
                    Some(Ok(op)) => {
                        let mut args = None;
                        if self.at("<") {
                            if let Some(c) = self.matching_angle(self.pos) {
                                args = Some(self.parse_template_args(self.pos, c));
                                self.pos = c + 1;
                            }
                        }
                        parts.push(NamePart {
                            name: operator_function_name(&op),
                            token,
                            args,
                        });
                        return Some((QualName { global, parts }, NameKind::Operator(op)));
                    }
                    Some(Err(ts)) => {
                        parts.push(NamePart {
                            name: format!("operator {}", ts.spelled()),
                            token,
                            args: None,
                        });
                        return Some((
                            QualName { global, parts },
                            NameKind::Conversion(Box::new(ts)),
                        ));
                    }
                    None => {
                        self.pos = start;
                        return None;
                    }
                }
            }
            if !is_identifier(t, self.cx.dialect) {
                self.pos = start;
                return None;
            }
            let token = self.cur_index();
            self.pos += 1;
            let mut args = None;
            if self.cpp() && self.at("<") {
                if let Some(c) = self.matching_angle(self.pos) {
                    args = Some(self.parse_template_args(self.pos, c));
                    self.pos = c + 1;
                }
            }
            parts.push(NamePart {
                name: t.to_string(),
                token,
                args,
            });
            if self.at("::") {
                let n = self.text(1);
                if is_identifier(n, self.cx.dialect)
                    || (self.cpp() && matches!(n, "~" | "operator" | "template"))
                {
                    self.pos += 1;
                    continue;
                }
            }
            return Some((QualName { global, parts }, NameKind::Identifier));
        }
    }

    /// After `operator`: `Ok(spelling)` for an operator function, `Err(type)` for a
    /// conversion function, `None` when nothing valid follows.
    fn parse_operator_name(&mut self) -> Option<Result<String, TypeSpec>> {
        let t = self.text(0);
        match t {
            "new" | "delete" => {
                self.pos += 1;
                if self.at("[") && self.text(1) == "]" {
                    self.pos += 2;
                    return Some(Ok(format!("{}[]", t)));
                }
                Some(Ok(t.to_string()))
            }
            "co_await" => {
                self.pos += 1;
                Some(Ok(t.to_string()))
            }
            "(" if self.text(1) == ")" => {
                self.pos += 2;
                Some(Ok("()".to_string()))
            }
            "[" if self.text(1) == "]" => {
                self.pos += 2;
                Some(Ok("[]".to_string()))
            }
            ">" => {
                let adjacent = self
                    .toks
                    .get(self.pos + 1)
                    .is_some_and(|n| n.text == ">" && self.toks[self.pos].adjacent_to(n));
                if adjacent {
                    self.pos += 2;
                    return Some(Ok(">>".to_string()));
                }
                self.pos += 1;
                Some(Ok(">".to_string()))
            }
            "\"" if self.text(1) == "\"" && self.ident_at(self.pos + 2) => {
                let s = format!("\"\"{}", self.text(2));
                self.pos += 3;
                Some(Ok(s))
            }
            _ if t.starts_with("\"\"") => {
                self.pos += 1;
                if t.len() > 2 {
                    return Some(Ok(t.to_string()));
                }
                if self.ident_at(self.pos) {
                    let s = format!("\"\"{}", self.text(0));
                    self.pos += 1;
                    return Some(Ok(s));
                }
                None
            }
            _ if is_operator_symbol(t) => {
                self.pos += 1;
                Some(Ok(t.to_string()))
            }
            _ => {
                let save = self.pos;
                let specs = self.parse_decl_specifiers(true);
                if specs.base == BaseType::None {
                    self.pos = save;
                    return None;
                }
                let mut derivations = Vec::new();
                loop {
                    match self.text(0) {
                        "*" => {
                            self.pos += 1;
                            let cv = self.parse_cv();
                            derivations.push(Derivation::Pointer { cv });
                        }
                        "&" => {
                            self.pos += 1;
                            derivations.push(Derivation::LValueRef);
                        }
                        "&&" => {
                            self.pos += 1;
                            derivations.push(Derivation::RValueRef);
                        }
                        _ => break,
                    }
                }
                derivations.reverse();
                Some(Err(TypeSpec {
                    base: specs.base,
                    cv: specs.cv,
                    derivations,
                    pack: false,
                }))
            }
        }
    }

    /// True when the identifier at the cursor begins the declarator rather than naming
    /// the type of a declaration without any type specifier so far.
    fn declarator_starts_here(&mut self, abstract_ok: bool) -> bool {
        let save = self.pos;
        let parsed = self.parse_id_expression();
        let next = self.text(0);
        self.pos = save;
        let (qn, kind) = match parsed {
            Some(x) => x,
            None => return false,
        };
        if !matches!(kind, NameKind::Identifier) {
            return true;
        }
        let last = qn.last_name();
        if next == "(" {
            if let Some(cls) = self.cx.class_name {
                if last == cls && qn.parts.len() == 1 {
                    return true;
                }
            }
            if qn.parts.len() >= 2 && qn.parts[qn.parts.len() - 2].name == last {
                return true;
            }
            // C89 implicit int: `main() { ... }`, `f(a, b) int a; { ... }`.
            return self.cx.dialect == Dialect::C
                && !abstract_ok
                && !self.cx.oracle.is_type_name(last);
        }
        match next {
            ";" | "=" | "[" | ":" | "{" => !abstract_ok,
            "," | ")" | ">" | "" => !abstract_ok,
            _ => false,
        }
    }

    /// Parses a decl-specifier sequence. With `abstract_ok` the sequence may be followed
    /// by an abstract declarator (parameters, type-ids), so a lone name is a type.
    pub fn parse_decl_specifiers(&mut self, abstract_ok: bool) -> DeclSpecifiers {
        let mut ds = DeclSpecifiers::default();
        let mut words: Vec<&'a str> = Vec::new();
        let mut have_named = false;
        self.deprecated = false;
        loop {
            self.skip_attributes();
            let t = self.text(0);
            if t.is_empty() {
                break;
            }
            let has_type = !words.is_empty() || have_named;
            match t {
                "typedef" => ds.storage |= SC_TYPEDEF,
                "static" => ds.storage |= SC_STATIC,
                "extern" => {
                    ds.storage |= SC_EXTERN;
                    if let Some(n) = self.toks.get(self.pos + 1) {
                        if n.kind == TokenType::StringLiteral || n.text.starts_with('"') {
                            ds.extern_lang = Some(n.text.trim_matches('"').to_string());
                            self.pos += 1;
                        }
                    }
                }
                "register" => ds.storage |= SC_REGISTER,
                "thread_local" | "_Thread_local" | "__thread" => ds.storage |= SC_THREAD_LOCAL,
                "mutable" if self.cpp() => ds.storage |= SC_MUTABLE,
                "auto" => {
                    if self.cx.dialect == Dialect::C {
                        ds.storage |= SC_AUTO;
                    } else if has_type {
                        break;
                    } else {
                        ds.base = BaseType::Auto;
                        have_named = true;
                    }
                }
                "inline" | "__inline" | "__inline__" | "__forceinline" => ds.specs |= FS_INLINE,
                "virtual" if self.cpp() => ds.specs |= FS_VIRTUAL,
                "explicit" if self.cpp() => {
                    ds.specs |= FS_EXPLICIT;
                    if self.text(1) == "(" {
                        if let Some(c) = self.matching(self.pos + 1) {
                            self.pos = c;
                        }
                    }
                }
                "constexpr" => ds.specs |= FS_CONSTEXPR,
                "consteval" if self.cpp() => ds.specs |= FS_CONSTEVAL,
                "constinit" if self.cpp() => ds.specs |= FS_CONSTINIT,
                "friend" if self.cpp() => ds.specs |= FS_FRIEND,
                "_Noreturn" => ds.specs |= FS_NORETURN,
                "const" | "__const" => ds.cv |= CV_CONST,
                "volatile" | "__volatile__" | "__volatile" => ds.cv |= CV_VOLATILE,
                "restrict" | "__restrict" | "__restrict__" => ds.cv |= CV_RESTRICT,
                "_Atomic" => {
                    if self.text(1) == "(" && !has_type {
                        let open = self.pos + 1;
                        if let Some(c) = self.matching(open) {
                            let mut sub = self.sub(open + 1, c);
                            if let Ok(ts) = sub.parse_type_id() {
                                ds.base = ts.base;
                                ds.cv |= ts.cv | CV_ATOMIC;
                                have_named = true;
                                self.pos = c + 1;
                                continue;
                            }
                        }
                        break;
                    }
                    ds.cv |= CV_ATOMIC;
                }
                "struct" | "union" | "enum" | "class" => {
                    if has_type || (t == "class" && !self.cpp()) {
                        break;
                    }
                    let save = self.pos;
                    match self.parse_tag_specifier() {
                        Some(tag) => {
                            ds.base = BaseType::Named {
                                key: Some(tag.key),
                                name: tag.name.clone().unwrap_or_default(),
                            };
                            ds.tag = Some(tag);
                            have_named = true;
                            continue;
                        }
                        None => {
                            self.pos = save;
                            break;
                        }
                    }
                }
                "typename" if self.cpp() => {
                    if has_type {
                        break;
                    }
                    let save = self.pos;
                    self.pos += 1;
                    match self.parse_id_expression() {
                        Some((qn, NameKind::Identifier)) => {
                            ds.base = BaseType::Named { key: None, name: qn };
                            have_named = true;
                            continue;
                        }
                        _ => {
                            self.pos = save;
                            break;
                        }
                    }
                }
                "decltype" | "typeof" | "__typeof__" | "__typeof" | "typeof_unqual"
                    if self.text(1) == "(" =>
                {
                    if has_type {
                        break;
                    }
                    let open = self.pos + 1;
                    let c = match self.matching(open) {
                        Some(c) => c,
                        None => break,
                    };
                    let inner = self.join(open + 1, c);
                    ds.base = if t == "decltype" && inner == "auto" {
                        BaseType::DecltypeAuto
                    } else if t == "decltype" {
                        BaseType::Decltype(inner)
                    } else {
                        let mut sub = self.sub(open + 1, c);
                        match sub.parse_type_id() {
                            Ok(ts) if sub.at_end() && ts.derivations.is_empty() => {
                                ds.cv |= ts.cv;
                                ts.base
                            }
                            _ => BaseType::Decltype(inner),
                        }
                    };
                    have_named = true;
                    self.pos = c + 1;
                    continue;
                }
                w if FundamentalType::is_specifier_word(w, self.cx.dialect) => {
                    if have_named {
                        break;
                    }
                    words.push(w);
                }
                _ if t == "::" || is_identifier(t, self.cx.dialect) => {
                    if has_type || self.declarator_starts_here(abstract_ok) {
                        break;
                    }
                    let save = self.pos;
                    match self.parse_id_expression() {
                        Some((qn, NameKind::Identifier)) => {
                            ds.base = BaseType::Named { key: None, name: qn };
                            have_named = true;
                            continue;
                        }
                        _ => {
                            self.pos = save;
                            break;
                        }
                    }
                }
                _ => break,
            }
            self.pos += 1;
        }
        if !words.is_empty() {
            ds.base = match FundamentalType::from_words(&words, self.cx.dialect) {
                Some(f) => BaseType::Fundamental(f),
                None => BaseType::Unknown(words.join(" ")),
            };
        } else if !have_named
            && self.cx.dialect == Dialect::C
            && (ds.storage != 0 || ds.specs != 0 || ds.cv != 0)
        {
            ds.base = BaseType::Fundamental(FundamentalType::Int);
            ds.implicit_int = true;
        }
        ds.deprecated = self.deprecated;
        ds
    }

    /// At `struct` / `class` / `union` / `enum`: parses the specifier, skipping (but
    /// recording) a member list.
    fn parse_tag_specifier(&mut self) -> Option<TagSpec> {
        let first_token = self.cur_index();
        let kw = self.text(0);
        self.pos += 1;
        let key = match kw {
            "struct" => TagKey::Struct,
            "class" => TagKey::Class,
            "union" => TagKey::Union,
            _ => TagKey::Enum,
        };
        let mut scoped_enum = false;
        if key == TagKey::Enum && self.cpp() && (self.at("class") || self.at("struct")) {
            scoped_enum = true;
            self.pos += 1;
        }
        self.skip_attributes();
        let save = self.pos;
        let name = match self.parse_id_expression() {
            Some((qn, NameKind::Identifier)) => Some(qn),
            _ => {
                self.pos = save;
                None
            }
        };
        self.skip_attributes();
        while matches!(self.text(0), "final" | "sealed" | "__final")
            && matches!(self.text(1), "{" | ":")
        {
            self.pos += 1;
        }
        if self.at(":") {
            // Base clause or enum-base: skipped up to the member list or the end.
            let save_colon = self.pos;
            self.pos += 1;
            match self.find_at_depth0(&["{", ";"]) {
                Some(p) if self.text_at(p) == "{" => self.pos = p,
                Some(p) if key == TagKey::Enum && self.text_at(p) == ";" => self.pos = p,
                _ => self.pos = save_colon,
            }
        }
        let body = if self.at("{") {
            let open = self.pos;
            let close = self.matching(open)?;
            self.pos = close + 1;
            Some((self.index_at(open), self.index_at(close)))
        } else {
            None
        };
        if name.is_none() && body.is_none() {
            return None;
        }
        Some(TagSpec {
            key,
            name,
            first_token,
            body,
            scoped_enum,
        })
    }

    fn member_pointer_at(&self, mut p: usize) -> bool {
        if !self.cpp() {
            return false;
        }
        if self.text_at(p) == "::" {
            p += 1;
        }
        loop {
            if !self.ident_at(p) {
                return false;
            }
            p += 1;
            if self.text_at(p) == "<" {
                match self.matching_angle(p) {
                    Some(c) => p = c + 1,
                    None => return false,
                }
            }
            if self.text_at(p) != "::" {
                return false;
            }
            p += 1;
            if self.text_at(p) == "*" {
                return true;
            }
        }
    }

    fn paren_is_nested_declarator(&self, abstract_ok: bool) -> bool {
        let mut p = self.pos + 1;
        while CALLING_CONVENTIONS.contains(&self.text_at(p)) {
            p += 1;
        }
        let t = self.text_at(p);
        if matches!(t, "*" | "^") {
            return true;
        }
        if self.cpp() && matches!(t, "&" | "&&") {
            return true;
        }
        if self.member_pointer_at(p) {
            return true;
        }
        if !abstract_ok
            && is_identifier(t, self.cx.dialect)
            && !self.cx.oracle.is_type_name(t)
            && matches!(self.text_at(p + 1), ")" | "[")
        {
            return true;
        }
        !abstract_ok && t == "("
    }

    /// Decides whether the `(` at `open`, directly after a declarator name, starts an
    /// initializer (`T x(a);`) rather than a parameter list (`T f(A);`).
    fn looks_like_initializer(&self, open: usize) -> bool {
        let close = match self.matching(open) {
            Some(c) => c,
            None => return false,
        };
        if close == open + 1 {
            return false;
        }
        let mut strong_expr = false;
        let mut weak_expr = false;
        let mut param = false;
        for (a, b) in self.split_commas(open + 1, close, true) {
            let first = &self.toks[a];
            let t = first.text;
            if first.is_literal()
                || matches!(
                    t,
                    "this"
                        | "nullptr"
                        | "true"
                        | "false"
                        | "new"
                        | "sizeof"
                        | "alignof"
                        | "_Alignof"
                        | "static_cast"
                        | "dynamic_cast"
                        | "const_cast"
                        | "reinterpret_cast"
                        | "typeid"
                        | "-"
                        | "+"
                        | "!"
                        | "~"
                        | "&"
                        | "*"
                        | "("
                        | "["
                        | "{"
                        | "++"
                        | "--"
                        | "NULL"
                )
            {
                strong_expr = true;
                continue;
            }
            if t == "..."
                || FundamentalType::is_specifier_word(t, self.cx.dialect)
                || is_c_family_keyword(t, self.cx.dialect)
            {
                param = true;
                continue;
            }
            let mut has_op = false;
            for p in a..b {
                if matches!(
                    self.text_at(p),
                    "." | "->" | "+" | "-" | "/" | "%" | "==" | "!=" | "||" | "?" | "|" | "^"
                ) {
                    has_op = true;
                    break;
                }
            }
            if has_op {
                strong_expr = true;
                continue;
            }
            if b - a == 1 {
                if self.cx.oracle.is_type_name(t) {
                    param = true;
                } else {
                    weak_expr = true;
                }
                continue;
            }
            let last = self.text_at(b - 1);
            let before_last = self.text_at(b - 2);
            if is_identifier(last, self.cx.dialect)
                && (is_identifier(before_last, self.cx.dialect)
                    || matches!(before_last, "*" | "&" | "&&" | ">"))
            {
                param = true;
                continue;
            }
            let mut sub = self.sub(a, b);
            match sub.parse_id_expression() {
                Some((qn, _)) if self.cx.oracle.is_type_name(qn.last_name()) => param = true,
                _ => weak_expr = true,
            }
        }
        if param && !strong_expr && !weak_expr {
            return false;
        }
        if self.cx.prefer_function {
            return strong_expr && !param;
        }
        if (strong_expr || weak_expr) && !param {
            return true;
        }
        self.cx.block_scope
    }

    /// Parses a declarator (abstract when `abstract_ok` and no name is present).
    pub fn parse_declarator(&mut self, abstract_ok: bool) -> Result<DeclCore, DeclError> {
        let mut prefix: Vec<Derivation> = Vec::new();
        let mut pack = false;
        loop {
            self.skip_attributes();
            match self.text(0) {
                "*" => {
                    self.pos += 1;
                    let cv = self.parse_cv();
                    prefix.push(Derivation::Pointer { cv });
                }
                "^" => {
                    self.pos += 1;
                    let cv = self.parse_cv();
                    prefix.push(Derivation::Pointer { cv });
                }
                "&" if self.cpp() => {
                    self.pos += 1;
                    prefix.push(Derivation::LValueRef);
                }
                "&&" if self.cpp() => {
                    self.pos += 1;
                    prefix.push(Derivation::RValueRef);
                }
                "..." if self.cpp() => {
                    self.pos += 1;
                    pack = true;
                }
                _ if self.member_pointer_at(self.pos) => {
                    let mut class = QualName {
                        global: self.eat("::"),
                        parts: Vec::new(),
                    };
                    while self.ident_at(self.pos) {
                        let token = self.cur_index();
                        let name = self.text(0).to_string();
                        self.pos += 1;
                        let mut args = None;
                        if self.at("<") {
                            if let Some(c) = self.matching_angle(self.pos) {
                                args = Some(self.parse_template_args(self.pos, c));
                                self.pos = c + 1;
                            }
                        }
                        class.parts.push(NamePart { name, token, args });
                        self.eat("::");
                        if self.at("*") {
                            break;
                        }
                    }
                    self.eat("*");
                    let cv = self.parse_cv();
                    prefix.push(Derivation::MemberPointer { class, cv });
                }
                _ => break,
            }
        }
        self.skip_attributes();
        let mut core = DeclCore {
            name: None,
            kind: NameKind::Identifier,
            token: u32::MAX,
            derivations: Vec::new(),
            pack,
        };
        if self.at("(") && self.paren_is_nested_declarator(abstract_ok) {
            let open = self.pos;
            let close = self
                .matching(open)
                .ok_or_else(|| self.error("unbalanced parenthesis in declarator"))?;
            let mut sub = self.sub(open + 1, close);
            while CALLING_CONVENTIONS.contains(&sub.text(0)) {
                sub.pos += 1;
            }
            let inner = sub.parse_declarator(abstract_ok)?;
            core.name = inner.name;
            core.kind = inner.kind;
            core.token = inner.token;
            core.derivations = inner.derivations;
            core.pack |= inner.pack;
            self.pos = close + 1;
        } else if self.starts_id() {
            let save = self.pos;
            match self.parse_id_expression() {
                Some((qn, kind)) => {
                    core.token = qn.last_token();
                    core.name = Some(qn);
                    core.kind = kind;
                }
                None => {
                    self.pos = save;
                    if !abstract_ok {
                        return Err(self.error("expected a declarator name"));
                    }
                }
            }
        } else if !abstract_ok {
            return Err(self.error("expected a declarator"));
        }
        loop {
            self.skip_attributes();
            if self.at("[") {
                let open = self.pos;
                let close = self
                    .matching(open)
                    .ok_or_else(|| self.error("unbalanced bracket in declarator"))?;
                let size = if close > open + 1 {
                    Some(self.join(open + 1, close))
                } else {
                    None
                };
                core.derivations.push(Derivation::Array { size });
                self.pos = close + 1;
            } else if self.at("(") {
                if core.derivations.is_empty()
                    && core.name.is_some()
                    && self.looks_like_initializer(self.pos)
                {
                    break;
                }
                let shape = self.parse_function_suffix()?;
                core.derivations.push(Derivation::Function(Box::new(shape)));
            } else {
                break;
            }
        }
        prefix.reverse();
        core.derivations.extend(prefix);
        Ok(core)
    }

    /// At `(`: parses a parameter list and the trailing qualifiers of a function declarator.
    pub fn parse_function_suffix(&mut self) -> Result<FunctionShape, DeclError> {
        let open = self.pos;
        let close = self
            .matching(open)
            .ok_or_else(|| self.error("unbalanced parameter list"))?;
        let mut shape = FunctionShape::default();
        let segs = self.split_commas(open + 1, close, true);
        let dialect = self.cx.dialect;
        if segs.is_empty() {
            // `f()`: no parameters in C++ and C23; no prototype in earlier C.
            shape.unprototyped = dialect == Dialect::C;
        } else if segs.len() == 1 && segs[0].1 - segs[0].0 == 1 && self.text_at(segs[0].0) == "void"
        {
            // `(void)`: no parameters.
        } else if dialect == Dialect::C
            && segs.iter().all(|&(a, b)| {
                b - a == 1
                    && self.ident_at(a)
                    && !self.cx.oracle.is_type_name(self.text_at(a))
            })
        {
            shape.unprototyped = true;
            shape.kr_names = segs.iter().map(|&(a, _)| self.text_at(a).to_string()).collect();
        } else {
            for (a, b) in segs {
                if b - a == 1 && self.text_at(a) == "..." {
                    shape.variadic = true;
                    continue;
                }
                let mut sub = self.sub(a, b);
                let specs = sub.parse_decl_specifiers(true);
                let core = match sub.parse_declarator(true) {
                    Ok(c) => c,
                    Err(_) => DeclCore {
                        name: None,
                        kind: NameKind::Identifier,
                        token: u32::MAX,
                        derivations: Vec::new(),
                        pack: false,
                    },
                };
                let mut pack = core.pack;
                if sub.eat("...") {
                    if dialect == Dialect::Cpp {
                        pack = true;
                    } else {
                        shape.variadic = true;
                    }
                }
                let has_default = sub.at("=");
                shape.params.push(ParamSpec {
                    name: core.name.as_ref().map(|q| q.last_name().to_string()),
                    token: core.token,
                    ty: TypeSpec {
                        base: specs.base,
                        cv: specs.cv,
                        derivations: core.derivations,
                        pack,
                    },
                    has_default,
                });
            }
        }
        self.pos = close + 1;
        loop {
            self.skip_attributes();
            match self.text(0) {
                "const" => shape.cv |= CV_CONST,
                "volatile" => shape.cv |= CV_VOLATILE,
                "&" if self.cpp() => shape.ref_qual = RefQualifier::LValue,
                "&&" if self.cpp() => shape.ref_qual = RefQualifier::RValue,
                "mutable" | "constexpr" | "consteval" | "static" if self.cpp() => {}
                "noexcept" => {
                    shape.noexcept = true;
                    if self.text(1) == "(" {
                        if let Some(c) = self.matching(self.pos + 1) {
                            self.pos = c;
                        }
                    }
                }
                "throw" if self.text(1) == "(" => {
                    if let Some(c) = self.matching(self.pos + 1) {
                        shape.noexcept = c == self.pos + 2;
                        self.pos = c;
                    }
                }
                "->" if self.cpp() => {
                    self.pos += 1;
                    let ts = self.parse_type_id()?;
                    shape.trailing_return = Some(Box::new(ts));
                    continue;
                }
                "override" if self.cpp() => shape.virt |= VIRT_OVERRIDE,
                "final" if self.cpp() => shape.virt |= VIRT_FINAL,
                "requires" if self.cpp() => {
                    self.pos += 1;
                    self.skip_requires_clause();
                    continue;
                }
                _ => break,
            }
            self.pos += 1;
        }
        Ok(shape)
    }

    fn skip_requires_clause(&mut self) {
        while !self.at_end() {
            let t = self.text(0);
            if matches!(t, "{" | ";" | "=" | ",") {
                return;
            }
            if matches!(t, "(" | "[") {
                match self.matching(self.pos) {
                    Some(c) => self.pos = c + 1,
                    None => return,
                }
                continue;
            }
            if t == "<" {
                if let Some(c) = self.matching_angle(self.pos) {
                    self.pos = c + 1;
                    continue;
                }
            }
            self.pos += 1;
        }
    }

    /// Parses a type-id (`const std::string&`, `int (*)(int)`).
    pub fn parse_type_id(&mut self) -> Result<TypeSpec, DeclError> {
        let specs = self.parse_decl_specifiers(true);
        if specs.base == BaseType::None {
            return Err(self.error("expected a type"));
        }
        let core = self.parse_declarator(true)?;
        if core.name.is_some() {
            return Err(self.error("a type-id declares no name"));
        }
        let mut pack = core.pack;
        if self.eat("...") {
            pack = true;
        }
        Ok(TypeSpec {
            base: specs.base,
            cv: specs.cv,
            derivations: core.derivations,
            pack,
        })
    }

    /// At `template`: parses a template header. `Ok(None)` for an explicit instantiation
    /// (`template class X<int>;`), which has no parameter list.
    pub fn parse_template_header(&mut self) -> Result<Option<Vec<TemplateParamSpec>>, DeclError> {
        if !self.eat("template") {
            return Err(self.error("expected `template`"));
        }
        if !self.at("<") {
            return Ok(None);
        }
        let open = self.pos;
        let close = self
            .matching_angle(open)
            .ok_or_else(|| self.error("unbalanced template parameter list"))?;
        let mut params = Vec::new();
        for (a, b) in self.split_commas(open + 1, close, true) {
            let mut sub = self.sub(a, b);
            params.push(sub.parse_template_param()?);
        }
        self.pos = close + 1;
        if self.eat("requires") {
            self.skip_requires_clause_header();
        }
        Ok(Some(params))
    }

    /// A requires-clause after a template header ends where the declaration begins; it is
    /// a primary expression sequence joined by `&&` / `||`.
    fn skip_requires_clause_header(&mut self) {
        loop {
            if self.at("(") {
                match self.matching(self.pos) {
                    Some(c) => self.pos = c + 1,
                    None => return,
                }
            } else if self.starts_id() {
                let _ = self.parse_id_expression();
            } else if self.at("true") || self.at("false") {
                self.pos += 1;
            } else {
                return;
            }
            if !(self.eat("&&") || self.eat("||")) {
                return;
            }
        }
    }

    fn rest_text(&self) -> String {
        self.join(self.pos, self.toks.len())
    }

    fn parse_template_param(&mut self) -> Result<TemplateParamSpec, DeclError> {
        let t0 = self.text(0);
        if t0 == "template" {
            self.pos += 1;
            if self.at("<") {
                if let Some(c) = self.matching_angle(self.pos) {
                    self.pos = c + 1;
                }
            }
            if self.at("class") || self.at("typename") {
                self.pos += 1;
            }
            let pack = self.eat("...");
            let (name, token) = if self.ident_at(self.pos) {
                let r = (Some(self.text(0).to_string()), self.cur_index());
                self.pos += 1;
                r
            } else {
                (None, u32::MAX)
            };
            let default = if self.eat("=") { Some(self.rest_text()) } else { None };
            return Ok(TemplateParamSpec {
                kind: TemplateParamKind::Template,
                name,
                token,
                pack,
                default,
                ty: None,
                constraint: None,
            });
        }
        if (t0 == "typename" || t0 == "class") && self.text(2) != "::" {
            self.pos += 1;
            let pack = self.eat("...");
            let (name, token) = if self.ident_at(self.pos) {
                let r = (Some(self.text(0).to_string()), self.cur_index());
                self.pos += 1;
                r
            } else {
                (None, u32::MAX)
            };
            let default = if self.eat("=") { Some(self.rest_text()) } else { None };
            return Ok(TemplateParamSpec {
                kind: TemplateParamKind::Type,
                name,
                token,
                pack,
                default,
                ty: None,
                constraint: None,
            });
        }
        let save = self.pos;
        if let Some((qn, NameKind::Identifier)) = self.parse_id_expression() {
            let pack = self.eat("...");
            if !self.cx.oracle.is_type_name(qn.last_name())
                && !FundamentalType::is_specifier_word(qn.last_name(), self.cx.dialect)
                && self.ident_at(self.pos)
                && matches!(self.text(1), "" | "=")
            {
                let name = Some(self.text(0).to_string());
                let token = self.cur_index();
                self.pos += 1;
                let default = if self.eat("=") { Some(self.rest_text()) } else { None };
                return Ok(TemplateParamSpec {
                    kind: TemplateParamKind::Type,
                    name,
                    token,
                    pack,
                    default,
                    ty: None,
                    constraint: Some(qn),
                });
            }
        }
        self.pos = save;
        let specs = self.parse_decl_specifiers(true);
        let core = self.parse_declarator(true)?;
        let default = if self.eat("=") { Some(self.rest_text()) } else { None };
        Ok(TemplateParamSpec {
            kind: TemplateParamKind::NonType,
            name: core.name.as_ref().map(|q| q.last_name().to_string()),
            token: core.token,
            pack: core.pack,
            default,
            ty: Some(TypeSpec {
                base: specs.base,
                cv: specs.cv,
                derivations: core.derivations,
                pack: core.pack,
            }),
            constraint: None,
        })
    }
}

/// The name and derivations produced by a declarator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclCore {
    pub name: Option<QualName>,
    pub kind: NameKind,
    pub token: u32,
    pub derivations: Vec<Derivation>,
    pub pack: bool,
}

impl<'p, 'a> DeclParser<'p, 'a> {
    /// Skips an initializer expression up to the next `,` or `;` outside brackets.
    fn skip_expression(&mut self) {
        while !self.at_end() {
            let t = self.text(0);
            if matches!(t, "," | ";") {
                return;
            }
            if matches!(t, "(" | "[" | "{") {
                match self.matching(self.pos) {
                    Some(c) => {
                        self.pos = c + 1;
                        continue;
                    }
                    None => {
                        self.pos = self.toks.len();
                        return;
                    }
                }
            }
            if t == "<" && self.pos > 0 && self.ident_at(self.pos - 1) {
                if let Some(c) = self.matching_angle(self.pos) {
                    self.pos = c + 1;
                    continue;
                }
            }
            if matches!(t, ")" | "]" | "}") {
                return;
            }
            self.pos += 1;
        }
    }

    /// At `:` after a constructor declarator: skips the mem-initializer list.
    fn skip_ctor_initializers(&mut self) {
        self.pos += 1;
        loop {
            self.skip_attributes();
            if self.parse_id_expression().is_none() {
                return;
            }
            if self.at("(") || self.at("{") {
                match self.matching(self.pos) {
                    Some(c) => self.pos = c + 1,
                    None => return,
                }
            }
            self.eat("...");
            if !self.eat(",") {
                return;
            }
        }
    }

    /// Reads the K&R parameter declarations between `)` and `{` of an old-style C
    /// definition and types the identifier-list parameters (undeclared ones are `int`).
    fn parse_kr_parameter_types(&mut self, d: &mut Declarator) -> Result<(), DeclError> {
        let mut typed: Vec<ParamSpec> = Vec::new();
        while !self.at_end() && !self.at("{") {
            let specs = self.parse_decl_specifiers(false);
            if specs.base == BaseType::None {
                return Err(self.error("malformed K&R parameter declaration"));
            }
            loop {
                let core = self.parse_declarator(false)?;
                if let Some(name) = &core.name {
                    typed.push(ParamSpec {
                        name: Some(name.last_name().to_string()),
                        token: core.token,
                        ty: TypeSpec {
                            base: specs.base.clone(),
                            cv: specs.cv,
                            derivations: core.derivations,
                            pack: false,
                        },
                        has_default: false,
                    });
                }
                if !self.eat(",") {
                    break;
                }
            }
            if !self.eat(";") {
                return Err(self.error("expected `;` after K&R parameter declaration"));
            }
        }
        if let Some(Derivation::Function(shape)) = d.ty.derivations.first_mut() {
            let names = std::mem::take(&mut shape.kr_names);
            shape.params = names
                .iter()
                .map(|n| {
                    typed
                        .iter()
                        .find(|p| p.name.as_deref() == Some(n.as_str()))
                        .cloned()
                        .unwrap_or(ParamSpec {
                            name: Some(n.clone()),
                            token: u32::MAX,
                            ty: TypeSpec::of(BaseType::Fundamental(FundamentalType::Int)),
                            has_default: false,
                        })
                })
                .collect();
            shape.kr_names = names;
        }
        Ok(())
    }

    /// Parses a simple-declaration, member-declaration or function-definition starting at
    /// the cursor, consuming the trailing `;` or the function body.
    pub fn parse_declaration(&mut self) -> Result<ParsedDeclaration, DeclError> {
        let first_token = self.cur_index();
        let mut templates = Vec::new();
        loop {
            self.skip_attributes();
            if self.at("template") {
                if self.text(1) == "<" {
                    if let Some(params) = self.parse_template_header()? {
                        templates.push(params);
                    }
                } else {
                    self.pos += 1;
                }
                continue;
            }
            if self.at("export") && self.cpp() {
                self.pos += 1;
                continue;
            }
            break;
        }
        let specifiers = self.parse_decl_specifiers(false);
        let mut decl = ParsedDeclaration {
            templates,
            specifiers,
            declarators: Vec::new(),
            body: None,
            ctor_initializer: false,
            first_token,
            last_token: first_token,
        };
        if self.at(";") || self.at_end() {
            self.eat(";");
            decl.last_token = self.last_index();
            return Ok(decl);
        }
        if self.cpp() && matches!(decl.specifiers.base, BaseType::Auto) {
            let save = self.pos;
            while matches!(self.text(0), "&" | "&&") {
                self.pos += 1;
            }
            if self.at("[") && self.text(1) != "[" {
                let open = self.pos;
                let close = self
                    .matching(open)
                    .ok_or_else(|| self.error("unbalanced structured binding"))?;
                let base = decl.specifiers.base.clone();
                for p in open + 1..close {
                    if self.ident_at(p) {
                        decl.declarators.push(Declarator {
                            name: Some(QualName::simple(self.text_at(p), self.index_at(p))),
                            name_kind: NameKind::Identifier,
                            name_token: self.index_at(p),
                            ty: TypeSpec::of(base.clone()),
                            init: InitKind::Expression,
                            bitfield: false,
                            structured_binding: true,
                            first_token: self.index_at(open),
                            last_token: self.index_at(close),
                        });
                    }
                }
                self.pos = close + 1;
                if self.eat("=") {
                    self.skip_expression();
                } else if self.at("{") || self.at("(") {
                    if let Some(c) = self.matching(self.pos) {
                        self.pos = c + 1;
                    }
                }
                self.eat(";");
                decl.last_token = self.last_index();
                return Ok(decl);
            }
            self.pos = save;
        }
        loop {
            let start_index = self.cur_index();
            let core = self.parse_declarator(false)?;
            let mut d = Declarator {
                name: core.name,
                name_kind: core.kind,
                name_token: core.token,
                ty: TypeSpec {
                    base: decl.specifiers.base.clone(),
                    cv: decl.specifiers.cv,
                    derivations: core.derivations,
                    pack: core.pack,
                },
                init: InitKind::None,
                bitfield: false,
                structured_binding: false,
                first_token: start_index,
                last_token: start_index,
            };
            self.skip_attributes();
            let is_function = d.ty.function().is_some();
            let has_kr = d.ty.function().is_some_and(|f| !f.kr_names.is_empty());
            if has_kr && !self.at("{") && !self.at(";") && !self.at(",") {
                self.parse_kr_parameter_types(&mut d)?;
            }
            if self.at(":") {
                if is_function {
                    self.skip_ctor_initializers();
                    decl.ctor_initializer = true;
                } else {
                    d.bitfield = true;
                    self.pos += 1;
                    self.skip_expression();
                }
            }
            if is_function && self.at("try") {
                self.pos += 1;
                if self.at(":") {
                    self.skip_ctor_initializers();
                    decl.ctor_initializer = true;
                }
            }
            match self.text(0) {
                "=" => {
                    self.pos += 1;
                    if is_function && self.at("0") {
                        d.init = InitKind::PureVirtual;
                        self.pos += 1;
                    } else if self.at("default") {
                        d.init = InitKind::Defaulted;
                        self.pos += 1;
                    } else if self.at("delete") {
                        d.init = InitKind::Deleted;
                        self.pos += 1;
                        if self.at("(") {
                            if let Some(c) = self.matching(self.pos) {
                                self.pos = c + 1;
                            }
                        }
                    } else {
                        d.init = InitKind::Expression;
                        self.skip_expression();
                    }
                }
                "{" => {
                    let open = self.pos;
                    let close = self
                        .matching(open)
                        .ok_or_else(|| self.error("unbalanced braces"))?;
                    self.pos = close + 1;
                    if is_function {
                        decl.body = Some((self.index_at(open), self.index_at(close)));
                        while self.at("catch") {
                            self.pos += 1;
                            for _ in 0..2 {
                                if self.at("(") || self.at("{") {
                                    match self.matching(self.pos) {
                                        Some(c) => self.pos = c + 1,
                                        None => break,
                                    }
                                }
                            }
                        }
                        d.last_token = self.last_index();
                        decl.declarators.push(d);
                        decl.last_token = self.last_index();
                        return Ok(decl);
                    }
                    d.init = InitKind::Braced;
                }
                "(" => match self.matching(self.pos) {
                    Some(c) => {
                        d.init = InitKind::Parenthesized;
                        self.pos = c + 1;
                    }
                    None => return Err(self.error("unbalanced initializer")),
                },
                _ => {}
            }
            self.skip_attributes();
            d.last_token = self.last_index();
            decl.declarators.push(d);
            if self.eat(",") {
                continue;
            }
            break;
        }
        self.eat(";");
        decl.last_token = self.last_index();
        Ok(decl)
    }

    /// Parses a class-head (`template<..> class A<T*> final : public B, virtual C {`).
    pub fn parse_class_head(&mut self) -> Result<ClassHead, DeclError> {
        let first_token = self.cur_index();
        let mut templates = Vec::new();
        let mut friend = false;
        loop {
            self.skip_attributes();
            if self.at("template") && self.text(1) == "<" {
                if let Some(p) = self.parse_template_header()? {
                    templates.push(p);
                }
                continue;
            }
            if self.at("friend") || self.at("typedef") || self.at("export") {
                friend |= self.at("friend");
                self.pos += 1;
                continue;
            }
            break;
        }
        let key = match self.text(0) {
            "class" => TagKey::Class,
            "struct" => TagKey::Struct,
            "union" => TagKey::Union,
            _ => return Err(self.error("expected a class key")),
        };
        self.pos += 1;
        self.skip_attributes();
        let save = self.pos;
        let name = match self.parse_id_expression() {
            Some((qn, NameKind::Identifier)) => Some(qn),
            _ => {
                self.pos = save;
                None
            }
        };
        self.skip_attributes();
        let mut is_final = false;
        while matches!(self.text(0), "final" | "sealed" | "__final") {
            is_final = true;
            self.pos += 1;
        }
        let mut bases = Vec::new();
        if self.at(":") {
            self.pos += 1;
            let end = self.find_at_depth0(&["{", ";"]).unwrap_or(self.toks.len());
            for (a, b) in self.split_commas(self.pos, end, true) {
                let mut sub = self.sub(a, b);
                if let Some(bs) = sub.parse_base_specifier() {
                    bases.push(bs);
                }
            }
            self.pos = end;
        }
        let body = if self.at("{") {
            let open = self.pos;
            let close = self
                .matching(open)
                .ok_or_else(|| self.error("unbalanced class body"))?;
            self.pos = close + 1;
            Some((open, close))
        } else {
            None
        };
        Ok(ClassHead {
            templates,
            key,
            name,
            is_final,
            bases,
            body,
            friend,
            first_token,
        })
    }

    fn parse_base_specifier(&mut self) -> Option<BaseSpec> {
        let mut access = None;
        let mut is_virtual = false;
        loop {
            self.skip_attributes();
            match self.text(0) {
                "virtual" => is_virtual = true,
                t => match Access::from_keyword(t) {
                    Some(a) => access = Some(a),
                    None => break,
                },
            }
            self.pos += 1;
        }
        let token = self.cur_index();
        let (name, kind) = self.parse_id_expression()?;
        if kind != NameKind::Identifier {
            return None;
        }
        let pack = self.eat("...");
        Some(BaseSpec {
            name,
            access,
            is_virtual,
            pack,
            token,
        })
    }

    /// Parses an enum-specifier or opaque-enum-declaration with its enumerators.
    pub fn parse_enum_head(&mut self) -> Result<EnumHead, DeclError> {
        loop {
            self.skip_attributes();
            if matches!(self.text(0), "typedef" | "export") {
                self.pos += 1;
                continue;
            }
            break;
        }
        if !self.eat("enum") {
            return Err(self.error("expected `enum`"));
        }
        let scoped = self.cpp() && (self.eat("class") || self.eat("struct"));
        self.skip_attributes();
        let save = self.pos;
        let name = match self.parse_id_expression() {
            Some((qn, NameKind::Identifier)) => Some(qn),
            _ => {
                self.pos = save;
                None
            }
        };
        self.skip_attributes();
        let underlying = if self.eat(":") {
            let end = self.find_at_depth0(&["{", ";"]).unwrap_or(self.toks.len());
            let mut sub = self.sub(self.pos, end);
            self.pos = end;
            sub.parse_type_id().ok()
        } else {
            None
        };
        let mut body = None;
        let mut enumerators = Vec::new();
        if self.at("{") {
            let open = self.pos;
            let close = self
                .matching(open)
                .ok_or_else(|| self.error("unbalanced enum body"))?;
            for (a, b) in self.split_commas(open + 1, close, true) {
                let mut p = a;
                while p < b && self.text_at(p) == "[" {
                    match self.matching(p) {
                        Some(c) => p = c + 1,
                        None => break,
                    }
                }
                if p < b && self.ident_at(p) {
                    let mut q = p + 1;
                    while q < b && self.text_at(q) != "=" {
                        q += 1;
                    }
                    let value = if q < b { Some(self.join(q + 1, b)) } else { None };
                    enumerators.push(EnumeratorSpec {
                        name: self.text_at(p).to_string(),
                        token: self.index_at(p),
                        value,
                    });
                }
            }
            body = Some((open, close));
            self.pos = close + 1;
        }
        Ok(EnumHead {
            scoped,
            name,
            underlying,
            body,
            enumerators,
        })
    }

    /// Parses `[inline] namespace [A::inline B] {`, `namespace {` or `namespace X = Y;`.
    pub fn parse_namespace_head(&mut self) -> Result<NamespaceHead, DeclError> {
        self.skip_attributes();
        let mut pending_inline = self.eat("inline");
        let inline_first = pending_inline;
        self.eat("export");
        if !self.eat("namespace") {
            return Err(self.error("expected `namespace`"));
        }
        self.skip_attributes();
        let mut parts = Vec::new();
        loop {
            if self.eat("inline") {
                pending_inline = true;
            }
            if !self.ident_at(self.pos) {
                break;
            }
            parts.push((self.text(0).to_string(), self.cur_index(), pending_inline));
            pending_inline = false;
            self.pos += 1;
            self.skip_attributes();
            if !self.eat("::") {
                break;
            }
        }
        let alias_target = if self.eat("=") {
            self.parse_id_expression().map(|(qn, _)| qn)
        } else {
            None
        };
        let body = if self.at("{") {
            let open = self.pos;
            let close = self
                .matching(open)
                .ok_or_else(|| self.error("unbalanced namespace body"))?;
            self.pos = close + 1;
            Some((open, close))
        } else {
            None
        };
        Ok(NamespaceHead {
            inline_unnamed: parts.is_empty() && inline_first,
            parts,
            alias_target,
            body,
        })
    }

    /// Parses a using-directive, using-declaration(s), alias-declaration or using-enum.
    pub fn parse_using(&mut self) -> Result<Vec<UsingForm>, DeclError> {
        loop {
            self.skip_attributes();
            if self.at("template") && self.text(1) == "<" {
                let _ = self.parse_template_header()?;
                continue;
            }
            if self.at("export") {
                self.pos += 1;
                continue;
            }
            break;
        }
        if !self.eat("using") {
            return Err(self.error("expected `using`"));
        }
        if self.eat("namespace") {
            let (target, _) = self
                .parse_id_expression()
                .ok_or_else(|| self.error("expected a namespace name"))?;
            return Ok(vec![UsingForm::Directive { target }]);
        }
        if self.eat("enum") {
            let (target, _) = self
                .parse_id_expression()
                .ok_or_else(|| self.error("expected an enumeration name"))?;
            return Ok(vec![UsingForm::Enum { target }]);
        }
        if self.ident_at(self.pos) {
            let mut p = self.pos + 1;
            if self.text_at(p) == "[" && self.text_at(p + 1) == "[" {
                if let Some(c) = self.matching(p) {
                    p = c + 1;
                }
            }
            if self.text_at(p) == "=" {
                let name = self.text(0).to_string();
                let token = self.cur_index();
                self.pos = p + 1;
                let target = self.parse_type_id()?;
                return Ok(vec![UsingForm::Alias {
                    name,
                    token,
                    target,
                }]);
            }
        }
        let mut forms = Vec::new();
        loop {
            let typename = self.eat("typename");
            let (target, _) = self
                .parse_id_expression()
                .ok_or_else(|| self.error("expected a qualified name"))?;
            self.eat("...");
            forms.push(UsingForm::Declaration { target, typename });
            if !self.eat(",") {
                break;
            }
        }
        Ok(forms)
    }

    /// Access labels (`public:`) directly inside the member list `open..close`, as
    /// (record index of the label, access). Qt's `public slots:` is recognized.
    pub fn scan_access_labels(&self, open: usize, close: usize) -> Vec<(u32, Access)> {
        let mut out = Vec::new();
        let mut p = open + 1;
        while p < close {
            let t = self.text_at(p);
            if matches!(t, "(" | "[" | "{") {
                p = self.matching(p).map(|c| c + 1).unwrap_or(close);
                continue;
            }
            if let Some(acc) = Access::from_keyword(t) {
                if self.text_at(p + 1) == ":" {
                    out.push((self.index_at(p), acc));
                    p += 2;
                    continue;
                }
                if self.ident_at(p + 1) && self.text_at(p + 2) == ":" {
                    out.push((self.index_at(p), acc));
                    p += 3;
                    continue;
                }
            }
            p += 1;
        }
        out
    }

    /// End position (inclusive) of the member-declaration starting at `p`: its `;`, or the
    /// `}` of an inline function body.
    fn statement_end(&self, mut p: usize, limit: usize) -> usize {
        while p < limit {
            let t = self.text_at(p);
            if t == ";" {
                return p;
            }
            if matches!(t, "(" | "[") {
                p = match self.matching(p) {
                    Some(c) => c + 1,
                    None => return limit.saturating_sub(1),
                };
                continue;
            }
            if t == "{" {
                return match self.matching(p) {
                    Some(c) if self.text_at(c + 1) == ";" => c + 1,
                    Some(c) => c,
                    None => limit.saturating_sub(1),
                };
            }
            p += 1;
        }
        limit.saturating_sub(1)
    }

    /// `friend` declarations directly inside the member list `open..close`.
    pub fn scan_friends(&self, open: usize, close: usize) -> Vec<FriendSpec> {
        let mut out = Vec::new();
        let mut p = open + 1;
        while p < close {
            let t = self.text_at(p);
            if t == "{" {
                p = self.matching(p).map(|c| c + 1).unwrap_or(close);
                continue;
            }
            if t != "friend" {
                p += 1;
                continue;
            }
            let end = self.statement_end(p, close);
            let mut sub = self.sub(p + 1, end + 1);
            let key = sub.text(0);
            let class_like = matches!(key, "class" | "struct" | "union");
            if class_like {
                sub.pos += 1;
            }
            let save = sub.pos;
            if let Some((qn, NameKind::Identifier)) = sub.parse_id_expression() {
                if sub.at(";") || sub.at_end() {
                    out.push(FriendSpec::Class {
                        token: qn.last_token(),
                        name: qn,
                    });
                    p = end + 1;
                    continue;
                }
            }
            sub.pos = if class_like { 0 } else { save };
            if !class_like {
                if let Ok(decl) = sub.parse_declaration() {
                    for d in &decl.declarators {
                        if let (Some(name), Some(f)) = (&d.name, d.ty.function()) {
                            out.push(FriendSpec::Function {
                                name: name.clone(),
                                token: d.name_token,
                                arity: f.params.len(),
                                has_body: decl.body.is_some(),
                            });
                        }
                    }
                }
            }
            p = end + 1;
        }
        out
    }

    /// At `[` of a lambda-introducer: captures, optional template parameters, parameters
    /// and the body.
    pub fn parse_lambda_head(&mut self) -> Result<LambdaHead, DeclError> {
        while !self.at_end() && !self.at("[") {
            self.pos += 1;
        }
        let open = self.pos;
        let close = self
            .matching(open)
            .ok_or_else(|| self.error("unbalanced lambda capture"))?;
        self.pos = close + 1;
        let mut templates = Vec::new();
        if self.at("<") {
            if let Some(c) = self.matching_angle(self.pos) {
                for (a, b) in self.split_commas(self.pos + 1, c, true) {
                    let mut sub = self.sub(a, b);
                    if let Ok(tp) = sub.parse_template_param() {
                        templates.push(tp);
                    }
                }
                self.pos = c + 1;
            }
        }
        self.skip_attributes();
        let params = if self.at("(") {
            Some(self.parse_function_suffix()?)
        } else {
            None
        };
        self.skip_attributes();
        let body = if self.at("{") {
            let o = self.pos;
            let c = self
                .matching(o)
                .ok_or_else(|| self.error("unbalanced lambda body"))?;
            Some((self.index_at(o), self.index_at(c)))
        } else {
            None
        };
        Ok(LambdaHead {
            templates,
            params,
            body,
        })
    }

    /// Loop variable(s) of a range-based for statement (`for (auto& [k, v] : m)`).
    pub fn parse_range_for(&mut self) -> Option<ParsedDeclaration> {
        while !self.at_end() && !self.at("(") {
            self.pos += 1;
        }
        let open = self.pos;
        let close = self.matching(open)?;
        let mut start = open + 1;
        let mut colon = None;
        let mut p = open + 1;
        while p < close {
            let t = self.text_at(p);
            if matches!(t, "(" | "[" | "{") {
                p = self.matching(p)? + 1;
                continue;
            }
            if t == ";" {
                start = p + 1;
            }
            if t == ":" {
                colon = Some(p);
                break;
            }
            p += 1;
        }
        let colon = colon?;
        let mut sub = self.sub(start, colon);
        let mut decl = sub.parse_declaration().ok()?;
        decl.first_token = self.index_at(start);
        decl.last_token = self.index_at(colon.saturating_sub(1));
        Some(decl)
    }

    /// The exception declaration of a handler; `None` for `catch (...)`.
    pub fn parse_catch_parameter(&mut self) -> Option<ParamSpec> {
        while !self.at_end() && !self.at("(") {
            self.pos += 1;
        }
        let open = self.pos;
        let close = self.matching(open)?;
        if close == open + 2 && self.text_at(open + 1) == "..." {
            return None;
        }
        let mut sub = self.sub(open + 1, close);
        let specs = sub.parse_decl_specifiers(true);
        let core = sub.parse_declarator(true).ok()?;
        Some(ParamSpec {
            name: core.name.as_ref().map(|q| q.last_name().to_string()),
            token: core.token,
            ty: TypeSpec {
                base: specs.base,
                cv: specs.cv,
                derivations: core.derivations,
                pack: false,
            },
            has_default: false,
        })
    }
}

/// Name of a labeled statement (`retry:`), with its record index.
pub fn label_name<'a>(toks: &[CTok<'a>], dialect: Dialect) -> Option<(&'a str, u32)> {
    match (toks.first(), toks.get(1)) {
        (Some(n), Some(c)) if c.text == ":" && is_identifier(n.text, dialect) => {
            if matches!(n.text, "default" | "case" | "public" | "private" | "protected") {
                None
            } else {
                Some((n.text, n.index))
            }
        }
        _ => None,
    }
}

// ── Preprocessor directives ──────────────────────────────────────────────────

/// A `#define`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MacroDefinition {
    pub name: String,
    /// Record index of the macro name token (`u32::MAX` for catalog macros).
    pub token: u32,
    pub line: u32,
    /// Parameter names of a function-like macro (`__VA_ARGS__` is implied by `variadic`).
    pub params: Option<Vec<String>>,
    pub variadic: bool,
    /// Replacement list as written.
    pub body: String,
}

/// An `#include` / `#include_next` / `#import`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncludeDirective {
    /// Header name without delimiters (`vector`, `util/log.h`), or the macro text of a
    /// computed include.
    pub path: String,
    /// `<...>` form.
    pub system: bool,
    pub token: u32,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Directive {
    Define(MacroDefinition),
    Undef { name: String, token: u32, line: u32 },
    Include(IncludeDirective),
}

/// The directive keyword of a token such as `#define` or `#  include`.
fn directive_keyword(text: &str) -> Option<&str> {
    let rest = text.strip_prefix('#')?.trim_start();
    let end = rest
        .find(|c: char| !c.is_ascii_alphabetic() && c != '_')
        .unwrap_or(rest.len());
    if end == 0 {
        None
    } else {
        Some(&rest[..end])
    }
}

fn is_macro_name(text: &str) -> bool {
    let mut chars = text.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Scans the significant tokens of one file for `#define`, `#undef` and `#include`.
/// Macro definitions are not part of the reduced AST, so they are recovered here.
pub fn scan_directives(toks: &[CTok<'_>]) -> Vec<Directive> {
    let mut out = Vec::new();
    for (i, t) in toks.iter().enumerate() {
        if !t.text.starts_with('#') || (i > 0 && toks[i - 1].line == t.line) {
            continue;
        }
        let kw = match directive_keyword(t.text) {
            Some(k) => k,
            None => continue,
        };
        let line_end = (i + 1..toks.len())
            .find(|&j| toks[j].line != t.line)
            .unwrap_or(toks.len());
        match kw {
            "define" => {
                let name_tok = match toks.get(i + 1) {
                    Some(n) if i + 1 < line_end && is_macro_name(n.text) => n,
                    _ => continue,
                };
                let mut j = i + 2;
                let mut params = None;
                let mut variadic = false;
                if j < line_end && toks[j].text == "(" && name_tok.adjacent_to(&toks[j]) {
                    let mut ps = Vec::new();
                    j += 1;
                    while j < line_end && toks[j].text != ")" {
                        let x = toks[j].text;
                        if x == "..." {
                            variadic = true;
                        } else if let Some(named) = x.strip_suffix("...") {
                            variadic = true;
                            ps.push(named.to_string());
                        } else if x != "," {
                            ps.push(x.to_string());
                        }
                        j += 1;
                    }
                    j += 1;
                    params = Some(ps);
                }
                let body = toks[j.min(line_end)..line_end]
                    .iter()
                    .map(|t| t.text)
                    .collect::<Vec<_>>()
                    .join(" ");
                out.push(Directive::Define(MacroDefinition {
                    name: name_tok.text.to_string(),
                    token: name_tok.index,
                    line: t.line,
                    params,
                    variadic,
                    body,
                }));
            }
            "undef" => {
                if let Some(n) = toks.get(i + 1).filter(|_| i + 1 < line_end) {
                    let name: String = n
                        .text
                        .chars()
                        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                        .collect();
                    if is_macro_name(&name) {
                        out.push(Directive::Undef {
                            name,
                            token: n.index,
                            line: t.line,
                        });
                    }
                }
            }
            "include" | "include_next" | "import" => {
                let mut raw = String::new();
                for j in i + 1..line_end {
                    if j > i + 1 && !toks[j - 1].adjacent_to(&toks[j]) {
                        raw.push(' ');
                    }
                    raw.push_str(toks[j].text);
                }
                let raw = raw.trim();
                if raw.is_empty() {
                    continue;
                }
                let (path, system) = if let Some(p) = raw.strip_prefix('<') {
                    (p.trim_end_matches('>').trim().to_string(), true)
                } else if let Some(p) = raw.strip_prefix('"') {
                    (p.trim_end_matches('"').trim().to_string(), false)
                } else {
                    (raw.to_string(), false)
                };
                out.push(Directive::Include(IncludeDirective {
                    path,
                    system,
                    token: toks.get(i + 1).map(|n| n.index).unwrap_or(t.index),
                    line: t.line,
                }));
            }
            _ => {}
        }
    }
    out
}

/// True when an object-like macro expands to nothing or only to attributes / calling
/// conventions (`#define API __declspec(dllexport)`), so declarations may skip it.
pub fn is_attribute_like_expansion(body: &str) -> bool {
    let toks = lex_c_text(body);
    let mut i = 0usize;
    while i < toks.len() {
        let t = toks[i].text;
        if (ATTRIBUTE_CALLS.contains(&t) && toks.get(i + 1).is_some_and(|n| n.text == "("))
            || (t == "[" && toks.get(i + 1).is_some_and(|n| n.text == "["))
        {
            let open = if t == "[" { i } else { i + 1 };
            let mut depth = 0i32;
            let mut j = open;
            while j < toks.len() {
                match toks[j].text {
                    "(" | "[" => depth += 1,
                    ")" | "]" => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
                j += 1;
            }
            if j >= toks.len() {
                return false;
            }
            i = j + 1;
            continue;
        }
        if CALLING_CONVENTIONS.contains(&t) || IGNORED_QUALIFIER_WORDS.contains(&t) {
            i += 1;
            continue;
        }
        return false;
    }
    true
}

// ── Type-name oracle ─────────────────────────────────────────────────────────

/// Oracle over a set of names known to be types, plus the ignorable macros.
#[derive(Debug, Clone, Default)]
pub struct NameSetOracle {
    pub types: HashSet<String>,
    pub ignorable_macros: HashSet<String>,
}

impl TypeOracle for NameSetOracle {
    fn is_type_name(&self, name: &str) -> bool {
        self.types.contains(name)
    }

    fn is_ignorable_macro(&self, name: &str) -> bool {
        self.ignorable_macros.contains(name)
    }
}

/// Position of the `;` ending the declaration that starts at `from` (outside brackets).
fn declaration_end(toks: &[CTok<'_>], from: usize) -> usize {
    let mut depth = 0i32;
    let mut j = from;
    while j < toks.len() {
        match toks[j].text {
            "(" | "[" | "{" => depth += 1,
            ")" | "]" | "}" => {
                depth -= 1;
                if depth < 0 {
                    return j;
                }
            }
            ";" if depth == 0 => return j,
            _ => {}
        }
        j += 1;
    }
    toks.len()
}

/// Pre-pass of a file: every name the file introduces as a type (class keys, typedef
/// names, alias declarations, template type parameters). The grammar consults the
/// project-wide union of these sets to disambiguate declarations from expressions.
pub fn collect_type_names(toks: &[CTok<'_>], dialect: Dialect, out: &mut HashSet<String>) {
    let mut i = 0usize;
    while i < toks.len() {
        let t = toks[i].text;
        match t {
            "struct" | "union" | "enum" | "class" => {
                if t == "class" && dialect == Dialect::C {
                    i += 1;
                    continue;
                }
                let mut j = i + 1;
                if t == "enum" && matches!(toks.get(j).map(|x| x.text), Some("class" | "struct")) {
                    j += 1;
                }
                loop {
                    match toks.get(j).map(|x| x.text) {
                        Some("[") => {
                            while j < toks.len() && toks[j].text != "]" {
                                j += 1;
                            }
                            j += 2;
                        }
                        Some(a) if ATTRIBUTE_CALLS.contains(&a) => {
                            let mut depth = 0i32;
                            j += 1;
                            while j < toks.len() {
                                match toks[j].text {
                                    "(" => depth += 1,
                                    ")" => {
                                        depth -= 1;
                                        if depth == 0 {
                                            j += 1;
                                            break;
                                        }
                                    }
                                    _ => {}
                                }
                                j += 1;
                            }
                        }
                        _ => break,
                    }
                }
                if let Some(n) = toks.get(j) {
                    if is_identifier(n.text, dialect) && n.text != "final" {
                        out.insert(n.text.to_string());
                    }
                }
            }
            "typedef" => {
                let end = declaration_end(toks, i);
                let oracle = NoTypeOracle;
                let mut p = DeclParser::new(&toks[i..end.min(toks.len())], DeclContext::plain(dialect, &oracle));
                if let Ok(decl) = p.parse_declaration() {
                    for d in &decl.declarators {
                        if let Some(n) = &d.name {
                            out.insert(n.last_name().to_string());
                        }
                    }
                }
            }
            "using" if dialect == Dialect::Cpp => {
                if let (Some(n), Some(eq)) = (toks.get(i + 1), toks.get(i + 2)) {
                    if eq.text == "=" && is_identifier(n.text, dialect) {
                        out.insert(n.text.to_string());
                    }
                }
            }
            "typename" if dialect == Dialect::Cpp => {
                let mut j = i + 1;
                if toks.get(j).is_some_and(|x| x.text == "...") {
                    j += 1;
                }
                if let (Some(n), Some(after)) = (toks.get(j), toks.get(j + 1)) {
                    if is_identifier(n.text, dialect) && matches!(after.text, "," | ">" | "=") {
                        out.insert(n.text.to_string());
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }
}

// ── Linkage rules ────────────────────────────────────────────────────────────

/// Linkage of a name declared at namespace (file) scope. `prior` is the linkage of a
/// previous declaration of the same entity, which `extern` and function declarations
/// inherit (C11 6.2.2/4, [basic.link]/3-4).
pub fn namespace_scope_linkage(
    dialect: Dialect,
    specs: &DeclSpecifiers,
    is_function: bool,
    ty: &TypeSpec,
    in_unnamed_namespace: bool,
    prior: Option<Linkage>,
) -> Linkage {
    if dialect == Dialect::Cpp && in_unnamed_namespace {
        return Linkage::Internal;
    }
    if specs.storage & SC_STATIC != 0 {
        return Linkage::Internal;
    }
    if specs.storage & SC_EXTERN != 0 || is_function {
        return prior.unwrap_or(Linkage::External);
    }
    if dialect == Dialect::Cpp && specs.specs & FS_INLINE == 0 && specs.storage & SC_TYPEDEF == 0
    {
        // A non-volatile const-qualified (or constexpr) variable that is neither `extern`
        // nor `inline` has internal linkage in C++; in C it keeps external linkage.
        let constant = ty.is_const_object() || specs.specs & FS_CONSTEXPR != 0;
        let volatile = ty.derivations.is_empty() && ty.cv & CV_VOLATILE != 0;
        if constant && !volatile {
            return prior.unwrap_or(Linkage::Internal);
        }
    }
    prior.unwrap_or(Linkage::External)
}

/// Linkage of a name declared at block scope: `extern` declarations and function
/// declarations refer to an entity with linkage; everything else has none.
pub fn block_scope_linkage(specs: &DeclSpecifiers, is_function: bool) -> Linkage {
    if specs.storage & SC_EXTERN != 0 || is_function {
        Linkage::External
    } else {
        Linkage::None
    }
}

// ── C view over the C-family engine ──────────────────────────────────────────

/// C-specific queries over a [`CppSemanticModel`] (C translation units are resolved by
/// the C-family engine with C rules: a tag name space separate from ordinary identifiers,
/// file-scope struct tags, no overloading, tentative definitions, unprototyped functions).
pub struct CSemanticView<'m> {
    model: &'m CppSemanticModel,
}

impl<'m> CSemanticView<'m> {
    pub fn new(model: &'m CppSemanticModel) -> Self {
        Self { model }
    }

    pub fn file_scope(&self) -> ScopeId {
        GLOBAL_SCOPE
    }

    /// Ordinary identifiers: objects, functions, typedef names and enumeration constants
    /// (C11 6.2.3).
    pub fn lookup_ordinary(&self, name: &str, scope: ScopeId, file: u16) -> LookupResult {
        self.model
            .lookup_unqualified(name, scope, file, LookupFilter::Any)
    }

    /// Tags of structures, unions and enumerations.
    pub fn lookup_tag(&self, name: &str, scope: ScopeId, file: u16) -> LookupResult {
        self.model
            .lookup_unqualified(name, scope, file, LookupFilter::Tag)
    }

    /// Labels have function scope.
    pub fn lookup_label(&self, name: &str, scope: ScopeId) -> Option<EntityId> {
        self.model.lookup_label(name, scope)
    }

    /// Member of a structure or union (each has its own member name space).
    pub fn lookup_member(&self, record: EntityId, name: &str) -> Option<EntityId> {
        match self.model.lookup_member(record, name, LookupFilter::Any) {
            MemberLookup::Found { decls, .. } => decls.first().copied(),
            _ => None,
        }
    }

    /// The `#define` of `name` in effect at record index `token` of `file`.
    pub fn macro_definition(&self, name: &str, file: u16, token: u32) -> Option<EntityId> {
        self.model.lookup_macro(name, file, token)
    }

    /// Signature of the function a function-pointer object or typedef points to.
    pub fn function_pointer_signature(&self, entity: EntityId) -> Option<&'m FunctionShape> {
        self.model.entity(entity)?.ty.as_ref()?.function_pointer()
    }

    /// True when the function was declared without a prototype (`int f();` before C23,
    /// K&R definitions); `None` when `entity` is not a function.
    pub fn is_unprototyped(&self, entity: EntityId) -> Option<bool> {
        self.model
            .entity(entity)?
            .signature
            .as_ref()
            .map(|s| s.unprototyped)
    }

    /// File-scope objects that only have tentative definitions (C11 6.9.2): they behave
    /// as if defined with a zero initializer at the end of the translation unit.
    pub fn tentative_definitions(&self) -> Vec<EntityId> {
        self.model
            .entities()
            .iter()
            .filter(|e| {
                e.kind == EntityKind::Variable
                    && e.flags & EntityFlags::TENTATIVE != 0
                    && e.definition.is_none()
                    && self.model.canonical(e.id) == e.id
            })
            .map(|e| e.id)
            .collect()
    }

    pub fn linkage(&self, entity: EntityId) -> Option<Linkage> {
        self.model.entity(entity).map(|e| e.linkage)
    }
}
