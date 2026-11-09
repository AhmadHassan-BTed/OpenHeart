//! Canonical content digests of the SCPG build artifacts (§7.2.3, §7.5.3, Invariant 3).
//!
//! Every digest is SHA-256 over a canonical, self-delimiting encoding of an artifact's
//! *content*: integers little-endian, every variable-length sequence prefixed by its length,
//! hash maps in key order, and a domain string per artifact kind. Excluded on purpose:
//!
//! - file modification times and path-table offsets (a `touch` or a fresh clone must not
//!   invalidate anything),
//! - the CRC "link" fields artifacts carry to their inputs (the hash chain links artifacts
//!   explicitly, and those CRCs cover volatile bytes such as mtimes),
//! - derived acceleration structures that are pure functions of hashed content (the BP
//!   rank/select, jump and RMQ tables are functions of the hashed parenthesis bits).
//!
//! These digests are the single hash definition shared by Phase 7 (hash chain), Phase 8
//! (`PathSummaryArtifact::ssa_hash`) and any consumer that needs to key data by build
//! content (the SCPG query cache should use [`ScpgHashChain::chain_digest`]).
//!
//! [`ScpgHashChain::chain_digest`]: crate::tra::types::ScpgHashChain

use std::collections::HashMap;

use sha2::{Digest, Sha256};

use crate::ast::BPASTArtifact;
use crate::cfg::serializer::CFGArtifact;
use crate::core::types::cg::CallGraphArtifact;
use crate::ingestion::TokenCorpusArtifact;
use crate::ssa::SSAArtifact;
use crate::symbol::SymbolTableArtifact;

/// A SHA-256 digest.
pub type Digest256 = [u8; 32];

/// A digest truncated to 128 bits (per-entity records).
pub type Digest128 = [u8; 16];

/// Bytes buffered before they are fed to SHA-256.
const FLUSH_THRESHOLD: usize = 1 << 16;

/// Streaming SHA-256 over a canonical encoding.
pub struct CanonicalHasher {
    inner: Sha256,
    buf: Vec<u8>,
}

impl CanonicalHasher {
    /// A hasher whose output is separated from every other domain by `domain`.
    pub fn new(domain: &str) -> Self {
        let mut h = Self {
            inner: Sha256::new(),
            buf: Vec::with_capacity(FLUSH_THRESHOLD),
        };
        h.str(domain);
        h
    }

    #[inline]
    fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
        if self.buf.len() >= FLUSH_THRESHOLD {
            self.inner.update(&self.buf);
            self.buf.clear();
        }
    }

    #[inline]
    pub fn u8(&mut self, v: u8) {
        self.push(&[v]);
    }

    #[inline]
    pub fn bool(&mut self, v: bool) {
        self.push(&[v as u8]);
    }

    #[inline]
    pub fn u16(&mut self, v: u16) {
        self.push(&v.to_le_bytes());
    }

    #[inline]
    pub fn u32(&mut self, v: u32) {
        self.push(&v.to_le_bytes());
    }

    #[inline]
    pub fn u64(&mut self, v: u64) {
        self.push(&v.to_le_bytes());
    }

    /// Length-prefixed byte string.
    pub fn bytes(&mut self, b: &[u8]) {
        self.u64(b.len() as u64);
        self.push(b);
    }

    /// Fixed-size bytes (no length prefix; the caller guarantees a fixed width).
    pub fn fixed(&mut self, b: &[u8]) {
        self.push(b);
    }

    /// Length-prefixed UTF-8 string.
    pub fn str(&mut self, s: &str) {
        self.bytes(s.as_bytes());
    }

    /// Length-prefixed `u32` sequence.
    pub fn u32s(&mut self, values: &[u32]) {
        self.u64(values.len() as u64);
        for &v in values {
            self.u32(v);
        }
    }

    /// Length-prefixed `u64` sequence.
    pub fn u64s(&mut self, values: &[u64]) {
        self.u64(values.len() as u64);
        for &v in values {
            self.u64(v);
        }
    }

    /// Sequence length marker (the caller then encodes the elements).
    pub fn seq_len(&mut self, n: usize) {
        self.u64(n as u64);
    }

    /// The SHA-256 digest of everything encoded so far.
    pub fn finish(mut self) -> Digest256 {
        if !self.buf.is_empty() {
            self.inner.update(&self.buf);
        }
        self.inner.finalize().into()
    }
}

/// First 64 bits of a digest, little-endian (short display / link tag).
pub fn digest_prefix_u64(d: &Digest256) -> u64 {
    let mut b = [0u8; 8];
    b.copy_from_slice(&d[..8]);
    u64::from_le_bytes(b)
}

/// First 32 bits of a digest, little-endian (short display / link tag).
pub fn digest_prefix_u32(d: &Digest256) -> u32 {
    let mut b = [0u8; 4];
    b.copy_from_slice(&d[..4]);
    u32::from_le_bytes(b)
}

/// First 128 bits of a digest.
pub fn digest_truncate_128(d: &Digest256) -> Digest128 {
    let mut b = [0u8; 16];
    b.copy_from_slice(&d[..16]);
    b
}

/// Canonical digest of the token corpus: per file its identity, language, flags, content
/// SHA-256, size and token range (no mtime, no path offset); per token its sort key,
/// length, type and text.
pub fn digest_tca(tca: &TokenCorpusArtifact) -> Digest256 {
    let mut h = CanonicalHasher::new("openheart.tca.v1");
    h.seq_len(tca.file_records.len());
    for f in &tca.file_records {
        h.u16(f.file_id);
        h.u8(f.language_id);
        h.u8(f.flags);
        h.fixed(&f.content_sha256);
        h.u64(f.file_size_bytes);
        h.u32(f.first_token_id);
        h.u32(f.file_token_count);
    }
    h.seq_len(tca.token_records.len());
    for t in &tca.token_records {
        h.u64(t.sort_key);
        h.u16(t.len);
        h.u8(t.token_type);
        h.bytes(tca.interner.lookup_text(t.text_id));
    }
    h.finish()
}

/// Canonical digest of the BP AST: node count, parenthesis bits and pre-order arrays.
pub fn digest_bpa(bpa: &BPASTArtifact) -> Digest256 {
    let mut h = CanonicalHasher::new("openheart.bpa.v1");
    h.u32(bpa.node_count);
    h.u64(bpa.bp_encoder.bit_count as u64);
    h.u64s(&bpa.bp_encoder.words);
    h.bytes(&bpa.preorder.node_types);
    h.u32s(&bpa.preorder.node_attrs);
    h.seq_len(bpa.preorder.token_ranges.len());
    for &(a, b) in &bpa.preorder.token_ranges {
        h.u32(a);
        h.u32(b);
    }
    h.u32s(&bpa.preorder.parent_map);
    h.finish()
}

/// Canonical digest of the symbol table: symbol, name-index, scope, type-hierarchy and
/// association records, qualified names and package-name maps (in key order).
pub fn digest_sta(sta: &SymbolTableArtifact) -> Digest256 {
    let mut h = CanonicalHasher::new("openheart.sta.v1");
    h.seq_len(sta.symbol_records.len());
    for s in &sta.symbol_records {
        h.u32(s.symbol_id);
        h.u32(s.name_id);
        h.u32(s.qual_name_id);
        h.u32(s.type_id);
        h.u32(s.decl_node);
        h.u32(s.def_node);
        h.u32(s.parent_sym);
        h.u32(s.first_child);
        h.u32(s.next_sibling);
        h.u32(s.scope_id);
        h.u32(s.uml_meta_offset);
        h.u16(s.param_count);
        h.u16(s.modifiers);
        h.u8(s.kind);
        h.u8(s.visibility);
        h.u8(s.type_param_count);
        h.u8(s.flags);
        h.u32(s.first_token_id);
        h.u32(s.last_token_id);
    }
    h.seq_len(sta.name_index.len());
    for &(name, sym) in &sta.name_index {
        h.u32(name);
        h.u32(sym);
    }
    h.seq_len(sta.scope_records.len());
    for s in &sta.scope_records {
        h.u32(s.scope_id);
        h.u32(s.parent_scope);
        h.u32(s.owner_symbol);
        h.u32(s.first_decl);
        h.u32(s.decl_count);
        h.u16(s.import_count);
        h.u8(s.scope_kind);
        h.u8(s.flags);
        h.u32(s.import_table_off);
    }
    h.seq_len(sta.th_edges.len());
    for e in &sta.th_edges {
        h.u32(e.from_sym);
        h.u32(e.to_sym);
        h.u8(e.relation as u8);
    }
    h.seq_len(sta.associations.len());
    for a in &sta.associations {
        h.u32(a.from_symbol_id);
        h.u32(a.to_symbol_id);
        h.u32(a.field_symbol_id);
        h.u8(a.assoc_kind);
        h.u16(a.mult_min);
        h.u16(a.mult_max);
        h.u8(a.is_navigable);
        h.u32(a.role_name_id);
    }
    h.seq_len(sta.qual_names.len());
    for q in &sta.qual_names {
        h.str(q);
    }
    hash_sorted_map(&mut h, &sta.custom_package_names, |h, k| h.u32(*k));
    hash_sorted_map(&mut h, &sta.file_package_names, |h, k| h.u16(*k));
    h.finish()
}

/// Encode a `HashMap<K, String>` in key order.
fn hash_sorted_map<K: Ord>(
    h: &mut CanonicalHasher,
    map: &HashMap<K, String>,
    mut key: impl FnMut(&mut CanonicalHasher, &K),
) {
    let mut entries: Vec<(&K, &String)> = map.iter().collect();
    entries.sort_by(|a, b| a.0.cmp(b.0));
    h.seq_len(entries.len());
    for (k, v) in entries {
        key(h, k);
        h.str(v);
    }
}

/// Canonical digest of the CFG artifact: every function's blocks (with statements and token
/// ranges), typed edges, CSR adjacency, dominator data, loop data and V(G).
pub fn digest_cfa(cfa: &CFGArtifact) -> Digest256 {
    let mut h = CanonicalHasher::new("openheart.cfa.v1");
    h.u32(cfa.total_blocks);
    h.u32(cfa.total_edges);
    h.seq_len(cfa.functions.len());
    for f in &cfa.functions {
        h.u32(f.sym_id);
        h.seq_len(f.blocks.len());
        for b in &f.blocks {
            h.u32(b.id);
            h.u32s(&b.stmts);
            h.u32(b.first_token);
            h.u32(b.last_token);
            h.bool(b.is_entry);
            h.bool(b.is_exit);
        }
        h.seq_len(f.edges.len());
        for &(u, v, t) in &f.edges {
            h.u32(u);
            h.u32(v);
            h.u8(t as u8);
        }
        h.u32s(&f.succ_offsets);
        h.u32s(&f.succ_adj);
        h.u32s(&f.pred_offsets);
        h.u32s(&f.pred_adj);
        h.bytes(&f.edge_types);
        h.u32s(&f.idom);
        h.u32s(&f.df_offsets);
        h.u32s(&f.df_adj);
        h.seq_len(f.loop_info.back_edges.len());
        for &(t, hd) in &f.loop_info.back_edges {
            h.u32(t);
            h.u32(hd);
        }
        h.bytes(&f.loop_info.loop_depth);
        h.u8(f.loop_info.max_loop_depth);
        h.u16(f.cyclomatic);
    }
    h.finish()
}

/// Canonical digest of the SSA artifact: every function's SSA records, φ records, def-use
/// and control-dependence CSRs and IFDS facts.
pub fn digest_ssa(ssa: &SSAArtifact) -> Digest256 {
    let mut h = CanonicalHasher::new("openheart.ssa.v1");
    h.u32(ssa.total_ssa_vars);
    h.u32(ssa.total_phi_funcs);
    h.seq_len(ssa.functions.len());
    for f in &ssa.functions {
        h.u32(f.sym_id);
        h.seq_len(f.ssa_records.len());
        for r in &f.ssa_records {
            h.u32(r.ssa_id);
            h.u32(r.orig_sym_id);
            h.u32(r.def_stmt);
            h.u16(r.version);
            h.u8(r.flags);
            h.u8(r.def_block);
        }
        h.seq_len(f.phi_records.len());
        for p in &f.phi_records {
            h.u32(p.ssa_id);
            h.u32(p.block_id);
            h.u32(p.orig_sym_id);
            h.seq_len(p.args.len());
            for a in &p.args {
                h.u32(a.pred_block_id);
                h.u32(a.arg_ssa_id);
            }
        }
        h.u32s(&f.def_use.def_offsets);
        h.u32s(&f.def_use.use_adj);
        h.u32s(&f.cdg.cd_offsets);
        h.u32s(&f.cdg.cd_adj);
        h.bytes(&f.cdg.cd_types);
        h.seq_len(f.ifds.taint_sparse.len());
        for &(id, blk) in &f.ifds.taint_sparse {
            h.u32(id);
            h.u16(blk);
        }
        h.u32s(&f.ifds.nullable_sparse);
        h.seq_len(f.ifds.type_state_sparse.len());
        for &(id, st) in &f.ifds.type_state_sparse {
            h.u32(id);
            h.u16(st);
        }
    }
    h.finish()
}

/// Canonical digest of the call graph: call sites, callee/caller CSRs, site map,
/// points-to table and SCCs.
pub fn digest_cga(cga: &CallGraphArtifact) -> Digest256 {
    let mut h = CanonicalHasher::new("openheart.cga.v1");
    h.u32(cga.method_count);
    h.u32(cga.call_site_count);
    h.u32(cga.call_edge_count);
    h.seq_len(cga.call_sites.len());
    for c in &cga.call_sites {
        h.u32(c.call_site_id);
        h.u32(c.caller_sym);
        h.u32(c.call_node);
        h.u32(c.receiver_ssa);
        h.u32(c.call_block);
        h.u32(c.call_token);
        h.u8(c.call_type);
        h.u8(c.flags);
        h.u16(c.arg_count);
    }
    for csr in [&cga.callee_csr, &cga.caller_csr] {
        h.u32s(&csr.offsets);
        h.u32s(&csr.adj);
        h.bytes(&csr.edge_types);
    }
    h.seq_len(cga.site_to_edge_map.len());
    for &(caller, callee, site) in &cga.site_to_edge_map {
        h.u32(caller);
        h.u32(callee);
        h.u32(site);
    }
    h.seq_len(cga.points_to_table.len());
    for p in &cga.points_to_table {
        h.u32(p.ssa_id);
        h.u32(p.alloc_type_sym_id);
    }
    h.seq_len(cga.sccs.len());
    for s in &cga.sccs {
        h.u32(s.scc_id);
        h.u32(s.member_offset);
        h.u16(s.member_count);
        h.u8(s.scc_class);
    }
    h.u32s(&cga.scc_members);
    h.finish()
}
