//! Shared type-name resolution and TypeRef lowering for Passes 3–6.
//!
//! `TypeLowering` resolves written type names through `NameResolver` (JLS 6.4.1/7.5 order),
//! materializes the binding as a symbol, records a type-reference occurrence, and lowers a
//! parsed `TypeSyntax` into the TypeRef table with its type arguments, wildcards and array
//! dimensions. Results that depend only on the compilation unit (imports, same package,
//! on-demand and implicit imports) are cached per (compilation unit, name).

use crate::core::types::symbol::SymbolKind;
use crate::symbol::adapter::SemanticAdapter;
use crate::symbol::builder::SymbolTableBuilder;
use crate::symbol::model::{
    Dialect, Occurrence, OccurrenceRole, ResolutionStatus, TypeRef, TypeRefFlags, WildcardKind,
};
use crate::symbol::scope_graph::{NameResolver, Namespace, ScopeFlags};
use crate::symbol::syntax::{
    TokenView, TypeBase, TypeOptions, TypeParser, TypeSyntax, WildcardSyntax,
};
use std::collections::HashMap;

pub struct TypeLowering<'v, 'a> {
    tv: &'v TokenView<'a>,
    adapter: &'v dyn SemanticAdapter,
    cache: HashMap<(u32, u32), (u32, ResolutionStatus)>,
}

impl<'v, 'a> TypeLowering<'v, 'a> {
    pub fn new(tv: &'v TokenView<'a>, adapter: &'v dyn SemanticAdapter) -> Self {
        Self {
            tv,
            adapter,
            cache: HashMap::new(),
        }
    }

    /// Resolves the simple type name written at `name_tok`, as seen from `scope`.
    pub fn resolve_simple(
        &mut self,
        b: &mut SymbolTableBuilder,
        scope: u32,
        name_tok: u32,
    ) -> (u32, ResolutionStatus) {
        let name_id = self.tv.text_id(name_tok);
        let name = self.tv.text_str(name_tok);

        let mut cu = u32::MAX;
        if name_id != u32::MAX {
            let sg = &b.scope_graph;
            for s in sg.ancestors(scope) {
                if sg.has_flag(s, ScopeFlags::PACKAGE) || sg.has_flag(s, ScopeFlags::ROOT) {
                    break;
                }
                if let Some(sym) = sg
                    .lookup_in_scope(s, Namespace::Type, name_id, name_tok)
                    .next()
                {
                    return (sym, ResolutionStatus::Project);
                }
                if sg.has_flag(s, ScopeFlags::COMPILATION_UNIT) {
                    cu = s;
                    break;
                }
            }
        }
        let key = (cu, name_id);
        if name_id != u32::MAX {
            if let Some(&hit) = self.cache.get(&key) {
                return hit;
            }
        }
        let binding = NameResolver::resolve_simple_type(b, scope, name, name_id, name_tok);
        let result = b.materialize_binding(binding, name_id, name_tok);
        if name_id != u32::MAX {
            self.cache.insert(key, result);
        }
        result
    }

    /// Resolves a (possibly qualified) type name given by its segment tokens.
    pub fn resolve_name(
        &mut self,
        b: &mut SymbolTableBuilder,
        scope: u32,
        segment_tokens: &[u32],
    ) -> (u32, ResolutionStatus) {
        match segment_tokens {
            [] => (u32::MAX, ResolutionStatus::Unresolved),
            [single] => self.resolve_simple(b, scope, *single),
            _ => {
                let names: Vec<(&str, u32)> = segment_tokens
                    .iter()
                    .map(|&t| (self.tv.text_str(t), self.tv.text_id(t)))
                    .collect();
                let binding =
                    NameResolver::resolve_qualified_type(b, scope, &names, segment_tokens[0]);
                let last = segment_tokens[segment_tokens.len() - 1];
                b.materialize_binding(binding, self.tv.text_id(last), last)
            }
        }
    }

    pub fn record_reference(
        &self,
        b: &mut SymbolTableBuilder,
        token: u32,
        node: u32,
        symbol: u32,
        status: ResolutionStatus,
    ) {
        if status == ResolutionStatus::Primitive {
            return;
        }
        b.add_occurrence(Occurrence {
            token,
            node,
            symbol,
            role: OccurrenceRole::TypeReference as u8,
            status: status as u8,
            candidates: Vec::new(),
        });
    }

    /// Parses the type written in `[first, last]` and lowers it. Returns the TypeRef id;
    /// a span that does not parse as exactly one type yields an UNKNOWN TypeRef.
    pub fn lower_span(
        &mut self,
        b: &mut SymbolTableBuilder,
        scope: u32,
        first: u32,
        last: u32,
        dialect: Dialect,
        declared: bool,
    ) -> u32 {
        let limit = last.saturating_add(1);
        let mut parser = TypeParser::new(self.tv, limit, dialect);
        let opts = TypeOptions {
            allow_varargs: true,
            allow_var: true,
        };
        match parser.parse(first, opts) {
            Some((ty, next)) if self.tv.sig(next, limit).is_none() => {
                self.lower(b, &ty, scope, declared)
            }
            _ => b.add_type_ref(TypeRef::unknown(first, last)),
        }
    }

    /// Lowers a parsed type. In a declaration (`declared`), an empty argument list `<>` is a
    /// Java `<?>` whose `?` Phase 1 does not tokenize.
    pub fn lower(
        &mut self,
        b: &mut SymbolTableBuilder,
        ty: &TypeSyntax,
        scope: u32,
        declared: bool,
    ) -> u32 {
        let mut tr = TypeRef {
            head: u32::MAX,
            written_name_id: u32::MAX,
            args: Vec::new(),
            array_dims: ty.dims,
            wildcard: WildcardKind::None as u8,
            flags: 0,
            status: ResolutionStatus::Unresolved as u8,
            first_token: ty.first,
            last_token: ty.last,
        };
        if ty.varargs {
            tr.flags |= TypeRefFlags::VARARGS;
        }
        if ty.nullable {
            tr.flags |= TypeRefFlags::NULLABLE;
        }
        match &ty.base {
            TypeBase::Primitive(t) => match self.adapter.primitive_type_id(self.tv.text_str(*t)) {
                Some(id) => {
                    tr.head = id;
                    tr.flags |= TypeRefFlags::PRIMITIVE;
                    tr.status = ResolutionStatus::Primitive as u8;
                }
                None => tr.flags |= TypeRefFlags::UNKNOWN,
            },
            TypeBase::Named(segments) => {
                let toks: Vec<u32> = segments.iter().map(|s| s.tok).collect();
                let (head, status) = self.resolve_name(b, scope, &toks);
                let written: String = toks
                    .iter()
                    .map(|&t| self.tv.text_str(t))
                    .collect::<Vec<_>>()
                    .join(".");
                tr.written_name_id = b.qual_names.get_or_intern(&written);
                tr.head = head;
                tr.status = status as u8;
                if b.symbol_kind(head) == Some(SymbolKind::SK_TYPE_PARAM) {
                    tr.flags |= TypeRefFlags::TYPE_VARIABLE;
                }
                if let Some(&last) = toks.last() {
                    self.record_reference(b, last, u32::MAX, head, status);
                }
                if let Some(seg) = segments.last() {
                    if seg.empty_args && declared {
                        let w = TypeRef {
                            head: u32::MAX,
                            written_name_id: u32::MAX,
                            args: Vec::new(),
                            array_dims: 0,
                            wildcard: WildcardKind::Unbounded as u8,
                            flags: 0,
                            status: ResolutionStatus::Project as u8,
                            first_token: seg.tok,
                            last_token: ty.last,
                        };
                        tr.args.push(b.add_type_ref(w));
                    }
                    for arg in &seg.args {
                        let id = self.lower(b, arg, scope, true);
                        tr.args.push(id);
                    }
                }
            }
            TypeBase::Wildcard(w) => match w {
                WildcardSyntax::Unbounded => {
                    tr.wildcard = WildcardKind::Unbounded as u8;
                    tr.status = ResolutionStatus::Project as u8;
                }
                WildcardSyntax::Extends(bound) => {
                    tr.wildcard = WildcardKind::Extends as u8;
                    let id = self.lower(b, bound, scope, true);
                    tr.status = b.type_ref(id).map(|t| t.status).unwrap_or(tr.status);
                    tr.args.push(id);
                }
                WildcardSyntax::Super(bound) => {
                    tr.wildcard = WildcardKind::Super as u8;
                    let id = self.lower(b, bound, scope, true);
                    tr.status = b.type_ref(id).map(|t| t.status).unwrap_or(tr.status);
                    tr.args.push(id);
                }
            },
            TypeBase::Inferred(_) => {
                tr.flags |= TypeRefFlags::INFERRED;
                tr.status = ResolutionStatus::TypeInference as u8;
            }
            TypeBase::Function => {
                tr.flags |= TypeRefFlags::FUNCTION | TypeRefFlags::UNKNOWN;
            }
        }
        b.add_type_ref(tr)
    }
}
