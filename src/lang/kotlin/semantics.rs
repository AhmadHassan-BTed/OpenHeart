//! Kotlin Semantic Adapter implementing Kotlin declaration classification and built-in types,
//! plus the Kotlin name-resolution rules Phase 3 applies on top of the scope graph:
//!
//! - imports: `import a.B`, `import a.B as C` (the alias hides `B` for that import),
//!   `import a.*`, and the default imports (`kotlin.*`, `kotlin.collections.*`, ... and, on the
//!   JVM, `kotlin.jvm.*` and `java.lang.*`);
//! - priority levels (Kotlin specification, "Scopes and identifiers"): lexical scopes, then
//!   explicit imports, then the declarations of the file's own package, then star imports,
//!   then default imports; two bindings on the same level are ambiguous;
//! - classifier scopes: nested classifiers of the enclosing classifiers and of their
//!   supertypes, companion objects, enum entries;
//! - implicit receivers: lambda receivers, the extension receiver, the dispatch receiver,
//!   companion objects of the class and of its superclasses, and outer instances (reachable
//!   only through `inner` classes);
//! - calls: members before extensions, extensions ranked by scope level and then by receiver
//!   specificity, top-level functions and constructor calls by scope level;
//! - JVM interop: Kotlin built-in types mapped to their Java classes and to the Java stubs,
//!   accessor names, Java getters seen as Kotlin properties, file facade classes.
//!
//! The resolvers work over [`KotlinDeclarationIndex`], which Phase 3 implements for project
//! declarations; [`crate::lang::kotlin::stdlib::KotlinStdLibManager`] implements it for the
//! library model, and [`LayeredIndex`] combines both.

use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::sync::OnceLock;

use crate::core::types::ast::ASTNodeType;
use crate::core::types::symbol::{ScopeKind, SymbolKind, SymbolVisibility};
use crate::lang::kotlin::stdlib::{type_stub_by_qual_name, KotlinTypeFlags, KOTLIN_TYPES};
use crate::symbol::adapter::java::{
    PRIMITIVE_BOOLEAN_ID, PRIMITIVE_BYTE_ID, PRIMITIVE_CHAR_ID, PRIMITIVE_DOUBLE_ID,
    PRIMITIVE_FLOAT_ID, PRIMITIVE_INT_ID, PRIMITIVE_LONG_ID, PRIMITIVE_SHORT_ID,
    PRIMITIVE_VOID_ID,
};
use crate::symbol::adapter::SemanticAdapter;
use crate::symbol::std_library::java_stubs::get_java_stubs;

/// Target platform; it decides the platform default imports and the JVM mappings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum KotlinPlatform {
    #[default]
    Jvm,
    Js,
    Native,
    /// Common (multiplatform) source set: only the platform-independent default imports.
    Common,
}

pub struct KotlinSemanticAdapter {
    platform: KotlinPlatform,
}

impl KotlinSemanticAdapter {
    pub fn new() -> Self {
        Self {
            platform: KotlinPlatform::Jvm,
        }
    }

    pub fn with_platform(platform: KotlinPlatform) -> Self {
        Self { platform }
    }

    pub fn platform(&self) -> KotlinPlatform {
        self.platform
    }

    /// Default-import tiers of this adapter's platform (see [`default_import_tiers`]).
    pub fn default_import_tiers(&self) -> &'static [&'static [&'static str]] {
        default_import_tiers(self.platform)
    }
}

impl Default for KotlinSemanticAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl SemanticAdapter for KotlinSemanticAdapter {
    fn is_declaration(&self, node_type: ASTNodeType) -> bool {
        matches!(
            node_type,
            ASTNodeType::NN_MODULE
                | ASTNodeType::NN_CLASS_DECL
                | ASTNodeType::NN_INTERFACE_DECL
                | ASTNodeType::NN_ENUM_DECL
                | ASTNodeType::NN_ANNOTATION_DECL
                | ASTNodeType::NN_TYPEDEF_DECL
                | ASTNodeType::NN_METHOD_DECL
                | ASTNodeType::NN_FUNCTION_DECL
                | ASTNodeType::NN_CONSTRUCTOR_DECL
                | ASTNodeType::NN_PROPERTY_DECL
                | ASTNodeType::NN_PARAM_DECL
                | ASTNodeType::NN_LOCAL_VAR_DECL
                | ASTNodeType::NN_TYPE_PARAM
                | ASTNodeType::NN_LAMBDA_EXPR
                | ASTNodeType::NN_ENUM_CONSTANT
                | ASTNodeType::NN_ANON_CLASS_BODY
                | ASTNodeType::NN_JAVA_INSTANCE_INIT
        )
    }

    fn symbol_kind(&self, node_type: ASTNodeType) -> SymbolKind {
        match node_type {
            ASTNodeType::NN_MODULE => SymbolKind::SK_MODULE,
            ASTNodeType::NN_CLASS_DECL => SymbolKind::SK_CLASS,
            ASTNodeType::NN_INTERFACE_DECL => SymbolKind::SK_INTERFACE,
            ASTNodeType::NN_ENUM_DECL => SymbolKind::SK_ENUM,
            ASTNodeType::NN_ANNOTATION_DECL => SymbolKind::SK_ANNOTATION_TYPE,
            // A type alias introduces a classifier name; `SymbolKind` has no alias kind, and
            // the class kind is the one under which type references bind to the name.
            ASTNodeType::NN_TYPEDEF_DECL => SymbolKind::SK_CLASS,
            ASTNodeType::NN_METHOD_DECL | ASTNodeType::NN_FUNCTION_DECL => SymbolKind::SK_METHOD,
            ASTNodeType::NN_CONSTRUCTOR_DECL => SymbolKind::SK_CONSTRUCTOR,
            ASTNodeType::NN_PROPERTY_DECL => SymbolKind::SK_FIELD,
            ASTNodeType::NN_PARAM_DECL => SymbolKind::SK_PARAM,
            ASTNodeType::NN_LOCAL_VAR_DECL => SymbolKind::SK_LOCAL_VAR,
            ASTNodeType::NN_TYPE_PARAM => SymbolKind::SK_TYPE_PARAM,
            ASTNodeType::NN_LAMBDA_EXPR => SymbolKind::SK_LAMBDA,
            ASTNodeType::NN_ENUM_CONSTANT => SymbolKind::SK_ENUM_CONSTANT,
            ASTNodeType::NN_ANON_CLASS_BODY => SymbolKind::SK_ANON_CLASS,
            ASTNodeType::NN_JAVA_INSTANCE_INIT => SymbolKind::SK_INSTANCE_INIT,
            _ => SymbolKind::SK_EXTERNAL,
        }
    }

    fn scope_kind(&self, node_type: ASTNodeType) -> ScopeKind {
        match node_type {
            ASTNodeType::NN_MODULE => ScopeKind::File,
            ASTNodeType::NN_CLASS_DECL
            | ASTNodeType::NN_INTERFACE_DECL
            | ASTNodeType::NN_ENUM_DECL
            | ASTNodeType::NN_ANNOTATION_DECL => ScopeKind::Class,
            ASTNodeType::NN_ANON_CLASS_BODY => ScopeKind::Anon,
            ASTNodeType::NN_METHOD_DECL
            | ASTNodeType::NN_FUNCTION_DECL
            | ASTNodeType::NN_CONSTRUCTOR_DECL
            | ASTNodeType::NN_JAVA_INSTANCE_INIT => ScopeKind::Method,
            ASTNodeType::NN_LAMBDA_EXPR => ScopeKind::Lambda,
            _ => ScopeKind::Block,
        }
    }

    /// Kotlin has no primitive keywords: `Int`, `Boolean`, ... are classes of package
    /// `kotlin`. A non-null use compiles to the JVM primitive, which is what these ids name;
    /// `Int?` is boxed (`java.lang.Integer`) and is recognised from the type reference's
    /// nullable flag. `Unit` in return position is `void`.
    fn primitive_type_id(&self, text: &str) -> Option<u32> {
        let name = text.strip_prefix("kotlin.").unwrap_or(text);
        match name {
            "Unit" => Some(PRIMITIVE_VOID_ID),
            "Boolean" => Some(PRIMITIVE_BOOLEAN_ID),
            "Byte" => Some(PRIMITIVE_BYTE_ID),
            "Short" => Some(PRIMITIVE_SHORT_ID),
            "Char" => Some(PRIMITIVE_CHAR_ID),
            "Int" => Some(PRIMITIVE_INT_ID),
            "Long" => Some(PRIMITIVE_LONG_ID),
            "Float" => Some(PRIMITIVE_FLOAT_ID),
            "Double" => Some(PRIMITIVE_DOUBLE_ID),
            _ => None,
        }
    }

    fn is_collection_type(&self, type_name: &str) -> bool {
        let name = type_name
            .strip_prefix("kotlin.collections.")
            .or_else(|| type_name.strip_prefix("kotlin.sequences."))
            .or_else(|| type_name.strip_prefix("kotlin."))
            .or_else(|| type_name.strip_prefix("java.util."))
            .or_else(|| type_name.strip_prefix("java.lang."))
            .unwrap_or(type_name);
        matches!(
            name,
            "Iterable"
                | "MutableIterable"
                | "Collection"
                | "MutableCollection"
                | "List"
                | "MutableList"
                | "Set"
                | "MutableSet"
                | "Map"
                | "MutableMap"
                | "ArrayList"
                | "LinkedList"
                | "HashMap"
                | "HashSet"
                | "LinkedHashMap"
                | "LinkedHashSet"
                | "TreeMap"
                | "TreeSet"
                | "SortedMap"
                | "SortedSet"
                | "ArrayDeque"
                | "Deque"
                | "Queue"
                | "Sequence"
                | "Array"
                | "BooleanArray"
                | "ByteArray"
                | "CharArray"
                | "ShortArray"
                | "IntArray"
                | "LongArray"
                | "FloatArray"
                | "DoubleArray"
        )
    }

    /// Declarations without a visibility modifier are `public` in Kotlin, at every level.
    fn default_visibility(&self, _kind: SymbolKind) -> SymbolVisibility {
        SymbolVisibility::Public
    }
}

// ─────────────────────────────────────────────────────────────────────────────────────────
// Modifiers and visibility
// ─────────────────────────────────────────────────────────────────────────────────────────

/// Set of Kotlin modifier keywords.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct KotlinModifierSet(u32);

impl KotlinModifierSet {
    pub const PUBLIC: u32 = 1 << 0;
    pub const PRIVATE: u32 = 1 << 1;
    pub const PROTECTED: u32 = 1 << 2;
    pub const INTERNAL: u32 = 1 << 3;
    pub const OPEN: u32 = 1 << 4;
    pub const FINAL: u32 = 1 << 5;
    pub const ABSTRACT: u32 = 1 << 6;
    pub const SEALED: u32 = 1 << 7;
    pub const OVERRIDE: u32 = 1 << 8;
    pub const LATEINIT: u32 = 1 << 9;
    pub const CONST: u32 = 1 << 10;
    pub const DATA: u32 = 1 << 11;
    pub const VALUE: u32 = 1 << 12;
    pub const INNER: u32 = 1 << 13;
    pub const ENUM: u32 = 1 << 14;
    pub const ANNOTATION: u32 = 1 << 15;
    pub const COMPANION: u32 = 1 << 16;
    pub const INLINE: u32 = 1 << 17;
    pub const NOINLINE: u32 = 1 << 18;
    pub const CROSSINLINE: u32 = 1 << 19;
    pub const INFIX: u32 = 1 << 20;
    pub const OPERATOR: u32 = 1 << 21;
    pub const TAILREC: u32 = 1 << 22;
    pub const SUSPEND: u32 = 1 << 23;
    pub const EXTERNAL: u32 = 1 << 24;
    pub const VARARG: u32 = 1 << 25;
    pub const REIFIED: u32 = 1 << 26;
    pub const EXPECT: u32 = 1 << 27;
    pub const ACTUAL: u32 = 1 << 28;
    /// Variance `in` (declaration-site or use-site).
    pub const IN: u32 = 1 << 29;
    /// Variance `out` (declaration-site or use-site).
    pub const OUT: u32 = 1 << 30;

    pub const fn empty() -> Self {
        Self(0)
    }

    pub const fn bits(self) -> u32 {
        self.0
    }

    pub fn insert(&mut self, bits: u32) {
        self.0 |= bits;
    }

    pub const fn contains(self, bits: u32) -> bool {
        self.0 & bits == bits
    }

    /// The modifier bit a keyword spells, or `None` for any other word.
    pub fn from_keyword(keyword: &str) -> Option<u32> {
        Some(match keyword {
            "public" => Self::PUBLIC,
            "private" => Self::PRIVATE,
            "protected" => Self::PROTECTED,
            "internal" => Self::INTERNAL,
            "open" => Self::OPEN,
            "final" => Self::FINAL,
            "abstract" => Self::ABSTRACT,
            "sealed" => Self::SEALED,
            "override" => Self::OVERRIDE,
            "lateinit" => Self::LATEINIT,
            "const" => Self::CONST,
            "data" => Self::DATA,
            "value" => Self::VALUE,
            "inner" => Self::INNER,
            "enum" => Self::ENUM,
            "annotation" => Self::ANNOTATION,
            "companion" => Self::COMPANION,
            "inline" => Self::INLINE,
            "noinline" => Self::NOINLINE,
            "crossinline" => Self::CROSSINLINE,
            "infix" => Self::INFIX,
            "operator" => Self::OPERATOR,
            "tailrec" => Self::TAILREC,
            "suspend" => Self::SUSPEND,
            "external" => Self::EXTERNAL,
            "vararg" => Self::VARARG,
            "reified" => Self::REIFIED,
            "expect" => Self::EXPECT,
            "actual" => Self::ACTUAL,
            "in" => Self::IN,
            "out" => Self::OUT,
            _ => return None,
        })
    }

    /// Collects the modifier keywords among `words` (e.g. the token texts in front of a
    /// declaration); other words are ignored.
    pub fn from_keywords<'a, I: IntoIterator<Item = &'a str>>(words: I) -> Self {
        let mut set = Self::empty();
        for word in words {
            if let Some(bit) = Self::from_keyword(word) {
                set.insert(bit);
            }
        }
        set
    }

    /// The explicit visibility modifier, if any.
    pub fn visibility(self) -> Option<KotlinVisibility> {
        if self.contains(Self::PRIVATE) {
            Some(KotlinVisibility::Private)
        } else if self.contains(Self::PROTECTED) {
            Some(KotlinVisibility::Protected)
        } else if self.contains(Self::INTERNAL) {
            Some(KotlinVisibility::Internal)
        } else if self.contains(Self::PUBLIC) {
            Some(KotlinVisibility::Public)
        } else {
            None
        }
    }
}

/// Kotlin declaration visibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KotlinVisibility {
    Public,
    /// Visible inside the same module (compilation unit set).
    Internal,
    /// Visible inside the declaring classifier and its subclasses.
    Protected,
    /// Visible inside the declaring classifier, or the declaring file for top-level
    /// declarations.
    Private,
}

impl KotlinVisibility {
    /// The `SymbolVisibility` recorded for this visibility. `internal` is recorded as
    /// `Package`, the narrower-than-public level `SymbolVisibility` offers (rendered `~` in
    /// UML); its true extent, the module, is checked with [`KotlinVisibility::is_accessible`].
    pub fn to_symbol_visibility(self) -> SymbolVisibility {
        match self {
            KotlinVisibility::Public => SymbolVisibility::Public,
            KotlinVisibility::Internal => SymbolVisibility::Package,
            KotlinVisibility::Protected => SymbolVisibility::Protected,
            KotlinVisibility::Private => SymbolVisibility::Private,
        }
    }

    /// Kotlin visibility from the 4-bit visibility field written by the Kotlin reducer.
    pub fn from_node_attr(visibility: u8) -> Option<Self> {
        use crate::core::types::ast::NodeAttr;
        use crate::lang::kotlin::reduce::KotlinAttr;
        match visibility {
            NodeAttr::VISIBILITY_PUBLIC => Some(KotlinVisibility::Public),
            NodeAttr::VISIBILITY_PRIVATE => Some(KotlinVisibility::Private),
            NodeAttr::VISIBILITY_PROTECTED => Some(KotlinVisibility::Protected),
            KotlinAttr::VISIBILITY_INTERNAL => Some(KotlinVisibility::Internal),
            _ => None,
        }
    }

    /// Kotlin accessibility rule (Kotlin specification, "Declaration visibility").
    pub fn is_accessible(self, top_level: bool, ctx: &KotlinAccessContext) -> bool {
        match self {
            KotlinVisibility::Public => true,
            KotlinVisibility::Internal => ctx.same_module,
            // `protected` does not apply to top-level declarations.
            KotlinVisibility::Protected => {
                !top_level && (ctx.inside_declaring_classifier || ctx.inside_subclass)
            }
            KotlinVisibility::Private => {
                if top_level {
                    ctx.same_file
                } else {
                    ctx.inside_declaring_classifier
                }
            }
        }
    }
}

/// Where an access happens, relative to the accessed declaration.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KotlinAccessContext {
    pub same_module: bool,
    pub same_file: bool,
    /// The access is inside the declaring classifier (or a classifier nested in it).
    pub inside_declaring_classifier: bool,
    /// The access is inside a subclass of the declaring classifier.
    pub inside_subclass: bool,
}

// ─────────────────────────────────────────────────────────────────────────────────────────
// Implicit names
// ─────────────────────────────────────────────────────────────────────────────────────────

/// Name of a companion object declared without a name.
pub const DEFAULT_COMPANION_NAME: &str = "Companion";
/// Implicit parameter of a lambda with one parameter and no parameter list.
pub const IMPLICIT_LAMBDA_PARAMETER: &str = "it";
/// Backing field of a property, visible inside its accessors.
pub const BACKING_FIELD_NAME: &str = "field";
/// Parameter name of a setter declared without one (`set(v)` names it explicitly).
pub const DEFAULT_SETTER_PARAMETER: &str = "value";

/// A position that introduces implicit names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KotlinImplicitScope {
    /// Lambda literal without a parameter list whose expected function type has one parameter.
    SingleParameterLambda,
    /// Body of a property getter.
    Getter,
    /// Body of a property setter (`value` only when the setter declares no parameter name).
    Setter { declares_parameter: bool },
}

/// Names a scope declares implicitly, as `SymbolFlags::IMPLICIT` declarations.
pub fn implicit_names(scope: KotlinImplicitScope) -> &'static [&'static str] {
    match scope {
        KotlinImplicitScope::SingleParameterLambda => &[IMPLICIT_LAMBDA_PARAMETER],
        KotlinImplicitScope::Getter => &[BACKING_FIELD_NAME],
        KotlinImplicitScope::Setter {
            declares_parameter: true,
        } => &[BACKING_FIELD_NAME],
        KotlinImplicitScope::Setter {
            declares_parameter: false,
        } => &[BACKING_FIELD_NAME, DEFAULT_SETTER_PARAMETER],
    }
}

/// Removes the backticks of an escaped identifier (`` `fun` `` is the identifier `fun`).
pub fn unescape_identifier(name: &str) -> &str {
    name.strip_prefix('`')
        .and_then(|n| n.strip_suffix('`'))
        .unwrap_or(name)
}

// ─────────────────────────────────────────────────────────────────────────────────────────
// Package and import directives
// ─────────────────────────────────────────────────────────────────────────────────────────

/// Default-import tiers of a platform. Every package listed is imported on demand by every
/// file; within the default level an earlier tier hides a later one, so `String` binds to
/// `kotlin.String` and not to `java.lang.String`.
pub fn default_import_tiers(platform: KotlinPlatform) -> &'static [&'static [&'static str]] {
    const COMMON: &[&str] = &[
        "kotlin",
        "kotlin.annotation",
        "kotlin.collections",
        "kotlin.comparisons",
        "kotlin.io",
        "kotlin.ranges",
        "kotlin.sequences",
        "kotlin.text",
    ];
    const JVM: &[&[&str]] = &[COMMON, &["kotlin.jvm"], &["java.lang"]];
    const JS: &[&[&str]] = &[COMMON, &["kotlin.js"]];
    const NATIVE_OR_COMMON: &[&[&str]] = &[COMMON];
    match platform {
        KotlinPlatform::Jvm => JVM,
        KotlinPlatform::Js => JS,
        KotlinPlatform::Native | KotlinPlatform::Common => NATIVE_OR_COMMON,
    }
}

/// True when `package` is imported by default on `platform`.
pub fn is_default_imported_package(package: &str, platform: KotlinPlatform) -> bool {
    default_import_tiers(platform)
        .iter()
        .any(|tier| tier.contains(&package))
}

/// A package or import directive that does not follow the Kotlin grammar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KotlinSyntaxError {
    /// The text does not start with the `import` keyword.
    NotAnImport,
    /// The text does not start with the `package` keyword.
    NotAPackageDirective,
    /// No name follows the keyword.
    EmptyPath,
    /// A backtick identifier is not closed on the same line.
    UnterminatedBacktick,
    /// `import a.* as b` (a star import cannot be aliased).
    AliasOnStarImport,
    /// `as` is not followed by a name.
    MissingAlias,
    /// A token that the directive grammar does not allow at this position.
    UnexpectedToken(String),
}

impl fmt::Display for KotlinSyntaxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KotlinSyntaxError::NotAnImport => write!(f, "not an import directive"),
            KotlinSyntaxError::NotAPackageDirective => write!(f, "not a package directive"),
            KotlinSyntaxError::EmptyPath => write!(f, "directive names no package or declaration"),
            KotlinSyntaxError::UnterminatedBacktick => write!(f, "unterminated backtick identifier"),
            KotlinSyntaxError::AliasOnStarImport => write!(f, "a star import cannot have an alias"),
            KotlinSyntaxError::MissingAlias => write!(f, "`as` is not followed by an alias name"),
            KotlinSyntaxError::UnexpectedToken(t) => write!(f, "unexpected token `{}`", t),
        }
    }
}

impl std::error::Error for KotlinSyntaxError {}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DirectiveToken {
    /// Identifier or keyword; `true` when written in backticks (never a keyword then).
    Word(String, bool),
    Dot,
    Star,
    Semicolon,
}

/// Splits directive text into tokens. Whitespace (including the separators of token texts
/// joined from the corpus) and comments are skipped.
fn lex_directive(text: &str) -> Result<Vec<DirectiveToken>, KotlinSyntaxError> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
        } else if c == '/' && chars.get(i + 1) == Some(&'*') {
            // Kotlin block comments nest.
            let mut depth = 0usize;
            while i < chars.len() {
                if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
                    depth += 1;
                    i += 2;
                } else if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    i += 1;
                }
            }
        } else if c == '.' {
            tokens.push(DirectiveToken::Dot);
            i += 1;
        } else if c == '*' {
            tokens.push(DirectiveToken::Star);
            i += 1;
        } else if c == ';' {
            tokens.push(DirectiveToken::Semicolon);
            i += 1;
        } else if c == '`' {
            let start = i + 1;
            let mut end = start;
            while end < chars.len() && chars[end] != '`' && chars[end] != '\n' {
                end += 1;
            }
            if end >= chars.len() || chars[end] != '`' || end == start {
                return Err(KotlinSyntaxError::UnterminatedBacktick);
            }
            tokens.push(DirectiveToken::Word(chars[start..end].iter().collect(), true));
            i = end + 1;
        } else if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            tokens.push(DirectiveToken::Word(chars[start..i].iter().collect(), false));
        } else {
            return Err(KotlinSyntaxError::UnexpectedToken(c.to_string()));
        }
    }
    Ok(tokens)
}

fn describe(token: &DirectiveToken) -> String {
    match token {
        DirectiveToken::Word(w, true) => format!("`{}`", w),
        DirectiveToken::Word(w, false) => w.clone(),
        DirectiveToken::Dot => ".".to_string(),
        DirectiveToken::Star => "*".to_string(),
        DirectiveToken::Semicolon => ";".to_string(),
    }
}

/// Parses `Ident (. Ident)*` starting at `pos`; returns the segments and the next position.
/// A trailing `.` is left unconsumed when followed by `*`.
fn parse_path(tokens: &[DirectiveToken], mut pos: usize) -> Result<(Vec<String>, usize), KotlinSyntaxError> {
    let mut segments = Vec::new();
    match tokens.get(pos) {
        Some(DirectiveToken::Word(w, _)) => {
            segments.push(w.clone());
            pos += 1;
        }
        Some(other) => return Err(KotlinSyntaxError::UnexpectedToken(describe(other))),
        None => return Err(KotlinSyntaxError::EmptyPath),
    }
    while let (Some(DirectiveToken::Dot), Some(DirectiveToken::Word(w, _))) =
        (tokens.get(pos), tokens.get(pos + 1))
    {
        segments.push(w.clone());
        pos += 2;
    }
    Ok((segments, pos))
}

/// Parses a package directive (`package a.b.c` with optional `;`) into its segments.
pub fn parse_package_directive(text: &str) -> Result<Vec<String>, KotlinSyntaxError> {
    let tokens = lex_directive(text)?;
    match tokens.first() {
        Some(DirectiveToken::Word(w, false)) if w == "package" => {}
        _ => return Err(KotlinSyntaxError::NotAPackageDirective),
    }
    let (segments, mut pos) = parse_path(&tokens, 1)?;
    if tokens.get(pos) == Some(&DirectiveToken::Semicolon) {
        pos += 1;
    }
    match tokens.get(pos) {
        None => Ok(segments),
        Some(other) => Err(KotlinSyntaxError::UnexpectedToken(describe(other))),
    }
}

/// One import directive of a Kotlin file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KotlinImport {
    /// Name segments, backticks removed (`import a.b.C` is `["a", "b", "C"]`; for a star
    /// import the segments name the package or classifier whose members are imported).
    pub path: Vec<String>,
    /// `import a.B as C`: `Some("C")`.
    pub alias: Option<String>,
    /// `import a.b.*`.
    pub on_demand: bool,
}

impl KotlinImport {
    /// Parses `import a.b.C`, `import a.b.C as D`, `import a.b.*` (optional `;`, comments
    /// and whitespace between tokens allowed, backtick identifiers unescaped).
    pub fn parse(text: &str) -> Result<Self, KotlinSyntaxError> {
        let tokens = lex_directive(text)?;
        match tokens.first() {
            Some(DirectiveToken::Word(w, false)) if w == "import" => {}
            _ => return Err(KotlinSyntaxError::NotAnImport),
        }
        let (path, mut pos) = parse_path(&tokens, 1)?;
        let mut on_demand = false;
        let mut alias = None;

        if let (Some(DirectiveToken::Dot), Some(DirectiveToken::Star)) =
            (tokens.get(pos), tokens.get(pos + 1))
        {
            on_demand = true;
            pos += 2;
        }
        if let Some(DirectiveToken::Word(w, false)) = tokens.get(pos) {
            if w == "as" {
                if on_demand {
                    return Err(KotlinSyntaxError::AliasOnStarImport);
                }
                match tokens.get(pos + 1) {
                    Some(DirectiveToken::Word(a, _)) => {
                        alias = Some(a.clone());
                        pos += 2;
                    }
                    _ => return Err(KotlinSyntaxError::MissingAlias),
                }
            }
        }
        if tokens.get(pos) == Some(&DirectiveToken::Semicolon) {
            pos += 1;
        }
        match tokens.get(pos) {
            None => Ok(Self {
                path,
                alias,
                on_demand,
            }),
            Some(other) => Err(KotlinSyntaxError::UnexpectedToken(describe(other))),
        }
    }

    /// `a.b.C` (for a star import, the package or classifier imported from).
    pub fn qualified_name(&self) -> String {
        self.path.join(".")
    }

    /// The simple name this import makes visible: the alias, or the last segment; `None`
    /// for a star import.
    pub fn visible_name(&self) -> Option<&str> {
        if self.on_demand {
            None
        } else {
            self.alias
                .as_deref()
                .or_else(|| self.path.last().map(|s| s.as_str()))
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────────────────
// Declaration index
// ─────────────────────────────────────────────────────────────────────────────────────────

/// Kind of a classifier declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KotlinClassifierKind {
    Class,
    Interface,
    Object,
    CompanionObject,
    Enum,
    Annotation,
    TypeAlias,
}

impl KotlinClassifierKind {
    /// `Foo(...)` constructs an instance (interfaces, objects and annotation classes cannot be
    /// instantiated with a call, enum classes only through their entries).
    pub fn is_constructible(self) -> bool {
        matches!(self, KotlinClassifierKind::Class | KotlinClassifierKind::TypeAlias)
    }
}

/// Kind of a member or callable declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KotlinMemberKind {
    Function,
    Property,
    /// A nested classifier (`Outer.Inner`).
    Classifier,
    EnumEntry,
}

/// A member of a classifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct KotlinMember {
    /// Qualified name of the declaring classifier.
    pub owner: String,
    pub name: String,
    pub kind: KotlinMemberKind,
}

impl KotlinMember {
    /// `owner.name`.
    pub fn qualified_name(&self) -> String {
        qualify(&self.owner, &self.name)
    }
}

/// An extension function or property.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct KotlinExtension {
    /// Package that declares the extension.
    pub package: String,
    pub name: String,
    /// Qualified name of the receiver type (`kotlin.Any` for an unbounded generic receiver).
    pub receiver: String,
    /// The receiver type is nullable (`T?.f()`, or an unbounded type parameter), so the
    /// extension applies to nullable receivers without a safe call.
    pub receiver_nullable: bool,
    pub kind: KotlinMemberKind,
}

impl KotlinExtension {
    /// `package.name`, the name an explicit import of the extension spells.
    pub fn qualified_name(&self) -> String {
        qualify(&self.package, &self.name)
    }
}

/// Declarations visible to the Kotlin resolvers. Phase 3 implements it over the project's
/// symbols; the standard-library model implements it over the built-in catalog.
pub trait KotlinDeclarationIndex {
    /// Kind of the classifier with this qualified name (nested: `a.Outer.Inner`), if known.
    fn classifier_kind(&self, qual_name: &str) -> Option<KotlinClassifierKind>;
    /// True when a top-level function or property has this qualified name.
    fn has_callable(&self, qual_name: &str) -> bool;
    /// Members named `name` declared directly in classifier `owner` (not inherited).
    fn members_named(&self, owner: &str, name: &str) -> Vec<KotlinMember>;
    /// Direct supertypes of `classifier`, as qualified names.
    fn supertypes(&self, classifier: &str) -> Vec<String>;
    /// Qualified name of `classifier`'s companion object, if it declares one.
    fn companion_object(&self, classifier: &str) -> Option<String>;
    /// Every extension function or property named `name`.
    fn extensions_named(&self, name: &str) -> Vec<KotlinExtension>;
    /// Expansion of a type alias.
    fn typealias_target(&self, _qual_name: &str) -> Option<String> {
        None
    }
    /// True when the classifier is declared in Java (its getters read as Kotlin properties).
    fn is_java_classifier(&self, _qual_name: &str) -> bool {
        false
    }
}

/// Several indexes queried in order (project first, then libraries).
pub struct LayeredIndex<'a> {
    layers: Vec<&'a dyn KotlinDeclarationIndex>,
}

impl<'a> LayeredIndex<'a> {
    pub fn new(layers: Vec<&'a dyn KotlinDeclarationIndex>) -> Self {
        Self { layers }
    }
}

impl KotlinDeclarationIndex for LayeredIndex<'_> {
    fn classifier_kind(&self, qual_name: &str) -> Option<KotlinClassifierKind> {
        self.layers.iter().find_map(|l| l.classifier_kind(qual_name))
    }

    fn has_callable(&self, qual_name: &str) -> bool {
        self.layers.iter().any(|l| l.has_callable(qual_name))
    }

    fn members_named(&self, owner: &str, name: &str) -> Vec<KotlinMember> {
        dedup(self.layers.iter().flat_map(|l| l.members_named(owner, name)))
    }

    fn supertypes(&self, classifier: &str) -> Vec<String> {
        dedup(self.layers.iter().flat_map(|l| l.supertypes(classifier)))
    }

    fn companion_object(&self, classifier: &str) -> Option<String> {
        self.layers.iter().find_map(|l| l.companion_object(classifier))
    }

    fn extensions_named(&self, name: &str) -> Vec<KotlinExtension> {
        dedup(self.layers.iter().flat_map(|l| l.extensions_named(name)))
    }

    fn typealias_target(&self, qual_name: &str) -> Option<String> {
        self.layers.iter().find_map(|l| l.typealias_target(qual_name))
    }

    fn is_java_classifier(&self, qual_name: &str) -> bool {
        self.layers.iter().any(|l| l.is_java_classifier(qual_name))
    }
}

fn dedup<T: Clone + Eq + std::hash::Hash, I: IntoIterator<Item = T>>(items: I) -> Vec<T> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for item in items {
        if seen.insert(item.clone()) {
            out.push(item);
        }
    }
    out
}

fn qualify(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{}.{}", prefix, name)
    }
}

/// The root supertype of every Kotlin class.
pub const KOTLIN_ANY: &str = "kotlin.Any";

/// `classifier` and all its supertypes with their inheritance distance (0 for the
/// classifier itself), breadth first, cycle-safe; type aliases are expanded, and
/// `kotlin.Any` closes the list when no declared supertype reached it.
pub fn supertype_distances(classifier: &str, index: &dyn KotlinDeclarationIndex) -> Vec<(String, usize)> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut queue = VecDeque::new();
    queue.push_back((classifier.to_string(), 0usize));
    while let Some((current, distance)) = queue.pop_front() {
        let current = match index.typealias_target(&current) {
            Some(target) if !seen.contains(&target) => target,
            _ => current,
        };
        if !seen.insert(current.clone()) {
            continue;
        }
        for supertype in index.supertypes(&current) {
            if !seen.contains(&supertype) {
                queue.push_back((supertype, distance + 1));
            }
        }
        out.push((current, distance));
    }
    if !seen.contains(KOTLIN_ANY) {
        let max = out.iter().map(|(_, d)| *d).max().unwrap_or(0);
        out.push((KOTLIN_ANY.to_string(), max + 1));
    }
    out
}

// ─────────────────────────────────────────────────────────────────────────────────────────
// File scope and classifier resolution
// ─────────────────────────────────────────────────────────────────────────────────────────

/// Resolution priority level; lower levels win.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum KotlinScopeLevel {
    /// Enclosing classifiers (nested classifiers of them and of their supertypes) and
    /// implicit receivers.
    Lexical = 0,
    /// Explicit (single-name or aliased) imports.
    Explicit = 1,
    /// Declarations of the file's own package, in any file.
    SamePackage = 2,
    /// Star imports.
    Star = 3,
    /// Default imports.
    Default = 4,
    /// A fully qualified name written in source.
    FullyQualified = 5,
}

/// Outcome of resolving a name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KotlinBinding {
    Bound {
        qual_name: String,
        level: KotlinScopeLevel,
    },
    /// Several declarations are visible at the same level; Kotlin reports an ambiguity.
    Ambiguous {
        level: KotlinScopeLevel,
        candidates: Vec<String>,
    },
    Unresolved,
}

impl KotlinBinding {
    pub fn qual_name(&self) -> Option<&str> {
        match self {
            KotlinBinding::Bound { qual_name, .. } => Some(qual_name),
            _ => None,
        }
    }

    pub fn level(&self) -> Option<KotlinScopeLevel> {
        match self {
            KotlinBinding::Bound { level, .. } | KotlinBinding::Ambiguous { level, .. } => {
                Some(*level)
            }
            KotlinBinding::Unresolved => None,
        }
    }
}

fn bind_at(level: KotlinScopeLevel, candidates: Vec<String>) -> Option<KotlinBinding> {
    let mut candidates = dedup(candidates);
    match candidates.len() {
        0 => None,
        1 => Some(KotlinBinding::Bound {
            qual_name: candidates.remove(0),
            level,
        }),
        _ => Some(KotlinBinding::Ambiguous { level, candidates }),
    }
}

/// The import scope of one Kotlin file.
#[derive(Debug, Clone)]
pub struct KotlinFileScope {
    package: String,
    /// (visible simple name, qualified name) of every explicit import.
    explicit: Vec<(String, String)>,
    /// Packages and classifiers imported on demand.
    star: Vec<String>,
    platform: KotlinPlatform,
}

impl KotlinFileScope {
    /// `package` is the file's package (empty for the root package).
    pub fn new(package: &str, imports: &[KotlinImport], platform: KotlinPlatform) -> Self {
        let mut explicit = Vec::new();
        let mut star = Vec::new();
        for import in imports {
            let qual = import.qualified_name();
            if import.on_demand {
                if !star.contains(&qual) {
                    star.push(qual);
                }
            } else if let Some(visible) = import.visible_name() {
                explicit.push((visible.to_string(), qual));
            }
        }
        Self {
            package: package.to_string(),
            explicit,
            star,
            platform,
        }
    }

    pub fn package(&self) -> &str {
        &self.package
    }

    pub fn platform(&self) -> KotlinPlatform {
        self.platform
    }

    /// (visible name, qualified name) pairs of the explicit imports.
    pub fn explicit_imports(&self) -> &[(String, String)] {
        &self.explicit
    }

    /// Packages and classifiers imported with `.*`.
    pub fn star_imports(&self) -> &[String] {
        &self.star
    }

    fn explicit_named<'s>(&'s self, name: &'s str) -> impl Iterator<Item = &'s String> + 's {
        self.explicit
            .iter()
            .filter(move |(visible, _)| visible == name)
            .map(|(_, qual)| qual)
    }

    /// Binds a simple classifier name by the file-level priority levels.
    pub fn resolve_classifier(&self, name: &str, index: &dyn KotlinDeclarationIndex) -> KotlinBinding {
        let name = unescape_identifier(name);

        // An explicit import binds its name even when the target is outside the analysed
        // sources and catalog, unless it is known to name a callable rather than a classifier.
        let explicit: Vec<String> = self
            .explicit_named(name)
            .filter(|q| index.classifier_kind(q).is_some() || !index.has_callable(q))
            .cloned()
            .collect();
        if let Some(binding) = bind_at(KotlinScopeLevel::Explicit, explicit) {
            return binding;
        }

        let own = qualify(&self.package, name);
        if index.classifier_kind(&own).is_some() {
            return KotlinBinding::Bound {
                qual_name: own,
                level: KotlinScopeLevel::SamePackage,
            };
        }

        let star: Vec<String> = self
            .star
            .iter()
            .map(|p| qualify(p, name))
            .filter(|q| index.classifier_kind(q).is_some())
            .collect();
        if let Some(binding) = bind_at(KotlinScopeLevel::Star, star) {
            return binding;
        }

        for tier in default_import_tiers(self.platform) {
            let found: Vec<String> = tier
                .iter()
                .map(|p| qualify(p, name))
                .filter(|q| index.classifier_kind(q).is_some())
                .collect();
            if let Some(binding) = bind_at(KotlinScopeLevel::Default, found) {
                return binding;
            }
        }
        KotlinBinding::Unresolved
    }

    /// Binds a simple classifier name used inside `enclosing` classifiers (innermost first):
    /// nested classifiers of each enclosing classifier and of its supertypes come before the
    /// file-level levels. Type parameters and local classifiers are lexical declarations of
    /// the scope graph and are resolved before this is consulted.
    pub fn resolve_classifier_in(
        &self,
        name: &str,
        enclosing: &[&str],
        index: &dyn KotlinDeclarationIndex,
    ) -> KotlinBinding {
        let simple = unescape_identifier(name);
        for classifier in enclosing {
            for (owner, _) in supertype_distances(classifier, index) {
                let nested = qualify(&owner, simple);
                if index.classifier_kind(&nested).is_some() {
                    return KotlinBinding::Bound {
                        qual_name: nested,
                        level: KotlinScopeLevel::Lexical,
                    };
                }
            }
        }
        self.resolve_classifier(simple, index)
    }

    /// Binds a dotted classifier name (`Outer.Inner`, `Alias.Inner`, `a.b.C`): the first
    /// segment is resolved as a classifier when possible and the rest are nested names;
    /// otherwise the name is fully qualified.
    pub fn resolve_qualified_classifier(
        &self,
        segments: &[&str],
        enclosing: &[&str],
        index: &dyn KotlinDeclarationIndex,
    ) -> KotlinBinding {
        let segments: Vec<&str> = segments.iter().map(|s| unescape_identifier(s)).collect();
        match segments.len() {
            0 => return KotlinBinding::Unresolved,
            1 => return self.resolve_classifier_in(segments[0], enclosing, index),
            _ => {}
        }
        let rest = segments[1..].join(".");
        let full = segments.join(".");

        if let KotlinBinding::Bound { qual_name, level } =
            self.resolve_classifier_in(segments[0], enclosing, index)
        {
            let nested = qualify(&qual_name, &rest);
            let head_known = index.classifier_kind(&qual_name).is_some();
            if index.classifier_kind(&nested).is_some() || !head_known {
                return KotlinBinding::Bound {
                    qual_name: nested,
                    level,
                };
            }
        }
        // A qualified name whose head is not a classifier is package-qualified; it is the
        // only legal reading, so it binds even when the declaration is outside the index.
        KotlinBinding::Bound {
            qual_name: full,
            level: KotlinScopeLevel::FullyQualified,
        }
    }

    /// Binds a top-level function or property name by the file-level priority levels.
    pub fn resolve_top_level_callable(
        &self,
        name: &str,
        index: &dyn KotlinDeclarationIndex,
    ) -> KotlinBinding {
        let name = unescape_identifier(name);
        let explicit: Vec<String> = self
            .explicit_named(name)
            .filter(|q| index.has_callable(q) || index.classifier_kind(q).is_none())
            .cloned()
            .collect();
        if let Some(binding) = bind_at(KotlinScopeLevel::Explicit, explicit) {
            return binding;
        }
        let own = qualify(&self.package, name);
        if index.has_callable(&own) {
            return KotlinBinding::Bound {
                qual_name: own,
                level: KotlinScopeLevel::SamePackage,
            };
        }
        let star: Vec<String> = self
            .star
            .iter()
            .map(|p| qualify(p, name))
            .filter(|q| index.has_callable(q))
            .collect();
        if let Some(binding) = bind_at(KotlinScopeLevel::Star, star) {
            return binding;
        }
        for tier in default_import_tiers(self.platform) {
            let found: Vec<String> = tier
                .iter()
                .map(|p| qualify(p, name))
                .filter(|q| index.has_callable(q))
                .collect();
            if let Some(binding) = bind_at(KotlinScopeLevel::Default, found) {
                return binding;
            }
        }
        KotlinBinding::Unresolved
    }

    /// `<package>.<name>` when exactly one non-default star import exists: the only binding
    /// the name can have if the file compiles, for names the index does not know.
    pub fn sole_star_candidate(&self, name: &str) -> Option<String> {
        match self.star.as_slice() {
            [only] => Some(qualify(only, unescape_identifier(name))),
            _ => None,
        }
    }

    /// The level at which `extension` is visible under the name `called`, or `None` when it
    /// is not imported into this file.
    pub fn extension_level(&self, extension: &KotlinExtension, called: &str) -> Option<KotlinScopeLevel> {
        let qual = extension.qualified_name();
        if self.explicit_named(called).any(|q| *q == qual) {
            return Some(KotlinScopeLevel::Explicit);
        }
        if extension.name != called {
            return None;
        }
        if extension.package == self.package {
            return Some(KotlinScopeLevel::SamePackage);
        }
        if self.star.contains(&extension.package) {
            return Some(KotlinScopeLevel::Star);
        }
        if is_default_imported_package(&extension.package, self.platform) {
            return Some(KotlinScopeLevel::Default);
        }
        None
    }

    /// Extensions callable as `called` on a receiver of type `receiver`, ranked by scope level
    /// and then by receiver specificity (nearest supertype first). Returns the best-ranked
    /// candidates (more than one means the call is ambiguous).
    pub fn applicable_extensions(
        &self,
        receiver: &str,
        receiver_nullable: bool,
        called: &str,
        index: &dyn KotlinDeclarationIndex,
    ) -> Vec<(KotlinExtension, KotlinScopeLevel)> {
        let called = unescape_identifier(called);
        let distances: HashMap<String, usize> =
            supertype_distances(receiver, index).into_iter().collect();

        // Aliased explicit imports make an extension callable under the alias.
        let mut pool = index.extensions_named(called);
        for qual in self.explicit_named(called) {
            if let Some(original) = qual.rsplit('.').next() {
                if original != called {
                    pool.extend(
                        index
                            .extensions_named(original)
                            .into_iter()
                            .filter(|e| e.qualified_name() == *qual),
                    );
                }
            }
        }

        let mut ranked: Vec<(KotlinScopeLevel, usize, KotlinExtension)> = Vec::new();
        for extension in dedup(pool) {
            if receiver_nullable && !extension.receiver_nullable {
                continue;
            }
            let distance = match distances.get(&extension.receiver) {
                Some(d) => *d,
                None => continue,
            };
            if let Some(level) = self.extension_level(&extension, called) {
                ranked.push((level, distance, extension));
            }
        }
        let best = match ranked.iter().map(|(l, d, _)| (*l, *d)).min() {
            Some(b) => b,
            None => return Vec::new(),
        };
        ranked
            .into_iter()
            .filter(|(l, d, _)| (*l, *d) == best)
            .map(|(l, _, e)| (e, l))
            .collect()
    }

    /// Resolves an unqualified call `name(...)` (or property read `name`) from a position
    /// whose implicit receivers are `receivers` (see [`implicit_receivers`]). Local functions
    /// and variables are lexical declarations of the scope graph and are resolved first.
    pub fn resolve_call(
        &self,
        name: &str,
        receivers: &[ImplicitReceiver],
        index: &dyn KotlinDeclarationIndex,
    ) -> KotlinCallTarget {
        let name = unescape_identifier(name);
        for receiver in receivers {
            if let Some((owner, members)) = find_callable_members(&receiver.qual_name, name, index) {
                return member_target(owner, members, receiver.kind);
            }
            let extensions = self.applicable_extensions(&receiver.qual_name, false, name, index);
            if let Some(target) = extension_target(extensions, &receiver.qual_name) {
                return target;
            }
        }

        let function = self.resolve_top_level_callable(name, index);
        let constructor = match self.resolve_classifier(name, index) {
            KotlinBinding::Bound { qual_name, level } => {
                let constructible = index
                    .classifier_kind(&qual_name)
                    .map(|k| k.is_constructible())
                    .unwrap_or(true);
                if constructible {
                    Some((qual_name, level))
                } else {
                    None
                }
            }
            _ => None,
        };
        match (function, constructor) {
            (KotlinBinding::Bound { qual_name, level }, Some((class, class_level))) => {
                if level < class_level {
                    KotlinCallTarget::TopLevel { qual_name, level }
                } else if class_level < level {
                    KotlinCallTarget::Constructor {
                        classifier: class,
                        level: class_level,
                    }
                } else {
                    KotlinCallTarget::Ambiguous {
                        candidates: vec![qual_name, class],
                    }
                }
            }
            (KotlinBinding::Bound { qual_name, level }, None) => {
                KotlinCallTarget::TopLevel { qual_name, level }
            }
            (KotlinBinding::Ambiguous { candidates, .. }, _) => {
                KotlinCallTarget::Ambiguous { candidates }
            }
            (KotlinBinding::Unresolved, Some((class, level))) => KotlinCallTarget::Constructor {
                classifier: class,
                level,
            },
            (KotlinBinding::Unresolved, None) => KotlinCallTarget::Unresolved,
        }
    }

    /// Resolves `receiver.name(...)` / `receiver?.name(...)` for a receiver expression of type
    /// `receiver_type`. Members win over extensions; a member cannot be called on a nullable
    /// receiver without a safe call, while an extension with a nullable receiver can.
    pub fn resolve_call_on(
        &self,
        receiver_type: &str,
        receiver_nullable: bool,
        safe_call: bool,
        name: &str,
        index: &dyn KotlinDeclarationIndex,
    ) -> KotlinCallTarget {
        let name = unescape_identifier(name);
        let nullable_unsafe = receiver_nullable && !safe_call;
        if !nullable_unsafe {
            if let Some((owner, members)) = find_callable_members(receiver_type, name, index) {
                return member_target(owner, members, KotlinReceiverKind::Explicit);
            }
            if let Some(target) = java_synthetic_property(receiver_type, name, index) {
                return target;
            }
        }
        let extensions = self.applicable_extensions(receiver_type, nullable_unsafe, name, index);
        extension_target(extensions, receiver_type).unwrap_or(KotlinCallTarget::Unresolved)
    }
}

/// Members named `name` of `owner` or of its nearest supertypes that declare one; all
/// supertypes at the nearest distance contribute (overload resolution chooses among them).
pub fn find_members(owner: &str, name: &str, index: &dyn KotlinDeclarationIndex) -> Option<(String, Vec<KotlinMember>)> {
    let mut best: Option<(usize, String, Vec<KotlinMember>)> = None;
    for (classifier, distance) in supertype_distances(owner, index) {
        if let Some((nearest, _, _)) = &best {
            if distance > *nearest {
                break;
            }
        }
        let members = index.members_named(&classifier, name);
        if members.is_empty() {
            continue;
        }
        if let Some(entry) = best.as_mut() {
            entry.2.extend(members);
            continue;
        }
        best = Some((distance, classifier, members));
    }
    best.map(|(_, classifier, members)| (classifier, dedup(members)))
}

fn find_callable_members(
    owner: &str,
    name: &str,
    index: &dyn KotlinDeclarationIndex,
) -> Option<(String, Vec<KotlinMember>)> {
    find_members(owner, name, index).and_then(|(classifier, members)| {
        let callable: Vec<KotlinMember> = members
            .into_iter()
            .filter(|m| matches!(m.kind, KotlinMemberKind::Function | KotlinMemberKind::Property))
            .collect();
        if callable.is_empty() {
            None
        } else {
            Some((classifier, callable))
        }
    })
}

fn member_target(owner: String, mut members: Vec<KotlinMember>, via: KotlinReceiverKind) -> KotlinCallTarget {
    if members.len() == 1 {
        KotlinCallTarget::Member {
            owner,
            member: members.remove(0),
            via,
        }
    } else {
        // Overloads of one name on one classifier: the member is identified, the overload is
        // left to argument typing.
        KotlinCallTarget::Overloaded {
            owner,
            members,
            via,
        }
    }
}

fn extension_target(
    mut extensions: Vec<(KotlinExtension, KotlinScopeLevel)>,
    receiver: &str,
) -> Option<KotlinCallTarget> {
    match extensions.len() {
        0 => None,
        1 => {
            let (extension, level) = extensions.remove(0);
            Some(KotlinCallTarget::Extension {
                extension,
                receiver: receiver.to_string(),
                level,
            })
        }
        _ => Some(KotlinCallTarget::Ambiguous {
            candidates: dedup(extensions.iter().map(|(e, _)| e.qualified_name())),
        }),
    }
}

/// Java getters and setters read as Kotlin properties (`obj.name` calls `getName()`).
fn java_synthetic_property(
    receiver_type: &str,
    name: &str,
    index: &dyn KotlinDeclarationIndex,
) -> Option<KotlinCallTarget> {
    let java_receiver = supertype_distances(receiver_type, index)
        .iter()
        .any(|(c, _)| index.is_java_classifier(c));
    if !java_receiver {
        return None;
    }
    for accessor in KotlinJavaInterop::java_getter_candidates(name) {
        if let Some((owner, members)) = find_callable_members(receiver_type, &accessor, index) {
            let getters: Vec<KotlinMember> = members
                .into_iter()
                .filter(|m| m.kind == KotlinMemberKind::Function)
                .collect();
            if !getters.is_empty() {
                return Some(member_target(owner, getters, KotlinReceiverKind::Explicit));
            }
        }
    }
    None
}

/// Resolves `Classifier.name` where the qualifier names a classifier (not an instance):
/// nested classifiers, then members of the classifier when it is an object, then members of
/// its companion object, then enum entries.
pub fn resolve_classifier_member(
    classifier: &str,
    name: &str,
    index: &dyn KotlinDeclarationIndex,
) -> KotlinCallTarget {
    let name = unescape_identifier(name);
    let nested = qualify(classifier, name);
    if index.classifier_kind(&nested).is_some() {
        return KotlinCallTarget::Member {
            owner: classifier.to_string(),
            member: KotlinMember {
                owner: classifier.to_string(),
                name: name.to_string(),
                kind: KotlinMemberKind::Classifier,
            },
            via: KotlinReceiverKind::Static,
        };
    }
    let kind = index.classifier_kind(classifier);
    if matches!(
        kind,
        Some(KotlinClassifierKind::Object) | Some(KotlinClassifierKind::CompanionObject)
    ) {
        if let Some((owner, members)) = find_callable_members(classifier, name, index) {
            return member_target(owner, members, KotlinReceiverKind::Static);
        }
    }
    if let Some(companion) = index.companion_object(classifier) {
        if let Some((owner, members)) = find_callable_members(&companion, name, index) {
            return member_target(owner, members, KotlinReceiverKind::Companion);
        }
    }
    let entries: Vec<KotlinMember> = index
        .members_named(classifier, name)
        .into_iter()
        .filter(|m| m.kind == KotlinMemberKind::EnumEntry)
        .collect();
    if !entries.is_empty() {
        return member_target(classifier.to_string(), entries, KotlinReceiverKind::Static);
    }
    KotlinCallTarget::Unresolved
}

// ─────────────────────────────────────────────────────────────────────────────────────────
// Implicit receivers and call targets
// ─────────────────────────────────────────────────────────────────────────────────────────

/// How a member is reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KotlinReceiverKind {
    /// Written receiver expression `x.f()`.
    Explicit,
    /// Receiver of a lambda with receiver (`apply { f() }`).
    LambdaReceiver,
    /// Extension receiver of the enclosing extension function.
    ExtensionReceiver,
    /// Dispatch receiver: the innermost enclosing classifier (`this`).
    This,
    /// Companion object of an enclosing classifier.
    Companion,
    /// Companion object of a superclass of an enclosing classifier.
    SupertypeCompanion,
    /// Instance of an outer classifier, reachable from `inner` classes (`this@Outer`).
    Outer,
    /// Classifier-qualified access `Type.member` (objects, enum entries, nested classifiers).
    Static,
}

/// One implicit receiver available at a position, in priority order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImplicitReceiver {
    pub qual_name: String,
    pub kind: KotlinReceiverKind,
}

/// A classifier enclosing a position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnclosingClassifier<'a> {
    pub qual_name: &'a str,
    pub kind: KotlinClassifierKind,
    /// `inner` class, or a local / anonymous class (both capture the enclosing instance).
    pub captures_outer: bool,
}

/// Implicit receivers at a position, highest priority first (Kotlin specification,
/// "Receivers"): receivers of enclosing lambdas with receiver (innermost first), the extension
/// receiver of the enclosing extension function, then for each enclosing classifier
/// (innermost first) its instance, its companion object and the companion objects of its
/// superclasses. A nested classifier that is not `inner` cannot reach the instances of the
/// classifiers around it, only their objects and companions.
pub fn implicit_receivers(
    lambda_receivers: &[&str],
    extension_receiver: Option<&str>,
    enclosing: &[EnclosingClassifier<'_>],
    index: &dyn KotlinDeclarationIndex,
) -> Vec<ImplicitReceiver> {
    fn push(out: &mut Vec<ImplicitReceiver>, qual_name: String, kind: KotlinReceiverKind) {
        if !out.iter().any(|r| r.qual_name == qual_name && r.kind == kind) {
            out.push(ImplicitReceiver { qual_name, kind });
        }
    }

    let mut out: Vec<ImplicitReceiver> = Vec::new();
    for receiver in lambda_receivers {
        push(&mut out, receiver.to_string(), KotlinReceiverKind::LambdaReceiver);
    }
    if let Some(receiver) = extension_receiver {
        push(&mut out, receiver.to_string(), KotlinReceiverKind::ExtensionReceiver);
    }

    let mut instance_reachable = true;
    for (depth, classifier) in enclosing.iter().enumerate() {
        let is_object = matches!(
            classifier.kind,
            KotlinClassifierKind::Object | KotlinClassifierKind::CompanionObject
        );
        if instance_reachable || is_object {
            let kind = if depth == 0 {
                KotlinReceiverKind::This
            } else {
                KotlinReceiverKind::Outer
            };
            push(&mut out, classifier.qual_name.to_string(), kind);
        }
        if let Some(companion) = index.companion_object(classifier.qual_name) {
            push(&mut out, companion, KotlinReceiverKind::Companion);
        }
        for (supertype, distance) in supertype_distances(classifier.qual_name, index) {
            if distance == 0 {
                continue;
            }
            if let Some(companion) = index.companion_object(&supertype) {
                push(&mut out, companion, KotlinReceiverKind::SupertypeCompanion);
            }
        }
        if !classifier.captures_outer {
            instance_reachable = false;
        }
    }
    out
}

/// What a call or name reference resolves to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KotlinCallTarget {
    /// A single member of a classifier (or of its nearest supertype declaring it).
    Member {
        owner: String,
        member: KotlinMember,
        via: KotlinReceiverKind,
    },
    /// Several overloads of one member name on one classifier.
    Overloaded {
        owner: String,
        members: Vec<KotlinMember>,
        via: KotlinReceiverKind,
    },
    /// An extension function or property.
    Extension {
        extension: KotlinExtension,
        receiver: String,
        level: KotlinScopeLevel,
    },
    /// A top-level function or property.
    TopLevel {
        qual_name: String,
        level: KotlinScopeLevel,
    },
    /// A constructor call `Foo(...)`.
    Constructor {
        classifier: String,
        level: KotlinScopeLevel,
    },
    /// Several targets at the same priority.
    Ambiguous { candidates: Vec<String> },
    Unresolved,
}

// ─────────────────────────────────────────────────────────────────────────────────────────
// Synthesized members
// ─────────────────────────────────────────────────────────────────────────────────────────

/// Classifier flavours whose members the compiler synthesizes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KotlinClassifierFlavor {
    Class,
    DataClass,
    DataObject,
    ValueClass,
    EnumClass,
    Interface,
    AnnotationClass,
    Object,
    CompanionObject,
}

/// A member the compiler generates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SynthesizedMember {
    pub name: String,
    pub kind: KotlinMemberKind,
    pub param_count: u16,
    /// Static on the JVM (enum `values` / `valueOf` / `entries`).
    pub is_static: bool,
    pub is_operator: bool,
}

/// Members generated for a classifier (Kotlin specification, "Data class declaration",
/// "Enum class declaration", "Value class declaration"). `primary_properties` are the
/// property-declaring primary-constructor parameters in order; names in `declared` are
/// declared explicitly by the classifier and are not generated.
pub fn synthesized_members(
    flavor: KotlinClassifierFlavor,
    primary_properties: &[&str],
    declared: &[&str],
) -> Vec<SynthesizedMember> {
    struct Synth<'d> {
        declared: &'d [&'d str],
        out: Vec<SynthesizedMember>,
    }

    impl Synth<'_> {
        fn add(&mut self, name: String, kind: KotlinMemberKind, params: u16, is_static: bool, is_operator: bool) {
            if !self.declared.contains(&name.as_str()) {
                self.out.push(SynthesizedMember {
                    name,
                    kind,
                    param_count: params,
                    is_static,
                    is_operator,
                });
            }
        }

        /// `equals` / `hashCode` / `toString` of data classes, data objects and value classes.
        fn object_methods(&mut self) {
            self.add("equals".to_string(), KotlinMemberKind::Function, 1, false, true);
            self.add("hashCode".to_string(), KotlinMemberKind::Function, 0, false, false);
            self.add("toString".to_string(), KotlinMemberKind::Function, 0, false, false);
        }
    }

    let mut synth = Synth {
        declared,
        out: Vec::new(),
    };
    match flavor {
        KotlinClassifierFlavor::DataClass => {
            for i in 0..primary_properties.len() {
                synth.add(format!("component{}", i + 1), KotlinMemberKind::Function, 0, false, true);
            }
            let arity = u16::try_from(primary_properties.len()).unwrap_or(u16::MAX);
            synth.add("copy".to_string(), KotlinMemberKind::Function, arity, false, false);
            synth.object_methods();
        }
        KotlinClassifierFlavor::DataObject | KotlinClassifierFlavor::ValueClass => {
            synth.object_methods();
        }
        KotlinClassifierFlavor::EnumClass => {
            synth.add("values".to_string(), KotlinMemberKind::Function, 0, true, false);
            synth.add("valueOf".to_string(), KotlinMemberKind::Function, 1, true, false);
            synth.add("entries".to_string(), KotlinMemberKind::Property, 0, true, false);
        }
        KotlinClassifierFlavor::Class
        | KotlinClassifierFlavor::Interface
        | KotlinClassifierFlavor::AnnotationClass
        | KotlinClassifierFlavor::Object
        | KotlinClassifierFlavor::CompanionObject => {}
    }
    synth.out
}

// ─────────────────────────────────────────────────────────────────────────────────────────
// JVM interop
// ─────────────────────────────────────────────────────────────────────────────────────────

/// What a Java method looks like from Kotlin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JavaAccessorKind {
    Getter,
    Setter,
}

/// Mappings between Kotlin declarations and their JVM / Java counterparts.
pub struct KotlinJavaInterop;

impl KotlinJavaInterop {
    /// Java class a Kotlin built-in type maps to (`kotlin.String` → `java.lang.String`,
    /// `kotlin.collections.MutableList` → `java.util.List`, `kotlin.Int` → `java.lang.Integer`
    /// when boxed) or aliases on the JVM (`kotlin.collections.ArrayList`).
    pub fn java_peer(kotlin_qual: &str) -> Option<&'static str> {
        type_stub_by_qual_name(kotlin_qual).and_then(|(_, stub)| stub.java_peer)
    }

    /// Kotlin view of a Java class: the mapped built-in type, choosing the mutable collection
    /// interface when `mutable` (Java's `List` is seen as `(Mutable)List`).
    pub fn kotlin_type_for_java(java_qual: &str, mutable: bool) -> Option<String> {
        if let Some(primitive) = Self::kotlin_type_for_java_primitive(java_qual) {
            return Some(primitive.to_string());
        }
        let mut read_only: Option<String> = None;
        let mut fallback: Option<String> = None;
        for stub in KOTLIN_TYPES.iter() {
            if stub.java_peer != Some(java_qual) || stub.flags & KotlinTypeFlags::MAPPED == 0 {
                continue;
            }
            let qual = stub.qual_name();
            if stub.flags & KotlinTypeFlags::MUTABLE_COLLECTION != 0 {
                if mutable {
                    return Some(qual);
                }
            } else if stub.flags & KotlinTypeFlags::READ_ONLY_COLLECTION != 0 {
                if read_only.is_none() {
                    read_only = Some(qual);
                }
            } else if fallback.is_none() {
                fallback = Some(qual);
            }
        }
        read_only.or(fallback)
    }

    /// Kotlin type of a JVM primitive type name (`int` → `kotlin.Int`, `void` → `kotlin.Unit`).
    pub fn kotlin_type_for_java_primitive(primitive: &str) -> Option<&'static str> {
        Some(match primitive {
            "boolean" => "kotlin.Boolean",
            "byte" => "kotlin.Byte",
            "short" => "kotlin.Short",
            "char" => "kotlin.Char",
            "int" => "kotlin.Int",
            "long" => "kotlin.Long",
            "float" => "kotlin.Float",
            "double" => "kotlin.Double",
            "void" => "kotlin.Unit",
            _ => return None,
        })
    }

    /// JVM primitive a non-null Kotlin built-in compiles to (`kotlin.Unit` in return
    /// position is `void`).
    pub fn jvm_primitive(kotlin_qual: &str) -> Option<&'static str> {
        Some(match kotlin_qual {
            "kotlin.Boolean" => "boolean",
            "kotlin.Byte" => "byte",
            "kotlin.Short" => "short",
            "kotlin.Char" => "char",
            "kotlin.Int" => "int",
            "kotlin.Long" => "long",
            "kotlin.Float" => "float",
            "kotlin.Double" => "double",
            "kotlin.Unit" => "void",
            _ => return None,
        })
    }

    /// Symbol id of the Java standard-library stub a Kotlin type maps to, if one exists.
    pub fn java_stub_symbol(kotlin_qual: &str) -> Option<u32> {
        let peer = Self::java_peer(kotlin_qual)?;
        java_stub_ids().get(peer).copied()
    }

    /// JVM getter name of a property (`name` → `getName`, `isOpen` → `isOpen`).
    pub fn getter_name(property: &str) -> String {
        if is_is_prefixed(property) {
            property.to_string()
        } else {
            format!("get{}", capitalize_ascii(property))
        }
    }

    /// JVM setter name of a mutable property (`name` → `setName`, `isOpen` → `setOpen`).
    pub fn setter_name(property: &str) -> String {
        if is_is_prefixed(property) {
            format!("set{}", &property[2..])
        } else {
            format!("set{}", capitalize_ascii(property))
        }
    }

    /// Java methods a Kotlin property access `obj.name` can stand for, getter first.
    pub fn java_getter_candidates(property: &str) -> Vec<String> {
        if is_is_prefixed(property) {
            vec![property.to_string()]
        } else {
            vec![format!("get{}", capitalize_ascii(property))]
        }
    }

    /// Kotlin property a Java accessor method is visible as: `getName()` → `name`,
    /// `isOpen()` → `isOpen`, `setName(v)` → `name`. Other methods are not accessors.
    pub fn property_for_java_accessor(method: &str, param_count: usize) -> Option<(String, JavaAccessorKind)> {
        if param_count == 0 {
            if let Some(rest) = method.strip_prefix("get") {
                if starts_upper(rest) {
                    return Some((decapitalize_ascii(rest), JavaAccessorKind::Getter));
                }
            }
            if is_is_prefixed(method) {
                return Some((method.to_string(), JavaAccessorKind::Getter));
            }
        } else if param_count == 1 {
            if let Some(rest) = method.strip_prefix("set") {
                if starts_upper(rest) {
                    return Some((decapitalize_ascii(rest), JavaAccessorKind::Setter));
                }
            }
        }
        None
    }

    /// JVM class holding a file's top-level declarations: the `@file:JvmName` value, or the
    /// file name without extension with every character that is neither a letter nor a digit
    /// replaced by `_`, prefixed with `_` when it starts with a digit, its first ASCII letter
    /// capitalized, followed by `Kt` (`main.kt` → `MainKt`, `my-utils.kt` → `My_utilsKt`).
    pub fn facade_class_name(file_name: &str, jvm_name: Option<&str>) -> String {
        if let Some(name) = jvm_name {
            return name.to_string();
        }
        let base = file_name.rsplit(['/', '\\']).next().unwrap_or(file_name);
        let stem = base
            .strip_suffix(".kts")
            .or_else(|| base.strip_suffix(".kt"))
            .unwrap_or(base);
        let sanitized: String = stem
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '_' })
            .collect();
        let mut out = String::with_capacity(sanitized.len() + 3);
        if sanitized.is_empty() || sanitized.starts_with(|c: char| c.is_ascii_digit()) {
            out.push('_');
        }
        out.push_str(&capitalize_ascii(&sanitized));
        out.push_str("Kt");
        out
    }

    /// JVM name of a companion object's static holder field and nested class: its name,
    /// or `Companion` when unnamed.
    pub fn companion_jvm_name(declared: Option<&str>) -> &str {
        declared.unwrap_or(DEFAULT_COMPANION_NAME)
    }
}

fn java_stub_ids() -> &'static HashMap<&'static str, u32> {
    static IDS: OnceLock<HashMap<&'static str, u32>> = OnceLock::new();
    IDS.get_or_init(|| {
        get_java_stubs()
            .into_iter()
            .map(|stub| (stub.qual_name, stub.symbol_id))
            .collect()
    })
}

fn is_is_prefixed(name: &str) -> bool {
    name.strip_prefix("is").is_some_and(starts_upper)
}

fn starts_upper(s: &str) -> bool {
    s.chars().next().is_some_and(|c| !c.is_lowercase() && c.is_alphabetic())
}

fn capitalize_ascii(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => {
            let mut out = String::with_capacity(s.len());
            out.push(first.to_ascii_uppercase());
            out.extend(chars);
            out
        }
        None => String::new(),
    }
}

/// `Name` → `name`, `URL` → `url`, `URLConnection` → `urlConnection` (the leading run of
/// capitals is lowered except the capital that starts the next word).
fn decapitalize_ascii(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let upper_run = chars.iter().take_while(|c| c.is_ascii_uppercase()).count();
    let lowered = match upper_run {
        0 => 0,
        1 => 1,
        n if n == chars.len() => n,
        n => n - 1,
    };
    chars
        .iter()
        .enumerate()
        .map(|(i, c)| if i < lowered { c.to_ascii_lowercase() } else { *c })
        .collect()
}
