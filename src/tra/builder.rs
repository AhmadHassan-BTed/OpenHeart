//! Traceability Artifact Builder & Invariants Verifier (§7.5.1, §7.8).

use crate::ast::BPASTArtifact;
use crate::cfg::serializer::CFGArtifact;
use crate::core::logger::log_info;
use crate::core::types::cg::CallGraphArtifact;
use crate::core::types::symbol::SymbolKind;
use crate::core::types::token::unpack_sort_key;
use crate::ingestion::TokenCorpusArtifact;
use crate::ssa::SSAArtifact;
use crate::symbol::SymbolTableArtifact;
use crate::tra::backward::{
    ASTBackwardIndex, BlockBackwardIndex, CallSiteBackwardIndex, SSABackwardIndex, SymbolBackwardIndex,
};
use crate::tra::forward::{CallSiteSpanIndex, SymbolSpanIndex};
use crate::tra::types::TraceabilityArtifact;
use crate::tra::uml_link::{ScpgHashChain, UMLLinkRegistry};

pub struct TraceabilityArtifactBuilder;

impl TraceabilityArtifactBuilder {
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
        let bi_blk = BlockBackwardIndex::build(cfa);
        let bi_ssa = SSABackwardIndex::build(ssa, bpa, &bi_blk);
        let bi_cs = CallSiteBackwardIndex::build(cga);

        let sym_span = SymbolSpanIndex::build(&bi_sym, tca, sta);
        let cs_span = CallSiteSpanIndex::build(&bi_cs, tca, cga);
        let uml_links = UMLLinkRegistry::build(&bi_sym, tca, sta, hashes.scpg_hash);

        let artifact = TraceabilityArtifact {
            format_version: 1,
            hashes,
            bi_ast,
            bi_sym,
            bi_blk,
            bi_ssa,
            bi_cs,
            sym_span,
            cs_span,
            uml_links,
        };

        Self::verify_invariants(&artifact, sta, tca);
        artifact
    }

    pub fn verify_invariants(tra: &TraceabilityArtifact, sta: &SymbolTableArtifact, tca: &TokenCorpusArtifact) {
        // Invariant 1 (BI_blk Global Completeness): Every basic block has a valid entry
        for (i, entry) in tra.bi_blk.iter().enumerate() {
            assert!(
                entry.first_token_id != u32::MAX,
                "Invariant 1 Violated: BI_blk[{}] first_tok is u32::MAX",
                i
            );
        }

        // Invariant 2 (UMLLink Symbol Coverage): Every UML symbol has a UMLLinkRecord
        let uml_kinds: &[u8] = &[
            SymbolKind::SK_CLASS as u8,
            SymbolKind::SK_INTERFACE as u8,
            SymbolKind::SK_ENUM as u8,
            SymbolKind::SK_RECORD as u8,
            SymbolKind::SK_ANNOTATION_TYPE as u8,
            SymbolKind::SK_METHOD as u8,
            SymbolKind::SK_CONSTRUCTOR as u8,
            SymbolKind::SK_FIELD as u8,
            SymbolKind::SK_ENUM_CONSTANT as u8,
        ];
        let expected_uml_count = sta.symbol_records
            .iter()
            .filter(|sym| uml_kinds.contains(&sym.kind))
            .count();
        assert_eq!(
            tra.uml_links.len(),
            expected_uml_count,
            "Invariant 2 Violated: UMLLink count mismatch"
        );

        // Invariant 3 (Hash Chain Validity): Composite scpg_hash check
        let expected_scpg_hash = ScpgHashChain::compute_composite(
            tra.hashes.tca_hash,
            tra.hashes.bpa_hash,
            tra.hashes.sta_hash,
            tra.hashes.cfa_hash,
            tra.hashes.ssa_hash,
            tra.hashes.cga_hash,
        );
        assert_eq!(
            tra.hashes.scpg_hash, expected_scpg_hash,
            "Invariant 3 Violated: scpg_hash mismatch"
        );

        // Invariant 4 (Backward-Forward Roundtrip): Spot-check 5% of symbols
        let total_syms = sta.symbol_records.len();
        let check_count = (total_syms / 20).max(1).min(total_syms);
        for s in 0..check_count {
            let entry = &tra.bi_sym[s];
            if entry.decl_first_tok != u32::MAX && (entry.decl_first_tok as usize) < tca.token_records.len() {
                let start_tok = &tca.token_records[entry.decl_first_tok as usize];
                let (file_id, _, _) = unpack_sort_key(start_tok.sort_key);
                let forward_res = SymbolSpanIndex::forward_sym_query(
                    entry.decl_first_tok,
                    file_id,
                    &tra.sym_span,
                );
                assert!(
                    forward_res.contains(&(s as u32)),
                    "Invariant 4 Violated: symbol {} not found in forward query result",
                    s
                );
            }
        }

        log_info("Phase 7 Invariants 1-4 asserted successfully.");
    }
}
