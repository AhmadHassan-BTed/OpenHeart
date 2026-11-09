//! C# Semantic Adapter implementing C#-specific declaration kinds, primitive types and default
//! accessibility, and the C# semantic model that Phase 3 runs over C# / .NET sources.
//!
//! The model parses every C# file's token stream into its declaration structure (namespaces,
//! types, members, parameters, attributes; bodies are skipped by brace matching), evaluates
//! `#if` / `#elif` / `#else` / `#define` / `#undef` against the configured symbols, and then
//! performs C# name resolution:
//! - block and file-scoped namespaces merged across files, extern aliases and `global::`;
//! - `using`, `using static`, `global using`, `using X = ...` aliases and the SDK implicit
//!   usings, with the C# rule that directives of one body do not see each other;
//! - partial types (and partial methods) merged across files, nested types, top-level
//!   statements (synthesized `Program.<Main>$`);
//! - base classes, interfaces, explicit interface member implementations, overrides and the
//!   interface implementation map, with generic substitution through constructed base types;
//! - member lookup through base types with hiding, overload resolution by arity and parameter
//!   types (optional, named and `params` arguments, generic type inference), extension methods
//!   visible through usings;
//! - properties, indexers, events and delegates as distinct symbol kinds, records with their
//!   synthesized members, all six accessibility levels and attributes.
//!
//! BCL types come from `lang::csharp::stdlib` and are loaded into the same symbol arena, so
//! source and library symbols share one lookup path. `apply_to_builder` writes the results
//! into the shared Phase 3 `SymbolTableBuilder` (names, qualified names, kinds, visibility,
//! types, namespace packages, type-hierarchy edges, type-reference resolutions, imports).

use std::collections::{HashMap, HashSet, VecDeque};

use crate::ast::BPASTArtifact;
use crate::core::types::ast::ASTNodeType;
use crate::core::types::symbol::{
    ScopeKind, SymbolKind, SymbolModifiers, SymbolRecord, SymbolVisibility, THRelation,
};
use crate::core::types::token::{unpack_sort_key, LangId, TokenType};
use crate::ingestion::TokenCorpusArtifact;
use crate::lang::csharp::stdlib::{
    bcl_symbol_id, bcl_type, implicit_global_usings, BclTypeFlags, BclTypeKind, CsStdLibManager,
    DotNetSdk, BCL_TYPES, CS_OBJECT_STUB_SYM_ID,
};
use crate::symbol::adapter::{
    SemanticAdapter, PRIMITIVE_BOOLEAN_ID, PRIMITIVE_BYTE_ID, PRIMITIVE_CHAR_ID,
    PRIMITIVE_DOUBLE_ID, PRIMITIVE_FLOAT_ID, PRIMITIVE_INT_ID, PRIMITIVE_LONG_ID,
    PRIMITIVE_SHORT_ID, PRIMITIVE_VOID_ID,
};
use crate::symbol::builder::SymbolTableBuilder;

pub struct CSharpSemanticAdapter;

impl CSharpSemanticAdapter {
    pub fn new() -> Self {
        Self
    }
}

impl Default for CSharpSemanticAdapter {
    fn default() -> Self {
        Self::new()
    }
}

// C# keyword types that share a Java primitive reuse its id (`sbyte` is the signed 8-bit
// type Java calls `byte`); the unsigned, native-sized and decimal types get their own ids.
pub const CS_PRIMITIVE_BYTE_ID: u32 = 0xB000_0040;
pub const CS_PRIMITIVE_USHORT_ID: u32 = 0xB000_0041;
pub const CS_PRIMITIVE_UINT_ID: u32 = 0xB000_0042;
pub const CS_PRIMITIVE_ULONG_ID: u32 = 0xB000_0043;
pub const CS_PRIMITIVE_NINT_ID: u32 = 0xB000_0044;
pub const CS_PRIMITIVE_NUINT_ID: u32 = 0xB000_0045;
pub const CS_PRIMITIVE_DECIMAL_ID: u32 = 0xB000_0046;

/// Primitive pseudo-id of a C# simple-type keyword. `object`, `string` and `dynamic` are
/// reference types and resolve to BCL stubs instead.
pub fn cs_primitive_id(keyword: &str) -> Option<u32> {
    match keyword {
        "void" => Some(PRIMITIVE_VOID_ID),
        "bool" => Some(PRIMITIVE_BOOLEAN_ID),
        "sbyte" => Some(PRIMITIVE_BYTE_ID),
        "short" => Some(PRIMITIVE_SHORT_ID),
        "char" => Some(PRIMITIVE_CHAR_ID),
        "int" => Some(PRIMITIVE_INT_ID),
        "long" => Some(PRIMITIVE_LONG_ID),
        "float" => Some(PRIMITIVE_FLOAT_ID),
        "double" => Some(PRIMITIVE_DOUBLE_ID),
        "byte" => Some(CS_PRIMITIVE_BYTE_ID),
        "ushort" => Some(CS_PRIMITIVE_USHORT_ID),
        "uint" => Some(CS_PRIMITIVE_UINT_ID),
        "ulong" => Some(CS_PRIMITIVE_ULONG_ID),
        "nint" => Some(CS_PRIMITIVE_NINT_ID),
        "nuint" => Some(CS_PRIMITIVE_NUINT_ID),
        "decimal" => Some(CS_PRIMITIVE_DECIMAL_ID),
        _ => None,
    }
}

/// Primitive pseudo-id of a BCL stub that a simple-type keyword aliases (`System.Int32`).
pub fn primitive_id_of_stub(stub_id: u32) -> Option<u32> {
    bcl_type(stub_id)
        .and_then(|t| t.keyword)
        .and_then(cs_primitive_id)
}

const COLLECTION_NAMESPACES: &[&str] = &[
    "System.Collections",
    "System.Collections.Generic",
    "System.Collections.ObjectModel",
    "System.Collections.Concurrent",
    "System.Collections.Immutable",
    "System.Linq",
];

const COLLECTION_SIMPLE_NAMES: &[&str] = &[
    "IEnumerable",
    "ICollection",
    "IList",
    "IDictionary",
    "IReadOnlyCollection",
    "IReadOnlyList",
    "IReadOnlyDictionary",
    "ISet",
    "IReadOnlySet",
    "List",
    "Dictionary",
    "HashSet",
    "SortedSet",
    "SortedDictionary",
    "SortedList",
    "Queue",
    "Stack",
    "LinkedList",
    "Collection",
    "ReadOnlyCollection",
    "ObservableCollection",
    "ConcurrentDictionary",
    "ConcurrentBag",
    "ConcurrentQueue",
    "ConcurrentStack",
    "BlockingCollection",
    "ImmutableArray",
    "ImmutableList",
    "ImmutableDictionary",
    "ImmutableHashSet",
    "ILookup",
    "IGrouping",
    "ArrayList",
    "Hashtable",
];

impl SemanticAdapter for CSharpSemanticAdapter {
    fn is_declaration(&self, node_type: ASTNodeType) -> bool {
        matches!(
            node_type,
            ASTNodeType::NN_MODULE
                | ASTNodeType::NN_CLASS_DECL
                | ASTNodeType::NN_STRUCT_DECL
                | ASTNodeType::NN_INTERFACE_DECL
                | ASTNodeType::NN_ENUM_DECL
                | ASTNodeType::NN_RECORD_DECL
                | ASTNodeType::NN_DELEGATE_DECL
                | ASTNodeType::NN_METHOD_DECL
                | ASTNodeType::NN_FUNCTION_DECL
                | ASTNodeType::NN_CONSTRUCTOR_DECL
                | ASTNodeType::NN_DESTRUCTOR_DECL
                | ASTNodeType::NN_OPERATOR_DECL
                | ASTNodeType::NN_PROPERTY_DECL
                | ASTNodeType::NN_INDEXER_DECL
                | ASTNodeType::NN_EVENT_DECL
                | ASTNodeType::NN_FIELD_DECL
                | ASTNodeType::NN_ENUM_CONSTANT
                | ASTNodeType::NN_PARAM_DECL
                | ASTNodeType::NN_LOCAL_VAR_DECL
                | ASTNodeType::NN_TYPE_PARAM
                | ASTNodeType::NN_LAMBDA_EXPR
        )
    }

    fn symbol_kind(&self, node_type: ASTNodeType) -> SymbolKind {
        match node_type {
            ASTNodeType::NN_MODULE => SymbolKind::SK_MODULE,
            ASTNodeType::NN_PARAM_DECL => SymbolKind::SK_PARAM,
            ASTNodeType::NN_LOCAL_VAR_DECL => SymbolKind::SK_LOCAL_VAR,
            ASTNodeType::NN_TYPE_PARAM => SymbolKind::SK_TYPE_PARAM,
            ASTNodeType::NN_LAMBDA_EXPR => SymbolKind::SK_LAMBDA,
            other => match CsSymbolKind::from_node_type(other) {
                Some(kind) => kind.core_kind(),
                None => SymbolKind::SK_EXTERNAL,
            },
        }
    }

    fn scope_kind(&self, node_type: ASTNodeType) -> ScopeKind {
        match node_type {
            ASTNodeType::NN_MODULE => ScopeKind::File,
            ASTNodeType::NN_CLASS_DECL
            | ASTNodeType::NN_STRUCT_DECL
            | ASTNodeType::NN_INTERFACE_DECL
            | ASTNodeType::NN_ENUM_DECL
            | ASTNodeType::NN_RECORD_DECL
            | ASTNodeType::NN_DELEGATE_DECL => ScopeKind::Class,
            ASTNodeType::NN_METHOD_DECL
            | ASTNodeType::NN_FUNCTION_DECL
            | ASTNodeType::NN_CONSTRUCTOR_DECL
            | ASTNodeType::NN_DESTRUCTOR_DECL
            | ASTNodeType::NN_OPERATOR_DECL
            | ASTNodeType::NN_PROPERTY_DECL
            | ASTNodeType::NN_INDEXER_DECL
            | ASTNodeType::NN_EVENT_DECL => ScopeKind::Method,
            ASTNodeType::NN_LAMBDA_EXPR => ScopeKind::Lambda,
            _ => ScopeKind::Block,
        }
    }

    fn primitive_type_id(&self, text: &str) -> Option<u32> {
        cs_primitive_id(text)
    }

    fn is_collection_type(&self, type_name: &str) -> bool {
        // Accept `List`, `List<T>`, `List`1` and namespace-qualified forms.
        let head = type_name
            .split(|c| c == '<' || c == '`')
            .next()
            .unwrap_or(type_name)
            .trim()
            .trim_start_matches("global::");
        let (ns, simple) = match head.rfind('.') {
            Some(i) => (&head[..i], &head[i + 1..]),
            None => ("", head),
        };
        if !COLLECTION_SIMPLE_NAMES.contains(&simple) {
            return false;
        }
        ns.is_empty() || COLLECTION_NAMESPACES.contains(&ns)
    }

    fn default_visibility(&self, kind: SymbolKind) -> SymbolVisibility {
        match kind {
            // Top-level types default to `internal`, which is assembly scope.
            SymbolKind::SK_CLASS
            | SymbolKind::SK_INTERFACE
            | SymbolKind::SK_ENUM
            | SymbolKind::SK_RECORD => SymbolVisibility::Package,
            SymbolKind::SK_METHOD
            | SymbolKind::SK_CONSTRUCTOR
            | SymbolKind::SK_FIELD
            | SymbolKind::SK_STATIC_INIT => SymbolVisibility::Private,
            _ => SymbolVisibility::Public,
        }
    }
}

// ── Symbol kinds, accessibility and modifiers ───────────────────────────────

/// C# symbol kinds. Properties, indexers, events and delegates are distinct kinds here; the
/// shared Phase 3 alphabet receives them through `core_kind`.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CsSymbolKind {
    Namespace = 0,
    Class = 1,
    Struct = 2,
    Interface = 3,
    Enum = 4,
    Record = 5,
    RecordStruct = 6,
    Delegate = 7,
    Method = 8,
    LocalFunction = 9,
    Constructor = 10,
    StaticConstructor = 11,
    Destructor = 12,
    Operator = 13,
    ConversionOperator = 14,
    Property = 15,
    Indexer = 16,
    Event = 17,
    Field = 18,
    Constant = 19,
    EnumMember = 20,
    Parameter = 21,
    TypeParameter = 22,
    Local = 23,
}

impl CsSymbolKind {
    pub fn is_type(self) -> bool {
        matches!(
            self,
            CsSymbolKind::Class
                | CsSymbolKind::Struct
                | CsSymbolKind::Interface
                | CsSymbolKind::Enum
                | CsSymbolKind::Record
                | CsSymbolKind::RecordStruct
                | CsSymbolKind::Delegate
        )
    }

    pub fn is_value_type(self) -> bool {
        matches!(
            self,
            CsSymbolKind::Struct | CsSymbolKind::RecordStruct | CsSymbolKind::Enum
        )
    }

    pub fn is_method_like(self) -> bool {
        matches!(
            self,
            CsSymbolKind::Method
                | CsSymbolKind::LocalFunction
                | CsSymbolKind::Constructor
                | CsSymbolKind::StaticConstructor
                | CsSymbolKind::Destructor
                | CsSymbolKind::Operator
                | CsSymbolKind::ConversionOperator
        )
    }

    /// Members that take part in ordinary member lookup by name.
    pub fn is_member(self) -> bool {
        self.is_method_like()
            || matches!(
                self,
                CsSymbolKind::Property
                    | CsSymbolKind::Indexer
                    | CsSymbolKind::Event
                    | CsSymbolKind::Field
                    | CsSymbolKind::Constant
                    | CsSymbolKind::EnumMember
            )
    }

    pub fn core_kind(self) -> SymbolKind {
        match self {
            CsSymbolKind::Namespace => SymbolKind::SK_PACKAGE,
            CsSymbolKind::Class | CsSymbolKind::Struct | CsSymbolKind::Delegate => {
                SymbolKind::SK_CLASS
            }
            CsSymbolKind::Interface => SymbolKind::SK_INTERFACE,
            CsSymbolKind::Enum => SymbolKind::SK_ENUM,
            CsSymbolKind::Record | CsSymbolKind::RecordStruct => SymbolKind::SK_RECORD,
            CsSymbolKind::Method
            | CsSymbolKind::LocalFunction
            | CsSymbolKind::Destructor
            | CsSymbolKind::Operator
            | CsSymbolKind::ConversionOperator
            | CsSymbolKind::Indexer => SymbolKind::SK_METHOD,
            CsSymbolKind::Constructor => SymbolKind::SK_CONSTRUCTOR,
            CsSymbolKind::StaticConstructor => SymbolKind::SK_STATIC_INIT,
            CsSymbolKind::Property
            | CsSymbolKind::Event
            | CsSymbolKind::Field
            | CsSymbolKind::Constant => SymbolKind::SK_FIELD,
            CsSymbolKind::EnumMember => SymbolKind::SK_ENUM_CONSTANT,
            CsSymbolKind::Parameter => SymbolKind::SK_PARAM,
            CsSymbolKind::TypeParameter => SymbolKind::SK_TYPE_PARAM,
            CsSymbolKind::Local => SymbolKind::SK_LOCAL_VAR,
        }
    }

    /// The C# kind a Phase 2 declaration node stands for, when it determines one.
    pub fn from_node_type(node_type: ASTNodeType) -> Option<Self> {
        Some(match node_type {
            ASTNodeType::NN_NAMESPACE_DECL => CsSymbolKind::Namespace,
            ASTNodeType::NN_CLASS_DECL => CsSymbolKind::Class,
            ASTNodeType::NN_STRUCT_DECL => CsSymbolKind::Struct,
            ASTNodeType::NN_INTERFACE_DECL => CsSymbolKind::Interface,
            ASTNodeType::NN_ENUM_DECL => CsSymbolKind::Enum,
            ASTNodeType::NN_RECORD_DECL => CsSymbolKind::Record,
            ASTNodeType::NN_DELEGATE_DECL => CsSymbolKind::Delegate,
            ASTNodeType::NN_METHOD_DECL => CsSymbolKind::Method,
            ASTNodeType::NN_FUNCTION_DECL => CsSymbolKind::LocalFunction,
            ASTNodeType::NN_CONSTRUCTOR_DECL => CsSymbolKind::Constructor,
            ASTNodeType::NN_DESTRUCTOR_DECL => CsSymbolKind::Destructor,
            ASTNodeType::NN_OPERATOR_DECL => CsSymbolKind::Operator,
            ASTNodeType::NN_PROPERTY_DECL => CsSymbolKind::Property,
            ASTNodeType::NN_INDEXER_DECL => CsSymbolKind::Indexer,
            ASTNodeType::NN_EVENT_DECL => CsSymbolKind::Event,
            ASTNodeType::NN_FIELD_DECL => CsSymbolKind::Field,
            ASTNodeType::NN_ENUM_CONSTANT => CsSymbolKind::EnumMember,
            ASTNodeType::NN_PARAM_DECL => CsSymbolKind::Parameter,
            ASTNodeType::NN_TYPE_PARAM => CsSymbolKind::TypeParameter,
            ASTNodeType::NN_LOCAL_VAR_DECL => CsSymbolKind::Local,
            _ => return None,
        })
    }

    /// True when a declaration of this kind may be represented by an AST node of `node_type`.
    fn matches_node(self, node_type: ASTNodeType) -> bool {
        match self {
            CsSymbolKind::Class
            | CsSymbolKind::Struct
            | CsSymbolKind::Interface
            | CsSymbolKind::Enum
            | CsSymbolKind::Record
            | CsSymbolKind::RecordStruct
            | CsSymbolKind::Delegate => matches!(
                node_type,
                ASTNodeType::NN_CLASS_DECL
                    | ASTNodeType::NN_STRUCT_DECL
                    | ASTNodeType::NN_INTERFACE_DECL
                    | ASTNodeType::NN_ENUM_DECL
                    | ASTNodeType::NN_RECORD_DECL
                    | ASTNodeType::NN_DELEGATE_DECL
            ),
            CsSymbolKind::Method | CsSymbolKind::LocalFunction => matches!(
                node_type,
                ASTNodeType::NN_METHOD_DECL | ASTNodeType::NN_FUNCTION_DECL
            ),
            CsSymbolKind::Constructor | CsSymbolKind::StaticConstructor => {
                node_type == ASTNodeType::NN_CONSTRUCTOR_DECL
            }
            CsSymbolKind::Destructor => matches!(
                node_type,
                ASTNodeType::NN_DESTRUCTOR_DECL | ASTNodeType::NN_METHOD_DECL
            ),
            CsSymbolKind::Operator | CsSymbolKind::ConversionOperator => matches!(
                node_type,
                ASTNodeType::NN_OPERATOR_DECL | ASTNodeType::NN_METHOD_DECL
            ),
            CsSymbolKind::Property => node_type == ASTNodeType::NN_PROPERTY_DECL,
            CsSymbolKind::Indexer => matches!(
                node_type,
                ASTNodeType::NN_INDEXER_DECL | ASTNodeType::NN_PROPERTY_DECL
            ),
            CsSymbolKind::Event => matches!(
                node_type,
                ASTNodeType::NN_EVENT_DECL | ASTNodeType::NN_FIELD_DECL
            ),
            CsSymbolKind::Field | CsSymbolKind::Constant => {
                node_type == ASTNodeType::NN_FIELD_DECL
            }
            CsSymbolKind::EnumMember => node_type == ASTNodeType::NN_ENUM_CONSTANT,
            CsSymbolKind::Namespace => node_type == ASTNodeType::NN_NAMESPACE_DECL,
            CsSymbolKind::Parameter => node_type == ASTNodeType::NN_PARAM_DECL,
            CsSymbolKind::TypeParameter => node_type == ASTNodeType::NN_TYPE_PARAM,
            CsSymbolKind::Local => matches!(
                node_type,
                ASTNodeType::NN_LOCAL_VAR_DECL | ASTNodeType::NN_VAR_DECLARATOR
            ),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            CsSymbolKind::Namespace => "namespace",
            CsSymbolKind::Class => "class",
            CsSymbolKind::Struct => "struct",
            CsSymbolKind::Interface => "interface",
            CsSymbolKind::Enum => "enum",
            CsSymbolKind::Record => "record",
            CsSymbolKind::RecordStruct => "record struct",
            CsSymbolKind::Delegate => "delegate",
            CsSymbolKind::Method => "method",
            CsSymbolKind::LocalFunction => "local function",
            CsSymbolKind::Constructor => "constructor",
            CsSymbolKind::StaticConstructor => "static constructor",
            CsSymbolKind::Destructor => "finalizer",
            CsSymbolKind::Operator => "operator",
            CsSymbolKind::ConversionOperator => "conversion operator",
            CsSymbolKind::Property => "property",
            CsSymbolKind::Indexer => "indexer",
            CsSymbolKind::Event => "event",
            CsSymbolKind::Field => "field",
            CsSymbolKind::Constant => "constant",
            CsSymbolKind::EnumMember => "enum member",
            CsSymbolKind::Parameter => "parameter",
            CsSymbolKind::TypeParameter => "type parameter",
            CsSymbolKind::Local => "local",
        }
    }
}

/// The six C# accessibility levels, plus `NotApplicable` for locals, parameters and type
/// parameters.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CsAccessibility {
    Public = 0,
    ProtectedInternal = 1,
    Internal = 2,
    Protected = 3,
    PrivateProtected = 4,
    Private = 5,
    NotApplicable = 6,
}

impl CsAccessibility {
    /// Visibility in the shared alphabet: `internal` is assembly scope (`Package`), and both
    /// protected combinations keep the derived-type access that `Protected` denotes.
    pub fn to_core(self) -> SymbolVisibility {
        match self {
            CsAccessibility::Public | CsAccessibility::NotApplicable => SymbolVisibility::Public,
            CsAccessibility::Internal => SymbolVisibility::Package,
            CsAccessibility::Protected
            | CsAccessibility::ProtectedInternal
            | CsAccessibility::PrivateProtected => SymbolVisibility::Protected,
            CsAccessibility::Private => SymbolVisibility::Private,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            CsAccessibility::Public => "public",
            CsAccessibility::ProtectedInternal => "protected internal",
            CsAccessibility::Internal => "internal",
            CsAccessibility::Protected => "protected",
            CsAccessibility::PrivateProtected => "private protected",
            CsAccessibility::Private => "private",
            CsAccessibility::NotApplicable => "",
        }
    }
}

/// Bits of `CsSymbol::modifiers` and `CsDecl::modifiers`.
pub struct CsModifiers;

impl CsModifiers {
    pub const STATIC: u32 = 1 << 0;
    pub const ABSTRACT: u32 = 1 << 1;
    pub const VIRTUAL: u32 = 1 << 2;
    pub const OVERRIDE: u32 = 1 << 3;
    pub const SEALED: u32 = 1 << 4;
    pub const READONLY: u32 = 1 << 5;
    pub const CONST: u32 = 1 << 6;
    pub const EXTERN: u32 = 1 << 7;
    pub const NEW: u32 = 1 << 8;
    pub const PARTIAL: u32 = 1 << 9;
    pub const ASYNC: u32 = 1 << 10;
    pub const UNSAFE: u32 = 1 << 11;
    pub const VOLATILE: u32 = 1 << 12;
    pub const REQUIRED: u32 = 1 << 13;
    /// `file` type modifier (C# 11): visible only in its source file.
    pub const FILE_LOCAL: u32 = 1 << 14;
    /// `ref struct`, or a `ref` return / `ref` local.
    pub const REF: u32 = 1 << 15;
    pub const FIXED: u32 = 1 << 16;
    /// Extension method (first parameter has `this`), or the `this` parameter itself.
    pub const EXTENSION: u32 = 1 << 17;
    /// Method whose last parameter is `params`, or the `params` parameter itself.
    pub const PARAMS: u32 = 1 << 18;
    /// Synthesized by a language rule (record members, default constructors, delegate
    /// `Invoke`, top-level `Program`).
    pub const IMPLICIT_DECL: u32 = 1 << 19;
    /// Primary constructor of a record, class or struct.
    pub const PRIMARY_CTOR: u32 = 1 << 20;
    /// Explicit interface member implementation (`void IFoo.Bar()`).
    pub const EXPLICIT_IMPL: u32 = 1 << 21;
    /// Positional record property synthesized from a primary-constructor parameter.
    pub const POSITIONAL: u32 = 1 << 22;
    /// Annotated `[Obsolete]`.
    pub const OBSOLETE: u32 = 1 << 23;
    pub const EXPRESSION_BODY: u32 = 1 << 24;
    pub const HAS_BODY: u32 = 1 << 25;
    /// `operator checked` (C# 11).
    pub const CHECKED: u32 = 1 << 26;
    /// The synthesized entry point of top-level statements.
    pub const TOP_LEVEL: u32 = 1 << 27;
    pub const REF_READONLY: u32 = 1 << 28;
    pub const IMPLICIT_CONVERSION: u32 = 1 << 29;
    pub const EXPLICIT_CONVERSION: u32 = 1 << 30;
    /// `scoped` parameter or local.
    pub const SCOPED: u32 = 1 << 31;
}

/// Bits of `CsSymbol::accessors`.
pub struct CsAccessors;

impl CsAccessors {
    pub const GET: u8 = 1 << 0;
    pub const SET: u8 = 1 << 1;
    pub const INIT: u8 = 1 << 2;
    pub const ADD: u8 = 1 << 3;
    pub const REMOVE: u8 = 1 << 4;
    /// Auto-implemented property or field-like event (accessors without bodies).
    pub const AUTO: u8 = 1 << 5;
}

/// Passing mode of a parameter.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CsRefKind {
    None = 0,
    Ref = 1,
    Out = 2,
    In = 3,
    RefReadonly = 4,
}

/// Variance annotation of a generic type parameter.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CsVariance {
    None = 0,
    In = 1,
    Out = 2,
}

// ── Syntax ─────────────────────────────────────────────────────────────────

/// One `.`-separated segment of a written name with its type argument list.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CsNameSegment {
    pub name: String,
    pub args: Vec<CsTypeSyntax>,
}

/// A type as written in source.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CsTypeSyntax {
    /// Simple-type keyword (`int`, `string`, `object`, `dynamic`, `nint`, ...).
    Predefined(String),
    /// Possibly qualified, possibly generic name; `alias` is the `X` of `X::Name`.
    Named {
        alias: Option<String>,
        segments: Vec<CsNameSegment>,
    },
    Array {
        element: Box<CsTypeSyntax>,
        rank: u8,
    },
    Nullable(Box<CsTypeSyntax>),
    Pointer(Box<CsTypeSyntax>),
    Tuple(Vec<(CsTypeSyntax, Option<String>)>),
    /// `delegate*<A, B, R>`: parameter types followed by the return type.
    FunctionPointer(Vec<CsTypeSyntax>),
    Ref {
        readonly: bool,
        inner: Box<CsTypeSyntax>,
    },
    /// Omitted type argument of an unbound generic name (`List<>`).
    Omitted,
}

impl CsTypeSyntax {
    fn simple(name: &str) -> Self {
        CsTypeSyntax::Named {
            alias: None,
            segments: vec![CsNameSegment {
                name: name.to_string(),
                args: Vec::new(),
            }],
        }
    }

    /// The single identifier of an unqualified, non-generic name.
    pub fn as_simple_name(&self) -> Option<&str> {
        match self {
            CsTypeSyntax::Named { alias: None, segments }
                if segments.len() == 1 && segments[0].args.is_empty() =>
            {
                Some(segments[0].name.as_str())
            }
            _ => None,
        }
    }

    pub fn display(&self) -> String {
        match self {
            CsTypeSyntax::Predefined(k) => k.clone(),
            CsTypeSyntax::Named { alias, segments } => {
                let mut s = String::new();
                if let Some(a) = alias {
                    s.push_str(a);
                    s.push_str("::");
                }
                for (i, seg) in segments.iter().enumerate() {
                    if i > 0 {
                        s.push('.');
                    }
                    s.push_str(&seg.name);
                    if !seg.args.is_empty() {
                        s.push('<');
                        let parts: Vec<String> = seg.args.iter().map(|a| a.display()).collect();
                        s.push_str(&parts.join(", "));
                        s.push('>');
                    }
                }
                s
            }
            CsTypeSyntax::Array { element, rank } => {
                format!("{}[{}]", element.display(), ",".repeat((*rank as usize).saturating_sub(1)))
            }
            CsTypeSyntax::Nullable(inner) => format!("{}?", inner.display()),
            CsTypeSyntax::Pointer(inner) => format!("{}*", inner.display()),
            CsTypeSyntax::Tuple(elems) => {
                let parts: Vec<String> = elems
                    .iter()
                    .map(|(t, n)| match n {
                        Some(n) => format!("{} {}", t.display(), n),
                        None => t.display(),
                    })
                    .collect();
                format!("({})", parts.join(", "))
            }
            CsTypeSyntax::FunctionPointer(ts) => {
                let parts: Vec<String> = ts.iter().map(|t| t.display()).collect();
                format!("delegate*<{}>", parts.join(", "))
            }
            CsTypeSyntax::Ref { readonly, inner } => {
                if *readonly {
                    format!("ref readonly {}", inner.display())
                } else {
                    format!("ref {}", inner.display())
                }
            }
            CsTypeSyntax::Omitted => String::new(),
        }
    }
}

/// One attribute of an attribute section (`[target: Name(args)]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CsAttributeSyntax {
    pub target: Option<String>,
    pub name: CsTypeSyntax,
    /// Argument texts in source order, tokens joined by single spaces.
    pub args: Vec<String>,
    pub token: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CsTypeParamSyntax {
    pub name: String,
    pub variance: CsVariance,
    pub token: u32,
    pub attributes: Vec<CsAttributeSyntax>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CsConstraintSyntax {
    Class { nullable: bool },
    Struct,
    Unmanaged,
    NotNull,
    Default,
    New,
    Type(CsTypeSyntax),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CsParamSyntax {
    pub name: String,
    pub token: u32,
    /// `None` for `__arglist` and implicitly typed lambda parameters.
    pub ty: Option<CsTypeSyntax>,
    pub ref_kind: CsRefKind,
    pub is_params: bool,
    pub is_this: bool,
    pub is_scoped: bool,
    pub has_default: bool,
    pub default_text: String,
    pub attributes: Vec<CsAttributeSyntax>,
}

/// A using directive as written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CsUsingSyntax {
    pub global: bool,
    pub is_static: bool,
    pub is_unsafe: bool,
    pub alias: Option<String>,
    pub target: CsTypeSyntax,
    pub token: u32,
    pub file_id: u16,
}

/// One source (or BCL signature) declaration. Partial types have one `CsDecl` per part and
/// one `CsSymbol`; a field declaration with several declarators has one `CsDecl` each.
#[derive(Debug, Clone)]
pub struct CsDecl {
    pub id: u32,
    pub kind: CsSymbolKind,
    pub name: String,
    pub name_token: u32,
    pub file_id: u16,
    pub first_token: u32,
    /// Last token of the declaration (closing brace or semicolon).
    pub end_token: u32,
    pub parent: u32,
    pub import_scope: u32,
    /// Phase 2 node of the declaration, or `u32::MAX` when none was linked.
    pub node: u32,
    pub declared_accessibility: Option<CsAccessibility>,
    pub modifiers: u32,
    pub attributes: Vec<CsAttributeSyntax>,
    pub type_params: Vec<CsTypeParamSyntax>,
    pub constraints: Vec<(String, Vec<CsConstraintSyntax>)>,
    pub params: Vec<CsParamSyntax>,
    /// Records and C# 12 primary constructors: the declaration has a parameter list.
    pub has_param_list: bool,
    /// Return, field, property, event, delegate-return or enum-underlying type.
    pub ty: Option<CsTypeSyntax>,
    pub bases: Vec<CsTypeSyntax>,
    /// The first entry of `bases` is known to be the base class (BCL entries).
    pub explicit_base_class: bool,
    pub explicit_interface: Option<CsTypeSyntax>,
    pub accessors: u8,
    pub accessor_accessibility: Vec<(u8, CsAccessibility)>,
    /// Operator token(s) of an operator declaration (`+`, `==`, `true`, `implicit`).
    pub operator: String,
    pub symbol: u32,
    /// Namespace declarations: `namespace X;` form.
    pub file_scoped: bool,
}

impl CsDecl {
    fn new(kind: CsSymbolKind, name: String, name_token: u32, first_token: u32) -> Self {
        Self {
            id: u32::MAX,
            kind,
            name,
            name_token,
            file_id: u16::MAX,
            first_token,
            end_token: first_token,
            parent: u32::MAX,
            import_scope: u32::MAX,
            node: u32::MAX,
            declared_accessibility: None,
            modifiers: 0,
            attributes: Vec::new(),
            type_params: Vec::new(),
            constraints: Vec::new(),
            params: Vec::new(),
            has_param_list: false,
            ty: None,
            bases: Vec::new(),
            explicit_base_class: false,
            explicit_interface: None,
            accessors: 0,
            accessor_accessibility: Vec::new(),
            operator: String::new(),
            symbol: u32::MAX,
            file_scoped: false,
        }
    }
}

// ── Semantic entities ──────────────────────────────────────────────────────

/// A resolved type.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CsType {
    /// A class, struct, interface, enum, record or delegate, with its type arguments.
    Named { symbol: u32, args: Vec<CsType> },
    TypeParam(u32),
    Array { element: Box<CsType>, rank: u8 },
    Pointer(Box<CsType>),
    /// A reference type with a nullable annotation (`string?`). Nullable value types are
    /// `Named` instances of `System.Nullable<T>`.
    NullableRef(Box<CsType>),
    FunctionPointer(Vec<CsType>),
    Dynamic,
    /// No declaration, import or BCL entry names the type; `name` is the qualified name as far
    /// as it could be determined.
    Unresolved { name: String, arity: u16 },
    /// More than one type is visible at the same precedence level.
    Ambiguous { name: String, candidates: Vec<u32> },
}

impl CsType {
    pub fn named(symbol: u32) -> Self {
        CsType::Named {
            symbol,
            args: Vec::new(),
        }
    }

    /// The class, struct, interface, enum, record or delegate symbol at the head.
    pub fn head_symbol(&self) -> Option<u32> {
        match self {
            CsType::Named { symbol, .. } => Some(*symbol),
            CsType::NullableRef(inner) => inner.head_symbol(),
            _ => None,
        }
    }

    pub fn is_error(&self) -> bool {
        matches!(self, CsType::Unresolved { .. } | CsType::Ambiguous { .. })
    }

    fn unknown() -> Self {
        CsType::Unresolved {
            name: String::new(),
            arity: 0,
        }
    }

    /// Removes a nullable reference annotation.
    fn strip_nullable_ref(&self) -> &CsType {
        match self {
            CsType::NullableRef(inner) => inner.strip_nullable_ref(),
            other => other,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CsConstraint {
    Class { nullable: bool },
    Struct,
    Unmanaged,
    NotNull,
    Default,
    New,
    Type(CsType),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CsAttributeUse {
    pub target: Option<String>,
    pub ty: CsType,
    pub args: Vec<String>,
}

/// Where a symbol comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CsOrigin {
    Source,
    /// A BCL type, with its stub symbol id.
    Library(u32),
    /// A member, parameter or type parameter of a BCL type.
    LibraryMember,
    /// Synthesized by a language rule for a source declaration.
    Synthesized,
}

#[derive(Debug, Clone)]
pub struct CsSymbol {
    pub id: u32,
    pub kind: CsSymbolKind,
    pub name: String,
    /// Containing namespace or type (members), owning member or type (parameters and type
    /// parameters). `u32::MAX` for the global namespace.
    pub container: u32,
    pub origin: CsOrigin,
    pub accessibility: CsAccessibility,
    /// The accessibility was written in source (as opposed to defaulted).
    pub declared_accessibility: bool,
    pub modifiers: u32,
    pub decls: Vec<u32>,
    /// Token naming the symbol (synthesized positional properties use their parameter).
    pub name_token: u32,
    pub type_params: Vec<u32>,
    pub params: Vec<u32>,
    pub members: Vec<u32>,
    /// Return, field, property, event, parameter or delegate-return type; enum underlying type.
    pub ty: Option<CsType>,
    pub base_class: Option<CsType>,
    pub interfaces: Vec<CsType>,
    /// Supertypes as written in the base lists (no implicit `object` / `ValueType`).
    pub declared_supertypes: Vec<CsType>,
    pub explicit_interface: Option<CsType>,
    pub attributes: Vec<CsAttributeUse>,
    pub accessors: u8,
    pub ref_kind: CsRefKind,
    pub has_default: bool,
    pub variance: CsVariance,
    pub constraints: Vec<CsConstraint>,
    pub ordinal: u16,
    /// Member this one overrides, or `u32::MAX`.
    pub overridden: u32,
    /// Interface members this member implements.
    pub implements: Vec<u32>,
}

impl CsSymbol {
    fn new(kind: CsSymbolKind, name: &str, container: u32, origin: CsOrigin) -> Self {
        Self {
            id: u32::MAX,
            kind,
            name: name.to_string(),
            container,
            origin,
            accessibility: CsAccessibility::NotApplicable,
            declared_accessibility: false,
            modifiers: 0,
            decls: Vec::new(),
            name_token: u32::MAX,
            type_params: Vec::new(),
            params: Vec::new(),
            members: Vec::new(),
            ty: None,
            base_class: None,
            interfaces: Vec::new(),
            declared_supertypes: Vec::new(),
            explicit_interface: None,
            attributes: Vec::new(),
            accessors: 0,
            ref_kind: CsRefKind::None,
            has_default: false,
            variance: CsVariance::None,
            constraints: Vec::new(),
            ordinal: 0,
            overridden: u32::MAX,
            implements: Vec::new(),
        }
    }

    pub fn has(&self, modifier: u32) -> bool {
        (self.modifiers & modifier) != 0
    }

    pub fn is_library(&self) -> bool {
        matches!(self.origin, CsOrigin::Library(_) | CsOrigin::LibraryMember)
    }
}

/// Target of a using alias.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CsAliasTarget {
    Namespace(u32),
    Type(CsType),
    /// A namespace or type of an assembly outside the model (or an extern alias).
    External(String),
}

/// The using directives of one compilation unit or namespace body.
#[derive(Debug, Clone)]
pub struct CsImportScope {
    pub id: u32,
    pub parent: u32,
    pub file_id: u16,
    /// Namespace this body declares members into (the global namespace for compilation units).
    pub namespace: u32,
    pub is_compilation_unit: bool,
    pub usings: Vec<CsUsingSyntax>,
    pub extern_aliases: Vec<String>,
    pub using_namespaces: Vec<u32>,
    /// Imported namespaces that no modelled assembly declares.
    pub external_namespaces: Vec<String>,
    pub using_static: Vec<CsType>,
    pub aliases: HashMap<String, CsAliasTarget>,
}

impl CsImportScope {
    fn new(id: u32, parent: u32, file_id: u16, namespace: u32, is_cu: bool) -> Self {
        Self {
            id,
            parent,
            file_id,
            namespace,
            is_compilation_unit: is_cu,
            usings: Vec::new(),
            extern_aliases: Vec::new(),
            using_namespaces: Vec::new(),
            external_namespaces: Vec::new(),
            using_static: Vec::new(),
            aliases: HashMap::new(),
        }
    }
}

/// Global usings of the project: `global using` directives, MSBuild `<Using>` items and the
/// SDK implicit usings. They apply to every compilation unit.
#[derive(Debug, Clone, Default)]
pub struct CsGlobalUsings {
    pub syntax: Vec<CsUsingSyntax>,
    pub namespaces: Vec<u32>,
    pub external_namespaces: Vec<String>,
    pub statics: Vec<CsType>,
    pub aliases: HashMap<String, CsAliasTarget>,
}

/// Per-file data.
#[derive(Debug, Clone)]
pub struct CsFileInfo {
    pub file_id: u16,
    pub unit_scope: u32,
    pub first_token: u32,
    pub last_token: u32,
    /// Phase 2 root node of the file, or `u32::MAX`.
    pub ast_root: u32,
    /// `(first_token, end_token, decl)` of every declaration of the file, by first token.
    pub decl_spans: Vec<(u32, u32, u32)>,
    pub attributes: Vec<CsAttributeSyntax>,
    pub bound_attributes: Vec<CsAttributeUse>,
    pub has_top_level_statements: bool,
}

/// When the SDK implicit usings apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CsImplicitUsings {
    Disabled,
    Enabled,
    /// Enabled unless some compilation unit writes `using System;` itself (projects with
    /// `<ImplicitUsings>enable</ImplicitUsings>` do not).
    Auto,
}

/// Project-level inputs (normally filled from the MSBuild project model).
#[derive(Debug, Clone)]
pub struct CsProjectOptions {
    pub sdk: Option<DotNetSdk>,
    pub implicit_usings: CsImplicitUsings,
    /// Preprocessor symbols defined for the analysed configuration.
    pub defined_symbols: Vec<String>,
    /// MSBuild `<Using>` items in directive syntax: `System.Text`, `static System.Math`,
    /// `Json = System.Text.Json`.
    pub global_usings: Vec<String>,
    pub assembly_name: String,
    /// Assembly of each file when the corpus spans several projects.
    pub file_assemblies: HashMap<u16, String>,
    pub root_namespace: Option<String>,
}

impl Default for CsProjectOptions {
    /// An SDK-style project analysed in its Debug configuration.
    fn default() -> Self {
        Self {
            sdk: Some(DotNetSdk::Base),
            implicit_usings: CsImplicitUsings::Auto,
            defined_symbols: vec![
                "DEBUG".to_string(),
                "TRACE".to_string(),
                "NET".to_string(),
                "NETCOREAPP".to_string(),
            ],
            global_usings: Vec::new(),
            assembly_name: String::new(),
            file_assemblies: HashMap::new(),
            root_namespace: None,
        }
    }
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CsDiagnosticCode {
    UnresolvedType = 1,
    AmbiguousType = 2,
    UnresolvedNamespace = 3,
    DuplicateType = 4,
    PartialMismatch = 5,
    MultipleBaseClasses = 6,
    HierarchyCycle = 7,
    AssumedRelation = 8,
    UnresolvedAttribute = 9,
    AmbiguousAttribute = 10,
    MalformedDeclaration = 11,
    ConflictingAccessibility = 12,
    InvalidUsing = 13,
}

/// A finding about the input that does not invalidate the model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CsDiagnostic {
    pub code: CsDiagnosticCode,
    pub file_id: u16,
    pub token: u32,
    pub symbol: u32,
    pub message: String,
}

/// Namespace-or-type lookup result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CsNsOrType {
    Namespace(u32),
    Type(CsType),
    /// A namespace or type of an assembly outside the model, by qualified name.
    External(String),
    NotFound,
}

/// Location at which a name is resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CsContext {
    pub file_id: u16,
    pub import_scope: u32,
    /// Innermost enclosing type, or `u32::MAX`.
    pub type_sym: u32,
    /// Innermost enclosing method-like member (its type parameters are in scope), or `u32::MAX`.
    pub member_sym: u32,
    /// Resolving the base list of `type_sym`: only its type parameters, not its members, are
    /// in scope.
    pub in_base_list: bool,
    /// Import scope whose own using directives are ignored (resolving those directives).
    pub skip_usings_of: u32,
    pub skip_global_usings: bool,
}

/// An argument of an invocation, for overload resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CsArgType {
    Typed(CsType),
    Null,
    /// `default` literal.
    Default,
    /// Integer constant (the C# constant-expression conversions apply).
    IntLiteral,
    /// Lambda or anonymous method with this many parameters.
    Lambda(u16),
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CsArgument {
    /// Named argument (`name: value`).
    pub name: Option<String>,
    pub ty: CsArgType,
    pub ref_kind: CsRefKind,
}

impl CsArgument {
    pub fn positional(ty: CsArgType) -> Self {
        Self {
            name: None,
            ty,
            ref_kind: CsRefKind::None,
        }
    }
}

/// Overload resolution outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CsOverloadResult {
    /// The best function member, with its parameter types after substitution and inference,
    /// and whether it applies in its expanded (`params`) form.
    Resolved {
        symbol: u32,
        param_types: Vec<CsType>,
        expanded: bool,
    },
    Ambiguous(Vec<u32>),
    /// Candidates existed but none is applicable to the arguments.
    NoneApplicable(Vec<u32>),
    NoCandidates,
}

/// Result of resolving an invocation `receiver.name<type_args>(args)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CsInvocation {
    pub result: CsOverloadResult,
    /// The method was found as an extension method (the receiver is its first argument).
    pub is_extension: bool,
}

/// Identifiers of special types the language rules refer to (symbol ids in the arena).
#[derive(Debug, Clone, Default)]
pub struct CsSpecialTypes {
    pub object: u32,
    pub string: u32,
    pub value_type: u32,
    pub enum_: u32,
    pub array: u32,
    pub delegate: u32,
    pub multicast_delegate: u32,
    pub attribute: u32,
    pub void: u32,
    pub boolean: u32,
    pub int32: u32,
    pub intptr: u32,
    pub nullable_t: u32,
    pub ienumerable_t: u32,
    pub type_: u32,
    pub iequatable_t: u32,
    pub string_builder: u32,
    pub async_callback: u32,
    pub iasync_result: u32,
    pub task: u32,
    pub task_t: u32,
    /// `ValueTuple`1` .. `ValueTuple`8`.
    pub value_tuples: Vec<u32>,
    /// Array-implemented generic interfaces (`IList<T>`, `ICollection<T>`, ...).
    pub array_interfaces: Vec<u32>,
    /// Numeric special types in `NUMERIC_KEYWORDS` order.
    pub numerics: Vec<u32>,
    pub char_: u32,
}

/// Simple types taking part in the implicit numeric conversions, in table order.
const NUMERIC_KEYWORDS: &[&str] = &[
    "sbyte", "byte", "short", "ushort", "int", "uint", "long", "ulong", "char", "float", "double",
    "decimal", "nint", "nuint",
];

/// Implicit numeric conversions (C# spec §10.2.3), as target indices into `NUMERIC_KEYWORDS`.
const IMPLICIT_NUMERIC: &[&[usize]] = &[
    /* sbyte  */ &[2, 4, 6, 9, 10, 11, 12],
    /* byte   */ &[2, 3, 4, 5, 6, 7, 9, 10, 11, 12, 13],
    /* short  */ &[4, 6, 9, 10, 11, 12],
    /* ushort */ &[4, 5, 6, 7, 9, 10, 11, 12, 13],
    /* int    */ &[6, 9, 10, 11, 12],
    /* uint   */ &[6, 7, 9, 10, 11, 13],
    /* long   */ &[9, 10, 11],
    /* ulong  */ &[9, 10, 11],
    /* char   */ &[3, 4, 5, 6, 7, 9, 10, 11, 12, 13],
    /* float  */ &[10],
    /* double */ &[],
    /* decimal*/ &[],
    /* nint   */ &[6, 9, 10, 11],
    /* nuint  */ &[7, 9, 10, 11],
];

/// File id used for BCL declarations.
const LIB_FILE_ID: u16 = u16::MAX;
/// File id of the project-level scope that binds MSBuild and implicit global usings.
const PROJECT_FILE_ID: u16 = u16::MAX - 1;
/// Name of the assembly BCL symbols belong to.
const BCL_ASSEMBLY: &str = "<bcl>";
