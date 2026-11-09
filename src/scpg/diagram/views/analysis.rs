//! Compiler-IR view builders: control flow graph with dominator tree, SSA def-use data
//! flow graph, control dependence graph, call graph with recursive SCCs and ROBDD path
//! summaries (§10.4). Each is rendered from its own artifact (`.cfa`, `.ssa`, `.cga`,
//! `.psa`); without that artifact the view is unavailable.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};

use crate::cfg::builder::FunctionCFGData;
use crate::core::types::cfg::CFGEdgeType;
use crate::core::types::cg::*;
use crate::core::types::ssa::{
    CD_EDGE_FALSE, SSA_FLAG_IS_CONST, SSA_FLAG_IS_FIELD_DEF, SSA_FLAG_IS_PARAM_DEF,
    SSA_FLAG_IS_RETURN_VAL,
};
use crate::psa::bdd::node::{FALSE_ID, FLAG_FALSE_TERMINAL, FLAG_TRUE_TERMINAL, TRUE_ID};
use crate::scpg::diagram::context::DiagramContext;
use crate::scpg::diagram::model::{EdgeKind, NodeKind, ViewEdge, ViewKind, ViewModel, ViewNode};
use crate::scpg::diagram::views::{
    function_fqn, function_label, ordered_functions, FunctionBudget,
};
use crate::ssa::serializer::FunctionSSAData;

const MISSING_CFA: &str =
    "This view needs the Phase 4 CFA artifact (.cfa), which was not provided to the exporter.";
const MISSING_SSA: &str =
    "This view needs the Phase 5 SSA artifact (.ssa), which was not provided to the exporter.";
const MISSING_CGA: &str =
    "This view needs the Phase 6 call-graph artifact (.cga), which was not provided to the exporter.";
const MISSING_PSA: &str =
    "This view needs the Phase 8 path-summary artifact (.psa), which was not provided to the exporter.";

/// Depth of callee expansion around a focused function in the call graph.
const CALLGRAPH_FOCUS_DEPTH: usize = 3;
/// Maximum characters of one statement line inside a node.
const STATEMENT_CHARS: usize = 60;

/// UML-style label of a CFG edge type (`None` for unconditional edges).
pub fn edge_type_label(edge_type: CFGEdgeType) -> Option<&'static str> {
    match edge_type {
        CFGEdgeType::Uncond => None,
        CFGEdgeType::True => Some("true"),
        CFGEdgeType::False => Some("false"),
        CFGEdgeType::Except => Some("exception"),
        CFGEdgeType::Return => Some("return"),
        CFGEdgeType::Switch => Some("case"),
        CFGEdgeType::LoopBack => Some("loop back"),
    }
}

/// Statement texts of a basic block (at most `max_lines`, plus a count of the rest). Falls
/// back to the block's own token span when no statement text is available.
pub fn block_lines(
    ctx: &DiagramContext,
    cfg: &FunctionCFGData,
    block_id: u32,
    max_lines: usize,
) -> Vec<String> {
    let blk = match cfg.blocks.get(block_id as usize) {
        Some(b) => b,
        None => return Vec::new(),
    };
    let mut lines = Vec::new();
    for (idx, &stmt) in blk.stmts.iter().enumerate() {
        if lines.len() >= max_lines {
            lines.push(format!("… +{} more statement(s)", blk.stmts.len() - idx));
            break;
        }
        if let Some(text) = ctx.stmt_text(stmt, STATEMENT_CHARS) {
            lines.push(text);
        }
    }
    if lines.is_empty() && blk.first_token != u32::MAX && blk.last_token >= blk.first_token {
        if let Some(text) = ctx.source_text(blk.first_token, blk.last_token, STATEMENT_CHARS) {
            lines.push(text);
        }
    }
    lines
}

/// Branch condition evaluated at the end of a block (`if`/`while`/`for`/`switch` header).
pub fn block_condition(ctx: &DiagramContext, cfg: &FunctionCFGData, block_id: u32) -> Option<String> {
    let blk = cfg.blocks.get(block_id as usize)?;
    let stmt = *blk.stmts.last()?;
    let (first, last) = ctx.stmt_tokens(stmt)?;
    ctx.condition_text(first, last, 48)
}

fn block_label(cfg: &FunctionCFGData, block_id: u32) -> String {
    let mut label = format!("B{}", block_id);
    if let Some(blk) = cfg.blocks.get(block_id as usize) {
        if blk.is_entry {
            label.push_str(" entry");
        }
        if blk.is_exit {
            label.push_str(" exit");
        }
    }
    label
}

fn block_span(ctx: &DiagramContext, cfg: &FunctionCFGData, block_id: u32) -> Option<crate::scpg::diagram::context::SourceSpan> {
    let blk = cfg.blocks.get(block_id as usize)?;
    if blk.first_token == u32::MAX || blk.last_token < blk.first_token {
        return None;
    }
    ctx.span_of_tokens(blk.first_token, blk.last_token)
}

fn function_group(ctx: &DiagramContext, model: &mut ViewModel, sym: u32, details: Vec<String>) -> String {
    let key = format!("fn:{}", sym);
    model.add_node(
        ViewNode::new(key.clone(), format!("fn_{}", sym), function_label(ctx, sym), NodeKind::Group)
            .with_qualified_name(Some(function_fqn(ctx, sym)))
            .with_span(ctx.symbol_span(sym))
            .with_details(details)
            .with_sym(sym),
    );
    key
}

// ── 15. CONTROL FLOW GRAPH & DOMINATOR TREE ──────────────────────────────────

pub fn cfg_view(ctx: &DiagramContext) -> ViewModel {
    let cfa = match ctx.cfa {
        Some(c) => c,
        None => return ViewModel::unavailable(ViewKind::Cfg, MISSING_CFA),
    };
    if cfa.functions.is_empty() {
        return ViewModel::unavailable(ViewKind::Cfg, "The CFA artifact contains no function bodies.");
    }
    let mut model = ViewModel::new(ViewKind::Cfg);
    model.source("cfa");
    let mut budget = FunctionBudget::new(ctx.options.max_nodes);
    for (fqn, sym) in ordered_functions(ctx, cfa.functions.iter().map(|f| f.sym_id)) {
        let cfg = match ctx.function_cfg(sym) {
            Some(c) => c,
            None => continue,
        };
        if !budget.admit(&fqn, cfg.blocks.len() + 1) {
            continue;
        }
        add_cfg(ctx, &mut model, sym, cfg);
    }
    budget.finish(&mut model, "function(s)");
    if ctx.bpa.is_none() && ctx.tra.is_none() {
        model.note("Per-statement text needs the BPA or TRA artifact; blocks show the text of their token span.");
    }
    model.note("Dashed edges form the dominator tree (immediate dominators); «loop header» marks targets of back edges.");
    model
}

fn add_cfg(ctx: &DiagramContext, model: &mut ViewModel, sym: u32, cfg: &FunctionCFGData) {
    let group = function_group(
        ctx,
        model,
        sym,
        vec![format!(
            "V(G)={} | {} blocks | {} edges | max loop depth {}",
            cfg.cyclomatic,
            cfg.blocks.len(),
            cfg.edges.len(),
            cfg.loop_info.max_loop_depth
        )],
    );
    let n = cfg.blocks.len();
    let headers: HashSet<u32> = cfg.loop_info.back_edges.iter().map(|&(_, h)| h).collect();
    let back: HashSet<(u32, u32)> = cfg.loop_info.back_edges.iter().copied().collect();
    let key = |b: u32| format!("bb:{}:{}", sym, b);

    for b in 0..n as u32 {
        let mut details = block_lines(ctx, cfg, b, 3);
        if let Some(&d) = cfg.idom.get(b as usize) {
            if d != b && (d as usize) < n {
                details.push(format!("idom: B{}", d));
            }
        }
        let mut node = ViewNode::new(key(b), format!("bb_{}_{}", sym, b), block_label(cfg, b), NodeKind::BasicBlock)
            .with_parent(Some(group.clone()))
            .with_span(block_span(ctx, cfg, b))
            .with_details(details);
        if headers.contains(&b) {
            node = node.with_stereotype("loop header");
        }
        model.add_node(node);
    }

    for (idx, &(u, v, etype)) in cfg.edges.iter().enumerate() {
        let kind = if etype == CFGEdgeType::Except {
            EdgeKind::ExceptionFlow
        } else {
            EdgeKind::ControlFlow
        };
        let mut label = edge_type_label(etype).map(str::to_string);
        if back.contains(&(u, v)) && etype != CFGEdgeType::LoopBack {
            label = Some(match label {
                Some(l) => format!("{} (back edge)", l),
                None => "back edge".to_string(),
            });
        }
        model.add_edge(ViewEdge::new(&key(u), &key(v), kind, &idx.to_string()).with_label(label));
    }

    for b in 0..n as u32 {
        if let Some(&d) = cfg.idom.get(b as usize) {
            if d != b && (d as usize) < n {
                model.add_edge(
                    ViewEdge::new(&key(d), &key(b), EdgeKind::DominatorTree, "idom")
                        .with_label(Some("idom".to_string())),
                );
            }
        }
    }
}

// ── 16. SSA DEF-USE DATA FLOW GRAPH ──────────────────────────────────────────

pub fn dfg_view(ctx: &DiagramContext) -> ViewModel {
    let ssa = match ctx.ssa {
        Some(s) => s,
        None => return ViewModel::unavailable(ViewKind::Dfg, MISSING_SSA),
    };
    if ssa.functions.is_empty() {
        return ViewModel::unavailable(ViewKind::Dfg, "The SSA artifact contains no functions.");
    }
    let mut model = ViewModel::new(ViewKind::Dfg);
    model.source("ssa");
    if ctx.cfa.is_some() {
        model.source("cfa");
    }
    let mut budget = FunctionBudget::new(ctx.options.max_nodes);
    for (fqn, sym) in ordered_functions(ctx, ssa.functions.iter().map(|f| f.sym_id)) {
        let func = match ctx.function_ssa(sym) {
            Some(f) => f,
            None => continue,
        };
        if func.ssa_records.is_empty() {
            continue;
        }
        let use_sites: HashSet<u32> = func
            .def_use
            .use_adj
            .iter()
            .copied()
            .filter(|&s| s != u32::MAX)
            .collect();
        if !budget.admit(&fqn, func.ssa_records.len() + use_sites.len() + 1) {
            continue;
        }
        add_dfg(ctx, &mut model, sym, func);
    }
    budget.finish(&mut model, "function(s)");
    if ctx.bpa.is_none() && ctx.tra.is_none() {
        model.note("Statement text needs the BPA or TRA artifact; statements are labelled by AST node id.");
    }
    model.note("Solid edges are def-use chains (value → using statement → values it defines); dashed edges are φ-function arguments.");
    model
}

fn add_dfg(ctx: &DiagramContext, model: &mut ViewModel, sym: u32, func: &FunctionSSAData) {
    let group = function_group(
        ctx,
        model,
        sym,
        vec![format!(
            "{} SSA values | {} φ-functions",
            func.ssa_records.len(),
            func.phi_records.len()
        )],
    );
    let mut stmt_block: HashMap<u32, u32> = HashMap::new();
    if let Some(cfg) = ctx.function_cfg(sym) {
        for blk in &cfg.blocks {
            for &stmt in &blk.stmts {
                stmt_block.entry(stmt).or_insert(blk.id);
            }
        }
    }
    let version_of: HashMap<u32, (u32, u16)> = func
        .ssa_records
        .iter()
        .map(|r| (r.ssa_id, (r.orig_sym_id, r.version)))
        .collect();
    let value_name = |ssa_id: u32| -> String {
        match version_of.get(&ssa_id) {
            Some(&(orig, version)) => format!(
                "{}_{}",
                ctx.simple_name(orig).unwrap_or_else(|| format!("#{}", orig)),
                version
            ),
            None => "⊥".to_string(),
        }
    };
    let value_key = |ssa_id: u32| format!("ssa:{}:{}", sym, ssa_id);
    let phi_by_ssa: HashMap<u32, usize> = func
        .phi_records
        .iter()
        .enumerate()
        .map(|(i, p)| (p.ssa_id, i))
        .collect();

    for rec in &func.ssa_records {
        let is_phi = rec.is_phi();
        let phi = phi_by_ssa.get(&rec.ssa_id).and_then(|&i| func.phi_records.get(i));
        let label = match phi {
            Some(p) if is_phi => {
                let args: Vec<String> = p.args.iter().map(|a| value_name(a.arg_ssa_id)).collect();
                format!("{} = φ({})", value_name(rec.ssa_id), args.join(", "))
            }
            _ => value_name(rec.ssa_id),
        };
        let block = match phi {
            Some(p) if is_phi => p.block_id,
            _ => stmt_block.get(&rec.def_stmt).copied().unwrap_or(rec.def_block),
        };
        let mut details = vec![format!("defined in B{}", block)];
        for (flag, text) in [
            (SSA_FLAG_IS_PARAM_DEF, "parameter"),
            (SSA_FLAG_IS_FIELD_DEF, "field"),
            (SSA_FLAG_IS_RETURN_VAL, "return value"),
            (SSA_FLAG_IS_CONST, "constant"),
        ] {
            if rec.flags & flag != 0 {
                details.push(text.to_string());
            }
        }
        let mut span = None;
        if !is_phi && rec.def_stmt != u32::MAX {
            if let Some(text) = ctx.stmt_text(rec.def_stmt, STATEMENT_CHARS) {
                details.push(format!("def: {}", text));
            }
            span = ctx
                .stmt_tokens(rec.def_stmt)
                .and_then(|(f, l)| ctx.span_of_tokens(f, l));
        }
        model.add_node(
            ViewNode::new(
                value_key(rec.ssa_id),
                format!("ssa_{}_{}", sym, rec.ssa_id),
                label,
                if is_phi { NodeKind::SsaPhi } else { NodeKind::SsaValue },
            )
            .with_parent(Some(group.clone()))
            .with_details(details)
            .with_span(span)
            .with_sym(rec.orig_sym_id),
        );
    }

    let mut defs_by_stmt: HashMap<u32, Vec<u32>> = HashMap::new();
    for rec in &func.ssa_records {
        if !rec.is_phi() && rec.def_stmt != u32::MAX {
            defs_by_stmt.entry(rec.def_stmt).or_default().push(rec.ssa_id);
        }
    }
    for rec in &func.ssa_records {
        for &use_stmt in func.def_use.uses_of(rec.ssa_id) {
            if use_stmt == u32::MAX {
                continue;
            }
            let stmt_key = format!("use:{}:{}", sym, use_stmt);
            if !model.has_node(&stmt_key) {
                let label = ctx
                    .stmt_text(use_stmt, STATEMENT_CHARS)
                    .unwrap_or_else(|| format!("statement #{}", use_stmt));
                let details = stmt_block
                    .get(&use_stmt)
                    .map(|b| vec![format!("in B{}", b)])
                    .unwrap_or_default();
                let span = ctx
                    .stmt_tokens(use_stmt)
                    .and_then(|(f, l)| ctx.span_of_tokens(f, l));
                model.add_node(
                    ViewNode::new(
                        stmt_key.clone(),
                        format!("use_{}_{}", sym, use_stmt),
                        label,
                        NodeKind::Statement,
                    )
                    .with_parent(Some(group.clone()))
                    .with_details(details)
                    .with_span(span),
                );
                if let Some(defs) = defs_by_stmt.get(&use_stmt) {
                    for &d in defs {
                        model.add_edge(
                            ViewEdge::new(&stmt_key, &value_key(d), EdgeKind::DataDef, "")
                                .with_label(Some("def".to_string())),
                        );
                    }
                }
            }
            model.add_edge(
                ViewEdge::new(&value_key(rec.ssa_id), &stmt_key, EdgeKind::DataUse, "")
                    .with_label(Some("use".to_string())),
            );
        }
    }

    for phi in &func.phi_records {
        for arg in &phi.args {
            if arg.arg_ssa_id == u32::MAX {
                continue;
            }
            model.add_edge(
                ViewEdge::new(
                    &value_key(arg.arg_ssa_id),
                    &value_key(phi.ssa_id),
                    EdgeKind::PhiArgument,
                    &arg.pred_block_id.to_string(),
                )
                .with_label(Some(format!("from B{}", arg.pred_block_id))),
            );
        }
    }
}

// ── 17. CONTROL DEPENDENCE GRAPH ─────────────────────────────────────────────

pub fn cdg_view(ctx: &DiagramContext) -> ViewModel {
    let ssa = match ctx.ssa {
        Some(s) => s,
        None => return ViewModel::unavailable(ViewKind::Cdg, MISSING_SSA),
    };
    let mut model = ViewModel::new(ViewKind::Cdg);
    model.source("ssa");
    if ctx.cfa.is_some() {
        model.source("cfa");
    }
    let mut budget = FunctionBudget::new(ctx.options.max_nodes);
    for (fqn, sym) in ordered_functions(ctx, ssa.functions.iter().map(|f| f.sym_id)) {
        let func = match ctx.function_ssa(sym) {
            Some(f) => f,
            None => continue,
        };
        let n = func.cdg.cd_offsets.len().saturating_sub(1);
        if n == 0 {
            continue;
        }
        if !budget.admit(&fqn, n + 1) {
            continue;
        }
        add_cdg(ctx, &mut model, sym, func, n);
    }
    budget.finish(&mut model, "function(s)");
    if model.nodes.is_empty() && model.omitted.is_empty() {
        return ViewModel::unavailable(ViewKind::Cdg, "The SSA artifact contains no control dependence data.");
    }
    if ctx.cfa.is_none() {
        model.note("Block statement text needs the CFA artifact; blocks are labelled by id.");
    }
    model.note("An edge X → Y labelled T/F means Y executes only when the branch at X takes that outcome; blocks under 'entry' execute unconditionally.");
    model
}

fn add_cdg(ctx: &DiagramContext, model: &mut ViewModel, sym: u32, func: &FunctionSSAData, n: usize) {
    let group = function_group(
        ctx,
        model,
        sym,
        vec![format!("{} control dependence(s)", func.cdg.cd_adj.len())],
    );
    let cfg = ctx.function_cfg(sym);
    let root = format!("cdg:{}:entry", sym);
    model.add_node(
        ViewNode::new(root.clone(), format!("cdgroot_{}", sym), "entry", NodeKind::Initial)
            .with_parent(Some(group.clone())),
    );
    let key = |b: usize| format!("cdb:{}:{}", sym, b);
    for b in 0..n {
        let (label, details, span) = match cfg {
            Some(c) => (
                block_label(c, b as u32),
                block_lines(ctx, c, b as u32, 2),
                block_span(ctx, c, b as u32),
            ),
            None => (format!("B{}", b), Vec::new(), None),
        };
        model.add_node(
            ViewNode::new(key(b), format!("cdb_{}_{}", sym, b), label, NodeKind::BasicBlock)
                .with_parent(Some(group.clone()))
                .with_details(details)
                .with_span(span),
        );
    }
    let mut controlled = vec![false; n];
    let offsets = &func.cdg.cd_offsets;
    for x in 0..n {
        let start = (offsets[x] as usize).min(func.cdg.cd_adj.len());
        let end = (offsets[x + 1] as usize).min(func.cdg.cd_adj.len());
        for k in start..end {
            let y = func.cdg.cd_adj[k] as usize;
            if y >= n {
                continue;
            }
            controlled[y] = true;
            let outcome = if func.cdg.cd_types.get(k).copied() == Some(CD_EDGE_FALSE) {
                "F"
            } else {
                "T"
            };
            let label = match cfg.and_then(|c| block_condition(ctx, c, x as u32)) {
                Some(cond) => format!("{}: {}", outcome, cond),
                None => outcome.to_string(),
            };
            model.add_edge(
                ViewEdge::new(&key(x), &key(y), EdgeKind::ControlDependence, &k.to_string())
                    .with_label(Some(label)),
            );
        }
    }
    for (y, is_controlled) in controlled.iter().enumerate() {
        if !is_controlled {
            model.add_edge(ViewEdge::new(&root, &key(y), EdgeKind::ControlDependence, "entry"));
        }
    }
}

// ── 18. CALL GRAPH & RECURSIVE SCCs ──────────────────────────────────────────

fn call_edge_name(edge_type: u8) -> &'static str {
    match edge_type {
        CG_EDGE_DIRECT => "direct",
        CG_EDGE_SPECIAL => "special",
        CG_EDGE_VIRTUAL => "virtual",
        CG_EDGE_INTERFACE => "interface",
        CG_EDGE_CONSTRUCTOR => "constructor",
        CG_EDGE_DYNAMIC => "dynamic",
        CG_EDGE_REFLECTION => "reflection",
        _ => "unknown",
    }
}

pub fn callgraph_view(ctx: &DiagramContext) -> ViewModel {
    let cga = match ctx.cga {
        Some(c) => c,
        None => return ViewModel::unavailable(ViewKind::CallGraph, MISSING_CGA),
    };
    if cga.site_to_edge_map.is_empty() {
        return ViewModel::unavailable(
            ViewKind::CallGraph,
            "The call-graph artifact contains no resolved call edges.",
        );
    }
    let mut model = ViewModel::new(ViewKind::CallGraph);
    model.source("cga");

    let site_type: HashMap<u32, u8> = cga
        .call_sites
        .iter()
        .map(|s| (s.call_site_id, s.call_type))
        .collect();
    let mut calls: BTreeMap<(u32, u32), (usize, BTreeSet<u8>)> = BTreeMap::new();
    let mut functions: BTreeSet<u32> = BTreeSet::new();
    for &(caller, callee, site) in &cga.site_to_edge_map {
        let entry = calls.entry((caller, callee)).or_insert((0, BTreeSet::new()));
        entry.0 += 1;
        entry.1.insert(site_type.get(&site).copied().unwrap_or(CG_EDGE_DIRECT));
        functions.insert(caller);
        functions.insert(callee);
    }

    let seeds: Vec<u32> = match ctx.options.focus.as_deref() {
        Some(focus) if !focus.is_empty() => {
            let matched: Vec<u32> = functions
                .iter()
                .copied()
                .filter(|&f| ctx.matches_focus(&function_fqn(ctx, f)))
                .collect();
            if matched.is_empty() {
                model.note(format!(
                    "The focus '{}' matched no function of the call graph; the whole graph is shown.",
                    focus
                ));
            }
            matched
        }
        _ => Vec::new(),
    };
    let included: BTreeSet<u32> = if !seeds.is_empty() {
        let mut callees: HashMap<u32, Vec<u32>> = HashMap::new();
        for &(caller, callee) in calls.keys() {
            callees.entry(caller).or_default().push(callee);
        }
        let mut keep: BTreeSet<u32> = seeds.iter().copied().collect();
        let mut queue: VecDeque<(u32, usize)> = seeds.iter().map(|&s| (s, 0)).collect();
        while let Some((f, depth)) = queue.pop_front() {
            if depth >= CALLGRAPH_FOCUS_DEPTH {
                continue;
            }
            if let Some(next) = callees.get(&f) {
                for &c in next {
                    if keep.insert(c) {
                        queue.push_back((c, depth + 1));
                    }
                }
            }
        }
        for &(caller, callee) in calls.keys() {
            if seeds.contains(&callee) {
                keep.insert(caller);
            }
        }
        for f in &functions {
            if !keep.contains(f) {
                model.omit(function_fqn(ctx, *f));
            }
        }
        keep
    } else {
        functions.clone()
    };

    let mut group_of: HashMap<u32, String> = HashMap::new();
    for scc in &cga.sccs {
        if scc.scc_class == SCC_CLASS_NON_RECURSIVE {
            continue;
        }
        let start = scc.member_offset as usize;
        let end = start.saturating_add(scc.member_count as usize).min(cga.scc_members.len());
        if start >= end {
            continue;
        }
        let members: Vec<u32> = cga.scc_members[start..end]
            .iter()
            .copied()
            .filter(|m| included.contains(m))
            .collect();
        if members.is_empty() {
            continue;
        }
        let key = format!("scc:{}", scc.scc_id);
        let label = if scc.scc_class == SCC_CLASS_SELF_RECURSIVE {
            format!("SCC {}: self-recursive", scc.scc_id)
        } else {
            format!("SCC {}: mutually recursive ({} methods)", scc.scc_id, end - start)
        };
        model.add_node(
            ViewNode::new(key.clone(), format!("scc_{}", scc.scc_id), label, NodeKind::Group)
                .with_stereotype("recursive"),
        );
        for m in members {
            group_of.insert(m, key.clone());
        }
    }

    for &f in &included {
        let mut details = Vec::new();
        if let (Some(psa), Some(idx)) = (ctx.psa, ctx.robdd_index(f)) {
            if let Some(h) = psa.function_dir.get(idx) {
                details.push(format!("V(G)={} | #paths={}", h.cyclomatic, h.sat_count));
            }
        }
        model.add_node(
            ViewNode::new(format!("fn:{}", f), format!("fn_{}", f), function_label(ctx, f), NodeKind::Function)
                .with_parent(group_of.get(&f).cloned())
                .with_qualified_name(Some(function_fqn(ctx, f)))
                .with_span(ctx.symbol_span(f))
                .with_details(details)
                .with_sym(f),
        );
    }

    for ((caller, callee), (count, kinds)) in &calls {
        if !included.contains(caller) || !included.contains(callee) {
            continue;
        }
        let mut label = kinds
            .iter()
            .map(|&k| call_edge_name(k))
            .collect::<Vec<_>>()
            .join("/");
        if *count > 1 {
            label.push_str(&format!(" ×{}", count));
        }
        model.add_edge(
            ViewEdge::new(&format!("fn:{}", caller), &format!("fn:{}", callee), EdgeKind::Call, "")
                .with_label(Some(label)),
        );
    }
    model.note("Edge labels give the dispatch kind of the call sites and their count; recursive strongly connected components (Tarjan) are grouped.");
    model
}

// ── 19. ROBDD PATH SUMMARIES ─────────────────────────────────────────────────

fn variable_label(cfg: Option<&FunctionCFGData>, ordering: Option<&Vec<u32>>, var: u16) -> String {
    let edge_idx = ordering.and_then(|o| o.get(var as usize)).copied();
    match (edge_idx, cfg) {
        (Some(e), Some(c)) => match c.edges.get(e as usize) {
            Some(&(u, v, t)) => {
                let kind = edge_type_label(t)
                    .map(|l| format!(" [{}]", l))
                    .unwrap_or_default();
                format!("x{}: B{}→B{}{}", var, u, v, kind)
            }
            None => format!("x{} (CFG edge #{})", var, e),
        },
        (Some(e), None) => format!("x{} (CFG edge #{})", var, e),
        (None, _) => format!("x{}", var),
    }
}

pub fn robdd_view(ctx: &DiagramContext) -> ViewModel {
    let psa = match ctx.psa {
        Some(p) => p,
        None => return ViewModel::unavailable(ViewKind::Robdd, MISSING_PSA),
    };
    if psa.function_dir.is_empty() {
        return ViewModel::unavailable(ViewKind::Robdd, "The path-summary artifact contains no functions.");
    }
    let mut model = ViewModel::new(ViewKind::Robdd);
    model.source("psa");
    if ctx.cfa.is_some() {
        model.source("cfa");
    }
    let mut budget = FunctionBudget::new(ctx.options.max_nodes);
    for (fqn, sym) in ordered_functions(ctx, psa.function_dir.iter().map(|h| h.sym_id)) {
        let idx = match ctx.robdd_index(sym) {
            Some(i) => i,
            None => continue,
        };
        let (header, nodes) = match (psa.function_dir.get(idx), psa.node_arrays.get(idx)) {
            (Some(h), Some(n)) => (h, n),
            _ => continue,
        };
        let ordering = psa.ordering_tables.get(idx);

        let mut reachable: BTreeSet<u32> = BTreeSet::new();
        let mut stack = vec![header.root_node];
        while let Some(id) = stack.pop() {
            if (id as usize) >= nodes.len() || !reachable.insert(id) {
                continue;
            }
            let node = nodes[id as usize];
            let terminal = id == FALSE_ID
                || id == TRUE_ID
                || node.flags & (FLAG_FALSE_TERMINAL | FLAG_TRUE_TERMINAL) != 0;
            if !terminal {
                stack.push(node.lo);
                stack.push(node.hi);
            }
        }
        if reachable.is_empty() {
            continue;
        }
        if !budget.admit(&fqn, reachable.len() + 1) {
            continue;
        }

        let group = function_group(
            ctx,
            &mut model,
            sym,
            vec![format!(
                "#SAT paths={} | V(G)={} | {} of {} ROBDD nodes reachable | longest path {} blocks",
                header.sat_count,
                header.cyclomatic,
                reachable.len(),
                nodes.len(),
                header.max_path_len
            )],
        );
        let cfg = ctx.function_cfg(sym);
        let key = |id: u32| format!("bdd:{}:{}", sym, id);
        for &id in &reachable {
            let node = nodes[id as usize];
            let (kind, label) = if id == FALSE_ID || node.flags & FLAG_FALSE_TERMINAL != 0 {
                (NodeKind::BddFalse, "0".to_string())
            } else if id == TRUE_ID || node.flags & FLAG_TRUE_TERMINAL != 0 {
                (NodeKind::BddTrue, "1".to_string())
            } else {
                (NodeKind::BddVariable, variable_label(cfg, ordering, node.var))
            };
            model.add_node(
                ViewNode::new(key(id), format!("bdd_{}_{}", sym, id), label, kind)
                    .with_parent(Some(group.clone())),
            );
        }
        for &id in &reachable {
            let node = nodes[id as usize];
            let terminal = id == FALSE_ID
                || id == TRUE_ID
                || node.flags & (FLAG_FALSE_TERMINAL | FLAG_TRUE_TERMINAL) != 0;
            if terminal {
                continue;
            }
            model.add_edge(
                ViewEdge::new(&key(id), &key(node.lo), EdgeKind::BddLow, "")
                    .with_label(Some("0".to_string())),
            );
            model.add_edge(
                ViewEdge::new(&key(id), &key(node.hi), EdgeKind::BddHigh, "")
                    .with_label(Some("1".to_string())),
            );
        }
    }
    budget.finish(&mut model, "function(s)");
    if ctx.cfa.is_none() {
        model.note("Variables map to CFG edges; their endpoints need the CFA artifact.");
    }
    model.note("Each variable xᵢ is one CFG edge; solid edges are the 1 (taken) branch, dashed edges the 0 branch.");
    model
}
