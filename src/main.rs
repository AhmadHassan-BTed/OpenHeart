//! OpenHeart CLI Engine Executable Entry Point.
//! Authored by Ahmad Hassan (B-Ted).

use openheart::ast::{ASTStage, ASTStageInput};
use openheart::cfg::serializer::CFGArtifact;
use openheart::cfg::Phase4Stage;
use openheart::core::io::mmap::MemoryMappedFile;
use openheart::ingestion::manifest::SourceManifest;
use openheart::ingestion::IngestionStage;
use openheart::symbol::serializer::SymbolTableArtifact;
use openheart::symbol::Phase3Stage;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

fn print_usage() {
    println!(
        r#"
================================================================================
 OPENHEART SCPG ENGINE v0.1.0 :: MAINTAINED BY AHMAD HASSAN (B-TED)
 High-Performance Succinct Compositional Program Graph Static Analysis Engine
================================================================================

USAGE:
    openheart <SUBCOMMAND> [OPTIONS]

SUBCOMMANDS:
    analyze <SOURCE_PATH> [OUTPUT_DIR]
        Recursively scans <SOURCE_PATH> for .java source files and executes
        the complete 4-phase static analysis pipeline:
          • Phase 1: Lexical Ingestion          ─► corpus.tca
          • Phase 2: CST Reduction & BP AST     ─► ast.bpa
          • Phase 3: Symbol Table & Hierarchy   ─► symbols.sta
          • Phase 4: Control Flow & Dominators  ─► cfg.cfa

    inspect <ARTIFACT_PATH>
        Inspects and validates the CRC-64 integrity of a binary artifact (.tca, .bpa, .sta, .cfa).

    help
        Prints this usage guide.

EXAMPLES:
    openheart analyze ./src/main/java ./out
    openheart inspect ./out/symbols.sta
================================================================================
"#
    );
}

fn collect_java_files(path: &Path, files: &mut Vec<PathBuf>) -> std::io::Result<()> {
    if path.is_file() {
        if path.extension().map_or(false, |ext| ext == "java") {
            files.push(path.to_path_buf());
        }
    } else if path.is_dir() {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let p = entry.path();
            if p.is_dir() {
                collect_java_files(&p, files)?;
            } else if p.extension().map_or(false, |ext| ext == "java") {
                files.push(p);
            }
        }
    }
    Ok(())
}

fn cmd_analyze(source_path_str: &str, out_dir_str: Option<&str>) -> Result<(), String> {
    let start_time = Instant::now();

    let source_path = Path::new(source_path_str);
    if !source_path.exists() {
        return Err(format!("Source path '{}' does not exist", source_path_str));
    }

    let mut java_files = Vec::new();
    collect_java_files(source_path, &mut java_files)
        .map_err(|e| format!("Failed to scan directory: {}", e))?;

    if java_files.is_empty() {
        return Err(format!(
            "No .java files found under source path '{}'",
            source_path_str
        ));
    }

    println!("================================================================================");
    println!(" OPENHEART STATIC ANALYSIS PIPELINE STARTING");
    println!(" Input Path  : {}", source_path.display());
    println!(" Java Files  : {}", java_files.len());
    println!("================================================================================");

    let out_dir = match out_dir_str {
        Some(d) => PathBuf::from(d),
        None => PathBuf::from("./openheart_output"),
    };
    fs::create_dir_all(&out_dir)
        .map_err(|e| format!("Failed to create output directory: {}", e))?;

    let tca_path = out_dir.join("corpus.tca");
    let bpa_path = out_dir.join("ast.bpa");
    let sta_path = out_dir.join("symbols.sta");
    let cfa_path = out_dir.join("cfg.cfa");

    // ── PHASE 1: Lexical Ingestion ──
    println!("\n[1/4] Running Phase 1: Lexical Ingestion...");
    let p1_start = Instant::now();
    let manifest = SourceManifest::new(java_files.clone());
    let tca_artifact = IngestionStage::run(manifest, &tca_path)
        .map_err(|e| format!("Phase 1 Ingestion failed: {}", e))?;
    let tca_bytes = fs::read(&tca_path).map_err(|e| format!("Failed to read .tca file: {}", e))?;
    println!(
        "   ✓ Phase 1 Complete in {:.2?} | Tokens Ingested: {} | Output: {}",
        p1_start.elapsed(),
        tca_artifact.token_records.len(),
        tca_path.display()
    );

    // ── PHASE 2: CST Reduction & BP AST Encoding ──
    println!("\n[2/4] Running Phase 2: CST Reduction & Succinct BP AST Encoding...");
    let p2_start = Instant::now();
    let stage_input = ASTStageInput {
        tca: MemoryMappedFile::open(&tca_path)
            .map_err(|e| format!("Failed to mmap .tca: {}", e))?,
    };
    let bpa_artifact = ASTStage::run(&stage_input, &bpa_path)
        .map_err(|e| format!("Phase 2 AST Reduction failed: {}", e))?;
    let bpa_bytes = fs::read(&bpa_path).map_err(|e| format!("Failed to read .bpa file: {}", e))?;
    println!(
        "   ✓ Phase 2 Complete in {:.2?} | AST Nodes Encoded: {} | Output: {}",
        p2_start.elapsed(),
        bpa_artifact.node_count,
        bpa_path.display()
    );

    // ── PHASE 3: Symbol Table & Type Hierarchy Construction ──
    println!("\n[3/4] Running Phase 3: Symbol Table & Type Hierarchy Construction...");
    let p3_start = Instant::now();
    let sta_artifact = Phase3Stage::run(&tca_artifact, &bpa_artifact, &tca_bytes, &bpa_bytes)
        .map_err(|e| format!("Phase 3 Symbol Table resolution failed: {}", e))?;
    let sta_bytes = sta_artifact.serialize();
    fs::write(&sta_path, &sta_bytes).map_err(|e| format!("Failed to write .sta file: {}", e))?;
    println!(
        "   ✓ Phase 3 Complete in {:.2?} | Symbols: {} | Scopes: {} | Hierarchy Edges: {} | Output: {}",
        p3_start.elapsed(),
        sta_artifact.symbol_count,
        sta_artifact.scope_count,
        sta_artifact.th_edge_count,
        sta_path.display()
    );

    // ── PHASE 4: Control Flow Graph Construction & Dominator Analysis ──
    println!("\n[4/4] Running Phase 4: Control Flow Graph & Dominator Analysis...");
    let p4_start = Instant::now();
    let cfa_artifact = Phase4Stage::run(
        &bpa_artifact,
        &sta_artifact,
        &sta_bytes,
        &bpa_bytes,
        &cfa_path,
    )
    .map_err(|e| format!("Phase 4 CFG Construction failed: {}", e))?;
    println!(
        "   ✓ Phase 4 Complete in {:.2?} | Functions Analyzed: {} | Total Basic Blocks: {} | Total Edges: {} | Output: {}",
        p4_start.elapsed(),
        cfa_artifact.function_count,
        cfa_artifact.total_blocks,
        cfa_artifact.total_edges,
        cfa_path.display()
    );

    println!("\n================================================================================");
    println!(
        " SUCCESS: Complete 4-Phase Static Analysis finished in {:.2?}",
        start_time.elapsed()
    );
    println!(" Output Directory: {}", out_dir.display());
    println!("================================================================================");

    Ok(())
}

fn cmd_inspect(artifact_path_str: &str) -> Result<(), String> {
    let path = Path::new(artifact_path_str);
    if !path.exists() {
        return Err(format!("File '{}' does not exist", artifact_path_str));
    }

    let bytes = fs::read(path).map_err(|e| format!("Failed to read file: {}", e))?;
    println!("Inspecting artifact: {}", path.display());
    println!("File Size: {} bytes", bytes.len());

    if let Ok(cfa) = CFGArtifact::deserialize(&bytes) {
        println!("Artifact Type  : Control Flow Graph (.cfa)");
        println!("Format Version : {}", cfa.format_version);
        println!("Function Count : {}", cfa.function_count);
        println!("Total Blocks   : {}", cfa.total_blocks);
        println!("Total Edges    : {}", cfa.total_edges);
        println!("STA Hash Link  : 0x{:016X}", cfa.sta_hash);
        println!("BPA Hash Link  : 0x{:016X}", cfa.bpa_hash);
        println!("CRC-64 Check   : VERIFIED VALID");
        return Ok(());
    }

    if let Ok(sta) = SymbolTableArtifact::deserialize(&bytes) {
        println!("Artifact Type  : Symbol Table & Hierarchy (.sta)");
        println!("Format Version : {}", sta.format_version);
        println!("Symbol Count   : {}", sta.symbol_count);
        println!("Scope Count    : {}", sta.scope_count);
        println!("TH Edge Count  : {}", sta.th_edge_count);
        println!("BPA Hash Link  : 0x{:016X}", sta.bpa_hash);
        println!("TCA Hash Link  : 0x{:016X}", sta.tca_hash);
        println!("CRC-64 Check   : VERIFIED VALID");
        return Ok(());
    }

    Err(format!(
        "Unknown or corrupted artifact format in '{}'",
        artifact_path_str
    ))
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        print_usage();
        return;
    }

    let command = args[1].as_str();
    let result = match command {
        "analyze" => {
            if args.len() < 3 {
                println!("Error: 'analyze' requires a source path argument.");
                print_usage();
                std::process::exit(1);
            }
            let source_path = &args[2];
            let out_dir = if args.len() >= 4 {
                Some(args[3].as_str())
            } else {
                None
            };
            cmd_analyze(source_path, out_dir)
        }
        "inspect" => {
            if args.len() < 3 {
                println!("Error: 'inspect' requires an artifact file path argument.");
                print_usage();
                std::process::exit(1);
            }
            cmd_inspect(&args[2])
        }
        "help" | "-h" | "--help" => {
            print_usage();
            Ok(())
        }
        unknown => {
            println!("Error: Unknown subcommand '{}'", unknown);
            print_usage();
            std::process::exit(1);
        }
    };

    if let Err(e) = result {
        eprintln!("\nFATAL ERROR: {}", e);
        std::process::exit(1);
    }
}
