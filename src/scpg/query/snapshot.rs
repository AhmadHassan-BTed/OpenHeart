//! ScpgSnapshot — immutable, query-ready view of one analysis build (§10.4, §10.5).
//!
//! A snapshot is built once from the nine Phase 1-9 artifacts, or decoded from a `.scpg`
//! file without re-running any phase. Every section of the `.scpg` file is the columnar
//! encoding of one snapshot index (the `Cfg` section embeds the canonical `.cfa` bytes), so
//! a reopened file answers exactly the same queries as the in-memory build. The snapshot is
//! never mutated: engines share it behind an `Arc`, and incremental updates produce a new one.

use serde::Serialize;

use crate::ast::BPASTArtifact;
use crate::cfg::serializer::CFGArtifact;
use crate::core::types::cg::CallGraphArtifact;
use crate::core::types::token::TokenType;
use crate::ingestion::TokenCorpusArtifact;
use crate::psa::types::PathSummaryArtifact;
use crate::scpg::query::graph::CallGraphIndex;
use crate::scpg::query::navigation::NavigationIndex;
use crate::scpg::query::robdd::PathSummaryIndex;
use crate::scpg::query::sdg::SystemDependenceGraph;
use crate::scpg::query::semantic::SemanticIndex;
use crate::scpg::query::source::{AstIndex, FileTable, TokenIndex};
use crate::scpg::query::symbols::{
    is_callable_kind, is_type_kind, SymbolIndex, TypeHierarchyIndex,
};
use crate::scpg::serializer::codec::{SectionDecoder, SectionEncoder};
use crate::scpg::serializer::integrity::{
    short_hash, source_tree_hash, ContentDigestBuilder, Fingerprinter,
};
use crate::scpg::serializer::layout::SectionLayoutPlanner;
use crate::scpg::types::{SCPGSectionType, ScpgDigest, ScpgFormatError};
use crate::ssa::serializer::SSAArtifact;
use crate::symbol::SymbolTableArtifact;
use crate::tra::types::TraceabilityArtifact;
use crate::uma::types::UMLMetadataArtifact;

/// Section payloads in on-disk order plus their content digest.
pub struct EncodedSections {
    pub sections: Vec<(SCPGSectionType, Vec<u8>)>,
    pub digest: ScpgDigest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SnapshotStats {
    pub files: usize,
    pub tokens: usize,
    pub ast_nodes: usize,
    pub symbols: usize,
    pub type_edges: usize,
    pub methods: usize,
    pub call_edges: usize,
    pub call_sites: usize,
    pub sdg_nodes: usize,
    pub sdg_edges: usize,
    pub navigable_entities: usize,
    pub path_summaries: usize,
    pub uml_classes: usize,
    pub pattern_instances: usize,
}

#[derive(Debug, Clone)]
pub struct ScpgSnapshot {
    pub digest: ScpgDigest,
    pub scpg_hash: u32,
    pub source_hash: [u8; 32],
    pub files: FileTable,
    pub tokens: TokenIndex,
    pub ast: AstIndex,
    pub symbols: SymbolIndex,
    pub hierarchy: TypeHierarchyIndex,
    pub call_graph: CallGraphIndex,
    pub sdg: SystemDependenceGraph,
    pub navigation: NavigationIndex,
    pub paths: PathSummaryIndex,
    pub semantic: SemanticIndex,
}

impl ScpgSnapshot {
    #[allow(clippy::too_many_arguments)]
    pub fn from_artifacts(
        tca: &TokenCorpusArtifact,
        bpa: &BPASTArtifact,
        sta: &SymbolTableArtifact,
        cfa: &CFGArtifact,
        ssa: &SSAArtifact,
        cga: &CallGraphArtifact,
        tra: &TraceabilityArtifact,
        uma: &UMLMetadataArtifact,
        psa: &PathSummaryArtifact,
    ) -> Self {
        Self::build(tca, bpa, sta, cfa, ssa, cga, tra, uma, psa).0
    }

    /// Builds the snapshot and its encoded sections in one pass (the digest is defined over
    /// the encoded bytes, so encoding is part of building).
    #[allow(clippy::too_many_arguments)]
    pub fn build(
        tca: &TokenCorpusArtifact,
        bpa: &BPASTArtifact,
        sta: &SymbolTableArtifact,
        cfa: &CFGArtifact,
        ssa: &SSAArtifact,
        cga: &CallGraphArtifact,
        tra: &TraceabilityArtifact,
        uma: &UMLMetadataArtifact,
        psa: &PathSummaryArtifact,
    ) -> (Self, EncodedSections) {
        let files = FileTable::from_tca(tca);
        let tokens = TokenIndex::from_tca(tca);
        let ast = AstIndex::from_bpa(bpa);
        let mut symbols = SymbolIndex::from_artifacts(sta, tca);
        let hierarchy = TypeHierarchyIndex::from_artifacts(sta);
        let call_graph = CallGraphIndex::build(cga);
        let sdg = SystemDependenceGraph::build(cfa, ssa, &call_graph, &ast);
        let navigation = NavigationIndex::build(tra, &tokens, symbols.len());
        let paths = PathSummaryIndex::from_psa(psa);
        let semantic = SemanticIndex::from_uma(uma);
        let (signature_fps, body_fps) =
            compute_fingerprints(tca, tra, &symbols, &hierarchy, &call_graph);
        symbols.set_fingerprints(signature_fps, body_fps);

        let mut snapshot = Self {
            digest: [0u8; 32],
            scpg_hash: 0,
            source_hash: source_tree_hash(&tca.file_records),
            files,
            tokens,
            ast,
            symbols,
            hierarchy,
            call_graph,
            sdg,
            navigation,
            paths,
            semantic,
        };
        let encoded = snapshot.encode_sections(&cfa.serialize());
        snapshot.digest = encoded.digest;
        snapshot.scpg_hash = short_hash(&encoded.digest);
        (snapshot, encoded)
    }

    /// Encodes every section in hot -> warm -> cold order and computes the content digest.
    pub fn encode_sections(&self, cfg_canonical: &[u8]) -> EncodedSections {
        let mut sections = Vec::with_capacity(SectionLayoutPlanner::ordered_sections().len());
        let mut digest = ContentDigestBuilder::new();
        for &ty in SectionLayoutPlanner::ordered_sections() {
            let mut enc = SectionEncoder::new(ty);
            match ty {
                SCPGSectionType::TokenTable => self.tokens.encode(&mut enc),
                SCPGSectionType::StringTable => self.files.encode(&mut enc),
                SCPGSectionType::Traceability => self.navigation.encode(&mut enc),
                SCPGSectionType::SymbolTable => self.symbols.encode(&mut enc),
                SCPGSectionType::TypeHierarchy => self.hierarchy.encode(&mut enc),
                SCPGSectionType::SemanticMetadata => self.semantic.encode(&mut enc),
                SCPGSectionType::CallGraph => self.call_graph.encode(&mut enc),
                SCPGSectionType::BpAst => self.ast.encode(&mut enc),
                SCPGSectionType::Cfg => enc.put_u8s(cfg_canonical),
                SCPGSectionType::SsaDfg => self.sdg.encode(&mut enc),
                SCPGSectionType::PathSummaries => self.paths.encode(&mut enc),
            }
            let payload = enc.finish();
            digest.section(ty, &payload);
            sections.push((ty, payload));
        }
        EncodedSections {
            sections,
            digest: digest.finish(),
        }
    }

    /// Decodes a snapshot from verified section payloads. `section(ty)` must return the
    /// checksum-verified payload of `ty`.
    pub fn decode<'a>(
        section: impl Fn(SCPGSectionType) -> Result<&'a [u8], ScpgFormatError>,
        digest: ScpgDigest,
        source_hash: [u8; 32],
    ) -> Result<Self, ScpgFormatError> {
        fn one<'b, T>(
            ty: SCPGSectionType,
            bytes: &'b [u8],
            f: impl FnOnce(&mut SectionDecoder<'b>) -> Result<T, ScpgFormatError>,
        ) -> Result<T, ScpgFormatError> {
            let mut dec = SectionDecoder::new(ty, bytes)?;
            let value = f(&mut dec)?;
            dec.finish()?;
            Ok(value)
        }

        let symbols = one(
            SCPGSectionType::SymbolTable,
            section(SCPGSectionType::SymbolTable)?,
            SymbolIndex::decode,
        )?;
        let n = symbols.len();
        let tokens = one(
            SCPGSectionType::TokenTable,
            section(SCPGSectionType::TokenTable)?,
            TokenIndex::decode,
        )?;
        let files = one(
            SCPGSectionType::StringTable,
            section(SCPGSectionType::StringTable)?,
            FileTable::decode,
        )?;
        let navigation = one(
            SCPGSectionType::Traceability,
            section(SCPGSectionType::Traceability)?,
            |d| NavigationIndex::decode(d, n),
        )?;
        let hierarchy = one(
            SCPGSectionType::TypeHierarchy,
            section(SCPGSectionType::TypeHierarchy)?,
            |d| TypeHierarchyIndex::decode(d, n),
        )?;
        let semantic = one(
            SCPGSectionType::SemanticMetadata,
            section(SCPGSectionType::SemanticMetadata)?,
            SemanticIndex::decode,
        )?;
        let call_graph = one(
            SCPGSectionType::CallGraph,
            section(SCPGSectionType::CallGraph)?,
            CallGraphIndex::decode,
        )?;
        let ast = one(
            SCPGSectionType::BpAst,
            section(SCPGSectionType::BpAst)?,
            AstIndex::decode,
        )?;
        // The canonical CFG is decoded on demand (`cfg_from_section`); only its framing is
        // validated here.
        one(SCPGSectionType::Cfg, section(SCPGSectionType::Cfg)?, |d| {
            d.u8s().map(|_| ())
        })?;
        let sdg = one(
            SCPGSectionType::SsaDfg,
            section(SCPGSectionType::SsaDfg)?,
            SystemDependenceGraph::decode,
        )?;
        let paths = one(
            SCPGSectionType::PathSummaries,
            section(SCPGSectionType::PathSummaries)?,
            PathSummaryIndex::decode,
        )?;

        Ok(Self {
            digest,
            scpg_hash: short_hash(&digest),
            source_hash,
            files,
            tokens,
            ast,
            symbols,
            hierarchy,
            call_graph,
            sdg,
            navigation,
            paths,
            semantic,
        })
    }

    /// Extracts and parses the canonical `.cfa` bytes embedded in a `Cfg` section payload.
    pub fn cfg_from_section(payload: &[u8]) -> Result<CFGArtifact, ScpgFormatError> {
        let mut dec = SectionDecoder::new(SCPGSectionType::Cfg, payload)?;
        let bytes = dec.u8s()?;
        let cfa = CFGArtifact::deserialize(bytes)
            .map_err(|e| dec.malformed(format!("embedded .cfa: {}", e)))?;
        dec.finish()?;
        Ok(cfa)
    }

    pub fn stats(&self) -> SnapshotStats {
        SnapshotStats {
            files: self.files.len(),
            tokens: self.tokens.len(),
            ast_nodes: self.ast.len(),
            symbols: self.symbols.len(),
            type_edges: self.hierarchy.edge_count(),
            methods: self.call_graph.node_count(),
            call_edges: self.call_graph.edge_count(),
            call_sites: self.call_graph.call_sites().len(),
            sdg_nodes: self.sdg.node_count(),
            sdg_edges: self.sdg.edge_count(),
            navigable_entities: self.navigation.len(),
            path_summaries: self.paths.functions().len(),
            uml_classes: self.semantic.classes.len(),
            pattern_instances: self.semantic.patterns.len(),
        }
    }
}

fn token_text(tca: &TokenCorpusArtifact, token: u32) -> Option<(&[u8], u8)> {
    let (text_id, ty) = if !tca.token_entries.is_empty() {
        let e = tca.token_entries.get(token as usize)?;
        (e.text_id, e.token_type)
    } else {
        let r = tca.token_records.get(token as usize)?;
        (r.text_id, r.token_type)
    };
    Some((tca.interner.lookup_text(text_id), ty))
}

fn is_trivia_type(ty: u8) -> bool {
    matches!(
        TokenType::from_u8(ty),
        TokenType::CommentLine
            | TokenType::CommentBlock
            | TokenType::CommentDoc
            | TokenType::Whitespace
            | TokenType::Newline
    )
}

/// Feeds the non-trivia token texts of `[first, last]`, skipping `[skip_first, skip_last]`.
fn feed_tokens(
    fp: &mut Fingerprinter,
    tca: &TokenCorpusArtifact,
    range: Option<(u32, u32)>,
    skip: Option<(u32, u32)>,
) {
    let Some((first, last)) = range else {
        return;
    };
    let mut t = first;
    while t <= last {
        if let Some((s, e)) = skip {
            if t >= s && t <= e {
                if e == u32::MAX {
                    break;
                }
                t = e + 1;
                continue;
            }
        }
        match token_text(tca, t) {
            Some((text, ty)) if !is_trivia_type(ty) => fp.bytes(text),
            Some(_) => {}
            None => break,
        }
        if t == u32::MAX {
            break;
        }
        t += 1;
    }
}

fn valid_range(first: u32, last: u32) -> Option<(u32, u32)> {
    if first == u32::MAX || last == u32::MAX || last < first {
        None
    } else {
        Some((first, last))
    }
}

/// Per-symbol signature and body fingerprints for incremental change detection.
///
/// - signature: stable id, kind, modifiers, visibility, arity, the non-trivia tokens of the
///   declaration outside the definition body, and (for types) supertypes and member ids;
/// - body: the non-trivia tokens of the definition body and (for callables) the stable ids
///   of resolved callees.
///
/// Fingerprints are position-independent: inserting lines above a declaration, or editing
/// comments and whitespace, leaves them unchanged.
fn compute_fingerprints(
    tca: &TokenCorpusArtifact,
    tra: &TraceabilityArtifact,
    symbols: &SymbolIndex,
    hierarchy: &TypeHierarchyIndex,
    cg: &CallGraphIndex,
) -> (Vec<u64>, Vec<u64>) {
    let n = symbols.len();
    let mut signature = Vec::with_capacity(n);
    let mut body = Vec::with_capacity(n);
    for sym in 0..n as u32 {
        let (decl, def) = match tra.bi_sym.get(sym as usize) {
            Some(e) => (
                valid_range(e.decl_first_tok, e.decl_last_tok).or_else(|| symbols.token_span(sym)),
                valid_range(e.def_first_tok, e.def_last_tok),
            ),
            None => (symbols.token_span(sym), None),
        };
        // The body only counts as separate when it nests inside the declaration.
        let def = match (decl, def) {
            (Some((df, dl)), Some((bf, bl))) if df <= bf && bl <= dl && (bf, bl) != (df, dl) => {
                Some((bf, bl))
            }
            (None, Some(b)) => Some(b),
            _ => None,
        };
        let kind = symbols.kind(sym).unwrap_or(0);

        let mut sig = Fingerprinter::new(b"openheart.scpg.signature");
        sig.bytes(symbols.stable_id(sym).unwrap_or("").as_bytes());
        sig.u8(kind);
        sig.u16(symbols.modifiers(sym).unwrap_or(0));
        sig.u8(symbols.visibility(sym).unwrap_or(0));
        sig.u16(symbols.param_count(sym).unwrap_or(0));
        feed_tokens(&mut sig, tca, decl, def);
        if is_type_kind(kind) {
            let mut supers: Vec<&str> = hierarchy
                .supertypes(sym)
                .iter()
                .filter_map(|&s| symbols.stable_id(s))
                .collect();
            supers.sort_unstable();
            for s in supers {
                sig.bytes(s.as_bytes());
            }
            let mut members: Vec<&str> = symbols
                .children(sym)
                .iter()
                .filter_map(|&c| symbols.stable_id(c))
                .collect();
            members.sort_unstable();
            for m in members {
                sig.bytes(m.as_bytes());
            }
        }
        signature.push(sig.finish());

        let mut bfp = Fingerprinter::new(b"openheart.scpg.body");
        feed_tokens(&mut bfp, tca, def, None);
        if is_callable_kind(kind) {
            let mut callees: Vec<&str> = cg
                .callees(sym)
                .iter()
                .filter_map(|&c| symbols.stable_id(c))
                .collect();
            callees.sort_unstable();
            for c in callees {
                bfp.bytes(c.as_bytes());
            }
        }
        body.push(bfp.finish());
    }
    (signature, body)
}
