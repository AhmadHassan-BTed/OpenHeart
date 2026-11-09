//! OpenHeart HTTP Web Server & REST API Adapter (§10.4).
//!
//! A std-only HTTP/1.1 server for the web studio and the REST API:
//!
//! * binds `127.0.0.1` unless another interface is configured explicitly, and requires an
//!   access token on any non-loopback bind (generated and printed when none is set);
//! * validates Host headers, refuses cross-site requests, and grants CORS only to an
//!   explicit allowlist (see [`crate::adapters::security`]);
//! * serves connections from a bounded worker pool, answering 503 when saturated, and runs
//!   at most `max_concurrent_analyses` pipelines at once, each on its own large-stack
//!   thread with a time budget;
//! * runs the shared [`runner`](crate::adapters::runner) pipeline (all 10 phases) per
//!   analysis into a private session directory, never into the served `web/` tree;
//! * confines `/api/source` to the files an analysis (or a configured sample root) indexed.

#![forbid(unsafe_code)]

use std::collections::VecDeque;
use std::fs;
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use serde_json::{json, Value};

use crate::adapters::env;
use crate::adapters::http::{
    percent_encode_path, read_request, write_response, Request, RequestLimits, Response,
};
use crate::adapters::runner::{
    canonical_diagram_type, canonicalize_clean, discover_sources, panic_message,
    parse_diagram_format, pipeline_stack_bytes_from_env, run_pipeline, DiscoveryOptions,
    PipelineOptions, PipelineRun, SourceInventory, SourceLimits, ANALYSIS_GRAPH_TYPES,
    DEFAULT_PIPELINE_STACK_BYTES, RUN_REPORT_FILE, SOURCE_LANGUAGES, UML_DIAGRAM_TYPES,
};
use crate::adapters::security::{self, AccessPolicy, Caller, Denial};
use crate::adapters::session::{self, AnalysisSession, SessionStore, SourceIndex, SourceLookup};
use crate::adapters::web_repo::{CheckedOutRepo, FetchOptions, GitHubRepoRef, WebRepoFetcher};
use crate::core::logger::{log_debug, log_error, log_info, log_warn};
use crate::scpg::diagram::DiagramFormat;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const WRITE_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_HEAD_BYTES: usize = 16 * 1024;
const TRACE_PREVIEW_LIMIT: usize = 10;
const DESIGN_PATTERNS: [&str; 11] = [
    "Singleton",
    "Observer",
    "Factory",
    "Builder",
    "State",
    "TemplateMethod",
    "Decorator",
    "Strategy",
    "Adapter",
    "Facade",
    "Composite",
];

// ── Configuration ────────────────────────────────────────────────────────────

/// Server configuration. [`ServerConfig::default`] is loopback-only;
/// [`ServerConfig::from_env`] applies the `OPENHEART_*` overrides documented in
/// `.env.example`.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Interface to bind (`127.0.0.1` by default).
    pub host: String,
    /// TCP port.
    pub port: u16,
    /// Directory holding the web studio.
    pub web_root: PathBuf,
    /// Directory for cached clones (`repos/`) and analysis sessions (`sessions/`).
    pub workspace: PathBuf,
    /// Source trees `/api/source` may serve when no analysis session is selected.
    pub source_roots: Vec<PathBuf>,
    /// Directory whose sub-directories may be analyzed by name (`"local_repo"`); disabled
    /// when `None`.
    pub local_repos_root: Option<PathBuf>,
    /// Whether missing repositories may be cloned; `None` enables cloning only when the
    /// server is bound to a loopback interface.
    pub allow_clone: Option<bool>,
    /// Access token; one is generated for non-loopback binds when this is `None`.
    pub api_token: Option<String>,
    /// Extra host names accepted in the Host header (loopback names and IP literals are
    /// always accepted).
    pub allowed_hosts: Vec<String>,
    /// Origins granted cross-origin (CORS) access, e.g. `https://example.github.io`.
    pub cors_origins: Vec<String>,
    /// Connection worker threads.
    pub worker_threads: usize,
    /// Pipelines allowed to run at the same time.
    pub max_concurrent_analyses: usize,
    /// Completed analysis sessions kept on disk.
    pub max_sessions: usize,
    /// Largest accepted request body in bytes.
    pub max_body_bytes: usize,
    /// Time allowed to receive one request.
    pub request_timeout: Duration,
    /// Time a synchronous analysis may take before the client receives 504.
    pub analysis_timeout: Duration,
    /// Time allowed for one `git clone`.
    pub clone_timeout: Duration,
    /// Largest accepted clone in bytes.
    pub max_repo_bytes: u64,
    /// Source selection limits per analysis.
    pub source_limits: SourceLimits,
    /// Stack reserved for each pipeline thread.
    pub pipeline_stack_bytes: usize,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 8080,
            web_root: PathBuf::from("web"),
            workspace: PathBuf::from("target_repos"),
            source_roots: vec![PathBuf::from("examples").join("test_patterns_codebase")],
            local_repos_root: None,
            allow_clone: None,
            api_token: None,
            allowed_hosts: Vec::new(),
            cors_origins: Vec::new(),
            worker_threads: 8,
            max_concurrent_analyses: 2,
            max_sessions: 8,
            max_body_bytes: 64 * 1024,
            request_timeout: Duration::from_secs(15),
            analysis_timeout: Duration::from_secs(600),
            clone_timeout: Duration::from_secs(300),
            max_repo_bytes: 1024 * 1024 * 1024,
            source_limits: SourceLimits::default(),
            pipeline_stack_bytes: DEFAULT_PIPELINE_STACK_BYTES,
        }
    }
}

impl ServerConfig {
    /// Defaults overridden by environment variables; malformed values are errors.
    pub fn from_env() -> Result<Self, String> {
        let mut config = Self::default();
        if let Some((_, host)) = env::first_string(&["OPENHEART_HOST", "HOST"]) {
            config.host = host;
        }
        if let Some((name, raw)) = env::first_string(&["OPENHEART_PORT", "PORT"]) {
            config.port = raw
                .parse()
                .map_err(|_| format!("environment variable {} has invalid port '{}'", name, raw))?;
        }
        if let Some(dir) = env::string("OPENHEART_WEB_ROOT") {
            config.web_root = PathBuf::from(dir);
        }
        if let Some(dir) = env::string("OPENHEART_WORKSPACE") {
            config.workspace = PathBuf::from(dir);
        }
        if let Some(roots) = env::paths("OPENHEART_SOURCE_ROOTS") {
            config.source_roots = roots;
        }
        if let Some(dir) = env::string("OPENHEART_LOCAL_REPOS") {
            config.local_repos_root = Some(PathBuf::from(dir));
        }
        config.allow_clone = env::flag("OPENHEART_ALLOW_CLONE")?;
        config.api_token = env::string("OPENHEART_API_TOKEN");
        config.allowed_hosts = env::list("OPENHEART_ALLOWED_HOSTS");
        config.cors_origins = env::list("OPENHEART_CORS_ORIGINS");
        if let Some(count) = env::number::<usize>("OPENHEART_WORKERS")? {
            config.worker_threads = count.clamp(1, 256);
        }
        if let Some(count) = env::number::<usize>("OPENHEART_MAX_ANALYSES")? {
            config.max_concurrent_analyses = count.clamp(1, 64);
        }
        if let Some(count) = env::number::<usize>("OPENHEART_MAX_SESSIONS")? {
            config.max_sessions = count.clamp(1, 1024);
        }
        if let Some(kib) = env::number::<usize>("OPENHEART_MAX_BODY_KB")? {
            config.max_body_bytes = kib.clamp(1, 16 * 1024) * 1024;
        }
        if let Some(secs) = env::number::<u64>("OPENHEART_ANALYSIS_TIMEOUT_SECS")? {
            config.analysis_timeout = Duration::from_secs(secs.max(1));
        }
        if let Some(secs) = env::number::<u64>("OPENHEART_CLONE_TIMEOUT_SECS")? {
            config.clone_timeout = Duration::from_secs(secs.max(1));
        }
        if let Some(mib) = env::number::<u64>("OPENHEART_MAX_REPO_MB")? {
            config.max_repo_bytes = mib.max(1).saturating_mul(1024 * 1024);
        }
        config.source_limits = SourceLimits::from_env()?;
        config.pipeline_stack_bytes = pipeline_stack_bytes_from_env()?;
        Ok(config)
    }
}

/// Tokens travel in cookies and headers, so they must be visible ASCII without
/// separators.
fn validate_token(token: Option<&str>) -> Result<(), String> {
    let Some(token) = token else {
        return Ok(());
    };
    let usable = token
        .bytes()
        .all(|byte| byte.is_ascii_graphic() && !b";,\"\\".contains(&byte));
    if !usable {
        return Err(
            "OPENHEART_API_TOKEN may only contain visible ASCII characters other than \
             ; , \" and \\"
                .to_string(),
        );
    }
    if token.len() < 16 {
        log_warn("OPENHEART_API_TOKEN is shorter than 16 characters; use a longer random value");
    }
    Ok(())
}

// ── Server ───────────────────────────────────────────────────────────────────

/// The OpenHeart HTTP server.
pub struct OpenHeartServer {
    /// Configuration used by [`OpenHeartServer::start`].
    pub config: ServerConfig,
}

impl OpenHeartServer {
    /// A loopback-only server on `port` with default settings.
    pub fn new(port: u16) -> Self {
        Self {
            config: ServerConfig {
                port,
                ..ServerConfig::default()
            },
        }
    }

    /// A server with an explicit configuration.
    pub fn with_config(config: ServerConfig) -> Self {
        Self { config }
    }

    /// Binds the listener and serves until accepting fails irrecoverably.
    pub fn start(&self) -> Result<(), String> {
        let mut config = self.config.clone();
        let bind_addr = resolve_bind_address(&config.host, config.port)?;
        let loopback = bind_addr.ip().is_loopback();
        let generated_token = if config.api_token.is_none() && !loopback {
            let token = security::random_hex(24);
            config.api_token = Some(token.clone());
            Some(token)
        } else {
            None
        };
        validate_token(config.api_token.as_deref())?;
        let allow_clone = config.allow_clone.unwrap_or(loopback);

        let state = Arc::new(ServerState::initialize(config, allow_clone)?);
        let listener = TcpListener::bind(bind_addr)
            .map_err(|err| format!("Failed to bind server to {}: {}", bind_addr, err))?;
        announce(&state, bind_addr, generated_token.as_deref());
        serve(state, listener)
    }
}

fn resolve_bind_address(host: &str, port: u16) -> Result<SocketAddr, String> {
    let bare = host.trim().trim_start_matches('[').trim_end_matches(']');
    (bare, port)
        .to_socket_addrs()
        .map_err(|err| format!("cannot resolve bind address '{}': {}", host, err))?
        .next()
        .ok_or_else(|| format!("bind address '{}' resolved to nothing", host))
}

fn announce(state: &ServerState, bind_addr: SocketAddr, generated_token: Option<&str>) {
    let loopback = bind_addr.ip().is_loopback();
    log_info(&format!(
        "[SERVER] OpenHeart {} listening on http://{}",
        VERSION, bind_addr
    ));
    log_info(&format!(
        "[SERVER] Workspace: {} | server-side cloning: {} | concurrent analyses: {}",
        state.fetch.repos_dir.display(),
        if state.fetch.allow_clone {
            "enabled"
        } else {
            "disabled"
        },
        state.config.max_concurrent_analyses
    ));
    if !loopback {
        log_warn(&format!(
            "[SERVER] Bound to non-loopback interface {}: the API is reachable from the \
             network and every request requires the access token.",
            bind_addr.ip()
        ));
    }
    let login_host = match bind_addr.ip() {
        ip if ip.is_unspecified() => "localhost".to_string(),
        IpAddr::V6(ip) => format!("[{}]", ip),
        IpAddr::V4(ip) => ip.to_string(),
    };
    match generated_token {
        // Printed directly rather than logged so the secret never lands in log files.
        Some(token) => println!(
            "[SERVER] Access token generated (set OPENHEART_API_TOKEN to choose one). \
             Open: http://{}:{}/?token={}",
            login_host,
            bind_addr.port(),
            token
        ),
        None if state.policy.token_required() => log_info(&format!(
            "[SERVER] Access token required (OPENHEART_API_TOKEN). \
             Open: http://{}:{}/?token=<token>",
            login_host,
            bind_addr.port()
        )),
        None => log_info(&format!(
            "[SERVER] Studio: http://{}:{}/",
            login_host,
            bind_addr.port()
        )),
    }
}

fn serve(state: Arc<ServerState>, listener: TcpListener) -> Result<(), String> {
    let workers = state.config.worker_threads.max(1);
    let (sender, receiver) = mpsc::sync_channel::<TcpStream>(workers * 8);
    let receiver = Arc::new(Mutex::new(receiver));
    for index in 0..workers {
        let state = Arc::clone(&state);
        let receiver = Arc::clone(&receiver);
        thread::Builder::new()
            .name(format!("openheart-http-{}", index))
            .spawn(move || worker_loop(&state, &receiver))
            .map_err(|err| format!("cannot start HTTP worker thread: {}", err))?;
    }
    for incoming in listener.incoming() {
        match incoming {
            Ok(stream) => dispatch_connection(&sender, stream),
            Err(err) => log_warn(&format!("[SERVER] Failed to accept a connection: {}", err)),
        }
    }
    Ok(())
}

fn dispatch_connection(sender: &SyncSender<TcpStream>, stream: TcpStream) {
    match sender.try_send(stream) {
        Ok(()) => {}
        Err(TrySendError::Full(mut stream)) => {
            let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
            let busy = Response::error(503, "the server is at capacity; retry shortly")
                .header("Retry-After", "5");
            let _ = write_response(&mut stream, &busy, true);
        }
        Err(TrySendError::Disconnected(_)) => {
            log_error("[SERVER] All HTTP worker threads have stopped; dropping connection");
        }
    }
}

fn worker_loop(state: &Arc<ServerState>, receiver: &Mutex<Receiver<TcpStream>>) {
    loop {
        let next = lock(receiver).recv();
        let Ok(mut stream) = next else {
            return;
        };
        let outcome = panic::catch_unwind(AssertUnwindSafe(|| {
            handle_connection(state, &mut stream);
        }));
        if let Err(payload) = outcome {
            log_error(&format!(
                "[SERVER] Request handler panicked: {}",
                panic_message(payload.as_ref())
            ));
            let internal_error = Response::error(500, "internal server error");
            let _ = write_response(&mut stream, &internal_error, true);
        }
    }
}

fn handle_connection(state: &Arc<ServerState>, stream: &mut TcpStream) {
    let _ = stream.set_write_timeout(Some(WRITE_TIMEOUT));
    let request = match read_request(stream, &state.request_limits) {
        Ok(request) => request,
        Err(err) => {
            if let Some(status) = err.status() {
                let _ = write_response(stream, &Response::error(status, &err.message()), true);
            }
            return;
        }
    };
    let started = Instant::now();
    let response = route(state, &request);
    if let Err(err) = write_response(stream, &response, !request.is_head()) {
        log_debug(&format!("[SERVER] Failed to send response: {}", err));
    }
    let line = format!(
        "[SERVER] {} {} -> {} ({} ms)",
        request.method,
        request.path,
        response.status(),
        started.elapsed().as_millis()
    );
    if response.status() >= 500 {
        log_warn(&line);
    } else {
        log_debug(&line);
    }
}

// ── Shared state ─────────────────────────────────────────────────────────────

struct ServerState {
    config: ServerConfig,
    policy: AccessPolicy,
    request_limits: RequestLimits,
    web_root: Option<PathBuf>,
    local_repos: Option<PathBuf>,
    fetch: FetchOptions,
    sessions: SessionStore,
    jobs: JobStore,
    analyses: Arc<Semaphore>,
    sample_sources: Vec<SourceIndex>,
    session_cookie: String,
}

impl ServerState {
    fn initialize(config: ServerConfig, allow_clone: bool) -> Result<Self, String> {
        fs::create_dir_all(&config.workspace).map_err(|err| {
            format!(
                "cannot create workspace '{}': {}",
                config.workspace.display(),
                err
            )
        })?;
        let workspace = canonicalize_clean(&config.workspace)
            .map_err(|err| format!("cannot resolve the workspace: {}", err))?;
        let repos_dir = workspace.join("repos");
        fs::create_dir_all(&repos_dir)
            .map_err(|err| format!("cannot create '{}': {}", repos_dir.display(), err))?;
        let sessions = SessionStore::open(workspace.join("sessions"), config.max_sessions)
            .map_err(|err| format!("cannot open the session directory: {}", err))?;

        let web_root = match canonicalize_clean(&config.web_root) {
            Ok(root) if root.is_dir() => Some(root),
            _ => {
                log_warn(&format!(
                    "[SERVER] Web root '{}' not found; only the API is served",
                    config.web_root.display()
                ));
                None
            }
        };

        let local_repos = match &config.local_repos_root {
            None => None,
            Some(dir) => match canonicalize_clean(dir) {
                Ok(root) if root.is_dir() => Some(root),
                _ => {
                    return Err(format!(
                        "OPENHEART_LOCAL_REPOS '{}' is not a directory",
                        dir.display()
                    ))
                }
            },
        };

        let policy = AccessPolicy::new(
            config.allowed_hosts.clone(),
            config.cors_origins.clone(),
            config.api_token.clone(),
            config.port,
        );
        let request_limits = RequestLimits {
            max_head_bytes: MAX_HEAD_BYTES,
            max_body_bytes: config.max_body_bytes,
            read_timeout: config.request_timeout,
        };
        let fetch = FetchOptions {
            repos_dir,
            allow_clone,
            clone_timeout: config.clone_timeout,
            max_repo_bytes: config.max_repo_bytes,
        };
        let sample_sources = index_sample_roots(&config);
        let analyses = Semaphore::new(config.max_concurrent_analyses);
        let jobs = JobStore::new(config.max_sessions.saturating_mul(4));
        let session_cookie = format!("openheart_session_{}", config.port);

        Ok(Self {
            config,
            policy,
            request_limits,
            web_root,
            local_repos,
            fetch,
            sessions,
            jobs,
            analyses,
            sample_sources,
            session_cookie,
        })
    }

    fn session_cookie(&self, id: &str) -> String {
        format!(
            "{}={}; Path=/; HttpOnly; SameSite=Strict",
            self.session_cookie, id
        )
    }

    /// The session named by the `session` query parameter or the session cookie.
    fn selected_session(&self, request: &Request) -> Option<Arc<AnalysisSession>> {
        let id = request
            .query_param("session")
            .or_else(|| request.cookie(&self.session_cookie))?;
        self.sessions.get(id)
    }
}

fn index_sample_roots(config: &ServerConfig) -> Vec<SourceIndex> {
    let options = DiscoveryOptions {
        limits: config.source_limits,
        include_tests: true,
        respect_ignore_files: false,
    };
    let mut indexes = Vec::new();
    for root in &config.source_roots {
        if !root.is_dir() {
            log_debug(&format!(
                "[SERVER] Sample source root '{}' does not exist; skipped",
                root.display()
            ));
            continue;
        }
        match discover_sources(root, &options) {
            Ok(inventory) => {
                let index = SourceIndex::from_inventory(&inventory);
                log_info(&format!(
                    "[SERVER] Serving {} sample sources from '{}'",
                    index.len(),
                    root.display()
                ));
                indexes.push(index);
            }
            Err(err) => log_warn(&format!(
                "[SERVER] Cannot index sample root '{}': {}",
                root.display(),
                err
            )),
        }
    }
    indexes
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Counting semaphore bounding concurrent pipelines.
struct Semaphore {
    available: Mutex<usize>,
    capacity: usize,
}

/// One acquired analysis slot, released on drop.
struct Permit {
    semaphore: Arc<Semaphore>,
}

impl Semaphore {
    fn new(capacity: usize) -> Arc<Self> {
        let capacity = capacity.max(1);
        Arc::new(Self {
            available: Mutex::new(capacity),
            capacity,
        })
    }

    fn try_acquire(self: &Arc<Self>) -> Option<Permit> {
        let mut available = lock(&self.available);
        if *available == 0 {
            return None;
        }
        *available -= 1;
        Some(Permit {
            semaphore: Arc::clone(self),
        })
    }

    fn in_use(&self) -> usize {
        self.capacity - *lock(&self.available)
    }
}

impl Drop for Permit {
    fn drop(&mut self) {
        *lock(&self.semaphore.available) += 1;
    }
}

enum JobState {
    Running { started: Instant },
    Finished { status: u16, body: Value },
}

/// Bounded record of asynchronous analyses (`"async": true`).
struct JobStore {
    entries: Mutex<VecDeque<(String, JobState)>>,
    capacity: usize,
}

impl JobStore {
    fn new(capacity: usize) -> Self {
        Self {
            entries: Mutex::new(VecDeque::new()),
            capacity: capacity.max(1),
        }
    }

    fn start(&self, id: &str) {
        let mut entries = lock(&self.entries);
        entries.push_back((
            id.to_string(),
            JobState::Running {
                started: Instant::now(),
            },
        ));
        while entries.len() > self.capacity {
            let finished = entries
                .iter()
                .position(|(_, state)| matches!(state, JobState::Finished { .. }));
            match finished {
                Some(index) => {
                    entries.remove(index);
                }
                None => break,
            }
        }
    }

    fn finish(&self, id: &str, status: u16, body: Value) {
        let mut entries = lock(&self.entries);
        if let Some((_, state)) = entries.iter_mut().find(|(key, _)| key == id) {
            *state = JobState::Finished { status, body };
        }
    }

    fn describe(&self, id: &str) -> Option<Value> {
        let entries = lock(&self.entries);
        let (_, state) = entries.iter().find(|(key, _)| key == id)?;
        Some(match state {
            JobState::Running { started } => json!({
                "job_id": id,
                "state": "running",
                "elapsed_ms": started.elapsed().as_millis() as u64,
            }),
            JobState::Finished { status, body } => json!({
                "job_id": id,
                "state": if *status < 400 { "succeeded" } else { "failed" },
                "http_status": status,
                "result": body,
            }),
        })
    }
}

// ── Routing ──────────────────────────────────────────────────────────────────

fn route(state: &Arc<ServerState>, request: &Request) -> Response {
    let host = match state.policy.check_host(request) {
        Ok(host) => host,
        Err(denial) => return denial.response(),
    };
    let caller = state.policy.classify(request, host);
    let response = if request.method == "OPTIONS" {
        preflight(&caller)
    } else if caller == Caller::CrossSite && !is_top_level_navigation(request) {
        Denial::CrossSite.response()
    } else if let Some(token) = login_token(request) {
        login(state, request, token)
    } else if request.path != "/api/health" && !state.policy.is_authenticated(request) {
        Denial::Unauthenticated.response()
    } else {
        dispatch_route(state, request)
    };
    response.headers(AccessPolicy::cors_headers(&caller))
}

/// A user-initiated top-level navigation (link, bookmark, typed URL). Its response is
/// shown to the user, never handed to the referring site, so it is safe to serve.
fn is_top_level_navigation(request: &Request) -> bool {
    (request.method == "GET" || request.method == "HEAD")
        && request.header("origin").is_none()
        && request
            .header("sec-fetch-mode")
            .is_some_and(|mode| mode.eq_ignore_ascii_case("navigate"))
}

fn preflight(caller: &Caller) -> Response {
    match caller {
        Caller::AllowedCrossOrigin(_) => Response::empty(204)
            .header("Access-Control-Allow-Methods", "GET, HEAD, POST, OPTIONS")
            .header(
                "Access-Control-Allow-Headers",
                "Content-Type, Authorization, X-OpenHeart-Token",
            )
            .header("Access-Control-Max-Age", "600"),
        Caller::CrossSite => Denial::CrossSite.response(),
        Caller::SameOrigin | Caller::NonBrowser => {
            Response::empty(204).header("Allow", "GET, HEAD, POST, OPTIONS")
        }
    }
}

fn login_token(request: &Request) -> Option<&str> {
    if request.method == "GET" || request.method == "HEAD" {
        request.query_param("token")
    } else {
        None
    }
}

fn login(state: &Arc<ServerState>, request: &Request, token: &str) -> Response {
    if !state.policy.token_required() {
        return dispatch_route(state, request);
    }
    if !state.policy.token_matches(token) {
        return Denial::Unauthenticated.response();
    }
    Response::empty(303)
        .header("Location", &percent_encode_path(&request.path))
        .header("Set-Cookie", &state.policy.token_cookie(token))
}

fn dispatch_route(state: &Arc<ServerState>, request: &Request) -> Response {
    let path = request.path.as_str();
    let method = request.method.as_str();
    let read = method == "GET" || method == "HEAD";
    match path {
        "/api/health" if read => health(state),
        "/api/version" if read => version(),
        "/api/supported-languages" if read => supported_languages(),
        "/api/supported-diagrams" if read => supported_diagrams(),
        "/api/patterns" if read => Response::json(200, &json!({ "patterns": DESIGN_PATTERNS })),
        "/api/source" if read => source(state, request),
        "/api/analyze" if method == "POST" => analyze(state, request),
        _ if read && path.starts_with("/api/jobs/") => {
            job_status(state, &path["/api/jobs/".len()..])
        }
        _ if read && path.starts_with("/api/sessions/") => {
            session_route(state, &path["/api/sessions/".len()..])
        }
        _ if path.starts_with("/api/") => match allowed_methods(path) {
            Some(allow) => Response::error(405, "method not allowed").header("Allow", allow),
            None => Response::error(404, "unknown API endpoint"),
        },
        _ if read => web_asset(state, request),
        _ => Response::error(405, "method not allowed").header("Allow", "GET, HEAD"),
    }
}

fn allowed_methods(path: &str) -> Option<&'static str> {
    match path {
        "/api/health"
        | "/api/version"
        | "/api/supported-languages"
        | "/api/supported-diagrams"
        | "/api/patterns"
        | "/api/source" => Some("GET, HEAD"),
        "/api/analyze" => Some("POST"),
        _ if path.starts_with("/api/jobs/") || path.starts_with("/api/sessions/") => {
            Some("GET, HEAD")
        }
        _ => None,
    }
}

// ── Informational endpoints ──────────────────────────────────────────────────

fn health(state: &ServerState) -> Response {
    Response::json(
        200,
        &json!({
            "status": "online",
            "engine": format!("OpenHeart SCPG v{}", VERSION),
            "version": VERSION,
            "plantuml": true,
            "auth_required": state.policy.token_required(),
            "clone_enabled": state.fetch.allow_clone,
            "analyses_in_progress": state.analyses.in_use(),
        }),
    )
}

fn version() -> Response {
    Response::json(
        200,
        &json!({
            "version": VERSION,
            "build": if cfg!(debug_assertions) { "debug" } else { "release" },
        }),
    )
}

fn supported_languages() -> Response {
    let ids: Vec<&str> = SOURCE_LANGUAGES.iter().map(|language| language.id).collect();
    Response::json(
        200,
        &json!({
            "languages": ids,
            "details": &SOURCE_LANGUAGES[..],
            "unsupported_files": "skipped and counted in each analysis' 'skipped' report",
        }),
    )
}

fn supported_diagrams() -> Response {
    let formats: Vec<&str> = [
        DiagramFormat::PlantUML,
        DiagramFormat::Mermaid,
        DiagramFormat::JSON,
        DiagramFormat::XMI,
    ]
    .iter()
    .map(|format| format.as_str())
    .collect();
    Response::json(
        200,
        &json!({
            "diagrams": UML_DIAGRAM_TYPES,
            "analysis_graphs": ANALYSIS_GRAPH_TYPES,
            "formats": formats,
        }),
    )
}

// ── Source viewer ────────────────────────────────────────────────────────────

fn source(state: &ServerState, request: &Request) -> Response {
    let Some(requested) = request
        .query_param("file")
        .or_else(|| request.query_param("path"))
    else {
        return Response::error(400, "missing 'file' query parameter");
    };
    let lookup = match state.selected_session(request) {
        Some(session) => session.sources.lookup(requested),
        None => lookup_samples(&state.sample_sources, requested),
    };
    match lookup {
        SourceLookup::Found { relative, content } => Response::json(
            200,
            &json!({ "found": true, "file": relative, "content": content }),
        ),
        SourceLookup::NotFound => Response::json(
            404,
            &json!({ "found": false, "error": "no analyzed source file matches the request" }),
        ),
        SourceLookup::Ambiguous(candidates) => Response::json(
            409,
            &json!({
                "found": false,
                "error": "several analyzed files match; request one of the candidates by path",
                "candidates": candidates,
            }),
        ),
        SourceLookup::Invalid(reason) => {
            Response::json(400, &json!({ "found": false, "error": reason }))
        }
        SourceLookup::Unreadable => Response::json(
            404,
            &json!({ "found": false, "error": "the matching file can no longer be read" }),
        ),
    }
}

fn lookup_samples(indexes: &[SourceIndex], requested: &str) -> SourceLookup {
    let mut fallback = SourceLookup::NotFound;
    for index in indexes {
        match index.lookup(requested) {
            found @ SourceLookup::Found { .. } => return found,
            SourceLookup::NotFound => {}
            other => {
                if matches!(fallback, SourceLookup::NotFound) {
                    fallback = other;
                }
            }
        }
    }
    fallback
}

// ── Analysis ─────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct AnalyzeRequest {
    #[serde(default)]
    repo_url: Option<String>,
    #[serde(default)]
    local_repo: Option<String>,
    #[serde(default)]
    diagram_types: Option<Vec<String>>,
    #[serde(default)]
    include_tests: bool,
    #[serde(default)]
    refresh: bool,
    #[serde(default, rename = "async")]
    run_async: bool,
}

/// What an analysis reads: a GitHub repository, or a directory under the configured
/// local repository root.
enum AnalysisSource {
    GitHub(GitHubRepoRef),
    Local(String),
}

impl AnalysisSource {
    fn label(&self) -> String {
        match self {
            Self::GitHub(repo) => repo.canonical_url(),
            Self::Local(name) => format!("local repository '{}'", name),
        }
    }
}

struct AnalysisPlan {
    session_id: String,
    source: AnalysisSource,
    inline_diagrams: Vec<&'static str>,
    include_tests: bool,
    refresh: bool,
}

/// The analysis root of a resolved source and its client-facing description.
struct ResolvedSource {
    root: PathBuf,
    repository: Value,
    display_root: String,
}

struct AnalysisOutcome {
    status: u16,
    body: Value,
    session: Option<AnalysisSession>,
}

impl AnalysisOutcome {
    fn failure(status: u16, session_id: &str, logs: Vec<String>, message: String) -> Self {
        Self {
            status,
            body: failure_body(Some(session_id), logs, &message),
            session: None,
        }
    }
}

fn failure_body(session_id: Option<&str>, logs: Vec<String>, message: &str) -> Value {
    json!({
        "status": "error",
        "session_id": session_id,
        "logs": logs,
        "errors": [message],
    })
}

fn failure(status: u16, message: &str) -> Response {
    Response::json(status, &failure_body(None, Vec::new(), message))
}

fn analyze(state: &Arc<ServerState>, request: &Request) -> Response {
    if !request.has_json_body() {
        return failure(
            415,
            "POST /api/analyze expects a JSON body with Content-Type: application/json",
        );
    }
    let payload: AnalyzeRequest = match serde_json::from_slice(&request.body) {
        Ok(payload) => payload,
        Err(err) => return failure(400, &format!("invalid request body: {}", err)),
    };
    let source = match (payload.repo_url.as_deref(), payload.local_repo.as_deref()) {
        (Some(url), None) => match WebRepoFetcher::parse_repo_url(url) {
            Ok(repo) => AnalysisSource::GitHub(repo),
            Err(reason) => return failure(422, &reason),
        },
        (None, Some(name)) if state.local_repos.is_some() => {
            AnalysisSource::Local(name.trim().to_string())
        }
        (None, Some(_)) => {
            return failure(
                403,
                "local repositories are not enabled on this server (set OPENHEART_LOCAL_REPOS)",
            )
        }
        (Some(_), Some(_)) => return failure(400, "send either 'repo_url' or 'local_repo'"),
        (None, None) => {
            return failure(
                400,
                "missing 'repo_url' (https://github.com/<owner>/<repo>) or 'local_repo'",
            )
        }
    };
    let inline_diagrams = match resolve_diagram_selection(payload.diagram_types.as_deref()) {
        Ok(selection) => selection,
        Err(reason) => return failure(400, &reason),
    };
    let Some(permit) = state.analyses.try_acquire() else {
        return failure(503, "all analysis slots are busy; retry shortly")
            .header("Retry-After", "30");
    };
    let plan = AnalysisPlan {
        session_id: format!("{}{}", session::SESSION_PREFIX, security::random_hex(12)),
        source,
        inline_diagrams,
        include_tests: payload.include_tests,
        refresh: payload.refresh,
    };
    log_info(&format!(
        "[SERVER] Analysis {} requested for {}",
        plan.session_id,
        plan.source.label()
    ));
    if payload.run_async {
        start_job(state, plan, permit)
    } else {
        run_and_wait(state, plan, permit)
    }
}

fn resolve_diagram_selection(requested: Option<&[String]>) -> Result<Vec<&'static str>, String> {
    let Some(names) = requested else {
        return Ok(UML_DIAGRAM_TYPES.to_vec());
    };
    let mut selection = Vec::new();
    let mut unknown = Vec::new();
    for name in names {
        match canonical_diagram_type(name) {
            Some(canonical) if !selection.contains(&canonical) => selection.push(canonical),
            Some(_) => {}
            None => unknown.push(name.as_str()),
        }
    }
    if unknown.is_empty() {
        Ok(selection)
    } else {
        Err(format!("unknown diagram types: {}", unknown.join(", ")))
    }
}

fn spawn_analysis<F>(state: &ServerState, session_id: &str, job: F) -> Result<(), String>
where
    F: FnOnce() + Send + 'static,
{
    thread::Builder::new()
        .name(format!("openheart-analysis-{}", session_id))
        .stack_size(state.config.pipeline_stack_bytes)
        .spawn(job)
        .map(|_| ())
        .map_err(|err| format!("cannot start the analysis thread: {}", err))
}

fn run_and_wait(state: &Arc<ServerState>, plan: AnalysisPlan, permit: Permit) -> Response {
    let session_id = plan.session_id.clone();
    let (sender, receiver) = mpsc::channel::<AnalysisOutcome>();
    let worker_state = Arc::clone(state);
    let spawned = spawn_analysis(state, &session_id, move || {
        let _permit = permit;
        let outcome = execute_guarded(&worker_state, &plan);
        if let Err(mpsc::SendError(unsent)) = sender.send(outcome) {
            if let Some(session) = unsent.session {
                worker_state.sessions.discard(&session.dir);
            }
            log_warn(&format!(
                "[SERVER] Analysis {} finished after its client stopped waiting; results discarded",
                plan.session_id
            ));
        }
    });
    if let Err(message) = spawned {
        log_error(&format!("[SERVER] {}", message));
        return failure(500, "the analysis could not be started");
    }

    match receiver.recv_timeout(state.config.analysis_timeout) {
        Ok(outcome) => complete(state, outcome),
        Err(RecvTimeoutError::Timeout) => Response::json(
            504,
            &failure_body(
                Some(session_id.as_str()),
                Vec::new(),
                &format!(
                    "the analysis exceeded its time budget of {} s; it keeps its slot until \
                     it stops, and its results will be discarded",
                    state.config.analysis_timeout.as_secs()
                ),
            ),
        ),
        Err(RecvTimeoutError::Disconnected) => Response::json(
            500,
            &failure_body(
                Some(session_id.as_str()),
                Vec::new(),
                "the analysis worker terminated unexpectedly",
            ),
        ),
    }
}

fn complete(state: &ServerState, outcome: AnalysisOutcome) -> Response {
    let response = Response::json(outcome.status, &outcome.body);
    match outcome.session {
        Some(session) => {
            let session = state.sessions.insert(session);
            response.header("Set-Cookie", &state.session_cookie(&session.id))
        }
        None => response,
    }
}

fn start_job(state: &Arc<ServerState>, plan: AnalysisPlan, permit: Permit) -> Response {
    let session_id = plan.session_id.clone();
    state.jobs.start(&session_id);
    let worker_state = Arc::clone(state);
    let spawned = spawn_analysis(state, &session_id, move || {
        let _permit = permit;
        let outcome = execute_guarded(&worker_state, &plan);
        if let Some(session) = outcome.session {
            worker_state.sessions.insert(session);
        }
        worker_state
            .jobs
            .finish(&plan.session_id, outcome.status, outcome.body);
    });
    if let Err(message) = spawned {
        log_error(&format!("[SERVER] {}", message));
        let body = failure_body(
            Some(session_id.as_str()),
            Vec::new(),
            "the analysis could not be started",
        );
        state.jobs.finish(&session_id, 500, body);
        return failure(500, "the analysis could not be started");
    }
    let poll = format!("/api/jobs/{}", session_id);
    Response::json(
        202,
        &json!({
            "status": "accepted",
            "job_id": session_id,
            "session_id": session_id,
            "poll": poll,
        }),
    )
    .header("Location", &poll)
    .header("Set-Cookie", &state.session_cookie(&session_id))
}

fn job_status(state: &ServerState, id: &str) -> Response {
    if !session::is_session_id(id) {
        return Response::error(404, "unknown job");
    }
    match state.jobs.describe(id) {
        Some(description) => Response::json(200, &description),
        None => Response::error(404, "unknown or expired job"),
    }
}

fn execute_guarded(state: &ServerState, plan: &AnalysisPlan) -> AnalysisOutcome {
    match panic::catch_unwind(AssertUnwindSafe(|| execute_analysis(state, plan))) {
        Ok(outcome) => outcome,
        Err(payload) => {
            log_error(&format!(
                "[SERVER] Analysis {} panicked: {}",
                plan.session_id,
                panic_message(payload.as_ref())
            ));
            AnalysisOutcome::failure(
                500,
                &plan.session_id,
                Vec::new(),
                "the analysis aborted with an internal error".to_string(),
            )
        }
    }
}

fn execute_analysis(state: &ServerState, plan: &AnalysisPlan) -> AnalysisOutcome {
    let id = plan.session_id.as_str();
    let mut logs = Vec::new();

    let resolved = match resolve_source(state, plan, &mut logs) {
        Ok(resolved) => resolved,
        Err((status, message)) => return AnalysisOutcome::failure(status, id, logs, message),
    };
    let options = DiscoveryOptions {
        limits: state.config.source_limits,
        include_tests: plan.include_tests,
        respect_ignore_files: true,
    };
    let inventory = match discover_sources(&resolved.root, &options) {
        Ok(inventory) => inventory,
        Err(err) => {
            log_warn(&format!("[SERVER] Analysis {}: {}", id, err));
            let status = if err.is_input_error() { 422 } else { 500 };
            return AnalysisOutcome::failure(status, id, logs, err.client_message());
        }
    };
    logs.push(describe_inventory(&resolved.display_root, &inventory));

    let dir = match state.sessions.allocate(id) {
        Ok(dir) => dir,
        Err(err) => {
            log_error(&format!(
                "[SERVER] Analysis {}: cannot create session directory: {}",
                id, err
            ));
            let message = "cannot create the session directory".to_string();
            return AnalysisOutcome::failure(500, id, logs, message);
        }
    };
    let run = match run_pipeline(&inventory, &dir, &PipelineOptions::default()) {
        Ok(run) => run,
        Err(err) => {
            log_warn(&format!("[SERVER] Analysis {} failed: {}", id, err));
            state.sessions.discard(&dir);
            let status = if err.is_input_error() { 422 } else { 500 };
            return AnalysisOutcome::failure(status, id, logs, err.client_message());
        }
    };
    if let Err(err) = run.write_report(&inventory, &dir) {
        log_warn(&format!("[SERVER] Analysis {}: {}", id, err));
        logs.push("> The machine-readable run report could not be written.".to_string());
    }
    logs.extend(describe_run(&run));

    let summary = session_summary(plan, &resolved.repository, &inventory, &run);
    let body = success_body(plan, &resolved.repository, &inventory, &run, logs);
    let session = AnalysisSession {
        id: id.to_string(),
        dir,
        sources: SourceIndex::from_inventory(&inventory),
        summary,
        created: SystemTime::now(),
    };
    AnalysisOutcome {
        status: 200,
        body,
        session: Some(session),
    }
}

/// Fetches or locates the analysis input, returning `(status, client message)` on failure.
fn resolve_source(
    state: &ServerState,
    plan: &AnalysisPlan,
    logs: &mut Vec<String>,
) -> Result<ResolvedSource, (u16, String)> {
    match &plan.source {
        AnalysisSource::GitHub(repo) => {
            let checkout = WebRepoFetcher::checkout(repo, &state.fetch, plan.refresh)
                .map_err(|err| {
                    log_warn(&format!(
                        "[SERVER] Analysis {}: fetching {} failed: {}",
                        plan.session_id,
                        repo.canonical_url(),
                        err.detail()
                    ));
                    (err.http_status(), err.to_string())
                })?;
            logs.push(describe_checkout(repo, &checkout));
            let root = WebRepoFetcher::resolve_subpath(&checkout.dir, repo.subpath.as_deref())
                .map_err(|err| (err.http_status(), err.to_string()))?;
            Ok(ResolvedSource {
                root,
                repository: repository_json(repo, &checkout),
                display_root: repo.subpath.clone().unwrap_or_else(|| "/".to_string()),
            })
        }
        AnalysisSource::Local(name) => {
            let base = state.local_repos.as_deref().ok_or_else(|| {
                (
                    403u16,
                    "local repositories are not enabled on this server".to_string(),
                )
            })?;
            let root = resolve_local_repo(base, name).map_err(|reason| (404u16, reason))?;
            logs.push(format!("> Local repository '{}' located.", name));
            Ok(ResolvedSource {
                root,
                repository: json!({ "local_repo": name, "commit": Value::Null }),
                display_root: "/".to_string(),
            })
        }
    }
}

/// Resolves `name` (one or more plain path segments) to a directory inside `base`.
fn resolve_local_repo(base: &Path, name: &str) -> Result<PathBuf, String> {
    let segments: Vec<&str> = name
        .split(['/', '\\'])
        .filter(|segment| !segment.is_empty())
        .collect();
    let acceptable = !segments.is_empty()
        && segments.iter().all(|segment| {
            !segment.starts_with('.')
                && !segment.contains(':')
                && !segment.chars().any(char::is_control)
        });
    if !acceptable {
        return Err(format!("'{}' is not an accepted local repository name", name));
    }
    let candidate = segments
        .iter()
        .fold(base.to_path_buf(), |path, segment| path.join(segment));
    match contained(base, &candidate) {
        Some(root) if root.is_dir() => Ok(root),
        _ => Err(format!("local repository '{}' does not exist", name)),
    }
}

fn describe_checkout(repo: &GitHubRepoRef, checkout: &CheckedOutRepo) -> String {
    format!(
        "> Repository {} ({}): {} at commit {}.",
        repo.canonical_url(),
        repo.git_ref.as_deref().unwrap_or("default branch"),
        if checkout.reused_cache {
            "reused the cached clone"
        } else {
            "cloned"
        },
        checkout.commit.as_deref().unwrap_or("unknown")
    )
}

fn describe_inventory(display_root: &str, inventory: &SourceInventory) -> String {
    let skipped = &inventory.skipped;
    format!(
        "> Selected {} source files ({} bytes) under '{}'; skipped {} unsupported, {} test, \
         {} oversized and {} unreadable files and {} excluded directories.",
        inventory.files.len(),
        inventory.total_bytes,
        display_root,
        skipped.unsupported_total(),
        skipped.test_files,
        skipped.oversized_files,
        skipped.unreadable_entries,
        skipped.excluded_directories
    )
}

fn describe_run(run: &PipelineRun) -> Vec<String> {
    let mut lines: Vec<String> = run
        .phases
        .iter()
        .map(|phase| {
            format!(
                "> Phase {}: {} completed in {:.1} ms.",
                phase.phase, phase.name, phase.duration_ms
            )
        })
        .collect();
    let ingestion = &run.ingestion;
    lines.push(format!(
        "> Phase 1 ingested {} files and skipped {}; {} files contain syntax errors \
         ({} ERROR and {} MISSING nodes).",
        ingestion.files_ingested,
        ingestion.skipped_total(),
        ingestion.files_with_syntax_errors,
        ingestion.syntax_error_nodes,
        ingestion.missing_nodes
    ));
    lines.push(format!(
        "> Exported {} diagram files; {} requested diagram/format pairs unavailable.",
        run.diagrams.len(),
        run.diagram_gaps.len()
    ));
    lines.push(format!(
        "> Pipeline complete in {} ms: {} of 10 phases executed.",
        run.elapsed.as_millis(),
        run.phases.len()
    ));
    lines
}

fn repository_json(repo: &GitHubRepoRef, checkout: &CheckedOutRepo) -> Value {
    json!({
        "url": repo.canonical_url(),
        "owner": repo.owner,
        "name": repo.repo,
        "ref": repo.git_ref,
        "subpath": repo.subpath,
        "commit": checkout.commit,
        "cached_checkout": checkout.reused_cache,
    })
}

fn diagram_urls(session_id: &str, run: &PipelineRun) -> Vec<String> {
    run.diagrams
        .iter()
        .map(|output| format!("/api/sessions/{}/diagrams/{}", session_id, output.file_name))
        .collect()
}

fn session_summary(
    plan: &AnalysisPlan,
    repository: &Value,
    inventory: &SourceInventory,
    run: &PipelineRun,
) -> Value {
    json!({
        "session_id": plan.session_id,
        "repository": repository,
        "files_processed": inventory.files.len(),
        "counts": run.counts(),
        "scpg_hash": format!("0x{:08X}", run.scpg_hash()),
        "phases_executed": run.phases.len(),
        "diagram_files": diagram_urls(&plan.session_id, run),
        "unavailable_diagrams": run.diagram_gaps,
        "report": format!("/api/sessions/{}/report", plan.session_id),
    })
}

fn success_body(
    plan: &AnalysisPlan,
    repository: &Value,
    inventory: &SourceInventory,
    run: &PipelineRun,
    logs: Vec<String>,
) -> Value {
    let mut diagrams = serde_json::Map::new();
    for diagram_type in &plan.inline_diagrams {
        if let Some(output) = run.diagram(diagram_type, DiagramFormat::PlantUML) {
            diagrams.insert(
                diagram_type.to_string(),
                Value::String(output.content.clone()),
            );
        }
    }
    let counts = run.counts();
    json!({
        "status": "success",
        "session_id": plan.session_id,
        "repository": repository,
        "stats": {
            "files_processed": inventory.files.len(),
            "total_tokens": counts.tokens,
            "total_classes": counts.uml_classes,
            "execution_time_ms": run.elapsed.as_millis() as u64,
        },
        "counts": counts,
        "scpg_hash": format!("0x{:08X}", run.scpg_hash()),
        "phases": run.phases,
        "phases_executed": run.phases.len(),
        "ingestion": run.ingestion,
        "input_selection": inventory.selection(),
        "skipped": inventory.skipped,
        "diagrams": diagrams,
        "diagram_files": diagram_urls(&plan.session_id, run),
        "unavailable_diagrams": run.diagram_gaps,
        "traceability": traceability_preview(inventory, run),
        "traceability_total": run.artifacts.tra.uml_links.len(),
        "logs": logs,
        "errors": [],
    })
}

/// The first traceability links, with files resolved through the corpus file id and
/// reported relative to the analysis root (`null` when the id has no file record).
fn traceability_preview(inventory: &SourceInventory, run: &PipelineRun) -> Vec<Value> {
    let tca = &run.artifacts.tca;
    run.artifacts
        .tra
        .uml_links
        .iter()
        .take(TRACE_PREVIEW_LIMIT)
        .map(|link| {
            let file = tca
                .file_path(link.file_id)
                .and_then(|path| inventory.relative_for_corpus_path(path))
                .map(str::to_string);
            json!({
                "tid": link.sym_id,
                "file": file,
                "span": format!(
                    "L{}:C{} - L{}:C{}",
                    link.line_start, link.col_start, link.line_end, link.col_end
                ),
                "hash": format!("0x{:08X}", link.scpg_hash),
            })
        })
        .collect()
}

// ── Sessions, diagrams and static files ──────────────────────────────────────

fn session_route(state: &ServerState, rest: &str) -> Response {
    let (id, tail) = rest.split_once('/').unwrap_or((rest, ""));
    let Some(session) = state.sessions.get(id) else {
        return Response::error(404, "unknown or expired analysis session");
    };
    match tail {
        "" => {
            let mut summary = session.summary.clone();
            if let Value::Object(fields) = &mut summary {
                let created = session
                    .created
                    .duration_since(UNIX_EPOCH)
                    .map(|elapsed| elapsed.as_secs())
                    .unwrap_or(0);
                fields.insert("created_unix_secs".to_string(), json!(created));
            }
            Response::json(200, &summary)
        }
        "report" => read_file_response(
            &session.dir.join(RUN_REPORT_FILE),
            "application/json; charset=utf-8",
        ),
        _ => match tail.strip_prefix("diagrams/") {
            Some(file) => session_diagram(&session, file)
                .unwrap_or_else(|| Response::error(404, "unknown diagram file name")),
            None => Response::error(404, "not found"),
        },
    }
}

/// Serves `<type>.<ext>` from a session, mapping aliases such as `use_case` or `state` to
/// the canonical file. `None` when `file` is not a diagram file name.
fn session_diagram(session: &AnalysisSession, file: &str) -> Option<Response> {
    let (name, extension) = file.rsplit_once('.')?;
    let diagram_type = canonical_diagram_type(name)?;
    let format = parse_diagram_format(extension)?;
    let content_type = match format {
        DiagramFormat::JSON => "application/json; charset=utf-8",
        DiagramFormat::XMI => "application/xml; charset=utf-8",
        DiagramFormat::PlantUML | DiagramFormat::Mermaid => "text/plain; charset=utf-8",
    };
    let path = session
        .dir
        .join("diagrams")
        .join(format!("{}.{}", diagram_type, format.file_extension()));
    if !path.is_file() {
        return Some(Response::error(
            404,
            "this diagram was not produced by the selected analysis",
        ));
    }
    Some(read_file_response(&path, content_type))
}

fn read_file_response(path: &Path, content_type: &str) -> Response {
    match fs::read(path) {
        Ok(bytes) => Response::new(200, content_type, bytes).header("Cache-Control", "no-store"),
        Err(_) => Response::error(404, "not found"),
    }
}

fn web_asset(state: &ServerState, request: &Request) -> Response {
    if let Some(file) = request.path.strip_prefix("/diagrams/") {
        if let Some(session) = state.selected_session(request) {
            if let Some(response) = session_diagram(&session, file) {
                return response;
            }
        }
    }
    static_file(state, &request.path)
}

fn static_file(state: &ServerState, path: &str) -> Response {
    let not_found = || Response::error(404, "not found");
    let Some(web_root) = &state.web_root else {
        return Response::error(404, "the web studio is not installed on this server");
    };
    let relative = path.trim_start_matches('/');
    let suspicious = relative.contains('\\')
        || relative
            .split('/')
            .any(|segment| segment.starts_with('.') || segment.contains(':'));
    if suspicious {
        return not_found();
    }
    let Some(mut file) = contained(web_root, &web_root.join(relative)) else {
        return not_found();
    };
    if file.is_dir() {
        match contained(web_root, &file.join("index.html")) {
            Some(index) => file = index,
            None => return not_found(),
        }
    }
    let Ok(bytes) = fs::read(&file) else {
        return not_found();
    };
    let content_type = content_type_for(&file);
    let response = Response::new(200, content_type, bytes).header("Cache-Control", "no-cache");
    if content_type.starts_with("text/html") {
        response.studio_document()
    } else {
        response
    }
}

/// Canonicalizes `candidate` and returns it only if it lies inside `root`.
fn contained(root: &Path, candidate: &Path) -> Option<PathBuf> {
    canonicalize_clean(candidate)
        .ok()
        .filter(|resolved| resolved.starts_with(root))
}

fn content_type_for(path: &Path) -> &'static str {
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .unwrap_or_default();
    match extension.as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        "txt" | "md" | "puml" | "mmd" => "text/plain; charset=utf-8",
        "xml" | "xmi" => "application/xml; charset=utf-8",
        "wasm" => "application/wasm",
        _ => "application/octet-stream",
    }
}
