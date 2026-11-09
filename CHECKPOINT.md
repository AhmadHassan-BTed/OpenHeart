# OpenHeart rewrite checkpoint

Last updated: 9 November 2026. Base commit before this work: `fc218cd`. Branch: `main`.

This file is the single place to resume from. Everything needed to continue on any machine is in this file and in [`checkpoint/`](checkpoint/).

> Status: **coding phase, uncompiled.** Nothing in this checkpoint has been compiled, tested or verified, by the owner's instruction. Expect build errors until the integration and build steps below are done.

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
