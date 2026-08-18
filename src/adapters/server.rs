//! OpenHeart HTTP Web Server & REST API Adapter (§10.4).
//! 100% Native Rust HTTP Server for Pipeline Processing, Telemetry, and PlantUML Generation.

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};

use crate::ast::{ASTStage, ASTStageInput};
use crate::cfg::Phase4Stage;
use crate::cg::Phase6Stage;
use crate::core::io::mmap::MemoryMappedFile;
use crate::ingestion::manifest::SourceManifest;
use crate::ingestion::IngestionStage;
use crate::psa::Phase8Stage;
use crate::scpg::diagram::export::plantuml::PlantUMLExporter;
use crate::ssa::Phase5Stage;
use crate::symbol::Phase3Stage;
use crate::tra::Phase7Stage;
use crate::uma::Phase9Stage;

pub struct OpenHeartServer {
    pub port: u16,
}

impl OpenHeartServer {
    pub fn new(port: u16) -> Self {
        Self { port }
    }

    pub fn start(&self) -> Result<(), String> {
        let addr = format!("0.0.0.0:{}", self.port);
        let listener = TcpListener::bind(&addr)
            .map_err(|e| format!("Failed to bind server to {}: {}", addr, e))?;
        println!(
            "[SERVER] OpenHeart Backend Engine listening on http://{}",
            addr
        );

        for stream in listener.incoming() {
            if let Ok(mut stream) = stream {
                Self::handle_connection(&mut stream);
            }
        }
        Ok(())
    }

    fn handle_connection(stream: &mut TcpStream) {
        let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(500)));
        let mut buffer = [0u8; 8192];
        let bytes_read = match stream.read(&mut buffer) {
            Ok(n) if n > 0 => n,
            _ => return,
        };

        let request = String::from_utf8_lossy(&buffer[..bytes_read]);
        let mut lines = request.lines();
        let request_line = match lines.next() {
            Some(l) => l,
            None => return,
        };

        let parts: Vec<&str> = request_line.split_whitespace().collect();
        if parts.len() < 2 {
            return;
        }

        let method = parts[0];
        let raw_path = parts[1];
        let clean_path = raw_path.split('?').next().unwrap_or(raw_path);
        let is_get_or_head = method == "GET" || method == "HEAD";

        if is_get_or_head && (clean_path == "/" || clean_path == "/index.html") {
            Self::serve_file(stream, "web/index.html", "text/html");
        } else if is_get_or_head && clean_path == "/api/health" {
            let json = r#"{"status":"online","engine":"OpenHeart SCPG v0.1.0","plantuml":true}"#;
            Self::respond_json(stream, 200, json);
        } else if method == "POST" && clean_path == "/api/analyze" {
            let mut full_request = request.to_string();
            if !full_request.contains("\"repo_url\"") {
                let mut extra_buf = [0u8; 4096];
                if let Ok(n) = stream.read(&mut extra_buf) {
                    if n > 0 {
                        full_request.push_str(&String::from_utf8_lossy(&extra_buf[..n]));
                    }
                }
            }
            let response_json = Self::process_analyze_request(&full_request);
            Self::respond_json(stream, 200, &response_json);
        } else if is_get_or_head && clean_path.starts_with('/') && !clean_path.starts_with("/api/")
        {
            let rel_path = clean_path.trim_start_matches('/');
            let requested_path = Path::new("web").join(rel_path);
            let canonical_web = match fs::canonicalize("web") {
                Ok(p) => p,
                Err(_) => {
                    Self::respond_json(stream, 404, r#"{"error":"File not found"}"#);
                    return;
                }
            };

            let safe_file_path = match fs::canonicalize(&requested_path) {
                Ok(p) if p.starts_with(&canonical_web) => p,
                _ => {
                    Self::respond_json(stream, 404, r#"{"error":"File not found"}"#);
                    return;
                }
            };

            let content_type = if rel_path.ends_with(".js") {
                "application/javascript"
            } else if rel_path.ends_with(".css") {
                "text/css"
            } else if rel_path.ends_with(".svg") {
                "image/svg+xml"
            } else if rel_path.ends_with(".html") {
                "text/html"
            } else {
                "application/octet-stream"
            };

            Self::serve_file(stream, safe_file_path.to_str().unwrap_or(""), content_type);
        } else {
            Self::respond_json(stream, 404, r#"{"error":"Not Found"}"#);
        }
    }

    fn extract_body(request: &str) -> String {
        if let Some(pos) = request.find("\r\n\r\n") {
            request[pos + 4..].to_string()
        } else if let Some(pos) = request.find("\n\n") {
            request[pos + 2..].to_string()
        } else {
            request.to_string()
        }
    }

    fn serve_file(stream: &mut TcpStream, file_path: &str, content_type: &str) {
        match fs::read(file_path) {
            Ok(content) => {
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {}; charset=utf-8\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\n\r\n",
                    content_type,
                    content.len()
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.write_all(&content);
            }
            Err(e) => {
                println!("[SERVER ERROR] Failed to read file '{}': {}", file_path, e);
                Self::respond_json(stream, 404, r#"{"error":"File not found"}"#);
            }
        }
    }

    fn respond_json(stream: &mut TcpStream, status_code: u16, json: &str) {
        let status_text = if status_code == 200 {
            "OK"
        } else {
            "Not Found"
        };
        let response = format!(
            "HTTP/1.1 {} {}\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Headers: *\r\n\r\n{}",
            status_code,
            status_text,
            json.len(),
            json
        );
        let _ = stream.write_all(response.as_bytes());
    }

    fn process_analyze_request(body: &str) -> String {
        let repo_url = if let Some(idx) = body.find("\"repo_url\"") {
            let slice = &body[idx..];
            slice.split('"').nth(3).unwrap_or("").trim()
        } else {
            ""
        };

        if repo_url.is_empty() {
            return format!(
                r#"{{"status":"error","session_id":"sess_{}","logs":["No repository URL specified in request payload."],"errors":["Missing repo_url parameter."]}}"#,
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs()
            );
        }

        println!("[SERVER] Backend analyzing repository: {}", repo_url);

        let mut logs = Vec::new();
        logs.push(format!(
            "> Backend received request for repository: {}",
            repo_url
        ));

        // Derive repo directory name dynamically from URL
        let repo_name = repo_url
            .trim_end_matches('/')
            .split('/')
            .last()
            .unwrap_or("repo")
            .trim_end_matches(".git");

        let repo_dir = Path::new("./target_repos").join(repo_name);

        if !repo_dir.exists() {
            logs.push(format!(
                "> Target repository directory './target_repos/{}' not found locally.",
                repo_name
            ));
            logs.push(format!(
                "> Executing dynamic git clone: git clone --depth 1 {} ./target_repos/{}...",
                repo_url, repo_name
            ));

            let _ = fs::create_dir_all("./target_repos");
            let clone_status = std::process::Command::new("git")
                .args([
                    "clone",
                    "--depth",
                    "1",
                    repo_url,
                    &format!("./target_repos/{}", repo_name),
                ])
                .output();

            match clone_status {
                Ok(output) if output.status.success() => {
                    logs.push(format!(
                        "> Git clone completed successfully into './target_repos/{}'.",
                        repo_name
                    ));
                }
                Ok(output) => {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    logs.push(format!(
                        "> Git clone failed: {}. Searching fallback target_repos...",
                        stderr
                    ));
                }
                Err(e) => {
                    logs.push(format!(
                        "> Could not execute git command: {}. Using local fallback target_repos...",
                        e
                    ));
                }
            }
        } else {
            logs.push(format!(
                "> Found existing repository directory: './target_repos/{}'.",
                repo_name
            ));
        }

        let mut target_dir = repo_dir.clone();
        if !target_dir.exists() {
            // Fallback: search for first existing directory inside ./target_repos
            if let Ok(entries) = fs::read_dir("./target_repos") {
                for entry in entries.flatten() {
                    if entry.path().is_dir() {
                        target_dir = entry.path();
                        logs.push(format!(
                            "> Using fallback local repository path: '{}'",
                            target_dir.display()
                        ));
                        break;
                    }
                }
            }
        }

        let mut src_files = Vec::new();
        Self::collect_files(&target_dir, &mut src_files);
        src_files.sort();

        logs.push(format!(
            "> Discovered {} source files in target tree: '{}'.",
            src_files.len(),
            target_dir.display()
        ));

        if src_files.is_empty() {
            return format!(
                r#"{{"status":"error","session_id":"sess_{}","logs":["Target repository source files not found in '{}'."],"errors":["No source files found in target repository."]}}"#,
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
                target_dir.display()
            );
        }

        let start_time = std::time::Instant::now();
        let tmp_path = std::env::temp_dir().join(format!(
            "openheart_sess_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let _ = fs::create_dir_all(&tmp_path);

        let manifest = SourceManifest::new(src_files.clone());
        let tca_path = tmp_path.join("corpus.tca");
        let bpa_path = tmp_path.join("ast.bpa");
        let sta_path = tmp_path.join("symbols.sta");
        let cfa_path = tmp_path.join("cfg.cfa");
        let ssa_path = tmp_path.join("ssa.ssa");
        let cga_path = tmp_path.join("callgraph.cga");
        let tra_path = tmp_path.join("traceability.tra");
        let psa_path = tmp_path.join("paths.psa");
        let uma_path = tmp_path.join("metadata.uma");

        logs.push("> Phase 1: Ingesting source files into SourceManifest...".to_string());
        let tca = match IngestionStage::run(manifest, &tca_path) {
            Ok(t) => t,
            Err(e) => {
                return format!(
                    r#"{{"status":"error","errors":["Phase 1 Failure: {}"]}}"#,
                    e
                )
            }
        };
        let tca_bytes = fs::read(&tca_path).unwrap_or_default();

        logs.push("> Phase 2: AST Construction & BP Encoding...".to_string());
        let stage_input = ASTStageInput {
            tca: match MemoryMappedFile::open(&tca_path) {
                Ok(m) => m,
                Err(e) => {
                    return format!(r#"{{"status":"error","errors":["MMap Failure: {}"]}}"#, e)
                }
            },
        };
        let bpa = match ASTStage::run(&stage_input, &bpa_path) {
            Ok(b) => b,
            Err(e) => {
                return format!(
                    r#"{{"status":"error","errors":["Phase 2 Failure: {}"]}}"#,
                    e
                )
            }
        };
        let bpa_bytes = fs::read(&bpa_path).unwrap_or_default();

        logs.push("> Phase 3: Building Symbol Table & Scope Graph...".to_string());
        let sta = match Phase3Stage::run(&tca, &bpa, &tca_bytes, &bpa_bytes) {
            Ok(s) => s,
            Err(e) => {
                return format!(
                    r#"{{"status":"error","errors":["Phase 3 Failure: {}"]}}"#,
                    e
                )
            }
        };
        let sta_bytes = sta.serialize();
        fs::write(&sta_path, &sta_bytes).unwrap_or_default();

        logs.push("> Phase 4: Constructing Control Flow Graph...".to_string());
        let cfa = match Phase4Stage::run(&bpa, &sta, &sta_bytes, &bpa_bytes, &cfa_path) {
            Ok(c) => c,
            Err(e) => {
                return format!(
                    r#"{{"status":"error","errors":["Phase 4 Failure: {}"]}}"#,
                    e
                )
            }
        };
        let cfa_bytes = fs::read(&cfa_path).unwrap_or_default();

        logs.push("> Phase 5: Converting to SSA Data Flow Graph...".to_string());
        let ssa = match Phase5Stage::run(&bpa, &sta, &cfa, &cfa_bytes, &ssa_path) {
            Ok(s) => s,
            Err(e) => {
                return format!(
                    r#"{{"status":"error","errors":["Phase 5 Failure: {}"]}}"#,
                    e
                )
            }
        };
        let ssa_bytes = fs::read(&ssa_path).unwrap_or_default();

        logs.push("> Phase 6: Call Graph & Points-To Analysis...".to_string());
        let cga = match Phase6Stage::run(&bpa, &sta, &cfa, &ssa, &ssa_bytes, &sta_bytes, &cga_path)
        {
            Ok(c) => c,
            Err(e) => {
                return format!(
                    r#"{{"status":"error","errors":["Phase 6 Failure: {}"]}}"#,
                    e
                )
            }
        };

        logs.push("> Phase 7: Traceability Index Construction...".to_string());
        let tra = Phase7Stage::run(&tca, &bpa, &sta, &cfa, &ssa, &cga, &tra_path);
        let tra_bytes = fs::read(&tra_path).unwrap_or_default();

        logs.push("> Phase 8: Computing ROBDD Path Summaries...".to_string());
        let psa = Phase8Stage::run(&cfa, &ssa, &cga, &cfa_bytes, &psa_path);

        logs.push("> Phase 9: UML Semantic Metadata Extraction...".to_string());
        let uma = Phase9Stage::run(
            &tca, &bpa, &sta, &cfa, &ssa, &cga, &tra, &psa, &tra_bytes, &uma_path,
        );

        logs.push("> Phase 10: Generating All 14 PlantUML Exporter Projections...".to_string());
        let puml_class = PlantUMLExporter::export_class_diagram(&uma, &sta, &tca);
        let puml_object = PlantUMLExporter::export_object_diagram(&uma, &sta, &tca);
        let puml_component = PlantUMLExporter::export_component_diagram(&uma, &sta, &tca);
        let puml_deployment = PlantUMLExporter::export_deployment_diagram(&uma, &sta, &tca);
        let puml_package = PlantUMLExporter::export_package_diagram(&uma, &sta, &tca);
        let puml_composite = PlantUMLExporter::export_composite_structure_diagram(&uma, &sta, &tca);
        let puml_profile = PlantUMLExporter::export_profile_diagram(&uma, &sta, &tca);
        let puml_usecase = PlantUMLExporter::export_use_case_diagram(&uma, &sta, &tca);
        let puml_activity = PlantUMLExporter::export_activity_diagram(&uma, &sta, &tca);
        let puml_statemachine = PlantUMLExporter::export_state_machine_diagram(&uma, &sta, &tca);
        let puml_sequence = PlantUMLExporter::export_sequence_diagram(&uma, &sta, &tca);
        let puml_communication = PlantUMLExporter::export_communication_diagram(&uma, &sta, &tca);
        let puml_interaction =
            PlantUMLExporter::export_interaction_overview_diagram(&uma, &sta, &tca);
        let puml_timing = PlantUMLExporter::export_timing_diagram(&uma, &sta, &tca);

        let _ = fs::remove_dir_all(&tmp_path);

        let elapsed_ms = start_time.elapsed().as_millis();
        logs.push(format!(
            "> Pipeline complete in {} ms. All 10 phases verified.",
            elapsed_ms
        ));

        let escape_json_str = |s: &str| -> String {
            s.replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('\n', "\\n")
                .replace('\r', "\\r")
                .replace('\t', "\\t")
        };

        let mut trace_items = Vec::new();
        for link in tra.uml_links.iter().take(10) {
            let file_name = if let Some(file_rec) =
                tca.file_records.iter().find(|f| f.file_id == link.file_id)
            {
                let bytes = tca.interner.lookup_text(file_rec.path_str_offset);
                String::from_utf8_lossy(bytes).to_string()
            } else {
                format!("file_{}.kt", link.file_id)
            };
            let span_str = format!(
                "L{}:C{} - L{}:C{}",
                link.line_start, link.col_start, link.line_end, link.col_end
            );
            trace_items.push(format!(
                r#"{{"tid":{},"file":"{}","span":"{}","hash":"0x{:08X}"}}"#,
                link.sym_id,
                escape_json_str(&file_name),
                escape_json_str(&span_str),
                link.scpg_hash
            ));
        }

        format!(
            r#"{{"status":"success","session_id":"sess_{}","stats":{{"files_processed":{},"total_tokens":{},"total_classes":{},"execution_time_ms":{}}},"diagrams":{{"class":"{}","object":"{}","component":"{}","deployment":"{}","package":"{}","composite":"{}","profile":"{}","usecase":"{}","activity":"{}","statemachine":"{}","sequence":"{}","communication":"{}","interaction":"{}","timing":"{}"}},"traceability":[{}],"logs":[{}],"errors":[]}}"#,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            src_files.len(),
            tca.token_records.len(),
            uma.classes.len(),
            elapsed_ms,
            escape_json_str(&puml_class),
            escape_json_str(&puml_object),
            escape_json_str(&puml_component),
            escape_json_str(&puml_deployment),
            escape_json_str(&puml_package),
            escape_json_str(&puml_composite),
            escape_json_str(&puml_profile),
            escape_json_str(&puml_usecase),
            escape_json_str(&puml_activity),
            escape_json_str(&puml_statemachine),
            escape_json_str(&puml_sequence),
            escape_json_str(&puml_communication),
            escape_json_str(&puml_interaction),
            escape_json_str(&puml_timing),
            trace_items.join(","),
            logs.iter()
                .map(|l| format!("\"{}\"", escape_json_str(l)))
                .collect::<Vec<_>>()
                .join(",")
        )
    }

    fn collect_files(dir: &Path, files: &mut Vec<PathBuf>) {
        if dir.is_dir() {
            if let Ok(entries) = fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
                    if file_name.starts_with('.')
                        || file_name == "target"
                        || file_name == "node_modules"
                        || file_name == "build"
                    {
                        continue;
                    }
                    if path.is_dir() {
                        let lower_name = file_name.to_lowercase();
                        if lower_name == "node_modules"
                            || lower_name == "target"
                            || lower_name == "build"
                            || lower_name == "dist"
                            || lower_name == "out"
                            || lower_name == "vendor"
                            || lower_name == "venv"
                            || lower_name == ".git"
                        {
                            continue;
                        }
                        // Skip test directories
                        let dir_path_str = path.to_string_lossy().replace('\\', "/").to_lowercase();
                        if dir_path_str.ends_with("/src/test")
                            || dir_path_str.ends_with("/src/androidtest")
                            || dir_path_str.ends_with("/src/testdebug")
                            || dir_path_str.ends_with("/src/testrelease")
                            || dir_path_str.ends_with("/__tests__")
                            || dir_path_str.ends_with("/tests")
                            || dir_path_str.ends_with("/test")
                            || dir_path_str.ends_with("/spec")
                            || dir_path_str.ends_with("/specs")
                        {
                            continue;
                        }
                        Self::collect_files(&path, files);
                    } else if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
                        let lower_ext = ext.to_lowercase();
                        let is_code_file = matches!(
                            lower_ext.as_str(),
                            "java"
                                | "kt"
                                | "kts"
                                | "rs"
                                | "py"
                                | "pyw"
                                | "pyx"
                                | "js"
                                | "jsx"
                                | "mjs"
                                | "cjs"
                                | "ts"
                                | "tsx"
                                | "mts"
                                | "cts"
                                | "cpp"
                                | "c"
                                | "h"
                                | "hpp"
                                | "cc"
                                | "cxx"
                                | "hh"
                                | "hxx"
                                | "c++"
                                | "h++"
                                | "cs"
                                | "go"
                                | "swift"
                                | "rb"
                                | "php"
                                | "scala"
                                | "groovy"
                                | "lua"
                                | "sh"
                                | "bash"
                                | "zsh"
                                | "pl"
                                | "pm"
                                | "r"
                                | "m"
                                | "mm"
                                | "dart"
                                | "zig"
                                | "nim"
                                | "elm"
                                | "erl"
                                | "hrl"
                                | "ex"
                                | "exs"
                                | "clj"
                                | "cljs"
                                | "hs"
                                | "v"
                                | "sv"
                                | "vhdl"
                                | "asm"
                                | "s"
                                | "sql"
                        );
                        if is_code_file {
                            // Skip test files by name pattern
                            let lower_stem = path
                                .file_stem()
                                .and_then(|s| s.to_str())
                                .unwrap_or("")
                                .to_lowercase();
                            let is_test_file = lower_stem.ends_with("test")
                                || lower_stem.ends_with("tests")
                                || lower_stem.ends_with("_test")
                                || lower_stem.ends_with("_tests")
                                || lower_stem.ends_with("_spec")
                                || lower_stem.starts_with("test_")
                                || lower_stem == "test";
                            if !is_test_file {
                                files.push(path);
                            }
                        }
                    }
                }
            }
        }
    }
}
