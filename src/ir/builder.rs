//! Function builder: block creation, instruction emission, terminators, sealing and the
//! SSA variable interface used by lowering.
//!
//! The builder keeps a current insertion block. Emitting into a block that has already
//! been terminated (code after `return`, `break`, `throw`) opens a fresh block that no edge
//! reaches; such blocks, and everything only they reach, are removed by `finish`, after SSA
//! construction has dropped the phi operands that flowed in from them.
//!
//! Call and allocation site ids inside a finished function are function-local
//! (`CallSiteId(i)` indexes `FinishedFunction::call_meta`, `AllocSiteId(i)` counts
//! allocations in emission order); the module assigns global ids when it registers the
//! function.

use crate::ir::func::{BlockIr, FuncIr, FuncKind};
use crate::ir::inst::{Callee, Inst, Region, Terminator};
use crate::ir::ssa::{SsaBuilder, VarId, VarInfo};
use crate::ir::types::{AllocSiteId, BlockId, CallSiteId, FuncId, Span, TypeId, ValueId};
use crate::ir::value::{Operand, ValueInfo};

/// Per-call facts the module needs for `CallSiteIr`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CallMeta {
    pub receiver: Option<ValueId>,
    pub candidates: Vec<u32>,
}

/// A function whose blocks are complete and in SSA form.
#[derive(Debug, Clone)]
pub struct FinishedFunction {
    pub func: FuncIr,
    /// Indexed by the function-local call site id of each `Inst::Call`.
    pub call_meta: Vec<CallMeta>,
}

pub struct FunctionBuilder {
    func: FuncIr,
    preds: Vec<Vec<BlockId>>,
    terminated: Vec<bool>,
    current: Option<BlockId>,
    ssa: SsaBuilder,
    call_meta: Vec<CallMeta>,
    alloc_count: u32,
}

impl FunctionBuilder {
    /// Starts a function with an empty, sealed entry block as the insertion point.
    pub fn new(id: FuncId, name: u32, ret: TypeId, span: Span, kind: FuncKind) -> Self {
        let mut b = Self {
            func: FuncIr::new(id, name, ret, span, kind),
            preds: Vec::new(),
            terminated: Vec::new(),
            current: None,
            ssa: SsaBuilder::new(),
            call_meta: Vec::new(),
            alloc_count: 0,
        };
        let entry = b.create_block();
        b.func.entry = entry;
        b.seal(entry);
        b.current = Some(entry);
        b
    }

    #[inline]
    pub fn func(&self) -> &FuncIr {
        &self.func
    }

    #[inline]
    pub fn func_mut(&mut self) -> &mut FuncIr {
        &mut self.func
    }

    pub fn create_block(&mut self) -> BlockId {
        let id = BlockId::new(self.func.blocks.len());
        self.func.blocks.push(BlockIr::new(id));
        self.preds.push(Vec::new());
        self.terminated.push(false);
        self.ssa.ensure_block(id);
        id
    }

    /// Makes `b` the insertion block.
    pub fn switch_to(&mut self, b: BlockId) {
        self.current = Some(b);
    }

    /// Current insertion block, when it is still open.
    pub fn current_block(&self) -> Option<BlockId> {
        match self.current {
            Some(b) if !self.terminated[b.index()] => Some(b),
            _ => None,
        }
    }

    /// True when code emitted now is reachable from the entry along the edges built so far
    /// or may still become reachable (an open block).
    #[inline]
    pub fn is_open(&self) -> bool {
        self.current_block().is_some()
    }

    /// The open insertion block, creating an unreachable one when the current block has
    /// been terminated.
    pub fn ensure_open(&mut self) -> BlockId {
        if let Some(b) = self.current_block() {
            return b;
        }
        let b = self.create_block();
        self.seal(b);
        self.current = Some(b);
        b
    }

    #[inline]
    pub fn is_terminated(&self, b: BlockId) -> bool {
        self.terminated[b.index()]
    }

    pub fn preds(&self, b: BlockId) -> &[BlockId] {
        &self.preds[b.index()]
    }

    pub fn has_preds(&self, b: BlockId) -> bool {
        !self.preds[b.index()].is_empty()
    }

    pub fn new_value(&mut self, ty: TypeId) -> ValueId {
        self.new_value_with(ValueInfo::temp(ty))
    }

    pub fn new_value_with(&mut self, info: ValueInfo) -> ValueId {
        let id = ValueId::new(self.func.values.len());
        self.func.values.push(info);
        id
    }

    pub fn value_type(&self, v: ValueId) -> TypeId {
        self.func.value_type(v)
    }

    /// Appends a parameter.
    pub fn add_param(&mut self, info: ValueInfo) -> ValueId {
        let v = self.new_value_with(info);
        self.func.params.push(v);
        v
    }

    /// Inserts a parameter at position `index` (closure captures are discovered while the
    /// body is lowered and precede the declared parameters).
    pub fn insert_param(&mut self, index: usize, info: ValueInfo) -> ValueId {
        let v = self.new_value_with(info);
        let at = index.min(self.func.params.len());
        self.func.params.insert(at, v);
        v
    }

    pub fn emit(&mut self, inst: Inst, span: Span) {
        let b = self.ensure_open();
        self.func.blocks[b.index()].push(inst, span);
    }

    /// Emits an instruction that defines a fresh value of type `ty`.
    pub fn emit_def(&mut self, ty: TypeId, span: Span, make: impl FnOnce(ValueId) -> Inst) -> ValueId {
        let dst = self.new_value(ty);
        self.emit(make(dst), span);
        dst
    }

    pub fn emit_alloc(&mut self, ty: TypeId, region: Region, span: Span) -> ValueId {
        let site = AllocSiteId(self.alloc_count);
        self.alloc_count += 1;
        self.emit_def(ty, span, |dst| Inst::Alloc {
            dst,
            site,
            ty,
            region,
        })
    }

    /// Emits a call. A call with a landing block ends its block, so the state at the end of
    /// the block (what the landing block observes) is the state at the call.
    pub fn emit_call(
        &mut self,
        dst_ty: Option<TypeId>,
        callee: Callee,
        args: Vec<Operand>,
        unwind: Option<BlockId>,
        meta: CallMeta,
        span: Span,
    ) -> Option<ValueId> {
        let b = self.ensure_open();
        let dst = dst_ty.map(|t| self.new_value(t));
        let site = CallSiteId::new(self.call_meta.len());
        self.call_meta.push(meta);
        self.func.blocks[b.index()].push(
            Inst::Call {
                dst,
                site,
                callee,
                args,
                unwind,
            },
            span,
        );
        if let Some(h) = unwind {
            self.add_edge(b, h);
            let next = self.create_block();
            self.terminate(Terminator::Jump(next), span);
            self.seal(next);
            self.switch_to(next);
        }
        dst
    }

    /// Ends the current block.
    pub fn terminate(&mut self, term: Terminator, span: Span) {
        let b = self.ensure_open();
        for s in term.successors() {
            self.add_edge(b, s);
        }
        let block = &mut self.func.blocks[b.index()];
        block.term = term;
        block.term_span = span;
        self.terminated[b.index()] = true;
        self.current = None;
    }

    pub fn jump(&mut self, target: BlockId, span: Span) {
        self.terminate(Terminator::Jump(target), span);
    }

    pub fn branch(&mut self, cond: Operand, then_b: BlockId, else_b: BlockId, span: Span) {
        if then_b == else_b {
            self.terminate(Terminator::Jump(then_b), span);
        } else {
            self.terminate(
                Terminator::Branch {
                    cond,
                    then_b,
                    else_b,
                },
                span,
            );
        }
    }

    /// Ends the current block with a throw that lands in `unwind` (or leaves the function).
    pub fn throw(&mut self, value: Operand, unwind: Option<BlockId>, span: Span) {
        let b = self.ensure_open();
        if let Some(h) = unwind {
            self.add_edge(b, h);
        }
        self.func.blocks[b.index()].unwind = unwind;
        self.terminate(Terminator::Throw(value), span);
    }

    fn add_edge(&mut self, from: BlockId, to: BlockId) {
        let list = &mut self.preds[to.index()];
        if !list.contains(&from) {
            list.push(from);
        }
    }

    /// Declares that every predecessor of `b` has been added.
    pub fn seal(&mut self, b: BlockId) {
        self.ssa.seal(b, &self.preds, &mut self.func.values);
    }

    pub fn is_sealed(&self, b: BlockId) -> bool {
        self.ssa.is_sealed(b)
    }

    pub fn declare_var(&mut self, info: VarInfo) -> VarId {
        self.ssa.declare_var(info)
    }

    pub fn var_info(&self, var: VarId) -> VarInfo {
        self.ssa.var_info(var)
    }

    /// Assigns `value` to `var` at the insertion point.
    pub fn write_var(&mut self, var: VarId, value: Operand) {
        let b = self.ensure_open();
        self.ssa.write(var, b, value);
    }

    /// Assigns `value` to `var` at the end of block `b` (definitions that belong to an
    /// earlier block, such as closure-capture parameters discovered mid-body, which are
    /// defined at the entry).
    pub fn write_var_in(&mut self, var: VarId, b: BlockId, value: Operand) {
        self.ssa.write(var, b, value);
    }

    /// Current value of `var` at the insertion point.
    pub fn read_var(&mut self, var: VarId) -> Operand {
        let b = self.ensure_open();
        self.ssa.read(var, b, &self.preds, &mut self.func.values)
    }

    /// Value of `var` at the end of block `b`.
    pub fn read_var_in(&mut self, var: VarId, b: BlockId) -> Operand {
        self.ssa.read(var, b, &self.preds, &mut self.func.values)
    }

    /// Resolves an operand through the phi replacements made so far.
    pub fn resolve(&self, op: Operand) -> Operand {
        self.ssa.resolve(op)
    }

    /// Completes the function: seals every block, removes redundant phis, drops the blocks
    /// the entry does not reach, materialises the phis, applies phi replacements to every
    /// operand and renumbers the blocks densely.
    pub fn finish(mut self) -> FinishedFunction {
        if self.is_open() {
            self.terminate(Terminator::Unreachable, Span::NONE);
        }
        for i in 0..self.func.blocks.len() {
            let b = BlockId::new(i);
            if !self.ssa.is_sealed(b) {
                self.seal(b);
            }
        }

        let reachable = self.reachable_blocks();
        self.ssa.retain_incoming(&|b: BlockId| reachable[b.index()]);
        self.ssa.remove_redundant_phi_sccs();

        let FunctionBuilder {
            mut func,
            ssa,
            call_meta,
            ..
        } = self;

        // Materialised phis, grouped by block in creation order.
        let mut phis_of: Vec<Vec<Inst>> = vec![Vec::new(); func.blocks.len()];
        for p in ssa.live_phis() {
            if let Some(node) = ssa.phi(p) {
                if reachable[node.block.index()] {
                    let incoming = node
                        .incoming
                        .iter()
                        .map(|&(b, op)| (b, ssa.resolve(op)))
                        .collect();
                    phis_of[node.block.index()].push(Inst::Phi { dst: p, incoming });
                }
            }
        }

        let values = &mut func.values;
        for (bi, block) in func.blocks.iter_mut().enumerate() {
            if !reachable[bi] {
                continue;
            }
            let old_insts = std::mem::take(&mut block.insts);
            let old_spans = std::mem::take(&mut block.inst_spans);
            let block_phis = std::mem::take(&mut phis_of[bi]);
            let mut insts = Vec::with_capacity(old_insts.len() + block_phis.len());
            let mut spans = Vec::with_capacity(old_insts.len() + block_phis.len());
            for phi in block_phis {
                insts.push(phi);
                spans.push(block.term_span);
            }
            for (mut inst, span) in old_insts.into_iter().zip(old_spans) {
                let mut pre: Vec<Inst> = Vec::new();
                inst.for_each_value_slot(&mut |v: &mut ValueId| {
                    match ssa.resolve(Operand::Value(*v)) {
                        Operand::Value(n) => *v = n,
                        other => {
                            // A replaced phi stood for a constant or for no value; slots that
                            // must hold a value receive a copy of it.
                            let ty = values
                                .get(v.index())
                                .map(|i| i.ty)
                                .unwrap_or(crate::ir::types::TypeTable::UNKNOWN);
                            let fresh = ValueId::new(values.len());
                            values.push(ValueInfo::temp(ty));
                            pre.push(Inst::Assign {
                                dst: fresh,
                                src: other,
                            });
                            *v = fresh;
                        }
                    }
                });
                inst.for_each_operand_slot(&mut |op: &mut Operand| *op = ssa.resolve(*op));
                for p in pre {
                    insts.push(p);
                    spans.push(span);
                }
                insts.push(inst);
                spans.push(span);
            }
            block.insts = insts;
            block.inst_spans = spans;
            block
                .term
                .for_each_operand_slot(&mut |op: &mut Operand| *op = ssa.resolve(*op));
        }

        // Dense renumbering of the reachable blocks.
        let mut remap: Vec<Option<BlockId>> = vec![None; func.blocks.len()];
        let mut next = 0usize;
        for (i, r) in reachable.iter().enumerate() {
            if *r {
                remap[i] = Some(BlockId::new(next));
                next += 1;
            }
        }
        let old_blocks = std::mem::take(&mut func.blocks);
        for (i, mut block) in old_blocks.into_iter().enumerate() {
            let Some(new_id) = remap[i] else { continue };
            block.id = new_id;
            let mut fix = |b: &mut BlockId| {
                if let Some(n) = remap.get(b.index()).copied().flatten() {
                    *b = n;
                }
            };
            for inst in block.insts.iter_mut() {
                inst.for_each_block_slot(&mut fix);
            }
            block.term.for_each_successor_slot(&mut fix);
            if let Some(u) = block.unwind.as_mut() {
                fix(u);
            }
            func.blocks.push(block);
        }
        func.entry = remap
            .get(func.entry.index())
            .copied()
            .flatten()
            .unwrap_or(BlockId(0));

        FinishedFunction { func, call_meta }
    }

    fn reachable_blocks(&self) -> Vec<bool> {
        let n = self.func.blocks.len();
        let mut seen = vec![false; n];
        let mut stack = vec![self.func.entry];
        if n == 0 {
            return seen;
        }
        seen[self.func.entry.index()] = true;
        while let Some(b) = stack.pop() {
            for s in self.func.blocks[b.index()].successors() {
                if s.index() < n && !seen[s.index()] {
                    seen[s.index()] = true;
                    stack.push(s);
                }
            }
        }
        seen
    }
}
