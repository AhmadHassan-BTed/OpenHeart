//! Phase 4: Control Flow Graph Construction & Dominator Analysis Module.

pub mod analysis;
pub mod builder;
pub mod serializer;
pub mod stmts;

use crate::ast::BPASTArtifact;
use crate::cfg::builder::{FunctionCFGBuilder, FunctionCFGData};
use crate::cfg::serializer::{CFGArtifact, CFGSerializer};
use crate::core::types::symbol::SymbolKind;
use crate::ingestion::serializer::crc64_ecma;
use crate::symbol::SymbolTableArtifact;
use std::path::Path;

pub struct Phase4Stage;

impl Phase4Stage {
    pub fn run(
        bpa: &BPASTArtifact,
        sta: &SymbolTableArtifact,
        sta_bytes: &[u8],
        bpa_bytes: &[u8],
        out_path: &Path,
    ) -> Result<CFGArtifact, String> {
        let sta_hash = crc64_ecma(sta_bytes);
        let bpa_hash = crc64_ecma(bpa_bytes);

        let mut artifact = CFGArtifact::new(sta_hash, bpa_hash);

        for sym_id in 0..sta.symbol_count {
            let sym = match sta.symbol(sym_id) {
                Some(s) => s,
                None => continue,
            };

            let kind = sym.kind;
            let is_fn_like = kind == SymbolKind::SK_METHOD as u8
                || kind == SymbolKind::SK_CONSTRUCTOR as u8
                || kind == SymbolKind::SK_STATIC_INIT as u8
                || kind == SymbolKind::SK_LAMBDA as u8;

            if !is_fn_like {
                continue;
            }

            let def_node = sym.def_node;
            if def_node == u32::MAX {
                continue; // abstract or native method without body
            }

            let cfg_data = FunctionCFGBuilder::build(sym_id, def_node, bpa, sta);
            Self::verify_function_invariants(&cfg_data)?;
            artifact.add_function(cfg_data);
        }

        CFGSerializer::write(&artifact, out_path)
            .map_err(|e| format!("Failed to serialize .cfa: {}", e))?;

        Ok(artifact)
    }

    fn verify_function_invariants(cfg: &FunctionCFGData) -> Result<(), String> {
        // ── Invariant 1: Unique EXIT ──
        let exit_count = cfg.blocks.iter().filter(|b| b.is_exit).count();
        if exit_count != 1 {
            return Err(format!(
                "Invariant 1 Violation (Unique EXIT): Function sym {} has {} exit blocks (expected 1)",
                cfg.sym_id, exit_count
            ));
        }

        // ── Invariant 2: Dominator Tree Completeness ──
        for (i, &idom_val) in cfg.idom.iter().enumerate() {
            if i > 0 && idom_val == u32::MAX {
                let pred_count = cfg
                    .pred_offsets
                    .get(i + 1)
                    .copied()
                    .unwrap_or(0)
                    .saturating_sub(cfg.pred_offsets.get(i).copied().unwrap_or(0));

                if pred_count > 0 {
                    return Err(format!(
                        "Invariant 2 Violation (Dominator Completeness): Function sym {} block {} reachable but idom is UNDEFINED",
                        cfg.sym_id, i
                    ));
                }
            }
        }

        Ok(())
    }
}
