//! OpenHeart CLI Engine Executable Entry Point.
//! Authored by Ahmad Hassan (B-Ted).
//!
//! Subcommands: `analyze`, `inspect`, `server` (alias `serve`), `health`, `help`,
//! `version`. Unknown options are errors, never silently ignored. Exit codes: 0 success,
//! 1 runtime failure, 2 invalid command line.

#![forbid(unsafe_code)]

use std::env;
use std::ffi::OsString;
use std::fs;
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream, ToSocketAddrs};
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::str::FromStr;
use std::time::{Duration, Instant};

use openheart::adapters::runner::{
    all_diagram_types, canonical_diagram_type, discover_sources, panic_message,
    parse_diagram_format, pipeline_stack_bytes_from_env, run_isolated, run_pipeline,
    DiscoveryOptions, PipelineOptions, SourceInventory, SourceLimits, WorkDir,
    DEFAULT_DIAGRAM_FORMATS, MAX_CORPUS_FILES, RUN_REPORT_FILE, UML_DIAGRAM_TYPES,
};
use openheart::adapters::{OpenHeartServer, ServerConfig};
use openheart::ast::serializer::BPASTSerializer;
use openheart::cfg::serializer::CFGArtifact;
use openheart::cg::serializer::CGASerializer;
use openheart::core::logger::{
    init_dual_logger, init_logger_from_env, log_info, log_warn, set_log_level, LogLevel,
};
use openheart::ingestion::TokenCorpusSerializer;
use openheart::scpg::diagram::DiagramFormat;
use openheart::scpg::OpenHeartEngine;
use openheart::ssa::serializer::SSASerializer;
use openheart::symbol::serializer::SymbolTableArtifact;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const EXIT_FAILURE: u8 = 1;
const EXIT_USAGE: u8 = 2;

fn print_usage() {
    println!(
        r#"
================================================================================
 OPENHEART SCPG ENGINE v{} :: MAINTAINED BY AHMAD HASSAN (B-TED)
 High-Performance Succinct Compositional Program Graph Static Analysis Engine
================================================================================

USAGE:
    openheart [-v | --verbose | --debug | --trace] <SUBCOMMAND> [OPTIONS]

SUBCOMMANDS:
    analyze <SOURCE_PATH> [OUTPUT_DIR] [OPTIONS]
        Selects the Java (.java) and Kotlin (.kt, .kts; experimental) sources under
        <SOURCE_PATH> and executes the complete 10-phase static analysis pipeline:
          • Phase 1:  Lexical Ingestion          ─► corpus.tca
          • Phase 2:  CST Reduction & BP AST     ─► ast.bpa
          • Phase 3:  Symbol Table & Hierarchy   ─► symbols.sta
          • Phase 4:  Control Flow & Dominators  ─► cfg.cfa
          • Phase 5:  SSA Form & Data Flow Graph ─► ssa.ssa
          • Phase 6:  Call Graph & Points-To     ─► callgraph.cga
          • Phase 7:  Traceability Index         ─► traceability.tra
          • Phase 8:  ROBDD Path Summaries       ─► paths.psa
          • Phase 9:  UML Semantic Extraction    ─► metadata.uma
          • Phase 10: SCPG Unified Binary        ─► unified.scpg
          • 14 UML + 5 analysis-graph diagrams   ─► diagrams/*.puml, *.mmd, *.json
          • Machine-readable run report          ─► run.json
        Hidden entries, symlinks, build/dependency directories and ignore-file matches
        are skipped; test sources are skipped unless --include-tests is given.
        Options:
          -o, --output <DIR>     Output directory (default: $OPENHEART_ARTIFACT_DIR,
                                 else ./openheart_output)
          --include-tests        Also analyze test sources
          --no-ignore            Do not honor .gitignore / .ignore files
          --diagrams <LIST>      Comma-separated diagram types, 'uml' or 'all' (default)
          --formats <LIST>       Comma-separated puml, mmd, json, xmi or 'all'
                                 (default: puml,mmd,json)
          --max-files <N>        Maximum number of source files
          --max-file-kb <N>      Skip source files larger than N KiB
          --max-source-mb <N>    Maximum combined source size in MiB

    inspect <ARTIFACT_PATH>
        Validates a binary artifact with its format reader (.tca, .bpa, .sta, .cfa, .ssa,
        .cga, .tra, .psa, .uma, .scpg) and prints its header fields. The line
        'Checksum' states whether a CRC-64 was actually verified.

    server [PORT] [OPTIONS]                                         (alias: serve)
        Launches the native OpenHeart HTTP backend for the web studio and REST API.
        Options:
          --host <ADDR>          Interface to bind (default 127.0.0.1). Any non-loopback
                                 bind requires an access token (OPENHEART_API_TOKEN, or
                                 one generated and printed at startup).
          -p, --port <PORT>      Port (default 8080)
          --workspace <DIR>      Clone cache and session directory (default ./target_repos)
          --web-root <DIR>       Web studio directory (default ./web)
          --source-root <DIR>    Sample source tree served by /api/source (repeatable)
          --local-repos <DIR>    Allow analyzing sub-directories of DIR by name
          --allow-clone          Enable server-side git clone (default: loopback binds only)
          --no-clone             Disable server-side git clone
        All settings also read OPENHEART_* environment variables (see .env.example).

    health [--host <ADDR>] [-p | --port <PORT>]
        Probes a running server's /api/health endpoint; exits 0 when it answers 200.

    help, version
        Print this guide or the version.

FLAGS:
    -v, --verbose, --debug    Enable verbose debug logging
    --trace                   Enable granular trace logging (all statement dispatches)
    -h, --help                Print this guide
    -V, --version             Print the version

EXIT CODES:
    0 success · 1 runtime failure · 2 invalid command line

EXAMPLES:
    openheart analyze ./src/main/java ./out --debug
    openheart inspect ./out/callgraph.cga
    openheart server --port 8080
================================================================================
"#,
        VERSION
    );
}

// ── Command-line model ───────────────────────────────────────────────────────

struct Invocation {
    log_level: Option<LogLevel>,
    command: Command,
}

enum Command {
    Analyze(AnalyzeArgs),
    Inspect(PathBuf),
    Server(ServerArgs),
    Health(HealthArgs),
    Help,
    Version,
}

#[derive(Debug, Default)]
struct AnalyzeArgs {
    source: PathBuf,
    output: Option<PathBuf>,
    include_tests: bool,
    no_ignore: bool,
    diagram_types: Option<Vec<&'static str>>,
    formats: Option<Vec<DiagramFormat>>,
    max_files: Option<usize>,
    max_file_kb: Option<u64>,
    max_source_mb: Option<u64>,
}

#[derive(Debug, Default)]
struct ServerArgs {
    host: Option<String>,
    port: Option<u16>,
    workspace: Option<PathBuf>,
    web_root: Option<PathBuf>,
    source_roots: Vec<PathBuf>,
    local_repos: Option<PathBuf>,
    allow_clone: Option<bool>,
}

#[derive(Debug, Default)]
struct HealthArgs {
    host: Option<String>,
    port: Option<u16>,
}

/// A command-line token after splitting `--name=value` forms.
enum Token {
    Option { name: String, inline: Option<String> },
    Positional(String),
}

fn tokenize(args: Vec<String>) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        if arg == "--" {
            tokens.extend(iter.by_ref().map(Token::Positional));
            break;
        }
        if arg.len() < 2 || !arg.starts_with('-') {
            tokens.push(Token::Positional(arg));
            continue;
        }
        let split = if arg.starts_with("--") {
            arg.split_once('=')
                .map(|(name, value)| (name.to_string(), value.to_string()))
        } else {
            None
        };
        tokens.push(match split {
            Some((name, value)) => Token::Option {
                name,
                inline: Some(value),
            },
            None => Token::Option {
                name: arg,
                inline: None,
            },
        });
    }
    tokens
}

fn option_value(
    name: &str,
    inline: Option<String>,
    tokens: &mut std::vec::IntoIter<Token>,
) -> Result<String, String> {
    if let Some(value) = inline {
        return Ok(value);
    }
    match tokens.next() {
        Some(Token::Positional(value)) => Ok(value),
        _ => Err(format!("option '{}' requires a value", name)),
    }
}

fn no_value(name: &str, inline: &Option<String>) -> Result<(), String> {
    match inline {
        Some(_) => Err(format!("option '{}' does not take a value", name)),
        None => Ok(()),
    }
}

fn parse_number<T: FromStr>(name: &str, raw: &str) -> Result<T, String> {
    raw.trim()
        .parse()
        .map_err(|_| format!("option '{}' expects a number, got '{}'", name, raw))
}

fn parse_port(name: &str, raw: &str) -> Result<u16, String> {
    match raw.trim().parse::<u16>() {
        Ok(port) if port > 0 => Ok(port),
        _ => Err(format!("{} expects a port between 1 and 65535, got '{}'", name, raw)),
    }
}

fn parse_diagram_list(raw: &str) -> Result<Vec<&'static str>, String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "all" => return Ok(all_diagram_types()),
        "uml" => return Ok(UML_DIAGRAM_TYPES.to_vec()),
        _ => {}
    }
    let mut selection = Vec::new();
    for name in raw.split(',').map(str::trim).filter(|name| !name.is_empty()) {
        let canonical = canonical_diagram_type(name)
            .ok_or_else(|| format!("unknown diagram type '{}'", name))?;
        if !selection.contains(&canonical) {
            selection.push(canonical);
        }
    }
    if selection.is_empty() {
        return Err("--diagrams needs at least one diagram type".to_string());
    }
    Ok(selection)
}

fn parse_format_list(raw: &str) -> Result<Vec<DiagramFormat>, String> {
    if raw.trim().eq_ignore_ascii_case("all") {
        return Ok(vec![
            DiagramFormat::PlantUML,
            DiagramFormat::Mermaid,
            DiagramFormat::JSON,
            DiagramFormat::XMI,
        ]);
    }
    let mut formats = Vec::new();
    for name in raw.split(',').map(str::trim).filter(|name| !name.is_empty()) {
        let format =
            parse_diagram_format(name).ok_or_else(|| format!("unknown diagram format '{}'", name))?;
        if !formats.contains(&format) {
            formats.push(format);
        }
    }
    if formats.is_empty() {
        return Err("--formats needs at least one format".to_string());
    }
    Ok(formats)
}

fn parse_invocation(raw: Vec<String>) -> Result<Invocation, String> {
    let mut log_level: Option<LogLevel> = None;
    let mut remaining = Vec::with_capacity(raw.len());
    let mut options_ended = false;
    for arg in raw {
        if !options_ended {
            match arg.as_str() {
                "--trace" => {
                    log_level = Some(LogLevel::Trace);
                    continue;
                }
                "-v" | "--verbose" | "--debug" => {
                    if log_level != Some(LogLevel::Trace) {
                        log_level = Some(LogLevel::Debug);
                    }
                    continue;
                }
                "--" => options_ended = true,
                _ => {}
            }
        }
        remaining.push(arg);
    }

    let mut remaining = remaining.into_iter();
    let Some(subcommand) = remaining.next() else {
        return Ok(Invocation {
            log_level,
            command: Command::Help,
        });
    };
    let args: Vec<String> = remaining.collect();
    let wants_help = args
        .iter()
        .take_while(|arg| arg.as_str() != "--")
        .any(|arg| arg == "-h" || arg == "--help");
    let command = match subcommand.as_str() {
        "help" | "-h" | "--help" => Command::Help,
        "version" | "-V" | "--version" => Command::Version,
        _ if wants_help => Command::Help,
        "analyze" => Command::Analyze(parse_analyze(args)?),
        "inspect" => Command::Inspect(parse_inspect(args)?),
        "server" | "serve" => Command::Server(parse_server(args)?),
        "health" => Command::Health(parse_health(args)?),
        other => return Err(format!("unknown subcommand '{}'", other)),
    };
    Ok(Invocation { log_level, command })
}

fn parse_analyze(args: Vec<String>) -> Result<AnalyzeArgs, String> {
    let mut parsed = AnalyzeArgs::default();
    let mut positionals = Vec::new();
    let mut tokens = tokenize(args).into_iter();
    while let Some(token) = tokens.next() {
        let (name, inline) = match token {
            Token::Positional(value) => {
                positionals.push(value);
                continue;
            }
            Token::Option { name, inline } => (name, inline),
        };
        match name.as_str() {
            "-o" | "--output" | "--out" => {
                parsed.output = Some(PathBuf::from(option_value(&name, inline, &mut tokens)?));
            }
            "--include-tests" => {
                no_value(&name, &inline)?;
                parsed.include_tests = true;
            }
            "--no-ignore" => {
                no_value(&name, &inline)?;
                parsed.no_ignore = true;
            }
            "--diagrams" => {
                let raw = option_value(&name, inline, &mut tokens)?;
                parsed.diagram_types = Some(parse_diagram_list(&raw)?);
            }
            "--formats" => {
                let raw = option_value(&name, inline, &mut tokens)?;
                parsed.formats = Some(parse_format_list(&raw)?);
            }
            "--max-files" => {
                let raw = option_value(&name, inline, &mut tokens)?;
                parsed.max_files = Some(parse_number(&name, &raw)?);
            }
            "--max-file-kb" => {
                let raw = option_value(&name, inline, &mut tokens)?;
                parsed.max_file_kb = Some(parse_number(&name, &raw)?);
            }
            "--max-source-mb" => {
                let raw = option_value(&name, inline, &mut tokens)?;
                parsed.max_source_mb = Some(parse_number(&name, &raw)?);
            }
            _ => return Err(format!("unknown option '{}' for 'analyze'", name)),
        }
    }

    let mut positionals = positionals.into_iter();
    let source = positionals
        .next()
        .ok_or("'analyze' requires a <SOURCE_PATH> argument")?;
    parsed.source = PathBuf::from(source);
    if let Some(output) = positionals.next() {
        if parsed.output.is_some() {
            return Err("the output directory was given both positionally and with --output".into());
        }
        parsed.output = Some(PathBuf::from(output));
    }
    if let Some(extra) = positionals.next() {
        return Err(format!("unexpected argument '{}' for 'analyze'", extra));
    }
    Ok(parsed)
}

fn parse_inspect(args: Vec<String>) -> Result<PathBuf, String> {
    let mut path = None;
    for token in tokenize(args) {
        match token {
            Token::Positional(value) if path.is_none() => path = Some(PathBuf::from(value)),
            Token::Positional(value) => {
                return Err(format!("unexpected argument '{}' for 'inspect'", value))
            }
            Token::Option { name, .. } => {
                return Err(format!("unknown option '{}' for 'inspect'", name))
            }
        }
    }
    path.ok_or_else(|| "'inspect' requires an <ARTIFACT_PATH> argument".to_string())
}

fn parse_server(args: Vec<String>) -> Result<ServerArgs, String> {
    let mut parsed = ServerArgs::default();
    let mut positional_port = None;
    let mut tokens = tokenize(args).into_iter();
    while let Some(token) = tokens.next() {
        let (name, inline) = match token {
            Token::Positional(value) if positional_port.is_none() => {
                positional_port = Some(parse_port("PORT", &value)?);
                continue;
            }
            Token::Positional(value) => {
                return Err(format!("unexpected argument '{}' for 'server'", value))
            }
            Token::Option { name, inline } => (name, inline),
        };
        match name.as_str() {
            "--host" => parsed.host = Some(option_value(&name, inline, &mut tokens)?),
            "-p" | "--port" => {
                let raw = option_value(&name, inline, &mut tokens)?;
                parsed.port = Some(parse_port(&name, &raw)?);
            }
            "--workspace" => {
                parsed.workspace = Some(PathBuf::from(option_value(&name, inline, &mut tokens)?));
            }
            "--web-root" => {
                parsed.web_root = Some(PathBuf::from(option_value(&name, inline, &mut tokens)?));
            }
            "--source-root" => {
                let raw = option_value(&name, inline, &mut tokens)?;
                parsed.source_roots.push(PathBuf::from(raw));
            }
            "--local-repos" => {
                let raw = option_value(&name, inline, &mut tokens)?;
                parsed.local_repos = Some(PathBuf::from(raw));
            }
            "--allow-clone" => {
                no_value(&name, &inline)?;
                parsed.allow_clone = Some(true);
            }
            "--no-clone" => {
                no_value(&name, &inline)?;
                parsed.allow_clone = Some(false);
            }
            _ => return Err(format!("unknown option '{}' for 'server'", name)),
        }
    }
    if let Some(port) = positional_port {
        if parsed.port.is_some() {
            return Err("the port was given both positionally and with --port".to_string());
        }
        parsed.port = Some(port);
    }
    Ok(parsed)
}

fn parse_health(args: Vec<String>) -> Result<HealthArgs, String> {
    let mut parsed = HealthArgs::default();
    let mut tokens = tokenize(args).into_iter();
    while let Some(token) = tokens.next() {
        let (name, inline) = match token {
            Token::Positional(value) => {
                return Err(format!("unexpected argument '{}' for 'health'", value))
            }
            Token::Option { name, inline } => (name, inline),
        };
        match name.as_str() {
            "--host" => parsed.host = Some(option_value(&name, inline, &mut tokens)?),
            "-p" | "--port" => {
                let raw = option_value(&name, inline, &mut tokens)?;
                parsed.port = Some(parse_port(&name, &raw)?);
            }
            _ => return Err(format!("unknown option '{}' for 'health'", name)),
        }
    }
    Ok(parsed)
}

// ── Entry point ──────────────────────────────────────────────────────────────

fn main() -> ExitCode {
    let raw = match env::args_os()
        .skip(1)
        .map(OsString::into_string)
        .collect::<Result<Vec<String>, OsString>>()
    {
        Ok(raw) => raw,
        Err(arg) => {
            eprintln!("error: argument {:?} is not valid UTF-8", arg);
            return ExitCode::from(EXIT_USAGE);
        }
    };
    let invocation = match parse_invocation(raw) {
        Ok(invocation) => invocation,
        Err(message) => {
            eprintln!("error: {}", message);
            eprintln!("Run 'openheart help' for usage.");
            return ExitCode::from(EXIT_USAGE);
        }
    };
    match execute(invocation) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("\nFATAL ERROR: {}", message);
            ExitCode::from(EXIT_FAILURE)
        }
    }
}

fn execute(invocation: Invocation) -> Result<(), String> {
    let level = invocation.log_level;
    match invocation.command {
        Command::Help => {
            print_usage();
            Ok(())
        }
        Command::Version => {
            println!("openheart {}", VERSION);
            Ok(())
        }
        Command::Health(args) => cmd_health(&args),
        Command::Analyze(args) => {
            // Applies the logging environment; `cmd_analyze` then re-targets the log
            // files to the output directory.
            init_logger_from_env();
            apply_cli_log_level(level);
            cmd_analyze(args)
        }
        Command::Inspect(path) => {
            init_logger_from_env();
            apply_cli_log_level(level);
            cmd_inspect(&path)
        }
        Command::Server(args) => {
            init_logger_from_env();
            apply_cli_log_level(level);
            cmd_server(args)
        }
    }
}

fn apply_cli_log_level(level: Option<LogLevel>) {
    if let Some(level) = level {
        set_log_level(level);
    }
}

// ── analyze ──────────────────────────────────────────────────────────────────

/// What the isolated pipeline thread reports back to the main thread.
struct AnalyzeSummary {
    lines: Vec<String>,
    gaps: Vec<String>,
    diagram_files: usize,
    published_files: usize,
}

fn cmd_analyze(args: AnalyzeArgs) -> Result<(), String> {
    let started = Instant::now();
    if !args.source.exists() {
        return Err(format!(
            "Source path '{}' does not exist",
            args.source.display()
        ));
    }

    let mut limits = SourceLimits::from_env()?;
    if let Some(files) = args.max_files {
        limits.max_files = files.clamp(1, MAX_CORPUS_FILES);
    }
    if let Some(kib) = args.max_file_kb {
        limits.max_file_bytes = kib.saturating_mul(1024).max(1);
    }
    if let Some(mib) = args.max_source_mb {
        limits.max_total_bytes = mib.saturating_mul(1024 * 1024).max(1);
    }
    let discovery = DiscoveryOptions {
        limits,
        include_tests: args.include_tests,
        respect_ignore_files: !args.no_ignore,
        ..DiscoveryOptions::default()
    };
    let pipeline = PipelineOptions {
        diagram_types: args.diagram_types.unwrap_or_else(all_diagram_types),
        formats: args
            .formats
            .unwrap_or_else(|| DEFAULT_DIAGRAM_FORMATS.to_vec()),
        ..PipelineOptions::default()
    };
    let out_dir = args
        .output
        .or_else(|| {
            env::var_os("OPENHEART_ARTIFACT_DIR")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        })
        .unwrap_or_else(|| PathBuf::from("./openheart_output"));

    let inventory = discover_sources(&args.source, &discovery).map_err(|err| err.to_string())?;

    fs::create_dir_all(&out_dir)
        .map_err(|err| format!("Failed to create output directory: {}", err))?;
    let session_log_path = out_dir.join("openheart_session.log");
    let persistent_log_path = out_dir.join("openheart_persistent.log");
    init_dual_logger(Some(&persistent_log_path), Some(&session_log_path));

    log_info("================================================================================");
    log_info(" OPENHEART STATIC ANALYSIS PIPELINE STARTING");
    log_info(&format!(" Input Path  : {}", args.source.display()));
    log_info(&format!(
        " Source Files: {} ({} bytes)",
        inventory.files.len(),
        inventory.total_bytes
    ));
    log_info(&format!(" Session Log : {}", session_log_path.display()));
    log_info(&format!(" Persist Log : {}", persistent_log_path.display()));
    log_info("================================================================================");
    log_skips(&inventory);

    let stack_bytes = pipeline_stack_bytes_from_env()?;
    let work = WorkDir::create_in(&out_dir, ".openheart-run-").map_err(|err| err.to_string())?;
    let destination = out_dir.clone();
    let summary = run_isolated("openheart-pipeline", stack_bytes, move || {
        analyze_in_work_dir(&inventory, work, &destination, &pipeline)
    })??;

    for line in &summary.lines {
        log_info(line);
    }
    for gap in &summary.gaps {
        log_warn(&format!(" Diagram unavailable: {}", gap));
    }
    log_info("================================================================================");
    log_info(&format!(
        " SUCCESS: 10-phase analysis finished in {:.2?} | {} files published to {} \
         ({} diagram files, {} unavailable) | report: {}",
        started.elapsed(),
        summary.published_files,
        out_dir.display(),
        summary.diagram_files,
        summary.gaps.len(),
        out_dir.join(RUN_REPORT_FILE).display()
    ));
    log_info("================================================================================");
    Ok(())
}

fn log_skips(inventory: &SourceInventory) {
    let skipped = &inventory.skipped;
    log_info(&format!(
        " Skipped     : {} unsupported, {} test, {} oversized, {} unreadable, {} symlinked, \
         {} duplicate files; {} excluded directories",
        skipped.unsupported_total(),
        skipped.test_files,
        skipped.oversized_files,
        skipped.unreadable_entries,
        skipped.symlinks,
        skipped.duplicates,
        skipped.excluded_directories
    ));
    for example in &skipped.oversized_examples {
        log_warn(&format!(" Oversized source skipped: {}", example));
    }
}

/// Runs the pipeline in `work`, writes the run report, and publishes everything into
/// `destination`. In-memory artifacts are dropped before publishing so no file is still
/// mapped when it is renamed.
fn analyze_in_work_dir(
    inventory: &SourceInventory,
    work: WorkDir,
    destination: &Path,
    pipeline: &PipelineOptions,
) -> Result<AnalyzeSummary, String> {
    let run = run_pipeline(inventory, work.path(), pipeline).map_err(|err| err.to_string())?;
    run.write_report(inventory, work.path())
        .map_err(|err| err.to_string())?;

    let counts = run.counts();
    let ingestion = &run.ingestion;
    let ingestion_line = format!(
        " Phase 1    : {} files ingested, {} skipped ({} without a grammar, {} too large, \
         {} aborted); {} files with syntax errors ({} ERROR / {} MISSING nodes)",
        ingestion.files_ingested,
        ingestion.skipped_total(),
        ingestion.skipped_unsupported_language,
        ingestion.skipped_too_large,
        ingestion.skipped_parse_aborted,
        ingestion.files_with_syntax_errors,
        ingestion.syntax_error_nodes,
        ingestion.missing_nodes
    );
    let summary_line = format!(
        " Summary: {} tokens, {} AST nodes, {} symbols, {} functions, {} blocks, {} CFG edges, \
         {} SSA vars, {} φ-funcs, {} call sites, {} call edges, {} SCCs, {} traceability \
         links, {} ROBDD functions, {} UML classes, {} UML activities, {} design patterns | \
         SCPG Hash: 0x{:08X} | {} of 10 phases executed.",
        counts.tokens,
        counts.ast_nodes,
        counts.symbols,
        counts.functions,
        counts.basic_blocks,
        counts.cfg_edges,
        counts.ssa_variables,
        counts.phi_functions,
        counts.call_sites,
        counts.call_edges,
        counts.call_graph_sccs,
        counts.traceability_links,
        counts.robdd_functions,
        counts.uml_classes,
        counts.uml_activities,
        counts.design_patterns,
        run.scpg_hash(),
        run.phases.len()
    );
    let lines = vec![ingestion_line, summary_line];
    let gaps = run
        .diagram_gaps
        .iter()
        .map(|gap| format!("{} ({}): {}", gap.diagram_type, gap.format, gap.reason))
        .collect();
    let diagram_files = run.diagrams.len();
    drop(run);

    let published = work
        .publish_into(destination)
        .map_err(|err| err.to_string())?;
    Ok(AnalyzeSummary {
        lines,
        gaps,
        diagram_files,
        published_files: published.len(),
    })
}

// ── inspect ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
enum ArtifactKind {
    TokenCorpus,
    Ast,
    SymbolTable,
    ControlFlow,
    Ssa,
    CallGraph,
    Traceability,
    PathSummary,
    UmlMetadata,
    Scpg,
}

impl ArtifactKind {
    fn label(self) -> &'static str {
        match self {
            Self::TokenCorpus => ".tca",
            Self::Ast => ".bpa",
            Self::SymbolTable => ".sta",
            Self::ControlFlow => ".cfa",
            Self::Ssa => ".ssa",
            Self::CallGraph => ".cga",
            Self::Traceability => ".tra",
            Self::PathSummary => ".psa",
            Self::UmlMetadata => ".uma",
            Self::Scpg => ".scpg",
        }
    }

    /// Every reader except the `.scpg` loader verifies its trailing CRC-64 before
    /// decoding; `inspect` reports which case applies instead of claiming validity.
    fn integrity_line(self) -> &'static str {
        match self {
            Self::Scpg => "Checksum       : NOT VERIFIED (inspect only loads .scpg structure)",
            _ => "Checksum       : CRC-64 verified by the format reader",
        }
    }
}

fn detect_artifact(bytes: &[u8]) -> Option<ArtifactKind> {
    if bytes.len() >= 4 && bytes[0..4] == openheart::ssa::serializer::SSA_MAGIC {
        return Some(ArtifactKind::Ssa);
    }
    if bytes.len() < 8 {
        return None;
    }
    let magic = u64::from_le_bytes(bytes[0..8].try_into().ok()?);
    let kind = if magic == openheart::ingestion::serializer::TCA_MAGIC {
        ArtifactKind::TokenCorpus
    } else if magic == u64::from_le_bytes(*openheart::ast::serializer::BPA_MAGIC) {
        ArtifactKind::Ast
    } else if magic == openheart::symbol::serializer::STA_MAGIC {
        ArtifactKind::SymbolTable
    } else if magic == openheart::cfg::serializer::CFA_MAGIC {
        ArtifactKind::ControlFlow
    } else if magic == openheart::cg::serializer::CGA_MAGIC {
        ArtifactKind::CallGraph
    } else if magic == openheart::tra::TRA_MAGIC {
        ArtifactKind::Traceability
    } else if magic == openheart::psa::PSA_MAGIC {
        ArtifactKind::PathSummary
    } else if magic == openheart::uma::UMA_MAGIC {
        ArtifactKind::UmlMetadata
    } else if magic == openheart::scpg::SCPG_MAGIC {
        ArtifactKind::Scpg
    } else {
        return None;
    };
    Some(kind)
}

fn inspect_artifact(kind: ArtifactKind, path: &Path, bytes: &[u8]) -> Result<Vec<String>, String> {
    let lines = match kind {
        ArtifactKind::TokenCorpus => {
            let tca = TokenCorpusSerializer::read(path)?;
            vec![
                "Artifact Type  : Token Corpus (.tca)".to_string(),
                format!(
                    "Format Version : {}",
                    openheart::ingestion::serializer::TCA_VERSION
                ),
                format!("File Count     : {}", tca.file_records.len()),
                format!("Token Count    : {}", tca.token_records.len()),
            ]
        }
        ArtifactKind::Ast => {
            let bpa = BPASTSerializer::read(path).map_err(|err| err.to_string())?;
            vec![
                "Artifact Type  : BP Succinct AST (.bpa)".to_string(),
                format!("Node Count     : {}", bpa.node_count),
                format!("TCA Hash Link  : 0x{:016X}", bpa.tca_hash),
            ]
        }
        ArtifactKind::SymbolTable => {
            let sta = SymbolTableArtifact::deserialize(bytes).map_err(|err| err.to_string())?;
            vec![
                "Artifact Type  : Symbol Table & Hierarchy (.sta)".to_string(),
                format!("Format Version : {}", sta.format_version),
                format!("Symbol Count   : {}", sta.symbol_count),
                format!("Scope Count    : {}", sta.scope_count),
                format!("TH Edge Count  : {}", sta.th_edge_count),
                format!("BPA Hash Link  : 0x{:016X}", sta.bpa_hash),
                format!("TCA Hash Link  : 0x{:016X}", sta.tca_hash),
            ]
        }
        ArtifactKind::ControlFlow => {
            let cfa = CFGArtifact::deserialize(bytes)?;
            vec![
                "Artifact Type  : Control Flow Graph (.cfa)".to_string(),
                format!("Format Version : {}", cfa.format_version),
                format!("Function Count : {}", cfa.function_count),
                format!("Total Blocks   : {}", cfa.total_blocks),
                format!("Total Edges    : {}", cfa.total_edges),
                format!("STA Hash Link  : 0x{:016X}", cfa.sta_hash),
                format!("BPA Hash Link  : 0x{:016X}", cfa.bpa_hash),
            ]
        }
        ArtifactKind::Ssa => {
            let ssa = SSASerializer::read(path)?;
            vec![
                "Artifact Type  : Static Single Assignment (.ssa)".to_string(),
                format!("Format Version : {}", ssa.format_version),
                format!("Function Count : {}", ssa.function_count),
                format!("Total SSA Vars : {}", ssa.total_ssa_vars),
                format!("Total Phi Funcs: {}", ssa.total_phi_funcs),
                format!("CFA Hash Link  : 0x{:016X}", ssa.cfa_hash),
            ]
        }
        ArtifactKind::CallGraph => {
            let cga = CGASerializer::deserialize(path).map_err(|err| err.to_string())?;
            vec![
                "Artifact Type  : Call Graph & Points-To (.cga)".to_string(),
                format!("Format Version : {}", cga.format_version),
                format!("Method Count   : {}", cga.method_count),
                format!("Call Site Count: {}", cga.call_site_count),
                format!("Call Edge Count: {}", cga.call_edge_count),
                format!("Points-To Size : {}", cga.points_to_table.len()),
                format!("SCC Count      : {}", cga.sccs.len()),
                format!("SSA Hash Link  : 0x{:016X}", cga.ssa_hash),
                format!("STA Hash Link  : 0x{:016X}", cga.sta_hash),
            ]
        }
        ArtifactKind::Traceability => {
            let tra = openheart::tra::TraceabilitySerializer::deserialize(path)
                .map_err(|err| err.to_string())?;
            vec![
                "Artifact Type  : Traceability Index (.tra)".to_string(),
                format!("Format Version : {}", tra.format_version),
                format!("AST Node Count : {}", tra.bi_ast.len()),
                format!("Symbol Count   : {}", tra.bi_sym.len()),
                format!("Block Count    : {}", tra.bi_blk.len()),
                format!("SSA Var Count  : {}", tra.bi_ssa.len()),
                format!("Call Site Count: {}", tra.bi_cs.len()),
                format!("Symbol Spans   : {}", tra.sym_span.len()),
                format!("UMLLink Count  : {}", tra.uml_links.len()),
                format!("SCPG Composite : 0x{:08X}", tra.hashes.scpg_hash),
            ]
        }
        ArtifactKind::PathSummary => {
            let psa = openheart::psa::PathSummarySerializer::read(path)
                .map_err(|err| err.to_string())?;
            vec![
                "Artifact Type  : Path Summary ROBDD (.psa)".to_string(),
                format!("Format Version : {}", psa.format_version),
                format!("Function Count : {}", psa.function_count()),
                format!("Total ROBDD Node: {}", psa.total_nodes),
                format!("CFA Hash Link  : 0x{:016X}", psa.cfa_hash),
                format!("SSA Hash Link  : 0x{:016X}", psa.ssa_hash),
            ]
        }
        ArtifactKind::UmlMetadata => {
            let uma = openheart::uma::UMASerializer::read(path).map_err(|err| err.to_string())?;
            vec![
                "Artifact Type  : UML Semantic Metadata (.uma)".to_string(),
                format!("Format Version : {}", uma.format_version),
                format!("Class Count    : {}", uma.classes.len()),
                format!("TRA Hash Link  : 0x{:016X}", uma.tra_hash),
            ]
        }
        ArtifactKind::Scpg => {
            let engine = OpenHeartEngine::open(path).map_err(|err| err.to_string())?;
            vec![
                "Artifact Type  : Unified SCPG Binary (.scpg)".to_string(),
                format!("SCPG Hash      : 0x{:08X}", engine.scpg_hash()),
            ]
        }
    };
    Ok(lines)
}

fn cmd_inspect(path: &Path) -> Result<(), String> {
    let bytes = fs::read(path)
        .map_err(|err| format!("Failed to read '{}': {}", path.display(), err))?;
    log_info(&format!("Inspecting artifact: {}", path.display()));
    log_info(&format!("File Size      : {} bytes", bytes.len()));
    let kind = detect_artifact(&bytes).ok_or_else(|| {
        format!(
            "'{}' is not a recognised OpenHeart artifact (unknown magic number)",
            path.display()
        )
    })?;

    // Readers index into file-supplied offsets; contain a panic and report the file as
    // malformed instead of crashing with a backtrace.
    let previous_hook = panic::take_hook();
    panic::set_hook(Box::new(|_| {}));
    let outcome = panic::catch_unwind(AssertUnwindSafe(|| inspect_artifact(kind, path, &bytes)));
    panic::set_hook(previous_hook);

    let lines = match outcome {
        Ok(Ok(lines)) => lines,
        Ok(Err(reason)) => {
            return Err(format!(
                "'{}' is not a valid {} artifact: {}",
                path.display(),
                kind.label(),
                reason
            ))
        }
        Err(payload) => {
            return Err(format!(
                "the {} reader failed on '{}' ({}); the file is corrupted or malformed",
                kind.label(),
                path.display(),
                panic_message(payload.as_ref())
            ))
        }
    };
    for line in &lines {
        log_info(line);
    }
    log_info(kind.integrity_line());
    Ok(())
}

// ── server & health ──────────────────────────────────────────────────────────

fn cmd_server(args: ServerArgs) -> Result<(), String> {
    let mut config = ServerConfig::from_env()?;
    if let Some(host) = args.host {
        config.host = host;
    }
    if let Some(port) = args.port {
        config.port = port;
    }
    if let Some(dir) = args.workspace {
        config.workspace = dir;
    }
    if let Some(dir) = args.web_root {
        config.web_root = dir;
    }
    if !args.source_roots.is_empty() {
        config.source_roots = args.source_roots;
    }
    if let Some(dir) = args.local_repos {
        config.local_repos_root = Some(dir);
    }
    if args.allow_clone.is_some() {
        config.allow_clone = args.allow_clone;
    }
    OpenHeartServer::with_config(config).start()
}

fn probe_address(host: &str, port: u16) -> Result<SocketAddr, String> {
    let bare = host.trim().trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = bare.parse::<IpAddr>() {
        let target = match ip {
            IpAddr::V4(v4) if v4.is_unspecified() => IpAddr::V4(Ipv4Addr::LOCALHOST),
            IpAddr::V6(v6) if v6.is_unspecified() => IpAddr::V6(Ipv6Addr::LOCALHOST),
            other => other,
        };
        return Ok(SocketAddr::new(target, port));
    }
    (bare, port)
        .to_socket_addrs()
        .map_err(|err| format!("cannot resolve '{}': {}", host, err))?
        .next()
        .ok_or_else(|| format!("'{}' resolved to no address", host))
}

fn cmd_health(args: &HealthArgs) -> Result<(), String> {
    let config = ServerConfig::from_env()?;
    let host = args.host.clone().unwrap_or(config.host);
    let address = probe_address(&host, args.port.unwrap_or(config.port))?;

    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(3))
        .map_err(|err| format!("cannot connect to {}: {}", address, err))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .and_then(|()| stream.set_write_timeout(Some(Duration::from_secs(5))))
        .map_err(|err| format!("cannot configure the probe socket: {}", err))?;
    let request = format!(
        "GET /api/health HTTP/1.1\r\nHost: {}\r\nAccept: application/json\r\nConnection: close\r\n\r\n",
        address
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|err| format!("cannot send the probe to {}: {}", address, err))?;
    let mut response = Vec::new();
    Read::take(&mut stream, 64 * 1024)
        .read_to_end(&mut response)
        .map_err(|err| format!("cannot read the probe response from {}: {}", address, err))?;

    let text = String::from_utf8_lossy(&response);
    let status_line = text.lines().next().unwrap_or("").trim();
    if status_line.starts_with("HTTP/1.1 200") || status_line.starts_with("HTTP/1.0 200") {
        println!("healthy: {} answered '{}'", address, status_line);
        Ok(())
    } else {
        Err(format!(
            "unhealthy: {} answered '{}'",
            address,
            if status_line.is_empty() {
                "<no response>"
            } else {
                status_line
            }
        ))
    }
}
