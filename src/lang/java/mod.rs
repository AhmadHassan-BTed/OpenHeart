//! Java frontend: the existing tree-sitter-java adapters for Phases 1–3 and the JDK
//! library catalog, presented as one `LanguageFrontend`.
//!
//! `JavaLanguageAdapter` classifies CST leaves (Phase 1), `JavaASTReductionAdapter` maps the
//! CST onto the shared BP AST alphabet (Phase 2), `JavaSemanticAdapter` holds the JLS
//! declaration, access and implicit-modifier rules (Phase 3), and the `java.*` / `javax.*`
//! part of the built-in JVM catalog (`StdLibManager`) becomes the neutral standard-library
//! model. The JVM helpers here (`library_entry_from_catalog`, `jvm_library_entries`,
//! `JVM_REFERENCE_TYPES`) are shared with the Kotlin frontend.

use std::sync::Arc;

use crate::ast::adapter::java::JavaASTReductionAdapter;
use crate::ast::adapter::ASTReductionAdapter;
use crate::core::types::ast::{ASTNodeType, NodeAttr};
use crate::ingestion::adapter::java::JavaLanguageAdapter;
use crate::ingestion::adapter::LanguageAdapter;
use crate::lang::frontend::{
    default_decode_modifiers, matches_type_pattern, Capabilities, Fidelity, HeadScan,
    IdiomHints, LanguageDescriptor, LanguageFrontend, LanguageTraits, LibraryEntry, LibraryRole,
    SniffVerdict, StandardLibrary, StatementTerminator, SyntaxProfile, TypeAnnotationStyle,
};
use crate::lang::neutral::{
    AggregateSemantics, ContainerKind, ControlFlowRules, DeclKind, DispatchPolicy,
    ExceptionModel, GenericsModel, InheritanceModel, MemoryModel, Modifiers, Nullability,
    Ownership, SupertypeRelation, TypeKind, YieldSemantics,
};
use crate::lang::registry::JAVA_LANGUAGE;
use crate::symbol::adapter::java::JavaSemanticAdapter;
use crate::symbol::adapter::SemanticAdapter;
use crate::symbol::model::Dialect;
use crate::symbol::std_library::java_stubs::{LibraryCategory, LibraryType, StdLibManager};

// ── Language facts ───────────────────────────────────────────────────────────

/// Java semantics (JLS 21): single class inheritance with interfaces, virtual instance
/// methods unless `final` / `private` / `static`, garbage-collected references that may be
/// null, erased generics, checked exceptions, fall-through `case :` groups and `yield` as the
/// value of a switch-expression arm.
pub const JAVA_TRAITS: LanguageTraits = LanguageTraits {
    memory: MemoryModel::Managed,
    aggregates: AggregateSemantics::References,
    inheritance: InheritanceModel::SingleWithInterfaces,
    dispatch: DispatchPolicy::VirtualByDefault,
    nullability: Nullability::Implicit,
    generics: GenericsModel::Erased,
    control_flow: ControlFlowRules {
        switch_fall_through: true,
        yield_semantics: YieldSemantics::SwitchValue,
        has_goto: false,
        exceptions: ExceptionModel::Checked,
    },
    raw_pointer_ownership: Ownership::Unknown,
    has_preprocessor: false,
    has_header_files: false,
    has_properties: false,
    has_operator_overloading: false,
    has_free_functions: false,
    case_sensitive: true,
};

/// Java surface syntax for the token-level recognizers.
pub static JAVA_SYNTAX: SyntaxProfile = SyntaxProfile {
    statement_terminator: StatementTerminator::Semicolon,
    qualified_name_separator: ".",
    name_separators: &["."],
    member_access_operators: &["."],
    type_annotation: TypeAnnotationStyle::Prefix,
    supertype_keywords: &[
        ("extends", SupertypeRelation::Extends),
        ("implements", SupertypeRelation::Implements),
    ],
    supertype_colon: false,
    base_clause_modifiers: &[],
    header_terminators: &["{", ";", "permits", "throws"],
    delegation_keyword: None,
    generic_brackets: Some(("<", ">")),
    nullable_suffix: None,
    self_keywords: &["this"],
    super_keywords: &["super"],
    new_keyword: Some("new"),
    non_call_keywords: &[
        "if",
        "for",
        "while",
        "switch",
        "catch",
        "synchronized",
        "return",
        "new",
        "this",
        "super",
        "try",
        "throw",
        "assert",
        "do",
        "else",
        "yield",
    ],
    import_keywords: &["import"],
    container_keywords: &[
        ("package", ContainerKind::Package),
        ("module", ContainerKind::Module),
    ],
    type_keywords: &[
        ("class", TypeKind::Class),
        ("interface", TypeKind::Interface),
        ("enum", TypeKind::Enum),
        ("record", TypeKind::Record),
        ("@interface", TypeKind::Annotation),
    ],
    function_keywords: &[],
    variable_keywords: &["var"],
    annotation_prefix: Some("@"),
    attribute_brackets: None,
    line_comment: Some("//"),
    block_comment: Some(("/*", "*/")),
    doc_comment_prefixes: &["/**"],
};

/// Java naming and JDK conventions for design-pattern detection.
pub static JAVA_IDIOMS: IdiomHints = IdiomHints {
    singleton_accessors: &["getInstance", "instance", "getDefault", "getSingleton"],
    singleton_constants: &["INSTANCE"],
    singleton_type_kinds: &[],
    observer_registration_prefixes: &["add", "register", "attach", "subscribe", "set"],
    observer_role_suffixes: &[
        "Listener",
        "Observer",
        "Subscriber",
        "Callback",
        "Handler",
        "Watcher",
    ],
    events_are_observer_subjects: false,
    iterator_supertypes: &[
        "java.util.Iterator",
        "java.lang.Iterable",
        "java.util.ListIterator",
        "java.util.Spliterator",
    ],
    iterator_methods: &["iterator", "hasNext", "next", "spliterator"],
    disposable_supertypes: &["java.lang.AutoCloseable", "java.io.Closeable"],
    disposal_methods: &["close"],
    clone_methods: &["clone"],
    factory_method_prefixes: &["create", "make", "new", "of", "from", "valueOf", "newInstance"],
    builder_terminal_methods: &["build", "create", "get"],
    function_types: &[
        "java.lang.Runnable",
        "java.util.concurrent.Callable",
        "java.util.Comparator",
        "java.util.function.*",
    ],
    iteration_calls: &["forEach", "forEachOrdered", "forEachRemaining"],
    allocation_functions: &[],
    entry_points: &["main"],
    destructors_release_resources: false,
};

// ── Library model ────────────────────────────────────────────────────────────

/// Packages of the JVM catalog that belong to Java.
pub const JAVA_LIBRARY_PREFIXES: &[&str] = &["java.", "javax."];

/// Roles the catalog's association categories do not express: functional interfaces are
/// callable values, reference holders are transparent wrappers, streams hold many elements.
pub const JAVA_ROLE_OVERRIDES: &[(&str, LibraryRole)] = &[
    ("java.lang.Runnable", LibraryRole::FunctionType),
    ("java.util.concurrent.Callable", LibraryRole::FunctionType),
    ("java.util.Comparator", LibraryRole::FunctionType),
    ("java.util.function.*", LibraryRole::FunctionType),
    ("java.util.concurrent.atomic.AtomicReference", LibraryRole::Transparent),
    ("java.lang.ThreadLocal", LibraryRole::Transparent),
    ("java.lang.InheritableThreadLocal", LibraryRole::Transparent),
    ("java.lang.ScopedValue", LibraryRole::Transparent),
    ("java.util.stream.Stream", LibraryRole::Collection),
];

/// `java.lang.ref` holders, which the catalog does not list: non-owning references that
/// the collector may clear (shared by every JVM language).
pub const JVM_REFERENCE_TYPES: &[LibraryEntry] = &[
    LibraryEntry::new(
        "java.lang.ref.Reference",
        DeclKind::Type(TypeKind::Class),
        1,
        LibraryRole::WeakReference,
        &[],
    ),
    LibraryEntry::new(
        "java.lang.ref.WeakReference",
        DeclKind::Type(TypeKind::Class),
        1,
        LibraryRole::WeakReference,
        &["java.lang.ref.Reference"],
    ),
    LibraryEntry::new(
        "java.lang.ref.SoftReference",
        DeclKind::Type(TypeKind::Class),
        1,
        LibraryRole::WeakReference,
        &["java.lang.ref.Reference"],
    ),
    LibraryEntry::new(
        "java.lang.ref.PhantomReference",
        DeclKind::Type(TypeKind::Class),
        1,
        LibraryRole::WeakReference,
        &["java.lang.ref.Reference"],
    ),
];

/// The neutral library entry of a JVM catalog type.
pub fn library_entry_from_catalog(ty: &LibraryType) -> LibraryEntry {
    let role = match ty.category {
        LibraryCategory::Value => LibraryRole::Value,
        LibraryCategory::Collection => LibraryRole::Collection,
        LibraryCategory::Map => LibraryRole::Map,
        LibraryCategory::Optional => LibraryRole::Optional,
        LibraryCategory::Other => LibraryRole::Other,
    };
    LibraryEntry::new(
        ty.fqn,
        DeclKind::from_symbol_kind(ty.kind),
        ty.type_params,
        role,
        ty.supertypes,
    )
}

/// The catalog types whose qualified name starts with one of `prefixes`.
pub fn jvm_library_entries(catalog: &StdLibManager, prefixes: &[&str]) -> Vec<LibraryEntry> {
    catalog
        .types
        .iter()
        .filter(|t| prefixes.iter().any(|p| t.fqn.starts_with(p)))
        .map(library_entry_from_catalog)
        .collect()
}

/// Replaces the role of every entry matched by an override pattern (qualified name or glob).
pub fn apply_role_overrides(entries: &mut [LibraryEntry], overrides: &[(&str, LibraryRole)]) {
    for entry in entries.iter_mut() {
        if let Some((_, role)) = overrides
            .iter()
            .find(|(pattern, _)| matches_type_pattern(pattern, entry.qualified_name))
        {
            entry.role = *role;
        }
    }
}

/// The JDK model: the `java.*` / `javax.*` catalog with the role refinements above, the
/// `java.lang.ref` holders, the implicit `java.lang` import, `Object` as the root type and
/// `Throwable` as the root of the exception hierarchy.
pub fn java_standard_library() -> StandardLibrary {
    let catalog = StdLibManager::new();
    let mut entries = jvm_library_entries(&catalog, JAVA_LIBRARY_PREFIXES);
    apply_role_overrides(&mut entries, JAVA_ROLE_OVERRIDES);
    entries.extend_from_slice(JVM_REFERENCE_TYPES);
    StandardLibrary::new(entries, ".")
        .with_implicit_imports(catalog.implicit_packages(Dialect::Java).iter().copied())
        .with_root_type("java.lang.Object")
        .with_exception_root("java.lang.Throwable")
}

// ── Content sniffing ─────────────────────────────────────────────────────────

/// `package a.b.c;` at a statement start (Kotlin omits the `;`).
fn has_terminated_package_declaration(scan: &HeadScan) -> bool {
    let count = scan.token_count();
    (0..count).any(|i| {
        if scan.token(i) != Some("package") {
            return false;
        }
        let at_statement_start = i == 0 || matches!(scan.token(i - 1), Some(";" | "}"));
        if !at_statement_start {
            return false;
        }
        let mut j = i + 1;
        loop {
            match scan.token(j) {
                Some(t) if t.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_') => {}
                _ => return false,
            }
            match scan.token(j + 1) {
                Some(".") => j += 2,
                Some(";") => return true,
                _ => return false,
            }
        }
    })
}

fn sniff_java(scan: &HeadScan) -> SniffVerdict {
    let preprocessor = scan.directives().any(|d| {
        let directive = d.trim_start_matches('#').trim_start();
        ["include", "define", "pragma", "ifdef", "ifndef"]
            .iter()
            .any(|k| directive.starts_with(k))
    });
    if preprocessor {
        return SniffVerdict::Impossible;
    }
    if scan.has_sequence(&["@", "interface"]) || scan.has_sequence(&["non", "-", "sealed"]) {
        return SniffVerdict::Definite;
    }
    let other_language = scan.has_word("fun")
        || scan.has_sequence(&["data", "class"])
        || scan.has_sequence(&["companion", "object"])
        || scan.has_word("namespace")
        || scan.has_sequence(&["using", "System"])
        || scan.has_sequence(&["template", "<"]);
    let java_evidence = has_terminated_package_declaration(scan)
        || scan.has_sequence(&["import", "java", "."])
        || scan.has_sequence(&["import", "javax", "."])
        || scan.has_word("implements")
        || scan.has_word("throws")
        || scan.has_sequence(&["public", "static", "void", "main"]);
    match (java_evidence, other_language) {
        (true, false) => SniffVerdict::Likely,
        (_, true) => SniffVerdict::Unlikely,
        (false, false) => SniffVerdict::Neutral,
    }
}

// ── The frontend ─────────────────────────────────────────────────────────────

/// Java as a pluggable frontend.
pub struct JavaFrontend {
    ingestion: Arc<dyn LanguageAdapter>,
    reduction: Arc<dyn ASTReductionAdapter>,
    semantics: Arc<dyn SemanticAdapter>,
    library: StandardLibrary,
}

impl JavaFrontend {
    pub fn new() -> Self {
        Self {
            ingestion: Arc::new(JavaLanguageAdapter::new()),
            reduction: Arc::new(JavaASTReductionAdapter::new()),
            semantics: Arc::new(JavaSemanticAdapter::new()),
            library: java_standard_library(),
        }
    }
}

impl Default for JavaFrontend {
    fn default() -> Self {
        Self::new()
    }
}

impl LanguageFrontend for JavaFrontend {
    fn descriptor(&self) -> &'static LanguageDescriptor {
        &JAVA_LANGUAGE
    }

    fn ingestion_adapter(&self) -> Option<Arc<dyn LanguageAdapter>> {
        Some(Arc::clone(&self.ingestion))
    }

    fn reduction_adapter(&self) -> Option<Arc<dyn ASTReductionAdapter>> {
        Some(Arc::clone(&self.reduction))
    }

    fn semantic_adapter(&self) -> Option<Arc<dyn SemanticAdapter>> {
        Some(Arc::clone(&self.semantics))
    }

    fn standard_library(&self) -> Option<&StandardLibrary> {
        Some(&self.library)
    }

    fn syntax_profile(&self) -> &'static SyntaxProfile {
        &JAVA_SYNTAX
    }

    fn idiom_hints(&self) -> &'static IdiomHints {
        &JAVA_IDIOMS
    }

    /// Every phase is built on the language's own grammar and the JLS rules.
    fn capabilities(&self) -> Capabilities {
        Capabilities::uniform(Fidelity::Structural, JAVA_TRAITS)
    }

    /// The modifier byte plus the Java language flags (`strictfp`, `default`, `non-sealed`,
    /// varargs, implied modifiers, `@Deprecated`), record components, implicit interface
    /// constants, and `final` variables as immutable bindings.
    fn decode_modifiers(&self, node_type: ASTNodeType, attr: u32) -> Modifiers {
        let mut mods = default_decode_modifiers(node_type, attr);
        let Some(kind) = DeclKind::from_ast(node_type) else {
            return mods;
        };
        let lang = NodeAttr::unpack_lang_flags(attr);
        let aux = NodeAttr::unpack_aux_flags(attr);
        let flags = [
            (NodeAttr::LANG_JAVA_STRICTFP, Modifiers::STRICTFP),
            (NodeAttr::LANG_JAVA_DEFAULT, Modifiers::DEFAULT),
            (NodeAttr::LANG_JAVA_NON_SEALED, Modifiers::NON_SEALED),
            (NodeAttr::LANG_JAVA_VARARGS, Modifiers::VARARGS),
            (NodeAttr::LANG_IMPLICIT_MODIFIERS, Modifiers::IMPLICIT),
            (NodeAttr::LANG_DEPRECATED, Modifiers::DEPRECATED),
        ];
        for (bit, flag) in flags {
            if lang & bit != 0 {
                mods.insert(flag);
            }
        }
        if node_type == ASTNodeType::NN_PARAM_DECL
            && aux & NodeAttr::AUX_PARAM_RECORD_COMPONENT != 0
        {
            mods.insert(Modifiers::PRIMARY_CONSTRUCTOR_MEMBER);
        }
        if node_type == ASTNodeType::NN_FIELD_DECL && aux & NodeAttr::AUX_FIELD_CONSTANT != 0 {
            mods.insert(Modifiers::STATIC | Modifiers::FINAL);
        }
        if kind.is_variable() && mods.contains(Modifiers::FINAL) {
            mods.insert(Modifiers::CONST);
        }
        mods
    }

    fn sniff(&self, scan: &HeadScan) -> SniffVerdict {
        sniff_java(scan)
    }
}
