//! Pass 1 — Declaration Discovery
//!
//! Sets up one compilation unit per source file (package symbol and scope, module symbol and
//! compilation-unit scope) and then discovers declarations with the front end that fits the
//! file's language:
//!
//! * Structural (Java): every name comes from the grammar position that holds it — the
//!   `identifier` child of a type, method, constructor, parameter or variable declarator —
//!   never from a token window. One symbol is created per variable declarator. Constructs that
//!   Phase 2 drops (enum constants, interface constants, catch parameters, inferred lambda
//!   parameters, type parameters) are recovered by grammar-anchored token recognizers, and the
//!   implicit members of records and enums are synthesized (JLS 8.9.3, 8.10.3).
//! * AstHeuristic (files that only have the generic AST): declarations from AST nodes, names
//!   from a token heuristic, flagged `HEURISTIC_NAME`.
//! * Lexical (Kotlin, whose Phase 2 tree is a JavaScript parse): the brace-scoped token front
//!   end in `pass1_lexical`.
//!
//! Containment is recorded through `parent_sym` only; child chains are derived at the end.

use crate::ast::BPASTArtifact;
use crate::core::types::ast::{ASTNodeType, NodeAttr};
use crate::core::types::symbol::{
    ScopeKind, SymbolKind, SymbolModifiers, SymbolRecord, SymbolVisibility,
};
use crate::ingestion::TokenCorpusArtifact;
use crate::symbol::adapter::SemanticAdapter;
use crate::symbol::ast_index::AstIndex;
use crate::symbol::builder::{SymbolTableBuilder, TypeSpan, UnitInfo};
use crate::symbol::model::{
    DiagnosticCode, Dialect, FrontEnd, Occurrence, OccurrenceRole, ResolutionStatus, SymbolFlags,
};
use crate::symbol::passes::pass1_lexical::KotlinFrontEnd;
use crate::symbol::scope_graph::{Namespace, ScopeFlags};
use crate::symbol::syntax::{
    parse_catch_parameter, parse_compilation_unit_header, parse_enum_constants,
    parse_field_declarations, parse_inferred_lambda_params, parse_java_type_header,
    parse_trailing_dims, scan_java_modifiers, skip_annotation, ModifierScan, TokenView,
    TypeOptions, TypeParser,
};
use std::collections::BTreeSet;

pub struct Pass1Discovery;

impl Pass1Discovery {
    pub fn run(
        bpa: &BPASTArtifact,
        tca: &TokenCorpusArtifact,
        adapter: &dyn SemanticAdapter,
        builder: &mut SymbolTableBuilder,
    ) {
        let ast = AstIndex::build(bpa, tca);
        Self::run_with_index(bpa, tca, &ast, adapter, builder);
    }

    /// Same as `run`, reusing an index the caller already built over `bpa`.
    pub fn run_with_index(
        bpa: &BPASTArtifact,
        tca: &TokenCorpusArtifact,
        ast: &AstIndex,
        adapter: &dyn SemanticAdapter,
        builder: &mut SymbolTableBuilder,
    ) {
        let pass1_start = std::time::Instant::now();
        let tv = TokenView::new(tca);
        builder.init_node_tables(bpa.node_count);
        builder.std_lib.bind_names(&tca.interner);

        let file_ranges = tv.file_ranges();
        let mut work: Vec<(Option<u16>, u32)> = Vec::new();
        let mut files_with_root: BTreeSet<u16> = BTreeSet::new();
        for &root in ast.roots() {
            let fid = ast.file_of_root(root);
            if let Some(f) = fid {
                files_with_root.insert(f);
            }
            work.push((fid, root));
        }
        for &fid in file_ranges.keys() {
            if !files_with_root.contains(&fid) {
                work.push((Some(fid), u32::MAX));
            }
        }

        for (fid, root) in work {
            let lang = fid.and_then(|f| ast.language_of_file(f));
            let front_end = adapter.front_end(lang);
            let dialect = adapter.dialect(lang);
            let token_range = fid
                .and_then(|f| file_ranges.get(&f).copied())
                .unwrap_or((u32::MAX, u32::MAX));
            let unit = Self::open_unit(
                bpa,
                ast,
                &tv,
                builder,
                fid,
                root,
                token_range,
                front_end,
                dialect,
            );

            match front_end {
                FrontEnd::Structural if root != u32::MAX => {
                    let walk = StructuralWalk {
                        bpa,
                        ast,
                        tv,
                        adapter,
                        unit: &unit,
                    };
                    walk.run(builder);
                }
                FrontEnd::AstHeuristic if root != u32::MAX => {
                    builder.add_diagnostic(
                        DiagnosticCode::HeuristicFrontEnd,
                        unit.module_sym,
                        Self::valid_or_max(&tv, token_range.0),
                        Vec::new(),
                        format!(
                            "file {:?} ({:?}) has no structural front end; declaration names are heuristic",
                            fid, lang
                        ),
                    );
                    let walk = HeuristicWalk {
                        bpa,
                        ast,
                        tv,
                        adapter,
                        unit: &unit,
                    };
                    walk.run(builder);
                }
                FrontEnd::Lexical => {
                    if root != u32::MAX {
                        builder.exclude_node_range(root, ast.subtree_end(root));
                    }
                    builder.add_diagnostic(
                        DiagnosticCode::LexicalFrontEnd,
                        unit.module_sym,
                        Self::valid_or_max(&tv, token_range.0),
                        Vec::new(),
                        format!(
                            "file {:?} ({:?}) has no usable AST; declarations come from the token-level front end and have no AST nodes",
                            fid, lang
                        ),
                    );
                    if dialect == Dialect::Kotlin && token_range.0 != u32::MAX {
                        KotlinFrontEnd::new(tv, &unit).run(builder);
                    }
                }
                _ => {
                    builder.add_diagnostic(
                        DiagnosticCode::MissingAst,
                        unit.module_sym,
                        Self::valid_or_max(&tv, token_range.0),
                        Vec::new(),
                        format!(
                            "file {:?} has tokens but Phase 2 produced no syntax tree; its declarations are not discovered",
                            fid
                        ),
                    );
                }
            }
        }

        builder.rebuild_child_chains();
        crate::core::logger::log_info(&format!(
            "[PASS1] Discovery done in {:?}: {} symbols, {} scopes, {} units",
            pass1_start.elapsed(),
            builder.symbols.len(),
            builder.scope_graph.scope_count(),
            builder.units.len()
        ));
    }

    fn valid_or_max(tv: &TokenView, tok: u32) -> u32 {
        if tv.valid(tok) {
            tok
        } else {
            u32::MAX
        }
    }

    /// Creates the package (if any), module symbol and compilation-unit scope of one file.
    #[allow(clippy::too_many_arguments)]
    fn open_unit(
        bpa: &BPASTArtifact,
        ast: &AstIndex,
        tv: &TokenView,
        b: &mut SymbolTableBuilder,
        file_id: Option<u16>,
        root: u32,
        token_range: (u32, u32),
        front_end: FrontEnd,
        dialect: Dialect,
    ) -> UnitInfo {
        let header = if token_range.0 != u32::MAX && dialect != Dialect::Other {
            parse_compilation_unit_header(tv, token_range.0, token_range.1, dialect)
        } else {
            Default::default()
        };

        let package_name: String = header
            .package
            .iter()
            .map(|&t| tv.text_str(t))
            .collect::<Vec<_>>()
            .join(".");
        let (package_sym, package_scope) = if header.package.is_empty() {
            (u32::MAX, b.unnamed_package_scope())
        } else {
            let segments: Vec<String> = header
                .package
                .iter()
                .map(|&t| tv.text_str(t).to_string())
                .collect();
            let ids: Vec<u32> = header.package.iter().map(|&t| tv.text_id(t)).collect();
            let entry = b.ensure_package(&segments, &ids);
            if let Some(sym) = b.symbols.get_mut(entry.sym as usize) {
                if sym.first_token_id == u32::MAX {
                    sym.first_token_id = header.package_token;
                    sym.last_token_id = header.package.last().copied().unwrap_or(u32::MAX);
                }
            }
            (entry.sym, entry.scope)
        };

        let (first, last) = if token_range.0 != u32::MAX {
            (token_range.0, token_range.1.saturating_sub(1))
        } else if root != u32::MAX {
            let (ft, lt) = bpa.token_range(root);
            (
                if tv.valid(ft) { ft } else { u32::MAX },
                if tv.valid(lt) { lt } else { u32::MAX },
            )
        } else {
            (u32::MAX, u32::MAX)
        };

        let mut rec = SymbolRecord::UNINIT;
        rec.kind = SymbolKind::SK_MODULE as u8;
        rec.visibility = SymbolVisibility::Public as u8;
        rec.decl_node = root;
        rec.def_node = root;
        rec.scope_id = package_scope;
        rec.first_token_id = first;
        rec.last_token_id = last;
        if root == u32::MAX {
            rec.flags |= SymbolFlags::LEXICAL;
        }
        let module_sym = b.create_symbol(rec);
        b.set_parent(module_sym, package_sym);
        let cu_scope = b.create_scope(module_sym, package_scope, ScopeKind::File);
        b.scope_graph
            .add_flags(cu_scope, ScopeFlags::COMPILATION_UNIT);
        b.set_owned_scope(module_sym, cu_scope);
        if root != u32::MAX {
            b.set_node_context(root, cu_scope, module_sym);
        }

        let node_range = if root != u32::MAX {
            (root, ast.subtree_end(root))
        } else {
            (u32::MAX, u32::MAX)
        };
        let unit = UnitInfo {
            file_id,
            root_node: root,
            node_range,
            module_sym,
            cu_scope,
            package_scope,
            package_sym,
            package_name,
            dialect,
            front_end,
            header_end: header.end,
            token_range,
        };
        b.add_unit(unit.clone());
        unit
    }
}

/// Builds a symbol record with the fields every front end sets.
#[allow(clippy::too_many_arguments)]
pub(crate) fn new_record(
    kind: SymbolKind,
    name_id: u32,
    decl_node: u32,
    scope: u32,
    visibility: SymbolVisibility,
    modifiers: u16,
    first: u32,
    last: u32,
) -> SymbolRecord {
    SymbolRecord {
        symbol_id: u32::MAX,
        name_id,
        qual_name_id: u32::MAX,
        type_id: u32::MAX,
        decl_node,
        def_node: decl_node,
        parent_sym: u32::MAX,
        first_child: u32::MAX,
        next_sibling: u32::MAX,
        scope_id: scope,
        uml_meta_offset: 0,
        param_count: 0,
        modifiers,
        kind: kind as u8,
        visibility: visibility as u8,
        type_param_count: 0,
        flags: 0,
        first_token_id: first,
        last_token_id: last,
        _reserved: 0,
    }
}

/// Records the definition occurrence of a symbol at its name token.
pub(crate) fn define(b: &mut SymbolTableBuilder, token: u32, node: u32, symbol: u32) {
    b.add_occurrence(Occurrence {
        token,
        node,
        symbol,
        role: OccurrenceRole::Definition as u8,
        status: ResolutionStatus::Project as u8,
        candidates: Vec::new(),
    });
}

fn is_type_decl(t: ASTNodeType) -> bool {
    matches!(
        t,
        ASTNodeType::NN_CLASS_DECL
            | ASTNodeType::NN_INTERFACE_DECL
            | ASTNodeType::NN_ENUM_DECL
            | ASTNodeType::NN_RECORD_DECL
            | ASTNodeType::NN_ANNOTATION_DECL
    )
}

fn is_type_kind(kind: SymbolKind) -> bool {
    matches!(
        kind,
        SymbolKind::SK_CLASS
            | SymbolKind::SK_INTERFACE
            | SymbolKind::SK_ENUM
            | SymbolKind::SK_RECORD
            | SymbolKind::SK_ANNOTATION_TYPE
    )
}

/// Kind of the declaration enclosing a member, `None` at top level.
fn enclosing_decl_kind(b: &SymbolTableBuilder, sym: u32) -> Option<SymbolKind> {
    match b.symbol_kind(sym) {
        Some(SymbolKind::SK_MODULE) | Some(SymbolKind::SK_PACKAGE) | None => None,
        k => k,
    }
}

/// Body information of a type declaration discovered in the walk.
struct TypeBody {
    sym: u32,
    node: u32,
    kind: SymbolKind,
    name_tok: u32,
    body_open: Option<u32>,
    body_close: Option<u32>,
}

struct StructuralWalk<'a, 'u> {
    bpa: &'a BPASTArtifact,
    ast: &'u AstIndex,
    tv: TokenView<'a>,
    adapter: &'u dyn SemanticAdapter,
    unit: &'u UnitInfo,
}

impl<'a, 'u> StructuralWalk<'a, 'u> {
    fn file_end(&self) -> u32 {
        if self.unit.token_range.1 == u32::MAX {
            self.tv.len()
        } else {
            self.unit.token_range.1
        }
    }

    fn tok(&self, t: u32) -> u32 {
        if self.tv.valid(t) {
            t
        } else {
            u32::MAX
        }
    }

    fn name_token_of(&self, node: u32) -> Option<u32> {
        self.ast
            .first_child_of_type(self.bpa, node, ASTNodeType::NN_IDENTIFIER_EXPR)
            .map(|n| self.bpa.token_range(n).0)
            .filter(|&t| self.tv.valid(t))
    }

    fn name_id_of(&self, tok: Option<u32>) -> u32 {
        tok.map(|t| self.tv.text_id(t)).unwrap_or(u32::MAX)
    }

    /// Modifier list at the start of `node`, scanned from tokens. Phase 2's attribute word is
    /// only used when the node covers no token.
    fn scan_modifiers(&self, node: u32, limit: Option<u32>) -> ModifierScan {
        let (ft, _) = self.bpa.token_range(node);
        if self.tv.valid(ft) {
            let limit = limit.unwrap_or(u32::MAX).min(self.file_end());
            let mut scan = scan_java_modifiers(&self.tv, ft, limit);
            if scan
                .annotations
                .iter()
                .any(|&a| self.tv.text(a) == b"Deprecated")
            {
                scan.modifiers |= SymbolModifiers::DEPRECATED;
            }
            return scan;
        }
        let attr = self.bpa.node_attr(node);
        let visibility = match NodeAttr::unpack_visibility(attr) {
            NodeAttr::VISIBILITY_PUBLIC => Some(SymbolVisibility::Public),
            NodeAttr::VISIBILITY_PRIVATE => Some(SymbolVisibility::Private),
            NodeAttr::VISIBILITY_PROTECTED => Some(SymbolVisibility::Protected),
            NodeAttr::VISIBILITY_PACKAGE_PRIVATE => Some(SymbolVisibility::Package),
            _ => None,
        };
        let raw = NodeAttr::unpack_modifiers(attr);
        let mut modifiers = 0u16;
        let table = [
            (NodeAttr::MOD_STATIC, SymbolModifiers::STATIC),
            (NodeAttr::MOD_FINAL, SymbolModifiers::FINAL),
            (NodeAttr::MOD_ABSTRACT, SymbolModifiers::ABSTRACT),
            (NodeAttr::MOD_SYNCHRONIZED, SymbolModifiers::SYNCHRONIZED),
            (NodeAttr::MOD_NATIVE, SymbolModifiers::NATIVE),
            (NodeAttr::MOD_VOLATILE, SymbolModifiers::VOLATILE),
            (NodeAttr::MOD_TRANSIENT, SymbolModifiers::TRANSIENT),
            (NodeAttr::MOD_SEALED, SymbolModifiers::SEALED),
        ];
        for (bit, m) in table {
            if raw & bit != 0 {
                modifiers |= m;
            }
        }
        ModifierScan {
            modifiers,
            visibility,
            end: u32::MAX,
            ..Default::default()
        }
    }

    /// Token span `[first, last]` of a written type that starts at `start` and is followed by
    /// the declarator name `name_tok`.
    fn type_span(&self, start: u32, name_tok: u32, extra_dims: u8) -> Option<TypeSpan> {
        if !self.tv.valid(start) || !self.tv.valid(name_tok) {
            return None;
        }
        let first = self.tv.sig(start, name_tok)?;
        let last = self.tv.prev(name_tok, first)?;
        if last < first {
            return None;
        }
        Some(TypeSpan {
            first,
            last,
            dialect: Dialect::Java,
            extra_dims,
            union_parts: Vec::new(),
        })
    }

    fn run(&self, b: &mut SymbolTableBuilder) {
        let root = self.unit.root_node;
        let end = self.ast.subtree_end(root);
        let mut types: Vec<TypeBody> = Vec::new();

        for node in (root + 1)..end {
            let parent = self.ast.parent(node);
            let (scope, enclosing) = if parent == u32::MAX {
                (self.unit.cu_scope, self.unit.module_sym)
            } else {
                (b.scope_of_node(parent), b.enclosing_symbol_of_node(parent))
            };
            let (child_scope, child_sym) =
                self.visit(b, node, parent, scope, enclosing, &mut types);
            b.set_node_context(node, child_scope, child_sym);
        }

        for t in &types {
            match t.kind {
                SymbolKind::SK_ENUM => self.complete_enum(b, t),
                SymbolKind::SK_INTERFACE => self.recover_interface_constants(b, t),
                SymbolKind::SK_RECORD => self.complete_record(b, t),
                _ => {}
            }
        }
    }

    fn visit(
        &self,
        b: &mut SymbolTableBuilder,
        node: u32,
        parent: u32,
        scope: u32,
        enclosing: u32,
        types: &mut Vec<TypeBody>,
    ) -> (u32, u32) {
        let ty = self.bpa.node_type(node);
        let parent_ty = self.bpa.node_type(parent);
        match ty {
            t if is_type_decl(t) => self.declare_type(b, node, t, scope, enclosing, types),
            ASTNodeType::NN_METHOD_DECL | ASTNodeType::NN_CONSTRUCTOR_DECL => {
                self.declare_method(b, node, ty, scope, enclosing)
            }
            ASTNodeType::NN_FIELD_DECL => {
                let has_declarators = self
                    .ast
                    .children_of_type(self.bpa, node, ASTNodeType::NN_LOCAL_VAR_DECL)
                    .next()
                    .is_some();
                if !has_declarators {
                    // A field declaration whose declarators Phase 2 did not keep: one field of
                    // unknown name stands for it.
                    self.declare_variable(b, node, None, SymbolKind::SK_FIELD, scope, enclosing);
                }
                (scope, enclosing)
            }
            ASTNodeType::NN_LOCAL_VAR_DECL => match parent_ty {
                ASTNodeType::NN_FIELD_DECL => self.declare_variable(
                    b,
                    node,
                    Some(parent),
                    SymbolKind::SK_FIELD,
                    scope,
                    enclosing,
                ),
                ASTNodeType::NN_LOCAL_VAR_DECL => self.declare_variable(
                    b,
                    node,
                    Some(parent),
                    SymbolKind::SK_LOCAL_VAR,
                    scope,
                    enclosing,
                ),
                // Declarator of a spread parameter: the parameter symbol carries its name.
                ASTNodeType::NN_PARAM_DECL => (scope, enclosing),
                _ => {
                    let has_declarators = self
                        .ast
                        .children_of_type(self.bpa, node, ASTNodeType::NN_LOCAL_VAR_DECL)
                        .next()
                        .is_some();
                    if has_declarators {
                        (scope, enclosing)
                    } else {
                        self.declare_variable(
                            b,
                            node,
                            None,
                            SymbolKind::SK_LOCAL_VAR,
                            scope,
                            enclosing,
                        )
                    }
                }
            },
            ASTNodeType::NN_PARAM_DECL => self.declare_param(b, node, parent_ty, scope, enclosing),
            ASTNodeType::NN_TYPE_PARAM => {
                self.declare_type_params(b, node, scope, enclosing);
                (scope, enclosing)
            }
            ASTNodeType::NN_LAMBDA_EXPR => self.declare_lambda(b, node, scope, enclosing),
            ASTNodeType::NN_NEW_EXPR => match self.anonymous_body(node) {
                Some((new_tok, close)) => {
                    let has_body_node = self
                        .ast
                        .first_child_of_type(self.bpa, node, ASTNodeType::NN_ANON_CLASS_BODY)
                        .is_some();
                    let (own, anon) =
                        self.declare_anonymous(b, node, new_tok, close, scope, enclosing);
                    if has_body_node {
                        // The type and the constructor arguments are outside the class body;
                        // the body node opens the anonymous class scope.
                        (scope, enclosing)
                    } else {
                        (own, anon)
                    }
                }
                None => (scope, enclosing),
            },
            ASTNodeType::NN_ANON_CLASS_BODY => match parent_ty {
                ASTNodeType::NN_NEW_EXPR => match b.symbol_at_node(parent) {
                    Some(anon) if b.symbol_kind(anon) == Some(SymbolKind::SK_ANON_CLASS) => {
                        (b.owned_scope(anon), anon)
                    }
                    _ => (scope, enclosing),
                },
                ASTNodeType::NN_ENUM_CONSTANT => {
                    self.declare_constant_body(b, node, scope, enclosing)
                }
                _ => (scope, enclosing),
            },
            ASTNodeType::NN_ENUM_CONSTANT => self.declare_enum_constant(b, node, scope, enclosing),
            ASTNodeType::NN_JAVA_INSTANCE_INIT => {
                self.declare_initializer(b, node, SymbolKind::SK_INSTANCE_INIT, scope, enclosing)
            }
            ASTNodeType::NN_PATTERN_MATCH => {
                self.declare_pattern_binding(b, node, scope, enclosing);
                (scope, enclosing)
            }
            ASTNodeType::NN_BLOCK => {
                let declares_body = matches!(
                    parent_ty,
                    ASTNodeType::NN_CLASS_DECL
                        | ASTNodeType::NN_ENUM_DECL
                        | ASTNodeType::NN_RECORD_DECL
                ) || (parent_ty == ASTNodeType::NN_NEW_EXPR
                    && b.symbol_at_node(parent) == Some(enclosing)
                    && b.symbol_kind(enclosing) == Some(SymbolKind::SK_ANON_CLASS));
                if declares_body {
                    self.declare_initializer(
                        b,
                        node,
                        SymbolKind::SK_INSTANCE_INIT,
                        scope,
                        enclosing,
                    )
                } else {
                    (self.open_block(b, scope, enclosing), enclosing)
                }
            }
            ASTNodeType::NN_JAVA_STATIC_INIT => {
                self.declare_initializer(b, node, SymbolKind::SK_STATIC_INIT, scope, enclosing)
            }
            ASTNodeType::NN_ENHANCED_FOR => {
                let block = self.open_block(b, scope, enclosing);
                self.declare_for_each_variable(b, node, block, enclosing);
                (block, enclosing)
            }
            ASTNodeType::NN_CATCH_CLAUSE => {
                let block = self.open_block(b, scope, enclosing);
                // Trees that keep `catch_formal_parameter` declare it as a parameter node
                // (handled by `declare_param`); otherwise it is recovered from the tokens.
                if self
                    .ast
                    .first_child_of_type(self.bpa, node, ASTNodeType::NN_PARAM_DECL)
                    .is_none()
                {
                    self.declare_catch_parameter(b, node, block, enclosing);
                }
                (block, enclosing)
            }
            ASTNodeType::NN_INSTANCEOF_EXPR => {
                self.declare_pattern_variable(b, node, scope, enclosing);
                (scope, enclosing)
            }
            t if self.adapter.opens_block_scope(t) => {
                (self.open_block(b, scope, enclosing), enclosing)
            }
            _ => (scope, enclosing),
        }
    }

    fn open_block(&self, b: &mut SymbolTableBuilder, scope: u32, owner: u32) -> u32 {
        let block = b.create_scope(owner, scope, ScopeKind::Block);
        b.scope_graph.add_flags(block, ScopeFlags::ORDERED);
        block
    }

    // ── Types ────────────────────────────────────────────────────────────────

    fn declare_type(
        &self,
        b: &mut SymbolTableBuilder,
        node: u32,
        ty: ASTNodeType,
        scope: u32,
        enclosing: u32,
        types: &mut Vec<TypeBody>,
    ) -> (u32, u32) {
        let kind = self.adapter.symbol_kind(ty);
        let name_tok = self.name_token_of(node);
        let name_id = self.name_id_of(name_tok);
        let (ft, lt) = self.bpa.token_range(node);
        let scan = self.scan_modifiers(node, name_tok);
        let encl_kind = enclosing_decl_kind(b, enclosing);
        let visibility = scan
            .visibility
            .unwrap_or_else(|| self.adapter.default_visibility_in(kind, encl_kind));
        let modifiers = scan.modifiers
            | self.adapter.implicit_modifiers(
                kind,
                encl_kind,
                scan.modifiers,
                scan.visibility,
                true,
            );

        let header = name_tok.map(|t| parse_java_type_header(&self.tv, t, self.file_end()));
        let body_open = header.as_ref().and_then(|h| h.body_open);
        let body_close = body_open.and_then(|o| self.tv.matching_close(o, self.file_end()));
        let last = body_close.unwrap_or(self.tok(lt));

        let mut rec = new_record(
            kind,
            name_id,
            node,
            scope,
            visibility,
            modifiers,
            self.tok(ft),
            last,
        );
        if kind == SymbolKind::SK_RECORD {
            let components = self
                .ast
                .children_of_type(self.bpa, node, ASTNodeType::NN_PARAM_DECL)
                .count();
            rec.param_count = u16::try_from(components).unwrap_or(u16::MAX);
        }
        let sym = b.create_symbol(rec);

        let top_level = b.symbol_kind(enclosing) == Some(SymbolKind::SK_MODULE);
        let parent = if top_level && self.unit.package_sym != u32::MAX {
            self.unit.package_sym
        } else {
            enclosing
        };
        b.set_parent(sym, parent);
        let order = name_tok.unwrap_or(0);
        b.bind_name(scope, Namespace::Type, name_id, sym, order);
        if top_level {
            b.bind_name(self.unit.package_scope, Namespace::Type, name_id, sym, 0);
        }

        if let Some(t) = name_tok {
            let simple = self.tv.text_str(t);
            let fqn = if top_level {
                Some(if self.unit.package_name.is_empty() {
                    simple.to_string()
                } else {
                    format!("{}.{}", self.unit.package_name, simple)
                })
            } else if b.symbol_kind(enclosing).map(is_type_kind).unwrap_or(false) {
                b.qualified_name_of(enclosing)
                    .map(|outer| format!("{}.{}", outer, simple))
            } else {
                None
            };
            if let Some(fqn) = fqn {
                b.register_type_fqn(&fqn, sym);
            }
            define(b, t, node, sym);
        }

        let own = b.create_scope(sym, scope, ScopeKind::Class);
        b.set_owned_scope(sym, own);
        b.structural_types
            .push((sym, node, name_tok.unwrap_or(u32::MAX)));
        types.push(TypeBody {
            sym,
            node,
            kind,
            name_tok: name_tok.unwrap_or(u32::MAX),
            body_open,
            body_close,
        });
        (own, sym)
    }

    // ── Methods and constructors ─────────────────────────────────────────────

    fn declare_method(
        &self,
        b: &mut SymbolTableBuilder,
        node: u32,
        ty: ASTNodeType,
        scope: u32,
        enclosing: u32,
    ) -> (u32, u32) {
        let kind = if ty == ASTNodeType::NN_CONSTRUCTOR_DECL {
            SymbolKind::SK_CONSTRUCTOR
        } else {
            SymbolKind::SK_METHOD
        };
        let name_tok = self.name_token_of(node);
        let name_id = self.name_id_of(name_tok);
        let (ft, lt) = self.bpa.token_range(node);
        let params: Vec<u32> = self
            .ast
            .children_of_type(self.bpa, node, ASTNodeType::NN_PARAM_DECL)
            .collect();
        let body = self
            .ast
            .first_child_of_type(self.bpa, node, ASTNodeType::NN_BLOCK);
        // A constructor always has a body in Java (some trees drop `constructor_body`).
        let has_body = kind == SymbolKind::SK_CONSTRUCTOR || body.is_some();
        // Compact canonical constructor of a record (JLS 8.10.4.2): no parameter list, the
        // body follows the name, and its parameters are the record components.
        let compact_body_open = if kind == SymbolKind::SK_CONSTRUCTOR && params.is_empty() {
            name_tok
                .and_then(|t| self.tv.next(t, self.file_end()))
                .filter(|&o| self.tv.punct(o, "{"))
        } else {
            None
        };
        let param_count = if compact_body_open.is_some() {
            let record = self.ast.parent(node);
            self.ast
                .children_of_type(self.bpa, record, ASTNodeType::NN_PARAM_DECL)
                .count()
        } else {
            params.len()
        };

        let scan = self.scan_modifiers(node, name_tok);
        let encl_kind = enclosing_decl_kind(b, enclosing);
        let visibility = scan
            .visibility
            .unwrap_or_else(|| self.adapter.default_visibility_in(kind, encl_kind));
        let mut modifiers = scan.modifiers
            | self.adapter.implicit_modifiers(
                kind,
                encl_kind,
                scan.modifiers,
                scan.visibility,
                has_body,
            );
        if params
            .last()
            .map(|&p| self.is_spread_parameter(p))
            .unwrap_or(false)
        {
            modifiers |= SymbolModifiers::VARARGS;
        }

        let last = match (body, compact_body_open) {
            (Some(block), _) => {
                let open = self.bpa.token_range(block).0;
                self.tv
                    .matching_close(open, self.file_end())
                    .unwrap_or(self.tok(lt))
            }
            (None, Some(open)) => self
                .tv
                .matching_close(open, self.file_end())
                .unwrap_or(self.tok(lt)),
            (None, None) if kind == SymbolKind::SK_CONSTRUCTOR => name_tok
                .and_then(|t| self.constructor_body_close(t))
                .unwrap_or(self.tok(lt)),
            (None, None) => self.tok(lt),
        };

        let mut rec = new_record(
            kind,
            name_id,
            node,
            scope,
            visibility,
            modifiers,
            self.tok(ft),
            last,
        );
        rec.param_count = u16::try_from(param_count).unwrap_or(u16::MAX);
        let sym = b.create_symbol(rec);
        b.set_parent(sym, enclosing);
        b.bind_name(
            scope,
            Namespace::Method,
            name_id,
            sym,
            name_tok.unwrap_or(0),
        );

        if kind == SymbolKind::SK_METHOD {
            if let Some(t) = name_tok {
                if let Some(span) = self.method_return_span(&scan, t) {
                    b.type_spans.insert(sym, span);
                }
            }
        }
        if let Some(t) = name_tok {
            define(b, t, node, sym);
        }
        let own = b.create_scope(sym, scope, ScopeKind::Method);
        b.set_owned_scope(sym, own);
        (own, sym)
    }

    /// Return type span of a method: after modifiers, type parameters and their annotations.
    fn method_return_span(&self, scan: &ModifierScan, name_tok: u32) -> Option<TypeSpan> {
        let mut pos = scan.end;
        let lt = self.tv.sig(pos, name_tok)?;
        if self.tv.punct(lt, "<") {
            let mut parser = TypeParser::new(&self.tv, name_tok, Dialect::Java);
            let (_, after, _) = parser.parse_type_params(lt)?;
            pos = after;
        }
        while let Some(at) = self.tv.sig(pos, name_tok) {
            if !self.tv.punct(at, "@") {
                break;
            }
            let (_, next) = skip_annotation(&self.tv, at, name_tok)?;
            pos = next;
        }
        self.type_span(pos, name_tok, 0)
    }

    /// Closing brace of a constructor body, located from the tokens after its name (Phase 2
    /// drops `constructor_body`).
    fn constructor_body_close(&self, name_tok: u32) -> Option<u32> {
        let limit = self.file_end();
        let open = self.tv.next(name_tok, limit)?;
        let close = self.tv.matching_close(open, limit)?;
        let mut pos = close + 1;
        loop {
            let t = self.tv.sig(pos, limit)?;
            if self.tv.punct(t, "{") {
                return self.tv.matching_close(t, limit);
            }
            if self.tv.punct(t, ";") || self.tv.is_closer(t) {
                return None;
            }
            pos = t + 1;
        }
    }

    fn is_spread_parameter(&self, param: u32) -> bool {
        if self
            .ast
            .first_child_of_type(self.bpa, param, ASTNodeType::NN_LOCAL_VAR_DECL)
            .is_some()
        {
            return true;
        }
        let (ft, lt) = self.bpa.token_range(param);
        if !self.tv.valid(ft) {
            return false;
        }
        let end = lt.saturating_add(1).min(self.file_end());
        (ft..end).any(|t| self.tv.punct(t, "..."))
    }

    // ── Variables and parameters ─────────────────────────────────────────────

    fn declare_variable(
        &self,
        b: &mut SymbolTableBuilder,
        declarator: u32,
        declaration: Option<u32>,
        kind: SymbolKind,
        scope: u32,
        enclosing: u32,
    ) -> (u32, u32) {
        let name_tok = self.name_token_of(declarator);
        let name_id = self.name_id_of(name_tok);
        let decl = declaration.unwrap_or(declarator);
        let first_name_tok = declaration
            .and_then(|d| {
                self.ast
                    .first_child_of_type(self.bpa, d, ASTNodeType::NN_LOCAL_VAR_DECL)
            })
            .and_then(|first| self.name_token_of(first))
            .or(name_tok);
        let scan = self.scan_modifiers(decl, first_name_tok);
        let encl_kind = enclosing_decl_kind(b, enclosing);
        let visibility = if kind == SymbolKind::SK_FIELD {
            scan.visibility
                .unwrap_or_else(|| self.adapter.default_visibility_in(kind, encl_kind))
        } else {
            SymbolVisibility::Package
        };
        let modifiers = scan.modifiers
            | self.adapter.implicit_modifiers(
                kind,
                encl_kind,
                scan.modifiers,
                scan.visibility,
                false,
            );

        let declarator_count = declaration
            .map(|d| {
                self.ast
                    .children_of_type(self.bpa, d, ASTNodeType::NN_LOCAL_VAR_DECL)
                    .count()
            })
            .unwrap_or(1);
        let (ft, lt) = if declarator_count == 1 {
            self.bpa.token_range(decl)
        } else {
            self.bpa.token_range(declarator)
        };

        let rec = new_record(
            kind,
            name_id,
            declarator,
            scope,
            visibility,
            modifiers,
            self.tok(ft),
            self.tok(lt),
        );
        let sym = b.create_symbol(rec);
        b.set_parent(sym, enclosing);
        b.bind_name(
            scope,
            Namespace::Variable,
            name_id,
            sym,
            name_tok.unwrap_or(0),
        );

        if let (Some(t), Some(first_t)) = (name_tok, first_name_tok) {
            let (extra_dims, _, _) = parse_trailing_dims(&self.tv, t + 1, self.file_end());
            if scan.end != u32::MAX {
                if let Some(span) = self.type_span(scan.end, first_t, extra_dims) {
                    b.type_spans.insert(sym, span);
                }
            }
            define(b, t, declarator, sym);
        }
        (scope, sym)
    }

    fn declare_param(
        &self,
        b: &mut SymbolTableBuilder,
        node: u32,
        parent_ty: ASTNodeType,
        scope: u32,
        enclosing: u32,
    ) -> (u32, u32) {
        let spread_declarator =
            self.ast
                .first_child_of_type(self.bpa, node, ASTNodeType::NN_LOCAL_VAR_DECL);
        let name_tok = self
            .name_token_of(node)
            .or_else(|| spread_declarator.and_then(|d| self.name_token_of(d)));
        let name_id = self.name_id_of(name_tok);
        let scan = self.scan_modifiers(node, name_tok);
        let mut modifiers = scan.modifiers;
        if self.is_spread_parameter(node) {
            modifiers |= SymbolModifiers::VARARGS;
        }
        let record_component = parent_ty == ASTNodeType::NN_RECORD_DECL;
        if record_component {
            modifiers |= SymbolModifiers::RECORD_COMPONENT;
        }
        // An exception parameter (JLS 14.20) is a local of its catch block, not a parameter
        // of the enclosing method.
        let catch_parameter = parent_ty == ASTNodeType::NN_CATCH_CLAUSE;
        let kind = if catch_parameter {
            SymbolKind::SK_LOCAL_VAR
        } else {
            SymbolKind::SK_PARAM
        };
        let (ft, lt) = self.bpa.token_range(node);
        let rec = new_record(
            kind,
            name_id,
            node,
            scope,
            SymbolVisibility::Package,
            modifiers,
            self.tok(ft),
            self.tok(lt),
        );
        let sym = b.create_symbol(rec);
        b.set_parent(sym, enclosing);
        // Record components are reached through the implicit fields synthesized for them.
        if !record_component {
            b.bind_name(
                scope,
                Namespace::Variable,
                name_id,
                sym,
                name_tok.unwrap_or(0),
            );
        }
        if let Some(t) = name_tok {
            if scan.end != u32::MAX {
                let span = if catch_parameter {
                    self.union_type_span(scan.end, t)
                } else {
                    self.type_span(scan.end, t, 0)
                };
                if let Some(span) = span {
                    b.type_spans.insert(sym, span);
                }
            }
            define(b, t, node, sym);
        }
        (scope, sym)
    }

    /// Type span of a catch parameter, split into the alternatives of a union type.
    fn union_type_span(&self, start: u32, name_tok: u32) -> Option<TypeSpan> {
        let mut span = self.type_span(start, name_tok, 0)?;
        let mut parts = Vec::new();
        let mut part_start = span.first;
        let mut pos = span.first;
        while let Some(t) = self.tv.sig(pos, span.last.saturating_add(1)) {
            if self.tv.punct(t, "|") {
                let part_last = self.tv.prev(t, part_start)?;
                parts.push((part_start, part_last));
                part_start = self.tv.next(t, name_tok)?;
            } else if self.tv.punct(t, "<") {
                // Type arguments may not contain `|`; skip them as a unit.
                let mut parser = TypeParser::new(&self.tv, name_tok, Dialect::Java);
                let (_, after) = parser.parse(part_start, TypeOptions::default())?;
                pos = after.max(t + 1);
                continue;
            }
            pos = t + 1;
        }
        if !parts.is_empty() {
            parts.push((part_start, span.last));
            span.union_parts = parts;
        }
        Some(span)
    }

    /// One SK_TYPE_PARAM per type parameter. A tree may keep one node per parameter (its
    /// tokens start at the parameter) or one node for the whole `<...>` list whose children
    /// were dropped (its tokens start at `<`); both are parsed from the tokens.
    fn declare_type_params(&self, b: &mut SymbolTableBuilder, node: u32, scope: u32, owner: u32) {
        let first = self.bpa.token_range(node).0;
        if !self.tv.valid(first) {
            return;
        }
        let mut parser = TypeParser::new(&self.tv, self.file_end(), Dialect::Java);
        let parsed = if self.tv.punct(first, "<") {
            parser.parse_type_params(first).map(|(params, _, _)| params)
        } else {
            parser.parse_type_param(first).map(|(param, _)| vec![param])
        };
        let lt = first;
        let params = match parsed {
            Some(params) => params,
            None => {
                b.add_diagnostic(
                    DiagnosticCode::MalformedDeclaration,
                    owner,
                    lt,
                    Vec::new(),
                    "type parameter list could not be parsed".to_string(),
                );
                return;
            }
        };
        for p in params {
            let name_id = self.tv.text_id(p.name_tok);
            let rec = new_record(
                SymbolKind::SK_TYPE_PARAM,
                name_id,
                node,
                scope,
                SymbolVisibility::Package,
                0,
                self.tok(p.first),
                self.tok(p.last),
            );
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
            define(b, p.name_tok, node, sym);
        }
    }

    fn declare_lambda(
        &self,
        b: &mut SymbolTableBuilder,
        node: u32,
        scope: u32,
        enclosing: u32,
    ) -> (u32, u32) {
        let (ft, lt) = self.bpa.token_range(node);
        let explicit = self
            .ast
            .children_of_type(self.bpa, node, ASTNodeType::NN_PARAM_DECL)
            .count();
        let rec = new_record(
            SymbolKind::SK_LAMBDA,
            u32::MAX,
            node,
            scope,
            SymbolVisibility::Package,
            0,
            self.tok(ft),
            self.tok(lt),
        );
        let sym = b.create_symbol(rec);
        b.set_parent(sym, enclosing);
        let own = b.create_scope(sym, scope, ScopeKind::Lambda);
        b.set_owned_scope(sym, own);

        let mut implicit: Vec<(u32, u32)> = Vec::new();
        if explicit == 0 && self.tv.valid(ft) {
            let first_child = self.ast.children(node).first().copied();
            let single = first_child.filter(|&c| {
                self.bpa.node_type(c) == ASTNodeType::NN_IDENTIFIER_EXPR && {
                    let t = self.bpa.token_range(c).0;
                    self.tv.valid(t)
                        && self
                            .tv
                            .next(t, self.file_end())
                            .is_some_and(|n| self.tv.punct(n, "->"))
                }
            });
            if let Some(c) = single {
                implicit.push((self.bpa.token_range(c).0, c));
            } else if let Some(names) = parse_inferred_lambda_params(&self.tv, ft, self.file_end())
            {
                implicit.extend(names.into_iter().map(|t| (t, u32::MAX)));
            }
        }
        for &(tok, ident_node) in &implicit {
            let name_id = self.tv.text_id(tok);
            let mut prec = new_record(
                SymbolKind::SK_PARAM,
                name_id,
                ident_node,
                own,
                SymbolVisibility::Package,
                0,
                tok,
                tok,
            );
            if ident_node == u32::MAX {
                prec.flags |= SymbolFlags::LEXICAL;
            }
            let p = b.create_symbol(prec);
            b.set_parent(p, sym);
            b.bind_name(own, Namespace::Variable, name_id, p, tok);
            define(b, tok, ident_node, p);
        }
        if let Some(l) = b.symbols.get_mut(sym as usize) {
            l.param_count = u16::try_from(explicit + implicit.len()).unwrap_or(u16::MAX);
        }
        (own, sym)
    }

    /// `new` token and closing body brace of the anonymous class declared by the instance
    /// creation expression `node`, if it declares one.
    fn anonymous_body(&self, node: u32) -> Option<(u32, u32)> {
        let (ft, lt) = self.bpa.token_range(node);
        if !self.tv.valid(ft) {
            return None;
        }
        let limit = self.file_end();
        let scan_end = lt.saturating_add(1).min(limit);
        let mut pos = ft;
        let new_tok = loop {
            let t = self.tv.sig(pos, scan_end)?;
            if self.tv.word(t, "new") {
                break t;
            }
            pos = if self.tv.is_opener(t) {
                self.tv.matching_close(t, limit)? + 1
            } else {
                t + 1
            };
        };
        let mut parser = TypeParser::new(&self.tv, limit, Dialect::Java);
        let (_, after_type) = parser.parse(new_tok + 1, TypeOptions::default())?;
        let open = self.tv.sig(after_type, limit)?;
        if !self.tv.punct(open, "(") {
            return None;
        }
        let close = self.tv.matching_close(open, limit)?;
        let brace = self.tv.next(close, limit)?;
        if !self.tv.punct(brace, "{") {
            return None;
        }
        let brace_close = self.tv.matching_close(brace, limit)?;
        Some((new_tok, brace_close))
    }

    fn declare_anonymous(
        &self,
        b: &mut SymbolTableBuilder,
        node: u32,
        new_tok: u32,
        close: u32,
        scope: u32,
        enclosing: u32,
    ) -> (u32, u32) {
        let (ft, _) = self.bpa.token_range(node);
        let rec = new_record(
            SymbolKind::SK_ANON_CLASS,
            u32::MAX,
            node,
            scope,
            SymbolVisibility::Package,
            SymbolModifiers::FINAL,
            self.tok(ft),
            close,
        );
        let sym = b.create_symbol(rec);
        b.set_parent(sym, enclosing);
        let own = b.create_scope(sym, scope, ScopeKind::Anon);
        b.set_owned_scope(sym, own);
        b.anonymous_classes.push((sym, node, new_tok));
        (own, sym)
    }

    fn declare_initializer(
        &self,
        b: &mut SymbolTableBuilder,
        node: u32,
        kind: SymbolKind,
        scope: u32,
        enclosing: u32,
    ) -> (u32, u32) {
        let (ft, lt) = self.bpa.token_range(node);
        let modifiers = if kind == SymbolKind::SK_STATIC_INIT {
            SymbolModifiers::STATIC
        } else {
            0
        };
        let rec = new_record(
            kind,
            u32::MAX,
            node,
            scope,
            SymbolVisibility::Package,
            modifiers,
            self.tok(ft),
            self.tok(lt),
        );
        let sym = b.create_symbol(rec);
        b.set_parent(sym, enclosing);
        let own = b.create_scope(sym, scope, ScopeKind::Block);
        b.scope_graph.add_flags(own, ScopeFlags::ORDERED);
        b.set_owned_scope(sym, own);
        (own, sym)
    }

    /// `for (T x : expr)`: the loop variable is the first identifier child of the statement.
    fn declare_for_each_variable(
        &self,
        b: &mut SymbolTableBuilder,
        node: u32,
        block: u32,
        enclosing: u32,
    ) {
        let ident =
            match self
                .ast
                .first_child_of_type(self.bpa, node, ASTNodeType::NN_IDENTIFIER_EXPR)
            {
                Some(i) => i,
                None => return,
            };
        let name_tok = self.bpa.token_range(ident).0;
        if !self.tv.valid(name_tok) {
            return;
        }
        let for_tok = self.bpa.token_range(node).0;
        let open = match self.tv.next(for_tok, name_tok) {
            Some(o) if self.tv.punct(o, "(") => o,
            _ => return,
        };
        let scan = scan_java_modifiers(&self.tv, open + 1, name_tok);
        let name_id = self.tv.text_id(name_tok);
        let first = self.tv.sig(open + 1, name_tok).unwrap_or(name_tok);
        let rec = new_record(
            SymbolKind::SK_LOCAL_VAR,
            name_id,
            ident,
            block,
            SymbolVisibility::Package,
            scan.modifiers,
            first,
            name_tok,
        );
        let sym = b.create_symbol(rec);
        b.set_parent(sym, enclosing);
        b.bind_name(block, Namespace::Variable, name_id, sym, name_tok);
        if let Some(span) = self.type_span(scan.end, name_tok, 0) {
            b.type_spans.insert(sym, span);
        }
        define(b, name_tok, ident, sym);
    }

    /// `catch (A | B e)`: Phase 2 drops `catch_formal_parameter`; the parameter is parsed from
    /// the tokens after `catch`.
    fn declare_catch_parameter(
        &self,
        b: &mut SymbolTableBuilder,
        node: u32,
        block: u32,
        enclosing: u32,
    ) {
        let catch_tok = self.bpa.token_range(node).0;
        if !self.tv.valid(catch_tok) {
            return;
        }
        let param = match parse_catch_parameter(&self.tv, catch_tok, self.file_end()) {
            Some(p) => p,
            None => return,
        };
        let name_id = self.tv.text_id(param.name_tok);
        let mut rec = new_record(
            SymbolKind::SK_LOCAL_VAR,
            name_id,
            u32::MAX,
            block,
            SymbolVisibility::Package,
            param.modifiers.modifiers,
            param.type_first,
            param.name_tok,
        );
        rec.flags |= SymbolFlags::LEXICAL;
        let sym = b.create_symbol(rec);
        b.set_parent(sym, enclosing);
        b.bind_name(block, Namespace::Variable, name_id, sym, param.name_tok);
        let union_parts: Vec<(u32, u32)> = if param.alternatives.len() > 1 {
            param
                .alternatives
                .iter()
                .map(|a| (a.first, a.last))
                .collect()
        } else {
            Vec::new()
        };
        b.type_spans.insert(
            sym,
            TypeSpan {
                first: param.type_first,
                last: param.type_last,
                dialect: Dialect::Java,
                extra_dims: 0,
                union_parts,
            },
        );
        define(b, param.name_tok, u32::MAX, sym);
    }

    /// `x instanceof T name`: the pattern variable is the identifier after the type.
    fn declare_pattern_variable(
        &self,
        b: &mut SymbolTableBuilder,
        node: u32,
        scope: u32,
        enclosing: u32,
    ) {
        let children = self.ast.children(node);
        if children.len() < 2 {
            return;
        }
        let ident = children[children.len() - 1];
        if self.bpa.node_type(ident) != ASTNodeType::NN_IDENTIFIER_EXPR {
            return;
        }
        let name_tok = self.bpa.token_range(ident).0;
        let (ft, _) = self.bpa.token_range(node);
        if !self.tv.valid(name_tok) || !self.tv.valid(ft) {
            return;
        }
        let kw = match (ft..name_tok)
            .rev()
            .find(|&t| self.tv.word(t, "instanceof"))
        {
            Some(k) => k,
            None => return,
        };
        let mut type_start = kw + 1;
        if let Some(t) = self.tv.sig(type_start, name_tok) {
            if self.tv.word(t, "final") {
                type_start = t + 1;
            }
        }
        let name_id = self.tv.text_id(name_tok);
        let rec = new_record(
            SymbolKind::SK_LOCAL_VAR,
            name_id,
            ident,
            scope,
            SymbolVisibility::Package,
            0,
            self.tv.sig(type_start, name_tok).unwrap_or(name_tok),
            name_tok,
        );
        let sym = b.create_symbol(rec);
        b.set_parent(sym, enclosing);
        b.bind_name(scope, Namespace::Variable, name_id, sym, name_tok);
        if let Some(span) = self.type_span(type_start, name_tok, 0) {
            b.type_spans.insert(sym, span);
        }
        define(b, name_tok, ident, sym);
    }

    /// Enum constant kept as a node by the tree (JLS 8.9.1).
    fn declare_enum_constant(
        &self,
        b: &mut SymbolTableBuilder,
        node: u32,
        scope: u32,
        enclosing: u32,
    ) -> (u32, u32) {
        let name_tok = self.name_token_of(node);
        let name_id = self.name_id_of(name_tok);
        let (ft, lt) = self.bpa.token_range(node);
        let visibility = self
            .adapter
            .default_visibility_in(SymbolKind::SK_ENUM_CONSTANT, Some(SymbolKind::SK_ENUM));
        let modifiers = self.adapter.implicit_modifiers(
            SymbolKind::SK_ENUM_CONSTANT,
            Some(SymbolKind::SK_ENUM),
            0,
            None,
            false,
        ) | self.scan_modifiers(node, name_tok).modifiers;
        let mut rec = new_record(
            SymbolKind::SK_ENUM_CONSTANT,
            name_id,
            node,
            scope,
            visibility,
            modifiers,
            self.tok(ft),
            self.tok(lt),
        );
        if b.symbol_kind(enclosing) == Some(SymbolKind::SK_ENUM) {
            rec.type_id = enclosing;
        }
        let sym = b.create_symbol(rec);
        b.set_parent(sym, enclosing);
        b.bind_name(
            scope,
            Namespace::Variable,
            name_id,
            sym,
            name_tok.unwrap_or(0),
        );
        if let Some(t) = name_tok {
            define(b, t, node, sym);
        }
        (scope, sym)
    }

    /// Constant-specific class body of an enum constant: an anonymous subclass of the enum.
    fn declare_constant_body(
        &self,
        b: &mut SymbolTableBuilder,
        node: u32,
        scope: u32,
        constant: u32,
    ) -> (u32, u32) {
        let (ft, lt) = self.bpa.token_range(node);
        let close = if self.tv.valid(ft) && self.tv.punct(ft, "{") {
            self.tv.matching_close(ft, self.file_end())
        } else {
            None
        };
        let rec = new_record(
            SymbolKind::SK_ANON_CLASS,
            u32::MAX,
            node,
            scope,
            SymbolVisibility::Package,
            SymbolModifiers::FINAL,
            self.tok(ft),
            close.unwrap_or(self.tok(lt)),
        );
        let sym = b.create_symbol(rec);
        b.set_parent(sym, constant);
        let own = b.create_scope(sym, scope, ScopeKind::Anon);
        b.set_owned_scope(sym, own);
        // The superclass is the enum itself; Pass 5 reads it from the constant's type.
        b.anonymous_classes.push((sym, node, u32::MAX));
        (own, sym)
    }

    /// Binding variable of a type pattern (`T name`, JLS 14.30.1).
    fn declare_pattern_binding(
        &self,
        b: &mut SymbolTableBuilder,
        node: u32,
        scope: u32,
        enclosing: u32,
    ) {
        let ident = match self.ast.children(node).last() {
            Some(&c) if self.bpa.node_type(c) == ASTNodeType::NN_IDENTIFIER_EXPR => c,
            _ => return,
        };
        if self.ast.children(node).len() < 2 || b.symbol_at_node(ident).is_some() {
            return;
        }
        let name_tok = self.bpa.token_range(ident).0;
        let (ft, _) = self.bpa.token_range(node);
        if !self.tv.valid(name_tok) || !self.tv.valid(ft) {
            return;
        }
        let mut type_start = ft;
        if self.tv.word(ft, "final") {
            type_start = ft + 1;
        }
        let name_id = self.tv.text_id(name_tok);
        let rec = new_record(
            SymbolKind::SK_LOCAL_VAR,
            name_id,
            ident,
            scope,
            SymbolVisibility::Package,
            0,
            ft,
            name_tok,
        );
        let sym = b.create_symbol(rec);
        b.set_parent(sym, enclosing);
        b.bind_name(scope, Namespace::Variable, name_id, sym, name_tok);
        if let Some(span) = self.type_span(type_start, name_tok, 0) {
            b.type_spans.insert(sym, span);
        }
        define(b, name_tok, ident, sym);
    }

    // ── Post-processing of type bodies ───────────────────────────────────────

    /// Members declared directly in a type node: (kind, name_id, param_count).
    fn declared_members(&self, b: &SymbolTableBuilder, node: u32) -> Vec<(SymbolKind, u32, u16)> {
        self.ast
            .children(node)
            .iter()
            .filter_map(|&c| b.symbol_at_node(c))
            .filter_map(|s| b.symbol(s))
            .map(|s| (SymbolKind::from(s.kind), s.name_id, s.param_count))
            .collect()
    }

    /// Enum constants (recovered from the body tokens), the implicit `final` (JLS 8.9) and the
    /// implicit `values()`/`valueOf(String)` methods (JLS 8.9.3).
    fn complete_enum(&self, b: &mut SymbolTableBuilder, t: &TypeBody) {
        let open = match t.body_open {
            Some(o) => o,
            None => return,
        };
        let own = b.owned_scope(t.sym);
        let constant_nodes: Vec<u32> = self
            .ast
            .children_of_type(self.bpa, t.node, ASTNodeType::NN_ENUM_CONSTANT)
            .collect();
        // Constants the tree kept were declared during the walk; otherwise Phase 2 dropped
        // them and they are recovered from the body tokens.
        let constants = if constant_nodes.is_empty() {
            parse_enum_constants(&self.tv, open, self.file_end())
        } else {
            Vec::new()
        };
        let mut any_body = constant_nodes.iter().any(|&c| {
            self.ast
                .first_child_of_type(self.bpa, c, ASTNodeType::NN_ANON_CLASS_BODY)
                .is_some()
        });
        for c in &constants {
            if self.tv.punct(c.last, "}") {
                any_body = true;
            }
            let name_id = self.tv.text_id(c.name_tok);
            let visibility = self
                .adapter
                .default_visibility_in(SymbolKind::SK_ENUM_CONSTANT, Some(SymbolKind::SK_ENUM));
            let modifiers = self.adapter.implicit_modifiers(
                SymbolKind::SK_ENUM_CONSTANT,
                Some(SymbolKind::SK_ENUM),
                0,
                None,
                false,
            );
            let mut rec = new_record(
                SymbolKind::SK_ENUM_CONSTANT,
                name_id,
                u32::MAX,
                own,
                visibility,
                modifiers,
                c.first,
                c.last,
            );
            rec.type_id = t.sym;
            rec.flags |= SymbolFlags::LEXICAL;
            let sym = b.create_symbol(rec);
            b.set_parent(sym, t.sym);
            b.bind_name(own, Namespace::Variable, name_id, sym, c.name_tok);
            define(b, c.name_tok, u32::MAX, sym);
        }
        if !any_body {
            if let Some(e) = b.symbols.get_mut(t.sym as usize) {
                e.modifiers |= SymbolModifiers::FINAL;
            }
        }

        // The implicit methods are named by corpus text ids; when no source token spells the
        // name, nothing in the corpus can refer to the method and it is not synthesized.
        let members = self.declared_members(b, t.node);
        for (name, params, array) in [("values", 0u16, true), ("valueOf", 1u16, false)] {
            let name_id = self.tv.find_text_id(name);
            if name_id == u32::MAX {
                continue;
            }
            if members
                .iter()
                .any(|&(k, n, p)| k == SymbolKind::SK_METHOD && n == name_id && p == params)
            {
                continue;
            }
            let mut rec = new_record(
                SymbolKind::SK_METHOD,
                name_id,
                u32::MAX,
                own,
                SymbolVisibility::Public,
                SymbolModifiers::STATIC | SymbolModifiers::SYNTHETIC,
                u32::MAX,
                u32::MAX,
            );
            rec.param_count = params;
            rec.type_id = t.sym;
            rec.flags |= SymbolFlags::IMPLICIT;
            if array {
                rec.flags |= SymbolFlags::ARRAY;
            }
            let sym = b.create_symbol(rec);
            b.set_parent(sym, t.sym);
            b.bind_name(own, Namespace::Method, name_id, sym, 0);
        }
    }

    /// Interface constants: Phase 2 drops `constant_declaration`, so the token runs of the
    /// interface body that no member node covers are parsed as field declarations.
    fn recover_interface_constants(&self, b: &mut SymbolTableBuilder, t: &TypeBody) {
        let (open, close) = match (t.body_open, t.body_close) {
            (Some(o), Some(c)) => (o, c),
            _ => return,
        };
        let mut covered: Vec<(u32, u32)> = Vec::new();
        for &c in self.ast.children(t.node) {
            let (ft, lt) = self.bpa.token_range(c);
            if !self.tv.valid(ft) || ft <= open {
                continue;
            }
            let last = b
                .symbol_at_node(c)
                .and_then(|s| b.symbol(s))
                .map(|s| s.last_token_id)
                .filter(|&l| l != u32::MAX)
                .unwrap_or(lt)
                .max(lt);
            covered.push((ft, last));
        }
        covered.sort_unstable();

        let mut gaps: Vec<(u32, u32)> = Vec::new();
        let mut cursor = open + 1;
        for &(s, e) in &covered {
            if s > cursor {
                gaps.push((cursor, s));
            }
            cursor = cursor.max(e.saturating_add(1));
        }
        if close > cursor {
            gaps.push((cursor, close));
        }

        let own = b.owned_scope(t.sym);
        for (start, end) in gaps {
            for decl in parse_field_declarations(&self.tv, start, end) {
                let visibility = decl.modifiers.visibility.unwrap_or_else(|| {
                    self.adapter
                        .default_visibility_in(SymbolKind::SK_FIELD, Some(SymbolKind::SK_INTERFACE))
                });
                let modifiers = decl.modifiers.modifiers
                    | self.adapter.implicit_modifiers(
                        SymbolKind::SK_FIELD,
                        Some(SymbolKind::SK_INTERFACE),
                        decl.modifiers.modifiers,
                        decl.modifiers.visibility,
                        false,
                    );
                let single = decl.declarators.len() == 1;
                for d in &decl.declarators {
                    let name_id = self.tv.text_id(d.name_tok);
                    let (first, last) = if single {
                        (decl.first, decl.last)
                    } else {
                        (d.first, d.last)
                    };
                    let mut rec = new_record(
                        SymbolKind::SK_FIELD,
                        name_id,
                        u32::MAX,
                        own,
                        visibility,
                        modifiers,
                        first,
                        last,
                    );
                    rec.flags |= SymbolFlags::LEXICAL;
                    let sym = b.create_symbol(rec);
                    b.set_parent(sym, t.sym);
                    b.bind_name(own, Namespace::Variable, name_id, sym, d.name_tok);
                    b.type_spans.insert(
                        sym,
                        TypeSpan {
                            first: decl.type_first,
                            last: decl.type_last,
                            dialect: Dialect::Java,
                            extra_dims: d.extra_dims,
                            union_parts: Vec::new(),
                        },
                    );
                    define(b, d.name_tok, u32::MAX, sym);
                }
            }
        }
    }

    /// Implicit record members (JLS 8.10.3): a private final field and a public accessor per
    /// component, and the canonical constructor unless one is declared explicitly.
    fn complete_record(&self, b: &mut SymbolTableBuilder, t: &TypeBody) {
        let own = b.owned_scope(t.sym);
        let components: Vec<u32> = self
            .ast
            .children_of_type(self.bpa, t.node, ASTNodeType::NN_PARAM_DECL)
            .filter_map(|p| b.symbol_at_node(p))
            .collect();
        let members = self.declared_members(b, t.node);
        let record_visibility = b
            .symbol(t.sym)
            .map(|s| SymbolVisibility::from(s.visibility))
            .unwrap_or(SymbolVisibility::Package);

        for &comp in &components {
            let (name_id, first, last) = match b.symbol(comp) {
                Some(s) => (s.name_id, s.first_token_id, s.last_token_id),
                None => continue,
            };
            let mut field = new_record(
                SymbolKind::SK_FIELD,
                name_id,
                u32::MAX,
                own,
                SymbolVisibility::Private,
                SymbolModifiers::FINAL | SymbolModifiers::SYNTHETIC,
                first,
                last,
            );
            field.flags |= SymbolFlags::IMPLICIT;
            let f = b.create_symbol(field);
            b.set_parent(f, t.sym);
            b.bind_name(own, Namespace::Variable, name_id, f, 0);
            b.implicit_type_sources.insert(f, comp);

            let explicit_accessor = members
                .iter()
                .any(|&(k, n, p)| k == SymbolKind::SK_METHOD && n == name_id && p == 0);
            if !explicit_accessor {
                let mut acc = new_record(
                    SymbolKind::SK_METHOD,
                    name_id,
                    u32::MAX,
                    own,
                    SymbolVisibility::Public,
                    SymbolModifiers::SYNTHETIC,
                    first,
                    last,
                );
                acc.flags |= SymbolFlags::IMPLICIT;
                let a = b.create_symbol(acc);
                b.set_parent(a, t.sym);
                b.bind_name(own, Namespace::Method, name_id, a, 0);
                b.implicit_type_sources.insert(a, comp);
            }
        }

        let arity = u16::try_from(components.len()).unwrap_or(u16::MAX);
        let explicit_canonical = members
            .iter()
            .any(|&(k, _, p)| k == SymbolKind::SK_CONSTRUCTOR && p == arity);
        if !explicit_canonical {
            let name_id = b.symbol(t.sym).map(|s| s.name_id).unwrap_or(u32::MAX);
            let mut ctor = new_record(
                SymbolKind::SK_CONSTRUCTOR,
                name_id,
                u32::MAX,
                own,
                record_visibility,
                SymbolModifiers::SYNTHETIC,
                self.tok(t.name_tok),
                self.tok(t.name_tok),
            );
            ctor.param_count = arity;
            ctor.flags |= SymbolFlags::IMPLICIT;
            let c = b.create_symbol(ctor);
            b.set_parent(c, t.sym);
            b.bind_name(own, Namespace::Method, name_id, c, 0);
        }
    }
}

// ── Heuristic front end for files with only a generic AST ────────────────────

/// Language keywords never taken as declaration names by the heuristic namer.
const HEURISTIC_KEYWORDS: &[&str] = &[
    "class",
    "interface",
    "object",
    "enum",
    "fun",
    "val",
    "var",
    "public",
    "private",
    "protected",
    "internal",
    "data",
    "sealed",
    "open",
    "abstract",
    "companion",
    "constructor",
    "package",
    "import",
    "override",
    "struct",
    "trait",
    "impl",
    "let",
    "const",
    "def",
    "fn",
    "new",
    "this",
    "super",
    "null",
    "true",
    "false",
    "void",
    "return",
    "if",
    "else",
    "for",
    "while",
    "do",
    "switch",
    "case",
    "break",
    "continue",
    "try",
    "catch",
    "finally",
    "throw",
    "by",
    "as",
    "in",
    "is",
    "default",
    "export",
    "declare",
    "async",
    "yield",
    "await",
    "function",
    "static",
    "final",
    "readonly",
    "pub",
    "get",
    "set",
    "extends",
    "implements",
    "type",
    "record",
];

struct HeuristicWalk<'a, 'u> {
    bpa: &'a BPASTArtifact,
    ast: &'u AstIndex,
    tv: TokenView<'a>,
    adapter: &'u dyn SemanticAdapter,
    unit: &'u UnitInfo,
}

impl<'a, 'u> HeuristicWalk<'a, 'u> {
    fn run(&self, b: &mut SymbolTableBuilder) {
        let root = self.unit.root_node;
        let end = self.ast.subtree_end(root);
        for node in (root + 1)..end {
            let parent = self.ast.parent(node);
            let (scope, enclosing) = if parent == u32::MAX {
                (self.unit.cu_scope, self.unit.module_sym)
            } else {
                (b.scope_of_node(parent), b.enclosing_symbol_of_node(parent))
            };
            let ty = self.bpa.node_type(node);
            if !self.adapter.is_declaration(ty) || ty == ASTNodeType::NN_MODULE {
                b.set_node_context(node, scope, enclosing);
                continue;
            }
            let kind = self.adapter.symbol_kind(ty);
            let (ft, lt) = self.bpa.token_range(node);
            let name_tok = self.heuristic_name(node, ty);
            let name_id = name_tok.map(|t| self.tv.text_id(t)).unwrap_or(u32::MAX);
            let mut rec = new_record(
                kind,
                name_id,
                node,
                scope,
                SymbolVisibility::Public,
                0,
                if self.tv.valid(ft) { ft } else { u32::MAX },
                if self.tv.valid(lt) { lt } else { u32::MAX },
            );
            rec.flags |= SymbolFlags::HEURISTIC_NAME;
            if matches!(
                kind,
                SymbolKind::SK_METHOD | SymbolKind::SK_CONSTRUCTOR | SymbolKind::SK_LAMBDA
            ) {
                rec.param_count = u16::try_from(
                    self.ast
                        .children_of_type(self.bpa, node, ASTNodeType::NN_PARAM_DECL)
                        .count(),
                )
                .unwrap_or(u16::MAX);
            }
            let sym = b.create_symbol(rec);
            let parent_sym = if b.symbol_kind(enclosing) == Some(SymbolKind::SK_MODULE)
                && self.unit.package_sym != u32::MAX
                && is_type_kind(kind)
            {
                self.unit.package_sym
            } else {
                enclosing
            };
            b.set_parent(sym, parent_sym);
            let ns = match kind {
                k if is_type_kind(k) => Some(Namespace::Type),
                SymbolKind::SK_METHOD | SymbolKind::SK_CONSTRUCTOR => Some(Namespace::Method),
                SymbolKind::SK_FIELD | SymbolKind::SK_PARAM | SymbolKind::SK_LOCAL_VAR => {
                    Some(Namespace::Variable)
                }
                _ => None,
            };
            if let Some(ns) = ns {
                b.bind_name(scope, ns, name_id, sym, name_tok.unwrap_or(0));
            }
            if let Some(t) = name_tok {
                define(b, t, node, sym);
            }
            let own = b.create_scope(sym, scope, self.adapter.scope_kind(ty));
            b.set_owned_scope(sym, own);
            b.set_node_context(node, own, sym);
        }
    }

    fn is_name_candidate(&self, tok: u32) -> bool {
        let text = self.tv.text_str(tok);
        let first = match text.as_bytes().first() {
            Some(&c) => c,
            None => return false,
        };
        (first.is_ascii_alphabetic() || first == b'_' || first == b'$')
            && !matches!(
                self.tv.kind(tok),
                crate::core::types::token::TokenType::StringLiteral
                    | crate::core::types::token::TokenType::CharLiteral
                    | crate::core::types::token::TokenType::CommentLine
                    | crate::core::types::token::TokenType::CommentBlock
                    | crate::core::types::token::TokenType::CommentDoc
                    | crate::core::types::token::TokenType::Annotation
            )
            && !self.tv.prev(tok, 0).is_some_and(|p| self.tv.punct(p, "@"))
            && !HEURISTIC_KEYWORDS.contains(&text)
    }

    /// Token-window name heuristic: identifier after the type keyword, last identifier before
    /// `(` for functions, last identifier before `=`/`;`/`:` for variables, otherwise the first
    /// identifier of the node.
    fn heuristic_name(&self, node: u32, ty: ASTNodeType) -> Option<u32> {
        let (ft, lt) = self.bpa.token_range(node);
        if !self.tv.valid(ft) {
            return None;
        }
        let end = lt
            .min(self.tv.len().saturating_sub(1))
            .min(ft.saturating_add(24));
        let file = self.tv.file(ft);
        let toks: Vec<u32> = (ft..=end)
            .filter(|&t| !self.tv.is_trivia(t) && self.tv.file(t) == file)
            .collect();
        match ty {
            ASTNodeType::NN_CLASS_DECL
            | ASTNodeType::NN_INTERFACE_DECL
            | ASTNodeType::NN_ENUM_DECL
            | ASTNodeType::NN_RECORD_DECL => {
                let kw = toks.iter().position(|&t| {
                    matches!(
                        self.tv.text(t),
                        b"class"
                            | b"interface"
                            | b"enum"
                            | b"struct"
                            | b"trait"
                            | b"record"
                            | b"object"
                    )
                })?;
                toks[kw + 1..]
                    .iter()
                    .copied()
                    .find(|&t| self.is_name_candidate(t))
            }
            ASTNodeType::NN_METHOD_DECL | ASTNodeType::NN_CONSTRUCTOR_DECL => {
                let mut candidate = None;
                for &t in &toks {
                    if self.tv.punct(t, "(") || self.tv.punct(t, "{") || self.tv.punct(t, ";") {
                        break;
                    }
                    if self.is_name_candidate(t) {
                        candidate = Some(t);
                    }
                }
                candidate
            }
            ASTNodeType::NN_FIELD_DECL | ASTNodeType::NN_LOCAL_VAR_DECL => {
                let mut candidate = None;
                for &t in &toks {
                    if self.tv.punct(t, "=") || self.tv.punct(t, ";") || self.tv.punct(t, ":") {
                        break;
                    }
                    if self.is_name_candidate(t) {
                        candidate = Some(t);
                    }
                }
                candidate
            }
            _ => toks.iter().copied().find(|&t| self.is_name_candidate(t)),
        }
    }
}
