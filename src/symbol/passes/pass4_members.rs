//! Pass 4 — Member Declaration Type Resolution
//!
//! Every typed declaration (field, variable, parameter, method return type, record component,
//! Kotlin property) carries the token span of its written type, recorded by its front end.
//! The span is parsed as a full type — generic arguments, wildcards, array dimensions,
//! qualified names, varargs, Kotlin nullability — and lowered into the TypeRef table, so
//! `List<Order>`, `Order[]`, `Map.Entry<K, V>` and `java.util.Date` all keep their structure.
//! `type_id` receives the erasure head (JLS 4.6): the head type, the element type of an array,
//! or the leftmost bound of a type variable.
//!
//! `var` (JLS 14.4.1) is inferred from `new T(..)`, casts, literals, a typed variable, or the
//! element type of the iterable of an enhanced `for`; otherwise the TypeRef stays marked
//! INFERRED | UNKNOWN.

use crate::ast::BPASTArtifact;
use crate::core::types::ast::ASTNodeType;
use crate::core::types::symbol::{SymbolKind, THRelation};
use crate::core::types::token::TokenType;
use crate::ingestion::TokenCorpusArtifact;
use crate::symbol::adapter::SemanticAdapter;
use crate::symbol::ast_index::AstIndex;
use crate::symbol::builder::{SymbolTableBuilder, TypeSpan};
use crate::symbol::model::{
    Dialect, ResolutionStatus, SupertypeRecord, SymbolFlags, TypeRef, TypeRefFlags, WildcardKind,
};
use crate::symbol::passes::type_lowering::TypeLowering;
use crate::symbol::scope_graph::NameResolver;
use crate::symbol::std_library::LibraryCategory;
use crate::symbol::syntax::{TokenView, TypeOptions, TypeParser};

pub struct Pass4Members;

impl Pass4Members {
    pub fn run(
        bpa: &BPASTArtifact,
        tca: &TokenCorpusArtifact,
        ast: &AstIndex,
        adapter: &dyn SemanticAdapter,
        builder: &mut SymbolTableBuilder,
    ) {
        let tv = TokenView::new(tca);
        let mut lowering = TypeLowering::new(&tv, adapter);

        // 1. Declared types from their written spans.
        let mut spans: Vec<(u32, TypeSpan)> = builder
            .type_spans
            .iter()
            .map(|(&s, sp)| (s, sp.clone()))
            .collect();
        spans.sort_unstable_by_key(|(s, _)| *s);
        for (sym, span) in spans {
            let scope = Self::resolution_scope(builder, sym);
            let tr = if span.union_parts.is_empty() {
                lowering.lower_span(builder, scope, span.first, span.last, span.dialect, true)
            } else {
                let mut parts = Vec::with_capacity(span.union_parts.len());
                for &(f, l) in &span.union_parts {
                    parts.push(lowering.lower_span(builder, scope, f, l, span.dialect, true));
                }
                builder.add_type_ref(TypeRef {
                    head: u32::MAX,
                    written_name_id: u32::MAX,
                    args: parts,
                    array_dims: 0,
                    wildcard: WildcardKind::None as u8,
                    flags: TypeRefFlags::UNION,
                    status: ResolutionStatus::Unresolved as u8,
                    first_token: span.first,
                    last_token: span.last,
                })
            };
            if span.extra_dims > 0 {
                if let Some(t) = builder.type_refs.get_mut(tr as usize) {
                    t.array_dims = t.array_dims.saturating_add(span.extra_dims);
                }
            }
            Self::assign(builder, sym, tr);
        }

        // 2. Declarations without a span (synthetic trees): fall back to the type reference
        //    child that Phase 2 kept, if any.
        Self::bind_from_type_ref_children(bpa, ast, builder);

        // 3. `var` inference.
        Self::infer_var_types(bpa, &tv, ast, &mut lowering, builder);

        // 4. Type parameter bounds: the leftmost bound is the erasure; all bounds are kept as
        //    SupertypeRecords whose `sub` is the type parameter (no hierarchy edge is added).
        let mut bounds: Vec<(u32, Vec<(u32, u32)>)> = builder
            .type_param_bounds
            .iter()
            .map(|(&s, b)| (s, b.clone()))
            .collect();
        bounds.sort_unstable_by_key(|(s, _)| *s);
        for (tp, list) in bounds {
            let scope = Self::resolution_scope(builder, tp);
            let dialect = match builder.symbol(tp) {
                Some(s) if s.flags & SymbolFlags::LEXICAL != 0 => Dialect::Kotlin,
                _ => Dialect::Java,
            };
            for (i, &(f, l)) in list.iter().enumerate() {
                let tr = lowering.lower_span(builder, scope, f, l, dialect, true);
                let head = builder.type_ref(tr).map(|t| t.head).unwrap_or(u32::MAX);
                if i == 0 {
                    Self::assign(builder, tp, tr);
                }
                if head != u32::MAX {
                    builder.supertypes.push(SupertypeRecord {
                        sub: tp,
                        sup: head,
                        relation: if i == 0 {
                            THRelation::TH_EXTENDS
                        } else {
                            THRelation::TH_IMPLEMENTS
                        },
                        type_ref: tr,
                    });
                }
            }
        }

        // 5. Implicit members share the declared type of their source (record components).
        let mut implicit: Vec<(u32, u32)> = builder
            .implicit_type_sources
            .iter()
            .map(|(&a, &b)| (a, b))
            .collect();
        implicit.sort_unstable();
        for (member, source) in implicit {
            let tr = builder.symbol_type_refs.get(source as usize).copied();
            if let Some(tr) = tr.filter(|&t| t != u32::MAX) {
                Self::assign(builder, member, tr);
            }
        }
    }

    /// Scope in which a declaration's type is resolved: methods and types see their own type
    /// parameters; everything else resolves in the scope it is declared in.
    fn resolution_scope(b: &SymbolTableBuilder, sym: u32) -> u32 {
        match b.symbol(sym) {
            Some(s)
                if matches!(
                    SymbolKind::from(s.kind),
                    SymbolKind::SK_METHOD | SymbolKind::SK_CONSTRUCTOR
                ) && b.owned_scope(sym) != u32::MAX =>
            {
                b.owned_scope(sym)
            }
            Some(s) => s.scope_id,
            None => u32::MAX,
        }
    }

    fn assign(b: &mut SymbolTableBuilder, sym: u32, tr: u32) {
        let (head, dims, generic, union) = match b.type_ref(tr) {
            Some(t) => (
                t.head,
                t.array_dims,
                !t.args.is_empty() && t.flags & TypeRefFlags::UNION == 0,
                t.flags & TypeRefFlags::UNION != 0,
            ),
            None => return,
        };
        b.set_symbol_type_ref(sym, tr);
        // The erasure of a union (lub of the alternatives, JLS 14.20) needs the hierarchy and
        // is left unset.
        if !union {
            b.set_type_id(sym, head);
        }
        if dims > 0 {
            b.add_flags(sym, SymbolFlags::ARRAY);
        }
        if generic {
            b.add_flags(sym, SymbolFlags::GENERIC_TYPE);
        }
    }

    fn bind_from_type_ref_children(bpa: &BPASTArtifact, ast: &AstIndex, b: &mut SymbolTableBuilder) {
        for sym_id in 0..b.symbol_count() as u32 {
            let (kind, decl_node, type_id) = match b.symbol(sym_id) {
                Some(s) => (SymbolKind::from(s.kind), s.decl_node, s.type_id),
                None => continue,
            };
            if type_id != u32::MAX
                || decl_node == u32::MAX
                || b.type_spans.contains_key(&sym_id)
                || !matches!(
                    kind,
                    SymbolKind::SK_FIELD
                        | SymbolKind::SK_PARAM
                        | SymbolKind::SK_LOCAL_VAR
                        | SymbolKind::SK_METHOD
                )
            {
                continue;
            }
            let mut holder = decl_node;
            let parent = ast.parent(decl_node);
            if bpa.node_type(decl_node) == ASTNodeType::NN_LOCAL_VAR_DECL
                && matches!(
                    bpa.node_type(parent),
                    ASTNodeType::NN_FIELD_DECL | ASTNodeType::NN_LOCAL_VAR_DECL
                )
            {
                holder = parent;
            }
            if let Some(tr_node) = ast.first_child_of_type(bpa, holder, ASTNodeType::NN_TYPE_REF) {
                if let Some(t) = b.get_type_ref_resolution(tr_node) {
                    b.set_type_id(sym_id, t);
                }
            }
        }
    }

    fn infer_var_types(
        bpa: &BPASTArtifact,
        tv: &TokenView,
        ast: &AstIndex,
        lowering: &mut TypeLowering,
        b: &mut SymbolTableBuilder,
    ) {
        for sym in 0..b.symbol_count() as u32 {
            let inferred = b
                .symbol_type_ref(sym)
                .map(|t| t.flags & TypeRefFlags::INFERRED != 0 && t.head == u32::MAX)
                .unwrap_or(false);
            if !inferred {
                continue;
            }
            let decl = match b.symbol(sym) {
                Some(s) if s.decl_node != u32::MAX => s.decl_node,
                _ => continue,
            };
            let parent = ast.parent(decl);
            let source = if bpa.node_type(parent) == ASTNodeType::NN_ENHANCED_FOR
                && bpa.node_type(decl) == ASTNodeType::NN_IDENTIFIER_EXPR
            {
                // Element type of the iterable that follows the loop variable.
                let children = ast.children(parent);
                let pos = children.iter().position(|&c| c == decl);
                pos.and_then(|i| children.get(i + 1).copied())
                    .and_then(|iter| Self::static_type_ref(bpa, tv, ast, lowering, b, iter))
                    .and_then(|tr| Self::element_type_ref(b, tr))
            } else {
                // Initializer of the declarator: the child after its name identifier.
                let children = ast.children(decl);
                let init = match children {
                    [first, second, ..]
                        if bpa.node_type(*first) == ASTNodeType::NN_IDENTIFIER_EXPR =>
                    {
                        Some(*second)
                    }
                    _ => None,
                };
                init.and_then(|n| Self::static_type_ref(bpa, tv, ast, lowering, b, n))
            };
            match source {
                Some(tr) => {
                    let mut copy = match b.type_ref(tr) {
                        Some(t) => t.clone(),
                        None => continue,
                    };
                    copy.flags |= TypeRefFlags::INFERRED;
                    copy.flags &= !TypeRefFlags::UNKNOWN;
                    let id = b.add_type_ref(copy);
                    Self::assign(b, sym, id);
                }
                None => {
                    if let Some(id) = b.symbol_type_refs.get(sym as usize).copied() {
                        if let Some(t) = b.type_refs.get_mut(id as usize) {
                            t.flags |= TypeRefFlags::UNKNOWN;
                        }
                    }
                }
            }
        }
    }

    /// TypeRef of the static type of a simple initializer expression, when it is evident from
    /// the expression form.
    fn static_type_ref(
        bpa: &BPASTArtifact,
        tv: &TokenView,
        ast: &AstIndex,
        lowering: &mut TypeLowering,
        b: &mut SymbolTableBuilder,
        node: u32,
    ) -> Option<u32> {
        let scope = b.scope_of_node(node);
        let (ft, _) = bpa.token_range(node);
        match bpa.node_type(node) {
            ASTNodeType::NN_NEW_EXPR => {
                if let Some(anon) = b.symbol_at_node(node) {
                    if b.symbol_kind(anon) == Some(SymbolKind::SK_ANON_CLASS) {
                        return Some(b.add_type_ref(TypeRef {
                            head: anon,
                            written_name_id: u32::MAX,
                            args: Vec::new(),
                            array_dims: 0,
                            wildcard: WildcardKind::None as u8,
                            flags: 0,
                            status: ResolutionStatus::Project as u8,
                            first_token: ft,
                            last_token: ft,
                        }));
                    }
                }
                let new_tok = (ft..tv.len()).find(|&t| tv.word(t, "new"))?;
                let mut parser = TypeParser::new(tv, tv.len(), Dialect::Java);
                let (ty, _) = parser.parse(new_tok + 1, TypeOptions::default())?;
                Some(lowering.lower(b, &ty, scope, false))
            }
            ASTNodeType::NN_CAST_EXPR => {
                if !tv.punct(ft, "(") {
                    return None;
                }
                let close = tv.matching_close(ft, tv.len())?;
                let mut parser = TypeParser::new(tv, close, Dialect::Java);
                let (ty, _) = parser.parse(ft + 1, TypeOptions::default())?;
                Some(lowering.lower(b, &ty, scope, true))
            }
            ASTNodeType::NN_LITERAL => {
                let head = Self::literal_type(tv, ft, b)?;
                let primitive = crate::symbol::adapter::is_primitive_type_id(head);
                Some(b.add_type_ref(TypeRef {
                    head,
                    written_name_id: u32::MAX,
                    args: Vec::new(),
                    array_dims: 0,
                    wildcard: WildcardKind::None as u8,
                    flags: if primitive { TypeRefFlags::PRIMITIVE } else { 0 },
                    status: if primitive {
                        ResolutionStatus::Primitive as u8
                    } else {
                        ResolutionStatus::Library as u8
                    },
                    first_token: ft,
                    last_token: ft,
                }))
            }
            ASTNodeType::NN_IDENTIFIER_EXPR => {
                let name_id = tv.text_id(ft);
                let (var, _) = NameResolver::lookup_lexical_variable(b, scope, name_id, ft)?;
                b.symbol_type_refs
                    .get(var as usize)
                    .copied()
                    .filter(|&t| t != u32::MAX)
            }
            _ => {
                let _ = ast;
                None
            }
        }
    }

    /// Primitive pseudo-id or library symbol of a literal token (JLS 3.10).
    pub(crate) fn literal_type(tv: &TokenView, tok: u32, b: &mut SymbolTableBuilder) -> Option<u32> {
        use crate::symbol::adapter::*;
        let text = tv.text(tok);
        match tv.kind(tok) {
            TokenType::IntegerLiteral => Some(if matches!(text.last(), Some(b'l') | Some(b'L')) {
                PRIMITIVE_LONG_ID
            } else {
                PRIMITIVE_INT_ID
            }),
            TokenType::FloatLiteral => Some(if matches!(text.last(), Some(b'f') | Some(b'F')) {
                PRIMITIVE_FLOAT_ID
            } else {
                PRIMITIVE_DOUBLE_ID
            }),
            TokenType::BooleanLiteral => Some(PRIMITIVE_BOOLEAN_ID),
            TokenType::CharLiteral => Some(PRIMITIVE_CHAR_ID),
            TokenType::StringLiteral => {
                let idx = b.std_lib.lookup_fqn("java.lang.String")?;
                Some(b.materialize_library(idx))
            }
            _ => None,
        }
    }

    /// Element type of an array or of a library collection/iterable TypeRef.
    pub(crate) fn element_type_ref(b: &mut SymbolTableBuilder, tr: u32) -> Option<u32> {
        let t = b.type_ref(tr)?.clone();
        if t.array_dims > 0 {
            let mut elem = t;
            elem.array_dims -= 1;
            elem.flags &= !TypeRefFlags::VARARGS;
            return Some(b.add_type_ref(elem));
        }
        match b.library_category_of(t.head)? {
            LibraryCategory::Collection => t.args.first().copied(),
            _ => None,
        }
    }
}
