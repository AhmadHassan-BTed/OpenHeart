//! Basic blocks and functions, with successor / predecessor and def-use computation.

use crate::ir::inst::{Inst, Terminator};
use crate::ir::types::{BlockId, FuncId, InstId, Span, TypeId, TypeTable, ValueId};
use crate::ir::value::ValueInfo;

/// How a function came into existence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FuncKind {
    /// Declared method or free function.
    Method,
    Constructor,
    /// Per-type initialisation: static field initialisers and static blocks, in order.
    StaticInit,
    /// Per-instance initialisation: instance field initialisers and instance blocks, in
    /// order. Constructors call it.
    InstanceInit,
    /// Per-file initialisation of module-level variables.
    ModuleInit,
    /// Lambda or anonymous function (`is_closure`).
    Lambda,
    /// Function synthesized to adapt a method reference to a function value (`is_closure`).
    Adapter,
    /// Implicit constructor of a type that declares none.
    ImplicitConstructor,
}

impl FuncKind {
    pub fn mnemonic(self) -> &'static str {
        match self {
            FuncKind::Method => "method",
            FuncKind::Constructor => "constructor",
            FuncKind::StaticInit => "static-init",
            FuncKind::InstanceInit => "instance-init",
            FuncKind::ModuleInit => "module-init",
            FuncKind::Lambda => "lambda",
            FuncKind::Adapter => "adapter",
            FuncKind::ImplicitConstructor => "implicit-constructor",
        }
    }

    /// True for functions whose body was synthesized from language rules rather than
    /// written as one declaration.
    pub fn is_synthetic(self) -> bool {
        matches!(
            self,
            FuncKind::StaticInit
                | FuncKind::InstanceInit
                | FuncKind::ModuleInit
                | FuncKind::Adapter
                | FuncKind::ImplicitConstructor
        )
    }
}

/// Basic block: phis first, then ordinary instructions, then exactly one terminator.
#[derive(Debug, Clone, PartialEq)]
pub struct BlockIr {
    pub id: BlockId,
    pub insts: Vec<Inst>,
    /// `inst_spans[i]` is the provenance of `insts[i]`.
    pub inst_spans: Vec<Span>,
    pub term: Terminator,
    pub term_span: Span,
    /// Landing block that receives the exception of a `Terminator::Throw`; `None` when the
    /// exception leaves the function.
    pub unwind: Option<BlockId>,
}

impl BlockIr {
    pub fn new(id: BlockId) -> Self {
        Self {
            id,
            insts: Vec::new(),
            inst_spans: Vec::new(),
            term: Terminator::Unreachable,
            term_span: Span::NONE,
            unwind: None,
        }
    }

    pub fn push(&mut self, inst: Inst, span: Span) {
        self.insts.push(inst);
        self.inst_spans.push(span);
    }

    /// Successors through the terminator.
    pub fn normal_successors(&self) -> Vec<BlockId> {
        self.term.successors()
    }

    /// Landing blocks reachable by an exception raised in this block.
    pub fn exceptional_successors(&self) -> Vec<BlockId> {
        let mut out = Vec::new();
        for inst in &self.insts {
            if let Some(u) = inst.unwind() {
                if !out.contains(&u) {
                    out.push(u);
                }
            }
        }
        if let (Terminator::Throw(_), Some(u)) = (&self.term, self.unwind) {
            if !out.contains(&u) {
                out.push(u);
            }
        }
        out
    }

    /// Normal then exceptional successors, without duplicates.
    pub fn successors(&self) -> Vec<BlockId> {
        let mut out = self.normal_successors();
        for s in self.exceptional_successors() {
            if !out.contains(&s) {
                out.push(s);
            }
        }
        out
    }

    /// Phi instructions of the block.
    pub fn phis(&self) -> impl Iterator<Item = &Inst> {
        self.insts.iter().take_while(|i| i.is_phi())
    }

    /// Index of the first non-phi instruction.
    pub fn first_non_phi(&self) -> usize {
        self.insts.iter().take_while(|i| i.is_phi()).count()
    }

    /// True when the block raises an exception that leaves the function.
    pub fn throws_out(&self) -> bool {
        matches!(self.term, Terminator::Throw(_)) && self.unwind.is_none()
    }
}

/// Where a value is defined.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefSite {
    /// `FuncIr::params[i]`.
    Param(u32),
    Inst { block: BlockId, index: u32 },
    /// No definition remains (an id released by SSA construction).
    None,
}

/// One use of a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UseSite {
    Inst { block: BlockId, index: u32 },
    Terminator(BlockId),
}

/// Function in SSA form.
///
/// Parameter layout: `params = captures ++ [receiver] ++ declared parameters`, where the
/// first `capture_count` parameters receive the `MakeClosure` captures and the receiver is
/// present when `has_receiver` is set.
#[derive(Debug, Clone, PartialEq)]
pub struct FuncIr {
    pub id: FuncId,
    /// Declaring symbol (method, constructor, initializer or lambda symbol).
    pub symbol: Option<u32>,
    /// Interned name (`ModuleIr::strings`).
    pub name: u32,
    pub params: Vec<ValueId>,
    pub ret: TypeId,
    pub blocks: Vec<BlockIr>,
    pub entry: BlockId,
    pub span: Span,
    pub is_closure: bool,
    /// Symbol of the type the function is a member of.
    pub enclosing_type: Option<u32>,
    pub kind: FuncKind,
    pub has_receiver: bool,
    pub capture_count: u32,
    /// Metadata of every value id; `values[v]` describes `ValueId(v)`.
    pub values: Vec<ValueInfo>,
}

impl FuncIr {
    pub fn new(id: FuncId, name: u32, ret: TypeId, span: Span, kind: FuncKind) -> Self {
        Self {
            id,
            symbol: None,
            name,
            params: Vec::new(),
            ret,
            blocks: Vec::new(),
            entry: BlockId(0),
            span,
            is_closure: matches!(kind, FuncKind::Lambda | FuncKind::Adapter),
            enclosing_type: None,
            kind,
            has_receiver: false,
            capture_count: 0,
            values: Vec::new(),
        }
    }

    #[inline]
    pub fn block(&self, b: BlockId) -> &BlockIr {
        &self.blocks[b.index()]
    }

    #[inline]
    pub fn block_mut(&mut self, b: BlockId) -> &mut BlockIr {
        &mut self.blocks[b.index()]
    }

    #[inline]
    pub fn block_count(&self) -> usize {
        self.blocks.len()
    }

    pub fn block_ids(&self) -> impl Iterator<Item = BlockId> {
        (0..self.blocks.len()).map(BlockId::new)
    }

    #[inline]
    pub fn value_count(&self) -> usize {
        self.values.len()
    }

    /// Type of a value; ids outside the table read as unknown.
    pub fn value_type(&self, v: ValueId) -> TypeId {
        self.values
            .get(v.index())
            .map(|i| i.ty)
            .unwrap_or(TypeTable::UNKNOWN)
    }

    pub fn value_info(&self, v: ValueId) -> Option<&ValueInfo> {
        self.values.get(v.index())
    }

    /// Closure captures (the leading parameters).
    pub fn captures(&self) -> &[ValueId] {
        let n = (self.capture_count as usize).min(self.params.len());
        &self.params[..n]
    }

    /// Receiver parameter.
    pub fn receiver(&self) -> Option<ValueId> {
        if self.has_receiver {
            self.params.get(self.capture_count as usize).copied()
        } else {
            None
        }
    }

    /// Declared parameters (after captures and receiver).
    pub fn declared_params(&self) -> &[ValueId] {
        let skip = self.capture_count as usize + self.has_receiver as usize;
        &self.params[skip.min(self.params.len())..]
    }

    /// All successors (normal and exceptional) of block `b`.
    pub fn successors(&self, b: BlockId) -> Vec<BlockId> {
        self.block(b).successors()
    }

    /// Successor lists of every block, indexed by block id.
    pub fn successor_lists(&self) -> Vec<Vec<BlockId>> {
        self.blocks.iter().map(|b| b.successors()).collect()
    }

    /// Predecessor lists of every block (normal and exceptional edges), in block order.
    pub fn predecessor_lists(&self) -> Vec<Vec<BlockId>> {
        let mut preds = vec![Vec::new(); self.blocks.len()];
        for block in &self.blocks {
            for s in block.successors() {
                if let Some(list) = preds.get_mut(s.index()) {
                    if !list.contains(&block.id) {
                        list.push(block.id);
                    }
                }
            }
        }
        preds
    }

    /// Blocks that leave the function: returns and throws without a landing block.
    pub fn exit_blocks(&self) -> Vec<BlockId> {
        self.blocks
            .iter()
            .filter(|b| b.term.is_return() || b.throws_out())
            .map(|b| b.id)
            .collect()
    }

    /// Blocks whose calls can propagate an exception out of the function (calls without a
    /// landing block), excluding explicit throws.
    pub fn blocks_with_escaping_calls(&self) -> Vec<BlockId> {
        self.blocks
            .iter()
            .filter(|b| {
                b.insts
                    .iter()
                    .any(|i| matches!(i, Inst::Call { unwind: None, .. }))
            })
            .map(|b| b.id)
            .collect()
    }

    pub fn inst_count(&self) -> usize {
        self.blocks.iter().map(|b| b.insts.len()).sum()
    }

    /// Definition site of every value.
    pub fn def_sites(&self) -> Vec<DefSite> {
        let mut defs = vec![DefSite::None; self.values.len()];
        for (i, &p) in self.params.iter().enumerate() {
            if let Some(slot) = defs.get_mut(p.index()) {
                *slot = DefSite::Param(i as u32);
            }
        }
        for block in &self.blocks {
            for (idx, inst) in block.insts.iter().enumerate() {
                if let Some(d) = inst.dst() {
                    if let Some(slot) = defs.get_mut(d.index()) {
                        *slot = DefSite::Inst {
                            block: block.id,
                            index: idx as u32,
                        };
                    }
                }
            }
        }
        defs
    }

    /// Use lists of every value, in block and instruction order.
    pub fn use_lists(&self) -> Vec<Vec<UseSite>> {
        let mut uses = vec![Vec::new(); self.values.len()];
        for block in &self.blocks {
            for (idx, inst) in block.insts.iter().enumerate() {
                for v in inst.uses() {
                    if let Some(list) = uses.get_mut(v.index()) {
                        list.push(UseSite::Inst {
                            block: block.id,
                            index: idx as u32,
                        });
                    }
                }
            }
            for v in block.term.uses() {
                if let Some(list) = uses.get_mut(v.index()) {
                    list.push(UseSite::Terminator(block.id));
                }
            }
        }
        uses
    }

    /// Instruction at a position.
    pub fn inst(&self, block: BlockId, index: usize) -> Option<&Inst> {
        self.blocks.get(block.index())?.insts.get(index)
    }

    /// Provenance of the instruction at a position.
    pub fn inst_span(&self, block: BlockId, index: usize) -> Span {
        self.blocks
            .get(block.index())
            .and_then(|b| b.inst_spans.get(index).copied())
            .unwrap_or(Span::NONE)
    }
}

/// Dense numbering of the instructions of a function: blocks in id order, instructions in
/// block order. `InstId`s are only meaningful together with the numbering of the same,
/// unmodified function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstNumbering {
    /// `offsets[b]` is the id of the first instruction of block `b`; the last entry is the
    /// total instruction count.
    offsets: Vec<u32>,
}

impl InstNumbering {
    pub fn new(func: &FuncIr) -> Self {
        let mut offsets = Vec::with_capacity(func.blocks.len() + 1);
        let mut acc = 0u32;
        for b in &func.blocks {
            offsets.push(acc);
            acc += b.insts.len() as u32;
        }
        offsets.push(acc);
        Self { offsets }
    }

    pub fn count(&self) -> usize {
        self.offsets.last().copied().unwrap_or(0) as usize
    }

    pub fn id(&self, block: BlockId, index: usize) -> InstId {
        InstId(self.offsets[block.index()] + index as u32)
    }

    /// Block and position of an instruction id.
    pub fn locate(&self, id: InstId) -> Option<(BlockId, usize)> {
        if id.index() >= self.count() {
            return None;
        }
        // Last block whose first id is <= id; empty blocks share their offset with the
        // following block, so the search skips to the block that holds the instruction.
        let pos = match self.offsets.binary_search(&id.0) {
            Ok(mut p) => {
                while p + 1 < self.offsets.len() && self.offsets[p + 1] == id.0 {
                    p += 1;
                }
                p
            }
            Err(p) => p - 1,
        };
        Some((BlockId::new(pos), (id.0 - self.offsets[pos]) as usize))
    }
}
