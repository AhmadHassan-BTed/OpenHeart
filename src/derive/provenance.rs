//! Provenance of diagram elements: the analysis facts and token spans each element was
//! derived from, and the readers that describe IR values, places and instructions in
//! labels and guards.
//!
//! Descriptions are reconstructed from SSA definitions (a branch guard is the comparison
//! that defines the branch operand, a message argument is the value's source variable or
//! its defining expression), never copied from source text. Expressions use OCL notation,
//! UML's own expression language, where OCL has the operator (`=`, `<>`, `and`, `not`,
//! `mod`, `oclIsKindOf`, `oclAsType`), and the common infix symbols elsewhere.

use crate::derive::DeriveContext;
use crate::ir::func::{BlockIr, DefSite, FuncIr};
use crate::ir::inst::{BinaryOp, Callee, ComparePred, Inst, Region, Terminator, UnaryOp};
use crate::ir::types::{AllocSiteId, CallSiteId, Span, ValueId};
use crate::ir::value::{ConstVal, Operand, Place};
use crate::memory::location::{HeapBase, ObjId};

/// One analysis fact an element is derived from. Ids are the raw `u32` of the IR id
/// newtypes and of symbol ids, so facts order and hash independently of their producers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Fact {
    /// A declaration of the symbol table.
    Symbol(u32),
    /// A declared supertype edge.
    Supertype { sub: u32, sup: u32 },
    /// An import declaration.
    Import { from: u32, target: u32 },
    /// A field of the IR.
    Field(u32),
    /// An instruction: function, block, index.
    Inst { func: u32, block: u32, index: u32 },
    /// A block terminator.
    Terminator { func: u32, block: u32 },
    AllocSite(u32),
    CallSite(u32),
    /// A resolved call-graph edge.
    CallEdge { site: u32, callee: u32 },
    /// The points-to set of an SSA value.
    PointsTo { func: u32, value: u32 },
    /// The points-to set of an abstract object's slot or of a static cell.
    CellPointsTo { cell: u32 },
    /// An abstract object of the heap.
    Object(u32),
    /// An ownership fact of a field.
    Ownership { field: u32 },
    /// A nullness fact of a field.
    Nullness { field: u32 },
    /// The element structure of a container type.
    Container(u32),
    /// A typestate machine of a type (and of its state field, `u32::MAX` when none).
    Typestate { owner: u32, field: u32 },
    /// A natural loop of a function.
    Loop { func: u32, header: u32 },
    /// A dominator-tree edge.
    Dominance { func: u32, block: u32 },
    /// A control dependence.
    ControlDependence { func: u32, controller: u32, dependent: u32 },
    /// A def-use chain of an SSA value.
    DefUse { func: u32, value: u32 },
    /// An entry point.
    EntryPoint(u32),
    /// A callback from external code.
    Callback(u32),
    /// A concurrency idiom of a call site.
    Concurrency(u32),
    ProjectUnit(u32),
    UnitDependency { from: u32, to: u32 },
    ExecutionNode(u32),
    /// A timing constraint (index in the provider's list).
    TimingConstraint(u32),
    /// A node of a path-summary ROBDD.
    BddNode { func: u32, node: u32 },
}

/// Sorts, deduplicates and caps a span list; synthetic spans are dropped.
pub fn normalize_spans(spans: &mut Vec<Span>, cap: usize) {
    spans.retain(|s| !s.is_none());
    spans.sort_unstable();
    spans.dedup();
    spans.truncate(cap);
}

/// Sorts, deduplicates and caps a fact list.
pub fn normalize_facts(facts: &mut Vec<Fact>, cap: usize) {
    facts.sort_unstable();
    facts.dedup();
    facts.truncate(cap);
}

/// `Some(span)` unless the span is synthetic.
#[inline]
pub fn real(span: Span) -> Option<Span> {
    if span.is_none() {
        None
    } else {
        Some(span)
    }
}

pub fn inst_span(block: &BlockIr, index: usize) -> Option<Span> {
    block.inst_spans.get(index).copied().and_then(real)
}

pub fn term_span(block: &BlockIr) -> Option<Span> {
    real(block.term_span)
}

/// Smallest span covering every instruction and the terminator of a block (in the file of
/// its first spanned instruction).
pub fn block_span(block: &BlockIr) -> Option<Span> {
    let mut acc = Span::NONE;
    for s in block.inst_spans.iter().copied().chain(std::iter::once(block.term_span)) {
        acc = acc.join(s);
    }
    real(acc)
}

pub fn func_span(func: &FuncIr) -> Option<Span> {
    real(func.span)
}

pub fn symbol_span(ctx: &DeriveContext, sym: u32) -> Option<Span> {
    ctx.symbol(sym).and_then(|s| s.span).and_then(real)
}

pub fn call_site_span(ctx: &DeriveContext, site: CallSiteId) -> Option<Span> {
    ctx.call_site(site).and_then(|cs| real(cs.span))
}

pub fn alloc_site_span(ctx: &DeriveContext, site: AllocSiteId) -> Option<Span> {
    ctx.alloc_site(site).and_then(|a| real(a.span))
}

pub fn object_span(ctx: &DeriveContext, obj: ObjId) -> Option<Span> {
    let heap = ctx.heap?;
    let locations = heap.locations();
    if obj.0 as usize >= locations.object_count() {
        return None;
    }
    let info = locations.object(obj);
    if let Some(s) = info.span.and_then(real) {
        return Some(s);
    }
    match info.object.base {
        HeapBase::Site(site) => alloc_site_span(ctx, site),
        _ => None,
    }
}

/// OCL / infix spelling of a unary operator (with its trailing space when it is a word).
pub fn unary_text(op: UnaryOp) -> &'static str {
    match op {
        UnaryOp::Neg => "-",
        UnaryOp::Plus => "+",
        UnaryOp::Not => "not ",
        UnaryOp::BitNot => "~",
    }
}

pub fn binary_text(op: BinaryOp) -> &'static str {
    match op {
        BinaryOp::Add => "+",
        BinaryOp::Sub => "-",
        BinaryOp::Mul => "*",
        BinaryOp::Div => "/",
        BinaryOp::Rem => "mod",
        BinaryOp::BitAnd => "&",
        BinaryOp::BitOr => "|",
        BinaryOp::BitXor => "^",
        BinaryOp::Shl => "<<",
        BinaryOp::Shr => ">>",
        BinaryOp::UShr => ">>>",
    }
}

pub fn compare_text(pred: ComparePred) -> &'static str {
    match pred {
        ComparePred::Eq => "=",
        ComparePred::Ne => "<>",
        ComparePred::Lt => "<",
        ComparePred::Le => "<=",
        ComparePred::Gt => ">",
        ComparePred::Ge => ">=",
    }
}

/// Literal text of a constant.
pub fn const_text(ctx: &DeriveContext, c: &ConstVal) -> String {
    match c {
        ConstVal::Bool(b) => b.to_string(),
        ConstVal::Int(i) => i.to_string(),
        ConstVal::Float(bits) => {
            let v = f64::from_bits(*bits);
            if v.is_finite() && v.fract() == 0.0 && v.abs() < 1e15 {
                format!("{:.1}", v)
            } else {
                v.to_string()
            }
        }
        ConstVal::Str(id) => format!("{:?}", ctx.text(*id)),
        ConstVal::Null => "null".to_string(),
    }
}

/// Name of a call target as written: the function, the method symbol or the name.
pub fn callee_text(ctx: &DeriveContext, renderer: &ValueRenderer, callee: &Callee) -> String {
    match callee {
        Callee::Static(f) => ctx
            .func(*f)
            .map(|fi| ctx.func_name(fi))
            .unwrap_or_else(|| format!("{}", f)),
        Callee::Symbol(s) => ctx.symbol_name(*s),
        Callee::Virtual { method, .. } | Callee::Interface { method, .. } => {
            ctx.symbol_name(*method)
        }
        Callee::Indirect(v) => renderer.name(ctx, *v),
        Callee::Unresolved(name) => ctx.text(*name),
    }
}

/// Describes the values of one function from their SSA definitions.
pub struct ValueRenderer<'f> {
    func: &'f FuncIr,
    defs: Vec<DefSite>,
}

impl<'f> ValueRenderer<'f> {
    pub fn new(func: &'f FuncIr) -> Self {
        Self {
            func,
            defs: func.def_sites(),
        }
    }

    pub fn func(&self) -> &'f FuncIr {
        self.func
    }

    /// Block index and instruction index of a value's definition.
    pub fn def_pos(&self, v: ValueId) -> Option<(usize, usize)> {
        match self.defs.get(v.index()) {
            Some(DefSite::Inst { block, index }) => Some((block.index(), *index as usize)),
            _ => None,
        }
    }

    pub fn def(&self, v: ValueId) -> Option<&'f Inst> {
        let (b, i) = self.def_pos(v)?;
        self.func.blocks.get(b)?.insts.get(i)
    }

    /// Parameter position of a value.
    pub fn param_index(&self, v: ValueId) -> Option<usize> {
        match self.defs.get(v.index()) {
            Some(DefSite::Param(i)) => Some(*i as usize),
            _ => None,
        }
    }

    /// Follows `Assign` copies of values to the value they copy.
    pub fn resolve_copy(&self, v: ValueId) -> ValueId {
        let mut cur = v;
        for _ in 0..64 {
            match self.def(cur) {
                Some(Inst::Assign {
                    src: Operand::Value(src),
                    ..
                }) => cur = *src,
                _ => return cur,
            }
        }
        cur
    }

    /// Source name of a value, or its SSA name.
    pub fn name(&self, ctx: &DeriveContext, v: ValueId) -> String {
        ctx.value_name(self.func, v)
            .unwrap_or_else(|| format!("{}", v))
    }

    /// Short description: a source name, a constant, or the SSA name.
    pub fn operand(&self, ctx: &DeriveContext, op: &Operand) -> String {
        match op {
            Operand::Value(v) => self.name(ctx, *v),
            Operand::Const(c) => const_text(ctx, c),
            Operand::Undef => "undef".to_string(),
        }
    }

    /// Expanded description: temporaries are replaced by their defining expressions.
    pub fn expr(&self, ctx: &DeriveContext, op: &Operand) -> String {
        match op {
            Operand::Value(v) => self.value_expr(ctx, *v, ctx.limits.max_expr_depth),
            _ => self.operand(ctx, op),
        }
    }

    pub fn value_expr(&self, ctx: &DeriveContext, v: ValueId, depth: usize) -> String {
        if let Some(name) = ctx.value_name(self.func, v) {
            return name;
        }
        if depth == 0 {
            return format!("{}", v);
        }
        let sub = |op: &Operand| -> String {
            match op {
                Operand::Value(x) => {
                    let text = self.value_expr(ctx, *x, depth - 1);
                    if self.is_compound(*x) && ctx.value_name(self.func, *x).is_none() {
                        format!("({})", text)
                    } else {
                        text
                    }
                }
                other => self.operand(ctx, other),
            }
        };
        match self.def(v) {
            Some(Inst::Assign { src, .. }) => sub(src),
            Some(Inst::Unary { op, src, .. }) => format!("{}{}", unary_text(*op), sub(src)),
            Some(Inst::Binary { op, lhs, rhs, .. }) => {
                format!("{} {} {}", sub(lhs), binary_text(*op), sub(rhs))
            }
            Some(Inst::Compare { pred, lhs, rhs, .. }) => {
                format!("{} {} {}", sub(lhs), compare_text(*pred), sub(rhs))
            }
            Some(Inst::Load { place, .. }) => self.place(ctx, place),
            Some(Inst::Call { callee, args, .. }) => {
                format!("{}({})", callee_text(ctx, self, callee), self.args(ctx, callee, args))
            }
            Some(Inst::Cast { src, ty, .. }) => {
                format!("{}.oclAsType({})", sub(src), ctx.render_type(*ty))
            }
            Some(Inst::TypeTest { src, ty, .. }) => {
                format!("{}.oclIsKindOf({})", sub(src), ctx.render_type(*ty))
            }
            Some(Inst::Alloc { ty, .. }) => format!("new {}", ctx.render_type(*ty)),
            Some(Inst::MakeClosure { func, .. }) => match ctx.func(*func) {
                Some(f) => format!("closure {}", ctx.func_name(f)),
                None => format!("closure {}", func),
            },
            Some(Inst::Intrinsic { name, args, .. }) => format!(
                "{}({})",
                ctx.text(*name),
                args.iter()
                    .map(|a| self.operand(ctx, a))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            _ => format!("{}", v),
        }
    }

    fn is_compound(&self, v: ValueId) -> bool {
        matches!(
            self.def(v),
            Some(Inst::Binary { .. }) | Some(Inst::Compare { .. })
        )
    }

    /// Guard text of `cond` evaluating to `holds`.
    pub fn condition(&self, ctx: &DeriveContext, cond: &Operand, holds: bool) -> String {
        let text = self.expr(ctx, cond);
        if holds {
            return text;
        }
        if let Operand::Value(v) = cond {
            let v = self.resolve_copy(*v);
            if ctx.value_name(self.func, v).is_none() {
                match self.def(v) {
                    Some(Inst::Compare { pred, lhs, rhs, .. }) => {
                        let neg = pred.negate();
                        return format!(
                            "{} {} {}",
                            self.expr(ctx, lhs),
                            compare_text(neg),
                            self.expr(ctx, rhs)
                        );
                    }
                    Some(Inst::Unary {
                        op: UnaryOp::Not,
                        src,
                        ..
                    }) => return self.expr(ctx, src),
                    _ => {}
                }
            }
        }
        if let Operand::Const(ConstVal::Bool(b)) = cond {
            return (!b).to_string();
        }
        format!("not ({})", text)
    }

    /// Description of a memory place.
    pub fn place(&self, ctx: &DeriveContext, place: &Place) -> String {
        match place {
            Place::Local(v) => self.name(ctx, *v),
            Place::Field { base, field } => {
                let fname = ctx
                    .field(*field)
                    .map(|f| ctx.text(f.name))
                    .unwrap_or_else(|| format!("{}", field));
                format!("{}.{}", self.name(ctx, *base), fname)
            }
            Place::Index { base, index } => {
                format!("{}[{}]", self.name(ctx, *base), self.operand(ctx, index))
            }
            Place::Global(g) => ctx
                .global(*g)
                .map(|gi| ctx.text(gi.name))
                .unwrap_or_else(|| format!("{}", g)),
            Place::Deref(v) => format!("*{}", self.name(ctx, *v)),
            Place::Static { owner, field } => {
                let fname = ctx
                    .field(*field)
                    .map(|f| ctx.text(f.name))
                    .unwrap_or_else(|| format!("{}", field));
                format!("{}.{}", ctx.symbol_name(*owner), fname)
            }
        }
    }

    /// Argument list of a call, without the receiver of an instance call.
    pub fn args(&self, ctx: &DeriveContext, callee: &Callee, args: &[Operand]) -> String {
        let skip = match callee {
            Callee::Virtual { .. } | Callee::Interface { .. } => 1,
            Callee::Static(f) => ctx
                .func(*f)
                .map(|fi| fi.has_receiver as usize + fi.capture_count as usize)
                .unwrap_or(0),
            _ => 0,
        };
        args.iter()
            .skip(skip.min(args.len()))
            .map(|a| self.operand(ctx, a))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// One-line description of an instruction (basic-block statements).
    pub fn inst(&self, ctx: &DeriveContext, inst: &Inst) -> String {
        let dst = |v: ValueId| self.name(ctx, v);
        match inst {
            Inst::Assign { dst: d, src } => format!("{} := {}", dst(*d), self.operand(ctx, src)),
            Inst::Unary { dst: d, op, src } => {
                format!("{} := {}{}", dst(*d), unary_text(*op), self.operand(ctx, src))
            }
            Inst::Binary { dst: d, op, lhs, rhs } => format!(
                "{} := {} {} {}",
                dst(*d),
                self.operand(ctx, lhs),
                binary_text(*op),
                self.operand(ctx, rhs)
            ),
            Inst::Compare { dst: d, pred, lhs, rhs } => format!(
                "{} := {} {} {}",
                dst(*d),
                self.operand(ctx, lhs),
                compare_text(*pred),
                self.operand(ctx, rhs)
            ),
            Inst::Load { dst: d, place } => format!("{} := {}", dst(*d), self.place(ctx, place)),
            Inst::Store { place, value } => {
                format!("{} := {}", self.place(ctx, place), self.operand(ctx, value))
            }
            Inst::Alloc { dst: d, ty, region, .. } => format!(
                "{} := alloc {} {}",
                dst(*d),
                region_text(*region),
                ctx.render_type(*ty)
            ),
            Inst::Call { dst: d, callee, args, unwind, .. } => {
                let call = format!(
                    "{}({})",
                    callee_text(ctx, self, callee),
                    args.iter()
                        .map(|a| self.operand(ctx, a))
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                let call = match unwind {
                    Some(h) => format!("{} unwind {}", call, h),
                    None => call,
                };
                match d {
                    Some(v) => format!("{} := call {}", dst(*v), call),
                    None => format!("call {}", call),
                }
            }
            Inst::Phi { dst: d, incoming } => format!(
                "{} := phi({})",
                dst(*d),
                incoming
                    .iter()
                    .map(|(b, op)| format!("{}: {}", b, self.operand(ctx, op)))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Inst::Cast { dst: d, src, ty } => format!(
                "{} := {}.oclAsType({})",
                dst(*d),
                self.operand(ctx, src),
                ctx.render_type(*ty)
            ),
            Inst::MakeClosure { dst: d, func, captures } => format!(
                "{} := closure {}[{}]",
                dst(*d),
                ctx.func(*func)
                    .map(|f| ctx.func_name(f))
                    .unwrap_or_else(|| format!("{}", func)),
                captures
                    .iter()
                    .map(|c| self.operand(ctx, c))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Inst::TypeTest { dst: d, src, ty } => format!(
                "{} := {}.oclIsKindOf({})",
                dst(*d),
                self.operand(ctx, src),
                ctx.render_type(*ty)
            ),
            Inst::Release { place } => format!("release {}", self.place(ctx, place)),
            Inst::Intrinsic { dst: d, name, args } => {
                let call = format!(
                    "{}({})",
                    ctx.text(*name),
                    args.iter()
                        .map(|a| self.operand(ctx, a))
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                match d {
                    Some(v) => format!("{} := {}", dst(*v), call),
                    None => call,
                }
            }
        }
    }

    /// One-line description of a terminator.
    pub fn terminator(&self, ctx: &DeriveContext, block: &BlockIr) -> String {
        match &block.term {
            Terminator::Jump(t) => format!("goto {}", t),
            Terminator::Branch { cond, then_b, else_b } => format!(
                "if {} then {} else {}",
                self.operand(ctx, cond),
                then_b,
                else_b
            ),
            Terminator::Switch {
                scrutinee,
                cases,
                default,
            } => {
                let mut arms: Vec<String> = cases
                    .iter()
                    .map(|(c, b)| format!("{} -> {}", const_text(ctx, c), b))
                    .collect();
                arms.push(format!("else -> {}", default));
                format!("switch {}: {}", self.operand(ctx, scrutinee), arms.join(", "))
            }
            Terminator::Return(Some(v)) => format!("return {}", self.operand(ctx, v)),
            Terminator::Return(None) => "return".to_string(),
            Terminator::Throw(v) => match block.unwind {
                Some(h) => format!("raise {} to {}", self.operand(ctx, v), h),
                None => format!("raise {}", self.operand(ctx, v)),
            },
            Terminator::Unreachable => "unreachable".to_string(),
        }
    }
}

pub fn region_text(region: Region) -> &'static str {
    region.mnemonic()
}
