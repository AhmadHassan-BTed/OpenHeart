//! Variable resolution and def/use extraction for SSA construction (§5.2.1).
//!
//! SSA is built over *variables*, not AST occurrences. For one target function this
//! module walks the AST of the function (and, for a lambda, of the enclosing functions up
//! to the first method boundary) with a lexical scope stack (JLS 6.3) and resolves every
//! identifier occurrence in expression position to the local variable or parameter it
//! denotes. Declarations are keyed by their symbol-table id when Phase 3 registered one
//! (`variable_declarator`, `formal_parameter`) and by `ORIG_NODE_TAG | name_node`
//! otherwise (enhanced-for variables, inferred lambda parameters, pattern bindings).
//!
//! Identifiers that do not resolve to a local (fields, types, packages, static members)
//! are listed in `unresolved`; they are not SSA variables. Variables of an enclosing
//! function used inside a lambda are *captured*: they are defined on entry to the lambda
//! and used where the lambda is created.
//!
//! Name comparison needs token text; without it only declarations produce definitions
//! and every use is reported as unresolved.

use crate::ast::BPASTArtifact;
use crate::cfg::ast_query::{
    children, declarator_name, is_nested_body, is_simple_assignment, is_type_decl,
    is_update_expression, param_name, subtree_end, ForEachParts,
};
use crate::cfg::builder::FunctionCFGData;
use crate::cfg::source::SourceText;
use crate::core::types::ast::ASTNodeType;
use crate::core::types::symbol::SymbolKind;
use crate::symbol::SymbolTableArtifact;
use std::collections::{HashMap, HashSet};

use ASTNodeType::*;

/// Tag marking an `orig_sym_id` that is an AST node (the declaring identifier) rather
/// than a symbol-table id.
pub const ORIG_NODE_TAG: u32 = 0x8000_0000;

/// How a definition arises.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum DefKind {
    /// Declaration (declarator, catch parameter, enhanced-for variable, pattern binding).
    Decl = 0,
    /// Formal parameter, defined on entry.
    Param = 1,
    /// Variable of an enclosing function captured by a lambda, defined on entry.
    Captured = 2,
    /// `x = e`.
    Assign = 3,
    /// `x op= e` (reads the previous value).
    Compound = 4,
    /// `x++`, `--x`, ... (reads the previous value).
    Update = 5,
}

/// A use or definition of a variable inside one statement, in evaluation order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VarEvent {
    Use {
        var: u32,
        node: u32,
    },
    Def {
        var: u32,
        /// Identifier (or declaration) node naming the defined variable.
        node: u32,
        /// Expression whose value is assigned, `u32::MAX` when there is none in the AST.
        value: u32,
        kind: DefKind,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Owner {
    /// Declared in an enclosing function of the target lambda.
    Outside,
    /// Declared in the target function itself.
    Target,
    /// Declared inside a lambda nested in the target.
    Nested,
}

#[derive(Debug, Clone)]
pub struct VarInfo {
    /// Symbol-table id, or `ORIG_NODE_TAG | name_node`.
    pub orig_id: u32,
    /// Interned name, when token text is available.
    pub name: Option<u32>,
    /// Declaration node (declarator, parameter, or the identifier itself).
    pub decl_node: u32,
    /// Identifier node naming the variable.
    pub name_node: u32,
    owner: Owner,
}

/// Variables of one function and the resolution of its identifier occurrences.
#[derive(Debug, Clone)]
pub struct FunctionVars {
    pub target: u32,
    pub vars: Vec<VarInfo>,
    /// Own formal parameters, in declaration order.
    pub params: Vec<u32>,
    /// Captured variables of enclosing functions, in first-use order.
    pub captured: Vec<u32>,
    /// Identifier occurrences in expression position that are not local variables.
    pub unresolved: Vec<u32>,
    pub names_available: bool,
    occ: HashMap<u32, u32>,
    decl_names: HashSet<u32>,
    nested_captures: HashMap<u32, Vec<u32>>,
}

/// Declaration node -> symbol id for locals and parameters registered by Phase 3.
pub fn local_symbol_index(sta: &SymbolTableArtifact) -> HashMap<u32, u32> {
    let mut map = HashMap::new();
    for sym in &sta.symbol_records {
        if (sym.kind == SymbolKind::SK_LOCAL_VAR as u8 || sym.kind == SymbolKind::SK_PARAM as u8)
            && sym.decl_node != u32::MAX
        {
            map.entry(sym.decl_node).or_insert(sym.symbol_id);
        }
    }
    map
}

/// Outermost function whose scope is visible from `target`: the chain of enclosing
/// lambdas up to (and including) the first method / constructor / initializer.
pub fn top_function(bpa: &BPASTArtifact, target: u32) -> u32 {
    if bpa.node_type(target) != NN_LAMBDA_EXPR {
        return target;
    }
    let mut top = target;
    let mut cur = target;
    loop {
        let p = bpa.parent(cur);
        if p == u32::MAX || p == cur {
            break;
        }
        let t = bpa.node_type(p);
        if is_type_decl(t) || t == NN_FIELD_DECL {
            break;
        }
        match t {
            NN_LAMBDA_EXPR => top = p,
            NN_METHOD_DECL | NN_CONSTRUCTOR_DECL | NN_JAVA_STATIC_INIT | NN_JAVA_INSTANCE_INIT => {
                top = p;
                break;
            }
            _ => {}
        }
        cur = p;
    }
    top
}

struct Walker<'w, 's> {
    bpa: &'w BPASTArtifact,
    src: Option<&'w SourceText<'s>>,
    sym_by_decl: &'w HashMap<u32, u32>,
    top: u32,
    target: u32,
    target_end: u32,
    /// name -> stack of variables currently bound to it.
    bindings: HashMap<u32, Vec<u32>>,
    /// Names bound in each open scope (unnamed variables are not bound).
    scopes: Vec<Vec<u32>>,
    out: FunctionVars,
    captured_set: HashSet<u32>,
}

impl<'w, 's> Walker<'w, 's> {
    #[inline]
    fn in_target(&self, node: u32) -> bool {
        node >= self.target && node < self.target_end
    }

    fn push_scope(&mut self) {
        self.scopes.push(Vec::new());
    }

    fn pop_scope(&mut self) {
        if let Some(names) = self.scopes.pop() {
            for n in names.into_iter().rev() {
                if let Some(stack) = self.bindings.get_mut(&n) {
                    stack.pop();
                }
            }
        }
    }

    fn declare(&mut self, decl_node: u32, name_node: u32, nested: Option<u32>) -> u32 {
        let name = self.src.and_then(|s| s.node_text_id(self.bpa, name_node));
        let orig_id = self
            .sym_by_decl
            .get(&decl_node)
            .copied()
            .unwrap_or(ORIG_NODE_TAG | name_node);
        let owner = if !self.in_target(name_node) {
            Owner::Outside
        } else if nested.is_some() {
            Owner::Nested
        } else {
            Owner::Target
        };
        let var = self.out.vars.len() as u32;
        self.out.vars.push(VarInfo {
            orig_id,
            name,
            decl_node,
            name_node,
            owner,
        });
        if let Some(n) = name {
            self.bindings.entry(n).or_default().push(var);
            if let Some(scope) = self.scopes.last_mut() {
                scope.push(n);
            }
        }
        if owner == Owner::Target {
            self.out.occ.insert(name_node, var);
            self.out.decl_names.insert(name_node);
        }
        var
    }

    fn lookup(&self, name: u32) -> Option<u32> {
        self.bindings.get(&name).and_then(|s| s.last().copied())
    }

    fn mark_captured(&mut self, var: u32) {
        if self.captured_set.insert(var) {
            self.out.captured.push(var);
        }
    }

    fn use_ident(&mut self, node: u32, nested: Option<u32>) {
        if !self.in_target(node) {
            return;
        }
        let var = self
            .src
            .and_then(|s| s.node_text_id(self.bpa, node))
            .and_then(|n| self.lookup(n));
        let var = match var {
            Some(v) => v,
            None => {
                if nested.is_none() {
                    self.out.unresolved.push(node);
                }
                return;
            }
        };
        let owner = self.out.vars[var as usize].owner;
        match nested {
            None => {
                self.out.occ.insert(node, var);
                if owner == Owner::Outside {
                    self.mark_captured(var);
                }
            }
            Some(lambda) => {
                if owner == Owner::Nested {
                    return;
                }
                let caps = self.out.nested_captures.entry(lambda).or_default();
                if !caps.contains(&var) {
                    caps.push(var);
                }
                if owner == Owner::Outside {
                    self.mark_captured(var);
                }
            }
        }
    }

    fn walk_children(&mut self, node: u32, nested: Option<u32>) {
        let mut cur = self.bpa.first_child(node);
        while let Some(c) = cur {
            self.walk(c, nested);
            cur = self.bpa.next_sibling(c);
        }
    }

    fn declare_params(&mut self, func: u32, nested: Option<u32>, record: bool) {
        let kids = children(self.bpa, func);
        let is_lambda = self.bpa.node_type(func) == NN_LAMBDA_EXPR;
        let last = kids.last().copied();
        for &c in &kids {
            let param = match self.bpa.node_type(c) {
                NN_PARAM_DECL => param_name(self.bpa, c).map(|n| (c, n)),
                // Inferred lambda parameters (`x -> ...`, `(a, b) -> ...`).
                NN_IDENTIFIER_EXPR if is_lambda && Some(c) != last => Some((c, c)),
                _ => None,
            };
            if let Some((decl, name)) = param {
                let v = self.declare(decl, name, nested);
                if record {
                    self.out.params.push(v);
                }
            }
        }
    }

    fn walk(&mut self, node: u32, nested: Option<u32>) {
        let bpa = self.bpa;
        let t = bpa.node_type(node);

        if node == self.target || (node == self.top && t != NN_LAMBDA_EXPR) {
            // The target itself, or the enclosing method of a target lambda: parameters
            // open the function scope; names, types and annotations of the declaration are
            // not variable references.
            self.push_scope();
            self.declare_params(node, nested, node == self.target);
            let kids = children(bpa, node);
            let last = kids.last().copied();
            for c in kids {
                let ct = bpa.node_type(c);
                let skip = if t == NN_LAMBDA_EXPR {
                    ct == NN_PARAM_DECL || (ct == NN_IDENTIFIER_EXPR && Some(c) != last)
                } else {
                    matches!(
                        ct,
                        NN_PARAM_DECL
                            | NN_IDENTIFIER_EXPR
                            | NN_TYPE_REF
                            | NN_TYPE_PARAM
                            | NN_ANNOTATION_USE
                    )
                };
                if !skip {
                    self.walk(c, nested);
                }
            }
            self.pop_scope();
            return;
        }

        match t {
            _ if is_type_decl(t) || t == NN_FIELD_DECL => {}
            NN_METHOD_DECL | NN_CONSTRUCTOR_DECL | NN_JAVA_STATIC_INIT | NN_JAVA_INSTANCE_INIT => {
                // Members of local / anonymous classes: separate functions.
            }
            NN_LAMBDA_EXPR => {
                let contains_target = node <= self.target && self.target < subtree_end(bpa, node);
                if !self.in_target(node) && !contains_target {
                    // A sibling lambda of an enclosing function: its declarations are not in
                    // scope at the target.
                    return;
                }
                let nested_here = if contains_target {
                    nested
                } else {
                    nested.or(Some(node))
                };
                self.push_scope();
                self.declare_params(node, nested_here, false);
                if let Some(body) = children(bpa, node).last().copied() {
                    let bt = bpa.node_type(body);
                    if bt != NN_PARAM_DECL {
                        self.walk(body, nested_here);
                    }
                }
                self.pop_scope();
            }
            NN_BLOCK | NN_FOR_STMT | NN_TRY_STMT | NN_SWITCH_STMT | NN_SWITCH_EXPR => {
                self.push_scope();
                self.walk_children(node, nested);
                self.pop_scope();
            }
            NN_ENHANCED_FOR => {
                let parts = ForEachParts::of(bpa, self.src, node);
                if let Some(it) = parts.iterable {
                    self.walk(it, nested);
                }
                self.push_scope();
                if let Some(v) = parts.var {
                    self.declare(v, v, nested);
                }
                if let Some(b) = parts.body {
                    self.walk(b, nested);
                }
                self.pop_scope();
            }
            NN_CATCH_CLAUSE => {
                self.push_scope();
                for c in children(bpa, node) {
                    match bpa.node_type(c) {
                        NN_PARAM_DECL => {
                            if let Some(n) = param_name(bpa, c) {
                                self.declare(c, n, nested);
                            }
                        }
                        NN_IDENTIFIER_EXPR => {
                            self.declare(c, c, nested);
                        }
                        NN_BLOCK => self.walk(c, nested),
                        _ => {}
                    }
                }
                self.pop_scope();
            }
            NN_LOCAL_VAR_DECL => match declarator_name(bpa, node) {
                Some(name) => {
                    // JLS 6.3: the scope of a local starts at its own declarator.
                    self.declare(node, name, nested);
                    for c in children(bpa, node) {
                        if c != name {
                            self.walk(c, nested);
                        }
                    }
                }
                None => self.walk_children(node, nested),
            },
            NN_PARAM_DECL => {
                if let Some(n) = param_name(bpa, node) {
                    self.declare(node, n, nested);
                }
            }
            NN_INSTANCEOF_EXPR => {
                let kids = children(bpa, node);
                for (i, &c) in kids.iter().enumerate() {
                    let is_binding = i > 0
                        && bpa.node_type(c) == NN_IDENTIFIER_EXPR
                        && bpa.node_type(kids[i - 1]) == NN_TYPE_REF;
                    if is_binding {
                        self.declare(c, c, nested);
                    } else {
                        self.walk(c, nested);
                    }
                }
            }
            NN_SWITCH_CASE => {
                let kids = children(bpa, node);
                for (i, &c) in kids.iter().enumerate() {
                    // Type pattern `case T name`.
                    let is_binding = i > 0
                        && bpa.node_type(c) == NN_IDENTIFIER_EXPR
                        && bpa.node_type(kids[i - 1]) == NN_TYPE_REF;
                    if is_binding {
                        self.declare(c, c, nested);
                    } else {
                        self.walk(c, nested);
                    }
                }
            }
            NN_CALL_EXPR => {
                // The method name is not a variable reference.
                let name = self
                    .src
                    .and_then(|s| crate::cfg::ast_query::call_name(bpa, s, node));
                for c in children(bpa, node) {
                    if Some(c) != name {
                        self.walk(c, nested);
                    }
                }
            }
            NN_FIELD_ACCESS | NN_METHOD_REF => {
                // `obj.field` / `obj::name`: only the qualifier can be a variable.
                let kids = children(bpa, node);
                if kids.len() >= 2 {
                    self.walk(kids[0], nested);
                } else if let Some(&only) = kids.first() {
                    if bpa.node_type(only) != NN_IDENTIFIER_EXPR {
                        self.walk(only, nested);
                    }
                }
            }
            NN_JAVA_LABELED_STMT => {
                for (i, c) in children(bpa, node).into_iter().enumerate() {
                    if i > 0 {
                        self.walk(c, nested);
                    }
                }
            }
            NN_BREAK_STMT | NN_CONTINUE_STMT | NN_ANNOTATION_USE | NN_TYPE_REF | NN_LITERAL
            | NN_TYPE_PARAM => {}
            NN_IDENTIFIER_EXPR => self.use_ident(node, nested),
            _ => self.walk_children(node, nested),
        }
    }
}

impl FunctionVars {
    /// Resolves the variables of the function declared at `target` (method, constructor,
    /// initializer or lambda node).
    pub fn build(
        bpa: &BPASTArtifact,
        src: Option<&SourceText<'_>>,
        sym_by_decl: &HashMap<u32, u32>,
        target: u32,
    ) -> Self {
        let top = top_function(bpa, target);
        let mut walker = Walker {
            bpa,
            src,
            sym_by_decl,
            top,
            target,
            target_end: subtree_end(bpa, target),
            bindings: HashMap::new(),
            scopes: Vec::new(),
            out: FunctionVars {
                target,
                vars: Vec::new(),
                params: Vec::new(),
                captured: Vec::new(),
                unresolved: Vec::new(),
                names_available: src.is_some(),
                occ: HashMap::new(),
                decl_names: HashSet::new(),
                nested_captures: HashMap::new(),
            },
            captured_set: HashSet::new(),
        };
        walker.walk(top, None);
        let mut out = walker.out;
        out.unresolved.sort_unstable();
        out.unresolved.dedup();
        out
    }

    pub fn var_count(&self) -> usize {
        self.vars.len()
    }

    /// Variable denoted by an identifier occurrence (use or declaring name).
    pub fn var_of_occurrence(&self, ident: u32) -> Option<u32> {
        self.occ.get(&ident).copied()
    }

    pub fn is_decl_name(&self, ident: u32) -> bool {
        self.decl_names.contains(&ident)
    }

    /// Variables captured by a lambda nested in this function, used at its creation.
    pub fn captures_of(&self, lambda: u32) -> &[u32] {
        self.nested_captures
            .get(&lambda)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    pub fn orig_id(&self, var: u32) -> u32 {
        self.vars
            .get(var as usize)
            .map(|v| v.orig_id)
            .unwrap_or(u32::MAX)
    }

    /// Definitions performed on entry: parameters (in order), then captured variables.
    /// Each item is `(var, defining node or u32::MAX, kind)`.
    pub fn entry_defs(&self) -> Vec<(u32, u32, DefKind)> {
        let mut out = Vec::with_capacity(self.params.len() + self.captured.len());
        for &p in &self.params {
            out.push((p, self.vars[p as usize].decl_node, DefKind::Param));
        }
        for &c in &self.captured {
            out.push((c, u32::MAX, DefKind::Captured));
        }
        out
    }

    /// Appends the uses and definitions performed by block statement `stmt`, in
    /// evaluation order. Lowered sub-expressions (evaluated in other blocks) and nested
    /// function bodies are not entered; a nested lambda contributes uses of the variables
    /// it captures.
    pub fn stmt_events(
        &self,
        stmt: u32,
        bpa: &BPASTArtifact,
        cfg: &FunctionCFGData,
        src: Option<&SourceText<'_>>,
        out: &mut Vec<VarEvent>,
    ) {
        self.events_of(stmt, stmt, bpa, cfg, src, out);
    }

    fn events_children(
        &self,
        node: u32,
        root: u32,
        bpa: &BPASTArtifact,
        cfg: &FunctionCFGData,
        src: Option<&SourceText<'_>>,
        out: &mut Vec<VarEvent>,
    ) {
        let mut cur = bpa.first_child(node);
        while let Some(c) = cur {
            self.events_of(c, root, bpa, cfg, src, out);
            cur = bpa.next_sibling(c);
        }
    }

    fn events_of(
        &self,
        node: u32,
        root: u32,
        bpa: &BPASTArtifact,
        cfg: &FunctionCFGData,
        src: Option<&SourceText<'_>>,
        out: &mut Vec<VarEvent>,
    ) {
        let t = bpa.node_type(node);
        if node != root {
            if cfg.is_lowered(node) {
                return;
            }
            if t == NN_LAMBDA_EXPR {
                for &v in self.captures_of(node) {
                    out.push(VarEvent::Use { var: v, node });
                }
                return;
            }
            if is_nested_body(t) {
                return;
            }
        }

        match t {
            NN_IDENTIFIER_EXPR => {
                if let Some(v) = self.var_of_occurrence(node) {
                    if self.is_decl_name(node) {
                        out.push(VarEvent::Def {
                            var: v,
                            node,
                            value: u32::MAX,
                            kind: DefKind::Decl,
                        });
                    } else {
                        out.push(VarEvent::Use { var: v, node });
                    }
                }
            }
            NN_LOCAL_VAR_DECL => match declarator_name(bpa, node) {
                Some(name) => {
                    let mut value = u32::MAX;
                    let mut after_name = false;
                    for c in children(bpa, node) {
                        if c == name {
                            after_name = true;
                            continue;
                        }
                        if after_name && value == u32::MAX {
                            value = c;
                        }
                        self.events_of(c, root, bpa, cfg, src, out);
                    }
                    if let Some(v) = self.var_of_occurrence(name) {
                        out.push(VarEvent::Def {
                            var: v,
                            node: name,
                            value,
                            kind: DefKind::Decl,
                        });
                    }
                }
                None => self.events_children(node, root, bpa, cfg, src, out),
            },
            NN_PARAM_DECL => {
                if let Some(name) = param_name(bpa, node) {
                    if let Some(v) = self.var_of_occurrence(name) {
                        out.push(VarEvent::Def {
                            var: v,
                            node: name,
                            value: u32::MAX,
                            kind: DefKind::Decl,
                        });
                    }
                }
            }
            NN_ASSIGN_EXPR => {
                let kids = children(bpa, node);
                if kids.len() != 2 {
                    self.events_children(node, root, bpa, cfg, src, out);
                    return;
                }
                let (lhs, rhs) = (kids[0], kids[1]);
                let target_var = if bpa.node_type(lhs) == NN_IDENTIFIER_EXPR {
                    self.var_of_occurrence(lhs)
                } else {
                    None
                };
                match target_var {
                    Some(v) => {
                        let compound = !is_simple_assignment(bpa, node);
                        if compound {
                            out.push(VarEvent::Use { var: v, node: lhs });
                        }
                        self.events_of(rhs, root, bpa, cfg, src, out);
                        out.push(VarEvent::Def {
                            var: v,
                            node: lhs,
                            value: rhs,
                            kind: if compound {
                                DefKind::Compound
                            } else {
                                DefKind::Assign
                            },
                        });
                    }
                    None => {
                        // Field / array element / unresolved target: its sub-expressions
                        // are evaluated before the right-hand side.
                        self.events_of(lhs, root, bpa, cfg, src, out);
                        self.events_of(rhs, root, bpa, cfg, src, out);
                    }
                }
            }
            NN_UNARY_EXPR => {
                let operand = bpa.first_child(node);
                let update_var = match (operand, src) {
                    (Some(op), Some(s))
                        if bpa.node_type(op) == NN_IDENTIFIER_EXPR
                            && is_update_expression(bpa, s, node) =>
                    {
                        self.var_of_occurrence(op).map(|v| (v, op))
                    }
                    _ => None,
                };
                match update_var {
                    Some((v, op)) => {
                        out.push(VarEvent::Use { var: v, node: op });
                        out.push(VarEvent::Def {
                            var: v,
                            node: op,
                            value: u32::MAX,
                            kind: DefKind::Update,
                        });
                    }
                    None => self.events_children(node, root, bpa, cfg, src, out),
                }
            }
            NN_BREAK_STMT | NN_CONTINUE_STMT | NN_ANNOTATION_USE | NN_TYPE_REF | NN_LITERAL
            | NN_TYPE_PARAM => {}
            _ => self.events_children(node, root, bpa, cfg, src, out),
        }
    }
}
