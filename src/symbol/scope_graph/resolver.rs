//! Type-name resolution over the scope graph, in the precedence order of JLS 6.4.1 and 7.5
//! (and the equivalent Kotlin rules):
//!
//! 1. local and member types of the enclosing scopes, innermost first, up to and including the
//!    compilation unit (whose scope holds the file's own top-level types);
//! 2. single-type imports (and Kotlin `import ... as` aliases) of the compilation unit;
//! 3. top-level types of the same package, from every compilation unit of that package;
//! 4. type-import-on-demand declarations together with the implicit imports of the language
//!    (`java.lang.*` for Java, the Kotlin default imports for Kotlin).
//!
//! Lookups are kind-directed: only the type namespace is consulted, so fields, parameters,
//! locals, methods and constructors never capture a type name (JLS 6.5).
//!
//! Limitation: member types inherited from supertypes (JLS 8.5) are not consulted in step 1,
//! because the hierarchy is itself built from resolved type names.

use crate::symbol::builder::SymbolTableBuilder;
use crate::symbol::model::Dialect;
use crate::symbol::scope_graph::{ImportKind, Namespace, ScopeFlags};

/// Outcome of resolving a type name, before it is materialized as a symbol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeBinding {
    /// A declaration of the project (type, type parameter) or an already materialized symbol.
    Symbol(u32),
    /// An entry of the built-in library catalog (index into `StdLibManager::types`).
    Library(usize),
    /// A fully qualified name named explicitly by the source but declared outside the project.
    Qualified(String),
    /// `<package>.<name>` where `<package>` is the only on-demand import that could supply it.
    InferredPackage(String),
    /// Several on-demand imports supply the name (a compile error in Java).
    Ambiguous(Vec<String>),
    /// No binding can be computed; `candidates` lists the possible qualified names.
    Unresolved {
        simple: String,
        candidates: Vec<String>,
    },
}

impl TypeBinding {
    /// True when the binding denotes a type (as opposed to a name that may be a package).
    fn is_type(&self) -> bool {
        matches!(
            self,
            TypeBinding::Symbol(_) | TypeBinding::Library(_) | TypeBinding::Qualified(_)
        )
    }
}

pub struct NameResolver;

impl NameResolver {
    /// Resolves the simple type name `name` (interned as `name_id`) used at `at_token` in `scope`.
    pub fn resolve_simple_type(
        b: &SymbolTableBuilder,
        scope: u32,
        name: &str,
        name_id: u32,
        at_token: u32,
    ) -> TypeBinding {
        let sg = &b.scope_graph;

        // 1. Lexical scopes up to the compilation unit.
        let mut cu = None;
        if name_id != u32::MAX {
            for s in sg.ancestors(scope) {
                if sg.has_flag(s, ScopeFlags::PACKAGE) || sg.has_flag(s, ScopeFlags::ROOT) {
                    break;
                }
                if let Some(sym) = sg
                    .lookup_in_scope(s, Namespace::Type, name_id, at_token)
                    .next()
                {
                    return TypeBinding::Symbol(sym);
                }
                if sg.has_flag(s, ScopeFlags::COMPILATION_UNIT) {
                    cu = Some(s);
                    break;
                }
            }
        } else {
            cu = sg.compilation_unit_of(scope);
        }

        let cu = match cu {
            Some(cu) => cu,
            None => {
                // No compilation unit (synthetic AST): only the language's implicit imports apply.
                return Self::resolve_on_demand(b, None, Dialect::Java, name);
            }
        };

        // 2. Single-type imports.
        for imp in sg.imports_of(cu) {
            if imp.kind == ImportKind::SingleType && imp.visible_name == name {
                return Self::bind_qualified_name(b, &imp.path);
            }
        }

        // 3. Same package, across compilation units.
        if name_id != u32::MAX {
            let pkg_scope = sg.parent_of(cu);
            if sg.has_flag(pkg_scope, ScopeFlags::PACKAGE) {
                if let Some(sym) = sg
                    .lookup_in_scope(pkg_scope, Namespace::Type, name_id, u32::MAX)
                    .next()
                {
                    return TypeBinding::Symbol(sym);
                }
            }
        }

        // 4. On-demand and implicit imports.
        Self::resolve_on_demand(b, Some(cu), b.dialect_of_unit(cu), name)
    }

    fn resolve_on_demand(
        b: &SymbolTableBuilder,
        cu: Option<u32>,
        dialect: Dialect,
        name: &str,
    ) -> TypeBinding {
        let mut packages: Vec<(String, bool)> = Vec::new();
        if let Some(cu) = cu {
            for imp in b.scope_graph.imports_of(cu) {
                if imp.kind == ImportKind::OnDemand && !packages.iter().any(|(p, _)| *p == imp.path)
                {
                    packages.push((imp.path.clone(), false));
                }
            }
        }
        for pkg in b.std_lib.implicit_packages(dialect) {
            if !packages.iter().any(|(p, _)| p == pkg) {
                packages.push((pkg.to_string(), true));
            }
        }

        let mut found: Vec<(String, TypeBinding)> = Vec::new();
        let mut open_candidates: Vec<String> = Vec::new();
        for (pkg, implicit) in &packages {
            let fqn = format!("{}.{}", pkg, name);
            if let Some(&sym) = b.types_by_fqn.get(&fqn) {
                found.push((fqn, TypeBinding::Symbol(sym)));
            } else if let Some(idx) = b.std_lib.lookup_fqn(&fqn) {
                found.push((fqn, TypeBinding::Library(idx)));
            } else if !*implicit && !b.is_project_package(pkg) {
                // Neither the project nor the catalog knows this package's full membership.
                open_candidates.push(fqn);
            }
        }

        match found.len() {
            1 => {
                found
                    .pop()
                    .map(|(_, binding)| binding)
                    .unwrap_or_else(|| TypeBinding::Unresolved {
                        simple: name.to_string(),
                        candidates: Vec::new(),
                    })
            }
            0 => {
                if open_candidates.len() == 1 {
                    TypeBinding::InferredPackage(open_candidates.remove(0))
                } else {
                    TypeBinding::Unresolved {
                        simple: name.to_string(),
                        candidates: open_candidates,
                    }
                }
            }
            _ => TypeBinding::Ambiguous(found.into_iter().map(|(fqn, _)| fqn).collect()),
        }
    }

    /// Binds a fully qualified name: project declaration, catalog entry, or an explicit
    /// qualified name outside the project.
    pub fn bind_qualified_name(b: &SymbolTableBuilder, fqn: &str) -> TypeBinding {
        if let Some(&sym) = b.types_by_fqn.get(fqn) {
            return TypeBinding::Symbol(sym);
        }
        if let Some(idx) = b.std_lib.lookup_fqn(fqn) {
            return TypeBinding::Library(idx);
        }
        TypeBinding::Qualified(fqn.to_string())
    }

    /// Resolves a qualified type name `a.b.C` or `Outer.Inner` (JLS 6.5.5.2). The first
    /// segment is a type when one is visible under that name; otherwise the whole name is a
    /// package-qualified name (JLS 6.5.2 reclassification).
    pub fn resolve_qualified_type(
        b: &SymbolTableBuilder,
        scope: u32,
        segments: &[(&str, u32)],
        at_token: u32,
    ) -> TypeBinding {
        let (first_name, first_id) = match segments.first() {
            Some(&s) => s,
            None => {
                return TypeBinding::Unresolved {
                    simple: String::new(),
                    candidates: Vec::new(),
                }
            }
        };
        if segments.len() == 1 {
            return Self::resolve_simple_type(b, scope, first_name, first_id, at_token);
        }
        let full: String = segments
            .iter()
            .map(|(s, _)| *s)
            .collect::<Vec<_>>()
            .join(".");
        let rest: String = segments[1..]
            .iter()
            .map(|(s, _)| *s)
            .collect::<Vec<_>>()
            .join(".");

        let head = Self::resolve_simple_type(b, scope, first_name, first_id, at_token);
        if !head.is_type() {
            return Self::bind_qualified_name(b, &full);
        }
        match head {
            TypeBinding::Symbol(mut sym) => {
                for (i, &(seg_name, seg_id)) in segments[1..].iter().enumerate() {
                    let scope = b.owned_scope(sym);
                    let member = if scope != u32::MAX {
                        b.scope_graph
                            .lookup_in_scope(scope, Namespace::Type, seg_id, u32::MAX)
                            .next()
                    } else {
                        None
                    };
                    match member {
                        Some(m) => sym = m,
                        None => {
                            let remaining: Vec<&str> =
                                segments[1 + i..].iter().map(|(s, _)| *s).collect();
                            return match b.qualified_name_of(sym) {
                                Some(owner) => {
                                    let fqn = format!("{}.{}", owner, remaining.join("."));
                                    Self::bind_qualified_name(b, &fqn)
                                }
                                None => TypeBinding::Unresolved {
                                    simple: seg_name.to_string(),
                                    candidates: Vec::new(),
                                },
                            };
                        }
                    }
                }
                TypeBinding::Symbol(sym)
            }
            TypeBinding::Library(idx) => {
                let fqn = format!("{}.{}", b.std_lib.types[idx].fqn, rest);
                Self::bind_qualified_name(b, &fqn)
            }
            TypeBinding::Qualified(owner) => {
                let fqn = format!("{}.{}", owner, rest);
                Self::bind_qualified_name(b, &fqn)
            }
            _ => Self::bind_qualified_name(b, &full),
        }
    }

    /// Innermost variable (local, parameter, field, enum constant) named `name_id` that is
    /// lexically visible from `at_token` in `scope`. Inherited fields are not considered here.
    pub fn lookup_lexical_variable(
        b: &SymbolTableBuilder,
        scope: u32,
        name_id: u32,
        at_token: u32,
    ) -> Option<(u32, u32)> {
        let sg = &b.scope_graph;
        for s in sg.ancestors(scope) {
            if sg.has_flag(s, ScopeFlags::PACKAGE) || sg.has_flag(s, ScopeFlags::ROOT) {
                return None;
            }
            let mut best: Option<u32> = None;
            for sym in sg.lookup_in_scope(s, Namespace::Variable, name_id, at_token) {
                best = Some(sym);
            }
            if let Some(sym) = best {
                return Some((sym, s));
            }
        }
        None
    }

    /// Static members imported under `name` by single-static or static-on-demand imports of
    /// the compilation unit enclosing `scope`. Returns the qualified names of the owning types.
    pub fn static_import_owners(b: &SymbolTableBuilder, scope: u32, name: &str) -> Vec<String> {
        let sg = &b.scope_graph;
        let cu = match sg.compilation_unit_of(scope) {
            Some(cu) => cu,
            None => return Vec::new(),
        };
        let mut single = Vec::new();
        let mut on_demand = Vec::new();
        for imp in sg.imports_of(cu) {
            match imp.kind {
                ImportKind::SingleStatic if imp.visible_name == name => {
                    if let Some((owner, _)) = imp.path.rsplit_once('.') {
                        single.push(owner.to_string());
                    }
                }
                ImportKind::StaticOnDemand => on_demand.push(imp.path.clone()),
                _ => {}
            }
        }
        if single.is_empty() {
            on_demand
        } else {
            single
        }
    }
}
