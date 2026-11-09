//! Traceability Artifact Builder & Invariants Verifier (§7.5.1, §7.8).

use std::collections::BTreeMap;

use crate::ast::BPASTArtifact;
use crate::cfg::serializer::CFGArtifact;
use crate::core::logger::{log_info, log_warn};
use crate::core::types::cg::CallGraphArtifact;
use crate::core::types::token::unpack_sort_key;
use crate::ingestion::TokenCorpusArtifact;
use crate::ssa::SSAArtifact;
use crate::symbol::SymbolTableArtifact;
use crate::tra::backward::{
    ASTBackwardIndex, BlockBackwardIndex, CallSiteBackwardIndex, FunctionBlockRange,
    FunctionSsaRange, SSABackwardIndex, SymbolBackwardIndex,
};
use crate::tra::entity::EntityIndex;
use crate::tra::forward::{CallSiteSpanIndex, SymbolSpanIndex};
use crate::tra::types::{
    FunctionIndexRecord, TraceabilityArtifact, NO_OFFSET, NO_SOURCE_TOKEN, TRA_FORMAT_VERSION,
};
use crate::tra::uml_link::{ScpgHashChain, UMLLinkRegistry, UML_SYMBOL_KINDS};

pub struct TraceabilityArtifactBuilder;

impl TraceabilityArtifactBuilder {
    /// Build the traceability artifact. Invariant violations are reported as diagnostics
    /// (see [`TraceabilityArtifactBuilder::verify_invariants`]); the artifact is returned
    /// either way so that the violations can be inspected.
    pub fn build(
        tca: &TokenCorpusArtifact,
        bpa: &BPASTArtifact,
        sta: &SymbolTableArtifact,
        cfa: &CFGArtifact,
        ssa: &SSAArtifact,
        cga: &CallGraphArtifact,
    ) -> TraceabilityArtifact {
        let hashes = ScpgHashChain::compute(tca, bpa, sta, cfa, ssa, cga);

        let bi_ast = ASTBackwardIndex::build(bpa);
        let bi_sym = SymbolBackwardIndex::build(sta, bpa);
        let (bi_blk, block_ranges) = BlockBackwardIndex::build(cfa, &bi_sym);
        let (bi_ssa, ssa_ranges) =
            SSABackwardIndex::build(ssa, bpa, &bi_blk, &block_ranges, &bi_sym);
        let function_index = Self::function_index(&block_ranges, &ssa_ranges);
        let (bi_cs, cs_report) = CallSiteBackwardIndex::build(cga);
        if !cs_report.is_clean() {
            log_warn(&format!(
                "Phase 7 DIAGNOSTIC: BI_cs: {} call sites with out-of-range ids, {} with duplicate ids, {} unclaimed slots",
                cs_report.out_of_range, cs_report.duplicates, cs_report.unclaimed
            ));
        }

        let sym_span = SymbolSpanIndex::build(&bi_sym, tca, sta);
        let cs_span = CallSiteSpanIndex::build(cga, tca);
        let uml_links = UMLLinkRegistry::build(&bi_sym, tca, sta, hashes.scpg_hash);
        let uml_slots = UMLLinkRegistry::slots(&uml_links, sta.symbol_records.len());
        let entities = EntityIndex::build(sta, tca, &bi_sym);

        let artifact = TraceabilityArtifact {
            format_version: TRA_FORMAT_VERSION,
            hashes,
            bi_ast,
            bi_sym,
            bi_blk,
            bi_ssa,
            bi_cs,
            sym_span,
            cs_span,
            uml_links,
            function_index,
            entities,
            uml_slots,
        };

        match Self::verify_invariants(&artifact, sta, tca) {
            Ok(()) => log_info("Phase 7 Invariants 1-4 verified."),
            Err(violations) => {
                for v in &violations {
                    log_warn(&format!("Phase 7 DIAGNOSTIC: {}", v));
                }
            }
        }
        artifact
    }

    /// Merge the CFA block ranges and SSA ranges into one record per function, sorted by
    /// sym_id.
    fn function_index(
        block_ranges: &[FunctionBlockRange],
        ssa_ranges: &[FunctionSsaRange],
    ) -> Vec<FunctionIndexRecord> {
        let mut by_sym: BTreeMap<u32, FunctionIndexRecord> = BTreeMap::new();
        let blank = |sym_id: u32| FunctionIndexRecord {
            sym_id,
            first_block: NO_OFFSET,
            block_count: 0,
            first_ssa: NO_OFFSET,
            ssa_count: 0,
        };
        for &(sym, first, count) in block_ranges {
            let r = by_sym.entry(sym).or_insert_with(|| blank(sym));
            r.first_block = first;
            r.block_count = count;
        }
        for &(sym, first, count) in ssa_ranges {
            let r = by_sym.entry(sym).or_insert_with(|| blank(sym));
            r.first_ssa = first;
            r.ssa_count = count;
        }
        by_sym.into_values().collect()
    }

    /// Check Invariants 1–4 (§7.8) and table consistency; returns every violation.
    ///
    /// - **Invariant 1 (BI_blk completeness):** every block maps to a token range — its own or
    ///   its owning function's; [`NO_SOURCE_TOKEN`] only where no span exists at all.
    /// - **Invariant 2 (UMLLink coverage):** one UMLLink per UML-kind symbol.
    /// - **Invariant 3 (hash chain):** the chain digest and tags follow from the stored
    ///   per-artifact digests.
    /// - **Invariant 4 (backward–forward round trip):** for *every* symbol with a declaration
    ///   span, the forward query at its first token returns it.
    pub fn verify_invariants(
        tra: &TraceabilityArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> Result<(), Vec<String>> {
        let mut violations: Vec<String> = Vec::new();
        let n_syms = sta.symbol_records.len();

        // Invariant 1.
        let unsourced = tra
            .bi_blk
            .iter()
            .filter(|e| e.first_token_id == NO_SOURCE_TOKEN)
            .count();
        if unsourced > 0 {
            violations.push(format!(
                "Invariant 1: {} basic blocks have neither tokens nor an owning function span",
                unsourced
            ));
        }
        if let Some(bad) = tra.bi_blk.iter().position(|e| {
            e.first_token_id != NO_SOURCE_TOKEN && e.last_token_id < e.first_token_id
        }) {
            violations.push(format!("Invariant 1: BI_blk[{}] has an inverted token range", bad));
        }

        // Invariant 2.
        let expected_uml_count = sta
            .symbol_records
            .iter()
            .filter(|sym| UML_SYMBOL_KINDS.contains(&sym.kind))
            .count();
        if tra.uml_links.len() != expected_uml_count {
            violations.push(format!(
                "Invariant 2: {} UMLLinks for {} UML-kind symbols",
                tra.uml_links.len(),
                expected_uml_count
            ));
        }

        // Invariant 3.
        if let Err(msg) = tra.hashes.verify() {
            violations.push(format!("Invariant 3: {}", msg));
        }

        // Table consistency.
        if tra.bi_sym.len() != n_syms || tra.entities.len() != n_syms || tra.uml_slots.len() != n_syms
        {
            violations.push(format!(
                "per-symbol tables disagree: {} symbols, {} BI_sym, {} entities, {} UML slots",
                n_syms,
                tra.bi_sym.len(),
                tra.entities.len(),
                tra.uml_slots.len()
            ));
        }

        // Invariant 4 (exhaustive).
        let n_tokens = tca.token_records.len();
        let mut missing = 0usize;
        let mut first_missing: Option<usize> = None;
        for (s, entry) in tra.bi_sym.iter().enumerate().take(n_syms) {
            if entry.decl_first_tok == u32::MAX || (entry.decl_first_tok as usize) >= n_tokens {
                continue;
            }
            if entry.decl_last_tok.min(n_tokens as u32 - 1) < entry.decl_first_tok {
                continue;
            }
            let start_tok = &tca.token_records[entry.decl_first_tok as usize];
            let (file_id, _, _) = unpack_sort_key(start_tok.sort_key);
            let forward =
                SymbolSpanIndex::forward_sym_query(entry.decl_first_tok, file_id, &tra.sym_span);
            if !forward.contains(&(s as u32)) {
                missing += 1;
                if first_missing.is_none() {
                    first_missing = Some(s);
                }
            }
        }
        if let Some(first) = first_missing {
            violations.push(format!(
                "Invariant 4: {} symbols are not returned by the forward query at their first \
                 token (first: sym_id={})",
                missing, first
            ));
        }

        if violations.is_empty() {
            Ok(())
        } else {
            Err(violations)
        }
    }
}
