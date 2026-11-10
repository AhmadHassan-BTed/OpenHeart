//! Inclusion-based (Andersen) points-to analysis (§6.2.5).
//! Authored solely by Ahmad Hassan (B-Ted).
//!
//! * Pointer nodes are **global** SSA values (`SSAArtifact::global_ssa_base()[f] + id`),
//!   so facts of different methods never merge, plus synthetic nodes for `this` and the
//!   return value of each method, expression temporaries and abstract heap fields.
//! * Abstract objects are allocation sites (`new`, array creation / initialisers).
//! * Constraints: allocation, copy (assignments, casts, φ, conditional expressions),
//!   field-sensitive load / store (`o.f` per abstract object and field name; array
//!   elements share one pseudo-field), parameter passing, receiver to `this`, return
//!   values to call results, constructor `this` to the allocated object.
//! * Solving: worklist with difference propagation (Pearce et al. 2004); complex load /
//!   store constraints are re-evaluated only for newly discovered objects.
//!
//! Limitations, by design of the current IR: the call graph used for parameter / return
//! constraints is the CHA/RTA one (no on-the-fly refinement); static fields are
//! field-based (one node per field name); captured lambda variables, reflection and
//! library code are not modelled; there is no cycle collapsing.

use crate::cfg::ast_query::{children, is_nested_body, subtree_end, NewParts};
use crate::cfg::serializer::CFGArtifact;
use crate::cg::call_sites::{FunctionContext, SiteInfo, SiteKind};
use crate::cg::index::ProgramIndex;
use crate::core::types::ssa::SSA_FLAG_IS_PARAM_DEF;
use crate::core::types::ASTNodeType;
use crate::ssa::SSAArtifact;
use std::collections::{HashMap, HashSet, VecDeque};

/// Pseudo-field shared by all elements of an array object.
pub const ARRAY_FIELD: u32 = u32::MAX - 1;

/// One abstract object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AllocSite {
    pub node: u32,
    /// Class symbol allocated, or `u32::MAX` when it is an array or the type could not
    /// be resolved to a single symbol.
    pub type_sym: u32,
}

/// Inclusion-constraint solver.
pub struct Andersen {
    pts: Vec<HashSet<u32>>,
    delta: Vec<Vec<u32>>,
    copy: Vec<Vec<u32>>,
    copy_set: HashSet<(u32, u32)>,
    loads: Vec<Vec<(u32, u32)>>,
    stores: Vec<Vec<(u32, u32)>>,
    field_nodes: HashMap<(u32, u32), u32>,
    queue: VecDeque<u32>,
    queued: Vec<bool>,
}

impl Andersen {
    pub fn new(nodes: usize) -> Self {
        Self {
            pts: vec![HashSet::new(); nodes],
            delta: vec![Vec::new(); nodes],
            copy: vec![Vec::new(); nodes],
            copy_set: HashSet::new(),
            loads: vec![Vec::new(); nodes],
            stores: vec![Vec::new(); nodes],
            field_nodes: HashMap::new(),
            queue: VecDeque::new(),
            queued: vec![false; nodes],
        }
    }

    pub fn node_count(&self) -> usize {
        self.pts.len()
    }

    pub fn new_node(&mut self) -> u32 {
        let id = self.pts.len() as u32;
        self.pts.push(HashSet::new());
        self.delta.push(Vec::new());
        self.copy.push(Vec::new());
        self.loads.push(Vec::new());
        self.stores.push(Vec::new());
        self.queued.push(false);
        id
    }

    fn insert(&mut self, node: u32, obj: u32) {
        let n = node as usize;
        if self.pts[n].insert(obj) {
            self.delta[n].push(obj);
            if !self.queued[n] {
                self.queued[n] = true;
                self.queue.push_back(node);
            }
        }
    }

    fn field_node(&mut self, obj: u32, field: u32) -> u32 {
        if let Some(&n) = self.field_nodes.get(&(obj, field)) {
            return n;
        }
        let n = self.new_node();
        self.field_nodes.insert((obj, field), n);
        n
    }

    /// Adds `dst ⊇ src`; returns true when the edge is new.
    fn link(&mut self, src: u32, dst: u32) -> bool {
        if src == dst || !self.copy_set.insert((src, dst)) {
            return false;
        }
        self.copy[src as usize].push(dst);
        let existing: Vec<u32> = self.pts[src as usize].iter().copied().collect();
        for o in existing {
            self.insert(dst, o);
        }
        true
    }

    /// `pts(node) ∋ obj`
    pub fn add_alloc(&mut self, node: u32, obj: u32) {
        self.insert(node, obj);
    }

    /// `pts(dst) ⊇ pts(src)`
    pub fn add_copy(&mut self, src: u32, dst: u32) {
        self.link(src, dst);
    }

    /// `pts(dst) ⊇ pts(o.field)` for every `o ∈ pts(base)`
    pub fn add_load(&mut self, base: u32, field: u32, dst: u32) {
        self.loads[base as usize].push((field, dst));
        let objs: Vec<u32> = self.pts[base as usize].iter().copied().collect();
        for o in objs {
            let f = self.field_node(o, field);
            self.link(f, dst);
        }
    }

    /// `pts(o.field) ⊇ pts(src)` for every `o ∈ pts(base)`
    pub fn add_store(&mut self, base: u32, field: u32, src: u32) {
        self.stores[base as usize].push((field, src));
        let objs: Vec<u32> = self.pts[base as usize].iter().copied().collect();
        for o in objs {
            let f = self.field_node(o, field);
            self.link(src, f);
        }
    }

    pub fn solve(&mut self) {
        while let Some(n) = self.queue.pop_front() {
            let ni = n as usize;
            self.queued[ni] = false;
            let delta = std::mem::take(&mut self.delta[ni]);
            if delta.is_empty() {
                continue;
            }
            for i in 0..self.copy[ni].len() {
                let t = self.copy[ni][i];
                for &o in &delta {
                    self.insert(t, o);
                }
            }
            for i in 0..self.loads[ni].len() {
                let (field, dst) = self.loads[ni][i];
                for &o in &delta {
                    let f = self.field_node(o, field);
                    self.link(f, dst);
                }
            }
            for i in 0..self.stores[ni].len() {
                let (field, src) = self.stores[ni][i];
                for &o in &delta {
                    let f = self.field_node(o, field);
                    self.link(src, f);
                }
            }
        }
    }

    pub fn points_to(&self, node: u32) -> Option<&HashSet<u32>> {
        self.pts.get(node as usize)
    }
}

/// Inputs of the whole-program points-to analysis.
pub struct PointsToInput<'x, 'a> {
    pub index: &'x ProgramIndex<'a>,
    pub cfa: &'x CFGArtifact,
    pub ssa: &'x SSAArtifact,
    pub contexts: &'x [FunctionContext],
    pub ctx_of_fn: &'x HashMap<u32, usize>,
    pub sites: &'x [SiteInfo],
    /// Resolved targets per entry of `sites`.
    pub site_targets: &'x [Vec<u32>],
}

pub struct PointsToResult {
    /// Global SSA id -> allocation types (`u32::MAX` = array / unresolved type).
    pub pts: HashMap<u32, HashSet<u32>>,
    pub alloc_sites: Vec<AllocSite>,
}

struct Gen<'g, 'x, 'a> {
    input: &'g PointsToInput<'x, 'a>,
    solver: Andersen,
    allocs: Vec<AllocSite>,
    alloc_of_node: HashMap<u32, u32>,
    this_nodes: HashMap<u32, u32>,
    ret_nodes: HashMap<u32, u32>,
    static_fields: HashMap<u32, u32>,
    site_of_node: HashMap<u32, usize>,
    memo: HashMap<(usize, u32), Option<u32>>,
}

impl<'g, 'x, 'a> Gen<'g, 'x, 'a> {
    fn this_of(&mut self, f: u32) -> u32 {
        if let Some(&n) = self.this_nodes.get(&f) {
            return n;
        }
        let n = self.solver.new_node();
        self.this_nodes.insert(f, n);
        n
    }

    fn ret_of(&mut self, f: u32) -> u32 {
        if let Some(&n) = self.ret_nodes.get(&f) {
            return n;
        }
        let n = self.solver.new_node();
        self.ret_nodes.insert(f, n);
        n
    }

    fn static_field(&mut self, name: u32) -> u32 {
        if let Some(&n) = self.static_fields.get(&name) {
            return n;
        }
        let n = self.solver.new_node();
        self.static_fields.insert(name, n);
        n
    }

    fn text_id(&self, node: u32) -> Option<u32> {
        let index = self.input.index;
        index
            .src
            .as_ref()
            .and_then(|s| s.node_text_id(index.bpa, node))
    }

    fn alloc(&mut self, node: u32) -> u32 {
        if let Some(&o) = self.alloc_of_node.get(&node) {
            return o;
        }
        let input: &'g PointsToInput<'x, 'a> = self.input;
        let index = input.index;
        let type_sym = if index.bpa.node_type(node) == ASTNodeType::NN_NEW_EXPR {
            let parts = NewParts::of(index.bpa, index.src.as_ref(), node);
            match parts.type_name {
                Some(t) if parts.anonymous_members.is_empty() => {
                    let all = index.types_named(t);
                    let project: Vec<u32> = all
                        .iter()
                        .copied()
                        .filter(|&s| {
                            index.symbol(s).is_some_and(|r| {
                                r.kind != crate::core::types::symbol::SymbolKind::SK_EXTERNAL as u8
                            })
                        })
                        .collect();
                    match (project.as_slice(), all) {
                        ([one], _) => *one,
                        ([], [one]) => *one,
                        _ => u32::MAX,
                    }
                }
                _ => u32::MAX,
            }
        } else {
            u32::MAX
        };
        let o = self.allocs.len() as u32;
        self.allocs.push(AllocSite { node, type_sym });
        self.alloc_of_node.insert(node, o);
        o
    }

    /// Class of the function owning context `c` (for implicit-`this` field accesses).
    fn class_of_ctx(&self, c: usize) -> Option<u32> {
        self.input
            .index
            .enclosing_class(self.input.contexts[c].sym_id)
    }

    /// Pointer node holding the value of `expr` evaluated in block statement `stmt` of
    /// context `c`; `None` when the expression cannot hold a modelled reference.
    fn value_node(&mut self, c: usize, expr: u32, stmt: u32, depth: usize) -> Option<u32> {
        if depth > 64 {
            return None;
        }
        if let Some(&v) = self.memo.get(&(c, expr)) {
            return v;
        }
        let v = self.compute_value(c, expr, stmt, depth);
        self.memo.insert((c, expr), v);
        v
    }

    fn compute_value(&mut self, c: usize, expr: u32, stmt: u32, depth: usize) -> Option<u32> {
        let input: &'g PointsToInput<'x, 'a> = self.input;
        let index = input.index;
        let bpa = index.bpa;
        let fn_sym = input.contexts[c].sym_id;
        match bpa.node_type(expr) {
            ASTNodeType::NN_IDENTIFIER_EXPR => {
                let ctx = &input.contexts[c];
                if ctx.ssa_base != u32::MAX {
                    if let Some(local) = ctx.local_ssa_of_ident(stmt, expr) {
                        return Some(ctx.ssa_base + local);
                    }
                }
                if ctx.vars.var_of_occurrence(expr).is_some() {
                    return None;
                }
                let name = self.text_id(expr)?;
                let class = self.class_of_ctx(c)?;
                if !index.has_field(class, name) {
                    return None;
                }
                let this = self.this_of(fn_sym);
                let tmp = self.solver.new_node();
                self.solver.add_load(this, name, tmp);
                Some(tmp)
            }
            ASTNodeType::NN_THIS_EXPR => Some(self.this_of(fn_sym)),
            ASTNodeType::NN_NEW_EXPR
            | ASTNodeType::NN_ARRAY_CREATE
            | ASTNodeType::NN_ARRAY_INIT => {
                let obj = self.alloc(expr);
                let tmp = self.solver.new_node();
                self.solver.add_alloc(tmp, obj);
                if let Some(&si) = self.site_of_node.get(&expr) {
                    let targets = input.site_targets[si].clone();
                    for t in targets {
                        let this_t = self.this_of(t);
                        self.solver.add_copy(tmp, this_t);
                    }
                }
                Some(tmp)
            }
            ASTNodeType::NN_CAST_EXPR => {
                let inner = children(bpa, expr)
                    .into_iter()
                    .rev()
                    .find(|&k| bpa.node_type(k) != ASTNodeType::NN_TYPE_REF)?;
                self.value_node(c, inner, stmt, depth + 1)
            }
            ASTNodeType::NN_TERNARY_EXPR => {
                let kids = children(bpa, expr);
                let tmp = self.solver.new_node();
                for &branch in kids.iter().skip(1) {
                    // Lowered: each branch operand is a block statement of its own.
                    if let Some(v) = self.value_node(c, branch, branch, depth + 1) {
                        self.solver.add_copy(v, tmp);
                    }
                }
                Some(tmp)
            }
            ASTNodeType::NN_ASSIGN_EXPR => {
                let rhs = children(bpa, expr).get(1).copied()?;
                self.value_node(c, rhs, stmt, depth + 1)
            }
            ASTNodeType::NN_CALL_EXPR => {
                let si = *self.site_of_node.get(&expr)?;
                let targets = input.site_targets[si].clone();
                if targets.is_empty() {
                    return None;
                }
                let tmp = self.solver.new_node();
                for t in targets {
                    let r = self.ret_of(t);
                    self.solver.add_copy(r, tmp);
                }
                Some(tmp)
            }
            ASTNodeType::NN_FIELD_ACCESS => {
                let kids = children(bpa, expr);
                if kids.len() < 2 {
                    return None;
                }
                let field = self.text_id(kids[kids.len() - 1])?;
                let tmp = self.solver.new_node();
                match self.value_node(c, kids[0], stmt, depth + 1) {
                    Some(base) => self.solver.add_load(base, field, tmp),
                    None => {
                        // `Type.field`: static fields are modelled field-based.
                        let sf = self.static_field(field);
                        self.solver.add_copy(sf, tmp);
                    }
                }
                Some(tmp)
            }
            ASTNodeType::NN_ARRAY_ACCESS => {
                let arr = children(bpa, expr).first().copied()?;
                let base = self.value_node(c, arr, stmt, depth + 1)?;
                let tmp = self.solver.new_node();
                self.solver.add_load(base, ARRAY_FIELD, tmp);
                Some(tmp)
            }
            _ => None,
        }
    }

    /// `lhs = value` where `lhs` is a heap location (field, array element, implicit-this
    /// field or static field).
    fn store(&mut self, c: usize, lhs: u32, value: u32, stmt: u32) {
        let input: &'g PointsToInput<'x, 'a> = self.input;
        let index = input.index;
        let bpa = index.bpa;
        let fn_sym = input.contexts[c].sym_id;
        match bpa.node_type(lhs) {
            ASTNodeType::NN_FIELD_ACCESS => {
                let kids = children(bpa, lhs);
                if kids.len() < 2 {
                    return;
                }
                let field = match self.text_id(kids[kids.len() - 1]) {
                    Some(f) => f,
                    None => return,
                };
                match self.value_node(c, kids[0], stmt, 0) {
                    Some(base) => self.solver.add_store(base, field, value),
                    None => {
                        let sf = self.static_field(field);
                        self.solver.add_copy(value, sf);
                    }
                }
            }
            ASTNodeType::NN_ARRAY_ACCESS => {
                if let Some(arr) = children(bpa, lhs).first().copied() {
                    if let Some(base) = self.value_node(c, arr, stmt, 0) {
                        self.solver.add_store(base, ARRAY_FIELD, value);
                    }
                }
            }
            ASTNodeType::NN_IDENTIFIER_EXPR => {
                if input.contexts[c].vars.var_of_occurrence(lhs).is_some() {
                    return; // a local: modelled through SSA
                }
                let name = match self.text_id(lhs) {
                    Some(n) => n,
                    None => return,
                };
                let class = match self.class_of_ctx(c) {
                    Some(k) => k,
                    None => return,
                };
                if index.has_field(class, name) {
                    let this = self.this_of(fn_sym);
                    self.solver.add_store(this, name, value);
                }
            }
            _ => {}
        }
    }

    /// Stores and returns performed by the block statements of context `c`.
    fn scan_statements(&mut self, c: usize) {
        let input: &'g PointsToInput<'x, 'a> = self.input;
        let bpa = input.index.bpa;
        let cfg = &input.cfa.functions[input.contexts[c].fn_index];
        let fn_sym = input.contexts[c].sym_id;
        for blk in &cfg.blocks {
            for &stmt in &blk.stmts {
                if bpa.node_type(stmt) == ASTNodeType::NN_RETURN_STMT {
                    if let Some(e) = bpa.first_child(stmt) {
                        if let Some(v) = self.value_node(c, e, stmt, 0) {
                            let r = self.ret_of(fn_sym);
                            self.solver.add_copy(v, r);
                        }
                    }
                }
                let end = subtree_end(bpa, stmt);
                let mut k = stmt;
                while k < end {
                    let t = bpa.node_type(k);
                    if k != stmt && (is_nested_body(t) || cfg.is_lowered(k)) {
                        k = subtree_end(bpa, k);
                        continue;
                    }
                    if t == ASTNodeType::NN_ASSIGN_EXPR {
                        let kids = children(bpa, k);
                        if kids.len() == 2 {
                            if let Some(v) = self.value_node(c, kids[1], stmt, 0) {
                                self.store(c, kids[0], v, stmt);
                            }
                        }
                    }
                    k += 1;
                }
            }
        }
    }

    /// Parameter, receiver and constructor constraints of call site `si`.
    fn call_constraints(&mut self, si: usize) {
        let input: &'g PointsToInput<'x, 'a> = self.input;
        let site = &input.sites[si];
        let c = match site.ctx {
            Some(c) => c,
            None => return,
        };
        let fn_sym = input.contexts[c].sym_id;
        let targets = input.site_targets[si].clone();
        if targets.is_empty() {
            return;
        }

        if matches!(site.kind, SiteKind::New { .. }) {
            // Materialises the allocation and its constructor `this` constraint even when
            // the created object is discarded.
            self.value_node(c, site.call_node, site.stmt, 0);
        }

        let arg_values: Vec<Option<u32>> = site
            .args()
            .to_vec()
            .into_iter()
            .map(|a| self.value_node(c, a, site.stmt, 0))
            .collect();
        let receiver_value = match &site.kind {
            SiteKind::Method {
                receiver: Some(r), ..
            } => self.value_node(c, *r, site.stmt, 0),
            SiteKind::Method { receiver: None, .. } | SiteKind::ConstructorChain { .. } => {
                Some(self.this_of(fn_sym))
            }
            _ => None,
        };

        for t in targets {
            if let Some(v) = receiver_value {
                let this_t = self.this_of(t);
                self.solver.add_copy(v, this_t);
            }
            let params = self.param_nodes(t);
            for (i, av) in arg_values.iter().enumerate() {
                if let (Some(v), Some(&p)) = (av, params.get(i)) {
                    self.solver.add_copy(*v, p);
                }
            }
        }
    }

    /// Global SSA nodes of the formal parameters of `callee`, in order.
    fn param_nodes(&self, callee: u32) -> Vec<u32> {
        let input = self.input;
        let ctx = match input.ctx_of_fn.get(&callee) {
            Some(&c) => &input.contexts[c],
            None => return Vec::new(),
        };
        let (si, base) = match (ctx.ssa_index, ctx.ssa_base) {
            (Some(si), base) if base != u32::MAX => (si, base),
            _ => return Vec::new(),
        };
        input.ssa.functions[si]
            .ssa_records
            .iter()
            .enumerate()
            .filter(|(_, r)| (r.flags & SSA_FLAG_IS_PARAM_DEF) != 0 && r.def_stmt != u32::MAX)
            .map(|(d, _)| base + d as u32)
            .collect()
    }
}

/// Anderson Points-To Analysis Fixpoint Solver
pub struct AndersonPointsTo;

impl AndersonPointsTo {
    pub fn run(input: &PointsToInput<'_, '_>) -> PointsToResult {
        let total_ssa: usize = input
            .ssa
            .functions
            .iter()
            .map(|f| f.ssa_records.len())
            .sum();
        let mut gen = Gen {
            input,
            solver: Andersen::new(total_ssa),
            allocs: Vec::new(),
            alloc_of_node: HashMap::new(),
            this_nodes: HashMap::new(),
            ret_nodes: HashMap::new(),
            static_fields: HashMap::new(),
            site_of_node: input
                .sites
                .iter()
                .enumerate()
                .map(|(i, s)| (s.call_node, i))
                .collect(),
            memo: HashMap::new(),
        };

        // ── Definitions: φ, copies, allocations, loads, call results ──
        for (c, ctx) in input.contexts.iter().enumerate() {
            let (si, base) = match (ctx.ssa_index, ctx.ssa_base) {
                (Some(si), base) if base != u32::MAX => (si, base),
                _ => continue,
            };
            let f = &input.ssa.functions[si];
            for (d, rec) in f.ssa_records.iter().enumerate() {
                let g = base + d as u32;
                if rec.is_phi() {
                    for &op in f.operands.operands_of(d as u32) {
                        gen.solver.add_copy(base + op, g);
                    }
                    continue;
                }
                let value = f.value_nodes.get(d).copied().unwrap_or(u32::MAX);
                if value != u32::MAX && rec.def_stmt != u32::MAX {
                    if let Some(v) = gen.value_node(c, value, rec.def_stmt, 0) {
                        gen.solver.add_copy(v, g);
                    }
                }
            }
            gen.scan_statements(c);
        }

        // ── Calls: parameters, receivers, constructors ──
        for si in 0..input.sites.len() {
            gen.call_constraints(si);
        }

        gen.solver.solve();

        let mut pts: HashMap<u32, HashSet<u32>> = HashMap::new();
        for g in 0..total_ssa as u32 {
            if let Some(objs) = gen.solver.points_to(g) {
                if !objs.is_empty() {
                    let types: HashSet<u32> = objs
                        .iter()
                        .map(|&o| gen.allocs[o as usize].type_sym)
                        .collect();
                    pts.insert(g, types);
                }
            }
        }

        PointsToResult {
            pts,
            alloc_sites: gen.allocs,
        }
    }
}
