//! OpenHeart CLI Engine Executable Entry Point.
//! Authored by Ahmad Hassan (B-Ted).

use openheart::ast::{ASTStage, ASTStageInput};
use openheart::cfg::serializer::CFGArtifact;
use openheart::cfg::Phase4Stage;
use openheart::core::io::mmap::MemoryMappedFile;
use openheart::core::logger::{init_logger_from_env, log_info, set_log_level, LogLevel};
use openheart::ingestion::manifest::SourceManifest;
use openheart::ingestion::IngestionStage;
use openheart::ssa::serializer::SSASerializer;
use openheart::ssa::Phase5Stage;
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
    analyze <SOURCE_PATH> [OUTPUT_DIR] [--verbose | --debug | --trace]
        Recursively scans <SOURCE_PATH> for .java source files and executes
        the complete 5-phase static analysis pipeline with structured logging:
          • Phase 1: Lexical Ingestion          ─► corpus.tca
          • Phase 2: CST Reduction & BP AST     ─► ast.bpa
          • Phase 3: Symbol Table & Hierarchy   ─► symbols.sta
          • Phase 4: Control Flow & Dominators  ─► cfg.cfa
          • Phase 5: SSA Form & Data Flow Graph ─► ssa.ssa

    inspect <ARTIFACT_PATH>
        Inspects and validates the CRC-64 integrity of a binary artifact (.tca, .bpa, .sta, .cfa, .ssa).

    help
        Prints this usage guide.

FLAGS:
    -v, --verbose, --debug    Enable verbose debug logging
    --trace                   Enable granular trace logging (all statement dispatches)

EXAMPLES:
    openheart analyze ./src/main/java ./out --debug
    openheart inspect ./out/ssa.ssa
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

    let out_dir = match out_dir_str {
        Some(d) => PathBuf::from(d),
        None => PathBuf::from("./openheart_output"),
    };
    fs::create_dir_all(&out_dir)
        .map_err(|e| format!("Failed to create output directory: {}", e))?;

    let session_log_path = out_dir.join("openheart_session.log");
    let persistent_log_path = out_dir.join("openheart_persistent.log");
    openheart::core::logger::init_dual_logger(Some(&persistent_log_path), Some(&session_log_path));

    log_info("================================================================================");
    log_info(" OPENHEART STATIC ANALYSIS PIPELINE STARTING");
    log_info(&format!(" Input Path  : {}", source_path.display()));
    log_info(&format!(" Java Files  : {}", java_files.len()));
    log_info(&format!(" Session Log : {}", session_log_path.display()));
    log_info(&format!(" Persist Log : {}", persistent_log_path.display()));
    log_info("================================================================================");

    let tca_path = out_dir.join("corpus.tca");
    let bpa_path = out_dir.join("ast.bpa");
    let sta_path = out_dir.join("symbols.sta");
    let cfa_path = out_dir.join("cfg.cfa");
    let ssa_path = out_dir.join("ssa.ssa");

    // ── PHASE 1: Lexical Ingestion ──
    let manifest = SourceManifest::new(java_files.clone());
    let tca_artifact = IngestionStage::run(manifest, &tca_path)
        .map_err(|e| format!("Phase 1 Ingestion failed: {}", e))?;
    let tca_bytes = fs::read(&tca_path).map_err(|e| format!("Failed to read .tca file: {}", e))?;

    // ── PHASE 2: CST Reduction & BP AST Encoding ──
    let stage_input = ASTStageInput {
        tca: MemoryMappedFile::open(&tca_path)
            .map_err(|e| format!("Failed to mmap .tca: {}", e))?,
    };
    let bpa_artifact = ASTStage::run(&stage_input, &bpa_path)
        .map_err(|e| format!("Phase 2 AST Reduction failed: {}", e))?;
    let bpa_bytes = fs::read(&bpa_path).map_err(|e| format!("Failed to read .bpa file: {}", e))?;

    // ── PHASE 3: Symbol Table & Type Hierarchy Construction ──
    let sta_artifact = Phase3Stage::run(&tca_artifact, &bpa_artifact, &tca_bytes, &bpa_bytes)
        .map_err(|e| format!("Phase 3 Symbol Table resolution failed: {}", e))?;
    let sta_bytes = sta_artifact.serialize();
    fs::write(&sta_path, &sta_bytes).map_err(|e| format!("Failed to write .sta file: {}", e))?;

    // ── PHASE 4: Control Flow Graph Construction & Dominator Analysis ──
    let cfa_artifact = Phase4Stage::run(
        &bpa_artifact,
        &sta_artifact,
        &sta_bytes,
        &bpa_bytes,
        &cfa_path,
    )
    .map_err(|e| format!("Phase 4 CFG Construction failed: {}", e))?;
    let cfa_bytes = fs::read(&cfa_path).map_err(|e| format!("Failed to read .cfa file: {}", e))?;

    // ── PHASE 5: SSA Conversion & Data Flow Graph Construction ──
    let ssa_artifact = Phase5Stage::run(
        &bpa_artifact,
        &sta_artifact,
        &cfa_artifact,
        &cfa_bytes,
        &ssa_path,
    )
    .map_err(|e| format!("Phase 5 SSA Conversion failed: {}", e))?;

    log_info("================================================================================");
    log_info(&format!(
        " SUCCESS: Complete 5-Phase Static Analysis finished in {:.2?} | Output: {}",
        start_time.elapsed(),
        out_dir.display()
    ));
    log_info(&format!(
        " Summary: {} tokens, {} AST nodes, {} symbols, {} functions, {} blocks, {} CFG edges, {} SSA vars, {} φ-funcs.",
        tca_artifact.token_records.len(),
        bpa_artifact.node_count,
        sta_artifact.symbol_count,
        cfa_artifact.function_count,
        cfa_artifact.total_blocks,
        cfa_artifact.total_edges,
        ssa_artifact.total_ssa_vars,
        ssa_artifact.total_phi_funcs
    ));
    log_info("================================================================================");

    Ok(())
}

fn cmd_inspect(artifact_path_str: &str) -> Result<(), String> {
    let path = Path::new(artifact_path_str);
    if !path.exists() {
        return Err(format!("File '{}' does not exist", artifact_path_str));
    }

    let bytes = fs::read(path).map_err(|e| format!("Failed to read file: {}", e))?;
    log_info(&format!("Inspecting artifact: {}", path.display()));
    log_info(&format!("File Size: {} bytes", bytes.len()));

    if let Ok(ssa) = SSASerializer::read(path) {
        log_info("Artifact Type  : Static Single Assignment (.ssa)");
        log_info(&format!("Format Version : {}", ssa.format_version));
        log_info(&format!("Function Count : {}", ssa.function_count));
        log_info(&format!("Total SSA Vars : {}", ssa.total_ssa_vars));
        log_info(&format!("Total Phi Funcs: {}", ssa.total_phi_funcs));
        log_info(&format!("CFA Hash Link  : 0x{:016X}", ssa.cfa_hash));
        log_info("CRC-64 Check   : VERIFIED VALID");
        return Ok(());
    }

    if let Ok(cfa) = CFGArtifact::deserialize(&bytes) {
        log_info("Artifact Type  : Control Flow Graph (.cfa)");
        log_info(&format!("Format Version : {}", cfa.format_version));
        log_info(&format!("Function Count : {}", cfa.function_count));
        log_info(&format!("Total Blocks   : {}", cfa.total_blocks));
        log_info(&format!("Total Edges    : {}", cfa.total_edges));
        log_info(&format!("STA Hash Link  : 0x{:016X}", cfa.sta_hash));
        log_info(&format!("BPA Hash Link  : 0x{:016X}", cfa.bpa_hash));
        log_info("CRC-64 Check   : VERIFIED VALID");
        return Ok(());
    }

    if let Ok(sta) = SymbolTableArtifact::deserialize(&bytes) {
        log_info("Artifact Type  : Symbol Table & Hierarchy (.sta)");
        log_info(&format!("Format Version : {}", sta.format_version));
        log_info(&format!("Symbol Count   : {}", sta.symbol_count));
        log_info(&format!("Scope Count    : {}", sta.scope_count));
        log_info(&format!("TH Edge Count  : {}", sta.th_edge_count));
        log_info(&format!("BPA Hash Link  : 0x{:016X}", sta.bpa_hash));
        log_info(&format!("TCA Hash Link  : 0x{:016X}", sta.tca_hash));
        log_info("CRC-64 Check   : VERIFIED VALID");
        return Ok(());
    }

    Err(format!(
        "Unknown or corrupted artifact format in '{}'",
        artifact_path_str
    ))
}

fn main() {
    init_logger_from_env();

    let mut args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        print_usage();
        return;
    }

    // Process logging verbosity flags
    if args.iter().any(|arg| arg == "--trace") {
        set_log_level(LogLevel::Trace);
        args.retain(|arg| arg != "--trace");
    } else if args
        .iter()
        .any(|arg| arg == "-v" || arg == "--verbose" || arg == "--debug")
    {
        set_log_level(LogLevel::Debug);
        args.retain(|arg| arg != "-v" && arg != "--verbose" && arg != "--debug");
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
