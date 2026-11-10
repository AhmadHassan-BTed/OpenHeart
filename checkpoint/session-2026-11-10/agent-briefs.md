# Agent briefs, 10 November 2026 session

> **After the stop,** only what `main` needed to build and run was completed: `src/session.rs`, `src/facts/`, `views/focused.rs`, Phase 6 with the token corpus, and the per-edge kinds. `CHECKPOINT.md` section 0.3 lists, per package, what is done and what is still open. The briefs below apply to the "Still open" items. `main` now builds and its tests pass, so keep it that way: run `cargo check --all-targets`, `cargo fmt --all -- --check` and `cargo test --all-targets` after each change.

These are the exact work packages that were running when the owner stopped the session. To resume one, start a fresh agent with its brief below. Prepend this:

> "Resume an interrupted package. The files listed under *State at stop* already contain your earlier, unreviewed partial edits: read them first, keep what is right, finish the rest."

Shared contract for all Rust and studio packages: [`docs/engine-query-protocol.md`](../../docs/engine-query-protocol.md). Standing rules: `CHECKPOINT.md` section 1, plus the owner's 10 November decisions in `CHECKPOINT.md` section 0.

Every package works directly on `main` in the main checkout, with no worktree and no branch. Each owns a disjoint set of files. Nothing outside those files is edited: anything else goes in the report as a handoff (file plus exact change).

---

## A1: real call graph and per-function facts (Rust, coding only)

**Owns:**
- `src/cg/**`
- `src/facts/**` (new; A4 declares `pub mod facts;` in `src/lib.rs`)
- the single Phase 6 call in `src/core/pipeline.rs` (about line 440)
- additive accessors in `src/symbol/serializer.rs`
- additive fields in `src/core/types/cg.rs`, keeping the cg serializer consistent

**Survey facts:**
- **Why there are 0 call edges:**
  - The runner calls `Phase6Stage::run`, which forwards `tca = None` (`src/cg/mod.rs:82-92`).
  - Without token text, `shape_of` returns `SiteKind::Opaque` for every non-lambda site (`call_sites.rs:232-243`).
  - `resolve(Opaque)` returns nothing (`resolution.rs:435-443`).
  - `Phase6Stage::run_with_source` exists (`cg/mod.rs:96`). A4 switches `src/adapters/runner.rs:2253` to it.
- **Where calls are handled:**
  - Sites are collected by `extract_call_sites` (`cg/call_sites.rs:300`).
  - They are resolved by `DispatchResolver::resolve` (`resolution.rs:415`; compile-time selection plus CHA/RTA).
  - Andersen points-to refines them (`cg/mod.rs:178-220`); edges are assembled at `cg/mod.rs:222-265`.
- **Phase 3 already resolves calls** into `sta.occurrences` (`SymbolTableArtifact::call_target`, `symbol/serializer.rs:167`). `src/cg` never uses it.
- **Data available:**
  - Persisted `CallSite` (`core/types/cg.rs:27-37`) has no callee name, receiver or arguments.
  - Field reads and writes are `sta.occurrences` with Read/Write roles (`symbol/model.rs:174-196`), not grouped per function.
  - Type references come from `type_lowering.rs:113`.
- **Wasm blocker:** `run_on_large_stack` (`cg/mod.rs:122`) fails on wasm32.

**Tasks:**
1. **Real call edges.**
   - Method, constructor and static calls resolve with token text.
   - When Phase 6 finds nothing, fall back to the Phase 3 `call_target`, with resolution `"symbol_table"`.
   - Record each edge's dispatch (static, virtual, interface, lambda, constructor) and resolution (exact, cha, points_to, symbol_table).
   - Keep unresolved sites with a reason.
   - On wasm32, run Phase 6 inline, without the large-stack thread.
2. **`src/facts/`**, with the shapes of contract section 2.2 and deterministic ordering:
   - `FactsError { status, message }`
   - `resolve_symbol(s, spec)`: numeric id, then stable id, then qualified name, then simple name. Ambiguous is 400 with the candidates; unknown is 404.
   - `functions`, `callers`, `callees` (depth 1 to 6, breadth-first), `dependencies`, `dependents`, all returning `serde_json::Value`.
   - Occurrences are grouped per enclosing function by symbol spans. File and line come from the token corpus and `relative_paths`.
3. **Honest notes:** say in `notes` what is not derivable (receiver unknown, reflection, function pointers).

**State at stop:**
- **Edited:** `src/cg/resolution.rs` (+370), `src/cg/serializer.rs` (+147), `src/core/types/cg.rs` (+135), `src/cg/mod.rs` (+13), `src/core/pipeline.rs` (+12).
- **Last message:** "Now Phase 6 `mod.rs`: in-memory entry point, wasm inline path, Phase 3 combination, per-edge kinds and unresolved table."
- **Not done:** the `cg/mod.rs` rework, and all of `src/facts/` (not created).

---

## A2: query protocol, server route, CLI (Rust, coding only)

**Owns:** `src/adapters/query.rs`, `src/adapters/mod.rs`, `src/adapters/server.rs`, `src/adapters/http.rs`, `src/adapters/security.rs`, `src/adapters/web_repo.rs` (cfg gating only) and `src/main.rs`.

**Survey facts:**
- **`query.rs` is not compiled.** `adapters/mod.rs` doesn't declare it. Its doc promises `openheart query` and `POST /api/sessions/<id>/query`, but neither exists (`main.rs:327-335`; `server.rs:743-761`, `1509-1536`).
- **The server discards the engine.** It keeps only the session directory (`server.rs:1199-1251`).
- **Nothing is feature-gated.** There is no `cfg(feature)` anywhere; the `server` feature must gate `server`, `http`, `security` and `web_repo` (handoffs ENG_2-7).

**Tasks:**
1. **Gating:**
   - Declare `query`.
   - Gate the server-only adapters and the `server` CLI command with `#[cfg(feature = "server")]`.
2. **`QueryRequest` and dispatch:**
   - Add the variants `Functions`, `Callers`, `Callees`, `Dependencies`, `Dependents` and `View`, with the contract's names.
   - Add `execute_session(s: &AnalysisSession, req) -> Result<Value, QueryFailure>`. It dispatches to `facts` and `focused::view`, and to the existing queries through `s.artifacts.engine`.
   - Map `FactsError` to `QueryFailure` with the same status.
3. **Server:**
   - Keep up to 8 `AnalysisSession`s in memory, with deterministic eviction.
   - Build them with `AnalysisSession::from_run`, passing `runner::discover_project_files` and the source texts; reopen with `AnalysisSession::open(dir)`.
   - Add `POST /api/sessions/<id>/query`: same authentication as the other session routes, the existing body limits, JSON errors per the contract. Update `allowed_methods`.
4. **CLI:** `openheart query <analysis-dir> '<json>'` (also `@file`). It prints the response, and exits non-zero with the error JSON on failure. Update the help text.

**State at stop:**
- **Edited:** `src/adapters/mod.rs` (+19), `src/adapters/query.rs` (+247), `src/adapters/server.rs` (+326), `src/main.rs` (+234; A2's last message was "Now main.rs: gating plus the `query` command", so `main.rs` is mid-edit).
- **Not verified:** whether `http.rs`, `security.rs` and `web_repo.rs` gating was done.

---

## A3: derived and focused diagrams (Rust, coding only)

**Owns:**
- `src/scpg/diagram/**` (including the new `views/focused.rs`)
- `src/uma/**`
- `src/project/**` (new; A4 declares `pub mod project;`)

**Survey facts:**
- **How the JSON is produced:** `runner.rs:2384` → `factory.rs:428` → `JSONFactory` (277) → `export/json.rs:211-218` → `views::build` (`views/mod.rs:30`; dispatch 58-79).
- **Builders:**
  - `structural.rs`: class 54, object 233, package 367, component 484, composite 575, deployment 680, profile 753.
  - `behavioral.rs`: usecase 25, activity 65, statemachine 235, sequence 593, communication 793, interaction 883, timing 945.
  - `analysis.rs`: cfg 131, dfg 225, cdg 427, callgraph 541, robdd 758.
- **Always unavailable:** deployment, usecase and timing. Composite has no connectors (`uma/structural/composite_diagram.rs:48-53`).
- **Empty only for lack of call edges:** sequence, communication, interaction and callgraph.
- **Context:** `DiagramContext` is at `context.rs:388`.

**Tasks:**
1. **Provenance on nodes.** Every program-element node gets `symbol` (a stable id) and `qualified_name`. Nodes inside a single function's view get `function`.
2. **Project files in the context.** `DiagramContext` gets `project_files` plus a `with_project_files` setter.
3. **`src/project/` parsers**, extracting only what each file states, with small hand-written parsers and no new dependencies:
   - Dockerfile, docker-compose, Kubernetes
   - Maven, Gradle
   - `package.json`
   - `.csproj` and `.sln`
   - CMake
4. **Deployment view:** environments, containers and services as nodes, artifacts inside them, and deploy/manifest/depends_on edges as the files state them. Unavailable, with the reason, when no files describe deployment.
5. **Use case view:**
   - Actors by entry kind: `main`, HTTP routes, CLI parsers, schedulers, listeners.
   - Use cases are the entry functions; «include» edges come from call reachability between them.
6. **Timing view:** each lifeline's activation states (idle, active, waiting) over the message index, derived from the sequences.
7. **Composite connectors** from the call edges between parts' types.
8. **`views/focused.rs`:** `view(s, view, symbol, depth)` per the contract table. It reuses the existing builders on restricted inputs, and returns 422 with the reason when a view isn't derivable.

**State at stop:**
- **Written:** `src/project/{compose,docker,kubernetes,paths,xml,yaml}.rs`.
- **Missing:** `src/project/mod.rs`.
- **Not done:** tasks 1, 2 and 4-8 in `src/scpg/diagram` and `src/uma`, which have no changes.

---

## A4: in-memory pipeline, session, WebAssembly (Rust, coding only)

**Owns:**
- `src/adapters/runner.rs`
- `src/session.rs` (new), `src/wasm.rs` (new), `src/lib.rs`
- `Cargo.toml`, `.cargo/config.toml` (new), `scripts/build-wasm.sh` (new), `Makefile`
- `src/ingestion/**`, `src/ast/**`
- `src/core/**`, except the Phase 6 call in `core/pipeline.rs`
- `src/cfg/parallel.rs`
- `src/psa/mod.rs` (threading only)
- `src/scpg/{api,mmap,serializer}/**` and `src/scpg/mod.rs`

**Survey facts:**
- **One pipeline:** the CLI and the server both call `runner::run_pipeline` (`runner.rs:2185`). Phase 1 reads the files (`ingestion/mod.rs:142,306,404`); Phase 2 re-reads them and checks their SHA (`ast/mod.rs:265-286`).
- **The 0-call-edge cause:** Phase 6 at `runner.rs:2252` gets no tca.
- **Disk round trips:** artifacts are written and read back for hash links (`runner.rs:2157-2163, 2230-2258, 2333, 2396`).
- **Wasm blockers:**
  - `std::fs` for sources and artifacts
  - `canonicalize`
  - `process::id` (`core/io/fs.rs:23`)
  - threads (`core/parallel.rs`, `psa/mod.rs:337`, `run_isolated`'s 256 MiB stack)
  - memmap (the `EngineBuilder` default)
  - environment variables and `temp_dir`
- **Installed toolchain:** rustc 1.99.1 with the `wasm32-wasip1` target, and wasi-sdk 34 at `C:\Users\ahmad_hassan\wasi-sdk-34`.

**Tasks:**
1. Phase 6 with the token corpus.
2. **In-memory pipeline.**
   - A source provider lets Phases 1 and 2 read file texts from memory, with the SHA computed from the text.
   - `run_pipeline_in_memory(files, project_files, options)` never touches the file system.
   - Single-threaded on wasm32, with Phase 10 built in memory there.
   - The disk pipeline stays unchanged.
3. `discover_project_files(root)`, size-capped and skipping ignored directories.
4. `src/session.rs`, exactly per contract section 5 (`from_run`, `open(dir)`, `diagram_context`).
5. `src/wasm.rs`, exactly per contract section 4 (`oh_*` exports). The analyze result includes all 19 diagrams.
6. **Crate setup.**
   - `lib.rs`: declare `session`, `facts`, `project`, and `wasm` behind its feature.
   - `Cargo.toml`: feature `wasm = []`, with no cdylib in the default build.
   - `.cargo/config.toml`: a 64 MiB stack for `wasm32-wasip1`.
   - `scripts/build-wasm.sh` and `make wasm`, using `CC_wasm32_wasip1`, `AR_wasm32_wasip1` and `CFLAGS_wasm32_wasip1` with wasi-sdk's sysroot. The output goes to `web/engine/openheart.wasm`.

**State at stop:**
- **Edited:** `src/ingestion/sources.rs` (new), `src/ingestion/mod.rs` (+117), `src/ast/mod.rs` (+67), `src/core/io/fs.rs`, `src/core/io/mmap.rs`, `src/core/logger.rs`, `src/core/parallel.rs`, `src/cfg/parallel.rs`, `src/psa/mod.rs`, `src/scpg/api/builder.rs`, `src/scpg/mod.rs` (+47), `src/scpg/serializer/mod.rs`, `src/adapters/runner.rs` (+44).
- **Last message:** "Now the RunReport gets `relative_paths`."
- **Not done:** `src/session.rs`, `src/wasm.rs`, `lib.rs`, `Cargo.toml`, `.cargo/config.toml`, the build script, `Makefile`, and probably most of `run_pipeline_in_memory` and `discover_project_files`.

---

## A5: studio WebAssembly engine and click-to-explore (JavaScript; Node and browser tests allowed)

**Owns:**
- `web/js/engine/**` (new)
- `web/js/main.js`
- `web/js/graph-canvas.js` (exploration APIs only, no layout changes)
- `web/js/graph-loader.js` (copy `symbol`, `function`, `qualified_name` into node data)
- `web/js/studio-store.js`, `web/js/github-engine.js`
- `web/index.html`, `web/style.css`

`web/js/layout/**` is not A5's.

**Tasks:**
1. **`web/js/engine/wasi-shim.js`:** an in-memory WASI preview 1 shim implementing exactly the contract's imports. Anything else returns ENOSYS and is logged once.
2. **`wasm-engine.js`:**
   - Loads `engine/openheart.wasm`, and states a clear reason when it's absent.
   - Calls `_initialize`, and marshals JSON through `oh_*`.
   - Runs in a module worker, with a main-thread fallback.
3. **`program-model.js`:** one `{ kind, query }` interface over the server (`POST /api/sessions/<id>/query`) and over WebAssembly, with helpers for `callers`, `callees`, `dependencies`, `dependents`, `functions` and `view`.
4. **Import flow (GitHub, folder, ZIP):**
   - When the module is present, run wasm analyze on code files plus project files.
   - All 19 views come from `result.diagrams`; unavailable views show their reason.
   - The class-model path stays as the fallback, `IN_BROWSER_VIEWS` follows, and the program model is stored in the session.
5. **Relations tab** in the inspector, for nodes with `data.symbol` or `data.function`:
   - **Lists:** callers and callees as a tree with depth 1-4; dependencies (calls, reads, writes, types); dependents.
   - **Entries are clickable:** focus the element if it's on the canvas, otherwise open its focused view.
   - **Actions:** call graph from here, control flow, data flow, control dependence, sequence from here, impact, show in class diagram.
   - **Rendering:** focused views go through `validateGraphIr`, `loadGraphIrToCytoscape` and `canvas.renderGraph`.
   - **Navigation:** back/forward history and breadcrumbs.
   - **Feedback:** notes and errors are shown as messages.
   - **Quality bar:** keyboard accessible, Trusted Types safe (`web/js/dom.js` helpers).
6. **CSP:** add `'wasm-unsafe-eval'` to `script-src` and nothing else.
7. **Tests:** Node tests of the shim against a mocked instance, and a browser check with a mocked program model injected only from a test harness outside `web/`.

**State at stop:**
- **New:** `web/js/engine/{program-model,relations-panel,wasi-shim,wasm-core,wasm-engine,wasm-worker}.js`. `relations-panel.js` was being written; the last message was "Now the Relations panel module."
- **Edited:** `web/js/github-engine.js` (+110), `web/js/graph-canvas.js` (A5's part of +210), `web/js/graph-loader.js`, `web/js/studio-store.js` (+87), `web/index.html`.
- **Not done:** `web/js/main.js` wiring (unchanged), `web/style.css`, the tests.
- **Checked at stop:** every changed JavaScript file passes `node --check`; nothing else was run.

---

## Router: orthogonal edge router speed-up (JavaScript; tests allowed)

**Owns:** `web/js/layout/router.js` only.

**Done:**
- Every edge is routed, with no obstacle crossings and deterministic output (a work budget of A* expansions).
- Nudging cut shared-segment overlaps from thousands to tens.
- Suite: `scripts/layout-gate/test-router.mjs`.

**Remaining task (was in progress):**
- Cut the routing cost on the two largest inputs (OpenHeart's own `src/` class model: 1,768 nodes, 1,714 edges; and `synthetic/class-large-1200`) to about 4 s each, still 100% routed and deterministic.
- Ideas:
  - hierarchical routing between sibling packages first
  - bundling edges of the same package pair
  - cached visibility grids
  - a cheaper `strictlyInside`
  - search windows from the package-level corridor
- Recreate the inputs with `OH_DUMP_ROUTER=<dir> npm run gate -- class-large,rust-src/class`.

**State at stop:**
- The agent was mid-refactor ("`coarse` now has to exist before any edge is routed, and obstacles need a stable `order` field").
- The file on disk passes `node --check`, and a three-case gate run passed with it afterwards.
- Rerun `npm run router` and the full gate before building on it.
