//! Phase 6: Inter-procedural Call Graph & Points-To Analysis Module.
//! Authored solely by Ahmad Hassan (B-Ted).
//!
//! Pipeline: program index -> per-function contexts (variable resolution, statement and
//! SSA maps) -> call-site extraction -> compile-time selection + CHA/RTA dispatch ->
//! Andersen points-to over global SSA values -> receiver-type refinement of virtual
//! calls -> CSR adjacency -> Tarjan SCCs over the call-graph nodes.
//!
//! Artifact conventions:
//! * `method_count` is the size of the symbol id space: the caller / callee CSRs are
//!   indexed by symbol id, and non-callable symbols have empty rows;
//! * SCCs cover only call-graph nodes (callables, plus classes owning field-initializer
//!   calls) and are emitted callee-first (see `scc.rs`);
//! * `CallSite::receiver_ssa` and `PointsToEntry::ssa_id` are **global** SSA ids
//!   (`SSAArtifact::global_ssa_base()[f] + local id`); `u32::MAX` means none;
//! * `CallSite::call_block` is `u32::MAX` when the call is not evaluated by a CFG block
//!   (initializer code); `PointsToEntry::alloc_type_sym_id == u32::MAX` denotes an array
//!   or an allocation whose type did not resolve to a single symbol.

pub mod call_sites;
pub mod index;
pub mod points_to;
pub mod resolution;
pub mod scc;
pub mod serializer;

pub use call_sites::*;
pub use index::*;
pub use points_to::*;
pub use resolution::*;
pub use scc::*;
pub use serializer::*;

use crate::ast::BPASTArtifact;
use crate::cfg::parallel::run_on_large_stack;
use crate::cfg::serializer::CFGArtifact;
use crate::cfg::source::SourceText;
use crate::core::logger::{log_info, log_trace, log_warn, PhaseTimer};
use crate::core::types::*;
use crate::ingestion::serializer::{crc64_ecma, TokenCorpusArtifact};
use crate::ssa::names::local_symbol_index;
use crate::ssa::SSAArtifact;
use crate::symbol::SymbolTableArtifact;

use std::collections::HashSet;
use std::path::Path;

pub struct Phase6Stage;

/// Builds a CSR keyed by `0..n` from `(row, value, label)` triples sorted by row.
fn csr_from_sorted(n: usize, rows: &[(u32, u32, u8)], with_labels: bool) -> CGCSR {
    let mut offsets = vec![0u32; n + 1];
    for &(r, _, _) in rows {
        if (r as usize) < n {
            offsets[r as usize + 1] += 1;
        }
    }
    for i in 0..n {
        offsets[i + 1] += offsets[i];
    }
    let mut adj = Vec::with_capacity(rows.len());
    let mut edge_types = Vec::with_capacity(if with_labels { rows.len() } else { 0 });
    for &(r, v, et) in rows {
        if (r as usize) < n {
            adj.push(v);
            if with_labels {
                edge_types.push(et);
            }
        }
    }
    CGCSR {
        offsets,
        adj,
        edge_types,
    }
}

impl Phase6Stage {
    /// Runs Phase 6 without token text: call names, receivers and arguments cannot be
    /// recovered, so only lambda-creation edges are produced. Prefer
    /// [`Phase6Stage::run_with_source`].
    pub fn run(
        bpa: &BPASTArtifact,
        sta: &SymbolTableArtifact,
        cfa: &CFGArtifact,
        ssa: &SSAArtifact,
        ssa_bytes: &[u8],
        sta_bytes: &[u8],
        out_path: &Path,
    ) -> Result<CallGraphArtifact, String> {
        Self::run_with_source(bpa, sta, None, cfa, ssa, ssa_bytes, sta_bytes, out_path)
    }

    /// Execute Phase 6 Inter-procedural Call Graph & Points-To Analysis
    #[allow(clippy::too_many_arguments)]
    pub fn run_with_source(
        bpa: &BPASTArtifact,
        sta: &SymbolTableArtifact,
        tca: Option<&TokenCorpusArtifact>,
        cfa: &CFGArtifact,
        ssa: &SSAArtifact,
        ssa_bytes: &[u8],
        sta_bytes: &[u8],
        out_path: &Path,
    ) -> Result<CallGraphArtifact, String> {
        let timer =
            PhaseTimer::start("Phase 6: Inter-procedural Call Graph & Points-To Analysis");

        let ssa_hash = crc64_ecma(ssa_bytes);
        let sta_hash = crc64_ecma(sta_bytes);
        log_info(&format!(
            "SSA Link Hash computed: {:#018X} | STA Link Hash: {:#018X}",
            ssa_hash, sta_hash
        ));
        if tca.is_none() {
            log_warn(
                "Phase 6 running without token text: method names cannot be recovered; only lambda edges are built.",
            );
        }

        // The analysis recurses over expression nesting; run it on a large stack.
        let artifact = run_on_large_stack(|| Self::analyze(bpa, sta, tca, cfa, ssa, ssa_hash, sta_hash))?;

        // Step 7: Assert Invariants (§6.8)
        log_trace("Asserting Phase 6 Invariants 1-3...");
        Self::assert_invariants(&artifact, bpa, cfa)?;

        // Step 8: Serialize Artifact
        log_info(&format!(
            "Serializing CallGraphArtifact (.cga) to {}...",
            out_path.display()
        ));
        CGASerializer::serialize(&artifact, out_path)
            .map_err(|e| format!("Failed to serialize CGA artifact: {}", e))?;

        let summary = format!(
            "Constructed Call Graph with {} call sites, {} call edges, {} SCCs ({} recursive), {} points-to entries.",
            artifact.call_site_count,
            artifact.call_edge_count,
            artifact.sccs.len(),
            artifact.sccs.iter().filter(|s| s.scc_class != SCC_CLASS_NON_RECURSIVE).count(),
            artifact.points_to_table.len()
        );
        log_info(&format!("Phase 6 Complete: {}", summary));
        timer.finish(&summary);

        Ok(artifact)
    }

    fn analyze(
        bpa: &BPASTArtifact,
        sta: &SymbolTableArtifact,
        tca: Option<&TokenCorpusArtifact>,
        cfa: &CFGArtifact,
        ssa: &SSAArtifact,
        ssa_hash: u64,
        sta_hash: u64,
    ) -> Result<CallGraphArtifact, String> {
        let src = tca.map(SourceText::new);
        let index = ProgramIndex::build(bpa, sta, src);
        let sym_by_decl = local_symbol_index(sta);

        // Step 1: per-function contexts and call sites
        let (contexts, ctx_of_fn) = build_contexts(&index, cfa, ssa, &sym_by_decl)?;
        let infos = extract_call_sites(&index, &contexts, &ctx_of_fn);
        log_info(&format!("Extracted {} call sites across AST.", infos.len()));

        // Step 2: compile-time selection + CHA / RTA dispatch
        let instantiated = DispatchResolver::instantiated_classes(&index);
        let mut resolver = DispatchResolver::new(&index, instantiated);
        let mut resolutions: Vec<Resolution> = infos
            .iter()
            .map(|site| resolver.resolve(site, &contexts))
            .collect();

        // Step 3: Andersen points-to over global SSA values
        let site_targets: Vec<Vec<u32>> = resolutions.iter().map(|r| r.targets.clone()).collect();
        let pta = AndersonPointsTo::run(&PointsToInput {
            index: &index,
            cfa,
            ssa,
            contexts: &contexts,
            ctx_of_fn: &ctx_of_fn,
            sites: &infos,
            site_targets: &site_targets,
        });
        log_info(&format!(
            "Andersen points-to complete: {} pointer variables, {} abstract objects.",
            pta.pts.len(),
            pta.alloc_sites.len()
        ));

        // Step 4: refine virtual calls whose receiver points only to known allocations
        let mut refined = 0usize;
        for (info, res) in infos.iter().zip(resolutions.iter_mut()) {
            let (selector, local, c) = match (res.selector, info.receiver_local_ssa, info.ctx) {
                (Some(s), Some(l), Some(c)) => (s, l, c),
                _ => continue,
            };
            let base = contexts[c].ssa_base;
            if base == u32::MAX {
                continue;
            }
            if let Some(types) = pta.pts.get(&(base + local)) {
                if types.is_empty() || types.contains(&u32::MAX) {
                    continue;
                }
                let mut ts: Vec<u32> = types.iter().copied().collect();
                ts.sort_unstable();
                let narrowed = resolver.dispatch_on(&ts, selector);
                if !narrowed.is_empty() && narrowed.len() < res.targets.len() {
                    res.targets = narrowed;
                    refined += 1;
                }
            }
        }
        log_info(&format!(
            "Points-to refinement narrowed {} virtual call sites.",
            refined
        ));

        // Step 5: call-site records and edges
        let mut call_sites = Vec::with_capacity(infos.len());
        let mut edges: Vec<(u32, u32, u32, u8)> = Vec::new();
        for (i, (info, res)) in infos.iter().zip(resolutions.iter()).enumerate() {
            let site_id = u32::try_from(i).map_err(|_| "call-site count exceeds the u32 range")?;
            let ctx = info.ctx.map(|c| &contexts[c]);
            let receiver_ssa = match (info.receiver_local_ssa, ctx) {
                (Some(l), Some(c)) if c.ssa_base != u32::MAX => c.ssa_base + l,
                _ => u32::MAX,
            };
            let mut flags = 0u8;
            if res.targets.len() > 1 {
                flags |= CALL_SITE_FLAG_IS_POLYMORPHIC;
            }
            if let (Some(l), Some(c)) = (info.receiver_local_ssa, ctx) {
                if let Some(si) = c.ssa_index {
                    if ssa.functions[si].ifds.nullable_sparse.binary_search(&l).is_ok() {
                        flags |= CALL_SITE_FLAG_HAS_NULL_RECEIVER;
                    }
                }
            }
            call_sites.push(CallSite::new(
                site_id,
                info.caller_sym,
                info.call_node,
                receiver_ssa,
                info.block,
                info.call_token,
                res.call_type,
                flags,
                info.arg_count,
            ));
            for &callee in &res.targets {
                edges.push((info.caller_sym, callee, site_id, res.call_type));
            }
        }

        let site_to_edge_map: Vec<(u32, u32, u32)> =
            edges.iter().map(|&(a, b, s, _)| (a, b, s)).collect();

        // Unique (caller, callee) pairs; the first site's edge type labels the pair.
        let mut pairs: Vec<(u32, u32, u8)> = edges.iter().map(|&(a, b, _, t)| (a, b, t)).collect();
        pairs.sort_by_key(|&(a, b, _)| (a, b));
        pairs.dedup_by_key(|&mut (a, b, _)| (a, b));

        // Step 6: CSR adjacency (indexed by symbol id)
        let method_count = sta.symbol_count;
        let n = method_count as usize;
        let callee_csr = csr_from_sorted(n, &pairs, true);
        let mut reversed: Vec<(u32, u32, u8)> = pairs.iter().map(|&(a, b, t)| (b, a, t)).collect();
        reversed.sort_by_key(|&(b, a, _)| (b, a));
        // Edge types are kept on the caller side too, so `CGCSR::edges_of` can slice them.
        let caller_csr = csr_from_sorted(n, &reversed, true);

        // Step 7: Tarjan SCCs over call-graph nodes
        let mut cg_nodes: Vec<u32> = index.callables.clone();
        for &(a, b, _) in &pairs {
            cg_nodes.push(a);
            cg_nodes.push(b);
        }
        cg_nodes.retain(|&v| (v as usize) < n);
        cg_nodes.sort_unstable();
        cg_nodes.dedup();
        let mut adj: Vec<Vec<u32>> = vec![Vec::new(); n];
        for &(a, b, _) in &pairs {
            if (a as usize) < n && (b as usize) < n {
                adj[a as usize].push(b);
            }
        }
        let (sccs, scc_members) = TarjanSCC::compute_over(&cg_nodes, &adj)?;

        let mut points_to_table: Vec<PointsToEntry> = Vec::new();
        for (&g, types) in &pta.pts {
            for &t in types {
                points_to_table.push(PointsToEntry {
                    ssa_id: g,
                    alloc_type_sym_id: t,
                });
            }
        }
        points_to_table.sort_unstable_by_key(|p| (p.ssa_id, p.alloc_type_sym_id));

        Ok(CallGraphArtifact {
            format_version: CGA_FORMAT_VERSION,
            method_count,
            call_site_count: u32::try_from(call_sites.len())
                .map_err(|_| "call-site count exceeds the u32 range")?,
            call_edge_count: u32::try_from(pairs.len())
                .map_err(|_| "call-edge count exceeds the u32 range")?,
            ssa_hash,
            sta_hash,
            call_sites,
            callee_csr,
            caller_csr,
            site_to_edge_map,
            points_to_table,
            sccs,
            scc_members,
        })
    }

    /// Assert Phase 6 Invariants (§6.8)
    fn assert_invariants(
        artifact: &CallGraphArtifact,
        bpa: &BPASTArtifact,
        cfa: &CFGArtifact,
    ) -> Result<(), String> {
        // ── Invariant 1: every call site anchors to an AST call node and, when mapped,
        //    to an existing block of its caller's CFG ──
        let blocks_of: std::collections::HashMap<u32, usize> = cfa
            .functions
            .iter()
            .map(|f| (f.sym_id, f.blocks.len()))
            .collect();
        for site in &artifact.call_sites {
            if site.call_node >= bpa.node_count {
                return Err(format!(
                    "Invariant 1 Violation: CallSite #{} references node {} outside the BPA",
                    site.call_site_id, site.call_node
                ));
            }
            if site.call_block != u32::MAX {
                let ok = blocks_of
                    .get(&site.caller_sym)
                    .is_some_and(|&nb| (site.call_block as usize) < nb);
                if !ok {
                    return Err(format!(
                        "Invariant 1 Violation: CallSite #{} block {} is not a block of caller {}",
                        site.call_site_id, site.call_block, site.caller_sym
                    ));
                }
            }
        }

        // ── Invariant 2: CSR consistency ──
        let n = artifact.method_count as usize;
        if artifact.callee_csr.offsets.len() != n + 1
            || artifact.caller_csr.offsets.len() != n + 1
            || artifact.callee_csr.adj.len() != artifact.call_edge_count as usize
            || artifact.caller_csr.adj.len() != artifact.call_edge_count as usize
            || artifact.callee_csr.edge_types.len() != artifact.callee_csr.adj.len()
        {
            return Err("Invariant 2 Violation: call-graph CSR sizes are inconsistent".to_string());
        }

        // ── Invariant 3: SCCs partition the call-graph nodes ──
        let total_scc_members: usize = artifact.sccs.iter().map(|s| s.member_count as usize).sum();
        let distinct: HashSet<u32> = artifact.scc_members.iter().copied().collect();
        if total_scc_members != artifact.scc_members.len() || distinct.len() != total_scc_members {
            return Err(format!(
                "Invariant 3 Violation: SCC members ({}) do not partition the call-graph nodes ({} distinct)",
                total_scc_members,
                distinct.len()
            ));
        }

        log_info("Phase 6 Invariants 1-3 asserted successfully.");
        Ok(())
    }
}
