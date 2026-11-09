//! Declared types: resolution of type names in scope (type parameters, member types,
//! single-type imports, the compilation unit, the package, on-demand imports) and lowering
//! of written type references to interned IR types.

use crate::core::types::symbol::SymbolKind;
use crate::ir::lower::syntax::NodeClass;
use crate::ir::lower::Lowerer;
use crate::ir::module::DiagnosticKind;
use crate::ir::types::{IrType, TypeId, TypeTable};

/// Scope in which a type name is resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct TypeCtx {
    pub file: Option<u16>,
    pub class_symbol: Option<u32>,
    pub function_symbol: Option<u32>,
}

impl<'t> Lowerer<'t> {
    pub(super) fn type_ctx_of_frame(&self, f: usize) -> TypeCtx {
        let st = &self.frames[f];
        TypeCtx {
            file: st.file,
            class_symbol: st.class_symbol,
            function_symbol: st.function_symbol,
        }
    }

    /// Scope of a node: its file and the nearest enclosing type and function declarations.
    pub(super) fn type_ctx_of_node(&self, node: u32) -> TypeCtx {
        let syn = self.syn_for(node);
        let mut class_symbol = None;
        let mut function_symbol = None;
        let mut cur = self.tree.parent(node);
        while let Some(p) = cur {
            let cl = syn.class(p);
            if function_symbol.is_none() && class_symbol.is_none() && (cl.is_function_like() || cl == NodeClass::Lambda) {
                function_symbol = self.index.at_node(p);
            }
            if class_symbol.is_none() && (cl == NodeClass::TypeDecl || cl == NodeClass::AnonBody) {
                class_symbol = self.index.at_node(p).or_else(|| {
                    // An anonymous body without a symbol of its own: the creation node may
                    // carry it.
                    self.tree.parent(p).and_then(|q| self.index.at_node(q))
                });
                if class_symbol.is_some() {
                    break;
                }
            }
            cur = self.tree.parent(p);
        }
        TypeCtx {
            file: self.tree.node_file(node),
            class_symbol,
            function_symbol,
        }
    }

    /// Type symbol (or type-parameter symbol) a simple type name denotes in `ctx`.
    pub(super) fn resolve_type_name(&mut self, name: u32, ctx: &TypeCtx) -> Option<u32> {
        if name == u32::MAX {
            return None;
        }
        // Type parameters of the enclosing function, then of the enclosing types.
        if let Some(f) = ctx.function_symbol {
            if let Some(p) = self.index.type_param_named(f, name) {
                return Some(p);
            }
        }
        let mut chain: Vec<u32> = Vec::new();
        let mut cur = ctx.class_symbol;
        while let Some(c) = cur {
            if chain.contains(&c) {
                break;
            }
            chain.push(c);
            cur = self.index.enclosing_type(c);
        }
        for &c in &chain {
            if let Some(p) = self.index.type_param_named(c, name) {
                return Some(p);
            }
            if self.index.name(c) == name {
                return Some(c);
            }
            if let Some(m) = self.index.member_type_in_hierarchy(c, name) {
                return Some(m);
            }
        }

        let file = ctx.file?;
        let text = self.module.strings.lookup_text(name).to_vec();
        let imports = self.imports.get(&file).cloned().unwrap_or_default();

        if let Some(path) = imports.single.get(&text) {
            if let Some(t) = self.index.qualified_type(path) {
                return Some(t);
            }
            // The import names a type outside the project: the binding is known to be
            // external, so no project type of the same simple name may be chosen.
            return None;
        }
        let local = self.index.top_level_types(file, name);
        if local.len() == 1 {
            return Some(local[0]);
        }
        if let Some(pkg) = self.index.package_of_file(file) {
            let same = self.index.package_types(pkg, name);
            if same.len() == 1 {
                return Some(same[0]);
            }
        }
        let mut found: Vec<u32> = Vec::new();
        for pkg in &imports.on_demand {
            for &t in self.index.package_types(pkg, name) {
                if !found.contains(&t) {
                    found.push(t);
                }
            }
            let nested = format!("{}.{}", pkg, String::from_utf8_lossy(&text));
            if let Some(t) = self.index.qualified_type(&nested) {
                if !found.contains(&t) {
                    found.push(t);
                }
            }
        }
        if found.len() == 1 {
            return Some(found[0]);
        }
        None
    }

    /// True when a type reference requests inference from the initializer.
    pub(super) fn is_inferred_type_ref(&self, node: u32) -> bool {
        let sem = self.sem_of_node(node);
        let text = self.tree.text(node);
        !text.is_empty() && sem.infers_type(text)
    }

    /// IR type of a written type reference.
    pub(super) fn type_of_ref(&mut self, node: u32) -> TypeId {
        if let Some(&t) = self.type_of_node.get(&node) {
            return t;
        }
        let ctx = self.type_ctx_of_node(node);
        let t = self.type_of_ref_in(node, &ctx);
        self.type_of_node.insert(node, t);
        t
    }

    fn type_of_ref_in(&mut self, node: u32, ctx: &TypeCtx) -> TypeId {
        let syn = self.syn_for(node);
        let sem = syn.sem;
        let parts = syn.type_ref_parts(node);
        if parts.is_wildcard {
            return match parts.args.first() {
                Some(&bound) => self.type_of_ref_in(bound, ctx),
                None => TypeTable::UNKNOWN,
            };
        }
        if self.is_inferred_type_ref(node) {
            return TypeTable::UNKNOWN;
        }

        let mut base = TypeTable::UNKNOWN;
        let mut resolved = false;
        match parts.segments.len() {
            0 => {
                if let Some(t) = parts.head_token {
                    if let Some(p) = sem.primitive_type(self.tree.token_text(t)) {
                        base = self.module.types.intern(p);
                        resolved = true;
                    }
                }
            }
            1 => {
                let seg = parts.segments[0];
                let name = self.tree.token_text_id(seg).unwrap_or(u32::MAX);
                if let Some(sym) = self.resolve_type_name(name, ctx) {
                    base = self.type_of_type_symbol(sym);
                    resolved = true;
                } else if let Some(p) = sem.primitive_type(self.tree.token_text(seg)) {
                    base = self.module.types.intern(p);
                    resolved = true;
                }
            }
            _ => {
                // The symbol table names qualified types with '.'-separated segments.
                let path = parts
                    .segments
                    .iter()
                    .map(|&t| String::from_utf8_lossy(self.tree.token_text(t)).into_owned())
                    .collect::<Vec<_>>()
                    .join(".");
                if let Some(sym) = self.index.qualified_type(&path) {
                    base = self.module.types.named(sym);
                    resolved = true;
                } else {
                    let first = self.tree.token_text_id(parts.segments[0]).unwrap_or(u32::MAX);
                    let mut cur = self.resolve_type_name(first, ctx);
                    for &seg in &parts.segments[1..] {
                        let name = self.tree.token_text_id(seg).unwrap_or(u32::MAX);
                        cur = cur.and_then(|c| self.index.member_type_in_hierarchy(c, name));
                    }
                    if let Some(sym) = cur {
                        base = self.module.types.named(sym);
                        resolved = true;
                    }
                }
            }
        }
        if !resolved {
            let written = match self.tree.token_range(node) {
                Some((ft, lt)) => String::from_utf8_lossy(&self.tree.joined_text(ft, lt)).into_owned(),
                None => String::new(),
            };
            let span = self.tree.span(node);
            self.diag_once(
                DiagnosticKind::Unresolved,
                span,
                format!("type '{}' is not declared in the project or by the language frontend", written),
            );
        }

        if !parts.args.is_empty() {
            let args: Vec<TypeId> = parts
                .args
                .iter()
                .map(|&a| self.type_of_ref_in(a, ctx))
                .collect();
            base = self.module.types.generic(base, args);
        }
        for _ in 0..parts.dims {
            base = self.module.types.array_of(base, None);
        }
        base
    }

    /// IR type denoted by a type or type-parameter symbol.
    pub(super) fn type_of_type_symbol(&mut self, sym: u32) -> TypeId {
        if self.index.kind(sym) == Some(SymbolKind::SK_TYPE_PARAM) {
            self.module.types.intern(IrType::TypeParam(sym))
        } else {
            self.module.types.named(sym)
        }
    }

    /// Declared type of a variable, field, parameter or method (its result type).
    pub(super) fn symbol_type(&mut self, sym: u32) -> TypeId {
        if let Some(&t) = self.type_of_symbol.get(&sym) {
            return t;
        }
        // Recursion guard for malformed declarations that refer to themselves.
        self.type_of_symbol.insert(sym, TypeTable::UNKNOWN);
        let kind = self.index.kind(sym);
        let decl = self.index.symbol(sym).map(|r| r.decl_node).unwrap_or(u32::MAX);
        let valid_decl = decl != u32::MAX && decl < self.tree.len();
        let mut ty = TypeTable::UNKNOWN;
        match kind {
            Some(SymbolKind::SK_METHOD) if valid_decl => {
                let parts = self.syn_for(decl).function_parts(decl);
                if let Some(r) = parts.ret_type {
                    ty = self.type_of_ref(r);
                }
            }
            Some(SymbolKind::SK_CONSTRUCTOR)
            | Some(SymbolKind::SK_STATIC_INIT)
            | Some(SymbolKind::SK_INSTANCE_INIT) => ty = TypeTable::VOID,
            Some(SymbolKind::SK_FIELD) | Some(SymbolKind::SK_LOCAL_VAR) if valid_decl => {
                ty = self.declarator_type(decl);
            }
            Some(SymbolKind::SK_PARAM) if valid_decl => {
                let parts = self.syn_for(decl).param_parts(decl);
                if let Some(&r) = parts.type_refs.first() {
                    ty = self.type_of_ref(r);
                }
            }
            Some(SymbolKind::SK_ENUM_CONSTANT) => {
                if let Some(e) = self.index.enclosing_type(sym) {
                    ty = self.module.types.named(e);
                }
            }
            Some(k) if crate::ir::lower::resolve::is_type_kind(k) => {
                ty = self.module.types.named(sym);
            }
            Some(SymbolKind::SK_TYPE_PARAM) => {
                ty = self.module.types.intern(IrType::TypeParam(sym));
            }
            _ => {}
        }
        if ty == TypeTable::UNKNOWN {
            if let Some(t) = self.index.declared_type_symbol(sym) {
                ty = self.type_of_type_symbol(t);
            }
        }
        self.type_of_symbol.insert(sym, ty);
        ty
    }

    /// Result type of a method symbol.
    pub(super) fn return_type(&mut self, method: u32) -> TypeId {
        if let Some(&t) = self.ret_of_symbol.get(&method) {
            return t;
        }
        let t = self.symbol_type(method);
        self.ret_of_symbol.insert(method, t);
        t
    }

    /// Declared type of a declarator (or of a declaration with a single declarator).
    pub(super) fn declarator_type(&mut self, d: u32) -> TypeId {
        let syn = self.syn_for(d);
        let own = match syn.class(d) {
            NodeClass::EnumConstant => {
                let ctx = self.type_ctx_of_node(d);
                return match ctx.class_symbol {
                    Some(c) => self.module.types.named(c),
                    None => TypeTable::UNKNOWN,
                };
            }
            NodeClass::Field => syn.local_decl_parts(d).type_ref,
            NodeClass::LocalDecl => syn
                .local_decl_parts(d)
                .type_ref
                .or_else(|| syn.declarator_parts(d).type_ref),
            _ => syn.declarator_parts(d).type_ref,
        };
        let type_ref = own.or_else(|| {
            self.tree.parent(d).and_then(|p| match syn.class(p) {
                NodeClass::Field | NodeClass::LocalDecl | NodeClass::Resources => {
                    syn.local_decl_parts(p).type_ref
                }
                _ => None,
            })
        });
        match type_ref {
            Some(r) => self.type_of_ref(r),
            None => TypeTable::UNKNOWN,
        }
    }
}
