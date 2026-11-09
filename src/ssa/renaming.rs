//! Variable Renaming Algorithm (Phase B) (§5.2.3 & §5.5.2).
//! Authored by Ahmad Hassan (B-Ted).
//!
//! Dominator-tree pre-order walk with an explicit stack (the tree can be as deep as the
//! number of sequential branches, so recursion is not an option). Parameters and captured
//! variables are defined on entry to block 0; φ-functions are defined at the head of
//! their block; every statement's events are processed in evaluation order.

use crate::ast::BPASTArtifact;
use crate::cfg::analysis::DominatorTree;
use crate::cfg::ast_query::subtree_end;
use crate::cfg::builder::FunctionCFGData;
use crate::core::types::ssa::{PhiArg, PhiRecord, SSARecord, SSA_FLAG_IS_PARAM_DEF, SSA_FLAG_IS_PHI};
use crate::ssa::liveness::BlockEvents;
use crate::ssa::names::{DefKind, FunctionVars, VarEvent};
use crate::ssa::version_stack::VersionStack;

/// Marker stored in `use_adj` for a use by a φ-function (the use has no statement).
pub const PHI_USE: u32 = u32::MAX;

pub struct RenamingResult {
    pub ssa_records: Vec<SSARecord>,
    /// Untruncated defining block of each SSA id (`SSARecord::def_block` keeps 8 bits).
    pub def_blocks: Vec<u32>,
    pub phi_records: Vec<PhiRecord>,
    /// Def-use CSR: for SSA id `d`, `use_adj[def_offsets[d]..def_offsets[d+1]]` are the
    /// statements using it (`PHI_USE` for φ arguments).
    pub def_offsets: Vec<u32>,
    pub use_adj: Vec<u32>,
    /// Block of each `use_adj` entry (the predecessor block for φ arguments).
    pub use_blocks: Vec<u32>,
    /// Value expression of each SSA id (`u32::MAX` for φ, entry and value-less defs).
    pub value_nodes: Vec<u32>,
    /// How each SSA id is defined (`None` for φ-functions).
    pub def_kinds: Vec<Option<DefKind>>,
    /// Operand CSR: SSA ids read to compute each definition (φ arguments for φs).
    pub operand_offsets: Vec<u32>,
    pub operand_adj: Vec<u32>,
}

struct Builder {
    records: Vec<SSARecord>,
    def_blocks: Vec<u32>,
    value_nodes: Vec<u32>,
    def_kinds: Vec<Option<DefKind>>,
    operands: Vec<Vec<u32>>,
    uses: Vec<Vec<(u32, u32)>>,
    versions: Vec<u32>,
}

impl Builder {
    #[allow(clippy::too_many_arguments)]
    fn new_def(
        &mut self,
        var: u32,
        orig: u32,
        def_stmt: u32,
        block: u32,
        flags: u8,
        value: u32,
        kind: Option<DefKind>,
        operands: Vec<u32>,
    ) -> Result<u32, String> {
        let id = u32::try_from(self.records.len())
            .map_err(|_| "SSA id space exceeds the u32 range".to_string())?;
        let counter = &mut self.versions[var as usize];
        let version = u16::try_from(*counter).map_err(|_| {
            format!(
                "variable {} has more than {} SSA versions (SSARecord.version is u16)",
                orig,
                u16::MAX
            )
        })?;
        *counter += 1;
        self.records
            .push(SSARecord::new(id, orig, def_stmt, version, flags, block));
        self.def_blocks.push(block);
        self.value_nodes.push(value);
        self.def_kinds.push(kind);
        self.operands.push(operands);
        self.uses.push(Vec::new());
        Ok(id)
    }
}

enum Frame {
    Enter(u32),
    Exit(usize),
}

pub fn rename_function(
    cfg: &FunctionCFGData,
    bpa: &BPASTArtifact,
    vars: &FunctionVars,
    events: &BlockEvents,
    phis: &[Vec<u32>],
) -> Result<RenamingResult, String> {
    let n = cfg.blocks.len();
    let var_count = vars.var_count();
    let mut b = Builder {
        records: Vec::new(),
        def_blocks: Vec::new(),
        value_nodes: Vec::new(),
        def_kinds: Vec::new(),
        operands: Vec::new(),
        uses: Vec::new(),
        versions: vec![0u32; var_count],
    };

    let mut phi_ids: Vec<Vec<u32>> = phis.iter().map(|l| vec![u32::MAX; l.len()]).collect();
    let mut phi_args: Vec<Vec<Vec<PhiArg>>> =
        phis.iter().map(|l| vec![Vec::new(); l.len()]).collect();

    let dom = DominatorTree::new(&cfg.idom, 0);
    let mut vstack = VersionStack::new(var_count);
    let mut stack: Vec<Frame> = if n > 0 { vec![Frame::Enter(0)] } else { Vec::new() };
    let mut stmt_uses: Vec<(u32, u32)> = Vec::new();

    while let Some(frame) = stack.pop() {
        let blk = match frame {
            Frame::Exit(mark) => {
                vstack.restore_to(mark);
                continue;
            }
            Frame::Enter(blk) => blk,
        };
        let bi = blk as usize;
        stack.push(Frame::Exit(vstack.mark()));

        // ── Part 0: definitions on entry ──
        if blk == 0 {
            for (var, def_node, kind) in vars.entry_defs() {
                let id = b.new_def(
                    var,
                    vars.orig_id(var),
                    def_node,
                    0,
                    SSA_FLAG_IS_PARAM_DEF,
                    u32::MAX,
                    Some(kind),
                    Vec::new(),
                )?;
                vstack.push(var, id);
            }
        }

        // ── Part 1: φ-functions at the head of the block ──
        if let Some(list) = phis.get(bi) {
            for (i, &var) in list.iter().enumerate() {
                let id = b.new_def(
                    var,
                    vars.orig_id(var),
                    u32::MAX,
                    blk,
                    SSA_FLAG_IS_PHI,
                    u32::MAX,
                    None,
                    Vec::new(),
                )?;
                phi_ids[bi][i] = id;
                vstack.push(var, id);
            }
        }

        // ── Part 2: statements, events in evaluation order ──
        if let Some(stmts) = events.stmts.get(bi) {
            for &(stmt, s, e) in stmts {
                stmt_uses.clear();
                for ev in events.of_stmt(s, e) {
                    match *ev {
                        VarEvent::Use { var, node } => {
                            let cur = vstack.top(var);
                            if cur != u32::MAX {
                                b.uses[cur as usize].push((stmt, blk));
                                stmt_uses.push((node, cur));
                            }
                        }
                        VarEvent::Def {
                            var,
                            node: _,
                            value,
                            kind,
                        } => {
                            let mut ops: Vec<u32> = Vec::new();
                            if matches!(kind, DefKind::Compound | DefKind::Update) {
                                let prev = vstack.top(var);
                                if prev != u32::MAX {
                                    ops.push(prev);
                                }
                            }
                            if value != u32::MAX {
                                let end = subtree_end(bpa, value);
                                for &(use_node, id) in &stmt_uses {
                                    if use_node >= value && use_node < end && !ops.contains(&id) {
                                        ops.push(id);
                                    }
                                }
                            }
                            let id = b.new_def(
                                var,
                                vars.orig_id(var),
                                stmt,
                                blk,
                                0,
                                value,
                                Some(kind),
                                ops,
                            )?;
                            vstack.push(var, id);
                        }
                    }
                }
            }
        }

        // ── Part 3: φ arguments in CFG successors (one per incoming edge) ──
        for &succ in cfg.succs_of(blk) {
            let si = succ as usize;
            if let Some(list) = phis.get(si) {
                for (i, &var) in list.iter().enumerate() {
                    let cur = vstack.top(var);
                    if cur != u32::MAX {
                        b.uses[cur as usize].push((PHI_USE, blk));
                    }
                    phi_args[si][i].push(PhiArg {
                        pred_block_id: blk,
                        arg_ssa_id: cur,
                    });
                }
            }
        }

        // ── Part 4: dominator-tree children (pushed reversed to visit in order) ──
        if let Some(children) = dom.children.get(bi) {
            for &c in children.iter().rev() {
                stack.push(Frame::Enter(c));
            }
        }
    }

    // ── φ records and φ operands ──
    let mut phi_records = Vec::new();
    for (bi, list) in phis.iter().enumerate() {
        for (i, &var) in list.iter().enumerate() {
            let id = phi_ids[bi][i];
            if id == u32::MAX {
                // Block not in the dominator tree (unreachable): no φ was materialised.
                continue;
            }
            let args = std::mem::take(&mut phi_args[bi][i]);
            b.operands[id as usize] = args
                .iter()
                .map(|a| a.arg_ssa_id)
                .filter(|&a| a != u32::MAX)
                .collect();
            phi_records.push(PhiRecord::new(id, bi as u32, vars.orig_id(var), args));
        }
    }

    // ── CSRs ──
    let total = b.records.len();
    let mut def_offsets = Vec::with_capacity(total + 1);
    let mut use_adj = Vec::new();
    let mut use_blocks = Vec::new();
    def_offsets.push(0u32);
    for list in &b.uses {
        for &(stmt, blk) in list {
            use_adj.push(stmt);
            use_blocks.push(blk);
        }
        def_offsets.push(
            u32::try_from(use_adj.len()).map_err(|_| "def-use CSR exceeds the u32 range")?,
        );
    }

    let mut operand_offsets = Vec::with_capacity(total + 1);
    let mut operand_adj = Vec::new();
    operand_offsets.push(0u32);
    for list in &b.operands {
        operand_adj.extend_from_slice(list);
        operand_offsets.push(
            u32::try_from(operand_adj.len()).map_err(|_| "operand CSR exceeds the u32 range")?,
        );
    }

    if crate::core::logger::get_log_level() >= crate::core::logger::LogLevel::Trace {
        crate::core::logger::log_trace(&format!(
            "Variable Renaming complete: generated {} SSA records, {} Phi records, {} total uses in DefUseCSR",
            b.records.len(),
            phi_records.len(),
            use_adj.len()
        ));
    }

    Ok(RenamingResult {
        ssa_records: b.records,
        def_blocks: b.def_blocks,
        phi_records,
        def_offsets,
        use_adj,
        use_blocks,
        value_nodes: b.value_nodes,
        def_kinds: b.def_kinds,
        operand_offsets,
        operand_adj,
    })
}
