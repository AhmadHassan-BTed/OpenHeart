//! Pass 6 — Reference Resolution
//!
//! Binds the use sites of structurally processed files and records them in the occurrence
//! table, each with a resolution status (resolved, ambiguous with its candidates, or
//! unresolved):
//!
//! * expression names (JLS 6.5.6): locals and parameters in scope (declaration order
//!   honoured), fields of the enclosing types including inherited ones, statically imported
//!   fields; a name that is not a variable is reclassified as a type name (JLS 6.5.2);
//! * field accesses `e.f` against the static type of `e`;
//! * method invocations (JLS 15.12): the type to search is the static type of the receiver,
//!   the innermost enclosing type that has a member of that name, or a static-import owner;
//!   members are searched through the EXTENDS/IMPLEMENTS hierarchy, nearest declaration
//!   first; candidates are filtered by arity (variable arity included) and, when argument
//!   types are known, by applicability (identity, subtyping, primitive widening) with a
//!   most-specific choice among exact matches;
//! * instance creation (constructor by arity, the implicit default constructor otherwise);
//! * method references `X::m` (candidates by name);
//! * annotation names (type references).
//!
//! Static types of expressions are derived from declarations (`type_id`), literals, `this`,
//! `super`, `new`, casts and resolved calls. Library members are not modelled, so calls on
//! library types stay unresolved with that status.

use crate::ast::BPASTArtifact;
use crate::core::types::ast::ASTNodeType;
use crate::core::types::symbol::{SymbolKind, SymbolModifiers, THRelation};
use crate::ingestion::TokenCorpusArtifact;
use crate::symbol::adapter::{is_primitive_type_id, SemanticAdapter, *};
use crate::symbol::ast_index::AstIndex;
use crate::symbol::builder::SymbolTableBuilder;
use crate::symbol::model::{FrontEnd, Occurrence, OccurrenceRole, ResolutionStatus};
use crate::symbol::passes::pass4_members::Pass4Members;
use crate::symbol::passes::type_lowering::TypeLowering;
use crate::symbol::scope_graph::{NameResolver, TypeBinding};
use crate::symbol::syntax::{count_call_arguments, TokenView, TypeOptions, TypeParser};
use crate::symbol::model::Dialect;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExprType {
    /// An expression whose static type has this head (symbol or primitive pseudo-id).
    Value(u32),
    /// A name that denotes a type (static member access).
    TypeName(u32),
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum MemberKind {
    Variable,
    Method,
    Constructor,
}

enum Selection {
    Resolved(u32),
    Ambiguous(Vec<u32>),
    Unresolved,
}

pub struct Pass6References;

impl Pass6References {
    pub fn run(
        bpa: &BPASTArtifact,
        tca: &TokenCorpusArtifact,
        ast: &AstIndex,
        adapter: &dyn SemanticAdapter,
        builder: &mut SymbolTableBuilder,
    ) {
        let tv = TokenView::new(tca);
        let mut members: HashMap<(u32, MemberKind, u32), Vec<u32>> = HashMap::new();
        let mut params: HashMap<u32, Vec<(u32, u32)>> = HashMap::new();
        for s in &builder.symbols {
            if s.parent_sym == u32::MAX {
                continue;
            }
            let kind = match SymbolKind::from(s.kind) {
                SymbolKind::SK_FIELD | SymbolKind::SK_ENUM_CONSTANT => Some(MemberKind::Variable),
                SymbolKind::SK_METHOD => Some(MemberKind::Method),
                SymbolKind::SK_CONSTRUCTOR => Some(MemberKind::Constructor),
                SymbolKind::SK_PARAM => {
                    params
                        .entry(s.parent_sym)
                        .or_default()
                        .push((s.first_token_id, s.symbol_id));
                    None
                }
                _ => None,
            };
            if let Some(k) = kind {
                members
                    .entry((s.parent_sym, k, s.name_id))
                    .or_default()
                    .push(s.symbol_id);
            }
        }
        for list in params.values_mut() {
            list.sort_unstable();
        }

        let mut ctx = Ctx {
            bpa,
            tv,
            ast,
            lowering: TypeLowering::new(&tv, adapter),
            supertypes: builder.supertype_index(),
            members,
            params,
            expr_types: HashMap::new(),
            selections: HashMap::new(),
        };

        let ranges: Vec<(u32, u32)> = builder
            .units
            .iter()
            .filter(|u| u.front_end == FrontEnd::Structural && u.node_range.0 != u32::MAX)
            .map(|u| u.node_range)
            .collect();
        for (start, end) in ranges {
            for node in start..end.min(bpa.node_count) {
                ctx.visit(builder, node);
            }
        }
    }
}

struct Ctx<'a, 'u, 'v> {
    bpa: &'a BPASTArtifact,
    tv: TokenView<'a>,
    ast: &'u AstIndex,
    lowering: TypeLowering<'v, 'a>,
    supertypes: HashMap<u32, Vec<u32>>,
    members: HashMap<(u32, MemberKind, u32), Vec<u32>>,
    params: HashMap<u32, Vec<(u32, u32)>>,
    expr_types: HashMap<u32, ExprType>,
    /// Memoized selections of call/new/field-access nodes: (symbol or MAX, status, candidates).
    selections: HashMap<u32, (u32, ResolutionStatus, Vec<u32>)>,
}

const MAX_DEPTH: u32 = 64;

impl<'a, 'u, 'v> Ctx<'a, 'u, 'v> {
    fn file_limit(&self) -> u32 {
        self.tv.len()
    }

    fn tok(&self, node: u32) -> u32 {
        self.bpa.token_range(node).0
    }

    fn occurrence(
        &self,
        b: &mut SymbolTableBuilder,
        token: u32,
        node: u32,
        role: OccurrenceRole,
        selection: (u32, ResolutionStatus, Vec<u32>),
    ) {
        if !self.tv.valid(token) {
            return;
        }
        b.add_occurrence(Occurrence {
            token,
            node,
            symbol: selection.0,
            role: role as u8,
            status: selection.1 as u8,
            candidates: selection.2,
        });
    }

    fn visit(&mut self, b: &mut SymbolTableBuilder, node: u32) {
        match self.bpa.node_type(node) {
            ASTNodeType::NN_CALL_EXPR => {
                if let Some(name_node) = self.call_name_node(node) {
                    let sel = self.resolve_call(b, node, 0);
                    let t = self.tok(name_node);
                    self.occurrence(b, t, node, OccurrenceRole::Call, sel);
                }
            }
            ASTNodeType::NN_NEW_EXPR => {
                let sel = self.resolve_new(b, node, 0);
                if let Some(t) = self.new_type_token(node) {
                    self.occurrence(b, t, node, OccurrenceRole::Instantiation, sel);
                }
            }
            ASTNodeType::NN_FIELD_ACCESS => {
                if let Some(&member) = self.ast.children(node).last() {
                    if self.bpa.node_type(member) == ASTNodeType::NN_IDENTIFIER_EXPR {
                        let sel = self.resolve_field_access(b, node, 0);
                        let role = self.access_role(node);
                        let t = self.tok(member);
                        self.occurrence(b, t, node, role, sel);
                    }
                }
            }
            ASTNodeType::NN_METHOD_REF => {
                let sel = self.resolve_method_ref(b, node);
                if let Some(&name) = self.ast.children(node).last() {
                    if self.bpa.node_type(name) == ASTNodeType::NN_IDENTIFIER_EXPR {
                        let t = self.tok(name);
                        self.occurrence(b, t, node, OccurrenceRole::Call, sel);
                    }
                }
            }
            ASTNodeType::NN_ANNOTATION_USE => {
                if let Some(name) =
                    self.ast
                        .first_child_of_type(self.bpa, node, ASTNodeType::NN_IDENTIFIER_EXPR)
                {
                    let t = self.tok(name);
                    if self.tv.valid(t) {
                        let scope = b.scope_of_node(node);
                        let (sym, status) = self.lowering.resolve_simple(b, scope, t);
                        self.lowering.record_reference(b, t, name, sym, status);
                    }
                }
            }
            ASTNodeType::NN_IDENTIFIER_EXPR => {
                if self.is_expression_name(b, node) {
                    let t = self.tok(node);
                    match self.resolve_expression_name(b, node) {
                        ExprType::Value(_) | ExprType::Unknown => {
                            let sel = match self.selections.get(&node).cloned() {
                                Some(sel) => sel,
                                // A qualifier that is neither a variable nor a type is a
                                // package name (JLS 6.5.2), not an unresolved variable.
                                None if self.is_qualifier(node) => return,
                                None => (u32::MAX, ResolutionStatus::Unresolved, Vec::new()),
                            };
                            let role = self.access_role(node);
                            self.occurrence(b, t, node, role, sel);
                        }
                        ExprType::TypeName(ty) => {
                            let status = if b.is_project_type(ty) {
                                ResolutionStatus::Project
                            } else {
                                ResolutionStatus::Library
                            };
                            self.lowering.record_reference(b, t, node, ty, status);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    /// True when `node` qualifies a field access or a call (`node.x`, `node.m()`).
    fn is_qualifier(&self, node: u32) -> bool {
        let parent = self.ast.parent(node);
        match self.bpa.node_type(parent) {
            ASTNodeType::NN_FIELD_ACCESS | ASTNodeType::NN_METHOD_REF => {
                self.ast.children(parent).last() != Some(&node)
            }
            ASTNodeType::NN_CALL_EXPR => match self.call_name_node(parent) {
                Some(name) => {
                    let children = self.ast.children(parent);
                    let ni = children.iter().position(|&c| c == name);
                    let mi = children.iter().position(|&c| c == node);
                    matches!((mi, ni), (Some(m), Some(n)) if m < n)
                }
                None => false,
            },
            _ => false,
        }
    }

    /// Read or write role of a name or field access, from its syntactic position.
    fn access_role(&self, node: u32) -> OccurrenceRole {
        let parent = self.ast.parent(node);
        let parent_ty = self.bpa.node_type(parent);
        if parent_ty == ASTNodeType::NN_ASSIGN_EXPR
            && self.ast.children(parent).first() == Some(&node)
        {
            return OccurrenceRole::Write;
        }
        if parent_ty == ASTNodeType::NN_UNARY_EXPR {
            let (ft, lt) = self.bpa.token_range(parent);
            let limit = lt.saturating_add(1);
            let incdec = |t: u32| self.tv.punct(t, "++") || self.tv.punct(t, "--");
            if self.tv.valid(ft) && (incdec(ft) || self.tv.sig(lt, limit).is_some_and(incdec)) {
                return OccurrenceRole::Write;
            }
        }
        OccurrenceRole::Read
    }

    fn is_expression_name(&self, b: &SymbolTableBuilder, node: u32) -> bool {
        if b.symbol_at_node(node).is_some() {
            return false; // declared here (lambda parameter, for-each or pattern variable)
        }
        let parent = self.ast.parent(node);
        let siblings = self.ast.children(parent);
        let first_ident = siblings
            .iter()
            .copied()
            .find(|&c| self.bpa.node_type(c) == ASTNodeType::NN_IDENTIFIER_EXPR);
        match self.bpa.node_type(parent) {
            ASTNodeType::NN_CLASS_DECL
            | ASTNodeType::NN_INTERFACE_DECL
            | ASTNodeType::NN_ENUM_DECL
            | ASTNodeType::NN_RECORD_DECL
            | ASTNodeType::NN_ANNOTATION_DECL
            | ASTNodeType::NN_METHOD_DECL
            | ASTNodeType::NN_CONSTRUCTOR_DECL
            | ASTNodeType::NN_PARAM_DECL
            | ASTNodeType::NN_LOCAL_VAR_DECL
            | ASTNodeType::NN_FIELD_DECL
            | ASTNodeType::NN_ENHANCED_FOR => first_ident != Some(node),
            ASTNodeType::NN_ANNOTATION_USE
            | ASTNodeType::NN_BREAK_STMT
            | ASTNodeType::NN_CONTINUE_STMT
            | ASTNodeType::NN_JAVA_LABELED_STMT => false,
            ASTNodeType::NN_CALL_EXPR => self.call_name_node(parent) != Some(node),
            ASTNodeType::NN_FIELD_ACCESS | ASTNodeType::NN_METHOD_REF => {
                siblings.last() != Some(&node) || siblings.len() < 2
            }
            _ => true,
        }
    }

    // ── Names ────────────────────────────────────────────────────────────────

    fn enclosing_types(&self, b: &SymbolTableBuilder, node: u32) -> Vec<u32> {
        let mut out = Vec::new();
        let mut cur = b.enclosing_symbol_of_node(node);
        let mut steps = 0usize;
        while cur != u32::MAX && steps <= b.symbol_count() {
            if b.is_project_type(cur) {
                out.push(cur);
            }
            cur = b.symbol(cur).map(|s| s.parent_sym).unwrap_or(u32::MAX);
            steps += 1;
        }
        out
    }

    /// Members of `kind` named `name_id` in `ty` or its supertypes, nearest level first.
    fn members_in_hierarchy(&self, ty: u32, kind: MemberKind, name_id: u32) -> Vec<u32> {
        let mut level = vec![ty];
        let mut seen = HashSet::new();
        while !level.is_empty() {
            let mut found = Vec::new();
            let mut next = Vec::new();
            for &t in &level {
                if !seen.insert(t) {
                    continue;
                }
                if let Some(m) = self.members.get(&(t, kind, name_id)) {
                    found.extend_from_slice(m);
                }
                if let Some(sups) = self.supertypes.get(&t) {
                    next.extend_from_slice(sups);
                }
            }
            if !found.is_empty() {
                return found;
            }
            level = next;
        }
        Vec::new()
    }

    fn static_import_types(&self, b: &SymbolTableBuilder, scope: u32, name: &str) -> Vec<u32> {
        NameResolver::static_import_owners(b, scope, name)
            .iter()
            .filter_map(|fqn| b.types_by_fqn.get(fqn).copied())
            .collect()
    }

    /// Classifies a simple expression name and records the selected variable for `node`.
    fn resolve_expression_name(&mut self, b: &mut SymbolTableBuilder, node: u32) -> ExprType {
        if let Some(&t) = self.expr_types.get(&node) {
            return t;
        }
        let tok = self.tok(node);
        if !self.tv.valid(tok) {
            return ExprType::Unknown;
        }
        let name_id = self.tv.text_id(tok);
        let scope = b.scope_of_node(node);

        let mut variable = NameResolver::lookup_lexical_variable(b, scope, name_id, tok).map(|(v, _)| v);
        if variable.is_none() {
            for ty in self.enclosing_types(b, node) {
                if let Some(&f) = self
                    .members_in_hierarchy(ty, MemberKind::Variable, name_id)
                    .first()
                {
                    variable = Some(f);
                    break;
                }
            }
        }
        if variable.is_none() {
            let name = self.tv.text_str(tok);
            for owner in self.static_import_types(b, scope, name) {
                if let Some(&f) = self
                    .members_in_hierarchy(owner, MemberKind::Variable, name_id)
                    .first()
                {
                    variable = Some(f);
                    break;
                }
            }
        }

        let result = if let Some(v) = variable {
            self.selections
                .insert(node, (v, ResolutionStatus::Project, Vec::new()));
            match b.symbol(v).map(|s| s.type_id) {
                Some(t) if t != u32::MAX => ExprType::Value(t),
                _ => ExprType::Unknown,
            }
        } else {
            // JLS 6.5.2: not a variable, so a type name if one is visible, else a package name.
            let name = self.tv.text_str(tok);
            match NameResolver::resolve_simple_type(b, scope, name, name_id, tok) {
                TypeBinding::Symbol(s) => ExprType::TypeName(s),
                TypeBinding::Library(idx) => ExprType::TypeName(b.materialize_library(idx)),
                binding @ TypeBinding::Qualified(_) => {
                    ExprType::TypeName(b.materialize_binding(binding, name_id, tok).0)
                }
                _ => ExprType::Unknown,
            }
        };
        self.expr_types.insert(node, result);
        result
    }

    // ── Expression types ─────────────────────────────────────────────────────

    fn expr_type(&mut self, b: &mut SymbolTableBuilder, node: u32, depth: u32) -> ExprType {
        if depth > MAX_DEPTH {
            return ExprType::Unknown;
        }
        if let Some(&t) = self.expr_types.get(&node) {
            return t;
        }
        let children: Vec<u32> = self.ast.children(node).to_vec();
        let t = match self.bpa.node_type(node) {
            ASTNodeType::NN_IDENTIFIER_EXPR => return self.resolve_expression_name(b, node),
            ASTNodeType::NN_THIS_EXPR => self
                .enclosing_types(b, node)
                .first()
                .map(|&t| ExprType::Value(t))
                .unwrap_or(ExprType::Unknown),
            ASTNodeType::NN_SUPER_EXPR => self
                .enclosing_types(b, node)
                .first()
                .and_then(|&t| self.superclass(b, t))
                .map(ExprType::Value)
                .unwrap_or(ExprType::Unknown),
            ASTNodeType::NN_NEW_EXPR => match b.symbol_at_node(node) {
                Some(anon) if b.symbol_kind(anon) == Some(SymbolKind::SK_ANON_CLASS) => {
                    ExprType::Value(anon)
                }
                _ => self
                    .instantiated_type(b, node)
                    .map(ExprType::Value)
                    .unwrap_or(ExprType::Unknown),
            },
            ASTNodeType::NN_CALL_EXPR => {
                let (m, _, _) = self.resolve_call(b, node, depth + 1);
                match b.symbol(m).map(|s| s.type_id) {
                    Some(t) if t != u32::MAX => ExprType::Value(t),
                    _ => ExprType::Unknown,
                }
            }
            ASTNodeType::NN_FIELD_ACCESS => {
                let (f, status, _) = self.resolve_field_access(b, node, depth + 1);
                if f != u32::MAX && status == ResolutionStatus::Project {
                    match b.symbol(f).map(|s| (SymbolKind::from(s.kind), s.type_id)) {
                        Some((k, _)) if b.is_project_type(f) || k == SymbolKind::SK_EXTERNAL => {
                            ExprType::TypeName(f)
                        }
                        Some((_, t)) if t != u32::MAX => ExprType::Value(t),
                        _ => ExprType::Unknown,
                    }
                } else {
                    ExprType::Unknown
                }
            }
            ASTNodeType::NN_LITERAL => {
                let tok = self.tok(node);
                Pass4Members::literal_type(&self.tv, tok, b)
                    .map(ExprType::Value)
                    .unwrap_or(ExprType::Unknown)
            }
            ASTNodeType::NN_CAST_EXPR | ASTNodeType::NN_ARRAY_CREATE => children
                .iter()
                .find(|&&c| self.bpa.node_type(c) == ASTNodeType::NN_TYPE_REF)
                .and_then(|&c| b.get_type_ref_resolution(c))
                .map(ExprType::Value)
                .unwrap_or(ExprType::Unknown),
            ASTNodeType::NN_ARRAY_ACCESS | ASTNodeType::NN_ASSIGN_EXPR => match children.first() {
                // `type_id` of an array variable is its element type.
                Some(&c) => match self.expr_type(b, c, depth + 1) {
                    ExprType::Value(t) => ExprType::Value(t),
                    _ => ExprType::Unknown,
                },
                None => ExprType::Unknown,
            },
            ASTNodeType::NN_TERNARY_EXPR => match children.get(1) {
                Some(&c) => self.expr_type(b, c, depth + 1),
                None => ExprType::Unknown,
            },
            _ => ExprType::Unknown,
        };
        self.expr_types.insert(node, t);
        t
    }

    fn superclass(&self, b: &SymbolTableBuilder, ty: u32) -> Option<u32> {
        b.th_edges
            .iter()
            .find(|e| e.from_sym == ty && e.relation == THRelation::TH_EXTENDS)
            .map(|e| e.to_sym)
    }

    fn instantiated_type(&mut self, b: &mut SymbolTableBuilder, node: u32) -> Option<u32> {
        if let Some(tr) = self
            .ast
            .first_child_of_type(self.bpa, node, ASTNodeType::NN_TYPE_REF)
            .and_then(|c| b.get_type_ref_resolution(c))
        {
            return Some(tr);
        }
        // Generic or qualified type: Phase 2 dropped it; parse it after `new`.
        let new_tok = self.new_keyword(node)?;
        let mut parser = TypeParser::new(&self.tv, self.file_limit(), Dialect::Java);
        let (ty, _) = parser.parse(new_tok + 1, TypeOptions::default())?;
        let scope = b.scope_of_node(node);
        let tr = self.lowering.lower(b, &ty, scope, false);
        b.type_ref(tr).map(|t| t.head).filter(|&h| h != u32::MAX)
    }

    fn new_keyword(&self, node: u32) -> Option<u32> {
        let (ft, lt) = self.bpa.token_range(node);
        if !self.tv.valid(ft) {
            return None;
        }
        (ft..=lt.min(self.file_limit().saturating_sub(1))).find(|&t| self.tv.word(t, "new"))
    }

    /// Token of the instantiated type's simple name (last segment).
    fn new_type_token(&self, node: u32) -> Option<u32> {
        let new_tok = self.new_keyword(node)?;
        let mut parser = TypeParser::new(&self.tv, self.file_limit(), Dialect::Java);
        let (ty, _) = parser.parse(new_tok + 1, TypeOptions::default())?;
        match &ty.base {
            crate::symbol::syntax::TypeBase::Named(segs) => segs.last().map(|s| s.tok),
            _ => None,
        }
    }

    // ── Calls ────────────────────────────────────────────────────────────────

    /// The method-name identifier of a call: the identifier child followed by `(`.
    fn call_name_node(&self, call: u32) -> Option<u32> {
        let limit = self.file_limit();
        self.ast.children(call).iter().copied().find(|&c| {
            self.bpa.node_type(c) == ASTNodeType::NN_IDENTIFIER_EXPR && {
                let t = self.tok(c);
                self.tv.valid(t) && self.tv.next(t, limit).is_some_and(|n| self.tv.punct(n, "("))
            }
        })
    }

    fn arity_after(&self, name_tok: u32) -> Option<(u16, u32)> {
        let open = self.tv.next(name_tok, self.file_limit())?;
        count_call_arguments(&self.tv, open, self.file_limit())
    }

    fn resolve_call(
        &mut self,
        b: &mut SymbolTableBuilder,
        call: u32,
        depth: u32,
    ) -> (u32, ResolutionStatus, Vec<u32>) {
        if let Some(sel) = self.selections.get(&call) {
            return sel.clone();
        }
        let unresolved = (u32::MAX, ResolutionStatus::Unresolved, Vec::new());
        let children: Vec<u32> = self.ast.children(call).to_vec();
        let name_node = match self.call_name_node(call) {
            Some(n) => n,
            None => return unresolved,
        };
        let name_tok = self.tok(name_node);
        let name_id = self.tv.text_id(name_tok);
        let arity = match self.arity_after(name_tok) {
            Some((a, _)) => a,
            None => return unresolved,
        };
        let name_pos = children.iter().position(|&c| c == name_node).unwrap_or(0);
        let receiver = children[..name_pos]
            .iter()
            .rev()
            .copied()
            .find(|&c| self.bpa.node_type(c) != ASTNodeType::NN_TYPE_REF);

        let mut static_only = false;
        let mut receiver_is_library = false;
        let candidates: Vec<u32> = match receiver {
            None => {
                let mut found = Vec::new();
                for ty in self.enclosing_types(b, call) {
                    found = self.members_in_hierarchy(ty, MemberKind::Method, name_id);
                    if !found.is_empty() {
                        break;
                    }
                }
                if found.is_empty() {
                    let scope = b.scope_of_node(call);
                    let name = self.tv.text_str(name_tok);
                    for owner in self.static_import_types(b, scope, name) {
                        found = self.members_in_hierarchy(owner, MemberKind::Method, name_id);
                        if !found.is_empty() {
                            break;
                        }
                    }
                }
                found
            }
            Some(r) => {
                let rt = self.expr_type(b, r, depth + 1);
                match rt {
                    ExprType::Value(t) | ExprType::TypeName(t) => {
                        static_only = matches!(rt, ExprType::TypeName(_));
                        receiver_is_library = !b.is_project_type(t);
                        if is_primitive_type_id(t) {
                            Vec::new()
                        } else {
                            self.members_in_hierarchy(t, MemberKind::Method, name_id)
                        }
                    }
                    ExprType::Unknown => Vec::new(),
                }
            }
        };

        let mut applicable: Vec<u32> = candidates
            .into_iter()
            .filter(|&m| Self::arity_matches(b, m, arity))
            .collect();
        if static_only {
            let statics: Vec<u32> = applicable
                .iter()
                .copied()
                .filter(|&m| {
                    b.symbol(m)
                        .is_some_and(|s| s.modifiers & SymbolModifiers::STATIC != 0)
                })
                .collect();
            if !statics.is_empty() {
                applicable = statics;
            }
        }
        let args: Vec<u32> = children[name_pos + 1..]
            .iter()
            .copied()
            .filter(|&c| self.bpa.node_type(c) != ASTNodeType::NN_TYPE_REF)
            .collect();
        let selection = match self.select(b, applicable, &args, arity, depth) {
            Selection::Resolved(m) => (m, ResolutionStatus::Project, Vec::new()),
            Selection::Ambiguous(c) => (u32::MAX, ResolutionStatus::Ambiguous, c),
            Selection::Unresolved => (
                u32::MAX,
                if receiver_is_library {
                    ResolutionStatus::Library
                } else {
                    ResolutionStatus::Unresolved
                },
                Vec::new(),
            ),
        };
        self.selections.insert(call, selection.clone());
        selection
    }

    fn arity_matches(b: &SymbolTableBuilder, method: u32, arity: u16) -> bool {
        match b.symbol(method) {
            Some(s) => {
                s.param_count == arity
                    || (s.modifiers & SymbolModifiers::VARARGS != 0
                        && arity.saturating_add(1) >= s.param_count)
            }
            None => false,
        }
    }

    /// Applicability by argument types (JLS 15.12.2, without boxing or inference) and the
    /// most specific choice among exact matches.
    fn select(
        &mut self,
        b: &mut SymbolTableBuilder,
        candidates: Vec<u32>,
        args: &[u32],
        arity: u16,
        depth: u32,
    ) -> Selection {
        match candidates.len() {
            0 => return Selection::Unresolved,
            1 => return Selection::Resolved(candidates[0]),
            _ => {}
        }
        if args.len() != arity as usize {
            return Selection::Ambiguous(candidates);
        }
        let arg_types: Vec<ExprType> = args
            .iter()
            .map(|&a| self.expr_type(b, a, depth + 1))
            .collect();
        let mut scored: Vec<(u32, usize)> = Vec::new();
        for &m in &candidates {
            let params: Vec<u32> = self
                .params
                .get(&m)
                .map(|v| v.iter().map(|&(_, p)| p).collect())
                .unwrap_or_default();
            let mut ok = true;
            let mut exact = 0usize;
            for (i, at) in arg_types.iter().enumerate() {
                let p = match params.get(i) {
                    Some(&p) => p,
                    None => continue,
                };
                let pt = b.symbol(p).map(|s| s.type_id).unwrap_or(u32::MAX);
                let a = match at {
                    ExprType::Value(t) => *t,
                    _ => continue,
                };
                if pt == u32::MAX {
                    continue;
                }
                if a == pt {
                    exact += 1;
                } else if !self.assignable(a, pt) {
                    ok = false;
                    break;
                }
            }
            if ok {
                scored.push((m, exact));
            }
        }
        match scored.len() {
            0 => Selection::Ambiguous(candidates),
            1 => Selection::Resolved(scored[0].0),
            _ => {
                let best = scored.iter().map(|&(_, e)| e).max().unwrap_or(0);
                let top: Vec<u32> = scored
                    .iter()
                    .filter(|&&(_, e)| e == best)
                    .map(|&(m, _)| m)
                    .collect();
                if top.len() == 1 {
                    Selection::Resolved(top[0])
                } else {
                    Selection::Ambiguous(top)
                }
            }
        }
    }

    fn assignable(&self, from: u32, to: u32) -> bool {
        if is_primitive_type_id(from) || is_primitive_type_id(to) {
            return Self::primitive_widening(from, to);
        }
        let mut seen = HashSet::new();
        let mut stack = vec![from];
        while let Some(t) = stack.pop() {
            if t == to {
                return true;
            }
            if !seen.insert(t) {
                continue;
            }
            if let Some(sups) = self.supertypes.get(&t) {
                stack.extend_from_slice(sups);
            }
        }
        false
    }

    /// JLS 5.1.2 widening primitive conversions (and identity).
    fn primitive_widening(from: u32, to: u32) -> bool {
        if from == to {
            return true;
        }
        let rank = |id: u32| -> Option<u8> {
            Some(match id {
                PRIMITIVE_BYTE_ID => 1,
                PRIMITIVE_SHORT_ID => 2,
                PRIMITIVE_CHAR_ID => 2,
                PRIMITIVE_INT_ID => 3,
                PRIMITIVE_LONG_ID => 4,
                PRIMITIVE_FLOAT_ID => 5,
                PRIMITIVE_DOUBLE_ID => 6,
                _ => return None,
            })
        };
        match (rank(from), rank(to)) {
            // char does not widen to short, and nothing widens to char.
            (Some(_), _) if to == PRIMITIVE_CHAR_ID => false,
            (Some(_), Some(_)) if from == PRIMITIVE_CHAR_ID && to == PRIMITIVE_SHORT_ID => false,
            (Some(_), Some(_)) if from == PRIMITIVE_BYTE_ID && to == PRIMITIVE_CHAR_ID => false,
            (Some(f), Some(t)) => f < t,
            _ => false,
        }
    }

    // ── Instance creation, field access, method references ───────────────────

    fn resolve_new(
        &mut self,
        b: &mut SymbolTableBuilder,
        node: u32,
        depth: u32,
    ) -> (u32, ResolutionStatus, Vec<u32>) {
        if let Some(sel) = self.selections.get(&node) {
            return sel.clone();
        }
        let ty = match self.instantiated_type(b, node) {
            Some(t) => t,
            None => return (u32::MAX, ResolutionStatus::Unresolved, Vec::new()),
        };
        let selection = if !b.is_project_type(ty) {
            (ty, ResolutionStatus::Library, Vec::new())
        } else {
            let name_id = b.symbol(ty).map(|s| s.name_id).unwrap_or(u32::MAX);
            let arity = self
                .new_type_token(node)
                .and_then(|t| {
                    // Skip type arguments (`new Box<T>(..)`) to reach the argument list.
                    let mut pos = t + 1;
                    if let Some(lt) = self.tv.sig(pos, self.file_limit()) {
                        if self.tv.punct(lt, "<") {
                            let mut parser =
                                TypeParser::new(&self.tv, self.file_limit(), Dialect::Java);
                            let (_, after) = parser.parse(t, TypeOptions::default())?;
                            pos = after;
                        }
                    }
                    let open = self.tv.sig(pos, self.file_limit())?;
                    count_call_arguments(&self.tv, open, self.file_limit())
                })
                .map(|(a, _)| a);
            let ctors: Vec<u32> = self
                .members
                .get(&(ty, MemberKind::Constructor, name_id))
                .cloned()
                .unwrap_or_default();
            match arity {
                Some(arity) if ctors.is_empty() => {
                    if arity == 0 {
                        // Implicit default constructor: the creation binds to the class.
                        (ty, ResolutionStatus::Project, Vec::new())
                    } else {
                        (u32::MAX, ResolutionStatus::Unresolved, Vec::new())
                    }
                }
                Some(arity) => {
                    let applicable: Vec<u32> = ctors
                        .into_iter()
                        .filter(|&c| Self::arity_matches(b, c, arity))
                        .collect();
                    let args: Vec<u32> = self
                        .ast
                        .children(node)
                        .iter()
                        .copied()
                        .filter(|&c| {
                            !matches!(
                                self.bpa.node_type(c),
                                ASTNodeType::NN_TYPE_REF
                                    | ASTNodeType::NN_METHOD_DECL
                                    | ASTNodeType::NN_FIELD_DECL
                                    | ASTNodeType::NN_BLOCK
                                    | ASTNodeType::NN_CLASS_DECL
                            )
                        })
                        .collect();
                    match self.select(b, applicable, &args, arity, depth) {
                        Selection::Resolved(c) => (c, ResolutionStatus::Project, Vec::new()),
                        Selection::Ambiguous(c) => (u32::MAX, ResolutionStatus::Ambiguous, c),
                        Selection::Unresolved => {
                            (u32::MAX, ResolutionStatus::Unresolved, Vec::new())
                        }
                    }
                }
                None => (u32::MAX, ResolutionStatus::Unresolved, Vec::new()),
            }
        };
        self.selections.insert(node, selection.clone());
        selection
    }

    fn resolve_field_access(
        &mut self,
        b: &mut SymbolTableBuilder,
        node: u32,
        depth: u32,
    ) -> (u32, ResolutionStatus, Vec<u32>) {
        if let Some(sel) = self.selections.get(&node) {
            return sel.clone();
        }
        let unresolved = (u32::MAX, ResolutionStatus::Unresolved, Vec::new());
        let children: Vec<u32> = self.ast.children(node).to_vec();
        if children.len() < 2 {
            return unresolved;
        }
        let member = children[children.len() - 1];
        let receiver = children[children.len() - 2];
        let member_tok = self.tok(member);
        if !self.tv.valid(member_tok) {
            return unresolved;
        }
        let name_id = self.tv.text_id(member_tok);
        let selection = match self.expr_type(b, receiver, depth + 1) {
            ExprType::Value(t) | ExprType::TypeName(t) if !is_primitive_type_id(t) => {
                match self
                    .members_in_hierarchy(t, MemberKind::Variable, name_id)
                    .first()
                {
                    Some(&f) => (f, ResolutionStatus::Project, Vec::new()),
                    None => {
                        // `Outer.Inner` used as a qualifier: a member type.
                        let scope = b.owned_scope(t);
                        let member_type = if scope != u32::MAX {
                            b.scope_graph
                                .lookup_in_scope(
                                    scope,
                                    crate::symbol::scope_graph::Namespace::Type,
                                    name_id,
                                    u32::MAX,
                                )
                                .next()
                        } else {
                            None
                        };
                        match member_type {
                            Some(mt) => (mt, ResolutionStatus::Project, Vec::new()),
                            None if !b.is_project_type(t) => {
                                (u32::MAX, ResolutionStatus::Library, Vec::new())
                            }
                            None => unresolved,
                        }
                    }
                }
            }
            _ => unresolved,
        };
        self.selections.insert(node, selection.clone());
        selection
    }

    fn resolve_method_ref(
        &mut self,
        b: &mut SymbolTableBuilder,
        node: u32,
    ) -> (u32, ResolutionStatus, Vec<u32>) {
        let unresolved = (u32::MAX, ResolutionStatus::Unresolved, Vec::new());
        let children: Vec<u32> = self.ast.children(node).to_vec();
        let name = match children.last() {
            Some(&n) if self.bpa.node_type(n) == ASTNodeType::NN_IDENTIFIER_EXPR && children.len() >= 2 => n,
            _ => return unresolved,
        };
        let name_id = self.tv.text_id(self.tok(name));
        let qualifier = children[children.len() - 2];
        let owner = if self.bpa.node_type(qualifier) == ASTNodeType::NN_TYPE_REF {
            b.get_type_ref_resolution(qualifier)
        } else {
            match self.expr_type(b, qualifier, 1) {
                ExprType::Value(t) | ExprType::TypeName(t) => Some(t),
                ExprType::Unknown => None,
            }
        };
        match owner {
            Some(t) if !is_primitive_type_id(t) => {
                let found = self.members_in_hierarchy(t, MemberKind::Method, name_id);
                match found.len() {
                    0 if !b.is_project_type(t) => (u32::MAX, ResolutionStatus::Library, Vec::new()),
                    0 => unresolved,
                    1 => (found[0], ResolutionStatus::Project, Vec::new()),
                    // The functional interface type decides among overloads (JLS 15.13.1).
                    _ => (u32::MAX, ResolutionStatus::Ambiguous, found),
                }
            }
            _ => unresolved,
        }
    }
}
