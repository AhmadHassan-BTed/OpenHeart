//! On-the-fly SSA construction (Braun, Buchwald, Hack, Leißa, Mallon, Zwinkau: "Simple and
//! Efficient Construction of Static Single Assignment Form", CC 2013).
//!
//! Lowering writes and reads source variables block by block; the builder answers each read
//! with the reaching SSA value, creating phi functions only at join points that need one:
//! * A block is *sealed* once all of its predecessors are known. Reads in an unsealed block
//!   create an operand-less ("incomplete") phi that is completed when the block is sealed.
//! * A phi whose operands are all the same value (or itself) is trivial and is replaced by
//!   that value; replacement cascades to the phis that used it (Algorithm 3).
//! * `remove_redundant_phi_sccs` removes the redundant phi cycles that trivial-phi removal
//!   cannot see in irreducible control flow (Algorithm 5).
//!
//! The algorithms are run with explicit work stacks instead of recursion, so the depth of a
//! read is not bounded by the native stack. Replacement is recorded in a forwarding map and
//! applied to the instruction stream when the function is finished.

use std::collections::{HashMap, HashSet};

use crate::ir::types::{BlockId, TypeId, ValueId};
use crate::ir::value::{Operand, ValueInfo};

/// Source variable tracked by SSA construction (a local, a parameter, the receiver, or a
/// temporary of a value-producing control construct).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct VarId(pub u32);

impl VarId {
    #[inline]
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// Declared type and provenance of a variable; its phis inherit them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VarInfo {
    pub ty: TypeId,
    pub name: Option<u32>,
    pub symbol: Option<u32>,
}

/// Phi function under construction.
#[derive(Debug, Clone, PartialEq)]
pub struct PhiNode {
    pub block: BlockId,
    pub var: VarId,
    pub incoming: Vec<(BlockId, Operand)>,
    /// Phis that use this phi as an operand.
    users: Vec<ValueId>,
}

/// Pending step of the iterative read.
enum Frame {
    Read {
        block: BlockId,
    },
    /// Records the value read from the single predecessor of `block` in `block`.
    AfterSinglePred {
        block: BlockId,
    },
    /// Reads the operand of `phi` for predecessor `next` of `block`; the value read for
    /// predecessor `next - 1` is the pending result.
    Operands {
        phi: ValueId,
        block: BlockId,
        next: usize,
        write_back: bool,
    },
}

#[derive(Debug, Clone, Default)]
pub struct SsaBuilder {
    vars: Vec<VarInfo>,
    /// `defs[b][var]`: value of `var` at the end of block `b`.
    defs: Vec<HashMap<VarId, Operand>>,
    sealed: Vec<bool>,
    incomplete: Vec<Vec<(VarId, ValueId)>>,
    phis: HashMap<ValueId, PhiNode>,
    /// Creation order of phis, for deterministic materialisation.
    phi_order: Vec<ValueId>,
    /// Removed phis and their replacement.
    forward: HashMap<ValueId, Operand>,
}

impl SsaBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Makes block `b` known to the builder.
    pub fn ensure_block(&mut self, b: BlockId) {
        let need = b.index() + 1;
        if self.defs.len() < need {
            self.defs.resize_with(need, HashMap::new);
            self.sealed.resize(need, false);
            self.incomplete.resize_with(need, Vec::new);
        }
    }

    pub fn declare_var(&mut self, info: VarInfo) -> VarId {
        let id = VarId(self.vars.len() as u32);
        self.vars.push(info);
        id
    }

    pub fn var_info(&self, var: VarId) -> VarInfo {
        self.vars[var.index()]
    }

    pub fn var_count(&self) -> usize {
        self.vars.len()
    }

    #[inline]
    pub fn is_sealed(&self, b: BlockId) -> bool {
        self.sealed.get(b.index()).copied().unwrap_or(false)
    }

    /// `var` holds `value` at the current end of `block`.
    pub fn write(&mut self, var: VarId, block: BlockId, value: Operand) {
        self.ensure_block(block);
        self.defs[block.index()].insert(var, value);
    }

    /// Value of `var` at the current end of `block`.
    pub fn read(
        &mut self,
        var: VarId,
        block: BlockId,
        preds: &[Vec<BlockId>],
        values: &mut Vec<ValueInfo>,
    ) -> Operand {
        self.run(var, vec![Frame::Read { block }], preds, values)
    }

    /// Declares that every predecessor of `block` is known and completes its incomplete
    /// phis.
    pub fn seal(&mut self, block: BlockId, preds: &[Vec<BlockId>], values: &mut Vec<ValueInfo>) {
        self.ensure_block(block);
        if self.sealed[block.index()] {
            return;
        }
        let pending = std::mem::take(&mut self.incomplete[block.index()]);
        // Sealing first lets the operand reads of the completed phis see a sealed block.
        self.sealed[block.index()] = true;
        for (var, phi) in pending {
            self.run(
                var,
                vec![Frame::Operands {
                    phi,
                    block,
                    next: 0,
                    write_back: false,
                }],
                preds,
                values,
            );
        }
    }

    fn run(
        &mut self,
        var: VarId,
        mut stack: Vec<Frame>,
        preds: &[Vec<BlockId>],
        values: &mut Vec<ValueInfo>,
    ) -> Operand {
        let mut result: Option<Operand> = None;
        while let Some(frame) = stack.pop() {
            match frame {
                Frame::Read { block } => {
                    self.ensure_block(block);
                    if let Some(op) = self.defs[block.index()].get(&var).copied() {
                        result = Some(self.resolve(op));
                        continue;
                    }
                    if !self.sealed[block.index()] {
                        let phi = self.new_phi(var, block, values);
                        self.incomplete[block.index()].push((var, phi));
                        self.write(var, block, Operand::Value(phi));
                        result = Some(Operand::Value(phi));
                        continue;
                    }
                    let block_preds = preds.get(block.index()).map(|p| p.as_slice()).unwrap_or(&[]);
                    match block_preds.len() {
                        0 => {
                            // Function entry or a block no path reaches: no definition.
                            self.write(var, block, Operand::Undef);
                            result = Some(Operand::Undef);
                        }
                        1 => {
                            stack.push(Frame::AfterSinglePred { block });
                            stack.push(Frame::Read {
                                block: block_preds[0],
                            });
                            result = None;
                        }
                        _ => {
                            // The phi is recorded before its operands are read, so a cycle
                            // through this block terminates on it.
                            let phi = self.new_phi(var, block, values);
                            self.write(var, block, Operand::Value(phi));
                            stack.push(Frame::Operands {
                                phi,
                                block,
                                next: 0,
                                write_back: true,
                            });
                            result = None;
                        }
                    }
                }
                Frame::AfterSinglePred { block } => {
                    let value = self.resolve(result.unwrap_or(Operand::Undef));
                    self.write(var, block, value);
                    result = Some(value);
                }
                Frame::Operands {
                    phi,
                    block,
                    next,
                    write_back,
                } => {
                    let block_preds = preds.get(block.index()).map(|p| p.as_slice()).unwrap_or(&[]);
                    if next > 0 {
                        let op = result.take().unwrap_or(Operand::Undef);
                        self.add_incoming(phi, block_preds[next - 1], op);
                    }
                    if next < block_preds.len() {
                        stack.push(Frame::Operands {
                            phi,
                            block,
                            next: next + 1,
                            write_back,
                        });
                        stack.push(Frame::Read {
                            block: block_preds[next],
                        });
                        result = None;
                    } else {
                        let value = self.try_remove_trivial(phi);
                        if write_back {
                            self.write(var, block, value);
                        }
                        result = Some(value);
                    }
                }
            }
        }
        result.map(|op| self.resolve(op)).unwrap_or(Operand::Undef)
    }

    fn new_phi(&mut self, var: VarId, block: BlockId, values: &mut Vec<ValueInfo>) -> ValueId {
        let info = self.vars[var.index()];
        let id = ValueId::new(values.len());
        values.push(ValueInfo {
            ty: info.ty,
            name: info.name,
            symbol: info.symbol,
        });
        self.phis.insert(
            id,
            PhiNode {
                block,
                var,
                incoming: Vec::new(),
                users: Vec::new(),
            },
        );
        self.phi_order.push(id);
        id
    }

    fn add_incoming(&mut self, phi: ValueId, pred: BlockId, op: Operand) {
        let op = self.resolve(op);
        if let Operand::Value(v) = op {
            if v != phi {
                if let Some(node) = self.phis.get_mut(&v) {
                    if !node.users.contains(&phi) {
                        node.users.push(phi);
                    }
                }
            }
        }
        if let Some(node) = self.phis.get_mut(&phi) {
            node.incoming.push((pred, op));
        }
    }

    /// Replaces `phi` (and, transitively, the phis that become trivial because of it) by
    /// its unique non-self operand. Returns what `phi` now stands for.
    fn try_remove_trivial(&mut self, phi: ValueId) -> Operand {
        let mut work = vec![phi];
        while let Some(p) = work.pop() {
            let same = match self.phis.get(&p) {
                Some(node) if self.is_sealed(node.block) => self.unique_operand(p, node),
                _ => continue,
            };
            let Some(same) = same else { continue };
            let node = match self.phis.remove(&p) {
                Some(n) => n,
                None => continue,
            };
            self.forward.insert(p, same);
            for u in node.users {
                if u == p || !self.phis.contains_key(&u) {
                    continue;
                }
                if let Operand::Value(s) = same {
                    if let Some(sn) = self.phis.get_mut(&s) {
                        if s != u && !sn.users.contains(&u) {
                            sn.users.push(u);
                        }
                    }
                }
                work.push(u);
            }
        }
        self.resolve(Operand::Value(phi))
    }

    /// `Some(v)` when every operand of `node` other than `p` itself resolves to `v`
    /// (`Undef` when there is none); `None` when the phi merges distinct values.
    fn unique_operand(&self, p: ValueId, node: &PhiNode) -> Option<Operand> {
        let mut same: Option<Operand> = None;
        for &(_, op) in &node.incoming {
            let op = self.resolve(op);
            if op == Operand::Value(p) || Some(op) == same {
                continue;
            }
            if same.is_some() {
                return None;
            }
            same = Some(op);
        }
        Some(same.unwrap_or(Operand::Undef))
    }

    /// Follows the forwarding of removed phis.
    pub fn resolve(&self, op: Operand) -> Operand {
        let mut cur = op;
        let mut steps = 0usize;
        while let Operand::Value(v) = cur {
            match self.forward.get(&v) {
                Some(&next) => {
                    cur = next;
                    steps += 1;
                    if steps > self.forward.len() {
                        break;
                    }
                }
                None => break,
            }
        }
        cur
    }

    /// True when `v` is a phi that is still present.
    pub fn is_live_phi(&self, v: ValueId) -> bool {
        self.phis.contains_key(&v)
    }

    pub fn phi(&self, v: ValueId) -> Option<&PhiNode> {
        self.phis.get(&v)
    }

    /// Present phis in creation order.
    pub fn live_phis(&self) -> Vec<ValueId> {
        self.phi_order
            .iter()
            .copied()
            .filter(|v| self.phis.contains_key(v))
            .collect()
    }

    /// True when `v` was a phi that construction removed.
    pub fn is_forwarded(&self, v: ValueId) -> bool {
        self.forward.contains_key(&v)
    }

    /// Drops the phi operands that flow in from blocks `keep` rejects (unreachable
    /// predecessors), then removes the phis that become trivial.
    pub fn retain_incoming(&mut self, keep: &dyn Fn(BlockId) -> bool) {
        for node in self.phis.values_mut() {
            node.incoming.retain(|(b, _)| keep(*b));
        }
        self.remove_all_trivial();
    }

    /// Removes every trivial phi (to a fixpoint).
    pub fn remove_all_trivial(&mut self) {
        for p in self.live_phis() {
            self.try_remove_trivial(p);
        }
    }

    /// Algorithm 5 of Braun et al.: a strongly connected set of phis that references a
    /// single value from outside is redundant and is replaced by that value. Inner phis of
    /// larger components are re-examined, since a nested cycle may itself be redundant.
    pub fn remove_redundant_phi_sccs(&mut self) {
        let mut pending: Vec<Vec<ValueId>> = vec![self.live_phis()];
        while let Some(set) = pending.pop() {
            let members: HashSet<ValueId> = set
                .iter()
                .copied()
                .filter(|v| self.phis.contains_key(v))
                .collect();
            if members.is_empty() {
                continue;
            }
            for scc in self.phi_sccs(&set, &members) {
                if scc.len() < 2 {
                    continue;
                }
                let in_scc: HashSet<ValueId> = scc.iter().copied().collect();
                let mut outer: Vec<Operand> = Vec::new();
                let mut inner: Vec<ValueId> = Vec::new();
                for &p in &scc {
                    let Some(node) = self.phis.get(&p) else {
                        continue;
                    };
                    let mut is_inner = true;
                    for &(_, op) in &node.incoming {
                        let op = self.resolve(op);
                        let inside = matches!(op, Operand::Value(v) if in_scc.contains(&v));
                        if !inside {
                            is_inner = false;
                            if !outer.contains(&op) {
                                outer.push(op);
                            }
                        }
                    }
                    if is_inner {
                        inner.push(p);
                    }
                }
                if outer.len() == 1 {
                    let value = outer[0];
                    for &p in &scc {
                        if let Some(node) = self.phis.remove(&p) {
                            self.forward.insert(p, value);
                            for u in node.users {
                                if let Operand::Value(s) = value {
                                    if let Some(sn) = self.phis.get_mut(&s) {
                                        if s != u && !sn.users.contains(&u) {
                                            sn.users.push(u);
                                        }
                                    }
                                }
                            }
                        }
                    }
                } else if outer.len() > 1 && !inner.is_empty() && inner.len() < scc.len() {
                    pending.push(inner);
                }
            }
        }
        self.remove_all_trivial();
    }

    /// Tarjan's SCCs of the operand graph induced by `members`, operands before users.
    fn phi_sccs(&self, order: &[ValueId], members: &HashSet<ValueId>) -> Vec<Vec<ValueId>> {
        let succs = |v: ValueId| -> Vec<ValueId> {
            let mut out = Vec::new();
            if let Some(node) = self.phis.get(&v) {
                for &(_, op) in &node.incoming {
                    if let Operand::Value(w) = self.resolve(op) {
                        if members.contains(&w) && !out.contains(&w) {
                            out.push(w);
                        }
                    }
                }
            }
            out
        };

        let mut index_of: HashMap<ValueId, u32> = HashMap::new();
        let mut low: HashMap<ValueId, u32> = HashMap::new();
        let mut on_stack: HashSet<ValueId> = HashSet::new();
        let mut stack: Vec<ValueId> = Vec::new();
        let mut next_index = 0u32;
        let mut out: Vec<Vec<ValueId>> = Vec::new();

        for &root in order {
            if !members.contains(&root) || index_of.contains_key(&root) {
                continue;
            }
            // (node, successors, next successor position)
            let mut call: Vec<(ValueId, Vec<ValueId>, usize)> = Vec::new();
            index_of.insert(root, next_index);
            low.insert(root, next_index);
            next_index += 1;
            stack.push(root);
            on_stack.insert(root);
            call.push((root, succs(root), 0));

            while let Some(top) = call.last_mut() {
                let v = top.0;
                if top.2 < top.1.len() {
                    let w = top.1[top.2];
                    top.2 += 1;
                    if !index_of.contains_key(&w) {
                        index_of.insert(w, next_index);
                        low.insert(w, next_index);
                        next_index += 1;
                        stack.push(w);
                        on_stack.insert(w);
                        let ws = succs(w);
                        call.push((w, ws, 0));
                    } else if on_stack.contains(&w) {
                        let lw = index_of[&w];
                        let lv = low[&v];
                        low.insert(v, lv.min(lw));
                    }
                } else {
                    call.pop();
                    if let Some(parent) = call.last() {
                        let lp = low[&parent.0];
                        let lv = low[&v];
                        low.insert(parent.0, lp.min(lv));
                    }
                    if low[&v] == index_of[&v] {
                        let mut scc = Vec::new();
                        while let Some(w) = stack.pop() {
                            on_stack.remove(&w);
                            scc.push(w);
                            if w == v {
                                break;
                            }
                        }
                        out.push(scc);
                    }
                }
            }
        }
        out
    }
}
