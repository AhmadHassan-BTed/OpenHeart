//! The `LanguageFrontend` trait: everything one source language contributes to the pipeline.
//!
//! A frontend bundles the Phase 1 ingestion adapter, the Phase 2 reduction adapter, the
//! Phase 3 semantic adapter and standard-library model of a language together with the
//! static facts the language-neutral algorithms consult instead of branching on a `LangId`:
//! its descriptor (ids, extensions, detection hints), capabilities per phase, semantic
//! traits (dispatch, memory and aggregate model, control-flow rules), token-level syntax
//! profile, design-pattern idioms and the ownership / multiplicity classification of member
//! types. Every method that has a language-neutral behaviour has a default, so a new
//! language starts with its descriptor and adds adapters as they exist.

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;
use std::sync::Arc;

use serde::ser::{Serialize, SerializeStruct, Serializer};

use crate::ast::adapter::ASTReductionAdapter;
use crate::core::types::ast::{ASTNodeType, NodeAttr};
use crate::core::types::symbol::AssocKind;
use crate::core::types::token::LangId;
use crate::ingestion::adapter::{LanguageAdapter, SourceLexer};
use crate::lang::neutral::{
    is_builtin_scalar, parse_integer_literal, simple_name_of, AggregateSemantics, ContainerKind,
    ControlFlowRules, DeclKind, DeclaredType, DispatchPolicy, GenericsModel, InheritanceModel,
    MemoryModel, Modifiers, Multiplicity, Nullability, Ownership, ReferenceKind,
    SupertypeRelation, TypeKind, Visibility,
};
use crate::symbol::adapter::SemanticAdapter;
use crate::symbol::model::FrontEnd;

// ── Language identity ────────────────────────────────────────────────────────

/// Group of languages that share runtime, tooling and most analysis rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum LanguageFamily {
    /// Java Virtual Machine languages (Java, Kotlin).
    Jvm,
    /// C, C++ and C# / .NET.
    CFamily,
    /// A family without dedicated shared rules yet.
    Other,
}

impl LanguageFamily {
    pub fn as_str(self) -> &'static str {
        match self {
            LanguageFamily::Jvm => "jvm",
            LanguageFamily::CFamily => "c-family",
            LanguageFamily::Other => "other",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            LanguageFamily::Jvm => "JVM",
            LanguageFamily::CFamily => "C family",
            LanguageFamily::Other => "Other",
        }
    }
}

/// Maturity of a frontend as reported to users.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SupportStatus {
    Supported,
    Experimental,
}

impl SupportStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            SupportStatus::Supported => "supported",
            SupportStatus::Experimental => "experimental",
        }
    }
}

/// How firmly a language claims a file extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExtensionClaim {
    /// The language owns the extension (`.c` and `.h` for C).
    Primary,
    /// The language may own files with the extension when evidence says so (`.h` for C++).
    Secondary,
}

/// Static description of a language. The registry's `LANGUAGES` list holds one per
/// supported language; it is the single list the CLI, the server and the web studio use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LanguageDescriptor {
    pub lang: LangId,
    /// Stable lower-case identifier (`java`, `cpp`, `csharp`).
    pub id: &'static str,
    pub display_name: &'static str,
    /// Short label for file badges (`J`, `C++`, `C#`).
    pub badge: &'static str,
    pub family: LanguageFamily,
    pub status: SupportStatus,
    /// Lower-case extensions without the dot that the language owns.
    pub extensions: &'static [&'static str],
    /// Extensions owned by another language that this one may claim on evidence.
    pub secondary_extensions: &'static [&'static str],
    /// The subset of the extensions (primary or secondary) that name header-like files,
    /// which are not compiled on their own.
    pub header_extensions: &'static [&'static str],
    /// File-name globs (`*` and `?`) that identify the language regardless of extension.
    pub filename_patterns: &'static [&'static str],
    /// Interpreter names accepted on a `#!` line (`kotlin` for `#!/usr/bin/env kotlin`).
    pub shebang_interpreters: &'static [&'static str],
    /// Emacs mode / Vim filetype names (`c++`, `cpp`).
    pub editor_modes: &'static [&'static str],
    /// Languages this one is a practical superset of: an ambiguous file goes to this
    /// language when the project contains it (C++ supersedes C for `.h`).
    pub supersedes: &'static [LangId],
}

impl LanguageDescriptor {
    /// How the language claims `ext` (case-insensitive, without the dot).
    pub fn claims_extension(&self, ext: &str) -> Option<ExtensionClaim> {
        let ext = ext.trim_start_matches('.');
        if self.extensions.iter().any(|e| e.eq_ignore_ascii_case(ext)) {
            Some(ExtensionClaim::Primary)
        } else if self
            .secondary_extensions
            .iter()
            .any(|e| e.eq_ignore_ascii_case(ext))
        {
            Some(ExtensionClaim::Secondary)
        } else {
            None
        }
    }

    /// Primary then secondary extensions.
    pub fn all_extensions(&self) -> impl Iterator<Item = &'static str> {
        let primary: &'static [&'static str] = self.extensions;
        let secondary: &'static [&'static str] = self.secondary_extensions;
        primary.iter().chain(secondary.iter()).copied()
    }

    pub fn is_header_extension(&self, ext: &str) -> bool {
        let ext = ext.trim_start_matches('.');
        self.header_extensions
            .iter()
            .any(|e| e.eq_ignore_ascii_case(ext))
    }

    /// True when a file name matches one of the language's file-name patterns.
    pub fn matches_file_name(&self, file_name: &str) -> bool {
        self.filename_patterns
            .iter()
            .any(|p| glob_match(p, file_name, true))
    }

    /// True when a `#!` interpreter (base name) runs this language. A trailing version
    /// (`kotlinc2.0`, `tcc-0.9`) is accepted.
    pub fn matches_interpreter(&self, interpreter: &str) -> bool {
        let interpreter = interpreter.to_ascii_lowercase();
        self.shebang_interpreters.iter().any(|name| {
            interpreter == *name
                || interpreter.strip_prefix(name).is_some_and(|rest| {
                    rest.trim_start_matches(['-', '_'])
                        .chars()
                        .all(|c| c.is_ascii_digit() || c == '.')
                })
        })
    }

    pub fn matches_editor_mode(&self, mode: &str) -> bool {
        self.editor_modes
            .iter()
            .any(|m| m.eq_ignore_ascii_case(mode.trim()))
    }

    pub fn supersedes_lang(&self, other: LangId) -> bool {
        self.supersedes.contains(&other)
    }
}

impl Serialize for LanguageDescriptor {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let supersedes: Vec<u8> = self.supersedes.iter().map(|l| *l as u8).collect();
        let mut s = serializer.serialize_struct("LanguageDescriptor", 12)?;
        s.serialize_field("id", self.id)?;
        s.serialize_field("name", self.display_name)?;
        s.serialize_field("badge", self.badge)?;
        s.serialize_field("family", self.family.as_str())?;
        s.serialize_field("status", self.status.as_str())?;
        s.serialize_field("lang_code", &(self.lang as u8))?;
        s.serialize_field("extensions", self.extensions)?;
        s.serialize_field("secondary_extensions", self.secondary_extensions)?;
        s.serialize_field("header_extensions", self.header_extensions)?;
        s.serialize_field("filename_patterns", self.filename_patterns)?;
        s.serialize_field("shebang_interpreters", self.shebang_interpreters)?;
        s.serialize_field("supersedes", &supersedes)?;
        s.end()
    }
}

// ── Capabilities ─────────────────────────────────────────────────────────────

/// The ten pipeline phases.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Phase {
    Ingestion = 1,
    Syntax = 2,
    Symbols = 3,
    ControlFlow = 4,
    Ssa = 5,
    CallGraph = 6,
    Traceability = 7,
    PathSummaries = 8,
    UmlModel = 9,
    UnifiedGraph = 10,
}

impl Phase {
    pub const ALL: [Phase; 10] = [
        Phase::Ingestion,
        Phase::Syntax,
        Phase::Symbols,
        Phase::ControlFlow,
        Phase::Ssa,
        Phase::CallGraph,
        Phase::Traceability,
        Phase::PathSummaries,
        Phase::UmlModel,
        Phase::UnifiedGraph,
    ];

    /// Phase number (1..=10).
    pub fn number(self) -> u8 {
        self as u8
    }

    pub fn from_number(number: u8) -> Option<Self> {
        Self::ALL.iter().copied().find(|p| p.number() == number)
    }

    fn index(self) -> usize {
        self as usize - 1
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Phase::Ingestion => "ingestion",
            Phase::Syntax => "syntax",
            Phase::Symbols => "symbols",
            Phase::ControlFlow => "control-flow",
            Phase::Ssa => "ssa",
            Phase::CallGraph => "call-graph",
            Phase::Traceability => "traceability",
            Phase::PathSummaries => "path-summaries",
            Phase::UmlModel => "uml",
            Phase::UnifiedGraph => "unified-graph",
        }
    }
}

/// How completely a phase models a language.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Fidelity {
    Unsupported = 0,
    /// Derived from tokens only.
    Lexical = 1,
    /// Derived from a generic syntax tree or token patterns with known gaps.
    Heuristic = 2,
    /// Derived from the language's own grammar; semantics approximated where documented.
    Structural = 3,
    /// The language's semantic rules are modelled (resolution, overloading, generics).
    Semantic = 4,
}

impl Fidelity {
    pub fn as_str(self) -> &'static str {
        match self {
            Fidelity::Unsupported => "unsupported",
            Fidelity::Lexical => "lexical",
            Fidelity::Heuristic => "heuristic",
            Fidelity::Structural => "structural",
            Fidelity::Semantic => "semantic",
        }
    }
}

/// The semantic traits of a language that neutral algorithms consult.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LanguageTraits {
    pub memory: MemoryModel,
    pub aggregates: AggregateSemantics,
    pub inheritance: InheritanceModel,
    pub dispatch: DispatchPolicy,
    pub nullability: Nullability,
    pub generics: GenericsModel,
    pub control_flow: ControlFlowRules,
    /// Ownership expressed by a raw pointer member (C++ Core Guidelines: non-owning; C:
    /// unknown).
    pub raw_pointer_ownership: Ownership,
    pub has_preprocessor: bool,
    pub has_header_files: bool,
    pub has_properties: bool,
    pub has_operator_overloading: bool,
    pub has_free_functions: bool,
    pub case_sensitive: bool,
}

impl LanguageTraits {
    /// Assumptions that keep the analyses sound when a frontend does not state its traits:
    /// every reference may be null, calls may dispatch virtually, exceptions are unchecked
    /// and pointer ownership is unknown.
    pub const CONSERVATIVE: LanguageTraits = LanguageTraits {
        memory: MemoryModel::Managed,
        aggregates: AggregateSemantics::References,
        inheritance: InheritanceModel::SingleWithInterfaces,
        dispatch: DispatchPolicy::VirtualByDefault,
        nullability: Nullability::Implicit,
        generics: GenericsModel::Erased,
        control_flow: ControlFlowRules::C_LIKE,
        raw_pointer_ownership: Ownership::Unknown,
        has_preprocessor: false,
        has_header_files: false,
        has_properties: false,
        has_operator_overloading: false,
        has_free_functions: false,
        case_sensitive: true,
    };
}

/// Which phases a frontend supports, at what fidelity, and the language's traits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    fidelity: [Fidelity; 10],
    pub traits: LanguageTraits,
}

impl Capabilities {
    /// Every phase at `fidelity`, clamped by phase prerequisites.
    pub fn uniform(fidelity: Fidelity, traits: LanguageTraits) -> Self {
        Self {
            fidelity: [fidelity; 10],
            traits,
        }
        .normalized()
    }

    /// Capabilities implied by the adapters a frontend provides: a grammar-backed Phase 1
    /// is structural, a lexer is lexical; Phase 2 needs both a grammar and a reduction
    /// adapter; Phase 3 is structural with a semantic adapter, heuristic over a reduced
    /// tree without one, lexical over tokens only. Later phases follow their prerequisites.
    pub fn derived(
        parser: bool,
        lexer: bool,
        reduction: bool,
        semantics: bool,
        traits: LanguageTraits,
    ) -> Self {
        let p1 = if parser {
            Fidelity::Structural
        } else if lexer {
            Fidelity::Lexical
        } else {
            Fidelity::Unsupported
        };
        let p2 = if parser && reduction {
            Fidelity::Structural
        } else {
            Fidelity::Unsupported
        };
        let p3 = if p2 >= Fidelity::Structural && semantics {
            Fidelity::Structural
        } else if p2 >= Fidelity::Structural {
            Fidelity::Heuristic
        } else if p1 >= Fidelity::Lexical {
            Fidelity::Lexical
        } else {
            Fidelity::Unsupported
        };
        let mut fidelity = [Fidelity::Structural; 10];
        fidelity[Phase::Ingestion.index()] = p1;
        fidelity[Phase::Syntax.index()] = p2;
        fidelity[Phase::Symbols.index()] = p3;
        Self { fidelity, traits }.normalized()
    }

    /// Sets the fidelity of one phase (clamped by its prerequisites).
    pub fn with(mut self, phase: Phase, fidelity: Fidelity) -> Self {
        self.fidelity[phase.index()] = fidelity;
        self.normalized()
    }

    pub fn fidelity(&self, phase: Phase) -> Fidelity {
        self.fidelity[phase.index()]
    }

    pub fn supports(&self, phase: Phase) -> bool {
        self.fidelity(phase) > Fidelity::Unsupported
    }

    pub fn iter(&self) -> impl Iterator<Item = (Phase, Fidelity)> + '_ {
        Phase::ALL.into_iter().map(move |p| (p, self.fidelity(p)))
    }

    /// The highest fidelity each phase can reach given the fidelity of its prerequisites.
    pub fn supported_ceiling(&self, phase: Phase) -> Fidelity {
        use Fidelity as F;
        let f = |p: Phase| self.fidelity(p);
        match phase {
            Phase::Ingestion => F::Semantic,
            Phase::Syntax => {
                if f(Phase::Ingestion) >= F::Structural {
                    F::Semantic
                } else {
                    F::Unsupported
                }
            }
            Phase::Symbols => match f(Phase::Syntax) {
                F::Structural | F::Semantic => F::Semantic,
                F::Heuristic => F::Heuristic,
                F::Lexical | F::Unsupported => {
                    if f(Phase::Ingestion) >= F::Lexical {
                        F::Lexical
                    } else {
                        F::Unsupported
                    }
                }
            },
            Phase::ControlFlow => f(Phase::Syntax),
            Phase::Ssa => f(Phase::ControlFlow).min(f(Phase::Symbols)),
            Phase::CallGraph => f(Phase::Ssa).min(f(Phase::Symbols)),
            Phase::Traceability => f(Phase::Symbols).min(f(Phase::CallGraph)),
            Phase::PathSummaries => f(Phase::ControlFlow).min(f(Phase::Ssa)),
            Phase::UmlModel => f(Phase::Symbols),
            Phase::UnifiedGraph => f(Phase::Symbols).max(f(Phase::ControlFlow)),
        }
    }

    /// Clamps every phase to the ceiling its prerequisites allow, in phase order.
    pub fn normalized(mut self) -> Self {
        for phase in Phase::ALL {
            let ceiling = self.supported_ceiling(phase);
            let i = phase.index();
            if self.fidelity[i] > ceiling {
                self.fidelity[i] = ceiling;
            }
        }
        self
    }

    /// The Phase 3 discovery front end matching the declared symbol fidelity.
    pub fn symbol_front_end(&self) -> Option<FrontEnd> {
        match self.fidelity(Phase::Symbols) {
            Fidelity::Structural | Fidelity::Semantic => Some(FrontEnd::Structural),
            Fidelity::Heuristic => Some(FrontEnd::AstHeuristic),
            Fidelity::Lexical => Some(FrontEnd::Lexical),
            Fidelity::Unsupported => None,
        }
    }
}

impl Serialize for Capabilities {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_map(self.iter().map(|(p, f)| (p.as_str(), f.as_str())))
    }
}

// ── Token-level syntax profile ───────────────────────────────────────────────

/// What ends a statement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StatementTerminator {
    /// Only `;` (C, C++, C#, Java).
    Semicolon,
    /// `;` or a line break (Kotlin, Swift, Go, JavaScript).
    SemicolonOrNewline,
    /// Line breaks only (Python).
    Newline,
}

/// Where a declaration writes its type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TypeAnnotationStyle {
    /// `Type name` (C family, Java).
    Prefix,
    /// `name: Type` (Kotlin, Swift, TypeScript, Rust, Python annotations).
    Postfix,
}

/// Surface-syntax facts token-level recognizers need (Phase 3 lexical front end, Phase 9
/// source scanner). Replaces per-language `Dialect` branches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyntaxProfile {
    pub statement_terminator: StatementTerminator,
    /// Separator used when qualified names are built (`.` or `::`).
    pub qualified_name_separator: &'static str,
    /// Every separator that may appear inside a written qualified name.
    pub name_separators: &'static [&'static str],
    /// Member-access operators (`.`, `->`, `?.`).
    pub member_access_operators: &'static [&'static str],
    pub type_annotation: TypeAnnotationStyle,
    /// Keywords that introduce supertypes in a type header, with their relation.
    pub supertype_keywords: &'static [(&'static str, SupertypeRelation)],
    /// A `:` introduces the supertype list (C++, C#, Kotlin, Swift).
    pub supertype_colon: bool,
    /// Words that may precede a base in a supertype list (C++ `public`, `virtual`).
    pub base_clause_modifiers: &'static [&'static str],
    /// Tokens that end a type header (`{`, `;`, `permits`, `where`, `throws`).
    pub header_terminators: &'static [&'static str],
    /// Kotlin `by`: delegation in a supertype list.
    pub delegation_keyword: Option<&'static str>,
    pub generic_brackets: Option<(&'static str, &'static str)>,
    /// `?` of nullable types.
    pub nullable_suffix: Option<&'static str>,
    pub self_keywords: &'static [&'static str],
    pub super_keywords: &'static [&'static str],
    /// Object-creation keyword (`new`); `None` when construction is a plain call.
    pub new_keyword: Option<&'static str>,
    /// Keywords that can precede `(` but never name an invoked function.
    pub non_call_keywords: &'static [&'static str],
    /// Keywords / directives that import names (`import`, `using`, `#include`).
    pub import_keywords: &'static [&'static str],
    pub container_keywords: &'static [(&'static str, ContainerKind)],
    pub type_keywords: &'static [(&'static str, TypeKind)],
    /// Keywords that start a function declaration (`fun`, `def`, `func`, `fn`).
    pub function_keywords: &'static [&'static str],
    /// Keywords that start a variable declaration (`val`, `var`, `let`, `auto`).
    pub variable_keywords: &'static [&'static str],
    /// Annotation prefix (`@`).
    pub annotation_prefix: Option<&'static str>,
    /// Attribute brackets (`[` `]` in C#, `[[` `]]` in C++).
    pub attribute_brackets: Option<(&'static str, &'static str)>,
    pub line_comment: Option<&'static str>,
    pub block_comment: Option<(&'static str, &'static str)>,
    /// Comment openings that mark documentation (`/**`, `///`).
    pub doc_comment_prefixes: &'static [&'static str],
}

impl SyntaxProfile {
    /// A neutral C-like profile: `;`-terminated, prefix types, `.` qualification, `new`,
    /// `this` / `super`, C comments.
    pub const C_LIKE: SyntaxProfile = SyntaxProfile {
        statement_terminator: StatementTerminator::Semicolon,
        qualified_name_separator: ".",
        name_separators: &["."],
        member_access_operators: &["."],
        type_annotation: TypeAnnotationStyle::Prefix,
        supertype_keywords: &[],
        supertype_colon: false,
        base_clause_modifiers: &[],
        header_terminators: &["{", ";"],
        delegation_keyword: None,
        generic_brackets: Some(("<", ">")),
        nullable_suffix: None,
        self_keywords: &["this"],
        super_keywords: &["super"],
        new_keyword: Some("new"),
        non_call_keywords: &[
            "if", "for", "while", "switch", "catch", "return", "sizeof", "do", "else",
        ],
        import_keywords: &[],
        container_keywords: &[],
        type_keywords: &[
            ("class", TypeKind::Class),
            ("struct", TypeKind::Struct),
            ("interface", TypeKind::Interface),
            ("enum", TypeKind::Enum),
            ("union", TypeKind::Union),
        ],
        function_keywords: &[],
        variable_keywords: &[],
        annotation_prefix: None,
        attribute_brackets: None,
        line_comment: Some("//"),
        block_comment: Some(("/*", "*/")),
        doc_comment_prefixes: &["/**"],
    };

    /// A line break may end a statement.
    pub fn newline_terminates_statements(&self) -> bool {
        self.statement_terminator != StatementTerminator::Semicolon
    }

    pub fn is_self_keyword(&self, word: &str) -> bool {
        self.self_keywords.contains(&word)
    }

    pub fn is_super_keyword(&self, word: &str) -> bool {
        self.super_keywords.contains(&word)
    }

    pub fn is_new_keyword(&self, word: &str) -> bool {
        self.new_keyword == Some(word)
    }

    pub fn is_non_call_keyword(&self, word: &str) -> bool {
        self.non_call_keywords.contains(&word)
    }

    pub fn is_import_keyword(&self, word: &str) -> bool {
        self.import_keywords.contains(&word)
    }

    pub fn type_keyword(&self, word: &str) -> Option<TypeKind> {
        self.type_keywords
            .iter()
            .find(|(w, _)| *w == word)
            .map(|(_, k)| *k)
    }

    pub fn container_keyword(&self, word: &str) -> Option<ContainerKind> {
        self.container_keywords
            .iter()
            .find(|(w, _)| *w == word)
            .map(|(_, k)| *k)
    }

    /// Relation introduced by a header token (`extends`, `implements`, or `:` in languages
    /// with a supertype colon).
    pub fn supertype_relation(&self, token: &str) -> Option<SupertypeRelation> {
        if token == ":" && self.supertype_colon {
            return Some(SupertypeRelation::Unspecified);
        }
        self.supertype_keywords
            .iter()
            .find(|(w, _)| *w == token)
            .map(|(_, r)| *r)
    }

    pub fn is_header_terminator(&self, token: &str) -> bool {
        self.header_terminators.contains(&token)
    }

    pub fn is_base_clause_modifier(&self, word: &str) -> bool {
        self.base_clause_modifiers.contains(&word)
    }

    /// Words that start a declaration (type, function or variable keywords).
    pub fn is_declaration_keyword(&self, word: &str) -> bool {
        self.type_keyword(word).is_some()
            || self.function_keywords.contains(&word)
            || self.variable_keywords.contains(&word)
    }

    /// Splits a written qualified name at every name separator (longest separator first).
    pub fn split_qualified<'n>(&self, name: &'n str) -> Vec<&'n str> {
        let mut separators: Vec<&str> = self.name_separators.to_vec();
        separators.sort_by_key(|s| std::cmp::Reverse(s.len()));
        let mut out = Vec::new();
        let mut rest = name;
        loop {
            let next = separators
                .iter()
                .filter(|s| !s.is_empty())
                .filter_map(|s| rest.find(*s).map(|i| (i, s.len())))
                .min_by_key(|(i, _)| *i);
            match next {
                Some((i, len)) => {
                    let segment = rest[..i].trim();
                    if !segment.is_empty() {
                        out.push(segment);
                    }
                    rest = &rest[i + len..];
                }
                None => {
                    let segment = rest.trim();
                    if !segment.is_empty() {
                        out.push(segment);
                    }
                    return out;
                }
            }
        }
    }

    /// Joins segments with the language's qualified-name separator.
    pub fn join_qualified(&self, segments: &[&str]) -> String {
        segments.join(self.qualified_name_separator)
    }

    /// `container` + separator + `name` (just `name` for an empty container).
    pub fn qualify(&self, container: &str, name: &str) -> String {
        if container.is_empty() {
            name.to_string()
        } else {
            format!("{}{}{}", container, self.qualified_name_separator, name)
        }
    }

    /// Last segment of a written qualified name.
    pub fn simple_name<'n>(&self, name: &'n str) -> &'n str {
        self.split_qualified(name).last().copied().unwrap_or(name)
    }
}

// ── Design-pattern idioms ────────────────────────────────────────────────────

/// Naming and library conventions pattern detection matches against. Type entries are
/// qualified names or globs (`java.util.function.*`); a written simple name matches the
/// simple name of a qualified entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IdiomHints {
    /// Accessors that return the sole instance (`getInstance`, `Instance`, `shared`).
    pub singleton_accessors: &'static [&'static str],
    /// Static fields / enum constants that hold the sole instance (`INSTANCE`).
    pub singleton_constants: &'static [&'static str],
    /// Type kinds that are singletons by declaration (Kotlin `object`).
    pub singleton_type_kinds: &'static [TypeKind],
    /// Prefixes of listener-registration methods (`add`, `register`, `subscribe`).
    pub observer_registration_prefixes: &'static [&'static str],
    /// Suffixes of observer role types (`Listener`, `Observer`, `Handler`).
    pub observer_role_suffixes: &'static [&'static str],
    /// C#-style `event` members make their owner an observer subject.
    pub events_are_observer_subjects: bool,
    pub iterator_supertypes: &'static [&'static str],
    /// Methods that make a type iterable by protocol (`iterator`, `GetEnumerator`, `begin`).
    pub iterator_methods: &'static [&'static str],
    pub disposable_supertypes: &'static [&'static str],
    pub disposal_methods: &'static [&'static str],
    pub clone_methods: &'static [&'static str],
    pub factory_method_prefixes: &'static [&'static str],
    pub builder_terminal_methods: &'static [&'static str],
    /// Library function types (functional interfaces, delegates, `std::function`).
    pub function_types: &'static [&'static str],
    /// Library iteration calls whose function argument runs once per element.
    pub iteration_calls: &'static [&'static str],
    /// Library functions that allocate objects (`malloc`, `std::make_unique`).
    pub allocation_functions: &'static [&'static str],
    /// Program entry points (`main`, `Main`).
    pub entry_points: &'static [&'static str],
    /// Destructors release resources (C++ RAII): a type with a destructor is a scoped
    /// resource holder.
    pub destructors_release_resources: bool,
}

impl IdiomHints {
    /// English naming conventions shared by most languages.
    pub const NEUTRAL: IdiomHints = IdiomHints {
        singleton_accessors: &["instance", "getInstance", "shared", "default"],
        singleton_constants: &["INSTANCE"],
        singleton_type_kinds: &[TypeKind::Object],
        observer_registration_prefixes: &["add", "register", "attach", "subscribe"],
        observer_role_suffixes: &["Listener", "Observer", "Subscriber", "Callback", "Handler"],
        events_are_observer_subjects: false,
        iterator_supertypes: &[],
        iterator_methods: &[],
        disposable_supertypes: &[],
        disposal_methods: &["close", "dispose"],
        clone_methods: &["clone"],
        factory_method_prefixes: &["create", "make", "new"],
        builder_terminal_methods: &["build"],
        function_types: &[],
        iteration_calls: &[],
        allocation_functions: &[],
        entry_points: &["main"],
        destructors_release_resources: false,
    };

    pub fn is_singleton_accessor(&self, name: &str) -> bool {
        self.singleton_accessors
            .iter()
            .any(|a| a.eq_ignore_ascii_case(name))
    }

    pub fn is_singleton_constant(&self, name: &str) -> bool {
        self.singleton_constants.contains(&name)
    }

    pub fn is_singleton_type_kind(&self, kind: TypeKind) -> bool {
        self.singleton_type_kinds.contains(&kind)
    }

    /// A method that registers an observer: a registration prefix at a word boundary and
    /// an observer role name in the rest (`addActionListener`, `subscribe_handler`).
    pub fn is_observer_registration(&self, method_name: &str) -> bool {
        let has_prefix = self
            .observer_registration_prefixes
            .iter()
            .any(|p| has_word_prefix(method_name, p));
        let lower = method_name.to_ascii_lowercase();
        has_prefix
            && self
                .observer_role_suffixes
                .iter()
                .any(|r| lower.contains(&r.to_ascii_lowercase()))
    }

    /// A type named as an observer role (`ClickListener`, `IObserver`).
    pub fn is_observer_role(&self, type_name: &str) -> bool {
        let simple = simple_name_of(type_name);
        self.observer_role_suffixes
            .iter()
            .any(|r| simple.ends_with(r))
    }

    pub fn is_iterator_supertype(&self, type_name: &str) -> bool {
        self.iterator_supertypes
            .iter()
            .any(|p| matches_type_pattern(p, type_name))
    }

    pub fn is_iterator_method(&self, name: &str) -> bool {
        self.iterator_methods.contains(&name)
    }

    pub fn is_disposable_supertype(&self, type_name: &str) -> bool {
        self.disposable_supertypes
            .iter()
            .any(|p| matches_type_pattern(p, type_name))
    }

    pub fn is_disposal_method(&self, name: &str) -> bool {
        self.disposal_methods.contains(&name)
    }

    pub fn is_clone_method(&self, name: &str) -> bool {
        self.clone_methods.contains(&name)
    }

    /// A factory method: a factory prefix at a word boundary (`create`, `createWidget`,
    /// `make_unique`), never a longer word (`newsletter`).
    pub fn is_factory_method(&self, name: &str) -> bool {
        self.factory_method_prefixes
            .iter()
            .any(|p| has_word_prefix(name, p))
    }

    pub fn is_builder_terminal(&self, name: &str) -> bool {
        self.builder_terminal_methods
            .iter()
            .any(|b| b.eq_ignore_ascii_case(name))
    }

    pub fn is_function_type(&self, type_name: &str) -> bool {
        self.function_types
            .iter()
            .any(|p| matches_type_pattern(p, type_name))
    }

    pub fn is_iteration_call(&self, name: &str) -> bool {
        self.iteration_calls.contains(&name)
    }

    pub fn is_allocation_function(&self, name: &str) -> bool {
        self.allocation_functions
            .iter()
            .any(|p| matches_type_pattern(p, name))
    }

    pub fn is_entry_point(&self, name: &str) -> bool {
        self.entry_points.contains(&name)
    }
}

/// `name` starts with `prefix` (case-insensitively) at a word boundary: the prefix is the
/// whole name or is followed by an upper-case letter, a digit or `_`.
fn has_word_prefix(name: &str, prefix: &str) -> bool {
    if name.len() < prefix.len() || !name.is_char_boundary(prefix.len()) {
        return false;
    }
    let (head, rest) = name.split_at(prefix.len());
    if !head.eq_ignore_ascii_case(prefix) {
        return false;
    }
    match rest.chars().next() {
        None => true,
        Some(c) => c.is_uppercase() || c.is_ascii_digit() || c == '_',
    }
}

/// Matches a written type or function name against a qualified-name pattern: globs match
/// the name or its simple name; a plain pattern matches the same qualified name, or its
/// simple name when the written name is unqualified.
pub fn matches_type_pattern(pattern: &str, name: &str) -> bool {
    if pattern.contains(['*', '?']) {
        return glob_match(pattern, name, false) || glob_match(pattern, simple_name_of(name), false);
    }
    if pattern == name {
        return true;
    }
    let unqualified = !name.contains('.') && !name.contains("::");
    unqualified && simple_name_of(pattern) == name
}

/// Glob match with `*` (any run) and `?` (one character).
pub fn glob_match(pattern: &str, text: &str, ignore_case: bool) -> bool {
    let fold = |s: &str| -> Vec<char> {
        if ignore_case {
            s.to_lowercase().chars().collect()
        } else {
            s.chars().collect()
        }
    };
    let p = fold(pattern);
    let t = fold(text);
    let (mut pi, mut ti) = (0usize, 0usize);
    let mut star: Option<usize> = None;
    let mut mark = 0usize;
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

// ── Standard-library model ───────────────────────────────────────────────────

/// How a library type takes part in ownership and multiplicity inference.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LibraryRole {
    /// Value types rendered as attributes, never as associations (strings, numbers, dates).
    Value,
    /// Many elements: type argument 0 (`List<T>`, `std::vector<T>`).
    Collection,
    /// Many values keyed by type argument 0; values are type argument 1.
    Map,
    /// Zero or one: type argument 0 (`Optional<T>`, `std::optional<T>`, `Nullable<T>`).
    Optional,
    /// Exactly N elements: element type argument 0, extent value argument 1 (`std::array`).
    FixedArray,
    /// Sole owner of type argument 0 (`std::unique_ptr<T>`, `Box<T>`).
    UniqueOwner,
    /// Shared owner of type argument 0 (`std::shared_ptr<T>`, `Rc<T>`, `Arc<T>`).
    SharedOwner,
    /// Non-owning, expiring reference to type argument 0 (`std::weak_ptr<T>`,
    /// `WeakReference<T>`).
    WeakReference,
    /// Wraps type argument 0 without changing ownership or multiplicity (`AtomicReference`,
    /// `Lazy<T>`, `std::atomic<T>`).
    Transparent,
    /// A callable value (`Runnable`, `Func<T>`, `std::function<F>`).
    FunctionType,
    /// Any other library type.
    Other,
}

impl LibraryRole {
    pub fn as_str(self) -> &'static str {
        match self {
            LibraryRole::Value => "value",
            LibraryRole::Collection => "collection",
            LibraryRole::Map => "map",
            LibraryRole::Optional => "optional",
            LibraryRole::FixedArray => "fixed-array",
            LibraryRole::UniqueOwner => "unique-owner",
            LibraryRole::SharedOwner => "shared-owner",
            LibraryRole::WeakReference => "weak-reference",
            LibraryRole::Transparent => "transparent",
            LibraryRole::FunctionType => "function",
            LibraryRole::Other => "other",
        }
    }

    /// Type argument holding the element / pointee / value type.
    pub fn element_argument(self) -> Option<usize> {
        match self {
            LibraryRole::Collection
            | LibraryRole::Optional
            | LibraryRole::FixedArray
            | LibraryRole::UniqueOwner
            | LibraryRole::SharedOwner
            | LibraryRole::WeakReference
            | LibraryRole::Transparent => Some(0),
            LibraryRole::Map => Some(1),
            LibraryRole::Value | LibraryRole::FunctionType | LibraryRole::Other => None,
        }
    }

    /// Type argument holding the key (UML qualifier) of a map.
    pub fn key_argument(self) -> Option<usize> {
        match self {
            LibraryRole::Map => Some(0),
            _ => None,
        }
    }

    /// Value argument holding a fixed extent.
    pub fn extent_argument(self) -> Option<usize> {
        match self {
            LibraryRole::FixedArray => Some(1),
            _ => None,
        }
    }

    /// Roles whose instances hold many elements.
    pub fn is_container(self) -> bool {
        matches!(
            self,
            LibraryRole::Collection | LibraryRole::Map | LibraryRole::FixedArray
        )
    }
}

/// One type of a language's built-in library model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LibraryEntry {
    /// Fully qualified name with the language's separator (`java.util.List`, `std::vector`).
    pub qualified_name: &'static str,
    pub kind: DeclKind,
    pub type_params: u8,
    pub role: LibraryRole,
    /// Direct supertypes by qualified name (the root type is implied).
    pub supertypes: &'static [&'static str],
}

impl LibraryEntry {
    pub const fn new(
        qualified_name: &'static str,
        kind: DeclKind,
        type_params: u8,
        role: LibraryRole,
        supertypes: &'static [&'static str],
    ) -> Self {
        Self {
            qualified_name,
            kind,
            type_params,
            role,
            supertypes,
        }
    }

    pub fn type_kind(&self) -> Option<TypeKind> {
        match self.kind {
            DeclKind::Type(k) => Some(k),
            _ => None,
        }
    }
}

/// A language's built-in library: the types Phase 3 binds without a classpath, include
/// path or assembly references, the names visible without an import, and the roots of the
/// type and exception hierarchies.
#[derive(Debug, Clone)]
pub struct StandardLibrary {
    entries: Vec<LibraryEntry>,
    by_qualified: HashMap<&'static str, usize>,
    by_simple: HashMap<&'static str, Vec<usize>>,
    separator: &'static str,
    implicit_imports: Vec<&'static str>,
    root_type: Option<&'static str>,
    exception_root: Option<&'static str>,
}

impl StandardLibrary {
    /// Builds the model; a later entry with the same qualified name replaces an earlier one.
    pub fn new(entries: impl IntoIterator<Item = LibraryEntry>, separator: &'static str) -> Self {
        let mut library = Self {
            entries: Vec::new(),
            by_qualified: HashMap::new(),
            by_simple: HashMap::new(),
            separator,
            implicit_imports: Vec::new(),
            root_type: None,
            exception_root: None,
        };
        for entry in entries {
            library.insert(entry);
        }
        library
    }

    /// Namespaces / packages whose types are visible without an import (`java.lang`, the
    /// C# SDK global usings), in lookup order.
    pub fn with_implicit_imports(
        mut self,
        imports: impl IntoIterator<Item = &'static str>,
    ) -> Self {
        self.implicit_imports = imports.into_iter().collect();
        self
    }

    /// Root of the class hierarchy (`java.lang.Object`, `System.Object`, `kotlin.Any`).
    pub fn with_root_type(mut self, root: &'static str) -> Self {
        self.root_type = Some(root);
        self
    }

    /// Root of the throwable types (`java.lang.Throwable`, `System.Exception`).
    pub fn with_exception_root(mut self, root: &'static str) -> Self {
        self.exception_root = Some(root);
        self
    }

    pub fn insert(&mut self, entry: LibraryEntry) {
        if let Some(&idx) = self.by_qualified.get(entry.qualified_name) {
            self.entries[idx] = entry;
            return;
        }
        let idx = self.entries.len();
        self.by_qualified.insert(entry.qualified_name, idx);
        let simple = last_segment(entry.qualified_name, self.separator);
        self.by_simple.entry(simple).or_default().push(idx);
        self.entries.push(entry);
    }

    pub fn entries(&self) -> &[LibraryEntry] {
        &self.entries
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn separator(&self) -> &'static str {
        self.separator
    }

    pub fn implicit_imports(&self) -> &[&'static str] {
        &self.implicit_imports
    }

    pub fn root_type(&self) -> Option<&'static str> {
        self.root_type
    }

    pub fn exception_root(&self) -> Option<&'static str> {
        self.exception_root
    }

    /// The entry with exactly this qualified name.
    pub fn get(&self, qualified: &str) -> Option<&LibraryEntry> {
        self.by_qualified.get(qualified).map(|&i| &self.entries[i])
    }

    /// The entry with this simple name, when every entry sharing the name has the same role
    /// (`java.util.List` and `kotlin.collections.List` are both collections).
    pub fn lookup_simple(&self, simple: &str) -> Option<&LibraryEntry> {
        let candidates = self.by_simple.get(simple)?;
        let first = &self.entries[*candidates.first()?];
        candidates
            .iter()
            .all(|&i| self.entries[i].role == first.role)
            .then_some(first)
    }

    /// Binds a written name without import information: the exact qualified name, a name
    /// in an implicitly imported namespace, a unique qualified suffix (`Map.Entry`), or a
    /// role-consistent simple name. Callers that know the file's imports resolve the name
    /// first and use [`StandardLibrary::get`].
    pub fn lookup(&self, written: &str) -> Option<&LibraryEntry> {
        let written = written.trim();
        let written = written.strip_prefix("global::").unwrap_or(written);
        let written = written.strip_prefix("::").unwrap_or(written);
        if written.is_empty() {
            return None;
        }
        if let Some(entry) = self.get(written) {
            return Some(entry);
        }
        for package in &self.implicit_imports {
            let qualified = format!("{}{}{}", package, self.separator, written);
            if let Some(entry) = self.get(&qualified) {
                return Some(entry);
            }
        }
        if written.contains(self.separator) {
            let suffix = format!("{}{}", self.separator, written);
            let mut matches = self
                .entries
                .iter()
                .filter(|e| e.qualified_name.ends_with(suffix.as_str()));
            let first = matches.next()?;
            return matches.next().is_none().then_some(first);
        }
        self.lookup_simple(written)
    }

    pub fn role_of(&self, written: &str) -> Option<LibraryRole> {
        self.lookup(written).map(|e| e.role)
    }

    pub fn supertypes(&self, qualified: &str) -> &'static [&'static str] {
        self.get(qualified).map(|e| e.supertypes).unwrap_or(&[])
    }

    /// `sub` is `sup` or reaches it through declared supertypes; every class type reaches
    /// the root type.
    pub fn is_subtype(&self, sub: &str, sup: &str) -> bool {
        if sub == sup {
            return true;
        }
        if self.root_type == Some(sup)
            && self
                .get(sub)
                .is_some_and(|e| matches!(e.kind, DeclKind::Type(k) if k != TypeKind::Interface))
        {
            return true;
        }
        let mut seen: HashSet<&str> = HashSet::new();
        let mut queue: VecDeque<&str> = VecDeque::new();
        queue.push_back(sub);
        while let Some(current) = queue.pop_front() {
            if !seen.insert(current) {
                continue;
            }
            for &parent in self.supertypes(current) {
                if parent == sup {
                    return true;
                }
                queue.push_back(parent);
            }
        }
        false
    }

    /// First segments of every qualified name (`java`, `javax`, `kotlin`, `std`, `System`),
    /// sorted: the namespaces exporters treat as library roots.
    pub fn root_namespaces(&self) -> Vec<&'static str> {
        let roots: BTreeSet<&'static str> = self
            .entries
            .iter()
            .filter_map(|e| e.qualified_name.split(self.separator).next())
            .filter(|s| !s.is_empty())
            .collect();
        roots.into_iter().collect()
    }
}

fn last_segment<'s>(qualified: &'s str, separator: &str) -> &'s str {
    if separator.is_empty() {
        return qualified;
    }
    qualified.rsplit(separator).next().unwrap_or(qualified)
}

// ── Member-type classification (ownership and multiplicity) ──────────────────

/// Resolves written type names against the analysed project for member classification.
pub trait TypeNameResolver {
    /// Kind of a project type named as written (`None`: the project declares no such type).
    fn project_type_kind(&self, written: &str) -> Option<TypeKind>;

    /// Qualified library name the written name binds to under the declaring file's imports,
    /// when the caller has resolved it.
    fn library_name(&self, _written: &str) -> Option<String> {
        None
    }
}

/// A resolver that knows no project types and no imports.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoProjectTypes;

impl TypeNameResolver for NoProjectTypes {
    fn project_type_kind(&self, _written: &str) -> Option<TypeKind> {
        None
    }
}

/// A resolver backed by a closure from written name to project type kind.
pub struct FnResolver<F>(pub F);

impl<F> TypeNameResolver for FnResolver<F>
where
    F: Fn(&str) -> Option<TypeKind>,
{
    fn project_type_kind(&self, written: &str) -> Option<TypeKind> {
        (self.0)(written)
    }
}

/// What a member's declared type makes the member.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MemberTypeRole {
    /// A relationship to `target` (UML association / aggregation / composition).
    Association,
    /// A value rendered in the attribute compartment (primitives, strings, library values).
    Attribute,
    /// A function value (function pointer, delegate, functional interface).
    Callable,
    /// The type could not be classified (unbounded wildcard, opaque `decltype`).
    Unknown,
}

/// Ownership and multiplicity of a member, derived from its declared type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemberTypeClass<'t> {
    pub role: MemberTypeRole,
    pub ownership: Ownership,
    pub multiplicity: Multiplicity,
    /// The type the relationship points to (collection element, pointee, value type of a
    /// map). Its pointer / array decorations are already folded into the ownership and
    /// multiplicity; read its `head`.
    pub target: Option<&'t DeclaredType<'t>>,
    /// Map key type (a UML qualifier).
    pub qualifier: Option<&'t DeclaredType<'t>>,
}

impl<'t> MemberTypeClass<'t> {
    pub fn unknown() -> Self {
        Self {
            role: MemberTypeRole::Unknown,
            ownership: Ownership::Unknown,
            multiplicity: Multiplicity::Unknown,
            target: None,
            qualifier: None,
        }
    }

    /// UML relationship kind; `AssocKind::None` unless the member is an association.
    pub fn assoc_kind(&self) -> AssocKind {
        match self.role {
            MemberTypeRole::Association => self.ownership.assoc_kind(self.multiplicity),
            _ => AssocKind::None,
        }
    }
}

/// Maximum type-argument nesting followed by the classifier.
const MAX_CLASSIFY_DEPTH: usize = 16;

/// The language-neutral member-type classification every frontend inherits: declarators
/// (references, pointers, arrays, `?`), library roles (collections, maps, optionals, smart
/// pointers, wrappers, value types, function types) and the language's aggregate, pointer
/// and nullability semantics combine into one ownership and multiplicity.
pub fn classify_member_type_default<'t, F>(
    frontend: &F,
    ty: &'t DeclaredType<'t>,
    resolver: &dyn TypeNameResolver,
) -> MemberTypeClass<'t>
where
    F: LanguageFrontend + ?Sized,
{
    let semantics = frontend.semantic_adapter();
    let classifier = MemberTypeClassifier {
        traits: frontend.capabilities().traits,
        library: frontend.standard_library(),
        semantics: semantics.as_deref(),
        resolver,
    };
    classifier.classify(ty, 0)
}

struct MemberTypeClassifier<'c> {
    traits: LanguageTraits,
    library: Option<&'c StandardLibrary>,
    semantics: Option<&'c dyn SemanticAdapter>,
    resolver: &'c dyn TypeNameResolver,
}

impl<'c> MemberTypeClassifier<'c> {
    fn classify<'t>(&self, ty: &'t DeclaredType<'t>, depth: usize) -> MemberTypeClass<'t> {
        if depth > MAX_CLASSIFY_DEPTH || ty.is_unbounded_wildcard() {
            return MemberTypeClass::unknown();
        }
        let (ownership, multiplicity) = self.declarator_layer(ty);
        if ty.function {
            return Self::callable(multiplicity);
        }
        let opaque = ["decltype", "typeof", "__typeof"]
            .iter()
            .any(|op| ty.head.starts_with(op));
        if opaque {
            return MemberTypeClass::unknown();
        }
        if self.is_primitive(ty) {
            return Self::attribute(ownership, multiplicity);
        }
        if let Some(kind) = self.resolver.project_type_kind(ty.head) {
            if kind.is_callable_type() {
                return Self::callable(multiplicity);
            }
            let (o, m) = self.instance_layer(Some(kind));
            return Self::association(ownership.compose(o), multiplicity.compose(m), ty);
        }
        let entry = self
            .resolver
            .library_name(ty.head)
            .and_then(|q| self.library.and_then(|l| l.get(&q).copied()))
            .or_else(|| self.library.and_then(|l| l.lookup(ty.head).copied()));
        match entry {
            Some(entry) => self.classify_library(ty, &entry, ownership, multiplicity, depth),
            None => {
                let (o, m) = self.instance_layer(None);
                Self::association(ownership.compose(o), multiplicity.compose(m), ty)
            }
        }
    }

    fn classify_library<'t>(
        &self,
        ty: &'t DeclaredType<'t>,
        entry: &LibraryEntry,
        ownership: Ownership,
        multiplicity: Multiplicity,
        depth: usize,
    ) -> MemberTypeClass<'t> {
        let role = entry.role;
        match role {
            LibraryRole::Value => return Self::attribute(ownership, multiplicity),
            LibraryRole::FunctionType => return Self::callable(multiplicity),
            LibraryRole::Other => {
                let (o, m) = self.instance_layer(entry.type_kind());
                return Self::association(ownership.compose(o), multiplicity.compose(m), ty);
            }
            _ => {}
        }
        let holder_ownership = match role {
            LibraryRole::UniqueOwner => Ownership::Owning,
            LibraryRole::SharedOwner => Ownership::Shared,
            LibraryRole::WeakReference => Ownership::Weak,
            _ => self.instance_layer(entry.type_kind()).0,
        };
        let holder_multiplicity = match role {
            LibraryRole::Collection | LibraryRole::Map => Multiplicity::MANY,
            LibraryRole::Optional
            | LibraryRole::UniqueOwner
            | LibraryRole::SharedOwner
            | LibraryRole::WeakReference => Multiplicity::OPTIONAL,
            LibraryRole::FixedArray => role
                .extent_argument()
                .and_then(|i| ty.value_arg(i))
                .and_then(parse_integer_literal)
                .map(Multiplicity::exactly)
                .unwrap_or(Multiplicity::MANY),
            _ => Multiplicity::ONE,
        };
        let ownership = ownership.compose(holder_ownership);
        let multiplicity = multiplicity.compose(holder_multiplicity);
        let qualifier = role.key_argument().and_then(|i| ty.type_arg(i));
        match role.element_argument().and_then(|i| ty.type_arg(i)) {
            Some(element) => {
                let inner = self.classify(element, depth + 1);
                MemberTypeClass {
                    role: inner.role,
                    ownership: ownership.compose(inner.ownership),
                    multiplicity: multiplicity.compose(inner.multiplicity),
                    target: inner.target,
                    qualifier: qualifier.or(inner.qualifier),
                }
            }
            None => MemberTypeClass {
                role: MemberTypeRole::Attribute,
                ownership,
                multiplicity,
                target: None,
                qualifier,
            },
        }
    }

    /// Ownership and multiplicity contributed by references, pointers, arrays and `?`.
    fn declarator_layer(&self, ty: &DeclaredType<'_>) -> (Ownership, Multiplicity) {
        let mut ownership = Ownership::Owning;
        let mut multiplicity = Multiplicity::ONE;
        if ty.reference != ReferenceKind::None {
            ownership = ownership.compose(Ownership::Borrowed);
        }
        if ty.pointer_depth > 0 {
            ownership = ownership.compose(self.traits.raw_pointer_ownership);
            multiplicity = multiplicity.compose(if ty.pointer_depth == 1 {
                Multiplicity::OPTIONAL
            } else {
                Multiplicity::MANY
            });
        }
        if ty.array_dims > 0 {
            let array_ownership = if self.traits.aggregates == AggregateSemantics::Values {
                Ownership::Owning
            } else {
                Ownership::Unknown
            };
            ownership = ownership.compose(array_ownership);
            for dim in 0..ty.array_dims as usize {
                let extent = ty
                    .extents
                    .get(dim)
                    .copied()
                    .flatten()
                    .map(Multiplicity::exactly)
                    .unwrap_or(Multiplicity::MANY);
                multiplicity = multiplicity.compose(extent);
            }
        }
        if ty.nullable {
            multiplicity = multiplicity.compose(Multiplicity::OPTIONAL);
        }
        (ownership, multiplicity)
    }

    /// Ownership and multiplicity of storing an instance of a type of `kind` directly.
    fn instance_layer(&self, kind: Option<TypeKind>) -> (Ownership, Multiplicity) {
        if self.traits.aggregates.stores_value(kind) {
            (Ownership::Owning, Multiplicity::ONE)
        } else {
            let multiplicity = match self.traits.nullability {
                Nullability::Implicit => Multiplicity::OPTIONAL,
                Nullability::Explicit | Nullability::NotApplicable => Multiplicity::ONE,
            };
            (Ownership::Unknown, multiplicity)
        }
    }

    fn is_primitive(&self, ty: &DeclaredType<'_>) -> bool {
        is_builtin_scalar(ty.head)
            || self.semantics.is_some_and(|s| {
                s.primitive_type_id(ty.head).is_some()
                    || s.primitive_type_id(ty.simple_name()).is_some()
            })
    }

    fn attribute<'t>(ownership: Ownership, multiplicity: Multiplicity) -> MemberTypeClass<'t> {
        MemberTypeClass {
            role: MemberTypeRole::Attribute,
            ownership,
            multiplicity,
            target: None,
            qualifier: None,
        }
    }

    fn callable<'t>(multiplicity: Multiplicity) -> MemberTypeClass<'t> {
        MemberTypeClass {
            role: MemberTypeRole::Callable,
            ownership: Ownership::Unknown,
            multiplicity,
            target: None,
            qualifier: None,
        }
    }

    fn association<'t>(
        ownership: Ownership,
        multiplicity: Multiplicity,
        target: &'t DeclaredType<'t>,
    ) -> MemberTypeClass<'t> {
        MemberTypeClass {
            role: MemberTypeRole::Association,
            ownership,
            multiplicity,
            target: Some(target),
            qualifier: None,
        }
    }
}

// ── Content sniffing ─────────────────────────────────────────────────────────

/// Bytes of a file head examined by detection.
pub const SNIFF_LIMIT_BYTES: usize = 16 * 1024;
/// Lines at the top of a file searched for an editor modeline.
pub const MODELINE_SCAN_LINES: usize = 5;

/// How strongly a file's content indicates a language.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SniffVerdict {
    /// The content cannot be this language.
    Impossible,
    Unlikely,
    /// No evidence either way.
    Neutral,
    Likely,
    /// Syntax only this language has.
    Definite,
}

/// The head of a source file, prepared for content sniffing: raw bytes, lossy UTF-8 text,
/// code with C-family comments and string / character literals blanked (line structure
/// kept), and the tokens of that code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeadScan {
    raw: Vec<u8>,
    text: String,
    code: String,
    tokens: Vec<(usize, usize)>,
}

impl HeadScan {
    pub fn new(head: &[u8]) -> Self {
        let raw = head[..head.len().min(SNIFF_LIMIT_BYTES)].to_vec();
        let lossy = String::from_utf8_lossy(&raw);
        let text = lossy.strip_prefix('\u{FEFF}').unwrap_or(&*lossy).to_string();
        let code = blank_comments_and_literals(&text);
        let tokens = tokenize(&code);
        Self {
            raw,
            text,
            code,
            tokens,
        }
    }

    /// Reads at most `SNIFF_LIMIT_BYTES` from the start of a file.
    pub fn read(path: &Path) -> io::Result<Self> {
        let file = File::open(path)?;
        let mut buf = Vec::with_capacity(SNIFF_LIMIT_BYTES);
        file.take(SNIFF_LIMIT_BYTES as u64).read_to_end(&mut buf)?;
        Ok(Self::new(&buf))
    }

    pub fn raw(&self) -> &[u8] {
        &self.raw
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// The text with comments and literals replaced by spaces.
    pub fn code(&self) -> &str {
        &self.code
    }

    pub fn first_line(&self) -> &str {
        self.text.lines().next().unwrap_or("")
    }

    pub fn token_count(&self) -> usize {
        self.tokens.len()
    }

    pub fn token(&self, index: usize) -> Option<&str> {
        self.tokens.get(index).map(|&(s, e)| &self.code[s..e])
    }

    pub fn tokens(&self) -> impl Iterator<Item = &str> {
        self.tokens.iter().map(move |&(s, e)| &self.code[s..e])
    }

    pub fn has_word(&self, word: &str) -> bool {
        self.tokens().any(|t| t == word)
    }

    pub fn count_word(&self, word: &str) -> usize {
        self.tokens().filter(|t| *t == word).count()
    }

    /// The tokens appear consecutively (`["public", "class"]`, `["template", "<"]`).
    pub fn has_sequence(&self, sequence: &[&str]) -> bool {
        if sequence.is_empty() {
            return true;
        }
        let tokens: Vec<&str> = self.tokens().collect();
        tokens.windows(sequence.len()).any(|w| w == sequence)
    }

    /// Raw substring search in the code (comments and literals excluded).
    pub fn has_text(&self, needle: &str) -> bool {
        self.code.contains(needle)
    }

    /// Preprocessor directive lines (`#include <x>`), trimmed.
    pub fn directives(&self) -> impl Iterator<Item = &str> {
        self.code
            .lines()
            .map(str::trim)
            .filter(|l| l.starts_with('#'))
    }

    /// Interpreter base name of a `#!` line (`#!/usr/bin/env -S kotlinc -script` gives
    /// `kotlinc`), lower-cased.
    pub fn shebang_interpreter(&self) -> Option<String> {
        let line = self.text.lines().next()?;
        let rest = line.strip_prefix("#!")?;
        let mut parts = rest.split_whitespace();
        let program = base_name(parts.next()?);
        if program != "env" {
            return Some(program.to_ascii_lowercase());
        }
        let mut skip_next = false;
        for part in parts {
            if skip_next {
                skip_next = false;
                continue;
            }
            if matches!(part, "-u" | "--unset" | "-C" | "--chdir") {
                skip_next = true;
                continue;
            }
            if part.starts_with('-') || part.contains('=') {
                continue;
            }
            return Some(base_name(part).to_ascii_lowercase());
        }
        None
    }

    /// Emacs mode (`-*- mode: c++ -*-`, `-*- C++ -*-`) or Vim filetype (`vim: set ft=cpp:`)
    /// named in the first lines, lower-cased.
    pub fn modeline_mode(&self) -> Option<String> {
        self.text
            .lines()
            .take(MODELINE_SCAN_LINES)
            .find_map(|line| emacs_mode(line).or_else(|| vim_filetype(line)))
    }

    /// The same head with the regions compiled only when `macro_name` is defined removed
    /// (`#ifdef __cplusplus ... #endif`, the `#else` branch of `#ifndef NAME`); conditional
    /// directive lines are blanked too.
    pub fn excluding_conditional(&self, macro_name: &str) -> HeadScan {
        let mut code = String::with_capacity(self.code.len());
        let mut frames: Vec<ConditionalFrame> = Vec::new();
        for line in self.code.split_inclusive('\n') {
            let mut blank = frames.iter().any(ConditionalFrame::excludes);
            if let Some(directive) = line.trim_start().strip_prefix('#') {
                let (keyword, argument) = split_directive(directive);
                match keyword {
                    "if" | "ifdef" | "ifndef" => {
                        frames.push(ConditionalFrame {
                            guard: guard_polarity(keyword, argument, macro_name),
                            in_else: false,
                        });
                        blank = true;
                    }
                    "elif" | "elifdef" | "elifndef" | "else" => {
                        if let Some(frame) = frames.last_mut() {
                            frame.in_else = true;
                        }
                        blank = true;
                    }
                    "endif" => {
                        frames.pop();
                        blank = true;
                    }
                    _ => {}
                }
            }
            if blank {
                code.extend(line.chars().map(|c| if c == '\n' { '\n' } else { ' ' }));
            } else {
                code.push_str(line);
            }
        }
        let tokens = tokenize(&code);
        HeadScan {
            raw: self.raw.clone(),
            text: self.text.clone(),
            code,
            tokens,
        }
    }
}

fn base_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

fn emacs_mode(line: &str) -> Option<String> {
    let start = line.find("-*-")? + 3;
    let end = start + line[start..].find("-*-")?;
    let body = line[start..end].trim();
    if body.is_empty() {
        return None;
    }
    if !body.contains(':') {
        return Some(body.to_ascii_lowercase());
    }
    body.split(';').find_map(|item| {
        let (key, value) = item.split_once(':')?;
        key.trim()
            .eq_ignore_ascii_case("mode")
            .then(|| value.trim().to_ascii_lowercase())
    })
}

fn vim_filetype(line: &str) -> Option<String> {
    for marker in ["vim:", "vi:", "ex:"] {
        for (idx, _) in line.match_indices(marker) {
            let at_boundary = idx == 0
                || line[..idx]
                    .chars()
                    .next_back()
                    .is_some_and(char::is_whitespace);
            if !at_boundary {
                continue;
            }
            let settings = &line[idx + marker.len()..];
            for item in settings.split(|c: char| c == ':' || c.is_whitespace()) {
                for key in ["filetype=", "ft=", "syntax=", "syn="] {
                    if let Some(value) = item.trim().strip_prefix(key) {
                        if !value.is_empty() {
                            return Some(value.to_ascii_lowercase());
                        }
                    }
                }
            }
        }
    }
    None
}

#[derive(Debug, Clone, Copy)]
struct ConditionalFrame {
    /// `Some(true)`: the `#if` branch is compiled only when the macro is defined;
    /// `Some(false)`: only when it is undefined; `None`: unrelated condition.
    guard: Option<bool>,
    in_else: bool,
}

impl ConditionalFrame {
    fn excludes(&self) -> bool {
        matches!((self.guard, self.in_else), (Some(true), false) | (Some(false), true))
    }
}

fn split_directive(directive: &str) -> (&str, &str) {
    let directive = directive.trim_start();
    let end = directive
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(directive.len());
    (&directive[..end], directive[end..].trim())
}

fn guard_polarity(keyword: &str, argument: &str, macro_name: &str) -> Option<bool> {
    match keyword {
        "ifdef" => (argument == macro_name).then_some(true),
        "ifndef" => (argument == macro_name).then_some(false),
        "if" => {
            let compact: String = argument.chars().filter(|c| !c.is_whitespace()).collect();
            let positive = [
                macro_name.to_string(),
                format!("defined({})", macro_name),
                format!("defined{}", macro_name),
            ];
            let negative = [
                format!("!{}", macro_name),
                format!("!defined({})", macro_name),
                format!("!defined{}", macro_name),
            ];
            if positive.iter().any(|p| *p == compact) {
                Some(true)
            } else if negative.iter().any(|n| *n == compact) {
                Some(false)
            } else {
                None
            }
        }
        _ => None,
    }
}

fn blank(c: char) -> char {
    if c == '\n' {
        '\n'
    } else {
        ' '
    }
}

/// Replaces C-family comments and string / character literals (including Java / Kotlin
/// text blocks, C# verbatim strings and C++ raw strings) with spaces, keeping line breaks.
fn blank_comments_and_literals(text: &str) -> String {
    #[derive(PartialEq)]
    enum State {
        Code,
        LineComment,
        BlockComment,
        Str,
        Verbatim,
        TextBlock,
        Char,
        Raw,
    }
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut state = State::Code;
    let mut raw_close: Vec<char> = Vec::new();
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        match state {
            State::Code => {
                if c == '/' && next == Some('/') {
                    state = State::LineComment;
                    out.push_str("  ");
                    i += 2;
                    continue;
                }
                if c == '/' && next == Some('*') {
                    state = State::BlockComment;
                    out.push_str("  ");
                    i += 2;
                    continue;
                }
                if c == '"' {
                    if next == Some('"') && chars.get(i + 2) == Some(&'"') {
                        state = State::TextBlock;
                        out.push_str("   ");
                        i += 3;
                        continue;
                    }
                    let before = if i > 0 { Some(chars[i - 1]) } else { None };
                    if before == Some('R') {
                        // C++ raw string R"delim( ... )delim"
                        let open = chars[i + 1..]
                            .iter()
                            .take(17)
                            .position(|&d| d == '(')
                            .filter(|&p| {
                                chars[i + 1..i + 1 + p]
                                    .iter()
                                    .all(|d| !d.is_whitespace() && *d != '\\' && *d != ')')
                            });
                        if let Some(p) = open {
                            raw_close.clear();
                            raw_close.push(')');
                            raw_close.extend_from_slice(&chars[i + 1..i + 1 + p]);
                            raw_close.push('"');
                            state = State::Raw;
                            for _ in 0..p + 2 {
                                out.push(' ');
                            }
                            i += p + 2;
                            continue;
                        }
                    }
                    state = if before == Some('@') {
                        State::Verbatim
                    } else {
                        State::Str
                    };
                    out.push(' ');
                    i += 1;
                    continue;
                }
                if c == '\'' {
                    let before = if i > 0 { Some(chars[i - 1]) } else { None };
                    if !before.is_some_and(|p| p.is_alphanumeric() || p == '_') {
                        state = State::Char;
                        out.push(' ');
                        i += 1;
                        continue;
                    }
                }
                out.push(c);
                i += 1;
            }
            State::LineComment => {
                if c == '\n' {
                    state = State::Code;
                }
                out.push(blank(c));
                i += 1;
            }
            State::BlockComment => {
                if c == '*' && next == Some('/') {
                    state = State::Code;
                    out.push_str("  ");
                    i += 2;
                    continue;
                }
                out.push(blank(c));
                i += 1;
            }
            State::Str | State::Char => {
                let quote = if state == State::Str { '"' } else { '\'' };
                if c == '\\' {
                    out.push(' ');
                    if let Some(n) = next {
                        out.push(blank(n));
                    }
                    i += 2;
                    continue;
                }
                if c == quote || c == '\n' {
                    state = State::Code;
                }
                out.push(blank(c));
                i += 1;
            }
            State::Verbatim => {
                if c == '"' {
                    if next == Some('"') {
                        out.push_str("  ");
                        i += 2;
                        continue;
                    }
                    state = State::Code;
                }
                out.push(blank(c));
                i += 1;
            }
            State::TextBlock => {
                if c == '"' && next == Some('"') && chars.get(i + 2) == Some(&'"') {
                    state = State::Code;
                    out.push_str("   ");
                    i += 3;
                    continue;
                }
                out.push(blank(c));
                i += 1;
            }
            State::Raw => {
                if chars[i..].starts_with(&raw_close) {
                    for _ in 0..raw_close.len() {
                        out.push(' ');
                    }
                    i += raw_close.len();
                    state = State::Code;
                    continue;
                }
                out.push(blank(c));
                i += 1;
            }
        }
    }
    out
}

/// Byte ranges of identifier, number and punctuation tokens of comment-free code.
fn tokenize(code: &str) -> Vec<(usize, usize)> {
    const PUNCT3: [&str; 4] = ["...", "<<=", ">>=", "->*"];
    const PUNCT2: [&str; 22] = [
        "::", "->", "==", "!=", "<=", ">=", "&&", "||", "++", "--", "<<", ">>", "+=", "-=", "*=",
        "/=", "%=", "&=", "|=", "^=", "=>", "?.",
    ];
    let bytes = code.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        if b.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        if b.is_ascii_alphabetic() || b == b'_' || b == b'$' || b >= 0x80 {
            i += 1;
            while i < bytes.len()
                && (bytes[i].is_ascii_alphanumeric()
                    || bytes[i] == b'_'
                    || bytes[i] == b'$'
                    || bytes[i] >= 0x80)
            {
                i += 1;
            }
        } else if b.is_ascii_digit() {
            i += 1;
            while i < bytes.len()
                && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'.' || bytes[i] == b'_')
            {
                i += 1;
            }
        } else if let Some(p) = PUNCT3.iter().find(|p| bytes[i..].starts_with(p.as_bytes())) {
            i += p.len();
        } else if let Some(p) = PUNCT2.iter().find(|p| bytes[i..].starts_with(p.as_bytes())) {
            i += p.len();
        } else {
            i += 1;
        }
        tokens.push((start, i));
    }
    tokens
}

// ── Detection context ────────────────────────────────────────────────────────

/// Project-level facts that decide ambiguous files: the languages already known to be
/// present (from files whose language is unambiguous).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DetectionContext {
    pub project_languages: BTreeSet<LangId>,
}

impl DetectionContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_language(mut self, lang: LangId) -> Self {
        self.project_languages.insert(lang);
        self
    }

    pub fn insert(&mut self, lang: LangId) {
        self.project_languages.insert(lang);
    }

    pub fn contains(&self, lang: LangId) -> bool {
        self.project_languages.contains(&lang)
    }
}

// ── NodeAttr decoding defaults ───────────────────────────────────────────────

/// Decodes the visibility nibble of a reduced node. A declaration that carries an access
/// level but has none written is `Unknown` (the language's default-access rule decides).
pub fn default_decode_visibility(node_type: ASTNodeType, attr: u32) -> Visibility {
    let visibility = Visibility::from_attr_nibble(NodeAttr::unpack_visibility(attr));
    let carries_access = DeclKind::from_ast(node_type).is_some_and(DeclKind::has_visibility);
    if visibility == Visibility::NotApplicable && carries_access {
        Visibility::Unknown
    } else {
        visibility
    }
}

/// Decodes the modifier byte of a reduced declaration node; non-declarations have none.
pub fn default_decode_modifiers(node_type: ASTNodeType, attr: u32) -> Modifiers {
    if DeclKind::from_ast(node_type).is_none() {
        return Modifiers::EMPTY;
    }
    Modifiers::from_node_attr(NodeAttr::unpack_modifiers(attr))
}

// ── The frontend trait ───────────────────────────────────────────────────────

/// Everything one source language provides to the pipeline. Implementations are
/// registered explicitly in `lang::registry::FrontendRegistry`.
pub trait LanguageFrontend: Send + Sync + 'static {
    /// The language's entry of `lang::registry::LANGUAGES`.
    fn descriptor(&self) -> &'static LanguageDescriptor;

    fn lang_id(&self) -> LangId {
        self.descriptor().lang
    }

    fn display_name(&self) -> &'static str {
        self.descriptor().display_name
    }

    fn family(&self) -> LanguageFamily {
        self.descriptor().family
    }

    fn extensions(&self) -> &'static [&'static str] {
        self.descriptor().extensions
    }

    fn filename_patterns(&self) -> &'static [&'static str] {
        self.descriptor().filename_patterns
    }

    /// The tree-sitter grammar the language is parsed with.
    fn ts_language(&self) -> Option<tree_sitter::Language> {
        self.ingestion_adapter()
            .map(|a| a.ts_language())
            .or_else(|| self.reduction_adapter().map(|a| a.ts_language()))
    }

    /// Phase 1 adapter over the language's grammar.
    fn ingestion_adapter(&self) -> Option<Arc<dyn LanguageAdapter>> {
        None
    }

    /// Phase 1 lexer for a language without a linked grammar.
    fn lexer(&self) -> Option<Arc<dyn SourceLexer>> {
        None
    }

    /// Phase 2 CST-to-AST reduction adapter.
    fn reduction_adapter(&self) -> Option<Arc<dyn ASTReductionAdapter>> {
        None
    }

    /// Phase 3 declaration, scoping and type rules.
    fn semantic_adapter(&self) -> Option<Arc<dyn SemanticAdapter>> {
        None
    }

    /// Built-in library model Phase 3 binds against.
    fn standard_library(&self) -> Option<&StandardLibrary> {
        None
    }

    fn syntax_profile(&self) -> &'static SyntaxProfile {
        &SyntaxProfile::C_LIKE
    }

    fn idiom_hints(&self) -> &'static IdiomHints {
        &IdiomHints::NEUTRAL
    }

    /// Phase fidelities and language traits; by default derived from the adapters present
    /// under conservative traits.
    fn capabilities(&self) -> Capabilities {
        Capabilities::derived(
            self.ingestion_adapter().is_some(),
            self.lexer().is_some(),
            self.reduction_adapter().is_some(),
            self.semantic_adapter().is_some(),
            LanguageTraits::CONSERVATIVE,
        )
    }

    /// Access level of a reduced node from its `NodeAttr` word.
    fn decode_visibility(&self, node_type: ASTNodeType, attr: u32) -> Visibility {
        default_decode_visibility(node_type, attr)
    }

    /// Neutral modifiers of a reduced node from its `NodeAttr` word; frontends that carry
    /// further modifiers in their language-flag byte add them.
    fn decode_modifiers(&self, node_type: ASTNodeType, attr: u32) -> Modifiers {
        default_decode_modifiers(node_type, attr)
    }

    /// Ownership and multiplicity of a member of declared type `ty`.
    fn classify_member_type<'t>(
        &self,
        ty: &'t DeclaredType<'t>,
        resolver: &dyn TypeNameResolver,
    ) -> MemberTypeClass<'t> {
        classify_member_type_default(self, ty, resolver)
    }

    /// Evidence in a file's head that it is written in this language.
    fn sniff(&self, _scan: &HeadScan) -> SniffVerdict {
        SniffVerdict::Neutral
    }

    /// Called for a file whose extension this language claims as primary: returns the
    /// language the file belongs to when the language's own rules decide it (GCC's
    /// upper-case `.C`, the `.h` rule), `None` to apply the registry's generic policy.
    fn resolve_detection(
        &self,
        _path: &Path,
        _scan: Option<&HeadScan>,
        _ctx: &DetectionContext,
    ) -> Option<LangId> {
        None
    }
}
