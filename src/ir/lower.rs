//! Lowering of the BP AST and the symbol table into the IR.
//!
//! Lowering works the way a compiler front end does:
//! 1. *Discovery* walks the declarations of every compilation unit (never function bodies)
//!    and reserves one function per body: methods, constructors, the per-type static and
//!    instance initialisers (field initialisers and initializer blocks in source order) and
//!    the per-file initialiser of module-level variables and statements.
//! 2. Each body is lowered with lexical name resolution (locals, then members of the
//!    enclosing type and its supertypes, then enclosing functions and types, then
//!    imports), typed declarations, and on-the-fly SSA construction. Expressions become
//!    three-address code; short-circuit and conditional operators become branches; memory
//!    accesses become explicit `Load`/`Store` on places; object creation becomes `Alloc`
//!    plus a constructor call; calls are bound by dispatch kind.
//! 3. Lambdas, local and anonymous classes are lowered as separate functions while their
//!    enclosing function is being lowered, so captured variables are known exactly.
//! 4. Structured exits (`break`, `continue`, `return`, `throw`, `goto`) run the cleanups
//!    they cross — `finally` blocks, resource releases, lock releases, deferred statements,
//!    scope-exit destructors — on every path, including the exceptional ones.
//!
//! What lowering cannot derive is never filled in by assumption: it is lowered to an
//! explicit not-derivable form (`IntrinsicKind::is_not_derivable`, `Callee::Unresolved`,
//! `IrType::Unknown`) and recorded in `ModuleIr::diagnostics`.
//!
//! Implicit exceptions raised by non-call instructions (null dereference, arithmetic,
//! bounds, failed casts) are not represented as unwind edges; each function with handlers
//! carries a diagnostic stating so.

mod decls;
mod expr;
mod frame;
pub mod resolve;
pub mod semantics;
mod stmt;
pub mod syntax;
mod typing;

pub use semantics::{
    CaptureMode, ConstructionRules, DispatchRule, FieldInitOrder, IterationStep, Literal,
    LiteralClass, LiteralValue, LoweringSemantics, NoSemantics, Punctuation, ResourceProtocol,
    SemanticsTable,
};
pub use syntax::NodeClass;

use std::collections::{HashMap, HashSet, VecDeque};

use crate::ast::BPASTArtifact;
use crate::core::types::ast::NodeAttr;
use crate::core::types::token::LangId;
use crate::ingestion::serializer::TokenCorpusArtifact;
use crate::ir::builder::FinishedFunction;
use crate::ir::func::{FuncIr, FuncKind};
use crate::ir::inst::Inst;
use crate::ir::module::{
    AllocSiteIr, CallSiteIr, DiagnosticKind, FieldIr, GlobalIr, IrDiagnostic, ModuleIr,
};
use crate::ir::types::{
    AllocSiteId, BlockId, CallSiteId, FieldId, FuncId, GlobalId, Span, TypeId, TypeTable,
};
use crate::symbol::serializer::SymbolTableArtifact;

use frame::FnState;
use resolve::SymbolIndex;
use syntax::{Syntax, Tree};

/// Inputs of lowering: the AST, the token corpus it indexes, the symbol table built over
/// them, and the semantics of every language present.
pub struct LowerInput<'a> {
    pub bpa: &'a BPASTArtifact,
    pub tca: &'a TokenCorpusArtifact,
    pub sta: &'a SymbolTableArtifact,
    pub semantics: &'a SemanticsTable<'a>,
}

/// Lowers the whole program.
pub fn lower_module(input: &LowerInput<'_>) -> ModuleIr {
    let tree = Tree::build(input.bpa, input.tca);
    let index = SymbolIndex::build(input.sta, input.tca);
    let mut lowerer = Lowerer::new(&tree, &index, input.semantics);
    lowerer.discover();
    lowerer.run_jobs();
    lowerer.finish()
}

/// Initialisation work of a type or file, in source order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InitItem {
    /// Field declarator with an initializer.
    Field { declarator: u32, target: FieldTarget },
    /// Initializer block.
    Block(u32),
    /// Enum constant: an instance of the enum (or of the constant's body) stored in the
    /// constant's static field.
    EnumConstant { node: u32, field: FieldId },
    /// Executable statement at file level.
    Stmt(u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FieldTarget {
    Field(FieldId),
    Global(GlobalId),
}

/// Value captured by a local or anonymous class: stored into `field` of every instance at
/// creation, read through `this` by the class's functions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ClassCapture {
    /// Binding in the declaring frame.
    outer: frame::Binding,
    field: FieldId,
    /// The captured binding is a shared cell (the field holds the cell pointer).
    cell: bool,
}

/// A class declared inside a function body.
#[derive(Debug, Clone, PartialEq, Eq)]
struct LocalClass {
    /// Stack index of the frame that declares the class.
    decl_frame: usize,
    /// Function id of that frame.
    decl_func: FuncId,
    captures: Vec<ClassCapture>,
}

/// Lowering plan of one type declaration (or anonymous class body).
#[derive(Debug, Clone)]
struct ClassPlan {
    node: u32,
    symbol: Option<u32>,
    /// Plan of the lexically enclosing type.
    outer: Option<u32>,
    file: Option<u16>,
    instance_items: Vec<InitItem>,
    static_items: Vec<InitItem>,
    instance_init: Option<FuncId>,
    static_init: Option<FuncId>,
    implicit_ctor: Option<FuncId>,
    explicit_ctors: usize,
    local: Option<LocalClass>,
    /// Synthetic field holding the enclosing instance of an inner member type.
    outer_field: Option<FieldId>,
    /// For anonymous classes: the type instantiated by the creation expression.
    anon_base: Option<TypeId>,
    lowered: bool,
}

impl ClassPlan {
    fn new(node: u32, symbol: Option<u32>, outer: Option<u32>, file: Option<u16>) -> Self {
        Self {
            node,
            symbol,
            outer,
            file,
            instance_items: Vec::new(),
            static_items: Vec::new(),
            instance_init: None,
            static_init: None,
            implicit_ctor: None,
            explicit_ctors: 0,
            local: None,
            outer_field: None,
            anon_base: None,
            lowered: false,
        }
    }
}

/// Imports of a compilation unit, as qualified paths in the symbol table's notation.
#[derive(Debug, Clone, Default)]
struct FileImports {
    /// Simple name -> imported qualified type name.
    single: HashMap<Vec<u8>, String>,
    /// Packages (or types) imported on demand.
    on_demand: Vec<String>,
    /// Simple name -> qualified name of the type declaring a statically imported member.
    static_single: HashMap<Vec<u8>, String>,
    /// Types whose static members are imported on demand.
    static_on_demand: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JobKind {
    Body(u32),
    InstanceInit(u32),
    StaticInit(u32),
    ImplicitCtor(u32),
    ModuleInit(u16),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Job {
    func: FuncId,
    kind: JobKind,
}

pub(crate) struct Lowerer<'t> {
    tree: &'t Tree<'t>,
    index: &'t SymbolIndex<'t>,
    semantics: &'t SemanticsTable<'t>,
    module: ModuleIr,
    slots: Vec<Option<FinishedFunction>>,
    jobs: VecDeque<Job>,
    func_of_symbol: HashMap<u32, FuncId>,
    func_of_node: HashMap<u32, FuncId>,
    field_of_symbol: HashMap<u32, FieldId>,
    loose_fields: HashMap<(u32, u32), FieldId>,
    global_of_symbol: HashMap<u32, GlobalId>,
    global_of_node: HashMap<u32, GlobalId>,
    /// Plans keyed by type declaration node.
    classes: HashMap<u32, ClassPlan>,
    class_order: Vec<u32>,
    class_of_symbol: HashMap<u32, u32>,
    /// File-level initialisation items.
    module_items: HashMap<u16, Vec<InitItem>>,
    module_root: HashMap<u16, u32>,
    module_order: Vec<u16>,
    globals_by_name: HashMap<(u16, u32), GlobalId>,
    imports: HashMap<u16, FileImports>,
    lang_of_file: HashMap<u16, LangId>,
    type_of_node: HashMap<u32, TypeId>,
    type_of_symbol: HashMap<u32, TypeId>,
    ret_of_symbol: HashMap<u32, TypeId>,
    frames: Vec<FnState<'t>>,
    noted: HashSet<(u32, String)>,
}

impl<'t> Lowerer<'t> {
    fn new(
        tree: &'t Tree<'t>,
        index: &'t SymbolIndex<'t>,
        semantics: &'t SemanticsTable<'t>,
    ) -> Self {
        let mut module = ModuleIr::new(tree.tca.interner.clone());
        module.symbol_names = index
            .sta
            .symbol_records
            .iter()
            .map(|s| s.name_id)
            .collect();
        let lang_of_file = tree
            .tca
            .file_records
            .iter()
            .map(|r| (r.file_id, LangId::from(r.language_id)))
            .collect();
        Self {
            tree,
            index,
            semantics,
            module,
            slots: Vec::new(),
            jobs: VecDeque::new(),
            func_of_symbol: HashMap::new(),
            func_of_node: HashMap::new(),
            field_of_symbol: HashMap::new(),
            loose_fields: HashMap::new(),
            global_of_symbol: HashMap::new(),
            global_of_node: HashMap::new(),
            classes: HashMap::new(),
            class_order: Vec::new(),
            class_of_symbol: HashMap::new(),
            module_items: HashMap::new(),
            module_root: HashMap::new(),
            module_order: Vec::new(),
            globals_by_name: HashMap::new(),
            imports: HashMap::new(),
            lang_of_file,
            type_of_node: HashMap::new(),
            type_of_symbol: HashMap::new(),
            ret_of_symbol: HashMap::new(),
            frames: Vec::new(),
            noted: HashSet::new(),
        }
    }

    // ── Language and syntax access ──

    fn sem_of_file(&self, file: Option<u16>) -> &'t dyn LoweringSemantics {
        let lang = file
            .and_then(|f| self.lang_of_file.get(&f).copied())
            .unwrap_or(LangId::Unknown);
        self.semantics.get(lang)
    }

    fn sem_of_node(&self, node: u32) -> &'t dyn LoweringSemantics {
        self.sem_of_file(self.tree.node_file(node))
    }

    fn syn_for(&self, node: u32) -> Syntax<'t, 't> {
        Syntax::new(self.tree, self.sem_of_node(node))
    }

    /// Syntax view of the function being lowered.
    fn syn(&self) -> Syntax<'t, 't> {
        match self.frames.last() {
            Some(f) => Syntax::new(self.tree, f.sem),
            None => Syntax::new(self.tree, self.semantics.get(LangId::Unknown)),
        }
    }

    // ── Diagnostics ──

    fn current_func(&self) -> Option<FuncId> {
        self.frames.last().map(|f| f.func)
    }

    fn diag(&mut self, kind: DiagnosticKind, span: Span, message: String) {
        let func = self.current_func();
        self.module.diagnostics.push(IrDiagnostic {
            kind,
            func,
            span,
            message,
        });
    }

    /// Records a diagnostic once per function (for facts missing from the language
    /// frontend that would otherwise repeat at every site).
    fn diag_once(&mut self, kind: DiagnosticKind, span: Span, message: String) {
        let key = (self.current_func().map(|f| f.0).unwrap_or(u32::MAX), message.clone());
        if self.noted.insert(key) {
            self.diag(kind, span, message);
        }
    }

    fn name_text(&self, id: u32) -> String {
        String::from_utf8_lossy(self.module.strings.lookup_text(id)).into_owned()
    }

    fn intern(&mut self, text: &[u8]) -> u32 {
        self.module.strings.intern(text)
    }

    // ── Function reservation ──

    fn reserve(&mut self) -> FuncId {
        let id = FuncId::new(self.slots.len());
        self.slots.push(None);
        id
    }

    fn reserve_job(&mut self, kind: JobKind) -> FuncId {
        let id = self.reserve();
        self.jobs.push_back(Job { func: id, kind });
        id
    }

    /// Function reserved for a declaration node; registers the symbol mapping.
    fn reserve_body(&mut self, node: u32, queue: bool) -> FuncId {
        if let Some(&f) = self.func_of_node.get(&node) {
            return f;
        }
        let id = if queue {
            self.reserve_job(JobKind::Body(node))
        } else {
            self.reserve()
        };
        self.func_of_node.insert(node, id);
        if let Some(sym) = self.index.at_node(node) {
            self.func_of_symbol.entry(sym).or_insert(id);
        }
        id
    }

    fn store_function(&mut self, id: FuncId, finished: FinishedFunction) {
        if let Some(slot) = self.slots.get_mut(id.index()) {
            *slot = Some(finished);
        }
    }

    // ── Discovery ──

    fn discover(&mut self) {
        let roots = self.tree.roots();
        for root in roots {
            let file = self.tree.node_file(root);
            if let Some(f) = file {
                self.module_root.entry(f).or_insert(root);
                if !self.module_order.contains(&f) {
                    self.module_order.push(f);
                }
            }
            self.collect_imports(root, file);
            self.discover_members(root, None, file);
        }

        // Initialiser functions of every type and file with initialisation work.
        for &node in &self.class_order.clone() {
            let (has_instance, has_static) = match self.classes.get(&node) {
                Some(p) if p.local.is_none() => {
                    (!p.instance_items.is_empty(), !p.static_items.is_empty())
                }
                _ => continue,
            };
            if has_instance {
                let f = self.reserve_job(JobKind::InstanceInit(node));
                if let Some(p) = self.classes.get_mut(&node) {
                    p.instance_init = Some(f);
                }
            }
            if has_static {
                let f = self.reserve_job(JobKind::StaticInit(node));
                if let Some(p) = self.classes.get_mut(&node) {
                    p.static_init = Some(f);
                }
            }
        }
        for f in self.module_order.clone() {
            if self.module_items.get(&f).is_some_and(|v| !v.is_empty()) {
                self.reserve_job(JobKind::ModuleInit(f));
            }
        }
    }

    /// Walks the declarations under `node` (a compilation unit or a type body).
    fn discover_members(&mut self, node: u32, class: Option<u32>, file: Option<u16>) {
        let mut work: Vec<(u32, Option<u32>)> = vec![(node, class)];
        while let Some((n, owner)) = work.pop() {
            let syn = self.syn_for(n);
            let kids: Vec<u32> = syn.kids(n).to_vec();
            for c in kids {
                match syn.class(c) {
                    NodeClass::TypeDecl => {
                        self.plan_class(c, owner, file, None);
                        work.push((c, Some(c)));
                    }
                    NodeClass::Module => work.push((c, owner)),
                    NodeClass::Function => {
                        self.reserve_body(c, true);
                    }
                    NodeClass::Constructor => {
                        self.reserve_body(c, true);
                        if let Some(o) = owner {
                            if let Some(p) = self.classes.get_mut(&o) {
                                p.explicit_ctors += 1;
                            }
                        }
                    }
                    NodeClass::Initializer { is_static } => match owner {
                        Some(o) => {
                            if let Some(p) = self.classes.get_mut(&o) {
                                if is_static {
                                    p.static_items.push(InitItem::Block(c));
                                } else {
                                    p.instance_items.push(InitItem::Block(c));
                                }
                            }
                        }
                        None => self.add_module_item(file, InitItem::Stmt(c)),
                    },
                    NodeClass::Field | NodeClass::LocalDecl => {
                        self.discover_variables(c, owner, file);
                    }
                    NodeClass::EnumConstant => {
                        if let Some(o) = owner {
                            let field = self.field_for_declaration(c, o);
                            if let Some(p) = self.classes.get_mut(&o) {
                                p.static_items.push(InitItem::EnumConstant { node: c, field });
                            }
                        }
                    }
                    cl if owner.is_none() && (cl.is_statement() || cl.is_expression()) => {
                        self.add_module_item(file, InitItem::Stmt(c));
                    }
                    _ => {}
                }
            }
        }
    }

    fn add_module_item(&mut self, file: Option<u16>, item: InitItem) {
        if let Some(f) = file {
            self.module_items.entry(f).or_default().push(item);
        }
    }

    /// Fields of a type, or module-level variables of a file.
    fn discover_variables(&mut self, decl: u32, owner: Option<u32>, file: Option<u16>) {
        let syn = self.syn_for(decl);
        let parts = syn.local_decl_parts(decl);
        let is_static = self.tree.modifiers(decl) & NodeAttr::MOD_STATIC != 0;
        for d in parts.declarators {
            let dp = syn.declarator_parts(d);
            match owner {
                Some(o) => {
                    let field = self.field_for_declaration(d, o);
                    let field_static = self.module.fields[field.index()].is_static || is_static;
                    if let Some(p) = self.classes.get_mut(&o) {
                        if dp.init.is_some() {
                            let item = InitItem::Field {
                                declarator: d,
                                target: FieldTarget::Field(field),
                            };
                            if field_static {
                                p.static_items.push(item);
                            } else {
                                p.instance_items.push(item);
                            }
                        }
                    }
                }
                None => {
                    let global = self.global_for_declaration(d, decl, file);
                    if dp.init.is_some() {
                        self.add_module_item(
                            file,
                            InitItem::Field {
                                declarator: d,
                                target: FieldTarget::Global(global),
                            },
                        );
                    }
                }
            }
        }
    }

    /// Registers a type declaration (or anonymous body) for lowering.
    fn plan_class(
        &mut self,
        node: u32,
        outer: Option<u32>,
        file: Option<u16>,
        local: Option<LocalClass>,
    ) -> u32 {
        if self.classes.contains_key(&node) {
            return node;
        }
        let symbol = self.index.at_node(node);
        let mut plan = ClassPlan::new(node, symbol, outer, file);
        plan.local = local;
        self.classes.insert(node, plan);
        self.class_order.push(node);
        if let Some(s) = symbol {
            self.class_of_symbol.entry(s).or_insert(node);
        }
        node
    }

    fn collect_imports(&mut self, root: u32, file: Option<u16>) {
        let Some(f) = file else { return };
        let syn = self.syn_for(root);
        let mut imports = FileImports::default();
        for &c in syn.kids(root) {
            if self.tree.node_type(c) != crate::core::types::ast::ASTNodeType::NN_IMPORT_DECL {
                continue;
            }
            let segments: Vec<&[u8]> = self
                .tree
                .identifier_tokens(c)
                .into_iter()
                .map(|t| self.tree.token_text(t))
                .collect();
            if segments.is_empty() {
                continue;
            }
            // The symbol table names qualified types with '.'-separated segments.
            let path = segments
                .iter()
                .map(|s| String::from_utf8_lossy(s).into_owned())
                .collect::<Vec<_>>()
                .join(".");
            let is_static = self.tree.has_aux(c, NodeAttr::AUX_IMPORT_STATIC);
            let on_demand = self.tree.has_aux(c, NodeAttr::AUX_IMPORT_ON_DEMAND);
            let last = segments[segments.len() - 1].to_vec();
            match (is_static, on_demand) {
                (false, false) => {
                    imports.single.insert(last, path);
                }
                (false, true) => imports.on_demand.push(path),
                (true, false) => {
                    let owner = segments[..segments.len() - 1]
                        .iter()
                        .map(|s| String::from_utf8_lossy(s).into_owned())
                        .collect::<Vec<_>>()
                        .join(".");
                    imports.static_single.insert(last, owner);
                }
                (true, true) => imports.static_on_demand.push(path),
            }
        }
        self.imports.insert(f, imports);
    }

    // ── Fields and globals ──

    /// Field of a declarator (or enum constant) declared in type plan `owner`.
    fn field_for_declaration(&mut self, decl: u32, owner: u32) -> FieldId {
        let syn = self.syn_for(decl);
        let symbol = self.index.at_node(decl).or_else(|| {
            self.tree
                .parent(decl)
                .and_then(|p| self.index.at_node(p))
                .filter(|&s| self.index.is_field(s))
        });
        // A declaration node shared by several declarators maps to the first one only; the
        // others are matched by name among the owner's fields.
        let name = match syn.class(decl) {
            NodeClass::EnumConstant => syn
                .first_kid_of(decl, NodeClass::Name)
                .and_then(|n| self.tree.text_id(n))
                .or_else(|| self.tree.text_id(decl)),
            _ => syn
                .declarator_parts(decl)
                .name_token
                .and_then(|t| self.tree.token_text_id(t)),
        }
        .unwrap_or(u32::MAX);
        let owner_symbol = self.classes.get(&owner).and_then(|p| p.symbol);
        let symbol = match (symbol, owner_symbol) {
            (Some(s), _) if self.index.name(s) == name || name == u32::MAX => Some(s),
            (_, Some(o)) => self
                .index
                .members(o)
                .iter()
                .copied()
                .find(|&m| self.index.is_field(m) && self.index.name(m) == name),
            _ => None,
        };
        if let Some(s) = symbol {
            if let Some(&f) = self.field_of_symbol.get(&s) {
                return f;
            }
        }
        let decl_static = self
            .tree
            .parent(decl)
            .map(|p| self.tree.modifiers(p) & NodeAttr::MOD_STATIC != 0)
            .unwrap_or(false)
            || self.tree.modifiers(decl) & NodeAttr::MOD_STATIC != 0
            || syn.class(decl) == NodeClass::EnumConstant;
        let is_static = symbol.map(|s| self.index.is_static(s)).unwrap_or(false) || decl_static;
        let ty = match symbol {
            Some(s) => self.symbol_type(s),
            None => self.declarator_type(decl),
        };
        let id = FieldId::new(self.module.fields.len());
        self.module.fields.push(FieldIr {
            id,
            owner_symbol: owner_symbol.unwrap_or(u32::MAX),
            name,
            ty,
            symbol,
            is_static,
            synthetic: false,
        });
        if let Some(s) = symbol {
            self.field_of_symbol.insert(s, id);
        } else if let Some(o) = owner_symbol {
            self.loose_fields.insert((o, name), id);
        }
        id
    }

    /// Field of a field symbol.
    fn field_for_symbol(&mut self, sym: u32) -> FieldId {
        if let Some(&f) = self.field_of_symbol.get(&sym) {
            return f;
        }
        let owner = self.index.enclosing_type(sym).unwrap_or(u32::MAX);
        let ty = self.symbol_type(sym);
        let id = FieldId::new(self.module.fields.len());
        self.module.fields.push(FieldIr {
            id,
            owner_symbol: owner,
            name: self.index.name(sym),
            ty,
            symbol: Some(sym),
            is_static: self.index.is_static(sym),
            synthetic: false,
        });
        self.field_of_symbol.insert(sym, id);
        id
    }

    /// Field accessed by name that no declaration binds (owner `u32::MAX` when unknown).
    fn loose_field(&mut self, owner: u32, name: u32, is_static: bool) -> FieldId {
        if let Some(&f) = self.loose_fields.get(&(owner, name)) {
            return f;
        }
        let id = FieldId::new(self.module.fields.len());
        self.module.fields.push(FieldIr {
            id,
            owner_symbol: owner,
            name,
            ty: TypeTable::UNKNOWN,
            symbol: None,
            is_static,
            synthetic: false,
        });
        self.loose_fields.insert((owner, name), id);
        id
    }

    /// Field synthesized by lowering (closure captures, enclosing-instance links).
    fn synthetic_field(&mut self, owner: Option<u32>, name: &[u8], ty: TypeId) -> FieldId {
        let name = self.intern(name);
        let id = FieldId::new(self.module.fields.len());
        self.module.fields.push(FieldIr {
            id,
            owner_symbol: owner.unwrap_or(u32::MAX),
            name,
            ty,
            symbol: None,
            is_static: false,
            synthetic: true,
        });
        id
    }

    fn global_for_declaration(&mut self, declarator: u32, decl: u32, file: Option<u16>) -> GlobalId {
        if let Some(&g) = self.global_of_node.get(&declarator) {
            return g;
        }
        let syn = self.syn_for(declarator);
        let name = syn
            .declarator_parts(declarator)
            .name_token
            .and_then(|t| self.tree.token_text_id(t))
            .unwrap_or(u32::MAX);
        let symbol = self
            .index
            .at_node(declarator)
            .or_else(|| self.index.at_node(decl))
            .filter(|&s| self.index.name(s) == name);
        if let Some(s) = symbol {
            if let Some(&g) = self.global_of_symbol.get(&s) {
                return g;
            }
        }
        let ty = match symbol {
            Some(s) => self.symbol_type(s),
            None => self.declarator_type(declarator),
        };
        let id = GlobalId::new(self.module.globals.len());
        self.module.globals.push(GlobalIr {
            id,
            symbol,
            name,
            ty,
            span: self.tree.span(declarator),
        });
        self.global_of_node.insert(declarator, id);
        if let Some(s) = symbol {
            self.global_of_symbol.insert(s, id);
        }
        if let Some(f) = file {
            self.globals_by_name.entry((f, name)).or_insert(id);
        }
        id
    }

    // ── Jobs ──

    fn run_jobs(&mut self) {
        while let Some(job) = self.jobs.pop_front() {
            match job.kind {
                JobKind::Body(node) => self.lower_body(job.func, node),
                JobKind::InstanceInit(class) => self.lower_initializer(job.func, class, false),
                JobKind::StaticInit(class) => self.lower_initializer(job.func, class, true),
                JobKind::ImplicitCtor(class) => self.lower_implicit_ctor(job.func, class),
                JobKind::ModuleInit(file) => self.lower_module_init(job.func, file),
            }
            debug_assert!(self.frames.is_empty(), "unbalanced lowering frames");
            self.frames.clear();
        }
    }

    // ── Finalisation ──

    fn finish(mut self) -> ModuleIr {
        let slots = std::mem::take(&mut self.slots);
        let mut module = self.module;
        for (i, slot) in slots.into_iter().enumerate() {
            let id = FuncId::new(i);
            let finished = match slot {
                Some(f) => f,
                None => {
                    // Reserved but never lowered: an internal inconsistency, stated rather
                    // than hidden.
                    module.diagnostics.push(IrDiagnostic {
                        kind: DiagnosticKind::NotDerivable,
                        func: Some(id),
                        span: Span::NONE,
                        message: "function was reserved but no body could be lowered".to_string(),
                    });
                    let mut f = FuncIr::new(id, u32::MAX, TypeTable::UNKNOWN, Span::NONE, FuncKind::Method);
                    f.blocks.push(crate::ir::func::BlockIr::new(BlockId(0)));
                    FinishedFunction {
                        func: f,
                        call_meta: Vec::new(),
                    }
                }
            };
            let FinishedFunction { mut func, call_meta } = finished;
            func.id = id;
            for block in func.blocks.iter_mut() {
                let bid = block.id;
                for (idx, inst) in block.insts.iter_mut().enumerate() {
                    let span = block.inst_spans.get(idx).copied().unwrap_or(Span::NONE);
                    match inst {
                        Inst::Call { site, args, .. } => {
                            let meta = call_meta.get(site.index()).cloned().unwrap_or_default();
                            let gid = CallSiteId::new(module.call_sites.len());
                            let receiver = if meta.receiver.is_some() {
                                args.first().and_then(|a| a.as_value())
                            } else {
                                None
                            };
                            module.call_sites.push(CallSiteIr {
                                id: gid,
                                func: id,
                                block: bid,
                                span,
                                receiver,
                                candidates: meta.candidates,
                            });
                            *site = gid;
                        }
                        Inst::Alloc {
                            site, ty, region, ..
                        } => {
                            let gid = AllocSiteId::new(module.alloc_sites.len());
                            module.alloc_sites.push(AllocSiteIr {
                                id: gid,
                                func: id,
                                ty: *ty,
                                span,
                                region: *region,
                            });
                            *site = gid;
                        }
                        _ => {}
                    }
                }
            }
            module.funcs.push(func);
        }
        module
    }
}
