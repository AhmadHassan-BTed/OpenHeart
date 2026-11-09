//! Statement lowering: blocks and scopes, declarations, conditionals, pre-test, post-test,
//! counted and iterator loops, switch with fall-through, labeled break and continue, goto,
//! return, throw, try/catch/finally, and the cleanup regions (resources, locks, deferred
//! statements, scope-exit destruction) whose work runs on every exit path.

use crate::ir::inst::{ComparePred, Inst, IntrinsicKind, Region, Terminator};
use crate::ir::lower::frame::{Binding, BreakKind, Cleanup, Exit, Resolved, Val};
use crate::ir::lower::semantics::IterationStep;
use crate::ir::lower::syntax::{CaseParts, NodeClass};
use crate::ir::lower::Lowerer;
use crate::ir::module::DiagnosticKind;
use crate::ir::types::{BlockId, IrType, Span, TypeId, TypeTable};
use crate::ir::value::{ConstVal, Operand, Place};

/// How a case label of a switch is tested.
enum LabelTest {
    Const(ConstVal),
    Value(u32),
    Pattern { ty: TypeId, binding: Option<u32>, node: u32 },
}

impl<'t> Lowerer<'t> {
    pub(super) fn stmt(&mut self, n: u32) {
        let syn = self.syn();
        let span = self.tree.span(n);
        match syn.class(n) {
            NodeClass::Block => self.block(n),
            NodeClass::LocalDecl | NodeClass::Field => self.local_decl(n),
            NodeClass::Declarator => {
                self.declare_variable(n, None);
            }
            NodeClass::ExprStmt => {
                for c in syn.active_kids(n) {
                    if syn.class(c).is_expression() {
                        self.expr(c);
                    } else {
                        self.stmt(c);
                    }
                }
            }
            NodeClass::If => self.if_stmt(n),
            NodeClass::While => self.while_stmt(n),
            NodeClass::DoWhile => self.do_while_stmt(n),
            NodeClass::For => self.for_stmt(n),
            NodeClass::ForEach => self.foreach_stmt(n),
            NodeClass::Switch | NodeClass::SwitchExpr => {
                self.switch_construct(n, false);
            }
            NodeClass::Try => self.try_stmt(n),
            NodeClass::Return => self.return_stmt(n),
            NodeClass::Throw => self.throw_stmt(n),
            NodeClass::Break => self.break_stmt(n, false),
            NodeClass::Continue => self.break_stmt(n, true),
            NodeClass::Yield => self.yield_stmt(n),
            NodeClass::Labeled => self.labeled_stmt(n),
            NodeClass::Goto => self.goto_stmt(n),
            NodeClass::Lock => self.lock_stmt(n),
            NodeClass::Using => self.using_stmt(n),
            NodeClass::Defer => {
                let body = syn.active_kids(n).last().copied().unwrap_or(n);
                self.push_cleanup(Cleanup::Defer(body));
            }
            NodeClass::Assert => self.assert_stmt(n),
            NodeClass::TypeDecl => self.local_class(n),
            NodeClass::Function => self.local_function(n),
            NodeClass::Initializer { .. } | NodeClass::Finally | NodeClass::Else => {
                for c in syn.active_kids(n) {
                    self.stmt(c);
                }
            }
            cl if cl.is_expression() => {
                self.expr(n);
            }
            cl if cl.is_inert() => {}
            other => {
                self.diag(
                    DiagnosticKind::NotDerivable,
                    span,
                    format!("no lowering rule for statement node class {:?}", other),
                );
                let kids = syn.active_kids(n);
                let mut operands = Vec::new();
                for c in kids {
                    let cl = syn.class(c);
                    if cl.is_expression() {
                        operands.push(self.expr(c).op);
                    } else if cl.is_statement() {
                        self.stmt(c);
                    }
                }
                self.opaque(
                    IntrinsicKind::Opaque,
                    format!("{:?}", other).as_bytes(),
                    operands,
                    span,
                );
            }
        }
    }

    /// Block: a lexical scope whose scope-bound cleanups (deferred statements, scope-exit
    /// destruction) run when it ends.
    fn block(&mut self, n: u32) {
        let span = self.tree.span(n);
        let depth = self.cur_ref().exits.len();
        self.push_scope();
        let items: Vec<u32> = self.syn().kids(n).to_vec();
        for s in items {
            self.stmt(s);
        }
        while self.cur_ref().exits.len() > depth {
            match self.cur_ref().exits.last() {
                Some(Exit::Cleanup { .. }) => self.pop_cleanup(span),
                _ => {
                    self.pop_exit();
                }
            }
        }
        self.pop_scope();
    }

    fn local_decl(&mut self, n: u32) {
        let syn = self.syn();
        let parts = syn.local_decl_parts(n);
        for d in parts.declarators {
            self.declare_variable(d, parts.type_ref);
        }
    }

    /// Declares one variable: evaluates its initializer, then brings the name into scope.
    pub(super) fn declare_variable(&mut self, d: u32, type_ref: Option<u32>) -> Binding {
        let syn = self.syn();
        let span = self.tree.span(d);
        let dp = syn.declarator_parts(d);
        let tr = dp.type_ref.or(type_ref);
        let name = dp.name_token.and_then(|t| self.tree.token_text_id(t));
        let symbol = self.index.at_node(d);
        let declared = match tr {
            Some(r) if !self.is_inferred_type_ref(r) => Some(self.type_of_ref(r)),
            _ => None,
        };
        let init = dp.init.map(|i| self.expr_hint(i, declared));
        let ty = declared
            .or(init.map(|v| v.ty))
            .unwrap_or(TypeTable::UNKNOWN);

        let destroyed = self.cur_ref().sem.scope_exit_destruction()
            && matches!(self.module.types.get(ty), IrType::Named(_) | IrType::Generic { .. });
        if destroyed {
            // The variable is a stack object: its storage is allocated here, initialised
            // from the initializer, and destroyed when the scope ends.
            let slot = self.b().emit_alloc(ty, Region::Stack, span);
            if let Some(v) = init {
                self.emit(
                    Inst::Store {
                        place: Place::Local(slot),
                        value: v.op,
                    },
                    span,
                );
            }
            let b = self.declare_local(name, symbol, ty, span);
            self.write_binding(b, Val::new(Operand::Value(slot), ty), span);
            self.push_cleanup(Cleanup::Destroy { slot, span });
            return b;
        }

        let b = self.declare_local(name, symbol, ty, span);
        if let Some(v) = init {
            self.write_binding(b, Val::new(v.op, ty), span);
        }
        b
    }

    fn take_label(&mut self) -> Option<u32> {
        self.cur().pending_label.take()
    }

    fn if_stmt(&mut self, n: u32) {
        let span = self.tree.span(n);
        let parts = self.syn().if_parts(n);
        let then_b = self.b().create_block();
        let join = self.b().create_block();
        let else_b = if parts.else_s.is_some() {
            self.b().create_block()
        } else {
            join
        };
        match parts.cond {
            Some(c) => self.cond(c, then_b, else_b),
            None => {
                let c = self.opaque(IntrinsicKind::Opaque, b"if-condition", Vec::new(), span);
                self.b().branch(c.op, then_b, else_b, span);
            }
        }
        self.b().seal(then_b);
        self.b().switch_to(then_b);
        if let Some(t) = parts.then_s {
            self.scoped_stmt(t);
        }
        if self.cur_ref().b.is_open() {
            self.b().jump(join, span);
        }
        if let Some(e) = parts.else_s {
            self.b().seal(else_b);
            self.b().switch_to(else_b);
            self.scoped_stmt(e);
            if self.cur_ref().b.is_open() {
                self.b().jump(join, span);
            }
        }
        self.b().seal(join);
        self.b().switch_to(join);
    }

    /// A statement in its own scope (branch and loop bodies).
    fn scoped_stmt(&mut self, n: u32) {
        let span = self.tree.span(n);
        let depth = self.cur_ref().exits.len();
        self.push_scope();
        self.stmt(n);
        while self.cur_ref().exits.len() > depth {
            match self.cur_ref().exits.last() {
                Some(Exit::Cleanup { .. }) => self.pop_cleanup(span),
                _ => {
                    self.pop_exit();
                }
            }
        }
        self.pop_scope();
    }

    fn loop_frame(&mut self, break_to: BlockId, continue_to: BlockId) {
        let label = self.take_label();
        self.push_exit(Exit::Breakable {
            label,
            kind: BreakKind::Loop,
            break_to,
            continue_to: Some(continue_to),
            result: None,
        });
    }

    /// Pre-test loop.
    fn while_stmt(&mut self, n: u32) {
        let span = self.tree.span(n);
        let kids = self.syn().active_kids(n);
        let cond = kids.first().copied();
        let body = kids.get(1).copied();
        let header = self.b().create_block();
        let body_b = self.b().create_block();
        let exit = self.b().create_block();
        self.b().jump(header, span);
        self.b().switch_to(header);
        match cond {
            Some(c) => self.cond(c, body_b, exit),
            None => self.b().jump(body_b, span),
        }
        self.b().seal(body_b);
        self.b().switch_to(body_b);
        self.loop_frame(exit, header);
        if let Some(s) = body {
            self.scoped_stmt(s);
        }
        if self.cur_ref().b.is_open() {
            self.b().jump(header, span);
        }
        self.pop_exit();
        self.b().seal(header);
        self.b().seal(exit);
        self.b().switch_to(exit);
    }

    /// Post-test loop.
    fn do_while_stmt(&mut self, n: u32) {
        let span = self.tree.span(n);
        let syn = self.syn();
        let kids = syn.active_kids(n);
        let (body, cond) = match kids.len() {
            0 => (None, None),
            1 if syn.class(kids[0]).is_statement() => (Some(kids[0]), None),
            1 => (None, Some(kids[0])),
            _ => (Some(kids[0]), kids.last().copied()),
        };
        let body_b = self.b().create_block();
        let cond_b = self.b().create_block();
        let exit = self.b().create_block();
        self.b().jump(body_b, span);
        self.b().switch_to(body_b);
        self.loop_frame(exit, cond_b);
        if let Some(s) = body {
            self.scoped_stmt(s);
        }
        if self.cur_ref().b.is_open() {
            self.b().jump(cond_b, span);
        }
        self.pop_exit();
        self.b().seal(cond_b);
        self.b().switch_to(cond_b);
        match cond {
            Some(c) => self.cond(c, body_b, exit),
            None => self.b().jump(body_b, span),
        }
        self.b().seal(body_b);
        self.b().seal(exit);
        self.b().switch_to(exit);
    }

    /// Counted loop: init; header (condition); body; update; back to the header.
    fn for_stmt(&mut self, n: u32) {
        let span = self.tree.span(n);
        let parts = self.syn().for_parts(n);
        self.push_scope();
        for &i in &parts.init {
            self.stmt(i);
        }
        let header = self.b().create_block();
        let body_b = self.b().create_block();
        let update_b = self.b().create_block();
        let exit = self.b().create_block();
        self.b().jump(header, span);
        self.b().switch_to(header);
        if !parts.exact {
            self.diag(
                DiagnosticKind::NotDerivable,
                span,
                "the clauses of this loop header cannot be told apart; every header expression \
                 is evaluated before each iteration and the loop condition is opaque"
                    .to_string(),
            );
            let mut operands = Vec::new();
            for &e in &parts.unassigned {
                operands.push(self.expr(e).op);
            }
            let c = self.opaque(IntrinsicKind::Opaque, b"loop-header", operands, span);
            self.b().branch(c.op, body_b, exit, span);
        } else {
            match parts.cond {
                Some(c) => self.cond(c, body_b, exit),
                None => self.b().jump(body_b, span),
            }
        }
        self.b().seal(body_b);
        self.b().switch_to(body_b);
        self.loop_frame(exit, update_b);
        if let Some(s) = parts.body {
            self.scoped_stmt(s);
        }
        if self.cur_ref().b.is_open() {
            self.b().jump(update_b, span);
        }
        self.pop_exit();
        self.b().seal(update_b);
        self.b().switch_to(update_b);
        for &u in &parts.update {
            self.stmt(u);
        }
        if self.cur_ref().b.is_open() {
            self.b().jump(header, span);
        }
        self.b().seal(header);
        self.b().seal(exit);
        self.b().switch_to(exit);
        self.pop_scope();
    }

    /// Iterator loop: element-wise over arrays, otherwise the iteration protocol of the
    /// language (begin, has-next, next) as explicit calls.
    fn foreach_stmt(&mut self, n: u32) {
        let span = self.tree.span(n);
        let parts = self.syn().foreach_parts(n);
        let Some(iterable) = parts.iterable else {
            self.diag(
                DiagnosticKind::NotDerivable,
                span,
                "iterator loop without an iterable".to_string(),
            );
            return;
        };
        let source = self.expr(iterable);
        let declared = match parts.type_ref {
            Some(r) if !self.is_inferred_type_ref(r) => Some(self.type_of_ref(r)),
            _ => None,
        };
        let var_name = parts.var_token.and_then(|t| self.tree.token_text_id(t));
        let var_symbol = parts
            .var_token
            .and_then(|_| self.syn().kids(n).iter().copied().find_map(|c| self.index.at_node(c)));
        self.push_scope();

        let header = self.b().create_block();
        let body_b = self.b().create_block();
        let exit = self.b().create_block();

        if let IrType::Array { elem, .. } = self.module.types.get(source.ty).clone() {
            let int = self.module.types.int(32, true);
            let arr = self.value_of(source, span);
            let len = self
                .intrinsic(IntrinsicKind::ArrayLength, vec![Operand::Value(arr)], Some(int), span)
                .expect("intrinsic with a result type defines a value");
            let i = self.temp_var(int);
            self.b().write_var(i, Operand::int(0));
            let update_b = self.b().create_block();
            self.b().jump(header, span);
            self.b().switch_to(header);
            let iv = self.b().read_var(i);
            let c = self.emit_def(TypeTable::BOOL, span, |dst| Inst::Compare {
                dst,
                pred: ComparePred::Lt,
                lhs: iv,
                rhs: Operand::Value(len),
            });
            self.b().branch(c.op, body_b, exit, span);
            self.b().seal(body_b);
            self.b().switch_to(body_b);
            let iv = self.b().read_var(i);
            let ty = declared.unwrap_or(elem);
            let x = self.emit_def(elem, span, |dst| Inst::Load {
                dst,
                place: Place::Index { base: arr, index: iv },
            });
            let b = self.declare_local(var_name, var_symbol, ty, span);
            self.write_binding(b, x, span);
            self.loop_frame(exit, update_b);
            if let Some(s) = parts.body {
                self.scoped_stmt(s);
            }
            if self.cur_ref().b.is_open() {
                self.b().jump(update_b, span);
            }
            self.pop_exit();
            self.b().seal(update_b);
            self.b().switch_to(update_b);
            let iv = self.b().read_var(i);
            let next = self.emit_def(int, span, |dst| Inst::Binary {
                dst,
                op: crate::ir::inst::BinaryOp::Add,
                lhs: iv,
                rhs: Operand::int(1),
            });
            self.b().write_var(i, next.op);
            self.b().jump(header, span);
        } else {
            let iterator = self.iteration_step(IterationStep::Begin, source, span);
            let it_var = self.temp_var(iterator.ty);
            self.b().write_var(it_var, iterator.op);
            self.b().jump(header, span);
            self.b().switch_to(header);
            let it = Val::new(self.b().read_var(it_var), iterator.ty);
            let more = self.iteration_step(IterationStep::HasNext, it, span);
            self.b().branch(more.op, body_b, exit, span);
            self.b().seal(body_b);
            self.b().switch_to(body_b);
            let it = Val::new(self.b().read_var(it_var), iterator.ty);
            let element = self.iteration_step(IterationStep::Next, it, span);
            let element = match declared {
                Some(t) if t != element.ty && t != TypeTable::UNKNOWN => {
                    self.emit_def(t, span, |dst| Inst::Cast {
                        dst,
                        src: element.op,
                        ty: t,
                    })
                }
                _ => element,
            };
            let ty = declared.unwrap_or(element.ty);
            let b = self.declare_local(var_name, var_symbol, ty, span);
            self.write_binding(b, element, span);
            self.loop_frame(exit, header);
            if let Some(s) = parts.body {
                self.scoped_stmt(s);
            }
            if self.cur_ref().b.is_open() {
                self.b().jump(header, span);
            }
            self.pop_exit();
        }
        self.b().seal(header);
        self.b().seal(exit);
        self.b().switch_to(exit);
        self.pop_scope();
    }

    /// One step of the iteration protocol on `receiver`.
    fn iteration_step(&mut self, step: IterationStep, receiver: Val, span: Span) -> Val {
        match self.cur_ref().sem.iteration_method(step) {
            Some(method) => {
                let name = self.intern(method);
                let owner = self.module.types.symbol_of(receiver.ty);
                let v = self.invoke_method(Some(receiver), owner, name, Vec::new(), false, span);
                if step == IterationStep::HasNext && v.ty == TypeTable::UNKNOWN {
                    Val::new(v.op, TypeTable::BOOL)
                } else {
                    v
                }
            }
            None => {
                self.diag_once(
                    DiagnosticKind::NotDerivable,
                    span,
                    "the language frontend names no iteration protocol; iterator loops use \
                     the neutral iteration intrinsics"
                        .to_string(),
                );
                let (kind, ty) = match step {
                    IterationStep::Begin => (IntrinsicKind::IterBegin, TypeTable::UNKNOWN),
                    IterationStep::HasNext => (IntrinsicKind::IterHasNext, TypeTable::BOOL),
                    IterationStep::Next => (IntrinsicKind::IterNext, TypeTable::UNKNOWN),
                };
                let v = self
                    .intrinsic(kind, vec![receiver.op], Some(ty), span)
                    .expect("intrinsic with a result type defines a value");
                Val::new(Operand::Value(v), ty)
            }
        }
    }

    /// Switch statement or expression. Constant labels become a `Switch` terminator;
    /// other labels a chain of equality and type tests in label order. Case groups fall
    /// through to the next group unless they are arrow cases.
    pub(super) fn switch_construct(&mut self, n: u32, as_expr: bool) -> Option<Val> {
        let span = self.tree.span(n);
        let syn = self.syn();
        let (scrutinee, cases) = syn.switch_parts(n);
        let subject = match scrutinee {
            Some(s) => self.expr(s),
            None => Val::undef(),
        };
        let label = self.take_label();
        let exit = self.b().create_block();
        let result = if as_expr {
            Some(self.temp_var(TypeTable::UNKNOWN))
        } else {
            None
        };
        let parts: Vec<CaseParts> = cases.iter().map(|&c| syn.case_parts(c)).collect();
        let bodies: Vec<BlockId> = parts.iter().map(|_| self.b().create_block()).collect();
        let default_target = parts
            .iter()
            .position(|p| p.is_default)
            .map(|i| bodies[i])
            .unwrap_or(exit);

        // Label tests, in order.
        let mut tests: Vec<(usize, LabelTest)> = Vec::new();
        for (i, p) in parts.iter().enumerate() {
            for &l in &p.labels {
                tests.push((i, self.label_test(l)));
            }
        }
        let all_const = tests.iter().all(|(_, t)| matches!(t, LabelTest::Const(_)));

        self.push_scope();
        if all_const && !tests.is_empty() {
            let mut table: Vec<(ConstVal, BlockId)> = Vec::new();
            for (i, t) in &tests {
                if let LabelTest::Const(c) = t {
                    if !table.iter().any(|(k, _)| k == c) {
                        table.push((*c, bodies[*i]));
                    }
                }
            }
            self.b().terminate(
                Terminator::Switch {
                    scrutinee: subject.op,
                    cases: table,
                    default: default_target,
                },
                span,
            );
        } else {
            let subject_value = self.value_of(subject, span);
            let subject_ty = subject.ty;
            for (i, t) in tests {
                let next = self.b().create_block();
                match t {
                    LabelTest::Const(c) => {
                        let test = self.emit_def(TypeTable::BOOL, span, |dst| Inst::Compare {
                            dst,
                            pred: ComparePred::Eq,
                            lhs: Operand::Value(subject_value),
                            rhs: Operand::Const(c),
                        });
                        self.b().branch(test.op, bodies[i], next, span);
                    }
                    LabelTest::Value(node) => {
                        let v = self.case_value(node, subject_ty);
                        let test = self.emit_def(TypeTable::BOOL, span, |dst| Inst::Compare {
                            dst,
                            pred: ComparePred::Eq,
                            lhs: Operand::Value(subject_value),
                            rhs: v.op,
                        });
                        self.b().branch(test.op, bodies[i], next, span);
                    }
                    LabelTest::Pattern { ty, binding, node } => {
                        let test = self.emit_def(TypeTable::BOOL, span, |dst| Inst::TypeTest {
                            dst,
                            src: Operand::Value(subject_value),
                            ty,
                        });
                        let bind_b = self.b().create_block();
                        self.b().branch(test.op, bind_b, next, span);
                        self.b().seal(bind_b);
                        self.b().switch_to(bind_b);
                        if binding.is_some() {
                            let narrowed = self.emit_def(ty, span, |dst| Inst::Cast {
                                dst,
                                src: Operand::Value(subject_value),
                                ty,
                            });
                            let symbol = self.index.at_node(node);
                            let b = self.declare_local(binding, symbol, ty, span);
                            self.write_binding(b, narrowed, span);
                        }
                        self.b().jump(bodies[i], span);
                    }
                }
                self.b().seal(next);
                self.b().switch_to(next);
            }
            self.b().jump(default_target, span);
        }

        self.push_exit(Exit::Breakable {
            label,
            kind: BreakKind::Switch,
            break_to: exit,
            continue_to: None,
            result,
        });
        for (i, p) in parts.iter().enumerate() {
            // Every predecessor of a case body (the dispatch and the previous group's
            // fall-through) exists once the previous group is lowered.
            self.b().seal(bodies[i]);
            self.b().switch_to(bodies[i]);
            if p.is_arrow {
                if let Some(&body) = p.body.first() {
                    let cl = self.syn().class(body);
                    if as_expr && (cl.is_expression() || cl == NodeClass::ExprStmt) {
                        let v = self.expr(body);
                        if let Some(r) = result {
                            self.b().write_var(r, v.op);
                        }
                    } else {
                        self.scoped_stmt(body);
                    }
                }
                if self.cur_ref().b.is_open() {
                    self.b().jump(exit, span);
                }
            } else {
                for &s in &p.body {
                    self.stmt(s);
                }
                if self.cur_ref().b.is_open() {
                    let next = bodies.get(i + 1).copied().unwrap_or(exit);
                    self.b().jump(next, span);
                }
            }
        }
        self.pop_exit();
        self.pop_scope();
        self.b().seal(exit);
        self.b().switch_to(exit);
        result.map(|r| Val::new(self.b().read_var(r), TypeTable::UNKNOWN))
    }

    /// Classifies a case label without emitting code for constant labels.
    fn label_test(&mut self, l: u32) -> LabelTest {
        let syn = self.syn();
        match syn.class(l) {
            NodeClass::Literal => {
                let v = self.expr(l);
                match v.op.as_const() {
                    Some(c) => LabelTest::Const(c),
                    None => LabelTest::Value(l),
                }
            }
            NodeClass::Pattern => {
                let ty = match syn.first_kid_of(l, NodeClass::TypeRef) {
                    Some(t) => self.type_of_ref(t),
                    None => TypeTable::UNKNOWN,
                };
                let binding = syn
                    .first_kid_of(l, NodeClass::Name)
                    .and_then(|b| self.tree.text_id(b));
                LabelTest::Pattern {
                    ty,
                    binding,
                    node: l,
                }
            }
            NodeClass::TypeRef => LabelTest::Pattern {
                ty: self.type_of_ref(l),
                binding: None,
                node: l,
            },
            _ => LabelTest::Value(l),
        }
    }

    /// Value of a non-constant case label. A simple name not bound in scope is looked up
    /// among the members of the scrutinee's type (enum constants).
    fn case_value(&mut self, node: u32, subject_ty: TypeId) -> Val {
        let span = self.tree.span(node);
        if self.syn().class(node) == NodeClass::Name {
            let name = self.tree.text_id(node).unwrap_or(u32::MAX);
            let r = self.resolve_name(name);
            if r == Resolved::Unresolved {
                if let Some(owner) = self.module.types.symbol_of(subject_ty) {
                    if let Some((fsym, declaring)) = self.index.field_in_hierarchy(owner, name) {
                        let field = self.field_for_symbol(fsym);
                        let ty = self.module.fields[field.index()].ty;
                        return self.emit_def(ty, span, |dst| Inst::Load {
                            dst,
                            place: Place::Static {
                                owner: declaring,
                                field,
                            },
                        });
                    }
                }
            }
            return self.resolved_value(r, name, span);
        }
        self.expr(node)
    }

    /// try / catch / finally, including resource regions. The finally region encloses the
    /// handlers, which enclose the resource regions, which enclose the body: resources are
    /// released before a handler runs, and the finally clause runs on every exit of all.
    fn try_stmt(&mut self, n: u32) {
        let span = self.tree.span(n);
        let syn = self.syn();
        let parts = syn.try_parts(n);
        if let Some(fin) = parts.finally {
            self.push_cleanup(Cleanup::Finally(fin));
        }
        let handler = if parts.catches.is_empty() {
            None
        } else {
            let landing = self.b().create_block();
            self.push_exit(Exit::Handler { landing });
            self.cur().has_handlers = true;
            Some(landing)
        };

        self.push_scope();
        let mut resources = 0usize;
        if let Some(r) = parts.resources {
            for c in syn.active_kids(r) {
                self.resource(c, &mut resources);
            }
        }
        if let Some(body) = parts.body {
            self.stmt(body);
        }
        for _ in 0..resources {
            self.pop_cleanup(span);
        }
        self.pop_scope();

        let after = self.b().create_block();
        if self.cur_ref().b.is_open() {
            self.b().jump(after, span);
        }
        if let Some(landing) = handler {
            self.pop_exit();
            self.catch_dispatch(landing, &parts.catches, after, span);
        }
        self.b().seal(after);
        self.b().switch_to(after);
        if parts.finally.is_some() {
            self.pop_cleanup(span);
        }
    }

    /// Opens the region of one resource (a declaration or an expression).
    fn resource(&mut self, c: u32, count: &mut usize) {
        let span = self.tree.span(c);
        let syn = self.syn();
        let v = match syn.class(c) {
            NodeClass::LocalDecl | NodeClass::Declarator => {
                let parts = syn.local_decl_parts(c);
                let mut last = None;
                for d in parts.declarators {
                    let b = self.declare_variable(d, parts.type_ref);
                    last = Some(self.read_binding(b, span));
                }
                match last {
                    Some(v) => v,
                    None => return,
                }
            }
            cl if cl.is_expression() => self.expr(c),
            _ => return,
        };
        let resource = self.value_of(v, span);
        self.push_cleanup(Cleanup::Release {
            resource,
            ty: v.ty,
            span,
        });
        *count += 1;
    }

    /// Dispatches the exception arriving at `landing` to the first catch clause whose type
    /// test succeeds; an exception no clause accepts is rethrown outward.
    fn catch_dispatch(&mut self, landing: BlockId, catches: &[u32], after: BlockId, span: Span) {
        self.b().seal(landing);
        self.b().switch_to(landing);
        let exc = self.exception_value(span);
        let syn = self.syn();
        let mut pending: Vec<(BlockId, u32, Vec<TypeId>)> = Vec::new();
        for &c in catches {
            let cp = syn.catch_parts(c);
            let types: Vec<TypeId> = cp.types.iter().map(|&t| self.type_of_ref(t)).collect();
            let catch_b = self.b().create_block();
            if types.is_empty() {
                self.diag(
                    DiagnosticKind::NotDerivable,
                    self.tree.span(c),
                    "the exception type of this catch clause is not recorded; whether it \
                     accepts an exception is opaque"
                        .to_string(),
                );
                let test = self.opaque(IntrinsicKind::Opaque, b"catch-type", vec![Operand::Value(exc)], span);
                let next = self.b().create_block();
                self.b().branch(test.op, catch_b, next, span);
                self.b().seal(next);
                self.b().switch_to(next);
            } else {
                for &t in &types {
                    let test = self.emit_def(TypeTable::BOOL, span, |dst| Inst::TypeTest {
                        dst,
                        src: Operand::Value(exc),
                        ty: t,
                    });
                    let next = self.b().create_block();
                    self.b().branch(test.op, catch_b, next, span);
                    self.b().seal(next);
                    self.b().switch_to(next);
                }
            }
            pending.push((catch_b, c, types));
        }
        let unwind = self.unwind_target();
        self.b().throw(Operand::Value(exc), unwind, span);

        for (catch_b, c, types) in pending {
            let cspan = self.tree.span(c);
            self.b().seal(catch_b);
            self.b().switch_to(catch_b);
            self.push_scope();
            let cp = syn.catch_parts(c);
            let ty = if types.len() == 1 {
                types[0]
            } else {
                TypeTable::UNKNOWN
            };
            let value = if ty != TypeTable::UNKNOWN {
                self.emit_def(ty, cspan, |dst| Inst::Cast {
                    dst,
                    src: Operand::Value(exc),
                    ty,
                })
            } else {
                Val::new(Operand::Value(exc), TypeTable::UNKNOWN)
            };
            let name = cp.name_token.and_then(|t| self.tree.token_text_id(t));
            let symbol = syn
                .first_kid_of(c, NodeClass::Param)
                .and_then(|p| self.index.at_node(p));
            if name.is_some() {
                let b = self.declare_local(name, symbol, ty, cspan);
                self.write_binding(b, value, cspan);
            }
            if let Some(body) = cp.body {
                self.stmt(body);
            }
            if self.cur_ref().b.is_open() {
                self.b().jump(after, cspan);
            }
            self.pop_scope();
        }
    }

    fn return_stmt(&mut self, n: u32) {
        let span = self.tree.span(n);
        let syn = self.syn();
        let ret = self.cur_ref().ret;
        let value = syn
            .active_kids(n)
            .into_iter()
            .find(|&c| syn.class(c).is_expression())
            .map(|c| self.expr_hint(c, Some(ret)));
        self.run_cleanups_down_to(0);
        self.b()
            .terminate(Terminator::Return(value.map(|v| v.op)), span);
    }

    fn throw_stmt(&mut self, n: u32) {
        let span = self.tree.span(n);
        let syn = self.syn();
        let value = match syn
            .active_kids(n)
            .into_iter()
            .find(|&c| syn.class(c).is_expression())
        {
            Some(c) => self.expr(c),
            None => Val::undef(),
        };
        let unwind = self.unwind_target();
        self.b().throw(value.op, unwind, span);
    }

    fn break_stmt(&mut self, n: u32, is_continue: bool) {
        let span = self.tree.span(n);
        let label = self.syn().label_of(n);
        match self.break_target(label, is_continue) {
            Some((depth, target, _)) => {
                self.run_cleanups_down_to(depth + 1);
                self.b().jump(target, span);
            }
            None => {
                self.diag(
                    DiagnosticKind::Unresolved,
                    span,
                    format!(
                        "{} has no enclosing target{}",
                        if is_continue { "continue" } else { "break" },
                        match label {
                            Some(l) => format!(" labeled '{}'", self.name_text(l)),
                            None => String::new(),
                        }
                    ),
                );
                self.b().terminate(Terminator::Unreachable, span);
            }
        }
    }

    fn yield_stmt(&mut self, n: u32) {
        let span = self.tree.span(n);
        let syn = self.syn();
        let value = syn
            .active_kids(n)
            .into_iter()
            .find(|&c| syn.class(c).is_expression())
            .map(|c| self.expr(c))
            .unwrap_or_else(Val::undef);
        match self.yield_target() {
            Some((depth, exit, var)) => {
                self.b().write_var(var, value.op);
                self.run_cleanups_down_to(depth + 1);
                self.b().jump(exit, span);
            }
            None => {
                self.diag(
                    DiagnosticKind::Unresolved,
                    span,
                    "yield outside a switch expression".to_string(),
                );
                self.b().terminate(Terminator::Unreachable, span);
            }
        }
    }

    /// Labeled statement: a goto target, and the target of `break label` (and of
    /// `continue label` when it labels a loop).
    fn labeled_stmt(&mut self, n: u32) {
        let span = self.tree.span(n);
        let syn = self.syn();
        let name = syn.label_of(n);
        let label_node = syn.first_kid_of(n, NodeClass::Name);
        let body = syn
            .active_kids(n)
            .into_iter()
            .rev()
            .find(|&c| Some(c) != label_node);
        let Some(name) = name else {
            if let Some(b) = body {
                self.stmt(b);
            }
            return;
        };
        let target = self.label_block(name);
        self.b().jump(target, span);
        self.b().switch_to(target);
        if !self.cur_ref().has_gotos() {
            self.b().seal(target);
        }
        let Some(body) = body else { return };
        match syn.class(body) {
            NodeClass::While
            | NodeClass::DoWhile
            | NodeClass::For
            | NodeClass::ForEach
            | NodeClass::Switch
            | NodeClass::SwitchExpr => {
                self.cur().pending_label = Some(name);
                self.stmt(body);
                self.cur().pending_label = None;
            }
            _ => {
                let after = self.b().create_block();
                self.push_exit(Exit::Breakable {
                    label: Some(name),
                    kind: BreakKind::Block,
                    break_to: after,
                    continue_to: None,
                    result: None,
                });
                self.stmt(body);
                if self.cur_ref().b.is_open() {
                    self.b().jump(after, span);
                }
                self.pop_exit();
                self.b().seal(after);
                self.b().switch_to(after);
            }
        }
    }

    fn label_block(&mut self, name: u32) -> BlockId {
        if let Some(&b) = self.cur_ref().labels.get(&name) {
            return b;
        }
        let b = self.b().create_block();
        self.cur().labels.insert(name, b);
        b
    }

    fn goto_stmt(&mut self, n: u32) {
        let span = self.tree.span(n);
        let Some(name) = self.syn().label_of(n) else {
            self.diag(
                DiagnosticKind::NotDerivable,
                span,
                "computed goto target".to_string(),
            );
            self.b().terminate(Terminator::Unreachable, span);
            return;
        };
        if self.has_cleanups_above(0) {
            self.diag(
                DiagnosticKind::UnderApproximate,
                span,
                "goto inside a cleanup region: the regions it leaves are not derivable before \
                 the label is seen, so their cleanups are not run on this edge"
                    .to_string(),
            );
        }
        let target = self.label_block(name);
        self.b().jump(target, span);
    }

    /// Region guarded by a lock: acquired at entry, released on every exit.
    fn lock_stmt(&mut self, n: u32) {
        let span = self.tree.span(n);
        let syn = self.syn();
        let kids = syn.active_kids(n);
        let lock = kids.first().copied();
        let body = kids.get(1).copied();
        let v = match lock {
            Some(l) => self.expr(l),
            None => Val::undef(),
        };
        let lv = self.value_of(v, span);
        self.intrinsic(IntrinsicKind::MonitorEnter, vec![Operand::Value(lv)], None, span);
        self.push_cleanup(Cleanup::Monitor { lock: lv, span });
        if let Some(b) = body {
            self.stmt(b);
        }
        self.pop_cleanup(span);
    }

    /// Resource region without handlers.
    fn using_stmt(&mut self, n: u32) {
        let span = self.tree.span(n);
        let syn = self.syn();
        let kids = syn.active_kids(n);
        let Some((&body, resources)) = kids.split_last() else {
            return;
        };
        self.push_scope();
        let mut count = 0usize;
        for &r in resources {
            if syn.class(r) == NodeClass::Resources {
                for c in syn.active_kids(r) {
                    self.resource(c, &mut count);
                }
            } else {
                self.resource(r, &mut count);
            }
        }
        self.stmt(body);
        for _ in 0..count {
            self.pop_cleanup(span);
        }
        self.pop_scope();
    }

    /// Assertion: the message is evaluated only when the condition fails.
    fn assert_stmt(&mut self, n: u32) {
        let span = self.tree.span(n);
        let kids = self.syn().active_kids(n);
        let Some(&cond) = kids.first() else { return };
        match kids.get(1).copied() {
            None => {
                let c = self.expr(cond);
                self.intrinsic(IntrinsicKind::Assert, vec![c.op], None, span);
            }
            Some(msg) => {
                let ok = self.b().create_block();
                let fail = self.b().create_block();
                let join = self.b().create_block();
                self.cond(cond, ok, fail);
                self.b().seal(ok);
                self.b().seal(fail);
                self.b().switch_to(fail);
                let m = self.expr(msg);
                self.intrinsic(
                    IntrinsicKind::Assert,
                    vec![Operand::bool(false), m.op],
                    None,
                    span,
                );
                self.b().jump(join, span);
                self.b().switch_to(ok);
                self.b().jump(join, span);
                self.b().seal(join);
                self.b().switch_to(join);
            }
        }
    }
}
