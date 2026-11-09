//! C Semantic Adapter implementing C declaration & fundamental types, plus the C-family
//! lexical toolkit shared with the C++ adapter: the significant-token view over the corpus,
//! the C/C++ declaration grammar (decl-specifiers, declarators, function pointers, template
//! headers, class / enum / namespace heads, using-forms), the `#define` / `#include`
//! scanners and the linkage rules of both languages.
//!
//! Name resolution itself (scopes, tag and ordinary name spaces, merging, lookup) is done by
//! the C-family engine in `crate::lang::cpp::semantics`, which applies C rules to C
//! translation units; [`CSemanticView`] exposes the C-specific queries over that engine.

use std::collections::HashMap;
use std::fmt;

use crate::core::types::ast::ASTNodeType;
use crate::core::types::symbol::{ScopeKind, SymbolKind, SymbolVisibility};
use crate::core::types::token::{unpack_sort_key, TokenType};
use crate::ingestion::TokenCorpusArtifact;
use crate::lang::c::Dialect;
use crate::lang::cpp::semantics::{
    CppSemanticModel, EntityId, EntityKind, Linkage, LookupFilter, LookupResult, ScopeId,
    NO_ENTITY,
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
