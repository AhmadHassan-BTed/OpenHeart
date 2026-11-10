# OpenHeart rewrite checkpoint

Last updated: 10 November 2026. Base commit before this work: `fc218cd`. Branch: `main`.

This file is the single place to resume from. Everything needed to continue on any machine is in this file and in [`checkpoint/`](checkpoint/). **Start with section 0**, the most recent session; sections 1 to 5 are the 9 November rewrite state, still valid.

> Status: **OpenHeart builds, passes its tests and runs.** The CI steps (`cargo check`, `cargo fmt --check`, `cargo test`) are clean, `openheart analyze` and `openheart query` work, and the call graph has real edges (section 0.5). The web studio's layout engine is written and tested (section 0.2). The owner's instruction to make this hold opened building and testing for this checkpoint.

---

## 0. Session of 10 November 2026: resume here

### 0.1 What the owner asked for and decided

1. **Diagram layout.**
   - Build a highly sophisticated, adaptive layout: components near their parts, everything untangled, nothing placed far from what it relates to, a clean, intuitive placement.
   - Use proven premade solutions where they fit (ELK.js was chosen for the canvas); write our own where none fits.
   - **The core Rust engine gets no new dependency.**
2. **Process.**
   - Work directly on `main`, with no worktrees or branches.
   - Use parallel agents with written contracts, even for pieces that depend on each other through those contracts, without lowering quality.
3. **All 19 diagrams in the web, for every source,** including GitHub, folder and ZIP imports.
   - The diagrams must be **derived by the pure Rust engine**, compiled to WebAssembly for the browser.
   - **Click-to-explore:** clicking an element shows its call graph, each function's dependencies, and more.
4. **Phase rules.**
   - Until this checkpoint the Rust engine was coding only. The owner then asked that OpenHeart work, so it was built and tested (`cargo check`, `cargo fmt`, `cargo test`, and `openheart analyze`, `query` and `server`).
   - From now on, keeping `main` green (build and tests) is part of every change. Ask the owner before the WebAssembly build, and before large new Rust work.
   - The JavaScript layout's **test phase is open**: the quality gate may run.
5. **Toolchain (approved and installed).**
   - The `wasm32-wasip1` Rust target.
   - wasi-sdk 34 at `C:\Users\ahmad_hassan\wasi-sdk-34` (clang 23.1). Its 619 MB download's SHA-256 matched GitHub's published digest.
   - Rust 1.99.1 (stable MSVC) is on this machine.

### 0.2 Done

**Adaptive layout engine** (`web/js/layout/`):

| Module | What it does |
|---|---|
| `geometry.js` | One source of truth for package padding, label bands and edge-label sizes, shared with the stylesheet |
| `semantics.js` | A profile per diagram type: family, conventional directions, relationship roles as ELK priorities, first/last-layer pinning, spacing |
| `model.js` | The containment model, pruning of empty package chains, edge roles, parallel-message merging for communication diagrams |
| `engine.js` | The pipeline. Components, detached-child packing, a candidate portfolio scored by metrics, the deterministic work budget, write-back. Candidates: ELK layered, stress + VPSC + routing, and a "cluster" strategy (packages laid out bottom-up, arranged by stress with aspect correction, mirrored toward their partners, cross-package edges routed) |
| `elk-backend.js` | ELK layered (INCLUDE_CHILDREN, orthogonal routing) and stress |
| `router.js` | Orthogonal obstacle-avoiding router with nudging |
| `overlap.js` | VPSC overlap removal and compaction |
| `pack.js` | Flexible grid and skyline packing |
| `metrics.js` | Quality metrics and score |
| `sequence.js` | Sequence diagrams (lifelines, one row per message, self messages drawn in the lifeline card) and timing |
| `apply-routes.js` | Draws routes and package boxes exactly in Cytoscape; moves invalidate only what they break |

**Wiring:**
- `web/js/graph-canvas.js` awaits the engine, with the legacy `uml-layout.js` as fallback, and applies geometry. It also re-applies geometry after a theme change and clears package sizing on collapse.
- The theme takes its package geometry from `geometry.js`, and labels are centred above packages.
- `web/index.html` loads `elkjs@0.9.3/lib/elk.bundled.js`, which is also added to the CSP.

**Quality gate:** `scripts/layout-gate` (how to run: its README). Last full result, re-run from the repo on the pushed code (`1374c2a`) after the stop:
- All 54 inputs pass every hard check, with deterministic output: 19 bundled diagrams, 12 real-code class/package models, 9 synthetic graphs, and a stress input for each diagram type.
- In the live browser, ELK loads under the CSP with no console errors, drawn bends match the routes to 0.002 px, and labels land where ELK reserved them.

**Contract** for the Rust and studio work: [`docs/engine-query-protocol.md`](docs/engine-query-protocol.md):
- symbol ids on diagram nodes
- the query JSON protocol (functions, callers, callees, dependencies, dependents, focused views)
- the CLI, server and WebAssembly transports
- the `oh_*` WebAssembly exports
- the Rust module layout

### 0.3 Packages: done for this checkpoint, and still open

Six packages were running in parallel when the owner stopped them. Their briefs, the files they own, and what each had on disk at the stop are in [`checkpoint/session-2026-11-10/agent-briefs.md`](checkpoint/session-2026-11-10/agent-briefs.md). Afterwards, only what `main` needed to build and run was completed; everything else stays open.

| Package | Done (verified at this checkpoint) | Still open |
|---|---|---|
| A1 call graph and facts | Phase 6 runs with the token corpus in the runner and in `core/pipeline.rs` (the 0-edge cause). Every edge records its dispatch and resolution kind (points-to refinement included), and unresolved sites keep the resolver's reason. `src/facts/`: `resolve_symbol`, `functions`, `callers`, `callees` (from the engine's call-graph index, with site locations from the token corpus) and `dependents` (from impact analysis at depth 1) | Per-function field reads and writes, and type uses, in `dependencies`/`dependents` (they are returned empty, with a note); wasm32 inline path for Phase 6 |
| A2 query, server, CLI | `QueryRequest` new variants, `execute_session`, `POST /api/sessions/<id>/query`, `openheart query`, `PathCount` output fixed | Review of the `cfg(feature = "server")` gating in `http.rs`, `security.rs` and `web_repo.rs` |
| A3 diagrams | `views/focused.rs`: whole-program views, and views focused on a symbol through the builders' existing focus | `symbol`/`function` ids on every Graph IR node; `src/project/` (parsers written, no `mod.rs`, not compiled); deployment, use case and timing derivations; composite connectors; focus deeper than one level |
| A4 pipeline, session, wasm | `src/session.rs` (`from_run`, `open(dir)` reloading every artifact, `diagram_context`); `discover_project_files`; `PipelineRun` fields; `lib.rs` declares `session` and `facts` | `run_pipeline_in_memory`, `src/wasm.rs`, the `wasm` feature, `.cargo/config.toml`, `scripts/build-wasm.sh`; declaring `project` once it has a `mod.rs` |
| A5 studio | Nothing new; the studio works as deployed | WASI shim, wasm loader and program model review; `relations-panel.js` finish; `main.js` wiring; tests |
| Router | Passes the full gate | Speed on about 1,700-card inputs (16-18 s total) |

Tests were updated to the current engine:
- Format-version assertions use the library's constants.
- Symbols are found by name, not by hard-coded id.
- The SCC assertions follow the documented invariant: SCCs cover the call-graph nodes, not the whole symbol-id space.
- Constructors now get CFGs, so the function counts include them.
- The engine summary no longer claims "all 14 UML diagrams".

### 0.4 What remains, in order

1. **Finish A1-A5** from the "Still open" column, with the same file ownership and the briefs. Keep `main` green: run `cargo check --all-targets`, `cargo fmt --all -- --check` and `cargo test --all-targets` after each package.
2. **WebAssembly** (ask the owner first):
   1. Finish A4's wasm items.
   2. Run `scripts/build-wasm.sh`.
   3. Test the studio end to end: a GitHub import, all 19 views, and the Relations tab (A5).
3. **Layout follow-ups:**
   - Router speed on about 1,700-card inputs.
   - Sprawl on dense random graphs: mean edges of 7-11 card sizes on stress inputs.
   - The aspect correction made the random 120-function call graph worse (keep the better of corrected and uncorrected).
   - The Message layer toggle doesn't hide self messages drawn in lifelines.
   - SVG export of overflowing lifeline images is unchecked.
   - Label collisions remain on dense graphs.
   - The stylesheet has invalid Cytoscape properties that predate this work (`border-radius`, `letter-spacing`, `taxi-radius`, `shadow-*`).
4. **Update `docs/status.md`** per diagram as derivations land.
5. **Then sections 3 to 7 below:** the remaining audit fix passes, and wiring the new modules (`src/ir`, `src/analysis`, `src/derive` and the rest).

### 0.5 Build and run state at this checkpoint (verified locally)

| Check | Result |
|---|---|
| `cargo check --all-targets` | 0 errors |
| `cargo fmt --all -- --check` | clean; the whole tree was formatted once, since it had never been `rustfmt`-clean |
| `cargo test --all-targets` | 73 passed, 0 failed, 1 ignored (the existing `#[ignore]`) |
| `openheart analyze examples/test_patterns_codebase <out>` | all 10 phases in 268 ms over 35 files; **68 call sites, 47 call edges** (0 at the previous checkpoint); 57 diagram files, 0 unavailable |
| `openheart query <out> '{"query":"callees","symbol":"com.patterns.behavioral.templatemethod.DataMiner.mine","depth":2}'` | 7 edges, for example `DataMiner.mine` → `CsvDataMiner.openFile` (virtual, CHA) at `DataMiner.java:5` |
| `query` operations `functions`, `dependents` and `view`, plus an unknown symbol | all answer per the protocol. The callgraph view focused on `DataMiner.mine` has 8 nodes and 7 edges. The unknown symbol gives a 404 error object and exit code 1 |
| `openheart server 18734` | `/api/health` reports online; `/api/supported-diagrams` answers; `POST /api/sessions/<id>/query` answers (404 for an unknown session) |
| Live studio `https://ahmadhassan-bted.github.io/OpenHeart/` | all 19 views render with routes and no console errors (the web code is unchanged since) |
| Layout gate (`scripts/layout-gate`) | 54/54 inputs pass, deterministic (the web code is unchanged since) |

### 0.6 Cautions

- **Pushing `main` deploys the studio to GitHub Pages** (`.github/workflows/deploy_pages.yml` runs on push to `main`). A5's unfinished studio modules are on `main` but not wired into `main.js`, so they are inert.
- **`src/project/` and the other undeclared modules** (`src/ir`, `src/analysis`, `src/derive`, and the rest) are not compiled. Declaring one means making it compile.
- **Commit identity:** commits use the personal identity (`Ahmad Hassan (B-Ted) <ahmadhassan.bted@gmail.com>`). The shell injects a work identity through `GIT_*` variables, so override them on every commit.

---

## 1. Standing rules from the owner

Follow these when resuming. They override defaults.

1. **Work directly on `main`**, in the current code. No worktrees or side branches unless the owner asks.
2. **Phase order is strict.** Right now this is the pure coding phase: implement fixes and features only. No compiling, linting, testing, benchmarking, verification or re-checking until the owner says the next phase starts.
3. **OpenHeart is language-independent.** It must support every language through pluggable frontends, and every algorithm must be language-agnostic. Perfect two families first: **JVM** (Java, Kotlin) and **C family** (C, C++, C#/.NET).
4. **Never hard-code, template or guess.** Work the way compilers do, statically: parse, resolve and type, lower to an IR with explicit memory operations, model the heap, abstractly interpret, and derive calls, packages, execution flow, sequences, architecture and memory-level behavior from those facts. When something is not derivable, say so explicitly (`NotDerivable`), never invent it.
5. **Semantics exactly per each language specification** (JLS, ISO C and C++, ECMA-334 C#, Kotlin specification). Keep the token layer lossless (comments, whitespace, preprocessor trivia).
6. **Maximum parallelism, no redundant work.** Run independent work at the same time; never repeat a step or run one that adds nothing.

## 2. What was done

### 2.1 Detection (complete)

An exhaustive raw audit of the whole repository, not verified (by instruction):

| Round | What | Result |
|---|---|---|
| Round 1 | 9 area auditors: core/ingestion/AST, symbols, CFG/SSA/call graph, traceability/ROBDD, UML/patterns, unified graph + exporters, server/CLI, web studio, engineering/docs | top issues per area |
| Lenses | 7 cross-cutting auditors: end-to-end data flow, benchmark integrity, scale, robustness, web security, test suite, language support | cross-area issues |
| Research | benchmark standards, competitor capability matrix, automated checks | `checkpoint/research.json` |
| Round 2 | 10 exhaustive no-cap sweeps, file by file, skipping everything already reported | the long tail |

**858 unique findings**, all in [`checkpoint/findings.json`](checkpoint/findings.json), browsable in [`checkpoint/FINDINGS.md`](checkpoint/FINDINGS.md). Line numbers in findings refer to `fc218cd`; many lines have since moved.

### 2.2 Fixing on `main`

Fix engineers each own a disjoint set of paths (ownership map in section 5). Overall finding status at this checkpoint: **53 fixed, 172 partial, 16 skipped, 530 queued for the next passes, 86 deferred to the test phase, 1 unrouted.**

| Owner | Paths | Pass 1 (fixed / partial / skipped) | Pass 2 |
|---|---|---|---|
| ENG | Cargo.toml, Makefile, dotfiles | 0 / 14 / 27 | 10 / 9 / 25 |
| BDD | src/psa, src/tra | 9 / 16 / 1 | stopped mid-way (edits on disk, no report) |
| CORE | src/core, src/ingestion, src/ast | 6 / 34 / 6 | queued: `CORE-2.json` (190) |
| SYM | src/symbol | stopped mid-way (edits on disk, no report) | queued: `SYM-2.json` (88) |
| FLOW | src/cfg, src/ssa, src/cg | 11 / 20 / 5 | queued: `FLOW-2.json` (106) |
| UML | src/uma | 8 / 21 / 0 | queued: `UML-2.json` (97) |
| SCPGX | src/scpg/diagram | 6 / 20 / 1 | stopped mid-way (edits on disk, no report) |
| SCPG | src/scpg (rest) | 4 / 13 / 0 | queued: `SCPG-2.json` (64) |
| SRV | src/adapters, src/main.rs, src/lib.rs, Docker, run scripts | 20 / 28 / 6 | stopped mid-way (edits on disk, no report) |
| WEB | web/ | 17 / 25 / 4 | queued: `WEB-2.json` (90) |
| DOCS | README, ROADMAP, CHANGELOG, SECURITY, docs/ | 7 / 64 / 0 | queued: `DOCS-2.json` (57) |

Per-finding status (fixed / partial / skipped / in-queue / not-yet-dispatched / deferred-testing-phase) is the `status` field in `checkpoint/findings.json`. Raw engineer results are in [`checkpoint/fix-results.json`](checkpoint/fix-results.json).

Notable changes already on `main`: tree-sitter moved to 0.25 with Java/JavaScript/C/C++/C# grammars 0.23 and `tree-sitter-kotlin-ng` 1.x; overflow checks enabled in dev/test; a `server` cargo feature; a shared pipeline runner (`src/adapters/runner.rs`) for CLI and server; server hardening work; a rewritten ROBDD core (`src/psa/bdd/library.rs`, `src/psa/count.rs`); UML model extended with real relationships, fragments, explicit unknowns; XSS-safe DOM helpers in the web studio (`web/js/dom.js`, `web/js/storage.js`); docs and README claims corrected, with `docs/status.md` as the honest status page.

### 2.3 New architecture (being written at checkpoint; new files only)

| Module | Purpose |
|---|---|
| `src/lang/neutral.rs`, `frontend.rs`, `registry.rs` | Universal vocabulary every frontend maps into; the `LanguageFrontend` trait; the single registry and list of supported languages |
| `src/lang/java`, `kotlin`, `c`, `cpp`, `csharp` | Frontends: Phase 1 ingestion, Phase 2 reduction, Phase 3 semantics, standard-library / BCL models |
| `src/lang/profile.rs`, `build/`, `preprocess.rs`, `project_model.rs` | Language registry by extension, compile_commands.json, CMake, .sln/.csproj, C/C++ include graph and conditional compilation, language-neutral project model |
| `src/lang/uml_semantics.rs`, `idioms.rs` | Ownership/multiplicity semantics (C++ value vs unique_ptr vs shared_ptr vs raw pointer; C# properties, events, structs) and pattern idioms |
| `src/ir` | Typed three-address SSA IR with explicit memory operations, lowering from the AST, dominators and loops |
| `src/memory` | Abstract heap, field-sensitive points-to with on-the-fly call graph, escape, ownership, mod/ref |
| `src/analysis` | Lattices, worklist solvers, SCCP, intervals, nullness, typestate, control dependence, IFDS/IDE, contexts, canonical branch conditions |
| `src/derive` | Every diagram derived from facts with provenance and a Derivation tag |
| `src/symex` | Bounded static symbolic execution: stack, heap, path conditions, execution traces |
| `src/abi` | C/C++ Itanium and MSVC, JVM and CLR object layouts, lifetimes, calling conventions |
| `src/arch` | Architecture recovery: dependency graph, DSM, layers, clustering, coupling metrics, boundaries |
| `src/memview` | Lifetime, ownership, heap-shape and stack-frame views; memory-safety and data-race facts |
| `web/js/lang/c-family.js` | Linear-time C/C++/C# parser for the in-browser studio |

The shared contracts these modules follow (exact type and module names) are in [`checkpoint/tooling/workflows/compiler-core.js`](checkpoint/tooling/workflows/compiler-core.js) (`CONTRACT`) and [`checkpoint/tooling/workflows/new-frontends.js`](checkpoint/tooling/workflows/new-frontends.js) (`CONTRACT`). Files created by each engineer are listed in [`checkpoint/new-modules.json`](checkpoint/new-modules.json).

## 3. Stopped at this checkpoint (continue exactly from here)

To close the session, these engineers were stopped before they reported. Everything they had written is committed; their per-finding results and handoffs were not produced. Progress of every sub-agent (status, tool calls, files edited, last message) is in [`checkpoint/session/agents-index.json`](checkpoint/session/agents-index.json) when present.

- Fix pass 1: SYM. Rerun `checkpoint/queues/SYM-1.json`, then `SYM-2.json`.
- Fix pass 2: BDD, SRV, SCPGX. Rerun `BDD-2.json`, `SRV-2.json`, `SCPGX-2.json`, then the `-3` queues.
- New modules, files on disk at stop time: `src/lang` 29 files (C/C++ and C# frontends and semantics, build systems, UML semantics, language-neutral core, Kotlin), `src/ir` 16, `src/analysis` 14, `src/derive` 12, `src/arch` 7, `src/memview` 6, `src/symex` 3, `src/abi` 3, `web/js/lang/c-family.js`. Only the memory model (`src/memory`, 7 files) and the C# frontend (`src/lang/csharp`) finished and reported. Complete the rest against their contracts in `checkpoint/tooling/workflows/compiler-core.js` and `new-frontends.js`. These modules are not yet declared in `src/lib.rs`, so they are not compiled into the current build.

### Build status at this checkpoint

- `cargo build` succeeds (Rust 1.99.1, MSVC, Windows) with `Cargo.lock` regenerated. 31 integration compile errors from the parallel edits were fixed (tree-sitter 0.25 API, missing modules and constants, signature and field-type mismatches, new enum variants, borrow errors).
- `openheart --help` runs, and `openheart analyze examples/test_patterns_codebase <out>` completes all 10 phases on 35 files in about 0.74 s, writing every phase artifact plus 57 diagram files and `run.json`.
- Visible gap in that run: **0 call edges resolved** (68 call sites, 0 edges). Call resolution is still being rebuilt (FLOW queue, `MANDATE-CORE-ARCH-FLOW`, finding E2E-01).
- The new modules in section 3 are not declared in `src/lib.rs`, so they are excluded from this build until they are wired in (section 4, step 2).
- Nothing beyond building and that one run was checked, by the owner's instruction.

## 4. Not started yet, in order

1. **Remaining fix passes.** Ready-made queues are in [`checkpoint/queues/`](checkpoint/queues/) as `<OWNER>-<pass>.json` (round-2 findings, cross-cutting findings, handoffs from other engineers, and a `MANDATE-CORE-ARCH-<OWNER>` item that says how that owner rewires its phase onto the new language-neutral, compiler-grade core):
   - Pass 2: `CORE-2` (190), `SYM-2` (88), `FLOW-2` (106), `UML-2` (97), `SCPG-2` (64), `WEB-2` (90), `DOCS-2` (57).
   - Reruns of the stopped work: `SYM-1`, `BDD-2`, `SRV-2`, `SCPGX-2`.
   - Pass 3 (handoffs addressed to owners that already ran pass 2): `ENG-3` (25), `BDD-3` (17), `SRV-3` (26), `SCPGX-3` (5).
   - Finish the stopped new modules (section 3).
2. **Integration wiring**: module declarations (`src/lib.rs`: `pub mod lang; ir; memory; analysis; derive; symex; abi; arch; memview;`, `src/lang/mod.rs` entries for neutral/frontend/registry/java/kotlin), new `LangId` and `ASTNodeType` variants, registry registration, call-site updates. Every required change is a handoff in [`checkpoint/handoffs.json`](checkpoint/handoffs.json).
3. **`Cargo.lock`** was regenerated at this checkpoint against tree-sitter 0.25 and the 0.23 grammars (Rust 1.99.1 was installed on the original machine; rust-version in Cargo.toml is 1.88).
4. **Build phase** (only when the owner says): `cargo check`, fix compile errors across the parallel edits, then clippy.
5. **Test phase** (only when the owner says): 86 findings are deferred as `deferred-testing-phase` in findings.json, plus the test architecture from the TST lens and the proposed OpenHeart Bench in `checkpoint/research.json` (`context:BENCH`).
6. **CI, release, Pages and Dependabot changes**: handoffs that target `.github/` (kept out of the coding phase because they configure checks).
7. **Final docs pass** once code settles, then publish the audit report if wanted (`checkpoint/tooling/template.html` + `build.js`).

## 5. How to resume on any machine

1. `git pull` on `main`. Install Node 20+ and Python 3.12+ (tooling). Install Rust 1.88+ only when moving to the build phase.
2. Open Claude Code in the repo root and say: *"Resume from CHECKPOINT.md."* Point it at section 4.
3. Fix passes use one engineer per owner, all in parallel, editing only their own paths:

| Owner | Paths |
|---|---|
| CORE | src/core/**, src/ingestion/**, src/ast/** |
| SYM | src/symbol/** |
| FLOW | src/cfg/**, src/ssa/**, src/cg/** |
| BDD | src/psa/**, src/tra/** |
| UML | src/uma/** |
| SCPGX | src/scpg/diagram/** |
| SCPG | src/scpg/** except diagram |
| SRV | src/adapters/**, src/main.rs, src/lib.rs, Dockerfile, run.sh, run.bat, scripts/restart_server.sh, .env.example |
| WEB | web/** |
| ENG | Cargo.toml, Makefile, .gitignore, .editorconfig, .gitattributes |
| DOCS | README.md, ROADMAP.md, CHANGELOG.md, SECURITY.md, SUPPORT.md, CODE_OF_CONDUCT.md, docs/**, examples/README.md |
| new modules | src/lang/**, src/ir/**, src/memory/**, src/analysis/**, src/derive/**, src/symex/**, src/abi/**, src/arch/**, src/memview/**, web/js/lang/** |

4. Workflow scripts used this session are in [`checkpoint/tooling/workflows/`](checkpoint/tooling/workflows/). `fix-pass.js` takes `{ owners: [...], pass: N }` and reads `<FIX><OWNER>-<pass>.json`; change its `FIX` constant to this repo's `checkpoint\queues\` path before reuse. `dispatch.js` built the queues from this session's agent journals; on a new machine, build further queues from `checkpoint/findings.json` and `checkpoint/handoffs.json` instead (filter by `status`, route by the ownership map above).

## 6. Files in `checkpoint/`

| File | Contents |
|---|---|
| `findings.json` | Every finding: id, source, kind, category, severity, title, files, evidence, impact, recommendation, effort, confidence, routing (`owners`), per-owner fix status (`fix`), overall `status` |
| `FINDINGS.md` | Human-readable index of all findings with status |
| `fix-results.json` | Each fix engineer's per-finding result and files changed |
| `handoffs.json` | Every cross-owner change request (file, change, reason, from) |
| `new-modules.json` | Files created by each new-module engineer and their notes |
| `area-summaries.json` | Maturity summary and strengths per audited area |
| `research.json` | Benchmark-suite design, competitor capability matrix, automated-check metrics |
| `queues/` | Fix batches per owner and pass (`<OWNER>-<pass>.json`) |
| `tooling/` | dispatch, export, watcher scripts, report template and builder, workflow scripts |

## 7. Commit identity

Commit only as **Ahmad Hassan (B-Ted) <ahmadhassan.bted@gmail.com>**. On the original machine the shell environment sets `GIT_AUTHOR_*` / `GIT_COMMITTER_*` to a work identity that overrides git config, so set the four variables explicitly on every commit and check `git log -1 --format='%an <%ae> / %cn <%ce>'` before pushing.

## 8. Known caveats

- The code is uncompiled. Many engineers edited in parallel; expect mismatches at API boundaries, which the integration step resolves.
- Some findings may already be fixed by another owner's change; check before re-fixing.
- `.gitattributes` now normalizes line endings, so Git reports CRLF to LF conversions on touched files.
