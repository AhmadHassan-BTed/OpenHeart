//! Per-function lowering state: lexical scopes and variable bindings, closure captures,
//! name resolution, and the stack of control and cleanup regions that structured exits
//! traverse.

use std::collections::{HashMap, HashSet};

use super::{ClassCapture, Lowerer};
use crate::ir::builder::{CallMeta, FunctionBuilder};
use crate::ir::func::FuncKind;
use crate::ir::inst::{Callee, Inst, IntrinsicKind, Region};
use crate::ir::lower::semantics::LoweringSemantics;
use crate::ir::module::DiagnosticKind;
use crate::ir::ssa::{VarId, VarInfo};
use crate::ir::types::{
    BlockId, FieldId, FuncId, GlobalId, IrType, Span, TypeId, TypeTable, ValueId,
};
use crate::ir::value::{ConstVal, Operand, Place, ValueInfo};

/// A lowered expression: its operand and static type.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Val {
    pub op: Operand,
    pub ty: TypeId,
}

impl Val {
    pub fn new(op: Operand, ty: TypeId) -> Self {
        Self { op, ty }
    }

    pub fn undef() -> Self {
        Self {
            op: Operand::Undef,
            ty: TypeTable::UNKNOWN,
        }
    }
}

/// Where a source variable lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum Binding {
    /// SSA variable of the frame.
    Var(VarId),
    /// The variable holds the address of a stack slot of this frame (a local shared with
    /// nested closures by reference).
    Slot(VarId),
    /// The variable holds the address of a cell owned by an enclosing frame.
    CellRef(VarId),
    /// Captured into a field of `this` (members of local and anonymous classes); `cell`
    /// when the field holds a cell address.
    Field { field: FieldId, cell: bool },
}

/// How a frame reaches the variables of its enclosing frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CaptureSite {
    /// Top-level function: no enclosing frame.
    None,
    /// Lambda: captured values arrive as leading parameters.
    Closure,
    /// Member of a local or anonymous class: captured values are fields of `this`.
    ClassMember,
}

/// Object reached from a binding by loading a chain of fields (enclosing instances).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Receiver {
    pub base: Binding,
    pub hops: Vec<FieldId>,
}

/// Meaning of a simple name at a program point.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Resolved {
    Local(Binding),
    Field { receiver: Receiver, field: FieldId },
    Static { owner: u32, field: FieldId },
    Global(GlobalId),
    Type(u32),
    Unresolved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BreakKind {
    Loop,
    Switch,
    Block,
}

/// Work a region performs on every exit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Cleanup {
    /// `finally` clause node.
    Finally(u32),
    /// Resource of a resource region.
    Release {
        resource: ValueId,
        ty: TypeId,
        span: Span,
    },
    /// Lock of a guarded region.
    Monitor { lock: ValueId, span: Span },
    /// Deferred statement node.
    Defer(u32),
    /// Stack object destroyed at scope exit.
    Destroy { slot: ValueId, span: Span },
}

/// Region of the control and exception stack.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Exit {
    Breakable {
        label: Option<u32>,
        kind: BreakKind,
        break_to: BlockId,
        continue_to: Option<BlockId>,
        /// Result variable of a switch expression.
        result: Option<VarId>,
    },
    /// Protected region whose calls unwind to `landing` (catch dispatch).
    Handler { landing: BlockId },
    /// Region with exit work; `landing` is created when the first instruction that can
    /// raise inside the region needs it.
    Cleanup {
        action: Cleanup,
        landing: Option<BlockId>,
    },
}

/// Lowering state of one function.
pub(super) struct FnState<'t> {
    pub b: FunctionBuilder,
    pub sem: &'t dyn LoweringSemantics,
    pub func: FuncId,
    pub kind: FuncKind,
    pub file: Option<u16>,
    /// Plan key (declaration node) of the enclosing type.
    pub class_node: Option<u32>,
    pub class_symbol: Option<u32>,
    pub function_symbol: Option<u32>,
    pub this: Option<Binding>,
    pub capture: CaptureSite,
    /// Closure captures: binding in the enclosing frame, binding in this frame.
    pub captures: Vec<(Binding, Binding)>,
    pub scopes: Vec<Vec<(u32, Binding)>>,
    pub exits: Vec<Exit>,
    pub labels: HashMap<u32, BlockId>,
    pub pending_label: Option<u32>,
    /// Names that nested closures of this function capture; such locals live in cells
    /// unless the language copies captured values.
    pub shared_names: HashSet<u32>,
    /// Value type of the slot or cell a `Slot`/`CellRef` variable addresses.
    pub slot_types: HashMap<VarId, TypeId>,
    pub ret: TypeId,
    pub has_handlers: bool,
    /// The body contains a `goto`: label blocks stay unsealed until the function ends,
    /// since a later jump may still target them.
    pub gotos: bool,
}

impl<'t> FnState<'t> {
    pub fn has_gotos(&self) -> bool {
        self.gotos
    }
}

/// Parameters of a new frame.
pub(super) struct FrameSpec<'t> {
    pub func: FuncId,
    pub name: u32,
    pub ret: TypeId,
    pub span: Span,
    pub kind: FuncKind,
    pub sem: &'t dyn LoweringSemantics,
    pub file: Option<u16>,
    pub class_node: Option<u32>,
    pub class_symbol: Option<u32>,
    pub function_symbol: Option<u32>,
    pub capture: CaptureSite,
}

impl<'t> Lowerer<'t> {
    // ── Frames ──

    pub(super) fn cur(&mut self) -> &mut FnState<'t> {
        self.frames.last_mut().expect("no function is being lowered")
    }

    pub(super) fn cur_ref(&self) -> &FnState<'t> {
        self.frames.last().expect("no function is being lowered")
    }

    pub(super) fn b(&mut self) -> &mut FunctionBuilder {
        &mut self.cur().b
    }

    pub(super) fn top(&self) -> usize {
        self.frames.len() - 1
    }

    pub(super) fn start_frame(&mut self, spec: FrameSpec<'t>) {
        let mut b = FunctionBuilder::new(spec.func, spec.name, spec.ret, spec.span, spec.kind);
        {
            let f = b.func_mut();
            f.symbol = spec.function_symbol;
            f.enclosing_type = spec.class_symbol;
        }
        self.frames.push(FnState {
            b,
            sem: spec.sem,
            func: spec.func,
            kind: spec.kind,
            file: spec.file,
            class_node: spec.class_node,
            class_symbol: spec.class_symbol,
            function_symbol: spec.function_symbol,
            this: None,
            capture: spec.capture,
            captures: Vec::new(),
            scopes: vec![Vec::new()],
            exits: Vec::new(),
            labels: HashMap::new(),
            pending_label: None,
            shared_names: HashSet::new(),
            slot_types: HashMap::new(),
            ret: spec.ret,
            has_handlers: false,
            gotos: false,
        });
    }

    /// Completes the innermost frame. Returns its id and the enclosing-frame bindings of
    /// its closure captures, in parameter order.
    pub(super) fn end_frame(&mut self) -> (FuncId, Vec<Binding>) {
        if self.cur_ref().b.is_open() {
            let span = self.cur_ref().b.func().span;
            self.b().terminate(crate::ir::inst::Terminator::Return(None), span);
        }
        if self.cur_ref().has_handlers {
            let span = self.cur_ref().b.func().span;
            self.diag_once(
                DiagnosticKind::UnderApproximate,
                span,
                "implicit exceptions of non-call instructions (null dereference, arithmetic, \
                 bounds, failed casts) have no unwind edges; handlers are reached from calls \
                 and throws only"
                    .to_string(),
            );
        }
        let st = self.frames.pop().expect("no function is being lowered");
        let outer: Vec<Binding> = st.captures.iter().map(|(o, _)| *o).collect();
        let mut b = st.b;
        b.func_mut().capture_count = st.captures.len() as u32;
        let finished = b.finish();
        self.store_function(st.func, finished);
        (st.func, outer)
    }

    // ── Scopes and bindings ──

    pub(super) fn push_scope(&mut self) {
        self.cur().scopes.push(Vec::new());
    }

    pub(super) fn pop_scope(&mut self) {
        let st = self.cur();
        if st.scopes.len() > 1 {
            st.scopes.pop();
        }
    }

    pub(super) fn bind(&mut self, name: u32, binding: Binding) {
        if name == u32::MAX {
            return;
        }
        if let Some(scope) = self.cur().scopes.last_mut() {
            scope.push((name, binding));
        }
    }

    /// Declares a local variable in the current scope. Locals captured by nested closures
    /// of a language that does not copy captures live in a stack cell.
    pub(super) fn declare_local(
        &mut self,
        name: Option<u32>,
        symbol: Option<u32>,
        ty: TypeId,
        span: Span,
    ) -> Binding {
        let shared = match name {
            Some(n) => {
                self.cur_ref().shared_names.contains(&n)
                    && self.cur_ref().sem.capture_mode()
                        != Some(crate::ir::lower::semantics::CaptureMode::ByValue)
            }
            None => false,
        };
        let binding = if shared {
            let ptr_ty = self.module.types.ptr_to(ty);
            let slot = self.b().emit_alloc(ty, Region::Stack, span);
            let var = self.b().declare_var(VarInfo {
                ty: ptr_ty,
                name,
                symbol,
            });
            self.b().write_var(var, Operand::Value(slot));
            self.cur().slot_types.insert(var, ty);
            Binding::Slot(var)
        } else {
            let var = self.b().declare_var(VarInfo { ty, name, symbol });
            Binding::Var(var)
        };
        if let Some(n) = name {
            self.bind(n, binding);
        }
        binding
    }

    /// Declares a temporary SSA variable (results of value-producing control constructs).
    pub(super) fn temp_var(&mut self, ty: TypeId) -> VarId {
        self.b().declare_var(VarInfo {
            ty,
            name: None,
            symbol: None,
        })
    }

    /// Value type, cell-ness, name and symbol of a binding of frame `f`.
    pub(super) fn binding_meta(
        &self,
        f: usize,
        b: Binding,
    ) -> (TypeId, bool, Option<u32>, Option<u32>) {
        let st = &self.frames[f];
        match b {
            Binding::Var(v) => {
                let info = st.b.var_info(v);
                (info.ty, false, info.name, info.symbol)
            }
            Binding::Slot(v) | Binding::CellRef(v) => {
                let info = st.b.var_info(v);
                let ty = st.slot_types.get(&v).copied().unwrap_or(TypeTable::UNKNOWN);
                (ty, true, info.name, info.symbol)
            }
            Binding::Field { field, cell } => {
                let fi = &self.module.fields[field.index()];
                let ty = if cell {
                    self.module.types.pointee(fi.ty).unwrap_or(TypeTable::UNKNOWN)
                } else {
                    fi.ty
                };
                (ty, cell, Some(fi.name), fi.symbol)
            }
        }
    }

    /// Value type of a binding of the current frame.
    pub(super) fn binding_type(&self, b: Binding) -> TypeId {
        self.binding_meta(self.top(), b).0
    }

    /// Receiver of the current frame (`this` inside a member function).
    fn own_receiver(&mut self, span: Span) -> Option<ValueId> {
        let this = self.cur_ref().this?;
        match this {
            Binding::Var(v) => {
                let op = self.b().read_var(v);
                Some(self.value_of(Val::new(op, TypeTable::UNKNOWN), span))
            }
            other => {
                let v = self.read_binding(other, span);
                Some(self.value_of(v, span))
            }
        }
    }

    /// Reads a binding of the current frame.
    pub(super) fn read_binding(&mut self, b: Binding, span: Span) -> Val {
        let (ty, _, _, _) = self.binding_meta(self.top(), b);
        match b {
            Binding::Var(v) => {
                let op = self.b().read_var(v);
                Val::new(op, ty)
            }
            Binding::Slot(v) => {
                let ptr = self.var_value(v, span);
                let dst = self.b().emit_def(ty, span, |dst| Inst::Load {
                    dst,
                    place: Place::Local(ptr),
                });
                Val::new(Operand::Value(dst), ty)
            }
            Binding::CellRef(v) => {
                let ptr = self.var_value(v, span);
                let dst = self.b().emit_def(ty, span, |dst| Inst::Load {
                    dst,
                    place: Place::Deref(ptr),
                });
                Val::new(Operand::Value(dst), ty)
            }
            Binding::Field { field, cell } => {
                let Some(this) = self.own_receiver(span) else {
                    return Val::undef();
                };
                let field_ty = self.module.fields[field.index()].ty;
                let loaded = self.b().emit_def(field_ty, span, |dst| Inst::Load {
                    dst,
                    place: Place::Field { base: this, field },
                });
                if cell {
                    let dst = self.b().emit_def(ty, span, |dst| Inst::Load {
                        dst,
                        place: Place::Deref(loaded),
                    });
                    Val::new(Operand::Value(dst), ty)
                } else {
                    Val::new(Operand::Value(loaded), ty)
                }
            }
        }
    }

    /// Assigns a binding of the current frame.
    pub(super) fn write_binding(&mut self, b: Binding, value: Val, span: Span) {
        match b {
            Binding::Var(v) => self.b().write_var(v, value.op),
            Binding::Slot(v) => {
                let ptr = self.var_value(v, span);
                self.b().emit(
                    Inst::Store {
                        place: Place::Local(ptr),
                        value: value.op,
                    },
                    span,
                );
            }
            Binding::CellRef(v) => {
                let ptr = self.var_value(v, span);
                self.b().emit(
                    Inst::Store {
                        place: Place::Deref(ptr),
                        value: value.op,
                    },
                    span,
                );
            }
            Binding::Field { field, cell } => {
                let Some(this) = self.own_receiver(span) else {
                    return;
                };
                if cell {
                    let field_ty = self.module.fields[field.index()].ty;
                    let ptr = self.b().emit_def(field_ty, span, |dst| Inst::Load {
                        dst,
                        place: Place::Field { base: this, field },
                    });
                    self.b().emit(
                        Inst::Store {
                            place: Place::Deref(ptr),
                            value: value.op,
                        },
                        span,
                    );
                } else {
                    self.b().emit(
                        Inst::Store {
                            place: Place::Field { base: this, field },
                            value: value.op,
                        },
                        span,
                    );
                }
            }
        }
    }

    /// Value a closure captures for a binding: the value itself, or the cell address for
    /// shared variables.
    pub(super) fn capture_operand(&mut self, b: Binding, span: Span) -> Operand {
        match b {
            Binding::Var(v) | Binding::Slot(v) | Binding::CellRef(v) => self.b().read_var(v),
            Binding::Field { field, .. } => {
                let Some(this) = self.own_receiver(span) else {
                    return Operand::Undef;
                };
                let field_ty = self.module.fields[field.index()].ty;
                let dst = self.b().emit_def(field_ty, span, |dst| Inst::Load {
                    dst,
                    place: Place::Field { base: this, field },
                });
                Operand::Value(dst)
            }
        }
    }

    fn var_value(&mut self, v: VarId, span: Span) -> ValueId {
        let ty = self.cur_ref().b.var_info(v).ty;
        let op = self.b().read_var(v);
        self.value_of(Val::new(op, ty), span)
    }

    /// The value of `v` as a value id (constants and undefined operands are copied into a
    /// fresh value).
    pub(super) fn value_of(&mut self, v: Val, span: Span) -> ValueId {
        match v.op {
            Operand::Value(id) => id,
            other => self.b().emit_def(v.ty, span, |dst| Inst::Assign { dst, src: other }),
        }
    }

    // ── Captures ──

    /// Makes `outer`, a binding of frame `f - 1`, available in frame `f`.
    pub(super) fn import_binding(&mut self, f: usize, outer: Binding) -> Binding {
        if f == 0 || f >= self.frames.len() {
            return outer;
        }
        match self.frames[f].capture {
            CaptureSite::None => outer,
            CaptureSite::Closure => {
                if let Some(&(_, inner)) = self.frames[f].captures.iter().find(|(o, _)| *o == outer)
                {
                    return inner;
                }
                let (ty, cell, name, symbol) = self.binding_meta(f - 1, outer);
                let param_ty = if cell {
                    self.module.types.ptr_to(ty)
                } else {
                    ty
                };
                let st = &mut self.frames[f];
                let pos = st.captures.len();
                let param = st.b.insert_param(
                    pos,
                    ValueInfo {
                        ty: param_ty,
                        name,
                        symbol,
                    },
                );
                let var = st.b.declare_var(VarInfo {
                    ty: param_ty,
                    name,
                    symbol,
                });
                let entry = st.b.func().entry;
                st.b.write_var_in(var, entry, Operand::Value(param));
                let inner = if cell {
                    st.slot_types.insert(var, ty);
                    Binding::CellRef(var)
                } else {
                    Binding::Var(var)
                };
                st.captures.push((outer, inner));
                inner
            }
            CaptureSite::ClassMember => {
                let Some(class_node) = self.frames[f].class_node else {
                    return outer;
                };
                if let Some(cap) = self
                    .classes
                    .get(&class_node)
                    .and_then(|p| p.local.as_ref())
                    .and_then(|l| l.captures.iter().find(|c| c.outer == outer).copied())
                {
                    return Binding::Field {
                        field: cap.field,
                        cell: cap.cell,
                    };
                }
                let (ty, cell, name, _) = self.binding_meta(f - 1, outer);
                self.add_class_capture(class_node, outer, ty, cell, name)
            }
        }
    }

    /// Adds a captured binding of the declaring frame to a local class.
    pub(super) fn add_class_capture(
        &mut self,
        class_node: u32,
        outer: Binding,
        ty: TypeId,
        cell: bool,
        name: Option<u32>,
    ) -> Binding {
        let owner = self.classes.get(&class_node).and_then(|p| p.symbol);
        let mut label = b"<capture:".to_vec();
        match name {
            Some(n) => label.extend_from_slice(self.module.strings.lookup_text(n)),
            None => label.extend_from_slice(b"this"),
        }
        label.push(b'>');
        let field_ty = if cell {
            self.module.types.ptr_to(ty)
        } else {
            ty
        };
        let field = self.synthetic_field(owner, &label, field_ty);
        if let Some(local) = self
            .classes
            .get_mut(&class_node)
            .and_then(|p| p.local.as_mut())
        {
            local.captures.push(ClassCapture { outer, field, cell });
        }
        Binding::Field { field, cell }
    }

    /// Makes a binding of frame `from` available in frame `to` (`to >= from`).
    pub(super) fn import_into(&mut self, to: usize, binding: Binding, from: usize) -> Binding {
        if to <= from {
            return binding;
        }
        let mid = self.import_into(to - 1, binding, from);
        self.import_binding(to, mid)
    }

    // ── Name resolution ──

    pub(super) fn resolve_name(&mut self, name: u32) -> Resolved {
        let f = self.top();
        self.resolve_in(f, name)
    }

    /// Resolves a simple name in frame `f`: lexical scopes, members of the enclosing type
    /// (declared or inherited), enclosing frames, enclosing types, module variables, types.
    pub(super) fn resolve_in(&mut self, f: usize, name: u32) -> Resolved {
        if name == u32::MAX {
            return Resolved::Unresolved;
        }
        for scope in self.frames[f].scopes.iter().rev() {
            if let Some(&(_, b)) = scope.iter().rev().find(|(n, _)| *n == name) {
                return Resolved::Local(b);
            }
        }

        let capture = self.frames[f].capture;
        if capture != CaptureSite::Closure {
            if let Some(cls) = self.frames[f].class_symbol {
                if let Some(r) = self.member_field_in(f, cls, name) {
                    return r;
                }
            }
        }

        if f > 0 && capture != CaptureSite::None {
            return match self.resolve_in(f - 1, name) {
                Resolved::Local(b) => Resolved::Local(self.import_binding(f, b)),
                Resolved::Field { receiver, field } => Resolved::Field {
                    receiver: Receiver {
                        base: self.import_binding(f, receiver.base),
                        hops: receiver.hops,
                    },
                    field,
                },
                other => other,
            };
        }

        // Enclosing member types: their static members, and their instance members through
        // the enclosing-instance chain.
        if let Some(cls) = self.frames[f].class_symbol {
            let mut hops: Vec<FieldId> = Vec::new();
            let mut linked = true;
            let mut cur = cls;
            while let Some(outer) = self.index.enclosing_type(cur) {
                match self.outer_field_of(cur) {
                    Some(l) if linked => hops.push(l),
                    _ => linked = false,
                }
                if let Some((fsym, declaring)) = self.index.field_in_hierarchy(outer, name) {
                    let field = self.field_for_symbol(fsym);
                    if self.index.is_static(fsym) {
                        return Resolved::Static {
                            owner: declaring,
                            field,
                        };
                    }
                    return match (linked, self.frames[f].this) {
                        (true, Some(this)) => Resolved::Field {
                            receiver: Receiver { base: this, hops },
                            field,
                        },
                        _ => {
                            let span = self.frames[f].b.func().span;
                            self.diag(
                                DiagnosticKind::NotDerivable,
                                span,
                                format!(
                                    "instance field '{}' of an enclosing type is not reachable: \
                                     the language frontend does not state that member types \
                                     hold an enclosing instance",
                                    self.name_text(name)
                                ),
                            );
                            Resolved::Unresolved
                        }
                    };
                }
                cur = outer;
            }
        }

        if let Some(file) = self.frames[f].file {
            if let Some(&g) = self.globals_by_name.get(&(file, name)) {
                return Resolved::Global(g);
            }
        }

        let ctx = self.type_ctx_of_frame(f);
        if let Some(t) = self.resolve_type_name(name, &ctx) {
            return Resolved::Type(t);
        }
        Resolved::Unresolved
    }

    /// Field `name` of the type `cls` (or inherited) seen from frame `f`.
    fn member_field_in(&mut self, f: usize, cls: u32, name: u32) -> Option<Resolved> {
        let (fsym, declaring) = self.index.field_in_hierarchy(cls, name)?;
        let field = self.field_for_symbol(fsym);
        if self.index.is_static(fsym) {
            return Some(Resolved::Static {
                owner: declaring,
                field,
            });
        }
        match self.frames[f].this {
            Some(this) => Some(Resolved::Field {
                receiver: Receiver {
                    base: this,
                    hops: Vec::new(),
                },
                field,
            }),
            None => Some(Resolved::Unresolved),
        }
    }

    /// Synthetic enclosing-instance field of an inner member type, when the language
    /// frontend states that non-static member types hold one.
    pub(super) fn outer_field_of(&mut self, class_symbol: u32) -> Option<FieldId> {
        let node = *self.class_of_symbol.get(&class_symbol)?;
        let plan = self.classes.get(&node)?;
        if plan.local.is_some() || self.index.is_static(class_symbol) {
            return None;
        }
        if let Some(f) = plan.outer_field {
            return Some(f);
        }
        let outer = self.index.enclosing_type(class_symbol)?;
        // Member types only: the parent symbol is the enclosing type itself.
        if self.index.parent(class_symbol) != Some(outer) {
            return None;
        }
        let sem = self.sem_of_file(plan.file);
        if sem.inner_types_capture_outer_instance() != Some(true) {
            return None;
        }
        let ty = self.module.types.named(outer);
        let f = self.synthetic_field(Some(class_symbol), b"<outer>", ty);
        if let Some(p) = self.classes.get_mut(&node) {
            p.outer_field = Some(f);
        }
        Some(f)
    }

    /// Loads the object a receiver path designates.
    pub(super) fn read_receiver(&mut self, r: &Receiver, span: Span) -> Option<ValueId> {
        let base = self.read_binding(r.base, span);
        let mut cur = self.value_of(base, span);
        for &hop in &r.hops {
            let ty = self.module.fields[hop.index()].ty;
            cur = self.b().emit_def(ty, span, |dst| Inst::Load {
                dst,
                place: Place::Field { base: cur, field: hop },
            });
        }
        Some(cur)
    }

    /// `this` of the current frame (closures see the receiver of their enclosing frame).
    pub(super) fn this_binding(&mut self) -> Option<Binding> {
        let f = self.top();
        self.this_binding_in(f)
    }

    fn this_binding_in(&mut self, f: usize) -> Option<Binding> {
        match self.frames[f].capture {
            CaptureSite::Closure if f > 0 => {
                let outer = self.this_binding_in(f - 1)?;
                Some(self.import_binding(f, outer))
            }
            _ => self.frames[f].this,
        }
    }

    /// Instance of type `target` (or a subtype) visible from the current frame: the
    /// receiver, the receiver of an enclosing frame, or an enclosing instance.
    pub(super) fn instance_of(&mut self, target: u32) -> Option<Receiver> {
        let f = self.top();
        self.instance_of_in(f, target)
    }

    fn instance_of_in(&mut self, f: usize, target: u32) -> Option<Receiver> {
        match self.frames[f].capture {
            CaptureSite::Closure if f > 0 => {
                let r = self.instance_of_in(f - 1, target)?;
                Some(Receiver {
                    base: self.import_binding(f, r.base),
                    hops: r.hops,
                })
            }
            capture => {
                let this = self.frames[f].this?;
                let cls = self.frames[f].class_symbol?;
                if self.index.is_subtype(cls, target) {
                    return Some(Receiver {
                        base: this,
                        hops: Vec::new(),
                    });
                }
                if capture == CaptureSite::ClassMember && f > 0 {
                    let r = self.instance_of_in(f - 1, target)?;
                    return Some(Receiver {
                        base: self.import_binding(f, r.base),
                        hops: r.hops,
                    });
                }
                let mut hops = Vec::new();
                let mut cur = cls;
                while let Some(link) = self.outer_field_of(cur) {
                    hops.push(link);
                    cur = self.index.enclosing_type(cur)?;
                    if self.index.is_subtype(cur, target) {
                        return Some(Receiver { base: this, hops });
                    }
                }
                None
            }
        }
    }

    // ── Exits ──

    /// Landing block an exception raised at the insertion point reaches.
    pub(super) fn unwind_target(&mut self) -> Option<BlockId> {
        let st = self.cur();
        for i in (0..st.exits.len()).rev() {
            match &mut st.exits[i] {
                Exit::Handler { landing } => return Some(*landing),
                Exit::Cleanup { landing, .. } => {
                    if let Some(l) = landing {
                        return Some(*l);
                    }
                    let l = st.b.create_block();
                    *landing = Some(l);
                    return Some(l);
                }
                Exit::Breakable { .. } => {}
            }
        }
        None
    }

    pub(super) fn push_exit(&mut self, exit: Exit) {
        self.cur().exits.push(exit);
    }

    pub(super) fn pop_exit(&mut self) -> Option<Exit> {
        self.cur().exits.pop()
    }

    pub(super) fn push_cleanup(&mut self, action: Cleanup) {
        self.cur().exits.push(Exit::Cleanup {
            action,
            landing: None,
        });
    }

    /// Leaves the innermost cleanup region: runs its work on the normal path and emits its
    /// landing block (work, then rethrow) when anything inside could raise.
    pub(super) fn pop_cleanup(&mut self, span: Span) {
        let Some(Exit::Cleanup { action, landing }) = self.pop_exit() else {
            return;
        };
        if self.cur_ref().b.is_open() {
            self.emit_cleanup(action);
        }
        if let Some(l) = landing {
            let resume = self.cur_ref().b.current_block();
            self.b().seal(l);
            self.b().switch_to(l);
            let exc = self.exception_value(span);
            self.emit_cleanup(action);
            let unwind = self.unwind_target();
            self.b().throw(Operand::Value(exc), unwind, span);
            if let Some(r) = resume {
                self.b().switch_to(r);
            }
        }
    }

    /// The exception arriving in the current landing block.
    pub(super) fn exception_value(&mut self, span: Span) -> ValueId {
        self.intrinsic(IntrinsicKind::Exception, Vec::new(), Some(TypeTable::UNKNOWN), span)
            .expect("intrinsic with a result type defines a value")
    }

    /// Runs, innermost first, the cleanups of the regions above `depth` (for an exit that
    /// leaves them). Each cleanup runs with only the regions outside it active.
    pub(super) fn run_cleanups_down_to(&mut self, depth: usize) {
        let len = self.cur_ref().exits.len();
        for j in (depth..len).rev() {
            let action = match self.cur_ref().exits.get(j) {
                Some(Exit::Cleanup { action, .. }) => Some(*action),
                _ => None,
            };
            if let Some(a) = action {
                let saved = self.cur().exits.split_off(j);
                self.emit_cleanup(a);
                let st = self.cur();
                st.exits.truncate(j);
                st.exits.extend(saved);
            }
        }
    }

    /// True when a region with exit work lies above `depth`.
    pub(super) fn has_cleanups_above(&self, depth: usize) -> bool {
        self.cur_ref().exits[depth.min(self.cur_ref().exits.len())..]
            .iter()
            .any(|e| matches!(e, Exit::Cleanup { .. }))
    }

    fn emit_cleanup(&mut self, action: Cleanup) {
        match action {
            Cleanup::Finally(node) => {
                let body = self.syn().finally_body(node);
                self.stmt(body);
            }
            Cleanup::Defer(node) => {
                let syn = self.syn();
                if syn.class(node).is_expression() {
                    self.expr(node);
                } else {
                    self.stmt(node);
                }
            }
            Cleanup::Release { resource, ty, span } => self.emit_release(resource, ty, span),
            Cleanup::Monitor { lock, span } => self.b().emit(
                Inst::Release {
                    place: Place::Deref(lock),
                },
                span,
            ),
            Cleanup::Destroy { slot, span } => self.b().emit(
                Inst::Release {
                    place: Place::Local(slot),
                },
                span,
            ),
        }
    }

    /// Releases the resource of a resource region: the frontend's release call (skipped for
    /// null when the language says so), then the end of the region.
    fn emit_release(&mut self, resource: ValueId, ty: TypeId, span: Span) {
        let protocol = self.cur_ref().sem.resource_protocol();
        match protocol {
            Some(p) => {
                let done = if p.null_checked {
                    let is_set = self.b().emit_def(TypeTable::BOOL, span, |dst| Inst::Compare {
                        dst,
                        pred: crate::ir::inst::ComparePred::Ne,
                        lhs: Operand::Value(resource),
                        rhs: Operand::Const(ConstVal::Null),
                    });
                    let release = self.b().create_block();
                    let done = self.b().create_block();
                    self.b().branch(Operand::Value(is_set), release, done, span);
                    self.b().seal(release);
                    self.b().switch_to(release);
                    Some(done)
                } else {
                    None
                };
                let name = self.intern(p.release_method);
                let recv = Val::new(Operand::Value(resource), ty);
                let owner = self.module.types.symbol_of(ty);
                self.invoke_method(Some(recv), owner, name, Vec::new(), false, span);
                self.b().emit(
                    Inst::Release {
                        place: Place::Deref(resource),
                    },
                    span,
                );
                if let Some(d) = done {
                    self.b().jump(d, span);
                    self.b().seal(d);
                    self.b().switch_to(d);
                }
            }
            None => {
                self.diag_once(
                    DiagnosticKind::NotDerivable,
                    span,
                    "the language frontend names no release method for resource regions; \
                     only the end of each resource region is recorded"
                        .to_string(),
                );
                self.b().emit(
                    Inst::Release {
                        place: Place::Deref(resource),
                    },
                    span,
                );
            }
        }
    }

    /// Target of `break` / `continue` (with an optional label): exit-stack depth, target
    /// block and the switch-expression result variable.
    pub(super) fn break_target(
        &self,
        label: Option<u32>,
        is_continue: bool,
    ) -> Option<(usize, BlockId, Option<VarId>)> {
        let exits = &self.cur_ref().exits;
        for i in (0..exits.len()).rev() {
            if let Exit::Breakable {
                label: l,
                kind,
                break_to,
                continue_to,
                result,
            } = &exits[i]
            {
                match label {
                    Some(name) if *l == Some(name) => {
                        return if is_continue {
                            continue_to.map(|c| (i, c, None))
                        } else {
                            Some((i, *break_to, *result))
                        };
                    }
                    Some(_) => {}
                    None if is_continue => {
                        if *kind == BreakKind::Loop {
                            if let Some(c) = continue_to {
                                return Some((i, *c, None));
                            }
                        }
                    }
                    None => {
                        if matches!(kind, BreakKind::Loop | BreakKind::Switch) {
                            return Some((i, *break_to, *result));
                        }
                    }
                }
            }
        }
        None
    }

    /// Innermost switch expression: depth, exit block and result variable.
    pub(super) fn yield_target(&self) -> Option<(usize, BlockId, VarId)> {
        let exits = &self.cur_ref().exits;
        for i in (0..exits.len()).rev() {
            if let Exit::Breakable {
                break_to,
                result: Some(r),
                ..
            } = &exits[i]
            {
                return Some((i, *break_to, *r));
            }
        }
        None
    }

    // ── Emission helpers ──

    pub(super) fn emit(&mut self, inst: Inst, span: Span) {
        self.b().emit(inst, span);
    }

    pub(super) fn emit_def(&mut self, ty: TypeId, span: Span, make: impl FnOnce(ValueId) -> Inst) -> Val {
        let v = self.b().emit_def(ty, span, make);
        Val::new(Operand::Value(v), ty)
    }

    /// Emits a call inside the active protected regions.
    pub(super) fn emit_call(
        &mut self,
        ret: TypeId,
        callee: Callee,
        args: Vec<Operand>,
        meta: CallMeta,
        span: Span,
    ) -> Val {
        let unwind = self.unwind_target();
        let dst_ty = if ret == TypeTable::VOID { None } else { Some(ret) };
        match self.b().emit_call(dst_ty, callee, args, unwind, meta, span) {
            Some(v) => Val::new(Operand::Value(v), ret),
            None => Val::new(Operand::Undef, TypeTable::VOID),
        }
    }

    pub(super) fn intrinsic(
        &mut self,
        kind: IntrinsicKind,
        args: Vec<Operand>,
        ty: Option<TypeId>,
        span: Span,
    ) -> Option<ValueId> {
        let name = self.intern(kind.as_str().as_bytes());
        match ty {
            Some(t) => Some(self.b().emit_def(t, span, |dst| Inst::Intrinsic {
                dst: Some(dst),
                name,
                args,
            })),
            None => {
                self.b().emit(
                    Inst::Intrinsic {
                        dst: None,
                        name,
                        args,
                    },
                    span,
                );
                None
            }
        }
    }

    /// An explicit not-derivable value: the intrinsic carries a description and the lowered
    /// operands of the construct.
    pub(super) fn opaque(&mut self, kind: IntrinsicKind, what: &[u8], mut operands: Vec<Operand>, span: Span) -> Val {
        let label = self.intern(what);
        operands.insert(0, Operand::Const(ConstVal::Str(label)));
        let v = self
            .intrinsic(kind, operands, Some(TypeTable::UNKNOWN), span)
            .expect("intrinsic with a result type defines a value");
        Val::new(Operand::Value(v), TypeTable::UNKNOWN)
    }

    /// Pointer type of a cell holding values of `ty`.
    pub(super) fn ptr_type(&mut self, ty: TypeId) -> TypeId {
        self.module.types.intern(IrType::Ptr(ty))
    }
}
