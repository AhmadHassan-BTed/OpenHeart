# Engine query protocol, WebAssembly interface and diagram provenance

Status (10 November 2026):

- **Built and verified:** the CLI and server transports; `functions`, `callers`, `callees`, `dependents` and `view` (focus one level deep). `dependencies` returns real calls, while field reads and writes and type uses are returned empty with a note.
- **Not built yet:** `symbol`/`function` ids on every Graph IR node, and the WebAssembly module (section 4).

See [`../CHECKPOINT.md`](../CHECKPOINT.md), section 0.

This page is the contract shared by the Rust engine (`src/`), the server (`openheart server`), the CLI (`openheart query`), the WebAssembly build (`web/engine/openheart.wasm`) and the web studio (`web/js/engine/`). One JSON protocol answers every interactive question the studio asks when you click an element: who calls this function, what it calls, what it reads, writes and depends on, and which focused diagram to show for it.

## 1. Symbols

Every program element the engine knows has a **symbol spec**: a string accepted wherever a request takes `symbol`. The engine resolves it in this order: numeric symbol id, stable id, fully qualified name, simple name (`QueryRequest::Resolve`). A simple name that matches several symbols is an error that lists the candidates.

Every Graph IR node that represents a program element carries:

| Field | Meaning |
|---|---|
| `symbol` | The element's stable id (a symbol spec). Absent for nodes that are not program elements (packages without a declaration, BDD terminals, CFG blocks use `function` below). |
| `qualified_name` | Fully qualified name, for display. |
| `function` | For nodes inside one function's view (CFG blocks, DFG values, CDG nodes, BDD gates): the stable id of that function. |
| `file`, `lines`, `anchor` | Source provenance, as today. |

The studio copies `symbol` and `function` into Cytoscape node data (`data.symbol`, `data.function`) and uses them for every query below.

## 2. Requests and responses

A request is a JSON object whose `query` field names the operation (snake case). A response is the JSON value described for that operation. A failure is `{ "error": { "status": <http-like status>, "message": "<reason>" } }`; `status` 404 means "no such symbol", 400 a malformed request, 422 "not derivable for this element", with the reason.

Facts that the analysis could not establish are never invented. Lists carry only what was derived; a `notes` array explains what is missing and why (for example "3 call sites have an unresolved receiver type").

### 2.1 Existing operations (`src/adapters/query.rs`)

`stats`, `verify`, `catalog`, `resolve`, `reachable`, `call_path`, `slice`, `impact`, `source`, `at`, `path_summary`, `datalog`. Shapes as implemented.

### 2.2 New operations

**`functions`**: list or search functions and methods.
Request `{ "query": "functions", "filter": "<case-insensitive substring>"?, "limit": 200? }`.
Response `{ "functions": [FunctionRef], "truncated": bool }`.

`FunctionRef = { "symbol", "name", "qualified_name", "kind": "method"|"constructor"|"function"|"lambda", "owner": "<owning type qualified name>"|null, "file", "line", "callers": n, "callees": n }`.

**`callers`** / **`callees`**: the call graph around a function.
Request `{ "query": "callers"|"callees", "symbol": "<spec>", "depth": 1? }` (depth 1 to 6).
Response:
```json
{
  "root": "<symbol>",
  "functions": { "<symbol>": FunctionRef },
  "edges": [ { "caller": "<symbol>", "callee": "<symbol>", "sites": [ { "file": "...", "line": 12, "column": 8 } ],
               "dispatch": "static"|"virtual"|"interface"|"lambda"|"constructor",
               "resolution": "exact"|"cha"|"points_to"|"symbol_table" } ],
  "unresolved_sites": [ { "file": "...", "line": 40, "reason": "receiver type unknown" } ],
  "notes": [ "..." ]
}
```

**`dependencies`**: everything a function (or type) depends on.
Request `{ "query": "dependencies", "symbol": "<spec>" }`.
Response:
```json
{
  "root": "<symbol>",
  "calls":  [ { "symbol": "...", "qualified_name": "...", "count": 2 } ],
  "reads":  [ { "symbol": "<field>", "qualified_name": "...", "count": 3 } ],
  "writes": [ { "symbol": "<field>", "qualified_name": "...", "count": 1 } ],
  "types":  [ { "symbol": "<type>", "qualified_name": "...", "relations": ["parameter","return","local","new","cast","field_type","extends","implements","annotation"] } ],
  "packages": [ "<package>" ],
  "notes": [ "..." ]
}
```

**`dependents`**: the reverse: who depends on a function, field or type.
Request `{ "query": "dependents", "symbol": "<spec>" }`.
Response `{ "root", "callers": [...], "readers": [...], "writers": [...], "type_users": [...], "subtypes": [...], "overrides": [...], "notes": [...] }`, entries shaped like the lists above.

**`view`**: a diagram as Graph IR (the studio's schema, `web/js/graph-ir-validate.js`), for the whole program or focused on one element.
Request `{ "query": "view", "view": "<diagram id>", "symbol": "<spec>"?, "depth": 2? }`.

| `view` | Without `symbol` | With `symbol` |
|---|---|---|
| any of the 19 ids | the whole-program diagram (same as the exported `<id>.json`) | see below |
| `callgraph` | | callers and callees of the function up to `depth` |
| `cfg`, `dfg`, `cdg`, `robdd`, `activity` | | that one function |
| `sequence`, `communication`, `timing`, `interaction` | | the scenario starting at that function |
| `class`, `object`, `composite` | | the type and its related types up to `depth` |
| `package`, `component` | | the element and its direct dependencies |
| `statemachine` | | the state machine of that type |

Response: a Graph IR object (`schema`, `diagram_type`, `nodes`, `edges`, `metadata`). A view that cannot be derived for the element is a 422 error with the reason, never an empty or invented diagram.

## 3. Transports

| Transport | How |
|---|---|
| CLI | `openheart query <analysis-dir> '<request json>'` prints the response. |
| Server | `POST /api/sessions/<id>/query` with the request as the body. The server keeps each session's engine in memory (reopened from `unified.scpg` after a restart). |
| WebAssembly | `oh_query` (section 4). |

## 4. WebAssembly interface

Target `wasm32-wasip1`, crate type `cdylib` (a WASI reactor module), feature `wasm`, built without the `server` feature. No `wasm-bindgen`; the interface is plain exported functions over linear memory, so the studio needs only a small WASI shim.

| Export | Signature | Meaning |
|---|---|---|
| `oh_alloc` | `(len: u32) -> u32` | Allocates `len` bytes; returns a pointer. |
| `oh_free` | `(ptr: u32, len: u32)` | Frees a buffer from `oh_alloc`. |
| `oh_analyze` | `(ptr: u32, len: u32) -> u32` | Runs the whole pipeline in memory on an analyze request (below). Returns a session handle (> 0), or 0 on failure. |
| `oh_query` | `(session: u32, ptr: u32, len: u32) -> u32` | Answers a section 2 request. Returns 0 on success, otherwise the error status. |
| `oh_result_ptr`, `oh_result_len` | `() -> u32` | The JSON result of the last `oh_analyze` or `oh_query` call (also the error object on failure). Valid until the next call. |
| `oh_release` | `(session: u32)` | Drops a session. |
| `oh_version` | `() -> u32` | Protocol version (1). |

Analyze request: `{ "files": [ { "path": "src/main/java/a/B.java", "text": "..." } ], "project_files": [ { "path": "Dockerfile", "text": "..." } ], "options": { "diagrams": ["class", ...]? } }`. `project_files` carries build and deployment files (Dockerfile, compose files, Kubernetes manifests, `pom.xml`, `build.gradle(.kts)`, `package.json`, `*.csproj`, `*.sln`, `CMakeLists.txt`) for the deployment and component views.

Analyze result: `{ "session": n, "summary": { "files": n, "functions": n, "call_edges": n, "phases": [...] }, "diagrams": { "<id>": GraphIR | { "unavailable": "<reason>" } }, "warnings": [...] }`.

WASI imports the module may use (the studio shim implements these, entirely in memory): `fd_write` (stdout and stderr only, to the console), `environ_get`, `environ_sizes_get` (empty), `args_get`, `args_sizes_get` (empty), `clock_time_get`, `random_get`, `proc_exit`, `fd_close`, `fd_seek`, `fd_fdstat_get`, `fd_prestat_get` (no preopened directories). The engine must not depend on any other WASI call in the `wasm` build.

Build (not run yet): `scripts/build-wasm.sh`, which sets `CC_wasm32_wasip1`, `AR_wasm32_wasip1` and `CFLAGS_wasm32_wasip1` to wasi-sdk's `clang`, `llvm-ar` and `--sysroot`, runs `cargo build --release --lib --target wasm32-wasip1 --no-default-features --features wasm`, and copies the module to `web/engine/openheart.wasm`. The studio's Content-Security-Policy allows WebAssembly compilation with `'wasm-unsafe-eval'` in `script-src`.

## 5. Rust module layout

| Module | Owns | Public API used by the others |
|---|---|---|
| `src/session.rs` (`crate::session`) | One finished analysis, kept for queries | `pub struct AnalysisSession { pub artifacts: crate::adapters::runner::AnalysisArtifacts, pub relative_paths: Vec<Option<String>>, pub project_files: Vec<ProjectFile>, pub sources: std::collections::BTreeMap<String, String> }`; `pub struct ProjectFile { pub path: String, pub text: String }`; `AnalysisSession::from_run(run: PipelineRun, project_files, sources) -> Self`; `AnalysisSession::open(dir: &Path) -> io::Result<Self>` (reloads every artifact a session directory holds); `fn diagram_context(&self) -> DiagramContext<'_>` (every `with_*` applied, project files attached). |
| `src/facts/` (`crate::facts`) | Per-function and per-symbol facts | `pub struct FactsError { pub status: u16, pub message: String }`; `functions(s: &AnalysisSession, filter: Option<&str>, limit: usize)`, `callers(s, symbol: &str, depth: u32)`, `callees(s, symbol, depth)`, `dependencies(s, symbol)`, `dependents(s, symbol)`, each `-> Result<serde_json::Value, FactsError>` in the section 2.2 shapes; `resolve_symbol(s, spec: &str) -> Result<u32, FactsError>`. |
| `src/scpg/diagram/views/focused.rs` | Focused diagrams | `pub fn view(s: &AnalysisSession, view: &str, symbol: Option<&str>, depth: u32) -> Result<serde_json::Value, crate::facts::FactsError>`: Graph IR per section 2.2 `view`. Every builder sets `symbol` / `function` on its nodes (section 1). |
| `src/adapters/query.rs` | The protocol | `QueryRequest` gains `Functions`, `Callers`, `Callees`, `Dependencies`, `Dependents`, `View`; `pub fn execute_session(s: &AnalysisSession, req: &QueryRequest) -> Result<Value, QueryFailure>` dispatches to `facts` and `focused::view`, and to the existing engine queries through `s.artifacts.engine`. |
| `src/adapters/runner.rs` | The pipeline | Phase 6 runs with the token corpus (`Phase6Stage::run_with_source(.., Some(&tca))`); `run_pipeline_in_memory(files: &[(String, String)], project_files: Vec<ProjectFile>, options) -> Result<PipelineRun, PipelineError>` runs every phase without touching the file system; project-file discovery for the CLI and server. |
| `src/wasm.rs` (`#[cfg(feature = "wasm")]`) | Section 4 exports | Sessions in a `Vec<Option<AnalysisSession>>`; JSON in and out with `serde_json`. |

Feature `server` gates `adapters::{server, http, security, web_repo}` and the `server` CLI command; feature `wasm` gates `src/wasm.rs`. Neither adds a dependency.
