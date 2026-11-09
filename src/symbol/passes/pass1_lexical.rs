//! Pass 1 (lexical front end) — Kotlin declaration discovery from the token stream.
//!
//! Phase 2 parses Kotlin with the JavaScript grammar, so the AST of a `.kt` file is unusable and
//! is ignored. This front end recognizes Kotlin declarations with grammar-directed token parsers:
//! class/interface/object headers (type parameters, primary constructor properties, delegation
//! specifiers), functions (type parameters, extension receivers, parameters, return types),
//! properties, secondary constructors, `init` blocks, companion objects and enum entries.
//! Every declaration body is delimited by its matching brace, so members attach to the type
//! whose body actually encloses them, and declarations inside function bodies become locals.
//! Symbols have no AST node and carry `SymbolFlags::LEXICAL`.

use crate::core::types::symbol::{ScopeKind, SymbolKind, SymbolModifiers, SymbolVisibility};
use crate::symbol::builder::{LexicalSupertype, SymbolTableBuilder, TypeSpan, UnitInfo};
use crate::symbol::model::{Dialect, SymbolFlags};
use crate::symbol::passes::pass1_discovery::{define, new_record};
use crate::symbol::scope_graph::{Namespace, ScopeFlags};
use crate::symbol::syntax::{skip_annotation, TokenView, TypeBase, TypeOptions, TypeParser};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FrameKind {
    File,
    TypeBody,
    FunctionBody,
}

#[derive(Debug, Clone, Copy)]
struct Frame {
    kind: FrameKind,
    /// Symbol that owns declarations made directly in this frame.
    owner: u32,
    scope: u32,
    /// Members are static on the JVM (object declarations, companion objects).
    static_members: bool,
    /// Owner is an interface: members without a body are abstract.
    interface: bool,
    /// Closing brace of the frame (exclusive end for the file frame).
    close: u32,
}

#[derive(Debug, Clone, Copy, Default)]
struct KotlinModifiers {
    bits: u16,
    visibility: Option<SymbolVisibility>,
    first: u32,
    is_enum: bool,
    is_annotation: bool,
    is_companion: bool,
    is_open: bool,
    is_const: bool,
    is_vararg: bool,
}

const MODIFIER_WORDS: &[&str] = &[
    "public", "private", "protected", "internal", "open", "final", "abstract", "sealed", "data",
    "enum", "annotation", "inner", "value", "inline", "const", "lateinit", "override",
    "companion", "external", "expect", "actual", "suspend", "tailrec", "operator", "infix",
    "crossinline", "noinline", "vararg",
];

pub struct KotlinFrontEnd<'a, 'u> {
    tv: TokenView<'a>,
    unit: &'u UnitInfo,
    start: u32,
    end: u32,
}

impl<'a, 'u> KotlinFrontEnd<'a, 'u> {
    pub fn new(tv: TokenView<'a>, unit: &'u UnitInfo) -> Self {
        let (start, end) = unit.token_range;
        Self {
            tv,
            unit,
            start,
            end: end.min(tv.len()),
        }
    }

    pub fn run(&self, b: &mut SymbolTableBuilder) {
        if self.start == u32::MAX || self.start >= self.end {
            return;
        }
        let mut frames = vec![Frame {
            kind: FrameKind::File,
            owner: self.unit.module_sym,
            scope: self.unit.cu_scope,
            static_members: true,
            interface: false,
            close: self.end,
        }];
        let mut pos = self.unit.header_end.max(self.start);

        while let Some(t) = self.tv.sig(pos, self.end) {
            while frames.len() > 1 && t > frames[frames.len() - 1].close {
                frames.pop();
            }
            let top = frames[frames.len() - 1];
            if !self.at_declaration_position(t) {
                pos = t + 1;
                continue;
            }
            let outcome = if self.tv.word(t, "class") || self.tv.word(t, "interface") {
                self.parse_class_like(b, t, top)
            } else if self.tv.word(t, "object") {
                self.parse_object(b, t, top)
            } else if self.tv.word(t, "fun") {
                match self.tv.next(t, self.end) {
                    Some(n) if self.tv.word(n, "interface") => self.parse_class_like(b, n, top),
                    _ => self.parse_function(b, t, top),
                }
            } else if self.tv.word(t, "val") || self.tv.word(t, "var") {
                self.parse_property(b, t, top)
            } else if self.tv.word(t, "constructor") && top.kind == FrameKind::TypeBody {
                self.parse_secondary_constructor(b, t, top)
            } else if self.tv.word(t, "init") && top.kind == FrameKind::TypeBody {
                self.parse_init_block(b, t, top)
            } else if self.tv.word(t, "typealias") {
                self.skip_line(t)
            } else {
                None
            };
            match outcome {
                Some((next, frame)) => {
                    if let Some(f) = frame {
                        frames.push(f);
                    }
                    pos = next.max(t + 1);
                }
                None => pos = t + 1,
            }
        }
    }

    // ── Helpers ──────────────────────────────────────────────────────────────

    fn at_declaration_position(&self, t: u32) -> bool {
        match self.tv.prev(t, self.start) {
            Some(p) => {
                !(self.tv.punct(p, ".") || self.tv.punct(p, "::") || self.tv.punct(p, "?."))
            }
            None => true,
        }
    }

    fn starts_declaration(&self, t: u32) -> bool {
        self.tv.punct(t, "@")
            || [
                "fun", "val", "var", "class", "interface", "object", "init", "constructor",
                "typealias",
            ]
            .iter()
            .any(|w| self.tv.word(t, w))
            || MODIFIER_WORDS.iter().any(|w| self.tv.word(t, w))
    }

    /// Last token of an expression (initializer, delegate or expression body) starting at
    /// `start`. The expression ends before a closer, `;` or `,` at depth zero, or before a
    /// token on a new line that starts a declaration.
    fn expression_end(&self, start: u32, limit: u32) -> Option<u32> {
        let first = self.tv.sig(start, limit)?;
        let mut pos = first;
        let mut last = first;
        let mut prev_line = self.tv.line(first);
        while let Some(t) = self.tv.sig(pos, limit) {
            if self.tv.is_closer(t) || self.tv.punct(t, ";") || self.tv.punct(t, ",") {
                break;
            }
            let line = self.tv.line(t);
            if t != first && line != prev_line && self.starts_declaration(t) {
                break;
            }
            if self.tv.is_opener(t) {
                match self.tv.matching_close(t, limit) {
                    Some(c) => {
                        last = c;
                        prev_line = self.tv.line(c);
                        pos = c + 1;
                        continue;
                    }
                    None => break,
                }
            }
            last = t;
            prev_line = line;
            pos = t + 1;
        }
        Some(last)
    }

    /// Frame for the body expression of a property initializer or expression-bodied function,
    /// so declarations inside it (lambda locals) belong to `owner`.
    fn expression_frame(
        &self,
        b: &mut SymbolTableBuilder,
        start: u32,
        owner: u32,
        parent_scope: u32,
        limit: u32,
    ) -> Option<(u32, Option<Frame>)> {
        let last = self.expression_end(start, limit)?;
        let scope = b.create_scope(owner, parent_scope, ScopeKind::Block);
        b.scope_graph.add_flags(scope, ScopeFlags::ORDERED);
        Some((
            start,
            Some(Frame {
                kind: FrameKind::FunctionBody,
                owner,
                scope,
                static_members: false,
                interface: false,
                close: last,
            }),
        ))
    }

    fn skip_line(&self, t: u32) -> Option<(u32, Option<Frame>)> {
        let line = self.tv.line(t);
        let mut pos = t + 1;
        while let Some(n) = self.tv.sig(pos, self.end) {
            if self.tv.line(n) != line {
                return Some((n, None));
            }
            pos = n + 1;
        }
        Some((self.end, None))
    }

    /// Modifiers and annotations written before the keyword at `kw`.
    fn modifiers_before(&self, kw: u32) -> KotlinModifiers {
        let mut m = KotlinModifiers {
            first: kw,
            ..Default::default()
        };
        let mut pos = kw;
        while let Some(p) = self.tv.prev(pos, self.start) {
            let text = self.tv.text_str(p);
            if self.tv.is_ident(p) || matches!(self.tv.kind(p), crate::core::types::token::TokenType::Keyword) {
                if let Some(at) = self.tv.prev(p, self.start) {
                    if self.tv.punct(at, "@") {
                        m.first = at;
                        pos = at;
                        continue;
                    }
                }
                if MODIFIER_WORDS.contains(&text) {
                    self.apply_modifier(&mut m, text);
                    m.first = p;
                    pos = p;
                    continue;
                }
                break;
            }
            if self.tv.punct(p, ")") {
                // Annotation with arguments: `@Name(...)`.
                let open = match self.tv.matching_open(p, self.start) {
                    Some(o) => o,
                    None => break,
                };
                let name = match self.tv.prev(open, self.start) {
                    Some(n) if self.tv.is_ident(n) => n,
                    _ => break,
                };
                match self.tv.prev(name, self.start) {
                    Some(at) if self.tv.punct(at, "@") => {
                        m.first = at;
                        pos = at;
                        continue;
                    }
                    _ => break,
                }
            }
            break;
        }
        m
    }

    fn apply_modifier(&self, m: &mut KotlinModifiers, text: &str) {
        match text {
            "public" => m.visibility = Some(SymbolVisibility::Public),
            "private" => m.visibility = Some(SymbolVisibility::Private),
            "protected" => m.visibility = Some(SymbolVisibility::Protected),
            // Kotlin `internal` (module-wide) has no exact counterpart; package access is the
            // closest restriction representable in SymbolVisibility.
            "internal" => m.visibility = Some(SymbolVisibility::Package),
            "abstract" => m.bits |= SymbolModifiers::ABSTRACT,
            "final" => m.bits |= SymbolModifiers::FINAL,
            "sealed" => m.bits |= SymbolModifiers::SEALED | SymbolModifiers::ABSTRACT,
            "external" => m.bits |= SymbolModifiers::NATIVE,
            "open" | "override" => m.is_open = true,
            "enum" => m.is_enum = true,
            "annotation" => m.is_annotation = true,
            "companion" => m.is_companion = true,
            "const" => m.is_const = true,
            "vararg" => m.is_vararg = true,
            _ => {}
        }
    }

    fn type_span_from(&self, colon: u32) -> Option<(TypeSpan, u32)> {
        let mut parser = TypeParser::new(&self.tv, self.end, Dialect::Kotlin);
        let (ty, next) = parser.parse(colon + 1, TypeOptions::default())?;
        Some((
            TypeSpan {
                first: ty.first,
                last: ty.last,
                dialect: Dialect::Kotlin,
                extra_dims: 0,
                union_parts: Vec::new(),
            },
            next,
        ))
    }

    fn parent_for(&self, b: &SymbolTableBuilder, top: Frame, is_type: bool) -> u32 {
        if top.kind == FrameKind::File
            && is_type
            && self.unit.package_sym != u32::MAX
            && b.symbol_kind(top.owner) == Some(SymbolKind::SK_MODULE)
        {
            self.unit.package_sym
        } else {
            top.owner
        }
    }

    // ── Types ────────────────────────────────────────────────────────────────

    fn parse_class_like(
        &self,
        b: &mut SymbolTableBuilder,
        kw: u32,
        top: Frame,
    ) -> Option<(u32, Option<Frame>)> {
        let mods = self.modifiers_before(kw);
        let kind = if self.tv.word(kw, "interface") {
            SymbolKind::SK_INTERFACE
        } else if mods.is_enum {
            SymbolKind::SK_ENUM
        } else if mods.is_annotation {
            SymbolKind::SK_ANNOTATION_TYPE
        } else {
            SymbolKind::SK_CLASS
        };
        let name = self.tv.next(kw, self.end).filter(|&n| self.tv.is_ident(n))?;
        let mut bits = mods.bits;
        match kind {
            SymbolKind::SK_INTERFACE | SymbolKind::SK_ANNOTATION_TYPE => {
                bits |= SymbolModifiers::ABSTRACT
            }
            _ => {
                // Kotlin classes are final unless open, abstract or sealed.
                if !mods.is_open && bits & SymbolModifiers::ABSTRACT == 0 {
                    bits |= SymbolModifiers::FINAL;
                }
            }
        }
        if top.kind == FrameKind::TypeBody && !self.is_inner_class(kw) {
            bits |= SymbolModifiers::STATIC;
        }
        self.declare_type(b, kind, Some(name), name + 1, mods, bits, top, false)
    }

    fn is_inner_class(&self, kw: u32) -> bool {
        let mut pos = kw;
        while let Some(p) = self.tv.prev(pos, self.start) {
            if self.tv.word(p, "inner") {
                return true;
            }
            if !MODIFIER_WORDS.contains(&self.tv.text_str(p)) {
                return false;
            }
            pos = p;
        }
        false
    }

    fn parse_object(
        &self,
        b: &mut SymbolTableBuilder,
        kw: u32,
        top: Frame,
    ) -> Option<(u32, Option<Frame>)> {
        let mods = self.modifiers_before(kw);
        let next = self.tv.next(kw, self.end);
        let name = next.filter(|&n| self.tv.is_ident(n));
        if mods.is_companion && top.kind == FrameKind::TypeBody {
            // Companion members are the JVM statics of the enclosing class.
            let mut pos = name.map(|n| n + 1).unwrap_or(kw + 1);
            if let Some(colon) = self.tv.sig(pos, self.end) {
                if self.tv.punct(colon, ":") {
                    pos = self.skip_supertypes(colon);
                }
            }
            let open = self.tv.sig(pos, self.end).filter(|&o| self.tv.punct(o, "{"));
            return match open {
                Some(o) => {
                    let close = self.tv.matching_close(o, self.end)?;
                    Some((
                        o + 1,
                        Some(Frame {
                            kind: FrameKind::TypeBody,
                            owner: top.owner,
                            scope: top.scope,
                            static_members: true,
                            interface: false,
                            close,
                        }),
                    ))
                }
                None => Some((pos, None)),
            };
        }
        match name {
            Some(n) => {
                let bits = mods.bits | SymbolModifiers::STATIC | SymbolModifiers::FINAL;
                self.declare_type(b, SymbolKind::SK_CLASS, Some(n), n + 1, mods, bits, top, true)
            }
            // `object : Listener { ... }` is an object expression (anonymous class).
            None => self.declare_type(
                b,
                SymbolKind::SK_ANON_CLASS,
                None,
                kw + 1,
                mods,
                SymbolModifiers::FINAL,
                top,
                false,
            ),
        }
    }

    fn skip_supertypes(&self, colon: u32) -> u32 {
        let mut pos = colon + 1;
        loop {
            let mut parser = TypeParser::new(&self.tv, self.end, Dialect::Kotlin);
            let (_, next) = match parser.parse(pos, TypeOptions::default()) {
                Some(r) => r,
                None => return pos,
            };
            pos = next;
            if let Some(o) = self.tv.sig(pos, self.end) {
                if self.tv.punct(o, "(") {
                    match self.tv.matching_close(o, self.end) {
                        Some(c) => pos = c + 1,
                        None => return pos,
                    }
                }
            }
            match self.tv.sig(pos, self.end) {
                Some(c) if self.tv.punct(c, ",") => pos = c + 1,
                _ => return pos,
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn declare_type(
        &self,
        b: &mut SymbolTableBuilder,
        kind: SymbolKind,
        name: Option<u32>,
        after_name: u32,
        mods: KotlinModifiers,
        bits: u16,
        top: Frame,
        object: bool,
    ) -> Option<(u32, Option<Frame>)> {
        let name_id = name.map(|n| self.tv.text_id(n)).unwrap_or(u32::MAX);
        let visibility = mods.visibility.unwrap_or(SymbolVisibility::Public);
        let mut rec = new_record(
            kind,
            name_id,
            u32::MAX,
            top.scope,
            visibility,
            bits,
            mods.first,
            name.unwrap_or(mods.first),
        );
        rec.flags |= SymbolFlags::LEXICAL;
        let sym = b.create_symbol(rec);
        let parent = if kind == SymbolKind::SK_ANON_CLASS {
            top.owner
        } else {
            self.parent_for(b, top, true)
        };
        b.set_parent(sym, parent);
        let own = b.create_scope(sym, top.scope, ScopeKind::Class);
        b.set_owned_scope(sym, own);

        if let Some(n) = name {
            b.bind_name(top.scope, Namespace::Type, name_id, sym, n);
            let simple = self.tv.text_str(n);
            let fqn = if top.kind == FrameKind::File {
                b.bind_name(self.unit.package_scope, Namespace::Type, name_id, sym, 0);
                Some(if self.unit.package_name.is_empty() {
                    simple.to_string()
                } else {
                    format!("{}.{}", self.unit.package_name, simple)
                })
            } else if top.kind == FrameKind::TypeBody {
                b.qualified_name_of(top.owner)
                    .map(|outer| format!("{}.{}", outer, simple))
            } else {
                None
            };
            if let Some(f) = fqn {
                b.register_type_fqn(&f, sym);
            }
            define(b, n, u32::MAX, sym);
        }

        let mut pos = after_name;
        if let Some(lt) = self.tv.sig(pos, self.end) {
            if self.tv.punct(lt, "<") {
                if let Some(after) = self.declare_type_params(b, lt, sym, own) {
                    pos = after;
                }
            }
        }

        // Primary constructor: `[annotations] [visibility] [constructor] ( params )`. The
        // prefix is consumed only when a parameter list follows it.
        let mut p = pos;
        while let Some(t) = self.tv.sig(p, self.end) {
            if self.tv.punct(t, "@") {
                match skip_annotation(&self.tv, t, self.end) {
                    Some((_, next)) => p = next,
                    None => break,
                }
            } else if ["public", "private", "protected", "internal", "constructor"]
                .iter()
                .any(|w| self.tv.word(t, w))
            {
                p = t + 1;
            } else {
                break;
            }
        }
        if let Some(open) = self.tv.sig(p, self.end) {
            if self.tv.punct(open, "(") && kind != SymbolKind::SK_ANON_CLASS {
                let close = self.tv.matching_close(open, self.end)?;
                self.declare_primary_constructor(b, sym, own, name_id, open, close, mods);
                pos = close + 1;
            }
        }

        // Delegation specifiers.
        if let Some(colon) = self.tv.sig(pos, self.end) {
            if self.tv.punct(colon, ":") {
                pos = self.record_supertypes(b, sym, colon);
            }
        }
        if let Some(w) = self.tv.sig(pos, self.end) {
            if self.tv.word(w, "where") {
                pos = self.skip_where_clause(w);
            }
        }

        let open = match self.tv.sig(pos, self.end) {
            Some(o) if self.tv.punct(o, "{") => o,
            _ => {
                if let Some(s) = b.symbols.get_mut(sym as usize) {
                    s.last_token_id = self.tv.prev(pos, mods.first).unwrap_or(s.last_token_id);
                }
                return Some((pos, None));
            }
        };
        let close = self.tv.matching_close(open, self.end)?;
        if let Some(s) = b.symbols.get_mut(sym as usize) {
            s.last_token_id = close;
        }
        let mut body_start = open + 1;
        if kind == SymbolKind::SK_ENUM {
            body_start = self.declare_enum_entries(b, sym, own, open, close);
        }
        Some((
            body_start,
            Some(Frame {
                kind: FrameKind::TypeBody,
                owner: sym,
                scope: own,
                static_members: object,
                interface: kind == SymbolKind::SK_INTERFACE,
                close,
            }),
        ))
    }

    fn skip_where_clause(&self, w: u32) -> u32 {
        let mut pos = w + 1;
        while let Some(t) = self.tv.sig(pos, self.end) {
            if self.tv.punct(t, "{") || self.tv.line(t) != self.tv.line(w) {
                return t;
            }
            pos = t + 1;
        }
        self.end
    }

    fn record_supertypes(&self, b: &mut SymbolTableBuilder, sub: u32, colon: u32) -> u32 {
        let mut pos = colon + 1;
        loop {
            let mut parser = TypeParser::new(&self.tv, self.end, Dialect::Kotlin);
            let (ty, next) = match parser.parse(pos, TypeOptions::default()) {
                Some(r) => r,
                None => return pos,
            };
            pos = next;
            let mut invokes_constructor = false;
            if let Some(o) = self.tv.sig(pos, self.end) {
                if self.tv.punct(o, "(") {
                    match self.tv.matching_close(o, self.end) {
                        Some(c) => {
                            invokes_constructor = true;
                            pos = c + 1;
                        }
                        None => return pos,
                    }
                }
            }
            b.lexical_supertypes.push(LexicalSupertype {
                sub,
                first: ty.first,
                last: ty.last,
                invokes_constructor,
                dialect: Dialect::Kotlin,
            });
            // Interface delegation `I by expr`: skip the delegate expression.
            if let Some(by) = self.tv.sig(pos, self.end) {
                if self.tv.word(by, "by") {
                    pos = by + 1;
                    while let Some(t) = self.tv.sig(pos, self.end) {
                        if self.tv.punct(t, ",") || self.tv.punct(t, "{") {
                            break;
                        }
                        if self.tv.is_opener(t) {
                            pos = match self.tv.matching_close(t, self.end) {
                                Some(c) => c + 1,
                                None => return pos,
                            };
                            continue;
                        }
                        if self.tv.line(t) != self.tv.line(by) {
                            return t;
                        }
                        pos = t + 1;
                    }
                }
            }
            match self.tv.sig(pos, self.end) {
                Some(c) if self.tv.punct(c, ",") => pos = c + 1,
                _ => return pos,
            }
        }
    }

    fn declare_type_params(
        &self,
        b: &mut SymbolTableBuilder,
        lt: u32,
        owner: u32,
        scope: u32,
    ) -> Option<u32> {
        let mut parser = TypeParser::new(&self.tv, self.end, Dialect::Kotlin);
        let (params, after, _) = parser.parse_type_params(lt)?;
        for p in params {
            let name_id = self.tv.text_id(p.name_tok);
            let mut rec = new_record(
                SymbolKind::SK_TYPE_PARAM,
                name_id,
                u32::MAX,
                scope,
                SymbolVisibility::Package,
                0,
                p.first,
                p.last,
            );
            rec.flags |= SymbolFlags::LEXICAL;
            let sym = b.create_symbol(rec);
            b.set_parent(sym, owner);
            b.bind_name(scope, Namespace::Type, name_id, sym, 0);
            let bounds: Vec<(u32, u32)> = p.bounds.iter().map(|t| (t.first, t.last)).collect();
            if !bounds.is_empty() {
                b.type_param_bounds.insert(sym, bounds);
            }
            if let Some(o) = b.symbols.get_mut(owner as usize) {
                o.type_param_count = o.type_param_count.saturating_add(1);
            }
            define(b, p.name_tok, u32::MAX, sym);
        }
        Some(after)
    }

    #[allow(clippy::too_many_arguments)]
    fn declare_primary_constructor(
        &self,
        b: &mut SymbolTableBuilder,
        class_sym: u32,
        class_scope: u32,
        class_name_id: u32,
        open: u32,
        close: u32,
        class_mods: KotlinModifiers,
    ) {
        let mut ctor = new_record(
            SymbolKind::SK_CONSTRUCTOR,
            class_name_id,
            u32::MAX,
            class_scope,
            SymbolVisibility::Public,
            0,
            open,
            close,
        );
        ctor.flags |= SymbolFlags::LEXICAL;
        let ctor_sym = b.create_symbol(ctor);
        b.set_parent(ctor_sym, class_sym);
        b.bind_name(class_scope, Namespace::Method, class_name_id, ctor_sym, 0);
        let ctor_scope = b.create_scope(ctor_sym, class_scope, ScopeKind::Method);
        b.set_owned_scope(ctor_sym, ctor_scope);

        let mut count = 0u16;
        for param in self.parse_parameters(open, close) {
            count = count.saturating_add(1);
            let name_id = self.tv.text_id(param.name);
            let mut prec = new_record(
                SymbolKind::SK_PARAM,
                name_id,
                u32::MAX,
                ctor_scope,
                SymbolVisibility::Package,
                if param.mods.is_vararg {
                    SymbolModifiers::VARARGS
                } else {
                    0
                },
                param.first,
                param.last,
            );
            prec.flags |= SymbolFlags::LEXICAL;
            let p = b.create_symbol(prec);
            b.set_parent(p, ctor_sym);
            b.bind_name(ctor_scope, Namespace::Variable, name_id, p, param.name);
            if let Some(span) = param.type_span.clone() {
                b.type_spans.insert(p, span);
            }
            define(b, param.name, u32::MAX, p);

            if let Some(binding) = param.property {
                // `val`/`var` constructor parameters are also properties of the class.
                let mut bits = if self.tv.word(binding, "val") {
                    SymbolModifiers::FINAL
                } else {
                    0
                };
                if class_mods.is_annotation {
                    bits |= SymbolModifiers::FINAL;
                }
                let mut frec = new_record(
                    SymbolKind::SK_FIELD,
                    name_id,
                    u32::MAX,
                    class_scope,
                    param.mods.visibility.unwrap_or(SymbolVisibility::Public),
                    bits,
                    param.first,
                    param.last,
                );
                frec.flags |= SymbolFlags::LEXICAL;
                let f = b.create_symbol(frec);
                b.set_parent(f, class_sym);
                b.bind_name(class_scope, Namespace::Variable, name_id, f, 0);
                if let Some(span) = param.type_span.clone() {
                    b.type_spans.insert(f, span);
                }
            }
        }
        if let Some(c) = b.symbols.get_mut(ctor_sym as usize) {
            c.param_count = count;
        }
    }

    /// Enum entries at the start of an enum class body; returns the position after them.
    fn declare_enum_entries(
        &self,
        b: &mut SymbolTableBuilder,
        enum_sym: u32,
        scope: u32,
        open: u32,
        close: u32,
    ) -> u32 {
        let mut pos = open + 1;
        loop {
            while let Some(at) = self.tv.sig(pos, close) {
                if !self.tv.punct(at, "@") {
                    break;
                }
                match skip_annotation(&self.tv, at, close) {
                    Some((_, n)) => pos = n,
                    None => return pos,
                }
            }
            let name = match self.tv.sig(pos, close) {
                Some(t) if self.tv.is_ident(t) && !MODIFIER_WORDS.contains(&self.tv.text_str(t)) => t,
                Some(t) if self.tv.punct(t, ";") => return t + 1,
                _ => return pos,
            };
            if self.tv.word(name, "fun") || self.tv.word(name, "val") || self.tv.word(name, "var") {
                return name;
            }
            let mut last = name;
            pos = name + 1;
            for opener in ["(", "{"] {
                if let Some(o) = self.tv.sig(pos, close) {
                    if self.tv.punct(o, opener) {
                        match self.tv.matching_close(o, close) {
                            Some(c) => {
                                last = c;
                                pos = c + 1;
                            }
                            None => return pos,
                        }
                    }
                }
            }
            let name_id = self.tv.text_id(name);
            let mut rec = new_record(
                SymbolKind::SK_ENUM_CONSTANT,
                name_id,
                u32::MAX,
                scope,
                SymbolVisibility::Public,
                SymbolModifiers::STATIC | SymbolModifiers::FINAL,
                name,
                last,
            );
            rec.type_id = enum_sym;
            rec.flags |= SymbolFlags::LEXICAL;
            let sym = b.create_symbol(rec);
            b.set_parent(sym, enum_sym);
            b.bind_name(scope, Namespace::Variable, name_id, sym, name);
            define(b, name, u32::MAX, sym);
            match self.tv.sig(pos, close) {
                Some(c) if self.tv.punct(c, ",") => pos = c + 1,
                Some(c) if self.tv.punct(c, ";") => return c + 1,
                _ => return pos,
            }
        }
    }

    // ── Functions and properties ─────────────────────────────────────────────

    fn parse_parameters(&self, open: u32, close: u32) -> Vec<KotlinParam> {
        let mut out = Vec::new();
        let mut pos = open + 1;
        loop {
            let first = match self.tv.sig(pos, close) {
                Some(t) => t,
                None => return out,
            };
            let mut mods = KotlinModifiers {
                first,
                ..Default::default()
            };
            let mut p = first;
            let mut property = None;
            while let Some(t) = self.tv.sig(p, close) {
                if self.tv.punct(t, "@") {
                    match skip_annotation(&self.tv, t, close) {
                        Some((_, n)) => p = n,
                        None => return out,
                    }
                } else if self.tv.word(t, "val") || self.tv.word(t, "var") {
                    property = Some(t);
                    p = t + 1;
                } else if MODIFIER_WORDS.contains(&self.tv.text_str(t))
                    && self
                        .tv
                        .next(t, close)
                        .is_some_and(|n| self.tv.is_ident(n) || self.tv.punct(n, "@"))
                {
                    self.apply_modifier(&mut mods, self.tv.text_str(t));
                    p = t + 1;
                } else {
                    break;
                }
            }
            let name = match self.tv.sig(p, close) {
                Some(n) if self.tv.is_ident(n) => n,
                _ => return out,
            };
            let mut last = name;
            pos = name + 1;
            let mut type_span = None;
            if let Some(colon) = self.tv.sig(pos, close) {
                if self.tv.punct(colon, ":") {
                    if let Some((span, next)) = self.type_span_from(colon) {
                        last = span.last;
                        pos = next;
                        type_span = Some(span);
                    }
                }
            }
            // Default value: skip to the next top-level comma.
            while let Some(t) = self.tv.sig(pos, close) {
                if self.tv.punct(t, ",") {
                    break;
                }
                if self.tv.is_opener(t) {
                    pos = match self.tv.matching_close(t, close) {
                        Some(c) => c + 1,
                        None => close,
                    };
                    continue;
                }
                pos = t + 1;
            }
            out.push(KotlinParam {
                name,
                first,
                last,
                mods,
                property,
                type_span,
            });
            match self.tv.sig(pos, close) {
                Some(c) if self.tv.punct(c, ",") => pos = c + 1,
                _ => return out,
            }
        }
    }

    fn parse_function(
        &self,
        b: &mut SymbolTableBuilder,
        kw: u32,
        top: Frame,
    ) -> Option<(u32, Option<Frame>)> {
        let mods = self.modifiers_before(kw);
        let mut pos = kw + 1;
        let mut type_param_lt = None;
        if let Some(lt) = self.tv.sig(pos, self.end) {
            if self.tv.punct(lt, "<") {
                let mut parser = TypeParser::new(&self.tv, self.end, Dialect::Kotlin);
                let (_, after, _) = parser.parse_type_params(lt)?;
                type_param_lt = Some(lt);
                pos = after;
            }
        }

        // `[Receiver.]name`: a qualified "type" whose last segment is the function name.
        let mut parser = TypeParser::new(&self.tv, self.end, Dialect::Kotlin);
        let (head, after_head) = parser.parse(pos, TypeOptions::default())?;
        let (receiver, name, mut pos) = match self.tv.sig(after_head, self.end) {
            Some(dot) if self.tv.punct(dot, ".") => {
                let n = self.tv.next(dot, self.end).filter(|&n| self.tv.is_ident(n))?;
                (Some((head.first, head.last)), n, n + 1)
            }
            _ => match &head.base {
                TypeBase::Named(segs) if !head.nullable => {
                    let last = segs.last()?;
                    if !last.args.is_empty() || last.empty_args {
                        return None;
                    }
                    if segs.len() >= 2 {
                        let recv_last = segs[segs.len() - 2]
                            .args
                            .last()
                            .map(|a| a.last)
                            .unwrap_or(segs[segs.len() - 2].tok);
                        (Some((head.first, recv_last)), last.tok, after_head)
                    } else {
                        (None, last.tok, after_head)
                    }
                }
                _ => return None,
            },
        };

        let open = self.tv.sig(pos, self.end).filter(|&o| self.tv.punct(o, "("))?;
        let close = self.tv.matching_close(open, self.end)?;
        pos = close + 1;
        let mut return_span = None;
        if let Some(colon) = self.tv.sig(pos, self.end) {
            if self.tv.punct(colon, ":") {
                if let Some((span, next)) = self.type_span_from(colon) {
                    return_span = Some(span);
                    pos = next;
                }
            }
        }
        if let Some(w) = self.tv.sig(pos, self.end) {
            if self.tv.word(w, "where") {
                pos = self.skip_where_clause(w);
            }
        }
        let body_open = self.tv.sig(pos, self.end).filter(|&o| self.tv.punct(o, "{"));
        let body_close = match body_open {
            Some(o) => Some(self.tv.matching_close(o, self.end)?),
            None => None,
        };
        let has_body = body_open.is_some()
            || self
                .tv
                .sig(pos, self.end)
                .is_some_and(|t| self.tv.punct(t, "="));

        let name_id = self.tv.text_id(name);
        let mut bits = mods.bits;
        if top.static_members || top.kind == FrameKind::File {
            bits |= SymbolModifiers::STATIC;
        }
        if top.interface && !has_body {
            bits |= SymbolModifiers::ABSTRACT;
        } else if !mods.is_open && bits & SymbolModifiers::ABSTRACT == 0 && !top.interface {
            bits |= SymbolModifiers::FINAL;
        }
        let visibility = mods.visibility.unwrap_or(SymbolVisibility::Public);
        let mut rec = new_record(
            SymbolKind::SK_METHOD,
            name_id,
            u32::MAX,
            top.scope,
            visibility,
            bits,
            mods.first,
            body_close.unwrap_or_else(|| self.tv.prev(pos, mods.first).unwrap_or(close)),
        );
        rec.flags |= SymbolFlags::LEXICAL;
        let sym = b.create_symbol(rec);
        let parent = self.parent_for(b, top, false);
        b.set_parent(sym, parent);
        b.bind_name(top.scope, Namespace::Method, name_id, sym, name);
        let own = b.create_scope(sym, top.scope, ScopeKind::Method);
        b.set_owned_scope(sym, own);
        define(b, name, u32::MAX, sym);
        if let Some(lt) = type_param_lt {
            self.declare_type_params(b, lt, sym, own);
        }
        if let Some(span) = return_span {
            b.type_spans.insert(sym, span);
        }

        let mut count = 0u16;
        if let Some((rf, rl)) = receiver {
            // Extension receiver: the implicit first parameter of the JVM method.
            let this_id = self.tv.find_text_id("this");
            let mut prec = new_record(
                SymbolKind::SK_PARAM,
                this_id,
                u32::MAX,
                own,
                SymbolVisibility::Package,
                SymbolModifiers::SYNTHETIC,
                rf,
                rl,
            );
            prec.flags |= SymbolFlags::LEXICAL | SymbolFlags::IMPLICIT;
            let p = b.create_symbol(prec);
            b.set_parent(p, sym);
            b.type_spans.insert(
                p,
                TypeSpan {
                    first: rf,
                    last: rl,
                    dialect: Dialect::Kotlin,
                    extra_dims: 0,
                    union_parts: Vec::new(),
                },
            );
            count = count.saturating_add(1);
        }
        for param in self.parse_parameters(open, close) {
            count = count.saturating_add(1);
            let pid = self.tv.text_id(param.name);
            let mut prec = new_record(
                SymbolKind::SK_PARAM,
                pid,
                u32::MAX,
                own,
                SymbolVisibility::Package,
                if param.mods.is_vararg {
                    SymbolModifiers::VARARGS
                } else {
                    0
                },
                param.first,
                param.last,
            );
            prec.flags |= SymbolFlags::LEXICAL;
            let p = b.create_symbol(prec);
            b.set_parent(p, sym);
            b.bind_name(own, Namespace::Variable, pid, p, param.name);
            if let Some(span) = param.type_span {
                b.type_spans.insert(p, span);
            }
            define(b, param.name, u32::MAX, p);
        }
        if let Some(m) = b.symbols.get_mut(sym as usize) {
            m.param_count = count;
        }

        match (body_open, body_close) {
            (Some(o), Some(c)) => {
                let body = b.create_scope(sym, own, ScopeKind::Block);
                b.scope_graph.add_flags(body, ScopeFlags::ORDERED);
                Some((
                    o + 1,
                    Some(Frame {
                        kind: FrameKind::FunctionBody,
                        owner: sym,
                        scope: body,
                        static_members: false,
                        interface: false,
                        close: c,
                    }),
                ))
            }
            _ => match self.tv.sig(pos, self.end) {
                Some(eq) if self.tv.punct(eq, "=") => {
                    self.expression_frame(b, eq + 1, sym, own, top.close)
                }
                _ => Some((pos, None)),
            },
        }
    }

    fn parse_property(
        &self,
        b: &mut SymbolTableBuilder,
        kw: u32,
        top: Frame,
    ) -> Option<(u32, Option<Frame>)> {
        let mods = self.modifiers_before(kw);
        let mut pos = kw + 1;
        if let Some(lt) = self.tv.sig(pos, self.end) {
            if self.tv.punct(lt, "<") {
                let mut parser = TypeParser::new(&self.tv, self.end, Dialect::Kotlin);
                let (_, after, _) = parser.parse_type_params(lt)?;
                pos = after;
            }
        }
        let local = top.kind == FrameKind::FunctionBody;
        let is_val = self.tv.word(kw, "val");

        // Destructuring declaration `val (a, b) = ...`.
        if let Some(open) = self.tv.sig(pos, self.end) {
            if self.tv.punct(open, "(") {
                let close = self.tv.matching_close(open, self.end)?;
                for param in self.parse_parameters(open, close) {
                    self.declare_property_symbol(b, param.name, param.first, param.last, param.type_span, mods, is_val, top, local);
                }
                return Some((close + 1, None));
            }
        }

        let mut parser = TypeParser::new(&self.tv, self.end, Dialect::Kotlin);
        let (head, after_head) = parser.parse(pos, TypeOptions::default())?;
        let name = match self.tv.sig(after_head, self.end) {
            Some(dot) if self.tv.punct(dot, ".") => {
                self.tv.next(dot, self.end).filter(|&n| self.tv.is_ident(n))?
            }
            _ => match &head.base {
                TypeBase::Named(segs) if !head.nullable => {
                    let last = segs.last()?;
                    if !last.args.is_empty() || last.empty_args {
                        return None;
                    }
                    last.tok
                }
                _ => return None,
            },
        };
        let mut next = name + 1;
        let mut type_span = None;
        let mut last = name;
        if let Some(colon) = self.tv.sig(next, self.end) {
            if self.tv.punct(colon, ":") {
                if let Some((span, after)) = self.type_span_from(colon) {
                    last = span.last;
                    next = after;
                    type_span = Some(span);
                }
            }
        }
        let sym =
            self.declare_property_symbol(b, name, mods.first, last, type_span, mods, is_val, top, local);
        if !local {
            if let Some(t) = self.tv.sig(next, self.end) {
                if self.tv.punct(t, "=") || self.tv.word(t, "by") {
                    return self.expression_frame(b, t + 1, sym, top.scope, top.close);
                }
            }
        }
        Some((next, None))
    }

    #[allow(clippy::too_many_arguments)]
    fn declare_property_symbol(
        &self,
        b: &mut SymbolTableBuilder,
        name: u32,
        first: u32,
        last: u32,
        type_span: Option<TypeSpan>,
        mods: KotlinModifiers,
        is_val: bool,
        top: Frame,
        local: bool,
    ) -> u32 {
        let name_id = self.tv.text_id(name);
        let kind = if local {
            SymbolKind::SK_LOCAL_VAR
        } else {
            SymbolKind::SK_FIELD
        };
        let mut bits = mods.bits;
        if is_val {
            bits |= SymbolModifiers::FINAL;
        }
        if mods.is_const {
            bits |= SymbolModifiers::STATIC | SymbolModifiers::FINAL;
        }
        if !local && (top.static_members || top.kind == FrameKind::File) {
            bits |= SymbolModifiers::STATIC;
        }
        if top.interface && !local {
            bits |= SymbolModifiers::ABSTRACT;
        }
        let visibility = if local {
            SymbolVisibility::Package
        } else {
            mods.visibility.unwrap_or(SymbolVisibility::Public)
        };
        let mut rec = new_record(
            kind,
            name_id,
            u32::MAX,
            top.scope,
            visibility,
            bits,
            first.min(name),
            last.max(name),
        );
        rec.flags |= SymbolFlags::LEXICAL;
        let sym = b.create_symbol(rec);
        let parent = self.parent_for(b, top, false);
        b.set_parent(sym, parent);
        b.bind_name(top.scope, Namespace::Variable, name_id, sym, name);
        if let Some(span) = type_span {
            b.type_spans.insert(sym, span);
        }
        define(b, name, u32::MAX, sym);
        sym
    }

    fn parse_secondary_constructor(
        &self,
        b: &mut SymbolTableBuilder,
        kw: u32,
        top: Frame,
    ) -> Option<(u32, Option<Frame>)> {
        let open = self.tv.next(kw, self.end).filter(|&o| self.tv.punct(o, "("))?;
        let close = self.tv.matching_close(open, self.end)?;
        let mods = self.modifiers_before(kw);
        let class_name = b.symbol(top.owner).map(|s| s.name_id).unwrap_or(u32::MAX);
        let mut pos = close + 1;
        // Delegation `: this(...)` / `: super(...)`.
        if let Some(colon) = self.tv.sig(pos, self.end) {
            if self.tv.punct(colon, ":") {
                if let Some(target) = self.tv.next(colon, self.end) {
                    if let Some(o) = self.tv.next(target, self.end) {
                        if self.tv.punct(o, "(") {
                            pos = self.tv.matching_close(o, self.end)? + 1;
                        }
                    }
                }
            }
        }
        let body_open = self.tv.sig(pos, self.end).filter(|&o| self.tv.punct(o, "{"));
        let body_close = match body_open {
            Some(o) => Some(self.tv.matching_close(o, self.end)?),
            None => None,
        };
        let mut rec = new_record(
            SymbolKind::SK_CONSTRUCTOR,
            class_name,
            u32::MAX,
            top.scope,
            mods.visibility.unwrap_or(SymbolVisibility::Public),
            mods.bits,
            mods.first,
            body_close.unwrap_or(close),
        );
        rec.flags |= SymbolFlags::LEXICAL;
        let sym = b.create_symbol(rec);
        b.set_parent(sym, top.owner);
        b.bind_name(top.scope, Namespace::Method, class_name, sym, kw);
        let own = b.create_scope(sym, top.scope, ScopeKind::Method);
        b.set_owned_scope(sym, own);
        let mut count = 0u16;
        for param in self.parse_parameters(open, close) {
            count = count.saturating_add(1);
            let pid = self.tv.text_id(param.name);
            let mut prec = new_record(
                SymbolKind::SK_PARAM,
                pid,
                u32::MAX,
                own,
                SymbolVisibility::Package,
                0,
                param.first,
                param.last,
            );
            prec.flags |= SymbolFlags::LEXICAL;
            let p = b.create_symbol(prec);
            b.set_parent(p, sym);
            b.bind_name(own, Namespace::Variable, pid, p, param.name);
            if let Some(span) = param.type_span {
                b.type_spans.insert(p, span);
            }
            define(b, param.name, u32::MAX, p);
        }
        if let Some(c) = b.symbols.get_mut(sym as usize) {
            c.param_count = count;
        }
        match (body_open, body_close) {
            (Some(o), Some(c)) => {
                let body = b.create_scope(sym, own, ScopeKind::Block);
                b.scope_graph.add_flags(body, ScopeFlags::ORDERED);
                Some((
                    o + 1,
                    Some(Frame {
                        kind: FrameKind::FunctionBody,
                        owner: sym,
                        scope: body,
                        static_members: false,
                        interface: false,
                        close: c,
                    }),
                ))
            }
            _ => Some((close + 1, None)),
        }
    }

    fn parse_init_block(
        &self,
        b: &mut SymbolTableBuilder,
        kw: u32,
        top: Frame,
    ) -> Option<(u32, Option<Frame>)> {
        let open = self.tv.next(kw, self.end).filter(|&o| self.tv.punct(o, "{"))?;
        let close = self.tv.matching_close(open, self.end)?;
        let kind = if top.static_members {
            SymbolKind::SK_STATIC_INIT
        } else {
            SymbolKind::SK_INSTANCE_INIT
        };
        let mut rec = new_record(
            kind,
            u32::MAX,
            u32::MAX,
            top.scope,
            SymbolVisibility::Package,
            if top.static_members {
                SymbolModifiers::STATIC
            } else {
                0
            },
            kw,
            close,
        );
        rec.flags |= SymbolFlags::LEXICAL;
        let sym = b.create_symbol(rec);
        b.set_parent(sym, top.owner);
        let body = b.create_scope(sym, top.scope, ScopeKind::Block);
        b.scope_graph.add_flags(body, ScopeFlags::ORDERED);
        b.set_owned_scope(sym, body);
        Some((
            open + 1,
            Some(Frame {
                kind: FrameKind::FunctionBody,
                owner: sym,
                scope: body,
                static_members: false,
                interface: false,
                close,
            }),
        ))
    }
}

struct KotlinParam {
    name: u32,
    first: u32,
    last: u32,
    mods: KotlinModifiers,
    /// `val`/`var` token when the parameter also declares a property.
    property: Option<u32>,
    type_span: Option<TypeSpan>,
}
