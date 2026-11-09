//! Pass 3 — Type Reference Resolution
//!
//! Binds every NN_TYPE_REF node of a structurally processed file. The written type starting at
//! the node's first token is parsed (so qualified names such as `java.util.Date` or
//! `Map.Entry`, parameterized types and wildcards are recognized whether the tree keeps them
//! as one node or nests them), and the node is bound to the erasure head of that type:
//! primitive keywords to their pseudo-ids, `var` to a type-inference marker (JLS 14.4.1),
//! wildcards to their bound (`?` alone to `java.lang.Object`, JLS 4.5.1).
//!
//! Names are resolved through the scope graph in JLS 6.4.1/7.5 order (local and member types,
//! single-type imports, same package across files, on-demand and implicit imports) and only in
//! the type namespace, so constructors, fields, parameters and locals never capture a type
//! name. Names that cannot be bound become externals keyed by qualified name, or explicitly
//! unresolved externals with their candidate packages — never bare simple-name externals.

use crate::ast::BPASTArtifact;
use crate::core::types::ast::ASTNodeType;
use crate::core::types::token::TokenType;
use crate::ingestion::TokenCorpusArtifact;
use crate::symbol::adapter::SemanticAdapter;
use crate::symbol::builder::SymbolTableBuilder;
use crate::symbol::model::{Dialect, FrontEnd, ResolutionStatus};
use crate::symbol::passes::type_lowering::TypeLowering;
use crate::symbol::syntax::{
    is_java_primitive, TokenView, TypeBase, TypeOptions, TypeParser, TypeSyntax, WildcardSyntax,
};

pub struct Pass3Types;

impl Pass3Types {
    pub fn run(
        bpa: &BPASTArtifact,
        tca: &TokenCorpusArtifact,
        adapter: &dyn SemanticAdapter,
        builder: &mut SymbolTableBuilder,
    ) {
        let tv = TokenView::new(tca);
        let mut lowering = TypeLowering::new(&tv, adapter);
        let units: Vec<((u32, u32), u32)> = builder
            .units
            .iter()
            .filter(|u| u.front_end == FrontEnd::Structural && u.node_range.0 != u32::MAX)
            .map(|u| {
                let file_end = if u.token_range.1 == u32::MAX {
                    tv.len()
                } else {
                    u.token_range.1
                };
                (u.node_range, file_end)
            })
            .collect();

        for ((start, end), file_end) in units {
            for node in start..end.min(bpa.node_count) {
                if bpa.node_type(node) != ASTNodeType::NN_TYPE_REF {
                    continue;
                }
                let (ft, _) = bpa.token_range(node);
                if !tv.valid(ft) {
                    builder.set_type_ref_resolution(node, u32::MAX, ResolutionStatus::Unresolved);
                    continue;
                }
                let scope = builder.scope_of_node(node);
                let mut parser = TypeParser::new(&tv, file_end, Dialect::Java);
                let opts = TypeOptions {
                    allow_varargs: false,
                    allow_var: true,
                };
                // A wildcard type argument node starts at its `?`.
                let start = if tv.punct(ft, "?") {
                    match tv.next(ft, file_end) {
                        Some(kw) if tv.word(kw, "extends") || tv.word(kw, "super") => Some(kw + 1),
                        _ => None,
                    }
                } else {
                    Some(ft)
                };
                let (sym, status) = match start {
                    None => Self::bind(
                        &tv,
                        adapter,
                        &mut lowering,
                        builder,
                        scope,
                        &TypeSyntax {
                            base: TypeBase::Wildcard(WildcardSyntax::Unbounded),
                            dims: 0,
                            varargs: false,
                            nullable: false,
                            annotations: Vec::new(),
                            first: ft,
                            last: ft,
                        },
                        node,
                    ),
                    Some(s) => match parser.parse(s, opts) {
                        Some((ty, _)) => {
                            Self::bind(&tv, adapter, &mut lowering, builder, scope, &ty, node)
                        }
                        None => Self::bind_first_token(
                            &tv,
                            adapter,
                            &mut lowering,
                            builder,
                            scope,
                            ft,
                            node,
                        ),
                    },
                };
                builder.set_type_ref_resolution(node, sym, status);
            }
        }
    }

    /// Binding of a parsed type: the erasure head of its outermost form.
    fn bind(
        tv: &TokenView,
        adapter: &dyn SemanticAdapter,
        lowering: &mut TypeLowering,
        b: &mut SymbolTableBuilder,
        scope: u32,
        ty: &TypeSyntax,
        node: u32,
    ) -> (u32, ResolutionStatus) {
        match &ty.base {
            TypeBase::Primitive(t) => match adapter.primitive_type_id(tv.text_str(*t)) {
                Some(id) => (id, ResolutionStatus::Primitive),
                None => (u32::MAX, ResolutionStatus::Unresolved),
            },
            TypeBase::Inferred(_) => (u32::MAX, ResolutionStatus::TypeInference),
            TypeBase::Named(segments) => {
                let toks: Vec<u32> = segments.iter().map(|s| s.tok).collect();
                if toks.len() == 1 && tv.word(toks[0], "var") {
                    // `var` is a reserved type name, not a type (JLS 3.9, 14.4.1).
                    return (u32::MAX, ResolutionStatus::TypeInference);
                }
                let (sym, status) = lowering.resolve_name(b, scope, &toks);
                if let Some(&last) = toks.last() {
                    lowering.record_reference(b, last, node, sym, status);
                }
                (sym, status)
            }
            TypeBase::Wildcard(WildcardSyntax::Unbounded) => match b.std_lib.lookup_fqn("java.lang.Object") {
                Some(idx) => (b.materialize_library(idx), ResolutionStatus::Library),
                None => (u32::MAX, ResolutionStatus::Unresolved),
            },
            TypeBase::Wildcard(WildcardSyntax::Extends(bound))
            | TypeBase::Wildcard(WildcardSyntax::Super(bound)) => {
                Self::bind(tv, adapter, lowering, b, scope, bound, node)
            }
            TypeBase::Function => (u32::MAX, ResolutionStatus::Unresolved),
        }
    }

    /// Fallback for a reference whose tokens do not parse as a type: bind its first token.
    fn bind_first_token(
        tv: &TokenView,
        adapter: &dyn SemanticAdapter,
        lowering: &mut TypeLowering,
        b: &mut SymbolTableBuilder,
        scope: u32,
        ft: u32,
        node: u32,
    ) -> (u32, ResolutionStatus) {
        let text = tv.text_str(ft);
        if tv.kind(ft) == TokenType::Keyword && is_java_primitive(text.as_bytes()) {
            return match adapter.primitive_type_id(text) {
                Some(id) => (id, ResolutionStatus::Primitive),
                None => (u32::MAX, ResolutionStatus::Unresolved),
            };
        }
        if text == "var" {
            return (u32::MAX, ResolutionStatus::TypeInference);
        }
        if !tv.is_name(ft) {
            return (u32::MAX, ResolutionStatus::Unresolved);
        }
        let (sym, status) = lowering.resolve_simple(b, scope, ft);
        lowering.record_reference(b, ft, node, sym, status);
        (sym, status)
    }
}
