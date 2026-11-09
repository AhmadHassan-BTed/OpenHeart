//! CFGBuilderState: maintains live state during construction of a single function CFG.
//!
//! Construction protocol, obeyed by every statement builder:
//! * on entry, `current_block` is the block in which control currently resides;
//! * every edge is wired explicitly when it becomes known — there is no deferred
//!   "pending exit" list, so a statement can never leave a reachable block dangling;
//! * on exit, `current_block` is the block where normal completion continues. When the
//!   statement cannot complete normally (`return`, `throw`, `break`, `continue`,
//!   `yield`), the builder leaves a fresh block without predecessors as the current
//!   block; whatever is built into it is unreachable and pruned at finalisation;
//! * abrupt completions are routed by [`CFGBuilderState::route_jump`], which walks the
//!   enclosing control frames innermost-first, so matching catch clauses and every
//!   enclosing `finally` / `synchronized` release are entered before the jump reaches its
//!   target. A `finally` body is built once and records the jumps that entered it
//!   ("dispatch block" lowering); after the body, each recorded jump continues outward.

use crate::ast::BPASTArtifact;
use crate::cfg::ast_query::{is_nested_body, operator_id, subtree_end};
use crate::cfg::builder::exceptions::{CatchMatch, CatchType, ExceptionTypes, ThrownType};
use crate::cfg::source::SourceText;
use crate::core::types::ast::{ASTNodeType, OperatorId};
use crate::core::types::cfg::{BasicBlock, CFGEdgeType};
use std::collections::HashSet;

/// A statement label: the label identifier node and its interned text (when known).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LabelRef {
    pub node: u32,
    pub text_id: Option<u32>,
}

/// One catch clause of an active `try`.
#[derive(Debug, Clone)]
pub struct Handler {
    pub block: u32,
    /// Alternatives of a (multi-)catch.
    pub types: Vec<CatchType>,
}

/// An abrupt completion. Frame indices refer to `CFGBuilderState::control`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Jump {
    Return,
    Throw(ThrownType),
    Break(usize),
    Continue(usize),
    Yield(usize),
}

#[derive(Debug, Clone)]
pub enum FrameKind {
    Loop {
        break_target: u32,
        continue_target: u32,
        continue_edge: CFGEdgeType,
    },
    Switch {
        break_target: u32,
    },
    SwitchExpr {
        yield_target: u32,
    },
    /// A labeled statement that is not itself a loop or switch (`l: { ... break l; }`).
    Labeled {
        break_target: u32,
    },
    /// A `try` statement, a try-with-resources resource scope, or a `synchronized` body.
    Protected {
        handlers: Vec<Handler>,
        /// False while the catch bodies of the same `try` are being built: exceptions
        /// raised by a catch body are not caught by its sibling handlers.
        handlers_active: bool,
        finally_entry: Option<u32>,
        /// Jumps that entered `finally_entry` and must continue after it.
        continuations: Vec<Jump>,
    },
}

#[derive(Debug, Clone)]
pub struct ControlFrame {
    pub kind: FrameKind,
    pub labels: Vec<LabelRef>,
}

/// Places where the CFG had to fall back to an approximation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum CfgDiagnosticKind {
    /// `for` header roles inferred from child positions (no token text).
    ForHeaderApproximated = 1,
    /// Enhanced-for roles inferred from child positions (no token text).
    ForEachHeaderApproximated = 2,
    /// Arrow rule vs. statement group of a switch case could not be determined.
    SwitchRuleKindUnknown = 3,
    /// `break` / `continue` target not found; the statement was treated as a no-op.
    UnresolvedJumpTarget = 4,
    /// Several labels in scope and no token text to tell them apart.
    AmbiguousLabel = 5,
    /// `yield` outside any switch expression.
    YieldOutsideSwitchExpression = 6,
    /// Cyclomatic complexity exceeded u16 and was saturated.
    CyclomaticSaturated = 7,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CfgDiagnostic {
    pub kind: CfgDiagnosticKind,
    pub node: u32,
}

pub struct CFGBuilderState<'a> {
    pub blocks: Vec<BasicBlock>,
    pub current_block: u32,
    pub edges: Vec<(u32, u32, CFGEdgeType)>,
    edge_set: HashSet<(u32, u32, u8)>,
    succs: Vec<Vec<u32>>,
    reachable: Vec<bool>,
    pub exit_block: u32,
    pub control: Vec<ControlFrame>,
    /// Labels of enclosing labeled statements not yet claimed by a frame.
    pub pending_labels: Vec<LabelRef>,
    /// Expression nodes lowered into their own blocks (see `FunctionCFGData::lowered_nodes`).
    pub lowered: HashSet<u32>,
    pub diagnostics: Vec<CfgDiagnostic>,
    /// Number of catch clauses built (each is a decision point for V(G)).
    pub catch_clauses: u32,
    pub src: Option<SourceText<'a>>,
    pub exceptions: &'a ExceptionTypes<'a>,
}

#[inline]
fn combine(a: CatchMatch, b: CatchMatch) -> CatchMatch {
    match (a, b) {
        (CatchMatch::Definite, _) | (_, CatchMatch::Definite) => CatchMatch::Definite,
        (CatchMatch::Possible, _) | (_, CatchMatch::Possible) => CatchMatch::Possible,
        _ => CatchMatch::Never,
    }
}

impl<'a> CFGBuilderState<'a> {
    pub fn new(src: Option<SourceText<'a>>, exceptions: &'a ExceptionTypes<'a>) -> Self {
        Self {
            blocks: Vec::new(),
            current_block: 0,
            edges: Vec::new(),
            edge_set: HashSet::new(),
            succs: Vec::new(),
            reachable: Vec::new(),
            exit_block: 0,
            control: Vec::new(),
            pending_labels: Vec::new(),
            lowered: HashSet::new(),
            diagnostics: Vec::new(),
            catch_clauses: 0,
            src,
            exceptions,
        }
    }

    pub fn new_block(&mut self) -> u32 {
        // Block ids are bounded by the number of BP AST nodes of one function body, which
        // is itself a u32 index space.
        let id = self.blocks.len() as u32;
        self.blocks.push(BasicBlock::new(id));
        self.succs.push(Vec::new());
        self.reachable.push(false);
        id
    }

    /// Marks `b` (the ENTRY block) as reachable.
    pub fn mark_root(&mut self, b: u32) {
        self.propagate_reachable(b);
    }

    #[inline]
    pub fn is_reachable(&self, b: u32) -> bool {
        self.reachable.get(b as usize).copied().unwrap_or(false)
    }

    fn propagate_reachable(&mut self, start: u32) {
        let s = start as usize;
        if s >= self.reachable.len() || self.reachable[s] {
            return;
        }
        self.reachable[s] = true;
        let mut stack = vec![start];
        while let Some(b) = stack.pop() {
            for i in 0..self.succs[b as usize].len() {
                let t = self.succs[b as usize][i];
                if !self.reachable[t as usize] {
                    self.reachable[t as usize] = true;
                    stack.push(t);
                }
            }
        }
    }

    /// Adds `from -> to`; exact duplicates (same endpoints and type) are ignored.
    pub fn add_edge(&mut self, from: u32, to: u32, edge_type: CFGEdgeType) {
        let n = self.blocks.len() as u32;
        if from >= n || to >= n {
            return;
        }
        if !self.edge_set.insert((from, to, edge_type as u8)) {
            return;
        }
        self.edges.push((from, to, edge_type));
        self.succs[from as usize].push(to);
        if self.reachable[from as usize] {
            self.propagate_reachable(to);
        }
    }

    /// Edge from the current block to `target`, which becomes the current block.
    pub fn goto(&mut self, target: u32, edge_type: CFGEdgeType) {
        let from = self.current_block;
        self.add_edge(from, target, edge_type);
        self.current_block = target;
    }

    /// Continues in a fresh block with no predecessors (after an abrupt completion).
    pub fn terminate(&mut self) {
        self.current_block = self.new_block();
    }

    fn append_stmt(&mut self, block: u32, node: u32, bpa: &BPASTArtifact) {
        let b = block as usize;
        if b < self.blocks.len() {
            self.blocks[b].stmts.push(node);
            let (ft, lt) = bpa.token_range(node);
            if ft != u32::MAX {
                self.blocks[b].first_token = self.blocks[b].first_token.min(ft);
                self.blocks[b].last_token = self.blocks[b].last_token.max(lt);
            }
        }
    }

    /// Appends a statement to the current block. Inside a protected region a statement
    /// that may raise an implicit exception ends its block: the block gets exceptional
    /// edges to the handlers that may catch it and falls through to a new block.
    pub fn add_stmt(&mut self, node: u32, bpa: &BPASTArtifact) {
        let cur = self.current_block;
        self.append_stmt(cur, node, bpa);
        if self.in_protected_region() && self.may_throw(node, bpa) {
            self.route_jump(cur, Jump::Throw(ThrownType::Unknown));
            let next = self.new_block();
            self.add_edge(cur, next, CFGEdgeType::Uncond);
            self.current_block = next;
        }
    }

    /// Appends a statement whose exceptional behaviour the caller routes itself.
    pub fn add_stmt_no_implicit(&mut self, node: u32, bpa: &BPASTArtifact) {
        let cur = self.current_block;
        self.append_stmt(cur, node, bpa);
    }

    /// Potentially-excepting evaluation (JLS 11.2/15): invocations, instance and array
    /// creation, array access, casts, integer division and explicit throws. Nested
    /// lambda / class bodies and expressions lowered into other blocks are skipped.
    pub fn may_throw(&self, node: u32, bpa: &BPASTArtifact) -> bool {
        use ASTNodeType::*;
        let end = subtree_end(bpa, node);
        let mut k = node;
        while k < end {
            let t = bpa.node_type(k);
            if k != node && (is_nested_body(t) || self.lowered.contains(&k)) {
                k = subtree_end(bpa, k);
                continue;
            }
            match t {
                NN_CALL_EXPR | NN_NEW_EXPR | NN_ARRAY_ACCESS | NN_ARRAY_CREATE | NN_CAST_EXPR
                | NN_THROW_STMT => return true,
                NN_BINARY_EXPR if operator_id(bpa, k) == OperatorId::Div as u8 => return true,
                _ => {}
            }
            k += 1;
        }
        false
    }

    pub fn mark_lowered(&mut self, node: u32) {
        self.lowered.insert(node);
    }

    pub fn take_labels(&mut self) -> Vec<LabelRef> {
        std::mem::take(&mut self.pending_labels)
    }

    pub fn push_frame(&mut self, kind: FrameKind, labels: Vec<LabelRef>) -> usize {
        self.control.push(ControlFrame { kind, labels });
        self.control.len() - 1
    }

    pub fn pop_frame(&mut self) -> Option<ControlFrame> {
        self.control.pop()
    }

    pub fn diag(&mut self, kind: CfgDiagnosticKind, node: u32) {
        self.diagnostics.push(CfgDiagnostic { kind, node });
    }

    /// Label of a labeled statement / labeled jump.
    pub fn label_ref(&self, bpa: &BPASTArtifact, label_node: u32) -> LabelRef {
        LabelRef {
            node: label_node,
            text_id: self
                .src
                .as_ref()
                .and_then(|s| s.node_text_id(bpa, label_node)),
        }
    }

    /// True when an implicit exception raised here could reach a handler or `finally`.
    pub fn in_protected_region(&self) -> bool {
        self.control.iter().any(|f| match &f.kind {
            FrameKind::Protected {
                handlers,
                handlers_active,
                finally_entry,
                ..
            } => (*handlers_active && !handlers.is_empty()) || finally_entry.is_some(),
            _ => false,
        })
    }

    /// Frames below a switch expression cannot be targeted by `break` / `continue`.
    fn jump_floor(&self) -> usize {
        self.control
            .iter()
            .rposition(|f| matches!(f.kind, FrameKind::SwitchExpr { .. }))
            .map(|i| i + 1)
            .unwrap_or(0)
    }

    fn is_loop(&self, i: usize) -> bool {
        matches!(self.control[i].kind, FrameKind::Loop { .. })
    }

    fn find_labeled(&mut self, label: LabelRef, node: u32, require_loop: bool) -> Option<usize> {
        let floor = self.jump_floor();
        if let Some(text) = label.text_id {
            for i in (floor..self.control.len()).rev() {
                if self.control[i]
                    .labels
                    .iter()
                    .any(|l| l.text_id == Some(text))
                {
                    if require_loop && !self.is_loop(i) {
                        self.diag(CfgDiagnosticKind::UnresolvedJumpTarget, node);
                        return None;
                    }
                    return Some(i);
                }
            }
            self.diag(CfgDiagnosticKind::UnresolvedJumpTarget, node);
            None
        } else {
            let candidates: Vec<usize> = (floor..self.control.len())
                .filter(|&i| !self.control[i].labels.is_empty() && (!require_loop || self.is_loop(i)))
                .collect();
            match candidates.len() {
                0 => {
                    self.diag(CfgDiagnosticKind::UnresolvedJumpTarget, node);
                    None
                }
                1 => Some(candidates[0]),
                _ => {
                    self.diag(CfgDiagnosticKind::AmbiguousLabel, node);
                    candidates.last().copied()
                }
            }
        }
    }

    /// Frame targeted by `break` / `break label`.
    pub fn find_break(&mut self, label: Option<LabelRef>, node: u32) -> Option<usize> {
        match label {
            Some(l) => self.find_labeled(l, node, false),
            None => {
                let floor = self.jump_floor();
                let found = (floor..self.control.len()).rev().find(|&i| {
                    matches!(
                        self.control[i].kind,
                        FrameKind::Loop { .. } | FrameKind::Switch { .. }
                    )
                });
                if found.is_none() {
                    self.diag(CfgDiagnosticKind::UnresolvedJumpTarget, node);
                }
                found
            }
        }
    }

    /// Frame targeted by `continue` / `continue label`.
    pub fn find_continue(&mut self, label: Option<LabelRef>, node: u32) -> Option<usize> {
        match label {
            Some(l) => self.find_labeled(l, node, true),
            None => {
                let floor = self.jump_floor();
                let found = (floor..self.control.len()).rev().find(|&i| self.is_loop(i));
                if found.is_none() {
                    self.diag(CfgDiagnosticKind::UnresolvedJumpTarget, node);
                }
                found
            }
        }
    }

    /// Innermost switch expression (target of `yield`).
    pub fn find_yield(&self) -> Option<usize> {
        self.control
            .iter()
            .rposition(|f| matches!(f.kind, FrameKind::SwitchExpr { .. }))
    }

    /// Routes an abrupt completion starting in block `from` (see module docs).
    pub fn route_jump(&mut self, from: u32, jump: Jump) {
        let floor = match jump {
            Jump::Break(d) | Jump::Continue(d) | Jump::Yield(d) => d + 1,
            Jump::Return | Jump::Throw(_) => 0,
        };
        let entry_edge = match jump {
            Jump::Return => CFGEdgeType::Return,
            Jump::Throw(_) => CFGEdgeType::Except,
            _ => CFGEdgeType::Uncond,
        };

        let mut i = self.control.len();
        while i > floor {
            i -= 1;
            let (handler_targets, finally_entry, caught) = match &self.control[i].kind {
                FrameKind::Protected {
                    handlers,
                    handlers_active,
                    finally_entry,
                    ..
                } => {
                    let mut targets = Vec::new();
                    let mut caught = false;
                    if let (Jump::Throw(thrown), true) = (jump, *handlers_active) {
                        for h in handlers {
                            let m = h
                                .types
                                .iter()
                                .map(|&ct| self.exceptions.relation(thrown, ct))
                                .fold(CatchMatch::Never, combine);
                            match m {
                                CatchMatch::Definite => {
                                    targets.push(h.block);
                                    caught = true;
                                    break;
                                }
                                CatchMatch::Possible => targets.push(h.block),
                                CatchMatch::Never => {}
                            }
                        }
                    }
                    (targets, *finally_entry, caught)
                }
                _ => continue,
            };

            for t in handler_targets {
                self.add_edge(from, t, CFGEdgeType::Except);
            }
            if caught {
                return;
            }
            if let Some(fe) = finally_entry {
                self.add_edge(from, fe, entry_edge);
                if let FrameKind::Protected { continuations, .. } = &mut self.control[i].kind {
                    if !continuations.contains(&jump) {
                        continuations.push(jump);
                    }
                }
                return;
            }
        }

        let exit = self.exit_block;
        match jump {
            Jump::Return => self.add_edge(from, exit, CFGEdgeType::Return),
            Jump::Throw(_) => self.add_edge(from, exit, CFGEdgeType::Except),
            Jump::Break(d) => {
                let target = match self.control.get(d).map(|f| &f.kind) {
                    Some(FrameKind::Loop { break_target, .. })
                    | Some(FrameKind::Switch { break_target })
                    | Some(FrameKind::Labeled { break_target }) => Some(*break_target),
                    _ => None,
                };
                if let Some(t) = target {
                    self.add_edge(from, t, CFGEdgeType::Uncond);
                }
            }
            Jump::Continue(d) => {
                let target = match self.control.get(d).map(|f| &f.kind) {
                    Some(FrameKind::Loop {
                        continue_target,
                        continue_edge,
                        ..
                    }) => Some((*continue_target, *continue_edge)),
                    _ => None,
                };
                if let Some((t, et)) = target {
                    self.add_edge(from, t, et);
                }
            }
            Jump::Yield(d) => {
                let target = match self.control.get(d).map(|f| &f.kind) {
                    Some(FrameKind::SwitchExpr { yield_target }) => Some(*yield_target),
                    _ => None,
                };
                if let Some(t) = target {
                    self.add_edge(from, t, CFGEdgeType::Uncond);
                }
            }
        }
    }

    /// Successor lists of the blocks built so far.
    pub fn succ_lists(&self) -> &[Vec<u32>] {
        &self.succs
    }
}
