//! Call-site extraction (§6.5.1).
//!
//! Every invocation-like node (method invocation, explicit constructor invocation,
//! instance creation, method reference, lambda creation) is attributed to the innermost
//! enclosing function via the program index (a lambda is attributed to the function that
//! *creates* it), mapped to the CFG block and block statement that evaluates it, and
//! decomposed into receiver / name / arguments using the token text. Calls in field
//! initializers and instance-initializer blocks have no function of their own and are
//! attributed to their class symbol (the class's `<init>` / `<clinit>` prologue).

use crate::ast::BPASTArtifact;
use crate::cfg::ast_query::{call_shape, children, CallShape, NewParts};
use crate::cfg::builder::FunctionCFGData;
use crate::cfg::parallel::par_map;
use crate::cfg::serializer::CFGArtifact;
use crate::cfg::source::SourceText;
use crate::cg::index::ProgramIndex;
use crate::core::types::ASTNodeType::*;
use crate::core::types::*;
use crate::ssa::names::FunctionVars;
use crate::ssa::renaming::PHI_USE;
use crate::ssa::serializer::FunctionSSAData;
use crate::ssa::SSAArtifact;
use std::collections::HashMap;

pub const VIS_PRIVATE: u8 = SymbolVisibility::Private as u8;

/// Per-function data used to resolve and model the calls of one function.
pub struct FunctionContext {
    /// Index of the function in `CFGArtifact::functions`.
    pub fn_index: usize,
    /// Index of the function in `SSAArtifact::functions`, when it was converted.
    pub ssa_index: Option<usize>,
    pub sym_id: u32,
    /// Global id of the function's SSA id 0 (see `SSAArtifact::global_ssa_base`).
    pub ssa_base: u32,
    pub vars: FunctionVars,
    /// Block statement -> block.
    stmt_block: HashMap<u32, u32>,
    /// (statement, orig_sym_id) -> local SSA id read by the statement.
    ssa_at: HashMap<(u32, u32), u32>,
    decl_node: u32,
}

impl FunctionContext {
    #[allow(clippy::too_many_arguments)]
    pub fn build(
        fn_index: usize,
        ssa_index: Option<usize>,
        cfg: &FunctionCFGData,
        ssa_fn: Option<&FunctionSSAData>,
        ssa_base: u32,
        bpa: &BPASTArtifact,
        src: Option<&SourceText<'_>>,
        sym_by_decl: &HashMap<u32, u32>,
    ) -> Self {
        let vars = FunctionVars::build(bpa, src, sym_by_decl, cfg.decl_node);
        let mut stmt_block = HashMap::new();
        for b in &cfg.blocks {
            for &s in &b.stmts {
                stmt_block.entry(s).or_insert(b.id);
            }
        }
        let mut ssa_at = HashMap::new();
        if let Some(f) = ssa_fn {
            for (d, rec) in f.ssa_records.iter().enumerate() {
                for &stmt in f.def_use.uses_of(d as u32) {
                    if stmt != PHI_USE {
                        ssa_at.entry((stmt, rec.orig_sym_id)).or_insert(d as u32);
                    }
                }
            }
        }
        Self {
            fn_index,
            ssa_index,
            sym_id: cfg.sym_id,
            ssa_base,
            vars,
            stmt_block,
            ssa_at,
            decl_node: cfg.decl_node,
        }
    }

    /// Innermost block statement (and its block) that evaluates `node`.
    pub fn stmt_of(&self, bpa: &BPASTArtifact, node: u32) -> Option<(u32, u32)> {
        let mut cur = node;
        let mut guard = 0usize;
        while cur != u32::MAX && guard < 1 << 20 {
            if let Some(&b) = self.stmt_block.get(&cur) {
                return Some((cur, b));
            }
            if cur == self.decl_node {
                return None;
            }
            let p = bpa.parent(cur);
            if p == cur {
                return None;
            }
            cur = p;
            guard += 1;
        }
        None
    }

    /// Local SSA id of the variable denoted by identifier `ident`, as read by `stmt`.
    pub fn local_ssa_of_ident(&self, stmt: u32, ident: u32) -> Option<u32> {
        let var = self.vars.var_of_occurrence(ident)?;
        let orig = self.vars.orig_id(var);
        self.ssa_at.get(&(stmt, orig)).copied()
    }
}

/// Builds the contexts of every function with a CFG.
pub fn build_contexts(
    index: &ProgramIndex<'_>,
    cfa: &CFGArtifact,
    ssa: &SSAArtifact,
    sym_by_decl: &HashMap<u32, u32>,
) -> Result<(Vec<FunctionContext>, HashMap<u32, usize>), String> {
    let bases = ssa.global_ssa_base();
    let ssa_pos: HashMap<u32, usize> = ssa
        .functions
        .iter()
        .enumerate()
        .map(|(i, f)| (f.sym_id, i))
        .collect();
    let positions: Vec<usize> = (0..cfa.functions.len()).collect();
    let built = par_map(&positions, |&fi| {
        let cfg = &cfa.functions[fi];
        let si = ssa_pos.get(&cfg.sym_id).copied();
        let ssa_fn = si.map(|i| &ssa.functions[i]);
        let base = si.and_then(|i| bases.get(i).copied()).unwrap_or(u32::MAX);
        Ok(FunctionContext::build(
            fi,
            si,
            cfg,
            ssa_fn,
            base,
            index.bpa,
            index.src.as_ref(),
            sym_by_decl,
        ))
    });

    let mut contexts = Vec::with_capacity(cfa.functions.len());
    let mut ctx_of_fn = HashMap::new();
    for (fi, result) in built.into_iter().enumerate() {
        let ctx = result.map_err(|e| {
            format!(
                "call-graph context construction failed for function sym {}: {}",
                cfa.functions[fi].sym_id, e
            )
        })?;
        ctx_of_fn.insert(ctx.sym_id, contexts.len());
        contexts.push(ctx);
    }
    Ok((contexts, ctx_of_fn))
}

/// Role-recovered shape of a call site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SiteKind {
    Method {
        receiver: Option<u32>,
        name: u32,
        name_id: u32,
        args: Vec<u32>,
    },
    ConstructorChain {
        is_super: bool,
        args: Vec<u32>,
    },
    New {
        type_name: Option<u32>,
        type_ref: Option<u32>,
        args: Vec<u32>,
        anonymous: bool,
    },
    MethodRef {
        qualifier: Option<u32>,
        name: Option<u32>,
        name_id: Option<u32>,
        is_new: bool,
    },
    Lambda,
    /// Roles of the children cannot be recovered (no token text).
    Opaque,
}

/// A call site before resolution.
#[derive(Debug, Clone)]
pub struct SiteInfo {
    pub call_node: u32,
    pub caller_sym: u32,
    pub kind: SiteKind,
    /// Index into the function contexts (None for initializer calls).
    pub ctx: Option<usize>,
    /// Block statement evaluating the call (`u32::MAX` when not mapped).
    pub stmt: u32,
    /// CFG block of `stmt` (`u32::MAX` when not mapped).
    pub block: u32,
    pub call_token: u32,
    pub arg_count: u16,
    /// Local SSA id of the receiver variable, when the receiver is a local.
    pub receiver_local_ssa: Option<u32>,
}

impl SiteInfo {
    pub fn args(&self) -> &[u32] {
        match &self.kind {
            SiteKind::Method { args, .. }
            | SiteKind::ConstructorChain { args, .. }
            | SiteKind::New { args, .. } => args,
            _ => &[],
        }
    }

    pub fn receiver(&self) -> Option<u32> {
        match &self.kind {
            SiteKind::Method { receiver, .. } => *receiver,
            _ => None,
        }
    }
}

fn first_token_of(bpa: &BPASTArtifact, node: u32) -> u32 {
    bpa.token_range(node).0
}

fn shape_of(index: &ProgramIndex<'_>, node: u32) -> SiteKind {
    let bpa = index.bpa;
    let src = match index.src.as_ref() {
        Some(s) => s,
        None => {
            return if bpa.node_type(node) == NN_LAMBDA_EXPR {
                SiteKind::Lambda
            } else {
                SiteKind::Opaque
            }
        }
    };
    match bpa.node_type(node) {
        NN_CALL_EXPR => match call_shape(bpa, src, node) {
            Some(CallShape::Method {
                receiver,
                name,
                args,
            }) => match src.node_text_id(bpa, name) {
                Some(name_id) => SiteKind::Method {
                    receiver,
                    name,
                    name_id,
                    args,
                },
                None => SiteKind::Opaque,
            },
            Some(CallShape::ConstructorChain { is_super, args }) => {
                SiteKind::ConstructorChain { is_super, args }
            }
            None => SiteKind::Opaque,
        },
        NN_NEW_EXPR => {
            let parts = NewParts::of(bpa, Some(src), node);
            SiteKind::New {
                type_name: parts.type_name,
                type_ref: parts.type_ref,
                args: parts.args,
                anonymous: !parts.anonymous_members.is_empty(),
            }
        }
        NN_METHOD_REF => {
            let kids = children(bpa, node);
            // `Type::new`: Phase 2 drops both `::` and `new`, so look past the qualifier.
            let (_, lt) = bpa.token_range(node);
            let is_new = lt != u32::MAX
                && src.token_is(lt.saturating_add(1), b"::")
                && src.token_is(lt.saturating_add(2), b"new");
            let name = if kids.len() >= 2 {
                kids.last()
                    .copied()
                    .filter(|&c| bpa.node_type(c) == NN_IDENTIFIER_EXPR)
            } else {
                None
            };
            SiteKind::MethodRef {
                qualifier: kids.first().copied().filter(|&q| Some(q) != name),
                name,
                name_id: name.and_then(|n| src.node_text_id(bpa, n)),
                is_new,
            }
        }
        NN_LAMBDA_EXPR => SiteKind::Lambda,
        _ => SiteKind::Opaque,
    }
}

/// Extracts all call sites from the BP AST (§6.5.1), in (caller, node) order.
pub fn extract_call_sites(
    index: &ProgramIndex<'_>,
    contexts: &[FunctionContext],
    ctx_of_fn: &HashMap<u32, usize>,
) -> Vec<SiteInfo> {
    let bpa = index.bpa;
    let mut sites = Vec::new();

    for pre_idx in 0..bpa.node_count {
        let ntype = bpa.node_type(pre_idx);
        if !matches!(
            ntype,
            NN_CALL_EXPR | NN_NEW_EXPR | NN_METHOD_REF | NN_LAMBDA_EXPR
        ) {
            continue;
        }

        // A lambda is a call site of the function that creates it, not of itself.
        let anchor = if ntype == NN_LAMBDA_EXPR {
            bpa.parent(pre_idx)
        } else {
            pre_idx
        };
        let caller_sym = match index
            .enclosing_function(anchor)
            .or_else(|| index.class_containing_node(anchor))
        {
            Some(c) => c,
            None => continue,
        };

        let ctx = ctx_of_fn.get(&caller_sym).copied();
        let (stmt, block) = ctx
            .and_then(|c| contexts[c].stmt_of(bpa, pre_idx))
            .unwrap_or((u32::MAX, u32::MAX));

        let kind = shape_of(index, pre_idx);
        let (call_token, arg_count) = match &kind {
            SiteKind::Method { name, args, .. } => (first_token_of(bpa, *name), args.len()),
            SiteKind::ConstructorChain { args, .. } => (first_token_of(bpa, pre_idx), args.len()),
            SiteKind::New { type_ref, args, .. } => (
                type_ref
                    .map(|t| first_token_of(bpa, t))
                    .unwrap_or_else(|| first_token_of(bpa, pre_idx)),
                args.len(),
            ),
            SiteKind::MethodRef { name, .. } => (
                name.map(|n| first_token_of(bpa, n))
                    .unwrap_or_else(|| first_token_of(bpa, pre_idx)),
                0,
            ),
            SiteKind::Lambda => (first_token_of(bpa, pre_idx), 0),
            SiteKind::Opaque => (first_token_of(bpa, pre_idx), children(bpa, pre_idx).len()),
        };

        let receiver_local_ssa = match (&kind, ctx) {
            (
                SiteKind::Method {
                    receiver: Some(r), ..
                },
                Some(c),
            ) if bpa.node_type(*r) == NN_IDENTIFIER_EXPR && stmt != u32::MAX => {
                contexts[c].local_ssa_of_ident(stmt, *r)
            }
            _ => None,
        };

        sites.push(SiteInfo {
            call_node: pre_idx,
            caller_sym,
            kind,
            ctx,
            stmt,
            block,
            call_token,
            arg_count: u16::try_from(arg_count).unwrap_or(u16::MAX),
            receiver_local_ssa,
        });
    }

    sites.sort_by_key(|s| (s.caller_sym, s.call_node));
    sites
}
