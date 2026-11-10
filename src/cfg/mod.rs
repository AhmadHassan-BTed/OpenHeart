//! Phase 4: Control Flow Graph Construction & Dominator Analysis Module.

pub mod analysis;
pub mod ast_query;
pub mod builder;
pub mod parallel;
pub mod serializer;
pub mod source;
pub mod stmts;

use crate::ast::BPASTArtifact;
use crate::cfg::ast_query::{children, first_child_of_type};
use crate::cfg::builder::exceptions::ExceptionTypes;
use crate::cfg::builder::{is_body_statement, FunctionBody, FunctionCFGBuilder, FunctionCFGData};
use crate::cfg::parallel::par_map;
use crate::cfg::serializer::{CFGArtifact, CFGSerializer};
use crate::cfg::source::SourceText;
use crate::core::logger::{get_log_level, log_debug, log_info, log_warn, LogLevel, PhaseTimer};
use crate::core::types::ast::ASTNodeType;
use crate::core::types::symbol::{SymbolKind, SymbolRecord};
use crate::ingestion::serializer::{crc64_ecma, TokenCorpusArtifact};
use crate::symbol::SymbolTableArtifact;
use std::path::Path;

pub struct Phase4Stage;

/// One function whose CFG is to be built.
#[derive(Debug, Clone, Copy)]
struct CfgJob {
    sym_id: u32,
    decl_node: u32,
    body: FunctionBody,
}

/// True for symbol kinds that own executable code.
pub fn is_callable_kind(kind: u8) -> bool {
    kind == SymbolKind::SK_METHOD as u8
        || kind == SymbolKind::SK_CONSTRUCTOR as u8
        || kind == SymbolKind::SK_STATIC_INIT as u8
        || kind == SymbolKind::SK_INSTANCE_INIT as u8
        || kind == SymbolKind::SK_LAMBDA as u8
}

/// Locates the body of a function symbol. `None` means the function has no body in the
/// BPA (abstract, native or interface method, or a body Phase 2 did not reduce).
pub fn locate_body(sym: &SymbolRecord, bpa: &BPASTArtifact) -> Option<FunctionBody> {
    let decl = if sym.decl_node != u32::MAX {
        sym.decl_node
    } else {
        sym.def_node
    };
    if decl == u32::MAX || decl >= bpa.node_count {
        return None;
    }
    match bpa.node_type(decl) {
        ASTNodeType::NN_BLOCK => Some(FunctionBody::Block(decl)),
        ASTNodeType::NN_LAMBDA_EXPR => {
            // `params -> expr` or `params -> { ... }`: the body is the last child.
            let last = children(bpa, decl).into_iter().last()?;
            match bpa.node_type(last) {
                ASTNodeType::NN_BLOCK => Some(FunctionBody::Block(last)),
                ASTNodeType::NN_PARAM_DECL => None,
                _ => Some(FunctionBody::Expression(last)),
            }
        }
        ASTNodeType::NN_METHOD_DECL
        | ASTNodeType::NN_CONSTRUCTOR_DECL
        | ASTNodeType::NN_JAVA_STATIC_INIT
        | ASTNodeType::NN_JAVA_INSTANCE_INIT => {
            if let Some(b) = first_child_of_type(bpa, decl, ASTNodeType::NN_BLOCK) {
                return Some(FunctionBody::Block(b));
            }
            let has_statements = children(bpa, decl)
                .into_iter()
                .any(|c| is_body_statement(bpa, c));
            if has_statements && bpa.node_type(decl) != ASTNodeType::NN_METHOD_DECL {
                Some(FunctionBody::Statements(decl))
            } else {
                None
            }
        }
        _ => None,
    }
}

impl Phase4Stage {
    /// Runs Phase 4 without token text. Labels, `for` headers, case kinds and constant
    /// conditions are then recovered structurally and the approximations are recorded in
    /// each function's `diagnostics`; prefer [`Phase4Stage::run_with_source`].
    pub fn run(
        bpa: &BPASTArtifact,
        sta: &SymbolTableArtifact,
        sta_bytes: &[u8],
        bpa_bytes: &[u8],
        out_path: &Path,
    ) -> Result<CFGArtifact, String> {
        Self::run_with_source(bpa, sta, None, sta_bytes, bpa_bytes, out_path)
    }

    pub fn run_with_source(
        bpa: &BPASTArtifact,
        sta: &SymbolTableArtifact,
        tca: Option<&TokenCorpusArtifact>,
        sta_bytes: &[u8],
        bpa_bytes: &[u8],
        out_path: &Path,
    ) -> Result<CFGArtifact, String> {
        let timer = PhaseTimer::start("Phase 4: Control Flow Graph & Dominator Analysis");

        let sta_hash = crc64_ecma(sta_bytes);
        let bpa_hash = crc64_ecma(bpa_bytes);
        log_info(&format!(
            "STA Link Hash: 0x{:016X} | BPA Link Hash: 0x{:016X}",
            sta_hash, bpa_hash
        ));

        let src = tca.map(SourceText::new);
        if src.is_none() {
            log_warn(
                "Phase 4 running without token text: labels, for-headers and switch rule kinds are approximated.",
            );
        }
        let exceptions = ExceptionTypes::new(sta, src);

        // ── Collect functions with a body ──
        let mut jobs = Vec::new();
        let mut bodiless = 0usize;
        let mut bodiless_ctors = 0usize;
        for sym in &sta.symbol_records {
            if !is_callable_kind(sym.kind) {
                continue;
            }
            match locate_body(sym, bpa) {
                Some(body) => jobs.push(CfgJob {
                    sym_id: sym.symbol_id,
                    decl_node: if sym.decl_node != u32::MAX {
                        sym.decl_node
                    } else {
                        sym.def_node
                    },
                    body,
                }),
                None => {
                    bodiless += 1;
                    if sym.kind == SymbolKind::SK_CONSTRUCTOR as u8 && sym.decl_node != u32::MAX {
                        bodiless_ctors += 1;
                    }
                }
            }
        }
        if bodiless_ctors > 0 {
            log_warn(&format!(
                "Phase 4: {} constructors have no body in the BPA (constructor_body not reduced by Phase 2); no CFG built for them.",
                bodiless_ctors
            ));
        }

        // ── Build in parallel on large-stack workers; merge in symbol order ──
        let results = par_map(&jobs, |job: &CfgJob| {
            FunctionCFGBuilder::build(
                job.sym_id,
                job.decl_node,
                job.body,
                bpa,
                sta,
                src,
                &exceptions,
            )
        });

        let mut artifact = CFGArtifact::new(sta_hash, bpa_hash);
        let mut diagnostics = 0usize;
        for (job, result) in jobs.iter().zip(results) {
            let cfg_data = result.map_err(|e| {
                format!(
                    "CFG construction failed for function sym {}: {}",
                    job.sym_id, e
                )
            })?;
            Self::verify_function_invariants(&cfg_data)?;
            diagnostics += cfg_data.diagnostics.len();

            if get_log_level() >= LogLevel::Debug {
                log_debug(&format!(
                    "  Function sym_id={}: {} basic blocks, {} CFG edges, Cyclomatic V(G)={}",
                    cfg_data.sym_id,
                    cfg_data.blocks.len(),
                    cfg_data.edges.len(),
                    cfg_data.cyclomatic
                ));
            }

            artifact.add_function(cfg_data)?;
        }

        log_info(&format!(
            "Serializing CFGArtifact (.cfa) to {}...",
            out_path.display()
        ));
        CFGSerializer::write(&artifact, out_path)
            .map_err(|e| format!("Failed to serialize .cfa: {}", e))?;

        let summary = format!(
            "Analyzed {} functions ({} blocks, {} edges); {} without body; {} approximation diagnostics.",
            artifact.function_count, artifact.total_blocks, artifact.total_edges, bodiless, diagnostics
        );
        log_info(&format!("Phase 4 Complete: {}", summary));
        timer.finish(&summary);

        Ok(artifact)
    }

    fn verify_function_invariants(cfg: &FunctionCFGData) -> Result<(), String> {
        // ── Invariant 1: Unique EXIT ──
        let exit_count = cfg.blocks.iter().filter(|b| b.is_exit).count();
        if exit_count != 1
            || !cfg
                .blocks
                .get(cfg.exit_block as usize)
                .is_some_and(|b| b.is_exit)
        {
            return Err(format!(
                "Invariant 1 Violation (Unique EXIT): Function sym {} has {} exit blocks (expected 1 at block {})",
                cfg.sym_id, exit_count, cfg.exit_block
            ));
        }

        // ── Invariant 2: Dominator Tree Completeness ──
        for (i, &idom_val) in cfg.idom.iter().enumerate() {
            if i > 0 && idom_val == u32::MAX && !cfg.preds_of(i as u32).is_empty() {
                return Err(format!(
                    "Invariant 2 Violation (Dominator Completeness): Function sym {} block {} reachable but idom is UNDEFINED",
                    cfg.sym_id, i
                ));
            }
        }

        // ── Invariant 3: No dangling blocks (only EXIT lacks successors) ──
        for b in 0..cfg.blocks.len() as u32 {
            if b != cfg.exit_block && cfg.succs_of(b).is_empty() {
                return Err(format!(
                    "Invariant 3 Violation (Dangling Block): Function sym {} block {} has no successor",
                    cfg.sym_id, b
                ));
            }
        }

        // ── Invariant 4: CSR / edge-list consistency ──
        if cfg.succ_adj.len() != cfg.edges.len()
            || cfg.pred_adj.len() != cfg.edges.len()
            || cfg.pred_edge_idx.len() != cfg.edges.len()
            || cfg.edge_types.len() != cfg.edges.len()
        {
            return Err(format!(
                "Invariant 4 Violation (CSR Consistency): Function sym {} has {} edges but CSR sizes {}/{}/{}",
                cfg.sym_id,
                cfg.edges.len(),
                cfg.succ_adj.len(),
                cfg.pred_adj.len(),
                cfg.edge_types.len()
            ));
        }

        Ok(())
    }
}
