//! Expression lowering: three-address code over SSA values, explicit places for memory
//! accesses, branches for short-circuit and conditional operators, calls bound by dispatch
//! kind, and object and array creation as allocation plus initialisation.

use crate::core::types::ast::{NodeAttr, OperatorArity, OperatorId};
use crate::ir::builder::CallMeta;
use crate::ir::inst::{BinaryOp, Callee, ComparePred, Inst, IntrinsicKind, Region, UnaryOp};
use crate::ir::lower::frame::{Binding, Resolved, Val};
use crate::ir::lower::semantics::{DispatchRule, LiteralClass, LiteralValue};
use crate::ir::lower::syntax::{CallForm, NodeClass};
use crate::ir::lower::{JobKind, Lowerer};
use crate::ir::module::DiagnosticKind;
use crate::ir::types::{BlockId, FuncId, IrType, Span, TypeId, TypeTable, ValueId};
use crate::ir::value::{ConstVal, Operand, Place};

/// What the qualifier of a member access or call denotes.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Qual {
    Type(u32),
    /// A dotted prefix that names no value or type (a package prefix, or unresolved).
    Package(Vec<u32>),
    Value(Val),
    /// `super`: members of the superclass on the current receiver.
    Super,
}

/// Assignable location.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum LValue {
    Binding(Binding),
    Place(Place, TypeId),
    /// The target is not derivable; stores are dropped (and stated).
    Opaque,
}

/// Outcome of overload resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Selection {
    One(u32),
    /// No unique applicable declaration; the remaining candidates.
    Open(Vec<u32>),
}

impl<'t> Lowerer<'t> {
    pub(super) fn expr(&mut self, n: u32) -> Val {
        self.expr_hint(n, None)
    }

    /// Lowers an expression; `hint` is the type the context expects (array literals take
    /// their element type from it).
    pub(super) fn expr_hint(&mut self, n: u32, hint: Option<TypeId>) -> Val {
        let syn = self.syn();
        let span = self.tree.span(n);
        match syn.class(n) {
            NodeClass::Literal => self.literal_expr(n),
            NodeClass::Name => self.name_expr(n),
            NodeClass::This => self.this_expr(n),
            NodeClass::Super => self.this_expr(n),
            NodeClass::FieldAccess => self.member_expr(n),
            NodeClass::Index => self.index_expr(n),
            NodeClass::Assign => self.assign_expr(n),
            NodeClass::Binary => self.binary_expr(n),
            NodeClass::Unary => self.unary_expr(n),
            NodeClass::Ternary => self.ternary_expr(n),
            NodeClass::Call => self.call_expr(n),
            NodeClass::New => self.new_expr(n),
            NodeClass::ArrayNew => self.array_new_expr(n),
            NodeClass::ArrayLit => {
                let ty = hint.unwrap_or(TypeTable::UNKNOWN);
                self.array_literal(n, ty)
            }
            NodeClass::Cast => self.cast_expr(n),
            NodeClass::TypeTest => self.type_test_expr(n),
            NodeClass::Lambda => self.lambda_expr(n),
            NodeClass::MethodRef => self.method_ref_expr(n),
            NodeClass::SwitchExpr | NodeClass::Switch => self
                .switch_construct(n, true)
                .unwrap_or_else(Val::undef),
            NodeClass::ExprStmt => match syn.active_kids(n).first().copied() {
                Some(inner) => self.expr_hint(inner, hint),
                None => Val::undef(),
            },
            NodeClass::TypeRef => {
                let ty = self.type_of_ref(n);
                self.type_literal(ty, n, span)
            }
            other => {
                let operands: Vec<Operand> = syn
                    .active_kids(n)
                    .into_iter()
                    .filter(|&c| syn.class(c).is_expression())
                    .map(|c| self.expr(c).op)
                    .collect();
                self.diag(
                    DiagnosticKind::NotDerivable,
                    span,
                    format!("no lowering rule for expression node class {:?}", other),
                );
                self.opaque(IntrinsicKind::Opaque, format!("{:?}", other).as_bytes(), operands, span)
            }
        }
    }

    // ── Literals and names ──

    fn literal_expr(&mut self, n: u32) -> Val {
        let syn = self.syn();
        let span = self.tree.span(n);
        let text = self.tree.text(n);
        let Some(class) = syn.literal_class(n) else {
            let id = self.intern(text);
            self.diag_once(
                DiagnosticKind::NotDerivable,
                span,
                "literal category is recorded neither by the AST nor by the token stream".to_string(),
            );
            return self.opaque(IntrinsicKind::OpaqueLiteral, text, vec![Operand::Const(ConstVal::Str(id))], span);
        };
        match class {
            LiteralClass::Null => return Val::new(Operand::null(), TypeTable::UNKNOWN),
            LiteralClass::Type => {
                let ty = match syn.first_kid_of(n, NodeClass::TypeRef) {
                    Some(t) => self.type_of_ref(t),
                    None => TypeTable::UNKNOWN,
                };
                return self.type_literal(ty, n, span);
            }
            _ => {}
        }
        match syn.sem.literal(class, text) {
            Some(lit) => {
                let ty = self.module.types.intern(lit.ty);
                let c = match lit.value {
                    LiteralValue::Bool(b) => ConstVal::Bool(b),
                    LiteralValue::Int(i) => ConstVal::Int(i),
                    LiteralValue::Float(f) => ConstVal::float(f),
                    LiteralValue::Char(c) => ConstVal::Int(c as i128),
                    LiteralValue::Str(bytes) => ConstVal::Str(self.intern(&bytes)),
                    LiteralValue::Null => ConstVal::Null,
                };
                Val::new(Operand::Const(c), ty)
            }
            None => {
                let id = self.intern(text);
                self.diag_once(
                    DiagnosticKind::NotDerivable,
                    span,
                    format!("the language frontend gives no value for {:?} literals", class),
                );
                let ty = match class {
                    LiteralClass::String | LiteralClass::TextBlock => TypeTable::STR,
                    LiteralClass::Char => TypeTable::CHAR,
                    LiteralClass::Boolean => TypeTable::BOOL,
                    _ => TypeTable::UNKNOWN,
                };
                let v = self.opaque(IntrinsicKind::OpaqueLiteral, text, vec![Operand::Const(ConstVal::Str(id))], span);
                Val::new(v.op, ty)
            }
        }
    }

    /// Runtime object denoting a type.
    fn type_literal(&mut self, ty: TypeId, n: u32, span: Span) -> Val {
        let arg = match self.module.types.symbol_of(ty) {
            Some(sym) => Operand::Const(ConstVal::Int(sym as i128)),
            None => {
                let text = match self.tree.token_range(n) {
                    Some((ft, lt)) => self.tree.joined_text(ft, lt),
                    None => Vec::new(),
                };
                Operand::Const(ConstVal::Str(self.intern(&text)))
            }
        };
        let v = self
            .intrinsic(IntrinsicKind::TypeLiteral, vec![arg], Some(TypeTable::UNKNOWN), span)
            .expect("intrinsic with a result type defines a value");
        Val::new(Operand::Value(v), TypeTable::UNKNOWN)
    }

    fn name_expr(&mut self, n: u32) -> Val {
        let span = self.tree.span(n);
        let name = self.tree.text_id(n).unwrap_or(u32::MAX);
        let r = self.resolve_name(name);
        self.resolved_value(r, name, span)
    }

    /// Value of a resolved name.
    pub(super) fn resolved_value(&mut self, r: Resolved, name: u32, span: Span) -> Val {
        match r {
            Resolved::Local(b) => self.read_binding(b, span),
            Resolved::Field { receiver, field } => {
                let ty = self.module.fields[field.index()].ty;
                match self.read_receiver(&receiver, span) {
                    Some(base) => self.emit_def(ty, span, |dst| Inst::Load {
                        dst,
                        place: Place::Field { base, field },
                    }),
                    None => Val::undef(),
                }
            }
            Resolved::Static { owner, field } => {
                let ty = self.module.fields[field.index()].ty;
                self.emit_def(ty, span, |dst| Inst::Load {
                    dst,
                    place: Place::Static { owner, field },
                })
            }
            Resolved::Global(g) => {
                let ty = self.module.globals[g.index()].ty;
                self.emit_def(ty, span, |dst| Inst::Load {
                    dst,
                    place: Place::Global(g),
                })
            }
            Resolved::Type(t) => {
                let v = self
                    .intrinsic(
                        IntrinsicKind::TypeLiteral,
                        vec![Operand::Const(ConstVal::Int(t as i128))],
                        Some(TypeTable::UNKNOWN),
                        span,
                    )
                    .expect("intrinsic with a result type defines a value");
                Val::new(Operand::Value(v), TypeTable::UNKNOWN)
            }
            Resolved::Unresolved => self.unresolved_name(name, None, span),
        }
    }

    fn unresolved_name(&mut self, name: u32, qualifier: Option<Operand>, span: Span) -> Val {
        let text = self.name_text(name);
        self.diag(
            DiagnosticKind::Unresolved,
            span,
            format!("name '{}' is bound by no declaration in scope", text),
        );
        let mut args = vec![Operand::Const(ConstVal::Str(name))];
        if let Some(q) = qualifier {
            args.push(q);
        }
        let v = self
            .intrinsic(IntrinsicKind::UnresolvedName, args, Some(TypeTable::UNKNOWN), span)
            .expect("intrinsic with a result type defines a value");
        Val::new(Operand::Value(v), TypeTable::UNKNOWN)
    }

    fn this_expr(&mut self, n: u32) -> Val {
        let span = self.tree.span(n);
        let syn = self.syn();
        // `Outer.this`: a type qualifier selects an enclosing instance.
        if let Some(q) = syn.kids(n).first().copied() {
            if matches!(syn.class(q), NodeClass::TypeRef | NodeClass::Name) {
                let ty = match syn.class(q) {
                    NodeClass::TypeRef => self.type_of_ref(q),
                    _ => {
                        let name = self.tree.text_id(q).unwrap_or(u32::MAX);
                        let ctx = self.type_ctx_of_frame(self.top());
                        match self.resolve_type_name(name, &ctx) {
                            Some(s) => self.module.types.named(s),
                            None => TypeTable::UNKNOWN,
                        }
                    }
                };
                if let Some(target) = self.module.types.symbol_of(ty) {
                    if let Some(r) = self.instance_of(target) {
                        if let Some(v) = self.read_receiver(&r, span) {
                            return Val::new(Operand::Value(v), ty);
                        }
                    }
                }
                self.diag(
                    DiagnosticKind::NotDerivable,
                    span,
                    "qualified receiver names no enclosing instance reachable here".to_string(),
                );
                return self.opaque(IntrinsicKind::Opaque, b"qualified-this", Vec::new(), span);
            }
        }
        match self.this_binding() {
            Some(b) => {
                let v = self.read_binding(b, span);
                let ty = match self.cur_ref().class_symbol {
                    Some(c) => self.module.types.named(c),
                    None => v.ty,
                };
                Val::new(v.op, ty)
            }
            None => {
                self.diag(
                    DiagnosticKind::NotDerivable,
                    span,
                    "receiver used where the function has none".to_string(),
                );
                self.opaque(IntrinsicKind::Opaque, b"receiver", Vec::new(), span)
            }
        }
    }

    // ── Member and element access ──

    /// Meaning of the qualifier of a member access or call, without loading a value for a
    /// type or package qualifier.
    pub(super) fn qualifier(&mut self, n: u32) -> Qual {
        let syn = self.syn();
        let span = self.tree.span(n);
        match syn.class(n) {
            NodeClass::Super => Qual::Super,
            NodeClass::Name => {
                let name = self.tree.text_id(n).unwrap_or(u32::MAX);
                match self.resolve_name(name) {
                    Resolved::Type(t) => Qual::Type(t),
                    Resolved::Unresolved => Qual::Package(vec![name]),
                    r => Qual::Value(self.resolved_value(r, name, span)),
                }
            }
            NodeClass::TypeRef => {
                let ty = self.type_of_ref(n);
                match self.module.types.symbol_of(ty) {
                    Some(s) => Qual::Type(s),
                    None => Qual::Package(Vec::new()),
                }
            }
            NodeClass::FieldAccess => {
                let (obj, name_tok) = syn.member_parts(n);
                let name = name_tok
                    .and_then(|t| self.tree.token_text_id(t))
                    .unwrap_or(u32::MAX);
                let inner = match obj {
                    Some(o) => self.qualifier(o),
                    None => return Qual::Value(Val::undef()),
                };
                let selects_this = syn
                    .kids(n)
                    .last()
                    .is_some_and(|&l| syn.class(l) == NodeClass::This);
                if selects_this {
                    // `Outer.this`: the enclosing instance of the qualifying type.
                    if let Qual::Type(t) = inner {
                        let ty = self.module.types.named(t);
                        if let Some(r) = self.instance_of(t) {
                            if let Some(v) = self.read_receiver(&r, span) {
                                return Qual::Value(Val::new(Operand::Value(v), ty));
                            }
                        }
                    }
                    self.diag(
                        DiagnosticKind::NotDerivable,
                        span,
                        "qualified receiver names no enclosing instance reachable here".to_string(),
                    );
                    return Qual::Value(self.opaque(IntrinsicKind::Opaque, b"qualified-this", Vec::new(), span));
                }
                match inner {
                    Qual::Package(mut path) => {
                        path.push(name);
                        let qname = path
                            .iter()
                            .map(|&p| self.name_text(p))
                            .collect::<Vec<_>>()
                            .join(".");
                        match self.index.qualified_type(&qname) {
                            Some(t) => Qual::Type(t),
                            None => Qual::Package(path),
                        }
                    }
                    Qual::Type(t) => {
                        if let Some(m) = self.index.member_type_in_hierarchy(t, name) {
                            return Qual::Type(m);
                        }
                        Qual::Value(self.static_member(t, name, span))
                    }
                    Qual::Value(v) => Qual::Value(self.field_of_value(v, name, span)),
                    Qual::Super => Qual::Value(self.super_field(name, span)),
                }
            }
            _ => Qual::Value(self.expr(n)),
        }
    }

    fn member_expr(&mut self, n: u32) -> Val {
        let span = self.tree.span(n);
        match self.qualifier(n) {
            Qual::Value(v) => v,
            Qual::Type(t) => {
                let ty = self.module.types.named(t);
                self.type_literal(ty, n, span)
            }
            Qual::Package(path) => {
                let name = path.last().copied().unwrap_or(u32::MAX);
                self.unresolved_name(name, None, span)
            }
            Qual::Super => self.this_expr(n),
        }
    }

    /// Static member `name` of type `t`.
    fn static_member(&mut self, t: u32, name: u32, span: Span) -> Val {
        match self.index.field_in_hierarchy(t, name) {
            Some((fsym, declaring)) => {
                let field = self.field_for_symbol(fsym);
                let ty = self.module.fields[field.index()].ty;
                if !self.index.is_static(fsym) {
                    // An instance field named through its type: the receiver is the enclosing
                    // instance of that type.
                    if let Some(r) = self.instance_of(declaring) {
                        if let Some(base) = self.read_receiver(&r, span) {
                            return self.emit_def(ty, span, |dst| Inst::Load {
                                dst,
                                place: Place::Field { base, field },
                            });
                        }
                    }
                }
                self.emit_def(ty, span, |dst| Inst::Load {
                    dst,
                    place: Place::Static {
                        owner: declaring,
                        field,
                    },
                })
            }
            None => {
                let field = self.loose_field(t, name, true);
                self.diag(
                    DiagnosticKind::Unresolved,
                    span,
                    format!(
                        "static member '{}' is not declared by the project type",
                        self.name_text(name)
                    ),
                );
                self.emit_def(TypeTable::UNKNOWN, span, |dst| Inst::Load {
                    dst,
                    place: Place::Static { owner: t, field },
                })
            }
        }
    }

    /// Field `name` of the object `v` denotes.
    fn field_of_value(&mut self, v: Val, name: u32, span: Span) -> Val {
        let length = self.cur_ref().sem.array_length_member();
        if let (Some(len_name), IrType::Array { .. }) = (length, self.module.types.get(v.ty).clone()) {
            if self.module.strings.lookup_text(name) == len_name {
                let base = v.op;
                let int = self.module.types.int(32, true);
                let r = self
                    .intrinsic(IntrinsicKind::ArrayLength, vec![base], Some(int), span)
                    .expect("intrinsic with a result type defines a value");
                return Val::new(Operand::Value(r), int);
            }
        }
        let base = self.value_of(v, span);
        let owner = self.module.types.symbol_of(v.ty);
        let (field, ty) = match owner.and_then(|o| self.index.field_in_hierarchy(o, name)) {
            Some((fsym, declaring)) => {
                let field = self.field_for_symbol(fsym);
                let ty = self.module.fields[field.index()].ty;
                if self.index.is_static(fsym) {
                    return self.emit_def(ty, span, |dst| Inst::Load {
                        dst,
                        place: Place::Static {
                            owner: declaring,
                            field,
                        },
                    });
                }
                (field, ty)
            }
            None => {
                let field = self.loose_field(owner.unwrap_or(u32::MAX), name, false);
                self.diag(
                    DiagnosticKind::Unresolved,
                    span,
                    format!(
                        "field '{}' is not declared by the static type of its object",
                        self.name_text(name)
                    ),
                );
                (field, TypeTable::UNKNOWN)
            }
        };
        self.emit_def(ty, span, |dst| Inst::Load {
            dst,
            place: Place::Field { base, field },
        })
    }

    fn super_field(&mut self, name: u32, span: Span) -> Val {
        let sup = self.cur_ref().class_symbol.and_then(|c| self.index.superclass(c));
        let this = self.this_binding();
        match (sup, this) {
            (Some(s), Some(b)) => {
                let recv = self.read_binding(b, span);
                let ty = self.module.types.named(s);
                self.field_of_value(Val::new(recv.op, ty), name, span)
            }
            _ => self.unresolved_name(name, None, span),
        }
    }

    fn index_expr(&mut self, n: u32) -> Val {
        let span = self.tree.span(n);
        let kids = self.syn().active_kids(n);
        let (Some(&arr), Some(&idx)) = (kids.first(), kids.get(1)) else {
            return self.opaque(IntrinsicKind::Opaque, b"index", Vec::new(), span);
        };
        let a = self.expr(arr);
        let i = self.expr(idx);
        let base = self.value_of(a, span);
        let ty = self.module.types.element(a.ty).unwrap_or(TypeTable::UNKNOWN);
        self.emit_def(ty, span, |dst| Inst::Load {
            dst,
            place: Place::Index { base, index: i.op },
        })
    }

    // ── Assignment ──

    /// Assignable location of an expression; evaluates its address operands.
    pub(super) fn lvalue(&mut self, n: u32) -> LValue {
        let syn = self.syn();
        let span = self.tree.span(n);
        match syn.class(n) {
            NodeClass::Name => {
                let name = self.tree.text_id(n).unwrap_or(u32::MAX);
                match self.resolve_name(name) {
                    Resolved::Local(b) => LValue::Binding(b),
                    Resolved::Field { receiver, field } => {
                        let ty = self.module.fields[field.index()].ty;
                        match self.read_receiver(&receiver, span) {
                            Some(base) => LValue::Place(Place::Field { base, field }, ty),
                            None => LValue::Opaque,
                        }
                    }
                    Resolved::Static { owner, field } => {
                        let ty = self.module.fields[field.index()].ty;
                        LValue::Place(Place::Static { owner, field }, ty)
                    }
                    Resolved::Global(g) => {
                        let ty = self.module.globals[g.index()].ty;
                        LValue::Place(Place::Global(g), ty)
                    }
                    Resolved::Type(_) | Resolved::Unresolved => {
                        self.diag(
                            DiagnosticKind::Unresolved,
                            span,
                            format!("assigned name '{}' is bound by no variable", self.name_text(name)),
                        );
                        LValue::Opaque
                    }
                }
            }
            NodeClass::FieldAccess => {
                let (obj, name_tok) = syn.member_parts(n);
                let name = name_tok
                    .and_then(|t| self.tree.token_text_id(t))
                    .unwrap_or(u32::MAX);
                let Some(o) = obj else { return LValue::Opaque };
                match self.qualifier(o) {
                    Qual::Value(v) => self.field_place(v, name, span),
                    Qual::Super => {
                        let sup = self.cur_ref().class_symbol.and_then(|c| self.index.superclass(c));
                        match (sup, self.this_binding()) {
                            (Some(s), Some(b)) => {
                                let recv = self.read_binding(b, span);
                                let ty = self.module.types.named(s);
                                self.field_place(Val::new(recv.op, ty), name, span)
                            }
                            _ => LValue::Opaque,
                        }
                    }
                    Qual::Type(t) => match self.index.field_in_hierarchy(t, name) {
                        Some((fsym, declaring)) => {
                            let field = self.field_for_symbol(fsym);
                            let ty = self.module.fields[field.index()].ty;
                            LValue::Place(
                                Place::Static {
                                    owner: declaring,
                                    field,
                                },
                                ty,
                            )
                        }
                        None => {
                            let field = self.loose_field(t, name, true);
                            LValue::Place(Place::Static { owner: t, field }, TypeTable::UNKNOWN)
                        }
                    },
                    Qual::Package(_) => LValue::Opaque,
                }
            }
            NodeClass::Index => {
                let kids = syn.active_kids(n);
                let (Some(&arr), Some(&idx)) = (kids.first(), kids.get(1)) else {
                    return LValue::Opaque;
                };
                let a = self.expr(arr);
                let i = self.expr(idx);
                let base = self.value_of(a, span);
                let ty = self.module.types.element(a.ty).unwrap_or(TypeTable::UNKNOWN);
                LValue::Place(Place::Index { base, index: i.op }, ty)
            }
            NodeClass::This => match self.this_binding() {
                Some(b) => LValue::Binding(b),
                None => LValue::Opaque,
            },
            _ => {
                self.diag(
                    DiagnosticKind::NotDerivable,
                    span,
                    "assignment target is not a variable, field or element".to_string(),
                );
                self.expr(n);
                LValue::Opaque
            }
        }
    }

    fn field_place(&mut self, v: Val, name: u32, span: Span) -> LValue {
        let base = self.value_of(v, span);
        let owner = self.module.types.symbol_of(v.ty);
        match owner.and_then(|o| self.index.field_in_hierarchy(o, name)) {
            Some((fsym, declaring)) => {
                let field = self.field_for_symbol(fsym);
                let ty = self.module.fields[field.index()].ty;
                if self.index.is_static(fsym) {
                    LValue::Place(
                        Place::Static {
                            owner: declaring,
                            field,
                        },
                        ty,
                    )
                } else {
                    LValue::Place(Place::Field { base, field }, ty)
                }
            }
            None => {
                let field = self.loose_field(owner.unwrap_or(u32::MAX), name, false);
                LValue::Place(Place::Field { base, field }, TypeTable::UNKNOWN)
            }
        }
    }

    pub(super) fn lvalue_type(&self, lv: &LValue) -> TypeId {
        match lv {
            LValue::Binding(b) => self.binding_type(*b),
            LValue::Place(_, ty) => *ty,
            LValue::Opaque => TypeTable::UNKNOWN,
        }
    }

    pub(super) fn load_lvalue(&mut self, lv: &LValue, span: Span) -> Val {
        match lv {
            LValue::Binding(b) => self.read_binding(*b, span),
            LValue::Place(place, ty) => {
                let place = *place;
                self.emit_def(*ty, span, |dst| Inst::Load { dst, place })
            }
            LValue::Opaque => self.opaque(IntrinsicKind::Opaque, b"assignment-target", Vec::new(), span),
        }
    }

    pub(super) fn store_lvalue(&mut self, lv: &LValue, value: Val, span: Span) {
        match lv {
            LValue::Binding(b) => self.write_binding(*b, value, span),
            LValue::Place(place, _) => self.emit(
                Inst::Store {
                    place: *place,
                    value: value.op,
                },
                span,
            ),
            LValue::Opaque => {}
        }
    }

    fn assign_expr(&mut self, n: u32) -> Val {
        let span = self.tree.span(n);
        let kids = self.syn().active_kids(n);
        let (Some(&lhs), Some(&rhs)) = (kids.first(), kids.last()) else {
            return self.opaque(IntrinsicKind::Opaque, b"assignment", Vec::new(), span);
        };
        let op = self.operator_of(n, OperatorArity::Assignment, Some(lhs));
        // Address operands of the target are evaluated before the value.
        let lv = self.lvalue(lhs);
        let target_ty = self.lvalue_type(&lv);
        if op == OperatorId::Assign {
            let v = self.expr_hint(rhs, Some(target_ty));
            self.store_lvalue(&lv, v, span);
            return v;
        }
        let current = self.load_lvalue(&lv, span);
        let r = self.expr(rhs);
        let result = match BinaryOp::from_compound_assignment(op) {
            Some(bop) => self.arith(bop, current, r, span),
            None => {
                let text = self.operator_text(n, Some(lhs));
                self.diag_once(
                    DiagnosticKind::NotDerivable,
                    span,
                    "assignment operator is recorded neither by the AST nor by the language frontend".to_string(),
                );
                self.opaque(IntrinsicKind::UnknownOperator, &text, vec![current.op, r.op], span)
            }
        };
        self.store_lvalue(&lv, result, span);
        result
    }

    // ── Operators ──

    /// Operator of an operator node: the attribute word, else the operator token named by
    /// the language frontend.
    fn operator_of(&mut self, n: u32, arity: OperatorArity, left: Option<u32>) -> OperatorId {
        let op = self.tree.operator(n);
        if op != OperatorId::None {
            return op;
        }
        let text = self.operator_text(n, left);
        if text.is_empty() {
            return OperatorId::None;
        }
        self.cur_ref()
            .sem
            .operator(&text, arity)
            .unwrap_or(OperatorId::None)
    }

    /// Spelling of the operator token of a node: the token after the left operand, or the
    /// node's first token for a prefix operator.
    fn operator_text(&self, n: u32, left: Option<u32>) -> Vec<u8> {
        let tok = match left {
            Some(l) => self.tree.token_after(l),
            None => self.tree.first_token(n),
        };
        tok.map(|t| self.tree.token_text(t).to_vec()).unwrap_or_default()
    }

    fn arith(&mut self, op: BinaryOp, lhs: Val, rhs: Val, span: Span) -> Val {
        let ty = if op == BinaryOp::Add && (lhs.ty == TypeTable::STR || rhs.ty == TypeTable::STR) {
            TypeTable::STR
        } else {
            self.module.types.arithmetic_result(lhs.ty, rhs.ty)
        };
        self.emit_def(ty, span, |dst| Inst::Binary {
            dst,
            op,
            lhs: lhs.op,
            rhs: rhs.op,
        })
    }

    fn binary_expr(&mut self, n: u32) -> Val {
        let span = self.tree.span(n);
        let kids = self.syn().active_kids(n);
        let (Some(&lhs), Some(&rhs)) = (kids.first(), kids.last()) else {
            return self.opaque(IntrinsicKind::Opaque, b"binary", Vec::new(), span);
        };
        let op = self.operator_of(n, OperatorArity::Binary, Some(lhs));
        match op {
            OperatorId::LogicalAnd | OperatorId::LogicalOr => self.condition_value(n),
            _ => {
                let l = self.expr(lhs);
                let r = self.expr(rhs);
                if let Some(pred) = ComparePred::from_operator(op) {
                    return self.emit_def(TypeTable::BOOL, span, |dst| Inst::Compare {
                        dst,
                        pred,
                        lhs: l.op,
                        rhs: r.op,
                    });
                }
                match BinaryOp::from_operator(op) {
                    Some(bop) => self.arith(bop, l, r, span),
                    None => {
                        let text = self.operator_text(n, Some(lhs));
                        self.diag_once(
                            DiagnosticKind::NotDerivable,
                            span,
                            "binary operator is recorded neither by the AST nor by the language frontend".to_string(),
                        );
                        self.opaque(IntrinsicKind::UnknownOperator, &text, vec![l.op, r.op], span)
                    }
                }
            }
        }
    }

    fn unary_expr(&mut self, n: u32) -> Val {
        let span = self.tree.span(n);
        let kids = self.syn().active_kids(n);
        let Some(&operand) = kids.first() else {
            return self.opaque(IntrinsicKind::Opaque, b"unary", Vec::new(), span);
        };
        let prefix = self.tree.first_token(n) != self.tree.first_token(operand);
        let mut postfix = self.tree.has_aux(n, NodeAttr::AUX_UNARY_POSTFIX);
        let mut op = self.tree.operator(n);
        if op == OperatorId::None {
            let text = if prefix {
                self.operator_text(n, None)
            } else {
                postfix = true;
                self.operator_text(n, Some(operand))
            };
            if !text.is_empty() {
                op = self
                    .cur_ref()
                    .sem
                    .operator(&text, OperatorArity::Unary)
                    .unwrap_or(OperatorId::None);
            }
        }
        match op {
            OperatorId::Inc | OperatorId::Dec => {
                let lv = self.lvalue(operand);
                let old = self.load_lvalue(&lv, span);
                let one = Val::new(Operand::int(1), old.ty);
                let bop = if op == OperatorId::Inc {
                    BinaryOp::Add
                } else {
                    BinaryOp::Sub
                };
                let new = self.arith(bop, old, one, span);
                let new = Val::new(new.op, old.ty);
                self.store_lvalue(&lv, new, span);
                if postfix {
                    old
                } else {
                    new
                }
            }
            OperatorId::LogicalNot => {
                let v = self.expr(operand);
                if let Some(ConstVal::Bool(b)) = v.op.as_const() {
                    return Val::new(Operand::bool(!b), TypeTable::BOOL);
                }
                self.emit_def(TypeTable::BOOL, span, |dst| Inst::Unary {
                    dst,
                    op: UnaryOp::Not,
                    src: v.op,
                })
            }
            other => {
                let v = self.expr(operand);
                match UnaryOp::from_operator(other) {
                    Some(uop) => {
                        if uop == UnaryOp::Neg {
                            match v.op.as_const() {
                                Some(ConstVal::Int(i)) => {
                                    return Val::new(Operand::int(-i), v.ty)
                                }
                                Some(ConstVal::Float(bits)) => {
                                    return Val::new(
                                        Operand::Const(ConstVal::float(-f64::from_bits(bits))),
                                        v.ty,
                                    )
                                }
                                _ => {}
                            }
                        }
                        self.emit_def(v.ty, span, |dst| Inst::Unary {
                            dst,
                            op: uop,
                            src: v.op,
                        })
                    }
                    None => {
                        let text = if prefix {
                            self.operator_text(n, None)
                        } else {
                            self.operator_text(n, Some(operand))
                        };
                        self.diag_once(
                            DiagnosticKind::NotDerivable,
                            span,
                            "unary operator is recorded neither by the AST nor by the language frontend".to_string(),
                        );
                        self.opaque(IntrinsicKind::UnknownOperator, &text, vec![v.op], span)
                    }
                }
            }
        }
    }

    // ── Conditions ──

    /// Lowers a condition into jumps to `t` and `f` (short-circuit evaluation). The caller
    /// seals `t` and `f`.
    pub(super) fn cond(&mut self, n: u32, t: BlockId, f: BlockId) {
        let syn = self.syn();
        let span = self.tree.span(n);
        match syn.class(n) {
            NodeClass::Binary => {
                let kids = syn.active_kids(n);
                if let (Some(&lhs), Some(&rhs)) = (kids.first(), kids.last()) {
                    let op = self.operator_of(n, OperatorArity::Binary, Some(lhs));
                    if op == OperatorId::LogicalAnd || op == OperatorId::LogicalOr {
                        let mid = self.b().create_block();
                        if op == OperatorId::LogicalAnd {
                            self.cond(lhs, mid, f);
                        } else {
                            self.cond(lhs, t, mid);
                        }
                        self.b().seal(mid);
                        self.b().switch_to(mid);
                        self.cond(rhs, t, f);
                        return;
                    }
                }
            }
            NodeClass::Unary => {
                let kids = syn.active_kids(n);
                if let Some(&operand) = kids.first() {
                    let op = self.operator_of(n, OperatorArity::Unary, None);
                    if op == OperatorId::LogicalNot {
                        self.cond(operand, f, t);
                        return;
                    }
                }
            }
            NodeClass::Ternary => {
                let kids = syn.active_kids(n);
                if let [c, a, b] = kids[..] {
                    let ta = self.b().create_block();
                    let tb = self.b().create_block();
                    self.cond(c, ta, tb);
                    self.b().seal(ta);
                    self.b().seal(tb);
                    self.b().switch_to(ta);
                    self.cond(a, t, f);
                    self.b().switch_to(tb);
                    self.cond(b, t, f);
                    return;
                }
            }
            NodeClass::ExprStmt => {
                if let Some(inner) = syn.active_kids(n).first().copied() {
                    self.cond(inner, t, f);
                    return;
                }
            }
            _ => {}
        }
        let v = self.expr(n);
        match v.op.as_const() {
            Some(ConstVal::Bool(true)) => self.b().jump(t, span),
            Some(ConstVal::Bool(false)) => self.b().jump(f, span),
            _ => self.b().branch(v.op, t, f, span),
        }
    }

    /// Boolean value of a short-circuit expression.
    fn condition_value(&mut self, n: u32) -> Val {
        let span = self.tree.span(n);
        let var = self.temp_var(TypeTable::BOOL);
        let t = self.b().create_block();
        let f = self.b().create_block();
        let join = self.b().create_block();
        self.cond(n, t, f);
        self.b().seal(t);
        self.b().seal(f);
        self.b().switch_to(t);
        self.b().write_var(var, Operand::bool(true));
        self.b().jump(join, span);
        self.b().switch_to(f);
        self.b().write_var(var, Operand::bool(false));
        self.b().jump(join, span);
        self.b().seal(join);
        self.b().switch_to(join);
        let op = self.b().read_var(var);
        Val::new(op, TypeTable::BOOL)
    }

    fn ternary_expr(&mut self, n: u32) -> Val {
        let span = self.tree.span(n);
        let kids = self.syn().active_kids(n);
        let [c, a, b] = kids[..] else {
            return self.opaque(IntrinsicKind::Opaque, b"conditional", Vec::new(), span);
        };
        let ta = self.b().create_block();
        let tb = self.b().create_block();
        let join = self.b().create_block();
        self.cond(c, ta, tb);
        self.b().seal(ta);
        self.b().seal(tb);
        self.b().switch_to(ta);
        let va = self.expr(a);
        let var = self.temp_var(va.ty);
        self.b().write_var(var, va.op);
        self.b().jump(join, span);
        self.b().switch_to(tb);
        let vb = self.expr(b);
        self.b().write_var(var, vb.op);
        self.b().jump(join, span);
        self.b().seal(join);
        self.b().switch_to(join);
        let op = self.b().read_var(var);
        let ty = if va.ty == vb.ty { va.ty } else { TypeTable::UNKNOWN };
        Val::new(op, ty)
    }

    // ── Casts and type tests ──

    fn cast_expr(&mut self, n: u32) -> Val {
        let span = self.tree.span(n);
        let syn = self.syn();
        let ty = match syn.first_kid_of(n, NodeClass::TypeRef) {
            Some(t) => self.type_of_ref(t),
            None => TypeTable::UNKNOWN,
        };
        let src = match syn
            .active_kids(n)
            .into_iter()
            .rev()
            .find(|&c| syn.class(c).is_expression())
        {
            Some(e) => self.expr(e),
            None => Val::undef(),
        };
        self.emit_def(ty, span, |dst| Inst::Cast {
            dst,
            src: src.op,
            ty,
        })
    }

    fn type_test_expr(&mut self, n: u32) -> Val {
        let span = self.tree.span(n);
        let syn = self.syn();
        let kids = syn.kids(n);
        let Some(&subject) = kids.first() else {
            return self.opaque(IntrinsicKind::Opaque, b"type-test", Vec::new(), span);
        };
        let src = self.expr(subject);
        let mut ty = TypeTable::UNKNOWN;
        let mut binding_node = None;
        for &c in &kids[1..] {
            match syn.class(c) {
                NodeClass::TypeRef => ty = self.type_of_ref(c),
                NodeClass::Pattern => {
                    if let Some(t) = syn.first_kid_of(c, NodeClass::TypeRef) {
                        ty = self.type_of_ref(t);
                    }
                    binding_node = syn.first_kid_of(c, NodeClass::Name);
                }
                NodeClass::Name => binding_node = Some(c),
                _ => {}
            }
        }
        let test = self.emit_def(TypeTable::BOOL, span, |dst| Inst::TypeTest {
            dst,
            src: src.op,
            ty,
        });
        if let Some(bn) = binding_node {
            // The pattern variable holds the narrowed subject where the test holds.
            let narrowed = self.emit_def(ty, span, |dst| Inst::Cast {
                dst,
                src: src.op,
                ty,
            });
            let name = self.tree.text_id(bn);
            let symbol = self.index.at_node(bn);
            let b = self.declare_local(name, symbol, ty, span);
            self.write_binding(b, narrowed, span);
        }
        test
    }

    // ── Calls ──

    fn call_expr(&mut self, n: u32) -> Val {
        let span = self.tree.span(n);
        let syn = self.syn();
        match syn.call_parts(n) {
            CallForm::ConstructorChain {
                to_super,
                qualifier,
                args,
            } => self.constructor_chain(to_super, qualifier, &args, span),
            CallForm::Method {
                receiver,
                name_token,
                args,
            } => {
                let name = self.tree.token_text_id(name_token).unwrap_or(u32::MAX);
                match receiver {
                    None => self.unqualified_call(name, &args, span),
                    Some(r) => self.qualified_call(r, name, &args, span),
                }
            }
            CallForm::Value { callee, args } => {
                let f = self.expr(callee);
                let fv = self.value_of(f, span);
                let ops: Vec<Operand> = args.iter().map(|&a| self.expr(a).op).collect();
                let ret = match self.module.types.get(f.ty).clone() {
                    IrType::Func { ret, .. } => ret,
                    _ => TypeTable::UNKNOWN,
                };
                self.emit_call(ret, Callee::Indirect(fv), ops, CallMeta::default(), span)
            }
            CallForm::Opaque => {
                let operands: Vec<Operand> = syn
                    .active_kids(n)
                    .into_iter()
                    .filter(|&c| syn.class(c).is_expression())
                    .map(|c| self.expr(c).op)
                    .collect();
                self.diag(
                    DiagnosticKind::NotDerivable,
                    span,
                    "the receiver, name and arguments of this call cannot be told apart".to_string(),
                );
                self.opaque(IntrinsicKind::Opaque, b"call", operands, span)
            }
        }
    }

    fn lower_args(&mut self, args: &[u32]) -> Vec<Val> {
        args.iter().map(|&a| self.expr(a)).collect()
    }

    /// `name(args)`: the innermost enclosing type with a method of that name, then
    /// module-level functions, statically imported members and callable locals.
    fn unqualified_call(&mut self, name: u32, args: &[u32], span: Span) -> Val {
        let mut owner = None;
        let mut cands: Vec<u32> = Vec::new();
        let mut cls = self.cur_ref().class_symbol;
        let mut guard = 0usize;
        while let Some(c) = cls {
            let ms = self.index.methods_in_hierarchy(c, name);
            if !ms.is_empty() {
                owner = Some(c);
                cands = ms;
                break;
            }
            guard += 1;
            if guard > self.index.count() as usize {
                break;
            }
            cls = self.index.enclosing_type(c);
        }
        if cands.is_empty() {
            cands = self.module_functions_named(name);
        }
        if cands.is_empty() {
            if let Some((t, ms)) = self.statically_imported(name) {
                owner = Some(t);
                cands = ms;
            }
        }
        if cands.is_empty() {
            if let Resolved::Local(b) = self.resolve_name(name) {
                let f = self.read_binding(b, span);
                let fv = self.value_of(f, span);
                let ops: Vec<Operand> = self.lower_args(args).into_iter().map(|v| v.op).collect();
                let ret = match self.module.types.get(f.ty).clone() {
                    IrType::Func { ret, .. } => ret,
                    _ => TypeTable::UNKNOWN,
                };
                return self.emit_call(ret, Callee::Indirect(fv), ops, CallMeta::default(), span);
            }
        }
        let vals = self.lower_args(args);
        self.dispatch_call(None, owner, cands, name, vals, false, span)
    }

    /// Functions declared outside any type, visible from the current file.
    fn module_functions_named(&self, name: u32) -> Vec<u32> {
        let file = self.cur_ref().file;
        let pkg = file.and_then(|f| self.index.package_of_file(f));
        self.func_of_symbol
            .keys()
            .copied()
            .filter(|&s| {
                self.index.name(s) == name
                    && self.index.kind(s) == Some(crate::core::types::symbol::SymbolKind::SK_METHOD)
                    && self.index.enclosing_type(s).is_none()
                    && {
                        let sf = self.index.file_of(s);
                        sf == file
                            || (pkg.is_some()
                                && sf.and_then(|f| self.index.package_of_file(f)) == pkg)
                    }
            })
            .collect()
    }

    /// Statically imported methods named `name`.
    fn statically_imported(&self, name: u32) -> Option<(u32, Vec<u32>)> {
        let file = self.cur_ref().file?;
        let imports = self.imports.get(&file)?;
        let text = self.module.strings.lookup_text(name);
        if let Some(owner) = imports.static_single.get(text) {
            if let Some(t) = self.index.qualified_type(owner) {
                let ms: Vec<u32> = self
                    .index
                    .methods_in_hierarchy(t, name)
                    .into_iter()
                    .filter(|&m| self.index.is_static(m))
                    .collect();
                if !ms.is_empty() {
                    return Some((t, ms));
                }
            }
        }
        for owner in &imports.static_on_demand {
            if let Some(t) = self.index.qualified_type(owner) {
                let ms: Vec<u32> = self
                    .index
                    .methods_in_hierarchy(t, name)
                    .into_iter()
                    .filter(|&m| self.index.is_static(m))
                    .collect();
                if !ms.is_empty() {
                    return Some((t, ms));
                }
            }
        }
        None
    }

    fn qualified_call(&mut self, recv: u32, name: u32, args: &[u32], span: Span) -> Val {
        match self.qualifier(recv) {
            Qual::Super => {
                let sup = self.cur_ref().class_symbol.and_then(|c| self.index.superclass(c));
                let this = self.this_binding();
                let recv_val = match this {
                    Some(b) => {
                        let v = self.read_binding(b, span);
                        let ty = match sup {
                            Some(s) => self.module.types.named(s),
                            None => v.ty,
                        };
                        Some(Val::new(v.op, ty))
                    }
                    None => None,
                };
                let cands = match sup {
                    Some(s) => self.index.methods_in_hierarchy(s, name),
                    None => Vec::new(),
                };
                let vals = self.lower_args(args);
                self.dispatch_call(recv_val, sup, cands, name, vals, true, span)
            }
            Qual::Type(t) => {
                let cands = self.index.methods_in_hierarchy(t, name);
                let vals = self.lower_args(args);
                self.dispatch_call(None, Some(t), cands, name, vals, false, span)
            }
            Qual::Package(path) => {
                let vals = self.lower_args(args);
                let qualified = path
                    .iter()
                    .map(|&p| self.name_text(p))
                    .collect::<Vec<_>>()
                    .join(".");
                self.diag(
                    DiagnosticKind::Unresolved,
                    span,
                    format!(
                        "call qualifier '{}' names no value or type of the project",
                        qualified
                    ),
                );
                let ops = vals.into_iter().map(|v| v.op).collect();
                self.emit_call(
                    TypeTable::UNKNOWN,
                    Callee::Unresolved(name),
                    ops,
                    CallMeta::default(),
                    span,
                )
            }
            Qual::Value(v) => {
                let owner = self.module.types.symbol_of(v.ty);
                let cands = match owner {
                    Some(o) => self.index.methods_in_hierarchy(o, name),
                    None => Vec::new(),
                };
                let vals = self.lower_args(args);
                self.dispatch_call(Some(v), owner, cands, name, vals, false, span)
            }
        }
    }

    /// Calls method `name` on a receiver (or statically on `owner` when `receiver` is
    /// `None`), selecting among the members of `owner`.
    pub(super) fn invoke_method(
        &mut self,
        receiver: Option<Val>,
        owner: Option<u32>,
        name: u32,
        args: Vec<Val>,
        super_call: bool,
        span: Span,
    ) -> Val {
        let cands = match owner {
            Some(o) => self.index.methods_in_hierarchy(o, name),
            None => Vec::new(),
        };
        self.dispatch_call(receiver, owner, cands, name, args, super_call, span)
    }

    #[allow(clippy::too_many_arguments)]
    fn dispatch_call(
        &mut self,
        receiver: Option<Val>,
        owner: Option<u32>,
        cands: Vec<u32>,
        name: u32,
        args: Vec<Val>,
        super_call: bool,
        span: Span,
    ) -> Val {
        match self.select_overload(&cands, &args) {
            Selection::One(m) => self.bind_call(m, receiver, owner, args, super_call, span),
            Selection::Open(remaining) => {
                let text = self.name_text(name);
                if remaining.is_empty() {
                    self.diag(
                        DiagnosticKind::Unresolved,
                        span,
                        match owner {
                            Some(_) => format!(
                                "method '{}' is declared by no project type of the receiver",
                                text
                            ),
                            None => format!("method '{}' is declared by no type in scope", text),
                        },
                    );
                } else {
                    self.diag(
                        DiagnosticKind::Ambiguous,
                        span,
                        format!(
                            "call of '{}' matches {} declarations that argument types do not separate",
                            text,
                            remaining.len()
                        ),
                    );
                }
                let mut ops = Vec::with_capacity(args.len() + 1);
                let mut meta = CallMeta {
                    receiver: None,
                    candidates: remaining,
                };
                if let Some(r) = receiver {
                    let rv = self.value_of(r, span);
                    ops.push(Operand::Value(rv));
                    meta.receiver = Some(rv);
                }
                ops.extend(args.iter().map(|a| a.op));
                self.emit_call(TypeTable::UNKNOWN, Callee::Unresolved(name), ops, meta, span)
            }
        }
    }

    /// Emits a call of the selected method `m` with the dispatch its declaration implies.
    fn bind_call(
        &mut self,
        m: u32,
        receiver: Option<Val>,
        owner: Option<u32>,
        args: Vec<Val>,
        super_call: bool,
        span: Span,
    ) -> Val {
        let ret = self.return_type(m);
        if self.index.is_static(m) {
            let ops = args.iter().map(|a| a.op).collect();
            let callee = self.direct_callee(m);
            return self.emit_call(ret, callee, ops, CallMeta::default(), span);
        }
        let recv = match receiver {
            Some(r) => Some(self.value_of(r, span)),
            None => {
                let declaring = self.index.enclosing_type(m).or(owner);
                match declaring.and_then(|d| self.instance_of(d)) {
                    Some(path) => self.read_receiver(&path, span),
                    None => None,
                }
            }
        };
        let Some(recv) = recv else {
            self.diag(
                DiagnosticKind::NotDerivable,
                span,
                format!(
                    "instance method '{}' is called where no receiver of its type is reachable",
                    self.name_text(self.index.name(m))
                ),
            );
            let ops = args.iter().map(|a| a.op).collect();
            return self.emit_call(
                ret,
                Callee::Unresolved(self.index.name(m)),
                ops,
                CallMeta {
                    receiver: None,
                    candidates: vec![m],
                },
                span,
            );
        };
        let mut ops = Vec::with_capacity(args.len() + 1);
        ops.push(Operand::Value(recv));
        ops.extend(args.iter().map(|a| a.op));
        let rule = if super_call || self.index.kind(m) == Some(crate::core::types::symbol::SymbolKind::SK_CONSTRUCTOR) {
            DispatchRule::Direct
        } else {
            match self.index.symbol(m) {
                Some(rec) => self.cur_ref().sem.instance_dispatch(rec).unwrap_or(DispatchRule::Virtual),
                None => DispatchRule::Virtual,
            }
        };
        let callee = match rule {
            DispatchRule::Direct => self.direct_callee(m),
            DispatchRule::Virtual => {
                let through_interface = owner.is_some_and(|o| self.index.is_interface(o))
                    || self
                        .index
                        .enclosing_type(m)
                        .is_some_and(|d| self.index.is_interface(d));
                if through_interface {
                    Callee::Interface {
                        receiver: recv,
                        method: m,
                    }
                } else {
                    Callee::Virtual {
                        receiver: recv,
                        method: m,
                    }
                }
            }
        };
        self.emit_call(
            ret,
            callee,
            ops,
            CallMeta {
                receiver: Some(recv),
                candidates: Vec::new(),
            },
            span,
        )
    }

    /// Direct target of a declared function.
    pub(super) fn direct_callee(&self, sym: u32) -> Callee {
        match self.func_of_symbol.get(&sym) {
            Some(&f) => Callee::Static(f),
            None => Callee::Symbol(sym),
        }
    }

    /// Overload resolution among `cands` for the argument list: arity (variable arity
    /// included), then argument types against declared parameter types (subtyping through
    /// the type hierarchy), then the most specific remaining declaration.
    pub(super) fn select_overload(&self, cands: &[u32], args: &[Val]) -> Selection {
        if cands.is_empty() {
            return Selection::Open(Vec::new());
        }
        let n = args.len();
        let by_arity: Vec<u32> = cands
            .iter()
            .copied()
            .filter(|&m| {
                let a = self.index.arity(m);
                a == n || (self.index.is_varargs(m) && n + 1 >= a)
            })
            .collect();
        match by_arity.len() {
            0 => return Selection::Open(cands.to_vec()),
            1 => return Selection::One(by_arity[0]),
            _ => {}
        }
        let arg_syms: Vec<Option<u32>> = args
            .iter()
            .map(|a| self.module.types.symbol_of(a.ty).filter(|&s| self.index.is_type(s)))
            .collect();
        let compatible: Vec<u32> = by_arity
            .iter()
            .copied()
            .filter(|&m| {
                let sig = self.index.signature(m);
                sig.iter().zip(arg_syms.iter()).all(|(&p, a)| match a {
                    Some(a) if self.index.is_type(p) => self.index.is_subtype(*a, p),
                    _ => true,
                })
            })
            .collect();
        match compatible.len() {
            0 => return Selection::Open(by_arity),
            1 => return Selection::One(compatible[0]),
            _ => {}
        }
        let more_specific = |a: u32, b: u32| -> bool {
            let (sa, sb) = (self.index.signature(a), self.index.signature(b));
            sa.len() == sb.len()
                && sa.iter().zip(sb.iter()).all(|(&x, &y)| {
                    x == y || (self.index.is_type(x) && self.index.is_type(y) && self.index.is_subtype(x, y))
                })
        };
        let best: Vec<u32> = compatible
            .iter()
            .copied()
            .filter(|&m| compatible.iter().all(|&o| o == m || more_specific(m, o)))
            .collect();
        if best.len() == 1 {
            Selection::One(best[0])
        } else {
            Selection::Open(compatible)
        }
    }

    /// `this(args)` / `super(args)` inside a constructor.
    pub(super) fn constructor_chain(
        &mut self,
        to_super: bool,
        qualifier: Option<u32>,
        args: &[u32],
        span: Span,
    ) -> Val {
        let cls = self.cur_ref().class_symbol;
        let target = if to_super {
            cls.and_then(|c| self.index.superclass(c))
        } else {
            cls
        };
        let outer = qualifier.map(|q| self.expr(q));
        let vals = self.lower_args(args);
        let this = match self.this_binding() {
            Some(b) => {
                let v = self.read_binding(b, span);
                Some(self.value_of(v, span))
            }
            None => None,
        };
        let Some(this) = this else {
            self.diag(
                DiagnosticKind::NotDerivable,
                span,
                "constructor chaining outside a constructor body".to_string(),
            );
            return Val::undef();
        };
        if let (Some(t), Some(o)) = (target, outer) {
            if let Some(link) = self.outer_field_of(t) {
                self.emit(
                    Inst::Store {
                        place: Place::Field {
                            base: this,
                            field: link,
                        },
                        value: o.op,
                    },
                    span,
                );
            }
        }
        let mut ops = vec![Operand::Value(this)];
        ops.extend(vals.iter().map(|v| v.op));
        let meta = CallMeta {
            receiver: Some(this),
            candidates: Vec::new(),
        };
        match target {
            Some(t) => {
                let ctors = self.index.constructors(t);
                match self.select_overload(&ctors, &vals) {
                    Selection::One(c) => {
                        let callee = self.direct_callee(c);
                        self.emit_call(TypeTable::VOID, callee, ops, meta, span)
                    }
                    Selection::Open(rest) => {
                        if ctors.is_empty() && vals.is_empty() {
                            if let Some(f) = self.implicit_ctor_for_symbol(t) {
                                return self.emit_call(TypeTable::VOID, Callee::Static(f), ops, meta, span);
                            }
                        }
                        self.diag(
                            if rest.len() > 1 {
                                DiagnosticKind::Ambiguous
                            } else {
                                DiagnosticKind::Unresolved
                            },
                            span,
                            "chained constructor is not selectable among the project declarations".to_string(),
                        );
                        let meta = CallMeta {
                            receiver: Some(this),
                            candidates: rest,
                        };
                        self.emit_call(TypeTable::VOID, Callee::Unresolved(self.index.name(t)), ops, meta, span)
                    }
                }
            }
            None => {
                self.diag(
                    DiagnosticKind::Unresolved,
                    span,
                    "the chained constructor's type is outside the project".to_string(),
                );
                let name = self.intern(if to_super { b"<super>" } else { b"<this>" });
                self.emit_call(TypeTable::VOID, Callee::Unresolved(name), ops, meta, span)
            }
        }
    }

    // ── Object and array creation ──

    fn new_expr(&mut self, n: u32) -> Val {
        let span = self.tree.span(n);
        let syn = self.syn();
        let parts = syn.new_parts(n);
        let outer = parts.qualifier.map(|q| self.expr(q));
        let ty = match parts.type_ref {
            Some(t) => self.type_of_ref(t),
            None => TypeTable::UNKNOWN,
        };
        let anon_plan = parts
            .anon_body
            .map(|body| self.anonymous_class(body, n, &parts.anon_members, ty));
        let vals = self.lower_args(&parts.args);
        self.construct(ty, anon_plan, vals, outer.map(|o| o.op), span)
    }

    /// Allocates an object of type `ty` (or of the anonymous class `anon`) and runs its
    /// constructor.
    pub(super) fn construct(
        &mut self,
        ty: TypeId,
        anon: Option<u32>,
        args: Vec<Val>,
        outer: Option<Operand>,
        span: Span,
    ) -> Val {
        let declared = self.module.types.symbol_of(ty);
        let plan_node = match anon {
            Some(a) => Some(a),
            None => declared.and_then(|s| self.class_of_symbol.get(&s).copied()),
        };
        let alloc_ty = match anon.and_then(|a| self.classes.get(&a)).and_then(|p| p.symbol) {
            Some(s) => self.module.types.named(s),
            None => ty,
        };
        let obj = self.b().emit_alloc(alloc_ty, Region::Heap, span);

        if let Some(node) = plan_node {
            self.store_class_captures(node, obj, span);
        }
        let class_sym = match anon {
            Some(a) => self.classes.get(&a).and_then(|p| p.symbol),
            None => declared,
        };
        if let Some(cs) = class_sym {
            if let Some(link) = self.outer_field_of(cs) {
                let outer_val = match outer {
                    Some(o) => Some(o),
                    None => match self.index.enclosing_type(cs).and_then(|e| self.instance_of(e)) {
                        Some(r) => self.read_receiver(&r, span).map(Operand::Value),
                        None => None,
                    },
                };
                match outer_val {
                    Some(o) => self.emit(
                        Inst::Store {
                            place: Place::Field { base: obj, field: link },
                            value: o,
                        },
                        span,
                    ),
                    None => self.diag(
                        DiagnosticKind::NotDerivable,
                        span,
                        "no enclosing instance is reachable for the created inner object".to_string(),
                    ),
                }
            }
        }

        let mut ops = vec![Operand::Value(obj)];
        ops.extend(args.iter().map(|a| a.op));
        let meta = CallMeta {
            receiver: Some(obj),
            candidates: Vec::new(),
        };
        let result = Val::new(Operand::Value(obj), alloc_ty);

        if let Some(a) = anon {
            if let Some(f) = self.classes.get(&a).and_then(|p| p.implicit_ctor) {
                self.emit_call(TypeTable::VOID, Callee::Static(f), ops, meta, span);
                return result;
            }
        }
        let Some(cs) = declared else {
            let name = self.type_name_for_diag(ty);
            self.diag(
                DiagnosticKind::Unresolved,
                span,
                format!("created type '{}' is outside the project; its constructor is unknown", name),
            );
            let id = self.intern(name.as_bytes());
            self.emit_call(TypeTable::VOID, Callee::Unresolved(id), ops, meta, span);
            return result;
        };
        let ctors = self.index.constructors(cs);
        match self.select_overload(&ctors, &args) {
            Selection::One(c) => {
                let callee = self.direct_callee(c);
                self.emit_call(TypeTable::VOID, callee, ops, meta, span);
            }
            Selection::Open(rest) => {
                if ctors.is_empty() && args.is_empty() {
                    if let Some(f) = self.implicit_ctor_for_symbol(cs) {
                        self.emit_call(TypeTable::VOID, Callee::Static(f), ops, meta, span);
                        return result;
                    }
                }
                self.diag(
                    if rest.len() > 1 {
                        DiagnosticKind::Ambiguous
                    } else {
                        DiagnosticKind::Unresolved
                    },
                    span,
                    "constructor is not selectable among the project declarations".to_string(),
                );
                let meta = CallMeta {
                    receiver: Some(obj),
                    candidates: rest,
                };
                self.emit_call(TypeTable::VOID, Callee::Unresolved(self.index.name(cs)), ops, meta, span);
            }
        }
        result
    }

    /// Stores the captured values of a local or anonymous class into a new instance.
    fn store_class_captures(&mut self, node: u32, obj: ValueId, span: Span) {
        let Some(local) = self.classes.get(&node).and_then(|p| p.local.clone()) else {
            return;
        };
        if local.decl_frame >= self.frames.len() || self.frames[local.decl_frame].func != local.decl_func {
            self.diag(
                DiagnosticKind::NotDerivable,
                span,
                "local class instantiated outside the function that declares it".to_string(),
            );
            return;
        }
        let top = self.top();
        for cap in local.captures {
            let b = self.import_into(top, cap.outer, local.decl_frame);
            let op = self.capture_operand(b, span);
            self.emit(
                Inst::Store {
                    place: Place::Field {
                        base: obj,
                        field: cap.field,
                    },
                    value: op,
                },
                span,
            );
        }
    }

    /// Implicit constructor of a project type that declares none.
    pub(super) fn implicit_ctor_for_symbol(&mut self, class_symbol: u32) -> Option<FuncId> {
        let node = *self.class_of_symbol.get(&class_symbol)?;
        let plan = self.classes.get(&node)?;
        if plan.explicit_ctors > 0 {
            return None;
        }
        if let Some(f) = plan.implicit_ctor {
            return Some(f);
        }
        if plan.local.is_some() {
            // Local classes get their implicit constructor when they are lowered.
            return None;
        }
        let f = self.reserve_job(JobKind::ImplicitCtor(node));
        if let Some(p) = self.classes.get_mut(&node) {
            p.implicit_ctor = Some(f);
        }
        Some(f)
    }

    fn type_name_for_diag(&self, ty: TypeId) -> String {
        match self.module.types.symbol_of(ty) {
            Some(s) => self.name_text(self.index.name(s)),
            None => "<unknown type>".to_string(),
        }
    }

    fn array_new_expr(&mut self, n: u32) -> Val {
        let span = self.tree.span(n);
        let syn = self.syn();
        let elem = match syn.first_kid_of(n, NodeClass::TypeRef) {
            Some(t) => self.type_of_ref(t),
            None => TypeTable::UNKNOWN,
        };
        let mut dims: Vec<Val> = Vec::new();
        let mut literal = None;
        for c in syn.active_kids(n) {
            match syn.class(c) {
                NodeClass::ArrayLit => literal = Some(c),
                cl if cl.is_expression() => dims.push(self.expr(c)),
                _ => {}
            }
        }
        if let Some(lit) = literal {
            let depth = self.literal_depth(lit).max(dims.len() as u32).max(1);
            let mut ty = elem;
            for _ in 0..depth {
                ty = self.module.types.array_of(ty, None);
            }
            return self.array_literal(lit, ty);
        }
        if dims.is_empty() {
            self.diag(
                DiagnosticKind::NotDerivable,
                span,
                "array creation without dimensions or initializer".to_string(),
            );
            let ty = self.module.types.array_of(elem, None);
            let v = self.b().emit_alloc(ty, Region::Heap, span);
            return Val::new(Operand::Value(v), ty);
        }
        self.alloc_array(elem, &dims, span)
    }

    fn literal_depth(&self, n: u32) -> u32 {
        let syn = self.syn();
        let mut depth = 1;
        let mut cur = n;
        while let Some(&first) = syn.kids(cur).first() {
            if syn.class(first) != NodeClass::ArrayLit {
                break;
            }
            depth += 1;
            cur = first;
        }
        depth
    }

    /// Allocates a (possibly multi-dimensional) array: one allocation per dimension level,
    /// the inner levels in a loop over the outer length.
    fn alloc_array(&mut self, elem: TypeId, dims: &[Val], span: Span) -> Val {
        let mut inner_ty = elem;
        for _ in 1..dims.len() {
            inner_ty = self.module.types.array_of(inner_ty, None);
        }
        let len0 = dims[0];
        let static_len = len0.op.as_const().and_then(|c| c.as_int()).and_then(|i| u64::try_from(i).ok());
        let ty = self.module.types.array_of(inner_ty, static_len);
        let arr = self.b().emit_alloc(ty, Region::Heap, span);
        if static_len.is_none() {
            self.intrinsic(
                IntrinsicKind::AllocLength,
                vec![Operand::Value(arr), len0.op],
                None,
                span,
            );
        }
        if dims.len() > 1 {
            let int = self.module.types.int(32, true);
            let i = self.temp_var(int);
            self.b().write_var(i, Operand::int(0));
            let header = self.b().create_block();
            let body = self.b().create_block();
            let exit = self.b().create_block();
            self.b().jump(header, span);
            self.b().switch_to(header);
            let iv = self.b().read_var(i);
            let c = self.emit_def(TypeTable::BOOL, span, |dst| Inst::Compare {
                dst,
                pred: ComparePred::Lt,
                lhs: iv,
                rhs: len0.op,
            });
            self.b().branch(c.op, body, exit, span);
            self.b().seal(body);
            self.b().switch_to(body);
            let inner = self.alloc_array(elem, &dims[1..], span);
            let iv = self.b().read_var(i);
            self.emit(
                Inst::Store {
                    place: Place::Index { base: arr, index: iv },
                    value: inner.op,
                },
                span,
            );
            let next = self.emit_def(int, span, |dst| Inst::Binary {
                dst,
                op: BinaryOp::Add,
                lhs: iv,
                rhs: Operand::int(1),
            });
            self.b().write_var(i, next.op);
            self.b().jump(header, span);
            self.b().seal(header);
            self.b().seal(exit);
            self.b().switch_to(exit);
        }
        Val::new(Operand::Value(arr), ty)
    }

    /// Array literal of type `ty`: allocation of the element count, then one store per
    /// element in order.
    pub(super) fn array_literal(&mut self, n: u32, ty: TypeId) -> Val {
        let span = self.tree.span(n);
        let syn = self.syn();
        let elems: Vec<u32> = syn
            .active_kids(n)
            .into_iter()
            .filter(|&c| syn.class(c).is_expression())
            .collect();
        let elem_ty = self.module.types.element(ty).unwrap_or(TypeTable::UNKNOWN);
        let alloc_ty = self.module.types.array_of(elem_ty, Some(elems.len() as u64));
        let arr = self.b().emit_alloc(alloc_ty, Region::Heap, span);
        for (i, &e) in elems.iter().enumerate() {
            let v = if syn.class(e) == NodeClass::ArrayLit {
                self.array_literal(e, elem_ty)
            } else {
                self.expr_hint(e, Some(elem_ty))
            };
            self.emit(
                Inst::Store {
                    place: Place::Index {
                        base: arr,
                        index: Operand::int(i as i128),
                    },
                    value: v.op,
                },
                span,
            );
        }
        Val::new(Operand::Value(arr), if ty == TypeTable::UNKNOWN { alloc_ty } else { ty })
    }
}
