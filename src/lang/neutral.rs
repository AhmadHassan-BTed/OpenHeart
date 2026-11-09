//! Universal concept vocabulary of the analysis engine.
//!
//! Every language frontend maps its grammar into these concepts (through the shared
//! `ASTNodeType` alphabet, the `NodeAttr` word and the symbol table), and every algorithm of
//! Phases 3–10 — CFG and dominators, SSA, call graph and points-to, ROBDD path summaries,
//! traceability, UML extraction, pattern detection and the exporters — reasons in these
//! concepts instead of in a language's own spelling.
//!
//! The vocabulary is a superset across languages: it names the constructs of Java, Kotlin,
//! C, C++ and C# and leaves room for Python, TypeScript, Go, Rust and Swift (protocols,
//! traits, extensions, `defer`, `with`, ownership-typed pointers) without a schema change.
//!
//! Conversions to the existing encodings (`ASTNodeType`, `SymbolKind`, `SymbolVisibility`,
//! `SymbolModifiers`, the `NodeAttr` bit fields, call-edge codes and `AssocKind`) are given
//! wherever a counterpart exists. A conversion that has to pick the closest counterpart says
//! so (`symbol_kind_is_exact`, `Option` results); nothing collapses silently.

use std::fmt;
use std::ops::{BitAnd, BitAndAssign, BitOr, BitOrAssign, Sub};

use crate::core::types::ast::{ASTNodeType, NodeAttr};
use crate::core::types::cg::{
    CG_EDGE_CONSTRUCTOR, CG_EDGE_DIRECT, CG_EDGE_DYNAMIC, CG_EDGE_INTERFACE, CG_EDGE_REFLECTION,
    CG_EDGE_SPECIAL, CG_EDGE_VIRTUAL,
};
use crate::core::types::symbol::{
    AssocKind, SymbolKind, SymbolModifiers, SymbolVisibility, THRelation,
};

// ── Containers, types, members, declarations ─────────────────────────────────

/// A declaration that groups other declarations and qualifies their names.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ContainerKind {
    /// Java / Kotlin / Go `package`.
    Package = 1,
    /// C++ / C# / TypeScript `namespace`.
    Namespace = 2,
    /// Java `module-info`, C++20 module, Rust `mod`, Python or Swift module.
    Module = 3,
    /// The top-level declarations of one source file (compilation / translation unit).
    FileScope = 4,
}

impl ContainerKind {
    pub const ALL: [ContainerKind; 4] = [
        ContainerKind::Package,
        ContainerKind::Namespace,
        ContainerKind::Module,
        ContainerKind::FileScope,
    ];

    pub fn from_u8(val: u8) -> Option<Self> {
        Self::ALL.iter().copied().find(|k| *k as u8 == val)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ContainerKind::Package => "package",
            ContainerKind::Namespace => "namespace",
            ContainerKind::Module => "module",
            ContainerKind::FileScope => "file",
        }
    }

    /// True when the container contributes a segment to the qualified names of its members.
    pub fn qualifies_names(self) -> bool {
        !matches!(self, ContainerKind::FileScope)
    }
}

/// A declaration that introduces (or names) a type.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TypeKind {
    Class = 0,
    /// C / C++ / C# / Swift `struct`, Go struct type.
    Struct = 1,
    /// Interface, protocol (Swift, Objective-C) or trait (Rust, Scala, PHP).
    Interface = 2,
    Enum = 3,
    /// Java `record`, Kotlin `data class`, C# `record`.
    Record = 4,
    /// C / C++ `union`.
    Union = 5,
    /// Kotlin / Scala `object`: a type with exactly one instance.
    Object = 6,
    /// `typedef`, C++ `using X = Y;`, Kotlin / Swift / TypeScript `typealias` / `type`.
    TypeAlias = 7,
    /// C# `delegate`, a named function type.
    Delegate = 8,
    /// Java `@interface`, C# attribute class, Python decorator type.
    Annotation = 9,
    /// Dart / Ruby / Scala mixin: a reusable member set composed into classes.
    Mixin = 10,
    /// Swift `extension`, Objective-C category, C# extension container: members added to
    /// an existing type.
    Extension = 11,
    /// Anonymous class body (`new T() { ... }`, Kotlin object expression).
    AnonymousClass = 12,
}

impl TypeKind {
    pub const ALL: [TypeKind; 13] = [
        TypeKind::Class,
        TypeKind::Struct,
        TypeKind::Interface,
        TypeKind::Enum,
        TypeKind::Record,
        TypeKind::Union,
        TypeKind::Object,
        TypeKind::TypeAlias,
        TypeKind::Delegate,
        TypeKind::Annotation,
        TypeKind::Mixin,
        TypeKind::Extension,
        TypeKind::AnonymousClass,
    ];

    pub fn from_u8(val: u8) -> Option<Self> {
        Self::ALL.get(val as usize).copied()
    }

    pub fn as_str(self) -> &'static str {
        match self {
            TypeKind::Class => "class",
            TypeKind::Struct => "struct",
            TypeKind::Interface => "interface",
            TypeKind::Enum => "enum",
            TypeKind::Record => "record",
            TypeKind::Union => "union",
            TypeKind::Object => "object",
            TypeKind::TypeAlias => "typealias",
            TypeKind::Delegate => "delegate",
            TypeKind::Annotation => "annotation",
            TypeKind::Mixin => "mixin",
            TypeKind::Extension => "extension",
            TypeKind::AnonymousClass => "anonymous class",
        }
    }

    /// UML keyword / stereotype shown in a class box (`None` for a plain class).
    pub fn uml_keyword(self) -> Option<&'static str> {
        match self {
            TypeKind::Class | TypeKind::AnonymousClass => None,
            TypeKind::Struct => Some("struct"),
            TypeKind::Interface => Some("interface"),
            TypeKind::Enum => Some("enumeration"),
            TypeKind::Record => Some("record"),
            TypeKind::Union => Some("union"),
            TypeKind::Object => Some("singleton"),
            TypeKind::TypeAlias => Some("typedef"),
            TypeKind::Delegate => Some("delegate"),
            TypeKind::Annotation => Some("annotation"),
            TypeKind::Mixin => Some("mixin"),
            TypeKind::Extension => Some("extension"),
        }
    }

    /// Types that can never be instantiated directly.
    pub fn is_abstract_by_nature(self) -> bool {
        matches!(self, TypeKind::Interface | TypeKind::Mixin)
    }

    /// False for declarations that only name or extend an existing type.
    pub fn defines_new_type(self) -> bool {
        !matches!(self, TypeKind::TypeAlias | TypeKind::Extension)
    }

    /// Instances are values (copied on assignment) in languages that distinguish value types
    /// from reference types (C#, Swift). C and C++ treat every type as a value; see
    /// [`AggregateSemantics`].
    pub fn is_value_type_by_default(self) -> bool {
        matches!(self, TypeKind::Struct | TypeKind::Enum | TypeKind::Union)
    }

    /// Declarations that may name supertypes or conformances.
    pub fn can_have_supertypes(self) -> bool {
        matches!(
            self,
            TypeKind::Class
                | TypeKind::Struct
                | TypeKind::Interface
                | TypeKind::Enum
                | TypeKind::Record
                | TypeKind::Object
                | TypeKind::Mixin
                | TypeKind::Extension
                | TypeKind::AnonymousClass
        )
    }

    /// Instances of the type are invocable (a delegate / function type).
    pub fn is_callable_type(self) -> bool {
        matches!(self, TypeKind::Delegate)
    }

    /// Supertypes of this kind are realized (`implements`) rather than specialized.
    pub fn is_realized_by_subtypes(self) -> bool {
        matches!(self, TypeKind::Interface | TypeKind::Mixin | TypeKind::Annotation)
    }
}

/// A declaration that belongs to a type.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MemberKind {
    Field = 0,
    /// C# / Kotlin / Swift property (storage or accessor pair behind one name).
    Property = 1,
    /// C# indexer `this[...]`.
    Indexer = 2,
    /// C# `event`.
    Event = 3,
    Method = 4,
    Constructor = 5,
    /// C++ destructor, C# finalizer, Python `__del__`.
    Destructor = 6,
    /// Overloaded operator (`operator+`).
    Operator = 7,
    /// User-defined conversion (C++ `operator T()`, C# `implicit` / `explicit operator`).
    Conversion = 8,
    /// `get` / `set` / `init` / `add` / `remove` accessor of a property, indexer or event.
    Accessor = 9,
    /// Static initializer block (Java `static { }`, Kotlin companion `init`).
    StaticInitializer = 10,
    /// Instance initializer block (Java `{ }`, Kotlin `init { }`).
    InstanceInitializer = 11,
    EnumConstant = 12,
    /// A type declared inside a type, listed as a member of its owner.
    NestedType = 13,
}

impl MemberKind {
    pub const ALL: [MemberKind; 14] = [
        MemberKind::Field,
        MemberKind::Property,
        MemberKind::Indexer,
        MemberKind::Event,
        MemberKind::Method,
        MemberKind::Constructor,
        MemberKind::Destructor,
        MemberKind::Operator,
        MemberKind::Conversion,
        MemberKind::Accessor,
        MemberKind::StaticInitializer,
        MemberKind::InstanceInitializer,
        MemberKind::EnumConstant,
        MemberKind::NestedType,
    ];

    pub fn from_u8(val: u8) -> Option<Self> {
        Self::ALL.get(val as usize).copied()
    }

    pub fn as_str(self) -> &'static str {
        match self {
            MemberKind::Field => "field",
            MemberKind::Property => "property",
            MemberKind::Indexer => "indexer",
            MemberKind::Event => "event",
            MemberKind::Method => "method",
            MemberKind::Constructor => "constructor",
            MemberKind::Destructor => "destructor",
            MemberKind::Operator => "operator",
            MemberKind::Conversion => "conversion",
            MemberKind::Accessor => "accessor",
            MemberKind::StaticInitializer => "static initializer",
            MemberKind::InstanceInitializer => "instance initializer",
            MemberKind::EnumConstant => "enum constant",
            MemberKind::NestedType => "nested type",
        }
    }

    /// Members with an executable body that owns a CFG.
    pub fn is_callable(self) -> bool {
        matches!(
            self,
            MemberKind::Method
                | MemberKind::Constructor
                | MemberKind::Destructor
                | MemberKind::Operator
                | MemberKind::Conversion
                | MemberKind::Accessor
                | MemberKind::StaticInitializer
                | MemberKind::InstanceInitializer
        )
    }

    /// Members rendered in the attribute compartment of a UML class.
    pub fn is_uml_attribute(self) -> bool {
        matches!(
            self,
            MemberKind::Field | MemberKind::Property | MemberKind::Event | MemberKind::EnumConstant
        )
    }

    /// Members rendered in the operation compartment of a UML class.
    pub fn is_uml_operation(self) -> bool {
        matches!(
            self,
            MemberKind::Method
                | MemberKind::Constructor
                | MemberKind::Destructor
                | MemberKind::Operator
                | MemberKind::Conversion
                | MemberKind::Indexer
        )
    }

    /// Construction and destruction hooks of the instance or type life cycle.
    pub fn is_lifecycle(self) -> bool {
        matches!(
            self,
            MemberKind::Constructor
                | MemberKind::Destructor
                | MemberKind::StaticInitializer
                | MemberKind::InstanceInitializer
        )
    }

    /// UML standard-profile stereotype of the member, when one applies.
    pub fn uml_stereotype(self) -> Option<&'static str> {
        match self {
            MemberKind::Property => Some("property"),
            MemberKind::Indexer => Some("indexer"),
            MemberKind::Event => Some("event"),
            MemberKind::Constructor => Some("create"),
            MemberKind::Destructor => Some("destroy"),
            MemberKind::Operator => Some("operator"),
            MemberKind::Conversion => Some("conversion"),
            _ => None,
        }
    }
}

/// The neutral kind of every declaration a symbol table can hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeclKind {
    Container(ContainerKind),
    Type(TypeKind),
    Member(MemberKind),
    /// Free (namespace-scope or top-level) function.
    Function,
    /// Free (namespace-scope or top-level) variable or constant.
    Variable,
    Parameter,
    Local,
    TypeParameter,
    Lambda,
    /// Statement label (target of `goto`, labeled `break` / `continue`).
    Label,
    /// Preprocessor macro (`#define`).
    Macro,
    /// A declaration outside the analysed sources whose kind is not known.
    External,
}

impl DeclKind {
    /// Stable one-byte code, for persisting the neutral kind next to a `SymbolRecord`.
    /// `0` is never produced.
    pub fn code(self) -> u8 {
        match self {
            DeclKind::Container(k) => k as u8,
            DeclKind::Type(k) => 0x10 + k as u8,
            DeclKind::Member(k) => 0x20 + k as u8,
            DeclKind::Function => 0x40,
            DeclKind::Variable => 0x41,
            DeclKind::Parameter => 0x42,
            DeclKind::Local => 0x43,
            DeclKind::TypeParameter => 0x44,
            DeclKind::Lambda => 0x45,
            DeclKind::Label => 0x46,
            DeclKind::Macro => 0x47,
            DeclKind::External => 0x48,
        }
    }

    /// Inverse of [`DeclKind::code`]; `None` for a code no kind has.
    pub fn from_code(code: u8) -> Option<Self> {
        match code {
            0x01..=0x0F => ContainerKind::from_u8(code).map(DeclKind::Container),
            0x10..=0x1F => TypeKind::from_u8(code - 0x10).map(DeclKind::Type),
            0x20..=0x3F => MemberKind::from_u8(code - 0x20).map(DeclKind::Member),
            0x40 => Some(DeclKind::Function),
            0x41 => Some(DeclKind::Variable),
            0x42 => Some(DeclKind::Parameter),
            0x43 => Some(DeclKind::Local),
            0x44 => Some(DeclKind::TypeParameter),
            0x45 => Some(DeclKind::Lambda),
            0x46 => Some(DeclKind::Label),
            0x47 => Some(DeclKind::Macro),
            0x48 => Some(DeclKind::External),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            DeclKind::Container(k) => k.as_str(),
            DeclKind::Type(k) => k.as_str(),
            DeclKind::Member(k) => k.as_str(),
            DeclKind::Function => "function",
            DeclKind::Variable => "variable",
            DeclKind::Parameter => "parameter",
            DeclKind::Local => "local",
            DeclKind::TypeParameter => "type parameter",
            DeclKind::Lambda => "lambda",
            DeclKind::Label => "label",
            DeclKind::Macro => "macro",
            DeclKind::External => "external",
        }
    }

    pub fn is_container(self) -> bool {
        matches!(self, DeclKind::Container(_))
    }

    pub fn is_type(self) -> bool {
        matches!(self, DeclKind::Type(_))
    }

    pub fn is_member(self) -> bool {
        matches!(self, DeclKind::Member(_))
    }

    /// Declarations with an executable body that owns a CFG.
    pub fn is_callable(self) -> bool {
        match self {
            DeclKind::Member(m) => m.is_callable(),
            DeclKind::Function | DeclKind::Lambda => true,
            _ => false,
        }
    }

    /// Declarations that name a storage location (SSA definition sites).
    pub fn is_variable(self) -> bool {
        matches!(
            self,
            DeclKind::Variable
                | DeclKind::Parameter
                | DeclKind::Local
                | DeclKind::Member(MemberKind::Field)
                | DeclKind::Member(MemberKind::Property)
                | DeclKind::Member(MemberKind::Event)
        )
    }

    /// Declarations that carry an access level.
    pub fn has_visibility(self) -> bool {
        match self {
            DeclKind::Type(k) => k != TypeKind::AnonymousClass,
            DeclKind::Member(m) => !matches!(
                m,
                MemberKind::StaticInitializer | MemberKind::InstanceInitializer
            ),
            DeclKind::Function | DeclKind::Variable => true,
            _ => false,
        }
    }

    /// The declaration kind a reduced AST node stands for, or `None` when the node is not
    /// a declaration (`NN_TEMPLATE_DECL` wraps one and `NN_VAR_DECLARATOR` is part of one).
    pub fn from_ast(t: ASTNodeType) -> Option<Self> {
        use ASTNodeType as A;
        use ContainerKind as C;
        use MemberKind as M;
        use TypeKind as T;
        Some(match t {
            A::NN_MODULE => DeclKind::Container(C::FileScope),
            A::NN_PACKAGE_DECL => DeclKind::Container(C::Package),
            A::NN_NAMESPACE_DECL => DeclKind::Container(C::Namespace),
            A::NN_JAVA_MODULE_DECL => DeclKind::Container(C::Module),
            A::NN_CLASS_DECL => DeclKind::Type(T::Class),
            A::NN_STRUCT_DECL => DeclKind::Type(T::Struct),
            A::NN_INTERFACE_DECL => DeclKind::Type(T::Interface),
            A::NN_ENUM_DECL => DeclKind::Type(T::Enum),
            A::NN_RECORD_DECL => DeclKind::Type(T::Record),
            A::NN_UNION_DECL => DeclKind::Type(T::Union),
            A::NN_TYPEDEF_DECL => DeclKind::Type(T::TypeAlias),
            A::NN_DELEGATE_DECL => DeclKind::Type(T::Delegate),
            A::NN_ANNOTATION_DECL => DeclKind::Type(T::Annotation),
            A::NN_ANON_CLASS_BODY => DeclKind::Type(T::AnonymousClass),
            A::NN_FIELD_DECL => DeclKind::Member(M::Field),
            A::NN_PROPERTY_DECL => DeclKind::Member(M::Property),
            A::NN_INDEXER_DECL => DeclKind::Member(M::Indexer),
            A::NN_EVENT_DECL => DeclKind::Member(M::Event),
            A::NN_METHOD_DECL => DeclKind::Member(M::Method),
            A::NN_CONSTRUCTOR_DECL => DeclKind::Member(M::Constructor),
            A::NN_DESTRUCTOR_DECL => DeclKind::Member(M::Destructor),
            A::NN_OPERATOR_DECL => DeclKind::Member(M::Operator),
            A::NN_JAVA_STATIC_INIT => DeclKind::Member(M::StaticInitializer),
            A::NN_JAVA_INSTANCE_INIT => DeclKind::Member(M::InstanceInitializer),
            A::NN_ENUM_CONSTANT => DeclKind::Member(M::EnumConstant),
            A::NN_FUNCTION_DECL => DeclKind::Function,
            A::NN_PARAM_DECL => DeclKind::Parameter,
            A::NN_LOCAL_VAR_DECL => DeclKind::Local,
            A::NN_TYPE_PARAM => DeclKind::TypeParameter,
            A::NN_LAMBDA_EXPR => DeclKind::Lambda,
            A::NN_LABELED_STMT | A::NN_JAVA_LABELED_STMT => DeclKind::Label,
            _ => return None,
        })
    }

    /// [`DeclKind::from_ast`] refined by the kind of the enclosing declaration: a free
    /// function nested in a type body is a method, and a field or property directly inside
    /// a container is a free variable.
    ///
    /// A frontend that keeps out-of-class member definitions at namespace scope (C++
    /// `int A::n = 0;`, `void A::f() {}`) must bind them to their class (Phase 3 qualified-
    /// name binding) before asking for the refined kind.
    pub fn from_ast_in(t: ASTNodeType, parent: Option<DeclKind>) -> Option<Self> {
        let kind = Self::from_ast(t)?;
        Some(match (kind, parent) {
            (DeclKind::Function, Some(DeclKind::Type(_))) => DeclKind::Member(MemberKind::Method),
            (
                DeclKind::Member(MemberKind::Field | MemberKind::Property),
                Some(DeclKind::Container(_)),
            ) => DeclKind::Variable,
            _ => kind,
        })
    }

    /// The AST node a frontend emits for a declaration of this kind, when the shared
    /// alphabet has one.
    pub fn to_ast(self) -> Option<ASTNodeType> {
        use ASTNodeType as A;
        Some(match self {
            DeclKind::Container(ContainerKind::FileScope) => A::NN_MODULE,
            DeclKind::Container(ContainerKind::Package) => A::NN_PACKAGE_DECL,
            DeclKind::Container(ContainerKind::Namespace) => A::NN_NAMESPACE_DECL,
            DeclKind::Container(ContainerKind::Module) => return None,
            DeclKind::Type(k) => match k {
                TypeKind::Class => A::NN_CLASS_DECL,
                TypeKind::Struct => A::NN_STRUCT_DECL,
                TypeKind::Interface => A::NN_INTERFACE_DECL,
                TypeKind::Enum => A::NN_ENUM_DECL,
                TypeKind::Record => A::NN_RECORD_DECL,
                TypeKind::Union => A::NN_UNION_DECL,
                TypeKind::TypeAlias => A::NN_TYPEDEF_DECL,
                TypeKind::Delegate => A::NN_DELEGATE_DECL,
                TypeKind::Annotation => A::NN_ANNOTATION_DECL,
                TypeKind::AnonymousClass => A::NN_ANON_CLASS_BODY,
                TypeKind::Object | TypeKind::Mixin | TypeKind::Extension => return None,
            },
            DeclKind::Member(m) => match m {
                MemberKind::Field => A::NN_FIELD_DECL,
                MemberKind::Property => A::NN_PROPERTY_DECL,
                MemberKind::Indexer => A::NN_INDEXER_DECL,
                MemberKind::Event => A::NN_EVENT_DECL,
                MemberKind::Method => A::NN_METHOD_DECL,
                MemberKind::Constructor => A::NN_CONSTRUCTOR_DECL,
                MemberKind::Destructor => A::NN_DESTRUCTOR_DECL,
                MemberKind::Operator | MemberKind::Conversion => A::NN_OPERATOR_DECL,
                MemberKind::StaticInitializer => A::NN_JAVA_STATIC_INIT,
                MemberKind::InstanceInitializer => A::NN_JAVA_INSTANCE_INIT,
                MemberKind::EnumConstant => A::NN_ENUM_CONSTANT,
                MemberKind::Accessor | MemberKind::NestedType => return None,
            },
            DeclKind::Function => A::NN_FUNCTION_DECL,
            DeclKind::Variable => A::NN_FIELD_DECL,
            DeclKind::Parameter => A::NN_PARAM_DECL,
            DeclKind::Local => A::NN_LOCAL_VAR_DECL,
            DeclKind::TypeParameter => A::NN_TYPE_PARAM,
            DeclKind::Lambda => A::NN_LAMBDA_EXPR,
            DeclKind::Label => A::NN_LABELED_STMT,
            DeclKind::Macro | DeclKind::External => return None,
        })
    }

    /// The neutral kind of a stored symbol. `SK_MODULE` is the compilation unit of a file.
    pub fn from_symbol_kind(kind: SymbolKind) -> Self {
        use SymbolKind as S;
        match kind {
            S::SK_PACKAGE => DeclKind::Container(ContainerKind::Package),
            S::SK_MODULE => DeclKind::Container(ContainerKind::FileScope),
            S::SK_CLASS => DeclKind::Type(TypeKind::Class),
            S::SK_INTERFACE => DeclKind::Type(TypeKind::Interface),
            S::SK_ENUM => DeclKind::Type(TypeKind::Enum),
            S::SK_RECORD => DeclKind::Type(TypeKind::Record),
            S::SK_ANNOTATION_TYPE => DeclKind::Type(TypeKind::Annotation),
            S::SK_ANON_CLASS => DeclKind::Type(TypeKind::AnonymousClass),
            S::SK_METHOD => DeclKind::Member(MemberKind::Method),
            S::SK_CONSTRUCTOR => DeclKind::Member(MemberKind::Constructor),
            S::SK_FIELD => DeclKind::Member(MemberKind::Field),
            S::SK_ENUM_CONSTANT => DeclKind::Member(MemberKind::EnumConstant),
            S::SK_STATIC_INIT => DeclKind::Member(MemberKind::StaticInitializer),
            S::SK_INSTANCE_INIT => DeclKind::Member(MemberKind::InstanceInitializer),
            S::SK_PARAM => DeclKind::Parameter,
            S::SK_LOCAL_VAR => DeclKind::Local,
            S::SK_TYPE_PARAM => DeclKind::TypeParameter,
            S::SK_LAMBDA => DeclKind::Lambda,
            S::SK_EXTERNAL => DeclKind::External,
        }
    }

    /// Closest stored symbol kind. `None` for kinds the symbol table cannot hold yet
    /// (labels, macros); [`DeclKind::symbol_kind_is_exact`] reports whether the mapping is
    /// lossless.
    pub fn symbol_kind(self) -> Option<SymbolKind> {
        use SymbolKind as S;
        Some(match self {
            DeclKind::Container(ContainerKind::Package)
            | DeclKind::Container(ContainerKind::Namespace) => S::SK_PACKAGE,
            DeclKind::Container(ContainerKind::Module)
            | DeclKind::Container(ContainerKind::FileScope) => S::SK_MODULE,
            DeclKind::Type(k) => match k {
                TypeKind::Class
                | TypeKind::Struct
                | TypeKind::Union
                | TypeKind::Object
                | TypeKind::TypeAlias
                | TypeKind::Extension => S::SK_CLASS,
                TypeKind::Interface | TypeKind::Mixin | TypeKind::Delegate => S::SK_INTERFACE,
                TypeKind::Enum => S::SK_ENUM,
                TypeKind::Record => S::SK_RECORD,
                TypeKind::Annotation => S::SK_ANNOTATION_TYPE,
                TypeKind::AnonymousClass => S::SK_ANON_CLASS,
            },
            DeclKind::Member(m) => match m {
                MemberKind::Field | MemberKind::Property | MemberKind::Event => S::SK_FIELD,
                MemberKind::Method
                | MemberKind::Indexer
                | MemberKind::Destructor
                | MemberKind::Operator
                | MemberKind::Conversion
                | MemberKind::Accessor => S::SK_METHOD,
                MemberKind::Constructor => S::SK_CONSTRUCTOR,
                MemberKind::StaticInitializer => S::SK_STATIC_INIT,
                MemberKind::InstanceInitializer => S::SK_INSTANCE_INIT,
                MemberKind::EnumConstant => S::SK_ENUM_CONSTANT,
                MemberKind::NestedType => S::SK_CLASS,
            },
            DeclKind::Function => S::SK_METHOD,
            DeclKind::Variable => S::SK_FIELD,
            DeclKind::Parameter => S::SK_PARAM,
            DeclKind::Local => S::SK_LOCAL_VAR,
            DeclKind::TypeParameter => S::SK_TYPE_PARAM,
            DeclKind::Lambda => S::SK_LAMBDA,
            DeclKind::External => S::SK_EXTERNAL,
            DeclKind::Label | DeclKind::Macro => return None,
        })
    }

    /// True when [`DeclKind::symbol_kind`] round-trips through [`DeclKind::from_symbol_kind`].
    pub fn symbol_kind_is_exact(self) -> bool {
        self.symbol_kind()
            .is_some_and(|k| DeclKind::from_symbol_kind(k) == self)
    }
}

// ── Statements, clauses, expressions, directives ─────────────────────────────

/// Meaning of a `yield` statement in a language.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum YieldSemantics {
    /// The language has no `yield` statement.
    None,
    /// Java: the value of a switch-expression arm (a jump out of the switch).
    SwitchValue,
    /// C#, Python, JavaScript: a generator suspension point.
    Generator,
}

/// Exceptional control flow of a language.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExceptionModel {
    /// No exceptions (C, Go): calls never transfer control to a handler.
    None,
    /// Exceptions without declared throws (C++, C#, Kotlin, Python).
    Unchecked,
    /// Exceptions with declared, compiler-checked throws (Java).
    Checked,
}

/// The language rules the CFG builders depend on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ControlFlowRules {
    /// A switch case without a jump continues into the next case (C, C++, Java colon form).
    pub switch_fall_through: bool,
    pub yield_semantics: YieldSemantics,
    pub has_goto: bool,
    pub exceptions: ExceptionModel,
}

impl ControlFlowRules {
    /// C-like default: fall-through switches, no `yield`, no `goto`, unchecked exceptions.
    pub const C_LIKE: ControlFlowRules = ControlFlowRules {
        switch_fall_through: true,
        yield_semantics: YieldSemantics::None,
        has_goto: false,
        exceptions: ExceptionModel::Unchecked,
    };
}

/// Proposed `NodeAttr` aux flag for `NN_YIELD_STMT` in generator languages: `yield break`
/// (the generator ends). Mirrors the `AUX_*` convention of `NodeAttr` (bits 8..11).
pub const AUX_YIELD_GENERATOR_END: u8 = 1 << 0;

/// What a `yield` statement does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum YieldKind {
    /// Java: completes the enclosing switch expression with a value.
    SwitchValue,
    /// `yield return x` / `yield x`: produces an element and suspends.
    GeneratorValue,
    /// `yield break`: ends the generator.
    GeneratorEnd,
}

/// Construct that acquires a resource for the duration of a block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResourceKind {
    /// C# `using`, Python `with`, Java try-with-resources: dispose on exit.
    Dispose,
    /// C# `lock`, Java `synchronized`: monitor held for the block.
    Lock,
    /// Go / Swift `defer`: run a statement when the scope exits.
    Defer,
}

/// Control-flow category of a statement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StatementCategory {
    Block,
    Declaration,
    Expression,
    /// Two-way branch (`if` / `else`).
    Branch,
    /// Multi-way branch (`switch`, `when`, `match`).
    MultiBranch {
        /// Cases without a jump continue into the next case.
        fall_through: bool,
    },
    /// Condition tested before each iteration (`while`, `for(;;)`).
    PreTestLoop,
    /// Condition tested after each iteration (`do ... while`, `repeat ... until`).
    PostTestLoop,
    /// Iteration over the elements of a sequence (`for (x : xs)`, `foreach`, `for in`).
    IteratorLoop,
    Break,
    Continue,
    Return,
    Goto,
    /// Labeled statement (declares a `goto` / `break` target).
    Label,
    Yield(YieldKind),
    Throw,
    /// `try` with `catch` handlers and / or a `finally` block.
    GuardedRegion,
    /// A block guarded by a resource (`using`, `with`, `lock`, `synchronized`, `defer`).
    ScopedResource(ResourceKind),
    /// A statement that suspends until an awaited value is available (`await x;`).
    Await,
    /// `assert` statement: a branch to an exceptional exit.
    Assert,
    /// `;`
    Empty,
}

impl StatementCategory {
    /// The statement category of a reduced AST node. `NN_SWITCH_EXPR` is a multi-way
    /// branch in statement position. `None` for nodes that are not statements of a language
    /// with the given rules.
    pub fn from_ast(t: ASTNodeType, attr: u32, rules: &ControlFlowRules) -> Option<Self> {
        use ASTNodeType as A;
        let aux = NodeAttr::unpack_aux_flags(attr);
        Some(match t {
            A::NN_BLOCK => StatementCategory::Block,
            A::NN_LOCAL_VAR_DECL => StatementCategory::Declaration,
            A::NN_EXPR_STMT => StatementCategory::Expression,
            A::NN_IF_STMT => StatementCategory::Branch,
            A::NN_SWITCH_STMT | A::NN_SWITCH_EXPR => StatementCategory::MultiBranch {
                fall_through: rules.switch_fall_through
                    && aux & NodeAttr::AUX_SWITCH_ARROW_FORM == 0,
            },
            A::NN_FOR_STMT | A::NN_WHILE_STMT => StatementCategory::PreTestLoop,
            A::NN_DO_WHILE_STMT => StatementCategory::PostTestLoop,
            A::NN_ENHANCED_FOR | A::NN_FOREACH_STMT => StatementCategory::IteratorLoop,
            A::NN_BREAK_STMT => StatementCategory::Break,
            A::NN_CONTINUE_STMT => StatementCategory::Continue,
            A::NN_RETURN_STMT => StatementCategory::Return,
            A::NN_GOTO_STMT => StatementCategory::Goto,
            A::NN_LABELED_STMT | A::NN_JAVA_LABELED_STMT => StatementCategory::Label,
            A::NN_YIELD_STMT => match rules.yield_semantics {
                YieldSemantics::SwitchValue => StatementCategory::Yield(YieldKind::SwitchValue),
                YieldSemantics::Generator if aux & AUX_YIELD_GENERATOR_END != 0 => {
                    StatementCategory::Yield(YieldKind::GeneratorEnd)
                }
                YieldSemantics::Generator => StatementCategory::Yield(YieldKind::GeneratorValue),
                YieldSemantics::None => return None,
            },
            A::NN_THROW_STMT => StatementCategory::Throw,
            A::NN_TRY_STMT => StatementCategory::GuardedRegion,
            A::NN_USING_STMT => StatementCategory::ScopedResource(ResourceKind::Dispose),
            A::NN_LOCK_STMT | A::NN_JAVA_SYNCHRONIZED => {
                StatementCategory::ScopedResource(ResourceKind::Lock)
            }
            A::NN_JAVA_ASSERT_STMT => StatementCategory::Assert,
            _ => return None,
        })
    }

    /// The AST node a frontend emits for the category, when one exists.
    pub fn canonical_ast(self) -> Option<ASTNodeType> {
        use ASTNodeType as A;
        Some(match self {
            StatementCategory::Block => A::NN_BLOCK,
            StatementCategory::Declaration => A::NN_LOCAL_VAR_DECL,
            StatementCategory::Expression => A::NN_EXPR_STMT,
            StatementCategory::Branch => A::NN_IF_STMT,
            StatementCategory::MultiBranch { .. } => A::NN_SWITCH_STMT,
            StatementCategory::PreTestLoop => A::NN_WHILE_STMT,
            StatementCategory::PostTestLoop => A::NN_DO_WHILE_STMT,
            StatementCategory::IteratorLoop => A::NN_FOREACH_STMT,
            StatementCategory::Break => A::NN_BREAK_STMT,
            StatementCategory::Continue => A::NN_CONTINUE_STMT,
            StatementCategory::Return => A::NN_RETURN_STMT,
            StatementCategory::Goto => A::NN_GOTO_STMT,
            StatementCategory::Label => A::NN_LABELED_STMT,
            StatementCategory::Yield(_) => A::NN_YIELD_STMT,
            StatementCategory::Throw => A::NN_THROW_STMT,
            StatementCategory::GuardedRegion => A::NN_TRY_STMT,
            StatementCategory::ScopedResource(ResourceKind::Dispose) => A::NN_USING_STMT,
            StatementCategory::ScopedResource(ResourceKind::Lock) => A::NN_LOCK_STMT,
            StatementCategory::ScopedResource(ResourceKind::Defer)
            | StatementCategory::Await
            | StatementCategory::Assert
            | StatementCategory::Empty => return None,
        })
    }

    pub fn is_loop(self) -> bool {
        matches!(
            self,
            StatementCategory::PreTestLoop
                | StatementCategory::PostTestLoop
                | StatementCategory::IteratorLoop
        )
    }

    /// Statements that transfer control unconditionally away from the next statement.
    pub fn is_jump(self) -> bool {
        matches!(
            self,
            StatementCategory::Break
                | StatementCategory::Continue
                | StatementCategory::Return
                | StatementCategory::Goto
                | StatementCategory::Throw
                | StatementCategory::Yield(YieldKind::SwitchValue)
                | StatementCategory::Yield(YieldKind::GeneratorEnd)
        )
    }

    /// Statements whose successors depend on a condition.
    pub fn is_conditional(self) -> bool {
        matches!(
            self,
            StatementCategory::Branch
                | StatementCategory::MultiBranch { .. }
                | StatementCategory::Assert
        ) || self.is_loop()
    }

    /// Statements that open a lexical scope for the declarations they contain.
    pub fn opens_scope(self) -> bool {
        matches!(
            self,
            StatementCategory::Block
                | StatementCategory::MultiBranch { .. }
                | StatementCategory::PreTestLoop
                | StatementCategory::IteratorLoop
                | StatementCategory::GuardedRegion
                | StatementCategory::ScopedResource(_)
        )
    }
}

/// Sub-parts of compound statements and declarations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClauseCategory {
    Else,
    /// One case of a multi-way branch (its labels and statements).
    Case,
    Catch,
    Finally,
    /// The resource list of a scoped-resource `try`.
    ResourceSpec,
    /// Declared exceptions (`throws`).
    Throws,
    /// Permitted subtypes of a sealed hierarchy.
    Permits,
}

impl ClauseCategory {
    pub fn from_ast(t: ASTNodeType) -> Option<Self> {
        use ASTNodeType as A;
        Some(match t {
            A::NN_ELSE_CLAUSE => ClauseCategory::Else,
            A::NN_SWITCH_CASE => ClauseCategory::Case,
            A::NN_CATCH_CLAUSE => ClauseCategory::Catch,
            A::NN_FINALLY_CLAUSE => ClauseCategory::Finally,
            A::NN_RESOURCE_SPEC => ClauseCategory::ResourceSpec,
            A::NN_THROWS_CLAUSE => ClauseCategory::Throws,
            A::NN_PERMITS_CLAUSE => ClauseCategory::Permits,
            _ => return None,
        })
    }
}

/// Semantic category of an expression.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExprCategory {
    /// A name use (variable, field, type or function name).
    Name,
    Literal,
    Call,
    /// Object creation (`new T(...)`, Kotlin `T(...)`, C++ `new T`).
    ConstructorCall,
    /// `a.b`, `a->b`, `a?.b`.
    MemberAccess,
    /// `a[i]`.
    Index,
    Assignment,
    /// Unary or binary operator application; the operator is in the `NodeAttr` word.
    Operator,
    /// `c ? a : b`, `if` expression.
    Conditional,
    /// Switch / `when` / `match` expression.
    Switch,
    Cast,
    /// `instanceof`, `is`, `dynamic_cast` test.
    TypeTest,
    /// Pattern with bindings (`case Point(var x, var y)`, `is Foo f`).
    PatternMatch,
    /// Lambda, closure, anonymous function.
    Lambda,
    /// `Foo::bar`, C# method group, C `&fn`.
    MethodReference,
    /// `new T[n]`.
    ArrayCreation,
    /// `{1, 2, 3}`, `[a, b]`, initializer list.
    CollectionLiteral,
    /// `this`, `self`.
    SelfRef,
    /// `super`, `base`.
    SuperRef,
    /// `await e`, `co_await e`.
    Await,
}

impl ExprCategory {
    pub fn from_ast(t: ASTNodeType) -> Option<Self> {
        use ASTNodeType as A;
        Some(match t {
            A::NN_IDENTIFIER_EXPR => ExprCategory::Name,
            A::NN_LITERAL => ExprCategory::Literal,
            A::NN_CALL_EXPR => ExprCategory::Call,
            A::NN_NEW_EXPR => ExprCategory::ConstructorCall,
            A::NN_FIELD_ACCESS => ExprCategory::MemberAccess,
            A::NN_ARRAY_ACCESS => ExprCategory::Index,
            A::NN_ASSIGN_EXPR => ExprCategory::Assignment,
            A::NN_BINARY_EXPR | A::NN_UNARY_EXPR => ExprCategory::Operator,
            A::NN_TERNARY_EXPR => ExprCategory::Conditional,
            A::NN_SWITCH_EXPR => ExprCategory::Switch,
            A::NN_CAST_EXPR => ExprCategory::Cast,
            A::NN_INSTANCEOF_EXPR => ExprCategory::TypeTest,
            A::NN_PATTERN_MATCH => ExprCategory::PatternMatch,
            A::NN_LAMBDA_EXPR => ExprCategory::Lambda,
            A::NN_METHOD_REF => ExprCategory::MethodReference,
            A::NN_ARRAY_CREATE => ExprCategory::ArrayCreation,
            A::NN_ARRAY_INIT => ExprCategory::CollectionLiteral,
            A::NN_THIS_EXPR => ExprCategory::SelfRef,
            A::NN_SUPER_EXPR => ExprCategory::SuperRef,
            _ => return None,
        })
    }

    pub fn canonical_ast(self) -> Option<ASTNodeType> {
        use ASTNodeType as A;
        Some(match self {
            ExprCategory::Name => A::NN_IDENTIFIER_EXPR,
            ExprCategory::Literal => A::NN_LITERAL,
            ExprCategory::Call => A::NN_CALL_EXPR,
            ExprCategory::ConstructorCall => A::NN_NEW_EXPR,
            ExprCategory::MemberAccess => A::NN_FIELD_ACCESS,
            ExprCategory::Index => A::NN_ARRAY_ACCESS,
            ExprCategory::Assignment => A::NN_ASSIGN_EXPR,
            ExprCategory::Operator => A::NN_BINARY_EXPR,
            ExprCategory::Conditional => A::NN_TERNARY_EXPR,
            ExprCategory::Switch => A::NN_SWITCH_EXPR,
            ExprCategory::Cast => A::NN_CAST_EXPR,
            ExprCategory::TypeTest => A::NN_INSTANCEOF_EXPR,
            ExprCategory::PatternMatch => A::NN_PATTERN_MATCH,
            ExprCategory::Lambda => A::NN_LAMBDA_EXPR,
            ExprCategory::MethodReference => A::NN_METHOD_REF,
            ExprCategory::ArrayCreation => A::NN_ARRAY_CREATE,
            ExprCategory::CollectionLiteral => A::NN_ARRAY_INIT,
            ExprCategory::SelfRef => A::NN_THIS_EXPR,
            ExprCategory::SuperRef => A::NN_SUPER_EXPR,
            ExprCategory::Await => return None,
        })
    }

    /// Invocations that transfer control to a callee (call-graph call sites).
    pub fn is_call_like(self) -> bool {
        matches!(self, ExprCategory::Call | ExprCategory::ConstructorCall)
    }

    /// Expressions whose value is a function (dynamic call-graph edges).
    pub fn is_function_value(self) -> bool {
        matches!(self, ExprCategory::Lambda | ExprCategory::MethodReference)
    }

    /// Expressions that allocate a new object (points-to allocation sites).
    pub fn is_allocation(self) -> bool {
        matches!(
            self,
            ExprCategory::ConstructorCall
                | ExprCategory::ArrayCreation
                | ExprCategory::CollectionLiteral
        )
    }
}

/// File- or unit-level directives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DirectiveKind {
    /// `import`, C# / C++ `using` directive.
    Import,
    /// `#include`.
    Include,
    /// Java module directive (`requires`, `exports`, ...).
    ModuleDirective,
}

impl DirectiveKind {
    pub fn from_ast(t: ASTNodeType) -> Option<Self> {
        use ASTNodeType as A;
        Some(match t {
            A::NN_IMPORT_DECL | A::NN_USING_DIRECTIVE => DirectiveKind::Import,
            A::NN_INCLUDE_DIRECTIVE => DirectiveKind::Include,
            A::NN_JAVA_MODULE_DIRECTIVE => DirectiveKind::ModuleDirective,
            _ => return None,
        })
    }
}

/// The single neutral role of a reduced AST node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NodeCategory {
    Declaration(DeclKind),
    Statement(StatementCategory),
    Clause(ClauseCategory),
    Expression(ExprCategory),
    Directive(DirectiveKind),
    /// A written type (`NN_TYPE_REF`).
    TypeReference,
    /// An annotation / attribute use.
    Attribute,
    /// A generic / template declaration wrapper: generic parameters and the wrapped declaration.
    GenericWrapper,
    /// One declarator of a multi-variable declaration.
    Declarator,
    Synthetic,
    Unknown,
}

impl NodeCategory {
    /// Classifies a reduced AST node. Lambdas are expressions and labeled statements are
    /// statements here (both also declare a symbol: see [`DeclKind::from_ast`]); a local
    /// variable declaration is a declaration (and a `Declaration` statement).
    pub fn of(t: ASTNodeType, attr: u32, rules: &ControlFlowRules) -> Self {
        use ASTNodeType as A;
        match t {
            A::NN_LAMBDA_EXPR => return NodeCategory::Expression(ExprCategory::Lambda),
            A::NN_LABELED_STMT | A::NN_JAVA_LABELED_STMT => {
                return NodeCategory::Statement(StatementCategory::Label)
            }
            A::NN_TYPE_REF => return NodeCategory::TypeReference,
            A::NN_ANNOTATION_USE => return NodeCategory::Attribute,
            A::NN_TEMPLATE_DECL => return NodeCategory::GenericWrapper,
            A::NN_VAR_DECLARATOR => return NodeCategory::Declarator,
            A::NN_SYNTHETIC => return NodeCategory::Synthetic,
            A::NN_UNKNOWN => return NodeCategory::Unknown,
            _ => {}
        }
        if let Some(kind) = DeclKind::from_ast(t) {
            return NodeCategory::Declaration(kind);
        }
        if let Some(stmt) = StatementCategory::from_ast(t, attr, rules) {
            if t != A::NN_SWITCH_EXPR {
                return NodeCategory::Statement(stmt);
            }
        }
        if let Some(expr) = ExprCategory::from_ast(t) {
            return NodeCategory::Expression(expr);
        }
        if let Some(clause) = ClauseCategory::from_ast(t) {
            return NodeCategory::Clause(clause);
        }
        if let Some(directive) = DirectiveKind::from_ast(t) {
            return NodeCategory::Directive(directive);
        }
        NodeCategory::Unknown
    }
}

// ── Neutral structural predicates (replace per-language node-kind lists) ──────

/// Type declarations, excluding anonymous class bodies (they belong to their creation
/// expression).
pub fn is_type_declaration(t: ASTNodeType) -> bool {
    matches!(DeclKind::from_ast(t), Some(DeclKind::Type(k)) if k != TypeKind::AnonymousClass)
}

/// Declarations with an executable body that owns a CFG (methods, constructors,
/// destructors, operators, initializer blocks, free functions, lambdas).
pub fn is_callable_declaration(t: ASTNodeType) -> bool {
    DeclKind::from_ast(t).is_some_and(DeclKind::is_callable)
}

/// Nodes whose subtrees are separate functions or types and therefore never part of the
/// statements of the function that lexically contains them.
pub fn is_nested_body(t: ASTNodeType) -> bool {
    is_type_declaration(t)
        || is_callable_declaration(t)
        || matches!(t, ASTNodeType::NN_TEMPLATE_DECL | ASTNodeType::NN_NAMESPACE_DECL)
        || matches!(
            DeclKind::from_ast(t),
            Some(DeclKind::Member(
                MemberKind::Field | MemberKind::Property | MemberKind::Indexer | MemberKind::Event
            ))
        )
}

/// Statement kinds (as opposed to expressions), including local type declarations.
pub fn is_statement_node(t: ASTNodeType, rules: &ControlFlowRules) -> bool {
    StatementCategory::from_ast(t, 0, rules).is_some() || is_type_declaration(t)
}

/// Declarations of storage locations (SSA definition sites).
pub fn is_variable_declaration(t: ASTNodeType) -> bool {
    t == ASTNodeType::NN_VAR_DECLARATOR || DeclKind::from_ast(t).is_some_and(DeclKind::is_variable)
}

/// Package, namespace, module and compilation-unit nodes.
pub fn is_container_node(t: ASTNodeType) -> bool {
    matches!(DeclKind::from_ast(t), Some(DeclKind::Container(_)))
}

// ── Visibility ───────────────────────────────────────────────────────────────

/// Proposed `NodeAttr` visibility nibble codes beyond `VISIBILITY_PACKAGE_PRIVATE`.
pub const ATTR_VISIBILITY_PROTECTED_INTERNAL: u8 = 0x05;
pub const ATTR_VISIBILITY_PRIVATE_PROTECTED: u8 = 0x06;
pub const ATTR_VISIBILITY_FILE_PRIVATE: u8 = 0x07;
pub const ATTR_VISIBILITY_UNKNOWN: u8 = 0x0F;

/// Access level, as the superset of every language's access modifiers. The discriminants
/// are the `NodeAttr` visibility nibble codes.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub enum Visibility {
    /// No access level applies (locals, parameters, expressions).
    NotApplicable = 0x00,
    Public = 0x01,
    Private = 0x02,
    Protected = 0x03,
    /// Java package-private, C# / Kotlin / Swift `internal`, Rust `pub(crate)`.
    Package = 0x04,
    /// C# `protected internal`: derived types or the same assembly.
    ProtectedInternal = 0x05,
    /// C# `private protected`: derived types within the same assembly.
    PrivateProtected = 0x06,
    /// Swift `fileprivate`, C / C++ `static` at file scope (internal linkage).
    FilePrivate = 0x07,
    /// The access level was not written and has not been derived from the language rules.
    #[default]
    Unknown = 0x0F,
}

impl Visibility {
    /// Decodes the `NodeAttr` visibility nibble.
    pub fn from_attr_nibble(nibble: u8) -> Self {
        match nibble & 0x0F {
            0x00 => Visibility::NotApplicable,
            0x01 => Visibility::Public,
            0x02 => Visibility::Private,
            0x03 => Visibility::Protected,
            0x04 => Visibility::Package,
            0x05 => Visibility::ProtectedInternal,
            0x06 => Visibility::PrivateProtected,
            0x07 => Visibility::FilePrivate,
            _ => Visibility::Unknown,
        }
    }

    /// `NodeAttr` visibility nibble of the access level.
    pub fn to_attr_nibble(self) -> u8 {
        self as u8
    }

    pub fn from_symbol_visibility(vis: SymbolVisibility) -> Self {
        match vis {
            SymbolVisibility::Public => Visibility::Public,
            SymbolVisibility::Private => Visibility::Private,
            SymbolVisibility::Protected => Visibility::Protected,
            SymbolVisibility::Package => Visibility::Package,
        }
    }

    /// Closest stored visibility; `None` when no access level is known or applies.
    pub fn to_symbol_visibility(self) -> Option<SymbolVisibility> {
        match self {
            Visibility::Public => Some(SymbolVisibility::Public),
            Visibility::Private | Visibility::FilePrivate => Some(SymbolVisibility::Private),
            Visibility::Protected
            | Visibility::ProtectedInternal
            | Visibility::PrivateProtected => Some(SymbolVisibility::Protected),
            Visibility::Package => Some(SymbolVisibility::Package),
            Visibility::NotApplicable | Visibility::Unknown => None,
        }
    }

    /// The access level named by an access-modifier spelling of any supported language
    /// (`"protected internal"`, `"fileprivate"`, `"pub(crate)"`, ...).
    pub fn from_keyword(spelling: &str) -> Option<Self> {
        let words: Vec<&str> = spelling.split_whitespace().collect();
        Some(match words.as_slice() {
            ["public"] | ["pub"] | ["open"] | ["export"] => Visibility::Public,
            ["private"] => Visibility::Private,
            ["protected"] => Visibility::Protected,
            ["internal"] | ["package"] | ["pub(crate)"] | ["pub(super)"] => Visibility::Package,
            ["protected", "internal"] | ["internal", "protected"] => {
                Visibility::ProtectedInternal
            }
            ["private", "protected"] | ["protected", "private"] => Visibility::PrivateProtected,
            ["fileprivate"] => Visibility::FilePrivate,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Visibility::NotApplicable => "none",
            Visibility::Public => "public",
            Visibility::Private => "private",
            Visibility::Protected => "protected",
            Visibility::Package => "package",
            Visibility::ProtectedInternal => "protected internal",
            Visibility::PrivateProtected => "private protected",
            Visibility::FilePrivate => "fileprivate",
            Visibility::Unknown => "unknown",
        }
    }

    /// UML visibility marker (`+`, `-`, `#`, `~`); empty when no access level is known.
    pub fn uml_symbol(self) -> &'static str {
        match self {
            Visibility::Public => "+",
            Visibility::Private | Visibility::FilePrivate => "-",
            Visibility::Protected
            | Visibility::ProtectedInternal
            | Visibility::PrivateProtected => "#",
            Visibility::Package => "~",
            Visibility::NotApplicable | Visibility::Unknown => "",
        }
    }

    /// Visible outside the declaring module / assembly (part of the published API).
    pub fn is_api(self) -> bool {
        matches!(
            self,
            Visibility::Public | Visibility::Protected | Visibility::ProtectedInternal
        )
    }

    /// Accessible from subtypes of the declaring type.
    pub fn is_inheritable(self) -> bool {
        matches!(
            self,
            Visibility::Public
                | Visibility::Protected
                | Visibility::ProtectedInternal
                | Visibility::PrivateProtected
        )
    }

    /// Accessible only inside the declaring type or file.
    pub fn is_private(self) -> bool {
        matches!(self, Visibility::Private | Visibility::FilePrivate)
    }
}

impl fmt::Display for Visibility {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

// ── Modifiers ────────────────────────────────────────────────────────────────

/// Declaration modifiers, as the superset of every language's modifier keywords.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default, PartialOrd, Ord)]
pub struct Modifiers(u64);

impl Modifiers {
    pub const EMPTY: Modifiers = Modifiers(0);
    pub const STATIC: Modifiers = Modifiers(1 << 0);
    pub const ABSTRACT: Modifiers = Modifiers(1 << 1);
    /// Cannot be overridden or subclassed: Java / Kotlin `final`, C# `sealed`, C++ `final`.
    pub const FINAL: Modifiers = Modifiers(1 << 2);
    /// Subclasses restricted to a permitted set: Java / Kotlin `sealed`.
    pub const SEALED_HIERARCHY: Modifiers = Modifiers(1 << 3);
    pub const NON_SEALED: Modifiers = Modifiers(1 << 4);
    /// Overridable: C++ / C# `virtual`, Kotlin / Swift `open`.
    pub const VIRTUAL: Modifiers = Modifiers(1 << 5);
    pub const OVERRIDE: Modifiers = Modifiers(1 << 6);
    /// Immutable binding: C / C++ `const`, C# `readonly`, Kotlin `val`, Java `final` local.
    pub const CONST: Modifiers = Modifiers(1 << 7);
    /// Compile-time constant: C# / Kotlin `const`, C++ `constexpr` / `consteval` / `constinit`.
    pub const COMPILE_TIME_CONSTANT: Modifiers = Modifiers(1 << 8);
    /// C# / JavaScript `async`, Kotlin `suspend`, C++20 coroutine.
    pub const ASYNC: Modifiers = Modifiers(1 << 9);
    pub const INLINE: Modifiers = Modifiers(1 << 10);
    /// Implemented outside the language: Java `native`, C# `extern`, C / C++ `extern`.
    pub const EXTERN: Modifiers = Modifiers(1 << 11);
    /// C# `partial`.
    pub const PARTIAL: Modifiers = Modifiers(1 << 12);
    /// Java `default` interface method.
    pub const DEFAULT: Modifiers = Modifiers(1 << 13);
    pub const SYNCHRONIZED: Modifiers = Modifiers(1 << 14);
    pub const VOLATILE: Modifiers = Modifiers(1 << 15);
    pub const TRANSIENT: Modifiers = Modifiers(1 << 16);
    pub const STRICTFP: Modifiers = Modifiers(1 << 17);
    /// Variable-arity method or parameter (`T...`, `params`, `vararg`).
    pub const VARARGS: Modifiers = Modifiers(1 << 18);
    pub const DEPRECATED: Modifiers = Modifiers(1 << 19);
    /// Generated by the compiler (bridges, accessors, implicit members).
    pub const SYNTHETIC: Modifiers = Modifiers(1 << 20);
    /// At least one modifier was implied by the language rules rather than written.
    pub const IMPLICIT: Modifiers = Modifiers(1 << 21);
    /// C# `new` member hiding.
    pub const NEW: Modifiers = Modifiers(1 << 22);
    pub const UNSAFE: Modifiers = Modifiers(1 << 23);
    /// C++ `mutable`.
    pub const MUTABLE: Modifiers = Modifiers(1 << 24);
    /// C++ `explicit` constructor / conversion, C# `explicit operator`.
    pub const EXPLICIT: Modifiers = Modifiers(1 << 25);
    pub const THREAD_LOCAL: Modifiers = Modifiers(1 << 26);
    pub const NOEXCEPT: Modifiers = Modifiers(1 << 27);
    /// C++ `= delete`.
    pub const DELETED: Modifiers = Modifiers(1 << 28);
    /// C++ `= default`.
    pub const DEFAULTED: Modifiers = Modifiers(1 << 29);
    /// C++ `friend`.
    pub const FRIEND: Modifiers = Modifiers(1 << 30);
    /// By-reference parameter or `ref struct` (C#).
    pub const REF: Modifiers = Modifiers(1 << 31);
    /// C# `out` parameter.
    pub const OUT: Modifiers = Modifiers(1 << 32);
    /// C# `in` (read-only by-reference) parameter.
    pub const IN: Modifiers = Modifiers(1 << 33);
    /// Kotlin `operator fun`.
    pub const OPERATOR: Modifiers = Modifiers(1 << 34);
    pub const INFIX: Modifiers = Modifiers(1 << 35);
    pub const TAILREC: Modifiers = Modifiers(1 << 36);
    pub const LATEINIT: Modifiers = Modifiers(1 << 37);
    /// Kotlin `companion object`.
    pub const COMPANION: Modifiers = Modifiers(1 << 38);
    pub const EXPECT: Modifiers = Modifiers(1 << 39);
    pub const ACTUAL: Modifiers = Modifiers(1 << 40);
    /// C# `required` member.
    pub const REQUIRED: Modifiers = Modifiers(1 << 41);
    /// Declared by a primary constructor: Java record component, Kotlin `val` / `var`
    /// constructor parameter, C# positional record / primary-constructor parameter.
    pub const PRIMARY_CONSTRUCTOR_MEMBER: Modifiers = Modifiers(1 << 42);

    const KNOWN_BITS: u64 = (1 << 43) - 1;

    const NAMES: [(Modifiers, &'static str); 43] = [
        (Modifiers::STATIC, "static"),
        (Modifiers::ABSTRACT, "abstract"),
        (Modifiers::FINAL, "final"),
        (Modifiers::SEALED_HIERARCHY, "sealed"),
        (Modifiers::NON_SEALED, "non-sealed"),
        (Modifiers::VIRTUAL, "virtual"),
        (Modifiers::OVERRIDE, "override"),
        (Modifiers::CONST, "const"),
        (Modifiers::COMPILE_TIME_CONSTANT, "constexpr"),
        (Modifiers::ASYNC, "async"),
        (Modifiers::INLINE, "inline"),
        (Modifiers::EXTERN, "extern"),
        (Modifiers::PARTIAL, "partial"),
        (Modifiers::DEFAULT, "default"),
        (Modifiers::SYNCHRONIZED, "synchronized"),
        (Modifiers::VOLATILE, "volatile"),
        (Modifiers::TRANSIENT, "transient"),
        (Modifiers::STRICTFP, "strictfp"),
        (Modifiers::VARARGS, "varargs"),
        (Modifiers::DEPRECATED, "deprecated"),
        (Modifiers::SYNTHETIC, "synthetic"),
        (Modifiers::IMPLICIT, "implicit"),
        (Modifiers::NEW, "new"),
        (Modifiers::UNSAFE, "unsafe"),
        (Modifiers::MUTABLE, "mutable"),
        (Modifiers::EXPLICIT, "explicit"),
        (Modifiers::THREAD_LOCAL, "thread_local"),
        (Modifiers::NOEXCEPT, "noexcept"),
        (Modifiers::DELETED, "deleted"),
        (Modifiers::DEFAULTED, "defaulted"),
        (Modifiers::FRIEND, "friend"),
        (Modifiers::REF, "ref"),
        (Modifiers::OUT, "out"),
        (Modifiers::IN, "in"),
        (Modifiers::OPERATOR, "operator"),
        (Modifiers::INFIX, "infix"),
        (Modifiers::TAILREC, "tailrec"),
        (Modifiers::LATEINIT, "lateinit"),
        (Modifiers::COMPANION, "companion"),
        (Modifiers::EXPECT, "expect"),
        (Modifiers::ACTUAL, "actual"),
        (Modifiers::REQUIRED, "required"),
        (Modifiers::PRIMARY_CONSTRUCTOR_MEMBER, "primary-constructor"),
    ];

    /// `SymbolModifiers` bit ↔ neutral modifier, both directions (`BRIDGE` maps one way).
    const SYMBOL_MAP: [(u16, Modifiers); 15] = [
        (SymbolModifiers::STATIC, Modifiers::STATIC),
        (SymbolModifiers::FINAL, Modifiers::FINAL),
        (SymbolModifiers::ABSTRACT, Modifiers::ABSTRACT),
        (SymbolModifiers::SYNCHRONIZED, Modifiers::SYNCHRONIZED),
        (SymbolModifiers::NATIVE, Modifiers::EXTERN),
        (SymbolModifiers::VOLATILE, Modifiers::VOLATILE),
        (SymbolModifiers::TRANSIENT, Modifiers::TRANSIENT),
        (SymbolModifiers::STRICTFP, Modifiers::STRICTFP),
        (SymbolModifiers::DEFAULT, Modifiers::DEFAULT),
        (SymbolModifiers::SEALED, Modifiers::SEALED_HIERARCHY),
        (SymbolModifiers::NON_SEALED, Modifiers::NON_SEALED),
        (SymbolModifiers::VARARGS, Modifiers::VARARGS),
        (SymbolModifiers::SYNTHETIC, Modifiers::SYNTHETIC),
        (SymbolModifiers::DEPRECATED, Modifiers::DEPRECATED),
        (SymbolModifiers::RECORD_COMPONENT, Modifiers::PRIMARY_CONSTRUCTOR_MEMBER),
    ];

    /// `NodeAttr` modifier-byte bit ↔ neutral modifier.
    const ATTR_MAP: [(u8, Modifiers); 8] = [
        (NodeAttr::MOD_STATIC, Modifiers::STATIC),
        (NodeAttr::MOD_FINAL, Modifiers::FINAL),
        (NodeAttr::MOD_ABSTRACT, Modifiers::ABSTRACT),
        (NodeAttr::MOD_SYNCHRONIZED, Modifiers::SYNCHRONIZED),
        (NodeAttr::MOD_NATIVE, Modifiers::EXTERN),
        (NodeAttr::MOD_VOLATILE, Modifiers::VOLATILE),
        (NodeAttr::MOD_TRANSIENT, Modifiers::TRANSIENT),
        (NodeAttr::MOD_SEALED, Modifiers::SEALED_HIERARCHY),
    ];

    pub const fn bits(self) -> u64 {
        self.0
    }

    /// Builds a set from raw bits; bits that name no modifier are dropped.
    pub const fn from_bits(bits: u64) -> Self {
        Modifiers(bits & Self::KNOWN_BITS)
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub const fn contains(self, other: Modifiers) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn intersects(self, other: Modifiers) -> bool {
        self.0 & other.0 != 0
    }

    pub const fn union(self, other: Modifiers) -> Self {
        Modifiers(self.0 | other.0)
    }

    pub const fn difference(self, other: Modifiers) -> Self {
        Modifiers(self.0 & !other.0)
    }

    pub fn insert(&mut self, other: Modifiers) {
        self.0 |= other.0;
    }

    pub fn remove(&mut self, other: Modifiers) {
        self.0 &= !other.0;
    }

    pub fn set(&mut self, other: Modifiers, on: bool) {
        if on {
            self.insert(other);
        } else {
            self.remove(other);
        }
    }

    /// The modifiers of the set with their display keywords, in bit order.
    pub fn iter(self) -> impl Iterator<Item = (Modifiers, &'static str)> {
        Self::NAMES
            .into_iter()
            .filter(move |(flag, _)| self.contains(*flag))
    }

    /// The modifier a keyword denotes in every language that has it. Keywords whose
    /// meaning differs between languages return `None` and are mapped by the frontend:
    /// `sealed` is `FINAL` in C# but `SEALED_HIERARCHY` in Java / Kotlin, `open` is
    /// `VIRTUAL` on Kotlin members but an access level in Swift. `const` maps to `CONST`;
    /// C# and Kotlin frontends add `COMPILE_TIME_CONSTANT`.
    pub fn from_keyword(word: &str) -> Option<Modifiers> {
        Some(match word {
            "static" => Modifiers::STATIC,
            "abstract" => Modifiers::ABSTRACT,
            "final" => Modifiers::FINAL,
            "non-sealed" => Modifiers::NON_SEALED,
            "virtual" => Modifiers::VIRTUAL,
            "override" => Modifiers::OVERRIDE,
            "const" | "readonly" | "val" => Modifiers::CONST,
            "constexpr" | "consteval" | "constinit" => Modifiers::COMPILE_TIME_CONSTANT,
            "async" | "suspend" => Modifiers::ASYNC,
            "inline" | "__inline" | "__inline__" | "__forceinline" => Modifiers::INLINE,
            "extern" | "native" => Modifiers::EXTERN,
            "partial" => Modifiers::PARTIAL,
            "default" => Modifiers::DEFAULT,
            "synchronized" => Modifiers::SYNCHRONIZED,
            "volatile" => Modifiers::VOLATILE,
            "transient" => Modifiers::TRANSIENT,
            "strictfp" => Modifiers::STRICTFP,
            "params" | "vararg" => Modifiers::VARARGS,
            "new" => Modifiers::NEW,
            "unsafe" => Modifiers::UNSAFE,
            "mutable" => Modifiers::MUTABLE,
            "explicit" => Modifiers::EXPLICIT,
            "thread_local" | "_Thread_local" | "__thread" => Modifiers::THREAD_LOCAL,
            "noexcept" => Modifiers::NOEXCEPT,
            "friend" => Modifiers::FRIEND,
            "ref" => Modifiers::REF,
            "out" => Modifiers::OUT,
            "in" => Modifiers::IN,
            "operator" => Modifiers::OPERATOR,
            "infix" => Modifiers::INFIX,
            "tailrec" => Modifiers::TAILREC,
            "lateinit" => Modifiers::LATEINIT,
            "companion" => Modifiers::COMPANION,
            "expect" => Modifiers::EXPECT,
            "actual" => Modifiers::ACTUAL,
            "required" => Modifiers::REQUIRED,
            _ => return None,
        })
    }

    /// Decodes `SymbolRecord::modifiers`.
    pub fn from_symbol_modifiers(bits: u16) -> Self {
        let mut out = Modifiers::EMPTY;
        for (bit, flag) in Self::SYMBOL_MAP {
            if bits & bit != 0 {
                out.insert(flag);
            }
        }
        if bits & SymbolModifiers::BRIDGE != 0 {
            out.insert(Modifiers::SYNTHETIC);
        }
        out
    }

    /// Encodes the modifiers `SymbolRecord::modifiers` can hold; the others are dropped.
    pub fn to_symbol_modifiers(self) -> u16 {
        Self::SYMBOL_MAP
            .iter()
            .filter(|(_, flag)| self.contains(*flag))
            .fold(0u16, |acc, (bit, _)| acc | bit)
    }

    /// Decodes the `NodeAttr` modifier byte (bits 27..20).
    pub fn from_node_attr(mods: u8) -> Self {
        let mut out = Modifiers::EMPTY;
        for (bit, flag) in Self::ATTR_MAP {
            if mods & bit != 0 {
                out.insert(flag);
            }
        }
        out
    }

    /// Encodes the modifiers the `NodeAttr` modifier byte can hold; the others are dropped
    /// (frontends carry them in their language-flag byte).
    pub fn to_node_attr(self) -> u8 {
        Self::ATTR_MAP
            .iter()
            .filter(|(_, flag)| self.contains(*flag))
            .fold(0u8, |acc, (bit, _)| acc | bit)
    }
}

impl BitOr for Modifiers {
    type Output = Modifiers;
    fn bitor(self, rhs: Modifiers) -> Modifiers {
        self.union(rhs)
    }
}

impl BitOrAssign for Modifiers {
    fn bitor_assign(&mut self, rhs: Modifiers) {
        self.insert(rhs);
    }
}

impl BitAnd for Modifiers {
    type Output = Modifiers;
    fn bitand(self, rhs: Modifiers) -> Modifiers {
        Modifiers(self.0 & rhs.0)
    }
}

impl BitAndAssign for Modifiers {
    fn bitand_assign(&mut self, rhs: Modifiers) {
        self.0 &= rhs.0;
    }
}

impl Sub for Modifiers {
    type Output = Modifiers;
    fn sub(self, rhs: Modifiers) -> Modifiers {
        self.difference(rhs)
    }
}

impl fmt::Debug for Modifiers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Modifiers(")?;
        for (i, (_, name)) in self.iter().enumerate() {
            if i > 0 {
                f.write_str(" | ")?;
            }
            f.write_str(name)?;
        }
        f.write_str(")")
    }
}

impl fmt::Display for Modifiers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, (_, name)) in self.iter().enumerate() {
            if i > 0 {
                f.write_str(" ")?;
            }
            f.write_str(name)?;
        }
        Ok(())
    }
}

// ── Ownership, multiplicity, association ─────────────────────────────────────

/// Who controls the lifetime of the object a member refers to.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Ownership {
    /// Lifetime unknown from the declaration (managed references without further evidence).
    #[default]
    Unknown = 0,
    /// The owner creates and destroys the object (by-value member, `unique_ptr`, `Box`):
    /// UML composition.
    Owning = 1,
    /// Lifetime shared among several holders (`shared_ptr`, `Rc`, `Arc`): UML aggregation.
    Shared = 2,
    /// Non-owning reference whose target outlives the holder (C++ `T&` and observer `T*`,
    /// Rust `&T`): UML association.
    Borrowed = 3,
    /// Non-owning reference that may expire (`weak_ptr`, `WeakReference`, Swift `weak`).
    Weak = 4,
}

/// Facts about how a member is initialised, used to refine an `Unknown` ownership.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct OwnershipEvidence {
    /// Every value stored in the member is created by the owner (constructor / initializer
    /// `new`, member initializer).
    pub created_by_owner: bool,
    /// The binding cannot be reassigned after construction (`final`, `readonly`, `val`).
    pub immutable_binding: bool,
    /// The member is assigned from a constructor or setter parameter.
    pub assigned_from_parameter: bool,
    /// The stored reference is handed out by the owner (returned or passed on).
    pub escapes: bool,
}

impl Ownership {
    pub fn code(self) -> u8 {
        self as u8
    }

    pub fn from_code(code: u8) -> Option<Self> {
        Some(match code {
            0 => Ownership::Unknown,
            1 => Ownership::Owning,
            2 => Ownership::Shared,
            3 => Ownership::Borrowed,
            4 => Ownership::Weak,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Ownership::Unknown => "unknown",
            Ownership::Owning => "owning",
            Ownership::Shared => "shared",
            Ownership::Borrowed => "borrowed",
            Ownership::Weak => "weak",
        }
    }

    /// Strength of the hold on the target, weakest first.
    fn rank(self) -> u8 {
        match self {
            Ownership::Weak => 0,
            Ownership::Borrowed => 1,
            Ownership::Unknown => 2,
            Ownership::Shared => 3,
            Ownership::Owning => 4,
        }
    }

    /// Ownership of the final target through a chain of holders (a container of pointers,
    /// a smart pointer to a container, ...): the weakest link decides.
    pub fn compose(self, inner: Ownership) -> Ownership {
        if inner.rank() < self.rank() {
            inner
        } else {
            self
        }
    }

    /// Refines an `Unknown` ownership with initialisation evidence: an immutable member
    /// that only ever holds objects created by its owner, and never hands them out, is
    /// owned. Known ownership is never changed.
    pub fn refine(self, evidence: &OwnershipEvidence) -> Ownership {
        if self == Ownership::Unknown
            && evidence.created_by_owner
            && evidence.immutable_binding
            && !evidence.assigned_from_parameter
            && !evidence.escapes
        {
            Ownership::Owning
        } else {
            self
        }
    }

    pub fn is_owning(self) -> bool {
        self == Ownership::Owning
    }

    /// UML relationship for a member with this ownership and multiplicity.
    pub fn assoc_kind(self, multiplicity: Multiplicity) -> AssocKind {
        match self {
            Ownership::Owning => AssocKind::Composition,
            Ownership::Shared => AssocKind::Aggregation,
            Ownership::Borrowed | Ownership::Weak => AssocKind::Association,
            Ownership::Unknown if multiplicity.is_many() => AssocKind::Aggregation,
            Ownership::Unknown => AssocKind::Association,
        }
    }
}

/// `mult_max` encoding of an unbounded upper bound (`*`); the same value as
/// `uma::types::MULTIPLICITY_MANY`.
pub const MULTIPLICITY_MANY: u16 = u16::MAX;
/// `mult_min` / `mult_max` encoding of a bound that cannot be derived; the same value as
/// `uma::types::MULTIPLICITY_UNKNOWN`.
pub const MULTIPLICITY_UNKNOWN: u16 = u16::MAX - 1;
/// Largest finite bound representable next to the two sentinels.
pub const MULTIPLICITY_MAX_FINITE: u16 = u16::MAX - 2;

/// How many target instances a member holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Multiplicity {
    /// `lower..upper`; `upper == None` is unbounded (`*`).
    Range { lower: u16, upper: Option<u16> },
    Unknown,
}

impl Default for Multiplicity {
    fn default() -> Self {
        Multiplicity::Unknown
    }
}

impl Multiplicity {
    pub const ZERO: Multiplicity = Multiplicity::Range {
        lower: 0,
        upper: Some(0),
    };
    pub const ONE: Multiplicity = Multiplicity::Range {
        lower: 1,
        upper: Some(1),
    };
    pub const OPTIONAL: Multiplicity = Multiplicity::Range {
        lower: 0,
        upper: Some(1),
    };
    pub const MANY: Multiplicity = Multiplicity::Range {
        lower: 0,
        upper: None,
    };
    pub const ONE_OR_MORE: Multiplicity = Multiplicity::Range {
        lower: 1,
        upper: None,
    };

    /// Exactly `n` instances (a fixed-size array). Counts above the representable maximum
    /// become "at least the maximum".
    pub fn exactly(n: u64) -> Self {
        if n > MULTIPLICITY_MAX_FINITE as u64 {
            Multiplicity::Range {
                lower: MULTIPLICITY_MAX_FINITE,
                upper: None,
            }
        } else {
            let n = n as u16;
            Multiplicity::Range {
                lower: n,
                upper: Some(n),
            }
        }
    }

    /// `lower..upper`; a contradictory range (`upper < lower`) is `Unknown`, and bounds above
    /// the representable maximum saturate (an upper bound becomes unbounded).
    pub fn range(lower: u16, upper: Option<u16>) -> Self {
        let lower = lower.min(MULTIPLICITY_MAX_FINITE);
        let upper = upper.and_then(|u| (u <= MULTIPLICITY_MAX_FINITE).then_some(u));
        match upper {
            Some(u) if u < lower => Multiplicity::Unknown,
            _ => Multiplicity::Range { lower, upper },
        }
    }

    pub fn lower(self) -> Option<u16> {
        match self {
            Multiplicity::Range { lower, .. } => Some(lower),
            Multiplicity::Unknown => None,
        }
    }

    /// `Some(None)` for an unbounded upper bound, `None` when unknown.
    pub fn upper(self) -> Option<Option<u16>> {
        match self {
            Multiplicity::Range { upper, .. } => Some(upper),
            Multiplicity::Unknown => None,
        }
    }

    /// More than one instance is possible.
    pub fn is_many(self) -> bool {
        match self {
            Multiplicity::Range { upper: None, .. } => true,
            Multiplicity::Range { upper: Some(u), .. } => u > 1,
            Multiplicity::Unknown => false,
        }
    }

    /// Zero instances are possible.
    pub fn is_optional(self) -> bool {
        matches!(self, Multiplicity::Range { lower: 0, .. })
    }

    /// Instances reachable through a holder of multiplicity `self` whose elements each hold
    /// `inner` instances (bounds multiply).
    pub fn compose(self, inner: Multiplicity) -> Multiplicity {
        match (self, inner) {
            (
                Multiplicity::Range {
                    lower: l1,
                    upper: u1,
                },
                Multiplicity::Range {
                    lower: l2,
                    upper: u2,
                },
            ) => {
                let lower = (l1 as u32 * l2 as u32).min(MULTIPLICITY_MAX_FINITE as u32) as u16;
                let upper = match (u1, u2) {
                    (Some(0), _) | (_, Some(0)) => Some(0),
                    (None, _) | (_, None) => None,
                    (Some(a), Some(b)) => {
                        let product = a as u32 * b as u32;
                        (product <= MULTIPLICITY_MAX_FINITE as u32).then_some(product as u16)
                    }
                };
                Multiplicity::Range { lower, upper }
            }
            _ => Multiplicity::Unknown,
        }
    }

    /// `(mult_min, mult_max)` with the `MULTIPLICITY_MANY` / `MULTIPLICITY_UNKNOWN` sentinels.
    pub fn to_bounds(self) -> (u16, u16) {
        match self {
            Multiplicity::Range { lower, upper } => {
                (lower, upper.unwrap_or(MULTIPLICITY_MANY))
            }
            Multiplicity::Unknown => (MULTIPLICITY_UNKNOWN, MULTIPLICITY_UNKNOWN),
        }
    }

    /// Inverse of [`Multiplicity::to_bounds`].
    pub fn from_bounds(min: u16, max: u16) -> Self {
        if min == MULTIPLICITY_UNKNOWN || max == MULTIPLICITY_UNKNOWN || min == MULTIPLICITY_MANY
        {
            return Multiplicity::Unknown;
        }
        let upper = if max == MULTIPLICITY_MANY {
            None
        } else {
            Some(max)
        };
        Multiplicity::range(min, upper)
    }

    /// UML multiplicity label (`1`, `0..1`, `0..*`, `2..5`); `None` when unknown.
    pub fn uml_label(self) -> Option<String> {
        match self {
            Multiplicity::Unknown => None,
            Multiplicity::Range {
                lower,
                upper: Some(u),
            } if lower == u => Some(u.to_string()),
            Multiplicity::Range {
                lower,
                upper: Some(u),
            } => Some(format!("{}..{}", lower, u)),
            Multiplicity::Range { lower, upper: None } => Some(format!("{}..*", lower)),
        }
    }
}

impl fmt::Display for Multiplicity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.uml_label() {
            Some(label) => f.write_str(&label),
            None => f.write_str("?"),
        }
    }
}

/// How a written supertype relates to the declaring type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SupertypeRelation {
    /// `extends` (specialization).
    Extends,
    /// `implements` (realization).
    Implements,
    /// A single list (`:` in C++, C#, Kotlin, Swift) whose entries are classified by the
    /// kind of the supertype.
    Unspecified,
}

impl SupertypeRelation {
    /// The hierarchy relation; an `Unspecified` relation is decided by the supertype's kind
    /// and is `None` while that kind is unknown (the caller records an assumed relation).
    pub fn to_th_relation(self, supertype: Option<TypeKind>) -> Option<THRelation> {
        match self {
            SupertypeRelation::Extends => Some(THRelation::TH_EXTENDS),
            SupertypeRelation::Implements => Some(THRelation::TH_IMPLEMENTS),
            SupertypeRelation::Unspecified => supertype.map(|k| {
                if k.is_realized_by_subtypes() {
                    THRelation::TH_IMPLEMENTS
                } else {
                    THRelation::TH_EXTENDS
                }
            }),
        }
    }
}

// ── Generics ─────────────────────────────────────────────────────────────────

/// Variance of a generic parameter (declaration site) or type argument (use site).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Variance {
    #[default]
    Invariant,
    /// `out T`, `+T`, `? extends T`.
    Covariant,
    /// `in T`, `-T`, `? super T`.
    Contravariant,
    /// `?`, `*`: any argument.
    Bivariant,
}

impl Variance {
    /// Variance named by a keyword or sign of any supported language.
    pub fn from_keyword(word: &str) -> Option<Self> {
        Some(match word {
            "out" | "+" | "extends" => Variance::Covariant,
            "in" | "-" | "super" => Variance::Contravariant,
            "?" | "*" => Variance::Bivariant,
            _ => return None,
        })
    }

    /// Decodes the `NodeAttr` lang byte of a wildcard `NN_TYPE_REF` (`WILDCARD_*`).
    pub fn from_node_attr_wildcard(lang_byte: u8) -> Self {
        match lang_byte {
            NodeAttr::WILDCARD_UNBOUNDED => Variance::Bivariant,
            NodeAttr::WILDCARD_EXTENDS => Variance::Covariant,
            NodeAttr::WILDCARD_SUPER => Variance::Contravariant,
            _ => Variance::Invariant,
        }
    }

    pub fn flip(self) -> Self {
        match self {
            Variance::Covariant => Variance::Contravariant,
            Variance::Contravariant => Variance::Covariant,
            other => other,
        }
    }

    /// Variance of a position nested at `inner` inside a position of variance `self`.
    pub fn compose(self, inner: Variance) -> Variance {
        match (self, inner) {
            (Variance::Bivariant, _) | (_, Variance::Bivariant) => Variance::Bivariant,
            (Variance::Invariant, _) | (_, Variance::Invariant) => Variance::Invariant,
            (Variance::Covariant, v) => v,
            (Variance::Contravariant, v) => v.flip(),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Variance::Invariant => "invariant",
            Variance::Covariant => "covariant",
            Variance::Contravariant => "contravariant",
            Variance::Bivariant => "bivariant",
        }
    }
}

/// What a generic parameter ranges over.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GenericParamKind {
    Type,
    /// C++ non-type template parameter, Rust const generic.
    Value,
    /// C++ template template parameter.
    Template,
    /// Rust lifetime parameter.
    Lifetime,
}

/// Kind of a constraint on a generic parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BoundKind {
    /// `T extends B`, `T : B`, `where T : B`, `T: Trait`.
    Upper,
    /// `T super B` (Java wildcards, Scala `>:`).
    Lower,
    /// C# `where T : class`.
    ReferenceType,
    /// C# `where T : struct`.
    ValueType,
    /// C# `where T : unmanaged`.
    Unmanaged,
    /// C# `where T : notnull`, Kotlin `T : Any`.
    NotNull,
    /// C# `where T : new()`.
    DefaultConstructible,
    /// C++20 concept / requires-clause constraint.
    Concept,
}

impl BoundKind {
    /// Constraint keywords that do not name a type (C# `class`, `struct`, `new()`, ...).
    pub fn from_constraint_keyword(word: &str) -> Option<Self> {
        Some(match word {
            "class" => BoundKind::ReferenceType,
            "struct" => BoundKind::ValueType,
            "unmanaged" => BoundKind::Unmanaged,
            "notnull" => BoundKind::NotNull,
            "new()" => BoundKind::DefaultConstructible,
            _ => return None,
        })
    }
}

/// One constraint of a generic parameter.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GenericBound {
    pub kind: BoundKind,
    /// The bounding type or concept as written; `None` for keyword constraints.
    pub type_name: Option<String>,
}

/// A generic / template parameter.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GenericParam {
    pub name: String,
    pub kind: GenericParamKind,
    pub variance: Variance,
    pub bounds: Vec<GenericBound>,
    /// Default argument as written (`template <class T = int>`).
    pub default: Option<String>,
    /// Variadic parameter pack (`typename... Ts`).
    pub is_pack: bool,
    /// Kotlin `reified`: the argument is available at run time.
    pub reified: bool,
}

impl GenericParam {
    pub fn new(name: impl Into<String>, kind: GenericParamKind) -> Self {
        Self {
            name: name.into(),
            kind,
            variance: Variance::Invariant,
            bounds: Vec::new(),
            default: None,
            is_pack: false,
            reified: false,
        }
    }

    pub fn with_bound(mut self, kind: BoundKind, type_name: Option<String>) -> Self {
        self.bounds.push(GenericBound { kind, type_name });
        self
    }

    /// Named upper bounds (supertypes every argument must have).
    pub fn upper_bounds(&self) -> impl Iterator<Item = &str> {
        self.bounds
            .iter()
            .filter(|b| b.kind == BoundKind::Upper)
            .filter_map(|b| b.type_name.as_deref())
    }

    pub fn is_unconstrained(&self) -> bool {
        self.bounds.is_empty()
    }
}

// ── Dispatch ─────────────────────────────────────────────────────────────────

/// How a call selects its callee.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CallDispatch {
    /// The callee is fixed at compile time.
    Static,
    /// Selected by the receiver's dynamic class among overrides.
    Virtual,
    /// Selected among the implementations of an interface / protocol member.
    Interface,
    /// Through a function value, delegate, reflection or run-time name lookup.
    Dynamic,
}

impl CallDispatch {
    /// Call-edge type code (`CG_EDGE_*`) of the dispatch; see [`CallDispatch::cg_edge_type`]
    /// for the constructor / special-invocation refinements.
    pub fn to_cg_edge(self) -> u8 {
        match self {
            CallDispatch::Static => CG_EDGE_DIRECT,
            CallDispatch::Virtual => CG_EDGE_VIRTUAL,
            CallDispatch::Interface => CG_EDGE_INTERFACE,
            CallDispatch::Dynamic => CG_EDGE_DYNAMIC,
        }
    }

    /// Dispatch of a stored call-edge type; `None` for an unknown code.
    pub fn from_cg_edge(code: u8) -> Option<Self> {
        Some(match code {
            CG_EDGE_DIRECT | CG_EDGE_SPECIAL | CG_EDGE_CONSTRUCTOR => CallDispatch::Static,
            CG_EDGE_VIRTUAL => CallDispatch::Virtual,
            CG_EDGE_INTERFACE => CallDispatch::Interface,
            CG_EDGE_DYNAMIC | CG_EDGE_REFLECTION => CallDispatch::Dynamic,
            _ => return None,
        })
    }

    /// Call-edge type code, distinguishing constructor calls and statically bound instance
    /// calls (`super.m()`, private methods) as the call graph records them.
    pub fn cg_edge_type(self, facts: &DispatchFacts) -> u8 {
        match self {
            CallDispatch::Static => match facts.target {
                Some(DeclKind::Member(MemberKind::Constructor)) | Some(DeclKind::Type(_)) => {
                    CG_EDGE_CONSTRUCTOR
                }
                Some(DeclKind::Member(m))
                    if m.is_callable()
                        && !facts.target_modifiers.contains(Modifiers::STATIC)
                        && (facts.super_receiver || facts.target_visibility.is_private()) =>
                {
                    CG_EDGE_SPECIAL
                }
                _ => CG_EDGE_DIRECT,
            },
            other => other.to_cg_edge(),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            CallDispatch::Static => "static",
            CallDispatch::Virtual => "virtual",
            CallDispatch::Interface => "interface",
            CallDispatch::Dynamic => "dynamic",
        }
    }
}

/// How a language binds instance-method calls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DispatchPolicy {
    /// Instance methods are overridable unless `final`, `private` or `static` (Java).
    VirtualByDefault,
    /// Instance methods are bound statically unless declared `virtual` / `abstract` /
    /// `override` / `open` or declared by an interface (C++, C#, Kotlin).
    ExplicitVirtual,
    /// No dynamic dispatch on named calls; only calls through function values are
    /// dynamic (C).
    StaticOnly,
    /// Every call is resolved at run time by name (Python, JavaScript, Ruby).
    LateBound,
}

/// What is known about a call site and its resolved target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DispatchFacts {
    /// Kind of the resolved callee; `None` when the call could not be resolved.
    pub target: Option<DeclKind>,
    pub target_modifiers: Modifiers,
    pub target_visibility: Visibility,
    /// Kind of the type that declares the callee.
    pub owner: Option<TypeKind>,
    pub owner_modifiers: Modifiers,
    /// The call names an explicit receiver (`obj.m()`, `Type.m()`).
    pub has_receiver: bool,
    /// The receiver is `super` / `base`.
    pub super_receiver: bool,
    /// The callee is named through a type qualifier that suppresses virtual dispatch
    /// (C++ `Base::m()`).
    pub type_qualified: bool,
    /// The call goes through a function value (function pointer, delegate, closure,
    /// method reference).
    pub indirect: bool,
    /// The callee overrides a virtual member of a supertype without restating `virtual`
    /// (C++ implicit virtual).
    pub overrides_virtual: bool,
}

/// Decides how a call is dispatched from the facts at the call site and the language policy.
/// Unresolved calls are over-approximated (virtual when the language could dispatch them),
/// so a call graph built on this decision stays sound.
pub fn resolve_dispatch(facts: &DispatchFacts, policy: DispatchPolicy) -> CallDispatch {
    if facts.indirect || policy == DispatchPolicy::LateBound {
        return CallDispatch::Dynamic;
    }
    let target = match facts.target {
        Some(target) => target,
        None => {
            return match policy {
                DispatchPolicy::StaticOnly => CallDispatch::Static,
                DispatchPolicy::VirtualByDefault => CallDispatch::Virtual,
                DispatchPolicy::ExplicitVirtual if facts.has_receiver => CallDispatch::Virtual,
                DispatchPolicy::ExplicitVirtual => CallDispatch::Static,
                DispatchPolicy::LateBound => CallDispatch::Dynamic,
            }
        }
    };
    let member = match target {
        DeclKind::Member(m) => m,
        // Calling a variable, parameter or field invokes the function value it holds.
        DeclKind::Variable | DeclKind::Parameter | DeclKind::Local | DeclKind::Lambda => {
            return CallDispatch::Dynamic
        }
        _ => return CallDispatch::Static,
    };
    match member {
        MemberKind::Constructor
        | MemberKind::StaticInitializer
        | MemberKind::InstanceInitializer
        | MemberKind::EnumConstant
        | MemberKind::NestedType => return CallDispatch::Static,
        MemberKind::Field | MemberKind::Event => return CallDispatch::Dynamic,
        _ => {}
    }
    if facts.target_modifiers.contains(Modifiers::STATIC)
        || facts.super_receiver
        || facts.type_qualified
    {
        return CallDispatch::Static;
    }
    let sealed_target = facts.target_modifiers.contains(Modifiers::FINAL)
        || facts.owner_modifiers.contains(Modifiers::FINAL)
        || facts.owner == Some(TypeKind::Object);
    let interface_owner = facts.owner.is_some_and(TypeKind::is_realized_by_subtypes);
    match policy {
        DispatchPolicy::StaticOnly => CallDispatch::Static,
        DispatchPolicy::VirtualByDefault => {
            if facts.target_visibility.is_private() || sealed_target {
                CallDispatch::Static
            } else if interface_owner {
                CallDispatch::Interface
            } else {
                CallDispatch::Virtual
            }
        }
        DispatchPolicy::ExplicitVirtual => {
            let overridable = facts.target_modifiers.intersects(
                Modifiers::VIRTUAL.union(Modifiers::ABSTRACT).union(Modifiers::OVERRIDE),
            ) || facts.overrides_virtual
                || interface_owner;
            if !overridable || sealed_target {
                CallDispatch::Static
            } else if interface_owner {
                CallDispatch::Interface
            } else {
                CallDispatch::Virtual
            }
        }
        DispatchPolicy::LateBound => CallDispatch::Dynamic,
    }
}

// ── Language semantics vocabulary ────────────────────────────────────────────

/// How object lifetimes are managed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MemoryModel {
    /// Tracing garbage collector (JVM, .NET, Go, JavaScript).
    Managed,
    /// Explicit allocation and release (C, C++).
    Manual,
    /// Reference counting (Swift ARC, Python, Objective-C).
    ReferenceCounted,
    /// Ownership and borrowing checked by the compiler (Rust).
    OwnershipTyped,
}

/// Whether variables of user-defined types hold values or references.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AggregateSemantics {
    /// Every user-defined type is accessed through references (Java, Kotlin, Python).
    References,
    /// Every type is a value unless a pointer or reference is written (C, C++, Rust).
    Values,
    /// Structs and enums are values, classes are references (C#, Swift).
    Mixed,
}

impl AggregateSemantics {
    /// True when a variable of a type of `kind` (`None`: not known) stores the instance
    /// itself rather than a reference to it.
    pub fn stores_value(self, kind: Option<TypeKind>) -> bool {
        match self {
            AggregateSemantics::Values => !matches!(kind, Some(TypeKind::Interface)),
            AggregateSemantics::Mixed => kind.is_some_and(TypeKind::is_value_type_by_default),
            AggregateSemantics::References => false,
        }
    }
}

/// How types are related by inheritance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InheritanceModel {
    /// No inheritance (C, Go structs).
    None,
    Single,
    /// One superclass plus any number of interfaces (Java, C#, Kotlin, Swift).
    SingleWithInterfaces,
    /// Several superclasses (C++, Python).
    Multiple,
    /// Prototype chains (JavaScript).
    Prototype,
}

/// Whether references may be null.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Nullability {
    /// Every reference may be null (Java, C# without nullable annotations, C pointers).
    Implicit,
    /// Only types marked nullable (`T?`) may be null (Kotlin, Swift, C# nullable context).
    Explicit,
    /// The language has no null references (Rust).
    NotApplicable,
}

/// How generic code is compiled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GenericsModel {
    None,
    /// One body for all arguments, type arguments erased (Java, Kotlin).
    Erased,
    /// One body, type arguments available at run time (C#).
    Reified,
    /// One instantiation per argument list (C++ templates, Rust monomorphization).
    Templates,
}

// ── Declared type expressions ────────────────────────────────────────────────

/// Reference declarator of a type (`T&`, `T&&`, C# `ref T`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ReferenceKind {
    #[default]
    None,
    LValue,
    RValue,
}

/// One argument of a generic / template instantiation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeArgument<'a> {
    Type(DeclaredType<'a>),
    /// A value argument (`4` in `std::array<int, 4>`) or an omitted argument (`<,>`) as
    /// written, trimmed.
    Value(&'a str),
}

/// A written type, parsed into the parts every algorithm needs, for any supported
/// language: `const std::vector<std::unique_ptr<Node>>&`, `Map<String, ? extends Order>`,
/// `List<Item>?`, `int[,]`, `struct node *next`, `(Int) -> Unit`.
///
/// All string fields borrow from the parsed text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclaredType<'a> {
    /// The whole type as written (trimmed).
    pub text: &'a str,
    /// Qualified head name without the type arguments of its last segment (`std::vector`,
    /// `java.util.Map.Entry`, `unsigned long`); a function type's whole text.
    pub head: &'a str,
    /// Type arguments of the last name segment.
    pub args: Vec<TypeArgument<'a>>,
    /// Array dimensions (each `[]`, `[n]` or `...`; a C# rank-`r` array counts `r`).
    pub array_dims: u8,
    /// Extent of each array dimension when written as an integer literal, outermost first.
    pub extents: Vec<Option<u64>>,
    /// `*` (and C++/CLI `^`) count.
    pub pointer_depth: u8,
    pub reference: ReferenceKind,
    /// `T?`.
    pub nullable: bool,
    /// `const`, `final`, `readonly`, C# `in`.
    pub is_const: bool,
    /// A function type or function pointer.
    pub function: bool,
    /// A wildcard type argument (`?`, `? extends T`, `*`); `head` is the bound, or `?` /
    /// `*` when unbounded.
    pub wildcard: bool,
    /// Use-site variance of a type argument.
    pub variance: Variance,
}

/// Why a type text could not be parsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeParseErrorKind {
    Empty,
    Unbalanced,
    Unexpected(char),
    TrailingInput,
    TooDeep,
}

/// A type text that is not a type of any supported language.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TypeParseError {
    pub kind: TypeParseErrorKind,
    /// Byte offset into the parsed text.
    pub offset: usize,
}

impl fmt::Display for TypeParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            TypeParseErrorKind::Empty => write!(f, "empty type at byte {}", self.offset),
            TypeParseErrorKind::Unbalanced => {
                write!(f, "unbalanced brackets in type at byte {}", self.offset)
            }
            TypeParseErrorKind::Unexpected(c) => {
                write!(f, "unexpected '{}' in type at byte {}", c, self.offset)
            }
            TypeParseErrorKind::TrailingInput => {
                write!(f, "text after the type at byte {}", self.offset)
            }
            TypeParseErrorKind::TooDeep => {
                write!(f, "type nested too deeply at byte {}", self.offset)
            }
        }
    }
}

impl std::error::Error for TypeParseError {}

/// Maximum nesting of type arguments, groups and bounds accepted by the parser.
const MAX_TYPE_NESTING: usize = 64;

/// Keywords that combine into one builtin scalar type (`unsigned long long int`).
const BUILTIN_SCALAR_WORDS: &[&str] = &[
    "signed",
    "unsigned",
    "short",
    "long",
    "int",
    "char",
    "float",
    "double",
    "void",
    "bool",
    "_Bool",
    "_Complex",
    "_Imaginary",
    "wchar_t",
    "char8_t",
    "char16_t",
    "char32_t",
    "__int8",
    "__int16",
    "__int32",
    "__int64",
    "__int128",
    "__float128",
    "__fp16",
    "_Float16",
    "_Float32",
    "_Float64",
    "_Float128",
];

fn is_builtin_scalar_word(word: &str) -> bool {
    BUILTIN_SCALAR_WORDS.contains(&word)
}

/// True when every word of `head` is a builtin scalar keyword (`unsigned long`, `double`).
pub fn is_builtin_scalar(head: &str) -> bool {
    let mut words = head.split_whitespace().peekable();
    words.peek().is_some() && words.all(is_builtin_scalar_word)
}

/// Last segment of a qualified name, for both `.` and `::` separators.
pub fn simple_name_of(name: &str) -> &str {
    let after_colons = name.rfind("::").map(|i| i + 2);
    let after_dot = name.rfind('.').map(|i| i + 1);
    let cut = after_colons.max(after_dot).unwrap_or(0);
    name[cut..].trim()
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_' || b == b'$' || b >= 0x80
}

fn is_ident_byte(b: u8) -> bool {
    is_ident_start(b) || b.is_ascii_digit()
}

/// Value of an integer literal of any supported language (`4`, `0x10`, `0b101`, `017`,
/// `1'000`, `1_000`, `8u`, `16UL`): array extents, `std::array` sizes, enum values.
pub fn parse_integer_literal(text: &str) -> Option<u64> {
    let cleaned: String = text
        .trim()
        .chars()
        .filter(|c| *c != '\'' && *c != '_')
        .collect();
    let digits = cleaned.trim_end_matches(['u', 'U', 'l', 'L', 'z', 'Z']);
    if digits.is_empty() {
        return None;
    }
    if let Some(hex) = digits.strip_prefix("0x").or_else(|| digits.strip_prefix("0X")) {
        return u64::from_str_radix(hex, 16).ok();
    }
    if let Some(bin) = digits.strip_prefix("0b").or_else(|| digits.strip_prefix("0B")) {
        return u64::from_str_radix(bin, 2).ok();
    }
    if digits.len() > 1 && digits.starts_with('0') {
        return u64::from_str_radix(&digits[1..], 8).ok();
    }
    digits.parse::<u64>().ok()
}

impl<'a> DeclaredType<'a> {
    /// A type with only a head name (no arguments or decorations).
    pub fn named(head: &'a str) -> Self {
        Self {
            text: head,
            head,
            args: Vec::new(),
            array_dims: 0,
            extents: Vec::new(),
            pointer_depth: 0,
            reference: ReferenceKind::None,
            nullable: false,
            is_const: false,
            function: false,
            wildcard: false,
            variance: Variance::Invariant,
        }
    }

    /// Parses a complete type text.
    pub fn parse(text: &'a str) -> Result<DeclaredType<'a>, TypeParseError> {
        let mut parser = TypeParser::new(text);
        let ty = parser.parse_type(0, false)?;
        parser.skip_ws();
        if parser.pos < parser.bytes.len() {
            return Err(parser.error(TypeParseErrorKind::TrailingInput));
        }
        Ok(ty)
    }

    /// Parses the type at the start of `text` and returns the unparsed rest (a declarator
    /// name, an initializer, ...).
    pub fn parse_prefix(text: &'a str) -> Result<(DeclaredType<'a>, &'a str), TypeParseError> {
        let mut parser = TypeParser::new(text);
        let ty = parser.parse_type(0, false)?;
        let rest = text[parser.pos..].trim_start();
        Ok((ty, rest))
    }

    /// Last segment of the head (`vector` for `std::vector`).
    pub fn simple_name(&self) -> &'a str {
        simple_name_of(self.head)
    }

    /// The `index`-th argument when it is a type.
    pub fn type_arg(&self, index: usize) -> Option<&DeclaredType<'a>> {
        match self.args.get(index)? {
            TypeArgument::Type(ty) => Some(ty),
            TypeArgument::Value(_) => None,
        }
    }

    /// The `index`-th argument when it is a value.
    pub fn value_arg(&self, index: usize) -> Option<&'a str> {
        match self.args.get(index)? {
            TypeArgument::Value(v) => Some(v),
            TypeArgument::Type(_) => None,
        }
    }

    /// The type arguments, skipping value arguments.
    pub fn type_args(&self) -> impl Iterator<Item = &DeclaredType<'a>> {
        self.args.iter().filter_map(|a| match a {
            TypeArgument::Type(ty) => Some(ty),
            TypeArgument::Value(_) => None,
        })
    }

    /// A pointer, reference or array declarator applies to the head.
    pub fn is_decorated(&self) -> bool {
        self.pointer_depth > 0 || self.array_dims > 0 || self.reference != ReferenceKind::None
    }

    /// An unbounded wildcard (`?`, `*`) that names no type.
    pub fn is_unbounded_wildcard(&self) -> bool {
        self.wildcard && (self.head == "?" || self.head == "*")
    }
}

struct TypeParser<'a> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> TypeParser<'a> {
    fn new(src: &'a str) -> Self {
        Self {
            src,
            bytes: src.as_bytes(),
            pos: 0,
        }
    }

    fn error(&self, kind: TypeParseErrorKind) -> TypeParseError {
        TypeParseError {
            kind,
            offset: self.pos,
        }
    }

    fn unexpected(&self) -> TypeParseError {
        match self.src.get(self.pos..).and_then(|s| s.chars().next()) {
            Some(c) => self.error(TypeParseErrorKind::Unexpected(c)),
            None => self.error(TypeParseErrorKind::Empty),
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<u8> {
        self.bytes.get(self.pos + offset).copied()
    }

    fn starts_with(&self, s: &str) -> bool {
        self.bytes[self.pos..].starts_with(s.as_bytes())
    }

    fn skip_ws(&mut self) {
        while self.peek().is_some_and(|b| b.is_ascii_whitespace()) {
            self.pos += 1;
        }
    }

    /// The identifier at the cursor (Kotlin backtick names included), not consumed.
    fn peek_word(&self) -> Option<&'a str> {
        let b = self.peek()?;
        if b == b'`' {
            let close = self.bytes[self.pos + 1..].iter().position(|&c| c == b'`')?;
            return Some(&self.src[self.pos..self.pos + close + 2]);
        }
        if !is_ident_start(b) {
            return None;
        }
        let mut end = self.pos + 1;
        while end < self.bytes.len() && is_ident_byte(self.bytes[end]) {
            end += 1;
        }
        Some(&self.src[self.pos..end])
    }

    fn take_word(&mut self) -> Option<&'a str> {
        let word = self.peek_word()?;
        self.pos += word.len();
        Some(word)
    }

    /// Index of the bracket closing the one at `open_pos`, skipping quoted literals.
    fn matching(&self, open_pos: usize, open: u8, close: u8) -> Result<usize, TypeParseError> {
        let mut depth = 0usize;
        let mut i = open_pos;
        while i < self.bytes.len() {
            let b = self.bytes[i];
            if b == b'"' || b == b'\'' {
                i += 1;
                while i < self.bytes.len() && self.bytes[i] != b {
                    if self.bytes[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
            } else if b == open {
                depth += 1;
            } else if b == close {
                depth -= 1;
                if depth == 0 {
                    return Ok(i);
                }
            }
            i += 1;
        }
        Err(TypeParseError {
            kind: TypeParseErrorKind::Unbalanced,
            offset: open_pos,
        })
    }

    fn skip_balanced(&mut self, open: u8, close: u8) -> Result<(), TypeParseError> {
        let end = self.matching(self.pos, open, close)?;
        self.pos = end + 1;
        Ok(())
    }

    /// Skips a Java / Kotlin type annotation `@Name(.Name)*[(...)]` at the cursor.
    fn skip_annotation(&mut self) -> Result<(), TypeParseError> {
        self.pos += 1;
        self.skip_ws();
        if self.take_word().is_none() {
            return Err(self.unexpected());
        }
        loop {
            if self.peek() == Some(b'.') && self.peek_at(1).is_some_and(is_ident_start) {
                self.pos += 1;
                self.take_word();
            } else {
                break;
            }
        }
        let save = self.pos;
        self.skip_ws();
        if self.peek() == Some(b'(') {
            self.skip_balanced(b'(', b')')
        } else {
            self.pos = save;
            Ok(())
        }
    }

    fn parse_type(&mut self, depth: usize, in_args: bool) -> Result<DeclaredType<'a>, TypeParseError> {
        if depth > MAX_TYPE_NESTING {
            return Err(self.error(TypeParseErrorKind::TooDeep));
        }
        self.skip_ws();
        let start = self.pos;
        if self.peek().is_none() {
            return Err(self.error(TypeParseErrorKind::Empty));
        }
        let mut ty = DeclaredType::named(&self.src[start..start]);

        // Prefix qualifiers, specifiers, annotations and use-site variance.
        loop {
            self.skip_ws();
            match self.peek() {
                Some(b'@') => {
                    self.skip_annotation()?;
                    continue;
                }
                Some(b'[') if self.peek_at(1) == Some(b'[') => {
                    self.skip_balanced(b'[', b']')?;
                    continue;
                }
                _ => {}
            }
            let Some(word) = self.peek_word() else { break };
            match word {
                "const" | "final" | "readonly" => ty.is_const = true,
                "volatile" | "restrict" | "__restrict" | "__restrict__" | "typename"
                | "struct" | "class" | "union" | "enum" | "scoped" | "register" | "static"
                | "extern" | "mutable" | "params" | "vararg" | "__unaligned" | "suspend"
                | "transient" => {}
                "ref" | "out" if !in_args => ty.reference = ReferenceKind::LValue,
                "in" if !in_args => {
                    ty.reference = ReferenceKind::LValue;
                    ty.is_const = true;
                }
                "out" => ty.variance = Variance::Covariant,
                "in" => ty.variance = Variance::Contravariant,
                _ => break,
            }
            self.pos += word.len();
        }

        self.skip_ws();
        let core_start = self.pos;
        match self.peek() {
            None => return Err(self.error(TypeParseErrorKind::Empty)),
            Some(b'?') if in_args => {
                // Java wildcard: `?`, `? extends T`, `? super T`.
                self.pos += 1;
                let after_mark = self.pos;
                self.skip_ws();
                let variance = match self.peek_word() {
                    Some("extends") => Some(Variance::Covariant),
                    Some("super") => Some(Variance::Contravariant),
                    _ => None,
                };
                match variance {
                    Some(v) => {
                        self.take_word();
                        let mut bound = self.parse_type(depth + 1, true)?;
                        bound.text = &self.src[start..self.pos];
                        bound.wildcard = true;
                        bound.variance = v;
                        bound.is_const |= ty.is_const;
                        return Ok(bound);
                    }
                    None => {
                        self.pos = after_mark;
                        ty.head = &self.src[core_start..after_mark];
                        ty.text = &self.src[start..after_mark];
                        ty.wildcard = true;
                        ty.variance = Variance::Bivariant;
                        return Ok(ty);
                    }
                }
            }
            Some(b'*') if in_args => {
                // Kotlin star projection.
                self.pos += 1;
                ty.head = &self.src[core_start..self.pos];
                ty.text = &self.src[start..self.pos];
                ty.wildcard = true;
                ty.variance = Variance::Bivariant;
                return Ok(ty);
            }
            Some(b'(') => {
                let close = self.matching(self.pos, b'(', b')')?;
                let mut look = close + 1;
                while look < self.bytes.len() && self.bytes[look].is_ascii_whitespace() {
                    look += 1;
                }
                if self.bytes[look..].starts_with(b"->") {
                    // Function type `(A, B) -> R` (Kotlin, Swift, TypeScript).
                    self.pos = look + 2;
                    self.parse_type(depth + 1, false)?;
                    ty.function = true;
                    ty.head = &self.src[core_start..self.pos];
                    ty.text = &self.src[start..self.pos];
                    return Ok(ty);
                }
                // Grouping parentheses: `(() -> Unit)?`.
                self.pos += 1;
                let inner = self.parse_type(depth + 1, in_args)?;
                self.skip_ws();
                if self.peek() != Some(b')') {
                    return Err(self.error(TypeParseErrorKind::Unbalanced));
                }
                self.pos += 1;
                let (outer_const, outer_ref, outer_variance) =
                    (ty.is_const, ty.reference, ty.variance);
                ty = inner;
                ty.is_const |= outer_const;
                if outer_ref != ReferenceKind::None {
                    ty.reference = outer_ref;
                }
                if outer_variance != Variance::Invariant {
                    ty.variance = outer_variance;
                }
            }
            Some(b)
                if is_ident_start(b)
                    || b == b'`'
                    || (b == b':' && self.peek_at(1) == Some(b':')) =>
            {
                self.parse_name(&mut ty, depth)?;
                if ty.function {
                    ty.text = &self.src[start..self.pos];
                    return Ok(ty);
                }
            }
            Some(_) => return Err(self.unexpected()),
        }

        self.parse_postfix(&mut ty)?;
        ty.text = &self.src[start..self.pos];
        Ok(ty)
    }

    /// Parses the head name (and the type arguments of its last segment) into `ty`.
    fn parse_name(&mut self, ty: &mut DeclaredType<'a>, depth: usize) -> Result<(), TypeParseError> {
        if let Some(word) = self.peek_word() {
            if is_builtin_scalar_word(word) {
                let first = self.pos;
                let mut head_end = self.pos;
                let mut consumed = self.pos;
                loop {
                    self.skip_ws();
                    match self.peek_word() {
                        Some(w) if is_builtin_scalar_word(w) => {
                            self.pos += w.len();
                            head_end = self.pos;
                            consumed = self.pos;
                        }
                        Some(w @ ("const" | "volatile")) => {
                            self.pos += w.len();
                            ty.is_const |= w == "const";
                            consumed = self.pos;
                        }
                        _ => break,
                    }
                }
                self.pos = consumed;
                ty.head = &self.src[first..head_end];
                return Ok(());
            }
            if matches!(word, "decltype" | "typeof" | "__typeof__" | "__typeof") {
                let first = self.pos;
                self.pos += word.len();
                let after_word = self.pos;
                self.skip_ws();
                if self.peek() == Some(b'(') {
                    self.skip_balanced(b'(', b')')?;
                } else {
                    self.pos = after_word;
                }
                ty.head = &self.src[first..self.pos];
                return Ok(());
            }
            if word == "_Atomic" {
                self.pos += word.len();
                let after_word = self.pos;
                self.skip_ws();
                if self.peek() == Some(b'(') {
                    // `_Atomic(T)` names T with an atomic qualifier.
                    self.pos += 1;
                    let inner = self.parse_type(depth + 1, false)?;
                    self.skip_ws();
                    if self.peek() != Some(b')') {
                        return Err(self.error(TypeParseErrorKind::Unbalanced));
                    }
                    self.pos += 1;
                    let is_const = ty.is_const || inner.is_const;
                    let variance = ty.variance;
                    *ty = inner;
                    ty.is_const = is_const;
                    ty.variance = variance;
                    return Ok(());
                }
                self.pos = after_word;
                self.skip_ws();
                return self.parse_name(ty, depth);
            }
        }

        let name_start = self.pos;
        if self.starts_with("::") {
            self.pos += 2;
        }
        loop {
            self.skip_ws();
            if self.peek_word() == Some("template") {
                self.pos += "template".len();
                self.skip_ws();
            }
            if self.take_word().is_none() {
                return Err(self.unexpected());
            }
            let name_end = self.pos;
            let save = self.pos;
            self.skip_ws();
            let args = if self.peek() == Some(b'<') {
                self.parse_args(depth + 1)?
            } else {
                self.pos = save;
                Vec::new()
            };
            let after_args = self.pos;
            self.skip_ws();
            if self.starts_with("::") {
                self.pos += 2;
                continue;
            }
            if self.peek() == Some(b'.') && self.peek_at(1) != Some(b'.') {
                let dot = self.pos;
                self.pos += 1;
                self.skip_ws();
                if self.peek() == Some(b'(') {
                    // Kotlin function type with a receiver: `A.(B) -> C`.
                    let close = self.matching(self.pos, b'(', b')')?;
                    self.pos = close + 1;
                    self.skip_ws();
                    if !self.starts_with("->") {
                        self.pos = dot;
                        return Err(self.unexpected());
                    }
                    self.pos += 2;
                    self.parse_type(depth + 1, false)?;
                    ty.function = true;
                    ty.head = &self.src[name_start..self.pos];
                    return Ok(());
                }
                continue;
            }
            self.pos = after_args;
            ty.head = &self.src[name_start..name_end];
            ty.args = args;
            return Ok(());
        }
    }

    /// Parses `<...>` at the cursor.
    fn parse_args(&mut self, depth: usize) -> Result<Vec<TypeArgument<'a>>, TypeParseError> {
        if depth > MAX_TYPE_NESTING {
            return Err(self.error(TypeParseErrorKind::TooDeep));
        }
        self.pos += 1;
        let mut args = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b'>') {
            self.pos += 1;
            return Ok(args);
        }
        loop {
            self.skip_ws();
            let arg = match self.peek() {
                None => return Err(self.error(TypeParseErrorKind::Unbalanced)),
                Some(b',') | Some(b'>') => TypeArgument::Value(&self.src[self.pos..self.pos]),
                _ if self.at_value_argument() => TypeArgument::Value(self.take_value_argument()?),
                _ => TypeArgument::Type(self.parse_type(depth, true)?),
            };
            args.push(arg);
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'>') => {
                    self.pos += 1;
                    return Ok(args);
                }
                None => return Err(self.error(TypeParseErrorKind::Unbalanced)),
                Some(_) => return Err(self.unexpected()),
            }
        }
    }

    fn at_value_argument(&self) -> bool {
        match self.peek() {
            Some(b) if b.is_ascii_digit() => true,
            Some(b'-' | b'+' | b'\'' | b'"' | b'!' | b'~') => true,
            _ => matches!(
                self.peek_word(),
                Some("true" | "false" | "nullptr" | "sizeof" | "alignof" | "noexcept")
            ),
        }
    }

    /// Consumes a value argument up to the `,` or `>` that ends it.
    fn take_value_argument(&mut self) -> Result<&'a str, TypeParseError> {
        let start = self.pos;
        let mut depth = 0usize;
        while let Some(b) = self.peek() {
            match b {
                b'"' | b'\'' => {
                    self.pos += 1;
                    while let Some(c) = self.peek() {
                        if c == b'\\' {
                            self.pos += 2;
                            continue;
                        }
                        self.pos += 1;
                        if c == b {
                            break;
                        }
                    }
                    continue;
                }
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => {
                    if depth == 0 {
                        return Err(self.unexpected());
                    }
                    depth -= 1;
                }
                b',' | b'>' if depth == 0 => {
                    return Ok(self.src[start..self.pos].trim());
                }
                _ => {}
            }
            self.pos += 1;
        }
        Err(self.error(TypeParseErrorKind::Unbalanced))
    }

    /// Parses declarators and qualifiers that follow the head.
    fn parse_postfix(&mut self, ty: &mut DeclaredType<'a>) -> Result<(), TypeParseError> {
        loop {
            let save = self.pos;
            self.skip_ws();
            match self.peek() {
                Some(b'*') | Some(b'^') => {
                    self.pos += 1;
                    ty.pointer_depth = ty.pointer_depth.saturating_add(1);
                }
                Some(b'&') => {
                    if self.peek_at(1) == Some(b'&') {
                        self.pos += 2;
                        ty.reference = ReferenceKind::RValue;
                    } else {
                        self.pos += 1;
                        ty.reference = ReferenceKind::LValue;
                    }
                }
                Some(b'[') if self.peek_at(1) == Some(b'[') => {
                    self.skip_balanced(b'[', b']')?;
                }
                Some(b'[') => {
                    let close = self.matching(self.pos, b'[', b']')?;
                    let inner = &self.src[self.pos + 1..close];
                    let mut rank = 1usize;
                    let mut nesting = 0i32;
                    for c in inner.bytes() {
                        match c {
                            b'(' | b'[' | b'{' => nesting += 1,
                            b')' | b']' | b'}' => nesting -= 1,
                            b',' if nesting == 0 => rank += 1,
                            _ => {}
                        }
                    }
                    if rank > 1 {
                        for _ in 0..rank {
                            ty.extents.push(None);
                        }
                    } else {
                        ty.extents.push(parse_integer_literal(inner));
                    }
                    let rank = u8::try_from(rank).unwrap_or(u8::MAX);
                    ty.array_dims = ty.array_dims.saturating_add(rank);
                    self.pos = close + 1;
                }
                Some(b'?') => {
                    self.pos += 1;
                    ty.nullable = true;
                }
                Some(b'!') => {
                    // Kotlin platform type `T!`.
                    self.pos += 1;
                }
                Some(b'.') if self.starts_with("...") => {
                    self.pos += 3;
                    ty.array_dims = ty.array_dims.saturating_add(1);
                    ty.extents.push(None);
                }
                Some(b'@') => self.skip_annotation()?,
                Some(b'(') => {
                    // Function declarator: `void (*)(int)`, `int(int)` in
                    // `std::function<int(int)>`, `void (Foo::*)()`.
                    self.skip_balanced(b'(', b')')?;
                    let after = self.pos;
                    self.skip_ws();
                    if self.peek() == Some(b'(') {
                        self.skip_balanced(b'(', b')')?;
                    } else {
                        self.pos = after;
                    }
                    ty.function = true;
                }
                Some(b) if is_ident_start(b) => match self.peek_word() {
                    Some("const") => {
                        self.pos += "const".len();
                        ty.is_const = true;
                    }
                    Some(w @ ("volatile" | "restrict" | "__restrict" | "__restrict__")) => {
                        self.pos += w.len();
                    }
                    Some("noexcept") => {
                        self.pos += "noexcept".len();
                        let after = self.pos;
                        self.skip_ws();
                        if self.peek() == Some(b'(') {
                            self.skip_balanced(b'(', b')')?;
                        } else {
                            self.pos = after;
                        }
                    }
                    _ => {
                        self.pos = save;
                        return Ok(());
                    }
                },
                _ => {
                    self.pos = save;
                    return Ok(());
                }
            }
        }
    }
}
