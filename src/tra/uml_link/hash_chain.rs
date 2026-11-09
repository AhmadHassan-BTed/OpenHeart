//! SCPG Fingerprint Hash Chain (§7.2.3, §7.5.3, Invariant 3).
//!
//! ```text
//! dᵢ = SHA-256(canonical content of artifact i)        i ∈ TCA, BPA, STA, CFA, SSA, CGA
//! h₀ = SHA-256("openheart.scpg.chain.v2")
//! hᵢ = SHA-256(hᵢ₋₁ ‖ labelᵢ ‖ dᵢ)                    chain_digest = h₆ (256 bits)
//! ```
//!
//! Each link depends on every artifact before it, so the chain identifies the whole build
//! and any single artifact change is detected. Verification recomputes the chain from the
//! stored per-artifact digests (on load and in Invariant 3) and, when the artifacts are at
//! hand, recomputes the per-artifact digests themselves ([`ScpgHashChain::verify_against`]).

use sha2::{Digest, Sha256};

use crate::ast::BPASTArtifact;
use crate::cfg::serializer::CFGArtifact;
use crate::core::types::cg::CallGraphArtifact;
use crate::ingestion::TokenCorpusArtifact;
use crate::ssa::SSAArtifact;
use crate::symbol::SymbolTableArtifact;
use crate::tra::types::ScpgHashChain;
use crate::tra::uml_link::digest::{
    digest_bpa, digest_cfa, digest_cga, digest_prefix_u32, digest_prefix_u64, digest_ssa,
    digest_sta, digest_tca, Digest256,
};

/// Chain domain separator.
const CHAIN_DOMAIN: &[u8] = b"openheart.scpg.chain.v2";

/// Artifact labels in chain order.
pub const CHAIN_LABELS: [&str; 6] = ["tca", "bpa", "sta", "cfa", "ssa", "cga"];

impl ScpgHashChain {
    /// Digest all six artifacts and chain them.
    pub fn compute(
        tca: &TokenCorpusArtifact,
        bpa: &BPASTArtifact,
        sta: &SymbolTableArtifact,
        cfa: &CFGArtifact,
        ssa: &SSAArtifact,
        cga: &CallGraphArtifact,
    ) -> Self {
        Self::from_artifact_digests(Self::digest_artifacts(tca, bpa, sta, cfa, ssa, cga))
    }

    /// The six canonical artifact digests in chain order.
    pub fn digest_artifacts(
        tca: &TokenCorpusArtifact,
        bpa: &BPASTArtifact,
        sta: &SymbolTableArtifact,
        cfa: &CFGArtifact,
        ssa: &SSAArtifact,
        cga: &CallGraphArtifact,
    ) -> [Digest256; 6] {
        [
            digest_tca(tca),
            digest_bpa(bpa),
            digest_sta(sta),
            digest_cfa(cfa),
            digest_ssa(ssa),
            digest_cga(cga),
        ]
    }

    /// Build the chain record from per-artifact digests.
    pub fn from_artifact_digests(artifact_digests: [Digest256; 6]) -> Self {
        let chain_digest = Self::chain(&artifact_digests);
        ScpgHashChain {
            tca_hash: digest_prefix_u64(&artifact_digests[0]),
            bpa_hash: digest_prefix_u64(&artifact_digests[1]),
            sta_hash: digest_prefix_u64(&artifact_digests[2]),
            cfa_hash: digest_prefix_u64(&artifact_digests[3]),
            ssa_hash: digest_prefix_u64(&artifact_digests[4]),
            cga_hash: digest_prefix_u64(&artifact_digests[5]),
            scpg_hash: digest_prefix_u32(&chain_digest),
            artifact_digests,
            chain_digest,
        }
    }

    /// `h₆` of the chain over `digests`.
    pub fn chain(digests: &[Digest256; 6]) -> Digest256 {
        let mut link: Digest256 = Sha256::digest(CHAIN_DOMAIN).into();
        for (label, d) in CHAIN_LABELS.iter().zip(digests.iter()) {
            let mut h = Sha256::new();
            h.update(link);
            h.update((label.len() as u64).to_le_bytes());
            h.update(label.as_bytes());
            h.update(d);
            link = h.finalize().into();
        }
        link
    }

    /// Check that the stored chain digest, tags and build hash all follow from the stored
    /// per-artifact digests.
    pub fn verify(&self) -> Result<(), String> {
        let expected = Self::from_artifact_digests(self.artifact_digests);
        if expected.chain_digest != self.chain_digest {
            return Err("chain digest does not match the per-artifact digests".to_string());
        }
        if expected != *self {
            return Err("hash tags do not match the per-artifact digests".to_string());
        }
        Ok(())
    }

    /// Check the chain against the actual artifacts: every per-artifact digest is
    /// recomputed. Returns the labels of the artifacts whose content changed.
    pub fn verify_against(
        &self,
        tca: &TokenCorpusArtifact,
        bpa: &BPASTArtifact,
        sta: &SymbolTableArtifact,
        cfa: &CFGArtifact,
        ssa: &SSAArtifact,
        cga: &CallGraphArtifact,
    ) -> Result<(), Vec<&'static str>> {
        self.verify().map_err(|_| CHAIN_LABELS.to_vec())?;
        let actual = Self::digest_artifacts(tca, bpa, sta, cfa, ssa, cga);
        let changed: Vec<&'static str> = CHAIN_LABELS
            .iter()
            .zip(actual.iter().zip(self.artifact_digests.iter()))
            .filter(|(_, (a, s))| a != s)
            .map(|(label, _)| *label)
            .collect();
        if changed.is_empty() {
            Ok(())
        } else {
            Err(changed)
        }
    }
}
