//! Phase 3 data model shared by the resolution passes: symbol flags, front-end selection,
//! the TypeRef table, resolution statuses, occurrences, supertype records and diagnostics.

use crate::core::types::symbol::THRelation;

/// Bits of `SymbolRecord::flags`. Phase 3 is the only writer of this byte.
pub struct SymbolFlags;

impl SymbolFlags {
    /// Recovered from the token stream because Phase 2 drops the construct (enum constants,
    /// interface constants, catch parameters) or because the file has no AST front end
    /// (Kotlin). `decl_node` is unset for such symbols.
    pub const LEXICAL: u8 = 1 << 0;
    /// Synthesized by a language rule: record fields/accessors/canonical constructor
    /// (JLS 8.10.3), enum `values`/`valueOf` (JLS 8.9.3), Kotlin extension receivers.
    pub const IMPLICIT: u8 = 1 << 1;
    /// The declared type has array dimensions; `type_id` is the element type.
    pub const ARRAY: u8 = 1 << 2;
    /// External type whose binding is unknown: no import, package or catalog entry names it.
    pub const UNRESOLVED: u8 = 1 << 3;
    /// Name chosen by a token-window heuristic (the file has no structural front end).
    pub const HEURISTIC_NAME: u8 = 1 << 4;
    /// External type whose package is inferred from the only applicable on-demand import.
    pub const INFERRED_PACKAGE: u8 = 1 << 5;
    /// The type participates in a cyclic hierarchy (reported as a diagnostic, edges kept).
    pub const HIERARCHY_CYCLE: u8 = 1 << 6;
    /// The declared type carries type arguments (they are kept in the TypeRef table).
    pub const GENERIC_TYPE: u8 = 1 << 7;
}

/// Bits of `TypeHierarchyEdge::flags`.
pub struct EdgeFlags;

impl EdgeFlags {
    /// The edge lies on a cycle of the EXTENDS/IMPLEMENTS graph.
    pub const CYCLE: u8 = 1 << 0;
    /// The relation (extends vs implements) was assumed because the supertype's kind is unknown.
    pub const RELATION_ASSUMED: u8 = 1 << 1;
}

/// Which discovery front end processes a source file.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FrontEnd {
    /// Names, members and types come from grammar fields of the language's own AST.
    Structural = 0,
    /// The file only has a generic AST: declarations come from AST nodes, names from tokens.
    AstHeuristic = 1,
    /// The file has no usable AST: declarations come from a brace-scoped token parser.
    Lexical = 2,
}

/// Surface syntax family used by the token-level recognizers.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Dialect {
    Java = 0,
    Kotlin = 1,
    Other = 2,
}

/// How a name occurrence (type reference, import, expression name or call) was bound.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ResolutionStatus {
    /// Bound to a declaration of the analysed project.
    Project = 0,
    /// Bound to a library type of the built-in JDK/Kotlin catalog.
    Library = 1,
    /// Bound by an explicit qualified name or single-type import to a type outside the project.
    Qualified = 2,
    /// Bound to `<only on-demand package>.<name>`, the sole legal binding if the code compiles.
    InferredPackage = 3,
    /// No binding could be computed from the available sources and catalog.
    Unresolved = 4,
    /// More than one binding is visible at the same precedence level.
    Ambiguous = 5,
    /// A primitive type keyword.
    Primitive = 6,
    /// `var` (JLS 14.4.1): the type is inferred from the initializer.
    TypeInference = 7,
}

impl ResolutionStatus {
    pub fn from_u8(val: u8) -> Option<Self> {
        Some(match val {
            0 => Self::Project,
            1 => Self::Library,
            2 => Self::Qualified,
            3 => Self::InferredPackage,
            4 => Self::Unresolved,
            5 => Self::Ambiguous,
            6 => Self::Primitive,
            7 => Self::TypeInference,
            _ => return None,
        })
    }
}

/// Wildcard form of a type argument.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WildcardKind {
    None = 0,
    Unbounded = 1,
    Extends = 2,
    Super = 3,
}

/// Bits of `TypeRef::flags`.
pub struct TypeRefFlags;

impl TypeRefFlags {
    pub const VARARGS: u8 = 1 << 0;
    /// Kotlin `T?`.
    pub const NULLABLE: u8 = 1 << 1;
    /// `var`: the declared type is inferred; `head` is the inferred type when one was found.
    pub const INFERRED: u8 = 1 << 2;
    /// The type exists in the source but could not be determined (e.g. `var` without an
    /// inferable initializer, or a type form the recognizers do not model).
    pub const UNKNOWN: u8 = 1 << 3;
    /// `head` is a primitive pseudo-id.
    pub const PRIMITIVE: u8 = 1 << 4;
    /// `head` is a type parameter symbol.
    pub const TYPE_VARIABLE: u8 = 1 << 5;
    /// Union type of a multi-catch parameter; the alternatives are in `args`, `head` is unset.
    pub const UNION: u8 = 1 << 6;
    /// Kotlin function type `(A) -> B`; not modelled further.
    pub const FUNCTION: u8 = 1 << 7;
}

/// One node of the TypeRef table: a written type with its resolved head, type arguments and
/// array dimensions. Wildcards keep their bound as the single entry of `args`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeRef {
    /// Symbol id of the head type, a primitive pseudo-id, or `u32::MAX` when absent.
    pub head: u32,
    /// Qualified-name id of the head exactly as written (`Map.Entry`), or `u32::MAX`.
    pub written_name_id: u32,
    /// Type argument TypeRef ids (or the bound of a wildcard / alternatives of a union).
    pub args: Vec<u32>,
    pub array_dims: u8,
    pub wildcard: u8,
    pub flags: u8,
    /// `ResolutionStatus` of the head as u8.
    pub status: u8,
    pub first_token: u32,
    pub last_token: u32,
}

impl TypeRef {
    pub fn unknown(first_token: u32, last_token: u32) -> Self {
        Self {
            head: u32::MAX,
            written_name_id: u32::MAX,
            args: Vec::new(),
            array_dims: 0,
            wildcard: WildcardKind::None as u8,
            flags: TypeRefFlags::UNKNOWN,
            status: ResolutionStatus::Unresolved as u8,
            first_token,
            last_token,
        }
    }

    pub fn is_wildcard(&self) -> bool {
        self.wildcard != WildcardKind::None as u8
    }
}

/// Role of an occurrence in the occurrence table.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum OccurrenceRole {
    Definition = 0,
    TypeReference = 1,
    Read = 2,
    Write = 3,
    Call = 4,
    Import = 5,
    Instantiation = 6,
}

/// A use or definition site of a symbol, keyed by the token of the name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Occurrence {
    pub token: u32,
    /// AST node of the occurrence, or `u32::MAX` for token-recovered sites.
    pub node: u32,
    /// Bound symbol, a primitive pseudo-id, or `u32::MAX` when unresolved/ambiguous.
    pub symbol: u32,
    pub role: u8,
    pub status: u8,
    /// Candidate symbols when `status` is `Ambiguous`.
    pub candidates: Vec<u32>,
}

/// A declared direct supertype together with its written type arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupertypeRecord {
    pub sub: u32,
    pub sup: u32,
    pub relation: THRelation,
    /// TypeRef id of the written supertype (head plus type arguments).
    pub type_ref: u32,
}

/// Diagnostic categories emitted by Phase 3.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DiagnosticCode {
    HierarchyCycle = 1,
    UnresolvedType = 2,
    AmbiguousType = 3,
    MultipleSuperclasses = 4,
    HeuristicFrontEnd = 5,
    LexicalFrontEnd = 6,
    MissingAst = 7,
    MalformedDeclaration = 8,
    AssumedRelation = 9,
    UnresolvedReference = 10,
    AmbiguousReference = 11,
}

impl DiagnosticCode {
    pub fn from_u8(val: u8) -> Option<Self> {
        Some(match val {
            1 => Self::HierarchyCycle,
            2 => Self::UnresolvedType,
            3 => Self::AmbiguousType,
            4 => Self::MultipleSuperclasses,
            5 => Self::HeuristicFrontEnd,
            6 => Self::LexicalFrontEnd,
            7 => Self::MissingAst,
            8 => Self::MalformedDeclaration,
            9 => Self::AssumedRelation,
            10 => Self::UnresolvedReference,
            11 => Self::AmbiguousReference,
            _ => return None,
        })
    }
}

/// A Phase 3 finding about the input that does not invalidate the artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub code: DiagnosticCode,
    /// Principal symbol, or `u32::MAX`.
    pub symbol: u32,
    /// Principal token, or `u32::MAX`.
    pub token: u32,
    /// Further symbols involved (cycle members, ambiguity candidates).
    pub related: Vec<u32>,
    pub message: String,
}
