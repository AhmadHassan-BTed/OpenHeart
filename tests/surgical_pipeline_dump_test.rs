//! Ruthless Surgical Pipeline Output Inspection Test.
//! Dumps exact artifact contents at every phase boundary.
//! Authored solely by Ahmad Hassan (B-Ted).

use openheart::ast::{ASTStage, ASTStageInput};
use openheart::cfg::Phase4Stage;
use openheart::cg::serializer::CGASerializer;
use openheart::cg::Phase6Stage;
use openheart::core::io::mmap::MemoryMappedFile;
use openheart::core::types::symbol::*;
use openheart::core::types::token::unpack_sort_key;
use openheart::ingestion::manifest::SourceManifest;
use openheart::ingestion::IngestionStage;
use openheart::ssa::Phase5Stage;
use openheart::symbol::Phase3Stage;
use std::fs;
use tempfile::tempdir;

#[test]
fn test_surgical_phase_by_phase_artifact_dump() {
    let dir = tempdir().unwrap();

    // Two-file Java codebase with inheritance, virtual dispatch, recursion
    let file_a = dir.path().join("Animal.java");
    fs::write(
        &file_a,
        r#"
package com.test;

public abstract class Animal {
    protected String name;
    protected int age;

    public Animal(String name, int age) {
        this.name = name;
        this.age = age;
    }

    public abstract void speak();

    public void breathe() {
        System.out.println(name + " breathes");
    }

    public static int count(int n) {
        if (n <= 0) return 0;
        return 1 + count(n - 1);
    }
}
"#,
    )
    .unwrap();

    let file_b = dir.path().join("Dog.java");
    fs::write(
        &file_b,
        r#"
package com.test;

public class Dog extends Animal {
    private boolean trained;

    public Dog(String name, int age, boolean trained) {
        super(name, age);
        this.trained = trained;
    }

    @Override
    public void speak() {
        System.out.println(name + " barks!");
    }

    public void fetch() {
        if (trained) {
            speak();
            breathe();
        }
    }

    public static void main(String[] args) {
        Dog d = new Dog("Rex", 5, true);
        d.fetch();
        Animal.count(10);
    }
}
"#,
    )
    .unwrap();

    let manifest = SourceManifest::new(vec![file_a.clone(), file_b.clone()]);
    let tca_path = dir.path().join("corpus.tca");
    let bpa_path = dir.path().join("ast.bpa");
    let sta_path = dir.path().join("symbols.sta");
    let cfa_path = dir.path().join("cfg.cfa");
    let ssa_path = dir.path().join("ssa.ssa");
    let cga_path = dir.path().join("callgraph.cga");

    // ============================================================
    // PHASE 1: Lexical Ingestion
    // ============================================================
    let tca = IngestionStage::run(manifest, &tca_path).unwrap();
    let tca_bytes = fs::read(&tca_path).unwrap();

    println!("\n========================================================================");
    println!("  PHASE 1 SURGICAL DUMP: TokenCorpusArtifact (.tca)");
    println!("========================================================================");
    println!("  .tca file size:       {} bytes", tca_bytes.len());
    println!("  Total token count:    {}", tca.token_records.len());
    println!("  Source file count:    {}", tca.file_records.len());
    println!("  String interner size: {} entries", tca.interner.count());

    for (i, sf) in tca.file_records.iter().enumerate() {
        println!(
            "    File[{}]: file_id={} lang_id={:#04x} first_token={} file_tokens={} size={}B",
            i,
            sf.file_id,
            sf.language_id,
            sf.first_token_id,
            sf.file_token_count,
            sf.file_size_bytes
        );
    }
    println!("  First 10 tokens:");
    for (i, tok) in tca.token_records.iter().take(10).enumerate() {
        let (file_id, line, col) = unpack_sort_key(tok.sort_key);
        println!(
            "    Token[{:3}]: sort_key={:#018X} file={} line={:3} col={:3} type={:#04x} len={:3} text_id={}",
            i, tok.sort_key, file_id, line, col, tok.token_type, tok.len, tok.text_id
        );
    }

    // ============================================================
    // PHASE 2: CST Reduction & BP AST
    // ============================================================
    let stage_input = ASTStageInput {
        tca: MemoryMappedFile::open(&tca_path).unwrap(),
    };
    let bpa = ASTStage::run(&stage_input, &bpa_path).unwrap();
    let bpa_bytes = fs::read(&bpa_path).unwrap();

    println!("\n========================================================================");
    println!("  PHASE 2 SURGICAL DUMP: BPASTArtifact (.bpa)");
    println!("========================================================================");
    println!("  .bpa file size:       {} bytes", bpa_bytes.len());
    println!("  Total AST node count: {}", bpa.node_count);
    println!(
        "  BP bitstring length:  {} bits ({}B)",
        bpa.node_count * 2,
        bpa.node_count * 2 / 8
    );

    // Node type distribution
    let mut type_counts = std::collections::HashMap::new();
    for i in 0..bpa.node_count {
        let nt = bpa.node_type(i);
        *type_counts.entry(format!("{:?}", nt)).or_insert(0u32) += 1;
    }
    println!("  AST Node Type Distribution:");
    let mut sorted_types: Vec<_> = type_counts.iter().collect();
    sorted_types.sort_by(|a, b| b.1.cmp(a.1));
    for (nt, count) in &sorted_types {
        println!("    {:30} : {:4}", nt, count);
    }

    println!("  First 15 nodes (pre-order):");
    for i in 0..std::cmp::min(15, bpa.node_count) {
        let nt = bpa.node_type(i);
        let parent = bpa.parent(i);
        let (tok_start, tok_end) = bpa.token_range(i);
        println!(
            "    Node[{:3}]: type={:?} parent={} token_range=[{}, {}]",
            i,
            nt,
            if parent == u32::MAX {
                "ROOT".to_string()
            } else {
                parent.to_string()
            },
            tok_start,
            tok_end
        );
    }

    // ============================================================
    // PHASE 3: Symbol Table & Type Hierarchy
    // ============================================================
    let sta = Phase3Stage::run(&tca, &bpa, &tca_bytes, &bpa_bytes).unwrap();
    let sta_bytes = sta.serialize();
    fs::write(&sta_path, &sta_bytes).unwrap();

    println!("\n========================================================================");
    println!("  PHASE 3 SURGICAL DUMP: SymbolTableArtifact (.sta)");
    println!("========================================================================");
    println!("  .sta file size:       {} bytes", sta_bytes.len());
    println!("  Symbol count:         {}", sta.symbol_count);
    println!("  Scope count:          {}", sta.scope_records.len());
    println!("  TH edge count:        {}", sta.th_edges.len());

    println!("  All {} symbols:", sta.symbol_count);
    for sym in &sta.symbol_records {
        let kind_name = match SymbolKind::from(sym.kind) {
            SymbolKind::SK_PACKAGE => "PACKAGE",
            SymbolKind::SK_CLASS => "CLASS",
            SymbolKind::SK_INTERFACE => "INTERFACE",
            SymbolKind::SK_METHOD => "METHOD",
            SymbolKind::SK_CONSTRUCTOR => "CONSTRUCTOR",
            SymbolKind::SK_FIELD => "FIELD",
            SymbolKind::SK_PARAM => "PARAM",
            SymbolKind::SK_LOCAL_VAR => "LOCAL_VAR",
            SymbolKind::SK_ENUM => "ENUM",
            SymbolKind::SK_STATIC_INIT => "STATIC_INIT",
            SymbolKind::SK_LAMBDA => "LAMBDA",
            SymbolKind::SK_TYPE_PARAM => "TYPE_PARAM",
            SymbolKind::SK_EXTERNAL => "EXTERNAL",
            _ => "OTHER",
        };
        let vis_name = match SymbolVisibility::from(sym.visibility) {
            SymbolVisibility::Package => "package",
            SymbolVisibility::Public => "public",
            SymbolVisibility::Private => "private",
            SymbolVisibility::Protected => "protected",
        };
        println!(
            "    Sym[{:3}]: kind={:12} vis={:9} mod={:#06x} parent={:3} name_id={:4} decl={:3} def={:3} params={} scope={}",
            sym.symbol_id, kind_name, vis_name, sym.modifiers,
            if sym.parent_sym == u32::MAX { -1i32 } else { sym.parent_sym as i32 },
            sym.name_id, sym.decl_node, sym.def_node, sym.param_count, sym.scope_id
        );
    }

    if !sta.th_edges.is_empty() {
        println!("  Type Hierarchy Edges:");
        for edge in &sta.th_edges {
            let rel = match edge.relation {
                r if r == THRelation::TH_EXTENDS => "EXTENDS",
                r if r == THRelation::TH_IMPLEMENTS => "IMPLEMENTS",
                r if r == THRelation::TH_USES => "USES",
                r if r == THRelation::TH_CREATES => "CREATES",
                _ => "?",
            };
            println!("    {} -> {} [{}]", edge.from_sym, edge.to_sym, rel);
        }
    }

    println!("  All {} scopes:", sta.scope_records.len());
    for sc in &sta.scope_records {
        println!(
            "    Scope[{:3}]: parent={:3} owner_sym={:3} kind={:?}",
            sc.scope_id,
            if sc.parent_scope == u32::MAX {
                -1i32
            } else {
                sc.parent_scope as i32
            },
            if sc.owner_symbol == u32::MAX {
                -1i32
            } else {
                sc.owner_symbol as i32
            },
            ScopeKind::from(sc.scope_kind)
        );
    }

    // ============================================================
    // PHASE 4: Control Flow Graph & Dominator Analysis
    // ============================================================
    let cfa = Phase4Stage::run(&bpa, &sta, &sta_bytes, &bpa_bytes, &cfa_path).unwrap();
    let cfa_bytes = fs::read(&cfa_path).unwrap();

    println!("\n========================================================================");
    println!("  PHASE 4 SURGICAL DUMP: CFGArtifact (.cfa)");
    println!("========================================================================");
    println!("  .cfa file size:       {} bytes", cfa_bytes.len());
    println!("  Function count:       {}", cfa.functions.len());

    let total_blocks: usize = cfa.functions.iter().map(|f| f.blocks.len()).sum();
    let total_edges: usize = cfa.functions.iter().map(|f| f.edges.len()).sum();
    println!("  Total basic blocks:   {}", total_blocks);
    println!("  Total CFG edges:      {}", total_edges);

    for func in &cfa.functions {
        println!(
            "\n    Function sym_id={} ({} blocks, {} edges, cyclomatic={}):",
            func.sym_id,
            func.blocks.len(),
            func.edges.len(),
            func.cyclomatic
        );
        for blk in &func.blocks {
            println!(
                "      BB[{:2}]: stmts={:?} entry={} exit={} first_tok={} last_tok={}",
                blk.id, blk.stmts, blk.is_entry, blk.is_exit, blk.first_token, blk.last_token
            );
        }
        println!("      Edges:");
        for &(from, to, ref etype) in &func.edges {
            println!("        {} -> {} [type={:?}]", from, to, etype);
        }
        println!("      idom[{}]: {:?}", func.idom.len(), func.idom);
        println!(
            "      Succ CSR offsets[{}]: {:?}",
            func.succ_offsets.len(),
            func.succ_offsets
        );
        println!(
            "      Succ CSR adj[{}]: {:?}",
            func.succ_adj.len(),
            func.succ_adj
        );
        println!(
            "      Pred CSR offsets[{}]: {:?}",
            func.pred_offsets.len(),
            func.pred_offsets
        );
        println!(
            "      Pred CSR adj[{}]: {:?}",
            func.pred_adj.len(),
            func.pred_adj
        );
        println!(
            "      DF offsets[{}]: {:?}",
            func.df_offsets.len(),
            func.df_offsets
        );
        println!("      DF adj[{}]: {:?}", func.df_adj.len(), func.df_adj);
    }

    // ============================================================
    // PHASE 5: SSA Conversion & Data Flow Graph
    // ============================================================
    let ssa = Phase5Stage::run(&bpa, &sta, &cfa, &cfa_bytes, &ssa_path).unwrap();
    let ssa_bytes = fs::read(&ssa_path).unwrap();

    println!("\n========================================================================");
    println!("  PHASE 5 SURGICAL DUMP: SSAArtifact (.ssa)");
    println!("========================================================================");
    println!("  .ssa file size:       {} bytes", ssa_bytes.len());
    println!("  Function count:       {}", ssa.functions.len());

    let total_ssa_vars: usize = ssa.functions.iter().map(|f| f.ssa_records.len()).sum();
    let total_phi_funcs: usize = ssa.functions.iter().map(|f| f.phi_records.len()).sum();
    println!("  Total SSA variables:  {}", total_ssa_vars);
    println!("  Total phi-functions:  {}", total_phi_funcs);

    for func in &ssa.functions {
        println!(
            "\n    Function sym_id={} ({} SSA vars, {} phi-funcs):",
            func.sym_id,
            func.ssa_records.len(),
            func.phi_records.len()
        );
        for rec in &func.ssa_records {
            println!(
                "      SSA[{:3}]: orig_sym={:3} version={:2} def_stmt={:3} def_block={:2} flags={:#04x}",
                rec.ssa_id, rec.orig_sym_id, rec.version, rec.def_stmt, rec.def_block, rec.flags
            );
        }
        for phi in &func.phi_records {
            println!(
                "      PHI: ssa_id={} block={} orig_sym={} args={:?}",
                phi.ssa_id, phi.block_id, phi.orig_sym_id, phi.args
            );
        }

        // CDG dump
        if !func.cdg.cd_offsets.is_empty() {
            println!(
                "      CDG offsets[{}]: {:?}",
                func.cdg.cd_offsets.len(),
                func.cdg.cd_offsets
            );
            println!(
                "      CDG adj[{}]: {:?}",
                func.cdg.cd_adj.len(),
                func.cdg.cd_adj
            );
            println!(
                "      CDG types[{}]: {:?}",
                func.cdg.cd_types.len(),
                func.cdg.cd_types
            );
        }

        // Def-Use dump
        if !func.def_use.def_offsets.is_empty() {
            println!(
                "      DefUse offsets[{}]: {:?}",
                func.def_use.def_offsets.len(),
                func.def_use.def_offsets
            );
            println!(
                "      DefUse use_adj[{}]: {:?}",
                func.def_use.use_adj.len(),
                func.def_use.use_adj
            );
        }
    }

    // ============================================================
    // PHASE 6: Inter-procedural Call Graph & Points-To Analysis
    // ============================================================
    let cga = Phase6Stage::run(&bpa, &sta, &cfa, &ssa, &ssa_bytes, &sta_bytes, &cga_path).unwrap();
    let cga_bytes = fs::read(&cga_path).unwrap();

    println!("\n========================================================================");
    println!("  PHASE 6 SURGICAL DUMP: CallGraphArtifact (.cga)");
    println!("========================================================================");
    println!("  .cga file size:       {} bytes", cga_bytes.len());
    println!("  Format version:       {}", cga.format_version);
    println!("  Method count:         {}", cga.method_count);
    println!("  Call site count:      {}", cga.call_site_count);
    println!("  Call edge count:      {}", cga.call_edge_count);
    println!("  SSA hash:             {:#018X}", cga.ssa_hash);
    println!("  STA hash:             {:#018X}", cga.sta_hash);
    println!("  Points-To entries:    {}", cga.points_to_table.len());
    println!("  SCC count:            {}", cga.sccs.len());
    println!("  SCC members total:    {}", cga.scc_members.len());

    if cga.call_site_count > 0 {
        println!("\n  All {} Call Sites:", cga.call_site_count);
        for site in &cga.call_sites {
            let type_name = match site.call_type {
                0x00 => "DIRECT",
                0x01 => "SPECIAL",
                0x02 => "VIRTUAL",
                0x03 => "INTERFACE",
                0x04 => "CONSTRUCTOR",
                0x05 => "DYNAMIC",
                0x06 => "REFLECTION",
                _ => "UNKNOWN",
            };
            println!(
                "    CS[{:3}]: caller_sym={:3} node={:3} rcv_ssa={} blk={:2} tok={:3} type={:12} args={}",
                site.call_site_id, site.caller_sym, site.call_node,
                if site.receiver_ssa == u32::MAX { "NONE".to_string() } else { site.receiver_ssa.to_string() },
                site.call_block, site.call_token, type_name, site.arg_count
            );
        }
    }

    if !cga.site_to_edge_map.is_empty() {
        println!(
            "\n  Site-to-Edge Map ({} entries):",
            cga.site_to_edge_map.len()
        );
        for &(caller, callee, site_id) in &cga.site_to_edge_map {
            println!(
                "    caller={} -> callee={} via site={}",
                caller, callee, site_id
            );
        }
    }

    if !cga.points_to_table.is_empty() {
        println!(
            "\n  Points-To Table ({} entries):",
            cga.points_to_table.len()
        );
        for pt in &cga.points_to_table {
            println!(
                "    SSA v{} -> AllocType Sym #{}",
                pt.ssa_id, pt.alloc_type_sym_id
            );
        }
    }

    println!("\n  Callee CSR (outgoing calls):");
    println!(
        "    offsets[{}]: {:?}",
        cga.callee_csr.offsets.len(),
        cga.callee_csr.offsets
    );
    println!(
        "    adj[{}]: {:?}",
        cga.callee_csr.adj.len(),
        cga.callee_csr.adj
    );
    println!(
        "    edge_types[{}]: {:?}",
        cga.callee_csr.edge_types.len(),
        cga.callee_csr.edge_types
    );

    println!("\n  Caller CSR (incoming calls):");
    println!(
        "    offsets[{}]: {:?}",
        cga.caller_csr.offsets.len(),
        cga.caller_csr.offsets
    );
    println!(
        "    adj[{}]: {:?}",
        cga.caller_csr.adj.len(),
        cga.caller_csr.adj
    );

    println!("\n  All {} SCCs:", cga.sccs.len());
    for scc in &cga.sccs {
        let class_name = match scc.scc_class {
            0 => "NON_RECURSIVE",
            1 => "SELF_RECURSIVE",
            2 => "MUTUAL_RECURSIVE",
            _ => "UNKNOWN",
        };
        let members_start = scc.member_offset as usize;
        let members_end = members_start + scc.member_count as usize;
        let members = &cga.scc_members[members_start..members_end];
        println!(
            "    SCC[{:3}]: class={:16} members={:?}",
            scc.scc_id, class_name, members
        );
    }

    let tra_path = dir.path().join("traceability.tra");

    // ============================================================
    // PHASE 7: Traceability Index Construction
    // ============================================================
    let tra = openheart::tra::Phase7Stage::run(&tca, &bpa, &sta, &cfa, &ssa, &cga, &tra_path);
    let tra_bytes = fs::read(&tra_path).unwrap();

    println!("\n========================================================================");
    println!("  PHASE 7 SURGICAL DUMP: TraceabilityArtifact (.tra)");
    println!("========================================================================");
    println!("  .tra file size:       {} bytes", tra_bytes.len());
    println!("  Format version:       {}", tra.format_version);
    println!("  AST BI count:         {}", tra.bi_ast.len());
    println!("  Symbol BI count:      {}", tra.bi_sym.len());
    println!("  Block BI count:       {}", tra.bi_blk.len());
    println!("  SSA BI count:         {}", tra.bi_ssa.len());
    println!("  Call Site BI count:   {}", tra.bi_cs.len());
    println!("  Symbol Spans:         {}", tra.sym_span.len());
    println!("  Call Site Spans:      {}", tra.cs_span.len());
    println!("  UMLLink Count:        {}", tra.uml_links.len());
    println!("  SCPG Composite Hash:  0x{:08X}", tra.hashes.scpg_hash);

    // ============================================================
    // ROUND-TRIP DESERIALIZATION VERIFICATION
    // ============================================================
    println!("\n========================================================================");
    println!("  ROUND-TRIP INTEGRITY VERIFICATION");
    println!("========================================================================");

    let cga_rt = CGASerializer::deserialize(&cga_path).unwrap();
    assert_eq!(cga.format_version, cga_rt.format_version);
    assert_eq!(cga.method_count, cga_rt.method_count);
    assert_eq!(cga.call_site_count, cga_rt.call_site_count);
    assert_eq!(cga.call_edge_count, cga_rt.call_edge_count);
    assert_eq!(cga.ssa_hash, cga_rt.ssa_hash);
    assert_eq!(cga.sta_hash, cga_rt.sta_hash);
    assert_eq!(cga.call_sites.len(), cga_rt.call_sites.len());
    assert_eq!(cga.callee_csr.offsets, cga_rt.callee_csr.offsets);
    assert_eq!(cga.callee_csr.adj, cga_rt.callee_csr.adj);
    assert_eq!(cga.callee_csr.edge_types, cga_rt.callee_csr.edge_types);
    assert_eq!(cga.caller_csr.offsets, cga_rt.caller_csr.offsets);
    assert_eq!(cga.caller_csr.adj, cga_rt.caller_csr.adj);
    assert_eq!(cga.site_to_edge_map, cga_rt.site_to_edge_map);
    assert_eq!(cga.sccs.len(), cga_rt.sccs.len());
    assert_eq!(cga.scc_members, cga_rt.scc_members);
    assert_eq!(cga.points_to_table.len(), cga_rt.points_to_table.len());
    println!("  [PASS] CGA round-trip deserialization: ALL FIELDS MATCH");

    let tra_rt = openheart::tra::TraceabilitySerializer::deserialize(&tra_path).unwrap();
    assert_eq!(tra.hashes.scpg_hash, tra_rt.hashes.scpg_hash);
    assert_eq!(tra.uml_links.len(), tra_rt.uml_links.len());
    assert_eq!(tra.sym_span.len(), tra_rt.sym_span.len());
    println!("  [PASS] TRA round-trip deserialization: ALL FIELDS MATCH");

    // ============================================================
    // CROSS-PHASE TRACEABILITY ASSERTIONS
    // ============================================================
    println!("\n========================================================================");
    println!("  CROSS-PHASE TRACEABILITY ASSERTIONS");
    println!("========================================================================");

    for site in &cga.call_sites {
        assert!(
            (site.caller_sym as usize) < sta.symbol_records.len(),
            "CS #{} caller_sym={} >= symbol_count={}",
            site.call_site_id,
            site.caller_sym,
            sta.symbol_count
        );
    }
    println!("  [PASS] All call site caller_sym values are valid STA symbol IDs");

    for site in &cga.call_sites {
        assert!(
            site.call_node < bpa.node_count,
            "CS #{} call_node={} >= node_count={}",
            site.call_site_id,
            site.call_node,
            bpa.node_count
        );
    }
    println!("  [PASS] All call site call_node values are valid AST pre-order indices");

    let scc_total: usize = cga.sccs.iter().map(|s| s.member_count as usize).sum();
    assert_eq!(scc_total, cga.method_count as usize);
    println!(
        "  [PASS] SCC total members ({}) == method_count ({})",
        scc_total, cga.method_count
    );

    assert_eq!(cga.callee_csr.offsets.len(), cga.method_count as usize + 1);
    println!(
        "  [PASS] Callee CSR offsets.len ({}) == method_count+1 ({})",
        cga.callee_csr.offsets.len(),
        cga.method_count + 1
    );

    assert_eq!(cga.caller_csr.offsets.len(), cga.method_count as usize + 1);
    println!(
        "  [PASS] Caller CSR offsets.len ({}) == method_count+1 ({})",
        cga.caller_csr.offsets.len(),
        cga.method_count + 1
    );

    for &callee in &cga.callee_csr.adj {
        assert!(
            (callee as usize) < sta.symbol_records.len(),
            "Callee CSR adj {} >= symbol_count {}",
            callee,
            sta.symbol_count
        );
    }
    println!("  [PASS] All callee CSR adjacency entries are valid STA symbol IDs");

    // ============================================================
    // AGGREGATE SUMMARY
    // ============================================================
    println!("\n========================================================================");
    println!("  AGGREGATE SURGICAL SUMMARY");
    println!("========================================================================");
    println!(
        "  Phase 1 (.tca): {:6} bytes | {:4} tokens | {:2} files | {:4} strings",
        tca_bytes.len(),
        tca.token_records.len(),
        tca.file_records.len(),
        tca.interner.count()
    );
    println!(
        "  Phase 2 (.bpa): {:6} bytes | {:4} AST nodes | {:4} bits BP",
        bpa_bytes.len(),
        bpa.node_count,
        bpa.node_count * 2
    );
    println!(
        "  Phase 3 (.sta): {:6} bytes | {:4} symbols | {:3} scopes | {:2} TH edges",
        sta_bytes.len(),
        sta.symbol_count,
        sta.scope_records.len(),
        sta.th_edges.len()
    );
    println!(
        "  Phase 4 (.cfa): {:6} bytes | {:4} functions | {:3} blocks | {:3} edges",
        cfa_bytes.len(),
        cfa.functions.len(),
        total_blocks,
        total_edges
    );
    println!(
        "  Phase 5 (.ssa): {:6} bytes | {:4} SSA vars | {:3} phi-funcs",
        ssa_bytes.len(),
        total_ssa_vars,
        total_phi_funcs
    );
    println!("  Phase 6 (.cga): {:6} bytes | {:4} call sites | {:3} call edges | {:3} SCCs | {:3} pts entries",
        cga_bytes.len(), cga.call_site_count, cga.call_edge_count, cga.sccs.len(), cga.points_to_table.len());
    println!(
        "  Phase 7 (.tra): {:6} bytes | {:4} UMLLinks | {:3} Symbol Spans",
        tra_bytes.len(),
        tra.uml_links.len(),
        tra.sym_span.len()
    );
    println!(
        "\n  TOTAL PIPELINE OUTPUT: {} bytes across 7 binary artifacts",
        tca_bytes.len()
            + bpa_bytes.len()
            + sta_bytes.len()
            + cfa_bytes.len()
            + ssa_bytes.len()
            + cga_bytes.len()
            + tra_bytes.len()
    );

    println!("\n  ALL SURGICAL INSPECTIONS PASSED.");
}
